// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Command dispatch — turns parsed [`Args`] into an [`Envelope`].

use crate::args::{Args, Command};
use crate::envelope::{Envelope, ErrorPayload, Metadata};
use pearlite_cargo::Cargo;
use pearlite_engine::Engine;
use pearlite_pacman::Pacman;
use pearlite_snapper::Snapper;
use pearlite_state::{State, StateError, StateStore};
use pearlite_systemd::Systemd;
use std::path::{Path, PathBuf};
use std::time::Instant;
use time::OffsetDateTime;

/// Runtime context the dispatcher uses to talk to the engine.
///
/// Constructed by `main.rs` for production runs and synthesized in
/// tests. The four adapter trait objects are owned by the context so
/// `apply` can hand them to [`Engine::apply_plan`] without rebuilding
/// per call. The plan and status paths ignore them.
pub struct RunContext {
    /// Configured Pearlite engine (ready to plan).
    pub engine: Engine,
    /// State file path. The dispatcher reads this via [`StateStore`].
    /// `plan` / `status` tolerate missing-file; `apply` does not.
    pub state_path: PathBuf,
    /// Hostname to record in the metadata block when no declared host
    /// is loaded (e.g. on an early failure).
    pub fallback_host: String,
    /// pacman / paru adapter (`apply` only).
    pub pacman: Box<dyn Pacman>,
    /// cargo adapter (`apply` only).
    pub cargo: Box<dyn Cargo>,
    /// systemd adapter (`apply` only).
    pub systemd: Box<dyn Systemd>,
    /// snapper adapter (`apply` and `rollback`).
    pub snapper: Box<dyn Snapper>,
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
        Command::Apply {
            host_file,
            snapper_config,
            failures_dir,
        } => dispatch_apply(
            args,
            ctx,
            host_file.as_ref(),
            snapper_config,
            failures_dir.as_ref(),
            &metadata_at,
        ),
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

/// Dispatch arm for `pearlite apply`.
///
/// Plan → apply → render. Pulled out of [`dispatch`] so the top-level
/// match stays under clippy's `too_many_lines` limit; logic is
/// otherwise identical to the inline form.
fn dispatch_apply(
    args: &Args,
    ctx: &RunContext,
    host_file: Option<&PathBuf>,
    snapper_config: &str,
    failures_dir: Option<&PathBuf>,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    let host_path = host_file
        .cloned()
        .unwrap_or_else(|| default_host_file(&args.config_dir, &ctx.fallback_host));
    let failures_dir = failures_dir
        .cloned()
        .unwrap_or_else(|| default_failures_dir(&ctx.state_path));
    let state = match read_state_strict(&ctx.state_path) {
        Ok(s) => s,
        Err(payload) => return Envelope::failure(metadata_at(None), payload),
    };
    let plan = match ctx.engine.plan(&host_path, &state) {
        Ok(p) => p,
        Err(e) => return Envelope::failure(metadata_at(None), engine_error_payload(&e)),
    };
    let host = plan.host.clone();
    let plan_id = plan.plan_id;
    match ctx.engine.apply_plan(
        &plan,
        ctx.pacman.as_ref(),
        ctx.cargo.as_ref(),
        ctx.systemd.as_ref(),
        ctx.snapper.as_ref(),
        snapper_config,
        &ctx.state_path,
        &failures_dir,
    ) {
        Ok(outcome) => Envelope::success(metadata_at(Some(host)), apply_outcome_view(&outcome)),
        Err(e) => Envelope::failure(
            metadata_at(Some(host)),
            apply_error_payload(&e, &ctx.state_path, plan_id),
        ),
    }
}

fn label_for(command: &Command) -> String {
    match command {
        Command::Plan { .. } => "pearlite plan".to_owned(),
        Command::Status { .. } => "pearlite status".to_owned(),
        Command::Apply { .. } => "pearlite apply".to_owned(),
        Command::Schema { .. } => "pearlite schema".to_owned(),
    }
}

/// Default failures directory: `<state_file dir>/failures`.
///
/// On a production install, with `state_file` =
/// `/var/lib/pearlite/state.toml`, this resolves to
/// `/var/lib/pearlite/failures`.
fn default_failures_dir(state_path: &Path) -> PathBuf {
    state_path
        .parent()
        .unwrap_or(Path::new("/var/lib/pearlite"))
        .join("failures")
}

/// Render-friendly subset of [`pearlite_engine::ApplyOutcome`].
///
/// `ApplyOutcome` itself doesn't `Serialize`; this view is what lands
/// in the envelope's `data` field.
fn apply_outcome_view(outcome: &pearlite_engine::ApplyOutcome) -> serde_json::Value {
    serde_json::json!({
        "plan_id": outcome.plan_id,
        "generation": outcome.generation,
        "actions_executed": outcome.actions_executed,
        "duration_ms": outcome.duration_ms,
        "snapshot_pre": {
            "id": outcome.snapshot_pre.id,
            "label": outcome.snapshot_pre.label,
        },
        "snapshot_post": {
            "id": outcome.snapshot_post.id,
            "label": outcome.snapshot_post.label,
        },
    })
}

fn default_host_file(config_dir: &std::path::Path, hostname: &str) -> PathBuf {
    let host = if hostname.is_empty() {
        "host".to_owned()
    } else {
        hostname.to_owned()
    };
    config_dir.join("hosts").join(format!("{host}.ncl"))
}

/// `apply` requires a real `state.toml` — there is no fallback to an
/// empty state because the post-apply phase-9 commit needs an existing
/// file to extend (CLAUDE.md hard invariant 7: only `pearlite-engine`
/// writes `state.toml`, and even it requires the file to pre-exist).
fn read_state_strict(path: &std::path::Path) -> Result<State, ErrorPayload> {
    let store = StateStore::new(path.to_path_buf());
    match store.read() {
        Ok(s) => Ok(s),
        Err(StateError::NotFound(_)) => Err(ErrorPayload {
            code: "STATE_NOT_FOUND".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("{} does not exist", path.display()),
            hint: format!(
                "pearlite reconcile  # to initialize {} from the live system",
                path.display()
            ),
            details: serde_json::Value::Null,
        }),
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

/// Map an [`ApplyError`](pearlite_engine::ApplyError) to a CLI
/// [`ErrorPayload`].
///
/// Per CLAUDE.md, "PRD §8.5 failure-class mapping is performed at the
/// CLI boundary using `Action::failure_coherence` on the action that
/// triggered the error". We don't have the action here, but
/// [`Engine::apply_plan`](pearlite_engine::Engine::apply_plan) wrote a
/// `FailureRef` to `state.toml` containing the exact class. Re-read it
/// to recover that class, then fall back to a sensible default if the
/// failure happened before the engine could record one (e.g. the
/// pre-snapshot itself failed).
fn apply_error_payload(
    err: &pearlite_engine::ApplyError,
    state_path: &Path,
    plan_id: uuid::Uuid,
) -> ErrorPayload {
    use pearlite_engine::ApplyError;

    let recorded = read_recorded_failure_class(state_path, plan_id);
    let (default_class, default_exit_code) = recorded.unwrap_or((3_u8, 4_u8));

    let (code, hint) = match err {
        ApplyError::Snapper(_) => (
            "APPLY_SNAPPER",
            "snapper -c root list  # verify snapper is healthy".to_owned(),
        ),
        ApplyError::Pacman(_) => (
            "APPLY_PACMAN",
            "paru -Syu  # ensure pacman db sync still works, then retry".to_owned(),
        ),
        ApplyError::Cargo(_) => (
            "APPLY_CARGO",
            "rustup show  # verify the toolchain is configured, then retry".to_owned(),
        ),
        ApplyError::Systemd(_) => (
            "APPLY_SYSTEMD",
            "systemctl --version  # verify systemd is reachable, then retry".to_owned(),
        ),
        ApplyError::Fs(_) => (
            "APPLY_FS",
            "verify [[config]].source paths exist and the target /etc path is writable".to_owned(),
        ),
        ApplyError::ContentSha256Mismatch { target, .. } => (
            "APPLY_SHA_MISMATCH",
            format!(
                "pearlite plan  # source for {} changed since plan was computed; re-plan and retry",
                target.display()
            ),
        ),
        ApplyError::State(_) => (
            "APPLY_STATE",
            format!(
                "verify {} is writable; pearlite reconcile if it is corrupt",
                state_path.display()
            ),
        ),
    };

    let class_label = match default_class {
        2 => "plan",
        3 => "apply-recoverable",
        4 => "apply-incoherent",
        _ => "apply",
    };

    ErrorPayload {
        code: code.to_owned(),
        class: class_label.to_owned(),
        exit_code: default_exit_code,
        message: format!("{err}"),
        hint,
        details: serde_json::json!({
            "plan_id": plan_id,
            "failure_class": default_class,
        }),
    }
}

/// Look up the [`FailureRef`](pearlite_state::FailureRef) the engine
/// just appended for `plan_id`.
///
/// Returns `(class, exit_code)` from that record, or `None` if the
/// engine never reached the failure-record step (e.g. pre-snapshot
/// fail) or `state.toml` cannot be read.
fn read_recorded_failure_class(state_path: &Path, plan_id: uuid::Uuid) -> Option<(u8, u8)> {
    let store = StateStore::new(state_path.to_path_buf());
    let state = store.read().ok()?;
    state
        .failures
        .iter()
        .rev()
        .find(|f| f.plan_id == plan_id)
        .map(|f| (f.class, f.exit_code))
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
    use pearlite_cargo::MockCargo;
    use pearlite_engine::MockProbe;
    use pearlite_nickel::MockNickel;
    use pearlite_pacman::MockPacman;
    use pearlite_schema::{
        CargoInventory, HostInfo, KernelInfo, PacmanInventory, ProbedState, ServiceInventory,
    };
    use pearlite_snapper::MockSnapper;
    use pearlite_state::SCHEMA_VERSION;
    use pearlite_systemd::MockSystemd;
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
            pacman: Box::new(MockPacman::new()),
            cargo: Box::new(MockCargo::new()),
            systemd: Box::new(MockSystemd::new()),
            snapper: Box::new(MockSnapper::new()),
        }
    }

    /// Pre-seed `state_path` with a minimal schema-valid `state.toml`.
    fn write_baseline_state(state_path: &Path) {
        let store = StateStore::new(state_path.to_path_buf());
        let state = State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed::default(),
            adopted: pearlite_state::Adopted::default(),
            history: Vec::new(),
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: std::collections::BTreeMap::new(),
        };
        store.write_atomic(&state).expect("write base state");
    }

