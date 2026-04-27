// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite orchestrator: ties schema, state, diff, and adapter crates
//! together.
//!
//! At M1 only the read-only [`Engine::plan`] path is implemented.
//! Apply, rollback, and reconcile arrive in M2+ per Plan §7.

mod errors;
mod plan;
mod probe;

pub use errors::{EngineError, ProbeError};
pub use plan::Engine;
pub use probe::{LiveProbe, SystemProbe};

#[cfg(any(test, feature = "test-mocks"))]
mod mock_probe;

#[cfg(feature = "test-mocks")]
pub use mock_probe::MockProbe;
