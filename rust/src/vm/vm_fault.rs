// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_fault.c and vm/vm_fault.h:
//   Copyright (c) 1994,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The page-fault routines of `vm/vm_fault.c` that Rust holds: finding the
//! resident page for an object/offset, wiring a map entry, unwiring it,
//! cleaning up an object/page pair, and copying pages between objects.
//! `vm_fault_init()`, `vm_fault_continue()` and `vm_fault()` still live in
//! C.

use crate::arch::i386::mp_desc::simple_lock_pause;
use crate::arch::i386::percpu::current_thread;
use crate::arch::i386::pmap::{
    pmap_change_wiring, pmap_clear_modify, pmap_enter, pmap_page_protect,
    pmap_pageable,
};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue::{
    self, memory_object_data_request, memory_object_data_unlock, vm_fault,
    vm_page_queue_lock,
};
use crate::kern::console::kprint;
use crate::kern::debug::kpanic;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, THREAD_RESTART, assert_wait, thread_block,
    thread_wakeup_prim,
};
use crate::kern::task::current_task;
use crate::kern::thread::Continuation;
use crate::vm::error::{
    KERN_FAILURE, KERN_MEMORY_ERROR, KERN_SUCCESS, MACH_SEND_INTERRUPTED,
};
use crate::vm::types::{VmObject, VmPage, VmProt};
use crate::vm::vm_external::{self, VM_EXTERNAL_STATE_ABSENT};
use crate::vm::vm_map::{VmMap, VmMapEntry, VmMapVersion};
use crate::vm::vm_object::{
    self, deallocate, page_free, page_wakeup_done, paging_begin, paging_end,
};
use crate::vm::vm_pageout_ffi::vm_pageout_page;
use crate::vm::vm_user::vm_stat;
use crate::vm::{vm_page, vm_resident};
use core::ffi::{c_int, c_uint, c_void};
use core::mem::{align_of, size_of};
use core::ptr::{self, NonNull, addr_of_mut};

/// `VM_PAGE_HIGHMEM` of <vm/vm_page.h>: the page may come from high
/// physical memory.
const VM_PAGE_HIGHMEM: c_uint = 0x08;

/// `VM_FAULT_*` of <vm/vm_fault.h>, the values `vm_fault_page()` returns.
pub(crate) const VM_FAULT_SUCCESS: c_int = 0;
const VM_FAULT_RETRY: c_int = 1;
const VM_FAULT_INTERRUPTED: c_int = 2;
const VM_FAULT_MEMORY_SHORTAGE: c_int = 3;
const VM_FAULT_FICTITIOUS_SHORTAGE: c_int = 4;
const VM_FAULT_MEMORY_ERROR: c_int = 5;

/// `vm_fault_state_t` of `vm/vm_fault.c`: the state `vm_fault()` saves on the
/// current thread for a continuation.
#[repr(C)]
struct VmFaultState {
    vmf_map: *mut VmMap,
    vmf_vaddr: VmOffset,
    vmf_fault_type: VmProt,
    vmf_change_wiring: c_int,
    vmf_continuation: Option<unsafe extern "C" fn(c_int)>,
    vmf_version: VmMapVersion,
    vmf_wired: c_int,
    vmf_object: *mut VmObject,
    vmf_offset: VmOffset,
    vmf_prot: VmProt,
    vmfp_backoff: c_int,
    vmfp_object: *mut VmObject,
    vmfp_offset: VmOffset,
    vmfp_first_m: *mut VmPage,
    vmfp_access: VmProt,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<VmFaultState>() == 96);
    assert!(align_of::<VmFaultState>() == 8);
    assert!(core::mem::offset_of!(VmFaultState, vmf_map) == 0);
    assert!(core::mem::offset_of!(VmFaultState, vmf_vaddr) == 8);
    assert!(core::mem::offset_of!(VmFaultState, vmf_fault_type) == 16);
    assert!(core::mem::offset_of!(VmFaultState, vmf_change_wiring) == 20);
    assert!(core::mem::offset_of!(VmFaultState, vmf_continuation) == 24);
    assert!(core::mem::offset_of!(VmFaultState, vmf_version) == 32);
    assert!(core::mem::offset_of!(VmFaultState, vmf_wired) == 36);
    assert!(core::mem::offset_of!(VmFaultState, vmf_object) == 40);
    assert!(core::mem::offset_of!(VmFaultState, vmf_offset) == 48);
    assert!(core::mem::offset_of!(VmFaultState, vmf_prot) == 56);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_backoff) == 60);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_object) == 64);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_offset) == 72);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_first_m) == 80);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_access) == 88);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<VmFaultState>() == 60);
    assert!(align_of::<VmFaultState>() == 4);
    assert!(core::mem::offset_of!(VmFaultState, vmf_map) == 0);
    assert!(core::mem::offset_of!(VmFaultState, vmf_vaddr) == 4);
    assert!(core::mem::offset_of!(VmFaultState, vmf_fault_type) == 8);
    assert!(core::mem::offset_of!(VmFaultState, vmf_change_wiring) == 12);
    assert!(core::mem::offset_of!(VmFaultState, vmf_continuation) == 16);
    assert!(core::mem::offset_of!(VmFaultState, vmf_version) == 20);
    assert!(core::mem::offset_of!(VmFaultState, vmf_wired) == 24);
    assert!(core::mem::offset_of!(VmFaultState, vmf_object) == 28);
    assert!(core::mem::offset_of!(VmFaultState, vmf_offset) == 32);
    assert!(core::mem::offset_of!(VmFaultState, vmf_prot) == 36);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_backoff) == 40);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_object) == 44);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_offset) == 48);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_first_m) == 52);
    assert!(core::mem::offset_of!(VmFaultState, vmfp_access) == 56);
};

