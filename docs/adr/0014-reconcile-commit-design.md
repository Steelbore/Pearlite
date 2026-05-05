# ADR-0014: `Engine::reconcile_commit` design

**Date:** 2026-05-05
**Status:** Accepted
**Supersedes:** —
**Superseded by:** —

## Context

PRD §11 splits reconcile into two phases. The read-side
(`Engine::reconcile`) shipped in M4 W1: it probes the live system
and writes `<config_dir>/hosts/<hostname>.imported.ncl` as a review
draft (#65, #67). The interactive write-side, `reconcile_commit`,
remains TBD.

`reconcile_commit` resolves the four-way drift classification from
PRD §7.3 — Forgotten / Manual / Adopted / Protected — by giving the
operator a chance to *adopt* Manual items into `state.toml`, moving
them out of drift forever. Without this command, drift either stays
(operator does nothing) or gets pruned (`apply --prune`); there is no
"keep this, but stop showing it as drift" path.

Three architectural questions block implementation:

1. **Drift-threshold for adoption.** Mass-adopting on a stale
   `state.toml` is a foot-gun parallel to ADR-0011's prune scenario:
   a host that drifted 200 packages out-of-band shouldn't be one
   `pearlite reconcile --commit` away from claiming all 200 as
   Pearlite-managed.
2. **Prompt UX and non-interactive policy.** What happens under
   `AI_AGENT=1` or a piped invocation? Per-package prompting, bulk
   `--adopt-all`, or refusal — each maps to different operator
   expectations and risk profiles.
3. **`ReconciliationEntry` semantics.** The
   [`pearlite_state::ReconciliationEntry`](../../crates/pearlite-state/src/reconciliation.rs)
   schema is sketched (`plan_id`, `committed_at`,
   `action: ReconciliationAction`, `package_count`) but missing the
   per-package decision trail that an audit needs three months later.

## Decision

**Mirror ADR-0011's prune-threshold pattern; refuse to silently
mass-adopt in non-interactive mode; extend `ReconciliationEntry`
rather than the action enum.**

Concretely:

1. **Default threshold of 5 drift items**, pooled across pacman,
   cargo, services, and config-files. Above the threshold, the CLI
   refuses to proceed unless the operator passes `--adopt-all` or
   raises `--commit-threshold N`. Pooling matches ADR-0011's
   reasoning — drift items are roughly equivalent in audit-blast-
   radius; per-category thresholds add complexity without observed
   demand.
2. **The threshold is enforced at the CLI boundary**, not in the
   engine. `Engine::reconcile_commit` takes the threshold as a
   parameter; `pearlite-cli` counts drift items in the probed plan
   and aborts with `RECONCILE_THRESHOLD_EXCEEDED` (class
   `preflight`, exit 2) before the engine method runs. Same
   mechanism / policy split as ADR-0011.
3. **`--adopt-all` bypasses the threshold and the prompts**, accepts
   every Manual item, and is *combinable* with
   `--commit-threshold N` to cap blast radius even when bypassing
   prompts (e.g. `--adopt-all --commit-threshold 50` says "adopt
   silently up to 50; refuse beyond").
4. **TTY + human:** per-item prompt with the menu
   `[y]es / [N]o (default) / [a]dopt-all / [s]kip-all / [q]uit`.
   Bare-Enter defaults to `N` (skip). `a` switches to bulk-accept
   for the rest of this run. `s` switches to bulk-skip. `q` aborts
   without writing `state.toml`.
5. **Non-interactive (no TTY, or any of `AI_AGENT=1` / `AGENT=1` /
   `CI=true`) without `--adopt-all`:** refuse with
   `RECONCILE_REQUIRES_INTERACTIVE` (class `preflight`, exit 2).
   Hint: `pearlite reconcile --commit --adopt-all` (or run from a
   TTY). This protects against silent mass-adoption when no operator
   is watching.
6. **Env-var detection** lives behind
   `pearlite_cli::agents::is_non_interactive()`, a helper that ships
   in the M4 implementation PR as a stub returning
   `!io::stdin().is_terminal() || env::var_os("AI_AGENT").is_some()`.
   M5 W2 fills in the full `AGENT` / `CI` / `CLAUDECODE` arms per
   the existing TODO.md schedule.
7. **`ReconciliationEntry` extends; the enum stays.**
   `ReconciliationAction` remains a `Copy` unit-variant enum
   (`AdoptAll`, `Interactive`, `Skipped`) — no breaking schema
   change. Two new fields land on `ReconciliationEntry`:
   - `adopted: Vec<String>` — the package names actually moved into
     `state.adopted` by this commit. Always present; may be empty.
   - `skipped: Vec<String>` — the package names the operator
     declined. Always present; may be empty.
   Both default to `Vec::new()` for backward-compatible
   deserialization of pre-ADR `[[reconciliations]]` entries. The
   enum still tells you the *policy*; the vectors tell you the
   *decisions*.
8. **`plan_id`** is a freshly generated `Uuid::now_v7()` per
   `reconcile_commit` invocation. Reconcile-commit is its own event;
   no reuse of any prior plan UUID.
9. **`package_count`** is the number of drift items *considered*
   (the count of Manual items in the probe), not just the adopted
   subset. This provides the audit denominator: "of N considered, K
   adopted, S skipped" with `K + S <= N` when `q` aborts mid-flight.

## Consequences

**Pro.** Operators get a recorded, auditable adoption trail (named
`adopted` and `skipped` vectors), not just a bulk count. Three
months later, `pearlite gen show <reconcile-id>` answers "what did
I adopt last Tuesday."

**Pro.** Threshold + interactive default protect against the
foot-gun where a host drifted unexpectedly (botched dotfile sync,
forgotten `pacman -S` session) and the operator reflexively reaches
for `pearlite reconcile --commit`.

**Pro.** No `state.toml` schema break. The `ReconciliationAction`
enum keeps its existing TOML representation (`adopt_all`,
`interactive`, `skipped` strings); old entries deserialize cleanly
with the new vectors defaulting to `[]`. The `migrate()` framework
is not invoked — the change is additive.

**Con.** The default threshold of 5 may be too tight for hosts with
genuinely rich Manual sets (a desktop with dozens of one-off
installs). Operators wedge themselves out via `--commit-threshold N`
or `--adopt-all`; the post-M6 retrospective revisits the number
alongside ADR-0011's prune threshold.

**Con.** Non-interactive refusal (rule 5) means automated reconcile
cron jobs need explicit `--adopt-all`. That is the point — silent
unattended adoption is the failure mode this ADR exists to prevent
— but it shifts operator workflow assumptions for anyone scripting
reconcile.

**Con.** `pearlite_cli::agents::is_non_interactive()` ships
partially in M4 (TTY + `AI_AGENT` only) and gets its full env-var
arms in M5. During the gap, `AGENT=1` from a TTY does NOT trigger
non-interactive mode. The M4 implementation PR documents this in a
`TODO(M5)` comment on the helper; the M5 PR removes the comment.

## References

- ADR-001 — Nickel for human config, TOML for machine state
  (Plan §13.1) — informs the no-schema-break stance in rule 7.
- ADR-0010 — `--plan-file` schema-stability rules (M2 W3) — same
  additive-deserialize discipline applied to `state.toml`.
- ADR-0011 — `apply --prune` drift threshold (M2 W3) — direct
  precedent for rules 1–3.
- [`crates/pearlite-engine/src/reconcile.rs`](../../crates/pearlite-engine/src/reconcile.rs) §lines 8–10 — existing roadmap comment that
  scoped this ADR.
- [`crates/pearlite-state/src/reconciliation.rs`](../../crates/pearlite-state/src/reconciliation.rs) —
  `ReconciliationEntry` / `ReconciliationAction` schema this ADR
  extends.
- PRD §7.3 + PRD §11 — drift classification and reconcile semantics.
- Plan §7.5 W1 — original `reconcile_commit` task.
- TODO.md §"Open Implementation Questions" — drift-threshold default
  (this ADR resolves the M4-relevant subset; ADR-0011 resolved the
  prune subset; the post-M6 retrospective consolidates).
