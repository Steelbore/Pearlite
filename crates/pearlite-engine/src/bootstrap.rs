// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Engine-side wiring for ADR-0012 (`pearlite bootstrap`).
//!
//! [`Engine::bootstrap`] is the apply-side companion to
//! [`Engine::plan`](crate::Engine::plan). It runs once per host, on
//! demand, and:
//!
//! 1. Loads the host's declared state via the existing Nickel adapter.
//! 2. Verifies a `[nix.installer]` block is declared (without it,
//!    bootstrapping makes no sense — operator should declare nix or
//!    not call bootstrap).
//! 3. Hands the [`NixInstaller`] adapter the operator-supplied
//!    installer script bytes plus the declared SHA-256 pin. The
//!    adapter's `install_if_missing` short-circuits when nix is
//!    already on `PATH`.
//! 4. Writes `/etc/nix/nix.conf` idempotently with
//!    `experimental-features = nix-command flakes` per ADR-0013.
//!
//! Bootstrap intentionally does **not** call
//! [`pearlite_schema::validate`]: it's a one-shot side-effect, not
//! the full apply contract. Schema validation is `plan`'s
//! responsibility. The post-validation `[nix.installer]` block check
//! lives here as a narrower precondition.
//!
//! Bootstrap state isn't recorded in `state.toml` (ADR-0012 decision
//! 4): nix presence is a runtime fact, not a managed declaration.

use crate::errors::BootstrapError;
use crate::plan::Engine;
use pearlite_userenv::{InstallOutcome, NixInstaller};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One literal line that ADR-0013 requires `/etc/nix/nix.conf` to
/// contain. Bootstrap writes this file iff the line isn't already
/// present (any line in the file that trims to this string counts).
const NIX_CONF_LINE: &str = "experimental-features = nix-command flakes";

/// Outcome of a successful [`Engine::bootstrap`] run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapOutcome {
    /// What [`NixInstaller::install_if_missing`] returned.
    pub install: InstallOutcome,
    /// `true` when bootstrap wrote `/etc/nix/nix.conf` (file was
    /// missing, or didn't contain the experimental-features line).
    /// `false` when the file already had what we'd write (idempotent
    /// no-op).
    pub nix_conf_written: bool,
}

impl Engine {
    /// Bootstrap nix on a fresh host (ADR-0012).
    ///
    /// `host_file` is the per-host Nickel config; the operator's
    /// declared `nix.installer.expected_sha256` is read from it.
    /// `installer` is the [`NixInstaller`] adapter (production:
    /// `LiveNixInstaller`). `script_path` points at the
    /// already-downloaded Determinate installer script — fetching is
    /// the CLI layer's responsibility (or future
    /// `pearlite-bootstrap` chunk), kept out of the engine to
    /// preserve the "no network I/O" invariant.  `nix_conf_path`
    /// targets `/etc/nix/nix.conf` in production; tests inject a
    /// tempdir.
    ///
    /// # Errors
    /// - [`BootstrapError::Nickel`] — Nickel evaluator failed.
    /// - [`BootstrapError::NixNotDeclared`] — the declared host has no
    ///   `[nix.installer]` block.
    /// - [`BootstrapError::Installer`] — installer SHA-256 mismatch
    ///   (ADR-004), missing shell, or non-zero exit from the script.
    /// - [`BootstrapError::Fs`] — atomic write of `nix.conf` failed.
    /// - [`BootstrapError::Io`] — reading existing `nix.conf` failed
    ///   for a reason other than "not found".
    pub fn bootstrap(
        &self,
        host_file: &Path,
        installer: &dyn NixInstaller,
        script_path: &Path,
        nix_conf_path: &Path,
    ) -> Result<BootstrapOutcome, BootstrapError> {
        let declared = pearlite_nickel::load_host(host_file, self.nickel())?;
        let nix = declared
            .nix
            .as_ref()
            .ok_or(BootstrapError::NixNotDeclared)?;

        let install = installer.install_if_missing(
            script_path,
            &nix.installer.expected_sha256,
            &["install", "--determinate", "--no-confirm"],
        )?;

        let nix_conf_written = write_nix_conf_if_needed(nix_conf_path)?;

        Ok(BootstrapOutcome {
            install,
            nix_conf_written,
        })
    }
}

/// Read existing `nix_conf_path` (if any) and write the
/// experimental-features line iff missing.
///
/// Idempotent: if any trimmed line equals [`NIX_CONF_LINE`], we leave
/// the file alone. Otherwise we write a fresh single-line file via
/// [`pearlite_fs::write_etc_atomic`] (ADR-0013 explicitly says
/// Pearlite owns only this minimum line; per-user nix.conf is HM's
/// territory).
fn write_nix_conf_if_needed(path: &Path) -> Result<bool, BootstrapError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(BootstrapError::Io(e)),
    };
    if let Some(content) = existing.as_ref() {
        if content.lines().any(|l| l.trim() == NIX_CONF_LINE) {
            return Ok(false);
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(BootstrapError::Io)?;
        }
    }

    let owner = pearlite_fs::name_for_uid(nix::unistd::getuid().as_raw());
    let group = pearlite_fs::name_for_gid(nix::unistd::getgid().as_raw());
    let body = format!("{NIX_CONF_LINE}\n");
    pearlite_fs::write_etc_atomic(path, body.as_bytes(), 0o644, &owner, &group)?;
    Ok(true)
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
    use crate::mock_probe::MockProbe;
    use pearlite_nickel::MockNickel;
    use pearlite_schema::{HostInfo, KernelInfo, ProbedState};
    use pearlite_userenv::MockNixInstaller;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    const HOST_WITH_NIX: &str = r#"
