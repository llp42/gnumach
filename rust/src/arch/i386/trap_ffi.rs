// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/trap.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University
// Derived from i386/i386/trap.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386/trap.c`, which the assembly trap
//! entries and the C debug paths call.

use crate::arch::i386::pcb::I386SavedState;
use crate::arch::i386::trap;
use core::ffi::{c_char, c_int, c_long, c_uint};

/// `trap_name()` of i386/i386/trap.h.
///
/// # Safety
///
/// No preconditions.  The returned pointer is to a static string and must not
/// be written through.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trap_name(trapnum: c_uint) -> *mut c_char {
    trap::trap_name(trapnum).as_ptr().cast_mut()
}

/// `i386_exception()` of i386/i386/trap.h, which never returns.
///
/// # Safety
///
/// Called on the trap or FPU path with nothing locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_exception(
    exc: c_int,
    code: c_int,
    subcode: c_long,
) -> ! {
    // SAFETY: the caller's contract.
    unsafe { trap::i386_exception(exc, code, subcode) }
}

/// `kernel_trap()` of i386/i386/trap.h.
///
/// # Safety
///
/// `regs` must point at the live frame the assembly trap entry built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel_trap(regs: *mut I386SavedState) {
    // SAFETY: the caller's contract.
    unsafe { trap::kernel_trap(&mut *regs) };
}

/// `user_trap()` of i386/i386/trap.h.
///
/// # Safety
///
/// `regs` must point at the live frame the assembly trap entry built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn user_trap(regs: *mut I386SavedState) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { trap::user_trap(&mut *regs) }
}

/// `i386_astintr()` of i386/i386/trap.h.
///
/// # Safety
///
/// Called from the interrupt path with the CPU's AST set by an IPI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_astintr() {
    // SAFETY: the caller's contract.
    unsafe { trap::astintr() };
}

/// `handle_double_fault()` of `i386/i386/trap.c`, called from locore.
///
/// # Safety
///
/// `regs` must point at the live frame the double-fault entry built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn handle_double_fault(regs: *mut I386SavedState) {
    // SAFETY: the caller's contract.
    unsafe { trap::handle_double_fault(&*regs) };
}
