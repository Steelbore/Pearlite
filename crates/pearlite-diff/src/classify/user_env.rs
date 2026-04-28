// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! User-environment (Home Manager) classification: which declared
//! users need a `home-manager switch` on the next apply.
//!
//! Plan §6.3 + PRD §8.2 phase 7. The classifier consumes:
//!
//! - `declared.users[*]` — which users have `home_manager.enabled =
//!   true` and what their `config_path` / `mode` / `channel` are.
//! - `declared_user_env_hash` — a `BTreeMap<user, sha256_hex>`
//!   computed by the engine over each user's `config_path` directory.
//!   Pure-data classifier means the I/O lives in the engine, not
//!   here.
//! - `state.managed.user_env` — last-recorded `(user, generation,
//!   config_hash)` triples. Drift is detected by comparing
//!   `config_hash` with the supplied hash for the same user.
//!
//! Output: a list of users that need `home-manager switch` on the
//! next apply, alongside the declaration fields the engine needs to
//! emit `Action::UserEnvSwitch`.

use pearlite_schema::{HomeManagerMode, UserDecl};
use pearlite_state::State;
use std::collections::BTreeMap;

/// One user that needs a `home-manager switch` action.
///
/// The engine consumes this in `compose.rs::user_env_actions` to
/// emit a `UserEnvSwitch` action carrying these fields verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserToSwitch {
    /// Login name.
    pub user: String,
    /// `<repo_root>/<config_path>` — a relative path the engine
    /// resolves against the user's config repo before passing to
    /// `home-manager`.
    pub config_path: std::path::PathBuf,
    /// HM invocation style.
    pub mode: HomeManagerMode,
    /// Channel / refspec (`release-24.11`, etc.).
    pub channel: String,
}

/// Result of classifying user-env state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserEnvClassification {
    /// Users that need `home-manager switch` on the next apply.
    /// Sorted alphabetically by login name (matches
    /// `Action::within_phase_key` for `UserEnvSwitch`).
    pub to_switch: Vec<UserToSwitch>,
}

