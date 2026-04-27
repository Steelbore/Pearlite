// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `systemctl list-unit-files` + `systemctl list-units` parsers and
//! composition into a [`ServiceInventory`].

use pearlite_schema::ServiceInventory;
use std::collections::BTreeSet;

/// Parse the stdout of
/// `systemctl list-unit-files --no-pager --no-legend` into the three
/// disjoint state buckets `(enabled, disabled, masked)`.
///
/// `enabled-runtime` and `masked-runtime` are folded into the
/// non-runtime variants — the difference matters at apply time but not
/// for drift detection.
///
/// `static`, `alias`, `indirect`, `generated`, and `transient` are
/// silently skipped: those states are not user-configurable enable
/// values and never appear in declared service config.
#[must_use]
pub fn parse_list_unit_files(
    stdout: &str,
) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
    let mut enabled = BTreeSet::new();
    let mut disabled = BTreeSet::new();
    let mut masked = BTreeSet::new();
    for line in stdout.lines() {
        let mut iter = line.split_whitespace();
        let Some(unit) = iter.next() else { continue };
        let Some(state) = iter.next() else { continue };
        match state {
            "enabled" | "enabled-runtime" => {
                enabled.insert(unit.to_owned());
            }
            "disabled" => {
                disabled.insert(unit.to_owned());
            }
            "masked" | "masked-runtime" => {
                masked.insert(unit.to_owned());
            }
            _ => {} // static / alias / indirect / generated / transient
        }
    }
    (enabled, disabled, masked)
}

/// Parse the stdout of
/// `systemctl list-units --no-pager --no-legend --all --plain` into the
/// set of currently active, loaded units.
///
/// Filter: `LOAD == loaded && ACTIVE == active`. Units that are
/// not-found, error, masked, or inactive are skipped.
#[must_use]
pub fn parse_list_units(stdout: &str) -> BTreeSet<String> {
    let mut active = BTreeSet::new();
    for line in stdout.lines() {
        let mut iter = line.split_whitespace();
        let Some(unit) = iter.next() else { continue };
        let Some(load) = iter.next() else { continue };
        let Some(active_state) = iter.next() else {
            continue;
        };
        if load == "loaded" && active_state == "active" {
            active.insert(unit.to_owned());
        }
    }
    active
}

/// Combine `list-unit-files` and `list-units` outputs into one
/// [`ServiceInventory`].
#[must_use]
pub fn compose_inventory(unit_files: &str, units: &str) -> ServiceInventory {
    let (enabled, disabled, masked) = parse_list_unit_files(unit_files);
    let active = parse_list_units(units);
    ServiceInventory {
        enabled,
        disabled,
        masked,
        active,
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

    const UNIT_FILES: &str = "\
nginx.service                              enabled        enabled
sshd.service                               enabled        enabled
NetworkManager.service                     enabled-runtime enabled
bluetooth.service                          disabled       enabled
systemd-resolved.service                   masked         enabled
shutdown.target                            static         -
console-getty.service                      indirect       -
systemd-tmpfiles-setup.service             alias          -
";

    const UNITS: &str = "\
nginx.service                       loaded    active   running   A high-performance web server
sshd.service                        loaded    active   running   OpenBSD Secure Shell server
bluetooth.service                   loaded    inactive dead      Bluetooth service
systemd-resolved.service            masked    inactive dead      systemd-resolved.service
not-found.service                   not-found inactive dead      not-found.service
";

    #[test]
    fn list_unit_files_distinguishes_enabled_disabled_masked() {
        let (enabled, disabled, masked) = parse_list_unit_files(UNIT_FILES);

        assert_eq!(enabled.len(), 3);
        assert!(enabled.contains("nginx.service"));
        assert!(enabled.contains("sshd.service"));
        assert!(enabled.contains("NetworkManager.service")); // enabled-runtime

        assert_eq!(disabled.len(), 1);
        assert!(disabled.contains("bluetooth.service"));

        assert_eq!(masked.len(), 1);
        assert!(masked.contains("systemd-resolved.service"));
    }

    #[test]
    fn list_unit_files_skips_static_alias_indirect() {
        let (enabled, disabled, masked) = parse_list_unit_files(UNIT_FILES);
        let all: BTreeSet<&str> = enabled
            .iter()
            .chain(disabled.iter())
            .chain(masked.iter())
            .map(String::as_str)
            .collect();
        assert!(!all.contains("shutdown.target"));
        assert!(!all.contains("console-getty.service"));
        assert!(!all.contains("systemd-tmpfiles-setup.service"));
    }

    #[test]
    fn list_units_filters_to_loaded_active() {
        let active = parse_list_units(UNITS);
        assert_eq!(active.len(), 2);
        assert!(active.contains("nginx.service"));
        assert!(active.contains("sshd.service"));
        // Inactive: not active.
        assert!(!active.contains("bluetooth.service"));
        // Masked: not loaded.
        assert!(!active.contains("systemd-resolved.service"));
        // Not-found: not loaded.
        assert!(!active.contains("not-found.service"));
    }

    #[test]
    fn empty_outputs_yield_empty_inventory() {
        let inv = compose_inventory("", "");
        assert!(inv.enabled.is_empty());
        assert!(inv.disabled.is_empty());
        assert!(inv.masked.is_empty());
        assert!(inv.active.is_empty());
    }

    #[test]
    fn compose_combines_both_streams() {
        let inv = compose_inventory(UNIT_FILES, UNITS);
        assert_eq!(inv.enabled.len(), 3);
        assert_eq!(inv.disabled.len(), 1);
        assert_eq!(inv.masked.len(), 1);
        assert_eq!(inv.active.len(), 2);
    }

    #[test]
    fn truncated_lines_are_skipped() {
        // Defensive: a corrupt or partial line must not panic.
        let truncated = "nginx.service\nsshd.service enabled\n";
        let (enabled, _, _) = parse_list_unit_files(truncated);
        assert_eq!(enabled.len(), 1);
        assert!(enabled.contains("sshd.service"));
    }
}
