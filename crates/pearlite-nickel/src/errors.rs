// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-nickel`.

use thiserror::Error;

/// Errors emitted while spawning, evaluating, or parsing Nickel output.
#[derive(Debug, Error)]
pub enum NickelError {
    /// The configured `nickel` binary could not be spawned. Usually
    /// resolves to a Class 1 preflight failure at the engine level.
    #[error("nickel binary not found: {hint}")]
    NotInPath {
        /// Hint string with a runnable command (`pacman -S nickel-lang`).
        hint: &'static str,
    },
    /// `std::io` error during spawn or stdout capture.
    #[error("I/O error invoking nickel: {0}")]
    Io(#[from] std::io::Error),
    /// The `nickel` process exited non-zero. `stderr` carries the
    /// structured diagnostic verbatim per Plan §6.5.
    #[error("nickel exited with code {code}:\n{stderr}")]
    EvaluationFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr.
        stderr: String,
    },
    /// `nickel` stdout was not valid UTF-8.
    #[error("nickel stdout was not valid UTF-8: {0}")]
    NotUtf8(#[from] std::string::FromUtf8Error),
    /// The emitted TOML did not match the schema in `pearlite-schema`.
    #[error(transparent)]
    Schema(#[from] pearlite_schema::SchemaError),
    /// `MockNickel` was invoked with a path it had no canned output for.
    /// Test-only — never returned by [`LiveNickel`](crate::LiveNickel).
    #[error("MockNickel: no canned output for {0}")]
    MockMissing(std::path::PathBuf),
}
