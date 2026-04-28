// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-userenv`.

use thiserror::Error;

/// Errors emitted while invoking `home-manager` (via `runuser`) or
/// parsing its output.
#[derive(Debug, Error)]
pub enum UserenvError {
    /// The configured `runuser` or `home-manager` binary could not be
    /// spawned. Carries a runnable hint.
    #[error("user-env binary not found: {hint}")]
    NotInPath {
        /// Hint string with a runnable command.
        hint: &'static str,
    },
    /// `std::io` error during spawn or stdout capture.
    #[error("I/O error invoking home-manager: {0}")]
    Io(#[from] std::io::Error),
    /// `home-manager` exited non-zero.
    #[error("home-manager exited with code {code}:\n{stderr}")]
    InvocationFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr verbatim.
        stderr: String,
    },
    /// `home-manager` stdout was not valid UTF-8.
    #[error("home-manager stdout was not valid UTF-8: {0}")]
    NotUtf8(#[from] std::string::FromUtf8Error),
    /// The generation number could not be parsed from
    /// `home-manager generations` / `switch` output.
    #[error("could not parse home-manager generation from output: {0}")]
    ParseFailed(String),
}
