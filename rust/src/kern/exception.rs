// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/exception.h:
//   Copyright (c) 2013 Free Software Foundation.
// Derived from kern/exception.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The no-server exception exit, which `kern/exception.c` used to define for
//! <kern/exception.h>.

use crate::arch::i386::percpu::current_thread;
use crate::glue;
use crate::kern::thread::Thread;
use core::ffi::c_int;

/// `AST_HALT` in <kern/ast.h>: the thread has been asked to halt at a clean
/// point.
const AST_HALT: c_int = 0x1;
/// `AST_TERMINATE` in <kern/ast.h>: the thread is terminating.
const AST_TERMINATE: c_int = 0x2;

/// The `thread_should_halt()` macro of <kern/thread.h>: the thread's pending
/// ASTs include a halt or a termination.
fn should_halt(thread: *const Thread) -> bool {
    // SAFETY: `thread` is the running thread, live for as long as it runs, and
    // `ast` is the one `int` the C macro reads.
    let ast = unsafe { (*thread).ast };
    ast & (AST_HALT | AST_TERMINATE) != 0
}

/// The continuation `thread_halt_self()` resumes a halted thread through,
/// <kern/sched_prim.h>'s `thread_exception_return`.
unsafe extern "C" fn exception_return() {
    // SAFETY: the routine returns to user mode and never comes back.
    unsafe { glue::thread_exception_return() }
}

/// `exception_no_server()` of kern/exception.c.
///
/// # Safety
///
/// The caller must be the exception path with nothing locked and no resources
/// held; `kern/exception.c` is the only caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_no_server() -> ! {
    let thread = current_thread();

    while should_halt(thread) {
        // SAFETY: `thread_exception_return` never returns; it is the
        // continuation the C passed, and `thread_halt_self()` only comes back
        // when the thread is released to halt cleanly.
        unsafe {
            glue::thread_halt_self(Some(exception_return));
        }
    }

    // SAFETY: the running thread is inside a live task.
    let task = unsafe { (*thread).task };
    // SAFETY: the caller holds no locks, as `task_terminate()` needs.
    let _ = unsafe { crate::kern::task::terminate(task.cast()) };

    // SAFETY: as in the loop above; the task of a live thread cannot have gone
    // away under it.
    unsafe {
        glue::thread_halt_self(Some(exception_return));
    }

    // SAFETY: `Panic` does not return; the file, function and message tags are
    // the C `panic()` macro's, and the line is this Rust file's.
    unsafe {
        glue::Panic(
            c"kern/exception.c".as_ptr(),
            line!() as c_int,
            c"exception_no_server".as_ptr(),
            c"terminating the task didn't kill us".as_ptr(),
        )
    }
}
