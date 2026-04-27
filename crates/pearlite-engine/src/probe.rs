// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! [`SystemProbe`] trait + production [`LiveProbe`] implementation.
//!
//! [`LiveProbe`] fans out across the pacman, cargo, and systemd
//! adapters with `rayon::join` (Plan §5.6: probe-phase parallelism is
//! the one place rayon is used in the engine).

use crate::errors::ProbeError;
use pearlite_cargo::Cargo;
use pearlite_pacman::Pacman;
use pearlite_schema::{HostInfo, KernelInfo, ProbedState};
use pearlite_systemd::Systemd;
use std::path::Path;
use time::OffsetDateTime;

/// Read the live system into a [`ProbedState`].
///
/// Implementations are expected to populate every subsystem field that
/// their adapters can speak to. Per Plan §6.11, `probe()` returns a
/// single value representing **one consistent moment in time** — the
/// `probed_at` field captures that instant.
///
/// `config_files` is left as `None` here; the engine populates it
/// after the declared `[[config]]` array is loaded via Nickel, since
/// it needs the target list to drive `pearlite-fs::probe_config_files`.
pub trait SystemProbe: Send + Sync {
    /// Snapshot the live system state.
    ///
    /// # Errors
    /// Returns the first adapter or I/O failure encountered. The
    /// engine maps this to a Class 2 plan failure (PRD §8.5).
    fn probe(&self) -> Result<ProbedState, ProbeError>;
}

/// Production [`SystemProbe`] backed by live adapter implementations.
pub struct LiveProbe {
    pacman: Box<dyn Pacman>,
    cargo: Box<dyn Cargo>,
    systemd: Box<dyn Systemd>,
}

impl std::fmt::Debug for LiveProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveProbe").finish_non_exhaustive()
    }
}

impl LiveProbe {
    /// Construct a [`LiveProbe`] from three trait-object adapters.
    #[must_use]
    pub fn new(pacman: Box<dyn Pacman>, cargo: Box<dyn Cargo>, systemd: Box<dyn Systemd>) -> Self {
        Self {
            pacman,
            cargo,
            systemd,
        }
    }
}

impl SystemProbe for LiveProbe {
    fn probe(&self) -> Result<ProbedState, ProbeError> {
        // Fan out the three subsystem inventories in parallel. Each
        // call is dominated by subprocess wall-clock; rayon::join is
        // close to free here.
        let pacman_ref = &*self.pacman;
        let cargo_ref = &*self.cargo;
        let systemd_ref = &*self.systemd;

        let (pacman_result, (cargo_result, systemd_result)) = rayon::join(
            || pacman_ref.inventory(),
            || rayon::join(|| cargo_ref.inventory(), || systemd_ref.inventory()),
        );

        let pacman = pacman_result?;
        let cargo = cargo_result?;
        let services = systemd_result?;

        let hostname = read_hostname();

        Ok(ProbedState {
            probed_at: OffsetDateTime::now_utc(),
            host: HostInfo { hostname },
            pacman: Some(pacman),
            cargo: Some(cargo),
            config_files: None,
            services: Some(services),
            kernel: KernelInfo::default(),
        })
    }
}

/// Best-effort hostname read. Empty string if `/etc/hostname` is
/// unreadable; [`crate::Engine::plan`] surfaces this as a warning rather
/// than a hard error.
fn read_hostname() -> String {
    match std::fs::read_to_string(Path::new("/etc/hostname")) {
        Ok(s) => s.trim().to_owned(),
        Err(_) => String::new(),
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
    use pearlite_cargo::MockCargo;
    use pearlite_pacman::MockPacman;
    use pearlite_schema::{CargoInventory, PacmanInventory, ServiceInventory};
    use pearlite_systemd::MockSystemd;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn live_probe_aggregates_three_adapters() {
        let mut pacman_explicit = BTreeSet::new();
        pacman_explicit.insert("base".to_owned());
        let pacman = MockPacman::with_inventory(PacmanInventory {
            explicit: pacman_explicit,
            ..Default::default()
        });

        let mut cargo_crates = BTreeMap::new();
        cargo_crates.insert("zellij".to_owned(), "0.41.2".to_owned());
        let cargo = MockCargo::with_inventory(CargoInventory {
            crates: cargo_crates,
        });

        let mut systemd_enabled = BTreeSet::new();
        systemd_enabled.insert("sshd.service".to_owned());
        let systemd = MockSystemd::with_inventory(ServiceInventory {
            enabled: systemd_enabled,
            ..Default::default()
        });

        let probe = LiveProbe::new(Box::new(pacman), Box::new(cargo), Box::new(systemd));
        let probed = probe.probe().expect("probe");

        assert!(probed.pacman.is_some());
        assert!(probed.cargo.is_some());
        assert!(probed.services.is_some());
        assert!(probed.config_files.is_none(), "config_files filled later");
        assert_eq!(probed.pacman.expect("pacman").explicit.len(), 1);
        assert_eq!(probed.cargo.expect("cargo").crates.len(), 1);
        assert_eq!(probed.services.expect("services").enabled.len(), 1);
    }

    #[test]
    fn live_probe_propagates_pacman_failure() {
        // A LivePacman pointed at a missing binary returns NotInPath;
        // we wrap MockPacman trivially as a stand-in. To simulate
        // failure we use a custom Pacman impl.
        struct FailingPacman;
        impl Pacman for FailingPacman {
            fn inventory(&self) -> Result<PacmanInventory, pearlite_pacman::PacmanError> {
                Err(pearlite_pacman::PacmanError::NotInPath {
                    tool: "pacman",
                    hint: "test",
                })
            }
            fn sync_databases(&self) -> Result<(), pearlite_pacman::PacmanError> {
                Ok(())
            }
            fn install(
                &self,
                _repo: &str,
                _packages: &[&str],
            ) -> Result<(), pearlite_pacman::PacmanError> {
                Ok(())
            }
            fn aur_install(&self, _packages: &[&str]) -> Result<(), pearlite_pacman::PacmanError> {
                Ok(())
            }
            fn remove(&self, _packages: &[&str]) -> Result<(), pearlite_pacman::PacmanError> {
                Ok(())
            }
        }

        let probe = LiveProbe::new(
            Box::new(FailingPacman),
            Box::new(MockCargo::new()),
            Box::new(MockSystemd::new()),
        );
        let err = probe.probe().expect_err("must fail");
        assert!(matches!(err, ProbeError::Pacman(_)), "got {err:?}");
    }
}
