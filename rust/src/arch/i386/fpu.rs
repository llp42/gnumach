// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/fpu.c:
//   Copyright (c) 1992-1990 Carnegie Mellon University
//   Copyright (C) 1994 Linus Torvalds
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The FPU save-area free and the coprocessor-not-present trap, which
//! `i386/i386/fpu.c` used to define and `i386/i386/fpu.h` declares.
//!
//! The rest of `fpu.c` stays C: the save and restore paths are inline
//! assembly, and the traps and per-CPU data couple the file to the
//! interrupt layer.  [`fp_free()`] returns an object to the
//! `ifps_cache` that the module's own `fpu_module_init()` builds, and
//! [`fpnoextflt()`] clears the task-switched flag and reloads the
//! running thread's state through the C `fp_load()`.

use crate::arch::i386::percpu::current_thread;
use crate::glue;
use core::arch::asm;
use core::ffi::c_void;

/// Clear CR0.TS.  `clear_ts()` in <i386/proc_reg.h>, which the C
/// spelled `asm volatile("clts")`; leaving `nomem` off the options
/// keeps the memory clobber that volatile asm carried.
fn clear_ts() {
    // SAFETY: `clts` is the CPU's own instruction and is valid at
    // CPL0.  It writes no memory and leaves the stack and the flags
    // alone.
    unsafe { asm!("clts", options(nostack, preserves_flags)) };
}

/// Coprocessor not present: enable FPU use and load the running
/// thread's state.  The body of `fpnoextflt()` in `i386/i386/fpu.c`.
///
/// The C `ASSERT_IPL(SPL0)` expands to nothing in this build
/// (`i386/i386/fpu.c:59-70`), so it has no Rust equivalent.
#[unsafe(no_mangle)]
pub extern "C" fn fpnoextflt() {
    clear_ts();
    // SAFETY: the trap runs on the current thread, which is the thread
    // `fp_load()` wants; `fpu_module_init()` builds `ifps_cache`
    // before any thread can reach this trap.
    unsafe { glue::fp_load(current_thread()) };
}

/// Free an FPU save area.  `fp_free()` in C.
///
/// The C calls this only when a thread terminates, which is why no
/// locking is needed and there is none here.
///
/// The C `ASSERT_IPL(SPL0)` in front of the free expands to nothing in
/// this build (`i386/i386/fpu.c:59-70`), so it has no Rust
/// equivalent.
///
/// # Safety
///
/// `fps` is an object address passed straight to the slab free and is
/// never dereferenced here.  It must be a live object allocated from
/// [`glue::ifps_cache`], nothing may use it afterwards, and the caller
/// must be tearing the owning thread down.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fp_free(fps: *mut c_void) {
    let cache = &raw mut glue::ifps_cache;
    // SAFETY: the caller promises `fps` is a live object from the
    // cache, and `fpu_module_init()` built `ifps_cache` before any
    // thread could reach this free.
    unsafe { glue::kmem_cache_free(cache, fps.addr()) };
}
