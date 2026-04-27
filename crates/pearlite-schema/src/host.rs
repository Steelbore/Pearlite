// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Host metadata and kernel declaration types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// CPU feature level a host targets.
///
/// CachyOS exposes per-feature-level package repositories (`cachyos-v3`,
/// `cachyos-v4`); this enum is what the host config declares and what the
/// preflight matches against `/proc/cpuinfo` at apply time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArchLevel {
    /// `x86-64-v3` — Haswell-era and newer.
    V3,
    /// `x86-64-v4` — AVX-512 capable parts.
    V4,
}

/// Per-host metadata declared in `hosts/<host>.ncl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HostMeta {
    /// `hostname(1)` value this configuration applies to.
    pub hostname: String,
    /// IANA timezone identifier (e.g. `Europe/London`).
    pub timezone: String,
    /// Declared CPU feature level.
    pub arch_level: ArchLevel,
    /// Locale identifier; defaults to `en_US.UTF-8` upstream.
    pub locale: String,
    /// Keymap identifier; defaults to `us` upstream.
    pub keymap: String,
}

/// Declared kernel selection and boot options.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct KernelDecl {
    /// Kernel package name (e.g. `linux-cachyos`, `linux-lts`).
    pub package: String,
    /// Extra kernel command-line parameters appended to the bootloader entry.
    #[serde(default)]
    pub cmdline: Vec<String>,
    /// Kernel modules to load at boot.
    #[serde(default)]
    pub modules: Vec<String>,
    /// Kernel modules to blacklist.
    #[serde(default)]
    pub blacklist: Vec<String>,
}