/// What [`fault_page`] produced: the `VM_FAULT_*` result and the outputs the
/// C signature carried in pointers.
pub(crate) struct Fault {
    /// The `VM_FAULT_*` result the C returned.
    pub(crate) result: c_int,
    /// The protection for the mapping, modified in place as the C did.
    pub(crate) protection: VmProt,
    /// The busy result page, or null when the fault failed.
    pub(crate) result_page: *mut VmPage,
    /// The busy page left in the top object, or null.
    pub(crate) top_page: *mut VmPage,
}

impl Fault {
    /// A failure result, which carries no pages.
    const fn error(result: c_int, protection: VmProt) -> Self {
        Self {
            result,
            protection,
            result_page: ptr::null_mut(),
            top_page: ptr::null_mut(),
        }
    }
}

/// The `vm_fault_state_t` the C `vm_fault()` allocated on the current thread
/// for a continuation.
///
/// # Safety
///
/// The current thread's `ith_other` must hold the state `vm_fault()` saved
/// before calling with a continuation.
unsafe fn fault_state() -> *mut VmFaultState {
    // SAFETY: the caller promises the allocation.
    unsafe { (*current_thread()).saved.other.cast::<VmFaultState>() }
}

/// The `after_block_and_backoff` code of the C: the wait result decides retry
/// or interruption.
///
/// # Safety
///
/// The current thread must have just returned from `thread_block()`.
unsafe fn after_block_and_backoff() -> c_int {
    // SAFETY: the block returned on the current thread.
    if unsafe { (*current_thread()).wait_result } == THREAD_AWAKENED {
        VM_FAULT_RETRY
    } else {
        VM_FAULT_INTERRUPTED
    }
}

/// The `after_thread_block` code of the C: take the object lock back and
/// report a failed wait, or `None` to continue the search.
///
/// # Safety
///
/// `object` must be the object the fault unlocked for the wait, `first_m` the
/// top page it left busy, and the current thread must have just returned from
/// `thread_block()`.
unsafe fn after_wait(
    object: *mut VmObject,
    first_m: *mut VmPage,
    protection: VmProt,
) -> Option<Fault> {
    // SAFETY: the block returned on the current thread.
    let wait_result = unsafe { (*current_thread()).wait_result };
    // SAFETY: the caller promises the live object.
    unsafe { (*object).lock.lock() };
    if wait_result == THREAD_AWAKENED {
        return None;
    }

    // SAFETY: the caller promises the locked object and its top page.
    unsafe { cleanup(object, first_m) };
    Some(Fault::error(
        if wait_result == THREAD_RESTART {
            VM_FAULT_RETRY
        } else {
            VM_FAULT_INTERRUPTED
        },
        protection,
    ))
}

/// The `block_and_backoff` epilogue of the C: clean up the fault's object and
/// top page, block, and report the wait result.
///
/// # Safety
///
/// `object` must be live and locked with its paging reference held, and
/// `first_m` the busy top page the search left.  With a continuation, the
/// current thread's `ith_other` must hold the state `vm_fault()` saved.
unsafe fn block_and_backoff(
    object: *mut VmObject,
    first_m: *mut VmPage,
    protection: VmProt,
    continuation: Continuation,
) -> Fault {
    // SAFETY: the caller promises the locked object and top page.
    unsafe { cleanup(object, first_m) };

    match continuation {
        Some(continuation) => {
            let state = unsafe { fault_state() };
            // SAFETY: the state is the live allocation the C made for this
            // continuation.
            unsafe {
                (*state).vmfp_backoff = 1;
                (*state).vmf_prot = protection;
                thread_block(Some(continuation));
            }
        }
        None => {
            // SAFETY: the block takes no continuation.
            unsafe { thread_block(None) };
        }
    }

    // SAFETY: the block returned on the current thread.
    Fault::error(unsafe { after_block_and_backoff() }, protection)
}

