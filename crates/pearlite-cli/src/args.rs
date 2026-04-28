// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! clap argument structures.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use uuid::Uuid;

/// Top-level Pearlite CLI argument structure.
#[derive(Parser, Debug)]
#[command(name = "pearlite", version, about)]
pub struct Args {
    /// Output format. `auto` selects `human` on a TTY and `json` when
    /// stdout is piped.
    #[arg(long, value_enum, default_value_t = OutputFormat::Auto, global = true)]
    pub format: OutputFormat,

    /// Pearlite config repository root.
    #[arg(
        long,
        env = "PEARLITE_CONFIG_DIR",
        default_value = "/etc/pearlite/repo",
        global = true
    )]
    pub config_dir: PathBuf,

    /// State file path.
    #[arg(
        long,
        env = "PEARLITE_STATE_FILE",
        default_value = "/var/lib/pearlite/state.toml",
        global = true
    )]
    pub state_file: PathBuf,

    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommand selector.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Compute the diff between declared and live state.
    Plan {
        /// Host file to evaluate. Defaults to
        /// `<config_dir>/hosts/<hostname>.ncl`.
        #[arg(long)]
        host_file: Option<PathBuf>,
    },
    /// Read-only diff summary, optimized for speed.
    Status {
        /// Host file to evaluate. Defaults to
        /// `<config_dir>/hosts/<hostname>.ncl`.
        #[arg(long)]
        host_file: Option<PathBuf>,
    },
    /// Apply the diff between declared and live state.
    ///
    /// Wraps the apply in pre/post Snapper snapshots, runs every
    /// action in PRD §8.2 phase order, and commits a `[[history]]`
    /// entry to `state.toml` (phase 9 — last write).
    Apply {
        /// Host file to evaluate. Defaults to
        /// `<config_dir>/hosts/<hostname>.ncl`.
        #[arg(long)]
        host_file: Option<PathBuf>,
        /// Snapper config to take pre/post snapshots against.
        #[arg(long, default_value = "root")]
        snapper_config: String,
        /// Directory for forensic JSON failure records. Defaults to
        /// `<state_file dir>/failures` (typically
        /// `/var/lib/pearlite/failures`).
        #[arg(long)]
        failures_dir: Option<PathBuf>,
        /// Plan but don't execute. Returns the same envelope shape as
        /// `pearlite plan`.
        #[arg(long)]
        dry_run: bool,
    },
    /// Inspect Pearlite's apply history (a.k.a. generations).
    ///
    /// Read-only. Each generation in `state.toml`'s `[[history]]`
    /// corresponds to one successful `pearlite apply`.
    Gen {
        /// Sub-action against the generation history.
        #[command(subcommand)]
        gen_command: GenCommand,
    },
    /// Roll back a previously applied plan.
    ///
    /// Looks up the `[[history]]` entry by `plan_id` and reverts the
    /// root subvolume to the entry's pre-apply Snapper snapshot. The
    /// next `pearlite plan` re-derives state from the live system.
    Rollback {
        /// Plan UUID to roll back to (the entry's `snapshot_pre`
        /// snapshot is what gets restored).
        plan_id: Uuid,
        /// Snapper config to roll back. Must match the config the
        /// original apply used (typically `"root"`).
        #[arg(long, default_value = "root")]
        snapper_config: String,
    },
    /// Emit JSON Schema describing the CLI surface.
    Schema {
        /// Emit a minimal placeholder schema (M1 scope).
        #[arg(long)]
        bare: bool,
    },
}

/// Sub-actions for [`Command::Gen`].
#[derive(Subcommand, Debug)]
pub enum GenCommand {
    /// List every generation in the history (oldest first).
    List,
    /// Show one generation by `plan_id`.
    Show {
        /// Plan UUID to display.
        plan_id: Uuid,
    },
}

/// Selected output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Choose `human` on a TTY, `json` otherwise.
    Auto,
    /// Pretty-printed TTY output.
    Human,
    /// Single-envelope JSON output (canonical agent format).
    Json,
    /// Streaming JSONL events (long-running ops).
    Jsonl,
}
