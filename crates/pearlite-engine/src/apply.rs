// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Apply-phase orchestration skeleton (PRD §8.2).
//!
//! [`Engine::apply_plan`] takes a planned [`Plan`] and the four apply
//! adapters (pacman, cargo, systemd, snapper) and walks the
//! seven-phase pipeline:
//!
//! 1. Snapshot pre.
//! 2. Repo prep — `pacman -Sy` (run only if the plan has any pacman
//!    or AUR `install` actions).
//! 3. Phases 2..6: actions partitioned by [`ApplyPhase`] and sorted
//!    within each phase by [`Action::within_phase_key`].
//! 4. Snapshot post.
//!
//! Scope:
//!
//! - **In**: snapshot pre/post, db sync, all pacman / cargo / service
//!   actions, `ConfigWrite` (read source, verify SHA-256, atomic write
//!   via `pearlite_fs::write_etc_atomic`), `state.toml` history-entry
//!   commit (PRD §8.2 phase 9 — last write), failure path (post-fail
//!   snapshot, `FailureRecord` JSON write, `[[failures]]` append),
//!   deterministic phase + within-phase ordering.
//! - **Out** (follow-up PRs): `Engine::rollback`.

use crate::errors::ApplyError;
use crate::plan::Engine;
use pearlite_cargo::Cargo;
use pearlite_diff::{Action, ApplyPhase, FailureCoherence, Plan, Scope};
use pearlite_pacman::Pacman;
use pearlite_snapper::{Snapper, SnapshotInfo};
use pearlite_state::{FailureRef, HistoryEntry, SnapshotRef, StateStore};
use pearlite_systemd::{Scope as SystemdScope, Systemd};
use pearlite_userenv::HomeManagerBackend;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;
use time::OffsetDateTime;
use uuid::Uuid;

/// Forensic record written to `<failures_dir>/<plan-id>.json` when an
/// apply halts mid-pipeline. The full record is what the user reads
/// during a post-mortem; [`FailureRef`] in `state.toml` is just the
/// pointer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Plan UUID this failure pertains to.
    pub plan_id: Uuid,
    /// UTC timestamp the failure was detected.
    #[serde(with = "time::serde::iso8601")]
    pub failed_at: OffsetDateTime,
    /// PRD §8.5 failure class: 3 recoverable, 4 incoherent.
    pub class: u8,
    /// Process exit code corresponding to the class (4 or 5).
    pub exit_code: u8,
    /// Action that errored.
    pub failed_action: Action,
    /// Index of the failed action in the plan's `actions` vec
    /// (post-stable-sort by `within_phase_key` within each phase).
    pub failed_action_executed_index: usize,
    /// `Display` form of the underlying [`ApplyError`].
    pub error_message: String,
    /// Post-failure forensic snapshot, if Snapper accepted it.
    /// `None` when the post-fail snapshot itself failed (Snapper
    /// disk full, btrfs read-only, etc.).
    pub post_fail_snapshot: Option<SnapshotRef>,
    /// Pre-apply snapshot (mirror of the entry the user would
    /// `pearlite rollback` to).
    pub snapshot_pre: SnapshotRef,
}

/// Outcome of a successful [`Engine::apply_plan`] run.
#[derive(Clone, Debug)]
pub struct ApplyOutcome {
    /// The plan ID this apply executed.
    pub plan_id: Uuid,
    /// Pre-apply snapshot returned by snapper.
    pub snapshot_pre: SnapshotInfo,
    /// Post-apply snapshot returned by snapper.
    pub snapshot_post: SnapshotInfo,
    /// Number of plan actions actually executed (excludes
    /// `SnapshotCreate` history records, which are not orchestrated).
    pub actions_executed: usize,
    /// Generation number assigned to the new history entry (max of
    /// previous generations + 1, or 1 for the first apply).
    pub generation: u64,
    /// Wall-clock duration of the apply, in milliseconds.
    pub duration_ms: u64,
}

