// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `state.toml` schema-version migrations.
//!
//! Each migration is a pure transformation `State -> State` that bumps
//! `schema_version` by one. [`migrate`] composes them in order so a v0
//! file becomes vN through (N) function applications.
//!
//! At M1, only the v0 → v1 migration exists; it adds the
//! `schema_version` field that pre-versioned alpha files lacked.

use crate::errors::StateError;
use crate::schema::{SCHEMA_VERSION, State};

/// Migrate a freshly-deserialized [`State`] up to [`SCHEMA_VERSION`].
///
/// # Errors
/// Returns [`StateError::UnsupportedSchemaVersion`] when the on-disk
/// file claims a `schema_version` higher than this build supports.
pub fn migrate(mut state: State) -> Result<State, StateError> {
    if state.schema_version == 0 {
        state = migrate_v0_to_v1(state);
    }
    if state.schema_version > SCHEMA_VERSION {
        return Err(StateError::UnsupportedSchemaVersion {
            found: state.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(state)
}

/// v0 → v1: the only change is that `schema_version` becomes mandatory
/// at the type level (it was implicitly `0` before).
fn migrate_v0_to_v1(mut state: State) -> State {
    state.schema_version = 1;
    state
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
    use std::path::PathBuf;

    fn empty_v0_state() -> State {
        State {
            schema_version: 0,
            host: "forge".to_owned(),
            tool_version: "0.1.0-alpha".to_owned(),
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

    #[test]
    fn v0_to_v1_adds_schema_version() {
        let v0 = empty_v0_state();
        let v1 = migrate(v0.clone()).expect("migrate");
        assert_eq!(v1.schema_version, 1);
        // Every other field unchanged.
        assert_eq!(v1.host, v0.host);
        assert_eq!(v1.tool_version, v0.tool_version);
        assert_eq!(v1.config_dir, v0.config_dir);
    }

    #[test]
    fn current_version_is_a_no_op() {
        let mut current = empty_v0_state();
        current.schema_version = SCHEMA_VERSION;
        let after = migrate(current.clone()).expect("migrate");
        assert_eq!(after, current);
    }

    #[test]
    fn future_version_rejected() {
        let mut future = empty_v0_state();
        future.schema_version = SCHEMA_VERSION + 1;
        let err = migrate(future).expect_err("future version must fail");
        assert!(
            matches!(err, StateError::UnsupportedSchemaVersion { .. }),
            "got {err:?}"
        );
    }
}
