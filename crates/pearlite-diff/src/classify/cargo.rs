// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! cargo classification: same four-way discriminator as pacman, no
//! repo distinction (`cargo install` always pulls from crates.io
//! unless overridden, which v1.0 does not support — see PRD §17.2.3).

use pearlite_schema::CargoInventory;
use pearlite_state::State;
use std::collections::BTreeSet;

/// Per-crate classification of cargo state vs declared state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CargoClassification {
    /// Crates declared and currently installed.
    pub managed: BTreeSet<String>,
    /// Crates declared but not installed — need install.
    pub to_install: Vec<String>,
    /// Crates that were once managed but are no longer declared.
    pub forgotten: Vec<String>,
    /// Crates installed out-of-band, never managed, not adopted.
    pub manual: Vec<String>,
    /// User-flagged "leave alone".
    pub adopted: Vec<String>,
}

/// Apply the four-way discriminator to cargo crates.
#[must_use]
pub fn classify_cargo(
    declared: &[String],
    probed: &CargoInventory,
    state: &State,
) -> CargoClassification {
    let declared_set: BTreeSet<&str> = declared.iter().map(String::as_str).collect();
    let installed: BTreeSet<&str> = probed.crates.keys().map(String::as_str).collect();
    let managed_state: BTreeSet<&str> = state.managed.cargo.iter().map(String::as_str).collect();
    let adopted_state: BTreeSet<&str> = state.adopted.cargo.iter().map(String::as_str).collect();

    let mut managed = BTreeSet::new();
    let mut to_install = Vec::new();
    let mut forgotten = Vec::new();
    let mut manual = Vec::new();
    let mut adopted = Vec::new();

    for &name in &declared_set {
        if installed.contains(name) {
            managed.insert(name.to_owned());
        } else {
            to_install.push(name.to_owned());
        }
    }

    for &name in &installed {
        if declared_set.contains(name) {
            continue;
        }
        if adopted_state.contains(name) {
            adopted.push(name.to_owned());
        } else if managed_state.contains(name) {
            forgotten.push(name.to_owned());
        } else {
            manual.push(name.to_owned());
        }
    }

    CargoClassification {
        managed,
        to_install,
        forgotten,
        manual,
        adopted,
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
    use pearlite_state::SCHEMA_VERSION;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn empty_state() -> State {
        State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed::default(),
            adopted: pearlite_state::Adopted::default(),
            history: Vec::new(),
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: BTreeMap::new(),
        }
    }

    fn inventory_with(crates: &[(&str, &str)]) -> CargoInventory {
        CargoInventory {
            crates: crates
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn declared_and_installed_is_managed() {
        let declared = vec!["zellij".to_owned()];
        let probed = inventory_with(&[("zellij", "0.41.2")]);
        let c = classify_cargo(&declared, &probed, &empty_state());
        assert!(c.managed.contains("zellij"));
        assert!(c.to_install.is_empty());
    }

    #[test]
    fn declared_not_installed_is_to_install() {
        let declared = vec!["zellij".to_owned()];
        let probed = inventory_with(&[]);
        let c = classify_cargo(&declared, &probed, &empty_state());
        assert_eq!(c.to_install, vec!["zellij".to_owned()]);
    }

    #[test]
    fn installed_was_managed_not_declared_is_forgotten() {
        let declared = vec![];
        let probed = inventory_with(&[("ripgrep-all", "0.10.6")]);
        let mut state = empty_state();
        state.managed.cargo = vec!["ripgrep-all".to_owned()];
        let c = classify_cargo(&declared, &probed, &state);
        assert_eq!(c.forgotten, vec!["ripgrep-all".to_owned()]);
    }

    #[test]
    fn installed_not_managed_not_adopted_is_manual() {
        let declared = vec![];
        let probed = inventory_with(&[("ripgrep-all", "0.10.6")]);
        let c = classify_cargo(&declared, &probed, &empty_state());
        assert_eq!(c.manual, vec!["ripgrep-all".to_owned()]);
    }

    #[test]
    fn adopted_suppresses_drift() {
        let declared = vec![];
        let probed = inventory_with(&[("ripgrep-all", "0.10.6")]);
        let mut state = empty_state();
        state.adopted.cargo = vec!["ripgrep-all".to_owned()];
        let c = classify_cargo(&declared, &probed, &state);
        assert_eq!(c.adopted, vec!["ripgrep-all".to_owned()]);
        assert!(c.manual.is_empty());
    }
}
