// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Snapper btrfs-snapshot adapter: create, list, and rollback.
//!
//! This is the last new adapter crate before M2 W2's apply-engine
//! orchestrator can wrap every apply in pre/post Snapper snapshots
//! per PRD §11.1.

mod errors;
mod list;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;

pub use errors::SnapperError;
pub use list::parse_list;
pub use live::{LiveSnapper, Snapper, SnapshotInfo};

#[cfg(feature = "test-mocks")]
pub use mock::MockSnapper;
