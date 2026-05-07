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
    /// Number of drift items considered (Manual classification count
    /// at probe time). Audit denominator for `adopted` / `skipped`.
    pub package_count: u32,
    /// Package names actually moved into `state.adopted` by this commit.
    /// Always present; may be empty.
    #[serde(default)]
    pub adopted: Vec<String>,
    /// Package names the operator declined to adopt. Always present;
    /// may be empty.
    #[serde(default)]
    pub skipped: Vec<String>,
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests may use expect()/unwrap() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;

    fn epoch() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts")
    }

    /// Pre-ADR-0014 entries lack `adopted` / `skipped`. They must
    /// deserialize cleanly with both vectors defaulting to `[]`.
    #[test]
    fn legacy_entry_without_decision_vectors_deserializes() {
        let legacy = r#"
plan_id = "00000000-0000-0000-0000-000000000000"
committed_at = "2026-04-01T00:00:00.000000000Z"
action = "interactive"
package_count = 3
"#;
        let entry: ReconciliationEntry = toml::from_str(legacy).expect("legacy parse");
        assert_eq!(entry.package_count, 3);
        assert_eq!(entry.action, ReconciliationAction::Interactive);
        assert!(entry.adopted.is_empty(), "missing adopted defaults to []");
        assert!(entry.skipped.is_empty(), "missing skipped defaults to []");
    }

    /// New entries with populated decision vectors round-trip through
    /// TOML without losing the order or contents of either list.
    #[test]
    fn decision_vectors_round_trip_through_toml() {
        let entry = ReconciliationEntry {
            plan_id: Uuid::nil(),
            committed_at: epoch(),
            action: ReconciliationAction::Interactive,
            package_count: 4,
            adopted: vec!["htop".to_owned(), "ripgrep".to_owned()],
            skipped: vec!["zellij".to_owned(), "fd".to_owned()],
        };
        let serialized = toml::to_string(&entry).expect("serialize");
        let parsed: ReconciliationEntry = toml::from_str(&serialized).expect("parse");
        assert_eq!(parsed, entry);
    }

    /// `--adopt-all` runs serialize the full adopted set with an empty
    /// skipped vector; the empty vector still round-trips.
    #[test]
    fn adopt_all_with_empty_skipped_round_trips() {
        let entry = ReconciliationEntry {
            plan_id: Uuid::nil(),
            committed_at: epoch(),
            action: ReconciliationAction::AdoptAll,
            package_count: 2,
            adopted: vec!["htop".to_owned(), "ripgrep".to_owned()],
            skipped: Vec::new(),
        };
        let serialized = toml::to_string(&entry).expect("serialize");
        let parsed: ReconciliationEntry = toml::from_str(&serialized).expect("parse");
        assert_eq!(parsed, entry);
        assert!(parsed.skipped.is_empty());
    }
}
