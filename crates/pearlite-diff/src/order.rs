// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Within-phase ordering keys for [`Action`].
//!
//! The engine partitions actions by phase (PRD §8.2) and sorts within
//! each phase by [`Action::within_phase_key`]. The key is a tuple of a
//! sub-phase rank (`u32`) and a name string — actions with smaller
//! sub-phase rank sort first, ties broken alphabetically.

use crate::action::{Action, Phase};

impl Action {
    /// Return this action's sort key, used to order actions within a
    /// single phase.
    ///
    /// The first element is the sub-phase rank. Within phases that
    /// contain a single action variant, the rank is `0` and ordering
    /// falls to the second element (alphabetical name). Within phases
    /// that contain multiple variants (phase 2 cargo→pacman; phase 3
    /// repo→cachyos→vN→AUR→cargo; phase 5 mask→disable→enable), the
    /// rank encodes that order.
    ///
    /// Note: clippy's `match_same_arms` is suppressed because the
    /// engine partitions actions by phase before sorting; arms that
    /// share a body (e.g. `ServiceMask` rank 0 in phase 5 and
    /// `ServiceRestart` rank 0 in phase 6) are correctly identical for
    /// their respective phases.
    #[must_use]
    #[allow(
        clippy::match_same_arms,
        reason = "phase boundary makes shared sub-phase ranks correct"
    )]
    pub fn within_phase_key(&self) -> (u32, String) {
        match self {
            // Phase 2 — removals: cargo first (free disk space + light
            // ops), then pacman.
            Self::CargoUninstall { crate_name } => (10, crate_name.clone()),
            Self::PacmanRemove { packages } => (20, joined(packages)),

            // Phase 3 — installs: repo > cachyos > vN > AUR > cargo.
            // arch_level preflight guarantees v3 and v4 are mutually
            // exclusive on a given host, so the combined arm is correct.
            Self::PacmanInstall { repo, packages } => {
                let rank = match repo.as_str() {
                    "core" | "extra" | "multilib" => 30,
                    "cachyos" => 40,
                    "cachyos-v3" | "cachyos-v4" => 50,
                    _ => 55,
                };
                (rank, joined(packages))
            }
            Self::AurInstall { packages } => (60, joined(packages)),
            Self::CargoInstall { crate_name, .. } => (70, crate_name.clone()),

            // Phase 4 — config writes: declaration index, alphabetical
            // by target on tie.
            Self::ConfigWrite {
                declaration_index,
                target,
                ..
            } => {
                let rank = u32::try_from(*declaration_index).unwrap_or(u32::MAX);
                (rank, target.to_string_lossy().into_owned())
            }

            // Phase 5 — service state: mask > disable > enable.
            Self::ServiceMask { unit } => (0, unit.clone()),
            Self::ServiceDisable { unit, .. } => (10, unit.clone()),
            Self::ServiceEnable { unit, .. } => (20, unit.clone()),

            // Phase 6 — restarts.
            Self::ServiceRestart { unit } => (0, unit.clone()),

            // Phase 7 — user env: one switch per user, alphabetical
            // by login name. Tied keys never happen because each user
            // emits at most one UserEnvSwitch per plan.
            Self::UserEnvSwitch { user, .. } => (0, user.clone()),

            // Phase 1 / 8 / post-fail — snapshots.
            Self::SnapshotCreate { label, phase } => {
                let rank = match phase {
                    Phase::Pre => 0,
                    Phase::Post => 100,
                    Phase::PostFail => 200,
                };
                (rank, label.clone())
            }
        }
    }
}

