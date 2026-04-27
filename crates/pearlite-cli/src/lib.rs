// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite CLI library: clap surface, output rendering, schema export.
//!
//! The M0 walking skeleton wires `--version` and `--help` only; subcommands
//! arrive in M1 once the engine's read-only plan path lands.

use clap::Parser;

/// Top-level CLI argument structure for the Pearlite binary.
#[derive(Parser, Debug)]
#[command(name = "pearlite", version, about)]
pub struct Args {}

/// Run the Pearlite CLI with parsed arguments.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn run(_args: Args) -> std::process::ExitCode {
    std::process::ExitCode::SUCCESS
}