    fn args_for_apply(host_file: PathBuf, state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Apply {
                host_file: Some(host_file),
                snapper_config: "root".to_owned(),
                failures_dir: None,
            },
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
            pacman: Box::new(MockPacman::new()),
            cargo: Box::new(MockCargo::new()),
            systemd: Box::new(MockSystemd::new()),
            snapper: Box::new(MockSnapper::new()),
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
    fn apply_succeeds_against_a_minimal_host() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let args = args_for_apply(host, state_path);
        let env = dispatch(&args, &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(env.metadata.command, "pearlite apply");
        assert_eq!(env.metadata.host.as_deref(), Some("forge"));
        assert_eq!(
            data.get("actions_executed")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert_eq!(
            data.get("generation").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(data.get("snapshot_pre").is_some());
        assert!(data.get("snapshot_post").is_some());
    }

    #[test]
    fn apply_fails_with_state_not_found_when_state_missing() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        // Don't write state.toml. apply must surface STATE_NOT_FOUND
        // rather than tolerate-and-substitute (the M1 plan path's
        // behaviour is wrong for apply).
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let args = args_for_apply(host, state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "STATE_NOT_FOUND");
        assert_eq!(err.exit_code, 2);
        assert_eq!(err.class, "preflight");
        assert!(!err.hint.is_empty());
    }

    #[test]
    fn apply_propagates_engine_plan_failure() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        // No nickel seed → MockNickel::MockMissing.
        let nickel = MockNickel::new();
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"));
        let ctx = RunContext {
            engine,
            state_path: state_path.clone(),
            fallback_host: "forge".to_owned(),
            pacman: Box::new(MockPacman::new()),
            cargo: Box::new(MockCargo::new()),
            systemd: Box::new(MockSystemd::new()),
            snapper: Box::new(MockSnapper::new()),
        };
        let args = args_for_apply(host, state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "NICKEL_FAILED");
        assert_eq!(err.exit_code, 2);
    }

    #[test]
    fn apply_default_failures_dir_is_state_sibling() {
        // Sanity-check that the default `<state_dir>/failures` is what
        // the dispatcher computes, even though it isn't used directly
        // when actions_executed == 0.
        let p = default_failures_dir(Path::new("/var/lib/pearlite/state.toml"));
        assert_eq!(p, PathBuf::from("/var/lib/pearlite/failures"));
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
