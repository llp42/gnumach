// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_resident.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Resident memory management, which `vm/vm_resident.c` used to define.

use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_SHIFT, PAGE_SIZE};
use crate::glue::{
    Panic, kernel_pmap, pmap_copy_page, pmap_enter, pmap_virtual_space,
    pmap_zero_page, printf, vm_page_bootalloc,
};
use crate::ipc::HashInfoBucket;
use crate::kern::list::{List, entry};
use crate::kern::lock::SimpleLock;
use crate::kern::queue::{queue_enter_tail, queue_remove_generic};
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::utils::cell::SyncCell;
use crate::vm::types::{VmObject, VmPage, VmProt};
use crate::vm::vm_map::{round_page, trunc_page};
use crate::vm::vm_page;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_int, c_uint, c_ushort};
use core::mem::{offset_of, size_of};
use core::ptr::{NonNull, addr_of_mut, null_mut};
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// `VM_PAGE_HIGHMEM` of <vm/vm_page.h>: the page may come from high physical
/// memory.
const VM_PAGE_HIGHMEM: c_uint = 0x08;

/// `VM_PAGE_DMA32` and `VM_PAGE_DIRECTMAP` of <vm/vm_page.h>: the flags that
/// ask for those segments.  The values follow the limit ordering, which the
/// 64-bit and the non-PAE 32-bit builds fix differently; the latter has no
/// DMA32 segment for `vm_page_grab()` to select.
#[cfg(target_arch = "x86_64")]
pub(crate) const VM_PAGE_DMA32: c_uint = 0x04;
#[cfg(target_arch = "x86_64")]
pub(crate) const VM_PAGE_DIRECTMAP: c_uint = 0x02;
#[cfg(target_arch = "x86")]
pub(crate) const VM_PAGE_DIRECTMAP: c_uint = 0x04;

/// `VM_PAGE_SEL_*` of <vm/vm_page.h>: the segment selectors
/// `vm_page_alloc_pa()` takes.  The DMA32 and DIRECTMAP indices swap with
/// the segment ordering.
#[cfg(target_arch = "x86_64")]
const VM_PAGE_SEL_DIRECTMAP: c_uint = 1;
#[cfg(target_arch = "x86_64")]
const VM_PAGE_SEL_DMA32: c_uint = 2;
#[cfg(target_arch = "x86")]
const VM_PAGE_SEL_DIRECTMAP: c_uint = 2;
const VM_PAGE_SEL_DMA: c_uint = 0;
const VM_PAGE_SEL_HIGHMEM: c_uint = 3;

/// `VM_PT_KERNEL` of <vm/vm_page.h>: the type for generic kernel
/// allocations.
const VM_PT_KERNEL: c_ushort = 3;

/// `vm_page_fictitious_quantum` of vm/vm_resident.c.
const VM_PAGE_FICTITIOUS_QUANTUM: c_int = 5;

/// `virtual_space_start` of vm/vm_resident.c: the first kernel virtual
/// address `pmap_steal_memory()` hands out.
#[unsafe(export_name = "virtual_space_start")]
static mut VIRTUAL_SPACE_START: VmOffset = 0;

/// `virtual_space_end` of vm/vm_resident.c: the end of the range
/// `pmap_steal_memory()` hands out.
#[unsafe(export_name = "virtual_space_end")]
static mut VIRTUAL_SPACE_END: VmOffset = 0;

/// `vm_page_queue_free_lock` of vm/vm_resident.c: the lock on the free page
/// queue and the fictitious-page list.
#[unsafe(export_name = "vm_page_queue_free_lock")]
static mut VM_PAGE_QUEUE_FREE_LOCK: SimpleLock = SimpleLock::new();

/// `vm_page_queue_lock` of vm/vm_resident.c: the lock on the active and
/// inactive page queues.
#[unsafe(export_name = "vm_page_queue_lock")]
static mut VM_PAGE_QUEUE_LOCK: SimpleLock = SimpleLock::new();

/// `vm_page_fictitious_addr` of vm/vm_resident.c: the fake physical address
/// of a fictitious page.
#[unsafe(export_name = "vm_page_fictitious_addr")]
static mut VM_PAGE_FICTITIOUS_ADDR: VmOffset = VmOffset::MAX;

/// `vm_page_fictitious_count` of vm/vm_resident.c: how many fictitious pages
/// are free.  `vm_page_queue_free_lock` serializes every access, so the
/// atomic only has to make each one indivisible.
#[unsafe(export_name = "vm_page_fictitious_count")]
pub(crate) static VM_PAGE_FICTITIOUS_COUNT: AtomicI32 = AtomicI32::new(0);

/// `vm_object_external_count` of vm/vm_object.h: how many objects are paged
/// externally.  Locked like `VM_PAGE_ACTIVE_COUNT`.
#[unsafe(export_name = "vm_object_external_count")]
pub(crate) static VM_OBJECT_EXTERNAL_COUNT: AtomicI32 = AtomicI32::new(0);

/// `vm_object_external_pages` of vm/vm_object.h: how many resident pages of
/// external objects there are.  Locked like `VM_PAGE_ACTIVE_COUNT`.
#[unsafe(export_name = "vm_object_external_pages")]
pub(crate) static VM_OBJECT_EXTERNAL_PAGES: AtomicI32 = AtomicI32::new(0);

