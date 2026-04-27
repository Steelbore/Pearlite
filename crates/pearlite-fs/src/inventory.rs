// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Produce a [`ConfigFileInventory`] from declared targets.

use crate::chown::{name_for_gid, name_for_uid};
use crate::hash::sha256_file;
use pearlite_schema::{ConfigEntry, ConfigFileInventory, ConfigFileMeta};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;

/// Stat each declared target; for those that exist on disk, record the
/// SHA-256 digest, mode, owner, and group.
///
/// Missing targets are silently absent from the returned inventory —
/// the diff engine treats absence as drift in its own pass; this
/// function does not encode any policy.
#[must_use]
pub fn probe_config_files(targets: &[ConfigEntry]) -> ConfigFileInventory {
    let mut entries: BTreeMap<_, _> = BTreeMap::new();
    for entry in targets {
        let Ok(metadata) = std::fs::metadata(&entry.target) else {
            continue;
        };
        let Ok(digest) = sha256_file(&entry.target) else {
            continue;
        };
        let mode = metadata.permissions().mode() & 0o7777;
        let owner = name_for_uid(metadata.uid());
        let group = name_for_gid(metadata.gid());
        entries.insert(
            entry.target.clone(),
            ConfigFileMeta {
                sha256: hex::encode(digest),
                mode,
                owner,
                group,
            },
        );
    }
    ConfigFileInventory { entries }
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
    use tempfile::TempDir;

    fn entry(target: PathBuf, source: PathBuf) -> ConfigEntry {
        ConfigEntry {
            target,
            source,
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            restart: Vec::new(),
        }
    }

    #[test]
    fn missing_target_yields_none_in_inventory() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("nonexistent");
        let entries = vec![entry(target.clone(), PathBuf::from("etc/x"))];
        let inv = probe_config_files(&entries);
        assert!(inv.entries.is_empty(), "missing target must not appear");
    }

    #[test]
    fn present_target_records_sha256_and_mode() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("hosts");
        std::fs::write(&target, b"127.0.0.1 localhost\n").expect("seed");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        let entries = vec![entry(target.clone(), PathBuf::from("etc/hosts"))];
        let inv = probe_config_files(&entries);

        let meta = inv.entries.get(&target).expect("entry present");
        assert_eq!(meta.sha256.len(), 64, "hex digest is 32 bytes = 64 chars");
        assert!(meta.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(meta.mode, 0o600);
    }

    #[test]
    fn empty_target_list_yields_empty_inventory() {
        let inv = probe_config_files(&[]);
        assert!(inv.entries.is_empty());
    }
}
