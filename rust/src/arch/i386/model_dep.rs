// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/model_dep.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1986 Avadis Tevanian, Jr., Michael Wayne Young.
// Derived from i386/i386at/model_dep.h:
//   Copyright (c) 2013 Free Software Foundation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The machine-dependent leaves of `i386/i386at/model_dep.c`: the two
//! processor-wait instructions, the `/dev/time` mmap hook, the wall clock, the
//! bootstrap allocator, and the debugger's halt and reboot entry points.

use crate::arch::i386::biosmem;
use crate::arch::i386::io_req::DevT;
use crate::arch::i386::rtc;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_MASK, PAGE_SHIFT, PAGE_SIZE};
use crate::glue;
use crate::glue::time_value::TimeValue64;
use crate::vm::types::VmProt;
use core::arch::asm;
use core::ffi::c_int;

/// Halt the calling CPU until the next interrupt.
fn idle() {
    // SAFETY: `hlt` stops the CPU until an interrupt is delivered.
    unsafe { asm!("hlt", options(nostack, preserves_flags)) };
}

/// Wait a moment for another CPU.
fn relax() {
    // SAFETY: `rep; nop` is a delay of one instruction.
    unsafe { asm!("rep; nop", options(nostack, preserves_flags)) };
}

/// `machine_idle()` of <i386/i386/model_dep.h>, which
/// `i386/i386at/model_dep.c` defined.
#[unsafe(no_mangle)]
pub extern "C" fn machine_idle(_cpu: c_int) {
    idle();
}

/// `machine_relax()` of <i386/i386/model_dep.h>, which
/// `i386/i386at/model_dep.c` defined.
#[unsafe(no_mangle)]
pub extern "C" fn machine_relax() {
    relax();
}

/// The page frame holding the kernel's mapped time value, or `None` when the
/// request asks for write access.
fn mapped_time_page(prot: VmProt) -> Option<VmOffset> {
    if prot.contains(VmProt::WRITE) {
        return None;
    }

    // SAFETY: `mtime` is the global `kern/mach_clock.c` fills with the address
    // of the page it wired at boot.
    let address = unsafe { glue::mtime } as VmOffset;
    // SAFETY: `kernel_pmap` is the kernel's own pmap, so it maps `address`;
    // the C called `pmap_extract` with the same two values.
    let phys = unsafe { glue::pmap_extract(glue::kernel_pmap, address) };
    Some(phys >> PAGE_SHIFT)
}

/// `timemmap()` of <i386at/model_dep.h>, the `d_mmap` hook of the `/dev/time`
/// device in `i386/i386at/conf.c`.
#[unsafe(no_mangle)]
pub extern "C" fn timemmap(
    _dev: DevT,
    _off: VmOffset,
    prot: c_int,
) -> VmOffset {
    match mapped_time_page(VmProt::from_bits(prot)) {
        Some(page) => page,
        None => VmOffset::MAX,
    }
}

/// Set the kernel's wall clock, at high IPL.
fn set_wallclock(seconds: i64) {
    // SAFETY: `splhigh()` is the real asm function <i386/spl.h> declares, and
    // the value it returns is only handed back to `splx()`.
    let s = unsafe { glue::splhigh() };
    // SAFETY: `time` is the wall-clock global `kern/mach_clock.c` defines, and
    // an interrupt cannot see the store half-written at this level; the C took
    // the same level around it.
    unsafe {
        glue::time = TimeValue64 {
            seconds,
            nanoseconds: 0,
        }
    };
    // SAFETY: `s` is the level `splhigh()` returned.
    unsafe { glue::splx(s) };
}

/// `inittodr()` of <i386at/model_dep.h>, which `i386/i386at/model_dep.c`
/// defined.
#[unsafe(no_mangle)]
pub extern "C" fn inittodr() {
    let mut seconds: u64 = 0;
    // SAFETY: `seconds` is a local valid for a write, and `readtodc` leaves it
    // alone when it fails.
    unsafe { rtc::readtodc(&mut seconds) };
    // The C converted the `uint64_t` seconds to the record's `int64_t` field;
    // the cast reinterprets the bits as that conversion does.
    set_wallclock(seconds as i64);
}

/// `resettodr()` of <i386/i386/model_dep.h>, which `i386/i386at/model_dep.c`
/// defined.
#[unsafe(no_mangle)]
pub extern "C" fn resettodr() {
    // SAFETY: `writetodc` takes no argument, and the C passed none.
    unsafe { rtc::writetodc() };
}

/// Allocate `size` bytes of physical memory during bootstrap, page-rounded, or
/// `None` when the bootstrap allocator is out of pages.
fn alloc_aligned(size: VmSize) -> Option<VmOffset> {
    let rounded = size.wrapping_add(PAGE_MASK) & !PAGE_MASK;
    // vm_page_atop(): the page count, whose C parameter is an `unsigned int`,
    // so only the low 32 bits reach the allocator.
    let pages = (rounded >> PAGE_SHIFT) as u32;
    let address = biosmem::bootalloc(pages);
    if address == 0 { None } else { Some(address) }
}

/// `init_alloc_aligned()` of <i386at/model_dep.h>, which
/// `i386/i386at/model_dep.c` defined.
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
    let address = alloc_aligned(size).unwrap_or(0);
    // SAFETY: the caller promises `addrp` is valid for a write.
    unsafe { *addrp = address };
    c_int::from(address != 0)
}

/// `pmap_grab_page()` of <vm/pmap.h>, which `i386/i386at/model_dep.c` defined.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when no page is left, as the C `panic()` did.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_grab_page() -> VmOffset {
    match alloc_aligned(PAGE_SIZE) {
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
    // SAFETY: `halt_all_cpus` is the real C routine in the same file the C
    // entry point lived in, it takes a plain flag, and it never returns.
    unsafe { glue::halt_all_cpus(0) }
}

/// `db_reset_cpu()` in i386/i386at/model_dep.c.
#[unsafe(no_mangle)]
pub extern "C" fn db_reset_cpu() -> ! {
    // SAFETY: as above.
    unsafe { glue::halt_all_cpus(1) }
}
