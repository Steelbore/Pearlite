// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-pacman`.

use thiserror::Error;

/// Errors emitted while invoking pacman/paru or parsing their output.
#[derive(Debug, Error)]
pub enum PacmanError {
    /// The configured `pacman` or `paru` binary could not be spawned.
    /// Always a Class 1 preflight failure: every CachyOS host has both.
    #[error("{tool} binary not found: {hint}")]
    NotInPath {
        /// Which tool was missing (`pacman` or `paru`).
        tool: &'static str,
        /// Hint string with a runnable command.
        hint: &'static str,
    },
    /// `std::io` error during spawn or stdout capture.
    #[error("I/O error invoking {tool}: {source}")]
    Io {
        /// Tool that was being invoked.
        tool: &'static str,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// `pacman` or `paru` exited non-zero.
    #[error("{tool} exited with code {code}:\n{stderr}")]
    InvocationFailed {
        /// Tool whose invocation failed.
        tool: &'static str,
        /// Process exit code.
        code: i32,
        /// Captured stderr verbatim.
        stderr: String,
    },
    /// Tool stdout was not valid UTF-8.
    #[error("{tool} stdout was not valid UTF-8: {source}")]
    NotUtf8 {
        /// Tool whose output was malformed.
        tool: &'static str,
        /// Underlying decode error.
        #[source]
        source: std::string::FromUtf8Error,
    },
}
