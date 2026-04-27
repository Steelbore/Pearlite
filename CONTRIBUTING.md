# Contributing to Pearlite

Thanks for considering a contribution. Pearlite is part of Project Steelbore;
contributions follow the [Steelbore Standard v1.0](#) (engineering rules) and
the [Steelbore Self-Documenting CLI Standard v1.0.0](#) (CLI surface).

If you are an AI coding agent, read `AGENTS.md` and `CLAUDE.md` first — they
encode the project-specific invariants you must respect.

## Development environment

Prerequisites:

- **Rust 1.85** — pinned via `rust-toolchain.toml`. `rustup` will install on
  first `cargo` invocation.
- **A C linker** (`cc` / `gcc`) — needed by `pearlite-cli` and `pearlite-audit`.
  On NixOS or systems without a system gcc, run cargo inside `nix-shell -p
  gcc`.
- **Nickel ≥ 1.10** — for evaluating fixture host configs (M1+).
- **paru, snapper, btrfs-progs, systemd** — runtime targets. Not required to
  build, but required to exercise apply paths against a real system.

Quick start:

```sh
git clone https://github.com/Steelbore/Pearlite
cd pearlite
just ci      # fmt + clippy + test + audit + spdx + compliance
```

Recipes are defined in the `justfile`; CI runs the same recipes a contributor
runs locally, so anything that passes locally passes in CI (and vice versa).

## Branching

- All work lands on `main` via short-lived feature branches.
- `main` is protected: no direct pushes, all changes via PR.
- PRs are squash-merged with a Conventional Commits subject line.
- Milestone exits are signed tags: `m0-exit`, `m1-exit`, …, `m6-exit`.
  `v1.0.0` is tagged from `main` once `m6-exit` lands.

## Commit messages

Pearlite follows [Conventional Commits 1.0.0](https://www.conventionalcommits.org).

Permitted types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`,
`chore`, `revert`. Scope is the crate name without the `pearlite-` prefix:

```
feat(diff): handle config-file mode changes
fix(pacman): map paru exit code 127 to PacmanError::ParuMissing
docs(adr): add ADR-007 on no async runtime
```

Breaking changes use the `!` suffix or a `BREAKING CHANGE:` footer; either
triggers a major-version bump under semver (post-v1.0).

## Hooks

Install the pre-commit / pre-push hooks once:

```sh
cargo install rusty-hook --locked
```

`rusty-hook` reads `.rusty-hook.toml` at the workspace root. The hooks run:

- **pre-commit:** `just check && just spdx`
- **pre-push:**   `just test`

Anything taking more than ~10 seconds belongs in CI, not in a hook.

## CI tiers

| Tier | Trigger | Runs |
|---|---|---|
| T1 — Lint | every PR commit | `just check`, `just spdx`, `just unused` |
| T2 — Unit | every PR commit | `cargo nextest`, doc-tests |
| T3 — Adapter | every PR commit | mocked integration; `cargo audit`; `cargo deny`; `pearlite-audit` |
| T4 — VM | nightly + label `vm-test` | full bootstrap → apply → rollback in a CachyOS VM |

T1+T2+T3 must be green for a PR to merge.

## Code review

PRs require at least one approving review from a `CODEOWNERS`-listed reviewer
for any modified path, plus zero unresolved comments. Trivial PRs (typos,
dependency bumps) may be self-approved by maintainers but still need the CI
gate.

## SPDX headers

Every `.rs` file begins with:

```
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad
```

`scripts/ci/check-spdx.sh` enforces. CI fails if any tracked `.rs` file is
missing the header.

## Architecture decisions

Non-trivial design decisions are recorded as ADRs under `docs/adr/`. Format:
Context / Decision / Consequences (Michael Nygard short-form). Existing ADRs
are listed in `Pearlite-Plan-v1.0.docx` §13 — consult them before challenging
a settled choice.

## Adding a new crate

The 13-crate workspace is fixed for v1.0. Adding a fourteenth requires a
written ADR explaining why an existing crate cannot host the work and what
the dependency-graph impact is.

## Reporting bugs

Open a GitHub issue with:

- Pearlite version (`pearlite --version`).
- CachyOS / Arch version (`cat /etc/os-release`).
- Command run, full stderr, and any failure record JSON from
  `/var/lib/pearlite/failures/`.
- Whether `pearlite plan` reproduces the issue read-only.

For security issues, email the maintainer privately rather than opening a
public issue.
