// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`Engine`] — orchestrator that ties Nickel, the probe, and the diff
//! crate together to produce a [`Plan`].

use crate::errors::EngineError;
use crate::probe::SystemProbe;
use pearlite_diff::Plan;
use pearlite_nickel::NickelEvaluator;
use pearlite_state::State;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use uuid::Uuid;

/// Pearlite's read-only orchestrator at M1.
///
/// Apply, rollback, and reconcile entry points arrive in M2+. The M1
/// surface is one method: [`Engine::plan`].
pub struct Engine {
    nickel: Box<dyn NickelEvaluator>,
    probe: Box<dyn SystemProbe>,
    repo_root: PathBuf,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("repo_root", &self.repo_root)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Construct an [`Engine`] from a Nickel evaluator, a system
    /// probe, and the user's config-repo root.
    #[must_use]
    pub fn new(
        nickel: Box<dyn NickelEvaluator>,
        probe: Box<dyn SystemProbe>,
        repo_root: PathBuf,
    ) -> Self {
        Self {
            nickel,
            probe,
            repo_root,
        }
    }

    /// Path of the user's config-repo root (where declared
    /// `[[config]].source` paths are resolved against).
    #[must_use]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Compute a [`Plan`] without changing any system state.
    ///
    /// Steps:
    ///
    /// 1. Evaluate `host_file` via the Nickel adapter.
    /// 2. Validate the declared state (`pearlite_schema::validate`).
    /// 3. Snapshot the live system via [`SystemProbe::probe`].
    /// 4. Build a `ConfigFileInventory` for declared targets via
    ///    `pearlite_fs::probe_config_files`.
    /// 5. Hash declared `[[config]].source` files via
    ///    `pearlite_fs::sha256_file`.
    /// 6. Compose the plan via `pearlite_diff::plan`.
    ///
    /// `prune` toggles ADR-0011 forgotten-package removal: when
    /// `false` (default), forgotten packages surface as drift only;
    /// when `true`, the plan additionally carries `PacmanRemove` /
    /// `CargoUninstall` actions for them. The drift-threshold guard
    /// that prevents mass-deletion lives at the CLI boundary.
    ///
    /// # Errors
    /// - [`EngineError::Nickel`] — evaluator failed.
    /// - [`EngineError::ContractViolations`] — schema invariants
    ///   broken.
    /// - [`EngineError::Probe`] — adapter or I/O failure during probe.
    /// - [`EngineError::Fs`] — config-source hashing failed.
    pub fn plan(&self, host_file: &Path, state: &State, prune: bool) -> Result<Plan, EngineError> {
        let declared = pearlite_nickel::load_host(host_file, &*self.nickel)?;
        pearlite_schema::validate(&declared).map_err(EngineError::ContractViolations)?;

        let mut probed = self.probe.probe()?;
        probed.config_files = Some(pearlite_fs::probe_config_files(&declared.config_files));

        let declared_source_sha256 = self.compute_source_sha256(&declared.config_files)?;
        let declared_user_env_hash = self.compute_user_env_hashes(&declared.users)?;

        Ok(pearlite_diff::plan(
            &declared,
            &probed,
            state,
            &declared_source_sha256,
            &declared_user_env_hash,
            Uuid::now_v7(),
            OffsetDateTime::now_utc(),
            prune,
        ))
    }

    /// Compute hex-encoded SHA-256 of each declared user's HM
    /// `config_path` (relative to `repo_root`). Users without an
    /// `home_manager` block, or whose `enabled = false`, or whose
    /// resolved path doesn't exist on disk, are silently omitted —
    /// the diff classifier treats a missing entry as "first apply or
    /// recompute defensively", which is the correct behaviour when we
    /// can't prove what the live config looks like.
    fn compute_user_env_hashes(
        &self,
        users: &[pearlite_schema::UserDecl],
    ) -> Result<BTreeMap<String, String>, EngineError> {
        let mut out = BTreeMap::new();
        for user in users {
            let Some(hm) = user.home_manager.as_ref() else {
                continue;
            };
            if !hm.enabled {
                continue;
            }
            let resolved = self.repo_root.join(&hm.config_path);
            if !resolved.exists() {
                continue;
            }
            let digest = pearlite_fs::sha256_dir(&resolved)?;
            out.insert(user.name.clone(), hex_encode(&digest));
        }
        Ok(out)
    }

