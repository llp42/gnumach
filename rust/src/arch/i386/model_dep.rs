// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/model_dep.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1986 Avadis Tevanian, Jr., Michael Wayne Young.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The debugger's halt and reboot entry points, which
//! `i386/i386at/model_dep.c` used to define.
//!
//! Both are one call to `halt_all_cpus()`, which stays C in
//! `model_dep.c` and is declared in `i386/i386/model_dep.h`.  Neither
//! of the two has a prototype in any header and neither has a caller
//! in the tree: they exist as names the kernel debugger reaches for,
//! which is why they are exported and why the C had them at all.
//!
//! The rest of `model_dep.c` stays C: it parses the boot information,
//! sizes physical memory and sets up the descriptor tables.

use crate::glue;

/// Halt every CPU without rebooting.  `db_halt_cpu()` in
/// i386/i386at/model_dep.c.
///
/// The C declared this `void`; it never returns, because
/// `halt_all_cpus()` does not, so the Rust signature diverges.  No C
/// header declares it, so no caller sees the difference.
#[unsafe(no_mangle)]
pub extern "C" fn db_halt_cpu() -> ! {
    // SAFETY: `halt_all_cpus` is the real C routine in the same file
    // the C entry point lived in, it takes a plain flag, and it never
    // returns.  Zero is the C's literal argument: halt, do not reboot.
    unsafe { glue::halt_all_cpus(0) }
}

/// Reboot the machine.  `db_reset_cpu()` in i386/i386at/model_dep.c.
///
/// The C declared this `void`; it never returns, for the same reason
/// [`db_halt_cpu()`] does not.
#[unsafe(no_mangle)]
pub extern "C" fn db_reset_cpu() -> ! {
    // SAFETY: as above.  One is the C's literal argument: reboot.
    unsafe { glue::halt_all_cpus(1) }
}
