// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from vm/vm_page.c and vm/vm_page.h:
//   Copyright (c) 2010-2014 Richard Braun.
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The physical-page module, which `vm/vm_page.c` used to define, and the
//! `struct vm_page` mirror of `vm/vm_page.h`.

use crate::arch::types::VmOffset;
use crate::glue::{vm_page_check, vm_page_queues_remove, vm_page_wire_count};
use crate::kern::list::List;
use crate::kern::queue::QueueEntry;
use crate::vm::types::{VmObject, VmProt};
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::NonNull;

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
}

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

    // SAFETY: the caller promises a live page; `vm_page_check()` is the C
    // validator of that and of its locks.
    unsafe { vm_page_check(ptr) };

    if unsafe { (*ptr).wire_count() } == 0 {
        // SAFETY: the caller holds the two locks the C's call relies on,
        // and `ptr` is a live page.
        unsafe { vm_page_queues_remove(ptr) };

        // SAFETY: the page-queues lock guards the page's flags.
        if !unsafe { (*ptr).is_private() }
            && !unsafe { (*ptr).is_fictitious() }
        {
            // SAFETY: the page-queues lock guards the global count, as in
            // the C.
            unsafe { vm_page_wire_count += 1 };
        }
    }

    // SAFETY: the page is live and the caller's locks serialize the field.
    unsafe { (*ptr).set_wire_count((*ptr).wire_count() + 1) };
}

/// `VM_PAGE_SEG_DMA` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_DMA: c_uint = 0;
/// `VM_PAGE_SEG_DIRECTMAP` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_DIRECTMAP: c_uint = 1;
/// `VM_PAGE_SEG_DMA32` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_DMA32: c_uint = 2;
/// `VM_PAGE_SEG_HIGHMEM` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_HIGHMEM: c_uint = 3;

/// `VM_PAGE_SEG_DMA` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
const SEG_DMA: c_uint = 0;
/// `VM_PAGE_SEG_DIRECTMAP` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
const SEG_DIRECTMAP: c_uint = 1;
/// `VM_PAGE_SEG_DMA32`: the direct-map index, as the non-PAE i686 build
/// aliases it.
#[cfg(target_arch = "x86")]
const SEG_DMA32: c_uint = SEG_DIRECTMAP;
/// `VM_PAGE_SEG_HIGHMEM` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
const SEG_HIGHMEM: c_uint = 2;

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
