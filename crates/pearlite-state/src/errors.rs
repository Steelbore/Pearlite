// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-state`.

use std::path::PathBuf;
use thiserror::Error;

/// Errors emitted while reading, writing, or migrating `state.toml`.
#[derive(Debug, Error)]
pub enum StateError {
    /// The TOML failed to parse.
    #[error("invalid TOML in state file: {0}")]
    InvalidToml(#[from] toml::de::Error),
    /// `state.toml` was not found at the expected path.
    #[error("state file not found: {0}")]
    NotFound(PathBuf),
    /// I/O error while reading or writing the file.
    #[error("I/O error on state file: {0}")]
    Io(#[from] std::io::Error),
    /// Unknown `schema_version` — the on-disk file is from a future
    /// Pearlite release and cannot be read by this build.
    #[error("unknown schema_version {found}; this build supports up to {supported}")]
    UnsupportedSchemaVersion {
        /// Version found in the file.
        found: u32,
        /// Highest version this build can read.
        supported: u32,
    },
    /// `state.host` does not match the system `hostname()`.
    #[error("state.host '{state_host}' does not match system hostname '{system_host}'")]
    HostMismatch {
        /// Host recorded in `state.toml`.
        state_host: String,
        /// Live `hostname(1)` value.
        system_host: String,
    },
    /// TOML serialisation failed (should be unreachable for well-formed
    /// `State` values).
    #[error("could not serialize state: {0}")]
    SerializeFailed(#[from] toml::ser::Error),
}
