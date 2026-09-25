// SPDX-License-Identifier: CMU-Mach
// Derived from i386/intel/pmap.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The Intel physical-map module, which `i386/intel/pmap.c` used to define and
//! `i386/intel/pmap.h` declares, with the `struct pmap`, `struct pv_entry`,
//! `pmap_update_list` and `pmap_mapwindow_t` mirrors of that header.
//!
//! `#[cfg(target_arch = "x86_64")]` is the header's PAE build, with the L4
//! table, and `#[cfg(target_arch = "x86")]` its non-PAE build, with the page
//! directory alone: the two configurations the ABI gate builds.  A PAE i386
//! build would need a `--cfg` in `rust/configfrag.ac` and its own branches.

use crate::arch::i386::atomic_bits::{bit_lock, bit_unlock};
use crate::arch::i386::biosmem;
#[cfg(target_arch = "x86_64")]
use crate::arch::i386::model_dep_ffi::init_alloc_aligned;
use crate::arch::i386::model_dep_ffi::pmap_grab_page;
use crate::arch::i386::mp_desc::interrupt_processor;
use crate::arch::i386::percpu::{cpu_number, current_thread};
use crate::arch::i386::phys::kvtophys;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_SHIFT, PAGE_SIZE};
use crate::config::NCPUS;
use crate::glue;
use crate::kern::lock::{LockData, SimpleLock};
use crate::kern::machine::slot as machine_slot;
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::kern::thread::Thread;
use crate::vm::error::KERN_SUCCESS;
use crate::vm::types::{VmObject, VmProt};
use crate::vm::vm_kern::VM_MIN_KERNEL_ADDRESS;
use crate::vm::vm_map::{VmMap, round_page};
use crate::vm::vm_page;
use core::arch::asm;
use core::ffi::{c_char, c_int};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull, with_exposed_provenance_mut};
use core::sync::atomic::{AtomicI32, AtomicIsize, Ordering, fence};

/// `LINEAR_DS` of <i386/gdt.h>: the flat data selector `gdt_fill()` builds
/// with base zero, which makes an offset in it a linear address.
const LINEAR_DS: u16 = 0x38;

/// `KERNEL_DS` of <i386/gdt.h>: the kernel data selector, which the TLB
/// invalidation puts back in `%es`.
const KERNEL_DS: u16 = 0x10;

/// `INTEL_OFFMASK` of `i386/intel/pmap.h`: the offset within a page.
const INTEL_OFFMASK: VmOffset = 0xfff;

/// `INTEL_PTE_PFN`: the page-frame field of a page table entry, whose width
/// follows the `phys_addr_t` the entry is.
#[cfg(target_pointer_width = "64")]
const INTEL_PTE_PFN: VmOffset = 0xffff_ffff_ffff_f000;
#[cfg(target_pointer_width = "32")]
const INTEL_PTE_PFN: VmOffset = 0xffff_f000;

pub(crate) const INTEL_PTE_VALID: VmOffset = 0x0000_0001;
pub(crate) const INTEL_PTE_WRITE: VmOffset = 0x0000_0002;
const INTEL_PTE_USER: VmOffset = 0x0000_0004;
const INTEL_PTE_WTHRU: VmOffset = 0x0000_0008;
const INTEL_PTE_NCACHE: VmOffset = 0x0000_0010;
pub(crate) const INTEL_PTE_REF: VmOffset = 0x0000_0020;
pub(crate) const INTEL_PTE_MOD: VmOffset = 0x0000_0040;
const INTEL_PTE_GLOBAL: VmOffset = 0x0000_0100;
const INTEL_PTE_WIRED: VmOffset = 0x0000_0200;

/// `PHYS_MODIFIED` of i386/intel/pmap.c, the low byte of `INTEL_PTE_MOD`.
const PHYS_MODIFIED: u8 = INTEL_PTE_MOD as u8;
/// `PHYS_REFERENCED` of i386/intel/pmap.c, the low byte of `INTEL_PTE_REF`.
const PHYS_REFERENCED: u8 = INTEL_PTE_REF as u8;

/// `VM_PROT_READ` of <mach/vm_prot.h>, as `pmap_page_protect()` switches on
/// the raw value.
const VM_PROT_READ: c_int = 0x1;
const VM_PROT_WRITE: c_int = 0x2;
const VM_PROT_EXECUTE: c_int = 0x4;
const VM_PROT_ALL: c_int = 0x7;

/// `CPU_FEATURE_PGE` of <i386/locore.h>: the global-page bit.
pub(crate) const CPU_FEATURE_PGE: u32 = 13;
/// `CPU_FEATURE_SEP` of <i386/locore.h>: the `sysenter`/`sysexit` pair.
#[cfg(target_pointer_width = "64")]
pub(crate) const CPU_FEATURE_SEP: u32 = 11;
#[cfg(target_arch = "x86_64")]
const CPU_FEATURE_PAE: u32 = 6;

/// `CPU_TYPE_I486` of <mach/machine.h>.
const CPU_TYPE_I486: c_int = 17;

#[cfg(target_arch = "x86_64")]
const CR4_PAE: usize = 0x0020;

/// `UPDATE_LIST_SIZE` of i386/intel/pmap.c: the invalidation requests one CPU
/// can queue before the last becomes a whole-address-space flush.
const UPDATE_LIST_SIZE: usize = 4;

/// `PMAP_NMAPWINDOWS` of <i386/pmap.h>: temporary map windows per CPU.
const PMAP_NMAPWINDOWS: usize = 2;

/// `MAPWINDOW_SIZE`: the virtual space the map windows take from the kernel
/// map's tail.
const MAPWINDOW_SIZE: VmOffset = PMAP_NMAPWINDOWS * NCPUS * PAGE_SIZE;

/// `ptes_per_vm_page` of i386/intel/pmap.c: one hardware entry per VM page.
const PTES_PER_VM_PAGE: usize = 1;

/// `NPTES` of <i386/pmap.h>: entries in one page table.
const NPTES: usize = PAGE_SIZE / size_of::<VmOffset>();

/// `PDPNUM_KERNEL` of <i386/pmap.h>: protected page directories at the kernel
/// end of the address space.
#[cfg(target_arch = "x86_64")]
const PDPNUM_KERNEL: usize =
    ((VM_MAX_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS) >> PDPSHIFT) + 1;

/// `PDPNUM` of <i386/pmap.h>: the page directories `pmap_create()` allocates.
/// The x86_64 build shadows the macro with `PDPNUM_KERNEL` inside the
/// function; the i386 non-PAE build has one.
#[cfg(target_arch = "x86_64")]
const PDPNUM: usize = PDPNUM_KERNEL;
#[cfg(target_arch = "x86")]
const PDPNUM: usize = 1;

/// `LINEAR_MIN_KERNEL_ADDRESS` of <i386/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const LINEAR_MIN_KERNEL_ADDRESS: VmOffset = VM_MIN_KERNEL_ADDRESS;
#[cfg(target_arch = "x86")]
const LINEAR_MIN_KERNEL_ADDRESS: VmOffset = 0xc000_0000;

/// `LINEAR_MAX_KERNEL_ADDRESS` of <i386/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const LINEAR_MAX_KERNEL_ADDRESS: VmOffset = usize::MAX;
#[cfg(target_arch = "x86")]
const LINEAR_MAX_KERNEL_ADDRESS: VmOffset = 0xffff_ffff;

/// `VM_MAX_KERNEL_ADDRESS` of <i386/vm_param.h>.
pub(crate) const VM_MAX_KERNEL_ADDRESS: VmOffset = LINEAR_MAX_KERNEL_ADDRESS
    - LINEAR_MIN_KERNEL_ADDRESS
    + VM_MIN_KERNEL_ADDRESS;

/// `VM_KERNEL_MAP_SIZE` of <i386/vm_param.h>: the room reserved for the
/// kernel map.
#[cfg(target_arch = "x86_64")]
pub(crate) const VM_KERNEL_MAP_SIZE: VmOffset = 1000 * 1024 * 1024;
#[cfg(target_arch = "x86")]
pub(crate) const VM_KERNEL_MAP_SIZE: VmOffset = 170 * 1024 * 1024;

/// `VM_MAX_USER_ADDRESS` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const VM_MAX_USER_ADDRESS: VmOffset = 0x8000_0000_0000;
#[cfg(target_arch = "x86")]
const VM_MAX_USER_ADDRESS: VmOffset = 0xc000_0000;

/// `VM_MIN_USER_ADDRESS` of <machine/vm_param.h>.
const VM_MIN_USER_ADDRESS: VmOffset = 0;

/// `INIT_VM_MIN_KERNEL_ADDRESS` of <i386/vm_param.h>, which the x86_64
/// build sets equal to `VM_MIN_KERNEL_ADDRESS` and whose temporary-mapping
/// code is then compiled out.
#[cfg(target_arch = "x86")]
const INIT_VM_MIN_KERNEL_ADDRESS: VmOffset = 0;

/// `PDPSHIFT` of <i386/pmap.h>: the page-directory-pointer shift.
#[cfg(target_arch = "x86_64")]
const PDPSHIFT: u32 = 30;

const PDESHIFT: u32 = if cfg!(target_arch = "x86_64") { 21 } else { 22 };
/// `PDE_MAPPED_SIZE`: the virtual memory one page-directory entry covers.
const PDE_MAPPED_SIZE: VmOffset = 1 << PDESHIFT;

/// `lin2pdenum()` of <i386/pmap.h>.
const fn lin2pdenum(addr: VmOffset) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        (addr >> PDESHIFT) & 0x1ff
    }
    #[cfg(target_arch = "x86")]
    {
        (addr >> PDESHIFT) & 0x3ff
    }
}

/// `lin2pdenum_cont()` of <i386/pmap.h>, which includes the directory-pointer
/// index when the directories are contiguous.
const fn lin2pdenum_cont(addr: VmOffset) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        (addr >> PDESHIFT) & 0x3ff
    }
    #[cfg(target_arch = "x86")]
    {
        lin2pdenum(addr)
    }
}

/// `ptenum()` of <i386/pmap.h>.
const fn ptenum(addr: VmOffset) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        (addr >> 12) & 0x1ff
    }
    #[cfg(target_arch = "x86")]
    {
        (addr >> 12) & 0x3ff
    }
}

#[cfg(target_arch = "x86_64")]
/// `lin2l4num()` of <i386/pmap.h>.
const fn lin2l4num(addr: VmOffset) -> usize {
    (addr >> 39) & 0x1ff
}

#[cfg(target_arch = "x86_64")]
/// `lin2pdpnum()` of <i386/pmap.h>.
const fn lin2pdpnum(addr: VmOffset) -> usize {
    (addr >> 30) & 0x1ff
}

#[cfg(target_arch = "x86")]
/// `pagenum2lin()` of <i386/pmap.h>: the linear address of a page, from its
/// indices.
const fn pagenum2lin(l4: usize, l3: usize, l2: usize, l1: usize) -> VmOffset {
    let _ = l4;
    let _ = l3;
    ((l2 as VmOffset) << 22) + ((l1 as VmOffset) << 12)
}

#[cfg(target_arch = "x86_64")]
/// `pagenum2lin()` of <i386/pmap.h>: the linear address of a page, from its
/// indices.
const fn pagenum2lin(l4: usize, l3: usize, l2: usize, l1: usize) -> VmOffset {
    ((l4 as VmOffset) << 39)
        + ((l3 as VmOffset) << 30)
        + ((l2 as VmOffset) << 21)
        + ((l1 as VmOffset) << 12)
}

/// `pa_to_pte()` of <i386/pmap.h>.
pub(crate) const fn pa_to_pte(pa: VmOffset) -> VmOffset {
    pa & INTEL_PTE_PFN
}

/// `pte_to_pa()` of <i386/pmap.h>.
const fn pte_to_pa(pte: VmOffset) -> VmOffset {
    pte & INTEL_PTE_PFN
}

/// `phystokv()` of <i386/vm_param.h>.
pub(crate) const fn phystokv(pa: VmOffset) -> VmOffset {
    pa.wrapping_add(VM_MIN_KERNEL_ADDRESS)
}

/// `_kvtophys()` of <i386/vm_param.h>.
const fn kvtophys_early(va: VmOffset) -> VmOffset {
    va.wrapping_sub(VM_MIN_KERNEL_ADDRESS)
}

/// `kvtolin()` of <i386/vm_param.h>, an identity in both configured builds.
const fn kvtolin(va: VmOffset) -> VmOffset {
    va.wrapping_sub(VM_MIN_KERNEL_ADDRESS)
        .wrapping_add(LINEAR_MIN_KERNEL_ADDRESS)
}

/// `lintokv()` of <i386/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const fn lintokv(lin: VmOffset) -> VmOffset {
    lin.wrapping_sub(LINEAR_MIN_KERNEL_ADDRESS)
        .wrapping_add(VM_MIN_KERNEL_ADDRESS)
}

/// `ptetokv()` of <i386/pmap.h>.
fn ptetokv(pte: VmOffset) -> *mut VmOffset {
    phystokv(pte_to_pa(pte)) as *mut VmOffset
}

/// `CPU_HAS_FEATURE()` of <i386/locore.h>, whose table the assembly fills.
pub(crate) fn cpu_has_feature(feature: u32) -> bool {
    // SAFETY: `cpu_features` is the two-word table <i386/locore.h> declares
    // and the boot assembly fills before C code runs.
    let word = unsafe { glue::cpu_features[(feature / 32) as usize] };
    word & (1u32 << (feature % 32)) != 0
}

/// `cpu_set` of <i386/pmap.h>: the set of CPUs a pmap is in use on, one bit
/// each.
#[derive(Debug)]
#[repr(transparent)]
pub struct CpuSet(AtomicIsize);

impl CpuSet {
    const fn new() -> Self {
        Self(AtomicIsize::new(0))
    }

    /// The bit a CPU number selects; the C `cpu_set` is at most 32 bits.
    fn mask(cpu: c_int) -> isize {
        1isize.wrapping_shl(cpu as u32)
    }

    fn bits(&self) -> isize {
        self.0.load(Ordering::Relaxed)
    }

    fn set_bits(&self, bits: isize) {
        self.0.store(bits, Ordering::Relaxed);
    }

    fn set(&self, cpu: c_int) {
        self.0.fetch_or(Self::mask(cpu), Ordering::SeqCst);
    }

