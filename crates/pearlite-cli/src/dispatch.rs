// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Command dispatch — turns parsed [`Args`] into an [`Envelope`].

use crate::args::{Args, Command, GenCommand};
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
            plans_dir,
            dry_run,
        } => dispatch_apply(
            args,
            ctx,
            host_file.as_ref(),
            snapper_config,
            failures_dir.as_ref(),
            plans_dir.as_ref(),
            *dry_run,
            &metadata_at,
        ),
        Command::Rollback {
            plan_id,
            snapper_config,
        } => dispatch_rollback(ctx, *plan_id, snapper_config, &metadata_at),
        Command::Gen { gen_command } => dispatch_gen(ctx, gen_command, &metadata_at),
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
#[allow(
    clippy::too_many_arguments,
    reason = "dispatch_apply mirrors Command::Apply's variant fields plus the \
              shared metadata closure; collapsing them would just hide what's \
              load-bearing"
)]
fn dispatch_apply(
    args: &Args,
    ctx: &RunContext,
    host_file: Option<&PathBuf>,
    snapper_config: &str,
    failures_dir: Option<&PathBuf>,
    plans_dir: Option<&PathBuf>,
    dry_run: bool,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    let host_path = host_file
        .cloned()
        .unwrap_or_else(|| default_host_file(&args.config_dir, &ctx.fallback_host));
    let failures_dir = failures_dir
        .cloned()
        .unwrap_or_else(|| default_failures_dir(&ctx.state_path));
    let plans_dir = plans_dir
        .cloned()
        .unwrap_or_else(|| default_plans_dir(&ctx.state_path));
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
    // Persist the plan before any side effects. Best-effort: the
    // `[[history]]` entry that `apply_plan` writes already carries the
    // plan_id, so a missing JSON file does not break the apply or any
    // future `gen show` lookup. PRD §11 still expects the file to
    // exist for richer per-action forensics, hence the write.
    persist_plan(&plan, &plans_dir);
    if dry_run {
        return match serde_json::to_value(&plan) {
            Ok(v) => Envelope::success(metadata_at(Some(host)), v),
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
        };
    }
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

/// Default plans directory: `<state_file dir>/plans`.
///
/// Mirrors [`default_failures_dir`]: on a production install with
/// `state_file` = `/var/lib/pearlite/state.toml`, this resolves to
/// `/var/lib/pearlite/plans`.
fn default_plans_dir(state_path: &Path) -> PathBuf {
    state_path
        .parent()
        .unwrap_or(Path::new("/var/lib/pearlite"))
        .join("plans")
}

/// Best-effort persist `plan` to `<plans_dir>/<plan-id>.json`.
///
/// Errors are intentionally swallowed: the apply still proceeds, and
/// the `[[history]]` entry carries `plan_id` regardless. The JSON file
/// is a forensic convenience for `gen show` and future `--plan-file`
/// consumers, not an apply prerequisite.
fn persist_plan(plan: &pearlite_diff::Plan, plans_dir: &Path) {
    let _ = std::fs::create_dir_all(plans_dir);
    let path = plans_dir.join(format!("{}.json", plan.plan_id.simple()));
    if let Ok(json) = serde_json::to_vec_pretty(plan) {
        let _ = std::fs::write(path, json);
    }
}

/// Dispatch arm for `pearlite rollback <plan-id>`.
///
/// `Engine::rollback` reads `state.toml`, finds the matching
/// `[[history]]` entry, and dispatches to `snapper.rollback(snapshot_pre.id)`.
/// `state.toml` is not rewritten — the btrfs revert restores the
/// entire root subvolume; the next `pearlite plan` re-derives.
fn dispatch_rollback(
    ctx: &RunContext,
    plan_id: uuid::Uuid,
    snapper_config: &str,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    match ctx.engine.rollback(
        plan_id,
        ctx.snapper.as_ref(),
        snapper_config,
        &ctx.state_path,
    ) {
        Ok(outcome) => Envelope::success(metadata_at(None), rollback_outcome_view(&outcome)),
        Err(e) => Envelope::failure(
            metadata_at(None),
            rollback_error_payload(&e, &ctx.state_path),
        ),
    }
}

/// Dispatch arm for `pearlite gen list` / `pearlite gen show`.
///
/// Both sub-actions are read-only views into `state.toml`'s
/// `[[history]]` array. Like `plan` and `status`, missing-state is
/// tolerated and surfaces as an empty list (`gen list`) or a typed
/// `GEN_NOT_FOUND` error (`gen show`).
fn dispatch_gen(
    ctx: &RunContext,
    gen_command: &GenCommand,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    let state = match read_state_or_empty(&ctx.state_path, &ctx.fallback_host) {
        Ok(s) => s,
        Err(payload) => return Envelope::failure(metadata_at(None), payload),
    };
    match gen_command {
        GenCommand::List => {
            let entries: Vec<serde_json::Value> =
                state.history.iter().map(history_entry_view).collect();
            Envelope::success(
                metadata_at(Some(state.host.clone())),
                serde_json::json!({
                    "generations": entries,
                    "count": entries.len(),
                }),
            )
        }
        GenCommand::Show { plan_id } => {
            match state.history.iter().find(|h| h.plan_id == *plan_id) {
                Some(entry) => Envelope::success(
                    metadata_at(Some(state.host.clone())),
                    full_history_entry_view(entry),
                ),
                None => Envelope::failure(
                    metadata_at(Some(state.host.clone())),
                    ErrorPayload {
                        code: "GEN_NOT_FOUND".to_owned(),
                        class: "preflight".to_owned(),
                        exit_code: 2,
                        message: format!(
                            "no generation with plan_id {plan_id} in state.toml history"
                        ),
                        hint: "pearlite gen list  # show known plan IDs and generations".to_owned(),
                        details: serde_json::Value::Null,
                    },
                ),
            }
        }
    }
}

/// Compact per-row view used by `gen list`: identifying fields plus
/// the headline summary string. Full snapshots / git revision are
/// reserved for `gen show`.
fn history_entry_view(entry: &pearlite_state::HistoryEntry) -> serde_json::Value {
    serde_json::json!({
        "plan_id": entry.plan_id,
        "generation": entry.generation,
        "applied_at": iso8601(entry.applied_at),
        "duration_ms": entry.duration_ms,
        "actions_executed": entry.actions_executed,
        "summary": entry.summary,
    })
}

/// Full per-entry view used by `gen show`: includes both snapshots,
/// the git-revision pair, and everything `history_entry_view` already
/// emits.
fn full_history_entry_view(entry: &pearlite_state::HistoryEntry) -> serde_json::Value {
    serde_json::json!({
        "plan_id": entry.plan_id,
        "generation": entry.generation,
        "applied_at": iso8601(entry.applied_at),
        "duration_ms": entry.duration_ms,
        "actions_executed": entry.actions_executed,
        "summary": entry.summary,
        "snapshot_pre": {
            "id": entry.snapshot_pre.id,
            "label": entry.snapshot_pre.label,
            "created_at": iso8601(entry.snapshot_pre.created_at),
        },
        "snapshot_post": {
            "id": entry.snapshot_post.id,
            "label": entry.snapshot_post.label,
            "created_at": iso8601(entry.snapshot_post.created_at),
        },
        "git_revision": entry.git_revision,
        "git_dirty": entry.git_dirty,
    })
}

fn iso8601(t: OffsetDateTime) -> String {
    use time::format_description::well_known::Iso8601;
    t.format(&Iso8601::DEFAULT)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn label_for(command: &Command) -> String {
    match command {
        Command::Plan { .. } => "pearlite plan".to_owned(),
        Command::Status { .. } => "pearlite status".to_owned(),
        Command::Apply { .. } => "pearlite apply".to_owned(),
        Command::Rollback { .. } => "pearlite rollback".to_owned(),
        Command::Gen { gen_command } => match gen_command {
            GenCommand::List => "pearlite gen list".to_owned(),
            GenCommand::Show { .. } => "pearlite gen show".to_owned(),
        },
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

/// Render-friendly subset of [`pearlite_engine::RollbackOutcome`].
fn rollback_outcome_view(outcome: &pearlite_engine::RollbackOutcome) -> serde_json::Value {
    serde_json::json!({
        "plan_id": outcome.plan_id,
        "generation": outcome.generation,
        "snapshot_pre": {
            "id": outcome.snapshot_pre.id,
            "label": outcome.snapshot_pre.label,
        },
    })
}

/// Map a [`RollbackError`](pearlite_engine::RollbackError) to a CLI
/// [`ErrorPayload`].
///
/// `PlanNotFound` is class 2 (preflight, exit 2) — the user typed a
/// `plan_id` that does not exist; nothing was changed. `Snapper`
/// failures are class 3 (apply-recoverable, exit 4) since the system
/// state is whatever Snapper left it (possibly partially reverted).
/// `State` read failures are class 2.
fn rollback_error_payload(err: &pearlite_engine::RollbackError, state_path: &Path) -> ErrorPayload {
    use pearlite_engine::RollbackError;
    let (code, class, exit_code, hint) = match err {
        RollbackError::PlanNotFound { .. } => (
            "ROLLBACK_NOT_FOUND",
            "preflight",
            2_u8,
            "pearlite gen list  # show known plan IDs and generations".to_owned(),
        ),
        RollbackError::Snapper(_) => (
            "ROLLBACK_SNAPPER",
            "apply-recoverable",
            4_u8,
            "snapper -c root list  # verify snapper is healthy, then retry".to_owned(),
        ),
        RollbackError::State(_) => (
            "ROLLBACK_STATE",
            "preflight",
            2,
            format!("verify {} exists and is readable", state_path.display()),
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
                plans_dir: None,
                dry_run: false,
            },
        }
    }

    fn args_for_apply_dry_run(host_file: PathBuf, state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Apply {
                host_file: Some(host_file),
                snapper_config: "root".to_owned(),
                failures_dir: None,
                plans_dir: None,
                dry_run: true,
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
    fn apply_dry_run_returns_plan_envelope_without_executing() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let env = dispatch(&args_for_apply_dry_run(host, state_path.clone()), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        // dry-run yields the Plan, not the ApplyOutcome — distinguishable
        // by the presence of `actions` (always present, possibly empty)
        // and the absence of `actions_executed`.
        assert!(
            data.get("actions").is_some(),
            "dry-run must return the Plan envelope shape"
        );
        assert!(
            data.get("actions_executed").is_none(),
            "dry-run must NOT return ApplyOutcome shape"
        );

        // No history was written — apply was skipped.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert!(
            read_back.history.is_empty(),
            "dry-run must not commit history"
        );
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
    fn apply_default_plans_dir_is_state_sibling() {
        let p = default_plans_dir(Path::new("/var/lib/pearlite/state.toml"));
        assert_eq!(p, PathBuf::from("/var/lib/pearlite/plans"));
    }

    #[test]
    fn apply_persists_plan_json_to_default_plans_dir() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let env = dispatch(&args_for_apply(host, state_path.clone()), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        let plan_id = data
            .get("plan_id")
            .and_then(serde_json::Value::as_str)
            .expect("plan_id")
            .to_owned();

        // Plan JSON sits at <state_dir>/plans/<plan-id>.json (with
        // hyphens stripped; uuid::Uuid::simple format).
        let plan_id_uuid: uuid::Uuid = plan_id.parse().expect("uuid parse");
        let plans_dir = state_path.parent().expect("parent").join("plans");
        let plan_path = plans_dir.join(format!("{}.json", plan_id_uuid.simple()));
        assert!(
            plan_path.exists(),
            "plan JSON must land at {}",
            plan_path.display()
        );

        // Round-trip: the file deserialises into a Plan whose plan_id
        // matches the apply outcome's plan_id.
        let raw = std::fs::read(&plan_path).expect("read plan json");
        let parsed: pearlite_diff::Plan = serde_json::from_slice(&raw).expect("parse plan");
        assert_eq!(parsed.plan_id, plan_id_uuid);
    }

    #[test]
    fn apply_dry_run_also_persists_plan_json() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let env = dispatch(&args_for_apply_dry_run(host, state_path.clone()), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        let plan_id_str = data
            .get("plan_id")
            .and_then(serde_json::Value::as_str)
            .expect("plan_id");
        let plan_id_uuid: uuid::Uuid = plan_id_str.parse().expect("uuid parse");
        let plans_dir = state_path.parent().expect("parent").join("plans");
        let plan_path = plans_dir.join(format!("{}.json", plan_id_uuid.simple()));
        assert!(
            plan_path.exists(),
            "dry-run must still persist the plan JSON for forensics"
        );
    }

    /// Pre-seed `state_path` with a `[[history]]` entry referencing
    /// snapshot id `pre_snapshot_id`. Returns the `plan_id`.
    fn write_state_with_history(state_path: &Path, pre_snapshot_id: u64) -> uuid::Uuid {
        let plan_id = uuid::Uuid::now_v7();
        let store = StateStore::new(state_path.to_path_buf());
        let entry = pearlite_state::HistoryEntry {
            plan_id,
            generation: 1,
            applied_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            duration_ms: 0,
            snapshot_pre: pearlite_state::SnapshotRef {
                id: pre_snapshot_id,
                label: "pre-pearlite-apply-aaaaaaaa".to_owned(),
                created_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            },
            snapshot_post: pearlite_state::SnapshotRef {
                id: pre_snapshot_id + 1,
                label: "post-pearlite-apply-aaaaaaaa".to_owned(),
                created_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            },
            actions_executed: 0,
            git_revision: None,
            git_dirty: false,
            summary: String::new(),
        };
        let state = State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed::default(),
            adopted: pearlite_state::Adopted::default(),
            history: vec![entry],
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: std::collections::BTreeMap::new(),
        };
        store.write_atomic(&state).expect("write state");
        plan_id
    }

    /// Build a [`MockSnapper`] pre-loaded with `n` snapshots so its
    /// monotonic ID counter is past whatever IDs the test seeds.
    fn snapper_with_n_snapshots(n: u64) -> MockSnapper {
        let snapper = MockSnapper::new();
        for i in 0..n {
            snapper
                .create("root", &format!("seed-{i}"))
                .expect("seed snapshot");
        }
        snapper
    }

    fn args_for_rollback(plan_id: uuid::Uuid, state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Rollback {
                plan_id,
                snapper_config: "root".to_owned(),
            },
        }
    }

    fn args_for_gen_list(state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Gen {
                gen_command: GenCommand::List,
            },
        }
    }

    fn args_for_gen_show(plan_id: uuid::Uuid, state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Gen {
                gen_command: GenCommand::Show { plan_id },
            },
        }
    }

    #[test]
    fn gen_list_returns_empty_count_zero_when_state_missing() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let env = dispatch(&args_for_gen_list(state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(
            data.get("count").and_then(serde_json::Value::as_u64),
            Some(0)
        );
        let gens = data
            .get("generations")
            .and_then(|v| v.as_array())
            .expect("generations array");
        assert!(gens.is_empty());
    }

    #[test]
    fn gen_list_enumerates_history_entries() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let plan_id = write_state_with_history(&state_path, 100);
        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let env = dispatch(&args_for_gen_list(state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(
            data.get("count").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        let gens = data
            .get("generations")
            .and_then(|v| v.as_array())
            .expect("generations array");
        assert_eq!(gens.len(), 1);
        assert_eq!(
            gens[0]
                .get("plan_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            Some(plan_id.to_string())
        );
        assert_eq!(
            gens[0]
                .get("generation")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn gen_show_returns_full_entry_for_known_plan_id() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let plan_id = write_state_with_history(&state_path, 77);
        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let env = dispatch(&args_for_gen_show(plan_id, state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(
            data.get("snapshot_pre")
                .and_then(|v| v.get("id"))
                .and_then(serde_json::Value::as_u64),
            Some(77)
        );
        assert_eq!(
            data.get("snapshot_post")
                .and_then(|v| v.get("id"))
                .and_then(serde_json::Value::as_u64),
            Some(78)
        );
        assert_eq!(
            data.get("generation").and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn gen_show_unknown_plan_id_yields_gen_not_found() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let _known = write_state_with_history(&state_path, 5);
        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let env = dispatch(&args_for_gen_show(uuid::Uuid::now_v7(), state_path), &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "GEN_NOT_FOUND");
        assert_eq!(err.exit_code, 2);
    }

    #[test]
    fn rollback_succeeds_against_known_plan_id() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let plan_id = write_state_with_history(&state_path, 42);

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
            snapper: Box::new(snapper_with_n_snapshots(50)),
        };
        let args = args_for_rollback(plan_id, state_path);
        let env = dispatch(&args, &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(env.metadata.command, "pearlite rollback");
        assert_eq!(
            data.get("generation").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            data.get("snapshot_pre")
                .and_then(|v| v.get("id"))
                .and_then(serde_json::Value::as_u64),
            Some(42)
        );
    }

    #[test]
    fn rollback_unknown_plan_id_yields_not_found() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let _known = write_state_with_history(&state_path, 10);
        let unknown = uuid::Uuid::now_v7();

        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let args = args_for_rollback(unknown, state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "ROLLBACK_NOT_FOUND");
        assert_eq!(err.exit_code, 2);
        assert_eq!(err.class, "preflight");
    }

    #[test]
    fn rollback_missing_state_file_yields_state_error() {
        let dir = TempDir::new().expect("tempdir");
        // Don't write a state.toml; rollback must surface a State
        // error rather than tolerate-and-substitute.
        let state_path = dir.path().join("state.toml");
        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let args = args_for_rollback(uuid::Uuid::now_v7(), state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "ROLLBACK_STATE");
        assert_eq!(err.exit_code, 2);
    }

    #[test]
    fn rollback_snapper_failure_maps_to_apply_recoverable() {
        use pearlite_snapper::{Snapper, SnapperError, SnapshotInfo};
        struct FailingSnapper;
        impl Snapper for FailingSnapper {
            fn create(&self, _: &str, _: &str) -> Result<SnapshotInfo, SnapperError> {
                Err(SnapperError::NotInPath { hint: "test" })
            }
            fn rollback(&self, _: &str, _: u64) -> Result<(), SnapperError> {
                Err(SnapperError::NotInPath { hint: "test" })
            }
            fn list(&self, _: &str) -> Result<Vec<SnapshotInfo>, SnapperError> {
                Ok(Vec::new())
            }
        }

        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let plan_id = write_state_with_history(&state_path, 7);

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
            snapper: Box::new(FailingSnapper),
        };
        let args = args_for_rollback(plan_id, state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "ROLLBACK_SNAPPER");
        assert_eq!(err.exit_code, 4);
        assert_eq!(err.class, "apply-recoverable");
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
