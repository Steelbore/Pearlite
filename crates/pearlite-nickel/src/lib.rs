// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Nickel evaluator adapter: spawns `nickel export -f toml` and captures
//! the result as a resolved-TOML string.
//!
//! No Nickel parsing in Rust — Plan §6.5 hard rule. The adapter is the
//! thinnest possible shim: argv-array subprocess invocation + stdout
//! capture + delegation to [`pearlite_schema::from_resolved_toml`] for
//! the actual deserialisation.

mod emit;
mod errors;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;

pub use emit::emit_host;
pub use errors::NickelError;
pub use live::{LiveNickel, NickelEvaluator};

#[cfg(feature = "test-mocks")]
pub use mock::MockNickel;

use pearlite_schema::DeclaredState;
use std::path::Path;

/// Evaluate a Nickel host file via `eval` and parse the resulting TOML
/// into a [`DeclaredState`].
///
/// # Errors
/// - [`NickelError::NotInPath`] if the configured binary cannot be
///   spawned.
/// - [`NickelError::EvaluationFailed`] if `nickel` exits non-zero.
/// - [`NickelError::Schema`] if the emitted TOML does not match the
///   schema in [`pearlite_schema`].
pub fn load_host(
    host_file: &Path,
    eval: &dyn NickelEvaluator,
) -> Result<DeclaredState, NickelError> {
    let toml = eval.evaluate(host_file)?;
    Ok(pearlite_schema::from_resolved_toml(&toml)?)
}
