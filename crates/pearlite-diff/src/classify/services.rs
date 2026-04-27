// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! systemd unit-state drift classification.

use pearlite_schema::{ServiceInventory, ServicesDecl};
use std::collections::BTreeSet;

/// Per-state classification of declared systemd unit state vs probed.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ServicesClassification {
    /// Units that need `systemctl enable`.
    pub to_enable: BTreeSet<String>,
    /// Units that need `systemctl disable`.
    pub to_disable: BTreeSet<String>,
    /// Units that need `systemctl mask`.
    pub to_mask: BTreeSet<String>,
    /// Units already in the declared state — no action.
    pub managed: BTreeSet<String>,
    /// Units in a state Pearlite did not declare (e.g. enabled in
    /// systemd but absent from the host config). Surfaced as drift.
    pub drift: BTreeSet<String>,
}

/// Compare declared `services` block against live `ServiceInventory`.
///
/// Drift detection is conservative: a unit is flagged only when it's
/// **enabled** out-of-band and not declared in any of the three sets.
/// Disabled-by-default units (the vast majority of systemd) don't
/// pollute the drift list.
#[must_use]
pub fn classify_services(
    declared: &ServicesDecl,
    probed: &ServiceInventory,
) -> ServicesClassification {
    let decl_enabled: BTreeSet<&str> = declared.enabled.iter().map(String::as_str).collect();
    let decl_disabled: BTreeSet<&str> = declared.disabled.iter().map(String::as_str).collect();
    let decl_masked: BTreeSet<&str> = declared.masked.iter().map(String::as_str).collect();

    let live_enabled: BTreeSet<&str> = probed.enabled.iter().map(String::as_str).collect();
    let live_disabled: BTreeSet<&str> = probed.disabled.iter().map(String::as_str).collect();
    let live_masked: BTreeSet<&str> = probed.masked.iter().map(String::as_str).collect();

    let mut to_enable = BTreeSet::new();
    let mut to_disable = BTreeSet::new();
    let mut to_mask = BTreeSet::new();
    let mut managed = BTreeSet::new();
    let mut drift = BTreeSet::new();

    // Declared enabled: should be live-enabled.
    for &unit in &decl_enabled {
        if live_enabled.contains(unit) {
            managed.insert(unit.to_owned());
        } else {
            to_enable.insert(unit.to_owned());
        }
    }

    // Declared disabled: should be live-disabled.
    for &unit in &decl_disabled {
        if live_disabled.contains(unit) {
            managed.insert(unit.to_owned());
        } else {
            to_disable.insert(unit.to_owned());
        }
    }

    // Declared masked: should be live-masked.
    for &unit in &decl_masked {
        if live_masked.contains(unit) {
            managed.insert(unit.to_owned());
        } else {
            to_mask.insert(unit.to_owned());
        }
    }

    // Drift: live-enabled units not declared in any bucket.
    for unit in &live_enabled {
        if !decl_enabled.contains(unit)
            && !decl_disabled.contains(unit)
            && !decl_masked.contains(unit)
        {
            drift.insert((*unit).to_owned());
        }
    }

    ServicesClassification {
        to_enable,
        to_disable,
        to_mask,
        managed,
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

    fn decl(enabled: &[&str], disabled: &[&str], masked: &[&str]) -> ServicesDecl {
        ServicesDecl {
            enabled: enabled.iter().map(|s| (*s).to_owned()).collect(),
            disabled: disabled.iter().map(|s| (*s).to_owned()).collect(),
            masked: masked.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn inv(enabled: &[&str], disabled: &[&str], masked: &[&str]) -> ServiceInventory {
        ServiceInventory {
            enabled: enabled.iter().map(|s| (*s).to_owned()).collect(),
            disabled: disabled.iter().map(|s| (*s).to_owned()).collect(),
            masked: masked.iter().map(|s| (*s).to_owned()).collect(),
            active: BTreeSet::new(),
        }
    }

    #[test]
    fn declared_enabled_and_live_enabled_is_managed() {
        let d = decl(&["sshd.service"], &[], &[]);
        let p = inv(&["sshd.service"], &[], &[]);
        let c = classify_services(&d, &p);
        assert!(c.managed.contains("sshd.service"));
        assert!(c.to_enable.is_empty());
    }

    #[test]
    fn declared_enabled_but_live_disabled_is_to_enable() {
        let d = decl(&["sshd.service"], &[], &[]);
        let p = inv(&[], &["sshd.service"], &[]);
        let c = classify_services(&d, &p);
        assert!(c.to_enable.contains("sshd.service"));
    }

    #[test]
    fn declared_disabled_but_live_enabled_is_to_disable() {
        let d = decl(&[], &["bluetooth.service"], &[]);
        let p = inv(&["bluetooth.service"], &[], &[]);
        let c = classify_services(&d, &p);
        assert!(c.to_disable.contains("bluetooth.service"));
    }

    #[test]
    fn declared_masked_but_not_live_masked_is_to_mask() {
        let d = decl(&[], &[], &["systemd-resolved.service"]);
        let p = inv(&["systemd-resolved.service"], &[], &[]);
        let c = classify_services(&d, &p);
        assert!(c.to_mask.contains("systemd-resolved.service"));
    }

    #[test]
    fn live_enabled_undeclared_surfaces_as_drift() {
        let d = decl(&[], &[], &[]);
        let p = inv(&["nginx.service"], &[], &[]);
        let c = classify_services(&d, &p);
        assert!(c.drift.contains("nginx.service"));
    }

    #[test]
    fn live_disabled_undeclared_does_not_drift() {
        // Conservatism: a unit that's disabled-by-default is not drift
        // unless someone declared it. Otherwise every system service
        // would pollute the drift list.
        let d = decl(&[], &[], &[]);
        let p = inv(&[], &["foo.service"], &[]);
        let c = classify_services(&d, &p);
        assert!(c.drift.is_empty());
    }

    #[test]
    fn full_three_way_split() {
        let d = decl(
            &["sshd.service"],             // declared enabled
            &["bluetooth.service"],        // declared disabled
            &["systemd-resolved.service"], // declared masked
        );
        let p = inv(
            &["sshd.service", "nginx.service"], // sshd in sync; nginx is drift
            &["bluetooth.service"],             // in sync
            &[],                                // resolved is not yet masked
        );
        let c = classify_services(&d, &p);

        assert!(c.managed.contains("sshd.service"));
        assert!(c.managed.contains("bluetooth.service"));
        assert!(c.to_mask.contains("systemd-resolved.service"));
        assert!(c.drift.contains("nginx.service"));
    }
}
