// SPDX-License-Identifier: CMU-Mach
// Derived from kern/debug.c:
//   Copyright (c) 1993 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The soft debugger and the panic path, which `kern/debug.c` used to define
//! and `kern/debug.h` declares.

use crate::arch::i386::model_dep;
use crate::arch::i386::percpu::cpu_number;
use crate::glue;
use crate::kern::console::{CStrArg, kprint};
use crate::kern::lock::SimpleLock;
use crate::kern::startup::reboot_on_panic;
use crate::utils::delay::delay;
use core::ffi::c_char;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// The `panic_lock` of `kern/debug.c`: a `struct slock_irq` whose `struct
/// slock` is the [`SimpleLock`] the C `simple_lock_irq()` takes.
#[unsafe(export_name = "panic_lock")]
static PANIC_LOCK: SimpleLock = SimpleLock::new();

/// `panicstr` of `kern/debug.c`, as the non-null marker that says a panic is
/// already being reported; the C stored the message pointer in a `char *`.
static PANIC_TAKEN: AtomicBool = AtomicBool::new(false);

/// `paniccpu` of `kern/debug.c`: the CPU that took the panic.
static PANIC_CPU: AtomicI32 = AtomicI32::new(0);

/// Print `message` as a panic and halt the machine, the body of the C
/// `Panic()`.
pub(crate) fn panic_fmt(
    file: &str,
    line: u32,
    fun: &str,
    message: fmt::Arguments,
) -> ! {
    panic_init();

    // SAFETY: `splhigh()` is the C spl call and returns the level to restore.
    let spl = unsafe { glue::splhigh() };
    PANIC_LOCK.lock();
    if PANIC_TAKEN.load(Ordering::Acquire) {
        if cpu_number() != PANIC_CPU.load(Ordering::Acquire) {
            PANIC_LOCK.unlock();
            // SAFETY: `spl` is the value `splhigh()` returned.
            unsafe { glue::splx(spl) };
            model_dep::halt_cpu();
        }
    } else {
        PANIC_TAKEN.store(true, Ordering::Release);
        PANIC_CPU.store(cpu_number(), Ordering::Release);
    }
    PANIC_LOCK.unlock();
    // SAFETY: `spl` is the value `splhigh()` returned.
    unsafe { glue::splx(spl) };

    kprint!("panic ");
    kprint!("{{cpu{}}} ", PANIC_CPU.load(Ordering::Acquire));
    kprint!("{}:{}: {}: ", file, line, fun);
    crate::kern::console::write_fmt(message);
    kprint!("\n");

    let mut i = 1000;
    while i > 0 {
        delay(1_000_000);
        i -= 1;
    }

    model_dep::halt_all_cpus(reboot_on_panic())
}

/// Report a panic from Rust.
macro_rules! kpanic {
    ($fun:expr, $($arg:tt)*) => {
        $crate::kern::debug::panic_fmt(
            file!(),
            line!(),
            $fun,
            format_args!($($arg)*),
        )
    };
}
pub(crate) use kpanic;

/// `SoftDebugger()` in C.
///
/// # Safety
///
/// `message` must point at a NUL-terminated string that stays readable for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SoftDebugger(message: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `message`.
    let message = unsafe { CStrArg::from_ptr(message) };
    kprint!("Debugger invoked: {}\n", message);
    kprint!("But no debugger, continuing.\n");
}

/// `Debugger()` in C.
///
/// # Safety
///
/// Never returns: the caller must accept the halt this panic causes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Debugger(_message: *const c_char) {
    kpanic!("Debugger", "Debugger invoked, but there isn't one!")
}

/// `__stack_chk_fail()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    kpanic!("__stack_chk_fail", "stack smashing detected")
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
