# ADR-0013: Per-user `nix.conf` is left to Home Manager

**Date:** 2026-04-29
**Status:** Accepted
**Supersedes:** —
**Superseded by:** —

## Context

Home Manager's `switch` requires `experimental-features =
nix-command flakes` to be enabled at the daemon or per-user level.
Without it, `nix profile`-style invocations that HM uses internally
fail with `error: experimental Nix feature 'nix-command' is disabled`.

Plan §7.4 W1 listed "per-user `nix.conf` handling" alongside the
`runuser` drop wrapper, but M3 only shipped runuser. The M3
retrospective marked nix.conf provisioning `[~]` and called for an
ADR before any code lands.

Three approaches were considered:

1. **Pearlite writes `~/.config/nix/nix.conf`** from a declared
   block in the per-host Nickel config — e.g.
   `users.alice.nix.conf.experimental_features =
   ["nix-command", "flakes"]`. Pearlite owns the file; HM doesn't
   touch it.
2. **Pearlite leaves `~/.config/nix/nix.conf` alone.** HM already
   exposes a `nix.settings` block that compiles down to a per-user
   nix.conf; the operator declares `experimental-features` there.
   Pearlite establishes the system-wide minimum (`/etc/nix/nix.conf`)
   during bootstrap and stays out of the per-user file's way.
3. **Hybrid.** Pearlite writes a baseline `experimental-features =
   nix-command flakes` to per-user nix.conf if the file is missing
   or doesn't contain that line; otherwise leaves it alone.

The hybrid option's "if missing" branch is the kind of magic that
breaks operator mental model: HM's `nix.settings` block silently
loses ground to a Pearlite-owned line that the operator didn't
declare. Ownership questions emerge: if HM and Pearlite both want to
write the same key, which wins? Three-way merges of generated config
are exactly the muddle ADR-001 (Nickel for human config, TOML for
machine state) was written to avoid.

## Decision

**Adopt option 2: Pearlite does not manage per-user nix.conf.**

Concretely:

1. **`pearlite bootstrap` (per ADR-0012) writes
   `/etc/nix/nix.conf`** with `experimental-features = nix-command
   flakes` and `auto-optimise-store = true` (the Determinate
   defaults). This is the system-wide minimum that lets HM's first
   `switch` succeed. The write is idempotent: if the keys are
   already present and match, the file isn't touched.
2. **`~/.config/nix/nix.conf` is HM territory.** Operators who want
   per-user overrides declare them in the HM `home.nix` via the
   `nix.settings` Nickel block. HM compiles that into the per-user
   file; the file's contents are owned by HM's generation, not by
   Pearlite.
3. **No `[[managed.nix_conf]]` block in `state.toml`.** Per-user
   nix.conf isn't a Pearlite-managed file. The system-wide
   `/etc/nix/nix.conf` is touched only by `bootstrap`, which
   doesn't record state (per ADR-0012 decision 4). If Pearlite later
   needs to verify the system-wide file has its expected contents,
   that's a preflight concern, not a `state.toml` row.
4. **The host config's documentation** — `docs/book/host-config.md`
   when it lands in M6 — points operators at HM's `nix.settings`
   for any per-user nix configuration questions. The cross-reference
   is "if you want to set a nix.conf option for user X, declare it
   in their HM config; Pearlite does not provide a separate path."

## Consequences

- **No new managed file.** `state.toml`'s `[[managed.*]]` blocks
  stay at the M3 set: pacman, cargo, services, config_files,
  user_env. The config-file inventory in PRD §7.3 is unchanged.
- **One write site for `/etc/nix/nix.conf`.** Inside
  `pearlite bootstrap`. Idempotent (read-then-write-if-different),
  no `[[managed.config_files]]` entry, no diff-time involvement.
- **HM's `nix.settings` is the documented per-user surface.**
  Pearlite's docs, error hints, and bootstrap output all cross-link
  to the HM option. Operators who try to declare a Pearlite-side
  per-user nix block get an explicit error pointing at HM.
- **No competing source of truth.** The "two systems writing the
  same file" problem (HM and Pearlite both managing
  `~/.config/nix/nix.conf`) is structurally eliminated. The
  `experimental-features` line that bootstrap establishes
  system-wide is sufficient for HM's first switch; per-user
  refinements happen exclusively through HM.
- **No fleet-wide knob for per-user nix settings.** v1.0 ships
  without this. v1.1's fleet mode (PRD §17.1) may revisit if real
  operations show a need; that revision supersedes this ADR.
- **Cost of holding.** Operators who prefer to manage everything
  through Pearlite's host file (rather than HM's `home.nix`) lose
  one knob. That preference is a v1.1+ topic; v1.0 commits to the
  Nickel-host-file + HM-home.nix split as the canonical division.

## References

- ADR-001 — Nickel for human config, TOML for machine state
  (Plan §13.1).
- ADR-0012 — Nix bootstrap wiring (this ADR's bootstrap-side
  partner).
- M3 retrospective `docs/retrospectives/M3.md` §"What didn't"
  and §"Actions for M4" item 3.
- Plan §7.4 W1 — original "per-user nix.conf" task.
- Home Manager docs — `nix.settings` option (linked from M6 book
  chapter when written).
