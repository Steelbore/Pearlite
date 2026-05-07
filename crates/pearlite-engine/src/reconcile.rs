// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Reconcile-import and reconcile-commit (PRD §11, Plan §7.5 M4 W1,
//! ADR-0014).
//!
//! Two entry points:
//!
//! - [`Engine::reconcile`] — read-only fresh-import. Probes the live
//!   system, renders a host file via
//!   [`pearlite_nickel::emit_host`](pearlite_nickel::emit_host), and
//!   atomically writes `<config_dir>/hosts/<hostname>.imported.ncl` for
//!   operator review. No `state.toml` mutation; the file is a review
//!   draft. Validation happens on the next `pearlite plan` once the
//!   operator hand-curates and renames it to `hosts/<hostname>.ncl`.
//! - [`Engine::reconcile_commit`] — the write-side of reconcile.
//!   Probes, classifies Manual drift, takes per-package adopt/skip
//!   decisions from the caller, and writes `state.adopted` plus one
//!   `[[reconciliations]]` entry. Drift-threshold enforcement and the
//!   interactive prompt loop live at the CLI boundary; this method is
//!   pure mechanism (ADR-0014 §2).

use crate::Engine;
use crate::errors::{ReconcileCommitError, ReconcileError};
use pearlite_diff::{classify_cargo, classify_pacman};
use pearlite_schema::{PackageSet, RemovePolicy};
use pearlite_state::{ReconciliationAction, ReconciliationEntry, StateStore};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use time::OffsetDateTime;
use uuid::Uuid;

/// Result of a successful [`Engine::reconcile`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Absolute path of the written `.imported.ncl` file.
    pub path: PathBuf,
    /// Hostname the file was rendered for.
    pub hostname: String,
}

/// Caller-supplied resolution of the Manual drift items found by a
/// reconcile-commit (ADR-0014).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileDecisions {
    /// Adopt every Manual item silently. Maps to
    /// [`ReconciliationAction::AdoptAll`] in the persisted record.
    AdoptAll,
    /// Per-package decisions taken by an interactive prompt loop. Only
    /// names in `adopt` are written to `state.adopted`; all other
    /// Manual items are recorded as skipped. Names in `adopt` that are
    /// not in the probed Manual set are silently dropped — the prompt
    /// loop in `pearlite-cli` only ever surfaces Manual items, so a
    /// mismatch implies stale caller state.
    ///
    /// Maps to [`ReconciliationAction::Interactive`] regardless of the
    /// final ratio of adopt vs. skip.
    Selective {
        /// Names the operator chose to adopt.
        adopt: BTreeSet<String>,
    },
}

/// Result of a successful [`Engine::reconcile_commit`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileCommitOutcome {
    /// Plan UUID generated for this commit (also written into
    /// `state.reconciliations`).
    pub plan_id: Uuid,
    /// UTC timestamp the commit was recorded.
    pub committed_at: OffsetDateTime,
    /// Resolution policy as recorded in `state.toml`.
    pub action: ReconciliationAction,
    /// Number of Manual drift items considered (audit denominator).
    pub considered: u32,
    /// Names actually moved into `state.adopted`. Sorted, deduplicated.
    pub adopted: Vec<String>,
    /// Names the operator declined. Sorted, deduplicated.
    pub skipped: Vec<String>,
}

