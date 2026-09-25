// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/slab.c and kern/slab.h:
//   Copyright (c) 2011 Free Software Foundation.
//   Copyright (c) 2010, 2011 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The object-caching memory allocator, which `kern/slab.c` used to define,
//! and the `struct kmem_cache` mirror of `kern/slab.h`.

use crate::arch::i386::phys::kvtophys;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue;
use crate::kern::list::{List, entry as list_entry};
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock;
use crate::kern::rbtree::{RBTREE_LEFT, Rbtree, RbtreeNode, rbtree_node_init};
use crate::utils::cell::SyncCell;
use crate::vm::error::KERN_SUCCESS;
use crate::vm::vm_kern::{self, VM_MIN_KERNEL_ADDRESS};
use crate::vm::vm_map::VmMap;
use crate::vm::vm_map::round_page;
use crate::vm::vm_resident::{self, VM_PAGE_DIRECTMAP};
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ops;
use core::ptr::{self, NonNull, addr_of_mut, with_exposed_provenance_mut};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// `KMEM_CACHE_NAME_SIZE` of <kern/slab.h>: the length of a cache name,
/// chosen so the mirror fits in two 64-byte cache lines.
pub const KMEM_CACHE_NAME_SIZE: usize = 24;

/// `CACHE_NAME_MAX_LEN` of <mach_debug/slab_info.h>.
const CACHE_NAME_MAX_LEN: usize = 32;

/// `KMEM_ALIGN_MIN` of kern/slab.c.
const KMEM_ALIGN_MIN: usize = 8;

/// `KMEM_BUF_SIZE_THRESHOLD` of kern/slab.c.
const KMEM_BUF_SIZE_THRESHOLD: usize = PAGE_SIZE / 8;

/// `KALLOC_FIRST_SHIFT` of kern/slab.c.
const KALLOC_FIRST_SHIFT: usize = 5;

/// `KALLOC_NR_CACHES` of kern/slab.c.
const KALLOC_NR_CACHES: usize = 13;

/// `KMEM_GC_INTERVAL` of kern/slab.c: the multiplier of the `hz` tick rate.
const KMEM_GC_TICKS: usize = 5;

/// `KMEM_REDZONE_BYTE` of kern/slab.c.
const KMEM_REDZONE_BYTE: u8 = 0xbb;

/// `KMEM_REDZONE_WORD` of kern/slab.c, little-endian.
#[cfg(target_pointer_width = "64")]
const KMEM_REDZONE_WORD: c_ulong = 0xcefa_edfe_cefa_edfe;
#[cfg(target_pointer_width = "32")]
const KMEM_REDZONE_WORD: c_ulong = 0xcefa_edfe;

/// `KMEM_FREE_PATTERN` of kern/slab.c, little-endian.
const KMEM_FREE_PATTERN: u64 = 0xefbe_adde_efbe_adde;

/// `KMEM_UNINIT_PATTERN` of kern/slab.c, little-endian.
const KMEM_UNINIT_PATTERN: u64 = 0xfeca_ddba_feca_ddba;

/// `KMEM_BUFTAG_ALLOC` of kern/slab.c, little-endian.
#[cfg(target_pointer_width = "64")]
const KMEM_BUFTAG_ALLOC: c_ulong = 0xedc8_10a1_edc8_10a1;
#[cfg(target_pointer_width = "32")]
const KMEM_BUFTAG_ALLOC: c_ulong = 0xedc8_10a1;

/// `KMEM_BUFTAG_FREE` of kern/slab.c, little-endian.
#[cfg(target_pointer_width = "64")]
const KMEM_BUFTAG_FREE: c_ulong = 0x0cb1_eef4_0cb1_eef4;
#[cfg(target_pointer_width = "32")]
const KMEM_BUFTAG_FREE: c_ulong = 0x0cb1_eef4;

/// The `KMEM_CACHE_*` flags of <kern/slab.h> that reach
/// [`KmemCache::init`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct CacheInitFlags(c_int);

impl CacheInitFlags {
    /// `KMEM_CACHE_NOOFFSLAB`: don't allocate external slab data.
    pub const NOOFFSLAB: Self = Self(0x1);
    /// `KMEM_CACHE_PHYSMEM`: allocate from physical memory.
    pub const PHYSMEM: Self = Self(0x2);
    /// `KMEM_CACHE_VERIFY`: use the debugging facilities.
    pub const VERIFY: Self = Self(0x4);
    /// The C callers' literal `0`.
    pub const EMPTY: Self = Self(0);

    /// The C `int` the initializer was called with.
    pub(crate) const fn from_bits(bits: c_int) -> Self {
        Self(bits)
    }

    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl ops::BitOr for CacheInitFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The `KMEM_CF_*` flags of kern/slab.c, the `flags` field of a cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
struct CacheFlags(c_int);

impl CacheFlags {
    /// `KMEM_CF_SLAB_EXTERNAL`.
    const SLAB_EXTERNAL: Self = Self(0x01);
    /// `KMEM_CF_PHYSMEM`.
    const PHYSMEM: Self = Self(0x02);
    /// `KMEM_CF_DIRECT`.
    const DIRECT: Self = Self(0x04);
    /// `KMEM_CF_USE_TREE`.
    const USE_TREE: Self = Self(0x08);
    /// `KMEM_CF_USE_PAGE`.
    const USE_PAGE: Self = Self(0x10);
    /// `KMEM_CF_VERIFY`.
    const VERIFY: Self = Self(0x20);

    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// The `KMEM_ERR_*` codes `kmem_cache_error()` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheError {
    /// `KMEM_ERR_INVALID`.
    Invalid,
    /// `KMEM_ERR_DOUBLEFREE`.
    DoubleFree,
    /// `KMEM_ERR_BUFTAG`.
    Buftag,
    /// `KMEM_ERR_MODIFIED`.
    Modified,
    /// `KMEM_ERR_REDZONE`.
    Redzone,
}

/// `union kmem_bufctl` of <kern/slab.h>: the free-list link a free buffer
/// carries, or the redzone word of a verify-mode buffer.
#[repr(C)]
union KmemBufctl {
    next: *mut KmemBufctl,
    redzone: c_ulong,
}

const _: () = {
    assert!(size_of::<KmemBufctl>() == size_of::<*mut KmemBufctl>());
    assert!(align_of::<KmemBufctl>() == align_of::<*mut KmemBufctl>());
};

/// `struct kmem_buftag` of <kern/slab.h>: the allocated/free state a verify
/// cache stamps on each buffer.
#[repr(C)]
struct KmemBuftag {
    state: c_ulong,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<KmemBuftag>() == 8);
    assert!(align_of::<KmemBuftag>() == 8);
    assert!(offset_of!(KmemBuftag, state) == 0);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<KmemBuftag>() == 4);
    assert!(align_of::<KmemBuftag>() == 4);
    assert!(offset_of!(KmemBuftag, state) == 0);
};

/// `struct kmem_slab` of <kern/slab.h>: a page-aligned collection of
/// unconstructed buffers.
#[repr(C)]
struct KmemSlab {
    cache: *mut KmemCache,
    list_node: List,
    tree_node: RbtreeNode,
    nr_refs: c_ulong,
    first_free: *mut KmemBufctl,
    addr: *mut u8,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<KmemSlab>() == 72);
    assert!(align_of::<KmemSlab>() == 8);
    assert!(offset_of!(KmemSlab, cache) == 0);
    assert!(offset_of!(KmemSlab, list_node) == 8);
    assert!(offset_of!(KmemSlab, tree_node) == 24);
    assert!(offset_of!(KmemSlab, nr_refs) == 48);
    assert!(offset_of!(KmemSlab, first_free) == 56);
    assert!(offset_of!(KmemSlab, addr) == 64);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<KmemSlab>() == 36);
    assert!(align_of::<KmemSlab>() == 4);
    assert!(offset_of!(KmemSlab, cache) == 0);
    assert!(offset_of!(KmemSlab, list_node) == 4);
    assert!(offset_of!(KmemSlab, tree_node) == 12);
    assert!(offset_of!(KmemSlab, nr_refs) == 24);
    assert!(offset_of!(KmemSlab, first_free) == 28);
    assert!(offset_of!(KmemSlab, addr) == 32);
};

/// The constructor a cache may hold; `kmem_cache_ctor_t` of <kern/slab.h>.
pub type KmemCacheCtor = Option<unsafe extern "C" fn(*mut c_void)>;

