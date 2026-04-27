// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Persistent Pearlite state: atomic read/write, migrations, history.
//!
//! [`State`] is the in-memory mirror of `/var/lib/pearlite/state.toml` —
//! the load-bearing artifact in PRD §7. The `read` / `write_atomic` /
//! `append_*` operations land in chunk M1-W1-E; this scaffold defines the
//! types and errors only.

mod errors;
mod failure;
mod history;
mod io;
mod migrate;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;
mod reconciliation;
mod schema;
mod store;

pub use errors::StateError;
pub use failure::FailureRef;
pub use history::{HistoryEntry, SnapshotRef};
pub use io::{FileSystem, LiveFs};
pub use reconciliation::{ReconciliationAction, ReconciliationEntry};
pub use schema::{
    Adopted, ConfigFileRecord, KernelRecord, Managed, SCHEMA_VERSION, ServicesState, State,
    UserEnvRecord,
};
pub use store::StateStore;

pub use migrate::migrate;

#[cfg(feature = "test-mocks")]
pub use mock::MockFs;
