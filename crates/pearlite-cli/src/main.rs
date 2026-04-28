// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite CLI binary entry point.

use clap::Parser as _;
use pearlite_cargo::LiveCargo;
use pearlite_cli::{Args, OutputFormat, RunContext, dispatch, render_human, render_json};
use pearlite_engine::{Engine, LiveProbe};
use pearlite_nickel::LiveNickel;
use pearlite_pacman::LivePacman;
use pearlite_snapper::LiveSnapper;
use pearlite_systemd::LiveSystemd;
use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = Args::parse();

    let probe = LiveProbe::new(
        Box::new(LivePacman::new()),
        Box::new(LiveCargo::new()),
        Box::new(LiveSystemd::new()),
    );
    let engine = Engine::new(
        Box::new(LiveNickel::new()),
        Box::new(probe),
        args.config_dir.clone(),
    );

    let fallback_host = read_hostname();
    let ctx = RunContext {
        engine,
        state_path: args.state_file.clone(),
        fallback_host,
        pacman: Box::new(LivePacman::new()),
        cargo: Box::new(LiveCargo::new()),
        systemd: Box::new(LiveSystemd::new()),
        snapper: Box::new(LiveSnapper::new()),
    };

    let envelope = dispatch(&args, &ctx);

    let exit_code = envelope.error.as_ref().map_or(0_u8, |e| e.exit_code);
    let format = resolve_format(args.format);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let render_result = match format {
        OutputFormat::Human => render_human(&envelope, &mut out),
        OutputFormat::Json | OutputFormat::Auto | OutputFormat::Jsonl => {
            // Auto/Jsonl currently render as compact JSON; Jsonl for
            // long-running ops is M2/M5 work.
            render_json(&envelope, &mut out)
        }
    };
    if let Err(e) = render_result {
        // Fall back to a stderr line if even render failed.
        let stderr = std::io::stderr();
        let _ = writeln!(stderr.lock(), "pearlite: render error: {e}");
        return ExitCode::from(1);
    }
    let _ = out.flush();
    ExitCode::from(exit_code)
}

fn resolve_format(requested: OutputFormat) -> OutputFormat {
    match requested {
        OutputFormat::Auto => {
            if std::io::stdout().is_terminal() {
                OutputFormat::Human
            } else {
                OutputFormat::Json
            }
        }
        other => other,
    }
}

fn read_hostname() -> String {
    std::fs::read_to_string(PathBuf::from("/etc/hostname"))
        .ok()
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests may use expect()/unwrap()/panic!() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;

    #[test]
    fn resolve_format_passthrough_for_explicit_choices() {
        assert_eq!(resolve_format(OutputFormat::Human), OutputFormat::Human);
        assert_eq!(resolve_format(OutputFormat::Json), OutputFormat::Json);
        assert_eq!(resolve_format(OutputFormat::Jsonl), OutputFormat::Jsonl);
    }
}
