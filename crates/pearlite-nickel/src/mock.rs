// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockNickel`] for unit tests.
//!
//! Compiled in `cargo test` (no feature) and behind `feature =
//! "test-mocks"` for downstream consumers (the engine's integration
//! tests in M2+).

use crate::errors::NickelError;
use crate::live::NickelEvaluator;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// In-memory canned-output evaluator: maps host-file paths to their
/// expected resolved-TOML strings.
#[derive(Clone, Debug, Default)]
pub struct MockNickel {
    canned: BTreeMap<PathBuf, String>,
}

impl MockNickel {
    /// Construct an empty [`MockNickel`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed `host_file → output`. Replaces any prior canned output
    /// for the same path.
    pub fn seed(&mut self, host_file: impl Into<PathBuf>, output: impl Into<String>) {
        self.canned.insert(host_file.into(), output.into());
    }
}

impl NickelEvaluator for MockNickel {
    fn evaluate(&self, host_file: &Path) -> Result<String, NickelError> {
        self.canned
            .get(host_file)
            .cloned()
            .ok_or_else(|| NickelError::MockMissing(host_file.to_path_buf()))
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
    use crate::load_host;

    const MINIMAL: &str = include_str!("../../../fixtures/schema/host_minimal.toml");

    #[test]
    fn canned_output_round_trips_to_declared_state() {
        let mut mock = MockNickel::new();
        let host = Path::new("/cfg/forge.ncl");
        mock.seed(host, MINIMAL);

        let declared = load_host(host, &mock).expect("load");
        assert_eq!(declared.host.hostname, "forge");
        assert_eq!(declared.kernel.package, "linux-cachyos");
    }

    #[test]
    fn missing_path_yields_mock_missing_error() {
        let mock = MockNickel::new();
        let err = load_host(Path::new("/cfg/nope.ncl"), &mock).expect_err("must fail");
        assert!(matches!(err, NickelError::MockMissing(_)), "got {err:?}");
    }
}
