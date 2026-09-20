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
//! The `extern "C"` edge lives in `vm_map_ffi.rs`, one adapter per
//! exported symbol; the native core they call is here.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::{
    kernel_pmap, kmem_cache_alloc, kmem_cache_free, lock_done, lock_init,
    lock_read, lock_write, pmap_destroy, projected_buffer_collect,
    vm_map_cache, vm_map_delete, vm_map_glue_pmap_attribute,
    vm_map_glue_privilege_dec, vm_map_glue_privilege_inc, vm_page_mem_size,
};
use crate::kern::list::List;
use crate::kern::lock::{LockData, SimpleLock};
use crate::kern::rbtree::{RBTREE_LEFT, Rbtree, RbtreeNode};
use crate::vm::error::{Error, error_from_kern_return};
use crate::vm::types::{Pmap, VmInherit, VmObject, VmPage, VmProt};
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_uint};
use core::mem::{ManuallyDrop, offset_of, size_of};
use core::ptr::{self, NonNull, addr_of_mut, with_exposed_provenance_mut};

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
    /// Reference count; guarded by `ref_lock`, so Rust reaches it
    /// through `UnsafeCell` while holding that lock.
    pub ref_count: UnsafeCell<c_int>,
    /// The reference-count lock.
    pub ref_lock: SimpleLock,
    /// Last used entry; guarded by `hint_lock`.
    pub hint: UnsafeCell<*mut VmMapEntry>,
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

/// `PAGE_SHIFT` of <machine/vm_param.h>: 12 on both x86 kernels.
const PAGE_SHIFT: u32 = 12;
/// `PAGE_SIZE`: one page.
const PAGE_SIZE: VmSize = 1 << PAGE_SHIFT;
/// `PAGE_MASK`: the in-page offset bits.
const PAGE_MASK: VmSize = PAGE_SIZE - 1;

/// Round `x` up to a page boundary; `round_page()` in C.
const fn round_page(x: VmOffset) -> VmOffset {
    x.wrapping_add(PAGE_MASK) & !PAGE_MASK
}

/// Round `x` down to a page boundary; `trunc_page()` in C.
const fn trunc_page(x: VmOffset) -> VmOffset {
    x & !PAGE_MASK
}

impl VmMap {
    /// Copy the virtual-memory limits from `src` to `dst`, which
    /// `vm_map_copy_limits()` in C does.  The maps must be distinct.
    pub(crate) fn copy_limits(dst: NonNull<VmMap>, src: NonNull<VmMap>) {
        // SAFETY: the caller promises both maps are valid and distinct.
        unsafe {
            (*dst.as_ptr()).size_cur_limit = (*src.as_ptr()).size_cur_limit;
            (*dst.as_ptr()).size_max_limit = (*src.as_ptr()).size_max_limit;
        }
    }

    /// Write-lock the map: take the sleep lock, bump the current
    /// thread's VM privilege and the map timestamp.
    /// `vm_map_lock()` in C.
    pub(crate) fn lock(map: NonNull<VmMap>) {
        // SAFETY: the caller owns the map, and `lock_write` is the C
        // sleep lock the mirror names.
        unsafe { lock_write(addr_of_mut!((*map.as_ptr()).lock)) };
        // SAFETY: the shim reads the current thread, if any.
        unsafe { vm_map_glue_privilege_inc() };
        // SAFETY: the write lock is held, so this is the only writer;
        // the C `timestamp++` wraps as an unsigned int.
        unsafe {
            let timestamp = addr_of_mut!((*map.as_ptr()).timestamp);
            timestamp.write(timestamp.read().wrapping_add(1));
        }
    }

    /// Unlock a map locked by `lock()`.  `vm_map_unlock()` in C.
    pub(crate) fn unlock(map: NonNull<VmMap>) {
        // SAFETY: the shim reads the current thread, if any, and the
        // C code balances the privilege bump with this call.
        unsafe { vm_map_glue_privilege_dec() };
        // SAFETY: the caller holds the write lock on this map.
        unsafe { lock_done(addr_of_mut!((*map.as_ptr()).lock)) };
    }

    /// Take the read lock and check the version; on mismatch the lock
    /// is released before returning, on match the caller owns it and
    /// must call `vm_map_verify_done()`.  `vm_map_verify()` in C.
    pub(crate) fn verify(map: NonNull<VmMap>, version: &VmMapVersion) -> bool {
        // SAFETY: the caller owns the map.
        unsafe { lock_read(addr_of_mut!((*map.as_ptr()).lock)) };
        // SAFETY: reading the timestamp holds at least the read lock.
        let timestamp = unsafe { (*map.as_ptr()).timestamp };
        let result = timestamp == version.main_timestamp;
        if !result {
            // SAFETY: the read lock was taken above.
            unsafe { lock_done(addr_of_mut!((*map.as_ptr()).lock)) };
        }
        result
    }

