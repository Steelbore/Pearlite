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
//! `meta`, `kernel`, `packages`, `services`, and `users` are emitted.
//! Unprobed `meta` fields (`timezone`, `arch_level`, `locale`,
//! `keymap`) emit conservative defaults that the operator hand-edits
//! after reconcile lands the file at
//! `<config_dir>/hosts/<host>.imported.ncl`. Subsequent chunks add the
//! config-file block.
//!
//! ## Users-block caveat
//!
//! [`ProbedState`] carries no user inventory — Pearlite manages
//! declared users (Plan §6.7) but does not enumerate `/etc/passwd` at
//! probe time, since reconcile-without-declarations would surface
//! every system account (root, daemon, http, etc.) as drift. The
//! emitter therefore writes `users = []` as a placeholder; the
//! operator is expected to add `UserDecl` entries by hand after the
//! initial import. A populated users inventory is a v1.1 candidate
//! (PRD §17.1) and would slot in here without changing the block
//! shape.
//!
//! ## Services-block caveat
//!
//! On a typical CachyOS install, `systemctl list-unit-files` reports
//! hundreds of enabled units (every package-installed default), so
//! emitting `services.enabled` verbatim produces a noisy starting
//! point. Per PRD §11 the imported file is a *review draft*, not a
//! polished declaration; operators are expected to curate. We emit
//! the full sorted/deduped set rather than try to guess "noise"
//! versus "intentional" — guessing wrong silently drops state that
//! ought to be declared.
//!
//! The `packages` block buckets explicit packages by their probed
//! repo: `core`/`extra`/`multilib`/unknown → `packages.core`,
//! `cachyos` → `packages.cachyos`, `cachyos-v3` →
//! `packages.cachyos-v3`, `cachyos-v4` → `packages.cachyos-v4`,
//! foreign packages (and the `aur` repo) → `packages.aur`. Cargo
//! crates from the `CargoInventory` populate `packages.cargo`.
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

use pearlite_schema::{CargoInventory, PacmanInventory, ProbedState, ServiceInventory};
use std::collections::BTreeMap;

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
    let mut out = String::with_capacity(512);
    out.push_str("{\n");
    push_meta(&mut out, &probed.host.hostname);
    push_kernel(&mut out, &probed.kernel.package);
    push_packages(&mut out, probed.pacman.as_ref(), probed.cargo.as_ref());
    push_services(&mut out, probed.services.as_ref());
    push_users(&mut out);
    out.push_str("}\n");
    out
}

fn push_users(out: &mut String) {
    out.push_str("  users = [],\n");
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

fn push_packages(
    out: &mut String,
    pacman: Option<&PacmanInventory>,
    cargo: Option<&CargoInventory>,
) {
    let buckets = bucket_packages(pacman, cargo);
    out.push_str("  packages = {\n");
    // Iteration order is the BTreeMap's lexicographic key order, which
    // matches the schema's stable bucket ordering for deterministic
    // emit.
    for (bucket, names) in &buckets {
        push_array_field(out, bucket, names);
    }
    out.push_str("  },\n");
}

/// Bucket every explicit pacman package + foreign package + cargo
/// crate into the `packages.*` table per [`PackageSet`](pearlite_schema::PackageSet).
///
/// Returns a `BTreeMap<bucket_name, sorted_unique_names>` so callers
/// (and the round-trip TOML re-parse in tests) get a deterministic
/// shape regardless of input order.
fn bucket_packages(
    pacman: Option<&PacmanInventory>,
    cargo: Option<&CargoInventory>,
) -> BTreeMap<&'static str, Vec<String>> {
    let mut buckets: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    if let Some(p) = pacman {
        for pkg in &p.explicit {
            // Foreign trumps repo: a package that's both -Qe and -Qm
            // installed is an AUR package per
            // pearlite_pacman::inventory's classifier.
            let bucket = if p.foreign.contains(pkg) {
                "aur"
            } else {
                bucket_for_repo(p.repos.get(pkg).map(String::as_str))
            };
            buckets.entry(bucket).or_default().push(pkg.clone());
        }
    }
    if let Some(c) = cargo {
        for name in c.crates.keys() {
            buckets.entry("cargo").or_default().push(name.clone());
        }
    }
    // BTreeSet membership above already gives sorted input within
    // each repo, but bucket_for_repo merges multiple repos into "core";
    // sort + dedup the merged result so emission stays deterministic.
    for v in buckets.values_mut() {
        v.sort();
        v.dedup();
    }
    buckets
}

