// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Output renderers.

use crate::envelope::Envelope;
use std::io::Write;

/// Render an envelope as compact JSON (canonical agent format).
///
/// # Errors
/// Propagates any I/O error from `out`.
pub fn render_json(envelope: &Envelope, out: &mut dyn Write) -> std::io::Result<()> {
    serde_json::to_writer(&mut *out, envelope)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Render an envelope in TTY-pretty form.
///
/// M1 implementation is intentionally simple: section headers + a
/// summary, no colours yet. The Steelbore palette and richer formatting
/// land alongside the schema/MCP work in M5.
///
/// # Errors
/// Propagates any I/O error from `out`.
pub fn render_human(envelope: &Envelope, out: &mut dyn Write) -> std::io::Result<()> {
    if let Some(error) = envelope.error.as_ref() {
        writeln!(out, "error [{}] {}", error.code, error.message)?;
        if !error.hint.is_empty() {
            writeln!(out, "  hint: {}", error.hint)?;
        }
        return Ok(());
    }

    writeln!(
        out,
        "{}  ({}ms)",
        envelope.metadata.command, envelope.metadata.duration_ms
    )?;
    if let Some(host) = &envelope.metadata.host {
        writeln!(out, "  host: {host}")?;
    }

    let Some(data) = envelope.data.as_ref() else {
        writeln!(out, "  (no data)")?;
        return Ok(());
    };

    if let Some(actions) = data.get("actions").and_then(|v| v.as_array()) {
        writeln!(out, "  actions: {}", actions.len())?;
    }
    if let Some(drift) = data.get("drift").and_then(|v| v.as_array()) {
        writeln!(out, "  drift:   {}", drift.len())?;
    }
    if let Some(warnings) = data.get("warnings").and_then(|v| v.as_array()) {
        writeln!(out, "  warnings: {}", warnings.len())?;
    }
    Ok(())
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
    use crate::envelope::{ErrorPayload, Metadata};

    fn meta() -> Metadata {
        Metadata {
            command: "pearlite plan".to_owned(),
            host: Some("forge".to_owned()),
            tool_version: "0.1.0".to_owned(),
            completed_at: "2026-04-27T15:14:00Z".to_owned(),
            duration_ms: 42,
            config_dir: None,
            invoking_agent: None,
        }
    }

    #[test]
    fn render_json_emits_newline_terminated_compact() {
        let e = Envelope::success(meta(), serde_json::json!({"actions": []}));
        let mut out = Vec::new();
        render_json(&e, &mut out).expect("render");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.ends_with('\n'));
        assert!(
            !text.contains("\n  "),
            "json output must be compact: {text}"
        );
    }

    #[test]
    fn render_human_summarizes_counts() {
        let e = Envelope::success(
            meta(),
            serde_json::json!({
                "actions": [{"kind": "pacman_install"}],
                "drift": [{"category": "manual_package"}],
                "warnings": []
            }),
        );
        let mut out = Vec::new();
        render_human(&e, &mut out).expect("render");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("actions: 1"));
        assert!(text.contains("drift:   1"));
        assert!(text.contains("warnings: 0"));
        assert!(text.contains("forge"));
    }

    #[test]
    fn render_human_shows_error_with_hint() {
        let e = Envelope::failure(
            meta(),
            ErrorPayload {
                code: "FOO".to_owned(),
                class: "preflight".to_owned(),
                exit_code: 2,
                message: "thing missing".to_owned(),
                hint: "do this".to_owned(),
                details: serde_json::Value::Null,
            },
        );
        let mut out = Vec::new();
        render_human(&e, &mut out).expect("render");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("error [FOO]"));
        assert!(text.contains("do this"));
    }
}
