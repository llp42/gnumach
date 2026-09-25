// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from vm/vm_page.c and vm/vm_page.h:
//   Copyright (c) 2010-2014 Richard Braun.
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The physical-page module, which `vm/vm_page.c` used to define, and the
//! `struct vm_page` mirror of `vm/vm_page.h`.

use crate::arch::i386::percpu::{cpu_number, current_thread};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_SHIFT, PAGE_SIZE};
use crate::config::NCPUS;
use crate::glue::{
    Panic, kernel_pmap, memory_manager_default, memory_manager_default_port,
    pmap_clear_modify, pmap_clear_reference, pmap_extract, pmap_is_modified,
    pmap_is_referenced, pmap_page_protect, printf, vm_object_collapse,
    vm_object_collect, vm_object_pager_create, vm_page_fictitious_addr,
    vm_page_free, vm_page_insert, vm_page_queue_free_lock, vm_page_queue_lock,
    vm_page_remove, vm_pageout_page, vm_pageout_start, vm_stat,
};
use crate::kern::list::{List, entry};
use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_block, thread_wakeup_prim,
};
use crate::utils::cell::SyncCell;
use crate::vm::types::{VmObject, VmProt};
use crate::vm::vm_resident;
use core::cell::UnsafeCell;
use core::cmp::min;
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull, addr_of_mut, null_mut};
use core::sync::atomic::{AtomicBool, Ordering};

/// `struct vm_page` of <vm/vm_page.h>.
///
/// C packs the three bitfield runs into two 32-bit words, and the accessors
/// below mask and shift within them in declaration order:
///
/// * `flags` carries `wire_count` in bits 0 to 14 and the seventeen
///   single-bit flags from bit 15 to bit 31, `inactive` through
///   `overwriting`.
/// * `lock_bits` carries `page_lock` in bits 0 to 2, `unlock_request` in
///   bits 3 to 5, and the `unsigned short` run in bits 8 to 15: `type` in
///   bits 8 and 9, `seg_index` in bits 10 and 11, `order` in bits 12 to 15.
///   Bits 6 and 7 are the C compiler's hole between the two runs.
#[repr(C)]
pub struct VmPage {
    pub node: List,
    pub node_lru: List,
    /// The C `priv`.
    pub priv_: *mut c_void,
    pub phys_addr: VmOffset,
    pub listq: QueueEntry,
    pub next: *mut VmPage,
    pub object: *mut VmObject,
    pub offset: VmOffset,
    flags: u32,
    lock_bits: u32,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<VmPage>() == 96);
    assert!(align_of::<VmPage>() == 8);
    assert!(offset_of!(VmPage, node) == 0);
    assert!(offset_of!(VmPage, node_lru) == 16);
    assert!(offset_of!(VmPage, priv_) == 32);
    assert!(offset_of!(VmPage, phys_addr) == 40);
    assert!(offset_of!(VmPage, listq) == 48);
    assert!(offset_of!(VmPage, next) == 64);
    assert!(offset_of!(VmPage, object) == 72);
    assert!(offset_of!(VmPage, offset) == 80);
    assert!(offset_of!(VmPage, flags) == 88);
    assert!(offset_of!(VmPage, lock_bits) == 92);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<VmPage>() == 52);
    assert!(align_of::<VmPage>() == 4);
    assert!(offset_of!(VmPage, node) == 0);
    assert!(offset_of!(VmPage, node_lru) == 8);
    assert!(offset_of!(VmPage, priv_) == 16);
    assert!(offset_of!(VmPage, phys_addr) == 20);
    assert!(offset_of!(VmPage, listq) == 24);
    assert!(offset_of!(VmPage, next) == 32);
    assert!(offset_of!(VmPage, object) == 36);
    assert!(offset_of!(VmPage, offset) == 40);
    assert!(offset_of!(VmPage, flags) == 44);
    assert!(offset_of!(VmPage, lock_bits) == 48);
};

const WIRE_COUNT_MASK: u32 = 0x7fff;
const INACTIVE_BIT: u32 = 1 << 15;
const ACTIVE_BIT: u32 = 1 << 16;
const LAUNDRY_BIT: u32 = 1 << 17;
const EXTERNAL_LAUNDRY_BIT: u32 = 1 << 18;
const FREE_BIT: u32 = 1 << 19;
const REFERENCE_BIT: u32 = 1 << 20;
const EXTERNAL_BIT: u32 = 1 << 21;
const BUSY_BIT: u32 = 1 << 22;
const WANTED_BIT: u32 = 1 << 23;
const TABLED_BIT: u32 = 1 << 24;
const FICTITIOUS_BIT: u32 = 1 << 25;
const PRIVATE_BIT: u32 = 1 << 26;
const ABSENT_BIT: u32 = 1 << 27;
const ERROR_BIT: u32 = 1 << 28;
const DIRTY_BIT: u32 = 1 << 29;
const PRECIOUS_BIT: u32 = 1 << 30;
const OVERWRITING_BIT: u32 = 1 << 31;

const PAGE_LOCK_SHIFT: u32 = 0;
const PAGE_LOCK_MASK: u32 = 0x7;
const UNLOCK_REQUEST_SHIFT: u32 = 3;
const UNLOCK_REQUEST_MASK: u32 = 0x7;
const TYPE_SHIFT: u32 = 8;
const TYPE_MASK: u32 = 0x3;
const SEG_INDEX_SHIFT: u32 = 10;
const SEG_INDEX_MASK: u32 = 0x3;
const ORDER_SHIFT: u32 = 12;
const ORDER_MASK: u32 = 0xf;

impl VmPage {
    /// `wire_count`; the C bitfield holds fifteen bits.
    pub fn wire_count(&self) -> u32 {
        self.flags & WIRE_COUNT_MASK
    }

    pub fn set_wire_count(&mut self, count: u32) {
        self.flags =
            (self.flags & !WIRE_COUNT_MASK) | (count & WIRE_COUNT_MASK);
    }

    pub fn is_inactive(&self) -> bool {
        self.flag(INACTIVE_BIT)
    }
    pub fn set_inactive(&mut self, on: bool) {
        self.set_flag(INACTIVE_BIT, on);
    }

    pub fn is_active(&self) -> bool {
        self.flag(ACTIVE_BIT)
    }
    pub fn set_active(&mut self, on: bool) {
        self.set_flag(ACTIVE_BIT, on);
    }

    pub fn is_laundry(&self) -> bool {
        self.flag(LAUNDRY_BIT)
    }
    pub fn set_laundry(&mut self, on: bool) {
        self.set_flag(LAUNDRY_BIT, on);
    }

    pub fn is_external_laundry(&self) -> bool {
        self.flag(EXTERNAL_LAUNDRY_BIT)
    }
    pub fn set_external_laundry(&mut self, on: bool) {
        self.set_flag(EXTERNAL_LAUNDRY_BIT, on);
    }

    pub fn is_free(&self) -> bool {
        self.flag(FREE_BIT)
    }
    pub fn set_free(&mut self, on: bool) {
        self.set_flag(FREE_BIT, on);
    }

    pub fn is_reference(&self) -> bool {
        self.flag(REFERENCE_BIT)
    }
    pub fn set_reference(&mut self, on: bool) {
        self.set_flag(REFERENCE_BIT, on);
    }

    pub fn is_external(&self) -> bool {
        self.flag(EXTERNAL_BIT)
    }
    pub fn set_external(&mut self, on: bool) {
        self.set_flag(EXTERNAL_BIT, on);
    }

    pub fn is_busy(&self) -> bool {
        self.flag(BUSY_BIT)
    }
    pub fn set_busy(&mut self, on: bool) {
        self.set_flag(BUSY_BIT, on);
    }

    pub fn is_wanted(&self) -> bool {
        self.flag(WANTED_BIT)
    }
    pub fn set_wanted(&mut self, on: bool) {
        self.set_flag(WANTED_BIT, on);
    }

    pub fn is_tabled(&self) -> bool {
        self.flag(TABLED_BIT)
    }
    pub fn set_tabled(&mut self, on: bool) {
        self.set_flag(TABLED_BIT, on);
    }

    pub fn is_fictitious(&self) -> bool {
        self.flag(FICTITIOUS_BIT)
    }
    pub fn set_fictitious(&mut self, on: bool) {
        self.set_flag(FICTITIOUS_BIT, on);
    }

    pub fn is_private(&self) -> bool {
        self.flag(PRIVATE_BIT)
    }
    pub fn set_private(&mut self, on: bool) {
        self.set_flag(PRIVATE_BIT, on);
    }

    pub fn is_absent(&self) -> bool {
        self.flag(ABSENT_BIT)
    }
    pub fn set_absent(&mut self, on: bool) {
        self.set_flag(ABSENT_BIT, on);
    }

    pub fn is_error(&self) -> bool {
        self.flag(ERROR_BIT)
    }
    pub fn set_error(&mut self, on: bool) {
        self.set_flag(ERROR_BIT, on);
    }

    pub fn is_dirty(&self) -> bool {
        self.flag(DIRTY_BIT)
    }
    pub fn set_dirty(&mut self, on: bool) {
        self.set_flag(DIRTY_BIT, on);
    }

    pub fn is_precious(&self) -> bool {
        self.flag(PRECIOUS_BIT)
    }
    pub fn set_precious(&mut self, on: bool) {
        self.set_flag(PRECIOUS_BIT, on);
    }

    pub fn is_overwriting(&self) -> bool {
        self.flag(OVERWRITING_BIT)
    }
    pub fn set_overwriting(&mut self, on: bool) {
        self.set_flag(OVERWRITING_BIT, on);
    }

    fn flag(&self, bit: u32) -> bool {
        self.flags & bit != 0
    }

