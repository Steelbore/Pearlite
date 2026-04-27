// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-fs`.

use std::path::PathBuf;
use thiserror::Error;

/// Errors emitted by the filesystem primitives.
#[derive(Debug, Error)]
pub enum FsError {
    /// Underlying I/O error from `std::fs` or `std::io`.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// Path that was being operated on.
        path: PathBuf,
        /// Wrapped I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `nix` (libc) error, typically from chown.
    #[error("libc error on {path}: {source}")]
    Nix {
        /// Path the operation targeted.
        path: PathBuf,
        /// Wrapped errno.
        #[source]
        source: nix::Error,
    },
    /// Username or group name resolution failed.
    #[error("unknown {kind} '{name}'")]
    UnknownPrincipal {
        /// `"user"` or `"group"`.
        kind: &'static str,
        /// Name that failed to resolve.
        name: String,
    },
    /// Atomic write target had no parent directory (e.g. `"/"`).
    #[error("target {0} has no parent directory")]
    NoParent(PathBuf),
}