    fn compute_source_sha256(
        &self,
        entries: &[pearlite_schema::ConfigEntry],
    ) -> Result<BTreeMap<PathBuf, String>, EngineError> {
        let mut out = BTreeMap::new();
        for entry in entries {
            let resolved = self.repo_root.join(&entry.source);
            // Missing source is not an engine error here — the diff
            // engine surfaces it as drift via classify_config. Skip.
            if !resolved.exists() {
                continue;
            }
            let digest = pearlite_fs::sha256_file(&resolved)?;
            out.insert(entry.source.clone(), hex_encode(&digest));
        }
        Ok(out)
    }
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push(hex_nibble(b >> 4));
        s.push(hex_nibble(b & 0x0f));
    }
    s
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => char::from(b'0' + n),
        10..=15 => char::from(b'a' + n - 10),
        _ => unreachable!("nibble fits in 4 bits"),
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
    use crate::mock_probe::MockProbe;
    use pearlite_nickel::MockNickel;
    use pearlite_schema::{
        ArchLevel, CargoInventory, HostInfo, HostMeta, KernelDecl, KernelInfo, PacmanInventory,
        ProbedState, ServiceInventory,
    };
    use pearlite_state::SCHEMA_VERSION;
    use std::collections::BTreeSet;
    use tempfile::TempDir;

    const MINIMAL_HOST: &str = r#"
[meta]
hostname = "forge"
timezone = "UTC"
arch_level = "v4"
locale = "en_US.UTF-8"
keymap = "us"

[kernel]
package = "linux-cachyos"
"#;

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

    fn empty_probed() -> ProbedState {
        ProbedState {
            probed_at: OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts"),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: Some(PacmanInventory::default()),
            cargo: Some(CargoInventory::default()),
            config_files: None,
            services: Some(ServiceInventory::default()),
            kernel: KernelInfo::default(),
        }
    }

    fn make_engine_with(probe: Box<dyn SystemProbe>, host_path: PathBuf) -> Engine {
        let mut nickel = MockNickel::new();
        nickel.seed(host_path, MINIMAL_HOST);
        Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"))
    }

    #[test]
    fn plan_empty_world_yields_empty_actions() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = make_engine_with(probe, host.clone());
        let p = engine.plan(&host, &empty_state(), false).expect("plan");
        assert!(p.actions.is_empty());
        assert!(p.drift.is_empty());
        assert_eq!(p.host, "forge");
    }

    #[test]
    fn plan_surfaces_manual_packages_as_drift() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");

        let mut probed = empty_probed();
        let mut explicit = BTreeSet::new();
        explicit.insert("vim".to_owned());
        probed.pacman = Some(PacmanInventory {
            explicit,
            ..Default::default()
        });
        let probe = Box::new(MockProbe::with_state(probed));

        let engine = make_engine_with(probe, host.clone());
        let p = engine.plan(&host, &empty_state(), false).expect("plan");
        assert_eq!(p.drift.len(), 1);
        assert_eq!(p.drift[0].identifier, "vim");
    }

    #[test]
    fn plan_propagates_nickel_failure_as_engine_error() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        // MockNickel without seed: load_host returns NickelError::MockMissing.
        let nickel = MockNickel::new();
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"));
        let err = engine
            .plan(&host, &empty_state(), false)
            .expect_err("must fail");
        assert!(matches!(err, EngineError::Nickel(_)), "got {err:?}");
    }

    #[test]
    fn plan_validates_declared_state() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let bogus = r#"
[meta]
hostname = "forge"
timezone = "UTC"
arch_level = "v3"
locale = "en_US.UTF-8"
keymap = "us"

[kernel]
package = "linux-cachyos"

[packages]
"cachyos-v4" = ["firefox"]
"#;
        let mut nickel = MockNickel::new();
        nickel.seed(host.clone(), bogus);
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = Engine::new(Box::new(nickel), probe, PathBuf::from("/cfg-repo"));
        let err = engine
            .plan(&host, &empty_state(), false)
            .expect_err("must fail");
        assert!(
            matches!(err, EngineError::ContractViolations(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn plan_id_and_generated_at_are_fresh_each_call() {
        let dir = TempDir::new().expect("tempdir");
        let host = dir.path().join("forge.ncl");
        let probe = Box::new(MockProbe::with_state(empty_probed()));
        let engine = make_engine_with(probe, host.clone());

        let p1 = engine.plan(&host, &empty_state(), false).expect("plan 1");
        let p2 = engine.plan(&host, &empty_state(), false).expect("plan 2");

        assert_ne!(p1.plan_id, p2.plan_id, "plan_id must be fresh per call");
        // generated_at is also fresh; on a fast machine it might happen
        // within the same nanosecond but generally differs. Loosen to
        // "non-decreasing".
        assert!(p2.generated_at >= p1.generated_at);
    }

    #[test]
    fn hex_encode_round_trips_known_input() {
        let bytes = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];
        assert_eq!(
            hex_encode(&bytes),
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
    }

    fn _used(_: HostMeta, _: KernelDecl, _: ArchLevel) {}
}