/// `struct kmem_cache` of <kern/slab.h>: a cache of objects.
///
/// The layout is the C record's, `__cacheline_aligned` (`1 << CPU_L1_SHIFT`,
/// 64 bytes on both builds) included; the field order is the C's, which put
/// every hot field in the first cache line.
#[repr(C, align(64))]
pub struct KmemCache {
    lock: SimpleLock,
    node: List,
    partial_slabs: List,
    free_slabs: List,
    active_slabs: Rbtree,
    flags: CacheFlags,
    bufctl_dist: usize,
    slab_size: usize,
    bufs_per_slab: c_ulong,
    nr_objs: c_ulong,
    nr_free_slabs: c_ulong,
    ctor: KmemCacheCtor,
    obj_size: usize,
    align: usize,
    buf_size: usize,
    color: usize,
    color_max: usize,
    nr_bufs: c_ulong,
    nr_slabs: c_ulong,
    name: [c_char; KMEM_CACHE_NAME_SIZE],
    buftag_dist: usize,
    redzone_pad: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<KmemCache>() == 256);
    assert!(align_of::<KmemCache>() == 64);
    assert!(offset_of!(KmemCache, lock) == 0);
    assert!(offset_of!(KmemCache, node) == 8);
    assert!(offset_of!(KmemCache, partial_slabs) == 24);
    assert!(offset_of!(KmemCache, free_slabs) == 40);
    assert!(offset_of!(KmemCache, active_slabs) == 56);
    assert!(offset_of!(KmemCache, flags) == 64);
    assert!(offset_of!(KmemCache, bufctl_dist) == 72);
    assert!(offset_of!(KmemCache, slab_size) == 80);
    assert!(offset_of!(KmemCache, bufs_per_slab) == 88);
    assert!(offset_of!(KmemCache, nr_objs) == 96);
    assert!(offset_of!(KmemCache, nr_free_slabs) == 104);
    assert!(offset_of!(KmemCache, ctor) == 112);
    assert!(offset_of!(KmemCache, obj_size) == 120);
    assert!(offset_of!(KmemCache, align) == 128);
    assert!(offset_of!(KmemCache, buf_size) == 136);
    assert!(offset_of!(KmemCache, color) == 144);
    assert!(offset_of!(KmemCache, color_max) == 152);
    assert!(offset_of!(KmemCache, nr_bufs) == 160);
    assert!(offset_of!(KmemCache, nr_slabs) == 168);
    assert!(offset_of!(KmemCache, name) == 176);
    assert!(offset_of!(KmemCache, buftag_dist) == 200);
    assert!(offset_of!(KmemCache, redzone_pad) == 208);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<KmemCache>() == 128);
    assert!(align_of::<KmemCache>() == 64);
    assert!(offset_of!(KmemCache, lock) == 0);
    assert!(offset_of!(KmemCache, node) == 4);
    assert!(offset_of!(KmemCache, partial_slabs) == 12);
    assert!(offset_of!(KmemCache, free_slabs) == 20);
    assert!(offset_of!(KmemCache, active_slabs) == 28);
    assert!(offset_of!(KmemCache, flags) == 32);
    assert!(offset_of!(KmemCache, bufctl_dist) == 36);
    assert!(offset_of!(KmemCache, slab_size) == 40);
    assert!(offset_of!(KmemCache, bufs_per_slab) == 44);
    assert!(offset_of!(KmemCache, nr_objs) == 48);
    assert!(offset_of!(KmemCache, nr_free_slabs) == 52);
    assert!(offset_of!(KmemCache, ctor) == 56);
    assert!(offset_of!(KmemCache, obj_size) == 60);
    assert!(offset_of!(KmemCache, align) == 64);
    assert!(offset_of!(KmemCache, buf_size) == 68);
    assert!(offset_of!(KmemCache, color) == 72);
    assert!(offset_of!(KmemCache, color_max) == 76);
    assert!(offset_of!(KmemCache, nr_bufs) == 80);
    assert!(offset_of!(KmemCache, nr_slabs) == 84);
    assert!(offset_of!(KmemCache, name) == 88);
    assert!(offset_of!(KmemCache, buftag_dist) == 112);
    assert!(offset_of!(KmemCache, redzone_pad) == 116);
};

/// `cache_info_t` of <mach_debug/slab_info.h>, the record `host_slab_info()`
/// copies out.
#[repr(C)]
pub struct CacheInfo {
    /// The `KMEM_CF_*` bits.
    pub flags: c_int,
    pub cpu_pool_size: VmSize,
    pub obj_size: VmSize,
    pub align: VmSize,
    pub buf_size: VmSize,
    pub slab_size: VmSize,
    pub bufs_per_slab: c_ulong,
    pub nr_objs: c_ulong,
    pub nr_bufs: c_ulong,
    pub nr_slabs: c_ulong,
    pub nr_free_slabs: c_ulong,
    pub name: [c_char; CACHE_NAME_MAX_LEN],
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<CacheInfo>() == 120);
    assert!(align_of::<CacheInfo>() == 8);
    assert!(offset_of!(CacheInfo, flags) == 0);
    assert!(offset_of!(CacheInfo, cpu_pool_size) == 8);
    assert!(offset_of!(CacheInfo, obj_size) == 16);
    assert!(offset_of!(CacheInfo, align) == 24);
    assert!(offset_of!(CacheInfo, buf_size) == 32);
    assert!(offset_of!(CacheInfo, slab_size) == 40);
    assert!(offset_of!(CacheInfo, bufs_per_slab) == 48);
    assert!(offset_of!(CacheInfo, nr_objs) == 56);
    assert!(offset_of!(CacheInfo, nr_bufs) == 64);
    assert!(offset_of!(CacheInfo, nr_slabs) == 72);
    assert!(offset_of!(CacheInfo, nr_free_slabs) == 80);
    assert!(offset_of!(CacheInfo, name) == 88);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<CacheInfo>() == 76);
    assert!(align_of::<CacheInfo>() == 4);
    assert!(offset_of!(CacheInfo, flags) == 0);
    assert!(offset_of!(CacheInfo, cpu_pool_size) == 4);
    assert!(offset_of!(CacheInfo, obj_size) == 8);
    assert!(offset_of!(CacheInfo, align) == 12);
    assert!(offset_of!(CacheInfo, buf_size) == 16);
    assert!(offset_of!(CacheInfo, slab_size) == 20);
    assert!(offset_of!(CacheInfo, bufs_per_slab) == 24);
    assert!(offset_of!(CacheInfo, nr_objs) == 28);
    assert!(offset_of!(CacheInfo, nr_bufs) == 32);
    assert!(offset_of!(CacheInfo, nr_slabs) == 36);
    assert!(offset_of!(CacheInfo, nr_free_slabs) == 40);
    assert!(offset_of!(CacheInfo, name) == 44);
};

/// `kmem_slab_cache` of kern/slab.c: the cache for off-slab data.
static KMEM_SLAB_CACHE: SyncCell<KmemCache> =
    SyncCell(UnsafeCell::new(KmemCache::zeroed()));

/// `kalloc_caches` of kern/slab.c: the general-purpose caches, from 32 bytes
/// to 128 KiB, one doubling per entry.
static KALLOC_CACHES: SyncCell<[KmemCache; KALLOC_NR_CACHES]> = SyncCell(
    UnsafeCell::new([const { KmemCache::zeroed() }; KALLOC_NR_CACHES]),
);

/// `kmem_cache_list` of kern/slab.c: every cache, in initialization order.
static KMEM_CACHE_LIST: SyncCell<List> =
    SyncCell(UnsafeCell::new(List::unlinked()));

/// `kmem_nr_caches` of kern/slab.c.
static KMEM_NR_CACHES: AtomicU32 = AtomicU32::new(0);

/// `kmem_cache_list_lock` of kern/slab.c.
static KMEM_CACHE_LIST_LOCK: SimpleLock = SimpleLock::new();

/// `kmem_gc_last_tick` of kern/slab.c.
static KMEM_GC_LAST_TICK: AtomicUsize = AtomicUsize::new(0);

/// The global cache list head; every walk is under `KMEM_CACHE_LIST_LOCK`.
fn cache_list() -> *mut List {
    KMEM_CACHE_LIST.0.get()
}

/// The off-slab data cache; live from `slab_init()` on.
fn slab_cache() -> *mut KmemCache {
    KMEM_SLAB_CACHE.0.get()
}

/// `P2ROUND()` of <kern/macros.h>.
const fn round_up(value: usize, align: usize) -> usize {
    value.wrapping_add(align - 1) & !(align - 1)
}