    fn set_flag(&mut self, bit: u32, on: bool) {
        if on {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }

    pub fn page_lock(&self) -> VmProt {
        // The mask bounds the field to three bits, so it fits a `c_int`.
        VmProt::from_bits((self.lock_bits & PAGE_LOCK_MASK) as c_int)
    }

    pub fn set_page_lock(&mut self, protection: VmProt) {
        self.set_lock_field(
            PAGE_LOCK_SHIFT,
            PAGE_LOCK_MASK,
            protection.bits() as u32,
        );
    }

    pub fn unlock_request(&self) -> VmProt {
        VmProt::from_bits(
            self.lock_field(UNLOCK_REQUEST_SHIFT, UNLOCK_REQUEST_MASK)
                as c_int,
        )
    }

    pub fn set_unlock_request(&mut self, protection: VmProt) {
        self.set_lock_field(
            UNLOCK_REQUEST_SHIFT,
            UNLOCK_REQUEST_MASK,
            protection.bits() as u32,
        );
    }

    pub fn page_type(&self) -> u16 {
        self.lock_field(TYPE_SHIFT, TYPE_MASK) as u16
    }

    pub fn set_page_type(&mut self, type_: u16) {
        self.set_lock_field(TYPE_SHIFT, TYPE_MASK, u32::from(type_));
    }

    pub fn seg_index(&self) -> u16 {
        self.lock_field(SEG_INDEX_SHIFT, SEG_INDEX_MASK) as u16
    }

    pub fn set_seg_index(&mut self, index: u16) {
        self.set_lock_field(SEG_INDEX_SHIFT, SEG_INDEX_MASK, u32::from(index));
    }

    pub fn order(&self) -> u16 {
        self.lock_field(ORDER_SHIFT, ORDER_MASK) as u16
    }

    pub fn set_order(&mut self, order: u16) {
        self.set_lock_field(ORDER_SHIFT, ORDER_MASK, u32::from(order));
    }

    fn lock_field(&self, shift: u32, mask: u32) -> u32 {
        (self.lock_bits >> shift) & mask
    }

    fn set_lock_field(&mut self, shift: u32, mask: u32, value: u32) {
        self.lock_bits =
            (self.lock_bits & !(mask << shift)) | ((value & mask) << shift);
    }

    /// `memcpy(&dest->vm_page_header, &src->vm_page_header,
    /// VM_PAGE_BODY_SIZE)` of `vm_page_seg_balance_page()`: copy `object`,
    /// `offset` and the flags, keeping the destination's own `type`,
    /// `seg_index` and `order`.
    pub(crate) fn copy_body_from(&mut self, src: &Self) {
        self.object = src.object;
        self.offset = src.offset;
        self.flags = src.flags;
        self.lock_bits = (self.lock_bits & !BODY_LOCK_BITS)
            | (src.lock_bits & BODY_LOCK_BITS);
    }
}

/// The `VM_PAGE_BODY_SIZE` bytes of the C struct: `lock_bits` bits 0 to 7,
/// below the `type`/`seg_index`/`order` run the body copy leaves alone.
const BODY_LOCK_BITS: u32 =
    PAGE_LOCK_MASK | (UNLOCK_REQUEST_MASK << UNLOCK_REQUEST_SHIFT);

/// `vm_page_set_type()` in C: stamp `type` on a run of `1 << order` pages.
///
/// # Safety
///
/// `page` must point at the first of `1 << order` live, contiguous page
/// descriptors, and the caller must serialize access to them.
pub(crate) unsafe fn set_type(
    page: NonNull<VmPage>,
    order: c_uint,
    type_: u16,
) {
    // The C shifted an `int` by `order`, and x86 masks the count; the
    // wrapping shift spells the same placement.
    let nr_pages = 1u32.wrapping_shl(order);

    for i in 0..nr_pages {
        // The cast cannot truncate: `i < nr_pages` and both targets are at
        // least 32 bits wide.
        let offset = i as usize;
        // SAFETY: the caller promises the run is that long, so `offset`
        // indexes a live descriptor.
        unsafe { (*page.as_ptr().add(offset)).set_page_type(type_) };
    }
}

/// `vm_page_wire()` in C: mark the page wired down by yet another map,
/// removing it from the paging queues when it was unwired.
///
/// # Safety
///
/// `page` must be a live page, and the caller must hold its object lock and
/// the page-queues lock, as the C requires.
pub(crate) unsafe fn wire(page: NonNull<VmPage>) {
    let ptr = page.as_ptr();

    // SAFETY: the caller promises a live page; `check()` is the C validator
    // of that and of its locks.
    unsafe { check(ptr) };

    if unsafe { (*ptr).wire_count() } == 0 {
        // SAFETY: the caller holds the two locks the C's call relies on,
        // and `ptr` is a live page.
        unsafe { queues_remove(ptr) };

        // SAFETY: the page-queues lock guards the page's flags.
        if !unsafe { (*ptr).is_private() }
            && !unsafe { (*ptr).is_fictitious() }
        {
            // The page-queues lock guards the global count, as in the C;
            // the atomic only makes the update indivisible.
            vm_resident::VM_PAGE_WIRE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    // SAFETY: the page is live and the caller's locks serialize the field.
    unsafe { (*ptr).set_wire_count((*ptr).wire_count() + 1) };
}

/// `VM_PAGE_SEG_DMA` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
pub(crate) const SEG_DMA: c_uint = 0;
/// `VM_PAGE_SEG_DIRECTMAP` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
pub(crate) const SEG_DIRECTMAP: c_uint = 1;
/// `VM_PAGE_SEG_DMA32` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
pub(crate) const SEG_DMA32: c_uint = 2;
/// `VM_PAGE_SEG_HIGHMEM` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
pub(crate) const SEG_HIGHMEM: c_uint = 3;

/// `VM_PAGE_SEG_DMA` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
pub(crate) const SEG_DMA: c_uint = 0;
/// `VM_PAGE_SEG_DIRECTMAP` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
pub(crate) const SEG_DIRECTMAP: c_uint = 1;
/// `VM_PAGE_SEG_DMA32`: the direct-map index, as the non-PAE i686 build
/// aliases it.
#[cfg(target_arch = "x86")]
pub(crate) const SEG_DMA32: c_uint = SEG_DIRECTMAP;
/// `VM_PAGE_SEG_HIGHMEM` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
pub(crate) const SEG_HIGHMEM: c_uint = 2;

/// `vm_page_seg_name()` in C: the name of a physical segment index.
pub(crate) fn seg_name(seg_index: c_uint) -> Option<&'static CStr> {
    // The C's if-chain, not a match: DMA32 is the DIRECTMAP index on i686,
    // and a duplicate match arm would not compile.
    if seg_index == SEG_HIGHMEM {
        Some(c"HIGHMEM")
    } else if seg_index == SEG_DIRECTMAP {
        Some(c"DIRECTMAP")
    } else if seg_index == SEG_DMA32 {
        Some(c"DMA32")
    } else if seg_index == SEG_DMA {
        Some(c"DMA")
    } else {
        None
    }
}

/// `VM_PAGE_MAX_SEGS` of <machine/vm_param.h>: the non-PAE i686 build has
/// no DMA32 segment and carries one fewer.
#[cfg(target_arch = "x86_64")]
pub(crate) const VM_PAGE_MAX_SEGS: usize = 4;
#[cfg(target_arch = "x86")]
pub(crate) const VM_PAGE_MAX_SEGS: usize = 3;

/// `VM_PAGE_SEL_*` of <vm/vm_page.h>: the selectors `vm_page_grab()` and
/// `vm_page_alloc_pa()` take.  The DMA32 and DIRECTMAP indices swap with
/// the segment ordering, and the non-PAE i686 build has no DMA32 entry.
const SEL_DMA: c_uint = 0;
#[cfg(target_arch = "x86_64")]
const SEL_DIRECTMAP: c_uint = 1;
#[cfg(target_arch = "x86_64")]
const SEL_DMA32: c_uint = 2;
#[cfg(target_arch = "x86")]
const SEL_DMA32: c_uint = 1;
#[cfg(target_arch = "x86")]
const SEL_DIRECTMAP: c_uint = 2;
const SEL_HIGHMEM: c_uint = 3;

/// `VM_PT_FREE`, `VM_PT_RESERVED` and `VM_PT_TABLE` of <vm/vm_page.h>;
/// `VM_PT_KERNEL` lives in `vm_resident.rs`.
const VM_PT_FREE: u16 = 0;
const VM_PT_RESERVED: u16 = 1;
const VM_PT_TABLE: u16 = 2;

/// `VM_PAGE_NR_FREE_LISTS` of `vm_page.c`.
const VM_PAGE_NR_FREE_LISTS: usize = 11;

/// `VM_PAGE_ORDER_UNLISTED`: a page that is not the head of a free block.
const VM_PAGE_ORDER_UNLISTED: u16 = (VM_PAGE_NR_FREE_LISTS + 1) as u16;

const VM_PAGE_CPU_POOL_RATIO: usize = 1024;
const VM_PAGE_CPU_POOL_MAX_SIZE: usize = 128;
const VM_PAGE_CPU_POOL_TRANSFER_RATIO: usize = 2;

const VM_PAGE_SEG_THRESHOLD_MIN_NUM: usize = 5;
const VM_PAGE_SEG_THRESHOLD_MIN_DENOM: usize = 100;
const VM_PAGE_SEG_THRESHOLD_MIN: usize = 500;
const VM_PAGE_SEG_THRESHOLD_LOW_NUM: usize = 6;
const VM_PAGE_SEG_THRESHOLD_LOW_DENOM: usize = 100;
const VM_PAGE_SEG_THRESHOLD_LOW: usize = 600;
const VM_PAGE_SEG_THRESHOLD_HIGH_NUM: usize = 10;
const VM_PAGE_SEG_THRESHOLD_HIGH_DENOM: usize = 100;
const VM_PAGE_SEG_THRESHOLD_HIGH: usize = 1000;
const VM_PAGE_SEG_MIN_PAGES: usize = 2000;

const VM_PAGE_HIGH_ACTIVE_PAGE_NUM: usize = 1;
const VM_PAGE_HIGH_ACTIVE_PAGE_DENOM: usize = 3;

const VM_PAGE_MAX_LAUNDRY: c_int = 5;
const VM_PAGE_MAX_EVICTIONS: usize = 5;

const _: () = assert!(VM_PAGE_ORDER_UNLISTED < 1 << 4);
const _: () = assert!(VM_PAGE_SEG_THRESHOLD_LOW > VM_PAGE_SEG_THRESHOLD_MIN);
const _: () = assert!(VM_PAGE_SEG_THRESHOLD_HIGH > VM_PAGE_SEG_THRESHOLD_LOW);
const _: () = assert!(VM_PAGE_SEG_MIN_PAGES > VM_PAGE_SEG_THRESHOLD_HIGH);

/// `vm_page_atop()` of <vm/vm_page.h>: a byte address to a page number.
const fn atop(addr: VmOffset) -> usize {
    addr >> PAGE_SHIFT
}

/// `vm_page_ptoa()` of <vm/vm_page.h>: a page number to a byte address.
const fn ptoa(page: usize) -> VmOffset {
    page << PAGE_SHIFT
}

/// `vm_page_round()` of <vm/vm_page.h>.
const fn round_page(addr: VmOffset) -> VmOffset {
    addr.wrapping_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// `panic()` of `vm_page.c` at the caller's line.
#[track_caller]
fn die(func: &'static CStr, message: &'static CStr) -> ! {
    let location = core::panic::Location::caller();
    // SAFETY: `Panic` does not return; the file, function and message are
    // this module's, and the line fits the `c_int` the format takes.
    unsafe {
        Panic(
            c"rust/src/vm/vm_page.rs".as_ptr(),
            location.line() as c_int,
            func.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// `struct vm_page_cpu_pool` of `vm_page.c`.
struct CpuPool {
    lock: SimpleLock,
    size: c_int,
    transfer_size: c_int,
    nr_pages: c_int,
    pages: List,
}

impl CpuPool {
    const fn new() -> Self {
        Self {
            lock: SimpleLock::new(),
            size: 0,
            transfer_size: 0,
            nr_pages: 0,
            pages: List::unlinked(),
        }
    }
}

/// `struct vm_page_free_list` of `vm_page.c`.
struct FreeList {
    size: usize,
    blocks: List,
}

impl FreeList {
    const fn new() -> Self {
        Self {
            size: 0,
            blocks: List::unlinked(),
        }
    }
}

/// `struct vm_page_list` of `vm_page.c`.
struct PageList {
    pages: List,
    nr_pages: usize,
}

impl PageList {
    const fn new() -> Self {
        Self {
            pages: List::unlinked(),
            nr_pages: 0,
        }
    }
}

/// `struct vm_page_queue` of `vm_page.c`.
struct PageQueue {
    internal: PageList,
    external: PageList,
}

impl PageQueue {
    const fn new() -> Self {
        Self {
            internal: PageList::new(),
            external: PageList::new(),
        }
    }
}

/// `struct vm_page_lru_queue` of `vm_page.c`.
struct LruQueue {
    internal: List,
    external: List,
}

impl LruQueue {
    const fn new() -> Self {
        Self {
            internal: List::unlinked(),
            external: List::unlinked(),
        }
    }
}

/// `struct vm_page_seg` of `vm_page.c`.  File-private after this port, so
/// it keeps Rust layout.
struct VmPageSeg {
    cpu_pools: [CpuPool; NCPUS],
    start: VmOffset,
    end: VmOffset,
    pages: *mut VmPage,
    pages_end: *mut VmPage,
    lock: SimpleLock,
    free_lists: [FreeList; VM_PAGE_NR_FREE_LISTS],
    nr_free_pages: usize,
    min_free_pages: usize,
    low_free_pages: usize,
    high_free_pages: usize,
    active_pages: PageQueue,
    high_active_pages: usize,
    inactive_pages: PageQueue,
}

impl VmPageSeg {
    const fn new() -> Self {
        Self {
            cpu_pools: [const { CpuPool::new() }; NCPUS],
            start: 0,
            end: 0,
            pages: null_mut(),
            pages_end: null_mut(),
            lock: SimpleLock::new(),
            free_lists: [const { FreeList::new() }; VM_PAGE_NR_FREE_LISTS],
            nr_free_pages: 0,
            min_free_pages: 0,
            low_free_pages: 0,
            high_free_pages: 0,
            active_pages: PageQueue::new(),
            high_active_pages: 0,
            inactive_pages: PageQueue::new(),
        }
    }
}

/// `struct vm_page_boot_seg` of `vm_page.c`.
struct BootSeg {
    start: VmOffset,
    end: VmOffset,
    heap_present: bool,
    avail_start: VmOffset,
    avail_end: VmOffset,
}

impl BootSeg {
    const fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            heap_present: false,
            avail_start: 0,
            avail_end: 0,
        }
    }
}

/// The module's `static` state: the segment table, the boot table and the
/// two LRU queues the C kept at file scope.  The C locks serialize it.
struct PageState {
    segs: [VmPageSeg; VM_PAGE_MAX_SEGS],
    boot_segs: [BootSeg; VM_PAGE_MAX_SEGS],
    segs_size: u32,
    is_ready: bool,
    alloc_paused: bool,
    active_lru: LruQueue,
    inactive_lru: LruQueue,
}

impl PageState {
    const fn new() -> Self {
        Self {
            segs: [const { VmPageSeg::new() }; VM_PAGE_MAX_SEGS],
            boot_segs: [const { BootSeg::new() }; VM_PAGE_MAX_SEGS],
            segs_size: 0,
            is_ready: false,
            alloc_paused: false,
            active_lru: LruQueue::new(),
            inactive_lru: LruQueue::new(),
        }
    }
}

static STATE: SyncCell<PageState> =
    SyncCell(UnsafeCell::new(PageState::new()));

/// The C `static boolean_t warned` of `vm_page_evict()`.
static WARNED: AtomicBool = AtomicBool::new(false);

fn state() -> *mut PageState {
    STATE.0.get()
}

/// The segment at `index`, which every caller keeps below
/// `VM_PAGE_MAX_SEGS`, as the C's unguarded array indexing assumed.
fn seg_ptr(index: usize) -> *mut VmPageSeg {
    debug_assert!(index < VM_PAGE_MAX_SEGS);
    // SAFETY: the caller promises `index` is in range; the array is static
    // storage that never moves.
    unsafe { addr_of_mut!((*state()).segs).cast::<VmPageSeg>().add(index) }
}

fn segs_size() -> usize {
    // SAFETY: the state is live for the kernel's lifetime.
    unsafe { (*state()).segs_size as usize }
}

/// The boot segment at `index`.
///
/// # Safety
///
/// `index` must be below `VM_PAGE_MAX_SEGS`.
unsafe fn boot_seg(index: usize) -> *mut BootSeg {
    if index >= VM_PAGE_MAX_SEGS {
        die(c"vm_page_load", c"vm_page: invalid segment index");
    }
    // SAFETY: the check above bounds the index.
    unsafe {
        addr_of_mut!((*state()).boot_segs)
            .cast::<BootSeg>()
            .add(index)
    }
}

/// `vm_page_init_pa()` in C.
///
/// # Safety
///
/// `page` must point at writable storage for one descriptor, not yet visible
/// to any other thread.
unsafe fn init_pa(page: *mut VmPage, seg_index: u16, pa: VmOffset) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe {
        ptr::write_bytes(page, 0, 1);
        vm_resident::init(&mut *page);
        (*page).set_page_type(VM_PT_RESERVED);
        (*page).set_seg_index(seg_index);
        (*page).set_order(VM_PAGE_ORDER_UNLISTED);
        (*page).priv_ = null_mut();
        (*page).phys_addr = pa;
    }
}

/// `vm_page_pageable()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor.
unsafe fn pageable(page: *const VmPage) -> bool {
    // SAFETY: the caller promises a live descriptor.
    unsafe {
        !(*page).object.is_null()
            && (*page).wire_count() == 0
            && ((*page).is_active() || (*page).is_inactive())
    }
}

/// `vm_page_can_move()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor on a page queue, holding its object
/// lock, as the C's callers did.
unsafe fn can_move(page: *const VmPage) -> bool {
    // SAFETY: the caller promises the live descriptor and the object lock.
    unsafe {
        !(*page).is_busy()
            && !(*page).is_wanted()
            && !(*page).is_absent()
            && (*(*page).object).is_alive()
    }
}

/// `vm_page_remove_mappings()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor.
unsafe fn remove_mappings(page: *mut VmPage) {
    // SAFETY: the caller promises a live descriptor; the physical address
    // names real memory.
    unsafe {
        (*page).set_busy(true);
        pmap_page_protect((*page).phys_addr, VmProt::NONE.bits());
        if !(*page).is_dirty() {
            (*page).set_dirty(pmap_is_modified((*page).phys_addr) != 0);
        }
    }
}

/// `vm_page_free_list_init()` in C.
fn free_list_init(free_list: &mut FreeList) {
    free_list.size = 0;
    // SAFETY: the list head is unlinked storage that stays put.
    unsafe {
        List::init_head_at(NonNull::new_unchecked(&raw mut free_list.blocks))
    };
}

fn page_node(page: *mut VmPage) -> NonNull<List> {
    // SAFETY: the caller promises a live descriptor whose storage stays put.
    let node = unsafe { &raw mut (*page).node };
    // SAFETY: the node is an aligned field of a live descriptor.
    unsafe { NonNull::new_unchecked(node) }
}

fn page_lru_node(page: *mut VmPage) -> NonNull<List> {
    // SAFETY: the caller promises a live descriptor whose storage stays put.
    let node = unsafe { &raw mut (*page).node_lru };
    // SAFETY: the node is an aligned field of a live descriptor.
    unsafe { NonNull::new_unchecked(node) }
}

/// `vm_page_free_list_insert()` in C.
fn free_list_insert(free_list: &mut FreeList, page: *mut VmPage) {
    free_list.size += 1;
    // SAFETY: the caller holds the segment lock, and the page descriptor
    // stays at its address while linked.
    unsafe { free_list.blocks.insert_head(page_node(page)) };
}

/// `vm_page_free_list_remove()` in C.
fn free_list_remove(free_list: &mut FreeList, page: *mut VmPage) {
    free_list.size -= 1;
    // SAFETY: the caller holds the segment lock, and the page is linked.
    unsafe { List::remove(page_node(page)) };
}

/// `vm_page_cpu_pool_init()` in C.
fn cpu_pool_init(cpu_pool: &mut CpuPool, size: c_int) {
    cpu_pool.lock.init();
    cpu_pool.size = size;
    cpu_pool.transfer_size =
        (size + (VM_PAGE_CPU_POOL_TRANSFER_RATIO as c_int) - 1)
            / (VM_PAGE_CPU_POOL_TRANSFER_RATIO as c_int);
    cpu_pool.nr_pages = 0;
    // SAFETY: the list head is unlinked storage that stays put.
    unsafe {
        List::init_head_at(NonNull::new_unchecked(&raw mut cpu_pool.pages))
    };
}

/// `vm_page_cpu_pool_get()` in C.
///
/// # Safety
///
/// `seg` must be a live segment.
unsafe fn cpu_pool_get(seg: *mut VmPageSeg) -> *mut CpuPool {
    let cpu = cpu_number();
    debug_assert!((cpu as usize) < NCPUS);
    // SAFETY: `cpu_number()` returns an index below `NCPUS`.
    unsafe {
        addr_of_mut!((*seg).cpu_pools)
            .cast::<CpuPool>()
            .add(cpu as usize)
    }
}

/// `vm_page_cpu_pool_pop()` in C.
///
/// # Safety
///
/// The pool lock must be held and the pool must not be empty.
fn cpu_pool_pop(cpu_pool: &mut CpuPool) -> *mut VmPage {
    cpu_pool.nr_pages -= 1;
    let Some(node) = cpu_pool.pages.first() else {
        die(c"vm_page_cpu_pool_pop", c"vm_page: empty CPU pool");
    };
    // SAFETY: the node is the `node` member of a live descriptor.
    let page =
        unsafe { entry::<VmPage>(node, offset_of!(VmPage, node)) }.as_ptr();
    // SAFETY: the page is linked into the pool list.
    unsafe { List::remove(page_node(page)) };
    page
}

/// `vm_page_cpu_pool_push()` in C.
///
/// # Safety
///
/// The pool lock must be held and `page` must be a live descriptor not on any
/// other list.
fn cpu_pool_push(cpu_pool: &mut CpuPool, page: *mut VmPage) {
    cpu_pool.nr_pages += 1;
    // SAFETY: the caller holds the pool lock and the page stays put.
    unsafe { cpu_pool.pages.insert_head(page_node(page)) };
}

/// `vm_page_cpu_pool_fill()` in C.
///
/// # Safety
///
/// `seg` must be a live segment, the caller must hold `vm_page_queue_free_lock`,
/// and the pool lock must not be held.
unsafe fn cpu_pool_fill(cpu_pool: *mut CpuPool, seg: *mut VmPageSeg) -> c_int {
    // SAFETY: the caller promises the segment; the C takes its lock here.
    unsafe { (*seg).lock.lock() };

    let mut i = 0;
    while i < unsafe { (*cpu_pool).transfer_size } {
        // SAFETY: the free lock is held, as the backend requires.
        let page = unsafe { seg_alloc_from_buddy(seg, 0) };
        if page.is_null() {
            break;
        }
        // SAFETY: the pool lock is held and the page came off the buddy.
        unsafe { cpu_pool_push(&mut *cpu_pool, page) };
        i += 1;
    }

    // SAFETY: the lock was taken above.
    unsafe { (*seg).lock.unlock() };

    i
}

/// `vm_page_cpu_pool_drain()` in C.
///
/// # Safety
///
/// `seg` must be a live segment, the caller must hold `vm_page_queue_free_lock`,
/// and the pool lock must not be held.
unsafe fn cpu_pool_drain(cpu_pool: *mut CpuPool, seg: *mut VmPageSeg) {
    // SAFETY: the caller promises the segment; the C takes its lock here.
    unsafe { (*seg).lock.lock() };

    let mut i = unsafe { (*cpu_pool).transfer_size };
    while i > 0 {
        // SAFETY: the pool was full, so the C's fixed transfer count is
        // available.
        let page = cpu_pool_pop(unsafe { &mut *cpu_pool });
        // SAFETY: the segment lock is held.
        unsafe { seg_free_to_buddy(seg, page, 0) };
        i -= 1;
    }

    // SAFETY: the lock was taken above.
    unsafe { (*seg).lock.unlock() };
}

/// `vm_page_list_init()` in C.
fn page_list_init(list: &mut PageList) {
    list.nr_pages = 0;
    // SAFETY: the list head is unlinked storage that stays put.
    unsafe { List::init_head_at(NonNull::new_unchecked(&raw mut list.pages)) };
}

/// `vm_page_queue_init()` in C.
fn page_queue_init(queue: &mut PageQueue) {
    page_list_init(&mut queue.internal);
    page_list_init(&mut queue.external);
}

/// `vm_page_queue_push()` in C.
///
/// # Safety
///
/// The page-queue lock must be held and `page` must be a live descriptor.
unsafe fn page_queue_push(queue: *mut PageQueue, page: *mut VmPage) {
    // SAFETY: the caller promises the queue and the live descriptor.
    let list = if unsafe { (*page).is_external() } {
        unsafe { &mut (*queue).external }
    } else {
        unsafe { &mut (*queue).internal }
    };
    // SAFETY: the page stays at its address while linked.
    unsafe { list.pages.insert_tail(page_node(page)) };
    list.nr_pages += 1;
}

/// `vm_page_queue_remove()` in C.
///
/// # Safety
///
/// The page-queue lock must be held and `page` must be linked in `queue`.
unsafe fn page_queue_remove(queue: *mut PageQueue, page: *mut VmPage) {
    // SAFETY: the caller promises the queue and the linked descriptor.
    let list = if unsafe { (*page).is_external() } {
        unsafe { &mut (*queue).external }
    } else {
        unsafe { &mut (*queue).internal }
    };
    // SAFETY: the page is linked in that list.
    unsafe { List::remove(page_node(page)) };
    list.nr_pages -= 1;
}

/// `vm_page_lru_queue_push()` in C.
///
/// # Safety
///
/// The page-queue lock must be held and `page` must be a live descriptor.
unsafe fn lru_queue_push(queue: *mut LruQueue, page: *mut VmPage) {
    // SAFETY: the caller promises the queue and the live descriptor.
    let list = if unsafe { (*page).is_external() } {
        unsafe { &mut (*queue).external }
    } else {
        unsafe { &mut (*queue).internal }
    };
    // SAFETY: the page stays at its address while linked.
    unsafe { list.insert_tail(page_lru_node(page)) };
}

/// `vm_page_lru_queue_remove()` in C.
///
/// # Safety
///
/// The page-queue lock must be held and `page` must be linked in a LRU queue.
unsafe fn lru_queue_remove(page: *mut VmPage) {
    // SAFETY: the page is linked by `node_lru`.
    unsafe { List::remove(page_lru_node(page)) };
}

/// `vm_page_seg_index()` in C.
///
/// # Safety
///
/// `seg` must point inside the segment table.
unsafe fn seg_index(seg: *const VmPageSeg) -> usize {
    let base = unsafe { addr_of_mut!((*state()).segs).cast::<VmPageSeg>() };
    (seg as usize - base as usize) / size_of::<VmPageSeg>()
}

/// `vm_page_seg_size()` in C.
///
/// # Safety
///
/// `seg` must be a live segment.
unsafe fn seg_size(seg: *const VmPageSeg) -> VmOffset {
    // SAFETY: the caller promises a live segment.
    unsafe { (*seg).end - (*seg).start }
}

/// `vm_page_seg_compute_pool_size()` in C.
///
/// # Safety
///
/// `seg` must be a live segment.
unsafe fn seg_compute_pool_size(seg: *const VmPageSeg) -> c_int {
    // SAFETY: the caller promises a live segment.
    let mut size = atop(unsafe { seg_size(seg) }) / VM_PAGE_CPU_POOL_RATIO;

    if size == 0 {
        size = 1;
    } else if size > VM_PAGE_CPU_POOL_MAX_SIZE {
        size = VM_PAGE_CPU_POOL_MAX_SIZE;
    }

    size as c_int
}

/// `vm_page_seg_compute_pageout_thresholds()` in C.
///
/// # Safety
///
/// `seg` must be a live segment.
unsafe fn seg_compute_pageout_thresholds(seg: *mut VmPageSeg) {
    // SAFETY: the caller promises a live segment.
    let nr_pages = atop(unsafe { seg_size(seg) });

    if nr_pages < VM_PAGE_SEG_MIN_PAGES {
        die(
            c"vm_page_seg_compute_pageout_thresholds",
            c"vm_page: segment too small",
        );
    }

    let min_free_pages = nr_pages.wrapping_mul(VM_PAGE_SEG_THRESHOLD_MIN_NUM)
        / VM_PAGE_SEG_THRESHOLD_MIN_DENOM;
    let low_free_pages = nr_pages.wrapping_mul(VM_PAGE_SEG_THRESHOLD_LOW_NUM)
        / VM_PAGE_SEG_THRESHOLD_LOW_DENOM;
    let high_free_pages = nr_pages
        .wrapping_mul(VM_PAGE_SEG_THRESHOLD_HIGH_NUM)
        / VM_PAGE_SEG_THRESHOLD_HIGH_DENOM;

    // SAFETY: the caller promises a live segment; the C's own minimums.
    unsafe {
        (*seg).min_free_pages = min_free_pages.max(VM_PAGE_SEG_THRESHOLD_MIN);
        (*seg).low_free_pages = low_free_pages.max(VM_PAGE_SEG_THRESHOLD_LOW);
        (*seg).high_free_pages =
            high_free_pages.max(VM_PAGE_SEG_THRESHOLD_HIGH);
    }
}

/// `vm_page_seg_init()` in C.
///
/// # Safety
///
/// `seg` must be a live segment, `pages` must point at a table of
/// `atop(end - start)` writable descriptors, and no other thread may be
/// using either.
unsafe fn seg_init(
    seg: *mut VmPageSeg,
    start: VmOffset,
    end: VmOffset,
    pages: *mut VmPage,
) {
    // SAFETY: the caller promises the live segment and page table.
    unsafe {
        (*seg).start = start;
        (*seg).end = end;
    }
    let pool_size = unsafe { seg_compute_pool_size(seg) };

    let mut i = 0;
    while i < NCPUS {
        // SAFETY: `i` is below `NCPUS`.
        let cpu_pool =
            unsafe { addr_of_mut!((*seg).cpu_pools).cast::<CpuPool>().add(i) };
        cpu_pool_init(unsafe { &mut *cpu_pool }, pool_size);
        i += 1;
    }

    // SAFETY: the caller promises the live segment and page table.
    unsafe {
        (*seg).pages = pages;
        (*seg).pages_end = pages.add(atop(seg_size(seg)));
        (*seg).lock.init();
    }

    let mut i = 0;
    while i < VM_PAGE_NR_FREE_LISTS {
        free_list_init(unsafe { &mut (*seg).free_lists[i] });
        i += 1;
    }

    // SAFETY: the caller promises the live segment.
    unsafe {
        (*seg).nr_free_pages = 0;
    }
    unsafe { seg_compute_pageout_thresholds(seg) };
    page_queue_init(unsafe { &mut (*seg).active_pages });
    page_queue_init(unsafe { &mut (*seg).inactive_pages });

    // SAFETY: the segment is inside the table.
    let index = unsafe { seg_index(seg) } as u16;

    let mut pa = start;
    while pa < end {
        // SAFETY: `pa` runs over the segment's pages.
        let page = unsafe { (*seg).pages.add(atop(pa - start)) };
        // SAFETY: the descriptor is inside the freshly reserved table.
        unsafe { init_pa(page, index, pa) };
        pa = pa.wrapping_add(PAGE_SIZE);
    }
}

/// `vm_page_seg_alloc_from_buddy()` in C.
///
/// # Safety
///
/// `seg` must be a live segment and the caller must hold
/// `vm_page_queue_free_lock` and the segment lock, as the C's callers did.
unsafe fn seg_alloc_from_buddy(
    seg: *mut VmPageSeg,
    order: c_uint,
) -> *mut VmPage {
    let thread = current_thread();
    // The C's `current_thread() && !current_thread()->vm_privilege`: an
    // early-boot call with no thread is not paused.
    let limited = !thread.is_null() && unsafe { (*thread).vm_privilege } == 0;

    // SAFETY: the caller promises the segment and the free lock.
    if unsafe { (*state()).alloc_paused } && limited {
        return null_mut();
    } else if unsafe { (*seg).nr_free_pages <= (*seg).low_free_pages } {
        // SAFETY: the C calls the daemon's entry point under the free lock.
        unsafe { vm_pageout_start() };

        if unsafe { (*seg).nr_free_pages <= (*seg).min_free_pages } && limited
        {
            // SAFETY: the free lock serializes the flag.
            unsafe { (*state()).alloc_paused = true };
            return null_mut();
        }
    }

    let mut i = order as usize;
    while i < VM_PAGE_NR_FREE_LISTS {
        // SAFETY: the caller holds the segment lock.
        if unsafe { (*seg).free_lists[i].size } != 0 {
            break;
        }
        i += 1;
    }

    if i == VM_PAGE_NR_FREE_LISTS {
        return null_mut();
    }

    // SAFETY: the list `i` is non-empty, so its head has a first node.
    let free_list = unsafe { &mut (*seg).free_lists[i] };
    let Some(node) = free_list.blocks.first() else {
        die(c"vm_page_seg_alloc_from_buddy", c"vm_page: empty free list");
    };
    // SAFETY: the node is the `node` member of a live descriptor.
    let page =
        unsafe { entry::<VmPage>(node, offset_of!(VmPage, node)) }.as_ptr();
    free_list_remove(free_list, page);
    // SAFETY: the page is live and the segment lock is held.
    unsafe { (*page).set_order(VM_PAGE_ORDER_UNLISTED) };

    while i > order as usize {
        i -= 1;
        // SAFETY: the split buddy lies inside the segment's page table.
        let buddy = unsafe { page.add(1 << i) };
        free_list_insert(unsafe { &mut (*seg).free_lists[i] }, buddy);
        // SAFETY: as above; the order fits four bits.
        unsafe { (*buddy).set_order(i as u16) };
    }

    // SAFETY: the segment lock serializes the counters.
    unsafe {
        (*seg).nr_free_pages -= 1usize << order;
        if (*seg).nr_free_pages < (*seg).min_free_pages {
            (*state()).alloc_paused = true;
        }
    }

    page
}

/// `vm_page_seg_free_to_buddy()` in C.
///
/// # Safety
///
/// `seg` must be a live segment, the caller must hold its lock, and `page`
/// must be the first of `1 << order` free descriptors inside it.
unsafe fn seg_free_to_buddy(
    seg: *mut VmPageSeg,
    mut page: *mut VmPage,
    order: c_uint,
) {
    let mut order = order;
    let nr_pages = 1usize << order;
    // SAFETY: the caller promises the live page.
    let mut pa = unsafe { (*page).phys_addr };

    while order < (VM_PAGE_NR_FREE_LISTS as c_uint - 1) {
        let buddy_pa = pa ^ ptoa(1usize << order);
        // SAFETY: the segment's bounds are live fields.
        if buddy_pa < unsafe { (*seg).start }
            || buddy_pa >= unsafe { (*seg).end }
        {
            break;
        }
        // SAFETY: `buddy_pa` is inside the segment, so the offset indexes
        // its page table.
        let buddy = unsafe { (*seg).pages.add(atop(buddy_pa - (*seg).start)) };
        // SAFETY: the buddy descriptor is live.
        if unsafe { (*buddy).order() } != order as u16 {
            break;
        }
        free_list_remove(
            unsafe { &mut (*seg).free_lists[order as usize] },
            buddy,
        );
        // SAFETY: the buddy is live and the segment lock is held.
        unsafe { (*buddy).set_order(VM_PAGE_ORDER_UNLISTED) };
        order += 1;
        pa &= !(ptoa(1usize << order) - 1);
        // SAFETY: the merged block starts inside the segment.
        page = unsafe { (*seg).pages.add(atop(pa - (*seg).start)) };
    }

    free_list_insert(unsafe { &mut (*seg).free_lists[order as usize] }, page);
    // SAFETY: the page is live and the segment lock is held.
    unsafe {
        (*page).set_order(order as u16);
        (*seg).nr_free_pages += nr_pages;
    }
}

/// `vm_page_seg_alloc()` in C.
///
/// # Safety
///
/// `seg` must be a live segment and the caller must hold
/// `vm_page_queue_free_lock`, as the C's callers did.
unsafe fn seg_alloc(
    seg: *mut VmPageSeg,
    order: c_uint,
    type_: u16,
) -> *mut VmPage {
    let page;

    if order == 0 {
        let thread = current_thread();
        // SAFETY: the free lock serializes `alloc_paused`.
        if unsafe { (*state()).alloc_paused }
            && !thread.is_null()
            && unsafe { (*thread).vm_privilege } == 0
        {
            return null_mut();
        }

        // SAFETY: the caller promises the segment.
        let cpu_pool = unsafe { cpu_pool_get(seg) };
        // SAFETY: the pool lock serializes the pool.
        unsafe { (*cpu_pool).lock.lock() };

        if unsafe { (*cpu_pool).nr_pages } == 0 {
            // SAFETY: the pool and segment locks are held, and the free lock
            // was taken by the caller.
            let filled = unsafe { cpu_pool_fill(cpu_pool, seg) };

            if filled == 0 {
                // SAFETY: the pool lock was taken above.
                unsafe { (*cpu_pool).lock.unlock() };
                return null_mut();
            }
        }

        // SAFETY: the pool is non-empty and its lock is held.
        page = cpu_pool_pop(unsafe { &mut *cpu_pool });
        // SAFETY: the pool lock was taken above.
        unsafe { (*cpu_pool).lock.unlock() };
    } else {
        // SAFETY: the caller promises the segment.
        unsafe { (*seg).lock.lock() };
        // SAFETY: the segment lock is held.
        page = unsafe { seg_alloc_from_buddy(seg, order) };
        // SAFETY: the segment lock was taken above.
        unsafe { (*seg).lock.unlock() };

        if page.is_null() {
            return null_mut();
        }
    }

    // SAFETY: the freshly allocated page is live.
    unsafe { set_type(NonNull::new_unchecked(page), order, type_) };
    page
}

/// `vm_page_seg_free()` in C.
///
/// # Safety
///
/// `seg` must be a live segment and the caller must hold
/// `vm_page_queue_free_lock`, as the C's callers did.
unsafe fn seg_free(seg: *mut VmPageSeg, page: *mut VmPage, order: c_uint) {
    // SAFETY: the caller promises the live page.
    unsafe { set_type(NonNull::new_unchecked(page), order, VM_PT_FREE) };

    if order == 0 {
        // SAFETY: the caller promises the segment.
        let cpu_pool = unsafe { cpu_pool_get(seg) };
        // SAFETY: the pool lock serializes the pool.
        unsafe { (*cpu_pool).lock.lock() };

        if unsafe { (*cpu_pool).nr_pages == (*cpu_pool).size } {
            // SAFETY: the pool is full, the pool lock is held, and the free
            // lock was taken by the caller.
            unsafe { cpu_pool_drain(cpu_pool, seg) };
        }

        // SAFETY: the pool lock is held and the page is off every list.
        cpu_pool_push(unsafe { &mut *cpu_pool }, page);
        // SAFETY: the pool lock was taken above.
        unsafe { (*cpu_pool).lock.unlock() };
    } else {
        // SAFETY: the caller promises the segment.
        unsafe { (*seg).lock.lock() };
        // SAFETY: the segment and free locks are held.
        unsafe { seg_free_to_buddy(seg, page, order) };
        // SAFETY: the segment lock was taken above.
        unsafe { (*seg).lock.unlock() };
    }
}

/// `vm_page_seg_add_active_page()` in C.
///
/// # Safety
///
/// The segment and page-queues locks must be held, and `page` must be a live
/// descriptor not already queued.
unsafe fn seg_add_active_page(seg: *mut VmPageSeg, page: *mut VmPage) {
    // SAFETY: the caller holds the locks and promises the live page.
    unsafe {
        (*page).set_active(true);
        (*page).set_reference(true);
        page_queue_push(addr_of_mut!((*seg).active_pages), page);
        lru_queue_push(addr_of_mut!((*state()).active_lru), page);
    }
    vm_resident::VM_PAGE_ACTIVE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// `vm_page_seg_remove_active_page()` in C.
///
/// # Safety
///
/// The segment and page-queues locks must be held, and `page` must be queued
/// active.
unsafe fn seg_remove_active_page(seg: *mut VmPageSeg, page: *mut VmPage) {
    // SAFETY: the caller holds the locks and promises the queued page.
    unsafe {
        (*page).set_active(false);
        page_queue_remove(addr_of_mut!((*seg).active_pages), page);
        lru_queue_remove(page);
    }
    vm_resident::VM_PAGE_ACTIVE_COUNT.fetch_sub(1, Ordering::Relaxed);
}

/// `vm_page_seg_add_inactive_page()` in C.
///
/// # Safety
///
/// The segment and page-queues locks must be held, and `page` must be a live
/// descriptor not already queued.
unsafe fn seg_add_inactive_page(seg: *mut VmPageSeg, page: *mut VmPage) {
    // SAFETY: the caller holds the locks and promises the live page.
    unsafe {
        (*page).set_inactive(true);
        page_queue_push(addr_of_mut!((*seg).inactive_pages), page);
        lru_queue_push(addr_of_mut!((*state()).inactive_lru), page);
    }
    vm_resident::VM_PAGE_INACTIVE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// `vm_page_seg_remove_inactive_page()` in C.
///
/// # Safety
///
/// The segment and page-queues locks must be held, and `page` must be queued
/// inactive.
unsafe fn seg_remove_inactive_page(seg: *mut VmPageSeg, page: *mut VmPage) {
    // SAFETY: the caller holds the locks and promises the queued page.
    unsafe {
        (*page).set_inactive(false);
        page_queue_remove(addr_of_mut!((*seg).inactive_pages), page);
        lru_queue_remove(page);
    }
    vm_resident::VM_PAGE_INACTIVE_COUNT.fetch_sub(1, Ordering::Relaxed);
}

/// The page at the head of a `struct list`, or null.
///
/// # Safety
///
/// `list` must be a valid list head whose nodes embed at `field`.
unsafe fn first_page(list: *const List, field: usize) -> *mut VmPage {
    // SAFETY: the caller promises a valid head and that its nodes embed a
    // `VmPage` at `field`.
    unsafe { list.as_ref() }
        .and_then(|list| list.first())
        .map_or(null_mut(), |node| {
            unsafe { entry::<VmPage>(node, field) }.as_ptr()
        })
}

/// `vm_page_seg_pull_active_page()` in C.
///
/// # Safety
///
/// The segment and page-queues locks must be held.  On success the object
/// lock is held and the page is off the queues; the caller keeps the segment
/// lock.
unsafe fn seg_pull_active_page(
    seg: *mut VmPageSeg,
    external: bool,
) -> *mut VmPage {
    // SAFETY: the caller promises the segment and its lock.
    let page_list = if external {
        unsafe { addr_of_mut!((*seg).active_pages.external.pages) }
    } else {
        unsafe { addr_of_mut!((*seg).active_pages.internal.pages) }
    };
    let mut first: *mut VmPage = null_mut();

    loop {
        // SAFETY: the page-queues lock is held and `page_list` is a live
        // head.
        let page = unsafe { first_page(page_list, offset_of!(VmPage, node)) };

        if page.is_null() || page == first {
            break;
        }

        if first.is_null() {
            first = page;
        }

        // SAFETY: the page is queued active.
        unsafe { seg_remove_active_page(seg, page) };
        let object = unsafe { (*page).object };
        // SAFETY: a queued page has a live object and the page-queues lock
        // is held.
        let locked = unsafe { (*object).lock.try_lock() };

        if !locked {
            // SAFETY: the page was removed above.
            unsafe { seg_add_active_page(seg, page) };
            continue;
        }

        // SAFETY: the object lock is held.
        if !unsafe { can_move(page) } {
            // SAFETY: as above.
            unsafe { seg_add_active_page(seg, page) };
            unsafe { (*object).lock.unlock() };
            continue;
        }

        return page;
    }

    null_mut()
}

/// `vm_page_seg_pull_inactive_page()` in C.
///
/// # Safety
///
/// Same contract as [`seg_pull_active_page()`].
unsafe fn seg_pull_inactive_page(
    seg: *mut VmPageSeg,
    external: bool,
) -> *mut VmPage {
    // SAFETY: the caller promises the segment and its lock.
    let page_list = if external {
        unsafe { addr_of_mut!((*seg).inactive_pages.external.pages) }
    } else {
        unsafe { addr_of_mut!((*seg).inactive_pages.internal.pages) }
    };
    let mut first: *mut VmPage = null_mut();

    loop {
        // SAFETY: the page-queues lock is held and `page_list` is a live
        // head.
        let page = unsafe { first_page(page_list, offset_of!(VmPage, node)) };

        if page.is_null() || page == first {
            break;
        }

        if first.is_null() {
            first = page;
        }

        // SAFETY: the page is queued inactive.
        unsafe { seg_remove_inactive_page(seg, page) };
        let object = unsafe { (*page).object };
        // SAFETY: a queued page has a live object and the page-queues lock
        // is held.
        let locked = unsafe { (*object).lock.try_lock() };

        if !locked {
            // SAFETY: the page was removed above.
            unsafe { seg_add_inactive_page(seg, page) };
            continue;
        }

        // SAFETY: the object lock is held.
        if !unsafe { can_move(page) } {
            // SAFETY: as above.
            unsafe { seg_add_inactive_page(seg, page) };
            unsafe { (*object).lock.unlock() };
            continue;
        }

        return page;
    }

    null_mut()
}

/// `vm_page_pull_active_page()` in C.
///
/// # Safety
///
/// The page-queues lock must be held.  On success the segment and object
/// locks are held and the page is off the queues.
unsafe fn pull_active_page(external: bool) -> *mut VmPage {
    // SAFETY: the state is live.
    let page_list = if external {
        unsafe { addr_of_mut!((*state()).active_lru.external) }
    } else {
        unsafe { addr_of_mut!((*state()).active_lru.internal) }
    };
    let mut first: *mut VmPage = null_mut();

    loop {
        // SAFETY: the page-queues lock is held and the LRU head is live.
        let page =
            unsafe { first_page(page_list, offset_of!(VmPage, node_lru)) };

        if page.is_null() || page == first {
            break;
        }

        if first.is_null() {
            first = page;
        }

        // SAFETY: a queued page's segment index is in range.
        let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
        // SAFETY: the segment is live; the C takes its lock here.
        unsafe { (*seg).lock.lock() };

        // SAFETY: the segment lock and page-queues lock are held.
        unsafe { seg_remove_active_page(seg, page) };
        let object = unsafe { (*page).object };
        // SAFETY: a queued page has a live object.
        let locked = unsafe { (*object).lock.try_lock() };

        if !locked {
            // SAFETY: the page was removed above.
            unsafe { seg_add_active_page(seg, page) };
            // SAFETY: the segment lock was taken above.
            unsafe { (*seg).lock.unlock() };
            continue;
        }

        // SAFETY: the object lock is held.
        if !unsafe { can_move(page) } {
            // SAFETY: as above.
            unsafe { seg_add_active_page(seg, page) };
            unsafe { (*object).lock.unlock() };
            unsafe { (*seg).lock.unlock() };
            continue;
        }

        return page;
    }

    null_mut()
}

/// `vm_page_pull_inactive_page()` in C.
///
/// # Safety
///
/// Same contract as [`pull_active_page()`].
unsafe fn pull_inactive_page(external: bool) -> *mut VmPage {
    // SAFETY: the state is live.
    let page_list = if external {
        unsafe { addr_of_mut!((*state()).inactive_lru.external) }
    } else {
        unsafe { addr_of_mut!((*state()).inactive_lru.internal) }
    };
    let mut first: *mut VmPage = null_mut();

    loop {
        // SAFETY: the page-queues lock is held and the LRU head is live.
        let page =
            unsafe { first_page(page_list, offset_of!(VmPage, node_lru)) };

        if page.is_null() || page == first {
            break;
        }

        if first.is_null() {
            first = page;
        }

        // SAFETY: a queued page's segment index is in range.
        let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
        // SAFETY: the segment is live; the C takes its lock here.
        unsafe { (*seg).lock.lock() };

        // SAFETY: the segment lock and page-queues lock are held.
        unsafe { seg_remove_inactive_page(seg, page) };
        let object = unsafe { (*page).object };
        // SAFETY: a queued page has a live object.
        let locked = unsafe { (*object).lock.try_lock() };

        if !locked {
            // SAFETY: the page was removed above.
            unsafe { seg_add_inactive_page(seg, page) };
            // SAFETY: the segment lock was taken above.
            unsafe { (*seg).lock.unlock() };
            continue;
        }

        // SAFETY: the object lock is held.
        if !unsafe { can_move(page) } {
            // SAFETY: as above.
            unsafe { seg_add_inactive_page(seg, page) };
            unsafe { (*object).lock.unlock() };
            unsafe { (*seg).lock.unlock() };
            continue;
        }

        return page;
    }

    null_mut()
}

/// `vm_page_seg_page_available()` in C.
///
/// # Safety
///
/// `seg` must be a live segment.
unsafe fn seg_page_available(seg: *const VmPageSeg) -> bool {
    // SAFETY: the caller promises a live segment.
    unsafe { (*seg).nr_free_pages > (*seg).high_free_pages }
}

/// `vm_page_seg_usable()` in C.
///
/// # Safety
///
/// `seg` must be a live segment.
unsafe fn seg_usable(seg: *const VmPageSeg) -> bool {
    // SAFETY: the caller promises a live segment.
    let queued = unsafe {
        (*seg).active_pages.internal.nr_pages
            + (*seg).active_pages.external.nr_pages
            + (*seg).inactive_pages.internal.nr_pages
            + (*seg).inactive_pages.external.nr_pages
    };

    queued == 0 || unsafe { (*seg).nr_free_pages >= (*seg).high_free_pages }
}

/// `vm_page_seg_double_lock()` in C.
///
/// # Safety
///
/// `seg1` and `seg2` must be live, distinct segments, unlocked.
unsafe fn seg_double_lock(seg1: *mut VmPageSeg, seg2: *mut VmPageSeg) {
    // SAFETY: the caller promises two distinct, unlocked segments.
    unsafe {
        if (seg1 as usize) < (seg2 as usize) {
            (*seg1).lock.lock();
            (*seg2).lock.lock();
        } else {
            (*seg2).lock.lock();
            (*seg1).lock.lock();
        }
    }
}

/// `vm_page_seg_double_unlock()` in C.
///
/// # Safety
///
/// Both locks must be held.
unsafe fn seg_double_unlock(seg1: *mut VmPageSeg, seg2: *mut VmPageSeg) {
    // SAFETY: the caller promises both locks are held.
    unsafe {
        (*seg1).lock.unlock();
        (*seg2).lock.unlock();
    }
}

/// `vm_page_seg_balance_page()` in C.
///
/// # Safety
///
/// Both segments must be live and unlocked; the page-queues and free locks
/// must not be held.  Returns with every lock released.
unsafe fn seg_balance_page(
    seg: *mut VmPageSeg,
    remote_seg: *mut VmPageSeg,
    priv_alloc: bool,
) -> bool {
    // SAFETY: the caller promises the segments and the two globals.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).lock();
        (*addr_of_mut!(vm_page_queue_free_lock)).lock();
    }
    // SAFETY: the caller promises two distinct, unlocked segments.
    unsafe { seg_double_lock(seg, remote_seg) };

    let unusable = unsafe { !seg_usable(seg) };
    let remote_full = if priv_alloc {
        // SAFETY: the segment lock is held.
        unsafe { (*remote_seg).nr_free_pages == 0 }
    } else {
        // SAFETY: the segment lock is held.
        unsafe { !seg_page_available(remote_seg) }
    };

    if !unusable || remote_full {
        // SAFETY: the locks were taken above.
        unsafe {
            seg_double_unlock(seg, remote_seg);
            (*addr_of_mut!(vm_page_queue_free_lock)).unlock();
            (*addr_of_mut!(vm_page_queue_lock)).unlock();
        }
        return false;
    }

    let mut was_active = true;
    // SAFETY: the locks the pull requires are held.
    let mut src = unsafe { seg_pull_active_page(seg, false) };
    if src.is_null() {
        // SAFETY: as above.
        src = unsafe { seg_pull_active_page(seg, true) };
    }

    if src.is_null() {
        was_active = false;
        // SAFETY: as above.
        src = unsafe { seg_pull_inactive_page(seg, false) };
        if src.is_null() {
            // SAFETY: as above.
            src = unsafe { seg_pull_inactive_page(seg, true) };
        }
    }

    if src.is_null() {
        // SAFETY: the locks were taken above.
        unsafe {
            seg_double_unlock(seg, remote_seg);
            (*addr_of_mut!(vm_page_queue_free_lock)).unlock();
            (*addr_of_mut!(vm_page_queue_lock)).unlock();
        }
        return false;
    }

    // SAFETY: the remote segment lock is held and the remote segment has
    // free pages, as the C's check above established.
    let dest = unsafe { seg_alloc_from_buddy(remote_seg, 0) };

    // SAFETY: the locks were taken above.
    unsafe {
        seg_double_unlock(seg, remote_seg);
        (*addr_of_mut!(vm_page_queue_free_lock)).unlock();
    }

    if dest.is_null() {
        die(c"vm_page_seg_balance_page", c"vm_page: no dest page");
    }

    // SAFETY: the source object lock is held.
    if !was_active
        && !unsafe { (*src).is_reference() }
        && unsafe { pmap_is_referenced((*src).phys_addr) } != 0
    {
        // SAFETY: the page is live.
        unsafe { (*src).set_reference(true) };
    }

    // SAFETY: the source page holds its object lock and the page-queues
    // lock.
    let object = unsafe { (*src).object };
    let offset = unsafe { (*src).offset };
    // SAFETY: the C's own contract for `vm_page_remove()`.
    unsafe { vm_page_remove(src) };

    // SAFETY: the page is live.
    unsafe { remove_mappings(src) };

    // SAFETY: both pages are live and the body copy is the C's memcpy of
    // `VM_PAGE_BODY_SIZE` bytes.
    unsafe {
        set_type(NonNull::new_unchecked(dest), 0, (*src).page_type());
        (*dest).copy_body_from(&*src);
    }
    // SAFETY: both pages are live and their object locks are held.
    unsafe {
        vm_resident::copy(
            NonNull::new_unchecked(src),
            NonNull::new_unchecked(dest),
        )
    };

    // SAFETY: the destination page is live.
    if !unsafe { (*src).is_dirty() } {
        unsafe { pmap_clear_modify((*dest).phys_addr) };
    }
    // SAFETY: the destination page is live and off every queue.
    unsafe { (*dest).set_busy(false) };

    // SAFETY: the free lock and the segment lock are taken in the C's
    // order, and the source page is live.
    unsafe {
        (*addr_of_mut!(vm_page_queue_free_lock)).lock();
        vm_resident::init(&mut *src);
        (*src).set_free(true);
        (*seg).lock.lock();
        set_type(NonNull::new_unchecked(src), 0, VM_PT_FREE);
        seg_free_to_buddy(seg, src, 0);
        (*seg).lock.unlock();
        (*addr_of_mut!(vm_page_queue_free_lock)).unlock();
    }

    // SAFETY: the destination holds the moved page's object lock, the
    // page-queues lock is held, and the destination is not queued.
    unsafe {
        vm_page_insert(dest, object, offset);
        (*object).lock.unlock();

        if was_active {
            activate(dest);
        } else {
            deactivate(dest);
        }

        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }

    true
}

/// `vm_page_seg_balance()` in C.
///
/// # Safety
///
/// `seg` must be a live segment; the C's caller holds no page lock around
/// it.
unsafe fn seg_balance(seg: *mut VmPageSeg, priv_alloc: bool) -> bool {
    let mut i = segs_size().wrapping_sub(1);

    while i < segs_size() {
        // SAFETY: `i` is inside the segment table.
        let remote_seg = seg_ptr(i);

        if remote_seg != seg
            && unsafe { seg_balance_page(seg, remote_seg, priv_alloc) }
        {
            return true;
        }

        i = i.wrapping_sub(1);
    }

    false
}

/// `vm_page_seg_compute_high_active_page()` in C.
///
/// # Safety
///
/// `seg` must be a live segment with its lock held.
unsafe fn seg_compute_high_active_page(seg: *mut VmPageSeg) {
    // SAFETY: the caller promises the segment and its lock.
    let nr_pages = unsafe {
        (*seg).active_pages.internal.nr_pages
            + (*seg).active_pages.external.nr_pages
            + (*seg).inactive_pages.internal.nr_pages
            + (*seg).inactive_pages.external.nr_pages
    };

    // SAFETY: the caller holds the segment lock.
    unsafe {
        (*seg).high_active_pages = nr_pages
            .wrapping_mul(VM_PAGE_HIGH_ACTIVE_PAGE_NUM)
            / VM_PAGE_HIGH_ACTIVE_PAGE_DENOM;
    }
}

/// `vm_page_seg_refill_inactive()` in C.
///
/// # Safety
///
/// `seg` must be a live segment whose lock is not held; the page-queues lock
/// must be held.
unsafe fn seg_refill_inactive(seg: *mut VmPageSeg) {
    // SAFETY: the caller promises the segment; the C takes its lock here.
    unsafe { (*seg).lock.lock() };

    // SAFETY: the segment and page-queues locks are held.
    unsafe { seg_compute_high_active_page(seg) };

    loop {
        // SAFETY: the segment lock is held.
        let actives = unsafe {
            (*seg).active_pages.internal.nr_pages
                + (*seg).active_pages.external.nr_pages
        };
        if actives <= unsafe { (*seg).high_active_pages } {
            break;
        }

        // SAFETY: the segment and page-queues locks are held.
        let mut page = unsafe { seg_pull_active_page(seg, true) };
        if page.is_null() {
            // SAFETY: as above.
            page = unsafe { seg_pull_active_page(seg, false) };
        }

        if page.is_null() {
            break;
        }

        // SAFETY: the pull holds the object lock; the page is live and the
        // segment lock is held.
        unsafe {
            (*page).set_reference(false);
            pmap_clear_reference((*page).phys_addr);
            seg_add_inactive_page(seg, page);
            (*(*page).object).lock.unlock();
        }
    }

    // SAFETY: the lock was taken above.
    unsafe { (*seg).lock.unlock() };
}

/// `vm_page_load()` in C.
pub(crate) fn load(seg_index: c_uint, start: VmOffset, end: VmOffset) {
    let index = seg_index as usize;
    // SAFETY: the architecture loader passes the segment index the C
    // declared, below `VM_PAGE_MAX_SEGS`.
    let seg = unsafe { boot_seg(index) };

    // SAFETY: the state is live and the boot loader runs single-threaded.
    unsafe {
        (*seg).start = start;
        (*seg).end = end;
        (*seg).heap_present = false;
        (*state()).segs_size += 1;
    }
}

/// `vm_page_load_heap()` in C.
pub(crate) fn load_heap(seg_index: c_uint, start: VmOffset, end: VmOffset) {
    let index = seg_index as usize;
    // SAFETY: the architecture loader passes the segment index the C
    // declared, below `VM_PAGE_MAX_SEGS`.
    let seg = unsafe { boot_seg(index) };

    // SAFETY: the state is live and the boot loader runs single-threaded.
    unsafe {
        (*seg).avail_start = start;
        (*seg).avail_end = end;
        (*seg).heap_present = true;
    }
}

/// `vm_page_ready()` in C.
pub(crate) fn is_ready() -> bool {
    // SAFETY: the state is live for the kernel's lifetime.
    unsafe { (*state()).is_ready }
}

/// `vm_page_select_alloc_seg()` in C.
fn select_alloc_seg(selector: c_uint) -> usize {
    let seg_index = match selector {
        SEL_DMA => SEG_DMA,
        SEL_DMA32 => SEG_DMA32,
        SEL_DIRECTMAP => SEG_DIRECTMAP,
        SEL_HIGHMEM => SEG_HIGHMEM,
        _ => die(c"vm_page_select_alloc_seg", c"vm_page: invalid selector"),
    };

    // The C `MIN(vm_page_segs_size - 1, seg_index)` wraps to all ones on an
    // empty table, so the selector wins.
    min(unsafe { (*state()).segs_size }.wrapping_sub(1), seg_index) as usize
}

/// `vm_page_boot_seg_loaded()` in C.
///
/// # Safety
///
/// `seg` must be a live boot segment.
unsafe fn boot_seg_loaded(seg: *const BootSeg) -> bool {
    // SAFETY: the caller promises a live boot segment.
    unsafe { (*seg).end != 0 }
}

/// `vm_page_check_boot_segs()` in C.
fn check_boot_segs() {
    if unsafe { (*state()).segs_size } == 0 {
        die(
            c"vm_page_check_boot_segs",
            c"vm_page: no physical memory loaded",
        );
    }

    let mut i = 0;
    while i < VM_PAGE_MAX_SEGS {
        let expect_loaded = i < segs_size();
        // SAFETY: `i` is inside the boot table.
        let seg = unsafe { boot_seg(i) };

        // SAFETY: the descriptor is live.
        if unsafe { boot_seg_loaded(seg) } == expect_loaded {
            i += 1;
            continue;
        }

        die(
            c"vm_page_check_boot_segs",
            c"vm_page: invalid boot segment table",
        );
    }
}

/// `vm_page_boot_seg_size()` in C.
///
/// # Safety
///
/// `seg` must be a live boot segment.
unsafe fn boot_seg_size(seg: *const BootSeg) -> VmOffset {
    // SAFETY: the caller promises a live boot segment.
    unsafe { (*seg).end - (*seg).start }
}

/// `vm_page_boot_seg_avail_size()` in C.
///
/// # Safety
///
/// `seg` must be a live boot segment.
unsafe fn boot_seg_avail_size(seg: *const BootSeg) -> VmOffset {
    // SAFETY: the caller promises a live boot segment.
    unsafe { (*seg).avail_end - (*seg).avail_start }
}

/// `vm_page_bootalloc()` in C: an early allocation from the boot table.  The
/// C's exhaustion `panic()` is final.
pub(crate) fn bootalloc(size: VmSize) -> VmOffset {
    let mut i = select_alloc_seg(SEL_DIRECTMAP);

    while i < segs_size() {
        // SAFETY: `i` is inside the boot table.
        let seg = unsafe { boot_seg(i) };

        // SAFETY: the descriptor is live.
        if size <= unsafe { boot_seg_avail_size(seg) } {
            // SAFETY: the descriptor is live and the boot loader is
            // single-threaded.
            let pa = unsafe { (*seg).avail_start };
            unsafe { (*seg).avail_start += round_page(size) };
            return pa;
        }

        i = i.wrapping_sub(1);
    }

    die(
        c"vm_page_bootalloc",
        c"vm_page: no physical memory available",
    )
}

/// `vm_page_setup()` in C: build the page table and release the segments.
pub(crate) fn setup() {
    check_boot_segs();

    // SAFETY: the state is live and setup runs once, single-threaded.
    unsafe {
        List::init_head_at(NonNull::new_unchecked(
            &raw mut (*state()).active_lru.internal,
        ));
        List::init_head_at(NonNull::new_unchecked(
            &raw mut (*state()).active_lru.external,
        ));
        List::init_head_at(NonNull::new_unchecked(
            &raw mut (*state()).inactive_lru.internal,
        ));
        List::init_head_at(NonNull::new_unchecked(
            &raw mut (*state()).inactive_lru.external,
        ));
    }

    let mut nr_pages: usize = 0;
    let mut i = 0;
    while i < segs_size() {
        // SAFETY: `i` is inside the boot table.
        let seg = unsafe { boot_seg(i) };
        // SAFETY: the descriptor is live.
        nr_pages += atop(unsafe { boot_seg_size(seg) });
        i += 1;
    }

    let table_size = round_page(nr_pages.wrapping_mul(size_of::<VmPage>()));
    // SAFETY: `printf` is the kernel's formatter and the values match the
    // C's `%lu` arguments.
    unsafe {
        printf(
            c"vm_page: page table size: %lu entries (%luk)\n".as_ptr(),
            nr_pages as c_ulong,
            (table_size >> 10) as c_ulong,
        )
    };

    // SAFETY: `pmap_steal_memory()` is the boot allocator and panics if the
    // kernel address space is exhausted, as the C did.
    let table =
        unsafe { crate::vm::vm_resident_ffi::pmap_steal_memory(table_size) }
            as *mut VmPage;
    let va = table as usize;

    let mut table = table;
    let mut i = 0;
    while i < segs_size() {
        // SAFETY: `i` is inside the segment and boot tables.
        let seg = seg_ptr(i);
        let boot = unsafe { boot_seg(i) };

        // SAFETY: the page table has room for every segment's descriptors,
        // and setup is single-threaded.
        unsafe {
            seg_init(seg, (*boot).start, (*boot).end, table);

            let mut page =
                (*seg).pages.add(atop((*boot).avail_start - (*boot).start));
            let end =
                (*seg).pages.add(atop((*boot).avail_end - (*boot).start));

            while page < end {
                (*page).set_page_type(VM_PT_FREE);
                seg_free_to_buddy(seg, page, 0);
                page = page.add(1);
            }

            table = table.add(atop(seg_size(seg)));
        }

        i += 1;
    }

    let mut va = va;
    while va < (table as usize) {
        // SAFETY: `pmap_extract()` reads the boot pmap for a mapped address.
        let pa = unsafe { pmap_extract(kernel_pmap, va) };
        // SAFETY: the address was just mapped by the pmap over the page
        // table, so it has a descriptor.
        let Some(page) = lookup_pa(pa) else {
            die(c"vm_page_setup", c"vm_page: page table not in any segment");
        };

        // SAFETY: the descriptor is live and setup is single-threaded.
        unsafe { (*page.as_ptr()).set_page_type(VM_PT_TABLE) };
        va = va.wrapping_add(PAGE_SIZE);
    }

    // SAFETY: the state is live and setup runs once.
    unsafe { (*state()).is_ready = true };
}

/// `vm_page_manage()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor the kernel is handing to the page module,
/// not yet on any list.
pub(crate) unsafe fn manage(page: *mut VmPage) {
    // SAFETY: the caller promises the descriptor.
    unsafe {
        set_type(NonNull::new_unchecked(page), 0, VM_PT_FREE);
        let seg = seg_ptr((*page).seg_index() as usize);
        seg_free_to_buddy(seg, page, 0);
    }
}

/// `vm_page_lookup_pa()` in C.
pub(crate) fn lookup_pa(pa: VmOffset) -> Option<NonNull<VmPage>> {
    let mut i = 0;
    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: `i` is inside the segment table.
        let (start, end) = unsafe { ((*seg).start, (*seg).end) };

        if start <= pa && pa < end {
            // SAFETY: the physical address is inside the segment.
            let page = unsafe { (*seg).pages.add(atop(pa - start)) };
            return NonNull::new(page);
        }

        i += 1;
    }

    None
}

/// `vm_page_lookup_seg()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor.
unsafe fn lookup_seg(page: *const VmPage) -> *mut VmPageSeg {
    // SAFETY: the caller promises the live descriptor.
    let pa = unsafe { (*page).phys_addr };
    let mut i = 0;

    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: `i` is inside the segment table.
        if pa >= unsafe { (*seg).start } && pa < unsafe { (*seg).end } {
            return seg;
        }

        i += 1;
    }

    null_mut()
}

/// `vm_page_check()` in C, whose `panic()` calls are final.
///
/// # Safety
///
/// `page` must be a live descriptor, and the caller must hold whatever lock
/// the C's `VM_PAGE_CHECK` call sites held.
pub(crate) unsafe fn check(page: *const VmPage) {
    // SAFETY: the caller promises the live descriptor.
    if unsafe { (*page).is_fictitious() } {
        if unsafe { (*page).is_private() } {
            die(
                c"vm_page_check",
                c"vm_page: page both fictitious and private",
            );
        }

        if unsafe { (*page).phys_addr } != unsafe { vm_page_fictitious_addr } {
            die(c"vm_page_check", c"vm_page: invalid fictitious page");
        }

        return;
    }

    if unsafe { (*page).phys_addr } == unsafe { vm_page_fictitious_addr } {
        die(
            c"vm_page_check",
            c"vm_page: real page has fictitious address",
        );
    }

    // SAFETY: as above.
    let seg = unsafe { lookup_seg(page) };

    if seg.is_null() {
        if !unsafe { (*page).is_private() } {
            die(
                c"vm_page_check",
                c"vm_page: page claims it's managed but not in any segment",
            );
        }
        return;
    }

    if unsafe { (*page).is_private() } {
        if unsafe { pageable(page) } {
            die(c"vm_page_check", c"vm_page: private page is pageable");
        }

        // SAFETY: the page's physical address is inside a segment.
        let Some(real_page) = lookup_pa(unsafe { (*page).phys_addr }) else {
            die(
                c"vm_page_check",
                c"vm_page: couldn't allocate page underlying private page",
            );
        };

        // SAFETY: the descriptor is live.
        if unsafe { pageable(real_page.as_ptr()) } {
            die(
                c"vm_page_check",
                c"vm_page: page underlying private page is pageable",
            );
        }

        // SAFETY: the descriptor is live.
        if unsafe { real_page.as_ref().page_type() } == VM_PT_FREE
            || unsafe { real_page.as_ref().order() } != VM_PAGE_ORDER_UNLISTED
        {
            die(
                c"vm_page_check",
                c"vm_page: page underlying private pagei is free",
            );
        }
        return;
    }

    // SAFETY: the segment is live.
    let index = unsafe { seg_index(seg) };
    if index != unsafe { (*page).seg_index() } as usize {
        die(c"vm_page_check", c"vm_page: page segment mismatch");
    }
}

/// `vm_page_alloc_pa()` in C.  Returns with `vm_page_queue_free_lock` held.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be ready to
/// release it, as the C's callers were.
pub(crate) unsafe fn alloc_pa(
    order: c_uint,
    selector: c_uint,
    type_: u16,
) -> *mut VmPage {
    let seg_index = select_alloc_seg(selector);

    loop {
        // SAFETY: the callers never hold the free lock on entry.
        unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).lock() };

        let mut i = seg_index;
        while i < segs_size() {
            // SAFETY: the free lock is held, as the backend requires.
            let page = unsafe { seg_alloc(seg_ptr(i), order, type_) };

            if !page.is_null() {
                return page;
            }

            i = i.wrapping_sub(1);
        }

        let thread = current_thread();
        if thread.is_null() || unsafe { (*thread).vm_privilege } != 0 {
            // SAFETY: the lock was taken above.
            unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).unlock() };

