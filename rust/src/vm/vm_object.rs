// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_object.c and vm/vm_object.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The virtual-memory object module, which `vm/vm_object.c` used to define
//! and `vm/vm_object.h` declares.

use crate::arch::i386::mp_desc::simple_lock_pause;
use crate::arch::i386::percpu::current_thread;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_SHIFT, PAGE_SIZE};
use crate::glue::{
    self, Panic, ipc_kobject_set, ipc_space_kernel,
    memory_manager_default_reference, memory_object_copy,
    memory_object_create, memory_object_init, memory_object_terminate,
    pmap_is_modified, pmap_page_protect, printf, thread_block,
    vm_fault_cleanup, vm_fault_page, vm_object_external_count,
    vm_page_fictitious_addr, vm_page_free, vm_page_grab_fictitious,
    vm_page_insert, vm_page_lookup, vm_page_more_fictitious,
    vm_page_queue_lock, vm_pageout_page, vm_stat,
};
use crate::ipc::{IpcPort, IpcSpace, ipc_port};
use crate::kern::debug::SoftDebugger;
use crate::kern::queue::{
    QueueEntry, queue_enter_tail, queue_init, queue_next, queue_remove_generic,
};
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_sleep, thread_wakeup_prim,
};
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::vm::error::Error;
use crate::vm::types::{Pmap, VmObject, VmPage, VmProt};
use crate::vm::{vm_external, vm_page, vm_resident};
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull, addr_of, addr_of_mut, null_mut};
use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, Ordering};

/// `VM_OBJECT_EVENT_*` of <vm/vm_object.h>: the `all_wanted` bit an event
/// waiting on the object sets.
const EVENT_INITIALIZED: u32 = 0;
const EVENT_PAGER_READY: u32 = 1;
const EVENT_PAGING_IN_PROGRESS: u32 = 2;

/// `IKOT_*` of <kern/ipc_kobject.h>.
const IKOT_NONE: c_uint = 0;
const IKOT_PAGER: c_uint = 8;
const IKOT_PAGING_REQUEST: c_uint = 9;
const IKOT_PAGER_TERMINATING: c_uint = 15;
const IKOT_PAGING_NAME: c_uint = 16;

/// `MEMORY_OBJECT_COPY_*` of <mach/memory_object.h>: the copy strategies
/// `vm_object_copy_strategically()` dispatches over.
const MEMORY_OBJECT_COPY_NONE: c_int = 0;
const MEMORY_OBJECT_COPY_CALL: c_int = 1;
const MEMORY_OBJECT_COPY_DELAY: c_int = 2;

/// `VM_FAULT_*` of <vm/vm_fault.h>, the values `vm_fault_page()` returns.
const VM_FAULT_SUCCESS: c_int = 0;
const VM_FAULT_RETRY: c_int = 1;
const VM_FAULT_INTERRUPTED: c_int = 2;
const VM_FAULT_MEMORY_SHORTAGE: c_int = 3;
const VM_FAULT_FICTITIOUS_SHORTAGE: c_int = 4;
const VM_FAULT_MEMORY_ERROR: c_int = 5;

/// `VM_MAX_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS` of
/// <machine/vm_param.h>, the size the kernel object and the submap
/// placeholder are created with.
#[cfg(target_arch = "x86_64")]
const KERNEL_OBJECT_SIZE: VmSize = 0x7fff_ffff;
#[cfg(target_arch = "x86")]
const KERNEL_OBJECT_SIZE: VmSize = 0x3fff_ffff;

/// `vm_object_cache` of vm/vm_object.c: the `struct vm_object` slab cache.
#[unsafe(export_name = "vm_object_cache")]
static mut VM_OBJECT_CACHE: KmemCache = KmemCache::zeroed();

/// `vm_object_cached_list`: the objects whose `can_persist` kept them after
/// their last reference went away.
#[unsafe(export_name = "vm_object_cached_list")]
static mut VM_OBJECT_CACHED_LIST: QueueEntry = QueueEntry::unlinked();

/// `vm_object_cached_lock_data`: serializes the cached list and the port
/// associations.
static VM_OBJECT_CACHED_LOCK: crate::kern::lock::SimpleLock =
    crate::kern::lock::SimpleLock::new();

/// `vm_object_template`: the image `_vm_object_setup()` copies into a fresh
/// object.
#[unsafe(export_name = "vm_object_template")]
static mut VM_OBJECT_TEMPLATE: VmObject = VmObject::zeroed();

/// `kernel_object_store`, file-private in the C.
static mut KERNEL_OBJECT_STORE: VmObject = VmObject::zeroed();

/// `vm_submap_object_store`, file-private in vm/vm_map_glue.c.
static mut VM_SUBMAP_OBJECT_STORE: VmObject = VmObject::zeroed();

/// `vm_submap_object` of <vm/vm_map.h>: the placeholder object dropped into
/// a submap's range until `vm_map_submap()` creates the submap.
#[unsafe(no_mangle)]
pub static mut vm_submap_object: *mut VmObject =
    &raw mut VM_SUBMAP_OBJECT_STORE;

/// `kernel_object` of <vm/vm_object.h>: the single object all wired-down
/// kernel memory belongs to.
#[unsafe(no_mangle)]
pub static mut kernel_object: *mut VmObject = &raw mut KERNEL_OBJECT_STORE;

/// `vm_object_pmap_protect_by_page` of vm/vm_object.c.
#[unsafe(export_name = "vm_object_pmap_protect_by_page")]
static VM_OBJECT_PMAP_PROTECT_BY_PAGE: AtomicI32 = AtomicI32::new(0);

/// `object_collapses` and `object_bypasses` of vm/vm_object.c: debugging
/// counters, written for a debugger to read and never synchronized against.
#[unsafe(export_name = "object_collapses")]
static OBJECT_COLLAPSES: AtomicIsize = AtomicIsize::new(0);
#[unsafe(export_name = "object_bypasses")]
static OBJECT_BYPASSES: AtomicIsize = AtomicIsize::new(0);

/// `vm_object_collapse_debug`, `vm_object_collapse_allowed` and
/// `vm_object_collapse_bypass_allowed`: the collapse switch a debugger sets.
#[unsafe(export_name = "vm_object_collapse_debug")]
static VM_OBJECT_COLLAPSE_DEBUG: AtomicI32 = AtomicI32::new(0);
#[unsafe(export_name = "vm_object_collapse_allowed")]
static VM_OBJECT_COLLAPSE_ALLOWED: AtomicI32 = AtomicI32::new(1);
#[unsafe(export_name = "vm_object_collapse_bypass_allowed")]
static VM_OBJECT_COLLAPSE_BYPASS_ALLOWED: AtomicI32 = AtomicI32::new(1);

/// `vm_object_page_remove_lookup` and `vm_object_page_remove_iterate` of
/// vm/vm_object.c: how each removal path was taken.
#[unsafe(export_name = "vm_object_page_remove_lookup")]
static PAGE_REMOVE_LOOKUP: AtomicU32 = AtomicU32::new(0);
#[unsafe(export_name = "vm_object_page_remove_iterate")]
static PAGE_REMOVE_ITERATE: AtomicU32 = AtomicU32::new(0);

