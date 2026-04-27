// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! JSON envelope shape per PRD §9.3.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Canonical JSON envelope every Pearlite command emits.
///
/// Either `data` or `error` is populated, never both. Both share
/// `metadata`. PRD §9.3 specifies the exact shape; field-by-field:
/// `schema_version` describes the envelope, not the data block;
/// `metadata.completed_at` is ISO 8601 UTC string; `error.hint` is a
/// runnable command (tips-thinking discipline).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// Envelope shape version. Independent of data shape.
    pub schema_version: String,
    /// Metadata common to success and failure.
    pub metadata: Metadata,
    /// Success payload. Populated when the command succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Failure payload. Populated when the command failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

/// Common metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metadata {
    /// Command name (e.g. `pearlite plan`).
    pub command: String,
    /// Host this invocation targeted, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Pearlite version.
    pub tool_version: String,
    /// ISO 8601 UTC timestamp the command completed.
    pub completed_at: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Config repo root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<PathBuf>,
    /// Invoking agent identifier from env (e.g. `claude-code`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoking_agent: Option<String>,
}

/// Failure payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Stable code (e.g. `PREFLIGHT_SNAPPER_MISSING`).
    pub code: String,
    /// Failure class (e.g. `preflight`, `plan`, `apply`).
    pub class: String,
    /// Process exit code.
    pub exit_code: u8,
    /// Human-readable message.
    pub message: String,
    /// Runnable command suggestion (tips-thinking).
    pub hint: String,
    /// Subsystem-specific details.
    #[serde(default)]
    pub details: serde_json::Value,
}

impl Envelope {
    /// Construct a success envelope.
    #[must_use]
    pub fn success(metadata: Metadata, data: serde_json::Value) -> Self {
        Self {
            schema_version: "1.0".to_owned(),
            metadata,
            data: Some(data),
            error: None,
        }
    }

    /// Construct a failure envelope.
    #[must_use]
    pub fn failure(metadata: Metadata, error: ErrorPayload) -> Self {
        Self {
            schema_version: "1.0".to_owned(),
            metadata,
            data: None,
            error: Some(error),
        }
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

    fn meta() -> Metadata {
        Metadata {
            command: "pearlite plan".to_owned(),
            host: Some("forge".to_owned()),
            tool_version: "0.1.0".to_owned(),
            completed_at: "2026-04-27T15:14:00Z".to_owned(),
            duration_ms: 234,
            config_dir: None,
            invoking_agent: None,
        }
    }

    #[test]
    fn success_envelope_serializes_with_data_only() {
        let e = Envelope::success(meta(), serde_json::json!({"actions": []}));
        let v = serde_json::to_value(&e).expect("serialize");
        assert!(v.get("data").is_some());
        assert!(v.get("error").is_none());
    }

    #[test]
    fn failure_envelope_serializes_with_error_only() {
        let e = Envelope::failure(
            meta(),
            ErrorPayload {
                code: "PREFLIGHT_FAILED".to_owned(),
                class: "preflight".to_owned(),
                exit_code: 2,
                message: "snapper missing".to_owned(),
                hint: "snapper -c root create-config /".to_owned(),
                details: serde_json::Value::Null,
            },
        );
        let v = serde_json::to_value(&e).expect("serialize");
        assert!(v.get("data").is_none());
        assert!(v.get("error").is_some());
    }

    #[test]
    fn metadata_omits_none_fields() {
        let m = Metadata {
            command: "pearlite plan".to_owned(),
            host: None,
            tool_version: "0.1.0".to_owned(),
            completed_at: "2026-04-27T15:14:00Z".to_owned(),
            duration_ms: 0,
            config_dir: None,
            invoking_agent: None,
        };
        let v = serde_json::to_value(&m).expect("serialize");
        assert!(v.get("host").is_none());
        assert!(v.get("config_dir").is_none());
        assert!(v.get("invoking_agent").is_none());
    }
}
