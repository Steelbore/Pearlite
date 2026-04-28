// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

#![allow(
    clippy::missing_panics_doc,
    reason = "test-only mock; lock contention shouldn't surface to callers"
)]

//! In-memory [`MockHmBackend`] for engine integration tests.

use crate::errors::UserenvError;
use crate::live::{HomeManagerBackend, UserEnvOutcome};
use pearlite_schema::HomeManagerMode;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One recorded `switch` invocation. Tests inspect these via
/// [`MockHmBackend::switch_history`] to verify the engine called the
/// adapter with the right argv.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchInvocation {
    /// Login name passed to `runuser -u`.
    pub user: String,
    /// `config_path` argument.
    pub config_path: PathBuf,
    /// `mode` argument.
    pub mode: HomeManagerMode,
    /// `channel` argument.
    pub channel: String,
}

#[derive(Debug, Default)]
struct State {
    history: Vec<SwitchInvocation>,
    next_generation: u64,
}

/// In-memory [`HomeManagerBackend`] that records every `switch` call.
///
/// Each successful `switch` increments an internal counter and reports
/// it as the new generation number — matching how a real Home Manager
/// install behaves on a fresh system.
#[derive(Clone, Debug, Default)]
pub struct MockHmBackend {
    state: Arc<Mutex<State>>,
}

impl MockHmBackend {
    /// Construct a fresh [`MockHmBackend`] starting at generation 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                history: Vec::new(),
                next_generation: 1,
            })),
        }
    }

    /// Snapshot the recorded switch history.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn switch_history(&self) -> Vec<SwitchInvocation> {
        self.state
            .lock()
            .expect("MockHmBackend mutex must not be poisoned")
            .history
            .clone()
    }

    /// Number of `switch` calls so far.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    pub fn switch_count(&self) -> usize {
        self.state
            .lock()
            .expect("MockHmBackend mutex must not be poisoned")
            .history
            .len()
    }
}

impl HomeManagerBackend for MockHmBackend {
    #[allow(
        clippy::expect_used,
        reason = "test utility — mutex-poison panic is unreachable"
    )]
    fn switch(
        &self,
        user: &str,
        config_path: &Path,
        mode: HomeManagerMode,
        channel: &str,
    ) -> Result<UserEnvOutcome, UserenvError> {
        let mut s = self
            .state
            .lock()
            .expect("MockHmBackend mutex must not be poisoned");
        s.history.push(SwitchInvocation {
            user: user.to_owned(),
            config_path: config_path.to_path_buf(),
            mode,
            channel: channel.to_owned(),
        });
        let generation = s.next_generation;
        s.next_generation += 1;
        Ok(UserEnvOutcome {
            user: user.to_owned(),
            generation,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests may use expect()/unwrap()/panic!() per Plan §4.2 + CLAUDE.md"
)]
mod tests {
    use super::*;

    #[test]
    fn switch_records_invocation_and_assigns_monotonic_generation() {
        let m = MockHmBackend::new();
        let out1 = m
            .switch(
                "alice",
                Path::new("/repo/users/alice"),
                HomeManagerMode::Standalone,
                "release-24.11",
            )
            .expect("switch");
        assert_eq!(out1.generation, 1);
        let out2 = m
            .switch(
                "bob",
                Path::new("/repo/users/bob"),
                HomeManagerMode::Flake,
                "default",
            )
            .expect("switch");
        assert_eq!(out2.generation, 2);

        let hist = m.switch_history();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].user, "alice");
        assert_eq!(hist[0].mode, HomeManagerMode::Standalone);
        assert_eq!(hist[1].user, "bob");
        assert_eq!(hist[1].mode, HomeManagerMode::Flake);
    }

    #[test]
    fn switch_count_matches_history_length() {
        let m = MockHmBackend::new();
        assert_eq!(m.switch_count(), 0);
        let _ = m.switch(
            "alice",
            Path::new("/cfg"),
            HomeManagerMode::Standalone,
            "release-24.11",
        );
        assert_eq!(m.switch_count(), 1);
    }
}
