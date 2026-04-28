// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Explicit rollback of an applied [`Plan`](pearlite_diff::Plan).
//!
//! [`Engine::rollback`] is the user-driven counterpart to
//! [`Engine::apply_plan`](crate::Engine::apply_plan). PRD §8.5
//! Class 4/5 failures and CLAUDE.md hard invariant 9 are explicit:
//! Pearlite never rolls back automatically. The operator runs
//! `pearlite rollback <plan-id>`, which lands here.
//!
//! Behaviour:
//!
//! 1. Read `state.toml`.
//! 2. Look up the [`HistoryEntry`](pearlite_state::HistoryEntry) with
//!    the requested `plan_id` (or [`RollbackError::PlanNotFound`]).
//! 3. Dispatch `snapper.rollback(config, entry.snapshot_pre.id)`.
//!
//! `state.toml` is **not** rewritten by rollback. The snapper revert
//! restores the entire root subvolume — including `state.toml` — to its
//! pre-apply contents. The next `pearlite plan` re-derives from the
//! live system; no resume / replay machinery is involved
//! (CLAUDE.md hard invariant 8).

use crate::errors::RollbackError;
use crate::plan::Engine;
use pearlite_snapper::Snapper;
use pearlite_state::{SnapshotRef, StateStore};
use std::path::Path;
use uuid::Uuid;

/// Outcome of a successful [`Engine::rollback`] run.
#[derive(Clone, Debug)]
pub struct RollbackOutcome {
    /// Plan UUID that was rolled back.
    pub plan_id: Uuid,
    /// Generation number of the rolled-back history entry.
    pub generation: u64,
    /// Pre-apply snapshot the system was reverted to.
    pub snapshot_pre: SnapshotRef,
}

impl Engine {
    /// Roll back to the pre-apply Snapper snapshot of a previously
    /// applied plan.
    ///
    /// `plan_id` must refer to a [`HistoryEntry`](pearlite_state::HistoryEntry)
    /// in `state.toml`'s `[[history]]`. The engine extracts the entry's
    /// `snapshot_pre.id` and dispatches it to `snapper.rollback`.
    ///
    /// `snapper_config` is the snapper config the original apply
    /// recorded its snapshots under (typically `"root"`). `state_path`
    /// is the absolute path to `state.toml`.
    ///
    /// # Errors
    /// - [`RollbackError::State`] — `state.toml` could not be read.
    /// - [`RollbackError::PlanNotFound`] — no `[[history]]` entry with
    ///   that `plan_id`.
    /// - [`RollbackError::Snapper`] — the snapper revert itself failed.
    pub fn rollback(
        &self,
        plan_id: Uuid,
        snapper: &dyn Snapper,
        snapper_config: &str,
        state_path: &Path,
    ) -> Result<RollbackOutcome, RollbackError> {
        let store = StateStore::new(state_path.to_path_buf());
        let state = store.read()?;

        let entry = state
            .history
            .iter()
            .find(|h| h.plan_id == plan_id)
            .ok_or(RollbackError::PlanNotFound { plan_id })?;

        snapper.rollback(snapper_config, entry.snapshot_pre.id)?;

        Ok(RollbackOutcome {
            plan_id,
            generation: entry.generation,
            snapshot_pre: entry.snapshot_pre.clone(),
        })
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
    use pearlite_nickel::MockNickel;
    use pearlite_schema::{HostInfo, KernelInfo, ProbedState};
    use pearlite_snapper::{MockSnapper, SnapperError, SnapshotInfo};
    use pearlite_state::{HistoryEntry, SCHEMA_VERSION, SnapshotRef, State, StateStore};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    fn epoch() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts")
    }

    fn engine() -> Engine {
        let probed = ProbedState {
            probed_at: epoch(),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: None,
            cargo: None,
            config_files: None,
            services: None,
            kernel: KernelInfo::default(),
        };
        Engine::new(
            Box::new(MockNickel::new()),
            Box::new(crate::mock_probe::MockProbe::with_state(probed)),
            PathBuf::from("/cfg-repo"),
        )
    }

    fn snapshot_ref(id: u64, label: &str) -> SnapshotRef {
        SnapshotRef {
            id,
            label: label.to_owned(),
            created_at: epoch(),
        }
    }

    fn history_entry(plan_id: Uuid, generation: u64, pre_id: u64) -> HistoryEntry {
        HistoryEntry {
            plan_id,
            generation,
            applied_at: epoch(),
            duration_ms: 0,
            snapshot_pre: snapshot_ref(pre_id, &format!("pre-pearlite-apply-{generation:08x}")),
            snapshot_post: snapshot_ref(
                pre_id + 1,
                &format!("post-pearlite-apply-{generation:08x}"),
            ),
            actions_executed: 0,
            git_revision: None,
            git_dirty: false,
            summary: String::new(),
        }
    }

