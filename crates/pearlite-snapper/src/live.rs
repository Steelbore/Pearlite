// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Snapper`] trait + production [`LiveSnapper`] implementation.

use crate::errors::SnapperError;
use crate::list::parse_list;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use time::OffsetDateTime;

/// One Snapper snapshot record.
///
/// The engine carries this in [`State::history`](pearlite_state::State)
/// via `state.toml`'s `[[history]]` entries (the `snapshot_pre` and
/// `snapshot_post` fields). Pearlite builds labels deterministically
/// from `(timestamp, plan_id)` so two `apply` invocations on the same
/// plan produce identical-looking snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotInfo {
    /// Snapper-assigned numeric ID.
    pub id: u64,
    /// Snapshot label (description).
    pub label: String,
    /// UTC creation timestamp.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub created_at: OffsetDateTime,
    /// Snapper config the snapshot belongs to (e.g. `"root"`).
    pub config: String,
}

/// Trait the rest of the workspace consumes to talk to Snapper.
///
/// Three operations cover Pearlite's needs: take a snapshot, list the
/// known snapshots, and roll back to a specific one. Higher-level
/// orchestration (pre/post wrap, label generation) lives in the
/// engine.
pub trait Snapper: Send + Sync {
    /// Create a new snapshot in `config` with the given label.
    ///
    /// # Errors
    /// Returns [`SnapperError`] on spawn / non-zero exit / parse
    /// failure.
    fn create(&self, config: &str, label: &str) -> Result<SnapshotInfo, SnapperError>;

    /// Roll back to the given snapshot ID in `config`.
    ///
    /// # Errors
    /// Returns [`SnapperError`] on spawn / non-zero exit.
    fn rollback(&self, config: &str, snapshot_id: u64) -> Result<(), SnapperError>;

    /// List every snapshot in `config`.
    ///
    /// # Errors
    /// Returns [`SnapperError`] on spawn / non-zero exit / decode
    /// failure.
    fn list(&self, config: &str) -> Result<Vec<SnapshotInfo>, SnapperError>;
}

/// Production [`Snapper`] backed by the `snapper` binary.
#[derive(Clone, Debug)]
pub struct LiveSnapper {
    binary: PathBuf,
}

impl LiveSnapper {
    /// Construct a [`LiveSnapper`] that resolves `snapper` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("snapper"),
        }
    }

    /// Construct a [`LiveSnapper`] with a caller-supplied binary path.
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Path of the `snapper` binary this adapter invokes.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    fn run(&self, args: &[&str]) -> Result<String, SnapperError> {
        let output = match Command::new(&self.binary).args(args).output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SnapperError::NotInPath {
                    hint: "paru -S snapper",
                });
            }
            Err(e) => return Err(SnapperError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(SnapperError::InvocationFailed { code, stderr });
        }

        Ok(String::from_utf8(output.stdout)?)
    }
}

impl Default for LiveSnapper {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapper for LiveSnapper {
    fn create(&self, config: &str, label: &str) -> Result<SnapshotInfo, SnapperError> {
        // `snapper create --print-number` writes the new snapshot's
        // numeric ID to stdout — much friendlier than re-listing.
        let stdout = self.run(&[
            "-c",
            config,
            "create",
            "--print-number",
            "--description",
            label,
        ])?;
        let id_str = stdout.trim();
        let id = id_str.parse::<u64>().map_err(|_| {
            SnapperError::ParseFailed(format!("expected numeric snapshot id, got: {id_str:?}"))
        })?;
        Ok(SnapshotInfo {
            id,
            label: label.to_owned(),
            created_at: OffsetDateTime::now_utc(),
            config: config.to_owned(),
        })
    }

    fn rollback(&self, config: &str, snapshot_id: u64) -> Result<(), SnapperError> {
        let id_string = snapshot_id.to_string();
        self.run(&["-c", config, "rollback", &id_string])?;
        Ok(())
    }

    fn list(&self, config: &str) -> Result<Vec<SnapshotInfo>, SnapperError> {
        let stdout = self.run(&[
            "-c",
            config,
            "list",
            "--columns",
            "number,date,description",
            "--no-headers",
        ])?;
        parse_list(&stdout, config)
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
    fn snapper_not_in_path_error_class() {
        let live = LiveSnapper::with_binary("/nonexistent/snapper-binary-12345");
        let err = live.create("root", "test").expect_err("must fail");
        assert!(matches!(err, SnapperError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn rollback_not_in_path_error_class() {
        let live = LiveSnapper::with_binary("/nonexistent/snapper-binary-12345");
        let err = live.rollback("root", 5).expect_err("must fail");
        assert!(matches!(err, SnapperError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn list_not_in_path_error_class() {
        let live = LiveSnapper::with_binary("/nonexistent/snapper-binary-12345");
        let err = live.list("root").expect_err("must fail");
        assert!(matches!(err, SnapperError::NotInPath { .. }), "got {err:?}");
    }
}
