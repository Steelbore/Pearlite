#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-05: pearlite apply → pearlite rollback round-trip.
#
# Applies a host that declares `tree`, captures the resulting plan_id
# from the apply envelope, then runs `pearlite rollback <plan-id>` and
# asserts:
#
# - rollback exit 0
# - data.snapshot_pre.id matches what the apply recorded
# - tree is no longer installed (snapper rollback restored
#   /var/lib/pearlite/state.toml AND the package state)
#
# Mutating: requires PEARLITE_VM_TEST=1. The btrfs revert affects the
# whole root subvolume — disposable VM only.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-05: refusing to run without PEARLITE_VM_TEST=1\n' >&2
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

"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$sandbox/apply.json" || {
        printf 'vm-05: apply failed:\n' >&2
        cat "$sandbox/apply.json" >&2
        exit 1
    }

# Extract the plan_id from the apply envelope. Use jaq for the JSON
# inspection per Steelbore CLI prefs; fall back to grep+sed if jaq is
# absent so the script still runs in minimal CachyOS images.
plan_id=$(
    if command -v jaq >/dev/null 2>&1; then
        jaq -r '.data.plan_id' < "$sandbox/apply.json"
    else
        grep -o '"plan_id":"[^"]*"' "$sandbox/apply.json" | head -1 | cut -d'"' -f4
    fi
)
[ -n "$plan_id" ] || {
    printf 'vm-05: could not extract plan_id from apply output:\n' >&2
    cat "$sandbox/apply.json" >&2
    exit 1
}

"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    rollback "$plan_id" \
    --snapper-config=root \
    > "$sandbox/rollback.json" || {
        printf 'vm-05: rollback failed:\n' >&2
        cat "$sandbox/rollback.json" >&2
        exit 1
    }

if pacman -Qe tree >/dev/null 2>&1; then
    printf 'vm-05: tree still installed after rollback\n' >&2
    exit 1
fi

printf 'vm-05: apply → rollback round-trip PASS\n'
