// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Per-subsystem classification: declared vs probed vs state.
//!
//! Each `classify_*` function is pure — no I/O, no clock, no spawn.
//! Outputs feed [`crate::plan`] (chunk M1-W3-D) which sequences them
//! into [`Action`](crate::Action) instances.

mod cargo;
mod config;
mod pacman;
mod services;

pub use cargo::{CargoClassification, classify_cargo};
pub use config::{ConfigClassification, ConfigFileDrift, classify_config};
pub use pacman::{PacmanClassification, classify_pacman};
pub use services::{ServicesClassification, classify_services};
