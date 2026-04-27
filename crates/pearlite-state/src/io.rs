// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Filesystem trait + production [`LiveFs`] implementation for atomic
//! `state.toml` writes per PRD §7.4.

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use tempfile::NamedTempFile;

/// Filesystem operations the [`StateStore`](crate::StateStore) needs.
///
/// Three primitives is enough: reading the file, writing through
/// temp-and-rename, and flushing the parent directory entry. Splitting
/// the rename and the dir-fsync into separate calls is what lets a mock
/// simulate the "crashed between rename and fsync" failure mode in
/// chunk M1-W1-F.
pub trait FileSystem: Send + Sync {
    /// Read a UTF-8 file into a string.
    ///
    /// # Errors
    /// Returns the underlying I/O error verbatim; callers translate it
    /// into [`StateError::Io`](crate::StateError::Io).
    fn read_string(&self, p: &Path) -> std::io::Result<String>;

    /// Write `data` to a sibling temp file, fsync it, then atomically
    /// rename to `p`.
    ///
    /// The temp file lives in `p`'s parent directory so the rename is
    /// guaranteed-atomic on btrfs (PRD §7.4).
    ///
    /// # Errors
    /// Returns the underlying I/O error verbatim.
    fn write_temp_then_rename(&self, p: &Path, data: &[u8]) -> std::io::Result<()>;

    /// `fsync(2)` on the directory entry that holds `p`.
    ///
    /// Without the directory fsync the rename can be reordered relative
    /// to subsequent activity on btrfs, breaking the "old version or new
    /// version, never partial" invariant.
    ///
    /// # Errors
    /// Returns the underlying I/O error verbatim.
    fn fsync_dir(&self, p: &Path) -> std::io::Result<()>;
}

/// Production [`FileSystem`] implementation backed by `std::fs` and
/// [`tempfile::NamedTempFile`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LiveFs;

impl FileSystem for LiveFs {
    fn read_string(&self, p: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(p)
    }

    fn write_temp_then_rename(&self, p: &Path, data: &[u8]) -> std::io::Result<()> {
        let parent = p.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "atomic write target has no parent directory",
            )
        })?;
        let mut tmp = NamedTempFile::new_in(parent)?;
        tmp.as_file_mut().write_all(data)?;
        tmp.as_file_mut().sync_all()?;
        tmp.persist(p).map_err(std::io::Error::from)?;
        Ok(())
    }

    fn fsync_dir(&self, p: &Path) -> std::io::Result<()> {
        let parent = p.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "fsync_dir target has no parent directory",
            )
        })?;
        let dir = File::open(parent)?;
        dir.sync_all()?;
        Ok(())
    }
}
