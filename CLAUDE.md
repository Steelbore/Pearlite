# CLAUDE.md — Pearlite

> Project-specific invariants for Claude (and other coding agents) working on Pearlite.
> The Steelbore Standard, Rust idioms, and CLI conventions are NOT duplicated here — load the relevant skills first.

---

## Read these skills before doing anything

1. `steelbore-standard` — non-negotiable project-wide rules (palette, typography, ISO 8601, GPL-3.0-or-later, naming).
2. `rust-guidelines` — Microsoft Pragmatic Rust Guidelines; consult before writing any `.rs` file.
3. `steelbore-cli-standard` — Self-Documenting CLI Standard (SFRS v1.0.0); governs every CLI surface.
4. `steelbore-agentic-cli` — agent-facing UX (AGENTS.md authoring, MCP lazy loading, threat model).
5. `steelbore-cli-preference` — preferred CLI tool substitutions (`eza` over `ls`, `rg` over `grep`, `bat` over `cat`, `fd` over `find`, etc.).
6. `steelbore-cli-shell` — shell-syntax compliance (Nushell / Ion / POSIX / Bash). Pearlite ships under `nu` and `bash`; avoid Bash-isms in `.sh` files.
7. `steelbore-missing-pkg` — when a tool is missing, prefer ephemeral (`nix-shell`) over installs.

When the user mentions "Pearlite" or any subsystem name (Zamak, Lattice, Ferrocast, etc.), reload `steelbore-standard` immediately if not already loaded.

---

## Authoritative source documents

| Document | Path | Purpose |
|---|---|---|
| PRD | `Pearlite-PRD-v1.0.docx` | What Pearlite does and why. Settled. |
| Plan | `Pearlite-Plan-v1.0.docx` | How it gets built. Operational. |
| TODO | `TODO.md` | Current task state. |

If anything in this file conflicts with the PRD or Plan, the PRD/Plan wins. Surface the conflict; do not silently choose.

---

## Workspace topology

```
crates/
  pearlite-schema/   pearlite-state/    pearlite-diff/    pearlite-fs/
  pearlite-nickel/   pearlite-pacman/   pearlite-cargo/   pearlite-systemd/
  pearlite-snapper/  pearlite-userenv/  pearlite-engine/  pearlite-cli/
  pearlite-audit/
```

Implementation order is fixed and topological:
**pure → adapter → integrator**. Pure crates depend on nothing in the workspace. Adapter crates implement traits defined in `pearlite-engine`. CLI is last.

---

## Hard invariants (do not violate)

1. **`#![forbid(unsafe_code)]`** at workspace level. No `unsafe` blocks anywhere, including FFI (use `nix` or wrapper crates).
2. **No async runtime.** Synchronous Rust throughout; `rayon::join` for probe-phase parallelism. Do not import `tokio`, `async-std`, `smol`, or `futures::executor`.
3. **No `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`** in production code paths. Tests may use `expect()` with descriptive messages.
4. **No `println!` / `eprintln!` / `print!`.** All CLI output goes through `pearlite-cli`'s renderer module. Lints enforce.
5. **No shell.** All subprocess invocations use `std::process::Command` with argv arrays. Never `sh -c`. Never string interpolation into a command string.
6. **No `std::collections::HashMap` in plan-bearing types.** Use `BTreeMap` / `BTreeSet` for determinism. Clippy lint enforces in `pearlite-diff`.
7. **`state.toml` is mutated only by `pearlite-engine`.** No other crate writes it. `apply()` is the only function that calls `StateStore::write_atomic`.
8. **`state.toml` is the last file written on apply.** If apply dies before that final write, the next plan re-derives. There is no resume mechanism.
9. **No automatic rollback.** On Class 4/5 failure, halt + `post_fail` snapshot + write failure record + exit. The user runs `pearlite rollback` explicitly.
10. **Every error has a runnable hint.** `error.hint` is a literal command, not prose. CI asserts coverage over every `error.code`.
11. **All timestamps are ISO 8601 UTC.** No AM/PM. No 12-hour clock. No locale-dependent formatting.
12. **All units are metric (SI).** No imperial units anywhere in code or docs.
13. **Every `.rs` file begins with the SPDX header.** Pre-commit hook enforces:
    ```
    // SPDX-License-Identifier: GPL-3.0-or-later
    // Copyright (C) 2026 Mohamed Hammad
    ```

