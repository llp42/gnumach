// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Virtual memory maps, which `vm/vm_map.c` defines and
//! `vm/vm_map.h` declares.
//!
//! This module starts as the L1 half of the port: `#[repr(C)]` mirrors
//! of the map, entry, header and copy structures, with the field
//! offsets the C compiler chose pinned by `const` assertions.  While C
//! still defines those structures, the mirrors are how Rust reads the
//! same memory; as the functions move, the mirrors become the core's
//! types and the C definitions stay the ABI until the last C reader is
//! gone.
//!
//! The bitfield words of `struct vm_map_entry` and `struct vm_map`
//! cannot be expressed in Rust, so they are one `u32` each with named
//! accessors; C keeps setting the same bits through its own fields.
//! `projected_on` is a tagged pointer -- null, all-ones (the
//! non-persistent kernel-map entry), or an entry -- and gets a
//! `Projection` view.
//!
//! No function has moved yet; the module has no `extern "C"` adapters
//! in this step.

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::list::List;
use crate::kern::lock::{LockData, SimpleLock};
use crate::kern::rbtree::{Rbtree, RbtreeNode};
use crate::vm::types::{Pmap, VmInherit, VmObject, VmPage, VmProt};
use core::ffi::{c_char, c_int, c_uint};
use core::mem::{ManuallyDrop, offset_of, size_of};
use core::ptr::{self, NonNull};

/// `VM_MAP_COPY_PAGE_LIST_MAX`: pages a page-list copy carries inline.
pub const VM_MAP_COPY_PAGE_LIST_MAX: usize = 64;

/// `VM_MAP_COPY_ENTRY_LIST`: the copy holds an entry list.
pub const VM_MAP_COPY_ENTRY_LIST: c_int = 1;
/// `VM_MAP_COPY_OBJECT`: the copy holds one object.
pub const VM_MAP_COPY_OBJECT: c_int = 2;
/// `VM_MAP_COPY_PAGE_LIST`: the copy holds a page list.
pub const VM_MAP_COPY_PAGE_LIST: c_int = 3;

/// Pins a mirror to the C layout it replaces: size, alignment and the
/// offsets of the fields C reads by name.  Any drift becomes a build
/// error here and, through the `_Static_assert`s in `vm/vm_map.h`, on
/// the C side too.
macro_rules! assert_layout {
    ($t:ty, $size:expr, $align:expr, { $($f:ident: $off:expr),* $(,)? }) => {
        const _: () = assert!(size_of::<$t>() == $size);
        const _: () = assert!(align_of::<$t>() == $align);
        $(
            const _: () = assert!(offset_of!($t, $f) == $off);
        )*
    };
}

