// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Engine::reconcile`] — read-only flow that probes the live system
//! and writes `<config_dir>/hosts/<hostname>.imported.ncl` for operator
//! review (PRD §11, Plan §7.5 M4 W1).
//!
//! This is the *fresh-import* half of reconcile. The interactive
//! `reconcile_commit` half (state.toml writes, drift-threshold safety,
//! adoption prompts) lands in a follow-up PR.
//!
//! The flow:
//!
//! 1. Probe the live system via [`SystemProbe::probe`](crate::probe::SystemProbe::probe).
//! 2. Render Nickel host text via
//!    [`pearlite_nickel::emit_host`](pearlite_nickel::emit_host).
//! 3. Atomically write to
//!    `<config_dir>/hosts/<hostname>.imported.ncl`.
//!
//! No state.toml mutation, no schema validation of the emitted text —
//! the file is a *review draft*, not a parsed declaration. Validation
//! happens on the next `pearlite plan` once the operator has hand-
//! curated and renamed it to `hosts/<hostname>.ncl`.

use crate::Engine;
use crate::errors::ReconcileError;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// Result of a successful [`Engine::reconcile`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Absolute path of the written `.imported.ncl` file.
    pub path: PathBuf,
    /// Hostname the file was rendered for.
    pub hostname: String,
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
}