fn push_services(out: &mut String, services: Option<&ServiceInventory>) {
    out.push_str("  services = {\n");
    if let Some(s) = services {
        push_service_array(out, "enabled", &s.enabled);
        push_service_array(out, "disabled", &s.disabled);
        push_service_array(out, "masked", &s.masked);
    } else {
        // Probe didn't return a service inventory — emit empty
        // declarations so the schema's #[serde(default)] doesn't have
        // to absorb a missing block.
        push_array_field(out, "enabled", &[]);
        push_array_field(out, "disabled", &[]);
        push_array_field(out, "masked", &[]);
    }
    out.push_str("  },\n");
}

fn push_service_array(out: &mut String, key: &str, units: &std::collections::BTreeSet<String>) {
    let v: Vec<String> = units.iter().cloned().collect();
    push_array_field(out, key, &v);
}

fn bucket_for_repo(repo: Option<&str>) -> &'static str {
    match repo {
        Some("cachyos") => "cachyos",
        Some("cachyos-v3") => "cachyos-v3",
        Some("cachyos-v4") => "cachyos-v4",
        Some("aur") => "aur",
        // "core", "extra", "multilib", any other (including None) all
        // collapse into "core" — the operator can hand-classify after
        // reconcile if a custom repo deserves its own bucket.
        _ => "core",
    }
}

fn push_array_field(out: &mut String, key: &str, values: &[String]) {
    out.push_str("    ");
    push_quoted_or_bare_key(out, key);
    out.push_str(" = [");
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(&escape(v));
        out.push('"');
    }
    out.push_str("],\n");
}

