// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite orchestrator: ties schema, state, diff, and adapter crates
//! together.
//!
//! At M1 only the read-only [`Engine::plan`] path is implemented.
//! Apply, rollback, and reconcile arrive in M2+ per Plan §7.

mod apply;
mod errors;
mod plan;
mod probe;
mod rollback;

pub use apply::{ApplyOutcome, FailureRecord};
pub use errors::{ApplyError, EngineError, ProbeError, RollbackError};
pub use plan::Engine;
pub use probe::{LiveProbe, SystemProbe};
pub use rollback::RollbackOutcome;

#[cfg(any(test, feature = "test-mocks"))]
mod mock_probe;

#[cfg(feature = "test-mocks")]
pub use mock_probe::MockProbe;
