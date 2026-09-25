// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/debug_i386.c and i386/i386/debug.h:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the debug module, one adapter per symbol
//! `i386/i386/debug_i386.c` used to define and `i386/i386/debug.h` declares.

use crate::arch::i386::debug_i386;
use crate::arch::i386::pcb::I386SavedState;
use core::ffi::c_int;

/// `dump_ss()` of <i386/debug.h>.
///
/// # Safety
///
/// `st` must point at a live `I386SavedState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dump_ss(st: *const I386SavedState) {
    // SAFETY: the caller promises `st` is live.
    unsafe { debug_i386::dump_ss(st) };
}

/// `debug_trace_reset()` of <i386/debug.h>.
#[unsafe(no_mangle)]
pub extern "C" fn debug_trace_reset() {
    debug_i386::debug_trace_reset();
}

/// `debug_trace_dump()` of <i386/debug.h>.
#[unsafe(no_mangle)]
pub extern "C" fn debug_trace_dump() {
    debug_i386::debug_trace_dump();
}

/// `syscall_trace_print()` of `i386/i386/debug_i386.c`.
///
/// # Safety
///
/// `locore.S` calls this with the syscall number and the argument list the
/// syscall passed, which must hold at least the trap's own
/// `mach_trap_arg_count` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_trace_print(
    syscallvec: c_int,
    mut args: ...
) -> c_int {
    debug_i386::trace_print(syscallvec, &mut args)
}
