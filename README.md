# Pearlite

> Declarative system management for CachyOS.
> *Forged in Steelbore.*

Pearlite is a Rust-implemented declarative system manager for CachyOS. It reads a
Nickel configuration describing one or more hosts and converges the live system
to that declared state by orchestrating `paru`, `cargo`, `systemd`, `snapper`,
and Home Manager.

Pearlite is the Arch-side counterpart to **Lattice** (NixOS). Where Lattice
provides reproducibility via the Nix store, Pearlite provides convergence to
declared state with latest-versions semantics — so CachyOS's `v3`/`v4` binary
repos and the AUR remain first-class.

## Status

`0.1.0-alpha.0` — Milestone 0 (Walking Skeleton). The binary builds, ships, and
packages, but does not yet manage any system state. See `TODO.md` for the live
roadmap; the PRD and Implementation Plan are the authoritative design
documents.

## Bootstrap (target user flow, post-v1.0)

```sh
paru -S pearlite-bin
sudo pearlite init
pearlite scaffold ~/pearlite-config
sudo pearlite reconcile
sudo pearlite reconcile --commit
sudo pearlite plan
sudo pearlite apply
```

Each step is reversible up to `apply`; rollback is explicit via `pearlite
rollback <plan-id>`.

## Building from source

Pearlite pins Rust 1.85 via `rust-toolchain.toml`. A C linker (`cc` /
`gcc`) is required for the binary crates; on systems without one, run cargo
inside an ephemeral shell, e.g. `nix-shell -p gcc --run 'cargo build
--workspace --release'`.

Common workflows:

```sh
just check          # fmt + clippy
just test           # cargo nextest
just compliance     # Steelbore Standard audit
just ci             # all of the above
```

## Architecture

Cargo workspace, 13 crates split pure → adapter → integrator:

| Layer | Crates |
|---|---|
| Pure | `pearlite-schema`, `pearlite-state`, `pearlite-diff`, `pearlite-fs` |
| Adapter | `pearlite-nickel`, `pearlite-pacman`, `pearlite-cargo`, `pearlite-systemd`, `pearlite-snapper`, `pearlite-userenv` |
| Integrator | `pearlite-engine`, `pearlite-cli` |
| Tooling | `pearlite-audit` |

Apply is a deterministic seven-phase pipeline wrapped in pre/post Snapper
snapshots. State is persisted to `/var/lib/pearlite/state.toml` and is the
last file written on a successful apply — there is no resume.

## License

GPL-3.0-or-later. See `LICENSE`.

## Contributing

See `CONTRIBUTING.md`. Agent contributors: read `AGENTS.md` and `CLAUDE.md`
before opening a PR.
