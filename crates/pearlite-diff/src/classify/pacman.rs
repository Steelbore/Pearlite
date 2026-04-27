// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! pacman classification: the four-way drift discriminator from PRD §7.3.

use pearlite_schema::{PackageSet, PacmanInventory, RemovePolicy};
use pearlite_state::State;
use std::collections::{BTreeMap, BTreeSet};

/// Per-package classification of pacman state vs declared state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PacmanClassification {
    /// Packages that are both declared and currently installed —
    /// no action needed.
    pub managed: BTreeSet<String>,
    /// Packages declared but not installed — need install. Each entry
    /// carries the canonical repo string (`"core"`, `"cachyos-v4"`,
    /// `"aur"`, etc.) so the plan composition step can group by repo.
    pub to_install: Vec<(String, String)>,
    /// Packages that were once installed by Pearlite (`state.managed`)
    /// but are no longer declared. Candidates for `--prune`.
    pub forgotten: Vec<String>,
    /// Packages installed out-of-band: in `pacman -Qe` but not in
    /// `state.managed` and not in `state.adopted`. Surfaced as drift;
    /// never auto-removed.
    pub manual: Vec<String>,
    /// User-flagged "leave alone" — installed and listed in
    /// `state.adopted`. Suppressed entirely from drift output.
    pub adopted: Vec<String>,
    /// Packages declared in `remove.ignore` — never flagged or removed
    /// regardless of `state` membership.
    pub protected: Vec<String>,
}

/// Apply the four-way discriminator from PRD §7.3 plus the install/managed split.
#[must_use]
pub fn classify_pacman(
    declared_packages: &PackageSet,
    remove_policy: &RemovePolicy,
    probed: &PacmanInventory,
    state: &State,
) -> PacmanClassification {
    // Build (declared package -> repo string) lookup.
    let declared_repo = declared_repo_map(declared_packages);
    let declared: BTreeSet<&str> = declared_repo.keys().copied().collect();

    let installed = &probed.explicit;
    let managed_state: BTreeSet<&str> = state.managed.pacman.iter().map(String::as_str).collect();
    let adopted_state: BTreeSet<&str> = state.adopted.pacman.iter().map(String::as_str).collect();
    let ignore: BTreeSet<&str> = remove_policy.ignore.iter().map(String::as_str).collect();

    let mut managed = BTreeSet::new();
    let mut to_install = Vec::new();
    let mut forgotten = Vec::new();
    let mut manual = Vec::new();
    let mut adopted = Vec::new();
    let mut protected = Vec::new();

    // Walk declared packages — present or to be installed.
    for (&name, &repo) in &declared_repo {
        if installed.contains(name) {
            managed.insert(name.to_owned());
        } else {
            to_install.push((name.to_owned(), repo.to_owned()));
        }
    }

    // Walk installed packages — anything not declared falls into one of
    // four buckets in priority order: protected > adopted > forgotten >
    // manual.
    for installed_name in installed {
        let n = installed_name.as_str();
        if declared.contains(n) {
            continue; // already classified as managed above
        }
        if ignore.contains(n) {
            protected.push(installed_name.clone());
        } else if adopted_state.contains(n) {
            adopted.push(installed_name.clone());
        } else if managed_state.contains(n) {
            forgotten.push(installed_name.clone());
        } else {
            manual.push(installed_name.clone());
        }
    }

    // to_install is already in declared_repo's iteration order
    // (BTreeMap keys are sorted).

    PacmanClassification {
        managed,
        to_install,
        forgotten,
        manual,
        adopted,
        protected,
    }
}

