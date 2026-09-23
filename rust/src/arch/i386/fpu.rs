// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/fpu.c:
//   Copyright (c) 1992-1990 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The FPU save-area free, which `i386/i386/fpu.c` used to define and
//! `i386/i386/fpu.h` declares.
//!
//! The rest of `fpu.c` stays C: the save and restore paths are inline
//! assembly, and the traps and per-CPU data couple the file to the
//! interrupt layer.  [`fp_free()`] is the one routine with no such
//! dependency, so it is the one that moved.  It returns an object to
//! the `ifps_cache` that the module's own `fpu_module_init()` builds.

use crate::glue;
use core::ffi::c_void;

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