impl Engine {
    /// Execute a [`Plan`] against the live system.
    ///
    /// Wraps the apply in pre/post Snapper snapshots, dispatches every
    /// `Action` to the appropriate adapter in PRD §8.2 phase order,
    /// then commits a [`HistoryEntry`] to `state.toml` (phase 9 — the
    /// last write per CLAUDE.md hard invariant 8). Halts on the first
    /// error.
    ///
    /// `snapper_config` is the snapper config name to take snapshots
    /// against (typically `"root"`). `state_path` is the absolute path
    /// to `state.toml`; it must already exist (initial creation is
    /// `pearlite init`). `failures_dir` is the directory where failure
    /// JSON records land — created if missing.
    ///
    /// # Errors
    /// Returns [`ApplyError`] wrapping the failing adapter's error.
    /// The CLI boundary maps this to a failure class via
    /// [`Action::failure_coherence`] on the failed action.
    ///
    /// On mid-pipeline failure (an `Action` errored), the engine takes
    /// a best-effort post-fail Snapper snapshot, writes a
    /// [`FailureRecord`] JSON to `<failures_dir>/<plan-id>.json`, and
    /// appends a [`FailureRef`] to `state.toml`. These side effects
    /// are best-effort: a record-writing failure does not mask the
    /// underlying [`ApplyError`].
    #[allow(
        clippy::too_many_arguments,
        reason = "apply_plan is the orchestrator entry point; passing all five \
                  trait-object adapters + plan + state path + failures dir is the \
                  natural surface, and a builder hides too much of what's load-bearing"
    )]
    pub fn apply_plan(
        &self,
        plan: &Plan,
        pacman: &dyn Pacman,
        cargo: &dyn Cargo,
        systemd: &dyn Systemd,
        snapper: &dyn Snapper,
        home_manager: &dyn HomeManagerBackend,
        snapper_config: &str,
        state_path: &Path,
        failures_dir: &Path,
    ) -> Result<ApplyOutcome, ApplyError> {
        let started = Instant::now();

        let snapshot_pre = snapper.create(snapper_config, &pre_label(plan.plan_id))?;

        let buckets = partition_by_phase(&plan.actions);

        if needs_db_sync(plan) {
            pacman.sync_databases()?;
        }

        let mut actions_executed = 0usize;
        let mut failure: Option<(usize, Action, ApplyError)> = None;
        'phases: for phase in [
            ApplyPhase::Removals,
            ApplyPhase::Installs,
            ApplyPhase::ConfigWrites,
            ApplyPhase::ServiceState,
            ApplyPhase::ServiceRestarts,
            ApplyPhase::UserEnv,
        ] {
            if let Some(actions) = buckets.get(&phase) {
                for action in actions {
                    if let Err(e) = exec_action(
                        action,
                        pacman,
                        cargo,
                        systemd,
                        home_manager,
                        self.repo_root(),
                    ) {
                        failure = Some((actions_executed, (*action).clone(), e));
                        break 'phases;
                    }
                    actions_executed += 1;
                }
            }
        }

        if let Some((idx, failed_action, err)) = failure {
            record_apply_failure(
                plan,
                idx,
                failed_action,
                &err,
                snapper,
                snapper_config,
                &snapshot_pre,
                state_path,
                failures_dir,
            );
            return Err(err);
        }

        let snapshot_post = snapper.create(snapper_config, &post_label(plan.plan_id))?;

        // Phase 9 — state.toml commit. PRD §8.2 invariant 8: this is
        // the last file written on apply.
        let store = StateStore::new(state_path.to_path_buf());
        let mut state = store.read()?;
        let generation = next_generation(&state);
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let summary = summarize(&plan.actions);
        let entry = HistoryEntry {
            plan_id: plan.plan_id,
            generation,
            applied_at: OffsetDateTime::now_utc(),
            duration_ms,
            snapshot_pre: to_snapshot_ref(&snapshot_pre),
            snapshot_post: to_snapshot_ref(&snapshot_post),
            actions_executed: u32::try_from(actions_executed).unwrap_or(u32::MAX),
            git_revision: None,
            git_dirty: false,
            summary,
        };
        state.history.push(entry);
        state.last_apply = Some(OffsetDateTime::now_utc());
        state.last_modified = state.last_apply;
        store.write_atomic(&state)?;

        Ok(ApplyOutcome {
            plan_id: plan.plan_id,
            snapshot_pre,
            snapshot_post,
            actions_executed,
            generation,
            duration_ms,
        })
    }
}

