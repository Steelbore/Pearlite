// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `[[history]]` and snapshot-reference records.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// One entry in `state.toml`'s `[[history]]` array — a summary of one
/// successful `apply`. The full plan and per-action breakdown live in
/// `/var/lib/pearlite/plans/<plan-id>.json`, referenced by `plan_id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HistoryEntry {
    /// Plan UUID (canonical persistent identifier).
    #[schemars(with = "String")]
    pub plan_id: Uuid,
    /// Host-scoped monotonic generation number.
    pub generation: u64,
    /// UTC timestamp the apply completed successfully.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub applied_at: OffsetDateTime,
    /// Wall-clock duration of the apply, in milliseconds.
    pub duration_ms: u64,
    /// Pre-apply snapshot reference.
    pub snapshot_pre: SnapshotRef,
    /// Post-apply snapshot reference.
    pub snapshot_post: SnapshotRef,
    /// Number of actions executed in this apply.
    pub actions_executed: u32,
    /// Git revision of the config repo at apply time, if available.
    #[serde(default)]
    pub git_revision: Option<String>,
    /// Whether the config repo had uncommitted changes at apply time.
    #[serde(default)]
    pub git_dirty: bool,
    /// One-line summary, e.g. `"+8 -2 ~4 (8 installs, 2 removals, 4 config updates)"`.
    pub summary: String,
}

/// Reference to a Snapper snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotRef {
    /// Snapper snapshot ID.
    pub id: u64,
    /// Snapper snapshot label.
    pub label: String,
    /// UTC creation timestamp.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub created_at: OffsetDateTime,
}
