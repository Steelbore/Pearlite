// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockSystemd`] for unit tests and engine integration.

#![allow(
    clippy::missing_panics_doc,
    reason = "test utility; the only panic is on Mutex poisoning, unreachable in sane tests"
)]

use crate::errors::SystemdError;
use crate::live::{Scope, Systemd};
use pearlite_schema::ServiceInventory;
use std::sync::{Arc, Mutex};

#[allow(
    clippy::expect_used,
    reason = "MockSystemd is a test utility; mutex-poison panic is the standard \
              Mutex<T> idiom and unreachable in any sane test."
)]
mod inner {
    use super::{Scope, ServiceInventory};

    /// One recorded `enable` / `disable` call: `(unit, scope)`.
    pub type ScopedCall = (String, Scope);

    #[derive(Default, Debug)]
    pub struct State {
        pub inventory: ServiceInventory,
        pub enables: Vec<ScopedCall>,
        pub disables: Vec<ScopedCall>,
        pub masks: Vec<String>,
        pub restarts: Vec<String>,
    }
}

use inner::{ScopedCall, State};

/// Canned-inventory [`Systemd`] implementation for tests.
///
/// Compiled in `cargo test` (no feature) and behind `feature =
/// "test-mocks"` for downstream consumers.
///
/// Apply-side methods record their call arguments instead of
/// shelling out — `MockSystemd::enable_history`,
/// `disable_history`, `mask_history`, and `restart_history` let
/// assertions inspect what the engine asked for.
#[derive(Clone, Debug, Default)]
pub struct MockSystemd {
    state: Arc<Mutex<State>>,
}

impl MockSystemd {
    /// Construct an empty [`MockSystemd`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`MockSystemd`] pre-seeded with the given inventory.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn with_inventory(inventory: ServiceInventory) -> Self {
        let me = Self::default();
        me.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .inventory = inventory;
        me
    }

    /// Inspect the recorded `enable` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn enable_history(&self) -> Vec<ScopedCall> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .enables
            .clone()
    }

    /// Inspect the recorded `disable` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn disable_history(&self) -> Vec<ScopedCall> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .disables
            .clone()
    }

    /// Inspect the recorded `mask` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn mask_history(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .masks
            .clone()
    }

    /// Inspect the recorded `restart` calls in order.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn restart_history(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .restarts
            .clone()
    }
}

impl Systemd for MockSystemd {
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn inventory(&self) -> Result<ServiceInventory, SystemdError> {
        Ok(self
            .state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .inventory
            .clone())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn enable(&self, unit: &str, scope: &Scope) -> Result<(), SystemdError> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .enables
            .push((unit.to_owned(), scope.clone()));
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn disable(&self, unit: &str, scope: &Scope) -> Result<(), SystemdError> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .disables
            .push((unit.to_owned(), scope.clone()));
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn mask(&self, unit: &str) -> Result<(), SystemdError> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .masks
            .push(unit.to_owned());
        Ok(())
    }

    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn restart(&self, unit: &str) -> Result<(), SystemdError> {
        self.state
            .lock()
            .expect("MockSystemd mutex must not be poisoned")
            .restarts
            .push(unit.to_owned());
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

    #[test]
    fn enable_records_unit_and_scope() {
        let mock = MockSystemd::new();
        mock.enable("nginx.service", &Scope::System)
            .expect("enable");
        mock.enable(
            "syncthing.service",
            &Scope::User {
                name: "alice".to_owned(),
            },
        )
        .expect("enable");
        assert_eq!(
            mock.enable_history(),
            vec![
                ("nginx.service".to_owned(), Scope::System),
                (
                    "syncthing.service".to_owned(),
                    Scope::User {
                        name: "alice".to_owned()
                    }
                ),
            ]
        );
    }

    #[test]
    fn disable_records_unit_and_scope() {
        let mock = MockSystemd::new();
        mock.disable("nginx.service", &Scope::System)
            .expect("disable");
        assert_eq!(
            mock.disable_history(),
            vec![("nginx.service".to_owned(), Scope::System)]
        );
    }

    #[test]
    fn mask_records_unit() {
        let mock = MockSystemd::new();
        mock.mask("nginx.service").expect("mask");
        mock.mask("apache.service").expect("mask");
        assert_eq!(
            mock.mask_history(),
            vec!["nginx.service".to_owned(), "apache.service".to_owned()]
        );
    }

    #[test]
    fn restart_records_unit() {
        let mock = MockSystemd::new();
        mock.restart("nginx.service").expect("restart");
        assert_eq!(mock.restart_history(), vec!["nginx.service".to_owned()]);
    }
}
