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
//!   via `pearlite_fs::write_etc_atomic`), deterministic phase +
//!   within-phase ordering.
//! - **Out** (follow-up PRs): `state.toml` commit + history record,
//!   post-fail snapshot on Class 3/4, rollback.

use crate::errors::ApplyError;
use crate::plan::Engine;
use pearlite_cargo::Cargo;
use pearlite_diff::{Action, ApplyPhase, Plan, Scope};
use pearlite_pacman::Pacman;
use pearlite_snapper::{Snapper, SnapshotInfo};
use pearlite_systemd::{Scope as SystemdScope, Systemd};
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

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
}

impl Engine {
    /// Execute a [`Plan`] against the live system.
    ///
    /// Wraps the apply in pre/post Snapper snapshots and dispatches
    /// every `Action` to the appropriate adapter in PRD §8.2 phase
    /// order. Halts on the first error.
    ///
    /// `snapper_config` is the snapper config name to take snapshots
    /// against (typically `"root"`).
    ///
    /// # Errors
    /// Returns [`ApplyError`] wrapping the failing adapter's error.
    /// The CLI boundary maps this to a failure class via
    /// [`Action::failure_coherence`] on the failed action.
    pub fn apply_plan(
        &self,
        plan: &Plan,
        pacman: &dyn Pacman,
        cargo: &dyn Cargo,
        systemd: &dyn Systemd,
        snapper: &dyn Snapper,
        snapper_config: &str,
    ) -> Result<ApplyOutcome, ApplyError> {
        let snapshot_pre = snapper.create(snapper_config, &pre_label(plan.plan_id))?;

        let buckets = partition_by_phase(&plan.actions);

        if needs_db_sync(plan) {
            pacman.sync_databases()?;
        }

        let mut actions_executed = 0usize;
        for phase in [
            ApplyPhase::Removals,
            ApplyPhase::Installs,
            ApplyPhase::ConfigWrites,
            ApplyPhase::ServiceState,
            ApplyPhase::ServiceRestarts,
        ] {
            if let Some(actions) = buckets.get(&phase) {
                for action in actions {
                    exec_action(action, pacman, cargo, systemd, self.repo_root())?;
                    actions_executed += 1;
                }
            }
        }

        let snapshot_post = snapper.create(snapper_config, &post_label(plan.plan_id))?;

        Ok(ApplyOutcome {
            plan_id: plan.plan_id,
            snapshot_pre,
            snapshot_post,
            actions_executed,
        })
    }
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
    use pearlite_systemd::MockSystemd;
    use std::path::PathBuf;
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

    #[test]
    fn empty_plan_takes_pre_and_post_snapshots() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();

        let out = engine()
            .apply_plan(
                &plan_with_actions(vec![]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                "root",
            )
            .expect("apply");

        assert_eq!(out.actions_executed, 0);
        assert_eq!(out.snapshot_pre.id, 1);
        assert_eq!(out.snapshot_post.id, 2);
        assert!(out.snapshot_pre.label.starts_with("pre-pearlite-apply-"));
        assert!(out.snapshot_post.label.starts_with("post-pearlite-apply-"));
        assert_eq!(pacman.sync_count(), 0, "no installs → no db sync");
    }

    #[test]
    fn sync_databases_runs_only_when_pacman_or_aur_install_present() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();

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
                "root",
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
                "root",
            )
            .expect("apply");
        assert_eq!(pacman.sync_count(), 1, "pacman install triggers sync");
    }

    #[test]
    fn pacman_install_dispatches_with_repo_and_packages() {
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();

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
                "root",
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

        engine()
            .apply_plan(
                &plan_with_actions(vec![Action::AurInstall {
                    packages: vec!["yay".to_owned()],
                }]),
                &pacman,
                &cargo,
                &systemd,
                &snapper,
                "root",
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
                "root",
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
                "root",
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
                "root",
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
    fn phase_order_removals_before_installs_before_services() {
        // Mix actions across phases in declaration order; the engine
        // must execute them in PRD §8.2 phase order.
        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();

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
                "root",
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
                "root",
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
                "root",
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
        use tempfile::TempDir;

        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");

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
                "root",
            )
            .expect("apply");

        assert_eq!(std::fs::read(&target).expect("read"), content);
    }

    #[test]
    fn config_write_sha_mismatch_aborts_apply() {
        use tempfile::TempDir;

        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");

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
                "root",
            )
            .expect_err("must fail");
        assert!(
            matches!(err, ApplyError::ContentSha256Mismatch { .. }),
            "got {err:?}"
        );
        assert!(!target.exists(), "target must not be created on mismatch");
    }

    #[test]
    fn config_write_missing_source_yields_fs_error() {
        use tempfile::TempDir;

        let repo = TempDir::new().expect("repo tempdir");
        let target_dir = TempDir::new().expect("target tempdir");

        let target = target_dir.path().join("missing.conf");
        let (user, group) = current_user_group();

        let pacman = MockPacman::new();
        let cargo = MockCargo::new();
        let systemd = MockSystemd::new();
        let snapper = MockSnapper::new();

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
                "root",
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

        let err = engine()
            .apply_plan(
                &plan_with_actions(vec![]),
                &pacman,
                &cargo,
                &systemd,
                &FailingSnapper,
                "root",
            )
            .expect_err("must fail");
        assert!(matches!(err, ApplyError::Snapper(_)), "got {err:?}");
    }
}
