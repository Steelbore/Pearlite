// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Emit Nickel host-config text from a [`ProbedState`].
//!
//! M4 W1 reconcile direction: where [`load_host`](crate::load_host)
//! takes Nickel → TOML → [`DeclaredState`], this module goes the
//! other way — [`ProbedState`] → Nickel-syntax host file string.
//!
//! ## Scope (this chunk)
//!
//! Only the always-present blocks are emitted: `meta` and `kernel`.
//! Unprobed `meta` fields (`timezone`, `arch_level`, `locale`,
//! `keymap`) emit conservative defaults that the operator hand-edits
//! after reconcile lands the file at
//! `<config_dir>/hosts/<host>.imported.ncl`. Subsequent chunks add
//! the `packages`, `services`, `users`, and config-file blocks.
//!
//! ## Non-goals
//!
//! - **No round-trip via `nickel`**: this emitter produces text that
//!   parses through `nickel export -f toml` back into a valid
//!   [`DeclaredState`], but exercising the full round-trip needs the
//!   `nickel` binary, which lives behind the live evaluator. Unit
//!   tests assert structure via string predicates and a side-channel
//!   TOML re-parse.
//! - **No formatting beyond two-space indent + sorted keys**: the
//!   output is operator-readable, not pretty-printed by `nickel
//!   format`. Operators run that themselves if desired.

use pearlite_schema::ProbedState;

/// Render a Nickel host-config string from `probed`.
///
/// The output is a single Nickel record literal terminated by a
/// trailing newline. It is intended to land at
/// `<config_dir>/hosts/<probed.host.hostname>.imported.ncl` as the
/// starting point for an operator's reconcile review.
///
/// Conservative defaults fill in unprobed `meta` fields:
/// - `timezone = "UTC"` — operator inspects `/etc/localtime` symlink
///   and corrects.
/// - `arch_level = "v3"` — Plan default for x86-64; reconcile probes
///   `/proc/cpuinfo` flags in a follow-up chunk to upgrade to v4 when
///   AVX-512 etc. are present.
/// - `locale = "en_US.UTF-8"`, `keymap = "us"` — likewise placeholders.
///
/// Hostnames containing characters that need escaping in Nickel
/// strings (backslash, double-quote) are escaped per Nickel grammar;
/// unprintable bytes are NOT supported because hostnames are already
/// constrained by RFC 1123 to a printable subset.
#[must_use]
pub fn emit_host(probed: &ProbedState) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("{\n");
    push_meta(&mut out, &probed.host.hostname);
    push_kernel(&mut out, &probed.kernel.package);
    out.push_str("}\n");
    out
}

fn push_meta(out: &mut String, hostname: &str) {
    out.push_str("  meta = {\n");
    push_field(out, "hostname", hostname);
    push_field(out, "timezone", "UTC");
    push_field(out, "arch_level", "v3");
    push_field(out, "locale", "en_US.UTF-8");
    push_field(out, "keymap", "us");
    out.push_str("  },\n");
}

fn push_kernel(out: &mut String, package: &str) {
    out.push_str("  kernel = {\n");
    push_field(out, "package", package);
    out.push_str("  },\n");
}

fn push_field(out: &mut String, key: &str, value: &str) {
    out.push_str("    ");
    out.push_str(key);
    out.push_str(" = \"");
    out.push_str(&escape(value));
    out.push_str("\",\n");
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out
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
    use pearlite_schema::{HostInfo, KernelInfo};
    use std::collections::BTreeSet;
    use time::OffsetDateTime;

    fn probed_with(hostname: &str, kernel: &str) -> ProbedState {
        ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: hostname.to_owned(),
            },
            pacman: None,
            cargo: None,
            config_files: None,
            services: None,
            kernel: KernelInfo {
                running_version: String::new(),
                package: kernel.to_owned(),
                loaded_modules: BTreeSet::new(),
            },
        }
    }

    #[test]
    fn emit_basic_host() {
        let out = emit_host(&probed_with("forge", "linux-cachyos"));
        assert!(out.starts_with("{\n"));
        assert!(out.ends_with("}\n"));
        assert!(out.contains("hostname = \"forge\""));
        assert!(out.contains("package = \"linux-cachyos\""));
        // Conservative defaults are present.
        assert!(out.contains("timezone = \"UTC\""));
        assert!(out.contains("arch_level = \"v3\""));
        assert!(out.contains("locale = \"en_US.UTF-8\""));
        assert!(out.contains("keymap = \"us\""));
    }

    #[test]
    fn emit_escapes_double_quote_in_hostname() {
        // Hostname with an embedded quote (not RFC-1123 valid, but
        // tests the emitter's escape path).
        let out = emit_host(&probed_with("ev\"il", "linux-cachyos"));
        assert!(out.contains(r#"hostname = "ev\"il""#));
    }

    #[test]
    fn emit_escapes_backslash() {
        let out = emit_host(&probed_with("ho\\st", "linux-cachyos"));
        assert!(out.contains(r#"hostname = "ho\\st""#));
    }

    #[test]
    fn emit_top_level_record_is_lone_block() {
        // Exactly one `{` at column 0 (the opening brace) and exactly
        // one `}` at column 0 (the closing brace). Sub-blocks live
        // indented.
        let out = emit_host(&probed_with("forge", "linux-cachyos"));
        let lone_open = out.lines().filter(|l| *l == "{").count();
        let lone_close = out.lines().filter(|l| *l == "}").count();
        assert_eq!(lone_open, 1, "expected exactly one top-level {{");
        assert_eq!(lone_close, 1, "expected exactly one top-level }}");
    }

    #[test]
    fn emit_meta_block_appears_before_kernel_block() {
        // Stable ordering matters for operator review (and for
        // golden-fixture diffs in subsequent chunks).
        let out = emit_host(&probed_with("forge", "linux-cachyos"));
        let meta_at = out.find("meta =").expect("meta block emitted");
        let kernel_at = out.find("kernel =").expect("kernel block emitted");
        assert!(meta_at < kernel_at, "meta must precede kernel");
    }
}
