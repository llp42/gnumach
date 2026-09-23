// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/pcb.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread-context leaves of `i386/i386/pcb.c`, which `i386/i386/pcb.h`
//! declares.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::thread::Thread;

/// Detach `thread`'s kernel stack and return it, leaving zero behind.
fn detach_stack(thread: &mut Thread) -> VmOffset {
    core::mem::replace(&mut thread.kernel_stack, 0)
}

/// Load `new`'s TSS and switch to its saved context, never returning.
///
/// # Safety
///
/// `new` must point at a live thread whose kernel stack and saved context are
/// ready to resume, and no other CPU may be running it.
unsafe fn switch_to(new: *mut Thread) -> ! {
    // SAFETY: the caller promises a live thread, and `Thread.pcb` is the
    // pointer `pcb_init()` installed.
    let pcb = unsafe { (*new).pcb };
    // SAFETY: `switch_ktss()` is the real C symbol <i386/i386/pcb.h> declares;
    // it reads only the pcb.
    unsafe { glue::switch_ktss(pcb) };
    // SAFETY: the caller promises a resumable thread.
    unsafe { glue::Load_context(new) }
}

/// `stack_detach()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must own its
/// `kernel_stack` field for the duration: `stack_free()` calls this at
/// splsched with the thread locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_detach(thread: *mut Thread) -> VmOffset {
    // SAFETY: the caller promises a live thread that no other CPU is detaching
    // from.
    detach_stack(unsafe { &mut *thread })
}

/// `load_context()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `new` must point at a live thread whose kernel stack and saved context are
/// ready to resume, and no other CPU may be running it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn load_context(new: *mut Thread) -> ! {
    // SAFETY: the caller's contract.
    unsafe { switch_to(new) }
}

/// `pcb_collect()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
#[unsafe(no_mangle)]
pub extern "C" fn pcb_collect(_thread: *mut Thread) {}
