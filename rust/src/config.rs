// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Build-wide constants of the Rust half.

/// The version string this kernel reports.
pub const KERNEL_VERSION: &str = "WIP Mach 2026.1~alpha1";

/// `KERNEL_VERSION_MAX` of <mach/host_info.h>: the bytes a `kernel_version_t`
/// holds.
pub const KERNEL_VERSION_MAX: usize = 512;

/// `NCPUS` of <config.h>: the processor count the build was configured with.
///
/// The ABI gate pins it at 2.  `--enable-ncpus` changes it, and the Rust
/// half then has to carry the same number here.
pub const NCPUS: usize = 2;

/// `NCOM` of <config.h>: the serial-port count the build was configured
/// with; the ABI gate pins it at 2, as it pins `NCPUS`.
pub const NCOM: usize = 2;

/// `NINTR` of <i386/i386/apic.h>: the interrupt lines the APIC build
/// addresses; 64, because both configured builds define `APIC`.
pub const NINTR: usize = 64;