            let mut i = seg_index;
            while i < segs_size() {
                // SAFETY: the C's balancing caller holds no page lock.
                if unsafe { seg_balance(seg_ptr(i), true) } {
                    break;
                }

                i = i.wrapping_sub(1);
            }

            if i < segs_size() {
                continue;
            }

            die(
                c"vm_page_alloc_pa",
                c"vm_page: privileged thread unable to allocate page",
            );
        }

        return null_mut();
    }
}

/// `vm_page_free_pa()` in C.
///
/// # Safety
///
/// `page` must be the first of `1 << order` descriptors the module handed
/// out, and the caller must hold `vm_page_queue_free_lock`.
pub(crate) unsafe fn free_pa(page: *mut VmPage, order: c_uint) {
    // SAFETY: the caller promises the live descriptor.
    let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
    // SAFETY: the free lock is held, as the backend requires.
    unsafe { seg_free(seg, page, order) };
}

/// `vm_page_seg_name()` in C at the caller's index, whose unknown index is
/// final.
fn name_ptr(seg_index: c_uint) -> *const c_char {
    match seg_name(seg_index) {
        Some(name) => name.as_ptr(),
        None => die(c"vm_page_seg_name", c"vm_page: invalid segment index"),
    }
}

/// `vm_page_info_all()` in C.
pub(crate) fn info_all() {
    let mut i = 0;
    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: `i` is inside the segment table.
        let pages =
            unsafe { (*seg).pages_end.offset_from((*seg).pages) as usize };
        let name = name_ptr(i as c_uint);

        // SAFETY: `printf` is the kernel's formatter and the values match
        // the C's `%s` and `%lu` arguments.
        unsafe {
            printf(
                c"vm_page: %s: pages: %lu (%luM), free: %lu (%luM)\n".as_ptr(),
                name,
                pages as c_ulong,
                (pages >> (20 - PAGE_SHIFT)) as c_ulong,
                (*seg).nr_free_pages as c_ulong,
                ((*seg).nr_free_pages >> (20 - PAGE_SHIFT)) as c_ulong,
            );
            printf(
                c"vm_page: %s: min:%lu low:%lu high:%lu\n".as_ptr(),
                name,
                (*seg).min_free_pages as c_ulong,
                (*seg).low_free_pages as c_ulong,
                (*seg).high_free_pages as c_ulong,
            );
        }

        i += 1;
    }
}