    /// Find the entry containing `address`, or the entry before it,
    /// saving it as the hint.  `vm_map_lookup_entry()` in C.
    ///
    /// The caller must hold the map lock for read or write, so the
    /// tree and the entry chain are stable; `hint` itself is guarded
    /// here by `hint_lock`.
    pub(crate) fn lookup_entry(
        &self,
        address: VmOffset,
    ) -> (bool, NonNull<VmMapEntry>) {
        let sentinel = self.to_entry();

        self.hint_lock.lock();
        // SAFETY: `hint` is written only under `hint_lock`, held here.
        let hint = unsafe { *self.hint.get() };
        self.hint_lock.unlock();

        if hint != sentinel.as_ptr() {
            // SAFETY: the hint names a live entry of this map, and the
            // caller's map lock keeps the chain stable.
            let hint_entry = unsafe { &*hint };
            if address >= hint_entry.links.start {
                if address < hint_entry.links.end {
                    // SAFETY: the hint is not null, just checked.
                    return (true, unsafe { NonNull::new_unchecked(hint) });
                }
                let next = hint_entry.links.next;
                match next {
                    None => {
                        // SAFETY: the hint is not null, just checked.
                        return (false, unsafe {
                            NonNull::new_unchecked(hint)
                        });
                    }
                    Some(next) if next == sentinel => {
                        // SAFETY: the hint is not null, just checked.
                        return (false, unsafe {
                            NonNull::new_unchecked(hint)
                        });
                    }
                    Some(next) => {
                        // SAFETY: the chain is stable under the map
                        // lock the caller holds.
                        let next_start = unsafe { next.as_ref() }.links.start;
                        if address < next_start {
                            // SAFETY: the hint is not null, just
                            // checked.
                            return (false, unsafe {
                                NonNull::new_unchecked(hint)
                            });
                        }
                    }
                }
            }
        }

        let node = self.hdr.tree.lookup_nearest(
            |node| {
                // SAFETY: `node` came from this map's entry tree,
                // whose entries are stable under the caller's lock.
                let entry = unsafe { VmMapEntry::from_tree_node(node) };
                // SAFETY: as above.
                let entry = unsafe { entry.as_ref() };
                if address < entry.links.start {
                    -1
                } else if address < entry.links.end {
                    0
                } else {
                    1
                }
            },
            RBTREE_LEFT,
        );

        match node {
            None => {
                self.save_hint(sentinel);
                (false, sentinel)
            }
            Some(node) => {
                // SAFETY: `node` came from this map's entry tree.
                let entry = unsafe { VmMapEntry::from_tree_node(node) };
                self.save_hint(entry);
                // SAFETY: as above.
                let end = unsafe { entry.as_ref() }.links.end;
                (address < end, entry)
            }
        }
    }

    /// Store `entry` as the map's lookup hint, under `hint_lock`.
    /// The `SAVE_HINT()` macro in C.
    fn save_hint(&self, entry: NonNull<VmMapEntry>) {
        self.hint_lock.lock();
        // SAFETY: `hint` is written only under `hint_lock`, held here.
        unsafe { *self.hint.get() = entry.as_ptr() };
        self.hint_lock.unlock();
    }

    /// Add a reference to the map.  `vm_map_reference()` in C.
    pub(crate) fn reference(map: NonNull<VmMap>) {
        // SAFETY: the caller owns the map for the call.
        let this = unsafe { &*map.as_ptr() };

        this.ref_lock.lock();
        // SAFETY: `ref_count` is guarded by `ref_lock`, held here.
        unsafe {
            let count = &mut *this.ref_count.get();
            *count = count.wrapping_add(1);
        }
        this.ref_lock.unlock();
    }

    /// Drop a reference, destroying the map when the last one goes.
    /// `vm_map_deallocate()` in C.
    pub(crate) fn deallocate(map: NonNull<VmMap>) {
        // SAFETY: the caller owns the map for the call.
        let this = unsafe { &*map.as_ptr() };

        this.ref_lock.lock();
        // SAFETY: `ref_count` is guarded by `ref_lock`, held here.
        let count = unsafe {
            let count = &mut *this.ref_count.get();
            *count = count.wrapping_sub(1);
            *count
        };
        this.ref_lock.unlock();

        if count > 0 {
            return;
        }

        // Read what the destruction path needs before the C calls
        // below mutate the map through the raw pointer; `this` is not
        // used past this point.
        let min = this.hdr.links.start;
        let max = this.hdr.links.end;
        let pmap = this.pmap;

        // The C callers discard projected_buffer_collect()'s result.
        // SAFETY: the map is exclusively owned by this deallocation.
        unsafe { projected_buffer_collect(map.as_ptr().cast()) };
        // SAFETY: as above; `vm_map_delete` stays C until M3.
        unsafe { vm_map_delete(map.as_ptr().cast(), min, max) };
        // SAFETY: as above, and the pmap is no longer used.
        unsafe { pmap_destroy(pmap) };
        // SAFETY: the map came from `vm_map_cache`, and nothing
        // references it now.
        unsafe {
            kmem_cache_free(addr_of_mut!(vm_map_cache), map.as_ptr().addr())
        };
    }