    fn clear(&self, cpu: c_int) {
        self.0.fetch_and(!Self::mask(cpu), Ordering::SeqCst);
    }

    fn contains(&self, cpu: c_int) -> bool {
        self.bits() & Self::mask(cpu) != 0
    }
}

/// `struct pmap_statistics` of <mach/vm_statistics.h>.
#[repr(C)]
pub struct PmapStatistics {
    pub resident_count: c_int,
    pub wired_count: c_int,
}

const _: () = assert!(size_of::<PmapStatistics>() == 2 * size_of::<c_int>());
const _: () = assert!(offset_of!(PmapStatistics, resident_count) == 0);
const _: () = assert!(offset_of!(PmapStatistics, wired_count) == 4);

/// `struct pmap` of <i386/pmap.h>: one physical map.
///
/// The C compiles one of three layouts from the `PAE` and `__x86_64__`
/// switches; the two below are the ones the ABI gate builds.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
pub struct Pmap {
    /// `l4base`: the level-4 table, or null.
    pub l4base: *mut VmOffset,
    pub ref_count: c_int,
    pub lock: SimpleLock,
    /// `stats`: resident and wired page counts.
    pub stats: PmapStatistics,
    /// `cpus_using`: the CPUs with this map active.
    pub cpus_using: CpuSet,
}

#[cfg(target_pointer_width = "32")]
#[repr(C)]
pub struct Pmap {
    /// `dirbase`: the page directory, or null.
    pub dirbase: *mut VmOffset,
    pub ref_count: c_int,
    pub lock: SimpleLock,
    /// `stats`: resident and wired page counts.
    pub stats: PmapStatistics,
    /// `cpus_using`: the CPUs with this map active.
    pub cpus_using: CpuSet,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Pmap>() == 32);
    assert!(align_of::<Pmap>() == 8);
    assert!(offset_of!(Pmap, l4base) == 0);
    assert!(offset_of!(Pmap, ref_count) == 8);
    assert!(offset_of!(Pmap, lock) == 12);
    assert!(offset_of!(Pmap, stats) == 16);
    assert!(offset_of!(Pmap, cpus_using) == 24);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Pmap>() == 24);
    assert!(align_of::<Pmap>() == 4);
    assert!(offset_of!(Pmap, dirbase) == 0);
    assert!(offset_of!(Pmap, ref_count) == 4);
    assert!(offset_of!(Pmap, lock) == 8);
    assert!(offset_of!(Pmap, stats) == 12);
    assert!(offset_of!(Pmap, cpus_using) == 20);
};

// SAFETY: the C mutates a live pmap under its own lock, and the boot code
// writes `kernel_pmap_store` before another CPU can see it.
unsafe impl Sync for Pmap {}

impl Pmap {
    /// The zero image a C `static` of `struct pmap` began with.
    const fn zeroed() -> Self {
        Self {
            #[cfg(target_pointer_width = "64")]
            l4base: ptr::null_mut(),
            #[cfg(target_pointer_width = "32")]
            dirbase: ptr::null_mut(),
            ref_count: 0,
            lock: SimpleLock::new(),
            stats: PmapStatistics {
                resident_count: 0,
                wired_count: 0,
            },
            cpus_using: CpuSet::new(),
        }
    }
}

/// `struct pv_entry` of i386/intel/pmap.c: one virtual mapping of a physical
/// page.
#[repr(C)]
pub struct PvEntry {
    pub next: *mut PvEntry,
    pub pmap: *mut Pmap,
    pub va: VmOffset,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<PvEntry>() == 24);
    assert!(align_of::<PvEntry>() == 8);
    assert!(offset_of!(PvEntry, next) == 0);
    assert!(offset_of!(PvEntry, pmap) == 8);
    assert!(offset_of!(PvEntry, va) == 16);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<PvEntry>() == 12);
    assert!(align_of::<PvEntry>() == 4);
    assert!(offset_of!(PvEntry, next) == 0);
    assert!(offset_of!(PvEntry, pmap) == 4);
    assert!(offset_of!(PvEntry, va) == 8);
};

/// `pmap_mapwindow_t` of <i386/pmap.h>: one temporary physical mapping.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PmapMapwindow {
    pub entry: *mut VmOffset,
    pub vaddr: VmOffset,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<PmapMapwindow>() == 16);
    assert!(align_of::<PmapMapwindow>() == 8);
    assert!(offset_of!(PmapMapwindow, entry) == 0);
    assert!(offset_of!(PmapMapwindow, vaddr) == 8);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<PmapMapwindow>() == 8);
    assert!(align_of::<PmapMapwindow>() == 4);
    assert!(offset_of!(PmapMapwindow, entry) == 0);
    assert!(offset_of!(PmapMapwindow, vaddr) == 4);
};

/// `struct pmap_update_item` of i386/intel/pmap.c: one queued invalidation.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PmapUpdateItem {
    pub pmap: *mut Pmap,
    pub start: VmOffset,
    pub end: VmOffset,
}

/// `struct pmap_update_list` of i386/intel/pmap.c: the invalidations queued
/// for one CPU.
#[repr(C)]
pub struct PmapUpdateList {
    pub lock: SimpleLock,
    pub count: c_int,
    pub item: [PmapUpdateItem; UPDATE_LIST_SIZE],
}

const _: () =
    assert!(size_of::<PmapUpdateItem>() == 3 * size_of::<VmOffset>());
const _: () =
    assert!(offset_of!(PmapUpdateItem, start) == size_of::<VmOffset>());
const _: () =
    assert!(offset_of!(PmapUpdateItem, end) == 2 * size_of::<VmOffset>());

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<PmapUpdateList>() == 104);
    assert!(align_of::<PmapUpdateList>() == 8);
    assert!(offset_of!(PmapUpdateList, lock) == 0);
    assert!(offset_of!(PmapUpdateList, count) == 4);
    assert!(offset_of!(PmapUpdateList, item) == 8);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<PmapUpdateList>() == 56);
    assert!(align_of::<PmapUpdateList>() == 4);
    assert!(offset_of!(PmapUpdateList, lock) == 0);
    assert!(offset_of!(PmapUpdateList, count) == 4);
    assert!(offset_of!(PmapUpdateList, item) == 8);
};

impl PmapUpdateList {
    const fn new() -> Self {
        Self {
            lock: SimpleLock::new(),
            count: 0,
            item: [PmapUpdateItem {
                pmap: ptr::null_mut(),
                start: 0,
                end: 0,
            }; UPDATE_LIST_SIZE],
        }
    }
}

/// `kernel_pmap_store` of i386/intel/pmap.c: the kernel's statically
/// allocated map.
#[unsafe(export_name = "kernel_pmap_store")]
static mut KERNEL_PMAP_STORE: Pmap = Pmap::zeroed();

/// `kernel_pmap` of <vm/pmap.h>: the kernel's physical map.
#[unsafe(no_mangle)]
pub static mut kernel_pmap: *mut Pmap = ptr::null_mut();

/// `PMAP_NULL`: the C's null map pointer.
const PMAP_NULL: *mut Pmap = ptr::null_mut();

/// The kernel pmap the C global holds.
pub(crate) fn kernel_pmap_ptr() -> *mut Pmap {
    // SAFETY: `kernel_pmap` is written once, in `pmap_bootstrap()`, before
    // any other CPU runs.
    unsafe { kernel_pmap }
}

/// `pmap_system_lock` of i386/intel/pmap.c: the pmap-system read/write lock.
#[unsafe(export_name = "pmap_system_lock")]
static mut PMAP_SYSTEM_LOCK: LockData = LockData::zeroed();

/// `pmap_initialized` of i386/intel/pmap.c.
#[unsafe(export_name = "pmap_initialized")]
static PMAP_INITIALIZED: AtomicI32 = AtomicI32::new(0);

/// `pmap_debug` of i386/intel/pmap.c: the flag that turns on the enter trace.
#[unsafe(export_name = "pmap_debug")]
static PMAP_DEBUG: AtomicI32 = AtomicI32::new(0);

/// `inuse_ptepages_count` of i386/intel/pmap.c, a debug counter nothing
/// increments.
#[unsafe(export_name = "inuse_ptepages_count")]
static INUSE_PTEPAGES_COUNT: AtomicI32 = AtomicI32::new(0);

/// `kernel_virtual_start` of i386/intel/pmap.c.
#[unsafe(no_mangle)]
pub static mut kernel_virtual_start: VmOffset = 0;

/// `kernel_virtual_end` of i386/intel/pmap.c.
#[unsafe(no_mangle)]
pub static mut kernel_virtual_end: VmOffset = 0;

/// `kernel_page_dir` of <i386/pmap.h>: the kernel's page directory.
#[unsafe(export_name = "kernel_page_dir")]
static mut KERNEL_PAGE_DIR: *mut VmOffset = ptr::null_mut();

/// `pv_head_table` of i386/intel/pmap.c: one pv list head per physical page.
#[unsafe(export_name = "pv_head_table")]
static mut PV_HEAD_TABLE: *mut PvEntry = ptr::null_mut();

/// `pv_free_list` of i386/intel/pmap.c: the free pv entries, at `SPLVM`.
#[unsafe(export_name = "pv_free_list")]
static mut PV_FREE_LIST: *mut PvEntry = ptr::null_mut();

/// `pv_free_list_lock` of i386/intel/pmap.c.
#[unsafe(export_name = "pv_free_list_lock")]
static PV_FREE_LIST_LOCK: SimpleLock = SimpleLock::new();

/// `pv_lock_table` of i386/intel/pmap.c: one lock bit per physical page.
#[unsafe(export_name = "pv_lock_table")]
static mut PV_LOCK_TABLE: *mut c_char = ptr::null_mut();

/// `pmap_phys_attributes` of i386/intel/pmap.c: one attribute byte per
/// physical page.
#[unsafe(export_name = "pmap_phys_attributes")]
static mut PMAP_PHYS_ATTRIBUTES: *mut u8 = ptr::null_mut();

/// `pmap_object` of i386/intel/pmap.c, which nothing reads.
#[unsafe(export_name = "pmap_object")]
static mut PMAP_OBJECT: *mut VmObject = ptr::null_mut();

/// `cpus_active` of <i386/pmap.h>: the CPUs that may use a pmap.
#[unsafe(no_mangle)]
pub static cpus_active: CpuSet = CpuSet::new();

/// `cpus_idle` of <i386/pmap.h>: the CPUs that are idle but will want the
/// kernel pmap updates when they wake.
#[unsafe(no_mangle)]
pub static cpus_idle: CpuSet = CpuSet::new();

/// `cpu_update_needed` of <i386/pmap.h>: the CPUs with queued invalidations.
#[unsafe(no_mangle)]
pub static cpu_update_needed: [AtomicI32; NCPUS] =
    [const { AtomicI32::new(0) }; NCPUS];

/// `cpu_update_list` of i386/intel/pmap.c: the queued invalidations.
#[unsafe(export_name = "cpu_update_list")]
static mut CPU_UPDATE_LIST: [PmapUpdateList; NCPUS] =
    [const { PmapUpdateList::new() }; NCPUS];

/// `mapwindows` of i386/intel/pmap.c: the per-CPU temporary mappings.
#[unsafe(export_name = "mapwindows")]
static mut MAPWINDOWS: [PmapMapwindow; PMAP_NMAPWINDOWS * NCPUS] =
    [PmapMapwindow {
        entry: ptr::null_mut(),
        vaddr: 0,
    }; PMAP_NMAPWINDOWS * NCPUS];

/// `pmap_cache` of i386/intel/pmap.c: the `struct pmap` slab cache.
#[unsafe(export_name = "pmap_cache")]
static mut PMAP_CACHE: KmemCache = KmemCache::zeroed();

/// `pt_cache` of i386/intel/pmap.c: the page-table slab cache.
#[unsafe(export_name = "pt_cache")]
static mut PT_CACHE: KmemCache = KmemCache::zeroed();

/// `pd_cache` of i386/intel/pmap.c: the page-directory slab cache.
#[unsafe(export_name = "pd_cache")]
static mut PD_CACHE: KmemCache = KmemCache::zeroed();

/// `pdpt_cache` of i386/intel/pmap.c: the directory-pointer slab cache.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "pdpt_cache")]
static mut PDPT_CACHE: KmemCache = KmemCache::zeroed();

/// `l4_cache` of i386/intel/pmap.c: the level-4 slab cache.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "l4_cache")]
static mut L4_CACHE: KmemCache = KmemCache::zeroed();

/// `pv_list_cache` of i386/intel/pmap.c: the `struct pv_entry` slab cache.
#[unsafe(export_name = "pv_list_cache")]
static mut PV_LIST_CACHE: KmemCache = KmemCache::zeroed();

/// Invalidate the TLB entry for one page, given its linear address.
fn invalidate_linear_page(linear: VmOffset) {
    // SAFETY: The two selectors are the architectural constants of
    // <i386/gdt.h>, and the kernel's GDT describes them from `gdt_init()` on,
    // so neither load can fault.
    unsafe {
        asm!(
            "movw {linear_ds:x}, %es",
            "invlpg %es:({addr})",
            "movw {kernel_ds:x}, %es",
            addr = in(reg) linear,
            linear_ds = in(reg) LINEAR_DS,
            kernel_ds = in(reg) KERNEL_DS,
            options(att_syntax, nostack, preserves_flags),
        );
    }
}

