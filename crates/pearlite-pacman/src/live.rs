// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Pacman`] trait + production [`LivePacman`] implementation.

use crate::errors::PacmanError;
use crate::inventory::compose_inventory;
use pearlite_schema::PacmanInventory;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Trait the rest of the workspace consumes to talk to pacman/paru.
///
/// At M1 only [`inventory`](Self::inventory) is implemented. Install,
/// remove, and `sync_databases` arrive with M2's apply-engine wiring.
pub trait Pacman: Send + Sync {
    /// Snapshot the explicit + foreign + per-package-repo state into a
    /// [`PacmanInventory`].
    ///
    /// # Errors
    /// Implementations propagate adapter-specific failures via
    /// [`PacmanError`].
    fn inventory(&self) -> Result<PacmanInventory, PacmanError>;
}

/// Production [`Pacman`] backed by the `pacman` binary.
///
/// Uses three subprocess invocations to assemble an inventory:
/// `pacman -Qe`, `pacman -Qm`, `pacman -Sl`. AUR install/remove
/// (M2) routes through `paru` via the configured `paru_path`.
#[derive(Clone, Debug)]
pub struct LivePacman {
    pacman_path: PathBuf,
    /// Reserved for M2's install/remove wiring.
    #[allow(dead_code)]
    paru_path: PathBuf,
}

impl LivePacman {
    /// Construct a [`LivePacman`] that resolves both binaries from
    /// `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pacman_path: PathBuf::from("pacman"),
            paru_path: PathBuf::from("paru"),
        }
    }

    /// Construct a [`LivePacman`] with caller-supplied binary paths.
    pub fn with_paths(pacman_path: impl Into<PathBuf>, paru_path: impl Into<PathBuf>) -> Self {
        Self {
            pacman_path: pacman_path.into(),
            paru_path: paru_path.into(),
        }
    }

    /// Path of the `pacman` binary this adapter invokes.
    #[must_use]
    pub fn pacman_path(&self) -> &Path {
        &self.pacman_path
    }

    fn run_pacman(&self, args: &[&str]) -> Result<String, PacmanError> {
        let output = match Command::new(&self.pacman_path).args(args).output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PacmanError::NotInPath {
                    tool: "pacman",
                    hint: "every CachyOS host has pacman; this is unexpected",
                });
            }
            Err(e) => {
                return Err(PacmanError::Io {
                    tool: "pacman",
                    source: e,
                });
            }
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(PacmanError::InvocationFailed {
                tool: "pacman",
                code,
                stderr,
            });
        }

        String::from_utf8(output.stdout).map_err(|e| PacmanError::NotUtf8 {
            tool: "pacman",
            source: e,
        })
    }
}

impl Default for LivePacman {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacman for LivePacman {
    fn inventory(&self) -> Result<PacmanInventory, PacmanError> {
        let qe = self.run_pacman(&["-Qe"])?;
        let qm = self.run_pacman(&["-Qm"])?;
        let sl = self.run_pacman(&["-Sl"])?;
        Ok(compose_inventory(&qe, &qm, &sl))
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
    fn pacman_not_in_path_error_class() {
        let live = LivePacman::with_paths("/nonexistent/pacman-1234", "/nonexistent/paru-1234");
        let err = live.inventory().expect_err("must fail");
        assert!(
            matches!(err, PacmanError::NotInPath { tool: "pacman", .. }),
            "got {err:?}"
        );
    }
}
