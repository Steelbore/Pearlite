// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-cargo`.

use thiserror::Error;

/// Errors emitted while invoking the `cargo` binary or parsing its
/// output.
#[derive(Debug, Error)]
pub enum CargoError {
    /// The configured `cargo` binary could not be spawned. Usually a
    /// Class 1 preflight failure at the engine level — the user does
    /// not have `rustup` installed.
    #[error("cargo binary not found: {hint}")]
    NotInPath {
        /// Hint string with a runnable command (`paru -S rustup`).
        hint: &'static str,
    },
    /// `std::io` error during spawn or stdout capture.
    #[error("I/O error invoking cargo: {0}")]
    Io(#[from] std::io::Error),
    /// `cargo install --list` exited non-zero.
    #[error("cargo exited with code {code}:\n{stderr}")]
    InvocationFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr verbatim.
        stderr: String,
    },
    /// `cargo` stdout was not valid UTF-8.
    #[error("cargo stdout was not valid UTF-8: {0}")]
    NotUtf8(#[from] std::string::FromUtf8Error),
}
