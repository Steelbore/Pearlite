// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Plan`] — the load-bearing artifact between `plan` and `apply`.

use crate::action::Action;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Serializable description of what an apply would do.
///
/// Per PRD §5.2 the Plan is **a value, not a control-flow object**:
/// `pearlite plan` produces it and renders it for human/agent
/// consumption; `pearlite apply` produces it and executes it. Same
/// machinery; same data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// Plan UUID (canonical persistent identifier).
    #[schemars(with = "String")]
    pub plan_id: Uuid,
    /// Host this plan applies to.
    pub host: String,
    /// UTC timestamp the plan was generated.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub generated_at: OffsetDateTime,
    /// Actions to execute in declaration / phase order.
    pub actions: Vec<Action>,
    /// Out-of-band changes detected during planning.
    pub drift: Vec<DriftItem>,
    /// Non-fatal advisories surfaced during planning.
    pub warnings: Vec<Warning>,
}

/// One out-of-band change detected during planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DriftItem {
    /// Which subsystem the drift was found in.
    pub category: DriftCategory,
    /// Stable identifier — package name, target path, unit name.
    pub identifier: String,
    /// Human-readable explanation, suitable for both TTY rendering and
    /// agent consumption.
    pub details: String,
}

/// Drift classification per PRD §10.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DriftCategory {
    /// Package present in `pacman -Qe` but not in the declared config
    /// and not adopted. Surfaced as drift; never auto-removed.
    ManualPackage,
    /// `/etc` file whose SHA-256 differs from the declared source.
    ConfigFile,
    /// systemd unit state differs from the declared `services` block.
    ServiceState,
}

/// Non-fatal advisory emitted during planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Warning {
    /// Stable code (e.g. `ARCH_LEVEL_MISMATCH`, `KERNEL_MODULE_NOT_FOUND`).
    pub code: String,
    /// Human-readable message.
    pub message: String,
}