/// Forensic-record bookkeeping for a mid-pipeline apply failure.
///
/// Steps, each best-effort and independent so a later failure
/// doesn't mask the underlying error:
///
/// 1. Take a post-fail Snapper snapshot.
/// 2. Build a [`FailureRecord`] from what we know.
/// 3. Serialize it to `<failures_dir>/<plan-id>.json`.
/// 4. Append a [`FailureRef`] to `state.toml`.
///
/// Errors are intentionally swallowed: the apply has already failed,
/// and a failed-record-write (or stale state.toml) is a secondary
/// concern. The caller still sees the original [`ApplyError`].
#[allow(
    clippy::too_many_arguments,
    reason = "record_apply_failure mirrors apply_plan's parameter set; \
              the fields are all load-bearing for the forensic record"
)]
fn record_apply_failure(
    plan: &Plan,
    executed_index: usize,
    failed_action: Action,
    err: &ApplyError,
    snapper: &dyn Snapper,
    snapper_config: &str,
    snapshot_pre: &SnapshotInfo,
    state_path: &Path,
    failures_dir: &Path,
) {
    let class = match failed_action.failure_coherence() {
        FailureCoherence::Recoverable => 3u8,
        FailureCoherence::Incoherent => 4u8,
    };
    let exit_code = if class == 3 { 4u8 } else { 5u8 };

    let post_fail_snapshot = snapper
        .create(snapper_config, &post_fail_label(plan.plan_id))
        .ok();

    let record = FailureRecord {
        plan_id: plan.plan_id,
        failed_at: OffsetDateTime::now_utc(),
        class,
        exit_code,
        failed_action,
        failed_action_executed_index: executed_index,
        error_message: err.to_string(),
        post_fail_snapshot: post_fail_snapshot.as_ref().map(to_snapshot_ref),
        snapshot_pre: to_snapshot_ref(snapshot_pre),
    };

    let record_path = failures_dir.join(format!("{}.json", plan.plan_id.simple()));
    let _ = std::fs::create_dir_all(failures_dir);
    if let Ok(json) = serde_json::to_vec_pretty(&record) {
        let _ = std::fs::write(&record_path, &json);
    }

    let failure_ref = FailureRef {
        plan_id: plan.plan_id,
        failed_at: record.failed_at,
        class,
        exit_code,
        record_path,
    };
    let store = StateStore::new(state_path.to_path_buf());
    let _ = store.append_failure(failure_ref);
}

fn post_fail_label(plan_id: Uuid) -> String {
    format!("post-fail-pearlite-apply-{}", short_id(plan_id))
}

fn next_generation(state: &pearlite_state::State) -> u64 {
    state
        .history
        .iter()
        .map(|h| h.generation)
        .max()
        .map_or(1, |m| m + 1)
}

fn to_snapshot_ref(s: &SnapshotInfo) -> SnapshotRef {
    SnapshotRef {
        id: s.id,
        label: s.label.clone(),
        created_at: s.created_at,
    }
}

fn summarize(actions: &[Action]) -> String {
    let mut installs = 0u32;
    let mut removals = 0u32;
    let mut config_writes = 0u32;
    let mut service_state = 0u32;
    let mut restarts = 0u32;
    let mut user_envs = 0u32;
    for a in actions {
        match a {
            Action::PacmanInstall { packages, .. } | Action::AurInstall { packages } => {
                installs += u32::try_from(packages.len()).unwrap_or(u32::MAX);
            }
            Action::CargoInstall { .. } => installs += 1,
            Action::PacmanRemove { packages } => {
                removals += u32::try_from(packages.len()).unwrap_or(u32::MAX);
            }
            Action::CargoUninstall { .. } => removals += 1,
            Action::ConfigWrite { .. } => config_writes += 1,
            Action::ServiceMask { .. }
            | Action::ServiceDisable { .. }
            | Action::ServiceEnable { .. } => service_state += 1,
            Action::ServiceRestart { .. } => restarts += 1,
            Action::UserEnvSwitch { .. } => user_envs += 1,
            Action::SnapshotCreate { .. } => {}
        }
    }
    format!(
        "+{installs} -{removals} ~{config_writes} ({installs} installs, {removals} removals, \
         {config_writes} config updates, {service_state} service-state changes, {restarts} restarts, \
         {user_envs} user-env switches)"
    )
}

fn partition_by_phase(actions: &[Action]) -> BTreeMap<ApplyPhase, Vec<&Action>> {
    let mut buckets: BTreeMap<ApplyPhase, Vec<&Action>> = BTreeMap::new();
    for action in actions {
        buckets.entry(action.phase()).or_default().push(action);
    }
    for bucket in buckets.values_mut() {
        bucket.sort_by_key(|a| a.within_phase_key());
    }
    buckets
}

fn needs_db_sync(plan: &Plan) -> bool {
    plan.actions
        .iter()
        .any(|a| matches!(a, Action::PacmanInstall { .. } | Action::AurInstall { .. }))
}

