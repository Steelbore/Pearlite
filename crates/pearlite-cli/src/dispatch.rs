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
use pearlite_userenv::{HomeManagerBackend, NixInstaller};
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
    /// Home Manager backend (`apply` phase 7).
    pub home_manager: Box<dyn HomeManagerBackend>,
    /// Determinate Nix installer adapter (`bootstrap` only). Per
    /// ADR-0012 / ADR-004: the only curl-piped script Pearlite
    /// tolerates, defended by a hash-pin from the host config.
    pub nix_installer: Box<dyn NixInstaller>,
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
            match ctx.engine.plan(&host_path, &state, false) {
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
            plan_file,
            prune,
            prune_threshold,
        } => dispatch_apply(
            args,
            ctx,
            &ApplyOpts {
                host_file: host_file.as_ref(),
                snapper_config,
                failures_dir: failures_dir.as_ref(),
                plans_dir: plans_dir.as_ref(),
                dry_run: *dry_run,
                plan_file: plan_file.as_ref(),
                prune: *prune,
                prune_threshold: *prune_threshold,
            },
            &metadata_at,
        ),
        Command::Rollback {
            plan_id,
            snapper_config,
        } => dispatch_rollback(ctx, *plan_id, snapper_config, &metadata_at),
        Command::Gen { gen_command } => dispatch_gen(ctx, gen_command, &metadata_at),
        Command::Bootstrap {
            host_file,
            installer_script,
            nix_conf,
        } => dispatch_bootstrap(
            args,
            ctx,
            host_file.as_ref(),
            installer_script,
            nix_conf,
            &metadata_at,
        ),
        Command::Reconcile => dispatch_reconcile(args, ctx, &metadata_at),
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
/// Bundle of `pearlite apply` options for [`dispatch_apply`].
///
/// Pulled out of [`dispatch`] as a struct so the helper signature
/// stays under clippy's `too_many_arguments` ceiling and so future
/// flags don't keep widening the function.
struct ApplyOpts<'a> {
    host_file: Option<&'a PathBuf>,
    snapper_config: &'a str,
    failures_dir: Option<&'a PathBuf>,
    plans_dir: Option<&'a PathBuf>,
    dry_run: bool,
    plan_file: Option<&'a PathBuf>,
    prune: bool,
    prune_threshold: usize,
}

