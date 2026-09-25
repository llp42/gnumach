// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_fault.c and vm/vm_fault.h:
//   Copyright (c) 1994,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The page-fault routines of `vm/vm_fault.c` that Rust holds: wiring a map
//! entry, unwiring it, cleaning up an object/page pair, and copying pages
//! between objects.  `vm_fault_init()`, `vm_fault_page()`,
//! `vm_fault_continue()` and `vm_fault()` still live in C;
//! `vm_fault_page()`'s copy-object retry loop is the blocker `MIGRATE.md`
//! records.

use crate::arch::i386::pmap::{pmap_change_wiring, pmap_enter, pmap_pageable};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue::{vm_fault, vm_fault_page, vm_page_queue_lock};
use crate::kern::debug::kpanic;
use crate::kern::task::current_task;
use crate::vm::error::{
    KERN_FAILURE, KERN_MEMORY_ERROR, KERN_SUCCESS, MACH_SEND_INTERRUPTED,
};
use crate::vm::types::{VmObject, VmPage, VmProt};
use crate::vm::vm_map::{VmMap, VmMapEntry, VmMapVersion};
use crate::vm::vm_object::{
    self, page_free, page_wakeup_done, paging_begin, paging_end,
};
use crate::vm::vm_user::vm_stat;
use crate::vm::{vm_page, vm_resident};
use core::ffi::c_int;
use core::ptr::{self, NonNull};

/// `VM_FAULT_*` of <vm/vm_fault.h>, the values `vm_fault_page()` returns.
const VM_FAULT_SUCCESS: c_int = 0;
const VM_FAULT_RETRY: c_int = 1;
const VM_FAULT_INTERRUPTED: c_int = 2;
const VM_FAULT_MEMORY_SHORTAGE: c_int = 3;
const VM_FAULT_FICTITIOUS_SHORTAGE: c_int = 4;
const VM_FAULT_MEMORY_ERROR: c_int = 5;

/// `vm_fault_wire()` in C: wire down every page of `entry` in `map`.
///
/// # Safety
///
/// `entry` must be a live entry of `map`, and `map` must be referenced and
/// read-locked (or otherwise stable) for the whole call, as the C requires.
pub(crate) unsafe fn wire(map: &VmMap, entry: NonNull<VmMapEntry>) {
    // SAFETY: the caller promises a live entry; the links are read once,
    // before the faults below can change the entry's wiring.
    let (start, end) = unsafe {
        let links = &(*entry.as_ptr()).links;
        (links.start, links.end)
    };

    pmap_pageable(map.pmap, start, end, c_int::from(false));

    let map = ptr::from_ref(map).cast_mut();
    let mut va = start;
    while va < end {
        // SAFETY: `map` and `entry` are live and read-locked by the caller,
        // and `va` is an address the entry covers.
        let wired = unsafe { wire_fast(&*map, va, entry.as_ptr()) };
        if wired != KERN_SUCCESS {
            // SAFETY: as above; the C fault path takes the same map and
            // address, wires the page and passes no continuation.
            unsafe {
                vm_fault(
                    map,
                    va,
                    VmProt::NONE,
                    c_int::from(true),
                    c_int::from(false),
                    None,
                )
            };
        }
        va = va.wrapping_add(PAGE_SIZE);
    }
}