fn exec_action(
    action: &Action,
    pacman: &dyn Pacman,
    cargo: &dyn Cargo,
    systemd: &dyn Systemd,
    home_manager: &dyn HomeManagerBackend,
    repo_root: &Path,
) -> Result<(), ApplyError> {
    match action {
        Action::PacmanInstall { repo, packages } => {
            let pkgs: Vec<&str> = packages.iter().map(String::as_str).collect();
            pacman.install(repo, &pkgs)?;
        }
        Action::PacmanRemove { packages } => {
            let pkgs: Vec<&str> = packages.iter().map(String::as_str).collect();
            pacman.remove(&pkgs)?;
        }
        Action::AurInstall { packages } => {
            let pkgs: Vec<&str> = packages.iter().map(String::as_str).collect();
            pacman.aur_install(&pkgs)?;
        }
        Action::CargoInstall { crate_name, locked } => {
            cargo.install(crate_name, *locked)?;
        }
        Action::CargoUninstall { crate_name } => {
            cargo.uninstall(crate_name)?;
        }
        Action::ConfigWrite {
            target,
            source,
            content_sha256,
            mode,
            owner,
            group,
            ..
        } => {
            exec_config_write(
                target,
                source,
                content_sha256,
                *mode,
                owner,
                group,
                repo_root,
            )?;
        }
        Action::ServiceMask { unit } => {
            systemd.mask(unit)?;
        }
        Action::ServiceDisable { unit, scope } => {
            systemd.disable(unit, &to_systemd_scope(scope))?;
        }
        Action::ServiceEnable { unit, scope } => {
            systemd.enable(unit, &to_systemd_scope(scope))?;
        }
        Action::ServiceRestart { unit } => {
            systemd.restart(unit)?;
        }
        Action::UserEnvSwitch {
            user,
            config_path,
            mode,
            channel,
        } => {
            home_manager.switch(user, config_path, *mode, channel)?;
        }
        // `SnapshotCreate` actions in plans are state-history records,
        // not orchestration steps. The pre/post snapshot bookkeeping
        // above handles every snapshot the apply engine actually takes.
        Action::SnapshotCreate { .. } => {}
    }
    Ok(())
}

fn exec_config_write(
    target: &Path,
    source: &Path,
    expected_sha256: &str,
    mode: u32,
    owner: &str,
    group: &str,
    repo_root: &Path,
) -> Result<(), ApplyError> {
    let resolved = repo_root.join(source);
    let content = std::fs::read(&resolved).map_err(|e| {
        ApplyError::Fs(pearlite_fs::FsError::Io {
            path: resolved.clone(),
            source: e,
        })
    })?;
    let actual_digest = pearlite_fs::sha256_bytes(&content);
    let actual_hex = hex_encode(&actual_digest);
    if actual_hex != expected_sha256 {
        return Err(ApplyError::ContentSha256Mismatch {
            target: target.to_path_buf(),
            planned: expected_sha256.to_owned(),
            actual: actual_hex,
        });
    }
    pearlite_fs::write_etc_atomic(target, &content, mode, owner, group)?;
    Ok(())
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push(hex_nibble(b >> 4));
        s.push(hex_nibble(b & 0x0f));
    }
    s
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => char::from(b'0' + n),
        10..=15 => char::from(b'a' + n - 10),
        _ => unreachable!("nibble fits in 4 bits"),
    }
}

fn to_systemd_scope(s: &Scope) -> SystemdScope {
    match s {
        Scope::System => SystemdScope::System,
        Scope::User { name } => SystemdScope::User { name: name.clone() },
    }
}

fn pre_label(plan_id: Uuid) -> String {
    format!("pre-pearlite-apply-{}", short_id(plan_id))
}

fn post_label(plan_id: Uuid) -> String {
    format!("post-pearlite-apply-{}", short_id(plan_id))
}

