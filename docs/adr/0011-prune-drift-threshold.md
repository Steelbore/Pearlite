# ADR-0011: `apply --prune` drift threshold

**Date:** 2026-04-28
**Status:** Accepted
**Supersedes:** —
**Superseded by:** —

## Context

PRD §7.3 splits packages into four state-machine layers:

- **Forgotten** (`pacman -Qe` ∧ `state.managed` ∧ ¬declared) — the
  user installed via Pearlite, then dropped the declaration; the
  package should be removed.
- **Manual** (`pacman -Qe` ∧ ¬`state.managed` ∧ ¬`state.adopted`) —
  installed out of band; surface as drift, never auto-remove.
- **Adopted** (`pacman -Qe` ∧ `state.adopted`) — user-flagged
  "leave alone".
- **Protected** (`remove.ignore` from declared) — never flag or
  remove.

Forgotten packages are the safe-to-remove set, but Pearlite has been
emitting them as drift entries with a hint (`pearlite apply --prune
would remove it`). `--prune` itself wasn't wired.

The risk in flipping the switch is mass-deletion: a host file edit
that drops 200 packages would, without a guard, cause 200 removals
on the next `pearlite apply --prune`. Plan §14 lists "drift-threshold
default value" as an Open Question that resolves post-M6 on the
basis of operational data.

We can't wait. M2 W3 needs `--prune` working with a *conservative*
threshold; the post-M6 retrospective adjusts the default once we
have telemetry.

## Decision

`pearlite apply --prune` removes forgotten packages, with a
**default threshold of 5 packages** above which the CLI refuses to
proceed unless the operator explicitly confirms.

Concretely:

1. **`pearlite-diff::plan(prune: true)`** emits `PacmanRemove` /
   `CargoUninstall` actions for every forgotten package, in addition
   to the drift entries it already produces. With `prune: false`
   (default), the diff behaves exactly as it does today — drift
   entries only, no removal actions.
2. **The threshold is enforced at the CLI boundary**, not in the
   diff crate. `pearlite-diff` stays pure: it produces the actions;
   policy lives in `pearlite-cli`. This mirrors the failure-class
   classification split (mechanism in diff, policy in CLI).
3. **`--prune-threshold <N>`** overrides the default. The CLI counts
   forgotten removals in the plan and aborts with a typed error
   (`PRUNE_THRESHOLD_EXCEEDED`, exit 2) when the count exceeds the
   threshold. The error names the count and the threshold so
   operators can decide whether to bump the flag or audit the
   regression.
4. **The default value of 5 is provisional.** Post-M6 retrospective
   reviews telemetry from real applies and either ratifies the
   number or adjusts it; that revision lands as ADR-NNNN superseding
   this one.
5. **Removal of forgotten _adopted_ packages is forbidden** — the
   adopted flag overrides the prune. Operators who want to remove
   an adopted package undo the adoption first (`pearlite unadopt`,
   landing in M4).

## Consequences

**Pro.** Operators who want pruning get it without having to handle
the dangerous case (a stale state.toml flagging hundreds of
packages). The threshold catches the most common foot-gun — a
botched config-repo merge that drops half the host file — without
preventing the routine 1-2 package cleanup case.

**Pro.** The mechanism / policy split (diff produces removals; CLI
gates) leaves the door open for richer policies in M4 (e.g. a
config-repo level allow-list, or per-bucket thresholds) without
restructuring the diff.

**Con.** The default 5 is a guess. It might be too tight for hosts
with churny declarations (e.g. development boxes that toggle dozens
of packages a week) or too loose for production immutables. The
post-M6 retrospective will revisit; in the meantime users wedge
themselves out via `--prune-threshold N`.

**Con.** Cargo and pacman counts are summed against a single
threshold. A scenario with 4 pacman + 4 cargo forgottens = 8 total,
which trips the default-5. That seems right; cargo crates are no
less load-bearing than system packages on developer hosts.

**Con.** No `--dry-run`-style preview of what would be pruned beyond
what `pearlite apply --dry-run --prune` already shows. The plan
envelope's actions array names every removal; agents and operators
can inspect before applying. We rely on that rather than a separate
preview.
