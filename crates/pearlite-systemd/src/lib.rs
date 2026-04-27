// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! systemctl adapter: enable, disable, mask, restart for system and
//! user scopes.
//!
//! Read side: [`Systemd::inventory`] parses `list-unit-files` and
//! `list-units`.
//! Apply side: [`Systemd::enable`], [`Systemd::disable`],
//! [`Systemd::mask`], [`Systemd::restart`] match the four service-side
//! [`Action`](pearlite_diff::Action) variants 1:1. User-scope ops
//! dispatch through `runuser -u <name> -- systemctl --user ...`.

mod errors;
mod inventory;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;

pub use errors::SystemdError;
pub use inventory::{compose_inventory, parse_list_unit_files, parse_list_units};
pub use live::{LiveSystemd, Scope, Systemd};

#[cfg(feature = "test-mocks")]
pub use mock::MockSystemd;

#[doc(no_inline)]
pub use pearlite_schema::ServiceInventory;
