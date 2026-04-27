// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Atomic write of `/etc` files preserving mode/owner/group per PRD §7.4.

use crate::chown::{chmod_file, chown_file, resolve_group, resolve_user};
use crate::errors::FsError;
use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use tempfile::NamedTempFile;

/// Atomically write `content` to `target` with the given mode, owner,
/// and group.
///
/// Steps (PRD §7.4):
///
/// 1. Create a sibling temp file in the target's parent directory.
/// 2. Write `content`.
/// 3. `chmod` and `chown` the temp file **before** the rename so the
///    new file appears with the right ownership atomically.
/// 4. `fsync` the file's data.
/// 5. Atomically rename to the target path.
/// 6. `fsync` the parent directory.
///
/// # Errors
/// - [`FsError::NoParent`] when the target has no parent directory.
/// - [`FsError::Io`] on any filesystem error.
/// - [`FsError::Nix`] on chown failure.
/// - [`FsError::UnknownPrincipal`] when `owner` or `group` does not
///   resolve.
pub fn write_etc_atomic(
    target: &Path,
    content: &[u8],
    mode: u32,
    owner: &str,
    group: &str,
) -> Result<(), FsError> {
    let parent = target
        .parent()
        .ok_or_else(|| FsError::NoParent(target.to_path_buf()))?;

    let uid = resolve_user(owner)?;
    let gid = resolve_group(group)?;

    let mut tmp = NamedTempFile::new_in(parent).map_err(|e| FsError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    tmp.as_file_mut()
        .write_all(content)
        .map_err(|e| FsError::Io {
            path: tmp.path().to_path_buf(),
            source: e,
        })?;

    chmod_file(tmp.path(), mode)?;
    chown_file(tmp.path(), uid, gid)?;

    tmp.as_file_mut().sync_all().map_err(|e| FsError::Io {
        path: tmp.path().to_path_buf(),
        source: e,
    })?;

    let tmp_path = tmp.path().to_path_buf();
    tmp.persist(target).map_err(|e| FsError::Io {
        path: tmp_path,
        source: e.error,
    })?;

    let dir = File::open(parent).map_err(|e| FsError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    dir.sync_all().map_err(|e| FsError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;

    Ok(())
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
    use crate::chown::{name_for_gid, name_for_uid};
    use nix::unistd::{getgid, getuid};
    use std::os::unix::fs::MetadataExt as _;
    use std::os::unix::fs::PermissionsExt as _;
    use tempfile::TempDir;

    fn current_user() -> String {
        name_for_uid(getuid().as_raw())
    }

    fn current_group() -> String {
        name_for_gid(getgid().as_raw())
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("hosts");
        write_etc_atomic(
            &target,
            b"127.0.0.1 localhost\n",
            0o644,
            &current_user(),
            &current_group(),
        )
        .expect("write");
        let read = std::fs::read(&target).expect("read");
        assert_eq!(read, b"127.0.0.1 localhost\n");
    }

    #[test]
    fn mode_preserved_through_temp() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("secret.conf");
        write_etc_atomic(
            &target,
            b"key=value\n",
            0o600,
            &current_user(),
            &current_group(),
        )
        .expect("write");
        let mode = std::fs::metadata(&target)
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn chown_to_self_succeeds() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("file");
        let user = current_user();
        let group = current_group();
        write_etc_atomic(&target, b"", 0o644, &user, &group).expect("write");
        let meta = std::fs::metadata(&target).expect("stat");
        assert_eq!(meta.uid(), getuid().as_raw());
        assert_eq!(meta.gid(), getgid().as_raw());
    }

    #[test]
    fn unknown_owner_yields_typed_error() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("file");
        let err = write_etc_atomic(&target, b"", 0o644, "no-such-user-9999", "root")
            .expect_err("must fail");
        assert!(
            matches!(err, FsError::UnknownPrincipal { kind: "user", .. }),
            "got {err:?}"
        );
    }

    /// PRD §7.4 atomicity claim: a kill-9 between temp write and rename
    /// must leave the target unchanged. Needs fork-based test machinery
    /// that's deferred to M1 W2 alongside the engine integration tier.
    #[test]
    #[ignore = "needs rusty-fork — lands in M1 W2 with engine integration tests"]
    fn crash_after_write_before_rename() {
        unimplemented!("M1 W2");
    }

    /// PRD §7.4: a concurrent reader sees either the old contents or
    /// the new, never a partial write. Same fork-based-test caveat.
    #[test]
    #[ignore = "needs rusty-fork — lands in M1 W2 with engine integration tests"]
    fn rename_atomic_visible_to_concurrent_reader() {
        unimplemented!("M1 W2");
    }
}