/// `RELEASE_PAGE()` of the C `vm_fault_page()`.
///
/// # Safety
///
/// `m` must be a live busy page whose object lock the caller holds, and the
/// page-queues lock must not be held.
unsafe fn release_page(m: *mut VmPage) {
    // SAFETY: the caller promises the live busy page.
    unsafe {
        page_wakeup_done(m);
        (*addr_of_mut!(vm_page_queue_lock)).lock();
        if !(*m).is_active() && !(*m).is_inactive() {
            vm_page::activate(m);
        }
        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }
}

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

/// `vm_fault_page()` of `vm/vm_fault.c`: find the resident page for the
/// object/offset pair, following the shadow chain and requesting the data
/// from the pager when it is absent.
///
/// # Safety
///
/// `first_object` must be live, locked and referenced and must donate one
/// paging reference; the call consumes the lock and the reference.  When
/// `resume` is set, the current thread's `ith_other` must hold the state
/// `vm_fault()` saved, and `continuation` must be the continuation that
/// state names.
#[expect(clippy::too_many_arguments)]
pub(crate) unsafe fn fault_page(
    first_object: *mut VmObject,
    first_offset: VmOffset,
    fault_type: VmProt,
    must_be_resident: bool,
    mut interruptible: bool,
    protection: VmProt,
    resume: bool,
    continuation: Continuation,
) -> Fault {
    let mut protection = protection;
    let mut object: *mut VmObject;
    let mut offset: VmOffset;
    let mut first_m: *mut VmPage;
    let mut access_required: VmProt;
    let mut m: *mut VmPage;
    let mut resume_after_thread_block = false;

    if resume {
        // SAFETY: with `resume`, the C caller saved the state on the current
        // thread before blocking.
        let state = unsafe { fault_state() };
        // SAFETY: `state` is the live state the C saved.
        if unsafe { (*state).vmfp_backoff } != 0 {
            // SAFETY: the backoff already cleaned up before it blocked.
            return Fault::error(
                unsafe { after_block_and_backoff() },
                protection,
            );
        }
        // SAFETY: `state` is the live state the C saved.
        unsafe {
            object = (*state).vmfp_object;
            offset = (*state).vmfp_offset;
            first_m = (*state).vmfp_first_m;
            access_required = (*state).vmfp_access;
        }
        resume_after_thread_block = true;
    } else {
        // SAFETY: the caller promises the live task.
        unsafe {
            vm_stat.faults += 1;
            (*current_task()).faults += 1;
        }

        // SAFETY: both C tunables are plain statics that live for the
        // kernel's lifetime.
        if unsafe { glue::vm_fault_dirty_handling } != 0
            && !fault_type.contains(VmProt::WRITE)
        {
            protection &= !VmProt::WRITE;
        }

        // SAFETY: as the dirty-handling tunable above.
        if unsafe { glue::vm_fault_interruptible } == 0 {
            interruptible = false;
        }

        object = first_object;
        offset = first_offset;
        first_m = ptr::null_mut();
        access_required = fault_type;
    }

    'search: loop {
        if resume_after_thread_block {
            resume_after_thread_block = false;
            // SAFETY: the thread just returned from the block that saved
            // this object and top page.
            if let Some(fault) =
                unsafe { after_wait(object, first_m, protection) }
            {
                return fault;
            }
            continue 'search;
        }

        // SAFETY: the object is live and locked.
        let found = unsafe {
            vm_resident::lookup(NonNull::new_unchecked(object), offset)
        };
        m = found.map_or(ptr::null_mut(), NonNull::as_ptr);

        if !m.is_null() {
            // SAFETY: the lookup returned the live, locked page.
            if unsafe { (*m).is_busy() } {
                // SAFETY: the page is live and locked.
                unsafe {
                    (*m).set_wanted(true);
                    assert_wait(
                        m.cast::<c_void>(),
                        c_int::from(interruptible),
                    );
                }
                // SAFETY: the object is live and locked.
                unsafe { (*object).lock.unlock() };

                match continuation {
                    Some(continuation) => {
                        let state = unsafe { fault_state() };
                        // SAFETY: the state is the live allocation the C
                        // made for this continuation.
                        unsafe {
                            (*state).vmfp_backoff = 0;
                            (*state).vmfp_object = object;
                            (*state).vmfp_offset = offset;
                            (*state).vmfp_first_m = first_m;
                            (*state).vmfp_access = access_required;
                            (*state).vmf_prot = protection;
                            thread_block(Some(continuation));
                        }
                    }
                    None => {
                        // SAFETY: the block takes no continuation.
                        unsafe { thread_block(None) };
                    }
                }

                // SAFETY: the object and top page are the ones the block
                // saved.
                if let Some(fault) =
                    unsafe { after_wait(object, first_m, protection) }
                {
                    return fault;
                }
                continue 'search;
            }

            // SAFETY: the page is live and locked.
            if unsafe { (*m).is_error() } {
                // SAFETY: the page is live and locked.
                unsafe {
                    page_free(m);
                    cleanup(object, first_m);
                }
                return Fault::error(VM_FAULT_MEMORY_ERROR, protection);
            }

            // SAFETY: the page is live and locked.
            if unsafe { (*m).is_absent() } {
                offset =
                    offset.wrapping_add(unsafe { (*object).shadow_offset });
                access_required = VmProt::READ;
                let next_object = unsafe { (*object).shadow };
                if next_object.is_null() {
                    // SAFETY: the allocator may spin while the object lock
                    // is held.
                    let Some(real_m) =
                        (unsafe { vm_resident::grab(VM_PAGE_HIGHMEM) })
                    else {
                        // SAFETY: the object and top page are the fault's.
                        unsafe { cleanup(object, first_m) };
                        return Fault::error(
                            VM_FAULT_MEMORY_SHORTAGE,
                            protection,
                        );
                    };

                    if object != first_object {
                        // SAFETY: the bottom absent page is the fault's,
                        // and the object lock is held.
                        unsafe {
                            page_free(m);
                            paging_end(object);
                            (*object).lock.unlock();
                        }
                        object = first_object;
                        offset = first_offset;
                        m = first_m;
                        first_m = ptr::null_mut();
                        // SAFETY: the top object is live.
                        unsafe { (*object).lock.lock() };
                    }

                    // SAFETY: the top absent page is the fault's and the
                    // queue lock guards the page table.
                    unsafe {
                        page_free(m);
                        (*addr_of_mut!(vm_page_queue_lock)).lock();
                        vm_resident::insert(
                            real_m,
                            NonNull::new_unchecked(object),
                            offset,
                        );
                        (*addr_of_mut!(vm_page_queue_lock)).unlock();
                    }
                    m = real_m.as_ptr();

                    // SAFETY: the page is tabled in the locked object; the
                    // lock is dropped for the zero fill as the C did.
                    unsafe {
                        (*object).lock.unlock();
                        vm_resident::zero_fill(real_m);
                        vm_stat.zero_fill_count += 1;
                        (*current_task()).zero_fills += 1;
                        (*object).lock.lock();
                        pmap_clear_modify((*m).phys_addr);
                    }
                    break 'search;
                }

                if must_be_resident {
                    // SAFETY: the object is live and locked.
                    unsafe { paging_end(object) };
                } else if object != first_object {
                    // SAFETY: the object is live and locked; the absent
                    // page is the fault's.
                    unsafe {
                        paging_end(object);
                        page_free(m);
                    }
                } else {
                    // SAFETY: the page is live and locked; the queue lock
                    // guards the page queues.
                    unsafe {
                        first_m = m;
                        (*m).set_absent(false);
                        vm_object::absent_release(object);
                        (*m).set_busy(true);

                        (*addr_of_mut!(vm_page_queue_lock)).lock();
                        vm_page::queues_remove(m);
                        (*addr_of_mut!(vm_page_queue_lock)).unlock();
                    }
                }

                // SAFETY: the next object is live; the C locks it before
                // unlocking the current one.
                unsafe {
                    (*next_object).lock.lock();
                    (*object).lock.unlock();
                }
                object = next_object;
                // SAFETY: the object is live and locked.
                unsafe { paging_begin(object) };
                continue 'search;
            }

            // SAFETY: the page is live and locked.
            if (access_required & unsafe { (*m).page_lock() }) != VmProt::NONE
            {
                if (access_required & unsafe { (*m).unlock_request() })
                    != access_required
                {
                    // SAFETY: the object is live and locked.
                    if !unsafe { (*object).is_pager_ready() } {
                        // SAFETY: the object is live and locked.
                        unsafe {
                            vm_object::assert_wait_event(
                                object,
                                vm_object::EVENT_PAGER_READY,
                                interruptible,
                            )
                        };
                        // SAFETY: the caller promises the locked object.
                        return unsafe {
                            block_and_backoff(
                                object,
                                first_m,
                                protection,
                                continuation,
                            )
                        };
                    }

                    // SAFETY: the page is live and locked.
                    let new_unlock_request =
                        access_required | unsafe { (*m).unlock_request() };
                    // SAFETY: the page is live and locked.
                    unsafe { (*m).set_unlock_request(new_unlock_request) };
                    // SAFETY: the object is live and locked.
                    unsafe { (*object).lock.unlock() };

                    // SAFETY: the pager port and its request are live, and
                    // the busy page holds the object's paging reference.
                    let rc = unsafe {
                        memory_object_data_unlock(
                            (*object).pager,
                            (*object).pager_request,
                            offset.wrapping_add((*object).paging_offset),
                            PAGE_SIZE,
                            new_unlock_request,
                        )
                    };
                    if rc != KERN_SUCCESS {
                        kprint!(
                            "vm_fault: memory_object_data_unlock failed\n"
                        );
                        // SAFETY: the object is live; its lock is taken
                        // back for the cleanup.
                        unsafe {
                            (*object).lock.lock();
                            cleanup(object, first_m);
                        }
                        return Fault::error(
                            if rc == MACH_SEND_INTERRUPTED {
                                VM_FAULT_INTERRUPTED
                            } else {
                                VM_FAULT_MEMORY_ERROR
                            },
                            protection,
                        );
                    }

                    // SAFETY: the object is live.
                    unsafe { (*object).lock.lock() };
                    continue 'search;
                }

                // SAFETY: the page is live and locked.
                unsafe {
                    (*m).set_wanted(true);
                    assert_wait(
                        m.cast::<c_void>(),
                        c_int::from(interruptible),
                    );
                }
                // SAFETY: the caller promises the locked object.
                return unsafe {
                    block_and_backoff(
                        object,
                        first_m,
                        protection,
                        continuation,
                    )
                };
            }

            // SAFETY: the C tunable is a plain static that lives for the
            // kernel's lifetime.
            if unsafe { glue::software_reference_bits } == 0 {
                // SAFETY: the queue lock guards the page queues.
                unsafe {
                    (*addr_of_mut!(vm_page_queue_lock)).lock();
                    if (*m).is_inactive() {
                        vm_stat.reactivations += 1;
                        (*current_task()).reactivations += 1;
                    }
                    vm_page::queues_remove(m);
                    (*addr_of_mut!(vm_page_queue_lock)).unlock();
                }
            }

            // SAFETY: the page is live and locked.
            unsafe { (*m).set_busy(true) };
            break 'search;
        }

        // SAFETY: the object is live and locked, and its existence map is
        // live for the object's lifetime.
        let look_for_page = unsafe {
            (*object).is_pager_created()
                && vm_external::state_get(
                    (*object).existence_info.cast(),
                    offset.wrapping_add((*object).paging_offset),
                ) != VM_EXTERNAL_STATE_ABSENT
        };

        if (look_for_page || object == first_object) && !must_be_resident {
            // SAFETY: the fictitious list is up.
            let Some(fictitious) = (unsafe { vm_resident::grab_fictitious() })
            else {
                // SAFETY: the object and top page are the fault's.
                unsafe { cleanup(object, first_m) };
                return Fault::error(VM_FAULT_FICTITIOUS_SHORTAGE, protection);
            };
            m = fictitious.as_ptr();
            // SAFETY: the queue lock guards the page table.
            unsafe {
                (*addr_of_mut!(vm_page_queue_lock)).lock();
                vm_resident::insert(
                    fictitious,
                    NonNull::new_unchecked(object),
                    offset,
                );
                (*addr_of_mut!(vm_page_queue_lock)).unlock();
            }
        }

        if look_for_page && !must_be_resident {
            // SAFETY: the object is live and locked.
            if !unsafe { (*object).is_pager_ready() } {
                // SAFETY: the object is live and locked, and `m` is the
                // fictitious page just tabled.
                unsafe {
                    vm_object::assert_wait_event(
                        object,
                        vm_object::EVENT_PAGER_READY,
                        interruptible,
                    );
                    page_free(m);
                }
                // SAFETY: the caller promises the locked object.
                return unsafe {
                    block_and_backoff(
                        object,
                        first_m,
                        protection,
                        continuation,
                    )
                };
            }

            // SAFETY: the object is live and locked.
            if unsafe { (*object).is_internal() } {
                // SAFETY: the page is live and locked.
                if unsafe { (*m).is_fictitious() } {
                    // SAFETY: the object is live and locked, and `m` is the
                    // fictitious page just tabled.
                    let Some(real) = (unsafe {
                        vm_resident::convert(NonNull::new_unchecked(m))
                    }) else {
                        // SAFETY: the page is the fault's, and the object
                        // and top page are the fault's.
                        unsafe {
                            page_free(m);
                            cleanup(object, first_m);
                        }
                        return Fault::error(
                            VM_FAULT_MEMORY_SHORTAGE,
                            protection,
                        );
                    };
                    m = real.as_ptr();
                }
            } else {
                // SAFETY: the C tunable is a plain static that lives for
                // the kernel's lifetime.
                let absent_max = unsafe { glue::vm_object_absent_max };
                let absent_max = u32::try_from(absent_max).unwrap_or(u32::MAX);
                // SAFETY: the object is live and locked.
                if unsafe { (*object).absent_count } > absent_max {
                    // SAFETY: the object is live and locked, and `m` is the
                    // page just tabled.
                    unsafe {
                        vm_object::absent_assert_wait(object, interruptible);
                        page_free(m);
                    }
                    // SAFETY: the caller promises the locked object.
                    return unsafe {
                        block_and_backoff(
                            object,
                            first_m,
                            protection,
                            continuation,
                        )
                    };
                }
            }

            // SAFETY: the page is live and locked.
            unsafe {
                (*m).set_absent(true);
                (*object).absent_count += 1;
                (*object).lock.unlock();
            }

            // SAFETY: the caller promises the live task.
            unsafe {
                vm_stat.pageins += 1;
                (*current_task()).pageins += 1;
            }
            // SAFETY: the pager port and its request are live, and the busy
            // page holds the object's paging reference.
            let rc = unsafe {
                memory_object_data_request(
                    (*object).pager,
                    (*object).pager_request,
                    (*m).offset.wrapping_add((*object).paging_offset),
                    PAGE_SIZE,
                    access_required,
                )
            };
            if rc != KERN_SUCCESS {
                // SAFETY: the object is live and referenced by the busy
                // page.
                if !unsafe { (*object).pager }.is_null()
                    && rc != MACH_SEND_INTERRUPTED
                {
                    // SAFETY: the pager and its request are read as the C's
                    // diagnostic did.
                    kprint!(
                        "memory_object_data_request({:p}, {:p}, 0x{:x}, \
                         0x{:x}, 0x{:x}) failed, 0x{:x}\n",
                        unsafe { (*object).pager },
                        unsafe { (*object).pager_request },
                        unsafe { (*m).offset }
                            .wrapping_add(unsafe { (*object).paging_offset }),
                        PAGE_SIZE,
                        access_required.bits(),
                        rc
                    );
                }
                // SAFETY: the object is live; its lock is taken back for
                // the cleanup.
                unsafe {
                    (*object).lock.lock();
                    let still = vm_resident::lookup(
                        NonNull::new_unchecked(object),
                        offset,
                    );
                    if still == NonNull::new(m)
                        && (*m).is_absent()
                        && (*m).is_busy()
                    {
                        page_free(m);
                    }
                    cleanup(object, first_m);
                }
                return Fault::error(
                    if rc == MACH_SEND_INTERRUPTED {
                        VM_FAULT_INTERRUPTED
                    } else {
                        VM_FAULT_MEMORY_ERROR
                    },
                    protection,
                );
            }

            // SAFETY: the object is live.
            unsafe { (*object).lock.lock() };
            continue 'search;
        }

        if object == first_object {
            first_m = m;
        }

        // SAFETY: the object is live and locked.
        access_required = VmProt::READ;
        offset = offset.wrapping_add(unsafe { (*object).shadow_offset });
        let next_object = unsafe { (*object).shadow };
        if next_object.is_null() {
            if object != first_object {
                // SAFETY: the bottom object is live and locked.
                unsafe {
                    paging_end(object);
                    (*object).lock.unlock();
                }
                object = first_object;
                // The C also reset `offset` here; this path only zero-fills
                // the page it already holds, so the dead store is dropped.
                // SAFETY: the top object is live.
                unsafe { (*object).lock.lock() };
            }

            m = first_m;
            first_m = ptr::null_mut();
            if unsafe { (*m).is_fictitious() } {
                // SAFETY: the object is live and locked, and `m` is the
                // top page just taken.
                let Some(real) = (unsafe {
                    vm_resident::convert(NonNull::new_unchecked(m))
                }) else {
                    // SAFETY: the page is the fault's, and the object is
                    // the fault's.
                    unsafe {
                        page_free(m);
                        cleanup(object, ptr::null_mut());
                    }
                    return Fault::error(VM_FAULT_MEMORY_SHORTAGE, protection);
                };
                m = real.as_ptr();
            }

            // SAFETY: the page is tabled in the locked object; the lock is
            // dropped for the zero fill as the C did.
            unsafe {
                (*object).lock.unlock();
                vm_resident::zero_fill(NonNull::new_unchecked(m));
                vm_stat.zero_fill_count += 1;
                (*current_task()).zero_fills += 1;
                (*object).lock.lock();
                pmap_clear_modify((*m).phys_addr);
            }
            break 'search;
        }

        // SAFETY: the next object is live; the C locks it before unlocking
        // the current one.
        unsafe {
            (*next_object).lock.lock();
            if (object != first_object) || must_be_resident {
                paging_end(object);
            }
            (*object).lock.unlock();
        }
        object = next_object;
        // SAFETY: the object is live and locked.
        unsafe { paging_begin(object) };
    }

    if object != first_object {
        if fault_type.contains(VmProt::WRITE) {
            // SAFETY: the allocator may spin while the bottom object is
            // locked.
            let Some(copy_m) = (unsafe { vm_resident::grab(VM_PAGE_HIGHMEM) })
            else {
                // SAFETY: the result page is the fault's busy page, and
                // the object and top page are the fault's.
                unsafe {
                    release_page(m);
                    cleanup(object, first_m);
                }
                return Fault::error(VM_FAULT_MEMORY_SHORTAGE, protection);
            };
            let copy_m = copy_m.as_ptr();

            // SAFETY: the source page is live and busy in the bottom
            // object, and the copy is a fresh real page.
            unsafe {
                (*object).lock.unlock();
                vm_resident::copy(
                    NonNull::new_unchecked(m),
                    NonNull::new_unchecked(copy_m),
                );
                (*object).lock.lock();

                (*addr_of_mut!(vm_page_queue_lock)).lock();
                vm_page::deactivate(m);
                pmap_page_protect((*m).phys_addr, VmProt::NONE.bits());
                (*addr_of_mut!(vm_page_queue_lock)).unlock();

                page_wakeup_done(m);
                paging_end(object);
                (*object).lock.unlock();
            }

            // SAFETY: the caller promises the live task.
            unsafe {
                vm_stat.cow_faults += 1;
                (*current_task()).cow_faults += 1;
            }
            object = first_object;
            offset = first_offset;

            // SAFETY: the top object is live and gets the copy; the top
            // page is the fault's.
            unsafe {
                (*object).lock.lock();
                page_free(first_m);
                first_m = ptr::null_mut();
                (*addr_of_mut!(vm_page_queue_lock)).lock();
                vm_resident::insert(
                    NonNull::new_unchecked(copy_m),
                    NonNull::new_unchecked(object),
                    offset,
                );
                (*addr_of_mut!(vm_page_queue_lock)).unlock();
            }
            m = copy_m;

            // SAFETY: the object is live and locked, and the top object
            // must not be collapsed while its page is busy.
            unsafe {
                paging_end(object);
                vm_object::collapse(object);
                paging_begin(object);
            }
        } else {
            protection &= !VmProt::WRITE;
        }
    }

    loop {
        // Read the copy object with a volatile load: the retry after a
        // busy pageout or a failed try_lock must re-test it, and a plain
        // reload lets the optimizer carry the non-null it proved for the
        // previous iteration's dereference across those calls, which is
        // the miscompile MIGRATE records.
        //
        // SAFETY: `first_object` is live and referenced for the whole call.
        let copy_object =
            unsafe { ptr::read_volatile(ptr::addr_of!((*first_object).copy)) };
        if copy_object.is_null() {
            break;
        }

        if !fault_type.contains(VmProt::WRITE) {
            protection &= !VmProt::WRITE;
            break;
        }

        if must_be_resident {
            break;
        }

        // SAFETY: the copy object is live; the C tries its lock here.
        if unsafe { !(*copy_object).lock.try_lock() } {
            // SAFETY: the object lock is held.
            unsafe { (*object).lock.unlock() };
            simple_lock_pause();
            // SAFETY: the object is live and is locked again.
            unsafe { (*object).lock.lock() };
            continue;
        }

        // SAFETY: the copy object is live and locked.
        unsafe { (*copy_object).ref_count += 1 };

        let copy_offset =
            first_offset.wrapping_sub(unsafe { (*copy_object).shadow_offset });
        // SAFETY: the copy object is live and locked.
        let copy_m = unsafe {
            vm_resident::lookup(
                NonNull::new_unchecked(copy_object),
                copy_offset,
            )
        }
        .map_or(ptr::null_mut(), NonNull::as_ptr);

        if !copy_m.is_null() {
            // SAFETY: the page is live and the copy object is locked.
            if unsafe { (*copy_m).is_busy() } {
                // SAFETY: the page is live and locked.
                unsafe {
                    (*copy_m).set_wanted(true);
                    assert_wait(
                        copy_m.cast::<c_void>(),
                        c_int::from(interruptible),
                    );
                    release_page(m);
                    (*copy_object).ref_count -= 1;
                    (*copy_object).lock.unlock();
                }
                // SAFETY: the caller promises the locked object.
                return unsafe {
                    block_and_backoff(
                        object,
                        first_m,
                        protection,
                        continuation,
                    )
                };
            }
        } else {
            // SAFETY: the allocator may spin and the copy object is
            // locked.
            let Some(allocated) = (unsafe {
                vm_resident::alloc(
                    NonNull::new_unchecked(copy_object),
                    copy_offset,
                )
            }) else {
                // SAFETY: the result page and the copy object are the
                // fault's.
                unsafe {
                    release_page(m);
                    (*copy_object).ref_count -= 1;
                    (*copy_object).lock.unlock();
                    cleanup(object, first_m);
                }
                return Fault::error(VM_FAULT_MEMORY_SHORTAGE, protection);
            };
            let copy_m = allocated.as_ptr();

            // SAFETY: the source page is busy and the copy is a fresh real
            // page; the queue lock guards the pmap flush.
            unsafe {
                vm_resident::copy(NonNull::new_unchecked(m), allocated);
                (*addr_of_mut!(vm_page_queue_lock)).lock();
                pmap_page_protect((*m).phys_addr, VmProt::NONE.bits());
                (*copy_m).set_dirty(true);
                (*addr_of_mut!(vm_page_queue_lock)).unlock();
            }

            // SAFETY: the copy object is live and locked.
            if !unsafe { (*copy_object).is_pager_created() } {
                // SAFETY: the queue lock guards the page queues; the page
                // is live and busy.
                unsafe {
                    (*addr_of_mut!(vm_page_queue_lock)).lock();
                    vm_page::activate(copy_m);
                    (*addr_of_mut!(vm_page_queue_lock)).unlock();
                    page_wakeup_done(copy_m);
                }
            } else {
                // SAFETY: the object lock is dropped around the pageout,
                // as the C did.
                unsafe { (*object).lock.unlock() };
                // SAFETY: the copy page is busy and off the pageout
                // queues, and the copy object is locked.
                unsafe { vm_pageout_page(copy_m, 1, 1) };

                // SAFETY: the copy object may have been deallocated for us
                // while the pageout dropped its lock.
                if unsafe { (*copy_object).shadow != object }
                    || unsafe { (*copy_object).ref_count == 1 }
                {
                    // SAFETY: the copy object lock is held here and the
                    // object reference is the fault's.
                    unsafe {
                        (*copy_object).lock.unlock();
                        deallocate(copy_object);
                        (*object).lock.lock();
                    }
                    continue;
                }

                // SAFETY: the object is live; the C takes the lock back.
                unsafe { (*object).lock.lock() };
            }

            // SAFETY: the source page is live and its object is locked.
            if unsafe { (*m).is_wanted() } {
                // SAFETY: the page is live and its object is locked.
                unsafe {
                    (*m).set_wanted(false);
                    thread_wakeup_prim(m.cast::<c_void>(), 0, THREAD_RESTART);
                }
            }
        }

        // SAFETY: the copy object is live and locked.
        unsafe {
            (*copy_object).ref_count -= 1;
            (*copy_object).lock.unlock();
        }
        break;
    }

    // SAFETY: the C tunable is a plain static that lives for the kernel's
    // lifetime.
    if unsafe { glue::vm_fault_dirty_handling } != 0
        && protection.contains(VmProt::WRITE)
    {
        // SAFETY: the result page is live and busy.
        unsafe { (*m).set_dirty(true) };
    }

    Fault {
        result: VM_FAULT_SUCCESS,
        protection,
        result_page: m,
        top_page: first_m,
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
            let fault = loop {
                // SAFETY: the object is live; each attempt takes the lock
                // and paging reference the fault consumes, exactly as the C
                // do/while does.
                let fault = unsafe {
                    (*object).lock.lock();
                    paging_begin(object);
                    fault_page(
                        object,
                        (*entry.as_ptr())
                            .offset
                            .wrapping_add(va.wrapping_sub(start)),
                        VmProt::NONE,
                        true,
                        false,
                        VmProt::NONE,
                        false,
                        None,
                    )
                };
                if fault.result != VM_FAULT_RETRY {
                    break fault;
                }
            };
            if fault.result != VM_FAULT_SUCCESS {
                kpanic!("vm_fault_unwire", "vm_fault_unwire: failure");
            }

            let result_page = fault.result_page;
            // SAFETY: the fault returned the live, busy page and holds its
            // object lock.
            unsafe {
                (*ptr::addr_of_mut!(vm_page_queue_lock)).lock();
                vm_page::unwire(result_page);
                (*ptr::addr_of_mut!(vm_page_queue_lock)).unlock();
                page_wakeup_done(result_page);
                cleanup((*result_page).object, fault.top_page);
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
        let (src_page, src_top_page) = if src_object.is_null() {
            (ptr::null_mut(), ptr::null_mut())
        } else {
            let (page, top) = 'source: loop {
                // SAFETY: each attempt takes the source object's lock and
                // paging reference, which `fault_page()` consumes.
                let fault = unsafe {
                    (*src_object).lock.lock();
                    paging_begin(src_object);
                    fault_page(
                        src_object,
                        src_offset,
                        VmProt::READ,
                        false,
                        interruptible,
                        VmProt::READ,
                        false,
                        None,
                    )
                };
                match fault.result {
                    VM_FAULT_SUCCESS => {
                        break 'source (fault.result_page, fault.top_page);
                    }
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
            (page, top)
        };

        let (dst_page, dst_top_page) = 'dest: loop {
            // SAFETY: each attempt takes the destination object's lock and
            // paging reference, which `fault_page()` consumes.
            let fault = unsafe {
                (*dst_object).lock.lock();
                paging_begin(dst_object);
                fault_page(
                    dst_object,
                    dst_offset,
                    VmProt::WRITE,
                    false,
                    false,
                    VmProt::WRITE,
                    false,
                    None,
                )
            };
            match fault.result {
                VM_FAULT_SUCCESS => {
                    break 'dest (fault.result_page, fault.top_page);
                }
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
