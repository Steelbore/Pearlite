// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! pacman/paru adapter: inventory, repo classification, install, remove.
//!
//! Read side:
//! - [`Pacman::inventory`] — explicit + foreign packages with per-package
//!   repo classification.
//! - [`detect_arch_level`] — `/proc/cpuinfo` → [`ArchLevel`].
//! - [`Repo`] — typed repository identifier covering CachyOS's per-feature
//!   repos plus standard Arch + AUR.
//!
//! Apply side (matches the four pacman-side
//! [`Action`](pearlite_diff::Action) variants):
//! - [`Pacman::sync_databases`] — `pacman -Sy` (PRD §8.2 phase 0.5).
//! - [`Pacman::install`] — qualified `pacman -S <repo>/<pkg>`.
//! - [`Pacman::aur_install`] — `paru -S` for AUR sources.
//! - [`Pacman::remove`] — `pacman -R`.

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
