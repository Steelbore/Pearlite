// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `/etc` config-file drift classification.
//!
//! Pure: receives pre-computed source-file SHA-256 digests rather than
//! reading them itself. The engine (which is allowed to do I/O)
//! computes the digests via `pearlite-fs::sha256_file` and passes them
//! in.

use pearlite_schema::{ConfigEntry, ConfigFileInventory};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One detected config-file drift item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFileDrift {
    /// Absolute target path under `/etc`.
    pub target: PathBuf,
    /// Index in the host's `[[config]]` array (preserves declaration
    /// order for downstream rendering).
    pub declaration_index: usize,
    /// Why this entry is drifting.
    pub reason: ConfigDriftReason,
    /// Declared SHA-256 (the source as rendered from the repo,
    /// hex-encoded).
    pub declared_sha256: String,
    /// Live SHA-256 read from disk, if the file exists.
    pub live_sha256: Option<String>,
}

/// Specific reason a config file is flagged as drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigDriftReason {
    /// Declared target is absent on disk; needs writing.
    Missing,
    /// Content SHA-256 differs from declared.
    Sha256Mismatch,
    /// Mode bits (`stat(2).st_mode & 0o7777`) differ from declared.
    ModeMismatch,
    /// Owner name differs from declared.
    OwnerMismatch,
    /// Group name differs from declared.
    GroupMismatch,
}

/// Per-`/etc`-target classification.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ConfigClassification {
    /// Files declared and matching on disk — no action needed.
    pub managed: Vec<PathBuf>,
    /// Files needing some kind of write or chmod/chown. Each entry is
    /// the index in the host's `[[config]]` array; consumers pull the
    /// full [`ConfigEntry`] back from the declared slice for execution.
    pub to_apply: Vec<usize>,
    /// Drift surfaced for human/agent review.
    pub drift: Vec<ConfigFileDrift>,
}