/// The `get_cr3()` of <i386/proc_reg.h>.
fn read_cr3() -> usize {
    let value: usize;
    // SAFETY: reading CR3 is legal at CPL 0.
    unsafe {
        asm!(
            "mov {value}, cr3",
            value = out(reg) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}

/// The `set_cr3()` of <i386/proc_reg.h>.
fn write_cr3(value: usize) {
    // SAFETY: writing CR3 is legal at CPL 0; the caller supplies a page
    // directory with the mappings the kernel is already using.
    unsafe {
        asm!(
            "mov cr3, {value}",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

/// The `get_cr4()` of <i386/proc_reg.h>.
#[cfg(target_arch = "x86_64")]
fn read_cr4() -> usize {
    let value: usize;
    // SAFETY: reading CR4 is legal at CPL 0.
    unsafe {
        asm!(
            "mov {value}, cr4",
            value = out(reg) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}

/// The `set_cr4()` of <i386/proc_reg.h>.
#[cfg(target_arch = "x86_64")]
fn write_cr4(value: usize) {
    // SAFETY: writing CR4 is legal at CPL 0; the caller only adds the PAE bit
    // the page tables were built for.
    unsafe {
        asm!(
            "mov cr4, {value}",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

/// The `flush_tlb()` of <i386/proc_reg.h>, which is `set_cr3(get_cr3())`.
fn flush_tlb() {
    write_cr3(read_cr3());
}

/// The `set_pmap()` of <i386/intel/pmap.h>, the level-4 table arm.
#[cfg(target_arch = "x86_64")]
fn set_pmap(pmap: *mut Pmap) {
    // SAFETY: the caller passes a live map whose `l4base` the kernel built.
    unsafe { write_cr3(kvtophys((*pmap).l4base as VmOffset) as usize) };
}

/// The `set_pmap()` of <i386/intel/pmap.h>, the page-directory arm.
#[cfg(target_arch = "x86")]
fn set_pmap(pmap: *mut Pmap) {
    // SAFETY: the caller passes a live map whose `dirbase` the kernel built.
    unsafe { write_cr3(kvtophys((*pmap).dirbase as VmOffset) as usize) };
}

/// The `PMAP_ACTIVATE_USER()` of <i386/intel/pmap.h>: make `pmap` current on
/// `cpu` and add the CPU to its active set.
///
/// # Safety
///
/// `pmap` must be live, and `cpu` must be the calling CPU with interrupts
/// blocked, as the context switch has them.
pub unsafe fn activate_user(pmap: *mut Pmap, cpu: c_int) {
    if pmap == kernel_pmap_ptr() {
        set_pmap(pmap);
        return;
    }

    cpus_active.clear(cpu);
    // SAFETY: the caller promises a live map; the lock protects
    // `cpus_using`, and `set_pmap()` reads the map the lock covers.
    unsafe {
        (*pmap).lock.lock();
        set_pmap(pmap);
        (*pmap).cpus_using.set(cpu);
    }
    cpus_active.set(cpu);
    // SAFETY: as above; the lock is held.
    unsafe { (*pmap).lock.unlock() };
}

/// The `PMAP_DEACTIVATE_USER()` of <i386/intel/pmap.h>: remove `cpu` from
/// `pmap`'s active set.
///
/// # Safety
///
/// `pmap` must be live, and `cpu` must be the calling CPU.
pub unsafe fn deactivate_user(pmap: *mut Pmap, cpu: c_int) {
    if pmap != kernel_pmap_ptr() {
        // SAFETY: the caller promises a live map; the C cleared the bit
        // without the lock.
        unsafe { (*pmap).cpus_using.clear(cpu) };
    }
}

/// The `PMAP_DEACTIVATE_KERNEL()` of <i386/intel/pmap.h>: remove `cpu` from
/// the kernel map's active set.
///
/// # Safety
///
/// `cpu` must be the calling CPU.
pub(crate) unsafe fn deactivate_kernel(cpu: c_int) {
    // SAFETY: the kernel map is live from `pmap_bootstrap()`, and the C macro
    // cleared the bit without the lock.
    unsafe { (*kernel_pmap_ptr()).cpus_using.clear(cpu) };
}

/// The `PMAP_ACTIVATE_KERNEL()` of <i386/intel/pmap.h>: make the kernel pmap
/// current on `cpu` and flush its queued updates.
///
/// # Safety
///
/// `cpu` must be the calling CPU with interrupts blocked, as the boot and
/// context-switch paths have them.
pub(crate) unsafe fn activate_kernel(cpu: c_int) {
    cpus_active.clear(cpu);
    // SAFETY: the kernel map is live from `pmap_bootstrap()`, and the lock
    // protects the queued updates and `cpus_using`.
    unsafe {
        (*kernel_pmap_ptr()).lock.lock();

        if cpu_update_needed[cpu as usize].load(Ordering::Relaxed) != 0 {
            process_pmap_updates(kernel_pmap_ptr());
        }

        (*kernel_pmap_ptr()).cpus_using.set(cpu);
        cpus_active.set(cpu);

        (*kernel_pmap_ptr()).lock.unlock();
    }
}

/// The `SPLVM()` of i386/intel/pmap.c, minus the assignment the macro makes.
fn raise_splvm() -> c_int {
    // SAFETY: `splvm` is the real routine <i386/spl.h> declares.
    let spl = unsafe { glue::splvm() };
    cpus_active.clear(cpu_number());
    spl
}

/// The `SPLX()` of i386/intel/pmap.c.
fn restore_spl(spl: c_int) {
    cpus_active.set(cpu_number());
    // SAFETY: `spl` came from `splvm()`, and `splx` accepts any level.
    unsafe { glue::splx(spl) };
}

/// The pmap-system lock `lock_init()` built.
fn system_lock() -> *mut LockData {
    &raw mut PMAP_SYSTEM_LOCK
}

/// The `PMAP_READ_LOCK()` of i386/intel/pmap.c: take the system lock for
/// read, then the map's own lock, and return the level to restore.
unsafe fn read_lock(pmap: *mut Pmap) -> c_int {
    let spl = raise_splvm();
    // SAFETY: the caller passes a live map, and the system lock is the one
    // `pmap_bootstrap()` initialized.
    unsafe {
        (*system_lock()).read();
        (*pmap).lock.lock();
    }
    spl
}

/// The `PMAP_READ_UNLOCK()` of i386/intel/pmap.c.
unsafe fn read_unlock(pmap: *mut Pmap, spl: c_int) {
    // SAFETY: the caller holds both locks, which `read_lock()` took; the
    // system lock's read count is dropped with `lock_done()`.
    unsafe {
        (*pmap).lock.unlock();
        (*system_lock()).done();
    }
    restore_spl(spl);
}

/// The `PMAP_WRITE_LOCK()` of i386/intel/pmap.c.
fn write_lock() -> c_int {
    let spl = raise_splvm();
    // SAFETY: the system lock is live from `pmap_bootstrap()` on.
    unsafe { (*system_lock()).write() };
    spl
}

/// The `PMAP_WRITE_UNLOCK()` of i386/intel/pmap.c.
fn write_unlock(spl: c_int) {
    // SAFETY: the caller holds the system lock for write.
    unsafe { (*system_lock()).done() };
    restore_spl(spl);
}

/// The `INVALIDATE_TLB()` of i386/intel/pmap.c: one `invlpg` for a single
/// page, a CR3 reload for anything wider.
unsafe fn invalidate_tlb(pmap: *mut Pmap, s: VmOffset, e: VmOffset) {
    if e.wrapping_sub(s) == PAGE_SIZE {
        let addr = if pmap == kernel_pmap_ptr() {
            kvtolin(s)
        } else {
            s
        };
        invalidate_linear_page(addr);
    } else {
        flush_tlb();
    }
}

/// The `PMAP_UPDATE_TLBS()` of i386/intel/pmap.c: signal every other CPU
/// using the map, wait for them to acknowledge, then invalidate locally.
///
/// # Safety
///
/// `pmap` must be live and locked, and the caller must be at `SPLVM` or
/// above with interrupts blocked, as the C's callers were.
unsafe fn update_tlbs(pmap: *mut Pmap, s: VmOffset, e: VmOffset) {
    let cpu_mask = CpuSet::mask(cpu_number());
    // SAFETY: the caller holds the map lock, so `cpus_using` is stable
    // against activation.
    let users = unsafe { (*pmap).cpus_using.bits() } & !cpu_mask;
    if users != 0 {
        // SAFETY: the caller passes the same live map and range to the
        // signaler.
        unsafe { signal_cpus(users, pmap, s, e) };
        while unsafe { (*pmap).cpus_using.bits() }
            & cpus_active.bits()
            & !cpu_mask
            != 0
        {
            core::hint::spin_loop();
        }
    }
    if unsafe { (*pmap).cpus_using.contains(cpu_number()) } {
        // SAFETY: `pmap` is live and locked; the range is the one entered.
        unsafe { invalidate_tlb(pmap, s, e) };
    }
}

#[cfg(target_arch = "x86_64")]
/// The `pmap_l4base()` of i386/intel/pmap.c.
unsafe fn l4base_of(pmap: *mut Pmap, addr: VmOffset) -> *mut VmOffset {
    // SAFETY: the caller passes a live map.
    let base = unsafe { (*pmap).l4base };
    if base.is_null() {
        return ptr::null_mut();
    }
    base.wrapping_add(lin2l4num(addr))
}

#[cfg(target_arch = "x86_64")]
/// The `pmap_ptp()` of i386/intel/pmap.c.
unsafe fn ptp_of(pmap: *mut Pmap, addr: VmOffset) -> *mut VmOffset {
    // SAFETY: the caller passes a live map, and `l4base_of` null-checks the
    // table it returns.
    let l4_table = unsafe { l4base_of(pmap, addr) };
    if l4_table.is_null() {
        return ptr::null_mut();
    }
    let pdp = unsafe { *l4_table };
    if pdp & INTEL_PTE_VALID == 0 {
        return ptr::null_mut();
    }
    ptetokv(pdp).wrapping_add(lin2pdpnum(addr))
}

/// The `pmap_pde()` of i386/intel/pmap.c.
unsafe fn pde_of(pmap: *mut Pmap, addr: VmOffset) -> *mut VmOffset {
    let addr = if pmap == kernel_pmap_ptr() {
        kvtolin(addr)
    } else {
        addr
    };
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the caller passes a live map; `ptp_of` returns null rather
        // than a bad pointer when a level is missing.
        let ptp = unsafe { ptp_of(pmap, addr) };
        if ptp.is_null() {
            return ptr::null_mut();
        }
        let pde = unsafe { *ptp };
        if pde & INTEL_PTE_VALID == 0 {
            return ptr::null_mut();
        }
        ptetokv(pde).wrapping_add(lin2pdenum(addr))
    }
    #[cfg(target_arch = "x86")]
    {
        // SAFETY: the caller passes a live map; `pmap_pte()` checked the
        // directory pointer before calling.
        let page_dir = unsafe { (*pmap).dirbase };
        page_dir.wrapping_add(lin2pdenum(addr))
    }
}

/// The `pmap_pte()` of i386/intel/pmap.c, whose C callers treated a null
/// return as "no mapping".
unsafe fn pte_of(pmap: *mut Pmap, addr: VmOffset) -> *mut VmOffset {
    #[cfg(target_arch = "x86_64")]
    let base_null = unsafe { (*pmap).l4base }.is_null();
    #[cfg(target_arch = "x86")]
    let base_null = unsafe { (*pmap).dirbase }.is_null();
    if base_null {
        return ptr::null_mut();
    }
    // SAFETY: the caller passes a live map; `pde_of` returns null rather than
    // a bad pointer.
    let ptp = unsafe { pde_of(pmap, addr) };
    if ptp.is_null() {
        return ptr::null_mut();
    }
    let pte = unsafe { *ptp };
    if pte & INTEL_PTE_VALID == 0 {
        return ptr::null_mut();
    }
    ptetokv(pte).wrapping_add(ptenum(addr))
}

/// `pmap_pte()` of <i386/pmap.h>.
///
/// # Safety
///
/// `pmap` must be a live physical map, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_pte(
    pmap: *mut Pmap,
    addr: VmOffset,
) -> *mut VmOffset {
    // SAFETY: the caller promises a live map.
    unsafe { pte_of(pmap, addr) }
}

/// Allocate one object from a slab cache, or null.
///
/// # Safety
///
/// `cache` must have been initialized and no other access may hold its lock.
unsafe fn cache_alloc(cache: *mut KmemCache) -> *mut u8 {
    // SAFETY: the caller promises the cache is live; `alloc` takes the cache
    // lock itself.
    match unsafe { (*cache).alloc() } {
        Some(buf) => buf.as_ptr(),
        None => ptr::null_mut(),
    }
}

/// Return one object to a slab cache.
///
/// # Safety
///
/// `obj` must be a live object of `cache`, and no other access may hold the
/// cache's lock.
unsafe fn cache_free(cache: *mut KmemCache, obj: *mut u8) {
    let Some(obj) = NonNull::new(obj) else {
        return;
    };
    // SAFETY: the caller promises a live object of this cache.
    unsafe { (*cache).free(obj) };
}

/// `pai_to_pvh()` of i386/intel/pmap.c: the pv list head of a page index.
fn pv_head(pai: usize) -> *mut PvEntry {
    // SAFETY: the caller's index is inside the table `pmap_init()` allocated
    // for every managed page.
    unsafe { PV_HEAD_TABLE.wrapping_add(pai) }
}

/// The `PV_ALLOC()` of i386/intel/pmap.c.
fn pv_alloc() -> *mut PvEntry {
    // SAFETY: the free list is only touched under its own lock.
    unsafe {
        PV_FREE_LIST_LOCK.lock();
        let entry = PV_FREE_LIST;
        if !entry.is_null() {
            PV_FREE_LIST = (*entry).next;
        }
        PV_FREE_LIST_LOCK.unlock();
        entry
    }
}

/// The `PV_FREE()` of i386/intel/pmap.c.
fn pv_free(entry: *mut PvEntry) {
    // SAFETY: the entry came from the pv list of a managed page, and the free
    // list is only touched under its own lock.
    unsafe {
        PV_FREE_LIST_LOCK.lock();
        (*entry).next = PV_FREE_LIST;
        PV_FREE_LIST = entry;
        PV_FREE_LIST_LOCK.unlock();
    }
}

/// The `valid_page()` of i386/intel/pmap.c: whether the physical address is
/// a managed page.
fn valid_page(addr: VmOffset) -> bool {
    if PMAP_INITIALIZED.load(Ordering::Relaxed) == 0 {
        return false;
    }
    vm_page::lookup_pa(addr).is_some()
}

/// `pmap_pageable()` in i386/intel/pmap.c: advisory, and empty in the C
/// too, since `pmap_enter()` already learns whether a page is wired.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_pageable(
    _pmap: *mut Pmap,
    _start: VmOffset,
    _end: VmOffset,
    _pageable: c_int,
) {
}

/// Unmap the page at virtual address zero so that a null reference faults.
fn unmap_page_zero() {
    // SAFETY: `printf` is the real C routine <kern/printf.h> declares, and
    // this format has no conversion specifier.
    unsafe {
        glue::printf(
            c"Unmapping the zero page.  Some BIOS functions may not be working any more.\n"
                .as_ptr(),
        )
    };
    // SAFETY: `kernel_pmap` is the kernel's live pmap from `pmap_bootstrap()`
    // on, and `pmap_pte()` returns null rather than something invalid for an
    // address with no page-table entry.
    let pte = unsafe { pte_of(kernel_pmap_ptr(), 0) };
    let Some(pte) = NonNull::new(pte) else {
        return;
    };
    // SAFETY: `pmap_pte()` returned a non-null pointer to a live page table
    // entry for address zero.
    unsafe { pte.as_ptr().write(0) };
    invalidate_linear_page(0);
}

/// Unmap the page at virtual address zero so that a null reference faults.
///
/// # Safety
///
/// The kernel pmap must be initialized, so that `pmap_pte()` can walk it, and
/// nothing may depend on the zero page's mapping afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_unmap_page_zero() {
    unmap_page_zero();
}

/// The `pmap_map_bd()` of i386/intel/pmap.c: the boot-time back door that
/// maps memory outside the direct map.
///
/// # Safety
///
/// `virt` must name a free kernel-virtual range covering `start` to `end`,
/// and the caller must be running with the mapping machinery usable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_map_bd(
    virt: VmOffset,
    start: VmOffset,
    end: VmOffset,
    prot: VmProt,
) -> VmOffset {
    // SAFETY: the caller promises the kernel map is live.
    unsafe { map_bd(virt, start, end, prot) }
}

/// # Safety
///
/// As `pmap_map_bd`.
unsafe fn map_bd(
    mut virt: VmOffset,
    mut start: VmOffset,
    end: VmOffset,
    prot: VmProt,
) -> VmOffset {
    let mut template = pa_to_pte(start)
        | INTEL_PTE_NCACHE
        | INTEL_PTE_WTHRU
        | INTEL_PTE_VALID;
    if cpu_has_feature(CPU_FEATURE_PGE) {
        template |= INTEL_PTE_GLOBAL;
    }
    if prot.contains(VmProt::WRITE) {
        template |= INTEL_PTE_WRITE;
    }

    // SAFETY: the caller promises the kernel map is live, which the C's
    // callers guaranteed.
    let spl = unsafe { read_lock(kernel_pmap_ptr()) };
    while start < end {
        // SAFETY: the kernel map is live, as `map_bd`'s caller promised.
        let pte = unsafe { pte_of(kernel_pmap_ptr(), virt) };
        if pte.is_null() {
            // SAFETY: `Panic` does not return, and the message is the C's.
            unsafe {
                glue::Panic(
                    c"i386/intel/pmap.c".as_ptr(),
                    line!() as c_int,
                    c"pmap_map_bd".as_ptr(),
                    c"pmap_map_bd: Invalid kernel address\n".as_ptr(),
                )
            }
        }
        // SAFETY: `pmap_pte()` returned a live entry for `virt`.
        unsafe { *pte = template };
        template = template.wrapping_add(PAGE_SIZE);
        virt = virt.wrapping_add(PAGE_SIZE);
        start = start.wrapping_add(PAGE_SIZE);
    }
    // SAFETY: the kernel map is live and the lock was taken above.
    unsafe { read_unlock(kernel_pmap_ptr(), spl) };
    virt
}

#[cfg(target_arch = "x86_64")]
/// The `pmap_bootstrap_pae()` of i386/intel/pmap.c.
fn bootstrap_pae() {
    let l4 =
        with_exposed_provenance_mut::<VmOffset>(phystokv(pmap_grab_page()));
    // SAFETY: `kernel_pmap` points at the static store, and the L4 table was
    // just grabbed for it.
    unsafe { (*kernel_pmap_ptr()).l4base = l4 };
    // SAFETY: the freshly grabbed page is the kernel's to clear.
    unsafe { ptr::write_bytes(l4.cast::<u8>(), 0, PAGE_SIZE) };

    let mut addr: VmOffset = 0;
    // SAFETY: `init_alloc_aligned()` either fills `addr` or halts the boot,
    // and this runs single-threaded with mapping off.
    unsafe { init_alloc_aligned(PDPNUM_KERNEL * PAGE_SIZE, &mut addr) };
    let page_dir = with_exposed_provenance_mut::<VmOffset>(phystokv(addr));
    // SAFETY: the boot allocator returned this page for the kernel directory.
    unsafe { KERNEL_PAGE_DIR = page_dir };
    // SAFETY: as above; the directory is PDPNUM_KERNEL pages.
    unsafe {
        ptr::write_bytes(page_dir.cast::<u8>(), 0, PDPNUM_KERNEL * PAGE_SIZE)
    };

    let pdp =
        with_exposed_provenance_mut::<VmOffset>(phystokv(pmap_grab_page()));
    // SAFETY: the freshly grabbed page is the kernel's to clear.
    unsafe { ptr::write_bytes(pdp.cast::<u8>(), 0, PAGE_SIZE) };
    for i in 0..PDPNUM_KERNEL {
        let index = i + lin2pdpnum(VM_MIN_KERNEL_ADDRESS);
        // SAFETY: the directory pointer table is one page, and the index is
        // below its `NPTES` entries.
        unsafe {
            *pdp.wrapping_add(index) = pa_to_pte(kvtophys_early(
                page_dir.cast::<u8>().wrapping_add(i * PAGE_SIZE).addr(),
            )) | INTEL_PTE_VALID
                | INTEL_PTE_WRITE;
        }
    }
    // SAFETY: the L4 table is one page and the kernel's index is in range.
    unsafe {
        *l4.wrapping_add(lin2l4num(VM_MIN_KERNEL_ADDRESS)) =
            pa_to_pte(kvtophys_early(pdp.addr()))
                | INTEL_PTE_VALID
                | INTEL_PTE_WRITE;
    }
}

/// `pmap_bootstrap()` in i386/intel/pmap.c: build the kernel's page tables
/// with mapping off, so only physical addresses are reachable.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_bootstrap() {
    // SAFETY: this runs before any other CPU, and nothing reads the global
    // before the store.
    unsafe { kernel_pmap = &raw mut KERNEL_PMAP_STORE };

    // SAFETY: the lock's storage is unshared at this point in the boot.
    unsafe { LockData::init(&raw mut PMAP_SYSTEM_LOCK, false) };
    // SAFETY: as above, for the kernel map's own lock and count.
    unsafe {
        (*kernel_pmap_ptr()).lock.init();
        (*kernel_pmap_ptr()).ref_count = 1;
    }

    let virtual_start = phystokv(biosmem::directmap_end());
    // SAFETY: the kernel virtual range is written here for the first time.
    unsafe {
        kernel_virtual_start = virtual_start;
        kernel_virtual_end = virtual_start.wrapping_add(VM_KERNEL_MAP_SIZE);
        if kernel_virtual_end < kernel_virtual_start
            || kernel_virtual_end
                > VM_MAX_KERNEL_ADDRESS.wrapping_sub(PAGE_SIZE)
        {
            kernel_virtual_end = VM_MAX_KERNEL_ADDRESS.wrapping_sub(PAGE_SIZE);
        }
    }

    // SAFETY: `printf` is the real routine, and the two values are the
    // addresses just computed.
    unsafe {
        glue::printf(
            c"kernel virtual area: %lx-%lx\n".as_ptr(),
            kernel_virtual_start,
            kernel_virtual_end,
        )
    };

    #[cfg(target_arch = "x86_64")]
    bootstrap_pae();

    #[cfg(target_arch = "x86")]
    {
        let page_dir = with_exposed_provenance_mut::<VmOffset>(phystokv(
            pmap_grab_page(),
        ));
        // SAFETY: the kernel map is the store, and the directory was just
        // grabbed for it; it is one page.
        unsafe {
            (*kernel_pmap_ptr()).dirbase = page_dir;
            KERNEL_PAGE_DIR = page_dir;
        }
        for i in 0..NPTES {
            // SAFETY: the directory is one page of `NPTES` entries, all
            // writable before paging is on.
            unsafe { *page_dir.wrapping_add(i) = 0 };
        }
    }

    let global = if cpu_has_feature(CPU_FEATURE_PGE) {
        INTEL_PTE_GLOBAL
    } else {
        0
    };
    let image_start = ptr::addr_of!(glue::_start).addr();
    let image_end = ptr::addr_of!(glue::etext).addr();
    let directmap_end = phystokv(biosmem::directmap_end());
    // SAFETY: the kernel directory is live from the branch above, and the
    // physical memory it maps is the machine's RAM.
    unsafe {
        let directory = KERNEL_PAGE_DIR;
        let mut va = phystokv(0);
        while va >= phystokv(0) && va < kernel_virtual_end {
            let pde = directory.wrapping_add(lin2pdenum_cont(kvtolin(va)));
            let ptable = with_exposed_provenance_mut::<VmOffset>(phystokv(
                pmap_grab_page(),
            ));
            *pde = pa_to_pte(kvtophys_early(ptable.addr()))
                | INTEL_PTE_VALID
                | INTEL_PTE_WRITE;

            let mut pte = ptable;
            while va < directmap_end && pte < ptable.wrapping_add(NPTES) {
                if (pte.offset_from(ptable) as usize) < ptenum(va) {
                    *pte = 0;
                } else {
                    if va >= image_start
                        && va.wrapping_add(PAGE_SIZE) <= image_end
                    {
                        *pte = pa_to_pte(kvtophys_early(va))
                            | INTEL_PTE_VALID
                            | global;
                    } else {
                        *pte = pa_to_pte(kvtophys_early(va))
                            | INTEL_PTE_VALID
                            | INTEL_PTE_WRITE
                            | global;
                    }
                    va = va.wrapping_add(PAGE_SIZE);
                }
                pte = pte.wrapping_add(1);
            }
            while pte < ptable.wrapping_add(NPTES) {
                let window_start = kernel_virtual_end - MAPWINDOW_SIZE;
                if va >= window_start && va < kernel_virtual_end {
                    let index = (va - window_start) >> PAGE_SHIFT;
                    let win = (&raw mut MAPWINDOWS)
                        .cast::<PmapMapwindow>()
                        .wrapping_add(index);
                    (*win).entry = pte;
                    (*win).vaddr = va;
                }
                *pte = 0;
                va = va.wrapping_add(PAGE_SIZE);
                pte = pte.wrapping_add(1);
            }
        }
    }
}

/// `pmap_get_mapwindow()` of <i386/pmap.h>: map a physical page in the
/// calling CPU's temporary window.
///
/// # Safety
///
/// `entry` must be a page-table template the caller intends to map now.
///
/// # Panics
///
/// Halts when both of the CPU's windows are busy, a state the two nested
/// users cannot reach.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_get_mapwindow(
    entry: VmOffset,
) -> *mut PmapMapwindow {
    // SAFETY: `kernel_pmap` is live from `pmap_bootstrap()` on, and the
    // window array is indexed by a CPU number below `NCPUS`.
    unsafe {
        let cpu = cpu_number() as usize;
        let windows = (&raw mut MAPWINDOWS).cast::<PmapMapwindow>();
        let start = windows.wrapping_add(cpu * PMAP_NMAPWINDOWS);
        let end = windows.wrapping_add((cpu + 1) * PMAP_NMAPWINDOWS);
        let mut map = start;
        while map < end {
            let slot = (*map).entry;
            if slot.is_null() || *slot == 0 {
                break;
            }
            map = map.wrapping_add(1);
        }
        if map == end {
            // SAFETY: `Panic` does not return; the message is the C's
            // condition, which the C left undefined.
            glue::Panic(
                c"i386/intel/pmap.c".as_ptr(),
                line!() as c_int,
                c"pmap_get_mapwindow".as_ptr(),
                c"pmap_get_mapwindow: no free map window\n".as_ptr(),
            )
        }
        *(*map).entry = entry;
        invalidate_tlb(
            kernel_pmap_ptr(),
            (*map).vaddr,
            (*map).vaddr.wrapping_add(PAGE_SIZE),
        );
        map
    }
}

/// `pmap_put_mapwindow()` of <i386/pmap.h>: drop a temporary mapping.
///
/// # Safety
///
/// `map` must be a window `pmap_get_mapwindow()` returned and not yet
/// released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_put_mapwindow(map: *mut PmapMapwindow) {
    // SAFETY: the caller promises a live window of the array.
    unsafe {
        *(*map).entry = 0;
        invalidate_tlb(
            kernel_pmap_ptr(),
            (*map).vaddr,
            (*map).vaddr.wrapping_add(PAGE_SIZE),
        );
    }
}

/// `pmap_virtual_space()` of <vm/pmap.h>: the kernel virtual range left for
/// the VM system.
///
/// # Safety
///
/// `startp` and `endp` must be valid for a write, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_virtual_space(
    startp: *mut VmOffset,
    endp: *mut VmOffset,
) {
    // SAFETY: the caller promises both out-parameters are writable, and the
    // bounds are set by `pmap_bootstrap()`.
    unsafe {
        *startp = kernel_virtual_start;
        *endp = kernel_virtual_end.wrapping_sub(MAPWINDOW_SIZE);
    }
}

/// `pmap_init()` in i386/intel/pmap.c: allocate the pv tables and the cache
/// of maps.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_init() {
    let npages = vm_page::table_size();
    let size = round_page(
        size_of::<PvEntry>() * npages + pv_lock_table_size(npages) + npages,
    );

    let mut addr: VmOffset = 0;
    // SAFETY: `kernel_map` is the live kernel map by the time `pmap_init()`
    // runs, and `addr` is a local valid for the write.
    let result = unsafe {
        glue::kmem_alloc_wired(glue::kernel_map.cast(), &mut addr, size)
    };
    if result != KERN_SUCCESS {
        // SAFETY: `Panic` does not return; the message is the C's.
        unsafe {
            glue::Panic(
                c"i386/intel/pmap.c".as_ptr(),
                line!() as c_int,
                c"pmap_init".as_ptr(),
                c"pmap_init\n".as_ptr(),
            )
        }
    }
    // SAFETY: `kmem_alloc_wired()` returned `size` writable bytes at `addr`.
    unsafe { ptr::write_bytes(addr as *mut u8, 0, size) };

    // SAFETY: the block the VM system handed over is now carved into the pv
    // head table, its lock bits and the attribute bytes.
    unsafe {
        PV_HEAD_TABLE = addr as *mut PvEntry;
        addr += size_of::<PvEntry>() * npages;
        PV_LOCK_TABLE = addr as *mut c_char;
        addr += pv_lock_table_size(npages);
        PMAP_PHYS_ATTRIBUTES = addr as *mut u8;
    }

    // SAFETY: each cache's storage is unshared and this runs once.  The
    // names are the C strings the C passed; the sizes are the mirror's.
    unsafe {
        let pmap_cache = &raw mut PMAP_CACHE;
        (*pmap_cache).init(
            c"pmap".to_bytes(),
            size_of::<Pmap>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
        let pt_cache = &raw mut PT_CACHE;
        (*pt_cache).init(
            c"pmap_L1".to_bytes(),
            PAGE_SIZE,
            PAGE_SIZE,
            None,
            CacheInitFlags::PHYSMEM,
        );
        let pd_cache = &raw mut PD_CACHE;
        (*pd_cache).init(
            c"pmap_L2".to_bytes(),
            PAGE_SIZE,
            PAGE_SIZE,
            None,
            CacheInitFlags::PHYSMEM,
        );
        #[cfg(target_arch = "x86_64")]
        {
            let pdpt_cache = &raw mut PDPT_CACHE;
            (*pdpt_cache).init(
                c"pmap_L3".to_bytes(),
                PAGE_SIZE,
                PAGE_SIZE,
                None,
                CacheInitFlags::PHYSMEM,
            );
            let l4_cache = &raw mut L4_CACHE;
            (*l4_cache).init(
                c"pmap_L4".to_bytes(),
                PAGE_SIZE,
                PAGE_SIZE,
                None,
                CacheInitFlags::PHYSMEM,
            );
        }
        let pv_list_cache = &raw mut PV_LIST_CACHE;
        (*pv_list_cache).init(
            c"pv_entry".to_bytes(),
            size_of::<PvEntry>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
    }

    for i in 0..NCPUS {
        let up = update_list(i as c_int);
        // SAFETY: each update list is this file's own storage.
        unsafe {
            (*up).lock.init();
            (*up).count = 0;
        }
    }

    PMAP_INITIALIZED.store(1, Ordering::Relaxed);
}

/// `pv_lock_table_size()` of i386/intel/pmap.c: the bytes one lock bit per
/// page needs.
const fn pv_lock_table_size(n: usize) -> usize {
    n.div_ceil(8)
}

/// `pmap_create()` in i386/intel/pmap.c: build a fresh physical map.
///
/// # Safety
///
/// The caches must be initialized, which `pmap_init()` does, and the caller
/// must be ready to run with the pmap system's locks free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_create(size: VmSize) -> *mut Pmap {
    if size != 0 {
        return PMAP_NULL;
    }

    // SAFETY: the caller promises the caches are live; `cache_alloc` takes
    // the cache's lock.
    let p = unsafe { cache_alloc(&raw mut PMAP_CACHE) }.cast::<Pmap>();
    if p.is_null() {
        return PMAP_NULL;
    }

    let mut page_dir: [*mut VmOffset; PDPNUM] = [ptr::null_mut(); PDPNUM];
    let mut i = 0;
    while i < PDPNUM {
        // SAFETY: as above, for the page-directory cache.
        page_dir[i] =
            unsafe { cache_alloc(&raw mut PD_CACHE) }.cast::<VmOffset>();
        if page_dir[i].is_null() {
            while i > 0 {
                i -= 1;
                // SAFETY: every earlier entry is an object this function
                // allocated from this cache.
                unsafe { cache_free(&raw mut PD_CACHE, page_dir[i].cast()) };
            }
            // SAFETY: `p` is this function's object from the pmap cache.
            unsafe { cache_free(&raw mut PMAP_CACHE, p.cast()) };
            return PMAP_NULL;
        }
        // SAFETY: the new directory is one page, and `kernel_page_dir` maps
        // the same `PDPNUM` pages from `pmap_bootstrap()` on.
        unsafe {
            ptr::copy_nonoverlapping(
                KERNEL_PAGE_DIR.cast::<u8>().wrapping_add(i * PAGE_SIZE),
                page_dir[i].cast::<u8>(),
                PAGE_SIZE,
            );
        }
        i += 1;
    }

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the caches are live, as above.
        unsafe {
            let pdp_kernel =
                cache_alloc(&raw mut PDPT_CACHE).cast::<VmOffset>();
            if pdp_kernel.is_null() {
                let mut k = 0;
                while k < PDPNUM {
                    cache_free(&raw mut PD_CACHE, page_dir[k].cast());
                    k += 1;
                }
                cache_free(&raw mut PMAP_CACHE, p.cast());
                return PMAP_NULL;
            }
            ptr::write_bytes(pdp_kernel.cast::<u8>(), 0, PAGE_SIZE);
            let mut k = 0;
            while k < PDPNUM {
                let index = k + lin2pdpnum(VM_MIN_KERNEL_ADDRESS);
                *pdp_kernel.wrapping_add(index) =
                    pa_to_pte(kvtophys(page_dir[k].addr()))
                        | INTEL_PTE_VALID
                        | INTEL_PTE_WRITE;
                k += 1;
            }

            let l4base = cache_alloc(&raw mut L4_CACHE).cast::<VmOffset>();
            if l4base.is_null() {
                glue::Panic(
                    c"i386/intel/pmap.c".as_ptr(),
                    line!() as c_int,
                    c"pmap_create".as_ptr(),
                    c"pmap_create\n".as_ptr(),
                )
            }
            ptr::write_bytes(l4base.cast::<u8>(), 0, PAGE_SIZE);
            *l4base.wrapping_add(lin2l4num(VM_MIN_KERNEL_ADDRESS)) =
                pa_to_pte(kvtophys(pdp_kernel.addr()))
                    | INTEL_PTE_VALID
                    | INTEL_PTE_WRITE;
            (*p).l4base = l4base;
        }
    }

    #[cfg(target_arch = "x86")]
    {
        // SAFETY: the directory is the first object allocated above.
        unsafe { (*p).dirbase = page_dir[0] };
    }

    // SAFETY: `p` is this function's fresh pmap, not yet visible to another
    // thread; the C initialized exactly these fields.
    unsafe {
        (*p).ref_count = 1;
        (*p).lock.init();
        (*p).cpus_using.set_bits(0);
        (*p).stats.resident_count = 0;
        (*p).stats.wired_count = 0;
    }

    p
}

