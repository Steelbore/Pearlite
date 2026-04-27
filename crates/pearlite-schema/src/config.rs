// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Config-file declarations and removal policy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One declared config file: rendered from a source path in the user's
/// config repo, written to a target under `/etc`, with mode/owner/group, and
/// optionally restarting services on change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigEntry {
    /// Absolute destination path on the host (e.g. `/etc/sshd_config`).
    pub target: PathBuf,
    /// Source path within the user's Pearlite config repository.
    pub source: PathBuf,
    /// File mode in decimal-encoded octal; default 420 = `0o644`.
    #[serde(default = "default_mode")]
    pub mode: u32,
    /// Owning user (default `root`).
    #[serde(default = "default_root")]
    pub owner: String,
    /// Owning group (default `root`).
    #[serde(default = "default_root")]
    pub group: String,
    /// systemd unit names to restart after this file is written.
    #[serde(default)]
    pub restart: Vec<String>,
}

fn default_mode() -> u32 {
    0o644
}

fn default_root() -> String {
    String::from("root")
}

/// Declared removal policy: packages to ensure removed, plus packages that
/// must never be flagged as drift even if present out-of-band.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct RemovePolicy {
    /// Packages to remove on apply.
    #[serde(default)]
    pub packages: Vec<String>,
    /// Packages that should never be flagged as drift.
    #[serde(default)]
    pub ignore: Vec<String>,
}
