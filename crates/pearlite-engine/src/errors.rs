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

/// Errors emitted while applying a [`Plan`](pearlite_diff::Plan).
///
/// Each variant wraps the failure of one adapter; the apply orchestrator
/// halts on the first error, so the variant identifies the failing
/// subsystem. PRD §8.5 failure-class mapping is performed at the CLI
/// boundary using [`pearlite_diff::Action::failure_coherence`] on the
/// action that triggered the error.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// Snapper adapter failed (pre / post snapshot).
    #[error(transparent)]
    Snapper(#[from] pearlite_snapper::SnapperError),
    /// pacman/paru adapter failed (`sync_databases` / install / remove).
    #[error(transparent)]
    Pacman(#[from] pearlite_pacman::PacmanError),
    /// cargo adapter failed (install / uninstall).
    #[error(transparent)]
    Cargo(#[from] pearlite_cargo::CargoError),
    /// systemd adapter failed (enable / disable / mask / restart).
    #[error(transparent)]
    Systemd(#[from] pearlite_systemd::SystemdError),
    /// Filesystem operation (sha256, atomic write) failed during
    /// phase-4 `ConfigWrite`.
    #[error(transparent)]
    Fs(#[from] pearlite_fs::FsError),
    /// `ConfigWrite` source file's SHA-256 changed between plan and
    /// apply. Class 3 recoverable: re-plan and retry.
    #[error(
        "config source SHA-256 mismatch for {target}: planned {planned}, found {actual} (re-plan and retry)"
    )]
    ContentSha256Mismatch {
        /// Target path the action wanted to write.
        target: std::path::PathBuf,
        /// SHA-256 the planner recorded.
        planned: String,
        /// SHA-256 actually read at apply time.
        actual: String,
    },
    /// `state.toml` read or write failed during the phase-9 commit.
    #[error(transparent)]
    State(#[from] pearlite_state::StateError),
}

/// Errors emitted by [`Engine::rollback`](crate::Engine::rollback).
///
/// Rollback is the user-driven counterpart to apply: the operator
/// invokes it explicitly after a Class 3/4 failure (PRD §8.5,
/// CLAUDE.md hard invariant 9). Each variant identifies whether the
/// failure was on the bookkeeping side (state read, plan lookup) or
/// the system side (snapper revert).
#[derive(Debug, Error)]
pub enum RollbackError {
    /// `state.toml` could not be read.
    #[error(transparent)]
    State(#[from] pearlite_state::StateError),
    /// Snapper adapter failed.
    #[error(transparent)]
    Snapper(#[from] pearlite_snapper::SnapperError),
    /// No `[[history]]` entry matches the requested plan ID.
    #[error("no apply with plan_id {plan_id} found in state.toml history")]
    PlanNotFound {
        /// Plan UUID the caller asked to roll back to.
        plan_id: uuid::Uuid,
    },
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
