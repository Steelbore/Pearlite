// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockPacman`] for unit tests and engine integration.

use crate::errors::PacmanError;
use crate::live::Pacman;
use pearlite_schema::PacmanInventory;

/// Canned-inventory [`Pacman`] implementation for tests.
///
/// Compiled in `cargo test` (no feature) and behind `feature =
/// "test-mocks"` for downstream consumers (the engine's integration
/// tests in M2+).
#[derive(Clone, Debug, Default)]
pub struct MockPacman {
    inventory: PacmanInventory,
}

impl MockPacman {
    /// Construct an empty [`MockPacman`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`MockPacman`] pre-seeded with the given inventory.
    #[must_use]
    pub fn with_inventory(inventory: PacmanInventory) -> Self {
        Self { inventory }
    }
}

impl Pacman for MockPacman {
    fn inventory(&self) -> Result<PacmanInventory, PacmanError> {
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
}
