// SPDX-License-Identifier: CMU-Mach
// Derived from kern/debug.c:
//   Copyright (c) 1993 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The soft debugger and the panic lock, which `kern/debug.c` used to define
//! for `kern/debug.h`.

use crate::glue;
use crate::kern::lock::SimpleLock;
use core::ffi::{c_char, c_int};

/// The `panic_lock` of `kern/debug.c`: a `struct slock_irq` whose `struct
/// slock` is the [`SimpleLock`] the C `simple_lock_irq()` takes.
#[unsafe(export_name = "panic_lock")]
static PANIC_LOCK: SimpleLock = SimpleLock::new();

/// `SoftDebugger()` in C.
///
/// # Safety
///
/// `message` must point at a NUL-terminated string that stays readable for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SoftDebugger(message: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `message`; the two
    // format strings are literals that match their arguments, as the C calls
    // had them.
    unsafe {
        glue::printf(c"Debugger invoked: %s\n".as_ptr(), message);
        glue::printf(c"But no debugger, continuing.\n".as_ptr());
    }
}

/// `Debugger()` in C.
///
/// # Safety
///
/// Never returns: the caller must accept the halt this panic causes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Debugger(_message: *const c_char) {
    // SAFETY: `Panic` does not return; the file, function and message are the
    // C `panic()` macro's.
    unsafe {
        glue::Panic(
            c"kern/debug.c".as_ptr(),
            // The line is this Rust file's; only `c_int` widths can reach
            // `Panic`'s varargs.
            line!() as c_int,
            c"Debugger".as_ptr(),
            c"Debugger invoked, but there isn't one!".as_ptr(),
        )
    }
}

/// `__stack_chk_fail()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    // SAFETY: `Panic` does not return; the file, function and message are the
    // C `panic()` macro's.
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

/// `panic_init()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn panic_init() {
    PANIC_LOCK.init();
}

/// `__stack_chk_guard[]` in C: the canary GCC's stack protector reads.
///
/// The last three bytes are the C initializer's marker; the immutable
/// section is enough because nothing writes the guard.
#[cfg(target_pointer_width = "64")]
#[unsafe(export_name = "__stack_chk_guard")]
static STACK_CHK_GUARD: [u8; 8] = [0, 0, 0, 0, 0, b'\r', b'\n', 0xff];

/// `__stack_chk_guard[]` in C, the i386 image.
#[cfg(target_pointer_width = "32")]
#[unsafe(export_name = "__stack_chk_guard")]
static STACK_CHK_GUARD: [u8; 4] = [0, b'\r', b'\n', 0xff];