/// Compare declared `[[config]]` entries against the live filesystem
/// state.
///
/// `declared_source_sha256` maps each declared `source` path (relative
/// to the user's config repo) to its hex-encoded SHA-256. The engine
/// computes these once and passes them in.
#[must_use]
pub fn classify_config(
    declared: &[ConfigEntry],
    declared_source_sha256: &BTreeMap<PathBuf, String>,
    probed: &ConfigFileInventory,
) -> ConfigClassification {
    let mut managed = Vec::new();
    let mut to_apply = Vec::new();
    let mut drift = Vec::new();

    for (index, entry) in declared.iter().enumerate() {
        let declared_sha = declared_source_sha256
            .get(&entry.source)
            .cloned()
            .unwrap_or_default();

        match probed.entries.get(&entry.target) {
            None => {
                drift.push(ConfigFileDrift {
                    target: entry.target.clone(),
                    declaration_index: index,
                    reason: ConfigDriftReason::Missing,
                    declared_sha256: declared_sha,
                    live_sha256: None,
                });
                to_apply.push(index);
            }
            Some(meta) => {
                let mut differs = false;
                let mut reason = ConfigDriftReason::Sha256Mismatch;
                if meta.sha256 != declared_sha {
                    differs = true;
                    reason = ConfigDriftReason::Sha256Mismatch;
                } else if meta.mode != entry.mode {
                    differs = true;
                    reason = ConfigDriftReason::ModeMismatch;
                } else if meta.owner != entry.owner {
                    differs = true;
                    reason = ConfigDriftReason::OwnerMismatch;
                } else if meta.group != entry.group {
                    differs = true;
                    reason = ConfigDriftReason::GroupMismatch;
                }

                if differs {
                    drift.push(ConfigFileDrift {
                        target: entry.target.clone(),
                        declaration_index: index,
                        reason,
                        declared_sha256: declared_sha,
                        live_sha256: Some(meta.sha256.clone()),
                    });
                    to_apply.push(index);
                } else {
                    managed.push(entry.target.clone());
                }
            }
        }
    }

    ConfigClassification {
        managed,
        to_apply,
        drift,
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
    use pearlite_schema::ConfigFileMeta;

    fn entry(target: &str, source: &str, mode: u32) -> ConfigEntry {
        ConfigEntry {
            target: PathBuf::from(target),
            source: PathBuf::from(source),
            mode,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            restart: Vec::new(),
        }
    }

    fn meta(sha: &str, mode: u32, owner: &str, group: &str) -> ConfigFileMeta {
        ConfigFileMeta {
            sha256: sha.to_owned(),
            mode,
            owner: owner.to_owned(),
            group: group.to_owned(),
        }
    }

    #[test]
    fn matching_target_is_managed() {
        let declared = vec![entry("/etc/hosts", "etc/hosts", 0o644)];
        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/hosts"), "abc".to_owned());
        let mut inv = ConfigFileInventory::default();
        inv.entries.insert(
            PathBuf::from("/etc/hosts"),
            meta("abc", 0o644, "root", "root"),
        );

        let c = classify_config(&declared, &sha, &inv);
        assert_eq!(c.managed, vec![PathBuf::from("/etc/hosts")]);
        assert!(c.drift.is_empty());
    }

    #[test]
    fn missing_target_yields_drift() {
        let declared = vec![entry("/etc/hosts", "etc/hosts", 0o644)];
        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/hosts"), "abc".to_owned());
        let inv = ConfigFileInventory::default();

        let c = classify_config(&declared, &sha, &inv);
        assert_eq!(c.drift.len(), 1);
        assert_eq!(c.drift[0].reason, ConfigDriftReason::Missing);
        assert_eq!(c.drift[0].declaration_index, 0);
        assert_eq!(c.to_apply, vec![0]);
    }

    #[test]
    fn sha256_mismatch_surfaces_in_drift() {
        let declared = vec![entry("/etc/hosts", "etc/hosts", 0o644)];
        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/hosts"), "abc".to_owned());
        let mut inv = ConfigFileInventory::default();
        inv.entries.insert(
            PathBuf::from("/etc/hosts"),
            meta("xyz", 0o644, "root", "root"),
        );

        let c = classify_config(&declared, &sha, &inv);
        assert_eq!(c.drift.len(), 1);
        assert_eq!(c.drift[0].reason, ConfigDriftReason::Sha256Mismatch);
        assert_eq!(c.drift[0].declared_sha256, "abc");
        assert_eq!(c.drift[0].live_sha256.as_deref(), Some("xyz"));
    }

    #[test]
    fn mode_mismatch_surfaces() {
        let declared = vec![entry("/etc/hosts", "etc/hosts", 0o600)];
        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/hosts"), "abc".to_owned());
        let mut inv = ConfigFileInventory::default();
        inv.entries.insert(
            PathBuf::from("/etc/hosts"),
            meta("abc", 0o644, "root", "root"),
        );

        let c = classify_config(&declared, &sha, &inv);
        assert_eq!(c.drift.len(), 1);
        assert_eq!(c.drift[0].reason, ConfigDriftReason::ModeMismatch);
    }

    #[test]
    fn owner_mismatch_surfaces() {
        let declared = vec![entry("/etc/hosts", "etc/hosts", 0o644)];
        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/hosts"), "abc".to_owned());
        let mut inv = ConfigFileInventory::default();
        inv.entries.insert(
            PathBuf::from("/etc/hosts"),
            meta("abc", 0o644, "alice", "root"),
        );

        let c = classify_config(&declared, &sha, &inv);
        assert_eq!(c.drift.len(), 1);
        assert_eq!(c.drift[0].reason, ConfigDriftReason::OwnerMismatch);
    }

    #[test]
    fn declaration_index_preserved() {
        let declared = vec![
            entry("/etc/a", "etc/a", 0o644),
            entry("/etc/b", "etc/b", 0o644),
            entry("/etc/c", "etc/c", 0o644),
        ];
        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/a"), "a".to_owned());
        sha.insert(PathBuf::from("etc/b"), "b".to_owned());
        sha.insert(PathBuf::from("etc/c"), "c".to_owned());
        let mut inv = ConfigFileInventory::default();
        inv.entries
            .insert(PathBuf::from("/etc/a"), meta("a", 0o644, "root", "root"));
        // /etc/b missing
        inv.entries.insert(
            PathBuf::from("/etc/c"),
            meta("c-different", 0o644, "root", "root"),
        );

        let c = classify_config(&declared, &sha, &inv);
        assert_eq!(c.managed, vec![PathBuf::from("/etc/a")]);
        assert_eq!(c.to_apply, vec![1, 2]);
        assert_eq!(c.drift.len(), 2);
        assert_eq!(c.drift[0].declaration_index, 1);
        assert_eq!(c.drift[1].declaration_index, 2);
    }
}