/// `struct vm_map_links`: the doubly-linked chain through the entries,
/// sorted by address.  The header's own links are the sentinel entry.
#[repr(C)]
pub struct VmMapLinks {
    /// The previous entry, or the header.
    pub prev: Option<NonNull<VmMapEntry>>,
    /// The next entry, or the header.
    pub next: Option<NonNull<VmMapEntry>>,
    /// The first address the entry covers.
    pub start: VmOffset,
    /// The first address after the entry.
    pub end: VmOffset,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapLinks, 32, 8, {
    prev: 0, next: 8, start: 16, end: 24,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapLinks, 16, 4, {
    prev: 0, next: 4, start: 8, end: 12,
});

/// Bits of the `vm_map_entry` flag word, in declaration order.  GCC
/// gives the first member bit 0.
pub const VME_IN_GAP_TREE: u32 = 1 << 0;
/// `is_shared`.
pub const VME_IS_SHARED: u32 = 1 << 1;
/// `is_sub_map`.
pub const VME_IS_SUB_MAP: u32 = 1 << 2;
/// `in_transition`.
pub const VME_IN_TRANSITION: u32 = 1 << 3;
/// `needs_wakeup`.
pub const VME_NEEDS_WAKEUP: u32 = 1 << 4;
/// `needs_copy`.
pub const VME_NEEDS_COPY: u32 = 1 << 5;

/// `struct vm_map_entry`: one mapping in a map.
///
/// The `links` field is the first member, so a pointer to it is also a
/// pointer to the entry; `vm_map_to_entry()` builds the header
/// sentinel from `hdr.links` that way.
#[repr(C)]
pub struct VmMapEntry {
    /// Chain links, `links` in C.
    pub links: VmMapLinks,
    /// Node in the address-ordered tree, `tree_node` in C.
    pub tree_node: RbtreeNode,
    /// Node in the gap-size tree, `gap_node` in C.
    pub gap_node: RbtreeNode,
    /// List of entries sharing this gap size, `gap_list` in C.
    pub gap_list: List,
    /// The gap after this entry, `gap_size` in C.
    pub gap_size: VmSize,
    /// The object or submap mapped, `object` in C.
    pub object: VmMapObject,
    /// Offset into the object, `offset` in C.
    pub offset: VmOffset,
    /// The packed boolean bits, `in_gap_tree` through `needs_copy`.
    pub flags: u32,
    /// Protection code, `protection` in C.
    pub protection: VmProt,
    /// Maximum protection, `max_protection` in C.
    pub max_protection: VmProt,
    /// Inheritance, `inheritance` in C.
    pub inheritance: VmInherit,
    /// Wire count, `wired_count` in C.
    pub wired_count: u16,
    /// Wiring access types, `wired_access` in C.
    pub wired_access: VmProt,
    /// The projected-buffer tag, `projected_on` in C: null for a
    /// normal entry, all ones for a non-persistent kernel-map entry,
    /// or the matching kernel-map entry.
    pub projected_on: *mut VmMapEntry,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapEntry, 152, 8, {
    links: 0, tree_node: 32, gap_node: 56, gap_list: 80,
    gap_size: 96, object: 104, offset: 112, flags: 120,
    protection: 124, max_protection: 128, inheritance: 132,
    wired_count: 136, wired_access: 140, projected_on: 144,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapEntry, 88, 4, {
    links: 0, tree_node: 16, gap_node: 28, gap_list: 40,
    gap_size: 48, object: 52, offset: 56, flags: 60,
    protection: 64, max_protection: 68, inheritance: 72,
    wired_count: 76, wired_access: 80, projected_on: 84,
});

/// The object-or-submap tag of an entry, `union vm_map_object`.
#[repr(C)]
pub union VmMapObject {
    /// The memory object mapped.
    pub vm_object: *mut VmObject,
    /// The subordinate map.
    pub sub_map: *mut VmMap,
}

// Both members are one pointer.
#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapObject, 8, 8, { vm_object: 0, sub_map: 0 });
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapObject, 4, 4, { vm_object: 0, sub_map: 0 });

/// Where an entry's wires came from, the three states of
/// `projected_on`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    /// The entry is normal.
    None,
    /// A non-persistent entry of the kernel map.
    NonPersistent,
    /// A projected region matched with this kernel-map entry.
    Entry(NonNull<VmMapEntry>),
}

impl VmMapEntry {
    /// The `vm_map_entry` a `tree_node` lives in; `rbtree_entry()` in
    /// C with the `tree_node` member.
    ///
    /// # Safety
    ///
    /// `node` must point at the `tree_node` member of a live
    /// `VmMapEntry`.
    pub unsafe fn from_tree_node(node: NonNull<RbtreeNode>) -> NonNull<Self> {
        // SAFETY: the caller promises the node is inside an entry.
        unsafe {
            NonNull::new_unchecked(
                node.as_ptr()
                    .cast::<u8>()
                    .sub(offset_of!(VmMapEntry, tree_node))
                    .cast::<VmMapEntry>(),
            )
        }
    }

    /// The `vm_map_entry` a `gap_node` lives in; `rbtree_entry()` in
    /// C with the `gap_node` member.
    ///
    /// # Safety
    ///
    /// `node` must point at the `gap_node` member of a live
    /// `VmMapEntry`.
    pub unsafe fn from_gap_node(node: NonNull<RbtreeNode>) -> NonNull<Self> {
        // SAFETY: the caller promises the node is inside an entry.
        unsafe {
            NonNull::new_unchecked(
                node.as_ptr()
                    .cast::<u8>()
                    .sub(offset_of!(VmMapEntry, gap_node))
                    .cast::<VmMapEntry>(),
            )
        }
    }

    /// Whether `is_shared` is set.
    pub fn is_shared(&self) -> bool {
        self.flags & VME_IS_SHARED != 0
    }

    /// Set or clear `is_shared`.
    pub fn set_shared(&mut self, shared: bool) {
        self.set_flag(VME_IS_SHARED, shared);
    }

    /// Whether `is_sub_map` is set.
    pub fn is_sub_map(&self) -> bool {
        self.flags & VME_IS_SUB_MAP != 0
    }

    /// Set or clear `is_sub_map`.
    pub fn set_sub_map(&mut self, sub_map: bool) {
        self.set_flag(VME_IS_SUB_MAP, sub_map);
    }

    /// Whether `in_gap_tree` is set.
    pub fn in_gap_tree(&self) -> bool {
        self.flags & VME_IN_GAP_TREE != 0
    }

    /// Set or clear `in_gap_tree`.
    pub fn set_in_gap_tree(&mut self, value: bool) {
        self.set_flag(VME_IN_GAP_TREE, value);
    }

    /// Whether `in_transition` is set.
    pub fn in_transition(&self) -> bool {
        self.flags & VME_IN_TRANSITION != 0
    }

