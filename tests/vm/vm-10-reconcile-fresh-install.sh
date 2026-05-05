#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-10: reconcile-fresh-install (M4 W1).
#
# Runs `pearlite reconcile --format=json` against an empty config dir
# and verifies the read-side reconcile pipeline end-to-end:
#
#   Phase A -- happy path:
#   - exit 0
#   - metadata.command == "pearlite reconcile"
#   - data.hostname is non-empty
#   - data.imported_path is non-empty and resolves to a file under
#     <sandbox>/repo/hosts/<hostname>.imported.ncl
#   - The .imported.ncl contains the expected top-level Nickel record
#     markers (meta = {, kernel = {, packages = {, services = {).
#
#   Phase B -- clobber refusal:
#   - Re-running with the same --config-dir fails with
#     RECONCILE_ALREADY_EXISTS, class=preflight, exit 2.
#   - The original .imported.ncl is unchanged byte-for-byte.
#
# Read-only with respect to system state: the only mutation is a single
# Nickel file inside a tempdir. Safe to run on a developer host;
# whitelisted alongside vm-01 in scripts/ci/run-vm-tests.sh.
#
# POSIX sh; no Bash-isms.

set -eu

PEARLITE_BIN="${PEARLITE_BIN:-}"
sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT INT TERM

if [ -z "$PEARLITE_BIN" ]; then
    printf 'vm-10: building pearlite-cli...\n' >&2
    cargo build --quiet --release -p pearlite-cli
    PEARLITE_BIN="$(cargo metadata --format-version=1 --no-deps \
        | grep -o '"target_directory":"[^"]*"' \
        | head -1 \
        | cut -d'"' -f4)/release/pearlite"
fi

[ -x "$PEARLITE_BIN" ] || {
    printf 'vm-10: PEARLITE_BIN=%s is not executable\n' "$PEARLITE_BIN" >&2
    exit 2
}

# Empty config repo -- reconcile creates hosts/ on demand.
mkdir -p "$sandbox/repo"

# ===== Phase A: happy path =====
phase_a="$sandbox/reconcile-a.json"
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    reconcile \
    > "$phase_a" || {
        printf 'vm-10: phase A reconcile exit %s\n' "$?" >&2
        cat "$phase_a" >&2
        exit 1
    }

fail=0
for needle in \
    '"command":"pearlite reconcile"' \
    '"hostname":"' \
    '"imported_path":"'
do
    if ! grep -q "$needle" "$phase_a"; then
        printf 'vm-10: phase A missing %s in envelope\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-10: phase A envelope shape check failed; full output:\n' >&2
    cat "$phase_a" >&2
    exit 1
fi

# Extract imported_path from the JSON envelope (substring grep, no jq).
imported_path=$(grep -o '"imported_path":"[^"]*"' "$phase_a" \
    | head -1 \
    | sed 's/^"imported_path":"//;s/"$//')

if [ -z "$imported_path" ] || [ ! -f "$imported_path" ]; then
    printf 'vm-10: phase A imported_path=%s is not a file on disk\n' "$imported_path" >&2
    exit 1
fi

# imported_path must live under <sandbox>/repo/hosts/.
case "$imported_path" in
    "$sandbox/repo/hosts/"*.imported.ncl) ;;
    *)
        printf 'vm-10: phase A imported_path=%s is outside <sandbox>/repo/hosts/\n' "$imported_path" >&2
        exit 1
        ;;
esac

# Verify the imported.ncl is a top-level Nickel record with the blocks
# pearlite_nickel::emit_host produces.
fail=0
for marker in \
    'meta = {' \
    'kernel = {' \
    'packages = {' \
    'services = {'
do
    if ! grep -qF "$marker" "$imported_path"; then
        printf 'vm-10: phase A imported.ncl missing marker %s\n' "$marker" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-10: phase A imported.ncl shape check failed; full output:\n' >&2
    cat "$imported_path" >&2
    exit 1
fi

# Capture for Phase B's tamper check.
snapshot="$sandbox/snapshot.imported.ncl"
cp "$imported_path" "$snapshot"

# ===== Phase B: clobber refusal =====
phase_b="$sandbox/reconcile-b.json"
set +e
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$sandbox/state.toml" \
    reconcile \
    > "$phase_b"
phase_b_exit=$?
set -e

if [ "$phase_b_exit" -ne 2 ]; then
    printf 'vm-10: phase B expected exit 2, got %s; full output:\n' "$phase_b_exit" >&2
    cat "$phase_b" >&2
    exit 1
fi

fail=0
for needle in \
    '"code":"RECONCILE_ALREADY_EXISTS"' \
    '"class":"preflight"'
do
    if ! grep -q "$needle" "$phase_b"; then
        printf 'vm-10: phase B missing %s in envelope\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-10: phase B envelope shape check failed; full output:\n' >&2
    cat "$phase_b" >&2
    exit 1
fi

# Original imported.ncl must be byte-identical to the Phase A capture.
if ! cmp -s "$imported_path" "$snapshot"; then
    printf 'vm-10: phase B imported.ncl was modified despite ALREADY_EXISTS\n' >&2
    diff "$snapshot" "$imported_path" >&2 || true
    exit 1
fi

printf 'vm-10: reconcile-fresh-install PASS\n'
