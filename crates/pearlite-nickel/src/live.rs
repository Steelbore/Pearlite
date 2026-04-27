// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`NickelEvaluator`] trait + production [`LiveNickel`] implementation.

use crate::errors::NickelError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Trait the rest of the workspace consumes to evaluate a Nickel host
/// file. Two implementations: [`LiveNickel`] (production) and
/// [`MockNickel`](crate::MockNickel) under `feature = "test-mocks"`.
pub trait NickelEvaluator: Send + Sync {
    /// Evaluate `host_file` and return its resolved TOML representation.
    ///
    /// # Errors
    /// Implementations propagate adapter-specific failures via
    /// [`NickelError`].
    fn evaluate(&self, host_file: &Path) -> Result<String, NickelError>;
}

/// Production [`NickelEvaluator`] backed by the `nickel` binary.
///
/// Uses argv-array subprocess invocation per Plan §6.5: the binary is
/// invoked as `nickel export -f toml <host_file>`, with stdout captured
/// as the resolved TOML and stderr surfaced verbatim on non-zero exit.
#[derive(Clone, Debug)]
pub struct LiveNickel {
    binary: PathBuf,
}

impl LiveNickel {
    /// Construct a [`LiveNickel`] that resolves `nickel` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("nickel"),
        }
    }

    /// Construct a [`LiveNickel`] with a caller-supplied binary path —
    /// useful for tests that ship a known-good binary alongside fixtures.
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

impl Default for LiveNickel {
    fn default() -> Self {
        Self::new()
    }
}

impl NickelEvaluator for LiveNickel {
    fn evaluate(&self, host_file: &Path) -> Result<String, NickelError> {
        let output = match Command::new(&self.binary)
            .args(["export", "-f", "toml"])
            .arg(host_file)
            .output()
        {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(NickelError::NotInPath {
                    hint: "paru -S nickel-lang",
                });
            }
            Err(e) => return Err(NickelError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(NickelError::EvaluationFailed { code, stderr });
        }

        Ok(String::from_utf8(output.stdout)?)
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
    use std::path::PathBuf;

    #[test]
    fn nickel_not_in_path_error_class() {
        let live = LiveNickel::with_binary("/nonexistent/path/to/nickel-binary-12345");
        let err = live
            .evaluate(Path::new("/tmp/whatever.ncl"))
            .expect_err("must fail");
        assert!(matches!(err, NickelError::NotInPath { .. }), "got {err:?}");
    }

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/")
            .parent()
            .expect("workspace")
            .join("fixtures")
            .join("nickel")
    }

    /// Plan §6.5 acceptance: nickel --version succeeds. Ignored when
    /// nickel isn't on PATH (CI installs it; local devs may not).
    #[test]
    fn version_probe_succeeds() {
        let live = LiveNickel::new();
        let out = Command::new(&live.binary).arg("--version").output();
        if !matches!(&out, Ok(o) if o.status.success()) {
            // nickel not on PATH; CI installs it. On dev boxes without it,
            // these tests pass silently rather than failing — install
            // nickel-lang via `paru -S nickel-lang` to actually run them.
            return;
        }
        let stdout = String::from_utf8_lossy(&out.expect("ok").stdout).into_owned();
        assert!(
            stdout.to_lowercase().contains("nickel"),
            "expected 'nickel' in --version output, got: {stdout}"
        );
    }

    /// Plan §6.5 acceptance: a minimal host file evaluates to TOML that
    /// parses as `DeclaredState`. Skipped when nickel isn't available.
    #[test]
    fn minimal_host_evaluates() {
        let probe = Command::new("nickel").arg("--version").output();
        if !matches!(&probe, Ok(o) if o.status.success()) {
            return;
        }
        let host = fixtures_dir().join("host_minimal.ncl");
        let live = LiveNickel::new();
        let toml = live.evaluate(&host).expect("evaluate");
        let declared = pearlite_schema::from_resolved_toml(&toml).expect("parse");
        assert_eq!(declared.host.hostname, "forge");
        assert_eq!(declared.kernel.package, "linux-cachyos");
    }
}
