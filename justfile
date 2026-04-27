# Pearlite developer recipes.
# CI calls the same recipes that humans do; anything that passes locally passes
# in CI and vice versa.

# Show the recipe list when run without arguments.
default:
    @just --list

# Format check + workspace lints.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

# Unit + integration tests via nextest.
test:
    cargo nextest run --workspace --all-targets

# Doc tests (separate runner).
test-doc:
    cargo test --workspace --doc

# Dependency audit: CVEs, licenses, duplicates.
# Ignored advisories are documented in audit.toml + deny.toml; passed
# explicitly here because cargo-audit's auto-discovery of audit.toml is
# unreliable across versions.
audit:
    cargo audit --ignore RUSTSEC-2026-0009
    cargo deny check

# Detect unused dependencies.
unused:
    cargo machete

# SPDX header check on all tracked .rs files.
spdx:
    ./scripts/ci/check-spdx.sh

# Steelbore Standard compliance audit.
compliance:
    cargo run -p pearlite-audit -- check .

# Full local CI suite (mirrors the green-on-merge gate).
ci: check test test-doc audit spdx compliance

# VM-tier integration tests (T4) — requires a self-hosted runner.
vm-test:
    ./scripts/ci/run-vm-tests.sh

# Release tarball dry run.
release-dry:
    cargo dist plan
