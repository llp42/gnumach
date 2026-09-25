// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/smp.c and i386/i386/smp.h:
//   Copyright (C) 2020 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386/smp.c`, which the i386 boot path
//! and the Rust interrupt- and pmap-synchronisation paths call.

use crate::arch::i386::smp;
use core::ffi::{c_int, c_uint, c_ulong};

/// `smp_remote_ast()` of i386/i386/smp.h.
///
/// # Safety
///
/// The local APIC must be initialized, as it is after `smp_init()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smp_remote_ast(logical_id: c_uint) {
    smp::remote_ast(logical_id);
}

/// `smp_pmap_update()` of i386/i386/smp.h.
///
/// # Safety
///
/// The local APIC must be initialized, as it is after `smp_init()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smp_pmap_update(logical_id: c_uint) {
    smp::pmap_update(logical_id);
}

/// `smp_startup_cpus()` of i386/i386/smp.h.
///
/// # Safety
///
/// Called once by the BSP, after the APIC has been probed and the AP boot
/// page claimed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smp_startup_cpus(
    bsp_apic_id: c_uint,
    start_eip: c_ulong,
) -> c_int {
    smp::startup_cpus(bsp_apic_id, start_eip)
}

/// `smp_init()` of i386/i386/smp.h.
///
/// # Safety
///
/// Called once at boot, after ACPI has described the CPUs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smp_init() -> c_int {
    smp::init()
}
