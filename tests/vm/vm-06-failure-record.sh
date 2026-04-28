#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-06: induced apply failure → forensic FailureRecord lands on disk.
#
# Declares a config-write whose `content_sha256` deliberately does not
# match the source file. The engine returns APPLY_SHA_MISMATCH (Class
# 3 recoverable, exit 4) and writes the forensic JSON record under
# <state_dir>/failures/<plan-id>.json. The script then runs
# `pearlite gen show <plan-id>` and asserts that:
#
#   - apply exit 4
#   - error.code == APPLY_SHA_MISMATCH
#   - <failures_dir>/<plan-id>.json exists
#   - gen show data.failure.record.error_message references SHA-256
#
# Mutating: requires PEARLITE_VM_TEST=1. Snapper takes a pre-snapshot;
# best-effort post-fail snapshot is also taken.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-06: refusing to run without PEARLITE_VM_TEST=1\n' >&2
    exit 2
fi

PEARLITE_BIN="${PEARLITE_BIN:-}"
sandbox=$(mktemp -d)
target=/etc/pearlite-vm-test-06.conf
cleanup() {
    rm -rf "$sandbox"
    rm -f "$target"
}
trap cleanup EXIT INT TERM

if [ -z "$PEARLITE_BIN" ]; then
    cargo build --quiet --release -p pearlite-cli
    PEARLITE_BIN="$(cargo metadata --format-version=1 --no-deps \
        | grep -o '"target_directory":"[^"]*"' \
        | head -1 \
        | cut -d'"' -f4)/release/pearlite"
fi

mkdir -p "$sandbox/repo/hosts" "$sandbox/repo/etc"
printf 'real content\n' > "$sandbox/repo/etc/vm-test.conf"

# Deliberately wrong SHA-256 — this is the failure trigger.
fake_sha="0000000000000000000000000000000000000000000000000000000000000000"

cat > "$sandbox/repo/hosts/vm-test.ncl" <<NCL
{
  meta = { hostname = "vm-test", timezone = "UTC", arch_level = "v3", locale = "en_US.UTF-8", keymap = "us" },
  kernel = { package = "linux-cachyos" },
  config = [
    {
      target = "$target",
      source = "etc/vm-test.conf",
      content_sha256 = "$fake_sha",
      mode = 420,
      owner = "root",
      group = "root",
    },
  ],
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

# Apply MUST exit non-zero (4 = recoverable apply per PRD §8.5).
set +e
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --failures-dir="$sandbox/failures" \
    --snapper-config=root \
    > "$sandbox/apply.json"
apply_exit=$?
set -e

if [ "$apply_exit" -ne 4 ]; then
    printf 'vm-06: expected apply exit 4 (recoverable), got %s\n' "$apply_exit" >&2
    cat "$sandbox/apply.json" >&2
    exit 1
fi

if ! grep -q '"code":"APPLY_SHA_MISMATCH"' "$sandbox/apply.json"; then
    printf 'vm-06: expected APPLY_SHA_MISMATCH error code:\n' >&2
    cat "$sandbox/apply.json" >&2
    exit 1
fi

# Pull plan_id out of error.details.plan_id.
plan_id=$(
    if command -v jaq >/dev/null 2>&1; then
        jaq -r '.error.details.plan_id' < "$sandbox/apply.json"
    else
        grep -o '"plan_id":"[^"]*"' "$sandbox/apply.json" | head -1 | cut -d'"' -f4
    fi
)
[ -n "$plan_id" ] || {
    printf 'vm-06: could not extract plan_id\n' >&2
    cat "$sandbox/apply.json" >&2
    exit 1
}

# uuid::Uuid::simple format (no hyphens).
plan_id_simple=$(printf '%s' "$plan_id" | tr -d -)
record_path="$sandbox/failures/${plan_id_simple}.json"
[ -f "$record_path" ] || {
    printf 'vm-06: forensic record %s missing\n' "$record_path" >&2
    ls -la "$sandbox/failures" >&2 || true
    exit 1
}

"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    gen show "$plan_id" \
    > "$sandbox/gen-show.json" || {
        printf 'vm-06: gen show failed:\n' >&2
        cat "$sandbox/gen-show.json" >&2
        exit 1
    }

if ! grep -q 'sha256\|SHA-256\|SHA256' "$sandbox/gen-show.json"; then
    printf 'vm-06: gen show output does not mention SHA-256:\n' >&2
    cat "$sandbox/gen-show.json" >&2
    exit 1
fi

printf 'vm-06: failure record forensics PASS\n'
