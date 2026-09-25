// SPDX-License-Identifier: CMU-Mach
// Derived from kern/startup.c and kern/startup.h:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/startup.c`, which the file used to define for
//! <kern/startup.h>.

use crate::kern::startup;
use crate::kern::thread::Thread;

/// `setup_main()` of kern/startup.c.
///
/// # Safety
///
/// Runs once, on the interrupt stack of the boot processor, before any other
/// CPU or thread exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setup_main() {
    // SAFETY: the caller's contract.
    unsafe { startup::setup_main() };
}

/// `start_kernel_threads()` of kern/startup.c.
///
/// # Safety
///
/// Runs once, in the startup thread, before the other CPUs are started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn start_kernel_threads() {
    // SAFETY: the caller's contract.
    unsafe { startup::start_kernel_threads() };
}

/// `cpu_launch_first_thread()` of kern/startup.c, which never returns.
///
/// # Safety
///
/// Runs on a CPU that is taking its first thread, with no thread of its own
/// yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpu_launch_first_thread(th: *mut Thread) -> ! {
    // SAFETY: the caller's contract.
    unsafe { startup::cpu_launch_first_thread(th) }
}