/// `pmap_destroy()` in i386/intel/pmap.c: drop a reference, freeing the page
/// tables and the map when the last one goes.
///
/// # Safety
///
/// `p` must be a live pmap or null, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_destroy(p: *mut Pmap) {
    if p.is_null() {
        return;
    }

    let spl = raise_splvm();
    // SAFETY: the caller promises a live pmap.
    let count = unsafe {
        (*p).lock.lock();
        (*p).ref_count = (*p).ref_count.wrapping_sub(1);
        let count = (*p).ref_count;
        (*p).lock.unlock();
        count
    };
    restore_spl(spl);

    if count != 0 {
        return;
    }

    #[cfg(target_arch = "x86_64")]
    {
        let mut l4i = 0;
        while l4i < NPTES {
            // SAFETY: the map owns its L4 table for as long as it exists.
            let pdp = unsafe { *(*p).l4base.wrapping_add(l4i) };
            if pdp & INTEL_PTE_VALID == 0 {
                l4i += 1;
                continue;
            }
            let pdpbase = ptetokv(pdp);
            let mut l3i = 0;
            while l3i < NPTES {
                // SAFETY: `pdpbase` is the directory pointer table the entry
                // above names.
                let pde = unsafe { *pdpbase.wrapping_add(l3i) };
                if pde & INTEL_PTE_VALID == 0 {
                    l3i += 1;
                    continue;
                }
                let pdebase = ptetokv(pde);
                if l4i < lin2l4num(VM_MAX_USER_ADDRESS)
                    || (l4i == lin2l4num(VM_MAX_USER_ADDRESS)
                        && l3i < lin2pdpnum(VM_MAX_USER_ADDRESS))
                {
                    let mut l2i = 0;
                    while l2i < NPTES {
                        // SAFETY: `pdebase` is the page directory the entry
                        // names.
                        let pte = unsafe { *pdebase.wrapping_add(l2i) };
                        if pte & INTEL_PTE_VALID != 0 {
                            // SAFETY: each valid entry names a page-table
                            // page from `pt_cache`.
                            unsafe {
                                cache_free(
                                    &raw mut PT_CACHE,
                                    ptetokv(pte).cast(),
                                )
                            };
                        }
                        l2i += 1;
                    }
                }
                // SAFETY: `pdebase` came from `pd_cache`.
                unsafe { cache_free(&raw mut PD_CACHE, pdebase.cast()) };
                l3i += 1;
            }
            // SAFETY: `pdpbase` came from `pdpt_cache`.
            unsafe { cache_free(&raw mut PDPT_CACHE, pdpbase.cast()) };
            l4i += 1;
        }
        // SAFETY: the L4 table came from `l4_cache`.
        unsafe { cache_free(&raw mut L4_CACHE, (*p).l4base.cast()) };
    }

    #[cfg(target_arch = "x86")]
    {
        // SAFETY: the map owns its page directory for as long as it exists.
        let pdebase = unsafe { (*p).dirbase };
        let mut l2i = 0;
        while l2i < lin2pdenum(VM_MAX_USER_ADDRESS) {
            // SAFETY: `pdebase` is one page of `NPTES` entries.
            let pte = unsafe { *pdebase.wrapping_add(l2i) };
            if pte & INTEL_PTE_VALID != 0 {
                // SAFETY: each valid entry names a page-table page from
                // `pt_cache`.
                unsafe { cache_free(&raw mut PT_CACHE, ptetokv(pte).cast()) };
            }
            l2i += 1;
        }
        // SAFETY: the directory came from `pd_cache`.
        unsafe { cache_free(&raw mut PD_CACHE, pdebase.cast()) };
    }

    // SAFETY: the map came from `pmap_cache`.
    unsafe { cache_free(&raw mut PMAP_CACHE, p.cast()) };
}