/// `vm_fault_wire_fast()` of `vm/vm_fault.c`.
///
/// # Safety
///
/// `map` must be the referenced, read-locked map `entry` belongs to, `entry`
/// must be a live entry of it, and `va` must be a page address the entry
/// covers.
pub(crate) unsafe fn wire_fast(
    map: &VmMap,
    va: VmOffset,
    entry: *mut VmMapEntry,
) -> c_int {
    // SAFETY: the caller promises the live task and entry.
    unsafe {
        vm_stat.faults += 1;
        (*current_task()).faults += 1;
    }

    // SAFETY: the caller promises the live entry.
    if unsafe { (*entry).is_sub_map() } {
        return KERN_FAILURE;
    }

    // SAFETY: the caller promises the live entry.
    let (object, offset, prot) = unsafe {
        (
            (*entry).object.vm_object,
            va.wrapping_sub((*entry).links.start)
                .wrapping_add((*entry).offset),
            (*entry).protection,
        )
    };

    // SAFETY: the object is live, and the C takes its lock to keep it from
    // being disposed while the page is wired.
    unsafe {
        (*object).lock.lock();
        (*object).ref_count += 1;
        (*object).set_paging_in_progress((*object).paging_in_progress() + 1);
    }

    // SAFETY: the object is live and locked.
    let m =
        unsafe { vm_resident::lookup(NonNull::new_unchecked(object), offset) };
    let Some(m) = m else {
        // SAFETY: the object is live and locked, holding the paging
        // reference taken above.
        return unsafe { give_up(object) };
    };

    // SAFETY: the lookup returned a live, locked page.
    let unusable = unsafe {
        (*m.as_ptr()).is_error()
            || (*m.as_ptr()).is_busy()
            || (*m.as_ptr()).is_absent()
            || (prot & (*m.as_ptr()).page_lock()) != VmProt::NONE
    };
    if unusable {
        // SAFETY: the object is live and locked, holding the paging
        // reference taken above.
        return unsafe { give_up(object) };
    }

    // SAFETY: the page is live; the C wires it under the page-queues lock.
    unsafe {
        (*ptr::addr_of_mut!(vm_page_queue_lock)).lock();
        vm_page::wire(m);
        (*ptr::addr_of_mut!(vm_page_queue_lock)).unlock();
        (*m.as_ptr()).set_busy(true);
    }

    // SAFETY: the object is live and locked; the page is live and busy, as
    // the C's `RELEASE_PAGE` assumes.
    if !unsafe { (*object).copy.is_null() }
        && (prot & VmProt::WRITE) != VmProt::NONE
    {
        // SAFETY: as the C's `RELEASE_PAGE`; the object lock is held.
        unsafe {
            page_wakeup_done(m.as_ptr());
            (*ptr::addr_of_mut!(vm_page_queue_lock)).lock();
            vm_page::unwire(m.as_ptr());
            (*ptr::addr_of_mut!(vm_page_queue_lock)).unlock();
            return give_up(object);
        }
    }

    // SAFETY: the object is live and locked.
    unsafe { (*object).lock.unlock() };

    // SAFETY: the page is live, and `map` is the caller's live map.
    unsafe {
        pmap_enter(
            map.pmap,
            va,
            (*m.as_ptr()).phys_addr,
            (prot & !(*m.as_ptr()).page_lock()).bits(),
            1,
        );
    }

    // SAFETY: the object is live; the C relocks it to clear the paging
    // reference and hands back the reference it took.
    unsafe {
        (*object).lock.lock();
        page_wakeup_done(m.as_ptr());
        (*object).set_paging_in_progress((*object).paging_in_progress() - 1);
        (*object).lock.unlock();
        vm_object::deallocate(object);
    }

    KERN_SUCCESS
}

/// The `GIVE_UP` path of [`wire_fast`]: drop the object's paging reference
/// and lock, then the object reference.
///
/// # Safety
///
/// `object` must be live, its lock held, and its paging reference taken by
/// [`wire_fast`].
unsafe fn give_up(object: *mut VmObject) -> c_int {
    // SAFETY: the caller holds the lock and the paging reference.
    unsafe {
        (*object).set_paging_in_progress((*object).paging_in_progress() - 1);
        (*object).lock.unlock();
        vm_object::deallocate(object);
    }
    KERN_FAILURE
}

/// `vm_fault_cleanup()` of `vm/vm_fault.c`.
///
/// # Safety
///
/// `object` must be live, its lock held, and its paging reference held;
/// `top_page` must be null or the busy page the fault left in the top
/// object, whose own object the call then cleans up.
pub(crate) unsafe fn cleanup(object: *mut VmObject, top_page: *mut VmPage) {
    // SAFETY: the caller holds the object lock and its paging reference.
    unsafe {
        paging_end(object);
        (*object).lock.unlock();

        if !top_page.is_null() {
            let top_object = (*top_page).object;
            (*top_object).lock.lock();
            page_free(top_page);
            paging_end(top_object);
            (*top_object).lock.unlock();
        }
    }
}

