#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-09: pearlite bootstrap idempotency + nix.conf write (ADR-0012).
#
# Two steps:
#
#   1. With nix already on PATH, run `pearlite bootstrap` against a
#      sandboxed host file declaring nix.installer.expected_sha256.
#      Expect install: "already" (short-circuit via `nix --version`)
#      and nix_conf_written: true (the test sandbox starts without
#      a nix.conf).
#   2. Re-run the same command. Expect install: "already" still, and
#      nix_conf_written: false — the experimental-features line is
#      already in the file from step 1, idempotent skip.
#
# This scenario does NOT exercise the actual Determinate installer
# fetch + exec path: doing that on the runner would mutate /nix and
# is not safe outside a disposable VM image. The SHA-verification
# path (ADR-004) is covered by unit tests in
# crates/pearlite-userenv/src/installer.rs.
#
# Mutating: requires PEARLITE_VM_TEST=1.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-09: refusing to run without PEARLITE_VM_TEST=1\n' >&2
    exit 2
fi

if ! command -v nix >/dev/null 2>&1; then
    printf 'vm-09: this scenario expects nix to be installed on the VM image.\n' >&2
    printf '       (vm-09 exercises the short-circuit path; the actual install\n' >&2
    printf '       path would mutate /nix and is unit-tested elsewhere.)\n' >&2
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

# A script the bootstrap command will hash. Content doesn't matter
# for the short-circuit path (install_if_missing returns Already
# before reading the script), but we declare the matching SHA so a
# future test that exercises the install path could reuse this fixture.
script="$sandbox/installer.sh"
cat > "$script" <<'SCRIPT'
#!/bin/sh
# vm-09 stub Determinate installer — must NEVER execute in this scenario.
exit 99
SCRIPT
chmod +x "$script"

script_sha=$(sha256sum "$script" | awk '{print $1}')

cat > "$sandbox/repo/hosts/vm-test.ncl" <<NCL
{
  meta = { hostname = "vm-test", timezone = "UTC", arch_level = "v3", locale = "en_US.UTF-8", keymap = "us" },
  kernel = { package = "linux-cachyos" },
  nix = { installer = { expected_sha256 = "$script_sha" } },
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

nix_conf="$sandbox/nix.conf"

# Step 1: bootstrap on a clean sandbox (no nix.conf yet).
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    bootstrap \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --installer-script="$script" \
    --nix-conf="$nix_conf" \
    > "$sandbox/bootstrap-1.json" || {
        printf 'vm-09: step 1 bootstrap failed:\n' >&2
        cat "$sandbox/bootstrap-1.json" >&2
        exit 1
    }

if ! grep -q '"install":"already"' "$sandbox/bootstrap-1.json"; then
    printf 'vm-09: step 1 install did not short-circuit:\n' >&2
    cat "$sandbox/bootstrap-1.json" >&2
    exit 1
fi
if ! grep -q '"nix_conf_written":true' "$sandbox/bootstrap-1.json"; then
    printf 'vm-09: step 1 expected nix_conf_written:true:\n' >&2
    cat "$sandbox/bootstrap-1.json" >&2
    exit 1
fi
if ! grep -q 'experimental-features = nix-command flakes' "$nix_conf"; then
    printf 'vm-09: nix.conf does not contain the expected line:\n' >&2
    cat "$nix_conf" >&2
    exit 1
fi

# Step 2: idempotent re-run.
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    bootstrap \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --installer-script="$script" \
    --nix-conf="$nix_conf" \
    > "$sandbox/bootstrap-2.json" || {
        printf 'vm-09: step 2 bootstrap failed:\n' >&2
        cat "$sandbox/bootstrap-2.json" >&2
        exit 1
    }

if ! grep -q '"install":"already"' "$sandbox/bootstrap-2.json"; then
    printf 'vm-09: step 2 install did not short-circuit:\n' >&2
    cat "$sandbox/bootstrap-2.json" >&2
    exit 1
fi
if ! grep -q '"nix_conf_written":false' "$sandbox/bootstrap-2.json"; then
    printf 'vm-09: step 2 expected nix_conf_written:false (idempotent):\n' >&2
    cat "$sandbox/bootstrap-2.json" >&2
    exit 1
fi

printf 'vm-09: nix bootstrap idempotency PASS\n'
