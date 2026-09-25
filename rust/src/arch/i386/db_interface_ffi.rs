// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/db_interface.c:
//   Copyright (c) 1993,1992,1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the debug register module, one adapter per symbol
//! `i386/i386/db_interface.c` used to define and `i386/i386/db_interface.h`
//! declares.

use crate::arch::i386::db_interface;
use crate::arch::i386::pcb::{I386DebugState, Pcb};
use crate::kern::types::KernError;
use core::ffi::c_int;

/// The `kern_return_t` a Rust result stands for.
fn kern_return(result: Result<(), KernError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error.as_u8()),
    }
}

/// `db_load_context()` of <i386/db_interface.h>.
///
/// # Safety
///
/// `pcb` must be the live pcb of a thread about to run on this CPU.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn db_load_context(pcb: *mut Pcb) {
    // SAFETY: the caller's contract.
    unsafe { db_interface::load_context(pcb) };
}

/// `db_get_debug_state()` of <i386/db_interface.h>.
///
/// # Safety
///
/// `pcb` must be live and `state` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn db_get_debug_state(
    pcb: *mut Pcb,
    state: *mut I386DebugState,
) {
    // SAFETY: the caller's contract.
    unsafe { db_interface::get_debug_state(pcb, state) };
}

/// `db_set_debug_state()` of <i386/db_interface.h>.
///
/// # Safety
///
/// `pcb` must be live and `state` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn db_set_debug_state(
    pcb: *mut Pcb,
    state: *const I386DebugState,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { db_interface::set_debug_state(pcb, state) })
}
