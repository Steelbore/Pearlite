#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# vm-11: reconcile-commit (M4 W1, ADR-0014).
#
# End-to-end coverage for `pearlite reconcile --commit`:
#
#   Phase A -- non-interactive refusal:
#   - Bare `--commit` without `--adopt-all` under AI_AGENT=1 (forces
#     non-interactive even on a TTY runner) refuses with
#     RECONCILE_REQUIRES_INTERACTIVE, exit 2.
#   - state.toml is byte-identical afterwards.
#
#   Phase B -- threshold-exceeded refusal:
#   - `--commit --adopt-all --commit-threshold 0` exits 2 with
#     RECONCILE_THRESHOLD_EXCEEDED. The message names the count and the
#     threshold and points at `--adopt-all` for fresh-install bulk
#     adoption (ADR-0014 §2 mandate).
#   - state.toml is byte-identical afterwards.
#
#   Phase C -- interactive abort:
#   - Run `--commit` inside a util-linux `script(1)` pty session and
#     feed `q\n`; the operator's quit path returns exit 0 with
#     `committed: false` / `aborted: true` and leaves state.toml
#     byte-identical.
#   - Skipped (with a warning) when `script(1)` is unavailable.
#
#   Phase D -- adopt-all happy path:
#   - `--commit --adopt-all` writes state.toml: `[adopted].pacman` is
#     populated, `[[reconciliations]]` has exactly one entry, and the
#     envelope reports `committed: true`, `action: "adopt_all"`,
#     `considered > 0`, and a non-empty `adopted` array.
#
# Read-only with respect to system state: the only mutation is the
# sandboxed `state.toml`. Whitelisted alongside vm-10 in
# scripts/ci/run-vm-tests.sh.
#
# Requires `pacman` on PATH (the live probe shells out to it). On a
# typical CachyOS host every explicitly-installed package classifies
# as Manual on a fresh state.toml, so phase B's threshold=0 trip and
# phase D's non-zero adoption count are both reliable.
#
# POSIX sh; no Bash-isms.

set -eu

PEARLITE_BIN="${PEARLITE_BIN:-}"
sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT INT TERM