/// `vm_page_active_count` of vm/vm_resident.c: how many pages are active.
/// Every update holds `vm_page_queue_lock`, so the atomic only has to make
/// each access indivisible.
#[unsafe(export_name = "vm_page_active_count")]
pub(crate) static VM_PAGE_ACTIVE_COUNT: AtomicI32 = AtomicI32::new(0);

/// `vm_page_inactive_count` of vm/vm_resident.c: how many pages are
/// inactive.
#[unsafe(export_name = "vm_page_inactive_count")]
pub(crate) static VM_PAGE_INACTIVE_COUNT: AtomicI32 = AtomicI32::new(0);

/// `vm_page_wire_count` of vm/vm_resident.c: how many pages are wired.
#[unsafe(export_name = "vm_page_wire_count")]
pub(crate) static VM_PAGE_WIRE_COUNT: AtomicI32 = AtomicI32::new(0);

/// `vm_page_laundry_count` of vm/vm_resident.c: how many pages are being
/// cleaned.  `vm_page_queue_lock` serializes every access.
#[unsafe(export_name = "vm_page_laundry_count")]
pub(crate) static VM_PAGE_LAUNDRY_COUNT: AtomicI32 = AtomicI32::new(0);

/// `vm_page_external_laundry_count` of vm/vm_resident.c: the same for
/// external pagers.
#[unsafe(export_name = "vm_page_external_laundry_count")]
pub(crate) static VM_PAGE_EXTERNAL_LAUNDRY_COUNT: AtomicI32 =
    AtomicI32::new(0);

/// `vm_page_deactivate_behind` of vm/vm_resident.c: whether a page inserted
/// right after the last allocation deactivates that last page.
pub(crate) static VM_PAGE_DEACTIVATE_BEHIND: AtomicBool =
    AtomicBool::new(true);

/// `vm_page_deactivate_hint` of vm/vm_resident.c: whether a clean request
/// deactivates the cleaned pages.
pub(crate) static VM_PAGE_DEACTIVATE_HINT: AtomicBool = AtomicBool::new(true);

/// `vm_page_cache` of vm/vm_resident.c: the `struct vm_page` slab cache.
static mut VM_PAGE_CACHE: KmemCache = KmemCache::zeroed();

/// `vm_page_bucket_t` of vm/vm_resident.c: one head of the
/// object/offset-to-page hash table.  File-private after this port.
struct PageBucket {
    lock: SimpleLock,
    pages: *mut VmPage,
}

/// The hash table and the fictitious-page list, the file-private state of
/// vm/vm_resident.c.  The bucket locks and `vm_page_queue_free_lock` serialize
/// access, as in the C.
struct ResidentState {
    buckets: *mut PageBucket,
    bucket_count: usize,
    hash_mask: usize,
    fictitious: List,
}

impl ResidentState {
    const fn new() -> Self {
        Self {
            buckets: null_mut(),
            bucket_count: 0,
            hash_mask: 0,
            fictitious: List::unlinked(),
        }
    }
}

static RESIDENT_STATE: SyncCell<ResidentState> =
    SyncCell(UnsafeCell::new(ResidentState::new()));

fn state() -> *mut ResidentState {
    RESIDENT_STATE.0.get()
}