    fn write_state_with(history: Vec<HistoryEntry>, dir: &TempDir) -> PathBuf {
        let path = dir.path().join("state.toml");
        let store = StateStore::new(path.clone());
        let state = State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed::default(),
            adopted: pearlite_state::Adopted::default(),
            history,
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: BTreeMap::new(),
        };
        store.write_atomic(&state).expect("write state");
        path
    }

    /// Pre-seed a [`MockSnapper`] with `count` snapshots so its
    /// monotonic ID counter is past whatever IDs the test passes in.
    fn snapper_with_n_snapshots(count: u64) -> MockSnapper {
        let snapper = MockSnapper::new();
        for i in 0..count {
            snapper
                .create("root", &format!("seed-{i}"))
                .expect("seed snapshot");
        }
        snapper
    }

    #[test]
    fn rollback_dispatches_pre_snapshot_id_to_snapper() {
        let dir = TempDir::new().expect("tempdir");
        let plan_id = Uuid::now_v7();
        let state_path = write_state_with(vec![history_entry(plan_id, 1, 42)], &dir);
        let snapper = snapper_with_n_snapshots(50);

        let out = engine()
            .rollback(plan_id, &snapper, "root", &state_path)
            .expect("rollback");

        assert_eq!(out.plan_id, plan_id);
        assert_eq!(out.generation, 1);
        assert_eq!(out.snapshot_pre.id, 42);
        let history = snapper.rollback_history();
        assert_eq!(history, vec![("root".to_owned(), 42)]);
    }

    #[test]
    fn rollback_unknown_plan_id_yields_plan_not_found() {
        let dir = TempDir::new().expect("tempdir");
        let known = Uuid::now_v7();
        let unknown = Uuid::now_v7();
        let state_path = write_state_with(vec![history_entry(known, 1, 10)], &dir);
        let snapper = MockSnapper::new();

        let err = engine()
            .rollback(unknown, &snapper, "root", &state_path)
            .expect_err("must fail");
        assert!(
            matches!(err, RollbackError::PlanNotFound { plan_id } if plan_id == unknown),
            "got {err:?}"
        );
        assert!(
            snapper.rollback_history().is_empty(),
            "snapper.rollback must not be invoked when plan_id is unknown"
        );
    }

    #[test]
    fn rollback_picks_correct_entry_when_history_has_multiple_generations() {
        let dir = TempDir::new().expect("tempdir");
        let p1 = Uuid::now_v7();
        let p2 = Uuid::now_v7();
        let p3 = Uuid::now_v7();
        let state_path = write_state_with(
            vec![
                history_entry(p1, 1, 10),
                history_entry(p2, 2, 20),
                history_entry(p3, 3, 30),
            ],
            &dir,
        );
        let snapper = snapper_with_n_snapshots(50);

        let out = engine()
            .rollback(p2, &snapper, "root", &state_path)
            .expect("rollback");
        assert_eq!(out.generation, 2);
        assert_eq!(out.snapshot_pre.id, 20);
        assert_eq!(snapper.rollback_history(), vec![("root".to_owned(), 20)]);
    }

    #[test]
    fn rollback_propagates_snapper_failure() {
        struct FailingSnapper;
        impl Snapper for FailingSnapper {
            fn create(&self, _: &str, _: &str) -> Result<SnapshotInfo, SnapperError> {
                Ok(SnapshotInfo {
                    id: 0,
                    label: String::new(),
                    created_at: OffsetDateTime::from_unix_timestamp(0).expect("ts"),
                    config: String::new(),
                })
            }
            fn rollback(&self, _: &str, _: u64) -> Result<(), SnapperError> {
                Err(SnapperError::NotInPath { hint: "test" })
            }
            fn list(&self, _: &str) -> Result<Vec<SnapshotInfo>, SnapperError> {
                Ok(Vec::new())
            }
        }

        let dir = TempDir::new().expect("tempdir");
        let plan_id = Uuid::now_v7();
        let state_path = write_state_with(vec![history_entry(plan_id, 1, 5)], &dir);

        let err = engine()
            .rollback(plan_id, &FailingSnapper, "root", &state_path)
            .expect_err("must fail");
        assert!(matches!(err, RollbackError::Snapper(_)), "got {err:?}");
    }

    #[test]
    fn rollback_missing_state_toml_yields_state_error() {
        let dir = TempDir::new().expect("tempdir");
        // Don't write a state.toml; rollback must surface NotFound,
        // not silently create one.
        let state_path = dir.path().join("state.toml");
        let snapper = MockSnapper::new();

        let err = engine()
            .rollback(Uuid::now_v7(), &snapper, "root", &state_path)
            .expect_err("must fail");
        assert!(matches!(err, RollbackError::State(_)), "got {err:?}");
    }

    #[test]
    fn rollback_does_not_mutate_state_toml() {
        let dir = TempDir::new().expect("tempdir");
        let plan_id = Uuid::now_v7();
        let state_path = write_state_with(vec![history_entry(plan_id, 1, 7)], &dir);
        let snapper = snapper_with_n_snapshots(20);

        let before = std::fs::read(&state_path).expect("read before");
        engine()
            .rollback(plan_id, &snapper, "root", &state_path)
            .expect("rollback");
        let after = std::fs::read(&state_path).expect("read after");

        assert_eq!(
            before, after,
            "rollback must leave state.toml byte-identical; snapper revert restores it"
        );
    }
}