/// `vm_page_seg_end()` in C.
pub(crate) fn seg_end(selector: c_uint) -> VmOffset {
    let index = select_alloc_seg(selector);
    // SAFETY: `index` came out of the selector table.
    unsafe { (*seg_ptr(index)).end }
}

/// `vm_page_boot_table_size()` in C.
fn boot_table_size() -> usize {
    let mut nr_pages = 0;
    let mut i = 0;

    while i < segs_size() {
        // SAFETY: `i` is inside the boot table.
        let seg = unsafe { boot_seg(i) };
        // SAFETY: the descriptor is live.
        nr_pages += atop(unsafe { boot_seg_size(seg) });
        i += 1;
    }

    nr_pages
}

/// `vm_page_table_size()` in C.
pub(crate) fn table_size() -> usize {
    if !is_ready() {
        return boot_table_size();
    }

    let mut nr_pages = 0;
    let mut i = 0;
    while i < segs_size() {
        // SAFETY: the segment is live.
        nr_pages += atop(unsafe { seg_size(seg_ptr(i)) });
        i += 1;
    }

    nr_pages
}

/// `vm_page_table_index()` in C, whose missing address is final.
pub(crate) fn table_index(pa: VmOffset) -> usize {
    let mut index = 0;
    let mut i = 0;

    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: the segment is live.
        let (start, end) = unsafe { ((*seg).start, (*seg).end) };

        if start <= pa && pa < end {
            return index + atop(pa - start);
        }

        index += atop(unsafe { seg_size(seg) });
        i += 1;
    }

    die(c"vm_page_table_index", c"vm_page: invalid physical address")
}

