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
        /// Directory where plan JSON files are persisted. Each plan
        /// lands at `<plans_dir>/<plan-id>.json` so `gen show` and
        /// future `--plan-file` consumers can recover the full
        /// per-action breakdown that `[[history]]` only summarizes.
        /// Defaults to `<state_file dir>/plans` (typically
        /// `/var/lib/pearlite/plans`).
        #[arg(long)]
        plans_dir: Option<PathBuf>,
        /// Plan but don't execute. Returns the same envelope shape as
        /// `pearlite plan`.
        #[arg(long)]
        dry_run: bool,
        /// Apply a previously persisted plan instead of computing a
        /// fresh one. The file must be a JSON-serialized
        /// `pearlite_diff::Plan` whose schema matches the current
        /// build (ADR-0010 §Decision).
        #[arg(long, conflicts_with = "dry_run")]
        plan_file: Option<PathBuf>,
        /// Remove packages that were once Pearlite-managed but are no
        /// longer declared (PRD §7.3 "Forgotten" classification). Per
        /// ADR-0011, the CLI refuses to proceed when the forgotten
        /// count exceeds `--prune-threshold`.
        #[arg(long, conflicts_with = "plan_file")]
        prune: bool,
        /// Maximum number of forgotten packages `--prune` will remove
        /// without explicit override. Default is 5 per ADR-0011 §3;
        /// the post-M6 retrospective revisits the value.
        #[arg(long, default_value_t = 5, requires = "prune")]
        prune_threshold: usize,
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
    /// Bootstrap nix on a fresh host (ADR-0012).
    ///
    /// One-shot side-effect: verifies the operator-supplied installer
    /// script against the SHA-256 declared in the host's
    /// `nix.installer.expected_sha256`, runs the Determinate Nix
    /// installer if `nix --version` fails, and writes
    /// `/etc/nix/nix.conf` with `experimental-features = nix-command
    /// flakes` (idempotent — operator preamble is preserved).
    ///
    /// Bootstrap is **not** rolled back by `pearlite rollback`: nix
    /// touches `/nix` (its own subvolume on btrfs) which lives outside
    /// the Snapper-managed root. Recovery uses Determinate's own
    /// uninstall path.
    Bootstrap {
        /// Host file to evaluate. Defaults to
        /// `<config_dir>/hosts/<hostname>.ncl`.
        #[arg(long)]
        host_file: Option<PathBuf>,
        /// Path to the already-downloaded Determinate Nix installer
        /// script. The SHA-256 of this file's bytes is checked against
        /// the host's `nix.installer.expected_sha256`; the script is
        /// **never** executed if the hash mismatches (ADR-004).
        #[arg(long)]
        installer_script: PathBuf,
        /// Path to the system-wide nix.conf. Defaults to
        /// `/etc/nix/nix.conf`. Override only for tests.
        #[arg(long, default_value = "/etc/nix/nix.conf")]
        nix_conf: PathBuf,
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
        /// Directory to look up the plan JSON in. The full
        /// per-action breakdown is embedded under `data.plan` if the
        /// file is found. Defaults to `<state_file dir>/plans`.
        #[arg(long)]
        plans_dir: Option<PathBuf>,
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
