// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockSnapper`] for unit tests and engine integration.

#![allow(
    clippy::missing_panics_doc,
    reason = "test utility; the only panic is on Mutex poisoning, unreachable in sane tests"
)]

use crate::errors::SnapperError;
use crate::live::{Snapper, SnapshotInfo};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

#[allow(
    clippy::expect_used,
    reason = "MockSnapper is a test utility; mutex-poison panic is the standard \
              Mutex<T> idiom and unreachable in any sane test."
)]
mod inner {
    use super::SnapshotInfo;

    #[derive(Default, Debug)]
    pub struct State {
        pub next_id: u64,
        pub snapshots: Vec<SnapshotInfo>,
        pub rollbacks: Vec<(String, u64)>,
    }
}

use inner::State;

/// In-memory [`Snapper`]: stores synthesized snapshots in a Mutex.
///
/// `create` synthesizes monotonically-increasing IDs starting from 1.
/// `rollback` records the call but does not mutate the snapshot list.
/// `list` returns whatever has been created so far.
#[derive(Clone, Debug, Default)]
pub struct MockSnapper {
    state: Arc<Mutex<State>>,
}

impl MockSnapper {
    /// Construct an empty [`MockSnapper`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inspect the recorded rollback calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn rollback_history(&self) -> Vec<(String, u64)> {
        self.state
            .lock()
            .expect("MockSnapper mutex must not be poisoned")
            .rollbacks
            .clone()
    }
}

impl Snapper for MockSnapper {
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn create(&self, config: &str, label: &str) -> Result<SnapshotInfo, SnapperError> {
        let mut s = self
            .state
            .lock()
            .expect("MockSnapper mutex must not be poisoned");
        s.next_id += 1;
        let info = SnapshotInfo {
            id: s.next_id,
            label: label.to_owned(),
            created_at: OffsetDateTime::now_utc(),
            config: config.to_owned(),
        };
        s.snapshots.push(info.clone());
        Ok(info)
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn rollback(&self, config: &str, snapshot_id: u64) -> Result<(), SnapperError> {
        let mut s = self
            .state
            .lock()
            .expect("MockSnapper mutex must not be poisoned");
        s.rollbacks.push((config.to_owned(), snapshot_id));
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn list(&self, config: &str) -> Result<Vec<SnapshotInfo>, SnapperError> {
        let s = self
            .state
            .lock()
            .expect("MockSnapper mutex must not be poisoned");
        Ok(s.snapshots
            .iter()
            .filter(|i| i.config == config)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests may use expect()/unwrap()/panic!() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;

    #[test]
    fn create_returns_synthesized_id() {
        let mock = MockSnapper::new();
        let s1 = mock.create("root", "label-1").expect("create");
        let s2 = mock.create("root", "label-2").expect("create");
        assert_eq!(s1.id, 1);
        assert_eq!(s2.id, 2);
        assert_eq!(s1.label, "label-1");
        assert_eq!(s2.label, "label-2");
        assert_eq!(s1.config, "root");
    }

    #[test]
    fn list_returns_created_snapshots_for_matching_config() {
        let mock = MockSnapper::new();
        mock.create("root", "a").expect("c");
        mock.create("home", "b").expect("c");
        mock.create("root", "c").expect("c");

        let root = mock.list("root").expect("list");
        assert_eq!(root.len(), 2);
        let home = mock.list("home").expect("list");
        assert_eq!(home.len(), 1);
    }

    #[test]
    fn rollback_records_call() {
        let mock = MockSnapper::new();
        mock.rollback("root", 7).expect("rollback");
        mock.rollback("root", 5).expect("rollback");
        let history = mock.rollback_history();
        assert_eq!(
            history,
            vec![("root".to_owned(), 7), ("root".to_owned(), 5)]
        );
    }
}
