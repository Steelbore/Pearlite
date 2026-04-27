// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-snapper`.

use thiserror::Error;

/// Errors emitted while invoking `snapper` or parsing its output.
#[derive(Debug, Error)]
pub enum SnapperError {
    /// The configured `snapper` binary could not be spawned.
    #[error("snapper binary not found: {hint}")]
    NotInPath {
        /// Hint string with a runnable command.
        hint: &'static str,
    },
    /// `std::io` error during spawn or stdout capture.
    #[error("I/O error invoking snapper: {0}")]
    Io(#[from] std::io::Error),
    /// `snapper` exited non-zero.
    #[error("snapper exited with code {code}:\n{stderr}")]
    InvocationFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr verbatim.
        stderr: String,
    },
    /// `snapper` stdout was not valid UTF-8.
    #[error("snapper stdout was not valid UTF-8: {0}")]
    NotUtf8(#[from] std::string::FromUtf8Error),
    /// The snapshot ID returned by `snapper create` couldn't be parsed.
    #[error("could not parse snapshot id from snapper output: {0}")]
    ParseFailed(String),
}
