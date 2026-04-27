// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Resolved configuration and probed-state types for Pearlite.
//!
//! This crate is pure: no I/O, no subprocess, no clock except via fields the
//! caller fills in. It defines the data layer that every adapter, the engine,
//! and the diff machinery share — see Plan §6.1.
//!
//! `from_resolved_toml` and `validate` ship in chunk M1-W1-B; this scaffold
//! lands the public type definitions only.

mod config;
mod declared;
mod errors;
mod host;
mod packages;
mod probed;
mod services;
mod snapshots;
mod users;

pub use config::{ConfigEntry, RemovePolicy};
pub use declared::DeclaredState;
pub use errors::{ContractViolation, SchemaError};
pub use host::{ArchLevel, HostMeta, KernelDecl};
pub use packages::PackageSet;
pub use probed::{
    CargoInventory, ConfigFileInventory, ConfigFileMeta, HostInfo, KernelInfo, PacmanInventory,
    ProbedState, ServiceInventory,
};
pub use services::ServicesDecl;
pub use snapshots::SnapshotPolicy;
pub use users::{HomeManagerDecl, HomeManagerMode, UserDecl};
