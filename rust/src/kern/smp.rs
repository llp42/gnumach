// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The generic SMP controller, which `kern/smp.c` used to define.

use core::sync::atomic::{AtomicU8, Ordering};

/// The number of CPUs in the machine, always at least one.
static NUM_CPUS: AtomicU8 = AtomicU8::new(1);

/// `smp_set_numcpus()` in C.
///
/// # Panics
///
/// If `numcpus` is zero: the count is always at least one, and zero is not a
/// number of CPUs.
#[unsafe(no_mangle)]
pub extern "C" fn smp_set_numcpus(numcpus: u8) {
    assert!(numcpus != 0, "smp_set_numcpus: zero CPUs");
    NUM_CPUS.store(numcpus, Ordering::Release);
}

/// `smp_get_numcpus()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn smp_get_numcpus() -> u8 {
    NUM_CPUS.load(Ordering::Acquire)
}
