// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! User declarations and the per-user Home Manager block.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One declared user: name, login shell, group memberships, and an optional
/// Home Manager configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserDecl {
    /// Login name.
    pub name: String,
    /// Login shell (e.g. `/usr/bin/nu`).
    pub shell: String,
    /// Supplementary groups.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Home Manager configuration. `None` means Pearlite manages the system
    /// account but never touches dotfiles.
    #[serde(default)]
    pub home_manager: Option<HomeManagerDecl>,
}

/// Per-user Home Manager declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HomeManagerDecl {
    /// Set to `false` to declare a user without HM management at apply time.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Home Manager invocation mode.
    pub mode: HomeManagerMode,
    /// Path within the config repo holding this user's HM config.
    pub config_path: String,
    /// Channel/refspec to use; e.g. `release-24.11`.
    pub channel: String,
}

/// Home Manager invocation style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HomeManagerMode {
    /// Classic Home Manager via the `home-manager` channel.
    Standalone,
    /// Flake-based Home Manager.
    Flake,
}

fn default_true() -> bool {
    true
}
