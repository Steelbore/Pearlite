// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Resolved-TOML to [`DeclaredState`] parsing.

use crate::declared::DeclaredState;
use crate::errors::SchemaError;

/// Parse a resolved-TOML host configuration into [`DeclaredState`].
///
/// The input is the stdout of `nickel export -f toml <host_file>`, captured
/// by `pearlite-nickel`. This function is pure — it never reads the
/// filesystem or spawns a subprocess. Cross-field invariants are not
/// checked here; call [`crate::validate`] on the result before relying on
/// the value.
///
/// # Errors
/// Returns [`SchemaError::InvalidToml`] when the input is not well-formed
/// TOML or when a field type or enum variant does not match the schema.
pub fn from_resolved_toml(s: &str) -> Result<DeclaredState, SchemaError> {
    toml::from_str::<DeclaredState>(s).map_err(SchemaError::from)
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
    use crate::host::ArchLevel;

    const MINIMAL: &str = include_str!("../../../fixtures/schema/host_minimal.toml");
    const FULL: &str = include_str!("../../../fixtures/schema/host_full.toml");
    const INVALID_ARCH: &str = include_str!("../../../fixtures/schema/host_invalid_arch.toml");
    const MALFORMED_MODE: &str = include_str!("../../../fixtures/schema/host_malformed_mode.toml");

    #[test]
    fn parse_minimal_succeeds() {
        let d = from_resolved_toml(MINIMAL).expect("minimal fixture parses");
        assert_eq!(d.host.hostname, "forge");
        assert_eq!(d.host.arch_level, ArchLevel::V4);
        assert_eq!(d.kernel.package, "linux-cachyos");
        assert!(d.packages.core.is_empty());
        assert!(d.config_files.is_empty());
        assert!(d.users.is_empty());
    }

    #[test]
    fn parse_full_succeeds() {
        let d = from_resolved_toml(FULL).expect("full fixture parses");
        assert_eq!(d.host.hostname, "forge");
        assert_eq!(d.host.locale, "en_GB.UTF-8");
        assert_eq!(d.kernel.cmdline.len(), 3);
        assert_eq!(d.kernel.modules.len(), 4);
        assert_eq!(d.kernel.blacklist, vec!["nouveau".to_owned()]);
        assert_eq!(d.packages.core.len(), 3);
        assert_eq!(d.packages.cachyos_v4.len(), 3);
        assert_eq!(d.packages.aur.len(), 2);
        assert_eq!(d.config_files.len(), 2);
        assert_eq!(d.config_files[0].mode, 384); // 0o600
        assert_eq!(d.config_files[1].mode, 0o644); // default
        assert_eq!(d.services.enabled.len(), 2);
        assert_eq!(d.users.len(), 2);
        assert!(d.users[0].home_manager.is_some());
        assert!(d.users[1].home_manager.is_none());
        assert_eq!(d.remove.packages, vec!["xterm".to_owned()]);
        assert_eq!(d.snapshots.keep, 30);
    }

    #[test]
    fn reject_invalid_arch_level() {
        let err = from_resolved_toml(INVALID_ARCH).expect_err("v5 must be rejected at parse time");
        assert!(matches!(err, SchemaError::InvalidToml(_)), "got {err:?}");
    }

    #[test]
    fn reject_malformed_mode() {
        let err = from_resolved_toml(MALFORMED_MODE)
            .expect_err("string mode must be rejected at parse time");
        assert!(matches!(err, SchemaError::InvalidToml(_)), "got {err:?}");
    }
}
