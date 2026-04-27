// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `pacman -Qe` / `-Qm` / `-Sl` parsers and inventory composition.

use pearlite_schema::PacmanInventory;
use std::collections::{BTreeMap, BTreeSet};

/// Parse `pacman -Qe` stdout into a `name -> version` map.
///
/// Each line is `<name> <version>`; trailing whitespace is tolerated.
#[must_use]
pub fn parse_qe(stdout: &str) -> BTreeMap<String, String> {
    parse_two_column(stdout)
}

/// Parse `pacman -Qm` stdout into a set of foreign (AUR) package names.
#[must_use]
pub fn parse_qm(stdout: &str) -> BTreeSet<String> {
    parse_two_column(stdout).into_keys().collect()
}

/// Parse `pacman -Sl` stdout (`<repo> <name> <version> [installed]`)
/// into a `name -> repo` map.
///
/// The same package may appear in more than one repo (e.g. an Arch
/// package overridden by `cachyos-v4`); pacman's own resolution order
/// applies the last entry, so this parser returns the **last** repo
/// seen for each name. Callers should feed the parser the raw output
/// of `pacman -Sl` (no repo argument) which respects pacman.conf
/// ordering.
#[must_use]
pub fn parse_sl(stdout: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in stdout.lines() {
        let mut iter = line.split_whitespace();
        let Some(repo) = iter.next() else { continue };
        let Some(name) = iter.next() else { continue };
        // Skip the version field; we only need name → repo here.
        if iter.next().is_none() {
            continue;
        }
        out.insert(name.to_owned(), repo.to_owned());
    }
    out
}

fn parse_two_column(stdout: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in stdout.lines() {
        let mut iter = line.split_whitespace();
        let Some(name) = iter.next() else { continue };
        let Some(version) = iter.next() else { continue };
        out.insert(name.to_owned(), version.to_owned());
    }
    out
}

/// Compose `pacman -Qe`, `-Qm`, `-Sl` outputs into a [`PacmanInventory`].
///
/// - `explicit` = keys of `parse_qe(qe)`.
/// - `foreign` = `parse_qm(qm)`.
/// - `repos` = for each explicit name: its repo from `parse_sl(sl)`,
///   or `"aur"` if it's in `foreign`, otherwise omitted (the diff
///   engine treats absence as "unknown repo, surface as drift").
#[must_use]
pub fn compose_inventory(qe: &str, qm: &str, sl: &str) -> PacmanInventory {
    let qe_map = parse_qe(qe);
    let foreign = parse_qm(qm);
    let sl_map = parse_sl(sl);

    let explicit: BTreeSet<String> = qe_map.into_keys().collect();
    let mut repos = BTreeMap::new();
    for name in &explicit {
        if foreign.contains(name) {
            repos.insert(name.clone(), "aur".to_owned());
        } else if let Some(repo) = sl_map.get(name) {
            repos.insert(name.clone(), repo.clone());
        }
        // else: unknown repo; the diff engine surfaces this as drift.
    }
    PacmanInventory {
        explicit,
        foreign,
        repos,
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

    const QE: &str = "\
linux-cachyos 6.10.6.cachyos1-1
firefox 130.0-1
htop 3.3.0-2
claude-code 1.0.0-1
";

    const QM: &str = "\
claude-code 1.0.0-1
antigravity-bin 0.1.0-1
";

    const SL: &str = "\
core linux-cachyos 6.10.6.cachyos1-1 [installed]
extra firefox 130.0-1
extra htop 3.3.0-2 [installed]
cachyos-v4 firefox 130.0-1.cachyosv4 [installed]
cachyos cachyos-mirrorlist 0.0.1-1
";

    #[test]
    fn parse_qe_known_output() {
        let qe = parse_qe(QE);
        assert_eq!(qe.len(), 4);
        assert_eq!(
            qe.get("linux-cachyos"),
            Some(&"6.10.6.cachyos1-1".to_owned())
        );
        assert_eq!(qe.get("firefox"), Some(&"130.0-1".to_owned()));
    }

    #[test]
    fn parse_qm_distinguishes_foreign() {
        let qm = parse_qm(QM);
        assert_eq!(qm.len(), 2);
        assert!(qm.contains("claude-code"));
        assert!(qm.contains("antigravity-bin"));
    }

    #[test]
    fn parse_sl_maps_name_to_repo_with_pacman_resolution() {
        let sl = parse_sl(SL);
        // firefox is in extra and cachyos-v4; the last one wins per
        // pacman's own resolution.
        assert_eq!(sl.get("firefox"), Some(&"cachyos-v4".to_owned()));
        assert_eq!(sl.get("linux-cachyos"), Some(&"core".to_owned()));
        assert_eq!(sl.get("htop"), Some(&"extra".to_owned()));
    }

    #[test]
    fn compose_classifies_explicit_packages() {
        let inv = compose_inventory(QE, QM, SL);
        assert_eq!(inv.explicit.len(), 4);
        assert_eq!(inv.foreign.len(), 2);

        // claude-code is both explicit and foreign → repo "aur".
        assert_eq!(inv.repos.get("claude-code"), Some(&"aur".to_owned()));
        // firefox: explicit, in cachyos-v4 (last win).
        assert_eq!(inv.repos.get("firefox"), Some(&"cachyos-v4".to_owned()));
        // linux-cachyos: explicit, core.
        assert_eq!(inv.repos.get("linux-cachyos"), Some(&"core".to_owned()));
    }

    #[test]
    fn compose_omits_packages_not_in_any_repo() {
        // A package in -Qe but not in -Sl and not in -Qm: its repo is
        // unknown. Compose omits it from `repos`; the diff engine
        // treats absence as drift.
        let qe = "weirdpkg 1.0\n";
        let inv = compose_inventory(qe, "", "");
        assert!(inv.explicit.contains("weirdpkg"));
        assert!(!inv.repos.contains_key("weirdpkg"));
    }

    #[test]
    fn empty_inputs_yield_empty_inventory() {
        let inv = compose_inventory("", "", "");
        assert!(inv.explicit.is_empty());
        assert!(inv.foreign.is_empty());
        assert!(inv.repos.is_empty());
    }

    #[test]
    fn truncated_qe_lines_are_skipped() {
        let qe = "linux-cachyos\nfirefox 130.0-1\n";
        let parsed = parse_qe(qe);
        assert_eq!(parsed.len(), 1);
        assert!(parsed.contains_key("firefox"));
    }
}