impl KmemCache {
    /// The zero image a C `static` began with; [`KmemCache::init`] completes
    /// it.
    pub(crate) const fn zeroed() -> Self {
        Self {
            lock: SimpleLock::new(),
            node: List::unlinked(),
            partial_slabs: List::unlinked(),
            free_slabs: List::unlinked(),
            active_slabs: Rbtree::new(),
            flags: CacheFlags(0),
            bufctl_dist: 0,
            slab_size: 0,
            bufs_per_slab: 0,
            nr_objs: 0,
            nr_free_slabs: 0,
            ctor: None,
            obj_size: 0,
            align: 0,
            buf_size: 0,
            color: 0,
            color_max: 0,
            nr_bufs: 0,
            nr_slabs: 0,
            name: [0; KMEM_CACHE_NAME_SIZE],
            buftag_dist: 0,
            redzone_pad: 0,
        }
    }

    /// `kmem_cache_init()` in C.
    pub(crate) fn init(
        &mut self,
        name: &[u8],
        obj_size: usize,
        align: usize,
        ctor: KmemCacheCtor,
        flags: CacheInitFlags,
    ) {
        self.flags = CacheFlags(0);

        if flags.contains(CacheInitFlags::VERIFY) {
            self.flags.insert(CacheFlags::VERIFY);
        }

        let align = align.max(KMEM_ALIGN_MIN);
        let mut buf_size = round_up(obj_size, align);

        self.lock.init();
        // SAFETY: this call builds the cache, so no list holds its nodes.
        unsafe {
            List::init_head_at(NonNull::from(&mut self.node));
            List::init_head_at(NonNull::from(&mut self.partial_slabs));
            List::init_head_at(NonNull::from(&mut self.free_slabs));
        }
        self.active_slabs.init();
        self.obj_size = obj_size;
        self.align = align;
        self.buf_size = buf_size;
        self.bufctl_dist = buf_size - size_of::<KmemBufctl>();
        self.color = 0;
        self.nr_objs = 0;
        self.nr_bufs = 0;
        self.nr_slabs = 0;
        self.nr_free_slabs = 0;
        self.ctor = ctor;
        self.name.fill(0);
        let len = name.len().min(KMEM_CACHE_NAME_SIZE - 1);
        for (dst, byte) in self.name.iter_mut().zip(&name[..len]) {
            *dst = c_char::from_ne_bytes([*byte]);
        }
        self.buftag_dist = 0;
        self.redzone_pad = 0;

        if self.flags.contains(CacheFlags::VERIFY) {
            self.bufctl_dist = buf_size;
            self.buftag_dist = self.bufctl_dist + size_of::<KmemBufctl>();
            self.redzone_pad = self.bufctl_dist - self.obj_size;
            buf_size += size_of::<KmemBufctl>() + size_of::<KmemBuftag>();
            buf_size = round_up(buf_size, align);
            self.buf_size = buf_size;
        }

        self.compute_properties(flags);

        KMEM_CACHE_LIST_LOCK.lock();
        // SAFETY: each cache is initialized once, before it can be shared,
        // and the list lock serializes the insertion.
        unsafe {
            (*cache_list()).insert_tail(NonNull::from(&mut self.node));
        }
        // `Relaxed` is enough: the list lock orders the insertion, and the
        // count is only a size hint outside the lock.
        KMEM_NR_CACHES.fetch_add(1, Ordering::Relaxed);
        KMEM_CACHE_LIST_LOCK.unlock();
    }

    /// `kmem_cache_compute_properties()` in C.
    fn compute_properties(&mut self, flags: CacheInitFlags) {
        let flags = if self.buf_size < KMEM_BUF_SIZE_THRESHOLD {
            flags | CacheInitFlags::NOOFFSLAB
        } else {
            flags
        };

        let mut slab_size = PAGE_SIZE;
        let mut embed;

        loop {
            if flags.contains(CacheInitFlags::NOOFFSLAB) {
                embed = true;
            } else {
                let waste = slab_size % self.buf_size;
                embed = size_of::<KmemSlab>() <= waste;
            }

            let mut size = slab_size;

            if embed {
                size -= size_of::<KmemSlab>();
            }

            if size >= self.buf_size {
                self.slab_size = slab_size;
                // The C divided two `size_t`s into a `long_natural_t`; both
                // are `unsigned long` on the two targets.
                self.bufs_per_slab = (size / self.buf_size) as c_ulong;
                self.color_max = size % self.buf_size;
                break;
            }

            slab_size += PAGE_SIZE;
        }

        if self.color_max >= PAGE_SIZE {
            self.color_max = 0;
        }

        if !embed {
            self.flags.insert(CacheFlags::SLAB_EXTERNAL);
        }

        if flags.contains(CacheInitFlags::PHYSMEM)
            || self.slab_size == PAGE_SIZE
        {
            self.flags.insert(CacheFlags::PHYSMEM);

            if self.slab_size != PAGE_SIZE {
                // SAFETY: `Panic` does not return; the file, line, function
                // and message are the C `panic()` call's.
                unsafe {
                    glue::Panic(
                        c"rust/src/kern/slab.rs".as_ptr(),
                        line!() as c_int,
                        c"kmem_cache_compute_properties".as_ptr(),
                        c"slab: invalid cache parameters".as_ptr(),
                    )
                };
            }
        }

        if self.flags.contains(CacheFlags::VERIFY) {
            self.flags.insert(CacheFlags::USE_TREE);
        }

        if self.flags.contains(CacheFlags::SLAB_EXTERNAL) {
            if self.flags.contains(CacheFlags::PHYSMEM) {
                self.flags.insert(CacheFlags::USE_PAGE);
            } else {
                self.flags.insert(CacheFlags::USE_TREE);
            }
        } else if self.slab_size == PAGE_SIZE {
            self.flags.insert(CacheFlags::DIRECT);
        } else {
            self.flags.insert(CacheFlags::USE_TREE);
        }
    }

    /// `kmem_buf_to_bufctl()` in C.
    fn bufctl_of(&self, buf: *mut u8) -> *mut KmemBufctl {
        // SAFETY: the bufctl of a buffer of this cache lies inside the
        // buffer's `buf_size` bytes.
        unsafe { buf.add(self.bufctl_dist).cast() }
    }

    /// `kmem_buf_to_buftag()` in C.
    fn buftag_of(&self, buf: *mut u8) -> *mut KmemBuftag {
        // SAFETY: the buftag of a buffer of this cache lies inside the
        // buffer's `buf_size` bytes.
        unsafe { buf.add(self.buftag_dist).cast() }
    }

    /// `kmem_bufctl_to_buf()` in C.
    fn buf_of(&self, bufctl: *mut KmemBufctl) -> NonNull<u8> {
        // SAFETY: the bufctl lies inside a buffer of this cache, so the
        // subtraction stays inside that allocation.
        unsafe {
            NonNull::new_unchecked(bufctl.cast::<u8>().sub(self.bufctl_dist))
        }
    }

    /// `kmem_cache_empty()` in C.
    fn is_empty(&self) -> bool {
        self.nr_objs == self.nr_bufs
    }

    /// `kmem_cache_alloc()` in C.
    pub(crate) fn alloc(&mut self) -> Option<NonNull<u8>> {
        loop {
            self.lock.lock();
            let buf = self.alloc_from_slab();
            self.lock.unlock();

            let Some(buf) = buf else {
                if !self.grow() {
                    return None;
                }
                continue;
            };

            if self.flags.contains(CacheFlags::VERIFY) {
                self.alloc_verify(buf);
            }

            if let Some(ctor) = self.ctor {
                // SAFETY: the constructor contract is the C typedef's: it
                // builds the object in place and never fails.
                unsafe { ctor(buf.as_ptr().cast()) };
            }

            return Some(buf);
        }
    }