/// `pmap_reference()` in i386/intel/pmap.c.
///
/// # Safety
///
/// `p` must be a live pmap or null, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_reference(p: *mut Pmap) {
    if p.is_null() {
        return;
    }
    let spl = raise_splvm();
    // SAFETY: the caller promises a live pmap.
    unsafe {
        (*p).lock.lock();
        (*p).ref_count = (*p).ref_count.wrapping_add(1);
        (*p).lock.unlock();
    }
    restore_spl(spl);
}

/// The `pmap_remove_range()` of i386/intel/pmap.c: drop a run of hardware
/// entries, collecting their modify and reference bits and unlinking them
/// from their pv lists.
///
/// # Safety
///
/// `pmap` must be live and locked, and `spte` to `epte` a run of its page
/// table entries, as the C's callers guaranteed.
unsafe fn remove_range(
    pmap: *mut Pmap,
    mut va: VmOffset,
    spte: *mut VmOffset,
    epte: *mut VmOffset,
) {
    let count = (epte as usize - spte as usize) / size_of::<VmOffset>();
    let end = va.wrapping_add(count * PAGE_SIZE);
    if pmap == kernel_pmap_ptr()
        && (va < unsafe { kernel_virtual_start }
            || end > unsafe { kernel_virtual_end })
    {
        // SAFETY: `Panic` does not return; the message and two values are the
        // C's.
        unsafe {
            glue::Panic(
                c"i386/intel/pmap.c".as_ptr(),
                line!() as c_int,
                c"pmap_remove_range".as_ptr(),
                c"pmap_remove_range(%lx-%lx) falls in physical memory area!\n"
                    .as_ptr(),
                va,
                end,
            )
        }
    }

    let mut num_removed: c_int = 0;
    let mut num_unwired: c_int = 0;
    let mut cpte = spte;
    while cpte < epte {
        // SAFETY: `cpte` walks the run the caller passed.
        if unsafe { *cpte } == 0 {
            cpte = cpte.wrapping_add(PTES_PER_VM_PAGE);
            va = va.wrapping_add(PAGE_SIZE);
            continue;
        }

        let pa = pte_to_pa(unsafe { *cpte });
        num_removed += 1;
        if unsafe { *cpte } & INTEL_PTE_WIRED != 0 {
            num_unwired += 1;
        }

        if !valid_page(pa) {
            let mut i = PTES_PER_VM_PAGE;
            let mut lpte = cpte;
            loop {
                // SAFETY: the entries are this page's run.
                unsafe { *lpte = 0 };
                lpte = lpte.wrapping_add(1);
                i -= 1;
                if i == 0 {
                    break;
                }
            }
            cpte = cpte.wrapping_add(PTES_PER_VM_PAGE);
            va = va.wrapping_add(PAGE_SIZE);
            continue;
        }

        let pai = vm_page::table_index(pa);
        // SAFETY: the caller holds the pmap lock, and the index is a managed
        // page's, so the lock bit exists.  The C passed the index through the
        // macro's `int` parameter.
        unsafe { bit_lock(pai as c_int, PV_LOCK_TABLE.cast()) };

        {
            let mut i = PTES_PER_VM_PAGE;
            let mut lpte = cpte;
            loop {
                // SAFETY: the attributes array has a byte per managed page,
                // and the pv lock bit serializes this byte.
                unsafe {
                    let attr = PMAP_PHYS_ATTRIBUTES.wrapping_add(pai);
                    *attr |= (*lpte as u8) & (PHYS_MODIFIED | PHYS_REFERENCED);
                    *lpte = 0;
                }
                lpte = lpte.wrapping_add(1);
                i -= 1;
                if i == 0 {
                    break;
                }
            }
        }

        // SAFETY: the pv list is locked by the bit above, and the C required
        // a non-empty head for a mapped page.
        unsafe {
            let pv_h = pv_head(pai);
            if (*pv_h).pmap.is_null() {
                glue::Panic(
                    c"i386/intel/pmap.c".as_ptr(),
                    line!() as c_int,
                    c"pmap_remove".as_ptr(),
                    c"pmap_remove: null pv_list for pai %lx at va %lx!"
                        .as_ptr(),
                    pai,
                    va,
                )
            }
            if (*pv_h).va == va && (*pv_h).pmap == pmap {
                let cur = (*pv_h).next;
                if !cur.is_null() {
                    ptr::copy_nonoverlapping(cur, pv_h, 1);
                    pv_free(cur);
                } else {
                    (*pv_h).pmap = PMAP_NULL;
                }
            } else {
                let mut cur = pv_h;
                let mut prev;
                loop {
                    prev = cur;
                    cur = (*prev).next;
                    if cur.is_null() {
                        glue::Panic(
                            c"i386/intel/pmap.c".as_ptr(),
                            line!() as c_int,
                            c"pmap_remove".as_ptr(),
                            c"pmap-remove: mapping not in pv_list!".as_ptr(),
                        )
                    }
                    if (*cur).va == va && (*cur).pmap == pmap {
                        break;
                    }
                }
                (*prev).next = (*cur).next;
                pv_free(cur);
            }
            bit_unlock(pai as c_int, PV_LOCK_TABLE.cast());
        }

        cpte = cpte.wrapping_add(PTES_PER_VM_PAGE);
        va = va.wrapping_add(PAGE_SIZE);
    }

    // SAFETY: the caller holds the pmap lock, and the counts are this run's.
    unsafe {
        (*pmap).stats.resident_count =
            (*pmap).stats.resident_count.wrapping_sub(num_removed);
        (*pmap).stats.wired_count =
            (*pmap).stats.wired_count.wrapping_sub(num_unwired);
    }
}