/// `panic()` of vm/vm_resident.c at the caller's line.
#[track_caller]
fn die(func: &'static CStr, message: &'static CStr) -> ! {
    let location = core::panic::Location::caller();
    // SAFETY: `Panic` does not return; the file, function and message are
    // this module's, and the line fits the `c_int` the format takes.
    unsafe {
        Panic(
            c"rust/src/vm/vm_resident.rs".as_ptr(),
            location.line() as c_int,
            func.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// The list node `page` links by, for the fictitious-page list.
fn page_node(page: *mut VmPage) -> NonNull<List> {
    // SAFETY: the caller promises a live page whose storage stays put.
    let node = unsafe { &raw mut (*page).node };
    // SAFETY: the node is an aligned field of a live page.
    unsafe { NonNull::new_unchecked(node) }
}

/// `vm_page_hash()` of vm/vm_resident.c at the given key.
///
/// # Safety
///
/// The bootstrap must have sized the table, as the C's callers had.
unsafe fn bucket_index(object: *mut VmObject, offset: VmOffset) -> usize {
    // The C truncated both terms to `unsigned int` before the mask, so on a
    // 64-bit machine the pointer contributes only its low 32 bits.
    let mask = unsafe { (*state()).hash_mask } as u32;
    let hash =
        (object as usize as u32).wrapping_add((offset >> PAGE_SHIFT) as u32);
    (hash & mask) as usize
}

/// The bucket head for `object`/`offset`.
///
/// # Safety
///
/// The bootstrap must have allocated the table.
unsafe fn bucket_ptr(
    object: *mut VmObject,
    offset: VmOffset,
) -> *mut PageBucket {
    // SAFETY: the caller promises the table is allocated, and the index is
    // masked into it.
    unsafe { (*state()).buckets.add(bucket_index(object, offset)) }
}

/// `pmap_steal_memory()` in C: reserve `size` of kernel virtual space and map
/// fresh physical pages into it.
///
/// On failure the error is the page-rounded size the C passed to its
/// exhaustion panic, in bytes.
pub(crate) fn pmap_steal_memory(size: VmSize) -> Result<VmOffset, VmSize> {
    let size = round_page(size);

    // SAFETY: the statics are this module's boot pair, and the caller runs
    // before anything else has taken a mapping out of them.
    let mut start = unsafe { VIRTUAL_SPACE_START };
    // SAFETY: as above.
    let mut end = unsafe { VIRTUAL_SPACE_END };
    if start == end {
        // SAFETY: the pair is empty, so the pmap has yet to report a range.
        unsafe { pmap_virtual_space(&mut start, &mut end) };
        start = round_page(start);
        end = trunc_page(end);
        // SAFETY: the locals hold the range the pmap just reported.
        unsafe {
            VIRTUAL_SPACE_START = start;
            VIRTUAL_SPACE_END = end;
        }
    }

    let addr = start;
    let new_start = start.wrapping_add(size);
    if new_start < start {
        return Err(size);
    }
    // SAFETY: the check above kept the new value inside the address space.
    unsafe { VIRTUAL_SPACE_START = new_start };

    let limit = addr.wrapping_add(size);
    let mut vaddr = round_page(addr);
    while vaddr < limit {
        // SAFETY: `vm_page_bootalloc()` is the real early allocator, and the
        // caller runs after the physical segments are loaded.
        let paddr = unsafe { vm_page_bootalloc(PAGE_SIZE) };
        // SAFETY: `kernel_pmap` is the boot pmap and `vaddr` is inside the
        // range just reserved; the C maps the page without wiring it.
        unsafe {
            pmap_enter(
                kernel_pmap,
                vaddr,
                paddr,
                VmProt::READ | VmProt::WRITE,
                c_int::from(false),
            )
        };
        vaddr = vaddr.wrapping_add(PAGE_SIZE);
    }

    Ok(addr)
}

/// `vm_page_bootstrap()` in C: initialize the page queues and the
/// object/offset hash table, then report the kernel virtual range.
pub(crate) fn bootstrap() -> (VmOffset, VmOffset) {
    // SAFETY: the bootstrap runs once, before any other user of the two
    // locks, and the fictitious list is not yet linked.
    unsafe {
        (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).init();
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).init();
        List::init_head_at(NonNull::new_unchecked(addr_of_mut!(
            (*state()).fictitious
        )));
    }

    let mut bucket_count = unsafe { (*state()).bucket_count };
    if bucket_count == 0 {
        let npages = vm_page::table_size();
        bucket_count = 1;
        while bucket_count < npages {
            bucket_count <<= 1;
        }
        // SAFETY: the bootstrap owns the state until the table is up.
        unsafe { (*state()).bucket_count = bucket_count };
    }

    let hash_mask = bucket_count - 1;
    // SAFETY: as above.
    unsafe { (*state()).hash_mask = hash_mask };

    if hash_mask & bucket_count != 0 {
        // SAFETY: `printf` is the kernel's formatter and the string is the
        // C's.
        unsafe {
            printf(
                c"vm_page_bootstrap: WARNING -- strange page hash\n".as_ptr(),
            )
        };
    }

    let size = bucket_count.wrapping_mul(size_of::<PageBucket>());
    let buckets = match pmap_steal_memory(size) {
        Ok(addr) => addr,
        Err(size) => {
            // SAFETY: `Panic` does not return; the message and its `%d`
            // argument are the C `panic()`'s.
            unsafe {
                Panic(
                    c"rust/src/vm/vm_resident.rs".as_ptr(),
                    line!() as c_int,
                    c"pmap_steal_memory".as_ptr(),
                    c"not enough kernel virtual space for %dMB virtual allocation!\n"
                        .as_ptr(),
                    (size >> 20) as c_int,
                )
            }
        }
    };
    // `vm_offset_t` and a pointer are the same width on both targets.
    let buckets = buckets as *mut PageBucket;

    // SAFETY: the fresh table is the module's own storage and no other
    // thread can see it yet.
    unsafe { (*state()).buckets = buckets };

    let mut i = 0;
    while i < bucket_count {
        // SAFETY: `i` is inside the table just allocated.
        let bucket = unsafe { buckets.add(i) };
        // SAFETY: as above; the bucket is fresh storage.
        unsafe {
            (*bucket).pages = null_mut();
            (*bucket).lock.init();
        }
        i += 1;
    }

    vm_page::setup();

    // SAFETY: the bootstrap owns the pair until the table is reported.
    let start = round_page(unsafe { VIRTUAL_SPACE_START });
    // SAFETY: as above.
    let end = trunc_page(unsafe { VIRTUAL_SPACE_END });
    // SAFETY: as above.
    unsafe {
        VIRTUAL_SPACE_START = start;
        VIRTUAL_SPACE_END = end;
    }

    (start, end)
}

/// `vm_page_insert()` in C: put `mem` in the object/offset hash table and the
/// object's page list.
///
/// # Safety
///
/// `mem` must be a live page and `object` a live, locked object, and the
/// caller must hold `vm_page_queue_lock`, as the C required.
pub(crate) unsafe fn insert(
    mem: NonNull<VmPage>,
    object: NonNull<VmObject>,
    offset: VmOffset,
) {
    let page = mem.as_ptr();
    let object = object.as_ptr();

    // SAFETY: the caller promises the live page and the object lock.
    unsafe { vm_page::check(page) };

    // SAFETY: the caller holds the object lock, and the page is live.
    unsafe {
        if !(*object).is_internal() {
            (*page).set_external(true);
            VM_OBJECT_EXTERNAL_PAGES.fetch_add(1, Ordering::Relaxed);
        }

        if (*page).is_tabled() {
            die(c"vm_page_insert", c"vm_page_insert");
        }

        (*page).object = object;
        (*page).offset = offset;
    }

    // SAFETY: the bootstrap built the table; the object lock serializes the
    // field, and the bucket lock serializes the chain.
    unsafe {
        let bucket = bucket_ptr(object, offset);
        (*bucket).lock.lock();
        (*page).next = (*bucket).pages;
        (*bucket).pages = page;
        (*bucket).lock.unlock();

        queue_enter_tail(
            addr_of_mut!((*object).memq),
            page.cast(),
            offset_of!(VmPage, listq),
        );
        (*page).set_tabled(true);
        (*object).resident_page_count += 1;
    }

    let deactivate_behind = VM_PAGE_DEACTIVATE_BEHIND.load(Ordering::Relaxed);
    // SAFETY: the page is live and the object locked, as `lookup` needs.
    let behind = unsafe {
        if deactivate_behind
            && offset == (*object).last_alloc.wrapping_add(PAGE_SIZE)
        {
            lookup(NonNull::new_unchecked(object), (*object).last_alloc)
        } else {
            None
        }
    };
    if let Some(behind) = behind {
        // SAFETY: the page came out of the table and the page-queues lock is
        // held, as `deactivate` needs.
        unsafe {
            if !(*behind.as_ptr()).is_busy() {
                vm_page::deactivate(behind.as_ptr());
            }
            (*object).last_alloc = offset;
        }
    } else {
        // SAFETY: the caller holds the object lock.
        unsafe { (*object).last_alloc = offset };
    }
}

/// `vm_page_replace()` in C: insert `mem`, first removing any page already at
/// the key.
///
/// # Safety
///
/// `mem` must be a live page and `object` a live, locked object, and the
/// caller must hold `vm_page_queue_lock`, as the C required.
pub(crate) unsafe fn replace(
    mem: NonNull<VmPage>,
    object: NonNull<VmObject>,
    offset: VmOffset,
) {
    let page = mem.as_ptr();
    let object = object.as_ptr();

    // SAFETY: the caller promises the live page and the object lock.
    unsafe { vm_page::check(page) };

    // SAFETY: the caller holds the object lock, and the page is live.
    unsafe {
        if !(*object).is_internal() {
            (*page).set_external(true);
            VM_OBJECT_EXTERNAL_PAGES.fetch_add(1, Ordering::Relaxed);
        }

        if (*page).is_tabled() {
            die(c"vm_page_replace", c"vm_page_replace");
        }

        (*page).object = object;
        (*page).offset = offset;
    }

    // SAFETY: the bootstrap built the table; the object lock serializes the
    // fields, and the bucket lock serializes the chain.
    unsafe {
        let bucket = bucket_ptr(object, offset);
        (*bucket).lock.lock();

        if !(*bucket).pages.is_null() {
            let mut link = addr_of_mut!((*bucket).pages);
            loop {
                let old = *link;
                if old.is_null() {
                    break;
                }
                if (*old).object == object && (*old).offset == offset {
                    *link = (*old).next;
                    queue_remove_generic(
                        addr_of_mut!((*object).memq),
                        old.cast(),
                        offset_of!(VmPage, listq),
                    );
                    (*old).set_tabled(false);
                    (*object).resident_page_count -= 1;
                    vm_page::queues_remove(old);

                    if (*old).is_external() {
                        (*old).set_external(false);
                        VM_OBJECT_EXTERNAL_PAGES
                            .fetch_sub(1, Ordering::Relaxed);
                    }

                    free(NonNull::new_unchecked(old));
                    break;
                }
                link = addr_of_mut!((*old).next);
            }
            (*page).next = (*bucket).pages;
        } else {
            (*page).next = null_mut();
        }

        (*bucket).pages = page;
        (*bucket).lock.unlock();
    }

    // SAFETY: the page is live and the caller holds the object lock.
    unsafe {
        queue_enter_tail(
            addr_of_mut!((*object).memq),
            page.cast(),
            offset_of!(VmPage, listq),
        );
        (*page).set_tabled(true);
        (*object).resident_page_count += 1;
    }
}

/// `vm_page_remove()` in C: unlink `mem` from the hash table, its object's
/// page list and the page queues.
///
/// # Safety
///
/// `mem` must be a live, tabled page whose object lock and page-queues lock
/// the caller holds, as the C required.
pub(crate) unsafe fn remove(mem: NonNull<VmPage>) {
    let page = mem.as_ptr();

    // SAFETY: the caller promises the live page.
    unsafe { vm_page::check(page) };

    // SAFETY: the caller holds the object lock, and a tabled page is live.
    unsafe {
        let object = (*page).object;
        let bucket = bucket_ptr(object, (*page).offset);

        (*bucket).lock.lock();
        if (*bucket).pages == page {
            (*bucket).pages = (*page).next;
        } else {
            let mut link = addr_of_mut!((*bucket).pages);
            loop {
                let this = *link;
                if this == page {
                    *link = (*this).next;
                    break;
                }
                link = addr_of_mut!((*this).next);
            }
        }
        (*bucket).lock.unlock();

        queue_remove_generic(
            addr_of_mut!((*object).memq),
            page.cast(),
            offset_of!(VmPage, listq),
        );
        (*object).resident_page_count -= 1;
        (*page).set_tabled(false);
        vm_page::queues_remove(page);

        if (*page).is_external() {
            (*page).set_external(false);
            VM_OBJECT_EXTERNAL_PAGES.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// `vm_page_lookup()` in C: the page at `object`/`offset`, when tabled.
///
/// # Safety
///
/// `object` must be a live, locked object.
pub(crate) unsafe fn lookup(
    object: NonNull<VmObject>,
    offset: VmOffset,
) -> Option<NonNull<VmPage>> {
    // SAFETY: the caller promises the live object; the bootstrap built the
    // table.
    unsafe {
        let bucket = bucket_ptr(object.as_ptr(), offset);
        (*bucket).lock.lock();

        let mut page = (*bucket).pages;
        while !page.is_null() {
            vm_page::check(page);
            if (*page).object == object.as_ptr() && (*page).offset == offset {
                break;
            }
            page = (*page).next;
        }

        (*bucket).lock.unlock();
        NonNull::new(page)
    }
}

/// `vm_page_grab_fictitious()` in C: take a fictitious page off the free
/// list.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock`.
pub(crate) unsafe fn grab_fictitious() -> Option<NonNull<VmPage>> {
    // SAFETY: the caller does not hold the free lock, which guards the list.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).lock() };

    let page = {
        // SAFETY: the free lock is held and the list is the module's.
        let list = unsafe { &mut (*state()).fictitious };
        match list.first() {
            None => None,
            Some(node) => {
                // SAFETY: a linked node is the `node` member of a page.
                let page =
                    unsafe { entry::<VmPage>(node, offset_of!(VmPage, node)) };
                // SAFETY: the free lock is held and the node is linked.
                unsafe {
                    List::remove(node);
                    (*page.as_ptr()).set_free(false);
                }
                VM_PAGE_FICTITIOUS_COUNT.fetch_sub(1, Ordering::Relaxed);
                Some(page)
            }
        }
    };

    // SAFETY: the lock was taken above.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };

    page
}

/// `vm_page_release_fictitious()` in C: return a fictitious page to the free
/// list.
///
/// # Safety
///
/// `mem` must be a live fictitious page the caller owns, and the caller must
/// not hold `vm_page_queue_free_lock`.
unsafe fn release_fictitious(mem: NonNull<VmPage>) {
    let page = mem.as_ptr();

    // SAFETY: the caller does not hold the free lock, which guards the list.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).lock() };

    // SAFETY: the free lock is held and the page is live.
    unsafe {
        if (*page).is_free() {
            die(c"vm_page_release_fictitious", c"vm_page_release_fictitious");
        }

        (*page).set_free(true);
        let list = &mut (*state()).fictitious;
        List::insert_head(list, page_node(page));
    }
    VM_PAGE_FICTITIOUS_COUNT.fetch_add(1, Ordering::Relaxed);

    // SAFETY: the lock was taken above.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };
}

/// `vm_page_more_fictitious()` in C: allocate more fictitious pages into the
/// free list.
///
/// # Safety
///
/// The slab package must be up and the caller must be allowed to block, as
/// the C required.
pub(crate) unsafe fn more_fictitious() {
    let mut i = 0;
    while i < VM_PAGE_FICTITIOUS_QUANTUM {
        // SAFETY: the caller runs after the cache is initialized, and its own
        // lock serializes the allocation.
        let page = unsafe { (*addr_of_mut!(VM_PAGE_CACHE)).alloc() }
            .map_or_else(
                || die(c"vm_page_more_fictitious", c"vm_page_more_fictitious"),
                |buf| buf.cast::<VmPage>(),
            );

        // SAFETY: the fresh page is writable storage that nothing sees yet.
        unsafe {
            init(&mut *page.as_ptr());
            (*page.as_ptr()).phys_addr = VM_PAGE_FICTITIOUS_ADDR;
            (*page.as_ptr()).set_fictitious(true);
        }
        // SAFETY: the fresh page is owned by this call and the free lock is
        // not held.
        unsafe { release_fictitious(page) };

        i += 1;
    }
}

/// `vm_page_convert()` in C: turn a fictitious page into a real one, or
/// report that no page was available.
///
/// # Safety
///
/// `fict` must be a live fictitious page whose object lock the caller holds,
/// as the C required.
pub(crate) unsafe fn convert(
    fict: NonNull<VmPage>,
) -> Option<NonNull<VmPage>> {
    // SAFETY: the caller's locks make the allocator's contract hold.
    let real = unsafe { grab(VM_PAGE_HIGHMEM) }?;
    let page = fict.as_ptr();

    // SAFETY: the caller promises the live page and its object lock.
    let (object, offset) = unsafe { ((*page).object, (*page).offset) };

    // SAFETY: the caller holds the page-queues lock, which `remove` needs.
    unsafe {
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).lock();
        remove(fict);

        (*real.as_ptr()).copy_body_from(&*page);
        (*real.as_ptr()).set_fictitious(false);

        insert(real, NonNull::new_unchecked(object), offset);
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).unlock();
    }

    // SAFETY: the page is the fictitious one just unlinked from every list.
    unsafe { release_fictitious(fict) };

    Some(real)
}

/// `vm_page_order()` of <vm/vm_page.h>: the power of two that holds `size`
/// bytes.
fn page_order(size: VmSize) -> c_uint {
    let pages = (round_page(size) >> PAGE_SHIFT) as usize;
    if pages == 1 {
        return 0;
    }
    // The C's `iorder2()`: the bit length of `pages - 1`, which wraps to the
    // word width for a zero size, as the C's unsigned shift did.
    (usize::BITS - pages.wrapping_sub(1).leading_zeros()) as c_uint
}

/// The number of pages `size` rounds up to, or zero for the wrapped
/// `usize::BITS` order the C computed.
fn contig_pages(size: VmSize) -> u32 {
    1u32.wrapping_shl(page_order(size))
}

/// `vm_page_grab_contig()` in C: remove a block of contiguous pages from the
/// free list.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be in a
/// context where the allocator may spin, as the C required.
pub(crate) unsafe fn grab_contig(
    size: VmSize,
    selector: c_uint,
) -> Option<NonNull<VmPage>> {
    let order = page_order(size);
    let nr_pages = contig_pages(size);

    // SAFETY: the caller's contract is the allocator's own; it returns with
    // the free lock held.
    let page = unsafe { vm_page::alloc_pa(order, selector, VM_PT_KERNEL) };
    let Some(page) = NonNull::new(page) else {
        // SAFETY: the allocator returned with the free lock held.
        unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };
        return None;
    };

    let mut i = 0;
    while i < nr_pages {
        // SAFETY: `i` is inside the block the allocator just returned.
        unsafe { (*page.as_ptr().add(i as usize)).set_free(false) };
        i += 1;
    }

    // SAFETY: the lock was taken by the allocator.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };

    Some(page)
}

/// `vm_page_free_contig()` in C: return a block of contiguous pages to the
/// free list.
///
/// # Safety
///
/// `mem` must be the first of the `contig_pages(size)` live descriptors a
/// `grab_contig()` returned, and the caller must not hold
/// `vm_page_queue_free_lock`.
pub(crate) unsafe fn free_contig(mem: NonNull<VmPage>, size: VmSize) {
    let order = page_order(size);
    let nr_pages = contig_pages(size);

    // SAFETY: the caller does not hold the free lock, which guards the block.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).lock() };

    let mut i = 0;
    while i < nr_pages {
        // SAFETY: `i` is inside the block the caller owns.
        let page = unsafe { &mut *mem.as_ptr().add(i as usize) };
        if page.is_free() {
            die(c"vm_page_free_contig", c"vm_page_free_contig");
        }
        page.set_free(true);
        i += 1;
    }

    // SAFETY: the free lock is held, as the backend needs.
    unsafe { vm_page::free_pa(mem.as_ptr(), order) };

    // SAFETY: the lock was taken above.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };
}

/// `vm_page_free()` in C: return a page to the free list, disassociating it
/// from any object.
///
/// # Safety
///
/// `mem` must be a live page the caller owns, and the caller must hold the
/// page-queues lock and the object lock, as the C required.
pub(crate) unsafe fn free(mem: NonNull<VmPage>) {
    let page = mem.as_ptr();

    // SAFETY: the caller promises the live page.
    unsafe {
        if (*page).is_free() {
            die(c"vm_page_free", c"vm_page_free");
        }

        if (*page).is_tabled() {
            remove(mem);
        }

        if (*page).wire_count() != 0 {
            if !(*page).is_private() && !(*page).is_fictitious() {
                VM_PAGE_WIRE_COUNT.fetch_sub(1, Ordering::Relaxed);
            }
            (*page).set_wire_count(0);
        }
    }

    // SAFETY: the page is live and its object lock is held.
    unsafe { crate::vm::vm_object::page_wakeup_done(page) };

    // SAFETY: the page is live; an absent page belongs to a live object.
    unsafe {
        if (*page).is_absent() {
            crate::vm::vm_object::absent_release((*page).object);
        }
    }

    let recycled = unsafe { (*page).is_private() || (*page).is_fictitious() };
    if recycled {
        // SAFETY: the page is live and no other holder exists.
        unsafe {
            init(&mut *page);
            (*page).phys_addr = VM_PAGE_FICTITIOUS_ADDR;
            (*page).set_fictitious(true);
        }
        // SAFETY: the page is the caller's and the free lock is not held.
        unsafe { release_fictitious(mem) };
    } else {
        // SAFETY: the page is live and its flags are the C's inputs.
        let (laundry, external_laundry) =
            unsafe { ((*page).is_laundry(), (*page).is_external_laundry()) };
        // SAFETY: the page is live and the caller's locks serialized it.
        unsafe { init(&mut *page) };
        // SAFETY: the page is the caller's and the free lock is not held.
        unsafe { release(mem, laundry, external_laundry) };
    }
}

/// `vm_page_info()` in C: fill `info` with the counts of the first `count`
/// hash buckets.
///
/// # Safety
///
/// `info` must be writable for `count` `hash_info_bucket_t` records, and the
/// caller must hold no lock, as the C required.
pub(crate) unsafe fn info(info: *mut HashInfoBucket, count: c_uint) -> c_uint {
    let mut count = count as usize;
    let bucket_count = unsafe { (*state()).bucket_count };
    if bucket_count < count {
        count = bucket_count;
    }

    let mut i = 0;
    while i < count {
        // SAFETY: `i` is inside the table.
        let bucket = unsafe { (*state()).buckets.add(i) };
        let mut page_count = 0;

        // SAFETY: the bucket lock serializes the chain.
        unsafe {
            (*bucket).lock.lock();
            let mut page = (*bucket).pages;
            while !page.is_null() {
                page_count += 1;
                page = (*page).next;
            }
            (*bucket).lock.unlock();
        }

        // The C writes the record after dropping the bucket lock so that no
        // lock is held while touching pageable memory.
        // SAFETY: the caller promises the record is writable.
        unsafe { (*info.add(i)).hib_count = page_count };
        i += 1;
    }

    // The C returned the `unsigned long` count through an `unsigned int`
    // result; the bucket count is a power of two below the page count.
    bucket_count as c_uint
}

/// `vm_page_rename()` in C: move a page to another object and offset.
///
/// # Safety
///
/// `page` must be a live page and `object` a live object; the object must be
/// locked, as the C requires.
pub(crate) unsafe fn rename(
    page: NonNull<VmPage>,
    object: NonNull<VmObject>,
    offset: VmOffset,
) {
    // SAFETY: the page-queue lock is the live lock the pageout daemon also
    // takes, and the caller holds the object's lock.
    unsafe {
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).lock();
        remove(page);
        insert(page, object, offset);
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).unlock();
    }
}

