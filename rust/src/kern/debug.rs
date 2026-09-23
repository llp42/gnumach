// SPDX-License-Identifier: CMU-Mach
// Derived from kern/debug.c:
//   Copyright (c) 1993 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The soft debugger and the panic lock, which `kern/debug.c` used to
//! define for `kern/debug.h`.
//!
//! [`SoftDebugger`] prints the message and continues, [`Debugger`]
//! panics because there is no debugger to enter, [`__stack_chk_fail`]
//! is the stack protector's halt, and [`panic_init`] initializes
//! [`PANIC_LOCK`]: the `struct slock_irq` the C `Panic()` still takes
//! with `simple_lock_irq()`.
//!
//! `Panic`, `log`, `do_cnputc`, `panicstr`, `paniccpu` and
//! `__stack_chk_guard` stay C in `kern/debug.c`; `Panic` and `log` are
//! variadic, so they cannot move.

use crate::glue;
use crate::kern::lock::SimpleLock;
use core::ffi::{c_char, c_int};

/// The `panic_lock` of `kern/debug.c`: a `struct slock_irq` whose
/// `struct slock` is the [`SimpleLock`] the C `simple_lock_irq()`
/// takes.
///
/// The C side now carries only the declaration, and both the C
/// `Panic()` and the Rust [`panic_init`] name this one object.
#[unsafe(export_name = "panic_lock")]
static PANIC_LOCK: SimpleLock = SimpleLock::new();

/// Print `message` and continue without a debugger.
/// `SoftDebugger()` in C.
///
/// # Safety
///
/// `message` must point at a NUL-terminated string that stays readable
/// for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SoftDebugger(message: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `message`;
    // the two format strings are literals that match their arguments,
    // as the C calls had them.
    unsafe {
        glue::printf(c"Debugger invoked: %s\n".as_ptr(), message);
        glue::printf(c"But no debugger, continuing.\n".as_ptr());
    }
}

/// Panic because there is no debugger to enter.  `Debugger()` in C.
///
/// The argument is unused, as in the C: the message is the fixed one
/// below.
///
/// # Safety
///
/// Never returns: the caller must accept the halt this panic causes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Debugger(_message: *const c_char) {
    // SAFETY: `Panic` does not return; the file, function and message
    // are the C `panic()` macro's.
    unsafe {
        glue::Panic(
            c"kern/debug.c".as_ptr(),
            // The line is this Rust file's; only `c_int` widths can
            // reach `Panic`'s varargs.
            line!() as c_int,
            c"Debugger".as_ptr(),
            c"Debugger invoked, but there isn't one!".as_ptr(),
        )
    }
}

/// Halt because a stack canary did not survive a function body.
/// `__stack_chk_fail()` in C.
///
/// The name is the compiler's, not ours: GCC emits the call from the
/// epilogue of every `-fstack-protector` function in the C half, so it
/// can neither be renamed nor be given arguments.  Nothing calls it
/// from Rust; this crate is built without a stack protector, so the
/// halt below cannot re-enter itself.
#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    // SAFETY: `Panic` does not return; the file, function and message
    // are the C `panic()` macro's.
    unsafe {
        glue::Panic(
            c"kern/debug.c".as_ptr(),
            // The line is this Rust file's, as in `Debugger` above.
            line!() as c_int,
            c"__stack_chk_fail".as_ptr(),
            c"stack smashing detected".as_ptr(),
        )
    }
}

/// Initialize the panic lock.  `panic_init()` in C.
///
/// Called repeatedly: the C `Panic()` starts with this call, and
/// `kern/startup.c` calls it before multiple processors start, so the
/// reset must stay idempotent.  There is no memory-safety
/// precondition: the lock word is atomic, and a reset racing a holder
/// is the C behaviour on the halt path.
#[unsafe(no_mangle)]
pub extern "C" fn panic_init() {
    PANIC_LOCK.init();
}
