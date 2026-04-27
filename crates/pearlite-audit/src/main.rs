// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite audit binary: runs Steelbore Standard checks on a workspace.

use clap::{Parser, Subcommand};
use pearlite_audit::{check_spdx, explain, list_checks};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "pearlite-audit", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run every Steelbore Standard check against `<path>`.
    Check {
        /// Workspace root to audit.
        path: PathBuf,
    },
    /// List every check ID and description.
    List,
    /// Print rationale and remediation for a single check.
    Explain {
        /// Check identifier (e.g. `SPDX-001`).
        check_id: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    match cli.command {
        Cmd::Check { path } => match check_spdx(&path) {
            Ok(violations) if violations.is_empty() => {
                let _ = writeln!(stdout.lock(), "pearlite-audit: 1 check, 0 violations");
                ExitCode::SUCCESS
            }
            Ok(violations) => {
                let mut out = stdout.lock();
                for v in &violations {
                    let _ = writeln!(out, "{}: {} ({})", v.check_id, v.path.display(), v.message);
                }
                let _ = writeln!(out, "pearlite-audit: {} violation(s)", violations.len());
                ExitCode::FAILURE
            }
            Err(e) => {
                let _ = writeln!(stderr.lock(), "pearlite-audit: {e}");
                ExitCode::from(2)
            }
        },
        Cmd::List => {
            let mut out = stdout.lock();
            for info in list_checks() {
                let _ = writeln!(out, "{}\t{}", info.id, info.description);
            }
            ExitCode::SUCCESS
        }
        Cmd::Explain { check_id } => {
            if let Some(text) = explain(&check_id) {
                let _ = writeln!(stdout.lock(), "{text}");
                ExitCode::SUCCESS
            } else {
                let _ = writeln!(stderr.lock(), "pearlite-audit: unknown check '{check_id}'");
                ExitCode::from(2)
            }
        }
    }
}
