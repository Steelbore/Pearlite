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

/// One contract violation produced by [`validate`](crate::validate).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractViolation {
    /// Stable contract identifier (e.g. `DUPLICATE_PACKAGES`).
    pub id: &'static str,
    /// Human-readable explanation of the violation.
    pub message: String,
}

impl ContractViolation {
    /// A package name appears in more than one declared list.
    pub const DUPLICATE_PACKAGES: &'static str = "DUPLICATE_PACKAGES";
    /// A kernel module appears more than once, or appears in both
    /// `kernel.modules` and `kernel.blacklist`.
    pub const KERNEL_MODULES_NOT_UNIQUE: &'static str = "KERNEL_MODULES_NOT_UNIQUE";
    /// `meta.arch_level` does not match the declared per-feature-level
    /// repository: e.g. `arch_level = "v3"` with non-empty `packages.cachyos-v4`.
    pub const ARCH_LEVEL_MISMATCH: &'static str = "ARCH_LEVEL_MISMATCH";
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id, self.message)
    }
}
