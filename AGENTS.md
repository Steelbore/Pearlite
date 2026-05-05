# AGENTS.md — Pearlite

Project-specific invariants for AI agents working on Pearlite. The Steelbore
Standard, Microsoft Pragmatic Rust Guidelines, and the Self-Documenting CLI
Standard apply across all Steelbore projects and are not duplicated here.

## Authoritative documents

| Document | Role |
|---|---|
| `Pearlite-PRD-v1.0.docx` | What Pearlite does and why. Settled. |
| `Pearlite-Plan-v1.0.docx` | How it gets built. Operational. |
| `TODO.md` | Live task tracker. |

The PRD wins on behavioural questions; the Plan wins on process questions. If
a request conflicts with either, surface the conflict — do not silently choose.

## Workspace topology

```
crates/
  pearlite-schema/   pearlite-state/    pearlite-diff/    pearlite-fs/
  pearlite-nickel/   pearlite-pacman/   pearlite-cargo/   pearlite-systemd/
  pearlite-snapper/  pearlite-userenv/  pearlite-engine/  pearlite-cli/
  pearlite-audit/
```

Implementation order is fixed and topological: **pure → adapter → integrator**.
Pure crates depend on nothing in the workspace. Adapter crates implement traits
defined in `pearlite-engine`. CLI is last.

## Hard invariants

1. `#![forbid(unsafe_code)]` workspace-wide. No `unsafe` blocks anywhere.
2. **No async runtime.** Synchronous Rust throughout; `rayon::join` for
   probe-phase parallelism.
3. **No `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`**
   in production code paths. Tests may use `expect()` with descriptive messages.
4. **No `println!` / `eprintln!` / `print!`.** All CLI output goes through
   `pearlite-cli`'s renderer module. Workspace lints enforce.
5. **No shell.** Subprocess invocations always use `std::process::Command` with
   argv arrays. Never `sh -c`. Never string interpolation into a command.
6. **No `std::collections::HashMap` in plan-bearing types.** Use `BTreeMap` /
   `BTreeSet` for determinism.
7. **`state.toml` is mutated only by `pearlite-engine`.** No other crate writes
   it. `apply()` is the only function that calls `StateStore::write_atomic`.
8. **`state.toml` is the last file written on apply.** No resume mechanism — if
   apply dies before that final write, the next plan re-derives.
9. **No automatic rollback.** On Class 4/5 failure, halt + `post_fail`
   snapshot + write failure record + exit. The user runs `pearlite rollback`
   explicitly.
10. **Every error has a runnable hint.** `error.hint` is a literal command,
    not prose. CI asserts coverage over every `error.code`.
11. **All timestamps are ISO 8601 UTC.** No AM/PM. No locale-dependent output.
12. **Every `.rs` file begins with the SPDX header.** A pre-commit hook enforces.
    ```
    // SPDX-License-Identifier: GPL-3.0-or-later
    // Copyright (C) 2026 Mohamed Hammad
    ```

## The seven phases of apply (PRD §8.2)

```
1.   Snapshot pre
0.5  Repo prep (pacman.conf writes, then pacman -Sy)
2.   Removals (cargo → pacman)
3.   Installs (repo → cachyos → vN → AUR → cargo)
4.   Config writes (declaration order)
5.   Service state (mask → disable → enable)
6.   Service restarts (deduplicated)
7.   User env (home-manager switch as target user via runuser)
8.   Snapshot post
9.   State commit (atomic, last)
```

Phase order is law. Do not reorder. New operations get a new sub-phase
documented in the PRD.

## The Action enum (`pearlite-diff::Action`)

Every primitive operation is one variant. Adding an operation = one variant
plus one match arm in `pearlite-engine::exec`. No dispatch tables, no trait
objects in the hot path. Each variant must implement:

- `within_phase_key()` — deterministic sort key for in-phase ordering.
- `failure_coherence()` — `Recoverable` (Class 3) or `Incoherent` (Class 4).

## Failure classes (PRD §8.5)

| Class | Exit | Recovery |
|---|---|---|
| 1 Preflight | 2 | Fix env, retry. |
| 2 Plan | 3 | Fix env, retry. |
| 3 Recoverable apply | 4 | Fix root cause, re-apply. |
| 4 Incoherent apply | 5 | `pearlite rollback <plan-id>`. |
| 5 Catastrophic | n/a | `snapper rollback` then `pearlite reconcile`. |

