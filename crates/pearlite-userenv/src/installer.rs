// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`NixInstaller`] trait + production [`LiveNixInstaller`].
//!
//! ADR-004: Pearlite uses the Determinate Nix installer rather than
//! the official one. The installer ships as a shell script the
//! operator (or this code) executes once on a fresh host. Because
//! that's the *only* curl-piped script Pearlite tolerates, the
//! invariant is hash-pinning: the caller hands us the script bytes
//! plus the expected SHA-256, we verify, then exec.
//!
//! CLAUDE.md hard invariant 5: subprocess invocations use
//! [`std::process::Command`] with argv arrays — never `sh -c`. We
//! exec `sh <path-to-installer.sh> -- <args>`, where the trailing
//! `--` separates the shell's own flags from the installer's, and
//! `<args>` is whatever Determinate-specific options the caller
//! passes (`install --determinate`, `--no-confirm`, etc.). The
//! script path itself is a tempfile we control, never operator
//! input.

use crate::errors::InstallerError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of [`NixInstaller::install_if_missing`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallOutcome {
    /// `nix --version` already succeeded; the installer was not run.
    Already,
    /// The installer ran and exited successfully.
    Installed,
}

/// Trait the engine (M3 W2 phase-7 wiring) consumes to bootstrap
/// Nix on a fresh host.
///
/// The trait is intentionally narrow: one method that does
/// "install if missing." Callers handle the network fetch and SHA
/// pin separately — that's policy, not the adapter's concern. A
/// future PR may add `install_force` or a probe-only
/// `is_installed`, but only when an engine consumer needs it.
pub trait NixInstaller: Send + Sync {
    /// Install Nix via the Determinate installer if `nix` is not
    /// already on `PATH`.
    ///
    /// `script_path` points to the installer script bytes already on
    /// disk; this trait does not fetch over the network — that's the
    /// caller's responsibility (and policy choice).
    /// `expected_sha256` is the hex-encoded hash the installer must
    /// match before execution. `installer_args` are the trailing
    /// arguments passed to the script (e.g. `["install",
    /// "--determinate", "--no-confirm"]`).
    ///
    /// # Errors
    /// - [`InstallerError::Sha256Mismatch`] when the script bytes
    ///   don't hash to `expected_sha256`. The script is **never**
    ///   executed in this case.
    /// - [`InstallerError::ScriptFailed`] when the installer ran
    ///   but exited non-zero.
    /// - [`InstallerError::ShellNotInPath`] / [`InstallerError::Io`]
    ///   on platform errors.
    fn install_if_missing(
        &self,
        script_path: &Path,
        expected_sha256: &str,
        installer_args: &[&str],
    ) -> Result<InstallOutcome, InstallerError>;
}

/// Production [`NixInstaller`] backed by the system shell.
#[derive(Clone, Debug)]
pub struct LiveNixInstaller {
    nix: PathBuf,
    sh: PathBuf,
}

impl LiveNixInstaller {
    /// Construct a [`LiveNixInstaller`] resolving `nix` and `sh`
    /// from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nix: PathBuf::from("nix"),
            sh: PathBuf::from("sh"),
        }
    }

    /// Construct with caller-supplied binary paths (FHS quirks /
    /// tests pointing at non-existent paths).
    pub fn with_binaries(nix: impl Into<PathBuf>, sh: impl Into<PathBuf>) -> Self {
        Self {
            nix: nix.into(),
            sh: sh.into(),
        }
    }

    /// Path of the `nix` binary this adapter probes.
    #[must_use]
    pub fn nix(&self) -> &Path {
        &self.nix
    }

    /// Path of the `sh` binary this adapter execs.
    #[must_use]
    pub fn sh(&self) -> &Path {
        &self.sh
    }

    /// Run `nix --version` and decide whether nix is already
    /// installed. Three outcomes: `Ok(true)` already installed,
    /// `Ok(false)` not installed (caller proceeds with the script),
    /// `Err` something else broke.
    fn nix_already_installed(&self) -> Result<bool, InstallerError> {
        match Command::new(&self.nix).arg("--version").output() {
            Ok(out) if out.status.success() => Ok(true),
            // Non-zero exit on a found binary is "the probe failed";
            // surface it rather than silently re-installing.
            Ok(out) => Err(InstallerError::NixProbeFailed(format!(
                "nix --version exited {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(InstallerError::Io(e)),
        }
    }
}

impl Default for LiveNixInstaller {
    fn default() -> Self {
        Self::new()
    }
}

impl NixInstaller for LiveNixInstaller {
    fn install_if_missing(
        &self,
        script_path: &Path,
        expected_sha256: &str,
        installer_args: &[&str],
    ) -> Result<InstallOutcome, InstallerError> {
        if self.nix_already_installed()? {
            return Ok(InstallOutcome::Already);
        }

        let bytes = std::fs::read(script_path)?;
        let digest = pearlite_fs::sha256_bytes(&bytes);
        let actual = hex_encode(&digest);
        if actual != expected_sha256 {
            return Err(InstallerError::Sha256Mismatch {
                expected: expected_sha256.to_owned(),
                actual,
            });
        }

        // Build:
        //   sh <script_path> -- <installer_args...>
        // The `--` separates shell flags from the script's own; argv
        // elements are individual array entries, never interpolated
        // into a string.
        let mut cmd = Command::new(&self.sh);
        cmd.arg(script_path).arg("--");
        for a in installer_args {
            cmd.arg(a);
        }

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(InstallerError::ShellNotInPath {
                    hint: "paru -S util-linux  # provides /bin/sh on a fresh CachyOS image",
                });
            }
            Err(e) => return Err(InstallerError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(InstallerError::ScriptFailed { code, stderr });
        }

        Ok(InstallOutcome::Installed)
    }
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push(hex_nibble(b >> 4));
        s.push(hex_nibble(b & 0x0f));
    }
    s
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => char::from(b'0' + n),
        10..=15 => char::from(b'a' + n - 10),
        _ => unreachable!("nibble fits in 4 bits"),
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
    use tempfile::TempDir;

    /// Produce a fixture installer script and its sha256, plus a
    /// [`LiveNixInstaller`] with `nix` pointed at a non-existent path
    /// so `nix_already_installed` returns false (forces the script
    /// path).
    fn installer_with_script(content: &[u8]) -> (LiveNixInstaller, std::path::PathBuf, String) {
        let dir = TempDir::new().expect("tempdir");
        let dir_path = dir.path().to_path_buf();
        // Leak the TempDir so the path stays valid for the test;
        // the OS reclaims at process exit.
        std::mem::forget(dir);
        let script = dir_path.join("nix-installer.sh");
        std::fs::write(&script, content).expect("write script");
        let sha = hex_encode(&pearlite_fs::sha256_bytes(content));
        let installer = LiveNixInstaller::with_binaries(
            "/nonexistent/nix-binary-12345",
            // /bin/sh is present on every Linux dev box; we want the
            // shell to run but the script itself to exit non-zero so
            // we test only the SHA path.
            "/bin/sh",
        );
        (installer, script, sha)
    }

    #[test]
    fn sha256_mismatch_refuses_to_execute() {
        let content = b"#!/bin/sh\necho 'should not run'\nexit 99\n";
        let (installer, script, _real_sha) = installer_with_script(content);

        let bogus_sha = "0".repeat(64);
        let err = installer
            .install_if_missing(&script, &bogus_sha, &[])
            .expect_err("must fail");
        assert!(
            matches!(err, InstallerError::Sha256Mismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn missing_script_yields_io_error() {
        let installer = LiveNixInstaller::with_binaries("/nonexistent/nix-binary-12345", "/bin/sh");
        let err = installer
            .install_if_missing(
                Path::new("/tmp/no-such-installer-12345.sh"),
                &"0".repeat(64),
                &[],
            )
            .expect_err("must fail");
        assert!(matches!(err, InstallerError::Io(_)), "got {err:?}");
    }

    #[test]
    fn nix_already_installed_short_circuits() {
        // Point `nix` at a tempfile script that prints something and
        // exits 0 — stands in for a real `nix --version` succeeding.
        // We deliberately avoid /bin/true (NixOS doesn't ship it) and
        // /bin/sh with empty args (would block).
        let dir = TempDir::new().expect("tempdir");
        let nix_stub = dir.path().join("nix-stub");
        std::fs::write(
            &nix_stub,
            b"#!/bin/sh\necho 'nix (Determinate) 2.99.0'\nexit 0\n",
        )
        .expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut perms = std::fs::metadata(&nix_stub).expect("stat").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&nix_stub, perms).expect("chmod");
        }

        let installer = LiveNixInstaller::with_binaries(&nix_stub, "/bin/sh");
        // Path / SHA don't matter — they should never be checked.
        let outcome = installer
            .install_if_missing(Path::new("/no-such-path"), "ignored", &[])
            .expect("must succeed");
        assert_eq!(outcome, InstallOutcome::Already);
    }

    #[test]
    fn script_non_zero_exit_surfaces_as_script_failed() {
        // Script that always exits 17.
        let content = b"#!/bin/sh\nexit 17\n";
        let (installer, script, sha) = installer_with_script(content);

        let err = installer
            .install_if_missing(&script, &sha, &[])
            .expect_err("must fail");
        match err {
            InstallerError::ScriptFailed { code, .. } => assert_eq!(code, 17),
            other => panic!("expected ScriptFailed, got {other:?}"),
        }
    }

    #[test]
    fn happy_path_returns_installed() {
        // Trivially-passing script stands in for the real Determinate
        // installer.
        let content = b"#!/bin/sh\nexit 0\n";
        let (installer, script, sha) = installer_with_script(content);

        let outcome = installer
            .install_if_missing(&script, &sha, &["--no-confirm"])
            .expect("must succeed");
        assert_eq!(outcome, InstallOutcome::Installed);
    }
}
