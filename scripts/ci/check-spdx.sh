#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mohamed Hammad
#
# SPDX-001: every tracked .rs file must begin with the two-line SPDX header.
# Used by the rusty-hook pre-commit hook and CI tier 1.
#
# POSIX sh; no Bash-isms.

set -eu

EXPECTED_LINE_1='// SPDX-License-Identifier: GPL-3.0-or-later'
EXPECTED_LINE_2='// Copyright (C) 2026 Mohamed Hammad'

violations=$(mktemp)
trap 'rm -f "$violations"' EXIT INT TERM

list_rs_files() {
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        git ls-files '*.rs'
    else
        find . -name '*.rs' -not -path './target/*' -not -path './.git/*'
    fi
}

list_rs_files | while IFS= read -r f; do
    [ -f "$f" ] || continue
    line1=$(sed -n '1p' "$f")
    line2=$(sed -n '2p' "$f")
    if [ "$line1" != "$EXPECTED_LINE_1" ] || [ "$line2" != "$EXPECTED_LINE_2" ]; then
        printf '%s\n' "$f" >> "$violations"
    fi
done

if [ -s "$violations" ]; then
    while IFS= read -r f; do
        printf 'SPDX-001 violation: %s\n' "$f" >&2
    done < "$violations"
    printf '\nEvery .rs file must begin with:\n  %s\n  %s\n' \
        "$EXPECTED_LINE_1" "$EXPECTED_LINE_2" >&2
    exit 1
fi

printf 'SPDX-001: all tracked .rs files have the canonical header\n'
