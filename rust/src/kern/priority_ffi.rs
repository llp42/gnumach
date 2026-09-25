// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/priority.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/priority.h:
//   Copyright (c) 2013 Free Software Foundation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entry `kern/priority.c` used to define.

use crate::kern::priority;
use crate::kern::thread::Thread;
use core::ffi::c_int;

/// `thread_quantum_update()` of kern/priority.c.
///
/// # Safety
///
/// `mycpu` must be the running CPU and `thread` its live interrupted thread
/// or the thread the clock charged; the caller must hold no lock the thread
/// or processor-set lock would nest under.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_quantum_update(
    mycpu: c_int,
    thread: *mut Thread,
    nticks: c_int,
    state: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { priority::thread_quantum_update(mycpu, thread, nticks, state) };
}