impl Engine {
    /// Probe the live system and write
    /// `<config_dir>/hosts/<hostname>.imported.ncl`.
    ///
    /// Refuses to clobber an existing `.imported.ncl` —
    /// [`ReconcileError::AlreadyExists`] surfaces and the operator
    /// removes or renames the existing file before retrying.
    ///
    /// The hostname comes from the probe (typically `/etc/hostname`).
    /// An empty hostname yields [`ReconcileError::EmptyHostname`]; a
    /// hostname containing path separators or NUL yields
    /// [`ReconcileError::InvalidHostname`].
    ///
    /// # Errors
    /// - [`ReconcileError::Probe`] — adapter or hostname-read failure.
    /// - [`ReconcileError::EmptyHostname`] — see above.
    /// - [`ReconcileError::InvalidHostname`] — see above.
    /// - [`ReconcileError::AlreadyExists`] — target already on disk.
    /// - [`ReconcileError::Io`] — mkdir or atomic-write failure.
    pub fn reconcile(&self, config_dir: &Path) -> Result<ReconcileOutcome, ReconcileError> {
        let probed = self.probe().probe()?;
        let hostname = validate_hostname(&probed.host.hostname)?;
        let nickel_text = pearlite_nickel::emit_host(&probed);

        let hosts_dir = config_dir.join("hosts");
        let target = hosts_dir.join(format!("{hostname}.imported.ncl"));

        if target.exists() {
            return Err(ReconcileError::AlreadyExists { path: target });
        }

        std::fs::create_dir_all(&hosts_dir).map_err(|e| ReconcileError::Io {
            path: hosts_dir.clone(),
            source: e,
        })?;

        write_text_atomic(&target, nickel_text.as_bytes())?;

        Ok(ReconcileOutcome {
            path: target,
            hostname: hostname.to_owned(),
        })
    }

    /// Probe the live system, classify Manual drift, adopt or skip per
    /// `decisions`, and append a `[[reconciliations]]` entry to the
    /// `state.toml` at `state_path` (ADR-0014, M4 W1).
    ///
    /// Manual drift here is "installed but not Pearlite-managed and not
    /// already adopted" — `classify_pacman` / `classify_cargo` driven by
    /// an empty declared `PackageSet`, since reconcile-commit is the
    /// fresh-import path that runs *before* the operator curates a host
    /// file (PRD §11). On a brand-new host every explicitly-installed
    /// package is Manual and will trip the threshold; that is the case
    /// `--adopt-all` exists for (ADR-0014 §2).
    ///
    /// `threshold` is the engine-side belt-and-suspenders cap from
    /// ADR-0014 §2: when `Some(N)`, refuse with
    /// [`ReconcileCommitError::ThresholdExceeded`] if the Manual count
    /// exceeds `N`. `None` means unbounded — typically what
    /// `pearlite-cli` passes when the operator opted into bulk adoption
    /// via `--adopt-all` without `--commit-threshold`.
    ///
    /// State writes:
    ///
    /// - Adopted names are unioned with the existing `state.adopted`
    ///   for the matching subsystem (pacman / cargo) and re-sorted.
    /// - One [`ReconciliationEntry`] is appended to
    ///   `state.reconciliations` with a fresh `Uuid::now_v7()`,
    ///   `committed_at`, the policy enum, the considered count, and
    ///   the resolved `adopted` / `skipped` vectors.
    /// - `state.last_modified` is updated to the commit timestamp.
    ///   `state.last_apply` is *not* touched — reconcile is not an
    ///   apply.
    ///
    /// The whole record is written atomically via
    /// [`StateStore::write_atomic`].
    ///
    /// # Errors
    /// - [`ReconcileCommitError::Probe`] — adapter or I/O failure during
    ///   probe.
    /// - [`ReconcileCommitError::State`] — `state.toml` read or atomic
    ///   write failed.
    /// - [`ReconcileCommitError::ThresholdExceeded`] — Manual count
    ///   exceeds the supplied threshold.
    pub fn reconcile_commit(
        &self,
        state_path: &Path,
        decisions: &ReconcileDecisions,
        threshold: Option<u32>,
    ) -> Result<ReconcileCommitOutcome, ReconcileCommitError> {
        let probed = self.probe().probe()?;
        let store = StateStore::new(state_path.to_path_buf());
        let mut state = store.read()?;

        let manual_pacman: Vec<String> = match probed.pacman.as_ref() {
            Some(inv) => {
                classify_pacman(
                    &PackageSet::default(),
                    &RemovePolicy::default(),
                    inv,
                    &state,
                )
                .manual
            }
            None => Vec::new(),
        };
        let manual_cargo: Vec<String> = match probed.cargo.as_ref() {
            Some(inv) => classify_cargo(&[], inv, &state).manual,
            None => Vec::new(),
        };

        let considered =
            u32::try_from(manual_pacman.len() + manual_cargo.len()).unwrap_or(u32::MAX);

        if let Some(limit) = threshold {
            if considered > limit {
                return Err(ReconcileCommitError::ThresholdExceeded {
                    count: considered,
                    threshold: limit,
                });
            }
        }

        let (adopted_pacman, skipped_pacman, adopted_cargo, skipped_cargo, action_kind) =
            match decisions {
                ReconcileDecisions::AdoptAll => (
                    manual_pacman,
                    Vec::new(),
                    manual_cargo,
                    Vec::new(),
                    ReconciliationAction::AdoptAll,
                ),
                ReconcileDecisions::Selective { adopt } => {
                    let (a_p, s_p) = partition_by_adopt(manual_pacman, adopt);
                    let (a_c, s_c) = partition_by_adopt(manual_cargo, adopt);
                    (a_p, s_p, a_c, s_c, ReconciliationAction::Interactive)
                }
            };

        merge_sorted(&mut state.adopted.pacman, adopted_pacman.iter());
        merge_sorted(&mut state.adopted.cargo, adopted_cargo.iter());

        let mut adopted_all: Vec<String> =
            adopted_pacman.into_iter().chain(adopted_cargo).collect();
        adopted_all.sort();
        adopted_all.dedup();

        let mut skipped_all: Vec<String> =
            skipped_pacman.into_iter().chain(skipped_cargo).collect();
        skipped_all.sort();
        skipped_all.dedup();

        let plan_id = Uuid::now_v7();
        let committed_at = OffsetDateTime::now_utc();

        state.reconciliations.push(ReconciliationEntry {
            plan_id,
            committed_at,
            action: action_kind,
            package_count: considered,
            adopted: adopted_all.clone(),
            skipped: skipped_all.clone(),
        });
        state.last_modified = Some(committed_at);

        store.write_atomic(&state)?;

        Ok(ReconcileCommitOutcome {
            plan_id,
            committed_at,
            action: action_kind,
            considered,
            adopted: adopted_all,
            skipped: skipped_all,
        })
    }