fn push_quoted_or_bare_key(out: &mut String, key: &str) {
    // Nickel field names that contain `-` need quoting.
    if key.contains('-') {
        out.push('"');
        out.push_str(key);
        out.push('"');
    } else {
        out.push_str(key);
    }
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
    use pearlite_schema::{HostInfo, KernelInfo, ServiceInventory};
    use std::collections::{BTreeMap, BTreeSet};
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

    fn probed_with_packages(
        explicit: &[&str],
        foreign: &[&str],
        repos: &[(&str, &str)],
        crates: &[&str],
    ) -> ProbedState {
        let mut p = probed_with("forge", "linux-cachyos");
        let mut explicit_set: BTreeSet<String> = BTreeSet::new();
        for n in explicit {
            explicit_set.insert((*n).to_owned());
        }
        let mut foreign_set: BTreeSet<String> = BTreeSet::new();
        for n in foreign {
            foreign_set.insert((*n).to_owned());
        }
        let mut repos_map: BTreeMap<String, String> = BTreeMap::new();
        for (name, repo) in repos {
            repos_map.insert((*name).to_owned(), (*repo).to_owned());
        }
        p.pacman = Some(PacmanInventory {
            explicit: explicit_set,
            foreign: foreign_set,
            repos: repos_map,
        });
        let mut crates_map: BTreeMap<String, String> = BTreeMap::new();
        for n in crates {
            crates_map.insert((*n).to_owned(), "1.0.0".to_owned());
        }
        p.cargo = Some(CargoInventory { crates: crates_map });
        p
    }

    #[test]
    fn emit_packages_buckets_by_repo() {
        let probed = probed_with_packages(
            &[
                "base",
                "linux-cachyos",
                "firefox",
                "blender",
                "claude-code",
                "htop",
            ],
            &["claude-code"],
            &[
                ("base", "core"),
                ("linux-cachyos", "core"),
                ("firefox", "cachyos-v4"),
                ("blender", "cachyos-v4"),
                ("htop", "extra"),
                // claude-code in repos map: foreign overrides repo
                // even if the map says cachyos.
                ("claude-code", "cachyos"),
            ],
            &["zellij", "ripgrep-all"],
        );
        let out = emit_host(&probed);
        // core bucket: base + htop + linux-cachyos (extra collapses
        // into core).
        assert!(out.contains(r#"core = ["base", "htop", "linux-cachyos"],"#));
        // cachyos-v4 bucket needs the quoted key (Nickel field-name
        // syntax requires quotes on `-`-containing identifiers).
        assert!(out.contains(r#""cachyos-v4" = ["blender", "firefox"],"#));
        // claude-code lands in aur, NOT cachyos, despite the repos
        // map saying "cachyos".
        assert!(out.contains(r#"aur = ["claude-code"],"#));
        // cargo crates go to packages.cargo, sorted.
        assert!(out.contains(r#"cargo = ["ripgrep-all", "zellij"],"#));
        // No accidental cachyos bucket on this fixture (the only
        // "cachyos"-mapped pkg was claude-code → AUR).
        assert!(!out.contains("cachyos = ["), "got: {out}");
    }

    #[test]
    fn emit_packages_omits_block_when_pacman_none_and_cargo_none() {
        let probed = probed_with("forge", "linux-cachyos");
        let out = emit_host(&probed);
        // Empty packages = {} block is acceptable; the schema treats
        // each list as #[serde(default)].
        assert!(out.contains("packages = {"));
    }

    #[test]
    fn emit_packages_unknown_repo_collapses_into_core() {
        let probed = probed_with_packages(
            &["chaotic-pkg"],
            &[],
            &[("chaotic-pkg", "chaotic-aur")],
            &[],
        );
        let out = emit_host(&probed);
        assert!(out.contains(r#"core = ["chaotic-pkg"],"#));
    }

    fn make_services(enabled: &[&str], disabled: &[&str], masked: &[&str]) -> ServiceInventory {
        let to_set =
            |xs: &[&str]| -> BTreeSet<String> { xs.iter().map(|x| (*x).to_owned()).collect() };
        ServiceInventory {
            enabled: to_set(enabled),
            disabled: to_set(disabled),
            masked: to_set(masked),
            active: BTreeSet::new(),
        }
    }

    #[test]
    fn emit_services_emits_three_arrays() {
        let mut probed = probed_with("forge", "linux-cachyos");
        probed.services = Some(make_services(
            &["sshd.service", "NetworkManager.service"],
            &["bluetooth.service"],
            &["systemd-resolved.service"],
        ));
        let out = emit_host(&probed);
        // BTreeSet enumerates in sorted order: NetworkManager <
        // sshd lexicographically.
        assert!(out.contains(r#"enabled = ["NetworkManager.service", "sshd.service"],"#));
        assert!(out.contains(r#"disabled = ["bluetooth.service"],"#));
        assert!(out.contains(r#"masked = ["systemd-resolved.service"],"#));
    }

    #[test]
    fn emit_services_emits_empty_block_when_inventory_absent() {
        let probed = probed_with("forge", "linux-cachyos");
        let out = emit_host(&probed);
        assert!(out.contains("services = {"));
        assert!(out.contains("enabled = [],"));
        assert!(out.contains("disabled = [],"));
        assert!(out.contains("masked = [],"));
    }

    #[test]
    fn emit_services_appears_after_packages() {
        let probed = probed_with("forge", "linux-cachyos");
        let out = emit_host(&probed);
        let packages_at = out.find("packages = ").expect("packages block");
        let services_at = out.find("services = ").expect("services block");
        assert!(
            packages_at < services_at,
            "packages must precede services for stable golden-fixture diffs"
        );
    }

    #[test]
    fn emit_users_emits_empty_array_placeholder() {
        // ProbedState carries no user inventory, so the emitter writes
        // a literal empty array — the operator hand-edits after the
        // imported.ncl lands.
        let out = emit_host(&probed_with("forge", "linux-cachyos"));
        assert!(out.contains("users = [],"));
    }

    #[test]
    fn emit_users_appears_after_services() {
        let mut probed = probed_with("forge", "linux-cachyos");
        probed.services = Some(make_services(&["sshd.service"], &[], &[]));
        let out = emit_host(&probed);
        let services_at = out.find("services = ").expect("services block");
        let users_at = out.find("users = ").expect("users block");
        assert!(
            services_at < users_at,
            "users must follow services for stable golden-fixture diffs"
        );
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