---

## Architectural commitments

### The seven phases of apply (PRD §8.2)

```
1. Snapshot pre
   0.5  Repo prep (pacman.conf writes + pacman -Sy)
2. Removals (cargo → pacman)
3. Installs (repo → cachyos → vN → AUR → cargo)
4. Config writes (declaration order)
5. Service state (mask → disable → enable)
6. Service restarts (deduplicated)
7. User env (home-manager switch as target user via runuser)
8. Snapshot post
9. State commit (atomic, last)
```

Phase order is law. Do not reorder. New operations get a new sub-phase, documented in the PRD.

### The Action enum (`pearlite-diff::Action`)

Every primitive operation is one variant. Adding an operation = one variant + one match arm in `pearlite-engine::exec`. No dispatch tables. No trait objects in the hot path.

Each variant must implement:
- `within_phase_key()` — deterministic sort key for in-phase ordering
- `failure_coherence()` — `Recoverable` (Class 3) or `Incoherent` (Class 4)

### Failure classes (PRD §8.5)

| Class | Exit | Meaning | Recovery |
|---|---|---|---|
| 1 | 2 | Preflight | Fix env, retry |
| 2 | 3 | Plan probe failed | Fix env, retry |
| 3 | 4 | Apply halted, coherent | Fix root cause, re-apply |
| 4 | 5 | Apply halted, incoherent | `pearlite rollback <plan-id>` |
| 5 | n/a | Catastrophic | `snapper rollback` + `pearlite reconcile` |

### State.toml layers (PRD §7.3)

| Category | Discriminator | Action |
|---|---|---|
| Forgotten | `pacman -Qe` ∧ `state.managed` ∧ ¬declared | Propose remove |
| Manual | `pacman -Qe` ∧ ¬`state.managed` ∧ ¬`state.adopted` | Surface as drift; never auto-remove |
| Adopted | `pacman -Qe` ∧ `state.adopted` | Ignore |
| Protected | `remove.ignore` from declared | Never flag or remove |

This four-way classification is the heart of safe pruning. Get it wrong and Pearlite removes packages the user wants to keep.

---

## Trait-first discipline

For every adapter:

1. Define the trait in `pearlite-engine::traits` first.
2. Review the trait against the engine's consumption pattern. Lock it.
3. Land two implementations: `LiveX` (production) and `MockX` (feature `test-mocks`).
4. Engine integration tests use the mock; adapter tests use both.

Never add a trait method without an immediate consumer in the engine. The trait surface is small because adding to it is expensive.

---

## CLI rules (SFRS v1.0.0)

- Noun-verb subcommands. `pearlite gen list`, not `pearlite list-generations`.
- Every command honors `--format <human|json|jsonl>`.
- `AI_AGENT=1`, `AGENT=1`, `CI=true` force `json` + no-color + non-interactive.
- `CLAUDECODE`, `CURSOR_AGENT`, `GEMINI_CLI` are informational; they appear in `metadata.invoking_agent` but do not change behavior.
- Every `error.code` has a `hint` registered in `pearlite-cli::hints`. CI asserts coverage.
- Exit codes: 0 success, 2 preflight, 3 plan, 4 recoverable apply, 5 incoherent apply, 6 reconcile-required, 64 usage. POSIX 126/127/128+N reserved.

---

## Agent-specific guidance

### Always

