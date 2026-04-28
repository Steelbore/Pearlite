#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-08: user-env config-hash drift detection.
#
# Three steps:
#
#   1. Apply with home.nix v1. state.toml records the v1 config_hash.
#   2. Re-plan without changing anything → no UserEnvSwitch action
#      (idempotent).
#   3. Mutate home.nix to v2. Re-plan → expect ONE UserEnvSwitch in
#      the actions list (drift detected via sha256_dir mismatch).
#      Apply v2 → state.toml's config_hash changes to the new value
#      and managed.user_env still has exactly one row for the user
#      (upsert, not append).
#
# Mutating: requires PEARLITE_VM_TEST=1.
#
# POSIX sh; no Bash-isms.

set -eu

if [ "${PEARLITE_VM_TEST:-}" != "1" ]; then
    printf 'vm-08: refusing to run without PEARLITE_VM_TEST=1\n' >&2
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

if ! id -u "$TEST_USER" >/dev/null 2>&1; then
    printf 'vm-08: PEARLITE_VM_USER=%s does not exist on this host\n' "$TEST_USER" >&2
    exit 2
fi

mkdir -p "$sandbox/repo/hosts" "$sandbox/repo/users/$TEST_USER"

write_home_nix() {
    # $1 is a sentinel comment so the file's bytes (and therefore the
    # sha256_dir hash) change between v1 and v2.
    cat > "$sandbox/repo/users/$TEST_USER/home.nix" <<NIX
# version: $1
{ config, pkgs, ... }: {
  home.username = "$TEST_USER";
  home.homeDirectory = "/home/$TEST_USER";
  home.stateVersion = "24.11";
  home.packages = with pkgs; [ ];
  programs.home-manager.enable = true;
}
NIX
}

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

write_home_nix v1

# Step 1: first apply.
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$sandbox/apply-v1.json" || {
        printf 'vm-08: step 1 apply failed:\n' >&2
        cat "$sandbox/apply-v1.json" >&2
        exit 1
    }

hash_v1=$(grep -E -o 'config_hash = "[a-f0-9]{64}"' "$sandbox/state.toml" | head -1)

# Step 2: idempotent re-plan with v1 unchanged should produce no actions.
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    plan \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    > "$sandbox/plan-idempotent.json" || {
        printf 'vm-08: idempotent plan failed:\n' >&2
        cat "$sandbox/plan-idempotent.json" >&2
        exit 1
    }
if grep -q '"user_env_switch"' "$sandbox/plan-idempotent.json"; then
    printf 'vm-08: idempotent re-plan emitted a user_env_switch (drift false-positive)\n' >&2
    cat "$sandbox/plan-idempotent.json" >&2
    exit 1
fi

# Step 3: mutate home.nix → re-plan should see drift.
write_home_nix v2

"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    plan \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    > "$sandbox/plan-drifted.json" || {
        printf 'vm-08: drift plan failed:\n' >&2
        cat "$sandbox/plan-drifted.json" >&2
        exit 1
    }
if ! grep -q '"user_env_switch"' "$sandbox/plan-drifted.json"; then
    printf 'vm-08: drift plan did NOT emit a user_env_switch action:\n' >&2
    cat "$sandbox/plan-drifted.json" >&2
    exit 1
fi

# Apply v2.
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    apply \
    --host-file="$sandbox/repo/hosts/vm-test.ncl" \
    --snapper-config=root \
    > "$sandbox/apply-v2.json" || {
        printf 'vm-08: step 3 apply failed:\n' >&2
        cat "$sandbox/apply-v2.json" >&2
        exit 1
    }

hash_v2=$(grep -E -o 'config_hash = "[a-f0-9]{64}"' "$sandbox/state.toml" | head -1)
if [ "$hash_v1" = "$hash_v2" ]; then
    printf 'vm-08: state.toml config_hash unchanged after drift apply\n' >&2
    printf '  v1: %s\n  v2: %s\n' "$hash_v1" "$hash_v2" >&2
    exit 1
fi

# managed.user_env must still contain exactly one row for $TEST_USER
# (upsert, not append).
count=$(grep -c "user = \"$TEST_USER\"" "$sandbox/state.toml" || true)
if [ "$count" -ne 1 ]; then
    printf 'vm-08: managed.user_env has %s rows for %s, expected 1 (upsert violated)\n' \
        "$count" "$TEST_USER" >&2
    cat "$sandbox/state.toml" >&2
    exit 1
fi

printf 'vm-08: user-env drift detection PASS\n'
