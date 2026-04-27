// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Cargo`] trait + production [`LiveCargo`] implementation.

use crate::errors::CargoError;
use crate::inventory::parse_install_list;
use pearlite_schema::CargoInventory;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Trait the rest of the workspace consumes to talk to `cargo`.
///
/// At M1 only [`inventory`](Self::inventory) is implemented. Install and
/// uninstall arrive with M2's apply-engine wiring per Plan §7.3.
pub trait Cargo: Send + Sync {
    /// Snapshot the `cargo install --list` output as a
    /// [`CargoInventory`].
    ///
    /// # Errors
    /// Implementations propagate adapter-specific failures via
    /// [`CargoError`].
    fn inventory(&self) -> Result<CargoInventory, CargoError>;
}

/// Production [`Cargo`] backed by the `cargo` binary.
///
/// Uses argv-array subprocess invocation per CLAUDE.md hard invariant
/// #5: never `sh -c`, never string interpolation into a shell string.
#[derive(Clone, Debug)]
pub struct LiveCargo {
    binary: PathBuf,
}

impl LiveCargo {
    /// Construct a [`LiveCargo`] that resolves `cargo` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("cargo"),
        }
    }

    /// Construct a [`LiveCargo`] with a caller-supplied binary path.
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Path of the `cargo` binary this adapter invokes.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }
}

impl Default for LiveCargo {
    fn default() -> Self {
        Self::new()
    }
}

impl Cargo for LiveCargo {
    fn inventory(&self) -> Result<CargoInventory, CargoError> {
        let output = match Command::new(&self.binary)
            .args(["install", "--list"])
            .output()
        {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CargoError::NotInPath {
                    hint: "paru -S rustup",
                });
            }
            Err(e) => return Err(CargoError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(CargoError::InvocationFailed { code, stderr });
        }

        let stdout = String::from_utf8(output.stdout)?;
        Ok(parse_install_list(&stdout))
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests may use expect()/unwrap()/panic!() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;

    #[test]
    fn cargo_not_in_path_error_class() {
        let live = LiveCargo::with_binary("/nonexistent/path/cargo-binary-12345");
        let err = live.inventory().expect_err("must fail");
        assert!(matches!(err, CargoError::NotInPath { .. }), "got {err:?}");
    }

    /// Plan §6.7 acceptance: `cargo install --list` parses correctly. CI
    /// has cargo installed via dtolnay/rust-toolchain; locally cargo is
    /// always present (we're inside a Rust workspace). The test asserts
    /// the call succeeds; whatever crates happen to be installed don't
    /// matter for parser coverage.
    #[test]
    fn live_cargo_inventory_succeeds_in_a_rust_environment() {
        let live = LiveCargo::new();
        let probe = Command::new(live.binary()).arg("--version").output();
        if !matches!(&probe, Ok(o) if o.status.success()) {
            return;
        }
        let inv = live.inventory().expect("inventory");
        // Every crate name is a valid cargo package name (kebab-case
        // alphanumerics + a few extras). Defensive sanity check.
        for name in inv.crates.keys() {
            assert!(!name.is_empty(), "empty crate name in inventory");
            assert!(!name.contains(' '), "crate name has whitespace: {name}");
        }
    }
}
