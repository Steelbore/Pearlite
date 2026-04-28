# Pearlite VM Integration Tests

End-to-end scenarios that run the `pearlite` binary against a real
CachyOS environment. Plan §8.5 calls these "T4" tier — they exercise
paru, snapper, systemd, and friends as full processes; the live-tier
`tests/` Rust integration tests (T3) stub those out.

The CI workflow `.github/workflows/vm.yml` runs these scripts on a
self-hosted CachyOS runner nightly and on PRs labelled `vm-test`. The
runner is gated by ADR-0009 (CachyOS-fidelity CI runner choice).

## Layout

| Script | Mode | Purpose |
|---|---|---|
| `vm-01-bootstrap-and-plan.sh` | read-only | Build the binary, run `pearlite plan` against a fixture, assert envelope shape. Safe to run on a developer host with `nickel` + `paru` installed. |
| `vm-02-installs.sh` | mutating | Pacman install via `pearlite apply` — declares `tree`, asserts `actions_executed == 1` and `pacman -Qe tree`. |
| `vm-03-removes.sh` | mutating | Removal flow — apply with `tree`, then re-plan with `tree` dropped, asserts a `forgotten_package` drift entry (action emission gates on `--prune`, follow-up PR). |
| `vm-04-config-write.sh` | mutating | Config-file write — declares `/etc/pearlite-vm-test-04.conf`, applies, asserts target SHA-256 matches source. |
| `vm-05-rollback.sh` | mutating | Apply → rollback round-trip — asserts `pacman -Qe tree` is gone after the snapper revert restores the pre-apply subvolume. |
| `vm-06-failure-record.sh` | mutating | Induced `APPLY_SHA_MISMATCH` failure — asserts exit 4, forensic JSON at `<failures_dir>/<plan-id>.json`, and that `gen show` surfaces the SHA-256 message. |
| `vm-07-user-env-apply.sh` | mutating | Per-user `home-manager switch` via `pearlite apply` (PRD §8.2 phase 7). Asserts `state.toml`'s `[[managed.user_env]]` records the user's `config_hash` after the switch. Requires `home-manager` on `PATH` and the `$PEARLITE_VM_USER` (default `pearlite-vm`) login present on the VM. |
| `vm-08-user-env-drift.sh` | mutating | Drift detection — apply v1, idempotent re-plan emits no action, mutate `home.nix` to v2, re-plan emits `user_env_switch`, apply v2 updates `state.toml` `config_hash` and keeps `managed.user_env` at one row per user (upsert). |
| `vm-09-nix-bootstrap.sh` | mutating | `pearlite bootstrap` (ADR-0012) idempotency — short-circuits when `nix --version` already succeeds, writes `/etc/nix/nix.conf` once, second run reports `nix_conf_written: false`. The actual Determinate-installer execution path is not exercised here; ADR-004 SHA verification is unit-tested in `crates/pearlite-userenv`. |

Mutating scripts refuse to run unless `PEARLITE_VM_TEST=1` is set in
the environment — they install/remove packages, write to `/etc`, and
take Snapper snapshots. Run only inside a disposable VM.

## Running

```sh
# Read-only smoke test (safe anywhere):
./tests/vm/vm-01-bootstrap-and-plan.sh

# All mutating scenarios (CachyOS VM only):
PEARLITE_VM_TEST=1 ./scripts/ci/run-vm-tests.sh
```

The runner picks up every `tests/vm/vm-*.sh` script in name order. A
non-zero exit aborts the rest.

## Conventions

- POSIX sh; no Bash-isms (CLAUDE.md mandate).
- Each script self-contains its sandbox under `$(mktemp -d)`.
- Scripts that need the binary honour `$PEARLITE_BIN`; default is
  `cargo run --quiet -p pearlite-cli --`.
- Assertions use `grep` for substrings; `jaq` (per the Steelbore CLI
  preference) where richer JSON shape inspection is needed.
- Exit code conventions match `pearlite` itself: 0 success, 2
  preflight, 4 recoverable apply, 5 incoherent apply.
