#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-04: /etc config-file write via `pearlite apply`.
#
# Declares a single config entry pointing at a unique sentinel file
# under /etc/pearlite-vm-test-04.conf so the script never collides
# with a real system file. Source SHA-256 is computed at script start
# and inlined into the host file, then `pearlite apply` runs and we
# verify the target was written byte-identical.
#
# Mutating: requires PEARLITE_VM_TEST=1. Also writes to /etc and so
# requires root in a real CachyOS VM.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-04: refusing to run without PEARLITE_VM_TEST=1\n' >&2
    exit 2
fi

PEARLITE_BIN="${PEARLITE_BIN:-}"
sandbox=$(mktemp -d)
target=/etc/pearlite-vm-test-04.conf
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
content='vm-test config-write fixture
'
printf '%s' "$content" > "$sandbox/repo/etc/vm-test.conf"
sha=$(sha256sum "$sandbox/repo/etc/vm-test.conf" | cut -d' ' -f1)

cat > "$sandbox/repo/hosts/vm-test.ncl" <<NCL
{
  meta = { hostname = "vm-test", timezone = "UTC", arch_level = "v3", locale = "en_US.UTF-8", keymap = "us" },
  kernel = { package = "linux-cachyos" },
  config = [
    {
      target = "$target",
      source = "etc/vm-test.conf",
      content_sha256 = "$sha",
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

"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$sandbox/apply.json" || {
        printf 'vm-04: apply failed:\n' >&2
        cat "$sandbox/apply.json" >&2
        exit 1
    }

[ -f "$target" ] || {
    printf 'vm-04: target %s not written\n' "$target" >&2
    exit 1
}

actual_sha=$(sha256sum "$target" | cut -d' ' -f1)
if [ "$actual_sha" != "$sha" ]; then
    printf 'vm-04: target sha mismatch: expected %s, got %s\n' "$sha" "$actual_sha" >&2
    exit 1
fi

printf 'vm-04: config write PASS\n'
