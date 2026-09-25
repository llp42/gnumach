// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/user_ldt.c:
//   Copyright (c) 1994,1993,1992,1991 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the user LDT and GDT entries, one adapter per
//! symbol `i386/i386/user_ldt.c` used to define and `i386/i386/user_ldt.h`
//! and the MIG `mach_i386` interface declare.

use crate::arch::i386::pcb::RealDescriptor;
use crate::arch::i386::user_ldt::{self, Descriptor};
use crate::kern::thread::Thread;
use core::ffi::{c_int, c_uint};
use core::slice;

/// The `kern_return_t` a Rust result stands for.
fn kern_return(result: Result<(), user_ldt::Error>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => error.as_kern_return(),
    }
}

/// `i386_set_ldt()` of the MIG `mach_i386` interface.
///
/// # Safety
///
/// `thread` must be null or a live thread; `descriptor_list` must point at
/// `count` writable descriptors when `desc_list_inline` is true, and at a
/// live `vm_map_copy` the caller owns when it is false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_set_ldt(
    thread: *mut Thread,
    first_selector: c_int,
    descriptor_list: *const Descriptor,
    count: c_uint,
    desc_list_inline: c_int,
) -> c_int {
    let descriptors = descriptor_list.cast::<RealDescriptor>().cast_mut();
    // SAFETY: the caller's contract makes the list writable, as the C
    // `i386_set_ldt()` mutates a kernel call gate in place.
    kern_return(unsafe {
        user_ldt::set_ldt(
            thread,
            first_selector,
            descriptors,
            count,
            desc_list_inline != 0,
        )
    })
}

/// `i386_get_ldt()` of the MIG `mach_i386` interface.
///
/// # Safety
///
/// `thread` must be null or a live thread, and `descriptor_list`/`count` must
/// be valid for a read and a write; `*descriptor_list` must be writable for
/// `*count` descriptors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_get_ldt(
    thread: *mut Thread,
    first_selector: c_int,
    selector_count: c_int,
    descriptor_list: *mut *mut Descriptor,
    count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller promises the two out-pointers are readable.
    let capacity = unsafe { *count };
    // SAFETY: the caller promises the list holds `capacity` writable
    // descriptors.
    let out = if capacity == 0 {
        None
    } else {
        Some(unsafe {
            slice::from_raw_parts_mut(
                (*descriptor_list).cast::<RealDescriptor>(),
                capacity as usize,
            )
        })
    };

    // SAFETY: the caller's contract covers the thread and the list.
    match unsafe {
        user_ldt::get_ldt(thread, first_selector, selector_count, out)
    } {
        Ok((new_count, copy)) => {
            // SAFETY: the caller promises both out-pointers are writable.
            unsafe {
                *count = new_count;
                if let Some(copy) = copy {
                    *descriptor_list = copy.as_ptr().cast::<Descriptor>();
                }
            }
            0
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `user_ldt_free()` of <i386/user_ldt.h>.
///
/// # Safety
///
/// `user_ldt` must be a live LDT allocation, given up by this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn user_ldt_free(
    user_ldt: *mut crate::arch::i386::pcb::UserLdt,
) {
    // SAFETY: the caller's contract.
    unsafe { user_ldt::free(user_ldt) };
}

/// `i386_set_gdt()` of the MIG `mach_i386` interface.
///
/// # Safety
///
/// `thread` must be null or a live thread; `selector` must be valid for a
/// read and a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_set_gdt(
    thread: *mut Thread,
    selector: *mut c_int,
    descriptor: Descriptor,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { user_ldt::set_gdt(thread, selector, descriptor) })
}

/// `i386_get_gdt()` of the MIG `mach_i386` interface.
///
/// # Safety
///
/// `thread` must be null or a live thread, and `descriptor` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_get_gdt(
    thread: *mut Thread,
    selector: c_int,
    descriptor: *mut Descriptor,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { user_ldt::get_gdt(thread, selector, descriptor) })
}
