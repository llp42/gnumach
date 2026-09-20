// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The generic SMP controller, which `kern/smp.c` used to define.
//!
//! Just the CPU count the architecture probe reports.  The count is
//! always at least one: a machine running Mach has a CPU, so the state
//! starts at one and the setter treats zero as a forbidden argument.

use core::sync::atomic::{AtomicU8, Ordering};

/// The number of CPUs in the machine, always at least one.
///
/// One is the floor, so the static starts there: before the probe runs,
/// the kernel is on the one CPU that brought it up.  The setter refuses
/// zero, which keeps the invariant at the only place the value changes.
/// The probe writes it once on the boot processor before the APs start,
/// and every CPU reads it afterwards: the release store and acquire
/// load below are that handoff.
static NUM_CPUS: AtomicU8 = AtomicU8::new(1);

/// Record the number of CPUs in the machine.  `smp_set_numcpus()` in C.
///
/// # Panics
///
/// If `numcpus` is zero: the count is always at least one, and zero is
/// not a number of CPUs.
#[unsafe(no_mangle)]
pub extern "C" fn smp_set_numcpus(numcpus: u8) {
    assert!(numcpus != 0, "smp_set_numcpus: zero CPUs");
    NUM_CPUS.store(numcpus, Ordering::Release);
}

/// The number of CPUs in the machine, at least one.
/// `smp_get_numcpus()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn smp_get_numcpus() -> u8 {
    NUM_CPUS.load(Ordering::Acquire)
}