    /// Probe + classify Manual drift without writing anything (ADR-0014).
    ///
    /// Returns the merged, sorted, deduplicated list of pacman + cargo
    /// names that would be candidates for `reconcile_commit` adoption.
    /// Used by the CLI to drive its threshold check (clean error before
    /// the engine does any work) and the per-package prompt loop.
    ///
    /// # Errors
    /// - [`ReconcileCommitError::Probe`] — adapter or I/O failure during
    ///   probe.
    /// - [`ReconcileCommitError::State`] — `state.toml` read failed.
    pub fn probe_manual_drift(
        &self,
        state_path: &Path,
    ) -> Result<Vec<String>, ReconcileCommitError> {
        let probed = self.probe().probe()?;
        let store = StateStore::new(state_path.to_path_buf());
        let state = store.read()?;

        let mut manual: Vec<String> = match probed.pacman.as_ref() {
            Some(inv) => {
                classify_pacman(
                    &PackageSet::default(),
                    &RemovePolicy::default(),
                    inv,
                    &state,
                )
                .manual
            }
            None => Vec::new(),
        };
        if let Some(inv) = probed.cargo.as_ref() {
            manual.extend(classify_cargo(&[], inv, &state).manual);
        }
        manual.sort();
        manual.dedup();
        Ok(manual)
    }
}

/// Split `manual` into `(adopted, skipped)` based on membership in
/// `adopt`. Names in `adopt` that are not in `manual` are silently
/// dropped (caller's prompt loop only surfaces Manual items, so this
/// only fires on stale caller state).
fn partition_by_adopt(manual: Vec<String>, adopt: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let mut adopted = Vec::new();
    let mut skipped = Vec::new();
    for name in manual {
        if adopt.contains(&name) {
            adopted.push(name);
        } else {
            skipped.push(name);
        }
    }
    (adopted, skipped)
}