    /// `kmem_cache_alloc_from_slab()` in C; the cache lock must be held.
    fn alloc_from_slab(&mut self) -> Option<NonNull<u8>> {
        let node = if !self.partial_slabs.is_empty() {
            self.partial_slabs.first()
        } else if !self.free_slabs.is_empty() {
            self.free_slabs.first()
        } else {
            None
        }?;

        // SAFETY: the node is linked, so it is the `list_node` of a live
        // slab, and the cache lock serializes its fields.
        let slab = unsafe { &mut *slab_from_list_node(node).as_ptr() };

        let bufctl = slab.first_free;
        // SAFETY: a listed slab has a free-buffer chain, so `first_free` is
        // a live bufctl.
        slab.first_free = unsafe { (*bufctl).next };
        slab.nr_refs += 1;
        self.nr_objs += 1;

        if slab.nr_refs == self.bufs_per_slab {
            // SAFETY: the slab was on one of the cache's lists.
            unsafe { List::remove(NonNull::from(&mut slab.list_node)) };

            if slab.nr_refs == 1 {
                self.nr_free_slabs -= 1;
            }
        } else if slab.nr_refs == 1 {
            // SAFETY: as above; the slab becomes partial, and inserting it
            // at the tail keeps the lists consistent.
            unsafe {
                List::remove(NonNull::from(&mut slab.list_node));
                self.partial_slabs
                    .insert_tail(NonNull::from(&mut slab.list_node));
            }
            self.nr_free_slabs -= 1;
        }

        if slab.nr_refs == 1 && self.flags.contains(CacheFlags::USE_TREE) {
            // SAFETY: `Slab::create` initialized the node, which is not in
            // the tree yet, and the slab is the one the lookup is keyed by.
            unsafe {
                self.active_slabs.insert_by(
                    NonNull::from(&mut slab.tree_node),
                    |visited| cmp_lookup(slab.addr.addr(), visited),
                );
            }
        }

        Some(self.buf_of(bufctl))
    }

    /// `kmem_cache_grow()` in C.
    fn grow(&mut self) -> bool {
        self.lock.lock();

        if !self.is_empty() {
            self.lock.unlock();
            return true;
        }

        let color = self.color;
        self.color += self.align;

        if self.color > self.color_max {
            self.color = 0;
        }

        self.lock.unlock();

        let slab = KmemSlab::create(self, color);

        self.lock.lock();

        if let Some(slab) = slab {
            // SAFETY: the fresh slab's list node is not on any list.
            unsafe {
                self.free_slabs.insert_head(NonNull::from(
                    &mut (*slab.as_ptr()).list_node,
                ));
            }
            self.nr_bufs += self.bufs_per_slab;
            self.nr_slabs += 1;
            self.nr_free_slabs += 1;
        }

        let empty = self.is_empty();

        self.lock.unlock();

        !empty
    }

    /// `kmem_cache_free()` in C.
    ///
    /// # Safety
    ///
    /// `obj` must be a live allocation from this cache that nothing uses.
    pub(crate) unsafe fn free(&mut self, obj: NonNull<u8>) {
        if self.flags.contains(CacheFlags::VERIFY) {
            // SAFETY: the caller's contract.
            unsafe { self.free_verify(obj) };
        }

        self.lock.lock();
        // SAFETY: the caller's contract.
        unsafe { self.free_to_slab(obj) };
        self.lock.unlock();
    }

    /// `kmem_cache_free_to_slab()` in C; the cache lock must be held.
    ///
    /// # Safety
    ///
    /// `buf` must be a live allocation from this cache.
    unsafe fn free_to_slab(&mut self, buf: NonNull<u8>) {
        let slab = if self.flags.contains(CacheFlags::DIRECT) {
            // SAFETY: a direct-mapped slab sits at the end of the page range
            // containing the buffer.
            unsafe { slab_from_direct(buf, self.slab_size) }
        } else if self.flags.contains(CacheFlags::USE_PAGE) {
            // SAFETY: the buffer's page carries the slab in its private
            // field.
            let page = unsafe {
                glue::vm_page_lookup_pa(kvtophys(buf.as_ptr().addr()))
            };
            let Some(page) = NonNull::new(page) else {
                self.error(buf.as_ptr(), CacheError::Invalid, ptr::null_mut());
            };
            // SAFETY: the page is live and this cache owns its private
            // field.
            unsafe { (*page.as_ptr()).priv_ }.cast::<KmemSlab>()
        } else {
            let node = self.active_slabs.lookup_nearest(
                |node| cmp_lookup(buf.as_ptr().addr(), node),
                RBTREE_LEFT,
            );
            let Some(node) = node else {
                self.error(buf.as_ptr(), CacheError::Invalid, ptr::null_mut());
            };
            // SAFETY: the node is an active slab of this cache.
            unsafe { slab_from_tree_node(node).as_ptr() }
        };

        // SAFETY: the slab and the bufctl are live, and the cache lock
        // serializes them.
        unsafe {
            let bufctl = self.bufctl_of(buf.as_ptr());
            (*bufctl).next = (*slab).first_free;
            (*slab).first_free = bufctl;
            (*slab).nr_refs -= 1;
        }
        self.nr_objs -= 1;

        // SAFETY: the slab fields and list nodes are this cache's, and the
        // lock serializes them.
        unsafe {
            if (*slab).nr_refs == 0 {
                if self.flags.contains(CacheFlags::USE_TREE) {
                    self.active_slabs
                        .remove_node(NonNull::from(&mut (*slab).tree_node));
                }

                if self.bufs_per_slab > 1 {
                    List::remove(NonNull::from(&mut (*slab).list_node));
                }

                self.free_slabs
                    .insert_head(NonNull::from(&mut (*slab).list_node));
                self.nr_free_slabs += 1;
            } else if (*slab).nr_refs == self.bufs_per_slab - 1 {
                self.partial_slabs
                    .insert_head(NonNull::from(&mut (*slab).list_node));
            }
        }
    }

    /// `kmem_cache_reap()` in C.
    fn reap(&mut self, dead_slabs: &mut List) {
        self.lock.lock();

        // SAFETY: both heads are valid, and the nodes of `free_slabs` move
        // to `dead_slabs` as the C's `list_concat()` did.
        unsafe {
            List::concat(
                NonNull::from(dead_slabs),
                NonNull::from(&mut self.free_slabs),
            );
            List::init_head_at(NonNull::from(&mut self.free_slabs));
        }

        self.nr_bufs -= self.bufs_per_slab * self.nr_free_slabs;
        self.nr_slabs -= self.nr_free_slabs;
        self.nr_free_slabs = 0;

        self.lock.unlock();
    }

    /// `kmem_cache_alloc_verify()` in C.
    ///
    /// The C's `construct` argument was `KMEM_AV_NOCONSTRUCT` at its only
    /// call site, so [`KmemCache::alloc`] calls the constructor itself.
    fn alloc_verify(&mut self, buf: NonNull<u8>) {
        let buftag = self.buftag_of(buf.as_ptr());

        // SAFETY: the buffer is live and holds its buftag.
        if unsafe { (*buftag).state } != KMEM_BUFTAG_FREE {
            self.error(buf.as_ptr(), CacheError::Buftag, buftag.cast());
        }

        // SAFETY: the buffer spans `bufctl_dist` readable and writable
        // bytes.
        let modified = unsafe {
            verify_fill(
                buf.as_ptr(),
                KMEM_FREE_PATTERN,
                KMEM_UNINIT_PATTERN,
                self.bufctl_dist,
            )
        };

        if let Some(addr) = modified {
            self.error(
                buf.as_ptr(),
                CacheError::Modified,
                addr.as_ptr().cast(),
            );
        }

        // SAFETY: `obj_size` plus `redzone_pad` is `bufctl_dist`, so the
        // write stays inside the buffer.
        unsafe {
            ptr::write_bytes(
                buf.as_ptr().add(self.obj_size),
                KMEM_REDZONE_BYTE,
                self.redzone_pad,
            );
        }

        let bufctl = self.bufctl_of(buf.as_ptr());
        // SAFETY: the buffer is live, so its bufctl and buftag are too.
        unsafe {
            (*bufctl).redzone = KMEM_REDZONE_WORD;
            (*buftag).state = KMEM_BUFTAG_ALLOC;
        }
    }

