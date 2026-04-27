// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Errors emitted by `pearlite-engine`.

use thiserror::Error;

/// Errors emitted while probing the live system.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// pacman/paru adapter failed.
    #[error(transparent)]
    Pacman(#[from] pearlite_pacman::PacmanError),
    /// cargo adapter failed.
    #[error(transparent)]
    Cargo(#[from] pearlite_cargo::CargoError),
    /// systemd adapter failed.
    #[error(transparent)]
    Systemd(#[from] pearlite_systemd::SystemdError),
    /// Filesystem I/O error reading host metadata.
    #[error("I/O error during probe: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors emitted by [`Engine::plan`](crate::Engine::plan) and friends.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Nickel evaluation failed.
    #[error(transparent)]
    Nickel(#[from] pearlite_nickel::NickelError),
    /// Schema validation produced contract violations.
    #[error("contract violations: {}", join_violations(.0))]
    ContractViolations(Vec<pearlite_schema::ContractViolation>),
    /// Probing the live system failed.
    #[error(transparent)]
    Probe(#[from] ProbeError),
    /// Filesystem operation (sha256, /etc inventory) failed.
    #[error(transparent)]
    Fs(#[from] pearlite_fs::FsError),
    /// state.toml read/write failed.
    #[error(transparent)]
    State(#[from] pearlite_state::StateError),
}

fn join_violations(v: &[pearlite_schema::ContractViolation]) -> String {
    v.iter()
        .map(|x| format!("{x}"))
        .collect::<Vec<_>>()
        .join("; ")
}
