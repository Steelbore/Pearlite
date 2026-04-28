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

/// Errors emitted while invoking the Determinate Nix installer.
///
/// Distinct from [`UserenvError`] because the install-or-skip
/// surface has its own failure modes (SHA-256 mismatch, the script
/// itself non-zero) that don't map cleanly onto Home Manager's.
#[derive(Debug, Error)]
pub enum InstallerError {
    /// `sh` (used to exec the installer) could not be spawned.
    #[error("installer host shell not found: {hint}")]
    ShellNotInPath {
        /// Hint string with a runnable command.
        hint: &'static str,
    },
    /// `std::io` error reading the installer script or spawning the
    /// shell.
    #[error("I/O error invoking installer: {0}")]
    Io(#[from] std::io::Error),
    /// The installer script's SHA-256 did not match the
    /// caller-supplied expected hash. ADR-004 §"hash-pinned": the
    /// installer is the *only* curl-piped script Pearlite tolerates,
    /// and the pin is what makes that tolerance defensible.
    #[error("installer SHA-256 mismatch: expected {expected}, found {actual} (refused to execute)")]
    Sha256Mismatch {
        /// Expected SHA-256 hex.
        expected: String,
        /// Actual SHA-256 hex computed from the script bytes.
        actual: String,
    },
    /// The installer ran but exited non-zero.
    #[error("installer exited with code {code}:\n{stderr}")]
    ScriptFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr verbatim.
        stderr: String,
    },
    /// Probing `nix --version` returned an unexpected error (i.e. not
    /// "binary missing" — that classifies as "needs install").
    #[error("could not probe nix presence: {0}")]
    NixProbeFailed(String),
}