    /// `kmem_cache_free_verify()` in C.
    ///
    /// # Safety
    ///
    /// `buf` must be a live allocation from this verify cache.
    unsafe fn free_verify(&mut self, buf: NonNull<u8>) {
        self.lock.lock();
        let node = self.active_slabs.lookup_nearest(
            |node| cmp_lookup(buf.as_ptr().addr(), node),
            RBTREE_LEFT,
        );
        self.lock.unlock();

        let Some(node) = node else {
            self.error(buf.as_ptr(), CacheError::Invalid, ptr::null_mut());
        };

        // SAFETY: the node is an active slab of this cache.
        let slab = unsafe { &*slab_from_tree_node(node).as_ptr() };
        let slabend =
            slab.addr.addr().wrapping_add(self.slab_size) & !(PAGE_SIZE - 1);

        if buf.as_ptr().addr() >= slabend {
            self.error(buf.as_ptr(), CacheError::Invalid, ptr::null_mut());
        }

        let offset = buf.as_ptr().addr() - slab.addr.addr();

        if !offset.is_multiple_of(self.buf_size) {
            self.error(buf.as_ptr(), CacheError::Invalid, ptr::null_mut());
        }

        let buftag = self.buftag_of(buf.as_ptr());
        // SAFETY: the buffer is valid, so its buftag is too.
        let state = unsafe { (*buftag).state };

        if state != KMEM_BUFTAG_ALLOC {
            if state == KMEM_BUFTAG_FREE {
                self.error(
                    buf.as_ptr(),
                    CacheError::DoubleFree,
                    ptr::null_mut(),
                );
            }

            self.error(buf.as_ptr(), CacheError::Buftag, buftag.cast());
        }

        let bufctl = self.bufctl_of(buf.as_ptr());
        // SAFETY: `obj_size` is inside the buffer.
        let mut redzone_byte = unsafe { buf.as_ptr().add(self.obj_size) };

        while redzone_byte.cast::<KmemBufctl>() < bufctl {
            // SAFETY: the range between the object and its bufctl is inside
            // the buffer.
            if unsafe { *redzone_byte } != KMEM_REDZONE_BYTE {
                self.error(
                    buf.as_ptr(),
                    CacheError::Redzone,
                    redzone_byte.cast(),
                );
            }
            // SAFETY: as above.
            redzone_byte = unsafe { redzone_byte.add(1) };
        }

        // SAFETY: the bufctl is live.
        let redzone = unsafe { (*bufctl).redzone };

        if redzone != KMEM_REDZONE_WORD {
            let word = KMEM_REDZONE_WORD;
            if let Some(addr) = unsafe {
                verify_bytes(
                    ptr::from_ref(&redzone).cast(),
                    &word.to_ne_bytes(),
                )
            } {
                self.error(
                    buf.as_ptr(),
                    CacheError::Redzone,
                    addr.as_ptr().cast(),
                );
            }
        }

        // SAFETY: the buffer's bytes are live and writable.
        unsafe { fill(buf.as_ptr(), KMEM_FREE_PATTERN, self.bufctl_dist) };
        // SAFETY: the buftag is the buffer's.
        unsafe { (*buftag).state = KMEM_BUFTAG_FREE };
    }

    /// `kmem_cache_error()` in C: report and halt.
    fn error(&self, buf: *mut u8, error: CacheError, arg: *mut c_void) -> ! {
        // SAFETY: the name is NUL-terminated by `init()`, and the format is
        // the C's with the arguments its conversions read.
        unsafe {
            glue::printf(
                c"mem: warning: kmem_cache_error(): cache: %s, buffer: %p\n"
                    .as_ptr(),
                self.name.as_ptr(),
                buf,
            );
        }

        // The C's `%td` reads a ptrdiff_t; the offset is inside one buffer,
        // far below `isize::MAX`.
        let offset = arg.addr().wrapping_sub(buf.addr());

        match error {
            CacheError::Invalid => unsafe {
                glue::Panic(
                    c"rust/src/kern/slab.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_cache_error".as_ptr(),
                    c"mem: error: kmem_cache_error(): freeing invalid address\n"
                        .as_ptr(),
                )
            },
            CacheError::DoubleFree => unsafe {
                glue::Panic(
                    c"rust/src/kern/slab.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_cache_error".as_ptr(),
                    c"mem: error: kmem_cache_error(): attempting to free the same address twice\n"
                        .as_ptr(),
                )
            },
            CacheError::Buftag => unsafe {
                glue::Panic(
                    c"rust/src/kern/slab.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_cache_error".as_ptr(),
                    c"mem: error: kmem_cache_error(): invalid buftag content, buftag state: %p\n"
                        .as_ptr(),
                    arg,
                )
            },
            CacheError::Modified => unsafe {
                glue::Panic(
                    c"rust/src/kern/slab.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_cache_error".as_ptr(),
                    c"mem: error: kmem_cache_error(): free buffer modified, fault address: %p, offset in buffer: %td\n"
                        .as_ptr(),
                    arg,
                    offset as isize,
                )
            },
            CacheError::Redzone => unsafe {
                glue::Panic(
                    c"rust/src/kern/slab.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_cache_error".as_ptr(),
                    c"mem: error: kmem_cache_error(): write beyond end of buffer, fault address: %p, offset in buffer: %td\n"
                        .as_ptr(),
                    arg,
                    offset as isize,
                )
            },
        }
    }

    /// The per-cache copy `host_slab_info()` makes.
    fn fill_info(&mut self, info: &mut CacheInfo) {
        self.lock.lock();
        info.flags = self.flags.0;
        info.cpu_pool_size = 0;
        info.obj_size = self.obj_size;
        info.align = self.align;
        info.buf_size = self.buf_size;
        info.slab_size = self.slab_size;
        info.bufs_per_slab = self.bufs_per_slab;
        info.nr_objs = self.nr_objs;
        info.nr_bufs = self.nr_bufs;
        info.nr_slabs = self.nr_slabs;
        info.nr_free_slabs = self.nr_free_slabs;
        info.name.fill(0);
        let len = self
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(KMEM_CACHE_NAME_SIZE);
        for (dst, src) in info.name.iter_mut().zip(&self.name[..len]) {
            *dst = *src;
        }
        self.lock.unlock();
    }
}

impl KmemSlab {
    /// `kmem_slab_create()` in C; the caller holds no cache lock.
    fn create(
        cache: &mut KmemCache,
        color: usize,
    ) -> Option<NonNull<KmemSlab>> {
        let slab_buf = pagealloc(cache.slab_size, cache.align, cache.flags)?;

        let slab = if cache.flags.contains(CacheFlags::SLAB_EXTERNAL) {
            // SAFETY: the slab cache was initialized before any cache could
            // grow.
            let external = unsafe { (*slab_cache()).alloc() };
            let Some(external) = external else {
                // SAFETY: the buffer is a live page allocation of this
                // cache.
                unsafe {
                    pagefree(
                        slab_buf.as_ptr().addr(),
                        cache.slab_size,
                        cache.flags,
                    )
                };
                return None;
            };

            if cache.flags.contains(CacheFlags::USE_PAGE) {
                let page = unsafe {
                    glue::vm_page_lookup_pa(kvtophys(slab_buf.as_ptr().addr()))
                };
                let Some(page) = NonNull::new(page) else {
                    // SAFETY: `Panic` does not return.
                    unsafe {
                        glue::Panic(
                            c"rust/src/kern/slab.rs".as_ptr(),
                            line!() as c_int,
                            c"kmem_slab_create".as_ptr(),
                            c"kmem_slab_create: missing page".as_ptr(),
                        )
                    }
                };
                // SAFETY: the page is live and this cache owns its private
                // field.
                unsafe { (*page.as_ptr()).priv_ = external.as_ptr().cast() };
            }

            external.cast::<KmemSlab>()
        } else {
            // SAFETY: the slab trailer fits the buffer, as
            // `compute_properties()` established.
            unsafe {
                NonNull::new_unchecked(with_exposed_provenance_mut(
                    slab_buf.as_ptr().addr() + cache.slab_size
                        - size_of::<KmemSlab>(),
                ))
            }
        };

        // SAFETY: the slab storage is fresh, so its fields are initialized
        // here before anything links it.
        unsafe {
            (*slab.as_ptr()).cache = cache;
            List::init_head_at(NonNull::from(&mut (*slab.as_ptr()).list_node));
            rbtree_node_init(addr_of_mut!((*slab.as_ptr()).tree_node));
            (*slab.as_ptr()).nr_refs = 0;
            (*slab.as_ptr()).first_free = ptr::null_mut();
            (*slab.as_ptr()).addr = slab_buf.as_ptr().add(color);
        }

        // SAFETY: `addr` is the buffer base the C used.
        let mut bufctl = cache.bufctl_of(unsafe { (*slab.as_ptr()).addr });

        for _ in 0..cache.bufs_per_slab {
            // SAFETY: each bufctl is inside the buffer, and the chain links
            // every buffer of the slab.
            unsafe {
                (*bufctl).next = (*slab.as_ptr()).first_free;
                (*slab.as_ptr()).first_free = bufctl;
                bufctl = bufctl.cast::<u8>().add(cache.buf_size).cast();
            }
        }

        if cache.flags.contains(CacheFlags::VERIFY) {
            // SAFETY: the slab is fresh and complete.
            unsafe { KmemSlab::create_verify(slab, cache) };
        }

        Some(slab)
    }

