// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Pearlite audit library: Steelbore Standard compliance checks.
//!
//! At M0 only `SPDX-001` is implemented. The remaining checks
//! (`LIC-001`, `LIC-002`, `NAMING-001`, `TIME-001`, `TIME-002`,
//! `UNITS-001`, `PALETTE-001`, `UNSAFE-001`, `PANIC-001`) land in
//! later milestones per Plan §6.13.

use std::path::{Path, PathBuf};

/// Expected first line of every Pearlite `.rs` source file.
pub const SPDX_LINE_1: &str = "// SPDX-License-Identifier: GPL-3.0-or-later";
/// Expected second line of every Pearlite `.rs` source file.
pub const SPDX_LINE_2: &str = "// Copyright (C) 2026 Mohamed Hammad";

/// A single compliance violation, attributable to one check and one path.
#[derive(Debug, Clone)]
pub struct Violation {
    /// Stable check identifier (e.g. `SPDX-001`).
    pub check_id: &'static str,
    /// Path of the offending file, relative to the audit root.
    pub path: PathBuf,
    /// Short human-readable explanation of why the check failed.
    pub message: String,
}

/// Static description of one registered check.
#[derive(Debug, Clone, Copy)]
pub struct CheckInfo {
    /// Stable check identifier (e.g. `SPDX-001`).
    pub id: &'static str,
    /// One-line summary, suitable for `pearlite-audit list` output.
    pub description: &'static str,
}

/// Errors emitted while gathering files or reading their contents.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// Filesystem I/O failed while walking the audit root.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Return the static list of every check the auditor knows about.
#[must_use]
pub fn list_checks() -> Vec<CheckInfo> {
    vec![CheckInfo {
        id: "SPDX-001",
        description: "Every .rs file begins with the two-line SPDX header.",
    }]
}

/// Return the rationale and remediation for a single check, if recognised.
#[must_use]
pub fn explain(check_id: &str) -> Option<&'static str> {
    match check_id {
        "SPDX-001" => Some(concat!(
            "SPDX-001 — every .rs file must begin with two specific comment lines:\n",
            "  // SPDX-License-Identifier: GPL-3.0-or-later\n",
            "  // Copyright (C) 2026 Mohamed Hammad\n",
            "\n",
            "Rationale: Steelbore Standard §4 mandates per-file license metadata so\n",
            "downstream tooling (cargo-deny, REUSE, AUR namcap) can prove license\n",
            "consistency without parsing prose. The header is also a quick visual cue\n",
            "for new contributors that the file is GPL-3.0-or-later licensed.\n",
            "\n",
            "Remediation: prepend the two lines above to the offending file. The\n",
            "scripts/ci/check-spdx.sh hook catches this pre-commit; if you see this\n",
            "error in CI, your local hooks aren't installed (`cargo install rusty-hook\n",
            "--locked`).",
        )),
        _ => None,
    }
}

/// Run every registered check against the given root and return all violations.
///
/// # Errors
/// Returns [`AuditError::Io`] if the filesystem walk fails.
pub fn run_all_checks(root: &Path) -> Result<Vec<Violation>, AuditError> {
    check_spdx(root)
}

/// Run `SPDX-001` against every `.rs` file under `root`.
///
/// # Errors
/// Returns [`AuditError::Io`] if a directory cannot be read or a source
/// file cannot be opened.
pub fn check_spdx(root: &Path) -> Result<Vec<Violation>, AuditError> {
    let mut files = Vec::new();
    collect_rs_files(root, &mut files)?;
    let mut violations = Vec::new();
    for path in files {
        let content = std::fs::read_to_string(&path)?;
        let mut iter = content.lines();
        let first = iter.next().unwrap_or("");
        let second = iter.next().unwrap_or("");
        if first != SPDX_LINE_1 || second != SPDX_LINE_2 {
            violations.push(Violation {
                check_id: "SPDX-001",
                path,
                message: "missing or malformed SPDX header".to_owned(),
            });
        }
    }
    Ok(violations)
}

/// Recursively collect every `.rs` file under `root`, skipping `target/` and
/// `.git/` directories.
fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root.is_dir() {
        if root.extension().is_some_and(|e| e == "rs") {
            out.push(root.to_path_buf());
        }
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if let Some(name) = path.file_name() {
                if name == "target" || name == ".git" {
                    continue;
                }
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
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

    fn write_rs(dir: &Path, name: &str, contents: &str) {
        std::fs::create_dir_all(dir).expect("create_dir_all");
        std::fs::write(dir.join(name), contents).expect("write");
    }

    #[test]
    fn spdx_passes_on_canonical_header() {
        let tmp = tempdir();
        write_rs(
            tmp.path(),
            "ok.rs",
            "// SPDX-License-Identifier: GPL-3.0-or-later\n\
             // Copyright (C) 2026 Mohamed Hammad\n\
             \n\
             fn x() {}\n",
        );
        let v = check_spdx(tmp.path()).expect("check_spdx");
        assert!(v.is_empty(), "expected no violations, got {v:?}");
    }

    #[test]
    fn spdx_fails_on_missing_header() {
        let tmp = tempdir();
        write_rs(tmp.path(), "bad.rs", "fn main() {}\n");
        let v = check_spdx(tmp.path()).expect("check_spdx");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].check_id, "SPDX-001");
    }

    #[test]
    fn list_includes_spdx_001() {
        let checks = list_checks();
        assert!(checks.iter().any(|c| c.id == "SPDX-001"));
    }

    #[test]
    fn explain_returns_text_for_known_check() {
        assert!(explain("SPDX-001").is_some());
        assert!(explain("BOGUS").is_none());
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }
}
