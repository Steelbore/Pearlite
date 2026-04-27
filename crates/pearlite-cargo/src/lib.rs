// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! cargo adapter: list installed crates; install and uninstall on apply.
//!
//! At M1 only the read side ([`Cargo::inventory`]) is implemented.
//! Install/uninstall land in M2 with the rest of the apply-engine
//! adapters per Plan §7.3.

mod errors;
mod inventory;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;

pub use errors::CargoError;
pub use inventory::parse_install_list;
pub use live::{Cargo, LiveCargo};

#[cfg(feature = "test-mocks")]
pub use mock::MockCargo;

#[doc(no_inline)]
pub use pearlite_schema::CargoInventory;
