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
| `vm-01-bootstrap-and-plan.sh` | read-only | Build the binary, run `pearlite plan` against a fixture, assert envelope shape. Safe to run on a developer host. |
| `vm-02-installs.sh` | mutating | M2 W3 — paru install via `pearlite apply`. |
| `vm-03-removes.sh` | mutating | M2 W3 — pacman remove via `pearlite apply`. |
| `vm-04-config-write.sh` | mutating | M2 W3 — `/etc` write via `pearlite apply`. |
| `vm-05-rollback.sh` | mutating | M2 W3 — `pearlite rollback <plan-id>`. |
| `vm-06-failure-record.sh` | mutating | M2 W3 — induced apply failure → forensic record. |

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
