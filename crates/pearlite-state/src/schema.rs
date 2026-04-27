// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Top-level [`State`] struct and the `[managed.*]` / `[adopted.*]` records
//! it owns.

use crate::failure::FailureRef;
use crate::history::HistoryEntry;
use crate::reconciliation::ReconciliationEntry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use time::OffsetDateTime;

/// Current `schema_version` written by this build.
///
/// Incremented when the on-disk shape changes incompatibly. Migration
/// from older versions lands in `migrate.rs` (chunk M1-W1-F).
pub const SCHEMA_VERSION: u32 = 1;

/// In-memory mirror of `/var/lib/pearlite/state.toml`.
///
/// The full schema is specified in PRD §7.2. `State` is mutated only by
/// `pearlite-engine`; every other crate consumes it read-only.
///
/// `Eq` is intentionally **not** derived: the `reserved` field carries a
/// `toml::Value` map for forward-compatibility, and `toml::Value::Float`
/// wraps an `f64` that is not `Eq`. Use `PartialEq` (which is derived)
/// for equality checks in tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct State {
    /// Schema version of the file at last write. Absent or `0` means a
    /// pre-versioned file written by an alpha build; the migration
    /// framework in `migrate.rs` upgrades it on read.
    #[serde(default)]
    pub schema_version: u32,
    /// Hostname this state belongs to. Mismatch with `hostname()` is
    /// preflight Class 1.
    pub host: String,
    /// Pearlite version that last wrote the file.
    pub tool_version: String,
    /// Path to the user's Pearlite config repository at last apply.
    pub config_dir: PathBuf,
    /// Timestamp of the last successful `apply`.
    #[serde(with = "time::serde::iso8601::option", default)]
    #[schemars(with = "Option<String>")]
    pub last_apply: Option<OffsetDateTime>,
    /// Timestamp of the last write of any kind.
    #[serde(with = "time::serde::iso8601::option", default)]
    #[schemars(with = "Option<String>")]
    pub last_modified: Option<OffsetDateTime>,
    /// Pearlite-managed packages, configs, services, kernel, and user envs.
    #[serde(default)]
    pub managed: Managed,
    /// User-flagged "leave alone" packages.
    #[serde(default)]
    pub adopted: Adopted,
    /// Append-only log of successful applies.
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// Log of `pearlite reconcile --commit` invocations.
    #[serde(default)]
    pub reconciliations: Vec<ReconciliationEntry>,
    /// Pointers to `/var/lib/pearlite/failures/<plan-id>.json` records.
    #[serde(default)]
    pub failures: Vec<FailureRef>,
    /// Forward-compat namespace; arbitrary unknown keys preserved on
    /// round-trip via the toml_edit-based atomic writer. Excluded from
    /// the JSON Schema because its values are untyped by design.
    #[serde(default)]
    #[schemars(skip)]
    pub reserved: BTreeMap<String, toml::Value>,
}

/// `[managed.*]` namespace: everything Pearlite has installed or written.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Managed {
    /// pacman/AUR packages installed via Pearlite.
    #[serde(default)]
    pub pacman: Vec<String>,
    /// Cargo crates installed via Pearlite.
    #[serde(default)]
    pub cargo: Vec<String>,
    /// Per-file metadata for managed `/etc` config files.
    #[serde(default)]
    pub config_files: Vec<ConfigFileRecord>,
    /// systemd unit state at last apply.
    #[serde(default)]
    pub services: ServicesState,
    /// Per-user Home Manager generation and config hash.
    #[serde(default)]
    pub user_env: Vec<UserEnvRecord>,
    /// Kernel package, version, cmdline, modules at last apply.
    #[serde(default)]
    pub kernel: Option<KernelRecord>,
}

/// One managed `/etc` config file's record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigFileRecord {
    /// Absolute destination path on the host.
    pub target: PathBuf,
    /// Source path within the user's config repo.
    pub source_in_repo: PathBuf,
    /// SHA-256 of the file contents at last successful write, hex-encoded.
    pub sha256: String,
    /// File mode (`stat(2).st_mode & 0o7777`).
    pub mode: u32,
    /// Owning user.
    pub owner: String,
    /// Owning group.
    pub group: String,
}

/// systemd-unit state recorded by Pearlite at apply time.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ServicesState {
    /// Units enabled (and started) by Pearlite.
    #[serde(default)]
    pub enabled: Vec<String>,
    /// Units disabled by Pearlite.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Units masked by Pearlite.
    #[serde(default)]
    pub masked: Vec<String>,
}

/// One user's Home Manager generation pointer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserEnvRecord {
    /// Login name.
    pub user: String,
    /// Home Manager generation number at last apply.
    pub generation: u64,
    /// SHA-256 of the user's HM config directory at last apply.
    pub config_hash: String,
}

/// Kernel record: package, version, cmdline, modules at last apply.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct KernelRecord {
    /// Kernel package name (e.g. `linux-cachyos`).
    pub package: String,
    /// Installed kernel version string.
    pub version: String,
    /// Kernel command-line as last applied.
    #[serde(default)]
    pub cmdline: Vec<String>,
    /// Loaded modules as last applied.
    #[serde(default)]
    pub modules: Vec<String>,
}

/// `[adopted.*]` namespace: drift items the user has explicitly told
/// Pearlite to leave alone.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Adopted {
    /// Adopted pacman/AUR packages.
    #[serde(default)]
    pub pacman: Vec<String>,
    /// Adopted cargo crates.
    #[serde(default)]
    pub cargo: Vec<String>,
}
