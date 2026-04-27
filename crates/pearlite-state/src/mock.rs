// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockFs`] for unit tests and engine integration tests.
//!
//! Compiled both inside `cargo test` (without any feature flag) and when
//! downstream crates enable the `test-mocks` feature so they can drive
//! [`StateStore`](crate::StateStore) without touching the real
//! filesystem.

#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "MockFs is a test utility; .expect() on the mutex matches the \
              standard Mutex<T> idiom and is unreachable in any sane test."
)]

use crate::io::FileSystem;
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Failure-injection knobs the next operation will honour.
#[derive(Clone, Copy, Debug, Default)]
struct Inject {
    /// Make the next `write_temp_then_rename` fail before mutating the
    /// target file.
    fail_next_rename: bool,
    /// Make the next `fsync_dir` fail (the rename has already
    /// committed; this models the "data persisted but parent dir not
    /// yet flushed" window).
    fail_next_fsync_dir: bool,
}

#[derive(Default)]
struct Inner {
    files: BTreeMap<PathBuf, Vec<u8>>,
    inject: Inject,
}

/// Shared, thread-safe in-memory filesystem.
///
/// Cloning gives a second handle to the same backing store — the same
/// pattern as `Arc<Mutex<...>>`-backed handles in production code.
#[derive(Clone, Default)]
pub struct MockFs {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for MockFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockFs").finish_non_exhaustive()
    }
}

impl MockFs {
    /// Construct an empty [`MockFs`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed the in-memory filesystem at `path` with `data`.
    pub fn seed(&self, path: impl Into<PathBuf>, data: impl Into<Vec<u8>>) {
        let mut inner = self
            .inner
            .lock()
            .expect("MockFs mutex must not be poisoned");
        inner.files.insert(path.into(), data.into());
    }

    /// Read whatever is stored at `path`, if anything.
    #[must_use]
    pub fn snapshot(&self, path: &Path) -> Option<Vec<u8>> {
        self.inner
            .lock()
            .expect("MockFs mutex must not be poisoned")
            .files
            .get(path)
            .cloned()
    }

    /// Cause the next `write_temp_then_rename` to fail before the
    /// target file is modified. Self-clearing — a single shot.
    pub fn fail_next_rename(&self) {
        self.inner
            .lock()
            .expect("MockFs mutex must not be poisoned")
            .inject
            .fail_next_rename = true;
    }

    /// Cause the next `fsync_dir` to fail. Self-clearing.
    pub fn fail_next_fsync_dir(&self) {
        self.inner
            .lock()
            .expect("MockFs mutex must not be poisoned")
            .inject
            .fail_next_fsync_dir = true;
    }
}

impl FileSystem for MockFs {
    fn read_string(&self, p: &Path) -> std::io::Result<String> {
        let inner = self
            .inner
            .lock()
            .expect("MockFs mutex must not be poisoned");
        match inner.files.get(p) {
            Some(bytes) => {
                String::from_utf8(bytes.clone()).map_err(|e| Error::new(ErrorKind::InvalidData, e))
            }
            None => Err(Error::new(ErrorKind::NotFound, "no such mock file")),
        }
    }

    fn write_temp_then_rename(&self, p: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .expect("MockFs mutex must not be poisoned");
        if inner.inject.fail_next_rename {
            inner.inject.fail_next_rename = false;
            return Err(Error::new(
                ErrorKind::Other,
                "MockFs: injected rename failure",
            ));
        }
        inner.files.insert(p.to_path_buf(), data.to_vec());
        Ok(())
    }

    fn fsync_dir(&self, _p: &Path) -> std::io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .expect("MockFs mutex must not be poisoned");
        if inner.inject.fail_next_fsync_dir {
            inner.inject.fail_next_fsync_dir = false;
            return Err(Error::new(
                ErrorKind::Other,
                "MockFs: injected fsync_dir failure",
            ));
        }
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

    #[test]
    fn seed_then_read_round_trip() {
        let fs = MockFs::new();
        fs.seed("/state.toml", "hello".as_bytes().to_vec());
        let read = fs.read_string(Path::new("/state.toml")).expect("read");
        assert_eq!(read, "hello");
    }

    #[test]
    fn read_missing_yields_not_found() {
        let fs = MockFs::new();
        let err = fs
            .read_string(Path::new("/nope"))
            .expect_err("missing must fail");
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn injected_rename_failure_leaves_old_content() {
        let fs = MockFs::new();
        fs.seed("/state.toml", b"v1".to_vec());

        fs.fail_next_rename();
        let err = fs
            .write_temp_then_rename(Path::new("/state.toml"), b"v2")
            .expect_err("rename should fail");
        assert_eq!(err.kind(), ErrorKind::Other);

        // Subsequent read still returns v1.
        let read = fs.read_string(Path::new("/state.toml")).expect("read");
        assert_eq!(read, "v1");
    }

    #[test]
    fn fail_injection_is_single_shot() {
        let fs = MockFs::new();
        fs.fail_next_rename();
        assert!(
            fs.write_temp_then_rename(Path::new("/x"), b"a").is_err(),
            "first call should fail"
        );
        // Second call should succeed.
        fs.write_temp_then_rename(Path::new("/x"), b"b")
            .expect("second call should succeed");
        assert_eq!(fs.snapshot(Path::new("/x")), Some(b"b".to_vec()));
    }
}
