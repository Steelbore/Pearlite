// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! systemctl adapter: enable, disable, mask, restart for system and
//! user scopes.
//!
//! At M1 only the read side ([`Systemd::inventory`]) is implemented.
//! Apply-side methods (enable, disable, mask, restart) and user-scope
//! support arrive in M2 per Plan §7.3.

mod errors;
mod inventory;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;

pub use errors::SystemdError;
pub use inventory::{compose_inventory, parse_list_unit_files, parse_list_units};
pub use live::{LiveSystemd, Systemd};

#[cfg(feature = "test-mocks")]
pub use mock::MockSystemd;

#[doc(no_inline)]
pub use pearlite_schema::ServiceInventory;
