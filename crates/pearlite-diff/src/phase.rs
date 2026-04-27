// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Apply-phase classification for [`Action`].
//!
//! PRD §8.2 defines the seven-phase apply pipeline. The diff engine
//! emits `Action` variants that fall into a subset of those phases;
//! the apply engine partitions a [`Plan`](crate::Plan)'s actions by
//! [`Action::phase`] and executes each phase in [`ApplyPhase`]'s
//! declared order, sorting within each phase by
//! [`Action::within_phase_key`].

use crate::action::{Action, Phase};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Pearlite's apply pipeline, partitioned by phase per PRD §8.2.
///
/// `Ord` is derived from declaration order, so the apply engine can
/// iterate phases in their canonical sequence with no auxiliary
/// table.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    /// PRD §8.2 phase 1 — pre-apply snapshot.
    SnapshotPre,
    /// PRD §8.2 phase 2 — removals (cargo first, then pacman).
    Removals,
    /// PRD §8.2 phase 3 — installs (repo → cachyos → vN → AUR → cargo).
    Installs,
    /// PRD §8.2 phase 4 — atomic config writes in declaration order.
    ConfigWrites,
    /// PRD §8.2 phase 5 — service state (mask → disable → enable).
    ServiceState,
    /// PRD §8.2 phase 6 — deduplicated service restarts.
    ServiceRestarts,
    /// PRD §8.2 phase 8 — post-apply snapshot.
    SnapshotPost,
    /// PRD §8.5 Class 3/4 — post-failure forensic snapshot.
    ///
    /// Outside the linear apply sequence; produced only on the
    /// failure path. Sorted last so a stable phase iteration won't
    /// accidentally take it.
    SnapshotPostFail,
}

impl Action {
    /// Classify this action into its apply phase per PRD §8.2.
    ///
    /// The match is exhaustive over every [`Action`] variant. Adding
    /// a new variant requires adding an arm here, keeping the apply
    /// engine's phase partitioner correct by construction.
    #[must_use]
    pub fn phase(&self) -> ApplyPhase {
        match self {
            Self::CargoUninstall { .. } | Self::PacmanRemove { .. } => ApplyPhase::Removals,
            Self::PacmanInstall { .. } | Self::AurInstall { .. } | Self::CargoInstall { .. } => {
                ApplyPhase::Installs
            }
            Self::ConfigWrite { .. } => ApplyPhase::ConfigWrites,
            Self::ServiceMask { .. } | Self::ServiceDisable { .. } | Self::ServiceEnable { .. } => {
                ApplyPhase::ServiceState
            }
            Self::ServiceRestart { .. } => ApplyPhase::ServiceRestarts,
            Self::SnapshotCreate { phase, .. } => match phase {
                Phase::Pre => ApplyPhase::SnapshotPre,
                Phase::Post => ApplyPhase::SnapshotPost,
                Phase::PostFail => ApplyPhase::SnapshotPostFail,
            },
        }
    }
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
    use crate::action::Scope;
    use std::path::PathBuf;

    #[test]
    fn cargo_uninstall_is_removals() {
        let a = Action::CargoUninstall {
            crate_name: "zellij".to_owned(),
        };
        assert_eq!(a.phase(), ApplyPhase::Removals);
    }

    #[test]
    fn pacman_remove_is_removals() {
        let a = Action::PacmanRemove {
            packages: vec!["xterm".to_owned()],
        };
        assert_eq!(a.phase(), ApplyPhase::Removals);
    }

    #[test]
    fn pacman_install_is_installs() {
        let a = Action::PacmanInstall {
            repo: "core".to_owned(),
            packages: vec!["base".to_owned()],
        };
        assert_eq!(a.phase(), ApplyPhase::Installs);
    }

    #[test]
    fn aur_install_is_installs() {
        let a = Action::AurInstall {
            packages: vec!["yay".to_owned()],
        };
        assert_eq!(a.phase(), ApplyPhase::Installs);
    }

    #[test]
    fn cargo_install_is_installs() {
        let a = Action::CargoInstall {
            crate_name: "zellij".to_owned(),
            locked: true,
        };
        assert_eq!(a.phase(), ApplyPhase::Installs);
    }

    #[test]
    fn config_write_is_config_writes() {
        let a = Action::ConfigWrite {
            target: PathBuf::from("/etc/sshd_config"),
            content_sha256: "abc".to_owned(),
            declaration_index: 0,
        };
        assert_eq!(a.phase(), ApplyPhase::ConfigWrites);
    }

    #[test]
    fn service_mask_is_service_state() {
        let a = Action::ServiceMask {
            unit: "telnet.service".to_owned(),
        };
        assert_eq!(a.phase(), ApplyPhase::ServiceState);
    }

    #[test]
    fn service_disable_is_service_state() {
        let a = Action::ServiceDisable {
            unit: "x.service".to_owned(),
            scope: Scope::System,
        };
        assert_eq!(a.phase(), ApplyPhase::ServiceState);
    }

    #[test]
    fn service_enable_is_service_state() {
        let a = Action::ServiceEnable {
            unit: "x.service".to_owned(),
            scope: Scope::System,
        };
        assert_eq!(a.phase(), ApplyPhase::ServiceState);
    }

    #[test]
    fn service_restart_is_service_restarts() {
        let a = Action::ServiceRestart {
            unit: "sshd.service".to_owned(),
        };
        assert_eq!(a.phase(), ApplyPhase::ServiceRestarts);
    }

    #[test]
    fn snapshot_pre_is_snapshot_pre() {
        let a = Action::SnapshotCreate {
            label: "pre".to_owned(),
            phase: Phase::Pre,
        };
        assert_eq!(a.phase(), ApplyPhase::SnapshotPre);
    }

    #[test]
    fn snapshot_post_is_snapshot_post() {
        let a = Action::SnapshotCreate {
            label: "post".to_owned(),
            phase: Phase::Post,
        };
        assert_eq!(a.phase(), ApplyPhase::SnapshotPost);
    }

    #[test]
    fn snapshot_post_fail_is_snapshot_post_fail() {
        let a = Action::SnapshotCreate {
            label: "fail".to_owned(),
            phase: Phase::PostFail,
        };
        assert_eq!(a.phase(), ApplyPhase::SnapshotPostFail);
    }

    #[test]
    fn phase_ordering_matches_prd_8_2_sequence() {
        assert!(ApplyPhase::SnapshotPre < ApplyPhase::Removals);
        assert!(ApplyPhase::Removals < ApplyPhase::Installs);
        assert!(ApplyPhase::Installs < ApplyPhase::ConfigWrites);
        assert!(ApplyPhase::ConfigWrites < ApplyPhase::ServiceState);
        assert!(ApplyPhase::ServiceState < ApplyPhase::ServiceRestarts);
        assert!(ApplyPhase::ServiceRestarts < ApplyPhase::SnapshotPost);
        assert!(ApplyPhase::SnapshotPost < ApplyPhase::SnapshotPostFail);
    }
}