/// Union `incoming` into `existing`, sort, and dedup. Used to merge
/// freshly-adopted names into `state.adopted.{pacman,cargo}` without
/// disturbing the existing entries' relative order beyond the post-sort
/// canonicalisation.
fn merge_sorted<'a, I>(existing: &mut Vec<String>, incoming: I)
where
    I: IntoIterator<Item = &'a String>,
{
    for name in incoming {
        existing.push(name.clone());
    }
    existing.sort();
    existing.dedup();
}

/// Atomically write `content` to `target` via a sibling tempfile +
/// fsync + rename. No mode/owner/group manipulation — the imported
/// file lives in the operator's config repo, not under `/etc`, so we
/// inherit the parent directory's defaults.
fn write_text_atomic(target: &Path, content: &[u8]) -> Result<(), ReconcileError> {
    let parent = target.parent().ok_or_else(|| ReconcileError::Io {
        path: target.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target has no parent directory",
        ),
    })?;

    let mut tmp = NamedTempFile::new_in(parent).map_err(|e| ReconcileError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    tmp.as_file_mut()
        .write_all(content)
        .map_err(|e| ReconcileError::Io {
            path: tmp.path().to_path_buf(),
            source: e,
        })?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| ReconcileError::Io {
            path: tmp.path().to_path_buf(),
            source: e,
        })?;

    let tmp_path = tmp.path().to_path_buf();
    tmp.persist(target).map_err(|e| ReconcileError::Io {
        path: tmp_path,
        source: e.error,
    })?;

    Ok(())
}

