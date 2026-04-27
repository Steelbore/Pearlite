// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Command dispatch — turns parsed [`Args`] into an [`Envelope`].

use crate::args::{Args, Command};
use crate::envelope::{Envelope, ErrorPayload, Metadata};
use pearlite_engine::Engine;
use pearlite_state::{State, StateError, StateStore};
use std::path::PathBuf;
use std::time::Instant;
use time::OffsetDateTime;

/// Runtime context the dispatcher uses to talk to the engine.
///
/// Constructed by `main.rs` for production runs and synthesized in
/// tests.
pub struct RunContext {
    /// Configured Pearlite engine (ready to plan).
    pub engine: Engine,
    /// State file path. The dispatcher reads this via [`StateStore`];
    /// missing-file is tolerated and substituted with an empty State.
    pub state_path: PathBuf,
    /// Hostname to record in the metadata block when no declared host
    /// is loaded (e.g. on an early failure).
    pub fallback_host: String,
}

/// Dispatch the parsed [`Args`] against a [`RunContext`] and return
/// the resulting [`Envelope`].
///
/// # Errors
/// Never returns an error directly — failures are reported in the
/// envelope's `error` field with the appropriate exit code.
#[must_use]
pub fn dispatch(args: &Args, ctx: &RunContext) -> Envelope {
    let started = Instant::now();
    let command_label = label_for(&args.command);

    let metadata_at = |host: Option<String>| Metadata {
        command: command_label.clone(),
        host,
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        completed_at: now_iso8601(),
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        config_dir: Some(args.config_dir.clone()),
        invoking_agent: None,
    };

    match &args.command {
        Command::Plan { host_file } | Command::Status { host_file } => {
            let host_path = host_file
                .clone()
                .unwrap_or_else(|| default_host_file(&args.config_dir, &ctx.fallback_host));
            let state = match read_state_or_empty(&ctx.state_path, &ctx.fallback_host) {
                Ok(s) => s,
                Err(payload) => {
                    return Envelope::failure(metadata_at(None), payload);
                }
            };
            match ctx.engine.plan(&host_path, &state) {
                Ok(plan) => match serde_json::to_value(&plan) {
                    Ok(v) => Envelope::success(metadata_at(Some(plan.host)), v),
                    Err(e) => Envelope::failure(
                        metadata_at(None),
                        ErrorPayload {
                            code: "RENDER_FAILED".to_owned(),
                            class: "internal".to_owned(),
                            exit_code: 1,
                            message: format!("could not serialize plan: {e}"),
                            hint: "report this as a Pearlite bug".to_owned(),
                            details: serde_json::Value::Null,
                        },
                    ),
                },
                Err(e) => Envelope::failure(metadata_at(None), engine_error_payload(&e)),
            }
        }
        Command::Schema { bare } => {
            if *bare {
                Envelope::success(metadata_at(None), bare_schema())
            } else {
                Envelope::failure(
                    metadata_at(None),
                    ErrorPayload {
                        code: "SCHEMA_FORMAT_M5".to_owned(),
                        class: "preflight".to_owned(),
                        exit_code: 2,
                        message: "non-bare schema formats (anthropic/openai/gemini/mcp) land in M5"
                            .to_owned(),
                        hint: "pearlite schema --bare".to_owned(),
                        details: serde_json::Value::Null,
                    },
                )
            }
        }
    }
}

fn label_for(command: &Command) -> String {
    match command {
        Command::Plan { .. } => "pearlite plan".to_owned(),
        Command::Status { .. } => "pearlite status".to_owned(),
        Command::Schema { .. } => "pearlite schema".to_owned(),
    }
}

fn default_host_file(config_dir: &std::path::Path, hostname: &str) -> PathBuf {
    let host = if hostname.is_empty() {
        "host".to_owned()
    } else {
        hostname.to_owned()
    };
    config_dir.join("hosts").join(format!("{host}.ncl"))
}

fn read_state_or_empty(path: &std::path::Path, fallback_host: &str) -> Result<State, ErrorPayload> {
    let store = StateStore::new(path.to_path_buf());
    match store.read() {
        Ok(s) => Ok(s),
        Err(StateError::NotFound(_)) => Ok(empty_state(fallback_host)),
        Err(e) => Err(ErrorPayload {
            code: "STATE_READ_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("{e}"),
            hint: format!(
                "pearlite reconcile  # to re-derive {} from the live system",
                path.display()
            ),
            details: serde_json::Value::Null,
        }),
    }
}

fn empty_state(host: &str) -> State {
    State {
        schema_version: pearlite_state::SCHEMA_VERSION,
        host: host.to_owned(),
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        config_dir: PathBuf::new(),
        last_apply: None,
        last_modified: None,
        managed: pearlite_state::Managed::default(),
        adopted: pearlite_state::Adopted::default(),
        history: Vec::new(),
        reconciliations: Vec::new(),
        failures: Vec::new(),
        reserved: std::collections::BTreeMap::new(),
    }
}

fn bare_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Pearlite CLI placeholder schema",
        "description": "M1 placeholder. Anthropic / OpenAI / Gemini / MCP formats land in M5.",
        "type": "object",
    })
}

