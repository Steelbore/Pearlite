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
}
