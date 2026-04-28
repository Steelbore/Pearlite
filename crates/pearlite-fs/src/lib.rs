// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Mohamed Hammad

//! Filesystem primitives for Pearlite: hashing, atomic write, ownership.
//!
//! Three operations the engine and adapters share:
//!
//! - [`sha256_file`] — chunked, constant-memory hash of a file.
//! - [`write_etc_atomic`] — atomic write of an `/etc` file preserving
//!   declared mode/owner/group per PRD §7.4.
//! - [`probe_config_files`] — produce a [`ConfigFileInventory`] from a
//!   list of declared [`ConfigEntry`] targets.
//!
//! No `std::process::Command`: chmod and chown go through libc via the
//! `nix` crate per Plan §6.4.

mod atomic;
mod chown;
mod errors;
mod hash;
mod inventory;

pub use atomic::write_etc_atomic;
pub use chown::{name_for_gid, name_for_uid};
pub use errors::FsError;
pub use hash::{sha256_bytes, sha256_dir, sha256_file};
pub use inventory::probe_config_files;

#[doc(no_inline)]
pub use pearlite_schema::{ConfigEntry, ConfigFileInventory, ConfigFileMeta};