/// `pmap_remove()` in i386/intel/pmap.c: remove every mapping in a range.
///
/// # Safety
///
/// `map` must be a live pmap or null, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_remove(
    map: *mut Pmap,
    s: VmOffset,
    e: VmOffset,
) {
    // SAFETY: the caller promises a live map or null.
    unsafe { remove(map, s, e) };
}

/// # Safety
///
/// As `pmap_remove`.
unsafe fn remove(map: *mut Pmap, mut s: VmOffset, e: VmOffset) {
    if map.is_null() {
        return;
    }
    let start = s;

    // SAFETY: the caller promises a live map.
    let spl = unsafe { read_lock(map) };
    while s < e {
        // SAFETY: the map is live and locked.
        let pde = unsafe { pde_of(map, s) };
        let mut l = s.wrapping_add(PDE_MAPPED_SIZE) & !(PDE_MAPPED_SIZE - 1);
        if l > e || l < s {
            l = e;
        }
        if !pde.is_null() && unsafe { *pde } & INTEL_PTE_VALID != 0 {
            // SAFETY: the entry is valid and names a page table, as the C
            // relied on for its own pointer arithmetic.
            let mut spte = ptetokv(unsafe { *pde });
            spte = spte.wrapping_add(ptenum(s));
            let epte = spte.wrapping_add((l - s) >> PAGE_SHIFT);
            // SAFETY: the run is inside that page table, as the C computed.
            unsafe { remove_range(map, s, spte, epte) };
        }
        s = l;
    }
    // SAFETY: `map` is live and held as `read_lock()` left it.
    unsafe { update_tlbs(map, start, e) };
    unsafe { read_unlock(map, spl) };
}

/// `pmap_page_protect()` in i386/intel/pmap.c: lower the permission of every
/// mapping of a physical page.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, as the C's callers
/// guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_page_protect(phys: VmOffset, prot: c_int) {
    // SAFETY: the caller promises a physical address, and the C's `prot` is
    // a `vm_prot_t`.
    unsafe { page_protect(phys, prot) };
}

/// The `VM_PROT_READ|VM_PROT_EXECUTE` case of `pmap_page_protect()`.
const VM_PROT_READ_EXECUTE: c_int = VM_PROT_READ | VM_PROT_EXECUTE;
/// The `VM_PROT_READ|VM_PROT_WRITE` case of `pmap_protect()`.
const VM_PROT_READ_WRITE: c_int = VM_PROT_READ | VM_PROT_WRITE;

/// # Safety
///
/// As `pmap_page_protect`.
unsafe fn page_protect(phys: VmOffset, prot: c_int) {
    if !valid_page(phys) {
        return;
    }

    let remove = match prot {
        VM_PROT_READ | VM_PROT_READ_EXECUTE => false,
        VM_PROT_ALL => return,
        _ => true,
    };

    let spl = write_lock();
    let pai = vm_page::table_index(phys);
    let pv_h = pv_head(pai);

    // SAFETY: the pmap system is locked for write, so the pv list is stable.
    if !unsafe { (*pv_h).pmap }.is_null() {
        let mut prev = pv_h;
        let mut pv_e = pv_h;
        loop {
            let pmap = unsafe { (*pv_e).pmap };
            // SAFETY: the pmap came from the pv list, which holds live maps.
            unsafe { (*pmap).lock.lock() };
            let va = unsafe { (*pv_e).va };
            // SAFETY: the entry is live and locked.
            let pte = unsafe { pte_of(pmap, va) };

            if remove || pmap == kernel_pmap_ptr() {
                if unsafe { *pte } & INTEL_PTE_WIRED != 0 {
                    // SAFETY: the map is live and locked.
                    unsafe { (*pmap).stats.wired_count -= 1 };
                }
                let mut i = PTES_PER_VM_PAGE;
                let mut p = pte;
                loop {
                    // SAFETY: the pv lock is covered by the system write
                    // lock here, and `p` walks the page's run.
                    unsafe {
                        let attr = PMAP_PHYS_ATTRIBUTES.wrapping_add(pai);
                        *attr |=
                            (*p as u8) & (PHYS_MODIFIED | PHYS_REFERENCED);
                        *p = 0;
                    }
                    p = p.wrapping_add(1);
                    i -= 1;
                    if i == 0 {
                        break;
                    }
                }
                // SAFETY: the map is live and locked.
                unsafe { (*pmap).stats.resident_count -= 1 };

                if pv_e == pv_h {
                    // SAFETY: the head itself held this mapping.
                    unsafe { (*pv_h).pmap = PMAP_NULL };
                } else {
                    // SAFETY: the entry is in the list, behind `prev`.
                    unsafe {
                        (*prev).next = (*pv_e).next;
                        pv_free(pv_e);
                    }
                }
            } else {
                let mut i = PTES_PER_VM_PAGE;
                let mut p = pte;
                loop {
                    // SAFETY: `p` walks the page's run.
                    unsafe { *p &= !INTEL_PTE_WRITE };
                    p = p.wrapping_add(1);
                    i -= 1;
                    if i == 0 {
                        break;
                    }
                }
                prev = pv_e;
            }

            // SAFETY: the map is live and locked.
            unsafe { update_tlbs(pmap, va, va.wrapping_add(PAGE_SIZE)) };
            unsafe { (*pmap).lock.unlock() };

            // SAFETY: the list link is kept under the system write lock.
            pv_e = unsafe { (*prev).next };
            if pv_e.is_null() {
                break;
            }
        }

        // SAFETY: if the walk emptied the list, its head is dead.
        if unsafe { (*pv_h).pmap }.is_null() {
            let pv_e = unsafe { (*pv_h).next };
            if !pv_e.is_null() {
                // SAFETY: the next entry becomes the new head.
                unsafe {
                    ptr::copy_nonoverlapping(pv_e, pv_h, 1);
                    pv_free(pv_e);
                }
            }
        }
    }

    write_unlock(spl);
}

/// `pmap_protect()` in i386/intel/pmap.c: lower permissions over a range.
///
/// # Safety
///
/// `map` must be a live pmap or null, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_protect(
    map: *mut Pmap,
    s: VmOffset,
    e: VmOffset,
    prot: c_int,
) {
    // SAFETY: the caller promises a live map or null.
    unsafe { protect(map, s, e, prot) };
}

/// # Safety
///
/// As `pmap_protect`.
unsafe fn protect(map: *mut Pmap, mut s: VmOffset, e: VmOffset, prot: c_int) {
    if map.is_null() {
        return;
    }

    match prot {
        VM_PROT_READ | VM_PROT_READ_EXECUTE => (),
        VM_PROT_READ_WRITE | VM_PROT_ALL => return,
        _ => {
            // SAFETY: the caller promises a live map.
            unsafe { remove(map, s, e) };
            return;
        }
    }

    let start = s;
    // The C's pre-i486 fallback, which removes kernel mappings because that
    // CPU ignores the write bit in kernel mode, is compiled out of both
    // configured builds.
    let spl = raise_splvm();
    // SAFETY: the caller promises a live map.
    unsafe { (*map).lock.lock() };

    while s < e {
        // SAFETY: the map is live and locked.
        let pde = unsafe { pde_of(map, s) };
        let mut l = s.wrapping_add(PDE_MAPPED_SIZE) & !(PDE_MAPPED_SIZE - 1);
        if l > e || l < s {
            l = e;
        }
        if !pde.is_null() && unsafe { *pde } & INTEL_PTE_VALID != 0 {
            // SAFETY: the entry is valid and names a page table.
            let mut spte = ptetokv(unsafe { *pde });
            spte = spte.wrapping_add(ptenum(s));
            let epte = spte.wrapping_add((l - s) >> PAGE_SHIFT);
            while spte < epte {
                // SAFETY: `spte` walks the page table.
                if unsafe { *spte } & INTEL_PTE_VALID != 0 {
                    unsafe { *spte &= !INTEL_PTE_WRITE };
                }
                spte = spte.wrapping_add(1);
            }
        }
        s = l;
    }
    // SAFETY: the map is live and locked.
    unsafe { update_tlbs(map, start, e) };

    unsafe { (*map).lock.unlock() };
    restore_spl(spl);
}

/// The page-table-level getter `pmap_expand_level()` calls.
type LevelGetter = unsafe fn(*mut Pmap, VmOffset) -> *mut VmOffset;

/// The `pmap_expand_level()` of i386/intel/pmap.c: allocate one level of the
/// page-table tree, unlocking the pmap around the allocation.
///
/// # Safety
///
/// `pmap` must be live and locked at `spl`, and `cache` initialized.
unsafe fn expand_level(
    pmap: *mut Pmap,
    v: VmOffset,
    spl: c_int,
    level: LevelGetter,
    upper: LevelGetter,
    n_per_vm_page: c_int,
    cache: *mut KmemCache,
) -> *mut VmOffset {
    loop {
        // SAFETY: the caller passes a live, locked map.
        let pte = unsafe { level(pmap, v) };
        if !pte.is_null() {
            return pte;
        }

        if pmap == kernel_pmap_ptr() {
            // SAFETY: `Panic` does not return; the message and value are the
            // C's.
            unsafe {
                glue::Panic(
                    c"i386/intel/pmap.c".as_ptr(),
                    line!() as c_int,
                    c"pmap_expand_level".as_ptr(),
                    c"pmap_expand kernel pmap to %#zx".as_ptr(),
                    v,
                )
            }
        }

        // SAFETY: the caller holds the map's lock.
        unsafe { read_unlock(pmap, spl) };
        let ptp = loop {
            // SAFETY: the caller promises the cache is initialized;
            // `cache_alloc` takes its lock.
            let ptp = unsafe { cache_alloc(cache) };
            if !ptp.is_null() {
                break ptp;
            }
            // SAFETY: as the C's `VM_PAGE_WAIT`, the allocation can sleep.
            unsafe { vm_page::wait(None) };
        };
        // SAFETY: the cache returned one `PAGE_SIZE` object for a table.
        unsafe { ptr::write_bytes(ptp, 0, PAGE_SIZE) };

        let spl = unsafe { read_lock(pmap) };
        if !unsafe { level(pmap, v) }.is_null() {
            // SAFETY: the lock was taken above.
            unsafe { read_unlock(pmap, spl) };
            unsafe { cache_free(cache, ptp) };
            let _spl = unsafe { read_lock(pmap) };
            continue;
        }

        let mut i = n_per_vm_page;
        // SAFETY: the upper level now holds the entry.
        let mut pdp = unsafe { upper(pmap, v) };
        let mut table = ptp;
        loop {
            // SAFETY: `pdp` walks the upper level's run for this page, and
            // `table` the fresh page-table pages.
            unsafe {
                *pdp = pa_to_pte(kvtophys(table.addr()))
                    | INTEL_PTE_VALID
                    | (if pmap != kernel_pmap_ptr() {
                        INTEL_PTE_USER
                    } else {
                        0
                    })
                    | INTEL_PTE_WRITE;
            }
            pdp = pdp.wrapping_add(1);
            table = table.wrapping_add(PAGE_SIZE);
            i -= 1;
            if i == 0 {
                break;
            }
        }
    }
}

/// The `pmap_expand()` of i386/intel/pmap.c: grow every level the address
/// needs.
///
/// # Safety
///
/// `pmap` must be live and locked at `spl`.
unsafe fn expand(pmap: *mut Pmap, v: VmOffset, spl: c_int) -> *mut VmOffset {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the caller passes a live, locked map and initialized
        // caches.
        unsafe {
            expand_level(
                pmap,
                v,
                spl,
                ptp_of,
                l4base_of,
                1,
                &raw mut PDPT_CACHE,
            );
            expand_level(pmap, v, spl, pde_of, ptp_of, 1, &raw mut PD_CACHE);
        }
    }
    // SAFETY: as above, for the level-1 table.
    unsafe {
        expand_level(
            pmap,
            v,
            spl,
            pte_of,
            pde_of,
            PTES_PER_VM_PAGE as c_int,
            &raw mut PT_CACHE,
        )
    }
}

/// The CPU type the machine probe recorded for the running CPU.
fn cpu_type() -> c_int {
    let cpu = cpu_number();
    debug_assert!(cpu >= 0 && (cpu as usize) < NCPUS);
    // SAFETY: `cpu_number()` is below the configured `NCPUS`, so the slot is
    // one the probe filled before any pmap call.
    unsafe { (*machine_slot(cpu as usize)).cpu_type }
}

/// The entry template `pmap_enter()` builds.
fn enter_template(
    pmap: *mut Pmap,
    pa: VmOffset,
    prot: c_int,
    wired: bool,
    is_physmem: bool,
) -> VmOffset {
    let mut template = pa_to_pte(pa) | INTEL_PTE_VALID;
    if pmap != kernel_pmap_ptr() {
        template |= INTEL_PTE_USER;
    }
    if prot & VM_PROT_WRITE != 0 {
        template |= INTEL_PTE_WRITE;
    }
    if cpu_type() >= CPU_TYPE_I486 && !is_physmem {
        template |= INTEL_PTE_NCACHE | INTEL_PTE_WTHRU;
    }
    if wired {
        template |= INTEL_PTE_WIRED;
    }
    template
}

