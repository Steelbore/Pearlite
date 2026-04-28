// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Nix bootstrap declaration block.
//!
//! Modeled per ADR-0012 (`docs/adr/0012-nix-bootstrap-wiring.md`):
//! the per-host Nickel config carries the SHA-256 pin of the
//! Determinate Nix installer that `pearlite bootstrap` will execute
//! on first run. The pin lives next to the consumer (the host that
//! needs nix) rather than in a repo-wide bootstrap file or a
//! Pearlite-baked release constant.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-host nix bootstrap declaration.
///
/// Optional at the [`DeclaredState`](crate::DeclaredState) level: hosts
/// that don't run Home Manager don't need to declare it. When any user
/// on the host has `home_manager.enabled = true`, the schema validator
/// requires this block — see
/// [`ContractViolation::NIX_INSTALLER_REQUIRED`](crate::ContractViolation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NixDecl {
    /// Determinate Nix installer pin.
    pub installer: NixInstallerDecl,
}

/// SHA-256 pin for the Determinate Nix installer script.
///
/// ADR-004 requires the installer-fetch step to verify the script's
/// content hash before execution. The expected hash is operator-supplied
/// per host, refreshed when Determinate ships a new installer version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NixInstallerDecl {
    /// SHA-256 of the installer script body, as 64 lowercase hex
    /// characters. Verified by
    /// [`pearlite_userenv::NixInstaller::install`] before the script is
    /// executed.
    pub expected_sha256: String,
}
