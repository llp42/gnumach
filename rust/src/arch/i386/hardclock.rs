// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/hardclock.c and i386/i386/hardclock.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The clock interrupt, which `i386/i386/hardclock.c` used to define and
//! `i386/i386/hardclock.h` declares.

use crate::arch::i386::pcb::I386InterruptState;
use crate::glue;
use crate::kern::mach_clock;
use core::ffi::{c_char, c_int, c_long};
use core::ptr;

/// `EFL_VM` of <mach/i386/eflags.h>: the interrupted context is in virtual
/// 8086 mode.
const EFL_VM: c_long = 0x0002_0000;

/// `SPL0` of <i386/ipl.h>: the base interrupt level.
const SPL0: c_int = 0;

/// `hardclock()` of `i386/i386/hardclock.c`: charge one clock tick to the
/// interrupted context.
pub(crate) fn hardclock(
    _iunit: c_int,
    old_ipl: c_int,
    ret_addr: *const c_char,
    regs: &I386InterruptState,
) {
    let interrupted_user = ret_addr == ptr::addr_of!(glue::return_to_iret);
    if interrupted_user {
        let usermode = regs.efl & EFL_VM != 0 || regs.cs & 0x03 != 0;
        mach_clock::interrupt(mach_clock::tick, usermode, old_ipl == SPL0);
    } else {
        mach_clock::interrupt(mach_clock::tick, false, false);
    }
}