/// Classify which declared users need a HM switch.
///
/// A user is added to `to_switch` when **all** of these hold:
///
/// 1. They have a `home_manager` block.
/// 2. That block's `enabled` is `true`.
/// 3. Either:
///    - There's no `[[managed.user_env]]` record for them yet (first
///      apply), or
///    - The record's `config_hash` differs from
///      `declared_user_env_hash[user]` (config drift).
///
/// Users with `home_manager: None` or `enabled: false` are never
/// added; HM is a per-user opt-in, not a default.
#[must_use]
pub fn classify_user_env(
    declared_users: &[UserDecl],
    declared_user_env_hash: &BTreeMap<String, String>,
    state: &State,
) -> UserEnvClassification {
    let mut to_switch = Vec::new();

    for user in declared_users {
        let Some(hm) = user.home_manager.as_ref() else {
            continue;
        };
        if !hm.enabled {
            continue;
        }

        let recorded = state
            .managed
            .user_env
            .iter()
            .find(|r| r.user == user.name)
            .map(|r| r.config_hash.as_str());

        // Truth table:
        // - record absent → first apply, switch.
        // - record present, hash matches declared → idempotent, skip.
        // - record present, hash differs OR declared hash missing →
        //   defensive re-apply.
        let needs_switch = match recorded {
            None => true,
            Some(prev) => declared_user_env_hash
                .get(&user.name)
                .is_none_or(|now| now != prev),
        };

        if needs_switch {
            to_switch.push(UserToSwitch {
                user: user.name.clone(),
                config_path: std::path::PathBuf::from(&hm.config_path),
                mode: hm.mode,
                channel: hm.channel.clone(),
            });
        }
    }

    to_switch.sort_by(|a, b| a.user.cmp(&b.user));

    UserEnvClassification { to_switch }
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
    use pearlite_schema::{HomeManagerDecl, HomeManagerMode};
    use pearlite_state::{SCHEMA_VERSION, UserEnvRecord};
    use std::path::PathBuf;

    fn empty_state() -> State {
        State {
            schema_version: SCHEMA_VERSION,
            host: "forge".to_owned(),
            tool_version: "0.1.0".to_owned(),
            config_dir: PathBuf::from("/cfg"),
            last_apply: None,
            last_modified: None,
            managed: pearlite_state::Managed::default(),
            adopted: pearlite_state::Adopted::default(),
            history: Vec::new(),
            reconciliations: Vec::new(),
            failures: Vec::new(),
            reserved: BTreeMap::new(),
        }
    }

    fn user_with_hm(name: &str, enabled: bool, channel: &str) -> UserDecl {
        UserDecl {
            name: name.to_owned(),
            shell: "/usr/bin/nu".to_owned(),
            groups: vec![],
            home_manager: Some(HomeManagerDecl {
                enabled,
                mode: HomeManagerMode::Standalone,
                config_path: format!("users/{name}"),
                channel: channel.to_owned(),
            }),
        }
    }

    fn user_no_hm(name: &str) -> UserDecl {
        UserDecl {
            name: name.to_owned(),
            shell: "/usr/bin/nu".to_owned(),
            groups: vec![],
            home_manager: None,
        }
    }

    #[test]
    fn user_without_hm_is_skipped() {
        let users = vec![user_no_hm("alice")];
        let c = classify_user_env(&users, &BTreeMap::new(), &empty_state());
        assert!(c.to_switch.is_empty());
    }

    #[test]
    fn user_with_disabled_hm_is_skipped() {
        let users = vec![user_with_hm("alice", false, "release-24.11")];
        let c = classify_user_env(&users, &BTreeMap::new(), &empty_state());
        assert!(c.to_switch.is_empty());
    }

    #[test]
    fn first_apply_emits_switch_even_with_no_hash() {
        let users = vec![user_with_hm("alice", true, "release-24.11")];
        // No hash supplied (engine hasn't computed one yet) and no
        // managed.user_env record → first apply, must switch.
        let c = classify_user_env(&users, &BTreeMap::new(), &empty_state());
        assert_eq!(c.to_switch.len(), 1);
        assert_eq!(c.to_switch[0].user, "alice");
    }

    #[test]
    fn matching_hash_skips_switch() {
        let users = vec![user_with_hm("alice", true, "release-24.11")];
        let mut hashes = BTreeMap::new();
        hashes.insert("alice".to_owned(), "abc123".to_owned());
        let mut state = empty_state();
        state.managed.user_env = vec![UserEnvRecord {
            user: "alice".to_owned(),
            generation: 7,
            config_hash: "abc123".to_owned(),
        }];
        let c = classify_user_env(&users, &hashes, &state);
        assert!(c.to_switch.is_empty(), "matching hash → no switch");
    }

    #[test]
    fn drifted_hash_emits_switch() {
        let users = vec![user_with_hm("alice", true, "release-24.11")];
        let mut hashes = BTreeMap::new();
        hashes.insert("alice".to_owned(), "newhash".to_owned());
        let mut state = empty_state();
        state.managed.user_env = vec![UserEnvRecord {
            user: "alice".to_owned(),
            generation: 7,
            config_hash: "oldhash".to_owned(),
        }];
        let c = classify_user_env(&users, &hashes, &state);
        assert_eq!(c.to_switch.len(), 1);
        assert_eq!(c.to_switch[0].user, "alice");
    }

    #[test]
    fn missing_declared_hash_with_existing_record_emits_switch() {
        // Defensive: if the engine couldn't compute a hash (e.g.
        // config_path missing on disk), we re-apply rather than
        // silently skip. The `home-manager switch` itself will
        // surface the missing-config-path error.
        let users = vec![user_with_hm("alice", true, "release-24.11")];
        let mut state = empty_state();
        state.managed.user_env = vec![UserEnvRecord {
            user: "alice".to_owned(),
            generation: 7,
            config_hash: "oldhash".to_owned(),
        }];
        let c = classify_user_env(&users, &BTreeMap::new(), &state);
        assert_eq!(c.to_switch.len(), 1);
    }

    #[test]
    fn switch_list_is_sorted_by_user() {
        let users = vec![
            user_with_hm("charlie", true, "release-24.11"),
            user_with_hm("alice", true, "release-24.11"),
            user_with_hm("bob", true, "release-24.11"),
        ];
        let c = classify_user_env(&users, &BTreeMap::new(), &empty_state());
        assert_eq!(
            c.to_switch
                .iter()
                .map(|u| u.user.clone())
                .collect::<Vec<_>>(),
            vec!["alice", "bob", "charlie"]
        );
    }

    #[test]
    fn user_to_switch_carries_declared_fields() {
        let users = vec![user_with_hm("alice", true, "release-24.11")];
        let c = classify_user_env(&users, &BTreeMap::new(), &empty_state());
        assert_eq!(c.to_switch[0].user, "alice");
        assert_eq!(c.to_switch[0].config_path, PathBuf::from("users/alice"));
        assert_eq!(c.to_switch[0].mode, HomeManagerMode::Standalone);
        assert_eq!(c.to_switch[0].channel, "release-24.11");
    }
}