/// `vm_page_alloc_flags()` in C: grab a free page and table it in `object`.
///
/// # Safety
///
/// `object` must be a live, locked object.
pub(crate) unsafe fn alloc_flags(
    object: NonNull<VmObject>,
    offset: VmOffset,
    flags: c_uint,
) -> Option<NonNull<VmPage>> {
    // SAFETY: `grab()` is the real allocator, and `flags` is its documented
    // selector set.
    let page = unsafe { grab(flags) }?;

    // SAFETY: the page-queue lock is the live lock, and the caller holds the
    // object's lock.
    unsafe {
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).lock();
        insert(page, object, offset);
        (*addr_of_mut!(VM_PAGE_QUEUE_LOCK)).unlock();
    }

    Some(page)
}

/// `vm_page_alloc()` in C: `vm_page_alloc_flags()` with `VM_PAGE_HIGHMEM`.
///
/// # Safety
///
/// `object` must be a live, locked object.
pub(crate) unsafe fn alloc(
    object: NonNull<VmObject>,
    offset: VmOffset,
) -> Option<NonNull<VmPage>> {
    // SAFETY: the caller promises a live, locked object.
    unsafe { alloc_flags(object, offset, VM_PAGE_HIGHMEM) }
}

/// `vm_page_init()` in C: initialize the fields of a page whose storage holds
/// random values.  The C kept the body in a `vm_page_init_template()` static
/// with this one caller.
pub(crate) fn init(page: &mut VmPage) {
    page.object = null_mut();
    page.offset = 0;
    page.set_wire_count(0);
    page.set_inactive(false);
    page.set_active(false);
    page.set_laundry(false);
    page.set_external_laundry(false);
    page.set_free(false);
    page.set_external(false);
    page.set_busy(true);
    page.set_wanted(false);
    page.set_tabled(false);
    page.set_fictitious(false);
    page.set_private(false);
    page.set_absent(false);
    page.set_error(false);
    page.set_dirty(false);
    page.set_precious(false);
    page.set_reference(false);
    page.set_page_lock(VmProt::NONE);
    page.set_unlock_request(VmProt::NONE);
}

