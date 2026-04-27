// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! systemd service-state declarations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Declared systemd-unit state across three disjoint categories.
///
/// At apply time the engine resolves these into ordered actions per the
/// `mask → disable → enable` order in PRD §8.2.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ServicesDecl {
    /// Units that must be enabled and started.
    #[serde(default)]
    pub enabled: Vec<String>,
    /// Units that must be disabled (and stopped).
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Units that must be masked.
    #[serde(default)]
    pub masked: Vec<String>,
}
