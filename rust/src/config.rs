// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Build-wide constants of the Rust half.
//!
//! The C kernel takes its banner string from `version.c`, which
//! `configure` generates from `version.c.in` and `version.m4`.  This
//! is the Rust half's copy of the same string, so that Rust code
//! names a constant rather than reaching through `glue` for a C
//! global.

/// The version string this kernel reports.
///
/// Calendar versioning, `YYYY.MINOR`: the year, then a counter that
/// restarts at 1 each January.  A prerelease takes a `~alphaN`,
/// `~betaN` or `~rcN` suffix, which dpkg sorts before the release
/// itself.
///
/// The string must never contain a colon.  The Hurd's `proc` server
/// truncates the kernel version there, then splits the rest at the
/// first space into the name and the version `uname` reports.
///
/// `version.m4` holds the same string for the boot banner and changes
/// in the same commit as this one.
pub const KERNEL_VERSION: &str = "WIP Mach 2026.1~alpha1";

/// `KERNEL_VERSION_MAX` of <mach/host_info.h>: the bytes a
/// `kernel_version_t` holds.
pub const KERNEL_VERSION_MAX: usize = 512;