/// `panic()` of vm/vm_object.c at the caller's line.
#[track_caller]
fn die(func: &'static CStr, message: &'static CStr) -> ! {
    let location = core::panic::Location::caller();
    // SAFETY: `Panic` does not return; the file, function and message are
    // this module's, and the line fits the `c_int` the format takes.
    unsafe {
        Panic(
            c"rust/src/vm/vm_object.rs".as_ptr(),
            location.line() as c_int,
            func.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// [`KmemCache::alloc`] of the object cache.
unsafe fn cache_alloc() -> *mut VmObject {
    // SAFETY: the caller runs after `vm_object_bootstrap()`, so the cache is
    // live, and its own lock serializes the call.
    let Some(buf) = (unsafe { (*addr_of_mut!(VM_OBJECT_CACHE)).alloc() })
    else {
        return null_mut();
    };
    buf.as_ptr().cast::<VmObject>()
}

/// `kmem_cache_free(&vm_object_cache, object)` of the C.
///
/// # Safety
///
/// `object` must be a dead object that came from [`cache_alloc()`] and has
/// no other holder.
unsafe fn cache_free(object: *mut VmObject) {
    // SAFETY: the caller promises the dead, owned object.
    unsafe {
        (*addr_of_mut!(VM_OBJECT_CACHE))
            .free(NonNull::new_unchecked(object.cast::<u8>()))
    };
}

/// `vm_object_cached_lock_data` acquisition.
///
/// # Safety
///
/// Nothing else may hold the lock in this thread, and the caller must keep
/// it until it unlocks.
unsafe fn cache_lock() {
    // SAFETY: the caller promises the lock is free for this thread.
    unsafe { (*addr_of!(VM_OBJECT_CACHED_LOCK)).lock() };
}

/// `vm_object_cached_lock_data` release.
///
/// # Safety
///
/// This thread must hold the lock.
unsafe fn cache_unlock() {
    // SAFETY: the caller promises this thread holds the lock.
    unsafe { (*addr_of!(VM_OBJECT_CACHED_LOCK)).unlock() };
}

/// `vm_object_cache_add()` of the C.
///
/// # Safety
///
/// The cache lock and the object lock must be held.
unsafe fn cache_add(object: *mut VmObject) {
    // SAFETY: the caller holds the cache lock; the object is live and its
    // `cached_list` is not linked into any queue.
    unsafe {
        queue_enter_tail(
            addr_of_mut!(VM_OBJECT_CACHED_LIST),
            object.cast(),
            offset_of!(VmObject, cached_list),
        );
        (*object).set_cached(true);
    }
}

/// `vm_object_cache_remove()` of the C.
///
/// # Safety
///
/// The cache lock and the object lock must be held, and the object must be
/// on the cached list.
unsafe fn cache_remove(object: *mut VmObject) {
    // SAFETY: the caller holds the cache lock and the object was linked by
    // `cache_add()`.
    unsafe {
        queue_remove_generic(
            addr_of_mut!(VM_OBJECT_CACHED_LIST),
            object.cast(),
            offset_of!(VmObject, cached_list),
        );
        (*object).set_cached(false);
    }
}

/// `IP_VALID()` of <ipc/ipc_object.h>.
fn port_valid(port: *mut c_void) -> bool {
    !port.is_null() && port as usize != usize::MAX
}

/// The event key `(vm_offset_t) object + event` every object wait uses.
fn event_ptr(object: *const VmObject, event: u32) -> *mut c_void {
    // The event is at most three, far below the object's own alignment.
    object
        .cast::<u8>()
        .wrapping_add(event as usize)
        .cast_mut()
        .cast::<c_void>()
}

/// `atop()` of `<vm/vm_page.h>`.
const fn atop(address: VmOffset) -> usize {
    address >> PAGE_SHIFT
}

/// The `VmPage` a `memq` link names.
///
/// Mach's queue macros store the container pointer in the links, not the
/// address of the chain field, so the entry a `queue_first()` returns is
/// already the `VmPage`.
///
/// # Safety
///
/// `entry` must be the container pointer a `memq` walk produced.
unsafe fn page_of(entry: *mut QueueEntry) -> *mut VmPage {
    entry.cast::<VmPage>()
}

/// The link after the container `entry`, or `None` when it is the queue's
/// last.
///
/// # Safety
///
/// `entry` must be linked into the queue headed by `head`, through the
/// `VmPage.listq` field.
unsafe fn next_entry(
    head: *mut QueueEntry,
    entry: *mut QueueEntry,
) -> Option<NonNull<QueueEntry>> {
    let chain = unsafe { addr_of_mut!((*entry.cast::<VmPage>()).listq) };
    // SAFETY: the caller promises the linked container.
    let next = unsafe { queue_next(chain) };
    if next == head {
        None
    } else {
        NonNull::new(next)
    }
}

/// `vm_object_wait()` of <vm/vm_object.h>.
///
/// # Safety
///
/// The object lock must be held; the call releases it as the C macro did.
unsafe fn wait(object: *mut VmObject, event: u32, interruptible: bool) {
    // SAFETY: the caller holds the object lock, and the object is live.
    unsafe {
        (*object).want(event);
        thread_sleep(
            event_ptr(object, event),
            addr_of_mut!((*object).lock),
            c_int::from(interruptible),
        );
    }
}

/// `vm_object_assert_wait()` of <vm/vm_object.h>.
///
/// # Safety
///
/// The object lock must be held.
unsafe fn assert_wait_event(
    object: *mut VmObject,
    event: u32,
    interruptible: bool,
) {
    // SAFETY: the caller holds the object lock, and the object is live.
    unsafe {
        (*object).want(event);
        assert_wait(event_ptr(object, event), c_int::from(interruptible));
    }
}

/// `vm_object_wakeup()` of <vm/vm_object.h>.
///
/// # Safety
///
/// The object lock must be held.
unsafe fn wakeup(object: *mut VmObject, event: u32) {
    // SAFETY: the caller holds the object lock, and the object is live.
    unsafe {
        if (*object).wants(event) {
            thread_wakeup_prim(event_ptr(object, event), 0, THREAD_AWAKENED);
        }
        (*object).clear_want(event);
    }
}

/// `vm_object_paging_begin()` of <vm/vm_object.h>.
///
/// # Safety
///
/// The object lock must be held.
pub(crate) unsafe fn paging_begin(object: *mut VmObject) {
    // SAFETY: the caller holds the object lock, and the object is live.
    unsafe {
        (*object).set_paging_in_progress((*object).paging_in_progress() + 1)
    };
}

/// `vm_object_paging_end()` of <vm/vm_object.h>.
///
/// # Safety
///
/// The object lock must be held.
pub(crate) unsafe fn paging_end(object: *mut VmObject) {
    // SAFETY: the caller holds the object lock, and the object is live.
    unsafe {
        let count = (*object).paging_in_progress() - 1;
        (*object).set_paging_in_progress(count);
        if count == 0 {
            wakeup(object, EVENT_PAGING_IN_PROGRESS);
        }
    }
}

/// `vm_object_paging_wait()` of <vm/vm_object.h>.
///
/// # Safety
///
/// The object lock must be held; the call may drop and retake it.
unsafe fn paging_wait(object: *mut VmObject, interruptible: bool) {
    while unsafe { (*object).paging_in_progress() } != 0 {
        // SAFETY: the caller holds the object lock.
        unsafe {
            wait(object, EVENT_PAGING_IN_PROGRESS, interruptible);
            (*object).lock.lock();
        }
    }
}

/// `vm_map_glue_object_make_shared()` in C.
///
/// # Safety
///
/// `object` must be a live object.
pub(crate) unsafe fn make_shared(object: *mut VmObject) {
    // SAFETY: the caller promises a live object.
    unsafe {
        (*object).lock.lock();
        (*object).set_use_shared_copy(true);
        (*object).ref_count += 1;
        (*object).lock.unlock();
    }
}

/// `VM_PAGE_FREE()` of <vm/vm_page.h>.
///
/// # Safety
///
/// `page` must be a live page that the caller owns, and the page-queues lock
/// must not be held.
unsafe fn page_free(page: *mut VmPage) {
    // SAFETY: the caller promises the live page; the queue lock is the C
    // macro's, and `vm_page_free()` is the C's.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).lock();
        vm_page_free(page);
        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }
}

/// `PAGE_WAKEUP()` of <vm/vm_page.h>.
///
/// # Safety
///
/// `page` must be a live page whose object lock the caller holds.
unsafe fn page_wakeup(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and its lock.
    unsafe {
        if (*page).is_wanted() {
            (*page).set_wanted(false);
            thread_wakeup_prim(page.cast(), 0, THREAD_AWAKENED);
        }
    }
}

/// `PAGE_WAKEUP_DONE()` of <vm/vm_page.h>.
///
/// # Safety
///
/// `page` must be a live page whose object lock the caller holds.
unsafe fn page_wakeup_done(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and its lock.
    unsafe {
        (*page).set_busy(false);
        page_wakeup(page);
    }
}

/// `_vm_object_setup()` of the C: stamp the template on a fresh object.
///
/// # Safety
///
/// `object` must be writable storage for a `VmObject` that no other thread
/// can see yet.
unsafe fn setup(object: *mut VmObject, size: VmSize) {
    // SAFETY: the caller promises the unshared storage; the template is
    // live for the kernel's lifetime.
    unsafe {
        ptr::copy_nonoverlapping(addr_of!(VM_OBJECT_TEMPLATE), object, 1);
        queue_init(addr_of_mut!((*object).memq));
        (*object).lock.init();
        (*object).size = size;
    }
}

/// `_vm_object_allocate()` of the C.
unsafe fn allocate_internal(size: VmSize) -> *mut VmObject {
    // SAFETY: the caller runs in a context where the cache may allocate.
    let object = unsafe { cache_alloc() };
    if object.is_null() {
        return null_mut();
    }
    // SAFETY: the fresh object is unshared storage.
    unsafe { setup(object, size) };
    object
}

/// `vm_object_allocate()` of the C.
///
/// # Safety
///
/// The slab allocator and the IPC space must be up; the C panicked when
/// either allocation failed.
pub(crate) unsafe fn allocate(size: VmSize) -> Option<NonNull<VmObject>> {
    // SAFETY: the caller runs after `vm_object_bootstrap()`.
    let object = unsafe { allocate_internal(size) };
    let object = NonNull::new(object)
        .unwrap_or_else(|| die(c"vm_object_allocate", c"vm_object_allocate"));

    // SAFETY: the caller runs after the IPC package is up; the port is the
    // object's name port, as in the C.
    unsafe {
        let port =
            ipc_port::alloc_special(IpcSpace::from_raw(ipc_space_kernel))
                .map_or(ptr::null_mut(), IpcPort::as_ptr);
        if port.is_null() {
            die(c"vm_object_allocate", c"vm_object_allocate");
        }
        (*object.as_ptr()).pager_name = port;
        ipc_kobject_set(port, object.as_ptr().addr(), IKOT_PAGING_NAME);
    }

    Some(object)
}

/// `vm_object_bootstrap()` of the C.
pub(crate) fn bootstrap() {
    // SAFETY: the call runs once in the bootstrap sequence, after the slab
    // package is up and before any allocation from the cache.
    unsafe {
        (*addr_of_mut!(VM_OBJECT_CACHE)).init(
            b"vm_object",
            size_of::<VmObject>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
        queue_init(addr_of_mut!(VM_OBJECT_CACHED_LIST));
    }

    // SAFETY: no object exists yet, so the template is unshared; the C's
    // assignments are written in the declaration order.
    unsafe {
        let template = addr_of_mut!(VM_OBJECT_TEMPLATE);
        (*template).ref_count = 1;
        (*template).size = 0;
        (*template).resident_page_count = 0;
        (*template).copy = null_mut();
        (*template).shadow = null_mut();
        (*template).shadow_offset = 0;
        (*template).pager = null_mut();
        (*template).paging_offset = 0;
        (*template).pager_request = null_mut();
        (*template).pager_name = null_mut();
        (*template).set_pager_created(false);
        (*template).set_pager_initialized(false);
        (*template).set_pager_ready(false);
        (*template).copy_strategy = MEMORY_OBJECT_COPY_NONE;
        (*template).set_use_shared_copy(false);
        (*template).set_shadowed(false);
        (*template).absent_count = 0;
        (*template).all_wanted = 0;
        (*template).set_paging_in_progress(0);
        (*template).set_used_for_pageout(false);
        (*template).set_can_persist(false);
        (*template).set_cached(false);
        (*template).set_internal(true);
        (*template).set_temporary(true);
        (*template).set_alive(true);
        (*template).last_alloc = 0;
        (*template).existence_info = null_mut();
    }

    // SAFETY: the two objects are the boot storage; nothing else has their
    // addresses yet.
    unsafe {
        setup(addr_of_mut!(KERNEL_OBJECT_STORE), KERNEL_OBJECT_SIZE);
        setup(addr_of_mut!(VM_SUBMAP_OBJECT_STORE), KERNEL_OBJECT_SIZE);
    }

    // SAFETY: the external map caches are part of the same bootstrap.
    unsafe { vm_external::vm_external_module_initialize() };
}

/// `vm_object_init()` of the C.
pub(crate) fn init() {
    // SAFETY: the kernel object is the boot storage, live and unshared at
    // this point in the sequence.
    unsafe {
        let object = addr_of_mut!(KERNEL_OBJECT_STORE);
        let port =
            ipc_port::alloc_special(IpcSpace::from_raw(ipc_space_kernel))
                .map_or(ptr::null_mut(), IpcPort::as_ptr);
        (*object).pager_name = port;
        ipc_kobject_set(port, object.addr(), IKOT_PAGING_NAME);
    }
}

/// `vm_object_collect()` of the C.
///
/// # Safety
///
/// The object lock must be held, and the caller must own a reference to the
/// object.
pub(crate) unsafe fn collect(object: *mut VmObject) {
    // SAFETY: the caller holds the object lock.
    unsafe {
        (*object).lock.unlock();
        cache_lock();
        (*object).lock.lock();
        if !(*object).is_collectable() {
            (*object).lock.unlock();
            cache_unlock();
            return;
        }
        cache_remove(object);
        terminate(object);
    }
}

/// `vm_object_reference()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object.
pub(crate) unsafe fn reference(object: *mut VmObject) {
    if object.is_null() {
        return;
    }
    // SAFETY: the caller promises the live object; its lock guards the
    // count.
    unsafe {
        (*object).lock.lock();
        (*object).ref_count += 1;
        (*object).lock.unlock();
    }
}

/// `vm_object_deallocate()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object the caller holds a reference to;
/// no lock of the caller's may be held.
pub(crate) unsafe fn deallocate(object: *mut VmObject) {
    let mut object = object;
    while !object.is_null() {
        // SAFETY: the caller promises the live object; the cache lock comes
        // first, as the C's ordering requires.
        unsafe {
            cache_lock();
            (*object).lock.lock();
            (*object).ref_count -= 1;
            if (*object).ref_count > 0 {
                (*object).lock.unlock();
                cache_unlock();
                return;
            }

            let persist =
                (*object).can_persist() && (*object).resident_page_count > 0;
            if persist {
                cache_add(object);
                cache_unlock();
                (*object).lock.unlock();
                return;
            }

            if (*object).is_pager_created()
                && !(*object).is_pager_initialized()
            {
                (*object).ref_count += 1;
                assert_wait_event(object, EVENT_INITIALIZED, false);
                (*object).lock.unlock();
                cache_unlock();
                thread_block(None);
                continue;
            }

            let shadow = (*object).shadow;
            terminate(object);
            object = shadow;
        }
    }
}

/// `vm_object_terminate()` of the C.
///
/// # Safety
///
/// On entry the object lock and the cache lock must be held, the object
/// must be live and out of references, and its shadow reference is left
/// alone; on exit the cache and the object are unlocked.
pub(crate) unsafe fn terminate(object: *mut VmObject) {
    // SAFETY: the caller holds both locks and the object is live.
    unsafe {
        (*object).set_alive(false);
        remove(object);
        cache_unlock();
    }

    let shadow = unsafe { (*object).shadow };
    if !shadow.is_null() {
        // SAFETY: the shadow is a live object the terminated object's chain
        // points at.
        unsafe {
            (*shadow).lock.lock();
            (*shadow).copy = null_mut();
            (*shadow).lock.unlock();
        }
    }

    // SAFETY: the object is dead but its lock is still held by the caller's
    // thread until the wait below.
    unsafe { paging_wait(object, false) };

    // SAFETY: the object is live until the free at the end.
    unsafe {
        let temporary = (*object).is_temporary();
        let pager = (*object).pager;
        let head = addr_of_mut!((*object).memq);

        if temporary || pager.is_null() {
            while !(*head).is_empty() {
                let Some(entry) = (*head).first() else { break };
                let page = page_of(entry.as_ptr());
                vm_page::check(page);
                page_free(page);
            }
        } else {
            while !(*head).is_empty() {
                let Some(entry) = (*head).first() else { break };
                let page = page_of(entry.as_ptr());
                vm_page::check(page);
                (*addr_of_mut!(vm_page_queue_lock)).lock();
                vm_page::queues_remove(page);
                (*addr_of_mut!(vm_page_queue_lock)).unlock();

                if (*page).is_absent() || (*page).is_private() {
                    page_free(page);
                    continue;
                }

                if !(*page).is_dirty() {
                    (*page)
                        .set_dirty(pmap_is_modified((*page).phys_addr) != 0);
                }

                if (*page).is_dirty() || (*page).is_precious() {
                    (*page).set_busy(true);
                    vm_pageout_page(page, 0, 1);
                } else {
                    page_free(page);
                }
            }
        }

        if !(*object).is_internal() {
            (*addr_of_mut!(vm_page_queue_lock)).lock();
            vm_object_external_count -= 1;
            (*addr_of_mut!(vm_page_queue_lock)).unlock();
        }

        (*object).lock.unlock();

        let pager = (*object).pager;
        if !pager.is_null() {
            memory_object_release(
                pager,
                (*object).pager_request,
                (*object).pager_name,
            );
        } else if !(*object).pager_name.is_null() {
            ipc_port::dealloc_special(IpcPort::from_raw((*object).pager_name));
        }

        vm_external::vm_external_destroy((*object).existence_info.cast());
    }
    // SAFETY: the object is dead and owned here, as the C's free required.
    unsafe { cache_free(object) };
}

/// `vm_object_pager_wakeup()` of the C.
///
/// # Safety
///
/// `pager` must be null or a live port.
pub(crate) unsafe fn pager_wakeup(pager: *mut c_void) {
    let Some(port) = IpcPort::new(pager) else {
        return;
    };

    // SAFETY: the caller promises the live port; the cache lock guards the
    // port association, as in the C.
    unsafe {
        cache_lock();
        let someone_waiting = !port.kobject().is_null();
        if port.is_active() {
            ipc_kobject_set(pager, 0, IKOT_NONE);
        }
        cache_unlock();
        if someone_waiting {
            thread_wakeup_prim(pager, 0, THREAD_AWAKENED);
        }
    }
}

/// `memory_object_release()` of the C.
///
/// # Safety
///
/// `pager` must be null or a live memory-object port; `pager_request` and
/// `pager_name` are its ports, consumed by the call.
pub(crate) unsafe fn memory_object_release(
    pager: *mut c_void,
    pager_request: *mut c_void,
    pager_name: *mut c_void,
) {
    let Some(port) = IpcPort::new(pager) else {
        return;
    };

    // SAFETY: the caller promises the live port; the reference keeps it
    // alive across the terminate, as the C's `ip_reference` did.
    unsafe {
        port.reference();
        memory_object_terminate(pager, pager_request, pager_name);
        pager_wakeup(pager);
        port.release();
    }
}

/// `vm_object_abort_activity()` of the C.
///
/// # Safety
///
/// The object lock must be held.
unsafe fn abort_activity(object: *mut VmObject) {
    // SAFETY: the caller holds the object lock; the pages on `memq` are
    // live.
    unsafe {
        let head = addr_of_mut!((*object).memq);
        let mut entry = (*head).first();
        while let Some(e) = entry {
            let next = next_entry(head, e.as_ptr());
            let page = page_of(e.as_ptr());

            if (*page).is_busy() && (*page).is_absent() {
                page_free(page);
            } else {
                if (*page).unlock_request() != VmProt::NONE {
                    (*page).set_unlock_request(VmProt::NONE);
                }
                page_wakeup(page);
            }

            entry = next;
        }

        (*object).set_pager_ready(true);
        wakeup(object, EVENT_PAGER_READY);
    }
}

/// `memory_object_destroy()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object, and the caller must own a
/// reference to it.
pub(crate) unsafe fn memory_object_destroy(object: *mut VmObject) {
    if object.is_null() {
        return;
    }

    // SAFETY: the caller promises the live object; the cache lock comes
    // before the object lock, as in the C.
    unsafe {
        cache_lock();
        (*object).lock.lock();
        remove(object);
        (*object).set_can_persist(false);
        cache_unlock();

        let old_object = (*object).pager;
        (*object).pager = null_mut();
        let old_control = (*object).pager_request;
        (*object).pager_request = null_mut();
        let old_name = (*object).pager_name;
        (*object).pager_name = null_mut();

        paging_wait(object, false);
        (*object).lock.unlock();

        if !old_object.is_null() {
            memory_object_release(old_object, old_control, old_name);
        } else if !old_name.is_null() {
            // The C reads the already-cleared field here; the null is kept
            // so the two halves behave alike.
            ipc_port::dealloc_special(IpcPort::from_raw((*object).pager_name));
        }

        deallocate(object);
    }
}

/// `vm_object_pmap_protect()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object; `pmap` must be null or a live
/// physical map, as the C's callers guaranteed.
pub(crate) unsafe fn pmap_protect(
    mut object: *mut VmObject,
    mut offset: VmOffset,
    size: VmSize,
    pmap: *mut Pmap,
    pmap_start: VmOffset,
    prot: VmProt,
) {
    if object.is_null() {
        return;
    }

    // SAFETY: the caller promises the live object.
    unsafe { (*object).lock.lock() };

    loop {
        // SAFETY: the object is live and locked.
        unsafe {
            if (*object).resident_page_count > atop(size) / 2
                && !pmap.is_null()
            {
                (*object).lock.unlock();
                glue::pmap_protect(
                    pmap,
                    pmap_start,
                    pmap_start.wrapping_add(size),
                    prot.bits(),
                );
                return;
            }

            let end = offset.wrapping_add(size);
            let head = addr_of_mut!((*object).memq);
            let mut entry = (*head).first();
            while let Some(e) = entry {
                let page = page_of(e.as_ptr());
                if !(*page).is_fictitious()
                    && offset <= (*page).offset
                    && (*page).offset < end
                {
                    if pmap.is_null()
                        || VM_OBJECT_PMAP_PROTECT_BY_PAGE
                            .load(Ordering::Relaxed)
                            != 0
                    {
                        pmap_page_protect(
                            (*page).phys_addr,
                            prot.bits() & !(*page).page_lock().bits(),
                        );
                    } else {
                        let start =
                            pmap_start.wrapping_add((*page).offset - offset);
                        glue::pmap_protect(
                            pmap,
                            start,
                            start.wrapping_add(PAGE_SIZE),
                            prot.bits(),
                        );
                    }
                }
                entry = next_entry(head, e.as_ptr());
            }

            if prot == VmProt::NONE {
                let next_object = (*object).shadow;
                if next_object.is_null() {
                    break;
                }
                offset = offset.wrapping_add((*object).shadow_offset);
                (*next_object).lock.lock();
                (*object).lock.unlock();
                object = next_object;
                continue;
            }
            break;
        }
    }

    // SAFETY: the lock was taken above and the chain walk keeps it held.
    unsafe { (*object).lock.unlock() };
}

/// `vm_object_pmap_remove()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object, and the caller must hold no lock
/// of it.
pub(crate) unsafe fn pmap_remove(
    mut object: *mut VmObject,
    mut start: VmOffset,
    mut end: VmOffset,
) {
    if object.is_null() {
        return;
    }

    // SAFETY: the caller promises the live object.
    unsafe { (*object).lock.lock() };

    loop {
        // SAFETY: the object is live and locked.
        unsafe {
            let head = addr_of_mut!((*object).memq);
            let mut entry = (*head).first();
            while let Some(e) = entry {
                let page = page_of(e.as_ptr());
                if !(*page).is_fictitious()
                    && start <= (*page).offset
                    && (*page).offset < end
                {
                    pmap_page_protect((*page).phys_addr, VmProt::NONE.bits());
                }
                entry = next_entry(head, e.as_ptr());
            }

            let shadow = (*object).shadow;
            if shadow.is_null() {
                break;
            }
            let prev_object = object;
            start = start.wrapping_add((*object).shadow_offset);
            end = end.wrapping_add((*object).shadow_offset);
            object = shadow;
            (*object).lock.lock();
            (*prev_object).lock.unlock();
        }
    }

    // SAFETY: the lock was taken above and the chain walk keeps it held.
    unsafe { (*object).lock.unlock() };
}

/// `vm_object_copy_slowly()` of the C.
///
/// # Safety
///
/// On entry the source object must be live, locked, and hold a reference;
/// on exit it is unlocked.  The returned object is owned by the caller.
pub(crate) unsafe fn copy_slowly(
    src_object: *mut VmObject,
    src_offset: VmOffset,
    size: VmSize,
    interruptible: bool,
) -> Result<NonNull<VmObject>, Error> {
    if size == 0 {
        // SAFETY: the caller holds the source lock, as the C required.
        unsafe { (*src_object).lock.unlock() };
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises the live, locked object.
    unsafe {
        (*src_object).ref_count += 1;
        (*src_object).lock.unlock();
    }

    // SAFETY: the caller runs in a context where the allocator may block.
    let Some(new_object) = (unsafe { allocate(size) }) else {
        die(c"vm_object_copy_slowly", c"vm_object_allocate");
    };

    let mut src_offset = src_offset;
    let mut new_offset: VmOffset = 0;
    let mut remaining = size;

    while remaining != 0 {
        // SAFETY: the new object is live and owned here.
        let new_page = unsafe {
            (*new_object.as_ptr()).lock.lock();
            loop {
                match vm_resident::alloc(new_object, new_offset) {
                    Some(page) => break page,
                    None => {
                        (*new_object.as_ptr()).lock.unlock();
                        vm_page::wait(None);
                        (*new_object.as_ptr()).lock.lock();
                    }
                }
            }
        };
        // SAFETY: the page was allocated for this object and the lock
        // guarded the allocation.
        unsafe { (*new_object.as_ptr()).lock.unlock() };

        let mut result;
        loop {
            // SAFETY: the source object is live and unlocked between the
            // page allocations, as the C's lock protocol has it.
            unsafe {
                (*src_object).lock.lock();
                paging_begin(src_object);

                let mut prot = VmProt::READ;
                let mut result_page: *mut VmPage = null_mut();
                let mut top_page: *mut VmPage = null_mut();
                result = vm_fault_page(
                    src_object,
                    src_offset,
                    VmProt::READ,
                    0,
                    c_int::from(interruptible),
                    &mut prot,
                    &mut result_page,
                    &mut top_page,
                    0,
                    None,
                );

                match result {
                    VM_FAULT_SUCCESS => {
                        (*(*result_page).object).lock.unlock();
                        vm_resident::copy(
                            NonNull::new_unchecked(result_page),
                            new_page,
                        );
                        (*new_page.as_ptr()).set_busy(false);
                        (*new_page.as_ptr()).set_dirty(true);
                        (*(*result_page).object).lock.lock();
                        page_wakeup_done(result_page);
                        (*addr_of_mut!(vm_page_queue_lock)).lock();
                        if !(*result_page).is_active()
                            && !(*result_page).is_inactive()
                        {
                            vm_page::activate(result_page);
                        }
                        vm_page::activate(new_page.as_ptr());
                        (*addr_of_mut!(vm_page_queue_lock)).unlock();
                        vm_fault_cleanup((*result_page).object, top_page);
                        break;
                    }
                    VM_FAULT_RETRY => (),
                    VM_FAULT_MEMORY_SHORTAGE => vm_page::wait(None),
                    VM_FAULT_FICTITIOUS_SHORTAGE => vm_page_more_fictitious(),
                    VM_FAULT_INTERRUPTED => {
                        vm_page_free(new_page.as_ptr());
                        deallocate(new_object.as_ptr());
                        deallocate(src_object);
                        return Err(Error::SendInterrupted);
                    }
                    VM_FAULT_MEMORY_ERROR => {
                        vm_page_free(new_page.as_ptr());
                        deallocate(new_object.as_ptr());
                        deallocate(src_object);
                        return Err(Error::MemoryError);
                    }
                    _ => (),
                }
            }
        }

        src_offset = src_offset.wrapping_add(PAGE_SIZE);
        new_offset = new_offset.wrapping_add(PAGE_SIZE);
        remaining = remaining.wrapping_sub(PAGE_SIZE);
    }

    // SAFETY: the extra reference taken above.
    unsafe { deallocate(src_object) };

    Ok(new_object)
}

/// What `vm_object_copy_temporary()` hands back when it can copy without
/// blocking.
pub(crate) struct TemporaryCopy {
    /// The object the caller's slot becomes.
    pub object: *mut VmObject,
    /// The source must make a shadow.
    pub src_needs_copy: bool,
    /// The destination must make a shadow.
    pub dst_needs_copy: bool,
}

/// `vm_object_copy_temporary()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object, unlocked.
pub(crate) unsafe fn copy_temporary(
    object: *mut VmObject,
) -> Option<TemporaryCopy> {
    if object.is_null() {
        return Some(TemporaryCopy {
            object: null_mut(),
            src_needs_copy: false,
            dst_needs_copy: false,
        });
    }

    // SAFETY: the caller promises the live, unlocked object.
    unsafe {
        (*object).lock.lock();

        if (*object).is_temporary() {
            if (*object).use_shared_copy() {
                (*object).lock.unlock();
                let object = copy_delayed(object).as_ptr();
                return Some(TemporaryCopy {
                    object,
                    src_needs_copy: false,
                    dst_needs_copy: true,
                });
            }

            (*object).ref_count += 1;
            (*object).set_shadowed(true);
            (*object).lock.unlock();

            return Some(TemporaryCopy {
                object,
                src_needs_copy: true,
                dst_needs_copy: true,
            });
        }

        // The C's `XXX Do something intelligent` arm changed nothing.
        (*object).lock.unlock();
    }

    None
}

/// `vm_object_copy_call()` of the C.
///
/// # Safety
///
/// The source object must be live and locked on entry and is unlocked on
/// exit; the returned object is owned by the caller.
unsafe fn copy_call(
    src_object: *mut VmObject,
    src_offset: VmOffset,
    size: VmSize,
) -> Result<NonNull<VmObject>, Error> {
    let src_end = src_offset.wrapping_add(size);

    // SAFETY: the kernel space is live for the kernel's lifetime.
    let new_memory_object = unsafe {
        ipc_port::alloc_special(IpcSpace::from_raw(ipc_space_kernel))
            .map_or(ptr::null_mut(), IpcPort::as_ptr)
    };
    if new_memory_object.is_null() {
        return Err(Error::ResourceShortage);
    }

    // SAFETY: the caller promises the live, locked source.
    unsafe {
        (*src_object).ref_count += 1;
        paging_begin(src_object);
        (*src_object).lock.unlock();

        ipc_port::make_send(IpcPort::from_raw(new_memory_object)).as_ptr();

        memory_object_copy(
            (*src_object).pager,
            (*src_object).pager_request,
            src_offset,
            size,
            new_memory_object,
        );

        (*src_object).lock.lock();
        paging_end(src_object);

        let head = addr_of_mut!((*src_object).memq);
        let mut entry = (*head).first();
        while let Some(e) = entry {
            let page = page_of(e.as_ptr());
            if !(*page).is_fictitious()
                && src_offset <= (*page).offset
                && (*page).offset < src_end
                && !(*page).page_lock().contains(VmProt::WRITE)
            {
                (*page).set_page_lock((*page).page_lock() | VmProt::WRITE);
                pmap_page_protect(
                    (*page).phys_addr,
                    (VmProt::ALL
                        & VmProt::from_bits(!(*page).page_lock().bits()))
                    .bits(),
                );
            }
            entry = next_entry(head, e.as_ptr());
        }

        (*src_object).lock.unlock();
    }

    // SAFETY: the new port carries the object the C looked up.
    let Some(new_object) = (unsafe { enter(new_memory_object, size, false) })
    else {
        die(c"vm_object_copy_call", c"vm_object_enter");
    };

    // SAFETY: the object is fresh and owned here.
    unsafe {
        (*new_object.as_ptr()).shadow = src_object;
        (*new_object.as_ptr()).shadow_offset = src_offset;
        ipc_port::release_send(IpcPort::from_raw(new_memory_object));
    }

    Ok(new_object)
}

/// `vm_object_copy_delayed()` of the C.
///
/// # Safety
///
/// The source object must be live and unlocked; the returned object is
/// owned by the caller.
pub(crate) unsafe fn copy_delayed(
    src_object: *mut VmObject,
) -> NonNull<VmObject> {
    // SAFETY: the caller promises the live, unlocked source.
    let Some(new_copy) = (unsafe { allocate((*src_object).size) }) else {
        die(c"vm_object_copy_delayed", c"vm_object_allocate");
    };

    // SAFETY: the source is live and unlocked on entry, as the C requires.
    unsafe {
        (*src_object).lock.lock();

        loop {
            let old_copy = (*src_object).copy;
            if !old_copy.is_null() {
                if !(*old_copy).lock.try_lock() {
                    (*src_object).lock.unlock();
                    simple_lock_pause();
                    (*src_object).lock.lock();
                    continue;
                }

                if (*old_copy).resident_page_count == 0
                    && !(*old_copy).is_pager_created()
                {
                    (*old_copy).ref_count += 1;
                    (*old_copy).lock.unlock();
                    (*src_object).lock.unlock();

                    deallocate(new_copy.as_ptr());

                    return NonNull::new_unchecked(old_copy);
                }

                (*src_object).ref_count -= 1;
                (*old_copy).shadow = new_copy.as_ptr();
                (*new_copy.as_ptr()).ref_count += 1;
                (*old_copy).lock.unlock();
            }

            (*new_copy.as_ptr()).shadow = src_object;
            (*new_copy.as_ptr()).shadow_offset = 0;
            (*new_copy.as_ptr()).set_shadowed(true);
            (*src_object).ref_count += 1;
            (*src_object).copy = new_copy.as_ptr();

            let head = addr_of_mut!((*src_object).memq);
            let mut entry = (*head).first();
            while let Some(e) = entry {
                let page = page_of(e.as_ptr());
                if !(*page).is_fictitious() {
                    pmap_page_protect(
                        (*page).phys_addr,
                        (VmProt::ALL
                            & VmProt::from_bits(!VmProt::WRITE.bits())
                            & VmProt::from_bits(!(*page).page_lock().bits()))
                        .bits(),
                    );
                }
                entry = next_entry(head, e.as_ptr());
            }

            (*src_object).lock.unlock();

            return new_copy;
        }
    }
}

/// What `copy_strategically()` wrote through the C's out-parameters before
/// returning.
pub(crate) enum StrategicResult {
    /// The copy succeeded and all three slots are set.
    Copied {
        object: NonNull<VmObject>,
        offset: VmOffset,
        needs_copy: bool,
    },
    /// The ready wait was interrupted: the C wrote null, zero and false.
    Interrupted,
    /// `copy_slowly()` failed after writing a null object; the other slots
    /// are untouched.
    NullObject(Error),
    /// The copy failed without touching the caller's slots.
    Failed(Error),
    /// The strategy was not one of the three: success, slots untouched.
    Unchanged,
}

/// `vm_object_copy_strategically()` of the C.
///
/// # Safety
///
/// The source object must be live and unlocked, and the caller must own a
/// reference to it.
pub(crate) unsafe fn copy_strategically(
    src_object: *mut VmObject,
    src_offset: VmOffset,
    size: VmSize,
) -> StrategicResult {
    let interruptible = true;

    // SAFETY: the caller promises the live, unlocked source.
    unsafe {
        (*src_object).lock.lock();

        while !(*src_object).is_pager_ready() {
            wait(src_object, EVENT_PAGER_READY, interruptible);
            if interruptible
                && (*current_thread()).wait_result != THREAD_AWAKENED
            {
                return StrategicResult::Interrupted;
            }
            (*src_object).lock.lock();
        }

        if (*src_object).is_temporary() {
            (*src_object).copy_strategy = MEMORY_OBJECT_COPY_DELAY;
        }

        match (*src_object).copy_strategy {
            MEMORY_OBJECT_COPY_NONE => {
                match copy_slowly(src_object, src_offset, size, interruptible)
                {
                    Ok(object) => StrategicResult::Copied {
                        object,
                        offset: 0,
                        needs_copy: false,
                    },
                    Err(error) => StrategicResult::NullObject(error),
                }
            }
            MEMORY_OBJECT_COPY_CALL => {
                match copy_call(src_object, src_offset, size) {
                    Ok(object) => StrategicResult::Copied {
                        object,
                        offset: 0,
                        needs_copy: false,
                    },
                    Err(error) => StrategicResult::Failed(error),
                }
            }
            MEMORY_OBJECT_COPY_DELAY => {
                (*src_object).lock.unlock();
                let object = copy_delayed(src_object);
                StrategicResult::Copied {
                    object,
                    offset: src_offset,
                    needs_copy: true,
                }
            }
            _ => StrategicResult::Unchanged,
        }
    }
}

/// `vm_object_shadow()` of the C.
///
/// # Safety
///
/// `source` must be null or a live object the caller holds a reference to;
/// the reference moves to the returned object.
pub(crate) unsafe fn shadow(
    source: *mut VmObject,
    offset: VmOffset,
    length: VmSize,
) -> NonNull<VmObject> {
    // SAFETY: the caller runs in a context where the allocator may block.
    let Some(result) = (unsafe { allocate(length) }) else {
        die(
            c"vm_object_shadow",
            c"vm_object_shadow: no object for shadowing",
        );
    };

    // SAFETY: the object is fresh and owned here.
    unsafe {
        (*result.as_ptr()).shadow = source;
        (*result.as_ptr()).shadow_offset = offset;
    }

    result
}

/// The `vm_object_lookup()`/`vm_object_lookup_name()` body.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
unsafe fn lookup_kotype(
    port: *mut c_void,
    kotype: c_uint,
) -> Option<NonNull<VmObject>> {
    if !port_valid(port) {
        return None;
    }
    let port = IpcPort::new(port)?;

    // SAFETY: `port_valid()` promises a live port.
    unsafe {
        port.lock();

        let mut object: *mut VmObject = null_mut();
        if port.is_active() && port.kotype() == kotype {
            cache_lock();
            object = port.kobject().cast::<VmObject>();
            (*object).lock.lock();

            if (*object).ref_count == 0 {
                cache_remove(object);
            }
            (*object).ref_count += 1;
            (*object).lock.unlock();
            cache_unlock();
        }

        port.unlock();
        NonNull::new(object)
    }
}

/// `vm_object_lookup()` of the C.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
pub(crate) unsafe fn lookup(port: *mut c_void) -> Option<NonNull<VmObject>> {
    // SAFETY: the caller promises the port.
    unsafe { lookup_kotype(port, IKOT_PAGING_REQUEST) }
}

/// `vm_object_lookup_name()` of the C.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
pub(crate) unsafe fn lookup_name(
    port: *mut c_void,
) -> Option<NonNull<VmObject>> {
    // SAFETY: the caller promises the port.
    unsafe { lookup_kotype(port, IKOT_PAGING_NAME) }
}

/// `vm_object_destroy()` of the C.
///
/// # Safety
///
/// `pager` must be null, dead, or a live memory-object port.
pub(crate) unsafe fn destroy(pager: *mut c_void) {
    let Some(port) = IpcPort::new(pager) else {
        return;
    };

    // SAFETY: the caller promises the live port.
    unsafe {
        cache_lock();
        if port.kotype() != IKOT_PAGER {
            cache_unlock();
            return;
        }

        let object = port.kobject().cast::<VmObject>();
        (*object).lock.lock();
        if (*object).ref_count == 0 {
            cache_remove(object);
        }
        (*object).ref_count += 1;
        (*object).set_can_persist(false);

        (*object).pager = null_mut();
        remove(object);
        let old_request = (*object).pager_request;
        (*object).pager_request = null_mut();
        let old_name = (*object).pager_name;
        (*object).pager_name = null_mut();

        (*object).lock.unlock();
        cache_unlock();

        ipc_port::release_send(IpcPort::from_raw(pager));
        if !old_request.is_null() {
            ipc_port::dealloc_special(IpcPort::from_raw(old_request));
        }
        if !old_name.is_null() {
            ipc_port::dealloc_special(IpcPort::from_raw(old_name));
        }

        (*object).lock.lock();
        abort_activity(object);
        (*object).lock.unlock();

        deallocate(object);
    }
}

/// `vm_object_enter()` of the C.
///
/// # Safety
///
/// `pager` must be null, dead, or a live port; the returned object is owned
/// by the caller.
pub(crate) unsafe fn enter(
    pager: *mut c_void,
    size: VmSize,
    internal: bool,
) -> Option<NonNull<VmObject>> {
    if !port_valid(pager) {
        // SAFETY: the caller runs in a context where the allocator may
        // block.
        return unsafe { allocate(size) };
    }

    let mut new_object: *mut VmObject = null_mut();
    let mut must_init = false;

    'restart: loop {
        // SAFETY: `port_valid()` promised a live port, and the cache lock
        // guards the association, as the C requires.
        unsafe {
            cache_lock();
            let mut po;
            loop {
                po = IpcPort::from_raw(pager).kotype();

                if po == IKOT_PAGER_TERMINATING {
                    IpcPort::from_raw(pager).set_kobject(pager);
                    assert_wait(pager, 0);
                    cache_unlock();
                    thread_block(None);
                    continue 'restart;
                }

                if po != IKOT_NONE {
                    break;
                }

                if new_object.is_null() {
                    cache_unlock();
                    let Some(object) = allocate(size) else {
                        die(c"vm_object_enter", c"vm_object_allocate");
                    };
                    new_object = object.as_ptr();
                    cache_lock();
                } else {
                    ipc_kobject_set(pager, new_object.addr(), IKOT_PAGER);
                    new_object = null_mut();
                    must_init = true;
                }
            }

            if internal {
                must_init = true;
            }

            let object = if po == IKOT_PAGER {
                IpcPort::from_raw(pager).kobject().cast::<VmObject>()
            } else {
                null_mut()
            };

            if !object.is_null() && !must_init {
                (*object).lock.lock();
                if (*object).ref_count == 0 {
                    cache_remove(object);
                }
                (*object).ref_count += 1;
                (*object).lock.unlock();

                vm_stat.hits += 1;
            }
            vm_stat.lookups += 1;
            cache_unlock();

            if !new_object.is_null() {
                deallocate(new_object);
            }

            if object.is_null() {
                return None;
            }

            if must_init {
                let pager = ipc_port::copy_send(pager);
                if !port_valid(pager) {
                    die(c"vm_object_enter", c"vm_object_enter: port died");
                }

                (*object).set_pager_created(true);
                (*object).pager = pager;

                let request = ipc_port::alloc_special(IpcSpace::from_raw(
                    ipc_space_kernel,
                ))
                .map_or(ptr::null_mut(), IpcPort::as_ptr);
                if request.is_null() {
                    die(
                        c"vm_object_enter",
                        c"vm_object_enter: pager request alloc",
                    );
                }
                (*object).pager_request = request;
                ipc_kobject_set(request, object.addr(), IKOT_PAGING_REQUEST);

                if internal {
                    let dmm = memory_manager_default_reference();
                    (*object).set_internal(true);
                    (*object).set_pager_ready(true);
                    memory_object_create(
                        dmm,
                        pager,
                        (*object).size,
                        (*object).pager_request,
                        (*object).pager_name,
                        PAGE_SIZE,
                    );
                } else {
                    (*object).set_internal(false);
                    (*object).set_temporary(false);
                    vm_object_external_count += 1;
                    (*object).set_pager_ready(false);
                    memory_object_init(
                        pager,
                        (*object).pager_request,
                        (*object).pager_name,
                        PAGE_SIZE,
                    );
                }

                (*object).lock.lock();
                (*object).set_pager_initialized(true);
                wakeup(object, EVENT_INITIALIZED);
            } else {
                (*object).lock.lock();
            }

            while !(*object).is_pager_initialized() {
                wait(object, EVENT_INITIALIZED, false);
                (*object).lock.lock();
            }
            (*object).lock.unlock();

            return NonNull::new(object);
        }
    }
}

/// `vm_object_pager_create()` of the C.
///
/// # Safety
///
/// The object must be live and locked on entry and is locked on exit.
pub(crate) unsafe fn pager_create(object: *mut VmObject) {
    // SAFETY: the caller promises the live, locked object.
    unsafe {
        if (*object).is_pager_created() {
            while !(*object).is_pager_initialized() {
                wait(object, EVENT_PAGER_READY, false);
                (*object).lock.lock();
            }
            return;
        }

        (*object).set_pager_created(true);
        paging_begin(object);
        (*object).lock.unlock();

        (*object).existence_info = vm_external::vm_external_create(
            (*object).size.wrapping_add((*object).paging_offset),
        )
        .cast();

        let pager =
            ipc_port::alloc_special(IpcSpace::from_raw(ipc_space_kernel))
                .map_or(ptr::null_mut(), IpcPort::as_ptr);
        if pager.is_null() {
            die(
                c"vm_object_pager_create",
                c"vm_object_pager_create: allocate pager port",
            );
        }

        ipc_port::make_send(IpcPort::from_raw(pager)).as_ptr();
        ipc_kobject_set(pager, object.addr(), IKOT_PAGER);

        if enter(pager, (*object).size, true).map(NonNull::as_ptr)
            != Some(object)
        {
            die(
                c"vm_object_pager_create",
                c"vm_object_pager_create: mismatch",
            );
        }

        ipc_port::release_send(IpcPort::from_raw(pager));

        (*object).lock.lock();
        paging_end(object);
    }
}

/// `vm_object_remove()` of the C.
///
/// # Safety
///
/// The cache lock must be held and `object` must be a live object.
pub(crate) unsafe fn remove(object: *mut VmObject) {
    // SAFETY: the caller holds the cache lock and the object is live.
    unsafe {
        let pager = (*object).pager;
        if !pager.is_null() {
            let port = IpcPort::from_raw(pager);
            let kotype = port.kotype();
            if kotype == IKOT_PAGER {
                ipc_kobject_set(pager, 0, IKOT_PAGER_TERMINATING);
            } else if kotype != IKOT_NONE {
                die(c"vm_object_remove", c"vm_object_remove: bad object port");
            }
        }

        let request = (*object).pager_request;
        if !request.is_null() {
            let port = IpcPort::from_raw(request);
            let kotype = port.kotype();
            if kotype == IKOT_PAGING_REQUEST {
                ipc_kobject_set(request, 0, IKOT_NONE);
            } else if kotype != IKOT_NONE {
                die(
                    c"vm_object_remove",
                    c"vm_object_remove: bad request port",
                );
            }
        }

        let name = (*object).pager_name;
        if !name.is_null() {
            let port = IpcPort::from_raw(name);
            let kotype = port.kotype();
            if kotype == IKOT_PAGING_NAME {
                ipc_kobject_set(name, 0, IKOT_NONE);
            } else if kotype != IKOT_NONE {
                die(c"vm_object_remove", c"vm_object_remove: bad name port");
            }
        }
    }
}

/// `vm_object_collapse()` of the C.
///
/// # Safety
///
/// The object lock must be held on entry and is held on exit; the caller
/// must own a reference to the object.
pub(crate) unsafe fn collapse(object: *mut VmObject) {
    if VM_OBJECT_COLLAPSE_ALLOWED.load(Ordering::Relaxed) == 0 {
        return;
    }

    loop {
        // SAFETY: the caller holds the object lock and a reference.
        unsafe {
            if object.is_null()
                || (*object).is_pager_created()
                || (*object).paging_in_progress() != 0
                || (*object).absent_count != 0
            {
                return;
            }

            let backing_object = (*object).shadow;
            if backing_object.is_null() {
                return;
            }

            (*backing_object).lock.lock();

            if !(*backing_object).is_internal()
                || (*backing_object).paging_in_progress() != 0
            {
                (*backing_object).lock.unlock();
                return;
            }

            if !(*backing_object).shadow.is_null()
                && !(*(*backing_object).shadow).copy.is_null()
            {
                (*backing_object).lock.unlock();
                return;
            }

            let backing_offset = (*object).shadow_offset;
            let size = (*object).size;

            if (*backing_object).ref_count == 1 {
                if !(*addr_of!(VM_OBJECT_CACHED_LOCK)).try_lock() {
                    (*backing_object).lock.unlock();
                    return;
                }

                let head = addr_of_mut!((*backing_object).memq);
                while !(*head).is_empty() {
                    let Some(entry) = (*head).first() else { break };
                    let page = page_of(entry.as_ptr());
                    let new_offset =
                        (*page).offset.wrapping_sub(backing_offset);

                    if (*page).offset < backing_offset || new_offset >= size {
                        page_free(page);
                    } else {
                        let pp = vm_page_lookup(object, new_offset);
                        if !pp.is_null() && !(*pp).is_absent() {
                            page_free(page);
                        } else {
                            vm_resident::rename(
                                NonNull::new_unchecked(page),
                                NonNull::new_unchecked(object),
                                new_offset,
                            );
                        }
                    }
                }

                match VM_OBJECT_COLLAPSE_DEBUG.load(Ordering::Relaxed) {
                    0 => (),
                    1 => {
                        if !(*backing_object).pager.is_null()
                            || !(*backing_object).pager_request.is_null()
                        {
                            collapse_print(backing_object, object);
                        }
                    }
                    debug => {
                        collapse_print(backing_object, object);
                        if debug > 2 {
                            SoftDebugger(c"vm_object_collapse".as_ptr());
                        }
                    }
                }

                (*object).pager = (*backing_object).pager;
                if !(*object).pager.is_null() {
                    ipc_kobject_set(
                        (*object).pager,
                        object.addr(),
                        IKOT_PAGER,
                    );
                }
                (*object).set_pager_initialized(
                    (*backing_object).is_pager_initialized(),
                );
                (*object).set_pager_ready((*backing_object).is_pager_ready());
                (*object)
                    .set_pager_created((*backing_object).is_pager_created());

                (*object).pager_request = (*backing_object).pager_request;
                if !(*object).pager_request.is_null() {
                    ipc_kobject_set(
                        (*object).pager_request,
                        object.addr(),
                        IKOT_PAGING_REQUEST,
                    );
                }
                let old_name_port = (*object).pager_name;
                if !old_name_port.is_null() {
                    ipc_kobject_set(old_name_port, 0, IKOT_NONE);
                }
                (*object).pager_name = (*backing_object).pager_name;
                if !(*object).pager_name.is_null() {
                    ipc_kobject_set(
                        (*object).pager_name,
                        object.addr(),
                        IKOT_PAGING_NAME,
                    );
                }

                cache_unlock();

                if !(*object).pager.is_null() {
                    (*object).paging_offset = (*backing_object)
                        .paging_offset
                        .wrapping_add(backing_offset);
                }

                (*object).existence_info = (*backing_object).existence_info;

                (*object).shadow = (*backing_object).shadow;
                (*object).shadow_offset = (*object)
                    .shadow_offset
                    .wrapping_add((*backing_object).shadow_offset);
                if !(*object).shadow.is_null()
                    && !(*(*object).shadow).copy.is_null()
                {
                    die(
                        c"vm_object_collapse",
                        c"vm_object_collapse: we collapsed a copy-object!",
                    );
                }

                (*backing_object).set_alive(false);
                (*backing_object).lock.unlock();

                (*object).lock.unlock();
                if !old_name_port.is_null() {
                    ipc_port::dealloc_special(IpcPort::from_raw(
                        old_name_port,
                    ));
                }
                cache_free(backing_object);
                (*object).lock.lock();

                OBJECT_COLLAPSES.fetch_add(1, Ordering::Relaxed);
            } else {
                if VM_OBJECT_COLLAPSE_BYPASS_ALLOWED.load(Ordering::Relaxed)
                    == 0
                {
                    (*backing_object).lock.unlock();
                    return;
                }

                if (*backing_object).is_pager_created() {
                    (*backing_object).lock.unlock();
                    return;
                }

                let head = addr_of_mut!((*backing_object).memq);
                let mut entry = (*head).first();
                while let Some(e) = entry {
                    let page = page_of(e.as_ptr());
                    let new_offset =
                        (*page).offset.wrapping_sub(backing_offset);

                    if (*page).offset >= backing_offset
                        && new_offset <= size
                        && vm_page_lookup(object, new_offset).is_null()
                    {
                        (*backing_object).lock.unlock();
                        return;
                    }
                    entry = next_entry(head, e.as_ptr());
                }

                (*object).shadow = (*backing_object).shadow;
                reference((*object).shadow);
                (*object).shadow_offset = (*object)
                    .shadow_offset
                    .wrapping_add((*backing_object).shadow_offset);

                if (*backing_object).copy == object {
                    (*backing_object).copy = null_mut();
                }

                (*backing_object).ref_count -= 1;
                (*backing_object).lock.unlock();

                OBJECT_BYPASSES.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The `printf` of `vm_object_collapse()`'s debug switch.
///
/// # Safety
///
/// Both objects must be live.
unsafe fn collapse_print(
    backing_object: *mut VmObject,
    object: *mut VmObject,
) {
    // SAFETY: the caller promises both objects live, and `printf` only
    // formats.
    unsafe {
        printf(
            c"vm_object_collapse: %p (pager %p, request %p) up to %p\n"
                .as_ptr(),
            backing_object,
            (*backing_object).pager,
            (*backing_object).pager_request,
            object,
        );
    }
}

/// `vm_object_page_remove()` of the C.
///
/// # Safety
///
/// The object must be live and locked.
pub(crate) unsafe fn page_remove(
    object: *mut VmObject,
    start: VmOffset,
    end: VmOffset,
) {
    // SAFETY: the caller holds the object lock and the object is live.
    unsafe {
        if atop(end.wrapping_sub(start)) < (*object).resident_page_count / 16 {
            PAGE_REMOVE_LOOKUP.fetch_add(1, Ordering::Relaxed);

            let mut start = start;
            while start < end {
                let page = vm_page_lookup(object, start);
                if !page.is_null() {
                    if !(*page).is_fictitious() {
                        pmap_page_protect(
                            (*page).phys_addr,
                            VmProt::NONE.bits(),
                        );
                    }
                    page_free(page);
                }
                start = start.wrapping_add(PAGE_SIZE);
            }
        } else {
            PAGE_REMOVE_ITERATE.fetch_add(1, Ordering::Relaxed);

            let head = addr_of_mut!((*object).memq);
            let mut entry = (*head).first();
            while let Some(e) = entry {
                let next = next_entry(head, e.as_ptr());
                let page = page_of(e.as_ptr());
                if start <= (*page).offset && (*page).offset < end {
                    if !(*page).is_fictitious() {
                        pmap_page_protect(
                            (*page).phys_addr,
                            VmProt::NONE.bits(),
                        );
                    }
                    page_free(page);
                }
                entry = next;
            }
        }
    }
}

/// `vm_object_coalesce()` of the C.
///
/// # Safety
///
/// Both objects must be null or live, and their references move exactly as
/// the C's did.
pub(crate) unsafe fn coalesce(
    prev_object: *mut VmObject,
    next_object: *mut VmObject,
    prev_offset: VmOffset,
    next_offset: VmOffset,
    prev_size: VmSize,
    next_size: VmSize,
) -> Option<(*mut VmObject, VmOffset)> {
    if prev_object == next_object {
        if prev_object.is_null() {
            return Some((null_mut(), 0));
        }

        if prev_offset.wrapping_add(prev_size) == next_offset {
            // SAFETY: the caller held a reference for each object, so one
            // of the two is dropped here, as the C drops it.
            unsafe { deallocate(prev_object) };
            return Some((prev_object, prev_offset));
        }

        return None;
    }

    let object = if !next_object.is_null() {
        if !prev_object.is_null() {
            return None;
        }
        next_object
    } else {
        prev_object
    };

    // SAFETY: the chosen object is live and unlocked, as the C requires.
    unsafe {
        (*object).lock.lock();
        collapse(object);

        if (*object).ref_count > 1
            || (*object).is_pager_created()
            || (*object).is_used_for_pageout()
            || !(*object).shadow.is_null()
            || !(*object).copy.is_null()
            || (*object).paging_in_progress() != 0
        {
            (*object).lock.unlock();
            return None;
        }

        let new_offset = if object == prev_object {
            page_remove(
                object,
                prev_offset.wrapping_add(prev_size),
                prev_offset.wrapping_add(prev_size).wrapping_add(next_size),
            );

            let newsize =
                prev_offset.wrapping_add(prev_size).wrapping_add(next_size);
            if newsize > (*object).size {
                (*object).size = newsize;
            }

            prev_offset
        } else {
            if next_offset < prev_size {
                (*object).lock.unlock();
                return None;
            }

            page_remove(
                object,
                next_offset.wrapping_sub(prev_size),
                next_offset,
            );

            next_offset.wrapping_sub(prev_size)
        };

        (*object).lock.unlock();
        Some((object, new_offset))
    }
}

/// `vm_object_name()` of the C.
///
/// # Safety
///
/// `object` must be null or a live object.
pub(crate) unsafe fn name(object: *mut VmObject) -> *mut c_void {
    if object.is_null() {
        return null_mut();
    }

    // SAFETY: the caller promises the live object.
    unsafe {
        (*object).lock.lock();

        let mut object = object;
        while !(*object).shadow.is_null() {
            let next = (*object).shadow;
            (*next).lock.lock();
            (*object).lock.unlock();
            object = next;
        }

        let mut port = (*object).pager_name;
        if !port.is_null() {
            port = ipc_port::make_send(IpcPort::from_raw(port)).as_ptr();
        }
        (*object).lock.unlock();

        port
    }
}

/// `vm_object_page_map()` of the C.
///
/// # Safety
///
/// `object` must be a live object, and `map_fn` must be the C callback that
/// names each page's physical address.
pub(crate) unsafe fn page_map(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    map_fn: Option<unsafe extern "C" fn(*mut c_void, VmOffset) -> VmOffset>,
    map_fn_data: *mut c_void,
) -> Result<(), Error> {
    let Some(map_fn) = map_fn else {
        die(
            c"vm_object_page_map",
            c"vm_object_page_map: no map function",
        );
    };

    let num_pages = atop(size);
    let mut offset = offset;

    for _ in 0..num_pages {
        // SAFETY: the caller promises the callback, and `map_fn_data` is
        // the private pointer it takes.
        let addr = unsafe { map_fn(map_fn_data, offset) };
        if addr == unsafe { vm_page_fictitious_addr } {
            return Err(Error::NoAccess);
        }

        let page = loop {
            // SAFETY: the caller runs in a context where the page allocator
            // may block.
            let page = unsafe { vm_page_grab_fictitious() };
            if !page.is_null() {
                break page;
            }
            // SAFETY: as above.
            unsafe { vm_page_more_fictitious() };
        };

        // SAFETY: the object is live, and the page is the fresh one just
        // grabbed.
        unsafe {
            (*object).lock.lock();

            let old_page = vm_page_lookup(object, offset);
            if !old_page.is_null() {
                page_free(old_page);
            }

            vm_resident::init(&mut *page);
            (*page).phys_addr = addr;
            (*page).set_private(true);
            (*page).set_wire_count(1);

            (*addr_of_mut!(vm_page_queue_lock)).lock();
            vm_page_insert(page, object, offset);
            (*addr_of_mut!(vm_page_queue_lock)).unlock();

            page_wakeup_done(page);
            (*object).lock.unlock();
        }

        offset = offset.wrapping_add(PAGE_SIZE);
    }

    Ok(())
}
