#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-07: per-user `home-manager switch` via `pearlite apply` (PRD §8.2
# phase 7).
#
# Declares one HM-enabled user pointing at a tiny inline home.nix
# fixture, applies, and verifies:
#
#   - apply exit 0
#   - data.actions_executed >= 1
#   - state.toml's [[managed.user_env]] grew an entry for the user
#     with a non-empty config_hash
#
# Prereqs (in addition to the vm-01 prereqs):
#   - home-manager installed and on PATH (paru -S home-manager-flake)
#   - the test user owns $HOME and can write a profile symlink
#
# Mutating: requires PEARLITE_VM_TEST=1. Runs `home-manager switch`
# under the configured test user — disposable VM only.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-07: refusing to run without PEARLITE_VM_TEST=1\n' >&2
    exit 2
fi

PEARLITE_BIN="${PEARLITE_BIN:-}"
TEST_USER="${PEARLITE_VM_USER:-pearlite-vm}"
sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT INT TERM

if [ -z "$PEARLITE_BIN" ]; then
    cargo build --quiet --release -p pearlite-cli
    PEARLITE_BIN="$(cargo metadata --format-version=1 --no-deps \
        | grep -o '"target_directory":"[^"]*"' \
        | head -1 \
        | cut -d'"' -f4)/release/pearlite"
fi

# Verify the test user exists on this host. We don't create one; that's
# the VM image's responsibility (cleaner than scripting useradd here).
if ! id -u "$TEST_USER" >/dev/null 2>&1; then
    printf 'vm-07: PEARLITE_VM_USER=%s does not exist on this host\n' "$TEST_USER" >&2
    exit 2
fi

mkdir -p "$sandbox/repo/hosts" "$sandbox/repo/users/$TEST_USER"
cat > "$sandbox/repo/users/$TEST_USER/home.nix" <<NIX
{ config, pkgs, ... }: {
  home.username = "$TEST_USER";
  home.homeDirectory = "/home/$TEST_USER";
  home.stateVersion = "24.11";
  home.packages = with pkgs; [ ];
  programs.home-manager.enable = true;
}
NIX

cat > "$sandbox/repo/hosts/vm-test.ncl" <<NCL
{
  meta = { hostname = "vm-test", timezone = "UTC", arch_level = "v3", locale = "en_US.UTF-8", keymap = "us" },
  kernel = { package = "linux-cachyos" },
  users = [
    {
      name = "$TEST_USER",
      shell = "/bin/sh",
      groups = [],
      home_manager = {
        enabled = true,
        mode = "standalone",
        config_path = "users/$TEST_USER",
        channel = "release-24.11",
      },
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

stdout="$sandbox/apply.json"
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$stdout" || {
        printf 'vm-07: apply exit %s\n' "$?" >&2
        cat "$stdout" >&2
        exit 1
    }

# Substring assertions — keep vm-07 jaq-free for portability.
fail=0
if ! grep -q '"actions_executed":1' "$stdout"; then
    if ! grep -q '"actions_executed":2' "$stdout"; then
        printf 'vm-07: expected at least one action executed\n' >&2
        fail=1
    fi
fi

# state.toml grew a [[managed.user_env]] entry for $TEST_USER with a
# non-empty config_hash. Use grep over the toml; avoids a jaq dep.
if ! grep -q "\\[\\[managed.user_env\\]\\]" "$sandbox/state.toml"; then
    printf 'vm-07: state.toml has no [[managed.user_env]] section\n' >&2
    fail=1
elif ! grep -q "user = \"$TEST_USER\"" "$sandbox/state.toml"; then
    printf 'vm-07: no managed.user_env entry for %s\n' "$TEST_USER" >&2
    fail=1
elif ! grep -E -q 'config_hash = "[a-f0-9]{64}"' "$sandbox/state.toml"; then
    printf 'vm-07: managed.user_env.config_hash is empty or malformed\n' >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    printf 'vm-07: assertions failed; full apply output:\n' >&2
    cat "$stdout" >&2
    printf '\nstate.toml:\n' >&2
    cat "$sandbox/state.toml" >&2
    exit 1
fi

printf 'vm-07: user-env apply PASS\n'