The five-way taxonomy is fixed. Never invent a sixth.

## State.toml drift discriminator (PRD §7.3)

| Category | Discriminator | Action |
|---|---|---|
| Forgotten | `pacman -Qe` ∧ `state.managed` ∧ ¬declared | Propose remove |
| Manual | `pacman -Qe` ∧ ¬`state.managed` ∧ ¬`state.adopted` | Surface as drift; never auto-remove |
| Adopted | `pacman -Qe` ∧ `state.adopted` | Ignore |
| Protected | `remove.ignore` from declared | Never flag or remove |

Get this wrong and Pearlite removes packages the user wants to keep.

## Reconcile flow (PRD §11, M4 W1)

`pearlite reconcile` (read-only) probes the live system and writes a fresh
`<config_dir>/hosts/<hostname>.imported.ncl` as a **review draft** for operator
hand-curation. The emitted Nickel record carries `meta`, `kernel`, `packages`,
and `services` blocks from probe data; `users` and `config` are deliberately
emitted as empty arrays per PRD §11 (no `/etc/passwd` enumeration; no clobbering
of operator config-repo paths). The imported file is a draft, not a validated
declaration — the operator hand-curates it and renames it to `<hostname>.ncl`
for the next `pearlite plan`.

The interactive counterpart `pearlite reconcile --commit` (M4 W1 remainder)
commits the import to `state.toml` with a drift-threshold safety check. Until
it lands, reconcile only writes the review draft.

Error codes (all class 1 preflight, exit 2 — reconcile never mutates
`state.toml` and a failed atomic write leaves the operator config repo
untouched):

| `error.code` | Triggers |
|---|---|
| `RECONCILE_PROBE_FAILED` | adapter failure during probe |
| `RECONCILE_EMPTY_HOSTNAME` | blank `/etc/hostname` |
| `RECONCILE_INVALID_HOSTNAME` | `/`, `\`, or NUL in hostname |
| `RECONCILE_ALREADY_EXISTS` | refuses to clobber an existing imported.ncl |
| `RECONCILE_IO_FAILED` | mkdir or atomic-write failure |

VM-tier coverage: [`tests/vm/vm-10-reconcile-fresh-install.sh`](tests/vm/vm-10-reconcile-fresh-install.sh)
exercises the read-side end-to-end (happy path + clobber refusal).

## Always

- Run `cargo clippy --workspace --all-targets -- -D warnings` before claiming a
  change is done.
- Run `just compliance` before opening a PR.
- Add a test for every behavior change.
- Use trait-first discipline: define the trait in `pearlite-engine::traits`
  before writing the adapter; ship a `Live*` and a `Mock*` together.

## Never

- Never run `pearlite apply` on a real system to test changes. Use the VM
  harness (`tests/vm/`) or `MockProbe`.
- Never modify `state.toml` from outside `pearlite-engine`. Even tests go
  through `StateStore`.
- Never call out to `paru`, `pacman`, `systemctl`, `snapper`, `nickel`, `nix`,
  or `home-manager` from anywhere except the corresponding adapter crate.
- Never use `std::env::var` outside `pearlite-cli::agents`. The CLI is the one
  place that reads the environment.

## Capability tags

Every CLI subcommand declares its capabilities in `pearlite-cli::args`:

- `read` — does not modify state.
- `write` — modifies user-space state.
- `destructive` — modifies system state.
- `recoverable` — failures leave the system in a documented state.
- `requires_root` — must run as uid 0.

Agents in sandboxed sessions filter by these tags.

## Steelbore name registry

Pearlite is a metallurgical name (iron-carbon microstructure of alternating
ferrite and cementite — matches the layered architecture). Sister projects:
**Lattice** (NixOS), **Zamak** (bootloader), **Ferrocast** (PowerShell rewrite),
**Craton** (universal package manager), **Ironway** (OpenTTD rewrite),
**Caliper** (raster-to-vector tracing), **Mawaqit** (prayer times).

Do not propose names outside this convention.

---

*Last updated: 2026-05-04.*
