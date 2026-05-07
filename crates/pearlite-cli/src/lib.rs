// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite CLI library: clap surface, output rendering, schema export.
//!
//! M1 wires `pearlite plan`, `pearlite status`, and `pearlite schema
//! --bare` into the engine's read-only path. Apply, rollback, reconcile,
//! the per-provider schema formats, and the MCP server arrive in M2+.

pub mod agents;
mod args;
mod dispatch;
mod envelope;
mod render;

pub use args::{Args, Command, GenCommand, OutputFormat};
pub use dispatch::{RunContext, dispatch};
pub use envelope::{Envelope, ErrorPayload, Metadata};
pub use render::{render_human, render_json};