    /// `kmem_slab_create_verify()` in C.
    ///
    /// # Safety
    ///
    /// `slab` must be a fresh slab of `cache`.
    unsafe fn create_verify(slab: NonNull<KmemSlab>, cache: &KmemCache) {
        let buf_size = cache.buf_size;
        // SAFETY: the caller promises a live slab.
        let mut buf = unsafe { (*slab.as_ptr()).addr };
        let mut buftag = cache.buftag_of(buf);

        for _ in 0..cache.bufs_per_slab {
            // SAFETY: `buf` is inside the slab, and `bufctl_dist` bytes are
            // writable from it.
            unsafe {
                fill(buf, KMEM_FREE_PATTERN, cache.bufctl_dist);
                (*buftag).state = KMEM_BUFTAG_FREE;
                buf = buf.add(buf_size);
            }
            buftag = cache.buftag_of(buf);
        }
    }

    /// `kmem_slab_destroy()` in C; the caller holds no cache lock.
    ///
    /// # Safety
    ///
    /// `slab` must be off every list, with nothing holding it.
    unsafe fn destroy(slab: NonNull<KmemSlab>, cache: &mut KmemCache) {
        if cache.flags.contains(CacheFlags::VERIFY) {
            // SAFETY: the caller promises a dead slab of this cache.
            unsafe { KmemSlab::destroy_verify(slab, cache) };
        }

        // SAFETY: the caller promises a live slab.
        let slab_buf =
            unsafe { (*slab.as_ptr()).addr.addr() } & !(PAGE_SIZE - 1);

        if cache.flags.contains(CacheFlags::SLAB_EXTERNAL) {
            if cache.flags.contains(CacheFlags::USE_PAGE) {
                let page =
                    unsafe { glue::vm_page_lookup_pa(kvtophys(slab_buf)) };
                if let Some(page) = NonNull::new(page) {
                    // SAFETY: the page is live, and the C cleared the field.
                    unsafe { (*page.as_ptr()).priv_ = ptr::null_mut() };
                }
            }

            // SAFETY: the slab came from the off-slab cache.
            unsafe {
                (*slab_cache())
                    .free(NonNull::new_unchecked(slab.as_ptr().cast::<u8>()))
            };
        }

        // SAFETY: the slab's buffer is the one `pagealloc()` returned.
        unsafe { pagefree(slab_buf, cache.slab_size, cache.flags) };
    }

    /// `kmem_slab_destroy_verify()` in C.
    ///
    /// # Safety
    ///
    /// `slab` must be a dead slab of `cache` whose buffers are all free.
    unsafe fn destroy_verify(slab: NonNull<KmemSlab>, cache: &KmemCache) {
        let buf_size = cache.buf_size;
        // SAFETY: the caller promises a live slab.
        let mut buf = unsafe { (*slab.as_ptr()).addr };
        let mut buftag = cache.buftag_of(buf);

        for _ in 0..cache.bufs_per_slab {
            // SAFETY: the caller promises the buffers are free and intact.
            unsafe {
                if (*buftag).state != KMEM_BUFTAG_FREE {
                    cache.error(buf, CacheError::Buftag, buftag.cast());
                }

                if let Some(addr) =
                    verify(buf, KMEM_FREE_PATTERN, cache.bufctl_dist)
                {
                    cache.error(
                        buf,
                        CacheError::Modified,
                        addr.as_ptr().cast(),
                    );
                }

                buf = buf.add(buf_size);
            }
            buftag = cache.buftag_of(buf);
        }
    }
}

/// `kmem_slab_cmp_lookup()` in C.
fn cmp_lookup(addr: VmOffset, node: NonNull<RbtreeNode>) -> c_int {
    // SAFETY: the tree only visits its own slabs' nodes.
    let slab = unsafe { slab_from_tree_node(node) };
    // SAFETY: the slab is live under the cache lock.
    let slab_addr = unsafe { (*slab.as_ptr()).addr.addr() };

    if addr == slab_addr {
        0
    } else if addr < slab_addr {
        -1
    } else {
        1
    }
}

/// The slab a list node belongs to.
///
/// # Safety
///
/// `node` must be the `list_node` of a live slab.
unsafe fn slab_from_list_node(node: NonNull<List>) -> NonNull<KmemSlab> {
    // SAFETY: the caller promises the field address.
    unsafe {
        NonNull::new_unchecked(
            node.as_ptr()
                .cast::<u8>()
                .sub(offset_of!(KmemSlab, list_node))
                .cast(),
        )
    }
}

/// The slab a tree node belongs to.
///
/// # Safety
///
/// `node` must be the `tree_node` of a live slab.
unsafe fn slab_from_tree_node(node: NonNull<RbtreeNode>) -> NonNull<KmemSlab> {
    // SAFETY: the caller promises the field address.
    unsafe {
        NonNull::new_unchecked(
            node.as_ptr()
                .cast::<u8>()
                .sub(offset_of!(KmemSlab, tree_node))
                .cast(),
        )
    }
}

/// The slab at the end of the `slab_size` block holding `buf`, the C's
/// `P2END(buf, slab_size) - 1` for a direct-mapped cache.
///
/// # Safety
///
/// `buf` must come from a direct-mapped slab of `slab_size` bytes.
unsafe fn slab_from_direct(
    buf: NonNull<u8>,
    slab_size: usize,
) -> *mut KmemSlab {
    let end = buf.as_ptr().addr() & !(slab_size - 1);
    // The caller promises the slab trailer is at the block's end.
    with_exposed_provenance_mut(end + slab_size - size_of::<KmemSlab>())
}

/// `kmem_buf_verify_bytes()` in C.
///
/// # Safety
///
/// `buf` must hold `pattern.len()` readable bytes.
unsafe fn verify_bytes(buf: *const u8, pattern: &[u8]) -> Option<NonNull<u8>> {
    for (i, byte) in pattern.iter().enumerate() {
        // SAFETY: the caller promises the range.
        if unsafe { *buf.add(i) } != *byte {
            // SAFETY: the mismatch byte is inside the range.
            return Some(unsafe {
                NonNull::new_unchecked(buf.add(i) as *mut u8)
            });
        }
    }

    None
}

/// `kmem_buf_verify()` in C.
///
/// # Safety
///
/// `buf` must hold `size` readable bytes.
unsafe fn verify(
    buf: *const u8,
    pattern: u64,
    size: usize,
) -> Option<NonNull<u8>> {
    let end = buf.wrapping_add(size);
    let mut ptr = buf.cast::<u64>();

    while ptr.cast::<u8>() < end {
        // SAFETY: the caller promises `size` readable bytes, and the loop
        // stays below `end`.
        if unsafe { ptr.read() } != pattern {
            // SAFETY: as above.
            return unsafe {
                verify_bytes(ptr.cast(), &pattern.to_ne_bytes())
            };
        }
        // SAFETY: as above.
        ptr = unsafe { ptr.add(1) };
    }

    None
}

/// `kmem_buf_fill()` in C.
///
/// # Safety
///
/// `buf` must hold `size` writable bytes.
unsafe fn fill(buf: *mut u8, pattern: u64, size: usize) {
    let end = buf.wrapping_add(size);
    let mut ptr = buf.cast::<u64>();

    while ptr.cast::<u8>() < end {
        // SAFETY: the caller promises `size` writable bytes, and the loop
        // stays below `end`.
        unsafe { ptr.write(pattern) };
        // SAFETY: as above.
        ptr = unsafe { ptr.add(1) };
    }
}

/// `kmem_buf_verify_fill()` in C.
///
/// # Safety
///
/// `buf` must hold `size` readable and writable bytes.
unsafe fn verify_fill(
    buf: *mut u8,
    old: u64,
    new: u64,
    size: usize,
) -> Option<NonNull<u8>> {
    let end = buf.wrapping_add(size);
    let mut ptr = buf.cast::<u64>();

    while ptr.cast::<u8>() < end {
        // SAFETY: the caller promises `size` readable and writable bytes,
        // and the loop stays below `end`.
        if unsafe { ptr.read() } != old {
            // SAFETY: as above.
            return unsafe { verify_bytes(ptr.cast(), &old.to_ne_bytes()) };
        }
        // SAFETY: as above.
        unsafe { ptr.write(new) };
        // SAFETY: as above.
        ptr = unsafe { ptr.add(1) };
    }

    None
}