/// `vm_page_module_init()` in C: create the `vm_page` slab cache.
///
/// # Safety
///
/// Must run once, in the bootstrap sequence, after the slab package is up.
pub(crate) unsafe fn module_init() {
    // SAFETY: the caller runs the sequence once; the cache is the module's
    // and its `init()` is the C `kmem_cache_init()`.  `size_of::<VmPage>()`
    // is the C `sizeof(struct vm_page)`.
    unsafe {
        (*addr_of_mut!(VM_PAGE_CACHE)).init(
            b"vm_page",
            size_of::<VmPage>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        )
    };
}

/// The selector `vm_page_grab()` computes from its flags.  The limit macros
/// order the segments differently on the two builds, so x86_64 tests the
/// DMA32 flag before the DIRECTMAP one and the non-PAE i686 build has no
/// DMA32 test at all.
#[cfg(target_arch = "x86_64")]
fn alloc_selector(flags: c_uint) -> c_uint {
    if flags & VM_PAGE_HIGHMEM != 0 {
        VM_PAGE_SEL_HIGHMEM
    } else if flags & VM_PAGE_DMA32 != 0 {
        VM_PAGE_SEL_DMA32
    } else if flags & VM_PAGE_DIRECTMAP != 0 {
        VM_PAGE_SEL_DIRECTMAP
    } else {
        VM_PAGE_SEL_DMA
    }
}