fn engine_error_payload(err: &pearlite_engine::EngineError) -> ErrorPayload {
    use pearlite_engine::{EngineError, ProbeError};
    let (code, class, exit_code, hint) = match err {
        EngineError::Nickel(_) => (
            "NICKEL_FAILED",
            "preflight",
            2_u8,
            "paru -S nickel-lang  # ensure nickel is installed".to_owned(),
        ),
        EngineError::ContractViolations(_) => (
            "CONTRACT_VIOLATION",
            "preflight",
            2,
            "edit hosts/<host>.ncl to satisfy the violations above".to_owned(),
        ),
        EngineError::Probe(ProbeError::Pacman(_)) => (
            "PROBE_PACMAN",
            "plan",
            3,
            "pacman -V  # verify pacman works on this host".to_owned(),
        ),
        EngineError::Probe(ProbeError::Cargo(_)) => (
            "PROBE_CARGO",
            "plan",
            3,
            "rustup show  # verify the toolchain is configured".to_owned(),
        ),
        EngineError::Probe(ProbeError::Systemd(_)) => (
            "PROBE_SYSTEMD",
            "plan",
            3,
            "systemctl --version  # verify systemd is reachable".to_owned(),
        ),
        EngineError::Probe(ProbeError::Io(_)) => (
            "PROBE_IO",
            "plan",
            3,
            "check /etc/hostname permissions and try again".to_owned(),
        ),
        EngineError::Fs(_) => (
            "CONFIG_SOURCE_HASH",
            "plan",
            3,
            "verify [[config]].source paths exist under config_dir".to_owned(),
        ),
        EngineError::State(_) => (
            "STATE_FAILED",
            "preflight",
            2,
            "pearlite reconcile".to_owned(),
        ),
    };
    ErrorPayload {
        code: code.to_owned(),
        class: class.to_owned(),
        exit_code,
        message: format!("{err}"),
        hint,
        details: serde_json::Value::Null,
    }
}

fn now_iso8601() -> String {
    use time::format_description::well_known::Iso8601;
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
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
    use crate::args::OutputFormat;
    use pearlite_engine::MockProbe;
    use pearlite_nickel::MockNickel;
    use pearlite_schema::{
        CargoInventory, HostInfo, KernelInfo, PacmanInventory, ProbedState, ServiceInventory,
    };
    use tempfile::TempDir;

    const MINIMAL_HOST: &str = r#"
[meta]
hostname = "forge"
timezone = "UTC"
arch_level = "v4"
locale = "en_US.UTF-8"
keymap = "us"

[kernel]
package = "linux-cachyos"
"#;

    fn empty_probed() -> ProbedState {
        ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: Some(PacmanInventory::default()),
            cargo: Some(CargoInventory::default()),
            config_files: None,
            services: Some(ServiceInventory::default()),
            kernel: KernelInfo::default(),
        }
    }

    fn ctx_with(host_path: PathBuf, host_body: &str, state_path: PathBuf) -> RunContext {
        let mut nickel = MockNickel::new();
        nickel.seed(host_path, host_body);
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"));
        RunContext {
            engine,
            state_path,
            fallback_host: "forge".to_owned(),
        }
    }

    fn args_for_plan(host_file: PathBuf, state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Plan {
                host_file: Some(host_file),
            },
        }
    }

    #[test]
    fn plan_succeeds_against_a_minimal_host() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let args = args_for_plan(host, state_path);
        let env = dispatch(&args, &ctx);
        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert!(data.get("actions").is_some());
        assert_eq!(env.metadata.host.as_deref(), Some("forge"));
    }

    #[test]
    fn plan_fails_with_typed_error_when_nickel_missing() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        // No seed → MockNickel returns MockMissing.
        let nickel = MockNickel::new();
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"));
        let ctx = RunContext {
            engine,
            state_path: state_path.clone(),
            fallback_host: "forge".to_owned(),
        };
        let args = args_for_plan(host, state_path);
        let env = dispatch(&args, &ctx);
        let err = env.error.expect("error");
        assert_eq!(err.code, "NICKEL_FAILED");
        assert_eq!(err.exit_code, 2);
        assert!(!err.hint.is_empty());
    }

    #[test]
    fn schema_bare_returns_draft_2020_12_placeholder() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let args = Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file: state_path,
            command: Command::Schema { bare: true },
        };
        let env = dispatch(&args, &ctx);
        let data = env.data.expect("data");
        let schema = data
            .get("$schema")
            .and_then(|v| v.as_str())
            .expect("$schema");
        assert!(schema.contains("2020-12"));
    }

    #[test]
    fn schema_without_bare_emits_m5_placeholder_error() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let args = Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file: state_path,
            command: Command::Schema { bare: false },
        };
        let env = dispatch(&args, &ctx);
        let err = env.error.expect("error");
        assert_eq!(err.code, "SCHEMA_FORMAT_M5");
    }

    #[test]
    fn missing_state_file_is_tolerated() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("does-not-exist.toml");
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let args = args_for_plan(host, state_path);
        let env = dispatch(&args, &ctx);
        assert!(
            env.error.is_none(),
            "missing state file must be tolerated, got {:?}",
            env.error
        );
    }
}