/// `vm_page_mem_size()` in C.
pub(crate) fn mem_size() -> VmOffset {
    let mut total = 0;
    let mut i = 0;

    while i < segs_size() {
        // SAFETY: the segment is live.
        total += unsafe { seg_size(seg_ptr(i)) };
        i += 1;
    }

    total
}

/// `vm_page_mem_free()` in C.
pub(crate) fn mem_free() -> usize {
    let mut total = 0;
    let mut i = 0;

    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: the segment is live.
        total += unsafe { (*seg).nr_free_pages };
        i += 1;
    }

    total
}

/// `vm_page_unwire()` in C.
///
/// # Safety
///
/// `page` must be a live page whose object lock and page-queues lock the
/// caller holds.
pub(crate) unsafe fn unwire(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and the object lock.
    unsafe { check(page) };

    // SAFETY: the caller's locks serialize the bitfield.
    let count = unsafe { (*page).wire_count() }.wrapping_sub(1);
    unsafe { (*page).set_wire_count(count) };

    if count != 0
        || unsafe { (*page).is_fictitious() }
        || unsafe { (*page).is_private() }
    {
        return;
    }

    // SAFETY: the page is live.
    let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
    // SAFETY: the C takes the segment lock under the page-queues lock.
    unsafe {
        (*seg).lock.lock();
        seg_add_active_page(seg, page);
        (*seg).lock.unlock();
    }
    vm_resident::VM_PAGE_WIRE_COUNT.fetch_sub(1, Ordering::Relaxed);
}