    /// Set or clear `in_transition`.
    pub fn set_in_transition(&mut self, value: bool) {
        self.set_flag(VME_IN_TRANSITION, value);
    }

    /// Whether `needs_wakeup` is set.
    pub fn needs_wakeup(&self) -> bool {
        self.flags & VME_NEEDS_WAKEUP != 0
    }

    /// Set or clear `needs_wakeup`.
    pub fn set_needs_wakeup(&mut self, value: bool) {
        self.set_flag(VME_NEEDS_WAKEUP, value);
    }

    /// Whether `needs_copy` is set.
    pub fn needs_copy(&self) -> bool {
        self.flags & VME_NEEDS_COPY != 0
    }

    /// Set or clear `needs_copy`.
    pub fn set_needs_copy(&mut self, value: bool) {
        self.set_flag(VME_NEEDS_COPY, value);
    }

    /// Set or clear one flag bit.
    fn set_flag(&mut self, bit: u32, value: bool) {
        if value {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }

    /// The projection state of this entry.
    pub fn projection(&self) -> Projection {
        if self.projected_on.is_null() {
            Projection::None
        } else if self.projected_on.addr() == usize::MAX {
            Projection::NonPersistent
        } else {
            // SAFETY: not null, just checked.
            Projection::Entry(unsafe {
                NonNull::new_unchecked(self.projected_on)
            })
        }
    }

    /// Mark the entry non-persistent, `(vm_map_entry_t) -1` in C.
    pub fn set_projection_non_persistent(&mut self) {
        self.projected_on = ptr::without_provenance_mut(usize::MAX);
    }
}

/// `struct vm_map_header`: the header of a map and of an entry-list
/// copy.
#[repr(C)]
pub struct VmMapHeader {
    /// First, last and bounds of the entry chain.
    pub links: VmMapLinks,
    /// The address-ordered tree.
    pub tree: Rbtree,
    /// The gap-size tree.
    pub gap_tree: Rbtree,
    /// Number of entries.
    pub nentries: c_int,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapHeader, 56, 8, {
    links: 0, tree: 32, gap_tree: 40, nentries: 48,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapHeader, 28, 4, {
    links: 0, tree: 16, gap_tree: 20, nentries: 24,
});

impl VmMapHeader {
    /// The sentinel entry the header's links stand in for;
    /// `vm_map_to_entry()` and `vm_map_copy_to_entry()` in C.
    ///
    /// `links` is the first member of both `VmMapHeader` and
    /// `VmMapEntry`, so the header's address is the sentinel's.
    pub fn to_entry(&self) -> NonNull<VmMapEntry> {
        NonNull::from(&self.links).cast()
    }
}

/// Bits of the `vm_map` flag word, in declaration order.
pub const VM_MAP_WAIT_FOR_SPACE: u32 = 1 << 0;
/// `wiring_required`.
pub const VM_MAP_WIRING_REQUIRED: u32 = 1 << 1;

/// `struct vm_map`: one address space.
#[repr(C)]
pub struct VmMap {
    /// The sleep lock protecting the map data.
    pub lock: LockData,
    /// The entry header.
    pub hdr: VmMapHeader,
    /// The physical map.
    pub pmap: *mut Pmap,
    /// Current virtual size.
    pub size: VmSize,
    /// Wired size.
    pub size_wired: VmSize,
    /// Size of the `VM_PROT_NONE` regions.
    pub size_none: VmSize,
    /// Reference count; guarded by `ref_lock`.
    pub ref_count: c_int,
    /// The reference-count lock.
    pub ref_lock: SimpleLock,
    /// Last used entry; guarded by `hint_lock`.
    pub hint: *mut VmMapEntry,
    /// The hint lock.
    pub hint_lock: SimpleLock,
    /// First free-space hint.
    pub first_free: *mut VmMapEntry,
    /// `wait_for_space` and `wiring_required`.
    pub flags: u32,
    /// Version number, bumped by every write lock.
    pub timestamp: c_uint,
    /// The map's name.
    pub name: *const c_char,
    /// Current virtual-memory limit.
    pub size_cur_limit: VmSize,
    /// Largest limit an unprivileged task may set.
    pub size_max_limit: VmSize,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMap, 168, 8, {
    lock: 0, hdr: 16, pmap: 72, size: 80, size_wired: 88,
    size_none: 96, ref_count: 104, ref_lock: 108, hint: 112,
    hint_lock: 120, first_free: 128, flags: 136, timestamp: 140,
    name: 144, size_cur_limit: 152, size_max_limit: 160,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMap, 96, 4, {
    lock: 0, hdr: 12, pmap: 40, size: 44, size_wired: 48,
    size_none: 52, ref_count: 56, ref_lock: 60, hint: 64,
    hint_lock: 68, first_free: 72, flags: 76, timestamp: 80,
    name: 84, size_cur_limit: 88, size_max_limit: 92,
});

impl VmMap {
    /// The sentinel entry of this map's chain; `vm_map_to_entry()` in
    /// C.
    pub fn to_entry(&self) -> NonNull<VmMapEntry> {
        self.hdr.to_entry()
    }

