// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `[[reconciliations]]` records — one per `pearlite reconcile --commit`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// One entry in `state.toml`'s `[[reconciliations]]` array.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReconciliationEntry {
    /// Plan UUID generated for this reconciliation.
    #[schemars(with = "String")]
    pub plan_id: Uuid,
    /// UTC timestamp when the reconciliation committed.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub committed_at: OffsetDateTime,
    /// Resolution chosen for the drift items at commit time.
    pub action: ReconciliationAction,
    /// Number of packages classified during this reconciliation.
    pub package_count: u32,
}

/// How a reconciliation resolved its detected drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationAction {
    /// Non-interactive `--adopt-all`: every drift item moved to `adopted`.
    AdoptAll,
    /// Interactive prompt path: per-package decisions taken.
    Interactive,
    /// Reconciliation aborted before any state was written.
    Skipped,
}
