// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/mp_desc.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Derived from i386/i386/mp_desc.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the descriptor-table module, one adapter per
//! symbol `i386/i386/mp_desc.c` used to define and `i386/i386/mp_desc.h`
//! declares.

use crate::arch::i386::mp_desc;
use core::ffi::{c_int, c_uint};

/// `simple_lock_pause()` of <kern/lock.h>.
#[unsafe(no_mangle)]
pub extern "C" fn simple_lock_pause() {
    mp_desc::simple_lock_pause();
}

/// `cpu_control()` of <i386/mp_desc.h>.
///
/// # Safety
///
/// `info` must be valid for `count` reads.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpu_control(
    cpu: c_int,
    info: *const c_int,
    count: c_uint,
) -> c_int {
    // SAFETY: the caller promises `info` is valid for `count` reads.
    unsafe { mp_desc::cpu_control(cpu, info, count) }
}

/// `interrupt_processor()` of <i386/mp_desc.h>.
#[unsafe(no_mangle)]
pub extern "C" fn interrupt_processor(cpu: c_int) {
    mp_desc::interrupt_processor(cpu);
}

/// `interrupt_stack_alloc()` of <i386/mp_desc.h>.
#[unsafe(no_mangle)]
pub extern "C" fn interrupt_stack_alloc() {
    mp_desc::interrupt_stack_alloc();
}

/// `mp_desc_init()` of <i386/mp_desc.h>.
#[unsafe(no_mangle)]
pub extern "C" fn mp_desc_init(mycpu: c_int) -> c_int {
    mp_desc::mp_desc_init(mycpu)
}

/// `cpu_ap_main()` of <i386/mp_desc.h>, the entry `cpuboot.S` calls.
#[unsafe(no_mangle)]
pub extern "C" fn cpu_ap_main() -> ! {
    mp_desc::cpu_ap_main()
}

/// `start_other_cpus()` of <i386/mp_desc.h>.
#[unsafe(no_mangle)]
pub extern "C" fn start_other_cpus() {
    mp_desc::start_other_cpus();
}
