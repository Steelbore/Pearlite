// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockSystemd`] for unit tests and engine integration.

use crate::errors::SystemdError;
use crate::live::Systemd;
use pearlite_schema::ServiceInventory;

/// Canned-inventory [`Systemd`] implementation for tests.
///
/// Compiled in `cargo test` (no feature) and behind `feature =
/// "test-mocks"` for downstream consumers.
#[derive(Clone, Debug, Default)]
pub struct MockSystemd {
    inventory: ServiceInventory,
}

impl MockSystemd {
    /// Construct an empty [`MockSystemd`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`MockSystemd`] pre-seeded with the given inventory.
    #[must_use]
    pub fn with_inventory(inventory: ServiceInventory) -> Self {
        Self { inventory }
    }
}

impl Systemd for MockSystemd {
    fn inventory(&self) -> Result<ServiceInventory, SystemdError> {
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
    use std::collections::BTreeSet;

    fn sample() -> ServiceInventory {
        let mut enabled = BTreeSet::new();
        enabled.insert("nginx.service".to_owned());
        let mut active = BTreeSet::new();
        active.insert("nginx.service".to_owned());
        ServiceInventory {
            enabled,
            disabled: BTreeSet::new(),
            masked: BTreeSet::new(),
            active,
        }
    }

    #[test]
    fn empty_mock_yields_empty_inventory() {
        let mock = MockSystemd::new();
        let inv = mock.inventory().expect("inventory");
        assert!(inv.enabled.is_empty());
        assert!(inv.active.is_empty());
    }

    #[test]
    fn with_inventory_round_trips() {
        let seeded = sample();
        let mock = MockSystemd::with_inventory(seeded.clone());
        let inv = mock.inventory().expect("inventory");
        assert_eq!(inv.enabled, seeded.enabled);
        assert_eq!(inv.active, seeded.active);
    }
}
