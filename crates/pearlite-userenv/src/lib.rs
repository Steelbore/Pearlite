// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! User-environment adapter: Home Manager via `runuser`.
//!
//! Plan §6.10 / §7.4 — the apply-side interface for phase 7
//! (PRD §8.2). The engine never spawns `home-manager` directly;
//! every invocation goes through this crate's
//! [`HomeManagerBackend`] trait. CLAUDE.md "no shell" applies:
//! [`LiveHmBackend`] uses [`std::process::Command`] with argv
//! arrays to exec `runuser -u <user> -- home-manager switch ...`.
//!
//! Engine integration (phase 7 wiring) lands in M3 W2; this scaffold
//! ships the trait + production / mock implementations the apply
//! orchestrator will consume.

mod errors;
mod live;
#[cfg(any(test, feature = "test-mocks"))]
mod mock;

pub use errors::UserenvError;
pub use live::{HomeManagerBackend, LiveHmBackend, UserEnvOutcome, parse_generation_from_switch};

#[cfg(feature = "test-mocks")]
pub use mock::{MockHmBackend, SwitchInvocation};