/// `vm_fault_unwire()` of `vm/vm_fault.c`.
///
/// # Safety
///
/// `map` must be a live, referenced map and `entry` a live entry of it whose
/// pages are wired down.
pub(crate) unsafe fn unwire(map: &VmMap, entry: NonNull<VmMapEntry>) {
    // SAFETY: the caller promises the live entry.
    let (start, end_addr, object) = unsafe {
        (
            (*entry.as_ptr()).links.start,
            (*entry.as_ptr()).links.end,
            if (*entry.as_ptr()).is_sub_map() {
                ptr::null_mut()
            } else {
                (*entry.as_ptr()).object.vm_object
            },
        )
    };

    let pmap = map.pmap;
    let map_ptr = ptr::from_ref(map).cast_mut();
    let mut va = start;
    while va < end_addr {
        // SAFETY: the caller promises the live map and entry.
        unsafe { pmap_change_wiring(pmap, va, 0) };

        if object.is_null() {
            // SAFETY: `map` is the caller's live map; the recursive lock is
            // the C's, so the fault below may take the map lock again.
            unsafe {
                map.lock.set_recursive();
                vm_fault(map_ptr, va, VmProt::NONE, 1, 0, None);
                map.lock.clear_recursive();
            }
        } else {
            let mut result;
            let mut result_page: *mut VmPage = ptr::null_mut();
            let mut top_page: *mut VmPage = ptr::null_mut();
            loop {
                let mut prot = VmProt::NONE;
                // SAFETY: the object is live; each attempt takes the lock
                // and paging reference the fault consumes, exactly as the C
                // do/while does.
                unsafe {
                    (*object).lock.lock();
                    paging_begin(object);
                    result = vm_fault_page(
                        object,
                        (*entry.as_ptr())
                            .offset
                            .wrapping_add(va.wrapping_sub(start)),
                        VmProt::NONE,
                        1,
                        0,
                        &mut prot,
                        &mut result_page,
                        &mut top_page,
                        0,
                        None,
                    );
                }
                if result != VM_FAULT_RETRY {
                    break;
                }
            }
            if result != VM_FAULT_SUCCESS {
                kpanic!("vm_fault_unwire", "vm_fault_unwire: failure");
            }

            // SAFETY: the fault returned the live, busy page and holds its
            // object lock.
            unsafe {
                (*ptr::addr_of_mut!(vm_page_queue_lock)).lock();
                vm_page::unwire(result_page);
                (*ptr::addr_of_mut!(vm_page_queue_lock)).unlock();
                page_wakeup_done(result_page);
                cleanup((*result_page).object, top_page);
            }
        }
        va = va.wrapping_add(PAGE_SIZE);
    }

    pmap_pageable(pmap, start, end_addr, 1);
}

