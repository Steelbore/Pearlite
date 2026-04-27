// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-schema`.

use std::path::PathBuf;
use thiserror::Error;

/// Errors emitted while parsing a resolved-TOML host configuration.
#[derive(Debug, Error)]
pub enum SchemaError {
    /// The TOML failed to parse.
    #[error("invalid TOML: {0}")]
    InvalidToml(#[from] toml::de::Error),
    /// The host file referenced a path that did not exist.
    #[error("host file not found: {0}")]
    MissingHostFile(PathBuf),
}

/// One contract violation produced by `validate` (lands in chunk M1-W1-B).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractViolation {
    /// Stable contract identifier (e.g. `DUPLICATE_PACKAGES`).
    pub id: &'static str,
    /// Human-readable explanation of the violation.
    pub message: String,
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id, self.message)
    }
}