- Run `cargo clippy --workspace --all-targets -- -D warnings` before claiming a change is done.
- Run `just compliance` (== `pearlite-audit check .`) before opening a PR.
- Add a test for every behavior change. If the change is a refactor, the existing tests prove it.
- Use `bulletRich` / table styles consistent with the rest of the codebase.

### Never

- Never run `pearlite apply` to test changes. Use the VM test harness (`tests/vm/`) or write a unit test against `MockProbe`.
- Never modify `state.toml` from outside `pearlite-engine`. Even in tests, go through `StateStore`.
- Never invent a new failure class. The five-way taxonomy is fixed.
- Never write to `/etc`, `/var/lib/pearlite`, `/nix`, or any other system path from a test that isn't in the VM tier (T4).
- Never call out to `paru`, `pacman`, `systemctl`, `snapper`, `nickel`, `nix`, or `home-manager` from anywhere except the corresponding adapter crate.
- Never use `std::env::var` outside `pearlite-cli::agents`. The CLI is the one place that reads the environment.

### When unsure

- The PRD answers behavior questions. The Plan answers process questions. If neither answers it, write an ADR proposal in `docs/adr/` and surface to the maintainer.
- Default to refusing the change rather than guessing on architecture.

---

## Steelbore name registry (do not invent new names)

| Project | Role |
|---|---|
| **Pearlite** | Declarative system manager for CachyOS (this project) |
| Lattice | NixOS configuration |
| Zamak | Rust bootloader |
| Ferrocast | Rust PowerShell rewrite |
| Craton | Rust universal package manager |
| Ironway | Rust OpenTTD rewrite |
| Caliper | Rust raster-to-vector tracing engine |
| Mawaqit | Prayer-times application |

Names are metallurgical, geological, or mechanical-engineering terms that reward curiosity. Do not propose names outside this convention.

---

## File layout (where things go)

```
.
├── AGENTS.md                # All agents (this file's superset is for Claude)
├── CLAUDE.md                # This file
├── SKILL.md                 # Capability manifest
├── CONTRIBUTING.md          # Process for human contributors
├── CHANGELOG.md             # Keep-A-Changelog
├── TODO.md                  # Live task tracker
├── docs/
│   ├── adr/                 # ADR-NNNN-title.md
│   ├── book/                # mdBook architecture docs
│   └── retrospectives/      # M0.md ... M6.md
├── crates/                  # 13 workspace members
├── fixtures/                # test inputs (pacman captures, sample configs)
├── tests/vm/                # T4 integration scenarios
├── packaging/               # AUR PKGBUILDs
└── scripts/ci/              # check-spdx.sh, etc.
```

Do not create new top-level directories without an ADR.

---

## Capabilities tags (used in tools/list and pearlite schema output)

Every command declares its capabilities:

- `read` — does not modify state
- `write` — modifies user-space state (`state.toml`, codified sidecar)
- `destructive` — modifies system state (packages, configs, services)
- `recoverable` — failures leave the system in a documented state
- `requires_root` — must run as uid 0

Agents in sandboxed sessions filter by these tags. When adding a subcommand, declare its capabilities in `pearlite-cli::args`.

---

## Quick reference for common tasks

| Task | Where |
|---|---|
| Add a new package category | `pearlite-schema::PackageSet` + `pearlite-pacman::Repo` enum + diff classification |
| Add a new failure code | `pearlite-cli::errors` + register hint in `pearlite-cli::hints` + add to schema docs |
| Add a new VM scenario | `tests/vm/<NN>-<name>.sh` + reference in Plan §8.5 |
| Add an ADR | `docs/adr/NNNN-title.md` (Context / Decision / Consequences format) |
| Bump MSRV | `rust-toolchain.toml` + workspace `Cargo.toml` `rust-version` + ADR |
| Add a workspace dependency | `Cargo.toml` `[workspace.dependencies]` + per-crate `dep = { workspace = true }` |

---

*Last updated: 2026-04-27. Length: ~200 lines (within Plan §9.5 budget).*
