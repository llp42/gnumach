// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/mp_desc.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt-IPI hook, the processor control hook and the lock
//! pause, which `i386/i386/mp_desc.c` used to define and
//! `i386/i386/mp_desc.h` and `kern/lock.h` declare.
//!
//! All three routines stand alone: [`interrupt_processor`] translates
//! a kernel CPU number to its APIC logical destination bit and hands
//! it to the C `smp_pmap_update()`, which sends the TLB-shootdown IPI;
//! [`cpu_control`] is the machine-dependent `processor_control` hook,
//! which has no implementation and reports [`KernError::Failure`]; and
//! [`simple_lock_pause`] is the spin a lock retry loop takes when it
//! loses an out-of-order acquisition.  The rest of `mp_desc.c` (the
//! per-CPU descriptor tables and the AP bring-up) stays C.

use crate::glue;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint};
use core::sync::atomic::{AtomicU32, Ordering};

/// The number of iterations [`simple_lock_pause`] spins, which the C
/// kept in the global `simple_lock_pause_loop`.
///
/// Nothing outside `mp_desc.c` read the global.
const PAUSE_LOOP: u32 = 100;

/// The count [`simple_lock_pause`] adds one to per call, which the C
/// kept in the global `simple_lock_pause_count`.
///
/// Nothing read the global; it is the debug count the C kept.
static PAUSE_COUNT: AtomicU32 = AtomicU32::new(0);

/// The counter the pause loop increments, which the C kept in a
/// function-local `static volatile int`.
///
/// The increment is the delay, not state anyone reads.
static PAUSE_DUMMY: AtomicU32 = AtomicU32::new(0);

/// `APIC_LOGICAL_CPU_GROUPS` in <i386/apic.h>: the logical destination
/// register has only eight mask bits, so it can name eight CPU groups.
const APIC_LOGICAL_CPU_GROUPS: u32 = 8;

/// Wait a bit for a lock another CPU holds in the opposite order,
/// which `kern/lock.h` declares and `i386/i386/mp_desc.c` used to
/// define.
///
/// There is no memory-safety precondition: the counters are private
/// atomics.  `PAUSE_COUNT` is the debug count the C kept and nothing
/// reads it, so its increment's ordering is `Relaxed`.
#[unsafe(no_mangle)]
pub extern "C" fn simple_lock_pause() {
    PAUSE_COUNT.fetch_add(1, Ordering::Relaxed);
    for _ in 0..PAUSE_LOOP {
        // Nothing is published and no one reads `PAUSE_DUMMY`, so the
        // ordering is `Relaxed`; the increment itself is the delay,
        // and a relaxed atomic keeps the spin from being optimized
        // away.
        PAUSE_DUMMY.fetch_add(1, Ordering::Relaxed);
    }
}

/// The machine-dependent processor control hook, which
/// `i386/i386/mp_desc.h` declares and `i386/i386/mp_desc.c` used to
/// define.
///
/// There is no implementation: the hook prints its arguments and
/// reports [`KernError::Failure`], as the C did.
///
/// # Safety
///
/// `info` must be valid for `count` reads.  The hook only prints the
/// pointer, but the C prototype takes it as a buffer of `count`
/// integers and the contract stays with the declaration.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpu_control(
    cpu: c_int,
    info: *const c_int,
    count: c_uint,
) -> c_int {
    // SAFETY: `printf` receives the same three values the C passed.
    // The format string is a literal whose conversions match them:
    // `info` reaches `%p` as a pointer value, and `%p` prints it
    // without dereferencing it.
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
///
/// The callers index machine slots by a small non-negative CPU number,
/// so taking the modulus in the unsigned domain, as
/// `src/arch/i386/ast_check.rs` does, cannot lose a bit and cannot
/// shift by more than the register has.
fn logical_id(cpu: c_int) -> u32 {
    let group = (cpu as u32) % APIC_LOGICAL_CPU_GROUPS;
    1u32 << group
}

/// Interrupt processor `cpu` to make it flush its pmap.  The body of
/// `interrupt_processor()` in `i386/i386/mp_desc.c`.
///
/// The C passes `APIC_LOGICAL_ID(cpu)` to `smp_pmap_update()`; the
/// translation is the macro's `1u << (cpu % 8)`.
#[unsafe(no_mangle)]
pub extern "C" fn interrupt_processor(cpu: c_int) {
    // SAFETY: `smp_pmap_update()` is the real C IPI routine, and
    // `logical_id` is the APIC destination bit the C macro computes.
    unsafe { glue::smp_pmap_update(logical_id(cpu)) };
}
