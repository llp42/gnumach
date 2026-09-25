// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/model_dep.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1986 Avadis Tevanian, Jr., Michael Wayne Young.
// Derived from i386/i386at/model_dep.h:
//   Copyright (c) 2013 Free Software Foundation.
// Derived from i386/i386/model_dep.h:
//   Copyright (C) 2008 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the machine-dependent boot module, one adapter per
//! symbol `i386/i386at/model_dep.c` used to define and `i386/i386/model_dep.h`
//! and `i386/i386at/model_dep.h` declare.

use crate::arch::i386::io_req::DevT;
use crate::arch::i386::model_dep;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue;
use crate::vm::types::VmProt;
use core::ffi::c_int;

/// `machine_idle()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn machine_idle(_cpu: c_int) {
    model_dep::machine_idle(_cpu);
}

/// `machine_relax()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn machine_relax() {
    model_dep::machine_relax();
}

/// `timemmap()` of <i386at/model_dep.h>, the `d_mmap` hook of the `/dev/time`
/// device in `i386/i386at/conf.c`.
#[unsafe(no_mangle)]
pub extern "C" fn timemmap(
    _dev: DevT,
    _off: VmOffset,
    prot: c_int,
) -> VmOffset {
    match model_dep::mapped_time_page(VmProt::from_bits(prot)) {
        Some(page) => page,
        None => VmOffset::MAX,
    }
}

/// `inittodr()` of <i386at/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn inittodr() {
    model_dep::inittodr();
}

/// `resettodr()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn resettodr() {
    model_dep::resettodr();
}

/// `init_alloc_aligned()` of <i386at/model_dep.h>.
///
/// # Safety
///
/// `addrp` must be valid for a write; the C wrote the allocated address
/// through it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_alloc_aligned(
    size: VmSize,
    addrp: *mut VmOffset,
) -> c_int {
    let address = model_dep::alloc_aligned(size).unwrap_or(0);
    // SAFETY: the caller promises `addrp` is valid for a write.
    unsafe { *addrp = address };
    c_int::from(address != 0)
}

/// `pmap_grab_page()` of <vm/pmap.h>.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when no page is left, as the C `panic()` did.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_grab_page() -> VmOffset {
    match model_dep::alloc_aligned(PAGE_SIZE) {
        Some(address) => address,
        None => {
            // SAFETY: `Panic` does not return; both pointers are
            // NUL-terminated `c"..."` literals, and the message holds no
            // conversion specifier for the varargs it never receives.
            unsafe {
                glue::Panic(
                    c"i386/i386at/model_dep.c".as_ptr(),
                    line!() as c_int,
                    c"pmap_grab_page".as_ptr(),
                    c"Not enough memory to initialize Mach".as_ptr(),
                )
            }
        }
    }
}

/// `db_halt_cpu()` in i386/i386at/model_dep.c.
#[unsafe(no_mangle)]
pub extern "C" fn db_halt_cpu() -> ! {
    model_dep::halt_all_cpus(0)
}

/// `db_reset_cpu()` in i386/i386at/model_dep.c.
#[unsafe(no_mangle)]
pub extern "C" fn db_reset_cpu() -> ! {
    model_dep::halt_all_cpus(1)
}

/// `machine_init()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn machine_init() {
    model_dep::machine_init();
}

/// `halt_cpu()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn halt_cpu() -> ! {
    model_dep::halt_cpu()
}

/// `halt_all_cpus()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn halt_all_cpus(reboot: c_int) -> ! {
    model_dep::halt_all_cpus(reboot)
}

/// `c_boot_entry()` of <i386/i386/model_dep.h>, the C entry `boothdr.S` calls.
#[unsafe(no_mangle)]
pub extern "C" fn c_boot_entry(bi: VmOffset) {
    model_dep::c_boot_entry(bi);
}

/// `startrtclock()` of <i386/i386/model_dep.h>.
#[unsafe(no_mangle)]
pub extern "C" fn startrtclock() {
    model_dep::startrtclock();
}
