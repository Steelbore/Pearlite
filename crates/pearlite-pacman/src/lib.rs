// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! pacman/paru adapter: inventory, repo classification, install, remove.
//!
//! At M1 only the read side is implemented:
//! - [`Pacman::inventory`] — explicit + foreign packages with per-package
//!   repo classification.
//! - [`detect_arch_level`] — `/proc/cpuinfo` → [`ArchLevel`].
//! - [`Repo`] — typed repository identifier covering CachyOS's per-feature
//!   repos plus standard Arch + AUR.
//!
//! Apply-side methods (`install`, `remove`, `sync_databases`) arrive in M2
//! per Plan §7.3.

mod errors;
mod inventory;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;
mod repos;

pub use errors::PacmanError;
pub use inventory::{compose_inventory, parse_qe, parse_qm, parse_sl};
pub use live::{LivePacman, Pacman};
pub use repos::{Repo, detect_arch_level, parse_pacman_conf};

#[cfg(feature = "test-mocks")]
pub use mock::MockPacman;

#[doc(no_inline)]
pub use pearlite_schema::PacmanInventory;