[meta]
hostname = "forge"
timezone = "UTC"
arch_level = "v4"
locale = "en_US.UTF-8"
keymap = "us"

[kernel]
package = "linux-cachyos"

[nix.installer]
expected_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;

    const HOST_WITHOUT_NIX: &str = r#"
[meta]
hostname = "forge"
timezone = "UTC"
arch_level = "v4"
locale = "en_US.UTF-8"
keymap = "us"

[kernel]
package = "linux-cachyos"
"#;

    fn engine_with_host(host_path: PathBuf, host_body: &str) -> Engine {
        let mut nickel = MockNickel::new();
        nickel.seed(host_path, host_body);
        let probed = ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: None,
            cargo: None,
            config_files: None,
            services: None,
            kernel: KernelInfo::default(),
        };
        Engine::new(
            Box::new(nickel),
            Box::new(MockProbe::with_state(probed)),
            PathBuf::from("/cfg-repo"),
        )
    }

    #[test]
    fn bootstrap_calls_installer_with_declared_sha() {
        let dir = TempDir::new().expect("tempdir");
        let host_file = dir.path().join("forge.ncl");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");

        let installer = MockNixInstaller::new();
        let engine = engine_with_host(host_file.clone(), HOST_WITH_NIX);

        let outcome = engine
            .bootstrap(&host_file, &installer, &script, &nix_conf)
            .expect("bootstrap");

        assert_eq!(outcome.install, InstallOutcome::Installed);
        assert!(outcome.nix_conf_written);

        let history = installer.install_history();
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].expected_sha256,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            history[0].args,
            vec!["install", "--determinate", "--no-confirm"]
        );
        assert_eq!(history[0].script_path, script);
    }

    #[test]
    fn bootstrap_writes_nix_conf_when_absent() {
        let dir = TempDir::new().expect("tempdir");
        let host_file = dir.path().join("forge.ncl");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");

        let installer = MockNixInstaller::new();
        let engine = engine_with_host(host_file.clone(), HOST_WITH_NIX);

        engine
            .bootstrap(&host_file, &installer, &script, &nix_conf)
            .expect("bootstrap");

        let written = std::fs::read_to_string(&nix_conf).expect("read");
        assert!(written.contains(NIX_CONF_LINE), "got {written:?}");
    }

    #[test]
    fn bootstrap_skips_nix_conf_when_line_already_present() {
        let dir = TempDir::new().expect("tempdir");
        let host_file = dir.path().join("forge.ncl");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");
        std::fs::write(
            &nix_conf,
            format!(
                "# operator preamble\n{NIX_CONF_LINE}\nsubstituters = https://cache.nixos.org\n"
            ),
        )
        .expect("write existing nix.conf");
        let original = std::fs::read_to_string(&nix_conf).expect("read original");

        let installer = MockNixInstaller::new();
        let engine = engine_with_host(host_file.clone(), HOST_WITH_NIX);

        let outcome = engine
            .bootstrap(&host_file, &installer, &script, &nix_conf)
            .expect("bootstrap");

        assert!(!outcome.nix_conf_written);
        let after = std::fs::read_to_string(&nix_conf).expect("read after");
        assert_eq!(
            after, original,
            "operator preamble must be preserved untouched"
        );
    }

    #[test]
    fn bootstrap_errors_when_nix_block_not_declared() {
        let dir = TempDir::new().expect("tempdir");
        let host_file = dir.path().join("forge.ncl");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");

        let installer = MockNixInstaller::new();
        let engine = engine_with_host(host_file.clone(), HOST_WITHOUT_NIX);

        let err = engine
            .bootstrap(&host_file, &installer, &script, &nix_conf)
            .expect_err("must fail");
        assert!(matches!(err, BootstrapError::NixNotDeclared), "got {err:?}");
        assert!(installer.install_history().is_empty());
    }

    #[test]
    fn bootstrap_short_circuits_when_nix_already_installed() {
        let dir = TempDir::new().expect("tempdir");
        let host_file = dir.path().join("forge.ncl");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");

        let installer = MockNixInstaller::with_already_installed();
        let engine = engine_with_host(host_file.clone(), HOST_WITH_NIX);

        let outcome = engine
            .bootstrap(&host_file, &installer, &script, &nix_conf)
            .expect("bootstrap");

        assert_eq!(outcome.install, InstallOutcome::Already);
        // nix.conf still gets written even if nix was already
        // installed — the experimental-features line is independent
        // of installer state.
        assert!(outcome.nix_conf_written);
    }
}
