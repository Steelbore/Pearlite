// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Systemd`] trait + production [`LiveSystemd`] implementation.

use crate::errors::SystemdError;
use crate::inventory::compose_inventory;
use pearlite_schema::ServiceInventory;
use std::path::{Path, PathBuf};
use std::process::Command;

/// systemctl scope: system-wide or per-user.
///
/// Mirrors [`pearlite_diff::Scope`](pearlite_diff::Scope) without
/// depending on `pearlite-diff` from this adapter — the engine's
/// match arm converts between them at dispatch time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// `systemctl --system` (the default).
    System,
    /// `systemctl --user`, run as the named user via `runuser -u`.
    User {
        /// Login name of the target user.
        name: String,
    },
}

/// Trait the rest of the workspace consumes to talk to systemd.
///
/// One method per primitive operation the apply engine emits — each
/// matches a single [`Action`](pearlite_diff::Action) variant on the
/// service side: [`ServiceEnable`](pearlite_diff::Action::ServiceEnable),
/// [`ServiceDisable`](pearlite_diff::Action::ServiceDisable),
/// [`ServiceMask`](pearlite_diff::Action::ServiceMask), and
/// [`ServiceRestart`](pearlite_diff::Action::ServiceRestart).
pub trait Systemd: Send + Sync {
    /// Snapshot the unit-file states + currently-active units into a
    /// [`ServiceInventory`].
    ///
    /// # Errors
    /// Implementations propagate adapter-specific failures via
    /// [`SystemdError`].
    fn inventory(&self) -> Result<ServiceInventory, SystemdError>;

    /// Enable a unit at the given scope.
    ///
    /// `Scope::System` runs `systemctl enable <unit>` directly.
    /// `Scope::User { name }` shells through
    /// `runuser -u <name> -- systemctl --user enable <unit>`.
    ///
    /// # Errors
    /// Returns [`SystemdError`] on spawn / non-zero exit.
    fn enable(&self, unit: &str, scope: &Scope) -> Result<(), SystemdError>;

    /// Disable a unit at the given scope.
    ///
    /// Same scope-dispatch rules as [`Self::enable`].
    ///
    /// # Errors
    /// Returns [`SystemdError`] on spawn / non-zero exit.
    fn disable(&self, unit: &str, scope: &Scope) -> Result<(), SystemdError>;

    /// Mask a system-wide unit (`systemctl mask <unit>`).
    ///
    /// Mask is system-scope only — masking a user unit is rare and
    /// not on the M2 plan. The diff engine's
    /// [`Action::ServiceMask`](pearlite_diff::Action::ServiceMask)
    /// reflects that.
    ///
    /// # Errors
    /// Returns [`SystemdError`] on spawn / non-zero exit.
    fn mask(&self, unit: &str) -> Result<(), SystemdError>;

    /// Restart a system-wide unit (`systemctl restart <unit>`).
    ///
    /// User-scope restarts land with M3's user-env phase per
    /// [`Action::ServiceRestart`](pearlite_diff::Action::ServiceRestart)
    /// docs.
    ///
    /// # Errors
    /// Returns [`SystemdError`] on spawn / non-zero exit.
    fn restart(&self, unit: &str) -> Result<(), SystemdError>;
}

/// Production [`Systemd`] backed by the `systemctl` binary.
#[derive(Clone, Debug)]
pub struct LiveSystemd {
    binary: PathBuf,
    runuser_path: PathBuf,
}

impl LiveSystemd {
    /// Construct a [`LiveSystemd`] that resolves `systemctl` and
    /// `runuser` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("systemctl"),
            runuser_path: PathBuf::from("runuser"),
        }
    }

    /// Construct a [`LiveSystemd`] with a caller-supplied `systemctl`
    /// path. `runuser` still resolves from `PATH`.
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            runuser_path: PathBuf::from("runuser"),
        }
    }

    /// Construct a [`LiveSystemd`] with caller-supplied paths for
    /// both `systemctl` and `runuser`.
    pub fn with_paths(binary: impl Into<PathBuf>, runuser: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            runuser_path: runuser.into(),
        }
    }

    /// Path of the `systemctl` binary this adapter invokes.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Path of the `runuser` binary used for user-scope dispatch.
    #[must_use]
    pub fn runuser_path(&self) -> &Path {
        &self.runuser_path
    }

    fn run(&self, args: &[&str]) -> Result<String, SystemdError> {
        run_systemctl(&self.binary, args)
    }

    fn run_as_user(&self, user: &str, systemctl_args: &[&str]) -> Result<(), SystemdError> {
        let bin_str = self.binary.to_string_lossy();
        let mut args: Vec<&str> = Vec::with_capacity(5 + systemctl_args.len());
        args.push("-u");
        args.push(user);
        args.push("--");
        args.push(&bin_str);
        args.push("--user");
        args.extend_from_slice(systemctl_args);

        let output = match Command::new(&self.runuser_path).args(&args).output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SystemdError::NotInPath {
                    hint: "every supported Pearlite host has runuser via util-linux",
                });
            }
            Err(e) => return Err(SystemdError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(SystemdError::InvocationFailed { code, stderr });
        }
        Ok(())
    }
}

fn run_systemctl(binary: &Path, args: &[&str]) -> Result<String, SystemdError> {
    let output = match Command::new(binary).args(args).output() {
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

    fn enable(&self, unit: &str, scope: &Scope) -> Result<(), SystemdError> {
        match scope {
            Scope::System => {
                self.run(&["enable", unit])?;
                Ok(())
            }
            Scope::User { name } => self.run_as_user(name, &["enable", unit]),
        }
    }

    fn disable(&self, unit: &str, scope: &Scope) -> Result<(), SystemdError> {
        match scope {
            Scope::System => {
                self.run(&["disable", unit])?;
                Ok(())
            }
            Scope::User { name } => self.run_as_user(name, &["disable", unit]),
        }
    }

    fn mask(&self, unit: &str) -> Result<(), SystemdError> {
        self.run(&["mask", unit])?;
        Ok(())
    }

    fn restart(&self, unit: &str) -> Result<(), SystemdError> {
        self.run(&["restart", unit])?;
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

    fn dead() -> LiveSystemd {
        LiveSystemd::with_paths(
            "/nonexistent/path/systemctl-binary-12345",
            "/nonexistent/path/runuser-binary-12345",
        )
    }

    #[test]
    fn systemctl_not_in_path_error_class() {
        let err = dead().inventory().expect_err("must fail");
        assert!(matches!(err, SystemdError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn enable_system_not_in_path_error_class() {
        let err = dead()
            .enable("nginx.service", &Scope::System)
            .expect_err("must fail");
        assert!(matches!(err, SystemdError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn enable_user_not_in_path_error_class() {
        let err = dead()
            .enable(
                "syncthing.service",
                &Scope::User {
                    name: "alice".to_owned(),
                },
            )
            .expect_err("must fail");
        assert!(matches!(err, SystemdError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn disable_system_not_in_path_error_class() {
        let err = dead()
            .disable("nginx.service", &Scope::System)
            .expect_err("must fail");
        assert!(matches!(err, SystemdError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn mask_not_in_path_error_class() {
        let err = dead().mask("nginx.service").expect_err("must fail");
        assert!(matches!(err, SystemdError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn restart_not_in_path_error_class() {
        let err = dead().restart("nginx.service").expect_err("must fail");
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
