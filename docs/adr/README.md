# Architecture Decision Records

Short-form (Michael Nygard) records of architectural decisions that
diverge from the obvious default. Each ADR has three sections:
**Context**, **Decision**, **Consequences**.

ADR-001 through ADR-007 are documented in long form in
`Pearlite-Plan-v1.0.docx` §13. They cover:

| ID | Title | Section |
|---|---|---|
| ADR-001 | Nickel for human config, TOML for machine state | Plan §13.1 |
| ADR-002 | No automatic rollback | Plan §13.2 |
| ADR-003 | `state.toml` is the last file written | Plan §13.3 |
| ADR-004 | Determinate Nix installer over the official one | Plan §13.4 |
| ADR-005 | Warn-not-fail on non-CachyOS Arch | Plan §13.5 |
| ADR-006 | Ship at x86-64-v3, not v4 | Plan §13.6 |
| ADR-007 | No async runtime | Plan §13.7 |

Backfill of these as standalone files in this directory is tracked
under Plan §9.6 but not gating any milestone exit.

## Active ADRs in this directory

| ID | Title | Filed |
|---|---|---|
| [ADR-0008](./0008-msrv-bump-policy.md) | MSRV bump policy | M2 W1 |
| [ADR-0009](./0009-cachyos-ci-runner.md) | CachyOS-fidelity CI runner choice | M2 W1 |
