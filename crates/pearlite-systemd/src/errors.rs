// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-systemd`.

use thiserror::Error;

/// Errors emitted while invoking `systemctl` or parsing its output.
#[derive(Debug, Error)]
pub enum SystemdError {
    /// The configured `systemctl` binary could not be spawned. Usually
    /// a Class 1 preflight failure: every supported host has systemd.
    #[error("systemctl binary not found: {hint}")]
    NotInPath {
        /// Hint string with a runnable command.
        hint: &'static str,
    },
    /// `std::io` error during spawn or stdout capture.
    #[error("I/O error invoking systemctl: {0}")]
    Io(#[from] std::io::Error),
    /// `systemctl` exited non-zero.
    #[error("systemctl exited with code {code}:\n{stderr}")]
    InvocationFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr verbatim.
        stderr: String,
    },
    /// `systemctl` stdout was not valid UTF-8.
    #[error("systemctl stdout was not valid UTF-8: {0}")]
    NotUtf8(#[from] std::string::FromUtf8Error),
}
