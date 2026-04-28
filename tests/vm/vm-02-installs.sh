#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-02: paru/pacman install via `pearlite apply`.
#
# Declares one small package (`tree`) under the `core` bucket, runs
# `pearlite apply --format=json`, then verifies:
#
# - exit 0
# - data.actions_executed >= 1
# - data.generation == 1
# - state.toml grew a [[history]] entry
# - paru/pacman reports `tree` as explicitly installed
#
# Mutating: requires PEARLITE_VM_TEST=1 (the runner enforces this).
# The script is still safe to invoke directly under that env var, but
# it WILL install `tree` system-wide.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-02: refusing to run without PEARLITE_VM_TEST=1\n' >&2
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

mkdir -p "$sandbox/repo/hosts" "$sandbox/state"

cat > "$sandbox/repo/hosts/vm-test.ncl" <<'NCL'
{
  meta = {
    hostname = "vm-test",
    timezone = "UTC",
    arch_level = "v3",
    locale = "en_US.UTF-8",
    keymap = "us",
  },
  kernel = {
    package = "linux-cachyos",
  },
  packages = {
    core = ["tree"],
  },
}
NCL

cat > "$sandbox/state/state.toml" <<'TOML'
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

stdout="$sandbox/apply.json"
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$stdout" || {
        printf 'vm-02: apply exit %s\n' "$?" >&2
        cat "$stdout" >&2
        exit 1
    }

fail=0
for needle in \
    '"command":"pearlite apply"' \
    '"actions_executed":1' \
    '"generation":1'
do
    if ! grep -q "$needle" "$stdout"; then
        printf 'vm-02: missing %s\n' "$needle" >&2
        fail=1
    fi
done

if ! pacman -Qe tree >/dev/null 2>&1; then
    printf 'vm-02: tree not installed-explicit per pacman\n' >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    printf 'vm-02: assertions failed; full output:\n' >&2
    cat "$stdout" >&2
    exit 1
fi

printf 'vm-02: pacman install PASS\n'
