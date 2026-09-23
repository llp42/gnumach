// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/pcb.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread-context leaves of `i386/i386/pcb.c`, which
//! `i386/i386/pcb.h` declares.
//!
//! [`stack_detach()`] takes a thread's kernel stack away and hands it
//! to the caller; [`load_context()`] loads a thread's TSS and resumes
//! its saved context, never returning; and [`pcb_collect()`] is the
//! empty collector the scheduler calls.  The rest of `pcb.c` stays C:
//! it allocates the pcb and saves and restores the user state.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::thread::Thread;

/// Detach `thread`'s kernel stack and return it, leaving zero behind.
/// The body of `stack_detach()` in `i386/i386/pcb.c`.
fn detach_stack(thread: &mut Thread) -> VmOffset {
    core::mem::replace(&mut thread.kernel_stack, 0)
}

/// Load `new`'s TSS and switch to its saved context, never returning.
/// The body of `load_context()` in `i386/i386/pcb.c`.
///
/// # Safety
///
/// `new` must point at a live thread whose kernel stack and saved
/// context are ready to resume, and no other CPU may be running it.
unsafe fn switch_to(new: *mut Thread) -> ! {
    // SAFETY: the caller promises a live thread, and `Thread.pcb` is
    // the pointer `pcb_init()` installed.
    let pcb = unsafe { (*new).pcb };
    // SAFETY: `switch_ktss()` is the real C symbol <i386/i386/pcb.h>
    // declares; it reads only the pcb.
    unsafe { glue::switch_ktss(pcb) };
    // SAFETY: the caller promises a resumable thread.  `Load_context`
    // switches to its kernel stack and jumps into its saved context,
    // so the call does not return.
    unsafe { glue::Load_context(new) }
}

/// Detach a kernel stack from a thread and return the old stack.
/// `stack_detach()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must own its
/// `kernel_stack` field for the duration: `stack_free()` calls this
/// at splsched with the thread locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_detach(thread: *mut Thread) -> VmOffset {
    // SAFETY: the caller promises a live thread that no other CPU is
    // detaching from.
    detach_stack(unsafe { &mut *thread })
}

/// Switch to the first thread on a CPU.  `load_context()` of
/// `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// The C declared this `noreturn`; the Rust signature says so, and no
/// caller expects a return.
///
/// # Safety
///
/// `new` must point at a live thread whose kernel stack and saved
/// context are ready to resume, and no other CPU may be running it.
/// The call never returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn load_context(new: *mut Thread) -> ! {
    // SAFETY: the caller's contract.
    unsafe { switch_to(new) }
}

/// Attempt to free excess pcb memory.  `pcb_collect()` of
/// `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// The C body was empty and so is this one; the parameter is unused.
#[unsafe(no_mangle)]
pub extern "C" fn pcb_collect(_thread: *mut Thread) {}
