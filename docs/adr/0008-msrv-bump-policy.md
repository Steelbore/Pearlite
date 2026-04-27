# ADR-0008: MSRV bump policy

**Date:** 2026-04-27
**Status:** Accepted
**Supersedes:** —
**Superseded by:** —

## Context

Pearlite's `rust-toolchain.toml` pins `1.85` per Plan §4.5. M1 closed
with one outstanding consequence of that pin:
[RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009)
flags `time` 0.3.45 for a parse-time stack-exhaustion DoS. The fix
ships in `time` 0.3.47, which raised its own MSRV to **1.88**. We
ignore the advisory in `audit.toml` and `deny.toml` with a documented
exposure analysis: Pearlite never feeds untrusted strings to
`time::parse`; we only serialize/deserialize machine-emitted ISO 8601
in `state.toml` and `plan.json`. The DoS surface is unreachable
through any Pearlite API.

This is the first MSRV-vs-dep collision and almost certainly not the
last. M2 brings:

- `snapper-rs` (or equivalent) bindings — early-stage crate with its
  own MSRV trajectory.
- `cargo-deny` 0.19+ already requires rustc 1.88 to install; we
  sidestep via `taiki-e/install-action` pre-built binaries today,
  but that escape hatch isn't guaranteed forever.
- More CI tooling (cargo-dist for M6 release work) with its own MSRV
  curve.

We need a written policy so each MSRV-friction event isn't relitigated.

## Decision

1. **Hold MSRV at 1.85 through M2.** Don't bump mid-milestone. M2's
   apply-engine work is where regressions are most likely; minimising
   moving parts in the toolchain helps isolate the regression vs.
   compiler interaction.
2. **Bump at the M3 boundary** to whatever stable toolchain is
   "n minus one minor releases" old at that point — i.e. one release
   behind upstream, never the bleeding edge. This is conservative
   enough for shared-CI contributors and aggressive enough to keep
   tooling working.
3. **Each bump gets its own ADR.** Body is short: trigger (which dep
   forced the bump or which milestone reached the cadence), new MSRV,
   and the date. The bump ADR lives in this directory; this ADR-0008
   is updated only if the policy itself changes.
4. **Ignored advisories are reviewed at every bump.** When MSRV moves
   forward, every entry in `audit.toml`/`deny.toml` is re-evaluated:
   if the patched dep is now available under the new MSRV, the ignore
   comes out and the lock advances.
5. **Mid-milestone exception:** if a CVE rated High or Critical lands
   on a dep where the patched version requires a higher MSRV, we bump
   immediately. RUSTSEC-2026-0009 (Medium severity, unreachable
   surface) explicitly does not meet this bar.

## Consequences

- **Predictable cadence.** Contributors can pin local toolchains
  without surprise mid-milestone bumps.
- **Documented advisory ignore.** `RUSTSEC-2026-0009` survives M2
  with the exposure analysis recorded in two places (deny.toml,
  audit.toml) and one ADR (this one).
- **Mandatory review point.** The M3 retrospective explicitly
  re-checks every advisory ignore. Drift is bounded by the milestone
  cadence rather than ad-hoc.
- **Cost of holding.** Newer compiler features (e.g. lifetime
  inference improvements, `let-else` ergonomics that may land in
  later releases) are delayed until M3. Acceptable: nothing in the
  M1 codebase strained 1.85's expressiveness.

## References

- Plan §4.5 — toolchain pin definition.
- Plan §13 — short-form ADR catalogue.
- M1 retrospective `docs/retrospectives/M1.md` — surfaced this issue
  as a roll-forward item.