    /// Initialize an empty map in caller storage.
    /// `vm_map_setup()` in C.
    pub(crate) fn setup(
        map: &mut VmMap,
        pmap: *mut Pmap,
        min: VmOffset,
        max: VmOffset,
    ) {
        let sentinel = map.to_entry();

        map.hdr.links.prev = Some(sentinel);
        map.hdr.links.next = Some(sentinel);
        map.hdr.nentries = 0;
        map.hdr.tree.init();
        map.hdr.gap_tree.init();

        map.size = 0;
        map.size_wired = 0;
        map.size_none = 0;
        // SAFETY: the map is freshly set up here, no lock needed yet.
        unsafe { *map.ref_count.get() = 1 };
        map.pmap = pmap;
        map.hdr.links.start = min;
        map.hdr.links.end = max;
        map.flags = 0;
        map.first_free = sentinel.as_ptr();
        // SAFETY: as above.
        unsafe { *map.hint.get() = sentinel.as_ptr() };
        map.name = ptr::null();

        // TODO add to default limit the swap size
        // SAFETY: `kernel_pmap` is the boot pmap, never null.
        if pmap != unsafe { kernel_pmap } {
            // SAFETY: reads a page-table statistic, no lock needed.
            let mem_size = unsafe { vm_page_mem_size() };
            map.size_cur_limit = mem_size;
            map.size_max_limit = mem_size;
        } else {
            map.size_cur_limit = !0;
            map.size_max_limit = !0;
        }

        // SAFETY: the sleep lock lives in the caller's map storage.
        unsafe { lock_init(addr_of_mut!(map.lock), 1) };
        map.timestamp = 0;
        map.ref_lock.init();
        map.hint_lock.init();
    }

    /// Allocate and set up an empty map.  `vm_map_create()` in C.
    pub(crate) fn create(
        pmap: *mut Pmap,
        min: VmOffset,
        max: VmOffset,
    ) -> Option<NonNull<VmMap>> {
        // SAFETY: `vm_map_cache` is initialized by `vm_map_init()`
        // before any map is created.
        let address = unsafe { kmem_cache_alloc(addr_of_mut!(vm_map_cache)) };
        // The allocator returns an address like the C code; `None`
        // when it is zero.
        let map = NonNull::new(with_exposed_provenance_mut::<VmMap>(address))?;

        // SAFETY: the object is freshly allocated and unshared.
        VmMap::setup(unsafe { &mut *map.as_ptr() }, pmap, min, max);
        Some(map)
    }

    /// The page-aligned `msync` operation.  `vm_map_msync()` in C,
    /// including its unfinished tail: a request with work in it still
    /// reports `KERN_INVALID_ARGUMENT`.
    pub(crate) fn msync(
        map: Option<NonNull<VmMap>>,
        address: VmOffset,
        size: VmSize,
        sync_flags: c_int,
    ) -> Result<(), Error> {
        const ASYNCHRONOUS: c_int = 0x01;
        const SYNCHRONOUS: c_int = 0x02;
        const BOTH: c_int = ASYNCHRONOUS | SYNCHRONOUS;

        if map.is_none() {
            return Err(Error::InvalidArgument);
        }

        if sync_flags & BOTH == BOTH {
            return Err(Error::InvalidArgument);
        }

        let size = round_page(address.wrapping_add(size))
            .wrapping_sub(trunc_page(address));

        if size == 0 {
            return Ok(());
        }

        // TODO: the C body has no msync implementation yet.
        Err(Error::InvalidArgument)
    }

    /// Apply a machine attribute to the map's pmap.
    /// `vm_map_machine_attribute()` in C.
    pub(crate) fn machine_attribute(
        map: NonNull<VmMap>,
        address: VmOffset,
        size: VmSize,
        attribute: c_uint,
        value: *mut c_int,
    ) -> Result<(), Error> {
        // SAFETY: the caller owns the map for the call.
        let (min, max) = unsafe {
            let links = addr_of_mut!((*map.as_ptr()).hdr.links);
            ((*links).start, (*links).end)
        };

        if address < min || address.wrapping_add(size) > max {
            return Err(Error::InvalidArgument);
        }

        VmMap::lock(map);
        // SAFETY: the map is write-locked, and the shim is the
        // machine's pmap_attribute.  On x86 it is the constant
        // KERN_INVALID_ADDRESS, so the `error_from_kern_return`
        // round-trip is exact; a real pmap returning an unnamed code
        // would be reported as KERN_FAILURE from here.
        let result = unsafe {
            vm_map_glue_pmap_attribute(
                (*map.as_ptr()).pmap,
                address,
                size,
                attribute,
                value,
            )
        };
        VmMap::unlock(map);

        error_from_kern_return(result)
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
