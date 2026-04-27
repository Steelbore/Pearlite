// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! cargo adapter: list installed crates; install and uninstall on apply.
//!
//! Read side: [`Cargo::inventory`] parses `cargo install --list`.
//! Apply side: [`Cargo::install`] / [`Cargo::uninstall`] match the
//! [`CargoInstall`](pearlite_diff::Action::CargoInstall) /
//! [`CargoUninstall`](pearlite_diff::Action::CargoUninstall)
//! `Action` variants 1:1.

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
