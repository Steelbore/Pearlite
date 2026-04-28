// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`HomeManagerBackend`] trait + production [`LiveHmBackend`].

use crate::errors::UserenvError;
use pearlite_schema::HomeManagerMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of a successful `home-manager switch`.
///
/// `generation` is the new HM profile generation parsed from the
/// switch output; the engine carries this in `state.toml`'s
/// `[[managed.user_env]]` so the next `pearlite plan` can detect
/// drift via a generation-pointer comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserEnvOutcome {
    /// Login name the switch ran for.
    pub user: String,
    /// New Home Manager profile generation number.
    pub generation: u64,
}

/// Trait the engine consumes to drive Home Manager.
///
/// One operation for now: `switch`. The probe path
/// (`current_generation`) lands when phase-7 wiring needs it (M3 W2).
/// We deliberately keep the surface tight per CLAUDE.md
/// "Trait-first discipline" — adding a method has a cost; methods
/// without an immediate consumer don't pay for themselves.
pub trait HomeManagerBackend: Send + Sync {
    /// Run `home-manager switch` for `user` against `config_path`,
    /// dropping privileges via `runuser`.
    ///
    /// `mode` selects classic (`Standalone`) vs flake-based (`Flake`)
    /// invocation; `channel` is e.g. `release-24.11` for standalone or
    /// the flake ref for flake mode.
    ///
    /// # Errors
    /// Returns [`UserenvError`] on spawn / non-zero exit / parse
    /// failure.
    fn switch(
        &self,
        user: &str,
        config_path: &Path,
        mode: HomeManagerMode,
        channel: &str,
    ) -> Result<UserEnvOutcome, UserenvError>;
}

/// Production [`HomeManagerBackend`] backed by `runuser` + the
/// `home-manager` binary.
///
/// CLAUDE.md hard invariant 5: subprocess invocations use
/// [`std::process::Command`] with argv arrays — never `sh -c`.
#[derive(Clone, Debug)]
pub struct LiveHmBackend {
    runuser: PathBuf,
    home_manager: PathBuf,
}

impl LiveHmBackend {
    /// Construct a [`LiveHmBackend`] resolving `runuser` and
    /// `home-manager` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runuser: PathBuf::from("runuser"),
            home_manager: PathBuf::from("home-manager"),
        }
    }

    /// Construct with caller-supplied binary paths (test / FHS quirks).
    pub fn with_binaries(runuser: impl Into<PathBuf>, home_manager: impl Into<PathBuf>) -> Self {
        Self {
            runuser: runuser.into(),
            home_manager: home_manager.into(),
        }
    }

    /// Path of the `runuser` binary this adapter invokes.
    #[must_use]
    pub fn runuser(&self) -> &Path {
        &self.runuser
    }

    /// Path of the `home-manager` binary this adapter delegates to via
    /// `runuser`.
    #[must_use]
    pub fn home_manager(&self) -> &Path {
        &self.home_manager
    }
}

impl Default for LiveHmBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl HomeManagerBackend for LiveHmBackend {
    fn switch(
        &self,
        user: &str,
        config_path: &Path,
        mode: HomeManagerMode,
        channel: &str,
    ) -> Result<UserEnvOutcome, UserenvError> {
        // Build:
        //   runuser -u <user> -- <home-manager> switch [-f <config_path> | --flake <config_path>#<channel>]
        let hm = self.home_manager.to_string_lossy().into_owned();
        let cfg = config_path.to_string_lossy().into_owned();
        let mut args: Vec<String> = vec![
            "-u".to_owned(),
            user.to_owned(),
            "--".to_owned(),
            hm,
            "switch".to_owned(),
        ];
        match mode {
            HomeManagerMode::Standalone => {
                args.push("-f".to_owned());
                args.push(cfg);
                if !channel.is_empty() {
                    // Standalone HM picks the channel from the user's
                    // nix-channel list; we set NIX_PATH inline so a
                    // missing channel becomes a clear error rather
                    // than a silent default.
                    args.push("-I".to_owned());
                    args.push(format!("home-manager={channel}"));
                }
            }
            HomeManagerMode::Flake => {
                args.push("--flake".to_owned());
                args.push(format!("{cfg}#{channel}"));
            }
        }

        let output = match Command::new(&self.runuser).args(&args).output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(UserenvError::NotInPath {
                    hint: "paru -S util-linux home-manager",
                });
            }
            Err(e) => return Err(UserenvError::Io(e)),
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(UserenvError::InvocationFailed { code, stderr });
        }

        let stdout = String::from_utf8(output.stdout)?;
        let generation = parse_generation_from_switch(&stdout).ok_or_else(|| {
            UserenvError::ParseFailed(format!(
                "no `Activating configuration` line with generation number; full output: {stdout:?}"
            ))
        })?;

        Ok(UserEnvOutcome {
            user: user.to_owned(),
            generation,
        })
    }
}

/// Parse the generation number out of `home-manager switch` stdout.
///
/// Recent Home Manager prints lines like:
///
/// ```text
/// Activating configuration...
/// Starting home manager activation
/// ...
/// home-manager generation 42 created.
/// ```
///
/// We scan for the substring `generation N` and take the first integer
/// that follows. Returns `None` when the format changes; the caller
/// converts that into [`UserenvError::ParseFailed`].
#[must_use]
pub fn parse_generation_from_switch(stdout: &str) -> Option<u64> {
    for line in stdout.lines() {
        if let Some(rest) = line
            .find("generation ")
            .map(|i| &line[i + "generation ".len()..])
        {
            let num: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !num.is_empty() {
                if let Ok(n) = num.parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
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
    fn runuser_not_in_path_error_class() {
        let backend = LiveHmBackend::with_binaries(
            "/nonexistent/runuser-binary-12345",
            "/nonexistent/home-manager-binary-12345",
        );
        let err = backend
            .switch(
                "alice",
                Path::new("/cfg"),
                HomeManagerMode::Standalone,
                "release-24.11",
            )
            .expect_err("must fail");
        assert!(matches!(err, UserenvError::NotInPath { .. }), "got {err:?}");
    }

    #[test]
    fn parse_generation_finds_first_match() {
        let stdout = "\
Activating configuration...
Starting home manager activation
home-manager generation 7 created.
Done.
";
        assert_eq!(parse_generation_from_switch(stdout), Some(7));
    }

    #[test]
    fn parse_generation_handles_large_numbers() {
        assert_eq!(
            parse_generation_from_switch("home-manager generation 1234567 created."),
            Some(1_234_567)
        );
    }

    #[test]
    fn parse_generation_returns_none_when_absent() {
        assert_eq!(parse_generation_from_switch("Done."), None);
        assert_eq!(parse_generation_from_switch(""), None);
    }
}