/// `vm_page_deactivate()` in C.
///
/// # Safety
///
/// `page` must be a live page and the caller must hold the page-queues lock.
pub(crate) unsafe fn deactivate(page: *mut VmPage) {
    // SAFETY: the caller promises the live page.
    unsafe { check(page) };

    if unsafe { (*page).is_active() }
        || (unsafe { (*page).is_inactive() }
            && unsafe { (*page).is_reference() })
    {
        if !unsafe { (*page).is_fictitious() }
            && !unsafe { (*page).is_private() }
            && !unsafe { (*page).is_absent() }
        {
            // SAFETY: the page is live and its physical address is real.
            unsafe { pmap_clear_reference((*page).phys_addr) };
        }

        // SAFETY: the page-queues lock is held.
        unsafe {
            (*page).set_reference(false);
            queues_remove(page);
        }
    }

    if unsafe { (*page).wire_count() } == 0
        && !unsafe { (*page).is_fictitious() }
        && !unsafe { (*page).is_private() }
        && !unsafe { (*page).is_inactive() }
    {
        // SAFETY: the page is live.
        let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
        // SAFETY: the C takes the segment lock under the page-queues lock.
        unsafe {
            (*seg).lock.lock();
            seg_add_inactive_page(seg, page);
            (*seg).lock.unlock();
        }
    }
}

