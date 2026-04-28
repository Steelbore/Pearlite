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
mod nix;
mod packages;
mod parse;
mod probed;
mod services;
mod snapshots;
mod users;
mod validate;

pub use config::{ConfigEntry, RemovePolicy};
pub use declared::DeclaredState;
pub use errors::{ContractViolation, SchemaError};
pub use host::{ArchLevel, HostMeta, KernelDecl};
pub use nix::{NixDecl, NixInstallerDecl};
pub use packages::PackageSet;
pub use parse::from_resolved_toml;
pub use probed::{
    CargoInventory, ConfigFileInventory, ConfigFileMeta, HostInfo, KernelInfo, PacmanInventory,
    ProbedState, ServiceInventory,
};
pub use services::ServicesDecl;
pub use snapshots::SnapshotPolicy;
pub use users::{HomeManagerDecl, HomeManagerMode, UserDecl};
pub use validate::validate;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests may use expect()/unwrap()/panic!() per Plan §4.2 + CLAUDE.md"
)]
mod cross_cutting_tests {
    use super::*;

    const FULL: &str = include_str!("../../../fixtures/schema/host_full.toml");

    /// Plan §6.1 acceptance: emitted JSON Schema is Draft 2020-12.
    #[test]
    fn schemars_output_is_draft_2020_12() {
        let schema = schemars::schema_for!(DeclaredState);
        let json = serde_json::to_value(&schema).expect("serialize schema");
        let dollar_schema = json
            .get("$schema")
            .and_then(|v| v.as_str())
            .expect("$schema field present in emitted JSON Schema");
        assert!(
            dollar_schema.contains("2020-12"),
            "expected Draft 2020-12, got: {dollar_schema}"
        );
    }

    /// Plan §6.1 acceptance: a representative resolved.toml fixture
    /// round-trips without loss.
    #[test]
    fn declared_state_round_trips_through_toml() {
        let original = from_resolved_toml(FULL).expect("parse full fixture");
        let serialized = toml::to_string(&original).expect("serialize");
        let reparsed = from_resolved_toml(&serialized).expect("re-parse serialized");
        assert_eq!(original, reparsed, "TOML round-trip must be lossless");
    }
}
