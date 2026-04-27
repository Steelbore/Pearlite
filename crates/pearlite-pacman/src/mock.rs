// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockPacman`] for unit tests and engine integration.

#![allow(
    clippy::missing_panics_doc,
    reason = "test utility; the only panic is on Mutex poisoning, unreachable in sane tests"
)]

use crate::errors::PacmanError;
use crate::live::Pacman;
use pearlite_schema::PacmanInventory;
use std::sync::{Arc, Mutex};

#[allow(
    clippy::expect_used,
    reason = "MockPacman is a test utility; mutex-poison panic is the standard \
              Mutex<T> idiom and unreachable in any sane test."
)]
mod inner {
    use super::PacmanInventory;

    /// One recorded `install` call: `(repo, packages)`.
    pub type InstallCall = (String, Vec<String>);

    #[derive(Default, Debug)]
    pub struct State {
        pub inventory: PacmanInventory,
        pub syncs: u32,
        pub installs: Vec<InstallCall>,
        pub aur_installs: Vec<Vec<String>>,
        pub removes: Vec<Vec<String>>,
    }
}

use inner::{InstallCall, State};

/// Canned-inventory [`Pacman`] implementation for tests.
///
/// Compiled in `cargo test` (no feature) and behind `feature =
/// "test-mocks"` for downstream consumers (the engine's integration
/// tests in M2+).
///
/// Apply-side methods record their call arguments instead of
/// shelling out — `MockPacman::install_history`, `aur_install_history`,
/// `remove_history`, and `sync_count` let assertions inspect what
/// the engine asked for.
#[derive(Clone, Debug, Default)]
pub struct MockPacman {
    state: Arc<Mutex<State>>,
}

impl MockPacman {
    /// Construct an empty [`MockPacman`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`MockPacman`] pre-seeded with the given inventory.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn with_inventory(inventory: PacmanInventory) -> Self {
        let me = Self::default();
        me.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .inventory = inventory;
        me
    }

    /// Inspect the recorded `install` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn install_history(&self) -> Vec<InstallCall> {
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .installs
            .clone()
    }

    /// Inspect the recorded `aur_install` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn aur_install_history(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .aur_installs
            .clone()
    }

    /// Inspect the recorded `remove` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn remove_history(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .removes
            .clone()
    }

    /// Number of times `sync_databases` has been invoked.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn sync_count(&self) -> u32 {
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .syncs
    }
}

impl Pacman for MockPacman {
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn inventory(&self) -> Result<PacmanInventory, PacmanError> {
        Ok(self
            .state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .inventory
            .clone())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn sync_databases(&self) -> Result<(), PacmanError> {
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .syncs += 1;
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn install(&self, repo: &str, packages: &[&str]) -> Result<(), PacmanError> {
        if packages.is_empty() {
            return Ok(());
        }
        let owned: Vec<String> = packages.iter().map(|p| (*p).to_owned()).collect();
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .installs
            .push((repo.to_owned(), owned));
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn aur_install(&self, packages: &[&str]) -> Result<(), PacmanError> {
        if packages.is_empty() {
            return Ok(());
        }
        let owned: Vec<String> = packages.iter().map(|p| (*p).to_owned()).collect();
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .aur_installs
            .push(owned);
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn remove(&self, packages: &[&str]) -> Result<(), PacmanError> {
        if packages.is_empty() {
            return Ok(());
        }
        let owned: Vec<String> = packages.iter().map(|p| (*p).to_owned()).collect();
        self.state
            .lock()
            .expect("MockPacman mutex must not be poisoned")
            .removes
            .push(owned);
        Ok(())
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
    use std::collections::{BTreeMap, BTreeSet};

    fn sample() -> PacmanInventory {
        let mut explicit = BTreeSet::new();
        explicit.insert("htop".to_owned());
        let mut repos = BTreeMap::new();
        repos.insert("htop".to_owned(), "extra".to_owned());
        PacmanInventory {
            explicit,
            foreign: BTreeSet::new(),
            repos,
        }
    }

    #[test]
    fn empty_mock_yields_empty_inventory() {
        let mock = MockPacman::new();
        let inv = mock.inventory().expect("inventory");
        assert!(inv.explicit.is_empty());
        assert!(inv.repos.is_empty());
    }

    #[test]
    fn with_inventory_round_trips() {
        let seeded = sample();
        let mock = MockPacman::with_inventory(seeded.clone());
        let inv = mock.inventory().expect("inventory");
        assert_eq!(inv.explicit, seeded.explicit);
        assert_eq!(inv.repos, seeded.repos);
    }

    #[test]
    fn sync_databases_increments_count() {
        let mock = MockPacman::new();
        assert_eq!(mock.sync_count(), 0);
        mock.sync_databases().expect("sync");
        mock.sync_databases().expect("sync");
        assert_eq!(mock.sync_count(), 2);
    }

    #[test]
    fn install_records_repo_and_packages() {
        let mock = MockPacman::new();
        mock.install("extra", &["htop", "ripgrep"])
            .expect("install");
        mock.install("cachyos-v3", &["firefox"]).expect("install");
        let history = mock.install_history();
        assert_eq!(
            history,
            vec![
                (
                    "extra".to_owned(),
                    vec!["htop".to_owned(), "ripgrep".to_owned()]
                ),
                ("cachyos-v3".to_owned(), vec!["firefox".to_owned()]),
            ]
        );
    }

    #[test]
    fn install_empty_slice_is_noop() {
        let mock = MockPacman::new();
        mock.install("extra", &[]).expect("install");
        assert!(mock.install_history().is_empty());
    }

    #[test]
    fn aur_install_records_packages() {
        let mock = MockPacman::new();
        mock.aur_install(&["yay", "paru"]).expect("aur_install");
        let history = mock.aur_install_history();
        assert_eq!(history, vec![vec!["yay".to_owned(), "paru".to_owned()]]);
    }

    #[test]
    fn aur_install_empty_slice_is_noop() {
        let mock = MockPacman::new();
        mock.aur_install(&[]).expect("aur_install");
        assert!(mock.aur_install_history().is_empty());
    }

    #[test]
    fn remove_records_packages() {
        let mock = MockPacman::new();
        mock.remove(&["htop"]).expect("remove");
        mock.remove(&["ripgrep", "fd"]).expect("remove");
        let history = mock.remove_history();
        assert_eq!(
            history,
            vec![
                vec!["htop".to_owned()],
                vec!["ripgrep".to_owned(), "fd".to_owned()],
            ]
        );
    }

    #[test]
    fn remove_empty_slice_is_noop() {
        let mock = MockPacman::new();
        mock.remove(&[]).expect("remove");
        assert!(mock.remove_history().is_empty());
    }
}
