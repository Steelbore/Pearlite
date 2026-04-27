// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `ProbedState` and per-subsystem inventory types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use time::OffsetDateTime;

/// One consistent snapshot of the live system state, produced by a
/// `SystemProbe` implementation in `pearlite-engine`.
///
/// Each subsystem's inventory is `Option`-wrapped because adapters can fail
/// independently; the engine surfaces partial probes as Class-2 plan
/// failures rather than refusing to render anything at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProbedState {
    /// UTC instant the probe was taken.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub probed_at: OffsetDateTime,
    /// Live host identity.
    pub host: HostInfo,
    /// pacman/AUR inventory.
    pub pacman: Option<PacmanInventory>,
    /// `cargo install` inventory.
    pub cargo: Option<CargoInventory>,
    /// Per-target `/etc` file metadata.
    pub config_files: Option<ConfigFileInventory>,
    /// systemd unit state.
    pub services: Option<ServiceInventory>,
    /// Live kernel info.
    pub kernel: KernelInfo,
}

/// Live host identity, as queried at probe time.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct HostInfo {
    /// `hostname(1)` value.
    pub hostname: String,
}

/// pacman/AUR inventory: explicitly-installed packages plus their repo
/// classification.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct PacmanInventory {
    /// Packages reported by `pacman -Qe`.
    pub explicit: BTreeSet<String>,
    /// Packages reported by `pacman -Qm` (foreign / AUR).
    pub foreign: BTreeSet<String>,
    /// Per-package source repo as resolved via `/etc/pacman.conf`.
    pub repos: BTreeMap<String, String>,
}

/// `cargo install --list` inventory.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct CargoInventory {
    /// Installed crate names with their version strings.
    pub crates: BTreeMap<String, String>,
}

/// Per-target `/etc` file metadata at probe time.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ConfigFileInventory {
    /// `target -> ConfigFileMeta`. Missing targets are absent from the map.
    pub entries: BTreeMap<PathBuf, ConfigFileMeta>,
}

/// Metadata for one observed `/etc` file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigFileMeta {
    /// SHA-256 of the file's contents at probe time, hex-encoded.
    pub sha256: String,
    /// File mode (`stat(2).st_mode & 0o7777`).
    pub mode: u32,
    /// Owning user (resolved from uid).
    pub owner: String,
    /// Owning group (resolved from gid).
    pub group: String,
}

/// systemd unit-state inventory at probe time.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ServiceInventory {
    /// Units reported as `enabled` by `systemctl list-unit-files`.
    pub enabled: BTreeSet<String>,
    /// Units reported as `disabled`.
    pub disabled: BTreeSet<String>,
    /// Units reported as `masked`.
    pub masked: BTreeSet<String>,
    /// Currently `active` units (as reported by `systemctl list-units`).
    pub active: BTreeSet<String>,
}

/// Live kernel info.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct KernelInfo {
    /// Running kernel version (`uname -r`).
    pub running_version: String,
    /// Currently-installed kernel package name.
    pub package: String,
    /// Kernel modules currently loaded.
    pub loaded_modules: BTreeSet<String>,
}