/// `kmem_pagealloc_physmem()` in C: a direct-mapped page, blocking until one
/// is free.
fn pagealloc_physmem(_size: VmSize) -> NonNull<u8> {
    loop {
        // SAFETY: no cache lock is held, so the allocation may block on the
        // page queues.
        let page = unsafe { vm_resident::grab(VM_PAGE_DIRECTMAP) };

        if let Some(page) = page {
            // SAFETY: the page is live and direct-mapped.
            let addr = unsafe { (*page.as_ptr()).phys_addr };
            return unsafe {
                NonNull::new_unchecked(with_exposed_provenance_mut(
                    VM_MIN_KERNEL_ADDRESS.wrapping_add(addr),
                ))
            };
        }

        // SAFETY: the continuation is the C's `NULL`.
        unsafe { glue::vm_page_wait(None) };
    }
}

/// `kmem_pagealloc_virtual()` in C.
fn pagealloc_virtual(size: VmSize, align: VmSize) -> Option<NonNull<u8>> {
    let size = round_page(size);
    // SAFETY: `kernel_map` is the live kernel map.
    let map =
        unsafe { NonNull::new_unchecked(glue::kernel_map.cast::<VmMap>()) };

    let addr = if align <= PAGE_SIZE {
        vm_kern::kmem_alloc_wired(map, size).ok()?
    } else {
        let mut addr: VmOffset = 0;
        // SAFETY: the out-pointer is a live local and the map is unlocked.
        let kr = unsafe {
            glue::kmem_alloc_aligned(glue::kernel_map, &mut addr, size)
        };

        if kr != KERN_SUCCESS {
            return None;
        }

        addr
    };

    // SAFETY: a successful allocation is a non-null kernel address.
    Some(unsafe { NonNull::new_unchecked(with_exposed_provenance_mut(addr)) })
}

/// `kmem_pagefree_physmem()` in C.
///
/// # Safety
///
/// `addr` must be a page [`pagealloc_physmem`] returned.
unsafe fn pagefree_physmem(addr: VmOffset, _size: VmSize) {
    // SAFETY: the caller promises the page was allocated here, so the lookup
    // finds it.
    let page = unsafe { glue::vm_page_lookup_pa(kvtophys(addr)) };
    let Some(page) = NonNull::new(page) else {
        // SAFETY: `Panic` does not return.
        unsafe {
            glue::Panic(
                c"rust/src/kern/slab.rs".as_ptr(),
                line!() as c_int,
                c"kmem_pagefree_physmem".as_ptr(),
                c"kmem_pagefree_physmem: missing page".as_ptr(),
            )
        }
    };

    // SAFETY: the page is live, and the release takes the page-queues lock
    // itself.
    unsafe { vm_resident::release(page, false, false) };
}

/// `kmem_pagefree_virtual()` in C.
///
/// # Safety
///
/// `addr..addr + size` must be a region [`pagealloc_virtual`] returned.
unsafe fn pagefree_virtual(addr: VmOffset, size: VmSize) {
    // SAFETY: the boot globals are live for the kernel's lifetime.
    let start = unsafe { glue::kernel_virtual_start };
    // SAFETY: as above.
    let end = unsafe { glue::kernel_virtual_end };

    if addr < start || addr.wrapping_add(size) > end {
        // SAFETY: `Panic` does not return; the format and its two arguments
        // are the C's.
        unsafe {
            glue::Panic(
                c"rust/src/kern/slab.rs".as_ptr(),
                line!() as c_int,
                c"kmem_pagefree_virtual".as_ptr(),
                c"kmem_pagefree_virtual(%lx-%lx) falls in physical memory area!\n"
                    .as_ptr(),
                addr,
                addr.wrapping_add(size),
            )
        };
    }

    let size = round_page(size);
    // SAFETY: `kernel_map` is the live kernel map.
    let map = unsafe { &mut *glue::kernel_map.cast::<VmMap>() };

    if vm_kern::kmem_free(map, addr, size).is_err() {
        // SAFETY: `Panic` does not return; the C `kmem_free()` panicked with
        // this message on failure.
        unsafe {
            glue::Panic(
                c"rust/src/kern/slab.rs".as_ptr(),
                line!() as c_int,
                c"kmem_free".as_ptr(),
                c"kmem_free".as_ptr(),
            )
        };
    }
}

/// `kmem_pagealloc()` in C.
fn pagealloc(
    size: VmSize,
    align: VmSize,
    flags: CacheFlags,
) -> Option<NonNull<u8>> {
    if flags.contains(CacheFlags::PHYSMEM) {
        Some(pagealloc_physmem(size))
    } else {
        pagealloc_virtual(size, align)
    }
}

/// `kmem_pagefree()` in C.
///
/// # Safety
///
/// `addr` must be a region the matching [`pagealloc`] returned.
unsafe fn pagefree(addr: VmOffset, size: VmSize, flags: CacheFlags) {
    if flags.contains(CacheFlags::PHYSMEM) {
        // SAFETY: the caller promises a physical-page allocation.
        unsafe { pagefree_physmem(addr, size) };
    } else {
        // SAFETY: the caller promises a virtual allocation.
        unsafe { pagefree_virtual(addr, size) };
    }
}

/// `kalloc_get_index()` in C; the caller passes a size greater than zero.
fn kalloc_index(size: usize) -> usize {
    let size = (size - 1) >> KALLOC_FIRST_SHIFT;

    if size == 0 {
        0
    } else {
        // Both operands are `u32`s and `usize` is at least as wide.
        (usize::BITS - size.leading_zeros()) as usize
    }
}

/// The `index`th general-purpose cache.
fn kalloc_cache(index: usize) -> *mut KmemCache {
    // SAFETY: the array is a static that never moves.
    let caches: *mut KmemCache = KALLOC_CACHES.0.get().cast();
    // SAFETY: every caller bounds-checks `index` against `KALLOC_NR_CACHES`.
    unsafe { caches.add(index) }
}

/// `kalloc_verify()` in C.
///
/// # Safety
///
/// `buf` must hold `cache.obj_size` writable bytes.
unsafe fn kalloc_verify(cache: &KmemCache, buf: NonNull<u8>, size: usize) {
    let redzone = buf.as_ptr().wrapping_add(size);
    let redzone_size = cache.obj_size - size;
    // SAFETY: the caller promises the object spans `obj_size` bytes.
    unsafe { ptr::write_bytes(redzone, KMEM_REDZONE_BYTE, redzone_size) };
}

/// `kfree_verify()` in C.
///
/// # Safety
///
/// `buf` must hold `cache.obj_size` readable bytes previously filled with
/// [`KMEM_REDZONE_BYTE`] past `size`.
unsafe fn kfree_verify(cache: &KmemCache, buf: NonNull<u8>, size: usize) {
    let mut redzone_byte = buf.as_ptr().wrapping_add(size);
    let redzone_end = buf.as_ptr().wrapping_add(cache.obj_size);

    while redzone_byte < redzone_end {
        // SAFETY: the caller promises the range readable.
        if unsafe { *redzone_byte } != KMEM_REDZONE_BYTE {
            cache.error(
                buf.as_ptr(),
                CacheError::Redzone,
                redzone_byte.cast(),
            );
        }
        // SAFETY: as above.
        redzone_byte = unsafe { redzone_byte.add(1) };
    }
}

/// `kalloc()` in C.
pub(crate) fn kalloc(size: usize) -> Option<NonNull<u8>> {
    if size == 0 {
        return None;
    }

    let index = kalloc_index(size);

    if index < KALLOC_NR_CACHES {
        let cache = kalloc_cache(index);
        // SAFETY: `kalloc_init()` built every cache before any allocation,
        // and the cache lock serializes the operation.
        let buf = unsafe { (*cache).alloc() }?;

        // SAFETY: the cache is initialized and its lock is free here.
        if unsafe { (*cache).flags.contains(CacheFlags::VERIFY) } {
            // SAFETY: the buffer is a live allocation of the cache's object
            // size.
            unsafe { kalloc_verify(&*cache, buf, size) };
        }

        Some(buf)
    } else if size <= PAGE_SIZE {
        Some(pagealloc_physmem(PAGE_SIZE))
    } else {
        pagealloc_virtual(size, 0)
    }
}

