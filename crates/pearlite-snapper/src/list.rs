// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! `snapper list` output parsing.
//!
//! Pearlite invokes `snapper -c <config> list --columns
//! number,date,description --no-headers` so the format is stable
//! across snapper versions. Each line is `<id> <date-with-space>
//! <description>`. The date occupies fields 1–2 (the local
//! "YYYY-MM-DD HH:MM:SS" form), followed by everything else as the
//! description.

use crate::errors::SnapperError;
use crate::live::SnapshotInfo;
use time::PrimitiveDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

const SNAPPER_DATE_FMT: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

/// Parse `snapper list --columns number,date,description --no-headers`
/// stdout into a vector of [`SnapshotInfo`].
///
/// Lines that fail to parse (truncated, malformed) are skipped — the
/// snapshot ID is the load-bearing field; a malformed timestamp on
/// one row should not prevent the rest of the listing from being
/// returned.
///
/// # Errors
/// This parser does not fail; malformed lines are silently dropped.
/// The function returns `Result` for forward-compat with future
/// stricter parsing modes.
pub fn parse_list(stdout: &str, config: &str) -> Result<Vec<SnapshotInfo>, SnapperError> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut iter = trimmed.split_whitespace();
        let Some(id_str) = iter.next() else { continue };
        let Some(date_part) = iter.next() else {
            continue;
        };
        let Some(time_part) = iter.next() else {
            continue;
        };
        let description: String = iter.collect::<Vec<_>>().join(" ");

        let Ok(id) = id_str.parse::<u64>() else {
            continue;
        };
        let combined = format!("{date_part} {time_part}");
        let Ok(naive) = PrimitiveDateTime::parse(&combined, &SNAPPER_DATE_FMT) else {
            continue;
        };
        let created_at = naive.assume_utc();
        out.push(SnapshotInfo {
            id,
            label: description.clone(),
            created_at,
            config: config.to_owned(),
        });
    }
    Ok(out)
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

    const SNAPPER_LIST: &str = "\
0 2026-04-27 10:00:00 current
1 2026-04-27 10:00:01 pre-pearlite-apply-abc1234
2 2026-04-27 10:00:30 post-pearlite-apply-abc1234
";

    #[test]
    fn parses_known_snapper_output() {
        let snapshots = parse_list(SNAPPER_LIST, "root").expect("parse");
        assert_eq!(snapshots.len(), 3);

        assert_eq!(snapshots[0].id, 0);
        assert_eq!(snapshots[0].label, "current");
        assert_eq!(snapshots[0].config, "root");

        assert_eq!(snapshots[1].id, 1);
        assert_eq!(snapshots[1].label, "pre-pearlite-apply-abc1234");

        assert_eq!(snapshots[2].id, 2);
        assert_eq!(snapshots[2].label, "post-pearlite-apply-abc1234");
    }

    #[test]
    fn empty_output_yields_empty_list() {
        let snapshots = parse_list("", "root").expect("parse");
        assert!(snapshots.is_empty());
    }

    #[test]
    fn malformed_line_is_skipped() {
        let stdout = "\
0 2026-04-27 10:00:00 current
not-a-number 2026-04-27 10:00:01 garbage
2 2026-04-27 10:00:30 post-apply
";
        let snapshots = parse_list(stdout, "root").expect("parse");
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].id, 0);
        assert_eq!(snapshots[1].id, 2);
    }

    #[test]
    fn description_with_spaces_round_trips() {
        let stdout = "1 2026-04-27 10:00:00 a label with spaces here\n";
        let snapshots = parse_list(stdout, "root").expect("parse");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].label, "a label with spaces here");
    }
}
