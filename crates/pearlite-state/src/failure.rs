// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `[[failures]]` records — pointers to per-plan failure JSON files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::OffsetDateTime;
use uuid::Uuid;

/// One entry in `state.toml`'s `[[failures]]` array.
///
/// The full forensic record (stderr, `investigate_commands`,
/// `rollback_command`, …) lives in `/var/lib/pearlite/failures/<plan-id>.json`
/// per PRD §11.4. This entry holds only the pointer plus enough metadata
/// to render `pearlite gen list` without reading every JSON record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FailureRef {
    /// Plan UUID this failure pertains to.
    #[schemars(with = "String")]
    pub plan_id: Uuid,
    /// UTC timestamp the apply failed.
    #[serde(with = "time::serde::iso8601")]
    #[schemars(with = "String")]
    pub failed_at: OffsetDateTime,
    /// Failure class per PRD §8.5: 1 preflight, 2 plan, 3 recoverable,
    /// 4 incoherent, 5 catastrophic.
    pub class: u8,
    /// Process exit code at failure.
    pub exit_code: u8,
    /// Path to the JSON record under `/var/lib/pearlite/failures/`.
    pub record_path: PathBuf,
}
