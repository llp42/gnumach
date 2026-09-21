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
//!
//! Map and entry storage comes from the C slab caches, so a freshly
//! allocated object is uninitialized bytes until the code here writes
//! it: fields and intrusive nodes are addressed through raw pointers
//! (`addr_of_mut!`) until the object is complete, and only then is it
//! treated as a reference.  This is transitional; when the caches and
//! the structures are Rust-owned, construction moves to `MaybeUninit`
//! or a `new()`.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::{
    Panic, assert_wait, kernel_map, kernel_object, kernel_pmap,
    kernel_virtual_end, kernel_virtual_start, kmem_cache_alloc,
    kmem_cache_free, lock_clear_recursive, lock_done, lock_init, lock_read,
    lock_read_to_write, lock_set_recursive, lock_write, lock_write_to_read,
    pmap_destroy, pmap_protect, pmap_remove, printf, projected_buffer_collect,
    thread_block, vm_fault_unwire, vm_fault_wire, vm_map_cache,
    vm_map_entry_cache, vm_map_glue_object_can_release,
    vm_map_glue_object_lock, vm_map_glue_object_unlock,
    vm_map_glue_pmap_attribute, vm_map_glue_privilege_dec,
    vm_map_glue_privilege_inc, vm_map_glue_thread_wakeup, vm_object_allocate,
    vm_object_coalesce, vm_object_deallocate, vm_object_page_remove,
    vm_object_pmap_remove, vm_object_reference, vm_object_shadow,
    vm_page_mem_size,
};
use crate::kern::list::{List, entry as list_entry};
use crate::kern::lock::{LockData, SimpleLock};
use crate::kern::rbtree::{RBTREE_LEFT, RBTREE_RIGHT, Rbtree, RbtreeNode};
use crate::vm::error::{Error, error_from_kern_return};
use crate::vm::types::{Pmap, VmInherit, VmObject, VmPage, VmProt};
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_uint, c_void};
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
        // SAFETY: as above; the map is unlocked, as the C contract of
        // vm_map_delete requires when the refcount is zero.
        let _ = unsafe { (*map.as_ptr()).delete(min, max) };
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

/// The object, offset, protection and wiring a successful
/// `vm_map_lookup()` reports, with the timestamp that validates it.
pub(crate) struct VmMapLookup {
    /// The object `vaddr` faults into, returned locked.
    pub object: *mut VmObject,
    /// Offset into `object`.
    pub offset: VmOffset,
    /// The effective protection.
    pub protection: VmProt,
    /// Whether the entry is wired.
    pub wired: bool,
    /// The map's timestamp at the time of the lookup.
    pub timestamp: c_uint,
}

/// One locked pass of `vm_map_lookup()`: either it is done, it failed,
/// it found a submap to descend into, or the read-to-write upgrade was
/// lost and the caller must retry with no lock held.
enum LookupAttempt {
    /// The lookup succeeded with the map read-locked.
    Found(VmMapLookup),
    /// The lookup failed with the map read-locked.
    Failed(Error),
    /// The entry is a submap; the map is read-locked.
    SubMap(NonNull<VmMap>),
    /// The upgrade failed and released the read lock.
    Retry,
}

impl VmMap {
    /// Upgrade the map's read lock to a write lock, bumping the
    /// timestamp as the `vm_map_lock_read_to_write()` macro does when
    /// the upgrade succeeds.  Returns `true` if the lock must be
    /// retried, in which case no lock is held.
    fn lock_read_to_write(map: NonNull<VmMap>) -> bool {
        // SAFETY: the caller holds the read lock; `lock_read_to_write`
        // releases it whether or not the upgrade succeeds.
        let failed = unsafe {
            lock_read_to_write(addr_of_mut!((*map.as_ptr()).lock)) != 0
        };
        if !failed {
            // SAFETY: the write lock is held, so the timestamp is
            // exclusively ours; the macro's `map->timestamp++` wraps
            // as an unsigned int.
            unsafe {
                let timestamp = addr_of_mut!((*map.as_ptr()).timestamp);
                timestamp.write(timestamp.read().wrapping_add(1));
            }
        }
        failed
    }

    /// One locked pass of `vm_map_lookup()`, without the submap loop.
    /// On `Found` and `Failed` the map's read lock is still held; on
    /// `Retry` it is not.
    fn lookup_locked(
        map: NonNull<VmMap>,
        vaddr: VmOffset,
        fault_type_in: VmProt,
    ) -> LookupAttempt {
        // SAFETY: the caller holds the map lock, so the entries are
        // stable and no reference outlives it.
        let this = unsafe { &*map.as_ptr() };

        let (found, entry) = this.lookup_entry(vaddr);
        if !found {
            return LookupAttempt::Failed(Error::InvalidAddress);
        }

        // SAFETY: `entry` is a live entry of a locked map.
        if unsafe { (*entry.as_ptr()).is_sub_map() } {
            // SAFETY: a submap entry's union member is a live map.
            let submap = unsafe {
                NonNull::new_unchecked((*entry.as_ptr()).object.sub_map)
            };
            return LookupAttempt::SubMap(submap);
        }

        // SAFETY: the entry is live under the map lock.
        let mut prot = unsafe { (*entry.as_ptr()).protection };

        if !prot.contains(fault_type_in) {
            return LookupAttempt::Failed(
                if prot.contains(VmProt::NOTIFY)
                    && fault_type_in.contains(VmProt::WRITE)
                {
                    Error::WriteProtectionFailure
                } else {
                    Error::ProtectionFailure
                },
            );
        }

        // SAFETY: as above.
        let wired = unsafe { (*entry.as_ptr()).wired_count } != 0;
        let mut fault_type = fault_type_in;
        if wired {
            // The C makes the fault type the entry protection, so a
            // wired entry faults for everything it allows.
            prot = unsafe { (*entry.as_ptr()).protection };
            fault_type = prot;
        }

        // SAFETY: as above.
        if unsafe { (*entry.as_ptr()).needs_copy() } {
            if fault_type.contains(VmProt::WRITE) {
                if VmMap::lock_read_to_write(map) {
                    return LookupAttempt::Retry;
                }
                // The C bumps the timestamp once more after the macro;
                // keep the extra bump it performs here.
                // SAFETY: the write lock is held; the integer wraps.
                unsafe {
                    let timestamp = addr_of_mut!((*map.as_ptr()).timestamp);
                    timestamp.write(timestamp.read().wrapping_add(1));
                }

                // SAFETY: the entry is live under the write lock and
                // `vm_object_shadow` keeps the reference it replaces.
                unsafe {
                    vm_object_shadow(
                        addr_of_mut!((*entry.as_ptr()).object.vm_object),
                        addr_of_mut!((*entry.as_ptr()).offset),
                        (*entry.as_ptr())
                            .links
                            .end
                            .wrapping_sub((*entry.as_ptr()).links.start),
                    );
                    (*entry.as_ptr()).set_needs_copy(false);
                }

                // SAFETY: the write lock taken by the upgrade.
                unsafe {
                    lock_write_to_read(addr_of_mut!((*map.as_ptr()).lock))
                };
            } else {
                // A read of a copy-on-write page must not be allowed
                // to write.
                prot &= VmProt::from_bits(!VmProt::WRITE.bits());
            }
        }

        // SAFETY: the entry is live under the map lock.
        if unsafe { (*entry.as_ptr()).object.vm_object }.is_null() {
            if VmMap::lock_read_to_write(map) {
                return LookupAttempt::Retry;
            }

            // SAFETY: the write lock is held; the entry is live.  The
            // map lock keeps the new object's reference.
            unsafe {
                (*entry.as_ptr()).object.vm_object = vm_object_allocate(
                    (*entry.as_ptr())
                        .links
                        .end
                        .wrapping_sub((*entry.as_ptr()).links.start),
                );
                (*entry.as_ptr()).offset = 0;
            }

            // SAFETY: the write lock taken by the upgrade.
            unsafe { lock_write_to_read(addr_of_mut!((*map.as_ptr()).lock)) };
        }

        // SAFETY: the entry is live under the lock.
        let (object, offset, timestamp) = unsafe {
            let e = &*entry.as_ptr();
            (
                e.object.vm_object,
                vaddr.wrapping_sub(e.links.start).wrapping_add(e.offset),
                (*map.as_ptr()).timestamp,
            )
        };

        // The object cannot be null: the branch above allocates one.
        // SAFETY: the map lock keeps the entry's reference alive, and
        // the caller receives the object locked, as in C.
        unsafe { vm_map_glue_object_lock(object) };

        LookupAttempt::Found(VmMapLookup {
            object,
            offset,
            protection: prot,
            wired,
            timestamp,
        })
    }

