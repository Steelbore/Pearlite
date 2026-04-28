#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-01: bootstrap-and-plan smoke test (M1 W3, deferred to M2 W3).
#
# Builds pearlite, sets up a temp config-dir + Nickel host file, then
# runs `pearlite plan --format=json` and verifies the envelope shape:
#
# - exit 0
# - data.actions exists (array, possibly empty)
# - data.host == "vm-test"
# - metadata.command == "pearlite plan"
#
# Read-only: makes no system changes. Safe to run on a developer host.
#
# POSIX sh; no Bash-isms.

set -eu

PEARLITE_BIN="${PEARLITE_BIN:-}"
sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT INT TERM

# Build (or use cached) pearlite binary if not supplied.
if [ -z "$PEARLITE_BIN" ]; then
    printf 'vm-01: building pearlite-cli...\n' >&2
    cargo build --quiet --release -p pearlite-cli
    PEARLITE_BIN="$(cargo metadata --format-version=1 --no-deps \
        | grep -o '"target_directory":"[^"]*"' \
        | head -1 \
        | cut -d'"' -f4)/release/pearlite"
fi

[ -x "$PEARLITE_BIN" ] || {
    printf 'vm-01: PEARLITE_BIN=%s is not executable\n' "$PEARLITE_BIN" >&2
    exit 2
}

# Sandbox layout:
#   $sandbox/repo/hosts/vm-test.ncl
#   $sandbox/state.toml (auto-substituted to empty by `plan`)
mkdir -p "$sandbox/repo/hosts"
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
}
NCL

stdout="$sandbox/plan.json"
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    plan \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    > "$stdout" || {
        printf 'vm-01: pearlite plan exit %s\n' "$?" >&2
        cat "$stdout" >&2
        exit 1
    }

# Substring assertions — keeping vm-01 free of jaq so it runs anywhere.
fail=0
for needle in \
    '"command":"pearlite plan"' \
    '"host":"vm-test"' \
    '"actions":'
do
    if ! grep -q "$needle" "$stdout"; then
        printf 'vm-01: missing %s in envelope\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-01: envelope shape check failed; full output:\n' >&2
    cat "$stdout" >&2
    exit 1
fi

printf 'vm-01: bootstrap-and-plan PASS\n'
