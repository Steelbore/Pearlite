// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Repository identifiers, `pacman.conf` parsing, `/proc/cpuinfo`
//! arch-level detection.

use pearlite_schema::ArchLevel;
use std::fmt;

/// Typed pacman repository identifier.
///
/// Every repo Pearlite knows about is enumerated here. Unknown repos
/// (e.g. `multilib`, `chaotic-aur`, user-added) round-trip as
/// [`Repo::Other`], preserving their string name for the diff engine
/// to surface as drift.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Repo {
    /// Arch official `core`.
    Core,
    /// Arch official `extra`.
    Extra,
    /// Arch `multilib`.
    Multilib,
    /// CachyOS generic `cachyos`.
    Cachyos,
    /// CachyOS x86-64-v3 repo.
    CachyosV3,
    /// CachyOS x86-64-v4 repo.
    CachyosV4,
    /// AUR (foreign packages reported by `pacman -Qm`).
    Aur,
    /// Repo named in `pacman.conf` but not one of the well-known names.
    Other(String),
}

impl Repo {
    /// Map a repo name (as it appears in `pacman.conf` and
    /// `pacman -Sl`) to a typed [`Repo`].
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "core" => Self::Core,
            "extra" => Self::Extra,
            "multilib" => Self::Multilib,
            "cachyos" => Self::Cachyos,
            "cachyos-v3" => Self::CachyosV3,
            "cachyos-v4" => Self::CachyosV4,
            "aur" => Self::Aur,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Render as the canonical repo name (the inverse of
    /// [`Self::from_name`]).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Core => "core",
            Self::Extra => "extra",
            Self::Multilib => "multilib",
            Self::Cachyos => "cachyos",
            Self::CachyosV3 => "cachyos-v3",
            Self::CachyosV4 => "cachyos-v4",
            Self::Aur => "aur",
            Self::Other(name) => name.as_str(),
        }
    }
}

impl fmt::Display for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Parse `/etc/pacman.conf` and return the list of repo names declared
/// in `[<repo>]` section headers.
///
/// Skips the `[options]` section. Whitespace and comment lines (`#`)
/// are ignored. The order returned matches the order in the file —
/// pacman's own resolution order matters when two repos provide the
/// same package.
#[must_use]
pub fn parse_pacman_conf(content: &str) -> Vec<String> {
    let mut repos = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if !line.starts_with('[') || !line.ends_with(']') {
            continue;
        }
        let inner = &line[1..line.len() - 1];
        if inner == "options" {
            continue;
        }
        repos.push(inner.to_owned());
    }
    repos
}

/// Detect the host's CPU feature level by scanning `/proc/cpuinfo`
/// flags.
///
/// Returns [`ArchLevel::V4`] when AVX-512F is present (and the other
/// v4 baseline features are implied). Otherwise [`ArchLevel::V3`] —
/// CachyOS's minimum supported feature level.
#[must_use]
pub fn detect_arch_level(cpuinfo: &str) -> ArchLevel {
    for line in cpuinfo.lines() {
        let trimmed = line.trim();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        if key.trim() != "flags" {
            continue;
        }
        // x86-64-v4 baseline includes avx512f among others; we require
        // it as the marker.
        let has_v4 = value
            .split_whitespace()
            .any(|flag| flag.eq_ignore_ascii_case("avx512f"));
        if has_v4 {
            return ArchLevel::V4;
        }
        return ArchLevel::V3;
    }
    ArchLevel::V3
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

    const PACMAN_CONF: &str = "\
# Pearlite test fixture
[options]
HoldPkg     = pacman glibc
ParallelDownloads = 5

[core]
Include = /etc/pacman.d/mirrorlist

[extra]
Include = /etc/pacman.d/mirrorlist

[cachyos]
Include = /etc/pacman.d/cachyos-mirrorlist

[cachyos-v4]
Include = /etc/pacman.d/cachyos-v4-mirrorlist
";

    #[test]
    fn from_name_round_trips_known_repos() {
        for name in [
            "core",
            "extra",
            "multilib",
            "cachyos",
            "cachyos-v3",
            "cachyos-v4",
            "aur",
        ] {
            assert_eq!(Repo::from_name(name).name(), name);
        }
    }

    #[test]
    fn from_name_preserves_unknown() {
        let r = Repo::from_name("chaotic-aur");
        assert_eq!(r, Repo::Other("chaotic-aur".to_owned()));
        assert_eq!(r.name(), "chaotic-aur");
    }

    #[test]
    fn pacman_conf_lists_repos_in_declaration_order() {
        let repos = parse_pacman_conf(PACMAN_CONF);
        assert_eq!(repos, vec!["core", "extra", "cachyos", "cachyos-v4"]);
    }

    #[test]
    fn pacman_conf_skips_options_section() {
        let repos = parse_pacman_conf(PACMAN_CONF);
        assert!(!repos.contains(&"options".to_owned()));
    }

    #[test]
    fn pacman_conf_ignores_comments_and_keys() {
        let with_noise = "\
# this is a comment
[options]
HoldPkg = pacman
[core]
Include = /etc/pacman.d/mirrorlist
SigLevel = Required
";
        let repos = parse_pacman_conf(with_noise);
        assert_eq!(repos, vec!["core"]);
    }

    #[test]
    fn detect_arch_level_v4_when_avx512f_present() {
        let cpuinfo = "\
processor : 0
flags     : fpu vme de pse avx2 bmi2 fma avx512f avx512bw
model name : Test
";
        assert_eq!(detect_arch_level(cpuinfo), ArchLevel::V4);
    }

    #[test]
    fn detect_arch_level_v3_when_avx512f_absent() {
        let cpuinfo = "\
processor : 0
flags     : fpu vme de pse avx2 bmi2 fma
model name : Test
";
        assert_eq!(detect_arch_level(cpuinfo), ArchLevel::V3);
    }

    #[test]
    fn detect_arch_level_v3_when_no_flags_line() {
        let cpuinfo = "processor : 0\nmodel name : Test\n";
        assert_eq!(detect_arch_level(cpuinfo), ArchLevel::V3);
    }
}
