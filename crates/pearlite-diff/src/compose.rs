// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Top-level [`plan`] composition: classifications → [`Plan`].

use crate::action::{Action, Scope};
use crate::classify::{
    ConfigDriftReason, classify_cargo, classify_config, classify_pacman, classify_services,
};
use crate::plan::{DriftCategory, DriftItem, Plan, Warning};
use pearlite_schema::{DeclaredState, ProbedState};
use pearlite_state::State;
use std::collections::BTreeMap;
use std::path::PathBuf;
use time::OffsetDateTime;
use uuid::Uuid;

/// Compose a [`Plan`] from declared / probed / on-disk state.
///
/// `plan_id` and `generated_at` are caller-supplied so the function
/// stays deterministic given identical inputs — a property test in
/// `tests` exercises this. Production callers pass `Uuid::now_v7()`
/// and `OffsetDateTime::now_utc()`.
///
/// `declared_source_sha256` maps each declared `[[config]].source`
/// path (relative to the user's repo) to its hex-encoded SHA-256. The
/// engine computes these once via `pearlite-fs::sha256_file` and
/// passes them in; this crate is forbidden from doing I/O per Plan §6.3.
#[must_use]
pub fn plan(
    declared: &DeclaredState,
    probed: &ProbedState,
    state: &State,
    declared_source_sha256: &BTreeMap<PathBuf, String>,
    plan_id: Uuid,
    generated_at: OffsetDateTime,
) -> Plan {
    let mut actions = Vec::new();
    let mut drift = Vec::new();
    let warnings: Vec<Warning> = Vec::new();

    // Phase 2 + 3: pacman install / remove + AUR install + drift.
    let pacman_class = if let Some(probed_pacman) = probed.pacman.as_ref() {
        classify_pacman(&declared.packages, &declared.remove, probed_pacman, state)
    } else {
        crate::PacmanClassification::default()
    };
    pacman_actions(&pacman_class, &mut actions);
    pacman_drift(&pacman_class, &mut drift);

    // Phase 2 + 3 (cargo half): install + drift.
    let cargo_class = if let Some(probed_cargo) = probed.cargo.as_ref() {
        classify_cargo(&declared.packages.cargo, probed_cargo, state)
    } else {
        crate::CargoClassification::default()
    };
    cargo_actions(&cargo_class, &mut actions);
    cargo_drift(&cargo_class, &mut drift);

    // Phase 4 + 6: config writes + restarts. Plus drift surfacing for
    // changed-on-disk files.
    let config_class = if let Some(probed_configs) = probed.config_files.as_ref() {
        classify_config(
            &declared.config_files,
            declared_source_sha256,
            probed_configs,
        )
    } else {
        crate::ConfigClassification::default()
    };
    config_actions(declared, &config_class, &mut actions);
    config_drift(&config_class, &mut drift);

    // Phase 5: service state.
    let services_class = if let Some(probed_services) = probed.services.as_ref() {
        classify_services(&declared.services, probed_services)
    } else {
        crate::ServicesClassification::default()
    };
    services_actions(&services_class, &mut actions);
    services_drift(&services_class, &mut drift);

    actions.sort_by_key(Action::within_phase_key);

    Plan {
        plan_id,
        host: declared.host.hostname.clone(),
        generated_at,
        actions,
        drift,
        warnings,
    }
}

fn pacman_actions(class: &crate::PacmanClassification, actions: &mut Vec<Action>) {
    let mut by_repo: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (pkg, repo) in &class.to_install {
        by_repo.entry(repo.clone()).or_default().push(pkg.clone());
    }
    for (repo, packages) in by_repo {
        if repo == "aur" {
            actions.push(Action::AurInstall { packages });
        } else {
            actions.push(Action::PacmanInstall { repo, packages });
        }
    }
}

