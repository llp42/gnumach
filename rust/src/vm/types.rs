// SPDX-License-Identifier: CMU-Mach
// Derived from include/mach/vm_prot.h, include/mach/vm_inherit.h and
// include/mach/vm_statistics.h, from vm/vm_object.h, and from the opaque
// handle of vm/pmap.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! VM scalar and handle types, from `mach/vm_prot.h` and `vm_inherit.h`,
//! the `struct vm_object` and `struct vm_statistics` records, and the VM
//! headers' opaque pointers.

use crate::arch::types::{VmOffset, VmSize};
pub(crate) use crate::arch::vm_param::{PAGE_MASK, PAGE_SHIFT, PAGE_SIZE};
use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use core::ffi::{c_int, c_uint, c_void};

/// `vm_prot_t` of <mach/vm_prot.h>: a set of bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct VmProt(c_int);

impl VmProt {
    /// `VM_PROT_NONE`.
    pub const NONE: Self = Self(0x0);
    /// `VM_PROT_READ`.
    pub const READ: Self = Self(0x1);
    /// `VM_PROT_WRITE`.
    pub const WRITE: Self = Self(0x2);
    /// `VM_PROT_EXECUTE`.
    pub const EXECUTE: Self = Self(0x4);
    /// `VM_PROT_ALL`: read, write and execute.
    pub const ALL: Self = Self(Self::READ.0 | Self::WRITE.0 | Self::EXECUTE.0);
    /// `VM_PROT_NO_CHANGE`: a marker `vm_map_protect` refuses to set.
    pub const NO_CHANGE: Self = Self(0x08);
    /// `VM_PROT_NOTIFY`: a marker bit for callers of `vm_map_protect`.
    pub const NOTIFY: Self = Self(0x10);

    /// The `c_int` the C side passes and stores.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// A protection value from the C side.
    pub const fn from_bits(bits: c_int) -> Self {
        Self(bits)
    }

    /// Whether every bit of `other` is set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for VmProt {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for VmProt {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for VmProt {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl core::ops::BitAndAssign for VmProt {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// `vm_inherit_t` of <mach/vm_inherit.h>.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct VmInherit(c_int);

impl VmInherit {
    /// `VM_INHERIT_SHARE`.
    pub const SHARE: Self = Self(0);
    /// `VM_INHERIT_COPY`.
    pub const COPY: Self = Self(1);
    /// `VM_INHERIT_NONE`.
    pub const NONE: Self = Self(2);

