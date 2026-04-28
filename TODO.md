# Pearlite — TODO

> Implementation tracker for Pearlite v1.0.0.
> Source of truth for tasks: `Pearlite-Plan-v1.0.docx` §7.
> Mark items with `✓` when complete; leave `[ ]` for outstanding.

---

## Phase 0 — Design

- [✓] Pearlite PRD v1.0 (55 pages, 18 sections)
- [✓] Pearlite Implementation Plan v1.0 (67 pages, 15 sections)
- [ ] Mawaqit-style brand assets adapted for Pearlite (logo, README banner)
- [✓] GitHub repository created at `github.com/Steelbore/Pearlite`
- [✓] Branch protection on `main` configured (per Plan §3.4)
- [✓] CODEOWNERS file authored
- [ ] AUR namespaces reserved (`pearlite-bin`, `pearlite`, `pearlite-git`) — deferred to M6 per Plan §7.7

---

## M0 — Walking Skeleton (1 week)

- [✓] Init git repo; configure branch protection (Plan §3.4 + §5.10 active; `require_code_owner_reviews` true after CODEOWNERS landed via PR #1)
- [✓] Workspace `Cargo.toml` (resolver "3", workspace metadata, lints, profiles)
- [✓] `rust-toolchain.toml` pinning 1.85
- [✓] `deny.toml`, `clippy.toml`, `rustfmt.toml`
- [✓] 13 crate skeletons under `crates/` — each compiles with empty `lib.rs`
- [✓] Wire clap in `pearlite-cli`; implement `--version` and `--help` only
- [✓] `LICENSE` (GPL-3.0-or-later), `README.md`, `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, `CHANGELOG.md`
- [✓] `justfile` with `check` / `test` / `audit` / `spdx` / `compliance` recipes
- [✓] `scripts/ci/check-spdx.sh`; `.rusty-hook.toml` config (contributors run `cargo install rusty-hook --locked` once)
- [✓] GitHub Actions: `ci.yml`, `vm.yml` (skeleton), `release.yml` (skeleton)
- [✓] `pearlite-audit` binary with one trivial check (SPDX-001) wired into CI
- [✓] Three AUR PKGBUILDs (pearlite-bin, pearlite, pearlite-git) for `0.1.0-alpha.0`
- [~] Tag `m0-exit` (signed tag from this commit); CachyOS-container PKGBUILD verification deferred to M6 since the container itself is M1 work

---

## M1 — Read-Only Plan (3 weeks)

### Week 1 — Pure crates
- [✓] `pearlite-schema`: full implementation per Plan §6.1 (PRs #3 / #4 / #5)
- [✓] `pearlite-state`: FileSystem trait, atomic write, migration framework (PRs #6 / #7 / #8)
- [✓] `pearlite-fs`: sha256, atomic write, ConfigFileInventory (PR #9)

### Week 2 — Adapter probe paths
- [✓] `pearlite-pacman::inventory` + classification (no install/remove yet) (PR #13)
- [✓] `pearlite-cargo::inventory` (PR #11)
- [✓] `pearlite-systemd::inventory` (PR #12)
- [✓] `pearlite-nickel` (LiveNickel + MockNickel) (PR #10)

### Week 3 — Diff and CLI wiring
- [✓] `pearlite-diff::plan()` with property tests (PRs #14 / #15 / #16)
- [✓] `pearlite-engine::plan()` (probe + diff composition) (PR #17)
- [✓] `pearlite-cli`: `plan`, `status`, `schema --bare` subcommands (PR #18)
- [✓] JSON envelope rendering (PR #18)
- [ ] VM scenario `vm-01-bootstrap-and-plan.sh` — deferred to M2 W3 per M1 retrospective (needs self-hosted CachyOS runner)
- [~] Tag `m1-exit` (signed tag from this commit)

---

## M2 — Apply Engine (3 weeks)

### Week 1 — Apply-side adapters
- [✓] `pearlite-pacman`: install / remove / sync_databases (PR #22)
- [✓] `pearlite-cargo`: install / uninstall (PR #23)
- [✓] `pearlite-systemd`: enable / disable / mask / restart (PR #24)
- [✓] `pearlite-snapper`: create / rollback / list (PR #21)

### Week 2 — Engine orchestration
- [✓] `pearlite-engine::apply()` — phases 1..6 + 8 + 9 (phase 7 user env in M3) (PRs #27 / #28 / #29)
- [✓] `Action::within_phase_key()` and `Action::failure_coherence()` for every variant (PRs #25 / #26)
- [✓] Failure record writing per PRD §11.4 (PR #30)
- [✓] `pearlite-engine::rollback()` (PR #31)

### Week 3 — CLI integration & VM tests
- [✓] `pearlite apply`, `apply --dry-run`, `apply --plan-file`, `apply --prune` (PRs #32 / #35 / #39 / #42)
- [✓] `pearlite rollback`, `gen list`, `gen show` (PRs #33 / #34 / #37 / #38)
- [✓] VM scenarios: `vm-02` installs, `vm-03` removes, `vm-04` config write, `vm-05` rollback, `vm-06` failure record (PRs #40 / #41)
- [~] Tag `m2-exit` (signed tag from this commit)

---

## M3 — User Environment (2 weeks)

### Week 1 — Adapter and bootstrap
- [✓] `pearlite-userenv` with `HomeManagerBackend` (PR #44)
- [✓] `DeterminateNixInstaller` (hash-pinned curl|sh) + `MockInstaller` (PR #45)
- [~] `runuser` drop wrapper; per-user `nix.conf` handling — runuser wrap shipped via `LiveHmBackend`; per-user `nix.conf` provisioning deferred to M4 alongside the bootstrap-config story

### Week 2 — Engine integration
- [✓] Wire phase 7 into `pearlite-engine::apply` (PRs #46 / #47 / #50)
- [✓] Extend `state.toml` schema with `[[managed.user_env]]` (PR #50; schema already shipped in M1)
- [✓] `pearlite-diff` surfaces HM config-hash drift (PRs #48 / #49)
- [✓] VM scenarios: `vm-07` user-env apply, `vm-08` user-env drift (PR #51); `vm-09` Nix bootstrap deferred until `LiveNixInstaller` has a wired apply-time caller (M4 W1)
- [~] Tag `m3-exit` (signed tag from this commit)

---

## M4 — Reconcile and Codify (2 weeks)

### Week 1 — Reconcile flow
- [ ] `pearlite-engine::reconcile()` (read-only) → writes `hosts/<host>.imported.ncl`
- [ ] `pearlite-engine::reconcile_commit()` — interactive prompts, drift threshold safety, state.toml write
- [ ] `pearlite reconcile`, `reconcile --commit`, `--adopt-all`
- [ ] VM scenario `vm-10-reconcile-fresh-install.sh`

### Week 2 — Codify and the sidecar
- [ ] Codified sidecar emitter — deterministic, byte-identical re-emission
- [ ] `pearlite codify`, `uncodify`, `adopt`, `remove`
- [ ] Drift-threshold check for `apply --prune`
- [ ] VM scenarios: `vm-11` codify roundtrip, `vm-12` prune threshold, `vm-13` adopt-suppresses-drift
- [ ] Tag `m4-exit`

---

## M5 — Schema and MCP (2 weeks)

### Week 1 — Schema sub-command
- [ ] `#[derive(JsonSchema)]` on every clap `Args` struct
- [ ] `output_schema` for every command (Envelope.data variants)
- [ ] `render/anthropic.rs`, `openai.rs`, `gemini.rs`, `mcp.rs`
- [ ] ≥2 examples per command, ≥1 with `--format=json`
- [ ] CI: parse schema, execute every example, assert exit codes

### Week 2 — MCP server and hints
- [ ] `pearlite mcp --transport stdio` with lazy `tools/list` + `tools/get`
- [ ] Assert `tools/list` response < 2 KB
- [ ] `hints.rs`: every error.code → runnable hint; coverage assertion in CI
- [ ] Agent env-var detection: `AI_AGENT`, `AGENT`, `CI`, `CLAUDECODE`, `CURSOR_AGENT`, `GEMINI_CLI`
- [ ] VM scenario `vm-14-mcp-server-roundtrip.sh`
- [ ] Tag `m5-exit`

---

## M6 — Polish and Release (2 weeks)

### Week 1 — Documentation and shell tooling
- [ ] Man pages: `pearlite(1)`, `pearlite.toml(5)`, `pearlite-state.toml(5)`
- [ ] Shell completions: bash, zsh, fish, nu via `clap-complete`
- [ ] Finalize `AGENTS.md`, `CLAUDE.md`, `SKILL.md`, `CONTRIBUTING.md`, `README.md`
- [ ] Generate per-crate rustdocs; mdBook for high-level architecture
- [ ] Run `pearlite-audit` on full workspace; remediate findings

### Week 2 — Release pipeline
- [ ] Configure `cargo-dist` for `x86_64-unknown-linux-gnu`
- [ ] Set up sequoia-chameleon signing key in CI secrets; test signature roundtrip
- [ ] Final PKGBUILDs with `validpgpkeys`
- [ ] Cut `v1.0.0-rc.1`; install via paru on fresh CachyOS VM; full bootstrap → apply → rollback flow
- [ ] Address rc-blockers; cut `v1.0.0`; submit AUR packages
- [ ] Tag `m6-exit` (== `v1.0.0`); write release notes

---

## Definition of Done — v1.0.0 Ship Criteria (Plan §1.5)

- [ ] All 7 milestones (M0–M6) closed; green CI on each exit commit
- [ ] Fresh-install CachyOS VM bootstraps via PRD §14 flow → steady-state plan with zero actions
- [ ] All 10 PRD goals (G1–G10) trace to a closed milestone + passing test
- [ ] `pearlite-audit` Standard §13 checklist passes with zero violations
- [ ] AUR packages build cleanly; `pearlite-bin` installable + plan returns 0 after reboot
- [ ] Release artifacts signed with sequoia-chameleon; signature verifies via paru `validpgpkeys`
- [ ] rustdoc on every public item; AGENTS/CLAUDE/SKILL/CONTRIBUTING present; 3 man pages installed

---

## Open Implementation Questions (Plan §14)

Defer until ADR is written; each blocks a specific milestone exit.

- [ ] Drift-threshold default value (resolves: post-M6 retrospective)
- [ ] `codify` batching: atomic vs per-package? (resolves: M4 prototype)
- [ ] Capture `jj_change_id` in `[[history]]`? (resolves: M2)
- [ ] `--plan-file` schema_version stability rules (resolves: M2 ADR)
- [ ] Cargo source pinning (`Array CargoSpec`)? (deferred to v1.1)
- [ ] `/etc/pacman.conf`: managed vs inferred? (resolves: M1 + observation)
- [ ] MCP server authentication? (resolves: M5 ADR)

---

## v1.1+ Backlog (PRD §17.1, out of scope for v1.0)

- [ ] Fleet mode (`pearlite fleet …`) with multi-host SSH coordination
- [ ] Unattended mode (`pearlite-apply.timer`)
- [ ] `pearlite explore` (ratatui TUI)
- [ ] `pearlite verify` with PQC-hybrid signature support

---

*Last updated: 2026-04-27. Mirrors `Pearlite-Plan-v1.0.docx`.*
