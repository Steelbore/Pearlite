// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Per-repo package declarations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Declared package lists, partitioned by source repository.
///
/// The `cachyos-v3` and `cachyos-v4` keys correspond to the CachyOS
/// CPU-feature-level repositories; only one applies at apply time per the
/// host's [`HostMeta::arch_level`](crate::HostMeta).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct PackageSet {
    /// Arch official `core`/`extra`/`multilib` repos.
    #[serde(default)]
    pub core: Vec<String>,
    /// CachyOS-flavored generic repository (`cachyos`).
    #[serde(default)]
    pub cachyos: Vec<String>,
    /// CachyOS x86-64-v3 repository.
    #[serde(default, rename = "cachyos-v3")]
    pub cachyos_v3: Vec<String>,
    /// CachyOS x86-64-v4 repository.
    #[serde(default, rename = "cachyos-v4")]
    pub cachyos_v4: Vec<String>,
    /// Arch User Repository packages, installed via `paru`.
    #[serde(default)]
    pub aur: Vec<String>,
    /// Crates installed via `cargo install`.
    #[serde(default)]
    pub cargo: Vec<String>,
}
