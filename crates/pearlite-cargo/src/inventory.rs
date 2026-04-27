// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `cargo install --list` output parsing.

use pearlite_schema::CargoInventory;
use std::collections::BTreeMap;

/// Parse the stdout of `cargo install --list` into a [`CargoInventory`].
///
/// The format is:
///
/// ```text
/// crate-name vX.Y.Z:
///     binary-name
///     other-binary
/// crate-name2 vA.B.C (registry+https://...):
///     binary
/// ```
///
/// Crate header lines start in column 0 and end with a colon. Binary
/// names are indented and ignored — Pearlite tracks installed crates,
/// not the binaries they ship.
///
/// Unknown lines are skipped silently; the parser is forgiving so a
/// future `cargo` release that adds a banner or a footer doesn't break
/// the probe.
#[must_use]
pub fn parse_install_list(stdout: &str) -> CargoInventory {
    let mut crates = BTreeMap::new();
    for line in stdout.lines() {
        // Indented lines list binary names.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let trimmed = line.trim_end().trim_end_matches(':');
        if trimmed.is_empty() {
            continue;
        }
        let Some((name, rest)) = trimmed.split_once(' ') else {
            continue;
        };
        // `rest` is `vX.Y.Z` optionally followed by ` (source...)`.
        let Some(version_field) = rest.split_whitespace().next() else {
            continue;
        };
        let version = version_field.strip_prefix('v').unwrap_or(version_field);
        if name.is_empty() || version.is_empty() {
            continue;
        }
        crates.insert(name.to_owned(), version.to_owned());
    }
    CargoInventory { crates }
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
    fn parse_known_output() {
        let stdout = "\
cargo-machete v0.7.0:
    cargo-machete
nickel-lang-cli v1.10.0 (registry+https://github.com/rust-lang/crates.io-index):
    nickel
ripgrep-all v0.10.6:
    rga
    rga-fzf
    rga-preproc
zellij v0.41.2:
    zellij
";
        let inv = parse_install_list(stdout);
        assert_eq!(inv.crates.len(), 4);
        assert_eq!(inv.crates.get("cargo-machete"), Some(&"0.7.0".to_owned()));
        assert_eq!(
            inv.crates.get("nickel-lang-cli"),
            Some(&"1.10.0".to_owned())
        );
        assert_eq!(inv.crates.get("ripgrep-all"), Some(&"0.10.6".to_owned()));
        assert_eq!(inv.crates.get("zellij"), Some(&"0.41.2".to_owned()));
    }

    #[test]
    fn empty_output_yields_empty_inventory() {
        let inv = parse_install_list("");
        assert!(inv.crates.is_empty());
    }

    #[test]
    fn parses_registry_source_spec() {
        let stdout = "\
foo v1.2.3 (registry+https://github.com/rust-lang/crates.io-index):
    foo
";
        let inv = parse_install_list(stdout);
        assert_eq!(inv.crates.get("foo"), Some(&"1.2.3".to_owned()));
    }

    #[test]
    fn parses_git_source_spec() {
        let stdout = "\
bar v0.1.0 (https://github.com/example/bar#abc123):
    bar
";
        let inv = parse_install_list(stdout);
        assert_eq!(inv.crates.get("bar"), Some(&"0.1.0".to_owned()));
    }

    #[test]
    fn ignores_binary_lines() {
        let stdout = "\
crate-a v1.0.0:
    binary-a
    other
crate-b v2.0.0:
\tbinary-b
";
        let inv = parse_install_list(stdout);
        assert_eq!(inv.crates.len(), 2);
        assert!(!inv.crates.contains_key("binary-a"));
        assert!(!inv.crates.contains_key("binary-b"));
    }

    #[test]
    fn version_without_v_prefix_still_parses() {
        // Defensive: future cargo release might drop the 'v' prefix.
        let stdout = "\
crate-x 1.0.0:
    crate-x
";
        let inv = parse_install_list(stdout);
        assert_eq!(inv.crates.get("crate-x"), Some(&"1.0.0".to_owned()));
    }
}
