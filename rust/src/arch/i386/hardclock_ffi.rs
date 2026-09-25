// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/hardclock.c and i386/i386/hardclock.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` export of `i386/i386/hardclock.c`, the `ivect` entry the
//! interrupt trampoline calls.

use crate::arch::i386::hardclock;
use crate::arch::i386::pcb::I386InterruptState;
use core::ffi::{c_char, c_int};

/// `hardclock()` of <i386/hardclock.h>: charge one clock tick to the
/// interrupted context.
///
/// # Safety
///
/// `regs` must point at the interrupt state the entry code pushed, which
/// stays valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hardclock(
    iunit: c_int,
    old_ipl: c_int,
    ret_addr: *const c_char,
    regs: *mut I386InterruptState,
) {
    // SAFETY: the caller promises the live saved state.
    hardclock::hardclock(iunit, old_ipl, ret_addr, unsafe { &*regs });
}
