// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Build-wide constants of the Rust half.

/// The version string this kernel reports.
pub const KERNEL_VERSION: &str = "WIP Mach 2026.1~alpha1";

/// `KERNEL_VERSION_MAX` of <mach/host_info.h>: the bytes a `kernel_version_t`
/// holds.
pub const KERNEL_VERSION_MAX: usize = 512;
