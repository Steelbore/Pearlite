# Changelog

All notable changes to Pearlite are documented here.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and Pearlite follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html)
from `v1.0.0` onward. Pre-`1.0` releases follow the looser `0.x` convention
where minor bumps may break behaviour.

## [Unreleased]

### Added

- M0 walking-skeleton scaffold:
  - 13-crate Cargo workspace under `crates/` (pure → adapter → integrator).
  - Workspace `Cargo.toml` with resolver `"3"`, edition `2024`, MSRV `1.85`,
    `unsafe_code = "forbid"`, and the full clippy lint set (deny `unwrap_used`
    / `expect_used` / `panic` / `todo` / `dbg_macro` / `print_stdout` /
    `print_stderr`; warn `pedantic`).
  - `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`, `deny.toml`.
  - `pearlite-cli` clap stub: `pearlite --version` and `pearlite --help`.
  - SPDX headers on every `.rs` source file.
  - `LICENSE` (GPL-3.0-or-later canonical text).
  - `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `CHANGELOG.md`.

### Notes

- `Cargo.lock` is checked in for reproducible CI per Plan §4.1.
- This repository's design documents (`PRD.md`, `Plan.md`, the corresponding
  `.docx` and `.pdf`) are excluded from version control via `.gitignore`; they
  remain the authoritative source for behaviour and process.

[Unreleased]: https://github.com/Steelbore/Pearlite/compare/HEAD
