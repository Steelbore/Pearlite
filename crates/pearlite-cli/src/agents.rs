// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Agent / interactivity detection.
//!
//! Per CLAUDE.md hard invariant: the CLI is the one place that reads
//! the environment. Other crates ask `pearlite_cli::agents` rather than
//! touching `std::env` directly.

use std::io::IsTerminal as _;

/// Returns `true` when the current invocation has no operator at the
/// keyboard.
///
/// M4 stub per ADR-0014 §6: looks at stdin's TTY-ness and the
/// `AI_AGENT` env var only. The full `AGENT` / `CI` / `CLAUDECODE`
/// matrix lands in M5 W2 alongside the broader agent-UX work.
///
// TODO(M5): extend with AGENT=1, CI=true, and CLAUDECODE/CURSOR_AGENT
// per the ADR-0014 §5 acceptance contract.
#[must_use]
pub fn is_non_interactive() -> bool {
    !std::io::stdin().is_terminal() || std::env::var_os("AI_AGENT").is_some()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests may use expect()/unwrap() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;

    /// Smoke-test: the helper must not panic when called. Real
    /// behaviour is environment-dependent and exercised by the
    /// dispatch-level tests that inject the input/policy directly.
    #[test]
    fn is_non_interactive_is_callable() {
        let _ = is_non_interactive();
    }
}
