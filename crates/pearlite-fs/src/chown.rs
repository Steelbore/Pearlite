// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! User/group name resolution and chown/chmod wrappers (no subprocess).

use crate::errors::FsError;
use nix::unistd::{Gid, Group, Uid, User, chown};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

/// Resolve a username to its uid via `getpwnam(3)`.
///
/// # Errors
/// - [`FsError::Nix`] on libc error.
/// - [`FsError::UnknownPrincipal`] if the user is not found.
pub fn resolve_user(name: &str) -> Result<Uid, FsError> {
    match User::from_name(name) {
        Ok(Some(u)) => Ok(u.uid),
        Ok(None) => Err(FsError::UnknownPrincipal {
            kind: "user",
            name: name.to_owned(),
        }),
        Err(e) => Err(FsError::Nix {
            path: name.into(),
            source: e,
        }),
    }
}

/// Resolve a group name to its gid via `getgrnam(3)`.
///
/// # Errors
/// - [`FsError::Nix`] on libc error.
/// - [`FsError::UnknownPrincipal`] if the group is not found.
pub fn resolve_group(name: &str) -> Result<Gid, FsError> {
    match Group::from_name(name) {
        Ok(Some(g)) => Ok(g.gid),
        Ok(None) => Err(FsError::UnknownPrincipal {
            kind: "group",
            name: name.to_owned(),
        }),
        Err(e) => Err(FsError::Nix {
            path: name.into(),
            source: e,
        }),
    }
}

/// Look up a username for the given uid via `getpwuid(3)`. Returns the
/// uid as a numeric string when no entry exists.
#[must_use]
pub fn name_for_uid(uid: u32) -> String {
    match User::from_uid(Uid::from_raw(uid)) {
        Ok(Some(u)) => u.name,
        _ => uid.to_string(),
    }
}

/// Look up a group name for the given gid via `getgrgid(3)`. Returns the
/// gid as a numeric string when no entry exists.
#[must_use]
pub fn name_for_gid(gid: u32) -> String {
    match Group::from_gid(Gid::from_raw(gid)) {
        Ok(Some(g)) => g.name,
        _ => gid.to_string(),
    }
}

/// `chown(path, uid, gid)` via libc.
///
/// # Errors
/// Returns [`FsError::Nix`] on libc error.
pub fn chown_file(path: &Path, uid: Uid, gid: Gid) -> Result<(), FsError> {
    chown(path, Some(uid), Some(gid)).map_err(|e| FsError::Nix {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Set the file mode (permission bits) on `path`.
///
/// # Errors
/// Returns [`FsError::Io`] on filesystem error.
pub fn chmod_file(path: &Path, mode: u32) -> Result<(), FsError> {
    std::fs::set_permissions(path, Permissions::from_mode(mode)).map_err(|e| FsError::Io {
        path: path.to_path_buf(),
        source: e,
    })
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
    fn unknown_user_yields_typed_error() {
        let err = resolve_user("definitely-not-a-real-user-12345").expect_err("must fail");
        assert!(
            matches!(err, FsError::UnknownPrincipal { kind: "user", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn unknown_group_yields_typed_error() {
        let err = resolve_group("definitely-not-a-real-group-12345").expect_err("must fail");
        assert!(
            matches!(err, FsError::UnknownPrincipal { kind: "group", .. }),
            "got {err:?}"
        );
    }
}