fn validate_hostname(raw: &str) -> Result<&str, ReconcileError> {
    if raw.is_empty() {
        return Err(ReconcileError::EmptyHostname);
    }
    if raw.contains('/') || raw.contains('\\') || raw.contains('\0') {
        return Err(ReconcileError::InvalidHostname {
            hostname: raw.to_owned(),
        });
    }
    Ok(raw)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests may use expect()/unwrap()/panic!() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;
    use crate::mock_probe::MockProbe;
    use crate::probe::SystemProbe;
    use pearlite_nickel::MockNickel;
    use pearlite_schema::{
        CargoInventory, HostInfo, KernelInfo, PacmanInventory, ProbedState, ServiceInventory,
    };
    use std::collections::BTreeSet;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    fn probed_with_hostname(hostname: &str) -> ProbedState {
        ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: hostname.to_owned(),
            },
            pacman: Some(PacmanInventory::default()),
            cargo: Some(CargoInventory::default()),
            config_files: None,
            services: Some(ServiceInventory::default()),
            kernel: KernelInfo {
                running_version: String::new(),
                package: "linux-cachyos".to_owned(),
                loaded_modules: BTreeSet::new(),
            },
        }
    }

    fn make_engine(probe: Box<dyn SystemProbe>) -> Engine {
        // reconcile() does not consult Nickel; an unseeded MockNickel
        // is fine since the path never reaches the evaluator.
        let nickel = MockNickel::new();
        Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"))
    }

    #[test]
    fn reconcile_writes_imported_ncl_at_expected_path() {
        let tmp = TempDir::new().expect("tempdir");
        let probe = Box::new(MockProbe::with_state(probed_with_hostname("forge")));
        let engine = make_engine(probe);

        let outcome = engine.reconcile(tmp.path()).expect("reconcile");

        let expected = tmp.path().join("hosts").join("forge.imported.ncl");
        assert_eq!(outcome.path, expected);
        assert_eq!(outcome.hostname, "forge");
        assert!(expected.is_file(), "imported.ncl was not created on disk");
    }

    #[test]
    fn reconcile_writes_emit_host_text_verbatim() {
        let tmp = TempDir::new().expect("tempdir");
        let probed = probed_with_hostname("forge");
        let probe = Box::new(MockProbe::with_state(probed.clone()));
        let engine = make_engine(probe);

        let outcome = engine.reconcile(tmp.path()).expect("reconcile");

        let on_disk = std::fs::read_to_string(&outcome.path).expect("read");
        let expected = pearlite_nickel::emit_host(&probed);
        assert_eq!(on_disk, expected);
    }

    #[test]
    fn reconcile_creates_hosts_dir_when_missing() {
        let tmp = TempDir::new().expect("tempdir");
        // No `hosts/` subdir exists yet.
        let probe = Box::new(MockProbe::with_state(probed_with_hostname("forge")));
        let engine = make_engine(probe);

        engine.reconcile(tmp.path()).expect("reconcile");

        assert!(tmp.path().join("hosts").is_dir());
    }

    #[test]
    fn reconcile_refuses_to_clobber_existing_file() {
        let tmp = TempDir::new().expect("tempdir");
        let hosts = tmp.path().join("hosts");
        std::fs::create_dir_all(&hosts).expect("mkdir");
        let target = hosts.join("forge.imported.ncl");
        std::fs::write(&target, "do not clobber").expect("seed");

        let probe = Box::new(MockProbe::with_state(probed_with_hostname("forge")));
        let engine = make_engine(probe);

        let err = engine.reconcile(tmp.path()).expect_err("must refuse");
        assert!(
            matches!(err, ReconcileError::AlreadyExists { .. }),
            "got {err:?}"
        );

        let preserved = std::fs::read_to_string(&target).expect("read");
        assert_eq!(preserved, "do not clobber", "existing file was modified");
    }

    #[test]
    fn reconcile_rejects_empty_hostname() {
        let tmp = TempDir::new().expect("tempdir");
        let probe = Box::new(MockProbe::with_state(probed_with_hostname("")));
        let engine = make_engine(probe);

        let err = engine.reconcile(tmp.path()).expect_err("must reject");
        assert!(matches!(err, ReconcileError::EmptyHostname), "got {err:?}");
    }

    #[test]
    fn reconcile_rejects_hostname_with_path_separator() {
        let tmp = TempDir::new().expect("tempdir");
        let probe = Box::new(MockProbe::with_state(probed_with_hostname("ev/il")));
        let engine = make_engine(probe);

        let err = engine.reconcile(tmp.path()).expect_err("must reject");
        assert!(
            matches!(err, ReconcileError::InvalidHostname { ref hostname } if hostname == "ev/il"),
            "got {err:?}"
        );
    }

    #[test]
    fn reconcile_propagates_probe_failure() {
        struct FailingProbe;
        impl SystemProbe for FailingProbe {
            fn probe(&self) -> Result<ProbedState, crate::errors::ProbeError> {
                Err(crate::errors::ProbeError::Pacman(
                    pearlite_pacman::PacmanError::NotInPath {
                        tool: "pacman",
                        hint: "test",
                    },
                ))
            }
        }

        let tmp = TempDir::new().expect("tempdir");
        let engine = make_engine(Box::new(FailingProbe));

        let err = engine.reconcile(tmp.path()).expect_err("must fail");
        assert!(matches!(err, ReconcileError::Probe(_)), "got {err:?}");
    }

    #[test]
    fn validate_hostname_accepts_normal_names() {
        assert_eq!(validate_hostname("forge").expect("ok"), "forge");
        assert_eq!(
            validate_hostname("anvil-01.local").expect("ok"),
            "anvil-01.local"
        );
    }

    // -------------------------------------------------------------------
    // reconcile_commit (ADR-0014, M4 W1)
    // -------------------------------------------------------------------

    use pearlite_state::{SCHEMA_VERSION, State, StateStore};
    use std::collections::BTreeMap;

    fn empty_state() -> State {
        State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed::default(),
            adopted: pearlite_state::Adopted::default(),
            history: Vec::new(),
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: BTreeMap::new(),
        }
    }

    /// Build a probed state with a given pacman explicit set + cargo
    /// install set. Cargo entries are `(name, version)` pairs.
    fn probed_with_inventory(
        hostname: &str,
        pacman_explicit: &[&str],
        cargo_installs: &[(&str, &str)],
    ) -> ProbedState {
        let mut state = probed_with_hostname(hostname);
        state.pacman = Some(PacmanInventory {
            explicit: pacman_explicit.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        });
        state.cargo = Some(CargoInventory {
            crates: cargo_installs
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
        });
        state
    }

    /// Seed a fresh `state.toml` at `<dir>/state.toml` with `state` and
    /// return its path.
    fn seed_state(dir: &TempDir, state: &State) -> PathBuf {
        let path = dir.path().join("state.toml");
        StateStore::new(path.clone())
            .write_atomic(state)
            .expect("seed write");
        path
    }

    #[test]
    fn reconcile_commit_adopt_all_moves_manual_into_state_adopted() {
        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());

        let probed = probed_with_inventory("forge", &["htop", "vim"], &[("ripgrep-all", "0.10.6")]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let outcome = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect("commit");

        assert_eq!(outcome.action, ReconciliationAction::AdoptAll);
        assert_eq!(outcome.considered, 3);
        assert_eq!(
            outcome.adopted,
            vec![
                "htop".to_owned(),
                "ripgrep-all".to_owned(),
                "vim".to_owned()
            ]
        );
        assert!(outcome.skipped.is_empty());

        let after = StateStore::new(state_path).read().expect("read");
        assert_eq!(
            after.adopted.pacman,
            vec!["htop".to_owned(), "vim".to_owned()]
        );
        assert_eq!(after.adopted.cargo, vec!["ripgrep-all".to_owned()]);
        assert_eq!(after.reconciliations.len(), 1);
        let entry = &after.reconciliations[0];
        assert_eq!(entry.action, ReconciliationAction::AdoptAll);
        assert_eq!(entry.package_count, 3);
        assert_eq!(entry.adopted, outcome.adopted);
        assert!(entry.skipped.is_empty());
        assert_eq!(after.last_modified, Some(outcome.committed_at));
        assert!(
            after.last_apply.is_none(),
            "reconcile must not bump last_apply"
        );
    }

    #[test]
    fn reconcile_commit_selective_partitions_into_adopted_and_skipped() {
        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());

        let probed =
            probed_with_inventory("forge", &["htop", "vim", "nano"], &[("zellij", "0.41.2")]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let mut adopt = BTreeSet::new();
        adopt.insert("htop".to_owned());
        adopt.insert("zellij".to_owned());

        let outcome = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::Selective { adopt }, None)
            .expect("commit");

        assert_eq!(outcome.action, ReconciliationAction::Interactive);
        assert_eq!(outcome.considered, 4);
        assert_eq!(
            outcome.adopted,
            vec!["htop".to_owned(), "zellij".to_owned()]
        );
        assert_eq!(outcome.skipped, vec!["nano".to_owned(), "vim".to_owned()]);

        let after = StateStore::new(state_path).read().expect("read");
        assert_eq!(after.adopted.pacman, vec!["htop".to_owned()]);
        assert_eq!(after.adopted.cargo, vec!["zellij".to_owned()]);
        assert_eq!(after.reconciliations.len(), 1);
        let entry = &after.reconciliations[0];
        assert_eq!(entry.action, ReconciliationAction::Interactive);
        assert_eq!(entry.adopted, outcome.adopted);
        assert_eq!(entry.skipped, outcome.skipped);
    }

    #[test]
    fn reconcile_commit_threshold_exceeded_refuses_without_writing() {
        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());
        let original = std::fs::read_to_string(&state_path).expect("read original");

        let probed = probed_with_inventory("forge", &["a", "b", "c", "d", "e", "f"], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let err = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, Some(5))
            .expect_err("must refuse");
        assert!(
            matches!(
                err,
                ReconcileCommitError::ThresholdExceeded {
                    count: 6,
                    threshold: 5,
                }
            ),
            "got {err:?}"
        );

        let after = std::fs::read_to_string(&state_path).expect("read after");
        assert_eq!(after, original, "state.toml must be untouched on refusal");
    }

    #[test]
    fn reconcile_commit_at_threshold_boundary_is_allowed() {
        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());

        // Exactly 5 Manual items, threshold = 5 → allowed.
        let probed = probed_with_inventory("forge", &["a", "b", "c", "d", "e"], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let outcome = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, Some(5))
            .expect("commit");
        assert_eq!(outcome.considered, 5);
        assert_eq!(outcome.adopted.len(), 5);
    }

    #[test]
    fn reconcile_commit_skips_already_managed_packages() {
        let tmp = TempDir::new().expect("tempdir");
        let mut state = empty_state();
        state.managed.pacman = vec!["htop".to_owned()];
        let state_path = seed_state(&tmp, &state);

        // htop is in state.managed → forgotten if installed but not declared,
        // not Manual. Only `vim` should be classified as Manual.
        let probed = probed_with_inventory("forge", &["htop", "vim"], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let outcome = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect("commit");
        assert_eq!(outcome.considered, 1);
        assert_eq!(outcome.adopted, vec!["vim".to_owned()]);

        let after = StateStore::new(state_path).read().expect("read");
        assert_eq!(after.adopted.pacman, vec!["vim".to_owned()]);
        assert_eq!(
            after.managed.pacman,
            vec!["htop".to_owned()],
            "managed unchanged"
        );
    }

    #[test]
    fn reconcile_commit_preserves_existing_adopted_when_unioning() {
        let tmp = TempDir::new().expect("tempdir");
        let mut state = empty_state();
        state.adopted.pacman = vec!["zellij".to_owned()];
        let state_path = seed_state(&tmp, &state);

        // zellij already adopted → not classified as Manual; vim is fresh.
        let probed = probed_with_inventory("forge", &["zellij", "vim"], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let outcome = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect("commit");
        assert_eq!(outcome.adopted, vec!["vim".to_owned()]);

        let after = StateStore::new(state_path).read().expect("read");
        assert_eq!(
            after.adopted.pacman,
            vec!["vim".to_owned(), "zellij".to_owned()],
            "existing adopted entry must survive and be merged in sort order"
        );
    }

    #[test]
    fn reconcile_commit_with_no_manual_drift_writes_empty_entry() {
        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());

        // No installed packages → nothing to adopt.
        let probed = probed_with_inventory("forge", &[], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let outcome = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect("commit");
        assert_eq!(outcome.considered, 0);
        assert!(outcome.adopted.is_empty());
        assert!(outcome.skipped.is_empty());

        let after = StateStore::new(state_path).read().expect("read");
        assert_eq!(
            after.reconciliations.len(),
            1,
            "an empty commit still records the [[reconciliations]] entry"
        );
    }

    #[test]
    fn reconcile_commit_propagates_probe_failure() {
        struct FailingProbe;
        impl SystemProbe for FailingProbe {
            fn probe(&self) -> Result<ProbedState, crate::errors::ProbeError> {
                Err(crate::errors::ProbeError::Pacman(
                    pearlite_pacman::PacmanError::NotInPath {
                        tool: "pacman",
                        hint: "test",
                    },
                ))
            }
        }

        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());
        let engine = make_engine(Box::new(FailingProbe));

        let err = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect_err("must fail");
        assert!(matches!(err, ReconcileCommitError::Probe(_)), "got {err:?}");
    }

    #[test]
    fn reconcile_commit_propagates_state_read_failure_when_missing() {
        let tmp = TempDir::new().expect("tempdir");
        // Don't seed state.toml — the engine should surface NotFound.
        let state_path = tmp.path().join("state.toml");
        let probed = probed_with_inventory("forge", &[], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed)));

        let err = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect_err("must fail");
        assert!(matches!(err, ReconcileCommitError::State(_)), "got {err:?}");
    }

    #[test]
    fn reconcile_commit_each_invocation_gets_fresh_plan_id() {
        let tmp = TempDir::new().expect("tempdir");
        let state_path = seed_state(&tmp, &empty_state());

        let probed = probed_with_inventory("forge", &["htop"], &[]);
        let engine = make_engine(Box::new(MockProbe::with_state(probed.clone())));
        let first = engine
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect("first");

        let engine2 = make_engine(Box::new(MockProbe::with_state(probed)));
        let second = engine2
            .reconcile_commit(&state_path, &ReconcileDecisions::AdoptAll, None)
            .expect("second");

        assert_ne!(first.plan_id, second.plan_id);
        let after = StateStore::new(state_path).read().expect("read");
        assert_eq!(after.reconciliations.len(), 2);
    }
}
