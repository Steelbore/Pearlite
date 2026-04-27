// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! In-memory [`MockProbe`] for unit tests and engine integration tests.

use crate::errors::ProbeError;
use crate::probe::SystemProbe;
use pearlite_schema::ProbedState;

/// Canned-state [`SystemProbe`]: returns whatever [`ProbedState`] the
/// caller seeded it with.
#[derive(Clone, Debug)]
pub struct MockProbe {
    canned: ProbedState,
}

impl MockProbe {
    /// Construct a [`MockProbe`] pre-seeded with the given state.
    #[must_use]
    pub fn with_state(canned: ProbedState) -> Self {
        Self { canned }
    }
}

impl SystemProbe for MockProbe {
    fn probe(&self) -> Result<ProbedState, ProbeError> {
        Ok(self.canned.clone())
    }
}