    /// The `c_int` the C side passes and stores.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// An inheritance value from the C side.
    pub const fn from_bits(bits: c_int) -> Self {
        Self(bits)
    }
}

/// `pmap_t`: the machine-dependent physical map of a VM map.
#[repr(C)]
pub struct Pmap {
    _private: [u8; 0],
}

/// The `unsigned int` run of flags in `struct vm_object`, in the C
/// declaration order the compiler packs: `paging_in_progress` in bits 0 to
/// 15, then one bit per boolean flag, `used_for_pageout` at bit 16 through
/// `cached` at bit 28.
const VM_OBJECT_PAGER_INITIALIZED_BIT: u32 = 1 << 18;
const VM_OBJECT_INTERNAL_BIT: u32 = 1 << 21;
const VM_OBJECT_ALIVE_BIT: u32 = 1 << 23;

/// `struct vm_object` of <vm/vm_object.h>: the memory object a page belongs
/// to and an entry maps.
#[repr(C)]
pub struct VmObject {
    /// `memq`: the object's resident-page queue head.
    pub memq: QueueEntry,
    pub lock: SimpleLock,
    pub size: VmSize,
    pub ref_count: c_int,
    pub resident_page_count: usize,
    pub copy: *mut VmObject,
    pub shadow: *mut VmObject,
    pub shadow_offset: VmOffset,
    /// `pager`: the memory-object port, or null.
    pub pager: *mut c_void,
    pub paging_offset: VmOffset,
    pub pager_request: *mut c_void,
    pub pager_name: *mut c_void,
    pub copy_strategy: c_int,
    pub absent_count: c_uint,
    pub all_wanted: c_uint,
    flags: u32,
    pub cached_list: QueueEntry,
    pub last_alloc: VmOffset,
    pub existence_info: *mut c_void,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<VmObject>() == 152);
    assert!(align_of::<VmObject>() == 8);
    assert!(core::mem::offset_of!(VmObject, memq) == 0);
    assert!(core::mem::offset_of!(VmObject, lock) == 16);
    assert!(core::mem::offset_of!(VmObject, size) == 24);
    assert!(core::mem::offset_of!(VmObject, ref_count) == 32);
    assert!(core::mem::offset_of!(VmObject, resident_page_count) == 40);
    assert!(core::mem::offset_of!(VmObject, copy) == 48);
    assert!(core::mem::offset_of!(VmObject, shadow) == 56);
    assert!(core::mem::offset_of!(VmObject, shadow_offset) == 64);
    assert!(core::mem::offset_of!(VmObject, pager) == 72);
    assert!(core::mem::offset_of!(VmObject, paging_offset) == 80);
    assert!(core::mem::offset_of!(VmObject, pager_request) == 88);
    assert!(core::mem::offset_of!(VmObject, pager_name) == 96);
    assert!(core::mem::offset_of!(VmObject, copy_strategy) == 104);
    assert!(core::mem::offset_of!(VmObject, absent_count) == 108);
    assert!(core::mem::offset_of!(VmObject, all_wanted) == 112);
    assert!(core::mem::offset_of!(VmObject, flags) == 116);
    assert!(core::mem::offset_of!(VmObject, cached_list) == 120);
    assert!(core::mem::offset_of!(VmObject, last_alloc) == 136);
    assert!(core::mem::offset_of!(VmObject, existence_info) == 144);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<VmObject>() == 84);
    assert!(align_of::<VmObject>() == 4);
    assert!(core::mem::offset_of!(VmObject, memq) == 0);
    assert!(core::mem::offset_of!(VmObject, lock) == 8);
    assert!(core::mem::offset_of!(VmObject, size) == 12);
    assert!(core::mem::offset_of!(VmObject, ref_count) == 16);
    assert!(core::mem::offset_of!(VmObject, resident_page_count) == 20);
    assert!(core::mem::offset_of!(VmObject, copy) == 24);
    assert!(core::mem::offset_of!(VmObject, shadow) == 28);
    assert!(core::mem::offset_of!(VmObject, shadow_offset) == 32);
    assert!(core::mem::offset_of!(VmObject, pager) == 36);
    assert!(core::mem::offset_of!(VmObject, paging_offset) == 40);
    assert!(core::mem::offset_of!(VmObject, pager_request) == 44);
    assert!(core::mem::offset_of!(VmObject, pager_name) == 48);
    assert!(core::mem::offset_of!(VmObject, copy_strategy) == 52);
    assert!(core::mem::offset_of!(VmObject, absent_count) == 56);
    assert!(core::mem::offset_of!(VmObject, all_wanted) == 60);
    assert!(core::mem::offset_of!(VmObject, flags) == 64);
    assert!(core::mem::offset_of!(VmObject, cached_list) == 68);
    assert!(core::mem::offset_of!(VmObject, last_alloc) == 76);
    assert!(core::mem::offset_of!(VmObject, existence_info) == 80);
};

impl VmObject {
    pub fn is_pager_initialized(&self) -> bool {
        self.flags & VM_OBJECT_PAGER_INITIALIZED_BIT != 0
    }

    pub fn is_internal(&self) -> bool {
        self.flags & VM_OBJECT_INTERNAL_BIT != 0
    }

    pub fn is_alive(&self) -> bool {
        self.flags & VM_OBJECT_ALIVE_BIT != 0
    }
}

/// `struct vm_statistics` of <mach/vm_statistics.h>.
#[repr(C)]
pub struct VmStatistics {
    pub pagesize: c_int,
    pub free_count: c_int,
    pub active_count: c_int,
    pub inactive_count: c_int,
    pub wire_count: c_int,
    pub zero_fill_count: c_int,
    pub reactivations: c_int,
    pub pageins: c_int,
    pub pageouts: c_int,
    pub faults: c_int,
    pub cow_faults: c_int,
    pub lookups: c_int,
    pub hits: c_int,
}

const _: () = assert!(size_of::<VmStatistics>() == 13 * size_of::<c_int>());
const _: () = assert!(align_of::<VmStatistics>() == align_of::<c_int>());
const _: () =
    assert!(core::mem::offset_of!(VmStatistics, reactivations) == 24);

pub use crate::vm::vm_page::VmPage;
