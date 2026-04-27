// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite CLI binary entry point.

use clap::Parser as _;
use pearlite_cli::{Args, run};

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    run(args)
}
