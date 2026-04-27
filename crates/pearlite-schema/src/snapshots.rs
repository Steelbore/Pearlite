// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Snapshot retention policy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-host snapshot retention configuration.
///
/// Pearlite delegates actual retention to Snapper (via
/// `/etc/snapper/configs/root`); this declaration records the desired policy
/// so reconciliation can detect drift between declared and live Snapper
/// settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotPolicy {
    /// Number of pre/post snapshot pairs to retain.
    #[serde(default = "default_keep")]
    pub keep: u32,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            keep: default_keep(),
        }
    }
}

fn default_keep() -> u32 {
    20
}