/// `pmap_enter()` in i386/intel/pmap.c: insert one mapping.
///
/// # Safety
///
/// `pmap` must be a live pmap or null, and `pa` a physical address the kernel
/// may map, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_enter(
    pmap: *mut Pmap,
    v: VmOffset,
    pa: VmOffset,
    prot: c_int,
    wired: c_int,
) {
    // SAFETY: the caller promises a live map or null.
    unsafe { enter(pmap, v, pa, prot, wired != 0) };
}

/// # Safety
///
/// As `pmap_enter`.
unsafe fn enter(
    pmap: *mut Pmap,
    v: VmOffset,
    pa: VmOffset,
    prot: c_int,
    wired: bool,
) {
    if PMAP_DEBUG.load(Ordering::Relaxed) != 0 {
        // SAFETY: `printf` is the real routine and this format has two
        // integer conversions for the two values.
        unsafe { glue::printf(c"pmap(%zx, %llx)\n".as_ptr(), v, pa) };
    }
    if pmap.is_null() {
        return;
    }

    if pmap == kernel_pmap_ptr()
        && (v < unsafe { kernel_virtual_start }
            || v >= unsafe { kernel_virtual_end })
    {
        // SAFETY: `Panic` does not return; the message and values are the
        // C's.
        unsafe {
            glue::Panic(
                c"i386/intel/pmap.c".as_ptr(),
                line!() as c_int,
                c"pmap_enter".as_ptr(),
                c"pmap_enter(%lx, %llx) falls in physical memory area!\n"
                    .as_ptr(),
                v,
                pa,
            )
        }
    }

    let mut pv_e: *mut PvEntry = ptr::null_mut();
    loop {
        // SAFETY: the caller promises a live map.
        let spl = unsafe { read_lock(pmap) };
        // SAFETY: the map is live and locked.
        let mut pte = unsafe { expand(pmap, v, spl) };

        let is_physmem = if vm_page::is_ready() {
            vm_page::lookup_pa(pa).is_some()
        } else {
            pa < biosmem::directmap_end()
        };

        let old_pa = pte_to_pa(unsafe { *pte });
        if unsafe { *pte } != 0 && old_pa == pa {
            if wired && unsafe { *pte } & INTEL_PTE_WIRED == 0 {
                // SAFETY: the map is live and locked.
                unsafe { (*pmap).stats.wired_count += 1 };
            } else if !wired && unsafe { *pte } & INTEL_PTE_WIRED != 0 {
                unsafe { (*pmap).stats.wired_count -= 1 };
            }

            let mut template =
                enter_template(pmap, pa, prot, wired, is_physmem);
            let mut i = PTES_PER_VM_PAGE;
            loop {
                if unsafe { *pte } & INTEL_PTE_MOD != 0 {
                    template |= INTEL_PTE_MOD;
                }
                // SAFETY: `pte` walks the page's run.
                unsafe { *pte = template };
                pte = pte.wrapping_add(1);
                template = template.wrapping_add(PAGE_SIZE);
                i -= 1;
                if i == 0 {
                    break;
                }
            }
            // SAFETY: the map is live and locked.
            unsafe { update_tlbs(pmap, v, v.wrapping_add(PAGE_SIZE)) };
        } else {
            if unsafe { *pte } != 0 {
                // SAFETY: `pte` names the entry run to replace.
                unsafe {
                    remove_range(
                        pmap,
                        v,
                        pte,
                        pte.wrapping_add(PTES_PER_VM_PAGE),
                    )
                };
                unsafe { update_tlbs(pmap, v, v.wrapping_add(PAGE_SIZE)) };
            }

            if valid_page(pa) {
                let pai = vm_page::table_index(pa);
                // SAFETY: the map is live and locked, and the index is a
                // managed page's.
                unsafe { bit_lock(pai as c_int, PV_LOCK_TABLE.cast()) };
                let pv_h = pv_head(pai);

                if unsafe { (*pv_h).pmap }.is_null() {
                    // SAFETY: the head was empty and the pv lock is held.
                    unsafe {
                        (*pv_h).va = v;
                        (*pv_h).pmap = pmap;
                        (*pv_h).next = ptr::null_mut();
                    }
                } else {
                    if pv_e.is_null() {
                        pv_e = pv_alloc();
                        if pv_e.is_null() {
                            // SAFETY: the pv lock was taken above.
                            unsafe {
                                bit_unlock(pai as c_int, PV_LOCK_TABLE.cast());
                                read_unlock(pmap, spl);
                            }
                            // SAFETY: the C refilled from the slab cache
                            // while unlocked.
                            pv_e = unsafe {
                                cache_alloc(&raw mut PV_LIST_CACHE)
                                    .cast::<PvEntry>()
                            };
                            continue;
                        }
                    }
                    // SAFETY: the head is non-empty and the new entry is
                    // this function's.
                    unsafe {
                        (*pv_e).va = v;
                        (*pv_e).pmap = pmap;
                        (*pv_e).next = (*pv_h).next;
                        (*pv_h).next = pv_e;
                    }
                    pv_e = ptr::null_mut();
                }
                // SAFETY: the pv lock was taken above.
                unsafe { bit_unlock(pai as c_int, PV_LOCK_TABLE.cast()) };
            }

            // SAFETY: the map is live and locked.
            unsafe {
                (*pmap).stats.resident_count += 1;
                if wired {
                    (*pmap).stats.wired_count += 1;
                }
            }

            let mut template =
                enter_template(pmap, pa, prot, wired, is_physmem);
            let mut i = PTES_PER_VM_PAGE;
            loop {
                // SAFETY: `pte` walks the page's run.
                unsafe { *pte = template };
                pte = pte.wrapping_add(1);
                template = template.wrapping_add(PAGE_SIZE);
                i -= 1;
                if i == 0 {
                    break;
                }
            }
        }

        if !pv_e.is_null() {
            pv_free(pv_e);
        }
        unsafe { read_unlock(pmap, spl) };
        break;
    }
}

/// `pmap_change_wiring()` in i386/intel/pmap.c: set the wired bit on an
/// existing mapping.
///
/// # Safety
///
/// `map` must be live and hold a mapping at `v`, as the C's callers
/// guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_change_wiring(
    map: *mut Pmap,
    v: VmOffset,
    wired: c_int,
) {
    // SAFETY: the caller promises a live map.
    unsafe { change_wiring(map, v, wired != 0) };
}

/// # Safety
///
/// As `pmap_change_wiring`.
unsafe fn change_wiring(map: *mut Pmap, v: VmOffset, wired: bool) {
    // SAFETY: the caller promises a live map.
    let spl = unsafe { read_lock(map) };
    // SAFETY: the map is live and locked.
    let pte = unsafe { pte_of(map, v) };
    if pte.is_null() {
        // SAFETY: `Panic` does not return; the message is the C's.
        unsafe {
            glue::Panic(
                c"i386/intel/pmap.c".as_ptr(),
                line!() as c_int,
                c"pmap_change_wiring".as_ptr(),
                c"pmap_change_wiring: pte missing".as_ptr(),
            )
        }
    }

    if wired && unsafe { *pte } & INTEL_PTE_WIRED == 0 {
        // SAFETY: the map is live and locked.
        unsafe { (*map).stats.wired_count += 1 };
        let mut i = PTES_PER_VM_PAGE;
        let mut p = pte;
        loop {
            // SAFETY: `p` walks the page's run.
            unsafe { *p |= INTEL_PTE_WIRED };
            p = p.wrapping_add(1);
            i -= 1;
            if i == 0 {
                break;
            }
        }
    } else if !wired && unsafe { *pte } & INTEL_PTE_WIRED != 0 {
        unsafe { (*map).stats.wired_count -= 1 };
        let mut i = PTES_PER_VM_PAGE;
        let mut p = pte;
        loop {
            // SAFETY: `p` walks the page's run.
            unsafe { *p &= !INTEL_PTE_WIRED };
            p = p.wrapping_add(1);
            i -= 1;
            if i == 0 {
                break;
            }
        }
    }

    unsafe { read_unlock(map, spl) };
}

/// `pmap_extract()` in i386/intel/pmap.c: the physical address a mapping
/// holds, or zero.
///
/// # Safety
///
/// `pmap` must be a live pmap, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_extract(
    pmap: *mut Pmap,
    va: VmOffset,
) -> VmOffset {
    // SAFETY: the caller promises a live map.
    unsafe { extract(pmap, va) }
}

/// # Safety
///
/// As `pmap_extract`.
unsafe fn extract(pmap: *mut Pmap, va: VmOffset) -> VmOffset {
    let spl = raise_splvm();
    // SAFETY: the caller promises a live map.
    unsafe { (*pmap).lock.lock() };
    // SAFETY: the map is live and locked.
    let pte = unsafe { pte_of(pmap, va) };
    let pa = if pte.is_null() || unsafe { *pte } & INTEL_PTE_VALID == 0 {
        0
    } else {
        pte_to_pa(unsafe { *pte }) + (va & INTEL_OFFMASK)
    };
    unsafe { (*pmap).lock.unlock() };
    restore_spl(spl);
    pa
}

/// `pmap_collect()` in i386/intel/pmap.c: free the user page tables of a map
/// whose pages are scarce.
///
/// # Safety
///
/// `p` must be a live user pmap or null, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_collect(p: *mut Pmap) {
    // SAFETY: the caller promises a live map or null.
    unsafe { collect(p) };
}

/// # Safety
///
/// As `pmap_collect`.
unsafe fn collect(p: *mut Pmap) {
    if p.is_null() {
        return;
    }
    if p == kernel_pmap_ptr() {
        return;
    }

    // SAFETY: the caller promises a live map.
    let mut spl = unsafe { read_lock(p) };

    #[cfg(target_arch = "x86_64")]
    {
        let mut l4i = 0;
        while l4i < lin2l4num(VM_MAX_USER_ADDRESS) {
            // SAFETY: the map holds its L4 table while it exists.
            let pdp = unsafe { *(*p).l4base.wrapping_add(l4i) };
            if pdp & INTEL_PTE_VALID == 0 {
                l4i += 1;
                continue;
            }
            let pdpbase = ptetokv(pdp);
            let mut l3i = 0;
            while l3i < NPTES {
                // SAFETY: `pdpbase` is the table the entry names.
                let pde = unsafe { *pdpbase.wrapping_add(l3i) };
                if pde & INTEL_PTE_VALID == 0 {
                    l3i += 1;
                    continue;
                }
                let pdebase = ptetokv(pde);
                let mut l2i = 0;
                while l2i < NPTES {
                    // SAFETY: `pdebase` is the directory the entry names.
                    let pte = unsafe { *pdebase.wrapping_add(l2i) };
                    if pte & INTEL_PTE_VALID == 0 {
                        l2i += 1;
                        continue;
                    }
                    let ptp = with_exposed_provenance_mut::<VmOffset>(
                        phystokv(pte_to_pa(pte)),
                    );
                    let eptp = ptp.wrapping_add(NPTES * PTES_PER_VM_PAGE);

                    let mut wired = false;
                    let mut ptep = ptp;
                    while ptep < eptp {
                        // SAFETY: `ptep` walks the page table.
                        if unsafe { *ptep } & INTEL_PTE_WIRED != 0 {
                            wired = true;
                            break;
                        }
                        ptep = ptep.wrapping_add(1);
                    }

                    if !wired {
                        let mut va = pagenum2lin(l4i, l3i, l2i, 0);
                        if p == kernel_pmap_ptr() {
                            va = lintokv(va);
                        }
                        // SAFETY: the map is live and locked, and the run is
                        // the page table just walked.
                        unsafe { remove_range(p, va, ptp, eptp) };

                        let mut i = PTES_PER_VM_PAGE;
                        let mut pdep = pdebase.wrapping_add(l2i);
                        loop {
                            // SAFETY: `pdep` walks the directory's run.
                            unsafe { *pdep = 0 };
                            pdep = pdep.wrapping_add(1);
                            i -= 1;
                            if i == 0 {
                                break;
                            }
                        }

                        // SAFETY: the lock was taken above.
                        unsafe { read_unlock(p, spl) };
                        unsafe {
                            cache_free(&raw mut PT_CACHE, ptetokv(pte).cast())
                        };
                        spl = unsafe { read_lock(p) };
                    }
                    l2i += 1;
                }
                l3i += 1;
            }
            l4i += 1;
        }
    }

    #[cfg(target_arch = "x86")]
    {
        // SAFETY: the map holds its directory while it exists.
        let pdebase = unsafe { (*p).dirbase };
        let mut l2i = 0;
        while l2i < lin2pdenum(VM_MAX_USER_ADDRESS) {
            // SAFETY: `pdebase` is one page of `NPTES` entries.
            let pte = unsafe { *pdebase.wrapping_add(l2i) };
            if pte & INTEL_PTE_VALID == 0 {
                l2i += 1;
                continue;
            }
            let ptp = with_exposed_provenance_mut::<VmOffset>(phystokv(
                pte_to_pa(pte),
            ));
            let eptp = ptp.wrapping_add(NPTES * PTES_PER_VM_PAGE);

            let mut wired = false;
            let mut ptep = ptp;
            while ptep < eptp {
                if unsafe { *ptep } & INTEL_PTE_WIRED != 0 {
                    wired = true;
                    break;
                }
                ptep = ptep.wrapping_add(1);
            }

            if !wired {
                let va = pagenum2lin(0, 0, l2i, 0);
                // SAFETY: the map is live and locked, and the run is the page
                // table just walked.
                unsafe { remove_range(p, va, ptp, eptp) };

                let mut i = PTES_PER_VM_PAGE;
                let mut pdep = pdebase.wrapping_add(l2i);
                loop {
                    // SAFETY: `pdep` walks the directory's run.
                    unsafe { *pdep = 0 };
                    pdep = pdep.wrapping_add(1);
                    i -= 1;
                    if i == 0 {
                        break;
                    }
                }

                unsafe { read_unlock(p, spl) };
                unsafe { cache_free(&raw mut PT_CACHE, ptetokv(pte).cast()) };
                spl = unsafe { read_lock(p) };
            }
            l2i += 1;
        }
    }

    // SAFETY: the map is live and locked.
    unsafe { update_tlbs(p, VM_MIN_USER_ADDRESS, VM_MAX_USER_ADDRESS) };
    unsafe { read_unlock(p, spl) };
}

