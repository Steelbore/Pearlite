#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-03: pacman remove via `pearlite apply`.
#
# Two-step scenario:
#   1. Apply a host file declaring `tree` → tree installed, state.managed
#      grows.
#   2. Apply a host file dropping `tree` → tree removed (PRD §7.3
#      "Forgotten" classification: in state.managed ∧ not declared →
#      propose remove via --prune; without --prune we expect drift only,
#      so this script asserts state-managed → drift surfaced).
#
# The actual removal action emission is gated on `--prune` per
# ADR-0011. vm-03 (this script) verifies the plan-side drift
# classification only — that removing `tree` from the host file with
# `tree` present in `state.managed.pacman` produces a drift entry.
# Asserting the `--prune` removal path end-to-end belongs in a
# dedicated vm-07-prune scenario when the post-M6 retrospective
# validates the threshold default; until then the unit tests in
# pearlite-cli::dispatch cover the threshold guard.
#
# Mutating: requires PEARLITE_VM_TEST=1.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-03: refusing to run without PEARLITE_VM_TEST=1\n' >&2
    exit 2
fi

PEARLITE_BIN="${PEARLITE_BIN:-}"
sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT INT TERM

if [ -z "$PEARLITE_BIN" ]; then
    cargo build --quiet --release -p pearlite-cli
    PEARLITE_BIN="$(cargo metadata --format-version=1 --no-deps \
        | grep -o '"target_directory":"[^"]*"' \
        | head -1 \
        | cut -d'"' -f4)/release/pearlite"
fi

mkdir -p "$sandbox/repo/hosts"

# Host that declares tree.
cat > "$sandbox/repo/hosts/vm-test.ncl" <<'NCL'
{
  meta = { hostname = "vm-test", timezone = "UTC", arch_level = "v3", locale = "en_US.UTF-8", keymap = "us" },
  kernel = { package = "linux-cachyos" },
  packages = { core = ["tree"] },
}
NCL

cat > "$sandbox/state.toml" <<'TOML'
schema_version = 1
host = "vm-test"
tool_version = "0.1.0"
config_dir = "/tmp/repo"

[managed]
pacman = []
cargo = []

[adopted]
pacman = []
cargo = []
TOML

# Step 1: install via apply.
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$sandbox/step1.json" || {
        printf 'vm-03: step 1 (install) failed:\n' >&2
        cat "$sandbox/step1.json" >&2
        exit 1
    }

# Step 2: drop tree from declared, plan only — expect a forgotten-package
# drift entry.
cat > "$sandbox/repo/hosts/vm-test.ncl" <<'NCL'
{
  meta = { hostname = "vm-test", timezone = "UTC", arch_level = "v3", locale = "en_US.UTF-8", keymap = "us" },
  kernel = { package = "linux-cachyos" },
  packages = { core = [] },
}
NCL

"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    plan \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    > "$sandbox/step2.json" || {
        printf 'vm-03: step 2 (plan after drop) failed:\n' >&2
        cat "$sandbox/step2.json" >&2
        exit 1
    }

if ! grep -q '"forgotten_package"' "$sandbox/step2.json"; then
    printf 'vm-03: expected a forgotten_package drift entry, got:\n' >&2
    cat "$sandbox/step2.json" >&2
    exit 1
fi

printf 'vm-03: pacman remove (plan-side drift) PASS\n'
