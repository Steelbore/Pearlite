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
/// One method per primitive operation the apply engine emits — each
/// matches a single [`Action`](pearlite_diff::Action) variant on the
/// pacman side: [`PacmanInstall`](pearlite_diff::Action::PacmanInstall),
/// [`PacmanRemove`](pearlite_diff::Action::PacmanRemove),
/// [`AurInstall`](pearlite_diff::Action::AurInstall). `sync_databases`
/// covers PRD §8.2 phase 0.5 (repo prep).
pub trait Pacman: Send + Sync {
    /// Snapshot the explicit + foreign + per-package-repo state into a
    /// [`PacmanInventory`].
    ///
    /// # Errors
    /// Implementations propagate adapter-specific failures via
    /// [`PacmanError`].
    fn inventory(&self) -> Result<PacmanInventory, PacmanError>;

    /// Refresh pacman's package databases (`pacman -Sy`).
    ///
    /// Phase 0.5 in PRD §8.2 — runs after `pacman.conf` writes so the
    /// install phase sees the host's declared repo set.
    ///
    /// # Errors
    /// Returns [`PacmanError`] on spawn / non-zero exit.
    fn sync_databases(&self) -> Result<(), PacmanError>;

    /// Install one or more packages from `repo` via pacman.
    ///
    /// Calls `pacman -S --noconfirm <repo>/<pkg> ...`; the qualified
    /// `<repo>/<pkg>` form pins the source so a name collision across
    /// `core` and `cachyos-v3` cannot silently pick the wrong build.
    /// An empty `packages` slice is a no-op.
    ///
    /// # Errors
    /// Returns [`PacmanError`] on spawn / non-zero exit.
    fn install(&self, repo: &str, packages: &[&str]) -> Result<(), PacmanError>;

    /// Install one or more AUR packages via paru.
    ///
    /// Calls `paru -S --noconfirm <pkg> ...`. paru must be configured
    /// to build without prompting (`--sudoloop` or a passwordless
    /// build user); Pearlite treats AUR builds as the operator's
    /// responsibility per PRD §11.4. An empty `packages` slice is a
    /// no-op.
    ///
    /// # Errors
    /// Returns [`PacmanError`] on spawn / non-zero exit.
    fn aur_install(&self, packages: &[&str]) -> Result<(), PacmanError>;

    /// Remove one or more pacman/AUR packages.
    ///
    /// Calls `pacman -R --noconfirm <pkg> ...`. Does not pass `-s`
    /// (orphan recursion) — the diff engine, not pacman, decides what
    /// counts as removable. An empty `packages` slice is a no-op.
    ///
    /// # Errors
    /// Returns [`PacmanError`] on spawn / non-zero exit.
    fn remove(&self, packages: &[&str]) -> Result<(), PacmanError>;
}

/// Production [`Pacman`] backed by the `pacman` binary.
///
/// Uses three subprocess invocations to assemble an inventory:
/// `pacman -Qe`, `pacman -Qm`, `pacman -Sl`. AUR install routes
/// through `paru` via the configured `paru_path`; pacman handles
/// install/remove/sync directly.
#[derive(Clone, Debug)]
pub struct LivePacman {
    pacman_path: PathBuf,
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

    /// Path of the `paru` binary this adapter invokes for AUR installs.
    #[must_use]
    pub fn paru_path(&self) -> &Path {
        &self.paru_path
    }

    fn run_pacman(&self, args: &[&str]) -> Result<String, PacmanError> {
        run_tool(
            &self.pacman_path,
            args,
            "pacman",
            "every CachyOS host has pacman; this is unexpected",
        )
    }

    fn run_paru(&self, args: &[&str]) -> Result<String, PacmanError> {
        run_tool(&self.paru_path, args, "paru", "pacman -S --noconfirm paru")
    }
}

fn run_tool(
    binary: &Path,
    args: &[&str],
    tool: &'static str,
    not_in_path_hint: &'static str,
) -> Result<String, PacmanError> {
    let output = match Command::new(binary).args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PacmanError::NotInPath {
                tool,
                hint: not_in_path_hint,
            });
        }
        Err(e) => return Err(PacmanError::Io { tool, source: e }),
    };

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(PacmanError::InvocationFailed { tool, code, stderr });
    }

    String::from_utf8(output.stdout).map_err(|e| PacmanError::NotUtf8 { tool, source: e })
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

    fn sync_databases(&self) -> Result<(), PacmanError> {
        self.run_pacman(&["-Sy", "--noconfirm"])?;
        Ok(())
    }

    fn install(&self, repo: &str, packages: &[&str]) -> Result<(), PacmanError> {
        if packages.is_empty() {
            return Ok(());
        }
        let qualified: Vec<String> = packages.iter().map(|p| format!("{repo}/{p}")).collect();
        let mut args: Vec<&str> = Vec::with_capacity(2 + qualified.len());
        args.push("-S");
        args.push("--noconfirm");
        args.extend(qualified.iter().map(String::as_str));
        self.run_pacman(&args)?;
        Ok(())
    }

    fn aur_install(&self, packages: &[&str]) -> Result<(), PacmanError> {
        if packages.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = Vec::with_capacity(2 + packages.len());
        args.push("-S");
        args.push("--noconfirm");
        args.extend_from_slice(packages);
        self.run_paru(&args)?;
        Ok(())
    }

    fn remove(&self, packages: &[&str]) -> Result<(), PacmanError> {
        if packages.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = Vec::with_capacity(2 + packages.len());
        args.push("-R");
        args.push("--noconfirm");
        args.extend_from_slice(packages);
        self.run_pacman(&args)?;
        Ok(())
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

    fn dead() -> LivePacman {
        LivePacman::with_paths("/nonexistent/pacman-1234", "/nonexistent/paru-1234")
    }

    #[test]
    fn pacman_not_in_path_error_class() {
        let err = dead().inventory().expect_err("must fail");
        assert!(
            matches!(err, PacmanError::NotInPath { tool: "pacman", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn sync_databases_not_in_path_error_class() {
        let err = dead().sync_databases().expect_err("must fail");
        assert!(
            matches!(err, PacmanError::NotInPath { tool: "pacman", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn install_not_in_path_error_class() {
        let err = dead().install("extra", &["htop"]).expect_err("must fail");
        assert!(
            matches!(err, PacmanError::NotInPath { tool: "pacman", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn install_empty_slice_is_noop() {
        let err = dead().install("extra", &[]);
        assert!(err.is_ok(), "empty install must short-circuit before spawn");
    }

    #[test]
    fn aur_install_not_in_path_error_class() {
        let err = dead().aur_install(&["yay"]).expect_err("must fail");
        assert!(
            matches!(err, PacmanError::NotInPath { tool: "paru", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn aur_install_empty_slice_is_noop() {
        let err = dead().aur_install(&[]);
        assert!(
            err.is_ok(),
            "empty aur_install must short-circuit before spawn"
        );
    }

    #[test]
    fn remove_not_in_path_error_class() {
        let err = dead().remove(&["htop"]).expect_err("must fail");
        assert!(
            matches!(err, PacmanError::NotInPath { tool: "pacman", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn remove_empty_slice_is_noop() {
        let err = dead().remove(&[]);
        assert!(err.is_ok(), "empty remove must short-circuit before spawn");
    }
}