/// `vm_page_activate()` in C, whose double activation is final.
///
/// # Safety
///
/// `page` must be a live page and the caller must hold the page-queues lock.
pub(crate) unsafe fn activate(page: *mut VmPage) {
    // SAFETY: the caller promises the live page.
    unsafe { check(page) };

    // SAFETY: the page-queues lock is held.
    unsafe { queues_remove(page) };

    if unsafe { (*page).wire_count() } == 0
        && !unsafe { (*page).is_fictitious() }
        && !unsafe { (*page).is_private() }
    {
        // SAFETY: the page is live.
        let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);

        if unsafe { (*page).is_active() } {
            die(c"vm_page_activate", c"vm_page_activate: already active");
        }

        // SAFETY: the C takes the segment lock under the page-queues lock.
        unsafe {
            (*seg).lock.lock();
            seg_add_active_page(seg, page);
            (*seg).lock.unlock();
        }
    }
}

/// `vm_page_queues_remove()` in C.
///
/// # Safety
///
/// `page` must be a live page and the caller must hold the page-queues lock
/// and, when the page is queued, its object lock, as the C required.
pub(crate) unsafe fn queues_remove(page: *mut VmPage) {
    if !unsafe { (*page).is_active() } && !unsafe { (*page).is_inactive() } {
        return;
    }

    // SAFETY: the page is live.
    let seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
    // SAFETY: the C takes the segment lock under the page-queues lock.
    unsafe {
        (*seg).lock.lock();

        if (*page).is_active() {
            seg_remove_active_page(seg, page);
        } else {
            seg_remove_inactive_page(seg, page);
        }

        (*seg).lock.unlock();
    }
}