    /// Whether callers wait for space instead of failing.
    pub fn wait_for_space(&self) -> bool {
        self.flags & VM_MAP_WAIT_FOR_SPACE != 0
    }

    /// Whether new mappings are wired.
    pub fn wiring_required(&self) -> bool {
        self.flags & VM_MAP_WIRING_REQUIRED != 0
    }
}

/// `struct vm_map_version`: a timestamp to validate a lookup.
#[repr(C)]
pub struct VmMapVersion {
    /// The map timestamp the lookup saw.
    pub main_timestamp: c_uint,
}

// Both C compilers lay a lone `unsigned int` out the same way.
const _: () = assert!(size_of::<VmMapVersion>() == 4);
const _: () = assert!(align_of::<VmMapVersion>() == 4);

/// `vm_map_copy_cont_fn`: a page-list copy's continuation.
pub type VmMapCopyContFn = unsafe extern "C" fn(
    args: *mut VmMapCopyinArgs,
    new_copy: *mut *mut VmMapCopy,
) -> c_int;

/// The `OBJECT` variant of a copy, `c_u.c_o`.
#[repr(C)]
pub struct VmMapCopyObject {
    /// The object the copy holds.
    pub object: *mut VmObject,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapCopyObject, 8, 8, { object: 0 });
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapCopyObject, 4, 4, { object: 0 });

/// The `PAGE_LIST` variant of a copy, `c_u.c_p`.
#[repr(C)]
pub struct VmMapCopyPageList {
    /// The pages, up to `VM_MAP_COPY_PAGE_LIST_MAX`.
    pub page_list: [*mut VmPage; VM_MAP_COPY_PAGE_LIST_MAX],
    /// How many of `page_list` are used.
    pub npages: c_int,
    /// The continuation that supplies more pages, if any.
    pub cont: Option<VmMapCopyContFn>,
    /// The continuation's argument.
    pub cont_args: *mut VmMapCopyinArgs,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapCopyPageList, 536, 8, {
    page_list: 0, npages: 512, cont: 520, cont_args: 528,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapCopyPageList, 268, 4, {
    page_list: 0, npages: 256, cont: 260, cont_args: 264,
});

/// The `c_u` union of a copy.
#[repr(C)]
pub union VmMapCopyU {
    /// The `ENTRY_LIST` variant.
    pub hdr: ManuallyDrop<VmMapHeader>,
    /// The `OBJECT` variant.
    pub c_o: ManuallyDrop<VmMapCopyObject>,
    /// The `PAGE_LIST` variant.
    pub c_p: ManuallyDrop<VmMapCopyPageList>,
}

// The union is as large as its largest member and aligned like the
// most demanding one, exactly as in C.
#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapCopyU, 536, 8, {});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapCopyU, 268, 4, {});

/// `struct vm_map_copy`: a region of memory in transit between maps.
#[repr(C)]
pub struct VmMapCopy {
    /// `VM_MAP_COPY_ENTRY_LIST`, `_OBJECT` or `_PAGE_LIST`.
    pub type_: c_int,
    /// Offset of the region within the object.
    pub offset: VmOffset,
    /// Size of the region.
    pub size: VmSize,
    /// The three shapes.
    pub c_u: VmMapCopyU,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapCopy, 560, 8, {
    type_: 0, offset: 8, size: 16, c_u: 24,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapCopy, 280, 4, {
    type_: 0, offset: 4, size: 8, c_u: 12,
});

/// `struct vm_map_copyin_args_data`: what a page-list continuation
/// remembers about the copyin.
#[repr(C)]
pub struct VmMapCopyinArgs {
    /// The map the copy came from.
    pub map: *mut VmMap,
    /// The source address.
    pub src_addr: VmOffset,
    /// The source length.
    pub src_len: VmSize,
    /// The address to destroy once copied.
    pub destroy_addr: VmOffset,
    /// The length to destroy.
    pub destroy_len: VmSize,
    /// Whether the pages were stolen.
    pub steal_pages: c_int,
}

#[cfg(target_pointer_width = "64")]
assert_layout!(VmMapCopyinArgs, 48, 8, {
    map: 0, src_addr: 8, src_len: 16, destroy_addr: 24,
    destroy_len: 32, steal_pages: 40,
});
#[cfg(target_pointer_width = "32")]
assert_layout!(VmMapCopyinArgs, 24, 4, {
    map: 0, src_addr: 4, src_len: 8, destroy_addr: 12,
    destroy_len: 16, steal_pages: 20,
});