fn pacman_drift(class: &crate::PacmanClassification, drift: &mut Vec<DriftItem>) {
    for pkg in &class.manual {
        drift.push(DriftItem {
            category: DriftCategory::ManualPackage,
            identifier: pkg.clone(),
            details: "installed out-of-band; pearlite codify / adopt / remove to resolve"
                .to_owned(),
        });
    }
    for pkg in &class.forgotten {
        drift.push(DriftItem {
            category: DriftCategory::ManualPackage,
            identifier: pkg.clone(),
            details: "declared once; pearlite apply --prune would remove it".to_owned(),
        });
    }
}

fn cargo_actions(class: &crate::CargoClassification, actions: &mut Vec<Action>) {
    for crate_name in &class.to_install {
        actions.push(Action::CargoInstall {
            crate_name: crate_name.clone(),
            locked: true,
        });
    }
}

fn cargo_drift(class: &crate::CargoClassification, drift: &mut Vec<DriftItem>) {
    for crate_name in &class.manual {
        drift.push(DriftItem {
            category: DriftCategory::ManualPackage,
            identifier: crate_name.clone(),
            details: "installed out-of-band via cargo".to_owned(),
        });
    }
    for crate_name in &class.forgotten {
        drift.push(DriftItem {
            category: DriftCategory::ManualPackage,
            identifier: crate_name.clone(),
            details: "declared once via cargo; pearlite apply --prune would uninstall".to_owned(),
        });
    }
}

fn config_actions(
    declared: &DeclaredState,
    class: &crate::ConfigClassification,
    actions: &mut Vec<Action>,
) {
    let mut restarts: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for &index in &class.to_apply {
        let Some(entry) = declared.config_files.get(index) else {
            continue;
        };
        let sha = class
            .drift
            .iter()
            .find(|d| d.declaration_index == index)
            .map(|d| d.declared_sha256.clone())
            .unwrap_or_default();
        actions.push(Action::ConfigWrite {
            target: entry.target.clone(),
            source: entry.source.clone(),
            content_sha256: sha,
            mode: entry.mode,
            owner: entry.owner.clone(),
            group: entry.group.clone(),
            declaration_index: index,
        });
        for unit in &entry.restart {
            restarts.insert(unit.clone());
        }
    }
    for unit in restarts {
        actions.push(Action::ServiceRestart { unit });
    }
}

fn config_drift(class: &crate::ConfigClassification, drift: &mut Vec<DriftItem>) {
    for d in &class.drift {
        if matches!(d.reason, ConfigDriftReason::Missing) {
            // Missing is "to apply", not drift in the user-visible sense.
            continue;
        }
        drift.push(DriftItem {
            category: DriftCategory::ConfigFile,
            identifier: d.target.to_string_lossy().into_owned(),
            details: format!(
                "{:?}: declared {}, live {}",
                d.reason,
                short(&d.declared_sha256),
                d.live_sha256.as_deref().map_or("absent", short)
            ),
        });
    }
}

fn services_actions(class: &crate::ServicesClassification, actions: &mut Vec<Action>) {
    for unit in &class.to_mask {
        actions.push(Action::ServiceMask { unit: unit.clone() });
    }
    for unit in &class.to_disable {
        actions.push(Action::ServiceDisable {
            unit: unit.clone(),
            scope: Scope::System,
        });
    }
    for unit in &class.to_enable {
        actions.push(Action::ServiceEnable {
            unit: unit.clone(),
            scope: Scope::System,
        });
    }
}

fn services_drift(class: &crate::ServicesClassification, drift: &mut Vec<DriftItem>) {
    for unit in &class.drift {
        drift.push(DriftItem {
            category: DriftCategory::ServiceState,
            identifier: unit.clone(),
            details: "enabled out-of-band; pearlite plan does not declare it".to_owned(),
        });
    }
}