fn joined(items: &[String]) -> String {
    items.join(",")
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
    fn install_order_repo_before_cachyos_before_vn_before_aur_before_cargo() {
        let core = Action::PacmanInstall {
            repo: "core".to_owned(),
            packages: vec!["base".to_owned()],
        };
        let cachyos = Action::PacmanInstall {
            repo: "cachyos".to_owned(),
            packages: vec!["cachyos-settings".to_owned()],
        };
        let vn = Action::PacmanInstall {
            repo: "cachyos-v4".to_owned(),
            packages: vec!["firefox".to_owned()],
        };
        let aur = Action::AurInstall {
            packages: vec!["claude-code".to_owned()],
        };
        let cargo_install = Action::CargoInstall {
            crate_name: "zellij".to_owned(),
            locked: true,
        };

        assert!(core.within_phase_key() < cachyos.within_phase_key());
        assert!(cachyos.within_phase_key() < vn.within_phase_key());
        assert!(vn.within_phase_key() < aur.within_phase_key());
        assert!(aur.within_phase_key() < cargo_install.within_phase_key());
    }

    #[test]
    fn removal_order_cargo_before_pacman() {
        let cargo_un = Action::CargoUninstall {
            crate_name: "zellij".to_owned(),
        };
        let pacman_rm = Action::PacmanRemove {
            packages: vec!["xterm".to_owned()],
        };
        assert!(cargo_un.within_phase_key() < pacman_rm.within_phase_key());
    }

    #[test]
    fn service_order_mask_before_disable_before_enable() {
        let m = Action::ServiceMask {
            unit: "x.service".to_owned(),
        };
        let d = Action::ServiceDisable {
            unit: "x.service".to_owned(),
            scope: Scope::System,
        };
        let e = Action::ServiceEnable {
            unit: "x.service".to_owned(),
            scope: Scope::System,
        };
        assert!(m.within_phase_key() < d.within_phase_key());
        assert!(d.within_phase_key() < e.within_phase_key());
    }

    #[test]
    fn config_writes_preserve_declaration_order() {
        let first = Action::ConfigWrite {
            target: PathBuf::from("/etc/b"),
            source: PathBuf::from("etc/b"),
            content_sha256: "abc".to_owned(),
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            declaration_index: 0,
        };
        let second = Action::ConfigWrite {
            target: PathBuf::from("/etc/a"),
            source: PathBuf::from("etc/a"),
            content_sha256: "xyz".to_owned(),
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            declaration_index: 1,
        };
        // /etc/b at index 0 sorts before /etc/a at index 1, even though
        // alphabetically /etc/a comes first.
        assert!(first.within_phase_key() < second.within_phase_key());
    }

    #[test]
    fn snapshot_order_pre_before_post_before_post_fail() {
        let pre = Action::SnapshotCreate {
            label: "pre".to_owned(),
            phase: Phase::Pre,
        };
        let post = Action::SnapshotCreate {
            label: "post".to_owned(),
            phase: Phase::Post,
        };
        let post_fail = Action::SnapshotCreate {
            label: "fail".to_owned(),
            phase: Phase::PostFail,
        };
        assert!(pre.within_phase_key() < post.within_phase_key());
        assert!(post.within_phase_key() < post_fail.within_phase_key());
    }

    #[test]
    fn within_phase_key_is_deterministic() {
        let a = Action::PacmanInstall {
            repo: "core".to_owned(),
            packages: vec!["htop".to_owned(), "vim".to_owned()],
        };
        assert_eq!(a.within_phase_key(), a.within_phase_key());
    }

    #[test]
    fn alphabetical_tie_break_within_same_subphase() {
        let a = Action::ServiceEnable {
            unit: "alpha.service".to_owned(),
            scope: Scope::System,
        };
        let b = Action::ServiceEnable {
            unit: "bravo.service".to_owned(),
            scope: Scope::System,
        };
        assert!(a.within_phase_key() < b.within_phase_key());
    }

    #[test]
    fn user_env_switch_orders_by_user() {
        let alice = Action::UserEnvSwitch {
            user: "alice".to_owned(),
            config_path: PathBuf::from("/repo/users/alice"),
            mode: pearlite_schema::HomeManagerMode::Standalone,
            channel: "release-24.11".to_owned(),
            config_hash: String::new(),
        };
        let bob = Action::UserEnvSwitch {
            user: "bob".to_owned(),
            config_path: PathBuf::from("/repo/users/bob"),
            mode: pearlite_schema::HomeManagerMode::Flake,
            channel: "default".to_owned(),
            config_hash: String::new(),
        };
        assert!(alice.within_phase_key() < bob.within_phase_key());
    }
}