/// `vm_fault_copy()` of `vm/vm_fault.c`: copy pages from `src_object` into
/// `dst_object`, advancing through the destination map's `dst_version`.
///
/// # Safety
///
/// The caller must hold a reference, but not a lock, to each object and to
/// `dst_map`; `src_size` must be writable and name the bytes to copy.
///
/// # Panics
///
/// Halts through the kernel panic path when `vm_fault_page()` cannot produce
/// the page the C asserted.
#[expect(clippy::too_many_arguments)]
pub(crate) unsafe fn copy(
    src_object: *mut VmObject,
    mut src_offset: VmOffset,
    src_size: &mut VmSize,
    dst_object: *mut VmObject,
    mut dst_offset: VmOffset,
    dst_map: NonNull<VmMap>,
    dst_version: &VmMapVersion,
    interruptible: bool,
) -> c_int {
    let mut amount_done: VmSize = 0;

    loop {
        let mut src_top_page: *mut VmPage = ptr::null_mut();
        let src_page = if src_object.is_null() {
            ptr::null_mut()
        } else {
            let page = 'source: loop {
                let mut prot = VmProt::READ;
                let mut result_page: *mut VmPage = ptr::null_mut();
                // SAFETY: each attempt takes the source object's lock and
                // paging reference, which `vm_fault_page()` consumes.
                let result = unsafe {
                    (*src_object).lock.lock();
                    paging_begin(src_object);
                    vm_fault_page(
                        src_object,
                        src_offset,
                        VmProt::READ,
                        0,
                        c_int::from(interruptible),
                        &mut prot,
                        &mut result_page,
                        &mut src_top_page,
                        0,
                        None,
                    )
                };
                match result {
                    VM_FAULT_SUCCESS => break 'source result_page,
                    VM_FAULT_RETRY => continue 'source,
                    VM_FAULT_INTERRUPTED => {
                        *src_size = amount_done;
                        return MACH_SEND_INTERRUPTED;
                    }
                    VM_FAULT_MEMORY_SHORTAGE => {
                        // SAFETY: the page wait takes no continuation.
                        unsafe { vm_page::wait(None) };
                        continue 'source;
                    }
                    VM_FAULT_FICTITIOUS_SHORTAGE => {
                        // SAFETY: the slab package is up in this path.
                        unsafe { vm_resident::more_fictitious() };
                        continue 'source;
                    }
                    VM_FAULT_MEMORY_ERROR => return KERN_MEMORY_ERROR,
                    _ => return KERN_MEMORY_ERROR,
                }
            };
            // SAFETY: the fault left the result page's object locked.
            unsafe { (*(*page).object).lock.unlock() };
            page
        };

        let mut dst_top_page: *mut VmPage = ptr::null_mut();
        let dst_page = 'dest: loop {
            let mut prot = VmProt::WRITE;
            let mut result_page: *mut VmPage = ptr::null_mut();
            // SAFETY: each attempt takes the destination object's lock and
            // paging reference, which `vm_fault_page()` consumes.
            let result = unsafe {
                (*dst_object).lock.lock();
                paging_begin(dst_object);
                vm_fault_page(
                    dst_object,
                    dst_offset,
                    VmProt::WRITE,
                    0,
                    0,
                    &mut prot,
                    &mut result_page,
                    &mut dst_top_page,
                    0,
                    None,
                )
            };
            match result {
                VM_FAULT_SUCCESS => break 'dest result_page,
                VM_FAULT_RETRY => continue 'dest,
                VM_FAULT_INTERRUPTED => {
                    if !src_page.is_null() {
                        // SAFETY: the source fault left the page and its
                        // top page for this call to release.
                        unsafe { copy_cleanup(src_page, src_top_page) };
                    }
                    *src_size = amount_done;
                    return MACH_SEND_INTERRUPTED;
                }
                VM_FAULT_MEMORY_SHORTAGE => {
                    // SAFETY: the page wait takes no continuation.
                    unsafe { vm_page::wait(None) };
                    continue 'dest;
                }
                VM_FAULT_FICTITIOUS_SHORTAGE => {
                    // SAFETY: the slab package is up in this path.
                    unsafe { vm_resident::more_fictitious() };
                    continue 'dest;
                }
                VM_FAULT_MEMORY_ERROR => {
                    if !src_page.is_null() {
                        // SAFETY: as in the interrupted arm.
                        unsafe { copy_cleanup(src_page, src_top_page) };
                    }
                    return KERN_MEMORY_ERROR;
                }
                _ => {
                    if !src_page.is_null() {
                        // SAFETY: as in the interrupted arm.
                        unsafe { copy_cleanup(src_page, src_top_page) };
                    }
                    return KERN_MEMORY_ERROR;
                }
            }
        };

        // SAFETY: the destination object is live and was left locked.
        let old_copy_object = unsafe { (*(*dst_page).object).copy };
        // SAFETY: the fault left the page's object locked.
        unsafe { (*(*dst_page).object).lock.unlock() };

        if !VmMap::verify(dst_map, dst_version) {
            // SAFETY: both faults left their page and top page to release.
            unsafe {
                if !src_page.is_null() {
                    copy_cleanup(src_page, src_top_page);
                }
                copy_cleanup(dst_page, dst_top_page);
            }
            break;
        }

        // SAFETY: the destination object is live; the C relocks it to
        // recheck the copy object.
        unsafe {
            (*(*dst_page).object).lock.lock();
            if (*(*dst_page).object).copy != old_copy_object {
                (*(*dst_page).object).lock.unlock();
                // SAFETY: `verify` left the read lock held; the failed
                // recheck releases it.
                (*dst_map.as_ptr()).lock.done();
                if !src_page.is_null() {
                    copy_cleanup(src_page, src_top_page);
                }
                copy_cleanup(dst_page, dst_top_page);
                break;
            }
            (*(*dst_page).object).lock.unlock();
        }

        if src_page.is_null() {
            // SAFETY: the destination page is live and busy.
            unsafe {
                vm_resident::zero_fill(NonNull::new_unchecked(dst_page))
            };
        } else {
            // SAFETY: both pages are live and busy.
            unsafe {
                vm_resident::copy(
                    NonNull::new_unchecked(src_page),
                    NonNull::new_unchecked(dst_page),
                )
            };
        }
        // SAFETY: the destination page is live.
        unsafe { (*dst_page).set_dirty(true) };
        // SAFETY: `verify` left the read lock held.
        unsafe { (*dst_map.as_ptr()).lock.done() };

        // SAFETY: both faults left their page and top page to release.
        unsafe {
            if !src_page.is_null() {
                copy_cleanup(src_page, src_top_page);
            }
            copy_cleanup(dst_page, dst_top_page);
        }

        amount_done = amount_done.wrapping_add(PAGE_SIZE);
        src_offset = src_offset.wrapping_add(PAGE_SIZE);
        dst_offset = dst_offset.wrapping_add(PAGE_SIZE);

        if amount_done == *src_size {
            break;
        }
    }

    *src_size = amount_done;
    KERN_SUCCESS
}

/// `vm_fault_copy_cleanup()` of `vm/vm_fault.c`: release the busy page a
/// [`copy`] fault returned, then its object through [`cleanup`].
///
/// # Safety
///
/// `page` must be the busy page the fault returned with its object locked,
/// and `top_page` that fault's top page.
unsafe fn copy_cleanup(page: *mut VmPage, top_page: *mut VmPage) {
    // SAFETY: the fault left the page's object locked.
    let object = unsafe { (*page).object };
    // SAFETY: the caller promises the live page, its object lock and the
    // page-queues lock the C takes.
    unsafe {
        (*object).lock.lock();
        page_wakeup_done(page);
        (*ptr::addr_of_mut!(vm_page_queue_lock)).lock();
        if !(*page).is_active() && !(*page).is_inactive() {
            vm_page::activate(page);
        }
        (*ptr::addr_of_mut!(vm_page_queue_lock)).unlock();
        cleanup(object, top_page);
    }
}
