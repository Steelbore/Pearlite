// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pure diff engine: turns (declared, probed, state) into a Pearlite
//! [`Plan`].
//!
//! Chunk M1-W3-A lands the type layer only. The classification rules
//! (`classify_pacman`, `classify_cargo`, config/service drift), the
//! `within_phase_key` ordering, and the top-level `plan()` composition
//! arrive in subsequent chunks.

mod action;
mod classify;
mod coherence;
mod compose;
mod order;
mod phase;
mod plan;

pub use action::{Action, Phase, Scope};
pub use classify::{
    CargoClassification, ConfigClassification, ConfigFileDrift, PacmanClassification,
    ServicesClassification, UserEnvClassification, UserToSwitch, classify_cargo, classify_config,
    classify_pacman, classify_services, classify_user_env,
};
pub use coherence::FailureCoherence;
pub use compose::plan;
pub use phase::ApplyPhase;
pub use plan::{DriftCategory, DriftItem, Plan, Warning};