/// Build a `name -> repo` lookup from a [`PackageSet`].
///
/// When a package appears in more than one declared list, the **first
/// list checked** wins (core > cachyos > cachyos-v3 > cachyos-v4 > aur).
/// The schema validator already flags duplicates as
/// [`ContractViolation::DUPLICATE_PACKAGES`](pearlite_schema::ContractViolation),
/// so this fallback only matters when validation has been bypassed.
fn declared_repo_map(packages: &PackageSet) -> BTreeMap<&str, &'static str> {
    let mut out = BTreeMap::new();
    for name in &packages.core {
        out.entry(name.as_str()).or_insert("core");
    }
    for name in &packages.cachyos {
        out.entry(name.as_str()).or_insert("cachyos");
    }
    for name in &packages.cachyos_v3 {
        out.entry(name.as_str()).or_insert("cachyos-v3");
    }
    for name in &packages.cachyos_v4 {
        out.entry(name.as_str()).or_insert("cachyos-v4");
    }
    for name in &packages.aur {
        out.entry(name.as_str()).or_insert("aur");
    }
    out
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
            reserved: std::collections::BTreeMap::new(),
        }
    }

    fn empty_inventory() -> PacmanInventory {
        PacmanInventory::default()
    }

    fn empty_remove() -> RemovePolicy {
        RemovePolicy::default()
    }

    fn pkg_set() -> PackageSet {
        PackageSet::default()
    }

    fn inventory_with(explicit: &[&str]) -> PacmanInventory {
        PacmanInventory {
            explicit: explicit.iter().map(|&s| s.to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn declared_and_installed_is_managed() {
        let mut decl = pkg_set();
        decl.core.push("htop".to_owned());
        let probed = inventory_with(&["htop"]);
        let state = empty_state();

        let c = classify_pacman(&decl, &empty_remove(), &probed, &state);
        assert!(c.managed.contains("htop"));
        assert!(c.to_install.is_empty());
        assert!(c.forgotten.is_empty());
        assert!(c.manual.is_empty());
    }

    #[test]
    fn declared_not_installed_is_to_install_with_repo() {
        let mut decl = pkg_set();
        decl.cachyos_v4.push("firefox".to_owned());
        let probed = empty_inventory();

        let c = classify_pacman(&decl, &empty_remove(), &probed, &empty_state());
        assert_eq!(
            c.to_install,
            vec![("firefox".to_owned(), "cachyos-v4".to_owned())]
        );
    }

    #[test]
    fn installed_was_managed_not_declared_is_forgotten() {
        let decl = pkg_set();
        let probed = inventory_with(&["xterm"]);
        let mut state = empty_state();
        state.managed.pacman = vec!["xterm".to_owned()];

        let c = classify_pacman(&decl, &empty_remove(), &probed, &state);
        assert_eq!(c.forgotten, vec!["xterm".to_owned()]);
        assert!(c.manual.is_empty());
    }

    #[test]
    fn installed_not_managed_not_adopted_is_manual() {
        let decl = pkg_set();
        let probed = inventory_with(&["vim"]);
        let state = empty_state();

        let c = classify_pacman(&decl, &empty_remove(), &probed, &state);
        assert_eq!(c.manual, vec!["vim".to_owned()]);
        assert!(c.forgotten.is_empty());
    }

    #[test]
    fn adopted_suppresses_drift() {
        let decl = pkg_set();
        let probed = inventory_with(&["vim"]);
        let mut state = empty_state();
        state.adopted.pacman = vec!["vim".to_owned()];

        let c = classify_pacman(&decl, &empty_remove(), &probed, &state);
        assert_eq!(c.adopted, vec!["vim".to_owned()]);
        assert!(
            c.manual.is_empty(),
            "adopted must not also appear as manual"
        );
        assert!(c.forgotten.is_empty());
    }

    #[test]
    fn remove_ignore_protects_packages_from_classification() {
        // Adopted + ignore → ignore wins (protected).
        let decl = pkg_set();
        let probed = inventory_with(&["nano"]);
        let mut state = empty_state();
        state.adopted.pacman = vec!["nano".to_owned()];
        let remove = RemovePolicy {
            ignore: vec!["nano".to_owned()],
            ..Default::default()
        };

        let c = classify_pacman(&decl, &remove, &probed, &state);
        assert_eq!(c.protected, vec!["nano".to_owned()]);
        assert!(c.adopted.is_empty(), "ignore takes precedence over adopted");
    }

    #[test]
    fn full_four_way_split() {
        // One package each in: managed (declared+installed), to_install
        // (declared, not installed), forgotten (installed, was-managed,
        // not declared), manual (installed, never managed), adopted
        // (installed, in adopted), protected (installed, in ignore).
        let mut decl = pkg_set();
        decl.core.push("base".to_owned()); // declared & installed → managed
        decl.cachyos_v4.push("firefox".to_owned()); // declared, not installed → to_install
        let probed = inventory_with(&["base", "xterm", "vim", "claude-code", "nano"]);
        let mut state = empty_state();
        state.managed.pacman = vec!["xterm".to_owned()]; // forgotten
        state.adopted.pacman = vec!["claude-code".to_owned()]; // adopted
        let remove = RemovePolicy {
            ignore: vec!["nano".to_owned()],
            ..Default::default()
        };

        let c = classify_pacman(&decl, &remove, &probed, &state);

        assert!(c.managed.contains("base"));
        assert_eq!(
            c.to_install,
            vec![("firefox".to_owned(), "cachyos-v4".to_owned())]
        );
        assert_eq!(c.forgotten, vec!["xterm".to_owned()]);
        assert_eq!(c.manual, vec!["vim".to_owned()]);
        assert_eq!(c.adopted, vec!["claude-code".to_owned()]);
        assert_eq!(c.protected, vec!["nano".to_owned()]);
    }
}
