// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockCargo`] for unit tests and engine integration.

use crate::errors::CargoError;
use crate::live::Cargo;
use pearlite_schema::CargoInventory;

/// Canned-inventory [`Cargo`] implementation for tests.
///
/// Compiled in `cargo test` (no feature) and behind `feature =
/// "test-mocks"` for downstream consumers.
#[derive(Clone, Debug, Default)]
pub struct MockCargo {
    inventory: CargoInventory,
}

impl MockCargo {
    /// Construct an empty [`MockCargo`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`MockCargo`] pre-seeded with the given inventory.
    #[must_use]
    pub fn with_inventory(inventory: CargoInventory) -> Self {
        Self { inventory }
    }
}

impl Cargo for MockCargo {
    fn inventory(&self) -> Result<CargoInventory, CargoError> {
        Ok(self.inventory.clone())
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
}
