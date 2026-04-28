// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `DeclaredState`: the resolved aggregate of one host's Nickel configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{ConfigEntry, RemovePolicy};
use crate::host::{HostMeta, KernelDecl};
use crate::nix::NixDecl;
use crate::packages::PackageSet;
use crate::services::ServicesDecl;
use crate::snapshots::SnapshotPolicy;
use crate::users::UserDecl;

/// The complete declared state of one host, resolved from Nickel into TOML
/// and parsed into Rust by `from_resolved_toml` (lands in chunk M1-W1-B).
///
/// Every adapter consumes this type read-only; only the engine ever owns it.
/// The TOML key `meta` maps to the `host` field, and `config` (a top-level
/// array of tables in Nickel) maps to `config_files`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeclaredState {
    /// Host metadata (hostname, timezone, arch level, locale, keymap).
    #[serde(rename = "meta")]
    pub host: HostMeta,
    /// Kernel package and boot options.
    pub kernel: KernelDecl,
    /// Per-repo package lists.
    #[serde(default)]
    pub packages: PackageSet,
    /// Config files to render under `/etc`.
    #[serde(default, rename = "config")]
    pub config_files: Vec<ConfigEntry>,
    /// systemd unit state by category.
    #[serde(default)]
    pub services: ServicesDecl,
    /// Declared users and their optional Home Manager blocks.
    #[serde(default)]
    pub users: Vec<UserDecl>,
    /// Removal policy.
    #[serde(default)]
    pub remove: RemovePolicy,
    /// Snapshot retention policy.
    #[serde(default)]
    pub snapshots: SnapshotPolicy,
    /// Optional nix bootstrap declaration. Required by the schema
    /// validator iff any user has `home_manager.enabled = true` —
    /// see [`ContractViolation::NIX_INSTALLER_REQUIRED`](crate::ContractViolation).
    #[serde(default)]
    pub nix: Option<NixDecl>,
}