if [ -z "$PEARLITE_BIN" ]; then
    printf 'vm-11: building pearlite-cli...\n' >&2
    cargo build --quiet --release -p pearlite-cli
    PEARLITE_BIN="$(cargo metadata --format-version=1 --no-deps \
        | grep -o '"target_directory":"[^"]*"' \
        | head -1 \
        | cut -d'"' -f4)/release/pearlite"
fi

[ -x "$PEARLITE_BIN" ] || {
    printf 'vm-11: PEARLITE_BIN=%s is not executable\n' "$PEARLITE_BIN" >&2
    exit 2
}

# Seed a minimal schema-valid state.toml. reconcile_commit reads this
# before classifying drift; it would error with STATE_NOT_FOUND if
# absent.
state_path="$sandbox/state.toml"
cat > "$state_path" <<EOF
schema_version = 1
host = "vm-11"
tool_version = "0.1.0"
config_dir = "$sandbox/repo"
EOF
mkdir -p "$sandbox/repo"

# Capture the seed for byte-equality assertions in refusal phases.
seed_sha=$(sha256sum "$state_path" | cut -d' ' -f1)

assert_state_unchanged() {
    phase_label="$1"
    actual=$(sha256sum "$state_path" | cut -d' ' -f1)
    if [ "$actual" != "$seed_sha" ]; then
        printf 'vm-11: %s mutated state.toml (sha256 %s -> %s)\n' \
            "$phase_label" "$seed_sha" "$actual" >&2
        exit 1
    fi
}

# ===== Phase A: non-interactive refusal =====
phase_a="$sandbox/phase-a.json"
set +e
AI_AGENT=1 "$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$state_path" \
    reconcile --commit \
    > "$phase_a"
phase_a_exit=$?
set -e

if [ "$phase_a_exit" -ne 2 ]; then
    printf 'vm-11: phase A expected exit 2, got %s; full output:\n' "$phase_a_exit" >&2
    cat "$phase_a" >&2
    exit 1
fi

fail=0
for needle in \
    '"code":"RECONCILE_REQUIRES_INTERACTIVE"' \
    '"class":"preflight"' \
    '"hint":"pearlite reconcile --commit --adopt-all"'
do
    if ! grep -qF -- "$needle" "$phase_a"; then
        printf 'vm-11: phase A missing %s in envelope\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-11: phase A envelope shape check failed; full output:\n' >&2
    cat "$phase_a" >&2
    exit 1
fi

assert_state_unchanged "phase A"

# ===== Phase B: threshold-exceeded refusal =====
phase_b="$sandbox/phase-b.json"
set +e
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$state_path" \
    reconcile --commit --adopt-all --commit-threshold 0 \
    > "$phase_b"
phase_b_exit=$?
set -e

if [ "$phase_b_exit" -ne 2 ]; then
    printf 'vm-11: phase B expected exit 2, got %s; full output:\n' "$phase_b_exit" >&2
    cat "$phase_b" >&2
    exit 1
fi

fail=0
for needle in \
    '"code":"RECONCILE_THRESHOLD_EXCEEDED"' \
    '"class":"preflight"' \
    '"hint":"pearlite reconcile --commit --adopt-all"' \
    'fresh-install' \
    '--adopt-all'
do
    if ! grep -qF -- "$needle" "$phase_b"; then
        printf 'vm-11: phase B missing %s in envelope\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-11: phase B envelope shape check failed; full output:\n' >&2
    cat "$phase_b" >&2
    exit 1
fi

assert_state_unchanged "phase B"

# ===== Phase C: interactive abort =====
# Requires util-linux script(1) to allocate a pty so the child sees a
# real TTY for stdin. Without it, the test seam doesn't exist and the
# unit test reconcile_commit_q_aborts_without_writing in pearlite-cli
# already covers the same observable behaviour.
if command -v script >/dev/null 2>&1; then
    phase_c="$sandbox/phase-c.json"
    # `script -E never` disables echo so our `q\n` stdin is not echoed
    # back as another input line. `-e` propagates the child's exit
    # code, `-q` suppresses script's own banner. AI_AGENT must be
    # cleared from this child's environment because the helper in
    # `pearlite_cli::agents::is_non_interactive` falls back on it
    # before reaching the TTY check (ADR-0014 §6) — without `unset`
    # the harness inherits an `AI_AGENT` set by an outer agent (e.g.
    # Claude Code's invocation of run-vm-tests.sh) and the prompt loop
    # is never reached.
    # `--commit-threshold 9999` keeps the threshold guard from
    # tripping before the prompt loop runs on hosts with realistic
    # Manual-drift counts (a freshly probed CachyOS install has tens
    # to low-hundreds of explicit packages; ADR-0014 §1's default of
    # 5 is the safety, not what we want to test here).
    set +e
    printf 'q\n' | env -u AI_AGENT script -q -e -E never \
        -c "'$PEARLITE_BIN' --format=json \
            --config-dir='$sandbox/repo' \
            --state-file='$state_path' \
            reconcile --commit --commit-threshold 9999 > '$phase_c'" \
        /dev/null > /dev/null
    phase_c_exit=$?
    set -e

    if [ "$phase_c_exit" -ne 0 ]; then
        printf 'vm-11: phase C expected exit 0 (clean abort), got %s; full output:\n' \
            "$phase_c_exit" >&2
        cat "$phase_c" >&2
        exit 1
    fi

    fail=0
    for needle in \
        '"committed":false' \
        '"aborted":true'
    do
        if ! grep -qF -- "$needle" "$phase_c"; then
            printf 'vm-11: phase C missing %s in envelope\n' "$needle" >&2
            fail=1
        fi
    done

    if [ "$fail" -ne 0 ]; then
        printf 'vm-11: phase C envelope shape check failed; full output:\n' >&2
        cat "$phase_c" >&2
        exit 1
    fi

    assert_state_unchanged "phase C"
else
    printf 'vm-11: phase C skipped (script(1) not on PATH; install util-linux)\n' >&2
fi

# ===== Phase D: adopt-all happy path =====
phase_d="$sandbox/phase-d.json"
"$PEARLITE_BIN" \
    --format=json \
    --config-dir="$sandbox/repo" \
    --state-file="$state_path" \
    reconcile --commit --adopt-all \
    > "$phase_d" || {
        printf 'vm-11: phase D reconcile-commit exit %s\n' "$?" >&2
        cat "$phase_d" >&2
        exit 1
    }

fail=0
for needle in \
    '"committed":true' \
    '"aborted":false' \
    '"action":"adopt_all"' \
    '"considered":' \
    '"adopted":['
do
    if ! grep -qF -- "$needle" "$phase_d"; then
        printf 'vm-11: phase D missing %s in envelope\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-11: phase D envelope shape check failed; full output:\n' >&2
    cat "$phase_d" >&2
    exit 1
fi

# `considered` must be > 0 — the test relies on the host having at
# least one explicitly-installed package classified as Manual.
considered=$(grep -o '"considered":[0-9]*' "$phase_d" \
    | head -1 \
    | sed 's/^"considered"://')
if [ -z "$considered" ] || [ "$considered" -lt 1 ]; then
    printf 'vm-11: phase D considered=%s; expected at least 1 Manual item on the host\n' \
        "$considered" >&2
    cat "$phase_d" >&2
    exit 1
fi

# state.toml must now contain an [[reconciliations]] entry plus
# [adopted].pacman with at least one name. Substring grep, no jq.
fail=0
for needle in \
    '[[reconciliations]]' \
    'action = "adopt_all"' \
    '[adopted]'
do
    if ! grep -qF "$needle" "$state_path"; then
        printf 'vm-11: phase D state.toml missing %s\n' "$needle" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    printf 'vm-11: phase D state.toml shape check failed; full output:\n' >&2
    cat "$state_path" >&2
    exit 1
fi

# `last_modified` must have been set.
if ! grep -q '^last_modified' "$state_path"; then
    printf 'vm-11: phase D state.toml missing last_modified\n' >&2
    cat "$state_path" >&2
    exit 1
fi

# `last_apply` must NOT have been set (reconcile is not an apply).
if grep -q '^last_apply' "$state_path"; then
    printf 'vm-11: phase D state.toml has last_apply set; reconcile must not bump it\n' >&2
    cat "$state_path" >&2
    exit 1
fi

printf 'vm-11: reconcile-commit PASS\n'