fn short_id(plan_id: Uuid) -> String {
    let hex = plan_id.simple().to_string();
    hex.chars().take(8).collect()
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
    use pearlite_cargo::MockCargo;
    use pearlite_diff::{Phase, Plan as DiffPlan};
    use pearlite_pacman::MockPacman;
    use pearlite_snapper::MockSnapper;
    use pearlite_state::SCHEMA_VERSION;
    use pearlite_systemd::MockSystemd;
    use pearlite_userenv::MockHmBackend;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    fn engine() -> Engine {
        use pearlite_nickel::MockNickel;
        use pearlite_schema::{HostInfo, KernelInfo, ProbedState};

        let probed = ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: None,
            cargo: None,
            config_files: None,
            services: None,
            kernel: KernelInfo::default(),
        };
        Engine::new(
            Box::new(MockNickel::new()),
            Box::new(crate::mock_probe::MockProbe::with_state(probed)),
            PathBuf::from("/cfg-repo"),
        )
    }

    fn plan_with_actions(actions: Vec<Action>) -> DiffPlan {
        DiffPlan {
            plan_id: Uuid::nil(),
            host: "forge".to_owned(),
            generated_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            actions,
            drift: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Create a `state.toml` in `dir` pre-populated with a minimal,
    /// schema-valid baseline, plus an adjacent `failures/` directory.
    /// Returns `(state_path, failures_dir)`.
    fn setup_state(dir: &TempDir) -> (PathBuf, PathBuf) {
        let path = dir.path().join("state.toml");
        let failures = dir.path().join("failures");
        let store = StateStore::new(path.clone());
        let state = pearlite_state::State {
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
        };
        store.write_atomic(&state).expect("write base state");
        (path, failures)
    }

    #[test]
    fn empty_plan_takes_pre_and_post_snapshots() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let out = engine()
            .apply_plan(
                &plan_with_actions(vec![]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");

        assert_eq!(out.actions_executed, 0);
        assert_eq!(out.snapshot_pre.id, 1);
        assert_eq!(out.snapshot_post.id, 2);
        assert!(out.snapshot_pre.label.starts_with("pre-pearlite-apply-"));
        assert!(out.snapshot_post.label.starts_with("post-pearlite-apply-"));
        assert_eq!(pacman.sync_count(), 0, "no installs → no db sync");
        assert_eq!(out.generation, 1, "first apply gets generation 1");

        // Verify state.toml grew a history entry.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert_eq!(read_back.history.len(), 1);
        assert_eq!(read_back.history[0].generation, 1);
        assert_eq!(read_back.history[0].actions_executed, 0);
        assert_eq!(read_back.history[0].snapshot_pre.id, 1);
        assert_eq!(read_back.history[0].snapshot_post.id, 2);
        assert!(read_back.last_apply.is_some());
    }

    #[test]
    fn sync_databases_runs_only_when_pacman_or_aur_install_present() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![Action::CargoInstall {
                    crate_name: "zellij".to_owned(),
                    locked: true,
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(pacman.sync_count(), 0, "cargo-only plan must not sync");

        engine()
            .apply_plan(
                &plan_with_actions(vec![Action::PacmanInstall {
                    repo: "core".to_owned(),
                    packages: vec!["base".to_owned()],
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(pacman.sync_count(), 1, "pacman install triggers sync");

        // Two applies → two history entries with generations 1 and 2.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert_eq!(read_back.history.len(), 2);
        assert_eq!(read_back.history[0].generation, 1);
        assert_eq!(read_back.history[1].generation, 2);
    }

    #[test]
    fn pacman_install_dispatches_with_repo_and_packages() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![Action::PacmanInstall {
                    repo: "extra".to_owned(),
                    packages: vec!["htop".to_owned(), "ripgrep".to_owned()],
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(
            pacman.install_history(),
            vec![(
                "extra".to_owned(),
                vec!["htop".to_owned(), "ripgrep".to_owned()],
            )]
        );
    }

    #[test]
    fn aur_install_dispatches_via_aur_install() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![Action::AurInstall {
                    packages: vec!["yay".to_owned()],
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(pacman.aur_install_history(), vec![vec!["yay".to_owned()]]);
        assert!(pacman.install_history().is_empty());
    }

    #[test]
    fn cargo_install_and_uninstall_dispatch_correctly() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![
                    Action::CargoUninstall {
                        crate_name: "old-crate".to_owned(),
                    },
                    Action::CargoInstall {
                        crate_name: "zellij".to_owned(),
                        locked: true,
                    },
                ]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(cargo.uninstall_history(), vec!["old-crate".to_owned()]);
        assert_eq!(cargo.install_history(), vec![("zellij".to_owned(), true)]);
    }

    #[test]
    fn service_actions_dispatch_to_corresponding_methods() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![
                    Action::ServiceEnable {
                        unit: "sshd.service".to_owned(),
                        scope: Scope::System,
                    },
                    Action::ServiceRestart {
                        unit: "sshd.service".to_owned(),
                    },
                    Action::ServiceMask {
                        unit: "telnet.service".to_owned(),
                    },
                ]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(
            systemd.enable_history(),
            vec![("sshd.service".to_owned(), SystemdScope::System)]
        );
        assert_eq!(systemd.mask_history(), vec!["telnet.service".to_owned()]);
        assert_eq!(systemd.restart_history(), vec!["sshd.service".to_owned()]);
    }

    #[test]
    fn user_scope_round_trips_through_to_systemd_scope() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![Action::ServiceEnable {
                    unit: "syncthing.service".to_owned(),
                    scope: Scope::User {
                        name: "alice".to_owned(),
                    },
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(
            systemd.enable_history(),
            vec![(
                "syncthing.service".to_owned(),
                SystemdScope::User {
                    name: "alice".to_owned()
                }
            )]
        );
    }

    #[test]
    fn user_env_switch_dispatches_through_home_manager() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let out = engine()
            .apply_plan(
                &plan_with_actions(vec![
                    Action::UserEnvSwitch {
                        user: "alice".to_owned(),
                        config_path: PathBuf::from("/repo/users/alice"),
                        mode: pearlite_schema::HomeManagerMode::Standalone,
                        channel: "release-24.11".to_owned(),
                    },
                    Action::UserEnvSwitch {
                        user: "bob".to_owned(),
                        config_path: PathBuf::from("/repo/users/bob"),
                        mode: pearlite_schema::HomeManagerMode::Flake,
                        channel: "default".to_owned(),
                    },
                ]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");

        assert_eq!(out.actions_executed, 2);
        let hist = home_manager.switch_history();
        assert_eq!(hist.len(), 2);
        // within_phase_key sorts alphabetically by user — alice runs
        // before bob even though declaration order matched.
        assert_eq!(hist[0].user, "alice");
        assert_eq!(hist[0].mode, pearlite_schema::HomeManagerMode::Standalone);
        assert_eq!(hist[1].user, "bob");
        assert_eq!(hist[1].mode, pearlite_schema::HomeManagerMode::Flake);
    }

    #[test]
    fn user_env_switch_failure_propagates_as_apply_error() {
        use pearlite_userenv::{HomeManagerBackend as HmTrait, UserEnvOutcome, UserenvError};
        struct FailingHm;
        impl HmTrait for FailingHm {
            fn switch(
                &self,
                _: &str,
                _: &Path,
                _: pearlite_schema::HomeManagerMode,
                _: &str,
            ) -> Result<UserEnvOutcome, UserenvError> {
                Err(UserenvError::NotInPath { hint: "test" })
            }
        }

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let err = engine()
            .apply_plan(
                &plan_with_actions(vec![Action::UserEnvSwitch {
                    user: "alice".to_owned(),
                    config_path: PathBuf::from("/repo/users/alice"),
                    mode: pearlite_schema::HomeManagerMode::Standalone,
                    channel: "release-24.11".to_owned(),
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &FailingHm,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");
        assert!(matches!(err, ApplyError::Userenv(_)), "got {err:?}");

        // FailureRef recorded with Class 3 / exit 4 (UserEnvSwitch is
        // Recoverable per ADR-0011 + diff::coherence).
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert_eq!(read_back.failures.len(), 1);
        assert_eq!(read_back.failures[0].class, 3);
        assert_eq!(read_back.failures[0].exit_code, 4);
    }

    #[test]
    fn phase_order_removals_before_installs_before_services() {
        // Mix actions across phases in declaration order; the engine
        // must execute them in PRD §8.2 phase order.
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![
                    Action::ServiceEnable {
                        unit: "x.service".to_owned(),
                        scope: Scope::System,
                    },
                    Action::PacmanInstall {
                        repo: "core".to_owned(),
                        packages: vec!["base".to_owned()],
                    },
                    Action::PacmanRemove {
                        packages: vec!["xterm".to_owned()],
                    },
                ]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");

        assert_eq!(pacman.sync_count(), 1);
        assert_eq!(pacman.remove_history().len(), 1);
        assert_eq!(pacman.install_history().len(), 1);
        assert_eq!(systemd.enable_history().len(), 1);
    }

    #[test]
    fn install_subphase_order_repo_before_aur_before_cargo() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![
                    Action::CargoInstall {
                        crate_name: "zellij".to_owned(),
                        locked: true,
                    },
                    Action::AurInstall {
                        packages: vec!["yay".to_owned()],
                    },
                    Action::PacmanInstall {
                        repo: "core".to_owned(),
                        packages: vec!["base".to_owned()],
                    },
                ]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");

        assert_eq!(pacman.install_history().len(), 1);
        assert_eq!(pacman.aur_install_history().len(), 1);
        assert_eq!(cargo.install_history().len(), 1);
    }

    #[test]
    fn snapshot_create_actions_in_plan_are_skipped() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let out = engine()
            .apply_plan(
                &plan_with_actions(vec![Action::SnapshotCreate {
                    label: "pre".to_owned(),
                    phase: Phase::Pre,
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");
        assert_eq!(out.snapshot_pre.id, 1);
        assert_eq!(out.snapshot_post.id, 2);
    }

    fn current_user_group() -> (String, String) {
        use nix::unistd::{getgid, getuid};
        let user = pearlite_fs::name_for_uid(getuid().as_raw());
        let group = pearlite_fs::name_for_gid(getgid().as_raw());
        (user, group)
    }

    fn engine_with_repo_root(repo_root: PathBuf) -> Engine {
        use pearlite_nickel::MockNickel;
        use pearlite_schema::{HostInfo, KernelInfo, ProbedState};
        let probed = ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: None,
            cargo: None,
            config_files: None,
            services: None,
            kernel: KernelInfo::default(),
        };
        Engine::new(
            Box::new(MockNickel::new()),
            Box::new(crate::mock_probe::MockProbe::with_state(probed)),
            repo_root,
        )
    }

    #[test]
    fn config_write_writes_target_atomically() {
        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let source_rel = PathBuf::from("etc/hello.conf");
        let source_abs = repo.path().join(&source_rel);
        std::fs::create_dir_all(source_abs.parent().expect("parent")).expect("mkdir");
        let content = b"key = value\n";
        std::fs::write(&source_abs, content).expect("write source");
        let expected_sha = hex_encode(&pearlite_fs::sha256_bytes(content));

        let target = target_dir.path().join("hello.conf");
        let (user, group) = current_user_group();

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();

        engine_with_repo_root(repo.path().to_path_buf())
            .apply_plan(
                &plan_with_actions(vec![Action::ConfigWrite {
                    target: target.clone(),
                    source: source_rel,
                    content_sha256: expected_sha,
                    mode: 0o644,
                    owner: user,
                    group,
                    declaration_index: 0,
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");

        assert_eq!(std::fs::read(&target).expect("read"), content);
    }

    #[test]
    fn config_write_sha_mismatch_aborts_apply() {
        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let source_rel = PathBuf::from("etc/x.conf");
        let source_abs = repo.path().join(&source_rel);
        std::fs::create_dir_all(source_abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(&source_abs, b"actual content\n").expect("write source");

        let target = target_dir.path().join("x.conf");
        let (user, group) = current_user_group();

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();

        let err = engine_with_repo_root(repo.path().to_path_buf())
            .apply_plan(
                &plan_with_actions(vec![Action::ConfigWrite {
                    target: target.clone(),
                    source: source_rel,
                    // Deliberately wrong SHA-256.
                    content_sha256: "0".repeat(64),
                    mode: 0o644,
                    owner: user,
                    group,
                    declaration_index: 0,
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");
        assert!(
            matches!(err, ApplyError::ContentSha256Mismatch { .. }),
            "got {err:?}"
        );
        assert!(!target.exists(), "target must not be created on mismatch");

        // Failure path: state.toml history must NOT have grown.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert!(
            read_back.history.is_empty(),
            "failed apply must not append history"
        );
    }

    #[test]
    fn config_write_missing_source_yields_fs_error() {
        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let target = target_dir.path().join("missing.conf");
        let (user, group) = current_user_group();

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();

        let err = engine_with_repo_root(repo.path().to_path_buf())
            .apply_plan(
                &plan_with_actions(vec![Action::ConfigWrite {
                    target,
                    source: PathBuf::from("etc/does-not-exist.conf"),
                    content_sha256: "0".repeat(64),
                    mode: 0o644,
                    owner: user,
                    group,
                    declaration_index: 0,
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");
        assert!(matches!(err, ApplyError::Fs(_)), "got {err:?}");
    }

    #[test]
    fn snapper_failure_on_pre_propagates() {
        struct FailingSnapper;
        impl Snapper for FailingSnapper {
            fn create(
                &self,
                _config: &str,
                _label: &str,
            ) -> Result<SnapshotInfo, pearlite_snapper::SnapperError> {
                Err(pearlite_snapper::SnapperError::NotInPath { hint: "test" })
            }
            fn rollback(
                &self,
                _config: &str,
                _id: u64,
            ) -> Result<(), pearlite_snapper::SnapperError> {
                Ok(())
            }
            fn list(
                &self,
                _config: &str,
            ) -> Result<Vec<SnapshotInfo>, pearlite_snapper::SnapperError> {
                Ok(Vec::new())
            }
        }

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let err = engine()
            .apply_plan(
                &plan_with_actions(vec![]),
                &pacman,
                &cargo,
                &systemd,
                &FailingSnapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");
        assert!(matches!(err, ApplyError::Snapper(_)), "got {err:?}");
    }

    #[test]
    fn missing_state_toml_yields_state_error() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        // Don't write a base state — apply_plan must error rather than
        // silently creating one.
        let state_path = state_dir.path().join("state.toml");
        let failures_dir = state_dir.path().join("failures");

        let err = engine()
            .apply_plan(
                &plan_with_actions(vec![]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");
        assert!(matches!(err, ApplyError::State(_)), "got {err:?}");
    }

    #[test]
    fn summary_counts_actions_by_kind() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        engine()
            .apply_plan(
                &plan_with_actions(vec![
                    Action::PacmanInstall {
                        repo: "core".to_owned(),
                        packages: vec!["a".to_owned(), "b".to_owned()],
                    },
                    Action::PacmanRemove {
                        packages: vec!["c".to_owned()],
                    },
                    Action::ServiceEnable {
                        unit: "x.service".to_owned(),
                        scope: Scope::System,
                    },
                ]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect("apply");

        let read_back = StateStore::new(state_path).read().expect("read state");
        let summary = &read_back.history[0].summary;
        assert!(
            summary.contains("+2"),
            "summary must show 2 installs: {summary}"
        );
        assert!(
            summary.contains("-1"),
            "summary must show 1 removal: {summary}"
        );
        assert!(
            summary.contains("1 service-state"),
            "summary must show service-state count: {summary}"
        );
    }

    #[test]
    fn config_write_failure_records_class_3_failure_ref() {
        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let source_rel = PathBuf::from("etc/x.conf");
        let source_abs = repo.path().join(&source_rel);
        std::fs::create_dir_all(source_abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(&source_abs, b"actual content\n").expect("write source");

        let target = target_dir.path().join("x.conf");
        let (user, group) = current_user_group();

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();

        let _ = engine_with_repo_root(repo.path().to_path_buf())
            .apply_plan(
                &plan_with_actions(vec![Action::ConfigWrite {
                    target,
                    source: source_rel,
                    content_sha256: "0".repeat(64),
                    mode: 0o644,
                    owner: user,
                    group,
                    declaration_index: 0,
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");

        // FailureRef appended with class 3 (Recoverable) / exit 4.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert_eq!(read_back.failures.len(), 1);
        assert_eq!(read_back.failures[0].class, 3);
        assert_eq!(read_back.failures[0].exit_code, 4);

        // Forensic JSON record was written.
        let record_path = &read_back.failures[0].record_path;
        assert!(record_path.exists(), "failure JSON must be written");
        let raw = std::fs::read(record_path).expect("read record");
        let record: FailureRecord = serde_json::from_slice(&raw).expect("parse record");
        assert_eq!(record.class, 3);
        assert_eq!(record.exit_code, 4);
        assert!(matches!(record.failed_action, Action::ConfigWrite { .. }));
    }

    #[test]
    fn service_restart_failure_is_class_4_incoherent() {
        struct FailingRestart;
        impl Systemd for FailingRestart {
            fn inventory(
                &self,
            ) -> Result<pearlite_schema::ServiceInventory, pearlite_systemd::SystemdError>
            {
                Ok(pearlite_schema::ServiceInventory::default())
            }
            fn enable(
                &self,
                _: &str,
                _: &SystemdScope,
            ) -> Result<(), pearlite_systemd::SystemdError> {
                Ok(())
            }
            fn disable(
                &self,
                _: &str,
                _: &SystemdScope,
            ) -> Result<(), pearlite_systemd::SystemdError> {
                Ok(())
            }
            fn mask(&self, _: &str) -> Result<(), pearlite_systemd::SystemdError> {
                Ok(())
            }
            fn restart(&self, _: &str) -> Result<(), pearlite_systemd::SystemdError> {
                Err(pearlite_systemd::SystemdError::NotInPath { hint: "test" })
            }
        }

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let snapper = MockSnapper::new();
        let home_manager = MockHmBackend::new();
        let state_dir = TempDir::new().expect("state tempdir");
        let (state_path, failures_dir) = setup_state(&state_dir);

        let err = engine()
            .apply_plan(
                &plan_with_actions(vec![Action::ServiceRestart {
                    unit: "sshd.service".to_owned(),
                }]),
                &pacman,
                &cargo,
                &FailingRestart,
                &snapper,
                &home_manager,
                "root",
                &state_path,
                &failures_dir,
            )
            .expect_err("must fail");
        assert!(matches!(err, ApplyError::Systemd(_)), "got {err:?}");

        // ServiceRestart is the only Incoherent variant: class 4 / exit 5.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert!(read_back.history.is_empty(), "no history on failure");
        assert_eq!(read_back.failures.len(), 1);
        assert_eq!(read_back.failures[0].class, 4);
        assert_eq!(read_back.failures[0].exit_code, 5);

        // Best-effort post-fail snapshot was taken: total snapshots
        // = pre (1) + post-fail (1) = 2.
        assert_eq!(snapper.list("root").expect("list").len(), 2);
        let labels: Vec<String> = snapper
            .list("root")
            .expect("list")
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert!(
            labels.iter().any(|l| l.starts_with("pre-pearlite-apply-")),
            "pre snapshot labelled, got {labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|l| l.starts_with("post-fail-pearlite-apply-")),
            "post-fail snapshot labelled, got {labels:?}"
        );
    }
}