/// The selector `vm_page_grab()` computes from its flags; the non-PAE i686
/// build has no DMA32 segment for the flag to select.
#[cfg(target_arch = "x86")]
fn alloc_selector(flags: c_uint) -> c_uint {
    if flags & VM_PAGE_HIGHMEM != 0 {
        VM_PAGE_SEL_HIGHMEM
    } else if flags & VM_PAGE_DIRECTMAP != 0 {
        VM_PAGE_SEL_DIRECTMAP
    } else {
        VM_PAGE_SEL_DMA
    }
}

/// `vm_page_grab()` in C: take a page out of the free list.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock`, and must be in a
/// context where the allocator's spinning is allowed.
pub(crate) unsafe fn grab(flags: c_uint) -> Option<NonNull<VmPage>> {
    // SAFETY: the caller promised the free lock is not held; the allocator
    // takes it and leaves it held for the release below.
    let page =
        unsafe { vm_page::alloc_pa(0, alloc_selector(flags), VM_PT_KERNEL) };
    let page = NonNull::new(page);

    if let Some(page) = page {
        // SAFETY: `page` is the live page just allocated, and the free lock,
        // still held, guards its free flag.
        unsafe { (*page.as_ptr()).set_free(false) };
    }

    // SAFETY: `vm_page_alloc_pa()` returns with the free lock held, on both
    // the found and the exhausted path.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };

    page
}

