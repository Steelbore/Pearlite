// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockCargo`] for unit tests and engine integration.

#![allow(
    clippy::missing_panics_doc,
    reason = "test utility; the only panic is on Mutex poisoning, unreachable in sane tests"
)]

use crate::errors::CargoError;
use crate::live::Cargo;
use pearlite_schema::CargoInventory;
use std::sync::{Arc, Mutex};

#[allow(
    clippy::expect_used,
    reason = "MockCargo is a test utility; mutex-poison panic is the standard \
              Mutex<T> idiom and unreachable in any sane test."
)]
mod inner {
    use super::CargoInventory;

    /// One recorded `install` call: `(crate_name, locked)`.
    pub type InstallCall = (String, bool);

    #[derive(Default, Debug)]
    pub struct State {
        pub inventory: CargoInventory,
        pub installs: Vec<InstallCall>,
        pub uninstalls: Vec<String>,
    }
}

use inner::{InstallCall, State};

/// Canned-inventory [`Cargo`] implementation for tests.
///
/// Compiled in `cargo test` (no feature) and behind `feature =
/// "test-mocks"` for downstream consumers.
///
/// Apply-side methods record their call arguments instead of
/// shelling out — `MockCargo::install_history` and
/// `MockCargo::uninstall_history` let assertions inspect what the
/// engine asked for.
#[derive(Clone, Debug, Default)]
pub struct MockCargo {
    state: Arc<Mutex<State>>,
}

impl MockCargo {
    /// Construct an empty [`MockCargo`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`MockCargo`] pre-seeded with the given inventory.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn with_inventory(inventory: CargoInventory) -> Self {
        let me = Self::default();
        me.state
            .lock()
            .expect("MockCargo mutex must not be poisoned")
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
            .expect("MockCargo mutex must not be poisoned")
            .installs
            .clone()
    }

    /// Inspect the recorded `uninstall` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn uninstall_history(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("MockCargo mutex must not be poisoned")
            .uninstalls
            .clone()
    }
}

impl Cargo for MockCargo {
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn inventory(&self) -> Result<CargoInventory, CargoError> {
        Ok(self
            .state
            .lock()
            .expect("MockCargo mutex must not be poisoned")
            .inventory
            .clone())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn install(&self, crate_name: &str, locked: bool) -> Result<(), CargoError> {
        self.state
            .lock()
            .expect("MockCargo mutex must not be poisoned")
            .installs
            .push((crate_name.to_owned(), locked));
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn uninstall(&self, crate_name: &str) -> Result<(), CargoError> {
        self.state
            .lock()
            .expect("MockCargo mutex must not be poisoned")
            .uninstalls
            .push(crate_name.to_owned());
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
    use std::collections::BTreeMap;

    fn sample_inventory() -> CargoInventory {
        let mut crates = BTreeMap::new();
        crates.insert("zellij".to_owned(), "0.41.2".to_owned());
        crates.insert("ripgrep-all".to_owned(), "0.10.6".to_owned());
        CargoInventory { crates }
    }

    #[test]
    fn empty_mock_yields_empty_inventory() {
        let mock = MockCargo::new();
        let inv = mock.inventory().expect("inventory");
        assert!(inv.crates.is_empty());
    }

    #[test]
    fn with_inventory_round_trips() {
        let seeded = sample_inventory();
        let mock = MockCargo::with_inventory(seeded.clone());
        let inv = mock.inventory().expect("inventory");
        assert_eq!(inv.crates, seeded.crates);
    }

    #[test]
    fn install_records_crate_and_locked_flag() {
        let mock = MockCargo::new();
        mock.install("zellij", false).expect("install");
        mock.install("ripgrep", true).expect("install");
        assert_eq!(
            mock.install_history(),
            vec![("zellij".to_owned(), false), ("ripgrep".to_owned(), true),]
        );
    }

    #[test]
    fn uninstall_records_crate() {
        let mock = MockCargo::new();
        mock.uninstall("zellij").expect("uninstall");
        mock.uninstall("ripgrep").expect("uninstall");
        assert_eq!(
            mock.uninstall_history(),
            vec!["zellij".to_owned(), "ripgrep".to_owned()]
        );
    }
}
