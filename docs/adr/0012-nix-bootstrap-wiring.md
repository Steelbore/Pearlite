# ADR-0012: Nix bootstrap wiring as a dedicated subcommand

**Date:** 2026-04-29
**Status:** Accepted
**Supersedes:** —
**Superseded by:** —

## Context

M3 PR #45 shipped `LiveNixInstaller` — the Determinate Nix installer
wrapper governed by ADR-004 — with full hash-pin defence and a
`MockNixInstaller` for tests. The adapter is complete in isolation,
but no place in the engine or CLI invokes it. Without a caller, the
M3 retrospective rolled both the apply-time wiring and
`vm-09-nix-bootstrap.sh` into M4 W1.

Two open questions block the wiring:

1. **Where does the expected SHA-256 come from?** The Determinate
   installer ships under a moving URL. ADR-004 mandates the hash-pin;
   the question is whose responsibility the pin is. Options:
   - Per-host Nickel config: `nix.installer.expected_sha256 = "…"`.
   - Top-level repo-wide `bootstrap.toml` next to `state.toml`.
   - Baked-in pinned hash refreshed at every Pearlite release.

2. **When does the installer run?** Two shapes:
   - Auto phase 0.0 of `pearlite apply` — the installer becomes part
     of the seven-phase pipeline, runs implicitly when nix is missing
     and any host has `home_manager.enabled = true`.
   - Dedicated `pearlite bootstrap` subcommand — runs explicitly,
     once, per host. `pearlite apply` halts with a preflight error
     if nix is missing.

The risk with auto-phase-0 is the snapper rollback story: the
installer touches `/nix` (and on btrfs hosts, `/nix` lives on its own
subvolume by Determinate's design) plus `/etc/profile.d/nix.sh`,
`/etc/nix/nix.conf`, and a systemd unit. None of those land inside
the snapper subvolume that wraps `/`. A pre-apply snapper snapshot
would not capture the installer's effects, breaking the
"apply is reversible via `pearlite rollback`" mental model embedded
in PRD §11 and CLAUDE.md hard invariant 9.

## Decision

1. **Run the installer from a dedicated `pearlite bootstrap`
   subcommand**, not as auto-phase-0 of `apply`. Apply stays a pure
   declarative diff-and-execute over snapshotted state; bootstrap is
   the explicit, one-shot side-effect on the host that establishes
   the prerequisites apply expects.
2. **The expected SHA-256 lives in the per-host Nickel config** under
   `nix.installer.expected_sha256`. Rationale: hosts already declare
   nix presence (`home_manager.enabled`); the SHA pin sits next to
   the consumer that needs it. A central `bootstrap.toml` decouples
   the pin from the host that uses it; a baked-in hash forces a
   Pearlite release for every Determinate version bump. Per-host is
   the right granularity.
3. **`pearlite apply` halts at preflight (exit 2) with
   `error.code = NIX_NOT_INSTALLED`** when any host has
   `home_manager.enabled = true` and `nix --version` fails. The
   `error.hint` is the literal `pearlite bootstrap` command. This
   keeps apply pure-declarative; bootstrap is the one-shot
   side-effect.
4. **Bootstrap state isn't recorded in `state.toml`.** Nix presence
   is a runtime fact (queried via `nix --version`), not a managed
   declaration. Re-bootstrapping is a no-op via the `nix --version`
   short-circuit already shipped in `LiveNixInstaller`. The
   `[[managed.*]]` blocks in state.toml are reserved for things
   Pearlite owns and might remove on prune; the nix daemon is
   neither.
5. **`vm-09-nix-bootstrap.sh`** exercises the bootstrap command
   end-to-end on a clean image (no nix present) and re-runs to
   verify the short-circuit. The scenario is gated behind
   `PEARLITE_VM_TEST=1` like the other VM scenarios.

## Consequences

- **One new CLI subcommand** (`pearlite bootstrap`) and one new
  preflight check in `apply`. The subcommand is `read + write`
  capability per CLAUDE.md's tag system; `destructive` would imply a
  rollback path the side-effect doesn't have.
- **`LiveNixInstaller` finally has a caller.** Gains an integration
  test wired through `pearlite-cli`'s bootstrap renderer.
- **Bootstrap output bypasses snapper.** Consequence of choosing a
  dedicated subcommand: if the installer corrupts the host, the user
  recovers via Determinate's own uninstall path, not
  `pearlite rollback`. Documented in the bootstrap subcommand's help
  text and in the M4 W1 retro callout.
- **Per-host SHA pin requires per-host edits.** When Determinate
  releases a new installer version, every host file needs its
  `expected_sha256` bumped. Acceptable — the cadence is "one bump per
  Pearlite-relevant Determinate release", which historically runs at
  ~quarterly intervals. The CLI's error message names the expected
  vs. observed SHA verbatim so operators can paste the correct value.
- **No `bootstrap.toml`.** The repo-root config-file count stays at
  `state.toml` only. Future state may grow more (e.g. fleet config in
  v1.1), but bootstrap doesn't earn a new file.

## References

- ADR-004 — Determinate Nix installer over the official one
  (Plan §13.4).
- M3 retrospective `docs/retrospectives/M3.md` §"What didn't" and
  §"Actions for M4" item 1.
- PRD §8.2 — seven-phase apply (no bootstrap phase added).
- PRD §11 + CLAUDE.md hard invariant 9 — apply reversibility via
  snapper.