/// The `phys_attribute_clear()` of i386/intel/pmap.c: clear modify or
/// reference bits on every mapping of a page.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, and `bits` the C
/// caller's attribute mask.
unsafe fn attribute_clear(phys: VmOffset, bits: c_int) {
    if !valid_page(phys) {
        return;
    }

    let spl = write_lock();
    let pai = vm_page::table_index(phys);
    let pv_h = pv_head(pai);

    // SAFETY: the pmap system is locked for write, so the pv list is stable.
    if !unsafe { (*pv_h).pmap }.is_null() {
        let mut pv_e = pv_h;
        while !pv_e.is_null() {
            let pmap = unsafe { (*pv_e).pmap };
            // SAFETY: the pmap came from the pv list, which holds live maps.
            unsafe { (*pmap).lock.lock() };
            let va = unsafe { (*pv_e).va };
            // SAFETY: the entry is live and locked.
            let pte = unsafe { pte_of(pmap, va) };

            for _ in 0..PTES_PER_VM_PAGE {
                // SAFETY: the pv lock is covered by the system write lock
                // here, and the C's loop cleared the same entry each time.
                unsafe { *pte &= !(bits as VmOffset) };
            }

            // SAFETY: the map is live and locked.
            unsafe { update_tlbs(pmap, va, va.wrapping_add(PAGE_SIZE)) };
            unsafe { (*pmap).lock.unlock() };

            // SAFETY: the list link is kept under the system write lock.
            pv_e = unsafe { (*pv_e).next };
        }
    }

    // SAFETY: the attribute byte is serialized by the system write lock.
    unsafe { *PMAP_PHYS_ATTRIBUTES.wrapping_add(pai) &= !(bits as u8) };

    write_unlock(spl);
}

/// The `phys_attribute_test()` of i386/intel/pmap.c: whether any mapping of
/// a page has the bits set.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, and `bits` the C
/// caller's attribute mask.
unsafe fn attribute_test(phys: VmOffset, bits: c_int) -> bool {
    if !valid_page(phys) {
        return false;
    }

    let spl = write_lock();
    let pai = vm_page::table_index(phys);
    let pv_h = pv_head(pai);

    // SAFETY: the attribute byte is serialized by the system write lock.
    if unsafe { *PMAP_PHYS_ATTRIBUTES.wrapping_add(pai) } as c_int & bits != 0
    {
        write_unlock(spl);
        return true;
    }

    // SAFETY: the pmap system is locked for write, so the pv list is stable.
    if !unsafe { (*pv_h).pmap }.is_null() {
        let mut pv_e = pv_h;
        while !pv_e.is_null() {
            let pmap = unsafe { (*pv_e).pmap };
            // SAFETY: the pmap came from the pv list, which holds live maps.
            unsafe { (*pmap).lock.lock() };
            let va = unsafe { (*pv_e).va };
            // SAFETY: the entry is live and locked.
            let pte = unsafe { pte_of(pmap, va) };

            for _ in 0..PTES_PER_VM_PAGE {
                // SAFETY: the C's loop checked the same entry each time.
                if unsafe { *pte } & (bits as VmOffset) != 0 {
                    unsafe { (*pmap).lock.unlock() };
                    write_unlock(spl);
                    return true;
                }
            }

            unsafe { (*pmap).lock.unlock() };

            // SAFETY: the list link is kept under the system write lock.
            pv_e = unsafe { (*pv_e).next };
        }
    }

    write_unlock(spl);
    false
}

/// `pmap_clear_modify()` of <vm/pmap.h>: clear the modify bits of a page.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, as the C's callers
/// guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_clear_modify(phys: VmOffset) {
    // SAFETY: the caller promises a physical address.
    unsafe { attribute_clear(phys, c_int::from(PHYS_MODIFIED)) };
}

/// `pmap_is_modified()` of <vm/pmap.h>: whether a page was modified.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, as the C's callers
/// guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_is_modified(phys: VmOffset) -> c_int {
    // SAFETY: the caller promises a physical address.
    c_int::from(unsafe { attribute_test(phys, c_int::from(PHYS_MODIFIED)) })
}

/// `pmap_clear_reference()` of <vm/pmap.h>: clear the reference bits of a
/// page.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, as the C's callers
/// guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_clear_reference(phys: VmOffset) {
    // SAFETY: the caller promises a physical address.
    unsafe { attribute_clear(phys, c_int::from(PHYS_REFERENCED)) };
}

/// `pmap_is_referenced()` of <vm/pmap.h>: whether a page was referenced.
///
/// # Safety
///
/// `phys` must be a physical address the kernel may map, as the C's callers
/// guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_is_referenced(phys: VmOffset) -> c_int {
    // SAFETY: the caller promises a physical address.
    c_int::from(unsafe { attribute_test(phys, c_int::from(PHYS_REFERENCED)) })
}

/// `__builtin_ffs()`: the one-based index of the lowest set bit, or zero.
fn ffs(value: isize) -> u32 {
    if value == 0 {
        0
    } else {
        value.trailing_zeros() + 1
    }
}

/// The update list of one CPU.
fn update_list(cpu: c_int) -> *mut PmapUpdateList {
    // SAFETY: taking the address of the array element does not read it, and
    // CPU numbers are below NCPUS.
    unsafe { &raw mut CPU_UPDATE_LIST[cpu as usize] }
}

/// The `signal_cpus()` of i386/intel/pmap.c: queue an invalidation for every
/// CPU in `use_list` and interrupt the ones that are awake.
///
/// # Safety
///
/// `pmap` must be a live, locked map and the range one it was changed in, as
/// the C's `PMAP_UPDATE_TLBS` required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn signal_cpus(
    use_list: isize,
    pmap: *mut Pmap,
    start: VmOffset,
    end: VmOffset,
) {
    let mut use_list = use_list;
    loop {
        let which = ffs(use_list);
        if which == 0 {
            break;
        }
        let which_cpu = (which - 1) as c_int;
        let update_list_p = update_list(which_cpu);

        // SAFETY: each update list has its own lock, live from `pmap_init()`.
        unsafe { (*update_list_p).lock.lock() };
        // SAFETY: the lock above serializes the list.
        let j = unsafe { (*update_list_p).count };
        if j >= UPDATE_LIST_SIZE as c_int {
            // SAFETY: the last entry becomes the whole-space flush, as the
            // C made it.
            unsafe {
                (*update_list_p).item[UPDATE_LIST_SIZE - 1].pmap =
                    kernel_pmap_ptr();
                (*update_list_p).item[UPDATE_LIST_SIZE - 1].start =
                    VM_MIN_USER_ADDRESS;
                (*update_list_p).item[UPDATE_LIST_SIZE - 1].end =
                    VM_MAX_KERNEL_ADDRESS;
            }
        } else {
            // SAFETY: `j` is below the list's capacity.
            unsafe {
                (*update_list_p).item[j as usize].pmap = pmap;
                (*update_list_p).item[j as usize].start = start;
                (*update_list_p).item[j as usize].end = end;
                (*update_list_p).count = j + 1;
            }
        }
        // SAFETY: the target CPU publishes the request with the same store.
        cpu_update_needed[which_cpu as usize].store(1, Ordering::Relaxed);
        unsafe { (*update_list_p).lock.unlock() };

        fence(Ordering::SeqCst);
        if cpus_idle.bits() & CpuSet::mask(which_cpu) == 0 {
            interrupt_processor(which_cpu);
        }
        use_list &= !CpuSet::mask(which_cpu);
    }
}

/// `process_pmap_updates()` of <i386/pmap.h>: flush the calling CPU's queued
/// invalidations.
///
/// # Safety
///
/// Must be called at `SPLVM`, with the caller's pmap live if it is not the
/// kernel map, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_pmap_updates(my_pmap: *mut Pmap) {
    let my_cpu = cpu_number();
    let update_list_p = update_list(my_cpu);

    // SAFETY: each update list has its own lock.
    unsafe { (*update_list_p).lock.lock() };
    let mut j = 0;
    while j < unsafe { (*update_list_p).count } {
        let pmap = unsafe { (*update_list_p).item[j as usize].pmap };
        if pmap == my_pmap || pmap == kernel_pmap_ptr() {
            let start = unsafe { (*update_list_p).item[j as usize].start };
            let end = unsafe { (*update_list_p).item[j as usize].end };
            // SAFETY: the queued range belongs to `pmap`, which the queueing
            // CPU held locked.
            unsafe { invalidate_tlb(pmap, start, end) };
        }
        j += 1;
    }
    // SAFETY: the lock above serializes the list.
    unsafe {
        (*update_list_p).count = 0;
    }
    cpu_update_needed[my_cpu as usize].store(0, Ordering::Relaxed);
    unsafe { (*update_list_p).lock.unlock() };
}

/// The `current_pmap()` of i386/intel/pmap.c.
///
/// # Safety
///
/// `thread` must be a live thread whose task has a map, as the C's interrupt
/// path ensured before calling.
unsafe fn current_pmap(thread: *mut Thread) -> *mut Pmap {
    // SAFETY: the caller promises a live thread and its task chain.
    unsafe {
        let task = (*thread).task;
        let map = (*task).map.cast::<VmMap>();
        (*map).pmap
    }
}

/// `pmap_update_interrupt()` of <i386/pmap.h>: the interprocessor handler
/// that flushes this CPU's TLB for another.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_update_interrupt() {
    let my_cpu = cpu_number();

    if cpus_idle.bits() & CpuSet::mask(my_cpu) != 0 {
        return;
    }

    let thread = current_thread();
    let mut my_pmap = kernel_pmap_ptr();
    if !thread.is_null() {
        // SAFETY: the running thread is live, and `current_pmap()` follows
        // its task's map.
        my_pmap = unsafe { current_pmap(thread) };
        // SAFETY: the pmap came from a live map, and the interrupt runs at
        // `SPLIP` so the field cannot change under it.
        if !unsafe { (*my_pmap).cpus_using.contains(my_cpu) } {
            my_pmap = kernel_pmap_ptr();
        }
    }

    // SAFETY: `splvm` is the real routine <i386/spl.h> declares.
    let s = unsafe { glue::splvm() };
    loop {
        cpus_active.clear(my_cpu);
        // SAFETY: both maps are the kernel's and a live user map; the spin
        // is the C's, waiting for updates in progress.
        while unsafe {
            (*my_pmap).lock.is_locked()
                || (*kernel_pmap_ptr()).lock.is_locked()
        } {
            core::hint::spin_loop();
        }
        // SAFETY: this runs at `SPLVM` with the interrupt blocked, as the C
        // did.
        unsafe { process_pmap_updates(my_pmap) };
        cpus_active.set(my_cpu);
        if cpu_update_needed[my_cpu as usize].load(Ordering::Relaxed) == 0 {
            break;
        }
    }
    // SAFETY: `s` came from `splvm()`.
    unsafe { glue::splx(s) };
}

/// `pmap_make_temporary_mapping()` of <i386/pmap.h>: mirror the low linear
/// kernel mapping into the high one while the boot moves between them.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_make_temporary_mapping() {
    #[cfg(target_arch = "x86")]
    {
        let delta =
            INIT_VM_MIN_KERNEL_ADDRESS.wrapping_sub(LINEAR_MIN_KERNEL_ADDRESS);
        let delta = if delta.wrapping_neg() < delta {
            delta.wrapping_neg()
        } else {
            delta
        };
        let nb_direct = delta >> PDESHIFT;
        // SAFETY: the kernel directory is live from `pmap_bootstrap()`, and
        // this runs before paging uses the new low mapping.
        unsafe {
            let directory = KERNEL_PAGE_DIR;
            for i in 0..nb_direct {
                let src = directory.wrapping_add(
                    lin2pdenum_cont(LINEAR_MIN_KERNEL_ADDRESS) + i,
                );
                let dst = directory.wrapping_add(
                    lin2pdenum_cont(INIT_VM_MIN_KERNEL_ADDRESS) + i,
                );
                *dst = *src;
            }
        }
    }
}

/// `pmap_set_page_dir()` of <i386/pmap.h>: load the kernel's page tables
/// into CR3 and turn on the paging features they need.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_set_page_dir() {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the kernel map holds its L4 table from `pmap_bootstrap()`.
        let physical =
            kvtophys_early(unsafe { (*kernel_pmap_ptr()).l4base }.addr());
        write_cr3(physical);
        if !cpu_has_feature(CPU_FEATURE_PAE) {
            // SAFETY: `Panic` does not return; the message is the C's.
            unsafe {
                glue::Panic(
                    c"i386/intel/pmap.c".as_ptr(),
                    line!() as c_int,
                    c"pmap_set_page_dir".as_ptr(),
                    c"CPU doesn't have support for PAE.".as_ptr(),
                )
            }
        }
        write_cr4(read_cr4() | CR4_PAE);
    }
    #[cfg(target_arch = "x86")]
    {
        // SAFETY: the kernel directory is live from `pmap_bootstrap()`.
        let physical = kvtophys_early(unsafe { KERNEL_PAGE_DIR }.addr());
        write_cr3(physical);
    }
}

/// `pmap_remove_temporary_mapping()` of <i386/pmap.h>: drop the boot-time
/// low mapping and flush it from the TLB.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_remove_temporary_mapping() {
    #[cfg(target_arch = "x86")]
    {
        let delta =
            INIT_VM_MIN_KERNEL_ADDRESS.wrapping_sub(LINEAR_MIN_KERNEL_ADDRESS);
        let delta = if delta.wrapping_neg() < delta {
            delta.wrapping_neg()
        } else {
            delta
        };
        let nb_direct = delta >> PDESHIFT;
        // SAFETY: the kernel directory is live from `pmap_bootstrap()`.
        unsafe {
            let directory = KERNEL_PAGE_DIR;
            for i in 0..nb_direct {
                let pde = directory.wrapping_add(
                    lin2pdenum_cont(INIT_VM_MIN_KERNEL_ADDRESS) + i,
                );
                *pde = 0;
            }
        }
    }
    flush_tlb();
}
