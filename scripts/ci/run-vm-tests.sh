#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# Pearlite VM-tier (T4) test runner. Wired into `just vm-test` and
# `.github/workflows/vm.yml`.
#
# Iterates every `tests/vm/vm-*.sh` script in name order. Read-only
# scripts (`vm-01-*`, `vm-10-*`) run unconditionally. Mutating scripts
# (everything else) require `PEARLITE_VM_TEST=1` because they
# install/remove packages, write to `/etc`, and take Snapper snapshots
# -- run only inside a disposable CachyOS VM.
#
# POSIX sh; no Bash-isms.

set -eu

cd "$(git rev-parse --show-toplevel 2>/dev/null || echo .)"

if [ ! -d tests/vm ]; then
    printf 'run-vm-tests: tests/vm/ does not exist\n' >&2
    exit 2
fi

mutating_allowed=0
if [ "${PEARLITE_VM_TEST:-}" = "1" ]; then
    mutating_allowed=1
fi

if [ "$mutating_allowed" -ne 1 ]; then
    printf 'run-vm-tests: PEARLITE_VM_TEST not set; running read-only scenarios only.\n' >&2
fi

failed=0
ran=0
for script in tests/vm/vm-*.sh; do
    [ -f "$script" ] || continue
    name=$(basename "$script")
    case "$name" in
        vm-01-*|vm-10-*)
            # Read-only; always runs.
            ;;
        *)
            if [ "$mutating_allowed" -ne 1 ]; then
                printf 'run-vm-tests: skipping %s (set PEARLITE_VM_TEST=1 to enable)\n' "$name"
                continue
            fi
            ;;
    esac
    printf '\nrun-vm-tests: ----- %s -----\n' "$name"
    ran=$((ran + 1))
    if ! sh "$script"; then
        printf 'run-vm-tests: %s FAILED\n' "$name" >&2
        failed=$((failed + 1))
    fi
done

if [ "$failed" -ne 0 ]; then
    printf '\nrun-vm-tests: %s/%s scripts failed\n' "$failed" "$ran" >&2
    exit 1
fi

printf '\nrun-vm-tests: %s scripts passed\n' "$ran"
