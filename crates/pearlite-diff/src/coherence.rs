// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Failure-coherence classification for [`Action`].
//!
//! When an apply step fails, the engine asks the failed action: "if you
//! halt now, is the rest of the system still coherent?" That answer
//! determines the failure class per PRD §8.5:
//!
//! - [`FailureCoherence::Recoverable`] → Class 3 (exit code 4). The
//!   user fixes the root cause and re-applies; no rollback required.
//! - [`FailureCoherence::Incoherent`] → Class 4 (exit code 5). The
//!   user must `pearlite rollback <plan-id>` before re-applying;
//!   re-running apply against a partially-applied state is unsafe.

use crate::action::Action;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether an action's mid-flight failure leaves the system in a
/// coherent state.
///
/// "Coherent" here means: every primitive `Action` is either fully
/// applied or fully not-applied. Most operations achieve this through
/// adapter atomicity (pacman transactions, atomic-rename writes,
/// systemd unit-file symlinks). [`Self::Incoherent`] flags the
/// exception: actions whose mid-flight failure can leave a
/// running-but-stale or transitioning-but-broken system.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureCoherence {
    /// PRD §8.5 Class 3 — apply halted, system coherent.
    Recoverable,
    /// PRD §8.5 Class 4 — apply halted, partial state means re-apply
    /// is unsafe.
    Incoherent,
}

impl Action {
    /// Classify how this action's mid-flight failure should be
    /// recovered.
    ///
    /// The reasoning, variant by variant:
    ///
    /// - **Pacman install / remove / AUR install**: pacman's
    ///   transaction model is atomic per package set; a failed
    ///   transaction leaves the package un-installed (or un-removed).
    ///   `paru` defers to pacman after the build step, so an AUR
    ///   build failure pre-install is also coherent. Recoverable.
    /// - **Cargo install / uninstall**: cargo writes to
    ///   `~/.cargo/bin` via atomic rename; a failed build aborts
    ///   before the rename. Recoverable.
    /// - **`ConfigWrite`**: `pearlite-fs::write_atomic` is the
    ///   canonical atomic-rename pattern; either the new file is in
    ///   place or it never appeared. Recoverable.
    /// - **Service enable / disable / mask**: systemd unit-file state
    ///   is a single symlink update; atomic per-unit. Recoverable.
    /// - **`SnapshotCreate`**: a failed snapshot doesn't mutate the
    ///   target subvolume; the snapshot just doesn't exist. Recoverable.
    /// - **`ServiceRestart`**: the *only* non-atomic primitive.
    ///   `systemctl restart` is stop-then-start; a failure between
    ///   them leaves the unit dead, partially started, or running
    ///   the old binary against new config. Re-applying without
    ///   knowing which leg failed is unsafe. **Incoherent**.
    #[must_use]
    pub fn failure_coherence(&self) -> FailureCoherence {
        match self {
            Self::PacmanInstall { .. }
            | Self::PacmanRemove { .. }
            | Self::AurInstall { .. }
            | Self::CargoInstall { .. }
            | Self::CargoUninstall { .. }
            | Self::ConfigWrite { .. }
            | Self::ServiceEnable { .. }
            | Self::ServiceDisable { .. }
            | Self::ServiceMask { .. }
            | Self::SnapshotCreate { .. } => FailureCoherence::Recoverable,
            Self::ServiceRestart { .. } => FailureCoherence::Incoherent,
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
    use crate::action::{Phase, Scope};
    use std::path::PathBuf;

    #[test]
    fn pacman_install_is_recoverable() {
        let a = Action::PacmanInstall {
            repo: "core".to_owned(),
            packages: vec!["base".to_owned()],
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn pacman_remove_is_recoverable() {
        let a = Action::PacmanRemove {
            packages: vec!["xterm".to_owned()],
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn aur_install_is_recoverable() {
        let a = Action::AurInstall {
            packages: vec!["yay".to_owned()],
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn cargo_install_is_recoverable() {
        let a = Action::CargoInstall {
            crate_name: "zellij".to_owned(),
            locked: true,
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn cargo_uninstall_is_recoverable() {
        let a = Action::CargoUninstall {
            crate_name: "zellij".to_owned(),
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn config_write_is_recoverable() {
        let a = Action::ConfigWrite {
            target: PathBuf::from("/etc/sshd_config"),
            source: PathBuf::from("etc/sshd_config"),
            content_sha256: "deadbeef".to_owned(),
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            declaration_index: 0,
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn service_enable_is_recoverable() {
        let a = Action::ServiceEnable {
            unit: "sshd.service".to_owned(),
            scope: Scope::System,
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn service_disable_is_recoverable() {
        let a = Action::ServiceDisable {
            unit: "sshd.service".to_owned(),
            scope: Scope::System,
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn service_mask_is_recoverable() {
        let a = Action::ServiceMask {
            unit: "telnet.service".to_owned(),
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn snapshot_create_is_recoverable() {
        let a = Action::SnapshotCreate {
            label: "pre-apply".to_owned(),
            phase: Phase::Pre,
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Recoverable);
    }

    #[test]
    fn service_restart_is_incoherent() {
        let a = Action::ServiceRestart {
            unit: "sshd.service".to_owned(),
        };
        assert_eq!(a.failure_coherence(), FailureCoherence::Incoherent);
    }

    #[test]
    fn coherence_is_independent_of_action_payload() {
        let a1 = Action::ServiceRestart {
            unit: "sshd.service".to_owned(),
        };
        let a2 = Action::ServiceRestart {
            unit: "nginx.service".to_owned(),
        };
        assert_eq!(a1.failure_coherence(), a2.failure_coherence());
    }
}
