// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Constant-memory SHA-256 of a file.

use crate::errors::FsError;
use sha2::{Digest as _, Sha256};
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

const CHUNK: usize = 64 * 1024;

/// Compute the SHA-256 of an in-memory byte slice.
///
/// Useful when the caller has already read the bytes (e.g. apply's
/// `ConfigWrite` reads the source once and verifies the digest before
/// writing).
#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Compute the SHA-256 of the file at `p`, returning the 32-byte digest.
///
/// Reads the file in 64 KiB chunks so memory usage stays constant
/// regardless of file size.
///
/// # Errors
/// Returns [`FsError::Io`] on any read failure.
pub fn sha256_file(p: &Path) -> Result<[u8; 32], FsError> {
    let mut file = File::open(p).map_err(|e| FsError::Io {
        path: p.to_path_buf(),
        source: e,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).map_err(|e| FsError::Io {
            path: p.to_path_buf(),
            source: e,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Compute a SHA-256 over a path that may be a file or a directory.
///
/// **File case:** delegates to [`sha256_file`].
///
/// **Directory case:** walks the tree in sorted order so the output
/// is deterministic across runs and platforms. Each entry contributes
/// (in order):
///
/// 1. The path of the entry **relative to** `root`, encoded in UTF-8.
/// 2. A separator byte (`0x00`) — directory entries stop here.
/// 3. The file's contents, for regular files only.
/// 4. A second separator byte (`0x00`) so a file's tail can't be
///    confused with a sibling's relative path.
///
/// Symlinks are read via `metadata()` (i.e. followed); we don't
/// special-case them. Non-regular non-directory entries (sockets,
/// FIFOs, devices) are skipped — operators can't store those in a
/// HM config repo anyway.
///
/// # Errors
/// Returns [`FsError::Io`] on any read / metadata failure.
#[allow(clippy::missing_panics_doc, reason = "infallible UTF-8 conversion")]
pub fn sha256_dir(root: &Path) -> Result<[u8; 32], FsError> {
    let meta = std::fs::metadata(root).map_err(|e| FsError::Io {
        path: root.to_path_buf(),
        source: e,
    })?;
    if meta.is_file() {
        return sha256_file(root);
    }

    let mut hasher = Sha256::new();
    let entries = collect_sorted(root)?;
    let mut buf = vec![0u8; CHUNK];

    for path in entries {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);

        let entry_meta = std::fs::metadata(&path).map_err(|e| FsError::Io {
            path: path.clone(),
            source: e,
        })?;
        if entry_meta.is_file() {
            let mut file = File::open(&path).map_err(|e| FsError::Io {
                path: path.clone(),
                source: e,
            })?;
            loop {
                let n = file.read(&mut buf).map_err(|e| FsError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        hasher.update([0u8]);
    }

    Ok(hasher.finalize().into())
}

/// Walk `root` recursively, returning every entry's path in sorted
/// (lexicographic) order. Directories are visited but their own paths
/// don't appear in the output — only file / symlink entries do; this
/// matches what `home-manager switch` actually loads.
fn collect_sorted(root: &Path) -> Result<Vec<std::path::PathBuf>, FsError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = std::fs::read_dir(&dir).map_err(|e| FsError::Io {
            path: dir.clone(),
            source: e,
        })?;
        for entry in read {
            let entry = entry.map_err(|e| FsError::Io {
                path: dir.clone(),
                source: e,
            })?;
            let path = entry.path();
            let meta = std::fs::metadata(&path).map_err(|e| FsError::Io {
                path: path.clone(),
                source: e,
            })?;
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() || meta.file_type().is_symlink() {
                out.push(path);
            }
            // Other types (sockets, FIFOs, devices) intentionally
            // skipped — see sha256_dir docstring.
        }
    }
    out.sort();
    Ok(out)
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
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    #[test]
    fn known_file_known_digest() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(b"abc").expect("write");
        f.flush().expect("flush");
        let digest = sha256_file(f.path()).expect("hash");
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let expected = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(digest, expected);
    }

    #[test]
    fn empty_file_known_digest() {
        let f = NamedTempFile::new().expect("tempfile");
        let digest = sha256_file(f.path()).expect("hash");
        // SHA-256 of "" = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let expected = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(digest, expected);
    }

    #[test]
    fn missing_file_yields_io_error() {
        let err = sha256_file(Path::new("/nonexistent/path/here")).expect_err("must fail");
        assert!(matches!(err, FsError::Io { .. }), "got {err:?}");
    }

    #[test]
    fn sha256_dir_on_a_file_delegates_to_sha256_file() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(b"hello").expect("write");
        f.flush().expect("flush");
        assert_eq!(
            sha256_dir(f.path()).expect("dir"),
            sha256_file(f.path()).expect("file"),
        );
    }

    #[test]
    fn sha256_dir_is_deterministic_across_directory_layouts() {
        // Two directories with identical (relative path, content) pairs
        // hash to the same value, even when entries are created in
        // different orders.
        let dir_a = tempfile::TempDir::new().expect("a");
        let dir_b = tempfile::TempDir::new().expect("b");

        // Layout A: write z then a.
        std::fs::write(dir_a.path().join("z.nix"), b"z-contents").expect("a/z");
        std::fs::write(dir_a.path().join("a.nix"), b"a-contents").expect("a/a");

        // Layout B: same content, written in reverse order.
        std::fs::write(dir_b.path().join("a.nix"), b"a-contents").expect("b/a");
        std::fs::write(dir_b.path().join("z.nix"), b"z-contents").expect("b/z");

        assert_eq!(
            sha256_dir(dir_a.path()).expect("a"),
            sha256_dir(dir_b.path()).expect("b"),
        );
    }

    #[test]
    fn sha256_dir_changes_when_file_content_changes() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("home.nix"), b"v1").expect("v1");
        let h1 = sha256_dir(dir.path()).expect("h1");
        std::fs::write(dir.path().join("home.nix"), b"v2").expect("v2");
        let h2 = sha256_dir(dir.path()).expect("h2");
        assert_ne!(h1, h2);
    }

    #[test]
    fn sha256_dir_changes_when_filename_changes() {
        // Renaming a file (same content) must change the hash, since
        // `home-manager` cares about which files exist with which names.
        let dir_a = tempfile::TempDir::new().expect("a");
        std::fs::write(dir_a.path().join("home.nix"), b"x").expect("a");
        let dir_b = tempfile::TempDir::new().expect("b");
        std::fs::write(dir_b.path().join("default.nix"), b"x").expect("b");
        assert_ne!(
            sha256_dir(dir_a.path()).expect("a"),
            sha256_dir(dir_b.path()).expect("b"),
        );
    }

    #[test]
    fn sha256_dir_descends_into_subdirectories() {
        let dir_a = tempfile::TempDir::new().expect("a");
        std::fs::create_dir_all(dir_a.path().join("modules")).expect("mkdir");
        std::fs::write(dir_a.path().join("modules/x.nix"), b"hello").expect("write");

        let dir_b = tempfile::TempDir::new().expect("b");
        // Same byte content but flat layout — different relative path,
        // therefore different digest.
        std::fs::write(dir_b.path().join("x.nix"), b"hello").expect("write");

        assert_ne!(
            sha256_dir(dir_a.path()).expect("a"),
            sha256_dir(dir_b.path()).expect("b"),
        );
    }

    #[test]
    fn sha256_dir_missing_path_yields_io_error() {
        let err = sha256_dir(Path::new("/nonexistent/dir/here")).expect_err("must fail");
        assert!(matches!(err, FsError::Io { .. }), "got {err:?}");
    }
}
