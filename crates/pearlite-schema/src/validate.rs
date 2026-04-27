// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Cross-field contract checks for [`DeclaredState`].

use crate::declared::DeclaredState;
use crate::errors::ContractViolation;
use crate::host::ArchLevel;
use std::collections::BTreeMap;

/// Check cross-field contracts on a parsed host configuration.
///
/// Serde catches type and enum errors at parse time. This function adds
/// the invariants serde cannot express:
///
/// - **`DUPLICATE_PACKAGES`** — every package name appears in at most one
///   list across [`PackageSet`](crate::PackageSet),
///   [`RemovePolicy::packages`](crate::RemovePolicy), and
///   [`RemovePolicy::ignore`](crate::RemovePolicy).
/// - **`KERNEL_MODULES_NOT_UNIQUE`** — `kernel.modules` and
///   `kernel.blacklist` each contain unique entries, and the two sets are
///   disjoint.
/// - **`ARCH_LEVEL_MISMATCH`** — `meta.arch_level = "v3"` forbids a
///   non-empty `packages.cachyos-v4`, and symmetrically.
///
/// # Errors
/// Returns every violation found, not just the first.
pub fn validate(d: &DeclaredState) -> Result<(), Vec<ContractViolation>> {
    let mut violations = Vec::new();
    check_duplicate_packages(d, &mut violations);
    check_kernel_modules_unique(d, &mut violations);
    check_arch_level_matches_packages(d, &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn check_duplicate_packages(d: &DeclaredState, violations: &mut Vec<ContractViolation>) {
    let p = &d.packages;
    let lists: [(&str, &[String]); 8] = [
        ("packages.core", &p.core),
        ("packages.cachyos", &p.cachyos),
        ("packages.cachyos-v3", &p.cachyos_v3),
        ("packages.cachyos-v4", &p.cachyos_v4),
        ("packages.aur", &p.aur),
        ("packages.cargo", &p.cargo),
        ("remove.packages", &d.remove.packages),
        ("remove.ignore", &d.remove.ignore),
    ];

    let mut seen: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (label, items) in lists {
        for name in items {
            seen.entry(name.clone()).or_default().push(label);
        }
    }

    for (name, sources) in seen {
        if sources.len() > 1 {
            violations.push(ContractViolation {
                id: ContractViolation::DUPLICATE_PACKAGES,
                message: format!("'{}' appears in {}", name, sources.join(", ")),
            });
        }
    }
}

fn check_kernel_modules_unique(d: &DeclaredState, violations: &mut Vec<ContractViolation>) {
    let modules = &d.kernel.modules;
    let blacklist = &d.kernel.blacklist;

    if let Some(dup) = first_duplicate(modules) {
        violations.push(ContractViolation {
            id: ContractViolation::KERNEL_MODULES_NOT_UNIQUE,
            message: format!("'{dup}' appears more than once in kernel.modules"),
        });
    }
    if let Some(dup) = first_duplicate(blacklist) {
        violations.push(ContractViolation {
            id: ContractViolation::KERNEL_MODULES_NOT_UNIQUE,
            message: format!("'{dup}' appears more than once in kernel.blacklist"),
        });
    }

    for name in modules {
        if blacklist.contains(name) {
            violations.push(ContractViolation {
                id: ContractViolation::KERNEL_MODULES_NOT_UNIQUE,
                message: format!("'{name}' appears in both kernel.modules and kernel.blacklist"),
            });
        }
    }
}

fn check_arch_level_matches_packages(d: &DeclaredState, violations: &mut Vec<ContractViolation>) {
    match d.host.arch_level {
        ArchLevel::V3 if !d.packages.cachyos_v4.is_empty() => {
            violations.push(ContractViolation {
                id: ContractViolation::ARCH_LEVEL_MISMATCH,
                message: "meta.arch_level = 'v3' but packages.cachyos-v4 is non-empty".to_owned(),
            });
        }
        ArchLevel::V4 if !d.packages.cachyos_v3.is_empty() => {
            violations.push(ContractViolation {
                id: ContractViolation::ARCH_LEVEL_MISMATCH,
                message: "meta.arch_level = 'v4' but packages.cachyos-v3 is non-empty".to_owned(),
            });
        }
        _ => {}
    }
}

fn first_duplicate(items: &[String]) -> Option<&str> {
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        if !seen.insert(item.as_str()) {
            return Some(item.as_str());
        }
    }
    None
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
    use crate::parse::from_resolved_toml;

    const MINIMAL: &str = include_str!("../../../fixtures/schema/host_minimal.toml");
    const FULL: &str = include_str!("../../../fixtures/schema/host_full.toml");

    #[test]
    fn validate_clean_minimal() {
        let d = from_resolved_toml(MINIMAL).expect("parse");
        assert!(validate(&d).is_ok());
    }

    #[test]
    fn validate_clean_full_fixture() {
        let d = from_resolved_toml(FULL).expect("parse");
        validate(&d).expect("full fixture must satisfy every contract");
    }

    #[test]
    fn validate_duplicate_packages() {
        let mut d = from_resolved_toml(MINIMAL).expect("parse");
        d.packages.core.push("htop".to_owned());
        d.packages.aur.push("htop".to_owned());

        let err = validate(&d).expect_err("duplicate must be flagged");
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].id, ContractViolation::DUPLICATE_PACKAGES);
        assert!(err[0].message.contains("htop"));
        assert!(err[0].message.contains("packages.core"));
        assert!(err[0].message.contains("packages.aur"));
    }

    #[test]
    fn validate_kernel_modules_overlap() {
        let mut d = from_resolved_toml(MINIMAL).expect("parse");
        d.kernel.modules = vec!["nvidia".to_owned(), "nouveau".to_owned()];
        d.kernel.blacklist = vec!["nouveau".to_owned()];

        let err = validate(&d).expect_err("overlap must be flagged");
        assert!(
            err.iter()
                .any(|v| v.id == ContractViolation::KERNEL_MODULES_NOT_UNIQUE
                    && v.message.contains("nouveau")
                    && v.message.contains("kernel.modules")
                    && v.message.contains("kernel.blacklist"))
        );
    }

    #[test]
    fn validate_kernel_modules_duplicate_within_list() {
        let mut d = from_resolved_toml(MINIMAL).expect("parse");
        d.kernel.modules = vec!["nvidia".to_owned(), "nvidia".to_owned()];

        let err = validate(&d).expect_err("duplicate must be flagged");
        assert!(
            err.iter()
                .any(|v| v.id == ContractViolation::KERNEL_MODULES_NOT_UNIQUE
                    && v.message.contains("nvidia"))
        );
    }

    #[test]
    fn validate_arch_level_v3_with_v4_packages() {
        let mut d = from_resolved_toml(MINIMAL).expect("parse");
        d.host.arch_level = ArchLevel::V3;
        d.packages.cachyos_v4 = vec!["firefox".to_owned()];

        let err = validate(&d).expect_err("arch-level mismatch must be flagged");
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].id, ContractViolation::ARCH_LEVEL_MISMATCH);
    }

    #[test]
    fn validate_arch_level_v4_with_v3_packages() {
        let mut d = from_resolved_toml(MINIMAL).expect("parse");
        d.host.arch_level = ArchLevel::V4;
        d.packages.cachyos_v3 = vec!["openssl".to_owned()];

        let err = validate(&d).expect_err("arch-level mismatch must be flagged");
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].id, ContractViolation::ARCH_LEVEL_MISMATCH);
    }

    #[test]
    fn validate_collects_multiple_violations() {
        let mut d = from_resolved_toml(MINIMAL).expect("parse");
        d.packages.core.push("htop".to_owned());
        d.packages.aur.push("htop".to_owned());
        d.kernel.modules = vec!["x".to_owned(), "x".to_owned()];

        let err = validate(&d).expect_err("two violations expected");
        assert!(
            err.iter()
                .any(|v| v.id == ContractViolation::DUPLICATE_PACKAGES)
        );
        assert!(
            err.iter()
                .any(|v| v.id == ContractViolation::KERNEL_MODULES_NOT_UNIQUE)
        );
    }
}