fn short(sha: &str) -> &str {
    if sha.len() > 12 { &sha[..12] } else { sha }
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
    use pearlite_schema::{
        ArchLevel, CargoInventory, HostInfo, HostMeta, KernelDecl, KernelInfo, PacmanInventory,
        ServiceInventory,
    };
    use pearlite_state::SCHEMA_VERSION;
    use proptest::prelude::*;

    fn fixed_uuid() -> Uuid {
        Uuid::nil()
    }

    fn fixed_time() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_777_000_000).expect("ts")
    }

    fn declared_minimal() -> DeclaredState {
        DeclaredState {
            host: HostMeta {
                hostname: "forge".to_owned(),
                timezone: "UTC".to_owned(),
                arch_level: ArchLevel::V4,
                locale: "en_US.UTF-8".to_owned(),
                keymap: "us".to_owned(),
            },
            kernel: KernelDecl {
                package: "linux-cachyos".to_owned(),
                cmdline: Vec::new(),
                modules: Vec::new(),
                blacklist: Vec::new(),
            },
            packages: pearlite_schema::PackageSet::default(),
            config_files: Vec::new(),
            services: pearlite_schema::ServicesDecl::default(),
            users: Vec::new(),
            remove: pearlite_schema::RemovePolicy::default(),
            snapshots: pearlite_schema::SnapshotPolicy::default(),
        }
    }

    fn probed_minimal() -> ProbedState {
        ProbedState {
            probed_at: fixed_time(),
            host: HostInfo {
                hostname: "forge".to_owned(),
            },
            pacman: Some(PacmanInventory::default()),
            cargo: Some(CargoInventory::default()),
            config_files: Some(pearlite_schema::ConfigFileInventory::default()),
            services: Some(ServiceInventory::default()),
            kernel: KernelInfo::default(),
        }
    }

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

    #[test]
    fn empty_inputs_yield_empty_plan() {
        let p = plan(
            &declared_minimal(),
            &probed_minimal(),
            &empty_state(),
            &BTreeMap::new(),
            fixed_uuid(),
            fixed_time(),
        );
        assert!(p.actions.is_empty());
        assert!(p.drift.is_empty());
        assert!(p.warnings.is_empty());
        assert_eq!(p.host, "forge");
        assert_eq!(p.plan_id, fixed_uuid());
    }

    #[test]
    fn idempotent_when_state_matches_declaration() {
        let mut declared = declared_minimal();
        declared.packages.core = vec!["base".to_owned()];
        let mut probed = probed_minimal();
        probed.pacman = Some(PacmanInventory {
            explicit: ["base".to_owned()].into_iter().collect(),
            ..Default::default()
        });

        let p = plan(
            &declared,
            &probed,
            &empty_state(),
            &BTreeMap::new(),
            fixed_uuid(),
            fixed_time(),
        );
        assert!(p.actions.is_empty(), "no actions when fully in sync");
        assert!(p.drift.is_empty());
    }

    #[test]
    fn pacman_to_install_groups_by_repo() {
        let mut declared = declared_minimal();
        declared.packages.core = vec!["base".to_owned(), "btrfs-progs".to_owned()];
        declared.packages.cachyos_v4 = vec!["firefox".to_owned()];
        declared.packages.aur = vec!["claude-code".to_owned()];

        let p = plan(
            &declared,
            &probed_minimal(),
            &empty_state(),
            &BTreeMap::new(),
            fixed_uuid(),
            fixed_time(),
        );
        // Three install actions, each grouped by repo.
        let installs: Vec<&Action> = p
            .actions
            .iter()
            .filter(|a| matches!(a, Action::PacmanInstall { .. } | Action::AurInstall { .. }))
            .collect();
        assert_eq!(installs.len(), 3);

        // Ordering: core first, then cachyos-v4, then AUR.
        if let Action::PacmanInstall { repo, packages } = installs[0] {
            assert_eq!(repo, "core");
            assert_eq!(packages.len(), 2);
        } else {
            panic!("expected core install first, got {:?}", installs[0]);
        }
        if let Action::PacmanInstall { repo, .. } = installs[1] {
            assert_eq!(repo, "cachyos-v4");
        } else {
            panic!("expected cachyos-v4, got {:?}", installs[1]);
        }
        assert!(matches!(installs[2], Action::AurInstall { .. }));
    }

    #[test]
    fn manual_pacman_packages_surface_as_drift() {
        let probed = ProbedState {
            pacman: Some(PacmanInventory {
                explicit: ["vim".to_owned()].into_iter().collect(),
                ..Default::default()
            }),
            ..probed_minimal()
        };
        let p = plan(
            &declared_minimal(),
            &probed,
            &empty_state(),
            &BTreeMap::new(),
            fixed_uuid(),
            fixed_time(),
        );
        assert_eq!(p.drift.len(), 1);
        assert_eq!(p.drift[0].category, DriftCategory::ManualPackage);
        assert_eq!(p.drift[0].identifier, "vim");
    }

    #[test]
    fn services_drift_surfaces_at_plan_level() {
        let probed = ProbedState {
            services: Some(ServiceInventory {
                enabled: ["nginx.service".to_owned()].into_iter().collect(),
                ..Default::default()
            }),
            ..probed_minimal()
        };
        let p = plan(
            &declared_minimal(),
            &probed,
            &empty_state(),
            &BTreeMap::new(),
            fixed_uuid(),
            fixed_time(),
        );
        assert_eq!(p.drift.len(), 1);
        assert_eq!(p.drift[0].category, DriftCategory::ServiceState);
    }

    #[test]
    fn config_sha256_mismatch_emits_drift_and_action() {
        use pearlite_schema::{ConfigEntry, ConfigFileMeta};

        let mut declared = declared_minimal();
        declared.config_files = vec![ConfigEntry {
            target: PathBuf::from("/etc/hosts"),
            source: PathBuf::from("etc/hosts"),
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            restart: Vec::new(),
        }];

        let mut sha = BTreeMap::new();
        sha.insert(PathBuf::from("etc/hosts"), "abc".to_owned());

        let probed = ProbedState {
            config_files: Some({
                let mut inv = pearlite_schema::ConfigFileInventory::default();
                inv.entries.insert(
                    PathBuf::from("/etc/hosts"),
                    ConfigFileMeta {
                        sha256: "xyz".to_owned(),
                        mode: 0o644,
                        owner: "root".to_owned(),
                        group: "root".to_owned(),
                    },
                );
                inv
            }),
            ..probed_minimal()
        };

        let p = plan(
            &declared,
            &probed,
            &empty_state(),
            &sha,
            fixed_uuid(),
            fixed_time(),
        );
        assert!(
            p.actions
                .iter()
                .any(|a| matches!(a, Action::ConfigWrite { .. })),
            "ConfigWrite must be present"
        );
        assert!(
            p.drift
                .iter()
                .any(|d| d.category == DriftCategory::ConfigFile && d.identifier == "/etc/hosts"),
            "drift must mention the target"
        );
    }

    proptest! {
        /// Plan §6.3 acceptance: `plan(declared, probed, state)` is
        /// deterministic given identical inputs.
        #[test]
        fn plan_is_deterministic_for_identical_inputs(
            pacman_pkgs in proptest::collection::vec("[a-z][a-z0-9-]{0,20}", 0..20),
            cargo_pkgs in proptest::collection::vec("[a-z][a-z0-9-]{0,20}", 0..10),
        ) {
            let mut declared = declared_minimal();
            declared.packages.core = pacman_pkgs.clone();
            declared.packages.cargo = cargo_pkgs.clone();

            let p1 = plan(
                &declared,
                &probed_minimal(),
                &empty_state(),
                &BTreeMap::new(),
                fixed_uuid(),
                fixed_time(),
            );
            let p2 = plan(
                &declared,
                &probed_minimal(),
                &empty_state(),
                &BTreeMap::new(),
                fixed_uuid(),
                fixed_time(),
            );
            prop_assert_eq!(p1, p2);
        }
    }
}
