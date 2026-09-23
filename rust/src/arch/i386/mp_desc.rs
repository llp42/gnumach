// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/mp_desc.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt-IPI hook, the processor control hook and the lock pause,
//! which `i386/i386/mp_desc.c` used to define and `i386/i386/mp_desc.h` and
//! `kern/lock.h` declare.

use crate::glue;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint};
use core::sync::atomic::{AtomicU32, Ordering};

/// The number of iterations [`simple_lock_pause`] spins, which the C kept in
/// the global `simple_lock_pause_loop`.
const PAUSE_LOOP: u32 = 100;

/// The count [`simple_lock_pause`] adds one to per call, which the C kept in
/// the global `simple_lock_pause_count`.
static PAUSE_COUNT: AtomicU32 = AtomicU32::new(0);

/// The counter the pause loop increments, which the C kept in a function-local
/// `static volatile int`.
static PAUSE_DUMMY: AtomicU32 = AtomicU32::new(0);

/// `APIC_LOGICAL_CPU_GROUPS` in <i386/apic.h>: the logical destination
/// register has only eight mask bits, so it can name eight CPU groups.
const APIC_LOGICAL_CPU_GROUPS: u32 = 8;

/// Wait a bit for a lock another CPU holds in the opposite order, which
/// `kern/lock.h` declares and `i386/i386/mp_desc.c` used to define.
#[unsafe(no_mangle)]
pub extern "C" fn simple_lock_pause() {
    PAUSE_COUNT.fetch_add(1, Ordering::Relaxed);
    for _ in 0..PAUSE_LOOP {
        // Nothing is published and no one reads `PAUSE_DUMMY`, so the ordering
        // is `Relaxed`; the increment itself is the delay, and a relaxed
        // atomic keeps the spin from being optimized away.
        PAUSE_DUMMY.fetch_add(1, Ordering::Relaxed);
    }
}

/// The machine-dependent processor control hook, which `i386/i386/mp_desc.h`
/// declares and `i386/i386/mp_desc.c` used to define.
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
    // SAFETY: `printf` receives the same three values the C passed.
    unsafe {
        glue::printf(
            c"cpu_control(%d, %p, %d) not implemented\n".as_ptr(),
            cpu,
            info,
            count,
        );
    }
    c_int::from(KernError::Failure)
}

/// The logical destination bit `APIC_LOGICAL_ID(cpu)` in <i386/apic.h>
/// computes, `1u << ((cpu) % APIC_LOGICAL_CPU_GROUPS)`.
fn logical_id(cpu: c_int) -> u32 {
    let group = (cpu as u32) % APIC_LOGICAL_CPU_GROUPS;
    1u32 << group
}

/// Interrupt processor `cpu` to make it flush its pmap.
#[unsafe(no_mangle)]
pub extern "C" fn interrupt_processor(cpu: c_int) {
    // SAFETY: `smp_pmap_update()` is the real C IPI routine, and `logical_id`
    // is the APIC destination bit the C macro computes.
    unsafe { glue::smp_pmap_update(logical_id(cpu)) };
}
