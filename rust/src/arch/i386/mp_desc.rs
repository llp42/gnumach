// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/mp_desc.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The processor control hook and the lock pause, which
//! `i386/i386/mp_desc.c` used to define and `i386/i386/mp_desc.h` and
//! `kern/lock.h` declare.
//!
//! Both routines stand alone: [`cpu_control`] is the machine-dependent
//! `processor_control` hook, which has no implementation and reports
//! [`KernError::Failure`], and [`simple_lock_pause`] is the spin a lock
//! retry loop takes when it loses an out-of-order acquisition.  The
//! rest of `mp_desc.c` (the per-CPU descriptor tables and the AP
//! bring-up) stays C.

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