/// `vm_page_grab_phys_addr()` in C: the physical address of a direct-mapped
/// page, or `-1` when the free list is exhausted.
///
/// # Safety
///
/// Same contract as [`grab()`].
pub(crate) unsafe fn grab_phys_addr() -> VmOffset {
    // SAFETY: the caller promises `grab()`'s contract.
    let Some(page) = (unsafe { grab(VM_PAGE_DIRECTMAP) }) else {
        return VmOffset::MAX;
    };

    // SAFETY: `page` is the live page just allocated.
    unsafe { (*page.as_ptr()).phys_addr }
}

/// `vm_page_release()` in C: return a page to the free list, resuming the
/// pageout daemon when the last laundered page is gone.
///
/// # Safety
///
/// `page` must be a live page that no one else holds, and the caller must
/// not hold `vm_page_queue_free_lock`.
pub(crate) unsafe fn release(
    page: NonNull<VmPage>,
    laundry: bool,
    external_laundry: bool,
) {
    let ptr = page.as_ptr();

    // SAFETY: the caller promised a live page with no other holder, and does
    // not hold the free lock.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).lock() };

    // SAFETY: the free lock serializes the flag, and the page is live.
    if unsafe { (*ptr).is_free() } {
        // SAFETY: `Panic` does not return; the function and message are the
        // C `panic()`'s.
        unsafe {
            Panic(
                c"rust/src/vm/vm_resident.rs".as_ptr(),
                line!() as c_int,
                c"vm_page_release".as_ptr(),
                c"vm_page_release".as_ptr(),
            )
        }
    }

    // SAFETY: the free lock is held and guards the flag.
    unsafe { (*ptr).set_free(true) };
    // SAFETY: the free lock is held; `vm_page_free_pa()` is the real backend
    // and order zero is the C's.
    unsafe { vm_page::free_pa(ptr, 0) };

    if laundry {
        let count = VM_PAGE_LAUNDRY_COUNT.fetch_sub(1, Ordering::Relaxed);
        if count == 1 {
            // SAFETY: the pageout daemon's resume path is the real symbol,
            // and the C calls it under the free lock.
            unsafe { crate::glue::vm_pageout_resume() };
        }
    }

    if external_laundry {
        // SAFETY: the free lock guards the counter.
        let count = VM_PAGE_EXTERNAL_LAUNDRY_COUNT.load(Ordering::Relaxed);
        if 0 < count {
            let count = count.wrapping_sub(1);
            VM_PAGE_EXTERNAL_LAUNDRY_COUNT.store(count, Ordering::Relaxed);
            if count == 0 {
                // SAFETY: as `vm_pageout_resume()` above.
                unsafe { crate::glue::vm_pageout_resume() };
            }
        }
    }

    // SAFETY: the lock was taken above and nothing released it since.
    unsafe { (*addr_of_mut!(VM_PAGE_QUEUE_FREE_LOCK)).unlock() };
}

/// `vm_page_zero_fill()` in C: zero the page's physical memory.
///
/// # Safety
///
/// `page` must be a live page.
pub(crate) unsafe fn zero_fill(page: NonNull<VmPage>) {
    // SAFETY: the caller promises a live page; `vm_page_check()` is the C
    // validator.
    unsafe { vm_page::check(page.as_ptr()) };
    // SAFETY: the page is live, so its physical address names real memory.
    unsafe { pmap_zero_page((*page.as_ptr()).phys_addr) };
}

/// `vm_page_copy()` in C: copy one page's physical memory to another.
///
/// # Safety
///
/// `src` and `dest` must be live pages.
pub(crate) unsafe fn copy(src: NonNull<VmPage>, dest: NonNull<VmPage>) {
    // SAFETY: the caller promises both pages are live.
    unsafe {
        vm_page::check(src.as_ptr());
        vm_page::check(dest.as_ptr());
    }
    // SAFETY: both pages are live, so their physical addresses name real
    // memory.
    unsafe {
        pmap_copy_page((*src.as_ptr()).phys_addr, (*dest.as_ptr()).phys_addr)
    };
}
