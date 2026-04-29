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
    /// Home Manager / user-env adapter failed during phase-7 dispatch.
    #[error(transparent)]
    Userenv(#[from] pearlite_userenv::UserenvError),
    /// Determinate Nix installer adapter failed while probing nix
    /// presence during the apply preflight (ADR-0012 decision 3).
    #[error(transparent)]
    NixProbe(#[from] pearlite_userenv::InstallerError),
    /// Apply preflight: plan would run a `UserEnvSwitch` but
    /// `nix --version` fails. Class 1 (preflight) — bootstrap first.
    #[error("nix is not installed; run `pearlite bootstrap` first")]
    NixNotInstalled,
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

/// Errors emitted by [`Engine::bootstrap`](crate::Engine::bootstrap)
/// (ADR-0012).
#[derive(Debug, Error)]
pub enum BootstrapError {
    /// Nickel evaluator failed loading the host file.
    #[error(transparent)]
    Nickel(#[from] pearlite_nickel::NickelError),
    /// The declared host has no `[nix.installer]` block; bootstrap
    /// makes no sense for hosts that don't need nix. Hint: declare
    /// `nix.installer.expected_sha256` or skip this command.
    #[error(
        "host file has no [nix.installer] block; declare nix.installer.expected_sha256 to bootstrap"
    )]
    NixNotDeclared,
    /// Determinate Nix installer adapter failed (SHA mismatch,
    /// non-zero script exit, missing shell, etc.).
    #[error(transparent)]
    Installer(#[from] pearlite_userenv::InstallerError),
    /// Filesystem operation (atomic write of `/etc/nix/nix.conf`)
    /// failed.
    #[error(transparent)]
    Fs(#[from] pearlite_fs::FsError),
    /// Reading existing `/etc/nix/nix.conf` failed for a reason other
    /// than "not found".
    #[error("failed to read /etc/nix/nix.conf: {0}")]
    Io(#[source] std::io::Error),
}

/// Errors emitted by [`Engine::reconcile`](crate::Engine::reconcile)
/// (PRD §11, M4 W1).
///
/// `reconcile` is read-only: it probes the live system and writes a
/// fresh `<config_dir>/hosts/<hostname>.imported.ncl` for the operator
/// to review. Variants identify whether the failure was on the probe
/// side, validation of the probed hostname, or the filesystem write.
#[derive(Debug, Error)]
pub enum ReconcileError {
    /// Probing the live system failed.
    #[error(transparent)]
    Probe(#[from] ProbeError),
    /// Probe returned an empty hostname; reconcile refuses rather than
    /// landing the import at `<config_dir>/hosts/.imported.ncl`.
    #[error(
        "probe returned an empty hostname; set /etc/hostname before running `pearlite reconcile`"
    )]
    EmptyHostname,
    /// Probed hostname contains characters disallowed in a filename
    /// (path separators, NUL). RFC 1123 hostnames are already
    /// constrained, so this only fires on malformed system state.
    #[error("probed hostname {hostname:?} is not a valid filename component")]
    InvalidHostname {
        /// The offending hostname value as probed.
        hostname: String,
    },
    /// Target `.imported.ncl` already exists. Reconcile refuses to
    /// clobber operator review state; the operator removes or renames
    /// the existing file and retries.
    #[error("{path} already exists; remove or rename it before re-running reconcile")]
    AlreadyExists {
        /// Path that would have been written.
        path: std::path::PathBuf,
    },
    /// Filesystem operation (mkdir, atomic write) failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: std::path::PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
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