/// `vm_page_check_usable()` in C.  Returns with `vm_page_queue_free_lock`
/// held, as the C did.
unsafe fn check_usable() -> bool {
    // SAFETY: the caller never holds the free lock.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).lock() };

    let mut i = 0;
    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: the C takes the segment lock under the free lock.
        unsafe {
            (*seg).lock.lock();
        }
        let usable = unsafe { seg_usable(seg) };
        // SAFETY: the lock was taken above.
        unsafe {
            (*seg).lock.unlock();
        }

        if !usable {
            return false;
        }

        i += 1;
    }

    vm_resident::VM_PAGE_EXTERNAL_LAUNDRY_COUNT.store(-1, Ordering::Relaxed);

    // SAFETY: the free lock is held and the state is live.
    unsafe {
        (*state()).alloc_paused = false;
        thread_wakeup_prim(
            addr_of_mut!((*state()).alloc_paused).cast(),
            0,
            THREAD_AWAKENED,
        );
    }

    true
}

/// `vm_page_may_balance()` in C.
unsafe fn may_balance() -> bool {
    let mut i = 0;
    while i < segs_size() {
        let seg = seg_ptr(i);
        // SAFETY: the C takes the segment lock for the probe.
        unsafe {
            (*seg).lock.lock();
        }
        let available = unsafe { seg_page_available(seg) };
        // SAFETY: the lock was taken above.
        unsafe {
            (*seg).lock.unlock();
        }

        if available {
            return true;
        }

        i += 1;
    }

    false
}

/// `vm_page_balance_once()` in C.
unsafe fn balance_once() -> bool {
    let mut i = 0;
    while i < segs_size() {
        // SAFETY: `i` is inside the segment table.
        if unsafe { seg_balance(seg_ptr(i), false) } {
            return true;
        }

        i += 1;
    }

    false
}

/// `vm_page_balance()` in C.  Returns with `vm_page_queue_free_lock` held.
pub(crate) unsafe fn balance() -> bool {
    while unsafe { may_balance() } {
        if !unsafe { balance_once() } {
            break;
        }
    }

    // SAFETY: the C's balancing caller holds no page lock.
    unsafe { check_usable() }
}

/// `vm_page_evict_one()` in C.
///
/// # Safety
///
/// No page lock may be held; the C's caller is the pageout path.
unsafe fn evict_one(external: bool, active: bool, alloc_paused: bool) -> bool {
    // SAFETY: the global is the C `ipc_port_t`.
    if !external && !port_valid(unsafe { memory_manager_default }) {
        return false;
    }

    let mut seg: *mut VmPageSeg = null_mut();
    let mut page: *mut VmPage = null_mut();
    let mut object: *mut VmObject;
    let mut double_paging = false;
    let mut reclaim;

    loop {
        // SAFETY: the caller holds no page lock.
        unsafe { (*addr_of_mut!(vm_page_queue_lock)).lock() };

        if page.is_null() {
            // SAFETY: the page-queues lock is held.
            page = unsafe {
                if active {
                    pull_active_page(external)
                } else {
                    pull_inactive_page(external)
                }
            };

            if page.is_null() {
                // SAFETY: the lock was taken above.
                unsafe { (*addr_of_mut!(vm_page_queue_lock)).unlock() };
                return false;
            }

            // SAFETY: the page came off a queue.
            seg = seg_ptr(unsafe { (*page).seg_index() } as usize);
        } else {
            // SAFETY: the second pass re-takes the locks the C did.
            unsafe {
                (*seg).lock.lock();
                (*(*page).object).lock.lock();
            }
        }

        // SAFETY: the page is live and its object lock is held.
        object = unsafe { (*page).object };

        if !active
            && (unsafe { (*page).is_reference() }
                || unsafe { pmap_is_referenced((*page).phys_addr) } != 0)
        {
            // SAFETY: the segment, object and page-queues locks are held;
            // the C reactivates the page and restarts.
            unsafe {
                seg_add_active_page(seg, page);
                (*seg).lock.unlock();
                (*object).lock.unlock();
                vm_stat.reactivations += 1;
                let thread = current_thread();
                if !thread.is_null() {
                    // SAFETY: the C's `current_task()->reactivations++`; the
                    // running thread's task is live for as long as the
                    // thread.
                    let task = (*thread).task;
                    (*task).reactivations =
                        (*task).reactivations.wrapping_add(1);
                }
                (*addr_of_mut!(vm_page_queue_lock)).unlock();
            }

            seg = null_mut();
            page = null_mut();
            continue;
        }

        // SAFETY: the page is live.
        unsafe { remove_mappings(page) };

        reclaim = !unsafe { (*page).is_dirty() }
            && !unsafe { (*page).is_precious() };

        // SAFETY: the object lock is held.
        if !reclaim {
            if unsafe { (*object).is_internal() }
                || !alloc_paused
                || !port_valid(unsafe { memory_manager_default })
                || unsafe { memory_manager_default_port((*object).pager) } != 0
            {
                double_paging = false;
            } else {
                double_paging = true;
                // SAFETY: the page is live and its object lock is held.
                unsafe { (*page).set_laundry(true) };
            }
        }

        // The `out:` label of the C: release the segment lock, then decide
        // with the object lock still held.
        if !seg.is_null() {
            // SAFETY: the segment lock is held.
            unsafe { (*seg).lock.unlock() };
        }

        if reclaim {
            // SAFETY: `vm_page_free()` is the real C symbol and the
            // page-queues lock is held, as its C caller had it.
            unsafe {
                vm_page_free(page);
                (*addr_of_mut!(vm_page_queue_lock)).unlock();

                if (*object).ref_count == 0
                    && (*object).resident_page_count == 0
                {
                    vm_object_collect(object);
                } else {
                    (*object).lock.unlock();
                }
            }

            return true;
        }

        // SAFETY: the lock was taken above.
        unsafe { (*addr_of_mut!(vm_page_queue_lock)).unlock() };

        if port_valid(unsafe { memory_manager_default }) {
            // SAFETY: the object lock is held and the C calls the object's
            // pager entries.
            unsafe {
                if !(*object).is_pager_initialized() {
                    vm_object_collapse(object);
                }
                if !(*object).is_pager_initialized() {
                    vm_object_pager_create(object);
                }
            }

            if !unsafe { (*object).is_pager_initialized() } {
                die(c"vm_page_seg_evict", c"vm_page_seg_evict");
            }
        }

        // SAFETY: the object lock is held, as the C's flush had it.
        unsafe {
            vm_pageout_page(page, 0, 1);
            (*object).lock.unlock();
        }

        if double_paging {
            continue;
        }

        return true;
    }
}

/// `vm_page_evict_once()` in C.
unsafe fn evict_once(alloc_paused: bool) -> bool {
    // SAFETY: the C tries the four combinations in order, short-circuiting.
    unsafe {
        evict_one(true, false, alloc_paused)
            || evict_one(false, false, alloc_paused)
            || evict_one(true, true, alloc_paused)
            || evict_one(false, true, alloc_paused)
    }
}

/// `vm_page_evict()` in C.  Returns with `vm_page_queue_free_lock` held.
///
/// # Safety
///
/// `should_wait` must be writable, and no page lock may be held.
pub(crate) unsafe fn evict(should_wait: *mut c_int) -> bool {
    // SAFETY: the caller promises the slot.
    unsafe { *should_wait = c_int::from(true) };

    // SAFETY: the caller holds no free lock.
    unsafe {
        (*addr_of_mut!(vm_page_queue_free_lock)).lock();
    }
    vm_resident::VM_PAGE_EXTERNAL_LAUNDRY_COUNT.store(0, Ordering::Relaxed);
    let alloc_paused = unsafe { (*state()).alloc_paused };
    // SAFETY: the lock was taken above.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).unlock() };

    // SAFETY: the caller holds no page-queues lock.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).lock();
    }
    let pause = vm_resident::VM_PAGE_LAUNDRY_COUNT.load(Ordering::Relaxed)
        >= VM_PAGE_MAX_LAUNDRY;
    // SAFETY: the lock was taken above.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }

    if pause {
        // SAFETY: the C returns with the free lock held.
        unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).lock() };
        return false;
    }

    let mut evicted = false;
    let mut i = 0;
    while i < VM_PAGE_MAX_EVICTIONS {
        // SAFETY: no page lock is held here.
        evicted = unsafe { evict_once(alloc_paused) };

        if !evicted {
            break;
        }

        i += 1;
    }

    // SAFETY: the C re-takes the free lock before the decision.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).lock() };

    if vm_resident::VM_PAGE_LAUNDRY_COUNT.load(Ordering::Relaxed) == 0
        && vm_resident::VM_PAGE_EXTERNAL_LAUNDRY_COUNT.load(Ordering::Relaxed)
            == 0
    {
        if evicted {
            // SAFETY: the caller promises the slot.
            unsafe { *should_wait = c_int::from(false) };
            return false;
        }

        // The free lock serializes the latch; a later page only skips the
        // warning.
        if !WARNED.swap(true, Ordering::Relaxed) {
            // SAFETY: `printf` is the kernel's formatter.
            unsafe {
                printf(
                    c"vm_page warning: unable to recycle any page\n".as_ptr(),
                )
            };
        }
    }

    // SAFETY: the lock was taken above.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).unlock() };

    // SAFETY: the C's eviction caller holds no page lock.
    unsafe { check_usable() }
}

/// `vm_page_refill_inactive()` in C.
pub(crate) fn refill_inactive() {
    // SAFETY: the caller holds no page-queues lock.
    unsafe { (*addr_of_mut!(vm_page_queue_lock)).lock() };

    let mut i = 0;
    while i < segs_size() {
        // SAFETY: the page-queues lock is held.
        unsafe { seg_refill_inactive(seg_ptr(i)) };
        i += 1;
    }

    // SAFETY: the lock was taken above.
    unsafe { (*addr_of_mut!(vm_page_queue_lock)).unlock() };
}

/// `vm_page_wait()` in C.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be ready to
/// block.
pub(crate) unsafe fn wait(continuation: Option<unsafe extern "C" fn()>) {
    // SAFETY: the caller promises the free lock is not held.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).lock() };

    if !unsafe { (*state()).alloc_paused } {
        // SAFETY: the lock was taken above.
        unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).unlock() };
        return;
    }

    // SAFETY: the C waits on the static's own address under the free lock.
    unsafe {
        assert_wait(addr_of_mut!((*state()).alloc_paused).cast(), 0);
        (*addr_of_mut!(vm_page_queue_free_lock)).unlock();
        thread_block(continuation);
    }
}

/// `IP_VALID()` of <ipc/ipc_object.h>: not null and not the dead marker.
fn port_valid(port: *mut c_void) -> bool {
    !port.is_null() && port as usize != usize::MAX
}
