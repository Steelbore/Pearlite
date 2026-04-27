// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`StateStore`] — read, write, and append helpers for `state.toml`.

use crate::errors::StateError;
use crate::failure::FailureRef;
use crate::history::HistoryEntry;
use crate::io::{FileSystem, LiveFs};
use crate::reconciliation::ReconciliationEntry;
use crate::schema::{SCHEMA_VERSION, State};
use std::path::{Path, PathBuf};

/// Read/write coordinator for a `state.toml` at a fixed path.
///
/// `StateStore` is generic over the filesystem so tests can swap in a
/// `MockFs` (chunk M1-W1-F). Production binaries use the default
/// [`LiveFs`].
#[derive(Clone, Debug)]
pub struct StateStore<F: FileSystem = LiveFs> {
    fs: F,
    path: PathBuf,
}

impl StateStore<LiveFs> {
    /// Construct a [`StateStore`] backed by [`LiveFs`].
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { fs: LiveFs, path }
    }
}

impl<F: FileSystem> StateStore<F> {
    /// Construct a [`StateStore`] with a caller-supplied filesystem.
    pub fn with_fs(fs: F, path: PathBuf) -> Self {
        Self { fs, path }
    }

    /// Path of the `state.toml` this store reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the current state from disk.
    ///
    /// # Errors
    /// - [`StateError::NotFound`] if the file does not exist.
    /// - [`StateError::Io`] on any other read failure.
    /// - [`StateError::InvalidToml`] on parse failure.
    /// - [`StateError::UnsupportedSchemaVersion`] if `schema_version`
    ///   is greater than this build understands.
    pub fn read(&self) -> Result<State, StateError> {
        let content = match self.fs.read_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StateError::NotFound(self.path.clone()));
            }
            Err(e) => return Err(StateError::Io(e)),
        };
        let state: State = toml::from_str(&content)?;
        if state.schema_version > SCHEMA_VERSION {
            return Err(StateError::UnsupportedSchemaVersion {
                found: state.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        Ok(state)
    }

    /// Atomically write `state` to disk per PRD §7.4: temp + fsync +
    /// rename + dir-fsync.
    ///
    /// # Errors
    /// - [`StateError::SerializeFailed`] if `state` cannot be serialized
    ///   (should be unreachable for well-formed values).
    /// - [`StateError::Io`] on any filesystem error.
    pub fn write_atomic(&self, state: &State) -> Result<(), StateError> {
        let serialized = toml::to_string_pretty(state)?;
        self.fs
            .write_temp_then_rename(&self.path, serialized.as_bytes())?;
        self.fs.fsync_dir(&self.path)?;
        Ok(())
    }

    /// Append a new [`HistoryEntry`] to the state file. Re-reads the
    /// current state, pushes the entry, and writes atomically.
    ///
    /// `[managed]`, `[adopted]`, and `[reserved]` sections are unchanged
    /// in value (the file is rewritten in full but other sections retain
    /// their semantic content).
    ///
    /// # Errors
    /// Same set as [`Self::read`] and [`Self::write_atomic`].
    pub fn append_history(&self, entry: HistoryEntry) -> Result<(), StateError> {
        let mut state = self.read()?;
        state.history.push(entry);
        self.write_atomic(&state)
    }

    /// Append a new [`FailureRef`] to the state file.
    ///
    /// # Errors
    /// Same set as [`Self::read`] and [`Self::write_atomic`].
    pub fn append_failure(&self, entry: FailureRef) -> Result<(), StateError> {
        let mut state = self.read()?;
        state.failures.push(entry);
        self.write_atomic(&state)
    }

    /// Record a [`ReconciliationEntry`] against the state file.
    ///
    /// # Errors
    /// Same set as [`Self::read`] and [`Self::write_atomic`].
    pub fn record_reconciliation(&self, entry: ReconciliationEntry) -> Result<(), StateError> {
        let mut state = self.read()?;
        state.reconciliations.push(entry);
        self.write_atomic(&state)
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
    use crate::history::SnapshotRef;
    use crate::reconciliation::ReconciliationAction;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn store_in(dir: &TempDir) -> StateStore<LiveFs> {
        StateStore::new(dir.path().join("state.toml"))
    }

    fn empty_state() -> State {
        State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/home/mohamed/pearlite-config"),
            last_apply: None,
            last_modified: None,
            managed: crate::schema::Managed::default(),
            adopted: crate::schema::Adopted::default(),
            history: Vec::new(),
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: std::collections::BTreeMap::new(),
        }
    }

    fn epoch() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts")
    }

    fn snapshot_ref(id: u64) -> SnapshotRef {
        SnapshotRef {
            id,
            label: format!("pearlite-{id}"),
            created_at: epoch(),
        }
    }

    fn history_entry(generation: u64) -> HistoryEntry {
        HistoryEntry {
            plan_id: Uuid::nil(),
            generation,
            applied_at: epoch(),
            duration_ms: 1234,
            snapshot_pre: snapshot_ref(generation * 10),
            snapshot_post: snapshot_ref(generation * 10 + 1),
            actions_executed: 7,
            git_revision: Some("abc1234".to_owned()),
            git_dirty: false,
            summary: "+1".to_owned(),
        }
    }

    #[test]
    fn full_state_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);

        let mut original = empty_state();
        original.last_apply = Some(epoch());
        original.last_modified = Some(epoch());
        original.managed.pacman = vec!["htop".to_owned(), "vim".to_owned()];
        original.adopted.cargo = vec!["zellij".to_owned()];
        original.history.push(history_entry(1));

        store.write_atomic(&original).expect("write");
        let reparsed = store.read().expect("read");
        assert_eq!(original, reparsed);
    }

    #[test]
    fn unknown_fields_preserved_in_reserved() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);

        let mut state = empty_state();
        state
            .reserved
            .insert("future_v2_field".to_owned(), toml::Value::Integer(42));
        state.reserved.insert(
            "future_string".to_owned(),
            toml::Value::String("survives".to_owned()),
        );

        store.write_atomic(&state).expect("write");
        let reparsed = store.read().expect("read");
        assert_eq!(state.reserved, reparsed.reserved);
    }

    #[test]
    fn read_missing_file_yields_not_found() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);
        let err = store.read().expect_err("missing file must fail");
        assert!(matches!(err, StateError::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn read_unsupported_schema_version() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);
        let mut future = empty_state();
        future.schema_version = SCHEMA_VERSION + 100;
        // Bypass the version guard on write by writing the raw TOML.
        let raw = toml::to_string_pretty(&future).expect("serialize");
        std::fs::write(store.path(), raw).expect("write raw");

        let err = store.read().expect_err("future schema must fail");
        assert!(
            matches!(err, StateError::UnsupportedSchemaVersion { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn write_fails_when_parent_missing() {
        let store = StateStore::new(PathBuf::from("/var/no/such/dir/under/this/path/state.toml"));
        let err = store.write_atomic(&empty_state()).expect_err("must fail");
        assert!(matches!(err, StateError::Io(_)), "got {err:?}");
    }

    #[test]
    fn history_append_does_not_rewrite_managed() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);

        let mut state = empty_state();
        state.managed.pacman = vec!["htop".to_owned()];
        state.adopted.cargo = vec!["zellij".to_owned()];
        state.history.push(history_entry(1));
        store.write_atomic(&state).expect("write base");

        let baseline = store.read().expect("read baseline");
        store.append_history(history_entry(2)).expect("append");
        let after = store.read().expect("read after");

        assert_eq!(after.managed, baseline.managed, "managed must be unchanged");
        assert_eq!(after.adopted, baseline.adopted, "adopted must be unchanged");
        assert_eq!(
            after.reserved, baseline.reserved,
            "reserved must be unchanged"
        );
        assert_eq!(after.history.len(), 2);
        assert_eq!(after.history[0].generation, 1);
        assert_eq!(after.history[1].generation, 2);
    }

    #[test]
    fn append_failure_grows_failures_array() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);
        store.write_atomic(&empty_state()).expect("write base");

        let entry = FailureRef {
            plan_id: Uuid::nil(),
            failed_at: epoch(),
            class: 4,
            exit_code: 5,
            record_path: PathBuf::from("/var/lib/pearlite/failures/x.json"),
        };
        store.append_failure(entry.clone()).expect("append");

        let after = store.read().expect("read");
        assert_eq!(after.failures.len(), 1);
        assert_eq!(after.failures[0], entry);
    }

    #[test]
    fn record_reconciliation_appends_entry() {
        let dir = TempDir::new().expect("tempdir");
        let store = store_in(&dir);
        store.write_atomic(&empty_state()).expect("write base");

        let entry = ReconciliationEntry {
            plan_id: Uuid::nil(),
            committed_at: epoch(),
            action: ReconciliationAction::Interactive,
            package_count: 5,
        };
        store.record_reconciliation(entry.clone()).expect("append");

        let after = store.read().expect("read");
        assert_eq!(after.reconciliations.len(), 1);
        assert_eq!(after.reconciliations[0], entry);
    }
}
