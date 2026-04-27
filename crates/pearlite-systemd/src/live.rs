// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Systemd`] trait + production [`LiveSystemd`] implementation.

use crate::errors::SystemdError;
use crate::inventory::compose_inventory;
use pearlite_schema::ServiceInventory;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Trait the rest of the workspace consumes to talk to systemd.
///
/// At M1 only [`inventory`](Self::inventory) is implemented. Enable /
/// disable / mask / restart arrive with M2's apply-engine wiring.
pub trait Systemd: Send + Sync {
    /// Snapshot the unit-file states + currently-active units into a
    /// [`ServiceInventory`].
    ///
    /// # Errors
    /// Implementations propagate adapter-specific failures via
    /// [`SystemdError`].
    fn inventory(&self) -> Result<ServiceInventory, SystemdError>;
}

/// Production [`Systemd`] backed by the `systemctl` binary.
#[derive(Clone, Debug)]
pub struct LiveSystemd {
    binary: PathBuf,
}

impl LiveSystemd {
    /// Construct a [`LiveSystemd`] that resolves `systemctl` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("systemctl"),
        }
    }

    /// Construct a [`LiveSystemd`] with a caller-supplied binary path.
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Path of the `systemctl` binary this adapter invokes.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    fn run(&self, args: &[&str]) -> Result<String, SystemdError> {
        let output = match Command::new(&self.binary).args(args).output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SystemdError::NotInPath {
                    hint: "every supported Pearlite host runs systemd; this is unusual",
                });
            }
            Err(e) => return Err(SystemdError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(SystemdError::InvocationFailed { code, stderr });
        }

        Ok(String::from_utf8(output.stdout)?)
    }
}

impl Default for LiveSystemd {
    fn default() -> Self {
        Self::new()
    }
}

impl Systemd for LiveSystemd {
    fn inventory(&self) -> Result<ServiceInventory, SystemdError> {
        let unit_files = self.run(&["list-unit-files", "--no-pager", "--no-legend"])?;
        let units = self.run(&[
            "list-units",
            "--no-pager",
            "--no-legend",
            "--all",
            "--plain",
        ])?;
        Ok(compose_inventory(&unit_files, &units))
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
    fn systemctl_not_in_path_error_class() {
        let live = LiveSystemd::with_binary("/nonexistent/path/systemctl-binary-12345");
        let err = live.inventory().expect_err("must fail");
        assert!(matches!(err, SystemdError::NotInPath { .. }), "got {err:?}");
    }

    /// Plan §6.8 acceptance: real systemctl invocation produces a
    /// well-formed inventory. CI runners are systemd-based; the test
    /// silent-skips on hosts without systemd.
    #[test]
    fn live_systemd_inventory_succeeds_when_systemctl_present() {
        let live = LiveSystemd::new();
        let probe = Command::new(live.binary()).arg("--version").output();
        if !matches!(&probe, Ok(o) if o.status.success()) {
            return;
        }
        let inv = live.inventory().expect("inventory");
        // Sanity: every unit name ends with a known suffix.
        for name in inv
            .enabled
            .iter()
            .chain(inv.disabled.iter())
            .chain(inv.masked.iter())
        {
            assert!(
                name.contains('.'),
                "unit name without a type suffix: {name}"
            );
        }
    }
}
