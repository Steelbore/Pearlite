# ADR-0010: `--plan-file` schema-stability rules

**Date:** 2026-04-28
**Status:** Accepted
**Supersedes:** —
**Superseded by:** —

## Context

PR #36 made `pearlite apply` persist its computed plan to
`<state_dir>/plans/<plan-id>.json` so operators have a forensic
artifact for every apply. The natural follow-on is `pearlite apply
--plan-file <path>`: re-execute a previously persisted plan instead
of computing a fresh one. Plan §14 lists this as an Open Question:

> `--plan-file` schema_version stability rules (resolves: M2 ADR)

The question is what happens when the plan file was produced by a
different `pearlite` build than the one consuming it. Two failure
modes are most concerning:

1. **Action-variant additions.** Future versions add new
   `pearlite_diff::Action` variants. An old plan written by the new
   binary must not be silently truncated when read by an old binary.
2. **Action-variant semantics.** Existing variants gain new fields
   (e.g. `ConfigWrite` adding a SELinux label). An old plan missing
   the new field must either be rejected or filled in with a
   conservative default.

Pearlite is pre-1.0 and has no backwards-compatibility commitment
yet, but the choice we make now constrains how messy migration gets
in v1.x.

## Decision

**v1.0 consumes only plans whose serialized form round-trips
losslessly through the current build's `pearlite_diff::Plan` type.**

Concrete rules:

1. The reader uses `serde_json::from_slice::<pearlite_diff::Plan>(...)`
   with no special handling. Unknown fields are tolerated by default
   (serde permissive), but **unknown enum variants** in `actions`
   cause deserialization to fail.
2. No `schema_version` field is added to `pearlite_diff::Plan` in
   this milestone. The hash of the workspace `tool_version` already
   appears in `state.toml`'s `[[history]].tool_version`; cross-build
   debugging routes through that field.
3. The reader does **not** verify `plan.host` against the current
   host. Operators are trusted to pass the right plan file; mismatch
   manifests at apply time when snapper / pacman invocations fail.
4. **Future bumps must each get an ADR** documenting the migration:
   either an explicit version field added to `Plan`, an explicit
   reader shim, or a documented break.

## Consequences

**Pro.** Zero new schema surface. The reader is a one-liner. The
strictness catches the most dangerous case (unknown action variants)
without needing version-comparison logic that itself can drift.

**Pro.** Plans persisted today by PR #36 are readable today. The
forensic artifact already in `<state_dir>/plans/` is usable from
the moment `--plan-file` ships.

**Con.** Cross-version replay is unsupported in v1.0. An operator
who upgrades `pearlite` and tries `apply --plan-file` with a plan
written by the old build gets a deserialization error if the action
shape changed. The hint surfaces `pearlite plan` as the recovery
path.

**Con.** No automated detection of "this plan is stale" — if the
user authors a plan, edits the host file, then `--plan-file`s the
old plan, they re-execute the stale plan. This matches how
`pearlite apply` already works without the flag (it computes fresh
each time), but the foot-gun is sharper with a saved plan. Mitigation
is documentation, not code.
