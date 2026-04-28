// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Action`] — every primitive operation the apply engine can execute.
//!
//! Per PRD §8.1, every primitive operation is one variant of a single
//! flat enum. Adding an operation is one variant plus one match arm in
//! `pearlite-engine::exec`. No dispatch tables, no trait objects in the
//! hot path.

use pearlite_schema::HomeManagerMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One primitive operation Pearlite can execute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Install one or more packages from a repository via pacman.
    PacmanInstall {
        /// Repo name as it appears in `pacman.conf` (e.g. `core`,
        /// `cachyos-v4`). Adapter crates resolve this to their typed
        /// `Repo` enum at execution time.
        repo: String,
        /// Package names to install.
        packages: Vec<String>,
    },
    /// Remove one or more pacman/AUR packages.
    PacmanRemove {
        /// Package names to remove.
        packages: Vec<String>,
    },
    /// Install one or more AUR packages via paru.
    AurInstall {
        /// AUR package names to install.
        packages: Vec<String>,
    },
    /// Install one cargo crate from crates.io.
    CargoInstall {
        /// Crate name.
        crate_name: String,
        /// Whether to pass `--locked`.
        locked: bool,
    },
    /// Uninstall one cargo crate.
    CargoUninstall {
        /// Crate name.
        crate_name: String,
    },
    /// Atomically write a config file to `/etc`.
    ConfigWrite {
        /// Absolute destination path (e.g. `/etc/sshd_config`).
        target: PathBuf,
        /// Path of the source file relative to the user's config repo
        /// root. The engine resolves it as `repo_root.join(source)`
        /// at apply time, reads the bytes, and verifies the digest
        /// against [`Self::content_sha256`] before writing.
        source: PathBuf,
        /// SHA-256 of the content to write, hex-encoded. Recomputed at
        /// apply time and compared against the source bytes; a
        /// mismatch aborts the apply with a Class 3 error.
        content_sha256: String,
        /// File mode (`stat(2).st_mode & 0o7777`) the target should
        /// end up with after the write.
        mode: u32,
        /// Owner login name to chown the target to.
        owner: String,
        /// Group name to chown the target to.
        group: String,
        /// Original index in the host's `[[config]]` array; the engine
        /// sorts within phase 4 by this value to honour the user's
        /// declared order (PRD §8.3).
        declaration_index: usize,
    },
    /// Enable a systemd unit.
    ServiceEnable {
        /// Unit name (e.g. `sshd.service`).
        unit: String,
        /// System or per-user scope.
        scope: Scope,
    },
    /// Disable a systemd unit.
    ServiceDisable {
        /// Unit name.
        unit: String,
        /// System or per-user scope.
        scope: Scope,
    },
    /// Mask a systemd unit.
    ServiceMask {
        /// Unit name.
        unit: String,
    },
    /// Restart a systemd unit (system-scope only; user-scope restarts
    /// land with M3's user-env phase).
    ServiceRestart {
        /// Unit name.
        unit: String,
    },
    /// Run `home-manager switch` for one declared user (PRD §8.2
    /// phase 7). Emitted by the diff when the user's
    /// `home_manager.enabled` is true and either the config-hash
    /// drifts or no `[[managed.user_env]]` record exists yet.
    UserEnvSwitch {
        /// Login name to drop privileges to via `runuser`.
        user: String,
        /// Absolute path of the user's HM config directory.
        config_path: PathBuf,
        /// Home Manager invocation style (Standalone vs Flake).
        mode: HomeManagerMode,
        /// Channel/refspec for HM (e.g. `release-24.11`).
        channel: String,
        /// Hex-encoded SHA-256 of the user's `config_path` directory
        /// at plan time. Recorded into `state.toml`'s
        /// `[[managed.user_env]]` after the switch succeeds so the
        /// next `pearlite plan` can detect drift via a hash compare.
        /// Empty when the engine couldn't compute one (defensive
        /// arm in `classify_user_env`); the apply path still records
        /// it but the next plan will recompute and re-apply.
        config_hash: String,
    },
    /// Take a Snapper snapshot.
    SnapshotCreate {
        /// Snapshot label.
        label: String,
        /// Which phase boundary this snapshot marks.
        phase: Phase,
    },
}

/// systemd unit scope: system or per-user.
///
/// User-scope operations drop privileges via runuser; the actual
/// drop happens in `pearlite-userenv`, not here.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// System-wide (`systemctl --system`).
    System,
    /// Per-user (`systemctl --user` after `runuser`).
    User {
        /// Login name of the target user.
        name: String,
    },
}

/// Snapshot phase identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Pre-apply snapshot (taken in phase 1; rollback target).
    Pre,
    /// Post-apply snapshot (taken in phase 8; forensic + forward-rollback target).
    Post,
    /// Post-fail snapshot (taken on Class 3/4 failure; forensic only).
    PostFail,
}