/// `kfree()` in C.
///
/// # Safety
///
/// `data` must be a live allocation of `size` bytes from [`kalloc`] that
/// nothing uses.
pub(crate) unsafe fn kfree(data: NonNull<u8>, size: usize) {
    if size == 0 {
        return;
    }

    let index = kalloc_index(size);

    if index < KALLOC_NR_CACHES {
        let cache = kalloc_cache(index);

        // SAFETY: the cache is initialized and its lock is free here.
        if unsafe { (*cache).flags.contains(CacheFlags::VERIFY) } {
            // SAFETY: the caller promises the live allocation.
            unsafe { kfree_verify(&*cache, data, size) };
        }

        // SAFETY: the caller promises a live allocation from this cache.
        unsafe { (*cache).free(data) };
    } else if size <= PAGE_SIZE {
        // SAFETY: the allocator returned a direct-mapped page for it.
        unsafe { pagefree_physmem(data.as_ptr().addr(), PAGE_SIZE) };
    } else {
        // SAFETY: as above, from the virtual allocator.
        unsafe { pagefree_virtual(data.as_ptr().addr(), size) };
    }
}

/// `slab_bootstrap()` in C.
pub(crate) fn slab_bootstrap() {
    // SAFETY: the global head is not linked into any list.
    unsafe { List::init_head_at(NonNull::from(&mut *cache_list())) };
    KMEM_CACHE_LIST_LOCK.init();
}

/// `slab_init()` in C.
pub(crate) fn slab_init() {
    // SAFETY: `slab_bootstrap()` ran, and this is the off-slab cache's only
    // initializer.
    unsafe {
        (*slab_cache()).init(
            b"kmem_slab",
            size_of::<KmemSlab>(),
            0,
            None,
            CacheInitFlags::NOOFFSLAB,
        )
    };
}

/// `kalloc_init()` in C.
pub(crate) fn kalloc_init() {
    let mut size = 1 << KALLOC_FIRST_SHIFT;

    for index in 0..KALLOC_NR_CACHES {
        let name = kalloc_name(size);
        let cache = kalloc_cache(index);

        // SAFETY: `slab_init()` ran, and each cache is initialized once, in
        // order.
        unsafe {
            (*cache).init(&name, size, 0, None, CacheInitFlags::EMPTY);
        }
        size <<= 1;
    }
}

/// The `sprintf(name, "kalloc_%lu", size)` of `kalloc_init()`, in place.
fn kalloc_name(mut value: usize) -> [u8; KMEM_CACHE_NAME_SIZE] {
    const PREFIX: &[u8] = b"kalloc_";

    let mut name = [0u8; KMEM_CACHE_NAME_SIZE];
    name[..PREFIX.len()].copy_from_slice(PREFIX);

    let mut digits = [0u8; KMEM_CACHE_NAME_SIZE];
    let mut len = 0;

    loop {
        digits[len] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
        len += 1;
        if value == 0 {
            break;
        }
    }

    for i in 0..len {
        name[PREFIX.len() + i] = digits[len - 1 - i];
    }

    name
}

/// `slab_collect()` in C.
pub(crate) fn slab_collect() {
    // SAFETY: `elapsed_ticks` is the live clock global, an `unsigned long`
    // the C kept in the target's `usize`.
    let now = unsafe { mach_clock::elapsed_ticks() };
    // The C read `hz` for `KMEM_GC_INTERVAL`, an `int` that is positive
    // after the probe sets it.
    let interval =
        usize::try_from(mach_clock::hz).unwrap_or(0) * KMEM_GC_TICKS;

    if now
        <= KMEM_GC_LAST_TICK
            .load(Ordering::Relaxed)
            .wrapping_add(interval)
    {
        return;
    }

    // `Relaxed` is enough: the collector only needs the value back, and a
    // stale one merely reaps a tick early or late.
    KMEM_GC_LAST_TICK.store(now, Ordering::Relaxed);

    let mut dead_slabs = List::unlinked();
    // SAFETY: the local head is not linked into any list.
    unsafe { List::init_head_at(NonNull::from(&mut dead_slabs)) };

    KMEM_CACHE_LIST_LOCK.lock();
    for_each_cache(|cache| cache.reap(&mut dead_slabs));
    KMEM_CACHE_LIST_LOCK.unlock();

    while let Some(node) = dead_slabs.first() {
        // SAFETY: the node is a dead slab's list node.
        let slab = unsafe { slab_from_list_node(node) };
        // SAFETY: the node is linked in `dead_slabs`.
        unsafe { List::remove(node) };

        // SAFETY: the slab is off every list and nothing holds it.
        unsafe { KmemSlab::destroy(slab, &mut *(*slab.as_ptr()).cache) };
    }
}

/// `slab_info()` in C.
pub(crate) fn slab_info() {
    /// The header `_slab_info()` prints.
    const HEADER: &core::ffi::CStr = c"cache                         obj slab  bufs   objs   bufs    total reclaimable\nname                 flags   size size /slab  usage  count   memory      memory\n";
    /// The per-cache row, the C's format exactly.
    const ROW: &core::ffi::CStr =
        c"%-20s %04x %7lu %3luk  %4lu %6lu %6lu %7uk %10uk\n";
    /// The summary row.
    const TOTAL: &core::ffi::CStr = c"total: %uk, reclaimable: %uk\n";

    // SAFETY: the format is a literal with no arguments.
    unsafe { glue::printf(HEADER.as_ptr()) };

    KMEM_CACHE_LIST_LOCK.lock();

    let mut mem_total: c_ulong = 0;
    let mut mem_total_reclaimable: c_ulong = 0;

    for_each_cache(|cache| {
        cache.lock.lock();

        // The C did this arithmetic in `unsigned long`; `c_ulong` and
        // `usize` are the same width on both targets.
        let slab_size = c_ulong::try_from(cache.slab_size).unwrap_or(0);
        let mem_usage = (cache.nr_slabs * slab_size) >> 10;
        let mem_reclaimable = (cache.nr_free_slabs * slab_size) >> 10;

        // SAFETY: every argument has the type the row format's conversion
        // reads, as in the C.
        unsafe {
            glue::printf(
                ROW.as_ptr(),
                cache.name.as_ptr(),
                cache.flags.0,
                cache.obj_size,
                cache.slab_size >> 10,
                cache.bufs_per_slab,
                cache.nr_objs,
                cache.nr_bufs,
                mem_usage,
                mem_reclaimable,
            );
        }

        cache.lock.unlock();

        mem_total += mem_usage;
        mem_total_reclaimable += mem_reclaimable;
    });

    KMEM_CACHE_LIST_LOCK.unlock();

    // SAFETY: as above, with the two totals.
    unsafe { glue::printf(TOTAL.as_ptr(), mem_total, mem_total_reclaimable) };
}

/// The number of caches on the global list, the C's unsynchronized read of
/// `kmem_nr_caches`.
pub(crate) fn nr_caches() -> u32 {
    KMEM_NR_CACHES.load(Ordering::Relaxed)
}

/// The `host_slab_info()` snapshot: fill `out` with every cache.
///
/// Returns [`None`] when the cache count changed under the caller, who
/// retries with a fresh allocation, as the C's `retry` label did.
pub(crate) fn collect(expected: u32, out: &mut [CacheInfo]) -> Option<u32> {
    if out.len() < expected as usize {
        return None;
    }

    KMEM_CACHE_LIST_LOCK.lock();

    if KMEM_NR_CACHES.load(Ordering::Relaxed) != expected {
        KMEM_CACHE_LIST_LOCK.unlock();
        return None;
    }

    let mut count = 0usize;

    for_each_cache(|cache| {
        let Some(info) = out.get_mut(count) else {
            return;
        };
        cache.fill_info(info);
        count += 1;
    });

    KMEM_CACHE_LIST_LOCK.unlock();

    // `count` never exceeds the list's length, `expected`, which is a `u32`.
    Some(count as u32)
}

/// Run `f` on every cache of the global list.
///
/// The caller must hold `KMEM_CACHE_LIST_LOCK`; no callback may remove a
/// cache from the list.
fn for_each_cache(mut f: impl FnMut(&mut KmemCache)) {
    // SAFETY: the caller holds the list lock, so the head is a valid list
    // for the walk.
    let head = unsafe { &*cache_list() };

    for node in head.iter() {
        // SAFETY: `KmemCache::init()` linked this node as the `node` field
        // of a live cache, which stays alive for the kernel's lifetime.
        let cache = unsafe {
            list_entry::<KmemCache>(node, offset_of!(KmemCache, node))
        };
        // SAFETY: as above; the caches are static storage.
        f(unsafe { &mut *cache.as_ptr() });
    }
}