    /// Find the object, offset and protection for `vaddr`, following
    /// submaps.  `vm_map_lookup()` in C.
    ///
    /// On success the map in `var_map` is left read-locked when
    /// `keep_map_locked` is set and unlocked otherwise; on failure it
    /// is unlocked.  The returned object is locked.
    pub(crate) fn lookup(
        var_map: &mut NonNull<VmMap>,
        vaddr: VmOffset,
        fault_type: VmProt,
        keep_map_locked: bool,
    ) -> Result<VmMapLookup, Error> {
        loop {
            let map = *var_map;
            // SAFETY: the caller owns the map for the call; the read
            // lock keeps the entries stable.
            unsafe { lock_read(addr_of_mut!((*map.as_ptr()).lock)) };

            match VmMap::lookup_locked(map, vaddr, fault_type) {
                LookupAttempt::Found(result) => {
                    if !keep_map_locked {
                        // SAFETY: the read lock taken above.
                        unsafe {
                            lock_done(addr_of_mut!((*map.as_ptr()).lock))
                        };
                    }
                    return Ok(result);
                }
                LookupAttempt::Failed(error) => {
                    // SAFETY: the read lock taken above.
                    unsafe { lock_done(addr_of_mut!((*map.as_ptr()).lock)) };
                    return Err(error);
                }
                LookupAttempt::SubMap(submap) => {
                    // SAFETY: the read lock taken above.
                    unsafe { lock_done(addr_of_mut!((*map.as_ptr()).lock)) };
                    *var_map = submap;
                }
                // The failed upgrade released the read lock, so the
                // next iteration takes it afresh.
                LookupAttempt::Retry => {}
            }
        }
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

/// The address-ordered entry comparison.  `vm_map_entry_cmp_lookup()`
/// in C: negative before the entry, zero inside it, positive after.
fn entry_cmp_lookup(address: VmOffset, node: NonNull<RbtreeNode>) -> c_int {
    // SAFETY: the node is a `tree_node` of an entry of the tree the
    // caller keeps stable.
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
}

/// The insert comparison: the inserted entry's start against a visited
/// node.  `vm_map_entry_cmp_insert()` in C.
fn entry_cmp_insert(
    entry: NonNull<VmMapEntry>,
    node: NonNull<RbtreeNode>,
) -> c_int {
    // SAFETY: the caller names a live entry of the map.
    let start = unsafe { (*entry.as_ptr()).links.start };
    entry_cmp_lookup(start, node)
}

/// The gap-size comparison.  `vm_map_entry_gap_cmp_lookup()` in C.
fn gap_cmp_lookup(gap_size: VmSize, node: NonNull<RbtreeNode>) -> c_int {
    // SAFETY: the node is a `gap_node` of an entry of the gap tree.
    let entry = unsafe { VmMapEntry::from_gap_node(node) };
    // SAFETY: as above.
    let entry = unsafe { entry.as_ref() };
    if gap_size < entry.gap_size {
        -1
    } else if gap_size == entry.gap_size {
        0
    } else {
        1
    }
}

/// The gap insert comparison.  `vm_map_entry_gap_cmp_insert()` in C.
fn gap_cmp_insert(
    entry: NonNull<VmMapEntry>,
    node: NonNull<RbtreeNode>,
) -> c_int {
    // SAFETY: the caller names a live entry of the map.
    let gap_size = unsafe { (*entry.as_ptr()).gap_size };
    gap_cmp_lookup(gap_size, node)
}

impl VmMapEntry {
    /// Allocate an entry from the cache.  `_vm_map_entry_create()` in
    /// C, including its halt when the cache is exhausted.
    ///
    /// # Safety
    ///
    /// `vm_map_init()` must have initialized the entry cache.
    #[must_use]
    pub(crate) unsafe fn create() -> NonNull<VmMapEntry> {
        // SAFETY: `vm_map_entry_cache` is initialized by
        // `vm_map_init()` before any entry is created.
        let address =
            unsafe { kmem_cache_alloc(addr_of_mut!(vm_map_entry_cache)) };
        match NonNull::new(with_exposed_provenance_mut::<VmMapEntry>(address))
        {
            Some(entry) => entry,
            // SAFETY: the C code panics on allocation failure, which
            // halts the kernel.  The line number always fits `c_int`.
            None => unsafe {
                Panic(
                    c"rust/src/vm/vm_map.rs".as_ptr(),
                    line!() as c_int,
                    c"VmMapEntry::create".as_ptr(),
                    c"vm_map_entry_create".as_ptr(),
                )
            },
        }
    }
}

impl VmMapHeader {
    /// Whether `entry` is this header's sentinel.
    pub(crate) fn is_sentinel(&self, entry: NonNull<VmMapEntry>) -> bool {
        entry == self.to_entry()
    }

    /// `vm_map_gap_valid()` in C: the sentinel is no gap entry.
    fn gap_valid(&self, entry: NonNull<VmMapEntry>) -> bool {
        !self.is_sentinel(entry)
    }

    /// Recompute `entry`'s gap to the next entry or the header end.
    /// `vm_map_gap_compute()` in C.
    fn gap_compute(&self, entry: NonNull<VmMapEntry>) {
        // SAFETY: the caller names a live entry of this header.
        let next = unsafe { (*entry.as_ptr()).links.next };
        // SAFETY: as above.
        let end = unsafe { (*entry.as_ptr()).links.end };

        let gap_end = match next {
            // SAFETY: the chain links live entries or the sentinel.
            Some(next) if self.gap_valid(next) => unsafe {
                (*next.as_ptr()).links.start
            },
            _ => self.links.end,
        };

        // SAFETY: as above; the invariant puts the next entry (or the
        // header end) at or after this entry's end.
        unsafe {
            (*entry.as_ptr()).gap_size = gap_end.wrapping_sub(end);
        }
    }

    /// Insert `entry` into the gap tree, or into the list of an entry
    /// with the same gap.  `vm_map_gap_insert_single()` in C.
    fn gap_insert_single(&mut self, entry: NonNull<VmMapEntry>) {
        if !self.gap_valid(entry) {
            return;
        }
        self.gap_compute(entry);
        // SAFETY: `entry` is a live entry of this header.
        let gap_size = unsafe { (*entry.as_ptr()).gap_size };
        if gap_size == 0 {
            return;
        }

        let (found, slot) = self
            .gap_tree
            .lookup_slot(|node| gap_cmp_lookup(gap_size, node));

        match found {
            None => {
                // SAFETY: the gap node is unlinked and the slot names
                // a point of this gap tree; the nodes are addressed
                // raw, because the allocation is still uninitialized.
                unsafe {
                    let node = NonNull::new_unchecked(addr_of_mut!(
                        (*entry.as_ptr()).gap_node
                    ));
                    self.gap_tree.insert_at(slot, node);
                    List::init_head_at(NonNull::new_unchecked(addr_of_mut!(
                        (*entry.as_ptr()).gap_list
                    )));
                    (*entry.as_ptr()).set_in_gap_tree(true);
                }
            }
            Some(node) => {
                // SAFETY: the node came from this gap tree, so it is
                // an entry's gap node.
                let same = unsafe { VmMapEntry::from_gap_node(node) };
                // SAFETY: both entries are live and distinct under the
                // map lock, and their list fields do not alias.
                unsafe {
                    let list = &mut (*same.as_ptr()).gap_list;
                    let node = NonNull::new_unchecked(addr_of_mut!(
                        (*entry.as_ptr()).gap_list
                    ));
                    list.insert_tail(node);
                    (*entry.as_ptr()).set_in_gap_tree(false);
                }
            }
        }
    }

    /// Remove `entry` from the gap tree, promoting a same-gap entry
    /// when it was the tree's representative.
    /// `vm_map_gap_remove_single()` in C.
    fn gap_remove_single(&mut self, entry: NonNull<VmMapEntry>) {
        if !self.gap_valid(entry) {
            return;
        }
        // SAFETY: `entry` is a live entry of this header.
        if unsafe { (*entry.as_ptr()).gap_size } == 0 {
            return;
        }
        // SAFETY: as above.
        if !unsafe { (*entry.as_ptr()).in_gap_tree() } {
            // SAFETY: the entry's list node is linked in a gap list.
            unsafe {
                List::remove(NonNull::from(&mut (*entry.as_ptr()).gap_list));
            }
            return;
        }

        // SAFETY: the entry's gap node is linked in this gap tree.
        unsafe {
            let node = NonNull::from(&mut (*entry.as_ptr()).gap_node);
            self.gap_tree.remove_node(node);
        }

        // SAFETY: as above; a list head is self-linked when empty.
        if unsafe { (*entry.as_ptr()).gap_list.is_empty() } {
            return;
        }
        // SAFETY: as above.
        let first = unsafe { (*entry.as_ptr()).gap_list.first() };
        let Some(first) = first else {
            return;
        };
        // SAFETY: `first` is the gap_list node of a live entry.
        let same = unsafe {
            list_entry::<VmMapEntry>(first, offset_of!(VmMapEntry, gap_list))
        };
        // SAFETY: `same` and `entry` are live and distinct entries of
        // this header; `same`'s list node is the one just read.
        unsafe {
            let list = NonNull::from(&mut (*same.as_ptr()).gap_list);
            List::remove(list);
            List::set_head(
                list,
                NonNull::from(&mut (*entry.as_ptr()).gap_list),
            );
            let node = NonNull::from(&mut (*same.as_ptr()).gap_node);
            self.gap_tree
                .insert_by(node, |visited| gap_cmp_insert(same, visited));
            (*same.as_ptr()).set_in_gap_tree(true);
        }
    }

    /// Remove and reinsert `entry`, after its gap changed.
    /// `vm_map_gap_update()` in C.
    fn gap_update(&mut self, entry: NonNull<VmMapEntry>) {
        self.gap_remove_single(entry);
        self.gap_insert_single(entry);
    }

    /// Insert `entry`, adjusting its predecessor's gap too.
    /// `vm_map_gap_insert()` in C.
    fn gap_insert(&mut self, entry: NonNull<VmMapEntry>) {
        // SAFETY: `entry` is a live entry, so its predecessor is.
        let prev = unsafe { (*entry.as_ptr()).links.prev };
        if let Some(prev) = prev {
            self.gap_remove_single(prev);
            self.gap_insert_single(prev);
        }
        self.gap_insert_single(entry);
    }

    /// Link `entry` after `after` in the chain and the entry tree.
    /// The `_vm_map_entry_link()` macro in C.
    ///
    /// # Safety
    ///
    /// Both entries must be valid and live in this header, `entry`
    /// unlinked, and nothing else may touch the header during the call.
    pub(crate) unsafe fn entry_link(
        &mut self,
        after: NonNull<VmMapEntry>,
        entry: NonNull<VmMapEntry>,
        link_gap: bool,
    ) {
        self.nentries = self.nentries.wrapping_add(1);
        // SAFETY: the caller promises both entries are valid and the
        // map lock keeps the chain stable.
        unsafe {
            let next = (*after.as_ptr()).links.next;
            (*entry.as_ptr()).links.prev = Some(after);
            (*entry.as_ptr()).links.next = next;
            (*after.as_ptr()).links.next = Some(entry);
            if let Some(next) = next {
                (*next.as_ptr()).links.prev = Some(entry);
            }

            let node = NonNull::new_unchecked(addr_of_mut!(
                (*entry.as_ptr()).tree_node
            ));
            self.tree
                .insert_by(node, |visited| entry_cmp_insert(entry, visited));

            if link_gap {
                self.gap_insert(entry);
            }
        }
    }
}

impl VmMap {
    /// The first free-space hint, or the sentinel.
    fn first_free_entry(&self) -> NonNull<VmMapEntry> {
        NonNull::new(self.first_free).unwrap_or_else(|| self.to_entry())
    }

    /// The C allocation-failure diagnostic.
    fn no_room(&self) {
        // SAFETY: `printf` only formats.
        unsafe {
            printf(
                c"no more room in %p (%s)\n".as_ptr(),
                ptr::from_ref(self).cast_mut().cast::<c_void>(),
                self.name,
            )
        };
    }

    /// Enforce the map's VM limit for a new region.
    /// `vm_map_enforce_limit()` in C.
    fn enforce_limit(&self, size: VmSize) -> Result<(), Error> {
        // The limit is ignored for the kernel map.
        // SAFETY: `kernel_pmap` is a boot global.
        if self.pmap == unsafe { kernel_pmap } {
            return Ok(());
        }

        // Avoid taking into account the total VM_PROT_NONE virtual
        // memory.
        let allocated = self.size.wrapping_sub(self.size_none);
        let new_size = allocated.wrapping_add(size);
        // Check for integer overflow.
        if new_size < size {
            return Err(Error::InvalidArgument);
        }
        if new_size > self.size_cur_limit {
            return Err(Error::NoSpace);
        }
        Ok(())
    }

    /// Find a range of available space.  `vm_map_find_entry_anywhere()`
    /// in C: on success, the returned entry is the one preceding the
    /// range and the returned address is its first one.
    fn find_entry_anywhere(
        &mut self,
        size: VmSize,
        mask_in: VmOffset,
        map_locked: bool,
    ) -> Option<(NonNull<VmMapEntry>, VmOffset)> {
        let mut mask = mask_in;
        let mut max = self.hdr.links.end;

        if mask.wrapping_add(1) & mask != 0 {
            // We have high bits in addition to the low bits.  The C
            // uses the `int`-typed `__builtin_ffs`, which would be
            // undefined for a mask with no set bit in its low 32 bits;
            // scanning the full `usize` gives the intended answer.
            let first0 = (!mask).trailing_zeros() + 1;
            let lowmask = (1usize << (first0 - 1)).wrapping_sub(1);
            let himask = mask.wrapping_sub(lowmask);
            let second1 = himask.trailing_zeros() + 1;

            max = 1usize << (second1 - 1);

            if himask.wrapping_add(max) != 0 {
                // High bits do not continue up to the end.
                // SAFETY: `printf` only formats.
                unsafe { printf(c"invalid mask %zx\n".as_ptr(), mask) };
                return None;
            }

            mask = lowmask;
        }

        if !map_locked {
            VmMap::lock(NonNull::from(&mut *self));
        }

        loop {
            if self.hdr.nentries == 0 {
                let entry = self.to_entry();
                let start = self.hdr.links.start.wrapping_add(mask) & !mask;
                let end = start.wrapping_add(size);

                if start < self.hdr.links.start || end <= start || end > max {
                    self.no_room();
                    return None;
                }

                return Some((entry, start));
            }

            let entry = self.first_free_entry();
            if !self.hdr.is_sentinel(entry) {
                // SAFETY: `entry` is a live entry of this map.
                let (entry_end, gap_size) = unsafe {
                    ((*entry.as_ptr()).links.end, (*entry.as_ptr()).gap_size)
                };
                let start = entry_end.wrapping_add(mask) & !mask;
                let end = start.wrapping_add(size);

                if start >= entry_end
                    && end > start
                    && end <= max
                    && end <= entry_end.wrapping_add(gap_size)
                {
                    return Some((entry, start));
                }
            }

            let max_size = size.wrapping_add(mask);
            if max_size < size {
                // SAFETY: `printf` only formats.
                unsafe {
                    printf(
                        c"max_size %zd got smaller than size %zd with mask %zd\n"
                            .as_ptr(),
                        max_size,
                        size,
                        mask,
                    )
                };
                self.no_room();
                return None;
            }

            let node = self.hdr.gap_tree.lookup_nearest(
                |node| gap_cmp_lookup(max_size, node),
                RBTREE_RIGHT,
            );

            let Some(node) = node else {
                if map_locked || !self.wait_for_space() {
                    self.no_room();
                    return None;
                }

                // SAFETY: the map belongs to this thread for the call;
                // sleep on its address as the C code does and retry
                // after waking.  Only reachable when a caller passes
                // `map_locked = false`; the port must re-derive any
                // Rust view of the map here once `vm_map_enter` moves.
                unsafe {
                    assert_wait(ptr::from_mut(self).cast(), 1);
                    VmMap::unlock(NonNull::from(&mut *self));
                    thread_block(None);
                    VmMap::lock(NonNull::from(&mut *self));
                }
                continue;
            };

            // SAFETY: the node came from this gap tree.
            let mut entry = unsafe { VmMapEntry::from_gap_node(node) };
            // SAFETY: `entry` is a live entry.  The C code takes the
            // *last* node of the same-gap list, so the allocation
            // address matches its choice.
            let last = unsafe { (*entry.as_ptr()).gap_list.last() };
            if let Some(last) = last {
                // SAFETY: the list node belongs to a live entry.
                entry = unsafe {
                    list_entry::<VmMapEntry>(
                        last,
                        offset_of!(VmMapEntry, gap_list),
                    )
                };
            }

            // SAFETY: `entry` is a live entry.
            let entry_end = unsafe { (*entry.as_ptr()).links.end };
            let start = entry_end.wrapping_add(mask) & !mask;
            let end = start.wrapping_add(size);
            if end > max {
                // Does not respect the allowed maximum.
                // SAFETY: `printf` only formats.
                unsafe {
                    printf(c"%lx does not respect %lx\n".as_ptr(), end, max)
                };
                return None;
            }

            return Some((entry, start));
        }
    }

    /// Allocate a range and initialize an entry for it.
    /// `vm_map_find_entry()` in C, with the caller holding the map
    /// lock and the out-parameters returned as a pair.
    pub(crate) fn find_entry(
        map: &mut VmMap,
        size: VmSize,
        mask: VmOffset,
        object: *mut VmObject,
        protection: VmProt,
        max_protection: VmProt,
    ) -> Result<(VmOffset, NonNull<VmMapEntry>), Error> {
        if max_protection != VmProt::NONE {
            map.enforce_limit(size)?;
        }

        let (entry, start) = map
            .find_entry_anywhere(size, mask, true)
            .ok_or(Error::NoSpace)?;
        let end = start.wrapping_add(size);

        // See whether the preceding entry can be extended instead of
        // creating a new one.  [So far, we only attempt to extend from
        // below.]
        // SAFETY: `entry` is a live entry and the map is locked.
        let extend = !object.is_null()
            && !map.hdr.is_sentinel(entry)
            && unsafe {
                let before = &*entry.as_ptr();
                before.links.end == start
                    && !before.is_shared()
                    && !before.is_sub_map()
                    && !before.in_transition()
                    && before.object.vm_object == object
                    && !before.needs_copy()
                    && before.inheritance == VmInherit::COPY
                    && before.protection == protection
                    && before.max_protection == max_protection
                    && before.wired_count != 0
                    && before.projected_on.is_null()
            };

        let new_entry = if extend {
            // SAFETY: `entry` is live, and the map lock is held.
            unsafe { (*entry.as_ptr()).links.end = end };
            map.hdr.gap_update(entry);
            entry
        } else {
            // SAFETY: the entry cache is initialized and the map lock
            // is held.
            let new_entry = unsafe { VmMapEntry::create() };
            // SAFETY: `new_entry` is unlinked, freshly allocated
            // storage; the link below makes it reachable.  The fields
            // are written through the pointer, because no part of the
            // entry is initialized yet.
            unsafe {
                let e = new_entry.as_ptr();
                (*e).links.start = start;
                (*e).links.end = end;
                (*e).set_shared(false);
                (*e).set_sub_map(false);
                (*e).object.vm_object = ptr::null_mut();
                (*e).offset = 0;
                (*e).set_needs_copy(false);
                (*e).inheritance = VmInherit::COPY;
                (*e).protection = protection;
                (*e).max_protection = max_protection;
                (*e).wired_count = 1;
                (*e).wired_access = VmProt::READ | VmProt::WRITE;
                (*e).set_in_transition(false);
                (*e).set_needs_wakeup(false);
                (*e).projected_on = ptr::null_mut();

                map.hdr.entry_link(entry, new_entry, true);
            }
            new_entry
        };

        map.size = map.size.wrapping_add(size);
        if max_protection == VmProt::NONE {
            map.size_none = map.size_none.wrapping_add(size);
        }
        map.first_free = new_entry.as_ptr();
        map.save_hint(new_entry);

        Ok((start, new_entry))
    }
}

impl VmMapEntry {
    /// Return an entry to the cache.  `_vm_map_entry_dispose()` in C.
    ///
    /// # Safety
    ///
    /// `entry` must be unlinked and came from `create()`.
    pub(crate) unsafe fn dispose(entry: NonNull<VmMapEntry>) {
        // SAFETY: the caller promises the entry is free.
        unsafe {
            kmem_cache_free(
                addr_of_mut!(vm_map_entry_cache),
                entry.as_ptr().addr(),
            )
        };
    }
}

impl VmMapHeader {
    /// Split `entry` at `start`, leaving the front part in a new
    /// entry.  `_vm_map_clip_start()` in C.
    ///
    /// # Safety
    ///
    /// `entry` must be a live entry of this header, and `start` must
    /// lie inside it.  The header's lock discipline is the caller's.
    pub(crate) unsafe fn clip_start(
        &mut self,
        entry: NonNull<VmMapEntry>,
        start: VmOffset,
        link_gap: bool,
    ) {
        // SAFETY: the caller promises a live entry.
        unsafe {
            let new_entry = VmMapEntry::create();
            // `vm_map_entry_copy_full()`: the whole entry, links and
            // all; the link below overwrites the chain fields.
            ptr::copy_nonoverlapping(entry.as_ptr(), new_entry.as_ptr(), 1);

            (*new_entry.as_ptr()).links.end = start;
            let entry_start = (*entry.as_ptr()).links.start;
            (*entry.as_ptr()).offset = (*entry.as_ptr())
                .offset
                .wrapping_add(start.wrapping_sub(entry_start));
            (*entry.as_ptr()).links.start = start;

            let prev = (*entry.as_ptr()).links.prev;
            if let Some(prev) = prev {
                self.entry_link(prev, new_entry, link_gap);
            }

            if (*new_entry.as_ptr()).is_sub_map() {
                let submap = NonNull::new_unchecked(
                    (*new_entry.as_ptr()).object.sub_map,
                );
                VmMap::reference(submap);
            } else {
                vm_object_reference((*new_entry.as_ptr()).object.vm_object);
            }
        }
    }

    /// The guarded `vm_map_clip_start()` macro: split `entry` only
    /// when `start` lies strictly inside it, leaving a start on the
    /// entry boundary alone.
    ///
    /// # Safety
    ///
    /// `entry` must be a live entry of this header, and `start` at or
    /// after its start.  The header's lock discipline is the caller's.
    pub(crate) unsafe fn clip_start_at(
        &mut self,
        entry: NonNull<VmMapEntry>,
        start: VmOffset,
        link_gap: bool,
    ) {
        // SAFETY: the caller promises the entry is live.
        if start > unsafe { (*entry.as_ptr()).links.start } {
            // SAFETY: as above.
            unsafe { self.clip_start(entry, start, link_gap) };
        }
    }

    /// Split `entry` at `end`, leaving the back part in a new entry.
    /// `_vm_map_clip_end()` in C.
    ///
    /// # Safety
    ///
    /// `entry` must be a live entry of this header, and `end` must lie
    /// inside it.  The header's lock discipline is the caller's.
    pub(crate) unsafe fn clip_end(
        &mut self,
        entry: NonNull<VmMapEntry>,
        end: VmOffset,
        link_gap: bool,
    ) {
        // SAFETY: the caller promises a live entry.
        unsafe {
            let new_entry = VmMapEntry::create();
            // `vm_map_entry_copy_full()`.
            ptr::copy_nonoverlapping(entry.as_ptr(), new_entry.as_ptr(), 1);

            (*new_entry.as_ptr()).links.start = end;
            (*entry.as_ptr()).links.end = end;
            let entry_start = (*entry.as_ptr()).links.start;
            (*new_entry.as_ptr()).offset = (*new_entry.as_ptr())
                .offset
                .wrapping_add(end.wrapping_sub(entry_start));

            self.entry_link(entry, new_entry, link_gap);

            if (*entry.as_ptr()).is_sub_map() {
                let submap = NonNull::new_unchecked(
                    (*new_entry.as_ptr()).object.sub_map,
                );
                VmMap::reference(submap);
            } else {
                vm_object_reference((*new_entry.as_ptr()).object.vm_object);
            }
        }
    }

    /// Unlink `entry` from the chain and the entry tree.
    /// The `_vm_map_entry_unlink()` macro in C.
    ///
    /// # Safety
    ///
    /// `entry` must be linked in this header, and nothing else may
    /// touch the header during the call.
    pub(crate) unsafe fn entry_unlink(
        &mut self,
        entry: NonNull<VmMapEntry>,
        unlink_gap: bool,
    ) {
        self.nentries = self.nentries.wrapping_sub(1);
        // SAFETY: the caller promises the entry is linked.
        unsafe {
            let prev = (*entry.as_ptr()).links.prev;
            let next = (*entry.as_ptr()).links.next;
            if let Some(next) = next {
                (*next.as_ptr()).links.prev = prev;
            }
            if let Some(prev) = prev {
                (*prev.as_ptr()).links.next = next;
            }

            let node = NonNull::new_unchecked(addr_of_mut!(
                (*entry.as_ptr()).tree_node
            ));
            self.tree.remove_node(node);

            if unlink_gap {
                self.gap_remove(entry);
            }
        }
    }

    /// Remove `entry`, then reinsert its predecessor with the merged
    /// gap.  `vm_map_gap_remove()` in C.
    fn gap_remove(&mut self, entry: NonNull<VmMapEntry>) {
        self.gap_remove_single(entry);
        // SAFETY: `entry` is a live entry, so its predecessor is.
        let prev = unsafe { (*entry.as_ptr()).links.prev };
        if let Some(prev) = prev {
            self.gap_remove_single(prev);
            self.gap_insert_single(prev);
        }
    }
}

impl VmMap {
    /// The saved lookup hint.
    fn hint(&self) -> *mut VmMapEntry {
        // SAFETY: `hint` is written only under `hint_lock`; this is a
        // snapshot, as the C `SAVE_HINT` readers take.
        unsafe { *self.hint.get() }
    }

    /// Forget `entry`'s wiring.  `vm_map_entry_reset_wired()` in C.
    fn entry_reset_wired(&mut self, entry: NonNull<VmMapEntry>) {
        // SAFETY: `entry` is a live entry of this map.
        unsafe {
            if (*entry.as_ptr()).wired_count != 0 {
                let size = (*entry.as_ptr())
                    .links
                    .end
                    .wrapping_sub((*entry.as_ptr()).links.start);
                self.size_wired = self.size_wired.wrapping_sub(size);
                (*entry.as_ptr()).wired_count = 0;
            }
        }
    }

    /// Wait for `in_transition` to clear: sleep on the header address
    /// with the map unlocked, as `vm_map_entry_wait()` in C does.  The
    /// caller relocks.
    fn entry_wait(&self) {
        // SAFETY: the map belongs to this thread; sleeping on the
        // header address is the C protocol.
        unsafe {
            assert_wait(
                ptr::from_ref(&self.hdr).cast_mut().cast::<c_void>(),
                0,
            );
        }
    }

    /// Deallocate `entry` from this map: unwire, remove its pmap
    /// entries and its object reference, then unlink and free it.
    /// `vm_map_entry_delete()` in C.
    ///
    /// # Safety
    ///
    /// `entry` must be linked in this map and the map write lock held.
    pub(crate) unsafe fn entry_delete(&mut self, entry: NonNull<VmMapEntry>) {
        // SAFETY: the caller promises a linked entry.
        let (s, e, size) = unsafe {
            let start = (*entry.as_ptr()).links.start;
            let end = (*entry.as_ptr()).links.end;
            (start, end, end.wrapping_sub(start))
        };

        // Check for a projected buffer: only a persistent kernel-map
        // entry may be manipulated directly.
        // SAFETY: the map is write-locked.
        if ptr::from_mut(self).cast::<c_void>() != unsafe { kernel_map }
            && !unsafe { (*entry.as_ptr()).projected_on.is_null() }
        {
            match unsafe { (*entry.as_ptr()).projection() } {
                Projection::Entry(kernel_entry) => {
                    let persistent =
                        unsafe { (*kernel_entry.as_ptr()).projected_on }
                            .is_null();
                    if persistent {
                        // Avoid an unwire fault.
                        unsafe { (*entry.as_ptr()).wired_count = 0 };
                    } else {
                        return;
                    }
                }
                // The non-persistent tag belongs to the kernel map,
                // where this branch is not taken.
                Projection::NonPersistent => return,
                Projection::None => {}
            }
        }

        // SAFETY: the entry is live and the map is locked.
        let object = unsafe { (*entry.as_ptr()).object.vm_object };

        if !object.is_null() {
            // Unwire before removing addresses from the pmap;
            // otherwise, unwiring puts the entries back.
            if unsafe { (*entry.as_ptr()).wired_count } != 0 {
                self.entry_reset_wired(entry);
                // SAFETY: the map and the linked entry are valid.
                unsafe {
                    vm_fault_unwire(
                        ptr::from_mut(self).cast::<c_void>(),
                        entry.as_ptr().cast::<c_void>(),
                    );
                }
            }

            // If the object is shared, every reference to this data
            // must go, since not all sharing pmaps can be found.
            if object == unsafe { kernel_object } {
                // SAFETY: the object is valid and the lock serializes
                // its page table.
                unsafe {
                    vm_map_glue_object_lock(object);
                    vm_object_page_remove(
                        object,
                        (*entry.as_ptr()).offset,
                        (*entry.as_ptr()).offset.wrapping_add(size),
                    );
                    vm_map_glue_object_unlock(object);
                }
            } else if unsafe { (*entry.as_ptr()).is_shared() } {
                // SAFETY: as above.
                unsafe {
                    vm_object_pmap_remove(
                        object,
                        (*entry.as_ptr()).offset,
                        (*entry.as_ptr()).offset.wrapping_add(size),
                    );
                }
            } else {
                // SAFETY: the map is locked and the pmap valid.
                unsafe { pmap_remove(self.pmap, s, e) };
                // If this object has no pager and this is the only
                // reference, the deleted pages can go now.
                // SAFETY: the object lock guards its counters.
                unsafe {
                    vm_map_glue_object_lock(object);
                    if vm_map_glue_object_can_release(object) != 0 {
                        vm_object_page_remove(
                            object,
                            (*entry.as_ptr()).offset,
                            (*entry.as_ptr()).offset.wrapping_add(size),
                        );
                    }
                    vm_map_glue_object_unlock(object);
                }
            }
        }

        // Deallocate the object only after removing all pmap entries
        // pointing to its pages.
        // SAFETY: the entry holds the reference being dropped.
        if unsafe { (*entry.as_ptr()).is_sub_map() } {
            // SAFETY: the union member is a non-null submap pointer.
            let submap = unsafe {
                NonNull::new_unchecked((*entry.as_ptr()).object.sub_map)
            };
            VmMap::deallocate(submap);
        } else {
            unsafe { vm_object_deallocate(object) };
        }

        // SAFETY: the entry is linked and the map is locked.
        unsafe {
            self.hdr.entry_unlink(entry, true);
        }
        self.size = self.size.wrapping_sub(size);
        if unsafe { (*entry.as_ptr()).max_protection } == VmProt::NONE {
            self.size_none = self.size_none.wrapping_sub(size);
        }
        // SAFETY: the entry is now unlinked and unused.
        unsafe { VmMapEntry::dispose(entry) };
    }

    /// Deallocate the given address range from this map.
    /// `vm_map_delete()` in C.  The map lock must be held unless the
    /// refcount is zero.
    pub(crate) fn delete(
        &mut self,
        start: VmOffset,
        end: VmOffset,
    ) -> Result<(), Error> {
        // SAFETY: `kernel_pmap`, `kernel_virtual_start` and
        // `kernel_virtual_end` are boot globals.
        if self.pmap == unsafe { kernel_pmap }
            && (start < unsafe { kernel_virtual_start }
                || end > unsafe { kernel_virtual_end })
        {
            // SAFETY: the C code halts here; the format has two
            // arguments as the C does.
            unsafe {
                Panic(
                    c"rust/src/vm/vm_map.rs".as_ptr(),
                    line!() as c_int,
                    c"VmMap::delete".as_ptr(),
                    c"vm_map_delete(%lx-%lx) falls in physical memory area!\n"
                        .as_ptr(),
                    start,
                    end,
                )
            };
        }

        let sentinel = self.to_entry();
        let (found, first_entry) = self.lookup_entry(start);
        let mut entry = if found {
            // SAFETY: the entry is live and the map is locked.
            unsafe { self.hdr.clip_start_at(first_entry, start, true) };
            // Fix the lookup hint now, not on every loop step.
            let prev = unsafe { (*first_entry.as_ptr()).links.prev };
            if let Some(prev) = prev {
                self.save_hint(prev);
            }
            first_entry
        } else {
            // SAFETY: the returned entry is the sentinel or live.
            unsafe { (*first_entry.as_ptr()).links.next.unwrap_or(sentinel) }
        };

        // Save the free space hint.
        let first_free = self.first_free_entry();
        if unsafe { (*first_free.as_ptr()).links.start } >= start {
            let prev = unsafe { (*entry.as_ptr()).links.prev };
            if let Some(prev) = prev {
                self.first_free = prev.as_ptr();
            }
        }

        // SAFETY: every entry touched here is linked while the map
        // lock is held.  The clip is the guarded `vm_map_clip_end()`
        // macro: an entry that already ends at or before `end` is not
        // split.
        while !self.hdr.is_sentinel(entry)
            && unsafe { (*entry.as_ptr()).links.start } < end
        {
            if end < unsafe { (*entry.as_ptr()).links.end } {
                // SAFETY: the entry is live and the map is locked.
                unsafe { self.hdr.clip_end(entry, end, true) };
            }

            // An entry in transition must be waited for; it can be
            // clipped while the map is unlocked.
            if unsafe { (*entry.as_ptr()).in_transition() } {
                // SAFETY: the entry is live and the map is locked; the
                // wakeup flag is the C wait protocol.
                unsafe {
                    (*entry.as_ptr()).set_needs_wakeup(true);
                    self.entry_wait();
                }
                // SAFETY: the entry is live.
                let map = NonNull::from(&mut *self);
                VmMap::unlock(map);
                // SAFETY: the C protocol sleeps with the map unlocked.
                unsafe { thread_block(None) };
                VmMap::lock(map);

                // The entry may have been clipped or removed: look it
                // up again.
                let (found, looked) = self.lookup_entry(start);
                entry = if found {
                    looked
                } else {
                    // SAFETY: the returned entry is the sentinel or
                    // live.
                    unsafe {
                        (*looked.as_ptr()).links.next.unwrap_or(sentinel)
                    }
                };
                continue;
            }

            let next = unsafe { (*entry.as_ptr()).links.next };
            // SAFETY: the entry is linked and the map is locked.
            unsafe { self.entry_delete(entry) };
            entry = next.unwrap_or(sentinel);
        }

        if self.wait_for_space() {
            // SAFETY: the C code wakes the map address.
            unsafe {
                vm_map_glue_thread_wakeup(ptr::from_mut(self).cast::<c_void>())
            };
        }

        Ok(())
    }

    /// Remove the given address range, clamping it to the map bounds
    /// and taking the map lock.  `vm_map_remove()` in C.
    pub(crate) fn remove(
        &mut self,
        start: VmOffset,
        end: VmOffset,
    ) -> Result<(), Error> {
        let map = NonNull::from(&mut *self);
        VmMap::lock(map);

        let mut start = start;
        let mut end = end;
        self.range_check(&mut start, &mut end);

        let result = self.delete(start, end);

        VmMap::unlock(map);
        result
    }

    /// Try to coalesce `entry` with its predecessor.
    /// `vm_map_coalesce_entry()` in C.  The map lock must be held; a
    /// coalesced entry is destroyed by the call.
    ///
    /// # Safety
    ///
    /// `entry` must be a live entry of this map.
    pub(crate) unsafe fn coalesce_entry(
        &mut self,
        entry: NonNull<VmMapEntry>,
    ) -> bool {
        // SAFETY: the caller promises a live entry.
        let Some(prev) = (unsafe { (*entry.as_ptr()).links.prev }) else {
            return false;
        };

        // SAFETY: the caller promises live entries; the checks below
        // only read them.
        if self.hdr.is_sentinel(entry)
            || self.hdr.is_sentinel(prev)
            || unsafe { (*prev.as_ptr()).links.end }
                != unsafe { (*entry.as_ptr()).links.start }
            || unsafe { (*prev.as_ptr()).is_shared() }
            || unsafe { (*entry.as_ptr()).is_shared() }
            || unsafe { (*prev.as_ptr()).is_sub_map() }
            || unsafe { (*entry.as_ptr()).is_sub_map() }
            || unsafe { (*prev.as_ptr()).inheritance }
                != unsafe { (*entry.as_ptr()).inheritance }
            || unsafe { (*prev.as_ptr()).protection }
                != unsafe { (*entry.as_ptr()).protection }
            || unsafe { (*prev.as_ptr()).max_protection }
                != unsafe { (*entry.as_ptr()).max_protection }
            || unsafe { (*prev.as_ptr()).needs_copy() }
                != unsafe { (*entry.as_ptr()).needs_copy() }
            || unsafe { (*prev.as_ptr()).in_transition() }
            || unsafe { (*entry.as_ptr()).in_transition() }
            || unsafe { (*prev.as_ptr()).wired_count }
                != unsafe { (*entry.as_ptr()).wired_count }
            || !unsafe { (*prev.as_ptr()).projected_on.is_null() }
            || !unsafe { (*entry.as_ptr()).projected_on.is_null() }
        {
            return false;
        }

        let prev_size = unsafe {
            (*prev.as_ptr())
                .links
                .end
                .wrapping_sub((*prev.as_ptr()).links.start)
        };
        let entry_size = unsafe {
            (*entry.as_ptr())
                .links
                .end
                .wrapping_sub((*entry.as_ptr()).links.start)
        };

        // See whether the two objects can be coalesced; the C passes
        // the predecessor's fields as the out-parameters.
        // SAFETY: both entries are live and the map is locked.
        let coalesced = unsafe {
            vm_object_coalesce(
                (*prev.as_ptr()).object.vm_object,
                (*entry.as_ptr()).object.vm_object,
                (*prev.as_ptr()).offset,
                (*entry.as_ptr()).offset,
                prev_size,
                entry_size,
                addr_of_mut!((*prev.as_ptr()).object.vm_object),
                addr_of_mut!((*prev.as_ptr()).offset),
            )
        };
        if coalesced == 0 {
            return false;
        }

        // Update the hints.
        if self.hint() == entry.as_ptr() {
            self.save_hint(prev);
        }
        if self.first_free_entry() == entry {
            self.first_free = prev.as_ptr();
        }

        // Get rid of the entry without changing wirings or the pmap,
        // and without altering the map size.
        // SAFETY: both entries are live and the map is locked.
        unsafe {
            (*prev.as_ptr()).links.end = (*entry.as_ptr()).links.end;
            self.hdr.entry_unlink(entry, true);
        }
        // SAFETY: the entry is now unlinked and unused.
        unsafe { VmMapEntry::dispose(entry) };

        true
    }
}

impl VmMap {
    /// Count one more wiring of `entry`.  `vm_map_entry_inc_wired()`
    /// in C.
    fn entry_inc_wired(&mut self, entry: NonNull<VmMapEntry>) {
        // SAFETY: `entry` is a live entry of this map.
        unsafe {
            if (*entry.as_ptr()).wired_count > 1 {
                return;
            }
            if (*entry.as_ptr()).wired_count == 0 {
                let size = (*entry.as_ptr())
                    .links
                    .end
                    .wrapping_sub((*entry.as_ptr()).links.start);
                self.size_wired = self.size_wired.wrapping_add(size);
            }
            (*entry.as_ptr()).wired_count =
                (*entry.as_ptr()).wired_count.wrapping_add(1);
        }
    }

    /// The `VM_MAP_RANGE_CHECK()` macro: clamp a range to the map's
    /// bounds.
    fn range_check(&self, start: &mut VmOffset, end: &mut VmOffset) {
        let min = self.hdr.links.start;
        let max = self.hdr.links.end;
        if *start < min {
            *start = min;
        }
        if *end > max {
            *end = max;
        }
        if *start > *end {
            *start = *end;
        }
    }

    /// Scan entries and update their wiring: unwire what left
    /// `wired_access`, wire what entered it, and fault in the newly
    /// wired pages.  `vm_map_pageable_scan()` in C.
    ///
    /// The map must be locked; on return it is read-locked if wiring
    /// faults were needed.  The C code unlocks the map while faulting
    /// the kernel map; the same trust applies here.
    fn pageable_scan(
        &mut self,
        start_entry: NonNull<VmMapEntry>,
        end: VmOffset,
    ) {
        let sentinel = self.to_entry();
        let map = NonNull::from(&mut *self);
        let mut do_wire_faults = false;

        // Pass 1. Update counters and prepare wiring faults.
        let mut entry = start_entry;
        while !self.hdr.is_sentinel(entry)
            && unsafe { (*entry.as_ptr()).links.start } < end
        {
            // SAFETY: the entries are live and the map is locked.
            let next =
                unsafe { (*entry.as_ptr()).links.next }.unwrap_or(sentinel);
            unsafe {
                // Unwiring faults can be done under the write lock.
                if (*entry.as_ptr()).wired_access == VmProt::NONE {
                    if (*entry.as_ptr()).wired_count != 0 {
                        self.entry_reset_wired(entry);
                        vm_fault_unwire(
                            ptr::from_mut(self).cast::<c_void>(),
                            entry.as_ptr().cast::<c_void>(),
                        );
                    }
                    entry = next;
                    continue;
                }

                // Entries that cannot be accessed must not be wired.
                if (*entry.as_ptr()).protection == VmProt::NONE {
                    if (*entry.as_ptr()).wired_count == 0 {
                        entry = next;
                        continue;
                    }
                    self.entry_reset_wired(entry);
                    vm_fault_unwire(
                        ptr::from_mut(self).cast::<c_void>(),
                        entry.as_ptr().cast::<c_void>(),
                    );
                    entry = next;
                    continue;
                }

                // With the write lock held, create any shadow or
                // zero-fill object the wiring needs, then raise the
                // count; the faults follow under a read lock.
                if (*entry.as_ptr()).wired_count == 0 {
                    if (*entry.as_ptr()).needs_copy()
                        && (*entry.as_ptr()).protection & VmProt::WRITE
                            != VmProt::NONE
                    {
                        let size = (*entry.as_ptr())
                            .links
                            .end
                            .wrapping_sub((*entry.as_ptr()).links.start);
                        let mut object = (*entry.as_ptr()).object.vm_object;
                        let mut offset = (*entry.as_ptr()).offset;

                        vm_object_shadow(&mut object, &mut offset, size);

                        (*entry.as_ptr()).object.vm_object = object;
                        (*entry.as_ptr()).offset = offset;
                        (*entry.as_ptr()).set_needs_copy(false);
                    }

                    if (*entry.as_ptr()).object.vm_object.is_null() {
                        let size = (*entry.as_ptr())
                            .links
                            .end
                            .wrapping_sub((*entry.as_ptr()).links.start);
                        (*entry.as_ptr()).object.vm_object =
                            vm_object_allocate(size);
                        (*entry.as_ptr()).offset = 0;
                    }
                }

                self.entry_inc_wired(entry);

                if (*entry.as_ptr()).wired_count == 1 {
                    do_wire_faults = true;
                }
            }
            entry = next;
        }

        // Pass 2. Trigger wiring faults.
        if !do_wire_faults {
            return;
        }

        let is_kernel = self.pmap == unsafe { kernel_pmap };

        if is_kernel {
            // In the kernel map, unlock rather than downgrade, and
            // mark the entries so they cannot be coalesced meanwhile.
            let mut entry = start_entry;
            while !self.hdr.is_sentinel(entry)
                && unsafe { (*entry.as_ptr()).links.end } <= end
            {
                // SAFETY: the entries are live and the map lock is
                // held; the flags keep them out of coalescing while
                // the kernel map is unlocked for the faults.
                unsafe {
                    (*entry.as_ptr()).set_in_transition(true);
                    (*entry.as_ptr()).set_needs_wakeup(false);
                }
                entry = unsafe { (*entry.as_ptr()).links.next }
                    .unwrap_or(sentinel);
            }
            VmMap::unlock(map);
        } else {
            // SAFETY: the map lock is held; the downgrade is the C
            // protocol for faulting with a read lock.
            unsafe {
                lock_set_recursive(addr_of_mut!((*map.as_ptr()).lock));
                lock_write_to_read(addr_of_mut!((*map.as_ptr()).lock));
            }
        }

        let mut entry = start_entry;
        while !self.hdr.is_sentinel(entry)
            && unsafe { (*entry.as_ptr()).links.end } <= end
        {
            // Only a count of one was raised by the pass above.
            if unsafe { (*entry.as_ptr()).wired_count } == 1 {
                // SAFETY: the map may be read-locked; the C code
                // assumes the faults always succeed.
                unsafe {
                    vm_fault_wire(
                        ptr::from_mut(self).cast::<c_void>(),
                        entry.as_ptr().cast::<c_void>(),
                    )
                };
            }
            entry =
                unsafe { (*entry.as_ptr()).links.next }.unwrap_or(sentinel);
        }

        if is_kernel {
            VmMap::lock(map);
            let mut entry = start_entry;
            while !self.hdr.is_sentinel(entry)
                && unsafe { (*entry.as_ptr()).links.end } <= end
            {
                // Nothing should have touched the region while the map
                // was unlocked.
                // SAFETY: the entries are live and the map is locked.
                unsafe { (*entry.as_ptr()).set_in_transition(false) };
                entry = unsafe { (*entry.as_ptr()).links.next }
                    .unwrap_or(sentinel);
            }
        } else {
            // SAFETY: the map read lock is held.
            unsafe {
                lock_clear_recursive(addr_of_mut!((*map.as_ptr()).lock));
            }
        }
    }

    /// Set the protection of a range.  `vm_map_protect()` in C.
    pub(crate) fn protect(
        &mut self,
        start_in: VmOffset,
        end_in: VmOffset,
        new_prot: VmProt,
        set_max: bool,
    ) -> Result<(), Error> {
        let map = NonNull::from(&mut *self);
        VmMap::lock(map);

        let mut start = start_in;
        let mut end = end_in;
        self.range_check(&mut start, &mut end);

        let sentinel = self.to_entry();
        let (found, temp_entry) = self.lookup_entry(start);
        let entry = if found {
            // SAFETY: the entry contains `start`.
            unsafe { self.hdr.clip_start_at(temp_entry, start, true) };
            temp_entry
        } else {
            // SAFETY: the entry before the range is live.
            unsafe { (*temp_entry.as_ptr()).links.next.unwrap_or(sentinel) }
        };

        // Pass 1: protection violations.
        let mut current = entry;
        while !self.hdr.is_sentinel(current)
            && unsafe { (*current.as_ptr()).links.start } < end
        {
            if unsafe { (*current.as_ptr()).is_sub_map() } {
                VmMap::unlock(map);
                return Err(Error::InvalidArgument);
            }
            let max = unsafe { (*current.as_ptr()).max_protection };
            if new_prot.bits() & (VmProt::NOTIFY.bits() | max.bits())
                != new_prot.bits()
            {
                VmMap::unlock(map);
                return Err(Error::ProtectionFailure);
            }
            current =
                unsafe { (*current.as_ptr()).links.next }.unwrap_or(sentinel);
        }

        // Pass 2: fix the protections.  Clipping is not necessary a
        // second time.
        current = entry;
        while !self.hdr.is_sentinel(current)
            && unsafe { (*current.as_ptr()).links.start } < end
        {
            if end < unsafe { (*current.as_ptr()).links.end } {
                // SAFETY: the entry spans `end`.
                unsafe { self.hdr.clip_end(current, end, true) };
            }

            // SAFETY: the entry is live and the map is locked.
            let old_prot = unsafe { (*current.as_ptr()).protection };
            if set_max {
                if unsafe { (*current.as_ptr()).max_protection } != new_prot
                    && new_prot == VmProt::NONE
                {
                    let size = unsafe {
                        (*current.as_ptr())
                            .links
                            .end
                            .wrapping_sub((*current.as_ptr()).links.start)
                    };
                    self.size_none = self.size_none.wrapping_add(size);
                }
                unsafe {
                    (*current.as_ptr()).max_protection = new_prot;
                    (*current.as_ptr()).protection = new_prot & old_prot;
                }
            } else {
                unsafe { (*current.as_ptr()).protection = new_prot };
            }

            // The new protection must not conflict with the desired
            // wired access, if any.
            if unsafe { (*current.as_ptr()).protection } != VmProt::NONE
                && (unsafe { (*current.as_ptr()).wired_access }
                    != VmProt::NONE
                    || self.wiring_required())
            {
                unsafe {
                    (*current.as_ptr()).wired_access =
                        (*current.as_ptr()).protection
                };
            }

            if unsafe { (*current.as_ptr()).protection } != old_prot {
                // SAFETY: the pmap is valid and the map is locked.
                unsafe {
                    pmap_protect(
                        self.pmap,
                        (*current.as_ptr()).links.start,
                        (*current.as_ptr()).links.end,
                        (*current.as_ptr()).protection.bits(),
                    )
                };
            }

            let next = unsafe { (*current.as_ptr()).links.next };
            // SAFETY: the entry is live and the map is locked.
            let _ = unsafe { self.coalesce_entry(current) };
            current = next.unwrap_or(sentinel);
        }

        let _ = unsafe { self.coalesce_entry(current) };

        self.pageable_scan(entry, end);

        VmMap::unlock(map);
        Ok(())
    }

    /// Set the inheritance of a range.  `vm_map_inherit()` in C.
    pub(crate) fn inherit(
        &mut self,
        start_in: VmOffset,
        end_in: VmOffset,
        new_inheritance: VmInherit,
    ) -> Result<(), Error> {
        let map = NonNull::from(&mut *self);
        VmMap::lock(map);

        let mut start = start_in;
        let mut end = end_in;
        self.range_check(&mut start, &mut end);

        let sentinel = self.to_entry();
        let (found, temp_entry) = self.lookup_entry(start);
        let mut entry = if found {
            // SAFETY: the entry contains `start`.
            unsafe { self.hdr.clip_start_at(temp_entry, start, true) };
            temp_entry
        } else {
            // SAFETY: the entry before the range is live.
            unsafe { (*temp_entry.as_ptr()).links.next.unwrap_or(sentinel) }
        };

        while !self.hdr.is_sentinel(entry)
            && unsafe { (*entry.as_ptr()).links.start } < end
        {
            if end < unsafe { (*entry.as_ptr()).links.end } {
                // SAFETY: the entry spans `end`.
                unsafe { self.hdr.clip_end(entry, end, true) };
            }
            unsafe { (*entry.as_ptr()).inheritance = new_inheritance };

            let next = unsafe { (*entry.as_ptr()).links.next };
            // SAFETY: the entry is live and the map is locked.
            let _ = unsafe { self.coalesce_entry(entry) };
            entry = next.unwrap_or(sentinel);
        }

        // SAFETY: the map is locked; coalescing the sentinel is a
        // no-op that the C also attempts.
        let _ = unsafe { self.coalesce_entry(entry) };

        VmMap::unlock(map);
        Ok(())
    }

    /// Set the pageability of a range.  `vm_map_pageable()` in C.
    ///
    /// With `lock_map`, the map is locked and unlocked here; without
    /// it, the caller holds the lock and the function returns with a
    /// read lock on success.
    pub(crate) fn pageable(
        &mut self,
        start_in: VmOffset,
        end_in: VmOffset,
        access_type: VmProt,
        lock_map: bool,
        check_range: bool,
    ) -> Result<(), Error> {
        let map = NonNull::from(&mut *self);
        if lock_map {
            VmMap::lock(map);
        }

        let mut start = start_in;
        let mut end = end_in;
        self.range_check(&mut start, &mut end);

        let (found, start_entry) = self.lookup_entry(start);
        if !found {
            // The start address is not in the map; this is fatal.
            if lock_map {
                VmMap::unlock(map);
            }
            return Err(Error::NoSpace);
        }
        // SAFETY: the entry contains `start`.
        unsafe { self.hdr.clip_start_at(start_entry, start, true) };

        let sentinel = self.to_entry();
        let mut entry = start_entry;
        while !self.hdr.is_sentinel(entry)
            && unsafe { (*entry.as_ptr()).links.start } < end
        {
            if end < unsafe { (*entry.as_ptr()).links.end } {
                // SAFETY: the entry spans `end`.
                unsafe { self.hdr.clip_end(entry, end, true) };
            }

            if check_range {
                let entry_end = unsafe { (*entry.as_ptr()).links.end };
                let next = unsafe { (*entry.as_ptr()).links.next };
                let hole = entry_end < end
                    && match next {
                        None => true,
                        Some(next) if next == sentinel => true,
                        Some(next) => {
                            // SAFETY: the next entry is live.
                            let next_start =
                                unsafe { (*next.as_ptr()).links.start };
                            next_start > entry_end
                        }
                    };
                let protection = unsafe { (*entry.as_ptr()).protection };
                if hole
                    || protection.bits() & access_type.bits()
                        != access_type.bits()
                {
                    if lock_map {
                        VmMap::unlock(map);
                    }
                    return Err(Error::NoSpace);
                }
            }

            entry =
                unsafe { (*entry.as_ptr()).links.next }.unwrap_or(sentinel);
        }
        let end_entry = entry;

        // Pass 2: set the desired wired access.
        let mut entry = start_entry;
        while entry != end_entry {
            // SAFETY: the entries up to `end_entry` are live and the
            // map is locked.
            unsafe { (*entry.as_ptr()).wired_access = access_type };
            entry =
                unsafe { (*entry.as_ptr()).links.next }.unwrap_or(sentinel);
        }

        self.pageable_scan(start_entry, end);

        if lock_map {
            VmMap::unlock(map);
        }
        Ok(())
    }

    /// Wire the whole map's current contents to match its protections.
    /// `vm_map_pageable_current()` in C.
    fn pageable_current(&mut self, access_type: VmProt) -> Result<(), Error> {
        let Some(min_node) = self.hdr.tree.firstlast_node(RBTREE_LEFT) else {
            // The C would fault on an empty map's null node; there is
            // nothing to wire, so report success instead.
            return Ok(());
        };
        let Some(max_node) = self.hdr.tree.firstlast_node(RBTREE_RIGHT) else {
            return Ok(());
        };
        // SAFETY: the nodes came from this map's entry tree.
        let min = unsafe { VmMapEntry::from_tree_node(min_node) };
        // SAFETY: as above.
        let max = unsafe { VmMapEntry::from_tree_node(max_node) };
        let min_address = unsafe { (*min.as_ptr()).links.start };
        let max_address = unsafe { (*max.as_ptr()).links.end };

        self.pageable(min_address, max_address, access_type, false, false)
    }

    /// Wire a whole map, now and/or in the future.
    /// `vm_map_pageable_all()` in C.
    pub(crate) fn pageable_all(&mut self, flags: c_int) -> Result<(), Error> {
        const WIRE_NONE: c_int = 0;
        const WIRE_CURRENT: c_int = 1;
        const WIRE_FUTURE: c_int = 2;
        const WIRE_ALL: c_int = WIRE_CURRENT | WIRE_FUTURE;

        if flags & !WIRE_ALL != 0 {
            return Err(Error::InvalidArgument);
        }

        let map = NonNull::from(&mut *self);
        VmMap::lock(map);

        if flags == WIRE_NONE {
            self.flags &= !VM_MAP_WIRING_REQUIRED;
            let result = self.pageable_current(VmProt::NONE);
            VmMap::unlock(map);
            return result;
        }

        let wiring_required = self.wiring_required();

        if flags & WIRE_FUTURE != 0 {
            self.flags |= VM_MAP_WIRING_REQUIRED;
        }

        if flags & WIRE_CURRENT != 0 {
            let result = self.pageable_current(VmProt::READ | VmProt::WRITE);

            if result.is_err() {
                if flags & WIRE_FUTURE != 0 {
                    if wiring_required {
                        self.flags |= VM_MAP_WIRING_REQUIRED;
                    } else {
                        self.flags &= !VM_MAP_WIRING_REQUIRED;
                    }
                }
                VmMap::unlock(map);
                return result;
            }
        }

        VmMap::unlock(map);
        Ok(())
    }
}