fn dispatch_apply(
    args: &Args,
    ctx: &RunContext,
    opts: &ApplyOpts<'_>,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    let host_path = opts
        .host_file
        .cloned()
        .unwrap_or_else(|| default_host_file(&args.config_dir, &ctx.fallback_host));
    let failures_dir = opts
        .failures_dir
        .cloned()
        .unwrap_or_else(|| default_failures_dir(&ctx.state_path));
    let plans_dir = opts
        .plans_dir
        .cloned()
        .unwrap_or_else(|| default_plans_dir(&ctx.state_path));
    let state = match read_state_strict(&ctx.state_path) {
        Ok(s) => s,
        Err(payload) => return Envelope::failure(metadata_at(None), payload),
    };
    let plan = match opts.plan_file {
        Some(path) => match load_plan_file(path) {
            Ok(p) => p,
            Err(payload) => return Envelope::failure(metadata_at(None), payload),
        },
        None => match ctx.engine.plan(&host_path, &state, opts.prune) {
            Ok(p) => p,
            Err(e) => return Envelope::failure(metadata_at(None), engine_error_payload(&e)),
        },
    };
    let host = plan.host.clone();
    let plan_id = plan.plan_id;

    // ADR-0011 threshold guard. Counts forgotten removals (the prune
    // surface), NOT every PacmanRemove / CargoUninstall — declared
    // removes via [remove] policy are out of scope for the threshold.
    if opts.prune {
        let pruned = count_pruned_packages(&plan);
        if pruned > opts.prune_threshold {
            return Envelope::failure(
                metadata_at(Some(host)),
                ErrorPayload {
                    code: "PRUNE_THRESHOLD_EXCEEDED".to_owned(),
                    class: "preflight".to_owned(),
                    exit_code: 2,
                    message: format!(
                        "{pruned} forgotten packages would be removed; \
                         threshold is {} (ADR-0011)",
                        opts.prune_threshold,
                    ),
                    hint: format!(
                        "audit the diff via `pearlite plan`, then re-run with \
                         `--prune-threshold {pruned}` if the removals are intentional",
                    ),
                    details: serde_json::json!({
                        "prune_count": pruned,
                        "prune_threshold": opts.prune_threshold,
                        "plan_id": plan_id,
                    }),
                },
            );
        }
    }

    // Persist the plan before any side effects. Best-effort: the
    // `[[history]]` entry that `apply_plan` writes already carries the
    // plan_id, so a missing JSON file does not break the apply or any
    // future `gen show` lookup. PRD §11 still expects the file to
    // exist for richer per-action forensics, hence the write.
    //
    // When --plan-file was used, the source JSON is by definition
    // already on disk. Re-persisting under <plans_dir>/<plan-id>.json
    // is still useful: it ensures a uniformly-located forensic copy
    // even if the operator passed an out-of-tree file.
    persist_plan(&plan, &plans_dir);
    if opts.dry_run {
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
    let apply_ctx = pearlite_engine::ApplyContext {
        pacman: ctx.pacman.as_ref(),
        cargo: ctx.cargo.as_ref(),
        systemd: ctx.systemd.as_ref(),
        snapper: ctx.snapper.as_ref(),
        home_manager: ctx.home_manager.as_ref(),
        nix_installer: ctx.nix_installer.as_ref(),
        snapper_config: opts.snapper_config,
        state_path: &ctx.state_path,
        failures_dir: &failures_dir,
    };
    match ctx.engine.apply_plan(&plan, &apply_ctx) {
        Ok(outcome) => Envelope::success(metadata_at(Some(host)), apply_outcome_view(&outcome)),
        Err(e) => Envelope::failure(
            metadata_at(Some(host)),
            apply_error_payload(&e, &ctx.state_path, plan_id),
        ),
    }
}

/// Dispatch arm for `pearlite bootstrap` (ADR-0012).
///
/// Wires `Engine::bootstrap` with the operator-supplied installer
/// script. The host file's `nix.installer.expected_sha256` defends
/// the script (ADR-004). Bootstrap state is not recorded in
/// `state.toml` — see ADR-0012 decision 4.
fn dispatch_bootstrap(
    args: &Args,
    ctx: &RunContext,
    host_file: Option<&PathBuf>,
    installer_script: &Path,
    nix_conf: &Path,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    let host_path = host_file
        .cloned()
        .unwrap_or_else(|| default_host_file(&args.config_dir, &ctx.fallback_host));
    match ctx.engine.bootstrap(
        &host_path,
        ctx.nix_installer.as_ref(),
        installer_script,
        nix_conf,
    ) {
        Ok(outcome) => Envelope::success(
            metadata_at(Some(ctx.fallback_host.clone())),
            bootstrap_outcome_view(outcome),
        ),
        Err(e) => Envelope::failure(
            metadata_at(Some(ctx.fallback_host.clone())),
            bootstrap_error_payload(&e),
        ),
    }
}

/// Render-friendly view of [`pearlite_engine::BootstrapOutcome`].
fn bootstrap_outcome_view(outcome: pearlite_engine::BootstrapOutcome) -> serde_json::Value {
    serde_json::json!({
        "install": match outcome.install {
            pearlite_userenv::InstallOutcome::Already => "already",
            pearlite_userenv::InstallOutcome::Installed => "installed",
        },
        "nix_conf_written": outcome.nix_conf_written,
    })
}

/// Map `BootstrapError` to a typed [`ErrorPayload`].
///
/// All bootstrap failures land in PRD §8.5 class 2 (preflight) —
/// nothing on the system has been irreversibly mutated by the time
/// these surface. Exit code 2 throughout.
fn bootstrap_error_payload(err: &pearlite_engine::BootstrapError) -> ErrorPayload {
    use pearlite_engine::BootstrapError as B;
    use pearlite_userenv::InstallerError as I;
    match err {
        B::Nickel(e) => ErrorPayload {
            code: "BOOTSTRAP_NICKEL_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("could not load host file: {e}"),
            hint: "verify the host file path; run `pearlite plan` first to surface schema issues"
                .to_owned(),
            details: serde_json::Value::Null,
        },
        B::NixNotDeclared => ErrorPayload {
            code: "NIX_NOT_DECLARED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: "host file has no [nix.installer] block".to_owned(),
            hint: "declare nix.installer.expected_sha256 in your host file, or skip \
                   `pearlite bootstrap` for hosts that don't need nix"
                .to_owned(),
            details: serde_json::Value::Null,
        },
        B::Installer(I::Sha256Mismatch { expected, actual }) => ErrorPayload {
            code: "NIX_INSTALLER_SHA256_MISMATCH".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!(
                "installer script SHA-256 mismatch: declared {expected}, got {actual}"
            ),
            hint: "update the host's nix.installer.expected_sha256 to match the script you're \
                   bootstrapping against, or fetch the matching installer version"
                .to_owned(),
            details: serde_json::json!({
                "expected_sha256": expected,
                "actual_sha256": actual,
            }),
        },
        B::Installer(other) => ErrorPayload {
            code: "NIX_INSTALLER_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("Determinate Nix installer failed: {other}"),
            hint: "inspect the installer's stderr above; re-run after addressing the cause"
                .to_owned(),
            details: serde_json::Value::Null,
        },
        B::Fs(e) => ErrorPayload {
            code: "BOOTSTRAP_NIX_CONF_WRITE_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("writing /etc/nix/nix.conf failed: {e}"),
            hint: "ensure pearlite is invoked as root for the nix.conf write".to_owned(),
            details: serde_json::Value::Null,
        },
        B::Io(e) => ErrorPayload {
            code: "BOOTSTRAP_NIX_CONF_READ_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("reading existing /etc/nix/nix.conf failed: {e}"),
            hint: "check the file's permissions; re-run as root if needed".to_owned(),
            details: serde_json::Value::Null,
        },
    }
}

/// Dispatch arm for `pearlite reconcile` (read-only).
///
/// Probes the live system and writes
/// `<config_dir>/hosts/<hostname>.imported.ncl` via
/// [`Engine::reconcile`]. Does not touch `state.toml` — the operator
/// reviews the import, hand-curates it, and renames it to
/// `<hostname>.ncl` for `pearlite plan` to pick up. The interactive
/// `--commit` and `--adopt-all` flags (which DO mutate state) ride
/// along with `Engine::reconcile_commit` in a follow-up PR.
fn dispatch_reconcile(
    args: &Args,
    ctx: &RunContext,
    metadata_at: &dyn Fn(Option<String>) -> Metadata,
) -> Envelope {
    match ctx.engine.reconcile(&args.config_dir) {
        Ok(outcome) => Envelope::success(
            metadata_at(Some(outcome.hostname.clone())),
            reconcile_outcome_view(&outcome),
        ),
        Err(e) => Envelope::failure(metadata_at(None), reconcile_error_payload(&e)),
    }
}

/// Render-friendly view of [`pearlite_engine::ReconcileOutcome`].
fn reconcile_outcome_view(outcome: &pearlite_engine::ReconcileOutcome) -> serde_json::Value {
    serde_json::json!({
        "imported_path": outcome.path.to_string_lossy(),
        "hostname": outcome.hostname,
    })
}

/// Map `ReconcileError` to a typed [`ErrorPayload`].
///
/// Reconcile is class 1 (preflight) throughout: the only system-side
/// effect is the atomic write of the `.imported.ncl` file, and a
/// failure of that write leaves the operator's config repo untouched
/// (the temp file is dropped). `state.toml` is never read or written
/// on this path, so no recoverable/incoherent classes apply.
fn reconcile_error_payload(err: &pearlite_engine::ReconcileError) -> ErrorPayload {
    use pearlite_engine::ReconcileError as R;
    match err {
        R::Probe(e) => ErrorPayload {
            code: "RECONCILE_PROBE_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("probing live system failed: {e}"),
            hint: "run `pearlite plan` first to surface the underlying probe error".to_owned(),
            details: serde_json::Value::Null,
        },
        R::EmptyHostname => ErrorPayload {
            code: "RECONCILE_EMPTY_HOSTNAME".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: "probe returned an empty hostname".to_owned(),
            hint: "set /etc/hostname to a non-empty value, then re-run `pearlite reconcile`"
                .to_owned(),
            details: serde_json::Value::Null,
        },
        R::InvalidHostname { hostname } => ErrorPayload {
            code: "RECONCILE_INVALID_HOSTNAME".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("hostname {hostname:?} is not a valid filename component"),
            hint: "set /etc/hostname to an RFC-1123-compliant value (no `/`, `\\`, or NUL)"
                .to_owned(),
            details: serde_json::json!({ "hostname": hostname }),
        },
        R::AlreadyExists { path } => ErrorPayload {
            code: "RECONCILE_ALREADY_EXISTS".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("{} already exists", path.display()),
            hint: format!(
                "rm {} or rename it before re-running `pearlite reconcile`",
                path.display()
            ),
            details: serde_json::json!({ "path": path.to_string_lossy() }),
        },
        R::Io { path, source } => ErrorPayload {
            code: "RECONCILE_IO_FAILED".to_owned(),
            class: "preflight".to_owned(),
            exit_code: 2,
            message: format!("I/O error at {}: {source}", path.display()),
            hint: format!(
                "ensure pearlite can write to {}; re-run as the user who owns the config repo",
                path.display()
            ),
            details: serde_json::json!({ "path": path.to_string_lossy() }),
        },
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

/// Read a [`pearlite_diff::Plan`] from a JSON file at `path`.
///
/// Per ADR-0010, the reader uses strict serde deserialization with no
/// version-comparison logic: the file must round-trip losslessly
/// through the current build's `Plan` type. Unknown fields are
/// tolerated; unknown enum variants in `actions` cause a parse error
/// — that is the load-bearing strictness that catches stale plans
/// authored by a future build.
///
/// # Errors
/// - `PLAN_FILE_READ_FAILED` — file is missing or unreadable.
/// - `PLAN_FILE_PARSE_FAILED` — file is not a valid `Plan` JSON
///   (malformed JSON, unknown variant, missing required field).
fn load_plan_file(path: &Path) -> Result<pearlite_diff::Plan, ErrorPayload> {
    let raw = std::fs::read(path).map_err(|e| ErrorPayload {
        code: "PLAN_FILE_READ_FAILED".to_owned(),
        class: "preflight".to_owned(),
        exit_code: 2,
        message: format!("could not read {}: {e}", path.display()),
        hint: format!(
            "verify {} exists and is readable, or run `pearlite plan` to compute fresh",
            path.display()
        ),
        details: serde_json::Value::Null,
    })?;
    serde_json::from_slice::<pearlite_diff::Plan>(&raw).map_err(|e| ErrorPayload {
        code: "PLAN_FILE_PARSE_FAILED".to_owned(),
        class: "preflight".to_owned(),
        exit_code: 2,
        message: format!("could not parse {}: {e}", path.display()),
        hint: "the plan file's schema does not match this `pearlite` build (ADR-0010); \
             re-run `pearlite plan` and persist the new file"
            .to_owned(),
        details: serde_json::Value::Null,
    })
}

/// Sum the package counts across `PacmanRemove` and `CargoUninstall`
/// actions in `plan`. With `prune: false` this is always 0 because no
/// other code path currently emits removal actions; the threshold
/// check in [`dispatch_apply`] runs only on the prune branch.
fn count_pruned_packages(plan: &pearlite_diff::Plan) -> usize {
    plan.actions
        .iter()
        .map(|a| match a {
            pearlite_diff::Action::PacmanRemove { packages } => packages.len(),
            pearlite_diff::Action::CargoUninstall { .. } => 1,
            _ => 0,
        })
        .sum()
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
        GenCommand::Show { plan_id, plans_dir } => {
            let history_entry = state.history.iter().find(|h| h.plan_id == *plan_id);
            let failure_ref = state.failures.iter().find(|f| f.plan_id == *plan_id);
            if history_entry.is_none() && failure_ref.is_none() {
                return Envelope::failure(
                    metadata_at(Some(state.host.clone())),
                    ErrorPayload {
                        code: "GEN_NOT_FOUND".to_owned(),
                        class: "preflight".to_owned(),
                        exit_code: 2,
                        message: format!(
                            "no generation with plan_id {plan_id} in state.toml history or failures"
                        ),
                        hint: "pearlite gen list  # show known plan IDs and generations".to_owned(),
                        details: serde_json::Value::Null,
                    },
                );
            }
            let plans_dir = plans_dir
                .clone()
                .unwrap_or_else(|| default_plans_dir(&ctx.state_path));
            Envelope::success(
                metadata_at(Some(state.host.clone())),
                build_show_view(*plan_id, history_entry, failure_ref, &plans_dir),
            )
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

/// Build the `gen show` envelope `data` for a `plan_id` that has at
/// least one of: a history entry, a failure ref, or both.
///
/// Output shape (every field always present, `null` when the
/// underlying record is absent):
///
/// ```text
/// {
///   "plan_id":     <Uuid>,
///   "history":     <HistoryEntry view> | null,
///   "failure":     <FailureRef + parsed FailureRecord> | null,
///   "plan":        <persisted Plan JSON> | null
/// }
/// ```
///
/// All four fields are emitted unconditionally so consumers can rely
/// on field presence; `null` distinguishes "no such record" from "the
/// JSON file was unreadable", whereas a missing field would conflate
/// the two.
fn build_show_view(
    plan_id: uuid::Uuid,
    history: Option<&pearlite_state::HistoryEntry>,
    failure_ref: Option<&pearlite_state::FailureRef>,
    plans_dir: &Path,
) -> serde_json::Value {
    serde_json::json!({
        "plan_id": plan_id,
        "history": history.map_or(serde_json::Value::Null, full_history_entry_view),
        "failure": failure_ref.map_or(serde_json::Value::Null, failure_view),
        "plan": load_plan_json(plans_dir, plan_id).unwrap_or(serde_json::Value::Null),
    })
}

/// Render a [`FailureRef`](pearlite_state::FailureRef) as JSON, with
/// the parsed forensic [`FailureRecord`](pearlite_engine::FailureRecord)
/// embedded under `record` when the JSON file at `record_path` is
/// readable.
///
/// Missing or unparseable record files surface `record: null` —
/// matching the [`load_plan_json`] convention. The `class` /
/// `exit_code` / `failed_at` fields come from `state.toml` and remain
/// authoritative even when the on-disk JSON is gone (e.g. the
/// failures directory was wiped).
fn failure_view(f: &pearlite_state::FailureRef) -> serde_json::Value {
    let record = std::fs::read(&f.record_path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok());
    serde_json::json!({
        "plan_id": f.plan_id,
        "failed_at": iso8601(f.failed_at),
        "class": f.class,
        "exit_code": f.exit_code,
        "record_path": f.record_path,
        "record": record.unwrap_or(serde_json::Value::Null),
    })
}

/// Load `<plans_dir>/<plan-id>.json` and return its parsed content
/// as a [`serde_json::Value`]. Returns `None` if the file is missing
/// or unparseable.
fn load_plan_json(plans_dir: &Path, plan_id: uuid::Uuid) -> Option<serde_json::Value> {
    let path = plans_dir.join(format!("{}.json", plan_id.simple()));
    let raw = std::fs::read(&path).ok()?;
    serde_json::from_slice(&raw).ok()
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
        Command::Bootstrap { .. } => "pearlite bootstrap".to_owned(),
        Command::Reconcile => "pearlite reconcile".to_owned(),
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
        ApplyError::Userenv(_) => (
            "APPLY_USERENV",
            "home-manager --version  # verify home-manager is reachable for the target user, then retry"
                .to_owned(),
        ),
        ApplyError::NixProbe(_) => (
            "APPLY_NIX_PROBE_FAILED",
            "nix --version  # verify the nix binary is healthy on PATH".to_owned(),
        ),
        ApplyError::NixNotInstalled => (
            "NIX_NOT_INSTALLED",
            "pearlite bootstrap --installer-script <path>  # ADR-0012".to_owned(),
        ),
    };

    // NIX_NOT_INSTALLED and APPLY_NIX_PROBE_FAILED are preflight
    // errors: apply_plan returns before any system mutation, so they
    // don't write a FailureRef and don't follow the class-3/4
    // recoverable taxonomy. Surface as class=preflight, exit=2.
    let (class_label, exit_code, surfaced_class) = match err {
        ApplyError::NixNotInstalled | ApplyError::NixProbe(_) => ("preflight", 2_u8, 1_u8),
        _ => {
            let label = match default_class {
                2 => "plan",
                3 => "apply-recoverable",
                4 => "apply-incoherent",
                _ => "apply",
            };
            (label, default_exit_code, default_class)
        }
    };

    ErrorPayload {
        code: code.to_owned(),
        class: class_label.to_owned(),
        exit_code,
        message: format!("{err}"),
        hint,
        details: serde_json::json!({
            "plan_id": plan_id,
            "failure_class": surfaced_class,
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
    use pearlite_userenv::{MockHmBackend, MockNixInstaller};
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
            home_manager: Box::new(MockHmBackend::new()),
            nix_installer: Box::new(MockNixInstaller::new()),
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
                plan_file: None,
                prune: false,
                prune_threshold: 5,
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
                plan_file: None,
                prune: false,
                prune_threshold: 5,
            },
        }
    }

    fn args_for_apply_plan_file(state_file: PathBuf, plan_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Apply {
                host_file: None,
                snapper_config: "root".to_owned(),
                failures_dir: None,
                plans_dir: None,
                dry_run: false,
                plan_file: Some(plan_file),
                prune: false,
                prune_threshold: 5,
            },
        }
    }

    fn args_for_apply_prune(
        host_file: PathBuf,
        state_file: PathBuf,
        prune_threshold: usize,
    ) -> Args {
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
                plan_file: None,
                prune: true,
                prune_threshold,
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
            home_manager: Box::new(MockHmBackend::new()),
            nix_installer: Box::new(MockNixInstaller::new()),
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
    fn apply_plan_file_executes_a_persisted_plan() {
        // First apply produces a plan file at <state_dir>/plans/<plan-id>.json.
        // Second apply consumes it via --plan-file; the same plan_id ends up
        // in state.toml's history.
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);

        let plan_id = apply_and_get_plan_id(&state_path, &host);
        let plan_file = state_path
            .parent()
            .expect("parent")
            .join("plans")
            .join(format!("{}.json", plan_id.simple()));
        assert!(plan_file.exists(), "first apply must persist plan");

        // Reset state.toml to a clean baseline so the second apply
        // starts from generation 0 again.
        write_baseline_state(&state_path);

        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let env = dispatch(
            &args_for_apply_plan_file(state_path.clone(), plan_file),
            &ctx,
        );

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(
            data.get("plan_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            Some(plan_id.to_string()),
            "--plan-file must execute the plan_id from the file, not compute fresh"
        );
        // Verify state.toml grew a history entry with that plan_id.
        let read_back = StateStore::new(state_path).read().expect("read state");
        assert_eq!(read_back.history.len(), 1);
        assert_eq!(read_back.history[0].plan_id, plan_id);
    }

    #[test]
    fn apply_plan_file_missing_yields_read_failed() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let env = dispatch(
            &args_for_apply_plan_file(state_path, dir.path().join("does-not-exist.json")),
            &ctx,
        );

        let err = env.error.expect("error");
        assert_eq!(err.code, "PLAN_FILE_READ_FAILED");
        assert_eq!(err.exit_code, 2);
        assert_eq!(err.class, "preflight");
    }

    #[test]
    fn apply_plan_file_malformed_yields_parse_failed() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let bogus = dir.path().join("bogus.json");
        std::fs::write(&bogus, b"{ not valid json").expect("write bogus");

        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let env = dispatch(&args_for_apply_plan_file(state_path, bogus), &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "PLAN_FILE_PARSE_FAILED");
        assert_eq!(err.exit_code, 2);
    }

    /// Build a `RunContext` whose probe reports `forgotten_pkg` as
    /// explicitly installed, and whose `state.toml` lists it under
    /// `managed.pacman`. With `MINIMAL_HOST` (no `packages.core`), the
    /// classifier puts `forgotten_pkg` in the forgotten bucket — the
    /// substrate every prune-threshold test needs.
    fn forgotten_pacman_ctx(
        host_path: PathBuf,
        state_path: PathBuf,
        forgotten_pkg: &str,
    ) -> RunContext {
        let mut nickel = MockNickel::new();
        nickel.seed(host_path, MINIMAL_HOST);
        let probe = Box::new(MockProbe::with_state(ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: Some(PacmanInventory {
                explicit: [forgotten_pkg.to_owned()].into_iter().collect(),
                ..Default::default()
            }),
            cargo: Some(CargoInventory::default()),
            config_files: None,
            services: Some(ServiceInventory::default()),
            kernel: KernelInfo::default(),
        }));
        let engine = Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"));

        // state.toml flags forgotten_pkg as previously managed.
        let store = StateStore::new(state_path.clone());
        let state = State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed {
                pacman: vec![forgotten_pkg.to_owned()],
                ..Default::default()
            },
            adopted: pearlite_state::Adopted::default(),
            history: Vec::new(),
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: std::collections::BTreeMap::new(),
        };
        store.write_atomic(&state).expect("write state");

        RunContext {
            engine,
            state_path,
            fallback_host: "forge".to_owned(),
            pacman: Box::new(MockPacman::new()),
            cargo: Box::new(MockCargo::new()),
            systemd: Box::new(MockSystemd::new()),
            snapper: Box::new(MockSnapper::new()),
            home_manager: Box::new(MockHmBackend::new()),
            nix_installer: Box::new(MockNixInstaller::new()),
        }
    }

    #[test]
    fn apply_prune_executes_pacman_remove_under_threshold() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = forgotten_pacman_ctx(host.clone(), state_path.clone(), "xterm");

        let env = dispatch(&args_for_apply_prune(host, state_path, 5), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(
            data.get("actions_executed")
                .and_then(serde_json::Value::as_u64),
            Some(1),
            "exactly one PacmanRemove action runs (xterm)"
        );
    }

    #[test]
    fn apply_prune_above_threshold_yields_typed_error() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = forgotten_pacman_ctx(host.clone(), state_path.clone(), "xterm");

        // Threshold of 0 forces every forgotten removal to trip the
        // guard.
        let env = dispatch(&args_for_apply_prune(host, state_path, 0), &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "PRUNE_THRESHOLD_EXCEEDED");
        assert_eq!(err.exit_code, 2);
        assert_eq!(err.class, "preflight");
        assert!(err.message.contains("threshold is 0"));
        // details carries the count + threshold for agents to inspect.
        assert_eq!(
            err.details
                .get("prune_count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            err.details
                .get("prune_threshold")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
    }

    #[test]
    fn apply_without_prune_ignores_forgotten_packages() {
        // Same forgotten state, but apply WITHOUT --prune. The forgotten
        // package surfaces as drift only, not a removal action — so apply
        // succeeds with actions_executed == 0.
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let ctx = forgotten_pacman_ctx(host.clone(), state_path.clone(), "xterm");

        let env = dispatch(&args_for_apply(host, state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        assert_eq!(
            data.get("actions_executed")
                .and_then(serde_json::Value::as_u64),
            Some(0),
            "without --prune, forgotten is drift only, no removal action"
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
            home_manager: Box::new(MockHmBackend::new()),
            nix_installer: Box::new(MockNixInstaller::new()),
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
                gen_command: GenCommand::Show {
                    plan_id,
                    plans_dir: None,
                },
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

    /// Run a complete `pearlite apply` against the test ctx so the
    /// plan JSON is persisted next to a real history entry. Returns
    /// the `plan_id` of the entry just written.
    fn apply_and_get_plan_id(state_path: &Path, host_path: &Path) -> uuid::Uuid {
        let ctx = ctx_with(
            host_path.to_path_buf(),
            MINIMAL_HOST,
            state_path.to_path_buf(),
        );
        let env = dispatch(
            &args_for_apply(host_path.to_path_buf(), state_path.to_path_buf()),
            &ctx,
        );
        assert!(env.error.is_none(), "apply failed: {env:?}");
        let data = env.data.expect("data");
        data.get("plan_id")
            .and_then(serde_json::Value::as_str)
            .expect("plan_id")
            .parse()
            .expect("uuid parse")
    }

    #[test]
    fn gen_show_embeds_plan_content_when_file_exists() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        write_baseline_state(&state_path);
        let plan_id = apply_and_get_plan_id(&state_path, &host);

        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let env = dispatch(&args_for_gen_show(plan_id, state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        let plan = data.get("plan").expect("plan field");
        assert!(
            !plan.is_null(),
            "plan field must be populated when the JSON exists"
        );
        // Sanity-check it's a real Plan: has a plan_id matching the
        // history entry, and an actions array.
        assert_eq!(
            plan.get("plan_id").and_then(serde_json::Value::as_str),
            Some(plan_id.to_string()).as_deref()
        );
        assert!(
            plan.get("actions").is_some(),
            "embedded plan must carry actions"
        );
    }

    #[test]
    fn gen_show_plan_field_is_null_when_file_missing() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        // History entry exists, but no <plans_dir>/<plan-id>.json was
        // written (this is the disk-full / pre-PR-#36 state).
        let plan_id = write_state_with_history(&state_path, 5);

        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let env = dispatch(&args_for_gen_show(plan_id, state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        let plan = data.get("plan").expect("plan field");
        assert!(
            plan.is_null(),
            "plan must be null when the JSON file is absent, got {plan:?}"
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
        let history = data.get("history").expect("history field");
        assert!(
            !history.is_null(),
            "history must be populated for a known plan_id"
        );
        assert_eq!(
            history
                .get("snapshot_pre")
                .and_then(|v| v.get("id"))
                .and_then(serde_json::Value::as_u64),
            Some(77)
        );
        assert_eq!(
            history
                .get("snapshot_post")
                .and_then(|v| v.get("id"))
                .and_then(serde_json::Value::as_u64),
            Some(78)
        );
        assert_eq!(
            history
                .get("generation")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(
            data.get("failure").expect("failure field").is_null(),
            "no failure for a successful apply"
        );
    }

    /// Pre-seed `state_path` with a history-only state plus one
    /// `FailureRef` pointing at a JSON record we also write to disk.
    /// Returns `(plan_id, record_path)`.
    fn write_state_with_failure(state_path: &Path, failures_dir: &Path) -> (uuid::Uuid, PathBuf) {
        let plan_id = uuid::Uuid::now_v7();
        std::fs::create_dir_all(failures_dir).expect("mkdir");
        let record_path = failures_dir.join(format!("{}.json", plan_id.simple()));
        // Write a forensic record JSON. Anything serde-parseable will do
        // since gen show treats it as opaque Value.
        let record = serde_json::json!({
            "plan_id": plan_id,
            "failed_at": "2026-04-28T00:00:00.000000000Z",
            "class": 4,
            "exit_code": 5,
            "error_message": "service restart failed",
            "failed_action_executed_index": 0,
            "snapshot_pre": { "id": 9, "label": "pre-test", "created_at": "2026-04-28T00:00:00.000000000Z" },
            "post_fail_snapshot": null,
            "failed_action": { "ServiceRestart": { "unit": "sshd.service" } },
        });
        std::fs::write(
            &record_path,
            serde_json::to_vec_pretty(&record).expect("ser"),
        )
        .expect("write record");

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
            failures: vec![pearlite_state::FailureRef {
                plan_id,
                failed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
                class: 4,
                exit_code: 5,
                record_path: record_path.clone(),
            }],
            reserved: std::collections::BTreeMap::new(),
        };
        store.write_atomic(&state).expect("write state");
        (plan_id, record_path)
    }

    #[test]
    fn gen_show_embeds_failure_record_when_failure_ref_present() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let failures_dir = dir.path().join("failures");
        let (plan_id, _record_path) = write_state_with_failure(&state_path, &failures_dir);

        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let env = dispatch(&args_for_gen_show(plan_id, state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        // History is null (this plan only ever failed, never committed).
        assert!(data.get("history").expect("history").is_null());
        // Failure populated with state.toml fields plus the parsed record.
        let failure = data.get("failure").expect("failure");
        assert!(!failure.is_null());
        assert_eq!(
            failure.get("class").and_then(serde_json::Value::as_u64),
            Some(4)
        );
        assert_eq!(
            failure.get("exit_code").and_then(serde_json::Value::as_u64),
            Some(5)
        );
        let record = failure.get("record").expect("record field");
        assert!(
            !record.is_null(),
            "record must be populated when JSON file is readable"
        );
        assert_eq!(
            record
                .get("error_message")
                .and_then(serde_json::Value::as_str),
            Some("service restart failed")
        );
    }

    #[test]
    fn gen_show_failure_record_field_is_null_when_json_missing() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let failures_dir = dir.path().join("failures");
        let (plan_id, record_path) = write_state_with_failure(&state_path, &failures_dir);
        // Wipe the record JSON to simulate a partially gone failures dir.
        std::fs::remove_file(&record_path).expect("rm record");

        let ctx = ctx_with(
            dir.path().join("forge.ncl"),
            MINIMAL_HOST,
            state_path.clone(),
        );
        let env = dispatch(&args_for_gen_show(plan_id, state_path), &ctx);

        assert!(env.error.is_none(), "expected success, got {env:?}");
        let data = env.data.expect("data");
        let failure = data.get("failure").expect("failure");
        assert!(
            !failure.is_null(),
            "FailureRef in state.toml must still surface"
        );
        assert!(
            failure.get("record").expect("record field").is_null(),
            "record must be null when the JSON file is absent"
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
            home_manager: Box::new(MockHmBackend::new()),
            nix_installer: Box::new(MockNixInstaller::new()),
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
            home_manager: Box::new(MockHmBackend::new()),
            nix_installer: Box::new(MockNixInstaller::new()),
        };
        let args = args_for_rollback(plan_id, state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("error");
        assert_eq!(err.code, "ROLLBACK_SNAPPER");
        assert_eq!(err.exit_code, 4);
        assert_eq!(err.class, "apply-recoverable");
    }

    const HOST_WITH_NIX: &str = r#"
[meta]
hostname = "forge"
timezone = "UTC"
arch_level = "v4"
locale = "en_US.UTF-8"
keymap = "us"

[kernel]
package = "linux-cachyos"

[nix.installer]
expected_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;

    fn args_for_bootstrap(
        host_file: PathBuf,
        state_file: PathBuf,
        installer_script: PathBuf,
        nix_conf: PathBuf,
    ) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir: PathBuf::from("/etc/pearlite/repo"),
            state_file,
            command: Command::Bootstrap {
                host_file: Some(host_file),
                installer_script,
                nix_conf,
            },
        }
    }

    #[test]
    fn bootstrap_dispatches_through_engine_and_renders_outcome() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");

        let ctx = ctx_with(host.clone(), HOST_WITH_NIX, state_path.clone());
        let args = args_for_bootstrap(host, state_path, script, nix_conf);
        let env = dispatch(&args, &ctx);

        assert!(env.error.is_none(), "got error {:?}", env.error);
        let data = env.data.expect("data populated");
        assert_eq!(
            data.get("install").and_then(serde_json::Value::as_str),
            Some("installed")
        );
        assert_eq!(
            data.get("nix_conf_written")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn bootstrap_surfaces_nix_not_declared_when_block_missing() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let state_path = dir.path().join("state.toml");
        let nix_conf = dir.path().join("nix.conf");
        let script = dir.path().join("installer.sh");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write script");

        let ctx = ctx_with(host.clone(), MINIMAL_HOST, state_path.clone());
        let args = args_for_bootstrap(host, state_path, script, nix_conf);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("must error");
        assert_eq!(err.code, "NIX_NOT_DECLARED");
        assert_eq!(err.class, "preflight");
        assert_eq!(err.exit_code, 2);
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

    fn args_for_reconcile(config_dir: PathBuf, state_file: PathBuf) -> Args {
        Args {
            format: OutputFormat::Json,
            config_dir,
            state_file,
            command: Command::Reconcile,
        }
    }

    #[test]
    fn reconcile_dispatches_through_engine_and_writes_imported_ncl() {
        let dir = TempDir::new().expect("tempdir");
        let config_dir = dir.path().join("repo");
        let state_path = dir.path().join("state.toml");
        // ctx_with seeds a host file path into MockNickel, but reconcile
        // never consults the evaluator (see reconcile.rs:174). Any path
        // is fine here.
        let host = config_dir.join("hosts").join("forge.ncl");

        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let args = args_for_reconcile(config_dir.clone(), state_path);
        let env = dispatch(&args, &ctx);

        assert!(env.error.is_none(), "got error {:?}", env.error);
        let data = env.data.expect("data populated");
        assert_eq!(
            data.get("hostname").and_then(serde_json::Value::as_str),
            Some("forge")
        );
        let path_str = data
            .get("imported_path")
            .and_then(serde_json::Value::as_str)
            .expect("imported_path");
        assert!(
            path_str.ends_with("forge.imported.ncl"),
            "imported_path was {path_str}"
        );
        assert!(
            config_dir
                .join("hosts")
                .join("forge.imported.ncl")
                .is_file(),
            "imported.ncl was not written to disk"
        );
    }

    #[test]
    fn reconcile_surfaces_already_exists_when_file_present() {
        let dir = TempDir::new().expect("tempdir");
        let config_dir = dir.path().join("repo");
        let hosts = config_dir.join("hosts");
        std::fs::create_dir_all(&hosts).expect("mkdir");
        std::fs::write(hosts.join("forge.imported.ncl"), b"prior").expect("seed");

        let state_path = dir.path().join("state.toml");
        let host = config_dir.join("hosts").join("forge.ncl");

        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let args = args_for_reconcile(config_dir.clone(), state_path);
        let env = dispatch(&args, &ctx);

        let err = env.error.expect("must error");
        assert_eq!(err.code, "RECONCILE_ALREADY_EXISTS");
        assert_eq!(err.class, "preflight");
        assert_eq!(err.exit_code, 2);
        // Pre-seeded file must be untouched.
        let preserved = std::fs::read_to_string(hosts.join("forge.imported.ncl")).expect("read");
        assert_eq!(preserved, "prior");
    }

    #[test]
    fn reconcile_metadata_command_label() {
        let dir = TempDir::new().expect("tempdir");
        let config_dir = dir.path().join("repo");
        let state_path = dir.path().join("state.toml");
        let host = config_dir.join("hosts").join("forge.ncl");

        let ctx = ctx_with(host, MINIMAL_HOST, state_path.clone());
        let args = args_for_reconcile(config_dir, state_path);
        let env = dispatch(&args, &ctx);
        assert_eq!(env.metadata.command, "pearlite reconcile");
    }
}
