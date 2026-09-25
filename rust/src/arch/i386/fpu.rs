// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/fpu.c and i386/i386/fpu.h:
//   Copyright (c) 1992-1990 Carnegie Mellon University
//   Copyright (C) 1994 Linus Torvalds
// Derived from i386/include/mach/i386/fp_reg.h and
// i386/include/mach/i386/thread_status.h:
//   Copyright (c) 1992-1989 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The FPU save areas and the FPU traps, which `i386/i386/fpu.c` used to
//! define and `i386/i386/fpu.h`,
//! `i386/include/mach/i386/fp_reg.h` and
//! `i386/include/mach/i386/thread_status.h` declare.

use crate::arch::i386::percpu::{cpu_number, current_thread};
use crate::arch::i386::trap;
use crate::arch::types::VmSize;
use crate::glue;
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_long, c_uint, c_ushort, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// `CR0_NE` of <i386/proc_reg.h>.
const CR0_NE: usize = 0x20;
/// `CR0_TS` of <i386/proc_reg.h>.
const CR0_TS: usize = 0x08;
/// `CR0_EM` of <i386/proc_reg.h>.
const CR0_EM: usize = 0x04;
/// `CR0_MP` of <i386/proc_reg.h>.
const CR0_MP: usize = 0x02;

/// `CR4_OSFXSR` of <i386/proc_reg.h>.
const CR4_OSFXSR: usize = 0x0200;
/// `CR4_OSXSAVE` of <i386/proc_reg.h>.
const CR4_OSXSAVE: usize = 0x40000;

/// `CPU_TYPE_I486` of <mach/machine.h>: the first CPU with a real FPU trap.
const CPU_TYPE_I486: c_int = 17;

/// `CPU_FEATURE_XSAVE` of <i386/locore.h>: a bit of `cpu_features[1]`.
const CPU_FEATURE_XSAVE: u32 = 32 + 26;
/// `CPU_FEATURE_FXSR` of <i386/locore.h>: a bit of `cpu_features[0]`.
const CPU_FEATURE_FXSR: u32 = 24;

/// `CPU_FEATURE_XSAVEOPT` of <i386/fpu.h>: a bit of CPUID leaf 0xd, subleaf 1.
const CPU_FEATURE_XSAVEOPT: u32 = 1 << 0;
/// `CPU_FEATURE_XSAVEC`: as above.
const CPU_FEATURE_XSAVEC: u32 = 1 << 1;
/// `CPU_FEATURE_XSAVES`: as above.
const CPU_FEATURE_XSAVES: u32 = 1 << 3;

/// `CPU_XCR0_X87` of <i386/fpu.h>: the x87 state bit of XCR0.
const CPU_XCR0_X87: u64 = 1 << 0;
/// `XSAVE_XCOMP_BV_COMPACT` of <machine/fp_reg.h>.
const XSAVE_XCOMP_BV_COMPACT: u64 = 1 << 63;

/// `i386_FLOAT_STATE` of <mach/i386/thread_status.h>.
pub(crate) const I386_FLOAT_STATE: c_int = 2;
/// `i386_XFLOAT_STATE` of <mach/i386/thread_status.h>.
pub(crate) const I386_XFLOAT_STATE: c_int = 8;

/// `FP_STATE_BYTES` of <mach/i386/thread_status.h>: the FNSAVE image.
const FP_STATE_BYTES: usize =
    size_of::<I386FpSave>() + size_of::<I386FpRegs>();

/// `struct i386_fp_save` of <machine/fp_reg.h>, the FNSAVE image.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386FpSave {
    pub fp_control: c_ushort,
    pub fp_unused_1: c_ushort,
    pub fp_status: c_ushort,
    pub fp_unused_2: c_ushort,
    pub fp_tag: c_ushort,
    pub fp_unused_3: c_ushort,
    pub fp_eip: c_uint,
    pub fp_cs: c_ushort,
    pub fp_opcode: c_ushort,
    pub fp_dp: c_uint,
    pub fp_ds: c_ushort,
    pub fp_unused_4: c_ushort,
}

const _: () = {
    assert!(size_of::<I386FpSave>() == 28);
    assert!(align_of::<I386FpSave>() == align_of::<c_uint>());
    assert!(offset_of!(I386FpSave, fp_control) == 0);
    assert!(offset_of!(I386FpSave, fp_status) == 4);
    assert!(offset_of!(I386FpSave, fp_tag) == 8);
    assert!(offset_of!(I386FpSave, fp_eip) == 12);
    assert!(offset_of!(I386FpSave, fp_cs) == 16);
    assert!(offset_of!(I386FpSave, fp_opcode) == 18);
    assert!(offset_of!(I386FpSave, fp_dp) == 20);
    assert!(offset_of!(I386FpSave, fp_ds) == 24);
};

/// `struct i386_fp_regs` of <machine/fp_reg.h>, the eight x87 registers.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386FpRegs {
    pub fp_reg_word: [[c_ushort; 5]; 8],
}

const _: () = {
    assert!(size_of::<I386FpRegs>() == 80);
    assert!(align_of::<I386FpRegs>() == align_of::<c_ushort>());
    assert!(offset_of!(I386FpRegs, fp_reg_word) == 0);
};

/// `struct i386_xfp_xstate_header` of <machine/fp_reg.h>, packed.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct I386XfpXstateHeader {
    pub xfp_features: u64,
    pub xcomp_bv: u64,
    pub reserved: [u64; 6],
}

const _: () = {
    assert!(size_of::<I386XfpXstateHeader>() == 64);
    assert!(align_of::<I386XfpXstateHeader>() == 1);
    assert!(offset_of!(I386XfpXstateHeader, xfp_features) == 0);
    assert!(offset_of!(I386XfpXstateHeader, xcomp_bv) == 8);
    assert!(offset_of!(I386XfpXstateHeader, reserved) == 16);
};

/// `struct i386_xfp_save` of <machine/fp_reg.h>, the XSAVE image.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct I386XfpSave {
    pub fp_control: c_ushort,
    pub fp_status: c_ushort,
    pub fp_tag: c_ushort,
    pub fp_opcode: c_ushort,
    pub fp_eip: c_uint,
    pub fp_cs: c_ushort,
    pub fp_eip3: c_ushort,
    pub fp_dp: c_uint,
    pub fp_ds: c_ushort,
    pub fp_dp3: c_ushort,
    pub fp_mxcsr: c_uint,
    pub fp_mxcsr_mask: c_uint,
    pub fp_reg_word: [[u8; 16]; 8],
    pub fp_xreg_word: [[u8; 16]; 16],
    pub padding: [c_uint; 24],
    pub header: I386XfpXstateHeader,
    pub extended: [u8; 0],
}

const _: () = {
    assert!(size_of::<I386XfpSave>() == 576);
    assert!(align_of::<I386XfpSave>() == 64);
    assert!(offset_of!(I386XfpSave, fp_control) == 0);
    assert!(offset_of!(I386XfpSave, fp_status) == 2);
    assert!(offset_of!(I386XfpSave, fp_tag) == 4);
    assert!(offset_of!(I386XfpSave, fp_opcode) == 6);
    assert!(offset_of!(I386XfpSave, fp_eip) == 8);
    assert!(offset_of!(I386XfpSave, fp_cs) == 12);
    assert!(offset_of!(I386XfpSave, fp_eip3) == 14);
    assert!(offset_of!(I386XfpSave, fp_dp) == 16);
    assert!(offset_of!(I386XfpSave, fp_ds) == 20);
    assert!(offset_of!(I386XfpSave, fp_dp3) == 22);
    assert!(offset_of!(I386XfpSave, fp_mxcsr) == 24);
    assert!(offset_of!(I386XfpSave, fp_mxcsr_mask) == 28);
    assert!(offset_of!(I386XfpSave, fp_reg_word) == 32);
    assert!(offset_of!(I386XfpSave, fp_xreg_word) == 160);
    assert!(offset_of!(I386XfpSave, padding) == 416);
    assert!(offset_of!(I386XfpSave, header) == 512);
    assert!(offset_of!(I386XfpSave, extended) == 576);
};

/// The FNSAVE arm of the `i386_fpsave_state` union.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386FpSaveNative {
    pub fp_save_state: I386FpSave,
    pub fp_regs: I386FpRegs,
}

const _: () = {
    assert!(size_of::<I386FpSaveNative>() == FP_STATE_BYTES);
    assert!(align_of::<I386FpSaveNative>() == align_of::<c_uint>());
    assert!(offset_of!(I386FpSaveNative, fp_save_state) == 0);
    assert!(offset_of!(I386FpSaveNative, fp_regs) == 28);
};

/// The anonymous union of `struct i386_fpsave_state`.
#[repr(C)]
pub union I386FpSaveStateUnion {
    pub native: I386FpSaveNative,
    pub xfp_save_state: I386XfpSave,
}

const _: () = {
    assert!(size_of::<I386FpSaveStateUnion>() == 576);
    assert!(align_of::<I386FpSaveStateUnion>() == 64);
};

/// `struct i386_fpsave_state` of <i386/thread.h>: one thread's saved FPU
/// state, either the FNSAVE image or the XSAVE one.
#[repr(C)]
pub struct I386FpSaveState {
    pub fp_valid: c_int,
    pub save: I386FpSaveStateUnion,
}

const _: () = {
    assert!(size_of::<I386FpSaveState>() == 640);
    assert!(align_of::<I386FpSaveState>() == 64);
    assert!(offset_of!(I386FpSaveState, fp_valid) == 0);
    assert!(offset_of!(I386FpSaveState, save) == 64);
};

impl I386FpSaveState {
    /// The FNSAVE arm, valid only while `fp_save_kind` is `FP_FNSAVE`.
    ///
    /// # Safety
    ///
    /// The caller must know the state was saved with FNSAVE.
    unsafe fn native(&mut self) -> &mut I386FpSaveNative {
        // SAFETY: the caller selected the FNSAVE arm.
        unsafe { &mut self.save.native }
    }

    /// The XSAVE arm, the 576 bytes of the union the XSAVE instructions
    /// write.
    ///
    /// # Safety
    ///
    /// The caller must know the XSAVE arm is the live one, or, as the C's
    /// XFLOAT path did, is deliberately writing the inactive arm.
    unsafe fn xfp(&mut self) -> &mut I386XfpSave {
        // SAFETY: the caller selected the XSAVE arm.
        unsafe { &mut self.save.xfp_save_state }
    }
}

/// `struct i386_float_state` of <machine/thread_status.h>.
#[repr(C)]
pub struct I386FloatState {
    pub fpkind: c_int,
    pub initialized: c_int,
    pub hw_state: [u8; FP_STATE_BYTES],
    pub exc_status: c_int,
}

const _: () = {
    assert!(size_of::<I386FloatState>() == 120);
    assert!(align_of::<I386FloatState>() == align_of::<c_int>());
    assert!(offset_of!(I386FloatState, fpkind) == 0);
    assert!(offset_of!(I386FloatState, initialized) == 4);
    assert!(offset_of!(I386FloatState, hw_state) == 8);
    assert!(offset_of!(I386FloatState, exc_status) == 116);
};

/// `struct i386_xfloat_state` of <machine/thread_status.h>.
#[repr(C)]
pub struct I386XfloatState {
    pub fpkind: c_int,
    pub initialized: c_int,
    pub exc_status: c_int,
    pub fp_save_kind: c_int,
    pub hw_state: [u8; 0],
}

const _: () = {
    assert!(size_of::<I386XfloatState>() == 16);
    assert!(align_of::<I386XfloatState>() == align_of::<c_int>());
    assert!(offset_of!(I386XfloatState, fpkind) == 0);
    assert!(offset_of!(I386XfloatState, initialized) == 4);
    assert!(offset_of!(I386XfloatState, exc_status) == 8);
    assert!(offset_of!(I386XfloatState, fp_save_kind) == 12);
    assert!(offset_of!(I386XfloatState, hw_state) == 16);
};

/// `enum fp_save_kind` of <i386/fpu.h>: the instruction that saves the state.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpSaveKind {
    FnSave = 0,
    FxSave = 1,
    XSave = 2,
    XSaveOpt = 3,
    XSaveC = 4,
    XSaveS = 5,
}

impl FpSaveKind {
    /// The kind a C `int` names, or [`None`] for a value C never writes.
    pub(crate) fn from_int(value: c_int) -> Option<Self> {
        match value {
            0 => Some(Self::FnSave),
            1 => Some(Self::FxSave),
            2 => Some(Self::XSave),
            3 => Some(Self::XSaveOpt),
            4 => Some(Self::XSaveC),
            5 => Some(Self::XSaveS),
            _ => None,
        }
    }
}

/// The `fp_kind` values of <machine/fp_reg.h>.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FpKind {
    No = 0,
    Soft = 1,
    Fp287 = 2,
    Fp387 = 3,
    Fp387Fx = 4,
    Fp387X = 5,
}

/// `fp_save_kind` of i386/i386/fpu.c.  `Relaxed` reads: `init_fpu()` writes
/// it before the CPU it runs on can schedule a thread, and no other CPU writes
/// it.
static FP_SAVE_KIND: AtomicU8 = AtomicU8::new(FpSaveKind::FnSave as u8);

/// `fp_kind` of i386/i386/fpu.c, written under the same rule.
static FP_KIND: AtomicU8 = AtomicU8::new(FpKind::Fp387 as u8);

/// `fp_xsave_size`: the size XSAVE writes, at least `sizeof(I386XfpSave)`.
static FP_XSAVE_SIZE: AtomicU32 =
    AtomicU32::new(size_of::<I386XfpSave>() as u32);

/// The low half of `fp_xsave_support`, the XCR0 mask the XSAVE instructions
/// take in `%eax`.
static FP_XSAVE_SUPPORT_LO: AtomicU32 = AtomicU32::new(0);

/// The high half of `fp_xsave_support`, passed in `%edx`.
static FP_XSAVE_SUPPORT_HI: AtomicU32 = AtomicU32::new(0);

/// `mxcsr_feature_mask` of i386/i386/fpu.c: the security mask every
/// user-supplied MXCSR is ANDed with.
static MXCSR_FEATURE_MASK: AtomicU32 = AtomicU32::new(0xffff_ffff);

/// `fp_default_state` of i386/i386/fpu.c: the state a thread starts with.
static mut FP_DEFAULT_STATE: *mut I386FpSaveState = ptr::null_mut();

/// `ifps_cache` of i386/i386/fpu.c: the slab cache of FPU save areas.
static mut IFPS_CACHE: KmemCache = KmemCache::zeroed();

/// `fp_free()`'s body: return a save area to the cache.
///
/// # Safety
///
/// `ifps` must be a live object from `ifps_cache` and must not be used again.
pub(crate) unsafe fn free_fp_state(ifps: *mut I386FpSaveState) {
    let Some(buf) = NonNull::new(ifps.cast::<u8>()) else {
        return;
    };
    // SAFETY: `fpu_module_init()` built the cache before any thread could
    // reach this free, and the caller gives up the object.
    unsafe { (*ptr::addr_of_mut!(IFPS_CACHE)).free(buf) };
}

/// Allocate a save area, the C's `kmem_cache_alloc()`, which never fails.
///
/// # Panics
///
/// Panics, halting the kernel, if the slab layer cannot extend the cache: the
/// C `kmem_cache_alloc()` aborts in the same case.
fn alloc_fp_state() -> *mut I386FpSaveState {
    // SAFETY: the cache is built by `fpu_module_init()` before any caller.
    match unsafe { (*ptr::addr_of_mut!(IFPS_CACHE)).alloc() } {
        Some(buf) => buf.as_ptr().cast(),
        None => {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"i386/i386/fpu.c".as_ptr(),
                    line!() as c_int,
                    c"kmem_cache_alloc".as_ptr(),
                    c"fpu: out of FP save areas".as_ptr(),
                )
            }
        }
    }
}

/// `fninit()` of <i386/fpu.h>.
fn fninit() {
    // SAFETY: `fninit` resets the x87 state and is valid at CPL0.
    unsafe { core::arch::asm!("fninit", options(nostack)) };
}

/// `fnstsw()` of <i386/fpu.h>.
fn fnstsw() -> c_ushort {
    let mut status: c_ushort = 0;
    // SAFETY: `fnstsw` writes the two-byte x87 status word to the local and
    // is valid at CPL0.
    unsafe {
        core::arch::asm!(
            "fnstsw [{status}]",
            status = in(reg) &mut status,
            options(nostack),
        )
    };
    status
}

/// `fnstcw()` of <i386/fpu.h>.
fn fnstcw() -> c_ushort {
    let mut control: c_ushort = 0;
    // SAFETY: `fnstcw` writes the two-byte x87 control word to the local and
    // is valid at CPL0.
    unsafe {
        core::arch::asm!(
            "fnstcw [{control}]",
            control = in(reg) &mut control,
            options(nostack),
        )
    };
    control
}

/// `fldcw()` of <i386/fpu.h>.
fn fldcw(control: c_ushort) {
    // SAFETY: `fldcw` loads the x87 control word from the local and is valid
    // at CPL0.
    unsafe {
        core::arch::asm!(
            "fldcw [{control}]",
            control = in(reg) &control,
            options(nostack),
        )
    };
}

/// `fnsave()` of <i386/fpu.h>.
fn fnsave(state: *mut I386FpSave) {
    // SAFETY: the caller passes a live FNSAVE image; `fnsave` writes its 108
    // bytes there.
    unsafe {
        core::arch::asm!("fnsave [{state}]", state = in(reg) state, options(nostack))
    };
}

/// `frstor()` of <i386/fpu.h>.
fn frstor(state: *const I386FpSave) {
    // SAFETY: the caller passes a live FNSAVE image; `frstor` reads 108 bytes.
    unsafe {
        core::arch::asm!("frstor [{state}]", state = in(reg) state, options(nostack))
    };
}

/// `fxsave()` of <i386/fpu.h>.
fn fxsave(state: *mut I386XfpSave) {
    // SAFETY: the caller passes a live save area, 64-byte aligned as FXSAVE
    // requires; FXSAVE writes its 512 bytes there.
    unsafe {
        core::arch::asm!("fxsave [{state}]", state = in(reg) state, options(nostack))
    };
}

/// `fxrstor()` of <i386/fpu.h>.
fn fxrstor(state: *const I386XfpSave) {
    // SAFETY: the caller passes a live, aligned XSAVE/FXSAVE image; FXRSTOR
    // reads its 512 bytes.
    unsafe {
        core::arch::asm!("fxrstor [{state}]", state = in(reg) state, options(nostack))
    };
}

/// The halves of `fp_xsave_support` the XSAVE instructions take.
fn xsave_support() -> (u32, u32) {
    (
        FP_XSAVE_SUPPORT_LO.load(Ordering::Relaxed),
        FP_XSAVE_SUPPORT_HI.load(Ordering::Relaxed),
    )
}

/// `xsave()` of <i386/fpu.h>.
fn xsave(state: *mut I386XfpSave) {
    let (lo, hi) = xsave_support();
    // SAFETY: the caller passes a live, 64-byte aligned save area and the
    // mask `set_xcr0()` enabled.
    unsafe {
        core::arch::asm!(
            "xsave [{state}]",
            state = in(reg) state,
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        )
    };
}

/// `xsaveopt()` of <i386/fpu.h>.
fn xsaveopt(state: *mut I386XfpSave) {
    let (lo, hi) = xsave_support();
    // SAFETY: as in `xsave()`; the CPU reported XSAVEOPT.
    unsafe {
        core::arch::asm!(
            "xsaveopt [{state}]",
            state = in(reg) state,
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        )
    };
}

/// `xsavec()` of <i386/fpu.h>.
fn xsavec(state: *mut I386XfpSave) {
    let (lo, hi) = xsave_support();
    // SAFETY: as in `xsave()`; the CPU reported XSAVEC.
    unsafe {
        core::arch::asm!(
            "xsavec [{state}]",
            state = in(reg) state,
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        )
    };
}

/// `xsaves()` of <i386/fpu.h>.
fn xsaves(state: *mut I386XfpSave) {
    let (lo, hi) = xsave_support();
    // SAFETY: as in `xsave()`; the CPU reported XSAVES.
    unsafe {
        core::arch::asm!(
            "xsaves [{state}]",
            state = in(reg) state,
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        )
    };
}

/// `xrstor()` of <i386/fpu.h>.
fn xrstor(state: *const I386XfpSave) {
    let (lo, hi) = xsave_support();
    // SAFETY: the caller passes a live, aligned image of a kind the CPU can
    // restore with the mask `set_xcr0()` enabled.
    unsafe {
        core::arch::asm!(
            "xrstor [{state}]",
            state = in(reg) state,
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        )
    };
}

/// `xrstors()` of <i386/fpu.h>.
fn xrstors(state: *const I386XfpSave) {
    let (lo, hi) = xsave_support();
    // SAFETY: as in `xrstor()`; the CPU reported XSAVES.
    unsafe {
        core::arch::asm!(
            "xrstors [{state}]",
            state = in(reg) state,
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        )
    };
}

/// `xsetbv()` of <i386/fpu.h>.
fn xsetbv(index: u32, value: u64) {
    // SAFETY: the caller ran CPUID for XSAVE and set CR4.OSXSAVE; `xsetbv`
    // writes an XCR the CPU reported.
    unsafe {
        core::arch::asm!(
            "xsetbv",
            in("ecx") index,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack),
        )
    };
}

/// `set_xcr0()` of <i386/fpu.h>.
fn set_xcr0(value: u64) {
    xsetbv(0, value);
}

/// The `get_cr0()` of <i386/proc_reg.h>.
fn read_cr0() -> usize {
    let value: usize;
    // SAFETY: reading CR0 is legal at CPL0.
    unsafe {
        core::arch::asm!("mov {value}, cr0", value = out(reg) value, options(nostack, preserves_flags, readonly))
    };
    value
}

/// The `set_cr0()` of <i386/proc_reg.h>.
fn write_cr0(value: usize) {
    // SAFETY: writing CR0 is legal at CPL0.
    unsafe {
        core::arch::asm!("mov cr0, {value}", value = in(reg) value, options(nostack, preserves_flags))
    };
}

/// The `get_cr4()` of <i386/proc_reg.h>.
fn read_cr4() -> usize {
    let value: usize;
    // SAFETY: reading CR4 is legal at CPL0.
    unsafe {
        core::arch::asm!("mov {value}, cr4", value = out(reg) value, options(nostack, preserves_flags, readonly))
    };
    value
}

/// The `set_cr4()` of <i386/proc_reg.h>.
fn write_cr4(value: usize) {
    // SAFETY: writing CR4 is legal at CPL0.
    unsafe {
        core::arch::asm!("mov cr4, {value}", value = in(reg) value, options(nostack, preserves_flags))
    };
}

/// `set_ts()` of <i386/proc_reg.h>.
pub(crate) fn set_ts() {
    write_cr0(read_cr0() | CR0_TS);
}

/// `clear_ts()` of <i386/proc_reg.h>.
pub(crate) fn clear_ts() {
    // SAFETY: `clts` is the CPU's own instruction and is valid at CPL0.
    unsafe { core::arch::asm!("clts", options(nostack, preserves_flags)) };
}

/// The `cpuid` macro of <i386/proc_reg.h>, whose `%ebx` is callee-saved.
fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    // SAFETY: `cpuid` is a plain CPU instruction; the surrounding moves save
    // and restore `%rbx`/`%ebx`, which LLVM reserves.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "mov {tmp:r}, rbx",
            "cpuid",
            "xchg {tmp:r}, rbx",
            tmp = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
        )
    };
    // SAFETY: as above, with the 32-bit `%ebx`.
    #[cfg(target_arch = "x86")]
    unsafe {
        core::arch::asm!(
            "mov {tmp:e}, ebx",
            "cpuid",
            "xchg {tmp:e}, ebx",
            tmp = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
        )
    };
    (eax, ebx, ecx, edx)
}

/// `CPU_HAS_FEATURE()` of <i386/locore.h>.
fn has_cpu_feature(feature: u32) -> bool {
    // SAFETY: `cpu_features` is written once by the early CPU probe.
    let word = unsafe { glue::cpu_features[(feature / 32) as usize] };
    word & (1 << (feature % 32)) != 0
}

/// The C's `fp_infinity == -fp_infinity` on the x87: true on an 80287, whose
/// infinity has no sign, false on a 387.
fn infinity_has_no_sign() -> bool {
    let equal: u8;
    // SAFETY: the sequence leaves the x87 stack as it found it: `fdivp` in
    // its no-operand form divides `st(1)` by `st(0)` and pops, `fcomip` pops
    // the negated copy, and `fstp` the remaining infinity.
    unsafe {
        core::arch::asm!(
            "fld1",
            "fldz",
            "fdivp",
            "fld st(0)",
            "fchs",
            "fcomip st, st(1)",
            "fstp st(0)",
            "sete {equal}",
            equal = out(reg_byte) equal,
            options(nostack),
        )
    };
    equal != 0
}

/// The `fnsetpm` the C emitted as `.byte 0xdb; .byte 0xe4`.
fn fnsetpm() {
    // SAFETY: the 80287's `fnsetpm` is a no-op on later FPUs.
    unsafe { core::arch::asm!(".byte 0xdb", ".byte 0xe4", options(nostack)) };
}

/// Save `ifps` with the instruction `fp_save_kind` selects.
///
/// # Safety
///
/// `ifps` must be a live save area, and the caller must have turned off the
/// FPU's task-switched trap for the running thread.
pub(crate) unsafe fn fpu_save(ifps: *mut I386FpSaveState) {
    // SAFETY: the caller passes a live object, and every arm writes only
    // inside it.
    let ifps = unsafe { &mut *ifps };
    match save_kind() {
        FpSaveKind::FnSave => {
            // SAFETY: the arm matches the kind, so the FNSAVE image is live.
            let native = unsafe { ifps.native() };
            fnsave(&mut native.fp_save_state);
        }
        FpSaveKind::FxSave => {
            // SAFETY: the arm matches the kind, so the XSAVE image is live.
            let xfp = unsafe { ifps.xfp() };
            fxsave(xfp);
        }
        FpSaveKind::XSave => {
            // SAFETY: the arm matches the kind, so the XSAVE image is live.
            let xfp = unsafe { ifps.xfp() };
            xsave(xfp);
        }
        FpSaveKind::XSaveOpt => {
            // SAFETY: as in the XSAVE arm.
            let xfp = unsafe { ifps.xfp() };
            xsaveopt(xfp);
        }
        FpSaveKind::XSaveC => {
            // SAFETY: as in the XSAVE arm.
            let xfp = unsafe { ifps.xfp() };
            xsavec(xfp);
        }
        FpSaveKind::XSaveS => {
            // SAFETY: as in the XSAVE arm.
            let xfp = unsafe { ifps.xfp() };
            xsaves(xfp);
        }
    }
    ifps.fp_valid = 1;
}

/// Restore `ifps` with the instruction `fp_save_kind` selects.
///
/// # Safety
///
/// `ifps` must hold an image of the kind `fp_save_kind` selects.
pub(crate) unsafe fn fpu_rstor(ifps: *mut I386FpSaveState) {
    // SAFETY: the caller passes a live object; every arm reads only its live
    // image.
    let ifps = unsafe { &*ifps };
    match save_kind() {
        FpSaveKind::FnSave => {
            // SAFETY: the arm matches the kind, so the FNSAVE image is live.
            let native = unsafe { &ifps.save.native };
            frstor(&native.fp_save_state);
        }
        FpSaveKind::FxSave => {
            // SAFETY: the arm matches the kind, so the XSAVE image is live.
            let xfp = unsafe { &ifps.save.xfp_save_state };
            fxrstor(xfp);
        }
        FpSaveKind::XSave | FpSaveKind::XSaveOpt | FpSaveKind::XSaveC => {
            // SAFETY: as in the FXSAVE arm; xrstor restores each of these.
            let xfp = unsafe { &ifps.save.xfp_save_state };
            xrstor(xfp);
        }
        FpSaveKind::XSaveS => {
            // SAFETY: as in the FXSAVE arm.
            let xfp = unsafe { &ifps.save.xfp_save_state };
            xrstors(xfp);
        }
    }
}

/// The global `fp_save_kind`.
fn save_kind() -> FpSaveKind {
    match FP_SAVE_KIND.load(Ordering::Relaxed) {
        0 => FpSaveKind::FnSave,
        1 => FpSaveKind::FxSave,
        2 => FpSaveKind::XSave,
        3 => FpSaveKind::XSaveOpt,
        4 => FpSaveKind::XSaveC,
        _ => FpSaveKind::XSaveS,
    }
}

fn set_save_kind(kind: FpSaveKind) {
    FP_SAVE_KIND.store(kind as u8, Ordering::Relaxed);
}

fn fp_kind() -> FpKind {
    match FP_KIND.load(Ordering::Relaxed) {
        0 => FpKind::No,
        1 => FpKind::Soft,
        2 => FpKind::Fp287,
        3 => FpKind::Fp387,
        4 => FpKind::Fp387Fx,
        _ => FpKind::Fp387X,
    }
}

fn set_fp_kind(kind: FpKind) {
    FP_KIND.store(kind as u8, Ordering::Relaxed);
}

fn xfp_save_size() -> u32 {
    FP_XSAVE_SIZE.load(Ordering::Relaxed)
}

/// `twd_i387_to_fxsr()` of i386/i386/fpu.c.
fn twd_i387_to_fxsr(twd: c_ushort) -> c_ushort {
    let mut tmp = !c_uint::from(twd);
    tmp = (tmp | (tmp >> 1)) & 0x5555;
    tmp = (tmp | (tmp >> 1)) & 0x3333;
    tmp = (tmp | (tmp >> 2)) & 0x0f0f;
    tmp = (tmp | (tmp >> 4)) & 0x00ff;
    tmp as c_ushort
}

/// One 80-bit register of an FXSAVE image, the C's local `struct` over
/// `fp_reg_word`.
#[repr(C)]
struct FxReg {
    significand: [c_ushort; 4],
    exponent: c_ushort,
    padding: [c_ushort; 3],
}

const _: () = {
    assert!(size_of::<FxReg>() == 16);
    assert!(offset_of!(FxReg, exponent) == 8);
};

/// `twd_fxsr_to_i387()` of i386/i386/fpu.c.
fn twd_fxsr_to_i387(fxsave: &I386XfpSave) -> c_uint {
    let tos = (c_uint::from(fxsave.fp_status) >> 11) & 7;
    let mut twd = c_uint::from(fxsave.fp_tag);
    let mut ret: c_uint = 0xffff_0000;
    for i in 0..8u32 {
        let tag = if twd & 1 != 0 {
            let index = i.wrapping_sub(tos) & 7;
            // SAFETY: the index is masked into the eight-register array, so
            // the cast points inside it, and the array is 16-byte aligned.
            let st = unsafe {
                &*fxsave.fp_reg_word[index as usize].as_ptr().cast::<FxReg>()
            };
            match st.exponent & 0x7fff {
                0x7fff => 2,
                0x0000 => {
                    if st.significand[0] == 0
                        && st.significand[1] == 0
                        && st.significand[2] == 0
                        && st.significand[3] == 0
                    {
                        1
                    } else {
                        2
                    }
                }
                _ => {
                    if st.significand[3] & 0x8000 != 0 {
                        0
                    } else {
                        2
                    }
                }
            }
        } else {
            3
        };
        ret |= tag << (2 * i);
        twd >>= 1;
    }
    ret
}

/// `fpinit()` of i386/i386/fpu.c.
unsafe fn fpinit(thread: *mut Thread) {
    clear_ts();
    // SAFETY: `fpu_module_init()` set `fp_default_state` before any thread
    // could reach this init, and the default image matches `fp_save_kind`.
    unsafe { fpu_rstor(FP_DEFAULT_STATE) };
    // SAFETY: the caller passes a live thread whose pcb `pcb_init()` built.
    let control = unsafe { (*(*thread).pcb).init_control };
    if control != 0 {
        fldcw(control);
    }
}

/// `init_fpu()` of i386/i386/fpu.c, called on each CPU.
///
/// # Safety
///
/// Runs at boot on each CPU before that CPU schedules any thread.
///
/// # Panics
///
/// Halts the kernel when no FPU answers the probe or the CPU reports an
/// XSAVE area smaller than [`I386XfpSave`], as the C `panic()` did.
pub(crate) unsafe fn init_fpu() {
    // SAFETY: `machine_slot` is the boot probe's record, one per CPU, and
    // `cpu_number()` names this one.
    let native =
        if unsafe { glue::machine_slot[cpu_number() as usize].cpu_type }
            >= CPU_TYPE_I486
        {
            CR0_NE
        } else {
            0
        };

    write_cr0((read_cr0() & !(CR0_EM | CR0_TS)) | native);
    fninit();
    let status = fnstsw();
    let control = fnstcw();

    if status & 0xff != 0 || control & 0x103f != 0x3f {
        // SAFETY: `Panic()` does not return.
        unsafe {
            glue::Panic(
                c"i386/i386/fpu.c".as_ptr(),
                line!() as c_int,
                c"init_fpu".as_ptr(),
                c"No FPU!".as_ptr(),
            )
        }
    }

    if infinity_has_no_sign() {
        set_fp_kind(FpKind::Fp287);
        set_save_kind(FpSaveKind::FnSave);
        fnsetpm();
    } else {
        set_fp_kind(FpKind::Fp387);
        set_save_kind(FpSaveKind::FnSave);

        if has_cpu_feature(CPU_FEATURE_XSAVE) {
            let (eax, _, _, edx) = cpuid(0xd, 0x0);
            FP_XSAVE_SUPPORT_LO.store(eax, Ordering::Relaxed);
            FP_XSAVE_SUPPORT_HI.store(edx, Ordering::Relaxed);

            write_cr4(read_cr4() | CR4_OSFXSR | CR4_OSXSAVE);
            set_xcr0(u64::from(eax) | (u64::from(edx) << 32));

            let (xsave_cpu_features, ebx, _, _) = cpuid(0xd, 0x1);

            if xsave_cpu_features & CPU_FEATURE_XSAVES != 0 {
                FP_XSAVE_SIZE.store(ebx, Ordering::Relaxed);
                if ebx < size_of::<I386XfpSave>() as u32 {
                    panic_xsave_size(ebx);
                }
                set_save_kind(FpSaveKind::XSaveS);
            } else {
                let (_, ebx, _, _) = cpuid(0xd, 0x0);
                FP_XSAVE_SIZE.store(ebx, Ordering::Relaxed);
                if ebx < size_of::<I386XfpSave>() as u32 {
                    panic_xsave_size(ebx);
                }

                if xsave_cpu_features & CPU_FEATURE_XSAVEOPT != 0 {
                    set_save_kind(FpSaveKind::XSaveOpt);
                } else if xsave_cpu_features & CPU_FEATURE_XSAVEC != 0 {
                    set_save_kind(FpSaveKind::XSaveC);
                } else {
                    set_save_kind(FpSaveKind::XSave);
                }
            }

            set_fp_kind(FpKind::Fp387X);
        } else if has_cpu_feature(CPU_FEATURE_FXSR) {
            write_cr4(read_cr4() | CR4_OSFXSR);
            set_fp_kind(FpKind::Fp387Fx);
            set_save_kind(FpSaveKind::FxSave);
        }

        if save_kind() != FpSaveKind::FnSave {
            // SAFETY: the area is a local aligned the way FXSAVE requires,
            // and the assignment below runs before it is used.
            let save = unsafe {
                let mut save = core::mem::MaybeUninit::<I386XfpSave>::zeroed();
                fxsave(save.as_mut_ptr());
                save.assume_init()
            };
            let mask = if save.fp_mxcsr_mask == 0 {
                0x0000_ffbf
            } else {
                save.fp_mxcsr_mask
            };
            MXCSR_FEATURE_MASK.fetch_and(mask, Ordering::Relaxed);
        }
    }

    write_cr0(read_cr0() | CR0_TS | CR0_MP);
}

/// The C's `panic()` for an XSAVE area smaller than the minimum.
fn panic_xsave_size(size: u32) -> ! {
    // SAFETY: `Panic()` accepts the C format and arguments, and does not
    // return.
    unsafe {
        glue::Panic(
            c"i386/i386/fpu.c".as_ptr(),
            line!() as c_int,
            c"init_fpu".as_ptr(),
            c"CPU-provided xstate size %d is smaller than our minimum %d!\n"
                .as_ptr(),
            size as c_int,
            size_of::<I386XfpSave>() as c_int,
        )
    }
}

/// `i386_get_xstate_size()` of i386/i386/fpu.c.
///
/// # Safety
///
/// `size` must be valid for a write.
pub(crate) unsafe fn i386_get_xstate_size(
    host: *mut c_void,
    size: *mut VmSize,
) -> Result<(), KernError> {
    if host.is_null() {
        return Err(KernError::InvalidArgument);
    }
    // SAFETY: the caller promises `size` is writable; the C `host_t` is null
    // for `HOST_NULL`.
    unsafe {
        *size = size_of::<I386XfloatState>() + xfp_save_size() as usize;
    }
    Ok(())
}

/// `fpu_module_init()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Called once at startup, before any thread can save FPU state.
///
/// # Panics
///
/// Halts the kernel if the slab layer reports no memory for the default
/// state, as the C `kmem_cache_alloc()` would have aborted.
pub(crate) unsafe fn fpu_module_init() {
    // SAFETY: the caller promises this runs once, before any FPU user, so
    // nothing else touches the cache object while the slab layer builds it.
    unsafe {
        (*ptr::addr_of_mut!(IFPS_CACHE)).init(
            b"i386_fpsave_state",
            offset_of!(I386FpSaveState, save) + xfp_save_size() as usize,
            align_of::<I386FpSaveState>(),
            None,
            CacheInitFlags::EMPTY,
        );
    }

    let state = alloc_fp_state();
    // SAFETY: the object just came from the cache with the size the C used.
    unsafe {
        ptr::write_bytes(
            state.cast::<u8>(),
            0,
            offset_of!(I386FpSaveState, save) + xfp_save_size() as usize,
        )
    };
    // SAFETY: `fpu_module_init()` is the only writer, before any FPU user.
    unsafe { FP_DEFAULT_STATE = state };

    clear_ts();
    fninit();
    // SAFETY: the default image was just built and matches `fp_save_kind`.
    unsafe { fpu_save(FP_DEFAULT_STATE) };
    set_ts();
}

/// `fpu_set_state()` of i386/i386/fpu.c.
///
/// # Safety
///
/// `thread` must point at a live thread; `state` must hold the record
/// `flavor` names, readable for the C's `i386_FLOAT_STATE_COUNT` or the
/// XFLOAT record's current size.
///
/// # Panics
///
/// Halts the kernel if the slab layer reports no memory for a save area.
pub(crate) unsafe fn fpu_set_state(
    thread: *mut Thread,
    state: *mut c_void,
    flavor: c_int,
) -> Result<(), KernError> {
    if fp_kind() == FpKind::No {
        return Err(KernError::Failure);
    }

    let xfstate = state.cast::<I386XfloatState>();
    if flavor == I386_XFLOAT_STATE
        // SAFETY: the caller passes the XFLOAT record for that flavor.
        && unsafe { (*xfstate).initialized != 0 }
        // SAFETY: as above.
        && unsafe { FpSaveKind::from_int((*xfstate).fp_save_kind) } != Some(save_kind())
    {
        return Err(KernError::InvalidArgument);
    }

    let fstate = state.cast::<I386FloatState>();
    let invalid = if flavor == I386_FLOAT_STATE {
        // SAFETY: the caller passes the FLOAT record for that flavor.
        unsafe { (*fstate).initialized == 0 }
    } else {
        // SAFETY: as above, with the XFLOAT record.
        unsafe { flavor == I386_XFLOAT_STATE && (*xfstate).initialized == 0 }
    };

    // SAFETY: the caller promises a live thread; its pcb came from
    // `pcb_init()`.
    let pcb = unsafe { (*thread).pcb };

    if invalid {
        // SAFETY: `pcb` is live and the lock protects `ims.ifps`.
        let ifps = unsafe {
            (*pcb).lock.lock();
            let ifps = (*pcb).ims.ifps;
            (*pcb).ims.ifps = ptr::null_mut();
            (*pcb).lock.unlock();
            ifps
        };
        // SAFETY: the pointer was the thread's own save area, now detached.
        unsafe { free_fp_state(ifps) };
        return Ok(());
    }

    let mut new_ifps: *mut I386FpSaveState = ptr::null_mut();
    loop {
        // SAFETY: `pcb` is live and the lock protects `ims.ifps`.
        let ifps = unsafe {
            (*pcb).lock.lock();
            let ifps = (*pcb).ims.ifps;
            if ifps.is_null() {
                if new_ifps.is_null() {
                    (*pcb).lock.unlock();
                    new_ifps = alloc_fp_state();
                    continue;
                }
                let ifps = new_ifps;
                new_ifps = ptr::null_mut();
                (*pcb).ims.ifps = ifps;
                ifps
            } else {
                ifps
            }
        };

        // SAFETY: the object is the thread's own live save area, and the C
        // zeroed the reserved part up to the xsave size.
        unsafe {
            let ifps_ref = &mut *ifps;
            ptr::write_bytes(
                ifps_ref as *mut I386FpSaveState as *mut u8,
                0,
                offset_of!(I386FpSaveState, save) + xfp_save_size() as usize,
            );
            ifps_ref.fp_valid = 1;
            fill_state(ifps_ref, state, flavor);
        }

        // SAFETY: the lock `pcb_init()` initialized is held here.
        unsafe { (*pcb).lock.unlock() };
        break;
    }

    // SAFETY: the retry path allocates at most one spare, which this frees.
    unsafe { free_fp_state(new_ifps) };
    Ok(())
}

/// Copy the user record into a fresh save area, the body of the valid-state
/// arm of `fpu_set_state()`.
///
/// # Safety
///
/// `ifps` must be live and zeroed, and `state` must hold the record `flavor`
/// names.
unsafe fn fill_state(
    ifps: &mut I386FpSaveState,
    state: *mut c_void,
    flavor: c_int,
) {
    if flavor == I386_FLOAT_STATE {
        let fstate = state.cast::<I386FloatState>();
        // SAFETY: the caller passes the FLOAT record for that flavor.
        let (user_fp_state, user_fp_regs) = unsafe {
            (
                (*fstate).hw_state.as_ptr().cast::<I386FpSave>(),
                (*fstate)
                    .hw_state
                    .as_ptr()
                    .add(size_of::<I386FpSave>())
                    .cast::<I386FpRegs>(),
            )
        };

        if save_kind() != FpSaveKind::FnSave {
            // SAFETY: the kind selected the XSAVE arm.
            let xfp = unsafe { ifps.xfp() };
            // SAFETY: both pointers name records inside the caller's state.
            unsafe {
                xfp.fp_control = (*user_fp_state).fp_control;
                xfp.fp_status = (*user_fp_state).fp_status;
                xfp.fp_tag = twd_i387_to_fxsr((*user_fp_state).fp_tag);
                xfp.fp_eip = (*user_fp_state).fp_eip;
                xfp.fp_cs = (*user_fp_state).fp_cs;
                xfp.fp_opcode = (*user_fp_state).fp_opcode;
                xfp.fp_dp = (*user_fp_state).fp_dp;
                xfp.fp_ds = (*user_fp_state).fp_ds;
            }
            xfp.fp_mxcsr = 0x1f80;
            xfp.fp_mxcsr_mask = MXCSR_FEATURE_MASK.load(Ordering::Relaxed);
            for (slot, word) in xfp
                .fp_reg_word
                .iter_mut()
                // SAFETY: the caller passes the FLOAT record's register
                // array, eight entries beside these eight slots.
                .zip(unsafe { (*user_fp_regs).fp_reg_word.iter() })
            {
                // SAFETY: the C copied one `unsigned short[5]`, ten bytes,
                // into the sixteen-byte xsave slot.
                unsafe {
                    ptr::copy_nonoverlapping(
                        word.as_ptr().cast::<u8>(),
                        slot.as_mut_ptr(),
                        size_of::<[c_ushort; 5]>(),
                    )
                };
            }
            xfp.header.xfp_features = CPU_XCR0_X87;
            if save_kind() == FpSaveKind::XSaveS {
                xfp.header.xcomp_bv = XSAVE_XCOMP_BV_COMPACT;
            }
        } else {
            // SAFETY: the kind selected the FNSAVE arm.
            let native = unsafe { ifps.native() };
            // SAFETY: as above.
            unsafe {
                native.fp_save_state.fp_control = (*user_fp_state).fp_control;
                native.fp_save_state.fp_status = (*user_fp_state).fp_status;
                native.fp_save_state.fp_tag = (*user_fp_state).fp_tag;
                native.fp_save_state.fp_eip = (*user_fp_state).fp_eip;
                native.fp_save_state.fp_cs = (*user_fp_state).fp_cs;
                native.fp_save_state.fp_opcode = (*user_fp_state).fp_opcode;
                native.fp_save_state.fp_dp = (*user_fp_state).fp_dp;
                native.fp_save_state.fp_ds = (*user_fp_state).fp_ds;
                native.fp_regs = *user_fp_regs;
            }
        }
    } else if flavor == I386_XFLOAT_STATE {
        let xfstate = state.cast::<I386XfloatState>();
        // SAFETY: the caller passes the XFLOAT record for that flavor.
        let user_fp_state =
            unsafe { (*xfstate).hw_state.as_ptr().cast::<I386XfpSave>() };
        // SAFETY: the caller supplied the XFLOAT record, and the union is the
        // 576-byte XSAVE image the C wrote to here, even when the save
        // instruction in use is FNSAVE.
        let xfp = unsafe { ifps.xfp() };
        // SAFETY: both pointers name live records.
        unsafe {
            xfp.fp_control = (*user_fp_state).fp_control;
            xfp.fp_status = (*user_fp_state).fp_status;
            xfp.fp_tag = (*user_fp_state).fp_tag;
            xfp.fp_eip = (*user_fp_state).fp_eip;
            xfp.fp_cs = (*user_fp_state).fp_cs;
            xfp.fp_opcode = (*user_fp_state).fp_opcode;
            xfp.fp_dp = (*user_fp_state).fp_dp;
            xfp.fp_ds = (*user_fp_state).fp_ds;
            xfp.fp_dp3 = (*user_fp_state).fp_dp3;
            xfp.fp_mxcsr = (*user_fp_state).fp_mxcsr
                & MXCSR_FEATURE_MASK.load(Ordering::Relaxed);
            xfp.fp_mxcsr_mask = (*user_fp_state).fp_mxcsr_mask
                & MXCSR_FEATURE_MASK.load(Ordering::Relaxed);
        }
        for (slot, word) in xfp
            .fp_reg_word
            .iter_mut()
            .zip(unsafe { (*user_fp_state).fp_reg_word.iter() })
        {
            // SAFETY: both sides are the sixteen-byte xsave slots.
            unsafe {
                ptr::copy_nonoverlapping(
                    word.as_ptr(),
                    slot.as_mut_ptr(),
                    size_of::<[u8; 16]>(),
                )
            };
        }
        for (slot, word) in xfp
            .fp_xreg_word
            .iter_mut()
            .zip(unsafe { (*user_fp_state).fp_xreg_word.iter() })
        {
            // SAFETY: as above.
            unsafe {
                ptr::copy_nonoverlapping(
                    word.as_ptr(),
                    slot.as_mut_ptr(),
                    size_of::<[u8; 16]>(),
                )
            };
        }
        // SAFETY: both sides are the same packed header record.
        xfp.header = unsafe { (*user_fp_state).header };
        let xsave_size = xfp_save_size() as usize;
        if xsave_size > size_of::<I386XfpSave>() {
            // SAFETY: the caller's record is at least `xsave_size` bytes,
            // and the thread's area was allocated with the same size; the
            // copy starts at the zero-length `extended` field.
            unsafe {
                ptr::copy_nonoverlapping(
                    (*user_fp_state).extended.as_ptr(),
                    xfp.extended.as_mut_ptr(),
                    xsave_size - size_of::<I386XfpSave>(),
                )
            };
        }
    }
}

/// `fpu_get_state()` of i386/i386/fpu.c.
///
/// # Safety
///
/// `thread` must point at a live thread; `state` must be writable for the
/// record `flavor` names, at the C's `i386_FLOAT_STATE_COUNT` or the XFLOAT
/// record's current size.
pub(crate) unsafe fn fpu_get_state(
    thread: *mut Thread,
    state: *mut c_void,
    flavor: c_int,
) -> Result<(), KernError> {
    if fp_kind() == FpKind::No {
        return Err(KernError::Failure);
    }
    if flavor != I386_FLOAT_STATE && save_kind() == FpSaveKind::FnSave {
        return Err(KernError::Failure);
    }

    // SAFETY: the caller promises a live thread; its pcb came from
    // `pcb_init()`.
    let pcb = unsafe { (*thread).pcb };

    // SAFETY: `pcb` is live and the lock protects `ims.ifps`.  The C held it
    // across the whole copy, so a concurrent `fpu_set_state()` cannot free
    // the area under us.
    unsafe { (*pcb).lock.lock() };
    // SAFETY: the lock is held.
    let ifps = unsafe { (*pcb).ims.ifps };
    if ifps.is_null() {
        // SAFETY: the lock taken above.
        unsafe { (*pcb).lock.unlock() };

        if flavor == I386_FLOAT_STATE {
            // SAFETY: the caller promises the FLOAT record is writable.
            unsafe {
                ptr::write_bytes(
                    state.cast::<u8>(),
                    0,
                    size_of::<I386FloatState>(),
                )
            };
        } else if flavor == I386_XFLOAT_STATE {
            // SAFETY: the caller promises the XFLOAT record is writable
            // for its current size.
            unsafe {
                ptr::write_bytes(
                    state.cast::<u8>(),
                    0,
                    size_of::<I386XfloatState>() + xfp_save_size() as usize,
                )
            };
        }
        return Ok(());
    }

    if thread == current_thread() {
        clear_ts();
        // SAFETY: the caller's thread owns `ifps` and no other CPU can run
        // it, as the C's comment on `fpu_get_state()` relies on.
        unsafe { fpu_save(ifps) };
        set_ts();
    }

    if flavor == I386_FLOAT_STATE {
        let fstate = state.cast::<I386FloatState>();
        // SAFETY: the caller promises the FLOAT record is writable.
        let (user_fp_state, user_fp_regs) = unsafe {
            (*fstate).fpkind = fp_kind() as c_int;
            (*fstate).exc_status = 0;
            (
                (*fstate).hw_state.as_mut_ptr().cast::<I386FpSave>(),
                (*fstate)
                    .hw_state
                    .as_mut_ptr()
                    .add(size_of::<I386FpSave>())
                    .cast::<I386FpRegs>(),
            )
        };
        // SAFETY: the caller promises the record is writable; the C zeroed
        // the FNSAVE image before filling it.
        unsafe {
            ptr::write_bytes(
                user_fp_state.cast::<u8>(),
                0,
                size_of::<I386FpSave>(),
            )
        };

        if save_kind() != FpSaveKind::FnSave {
            // SAFETY: the kind selected the XSAVE arm.
            let xfp = unsafe { &(*ifps).save.xfp_save_state };
            // SAFETY: both pointers name live records.
            unsafe {
                (*fstate).initialized = (*ifps).fp_valid;
                (*user_fp_state).fp_control = xfp.fp_control;
                (*user_fp_state).fp_status = xfp.fp_status;
                // The C's function returned `unsigned long`; the tag word
                // is its low 16 bits.
                (*user_fp_state).fp_tag = twd_fxsr_to_i387(xfp) as c_ushort;
                (*user_fp_state).fp_eip = xfp.fp_eip;
                (*user_fp_state).fp_cs = xfp.fp_cs;
                (*user_fp_state).fp_opcode = xfp.fp_opcode;
                (*user_fp_state).fp_dp = xfp.fp_dp;
                (*user_fp_state).fp_ds = xfp.fp_ds;
            }
            for (slot, word) in xfp
                .fp_reg_word
                .iter()
                .zip(unsafe { (*user_fp_regs).fp_reg_word.iter_mut() })
            {
                // SAFETY: the C copied one `unsigned short[5]`, ten bytes.
                unsafe {
                    ptr::copy_nonoverlapping(
                        slot.as_ptr(),
                        word.as_mut_ptr().cast::<u8>(),
                        size_of::<[c_ushort; 5]>(),
                    )
                };
            }
        } else {
            // SAFETY: the kind selected the FNSAVE arm.
            let native = unsafe { &(*ifps).save.native };
            // SAFETY: both pointers name live records.
            unsafe {
                (*fstate).initialized = (*ifps).fp_valid;
                (*user_fp_state).fp_control = native.fp_save_state.fp_control;
                (*user_fp_state).fp_status = native.fp_save_state.fp_status;
                (*user_fp_state).fp_tag = native.fp_save_state.fp_tag;
                (*user_fp_state).fp_eip = native.fp_save_state.fp_eip;
                (*user_fp_state).fp_cs = native.fp_save_state.fp_cs;
                (*user_fp_state).fp_opcode = native.fp_save_state.fp_opcode;
                (*user_fp_state).fp_dp = native.fp_save_state.fp_dp;
                (*user_fp_state).fp_ds = native.fp_save_state.fp_ds;
                *user_fp_regs = native.fp_regs;
            }
        }
    } else if flavor == I386_XFLOAT_STATE {
        let xfstate = state.cast::<I386XfloatState>();
        // SAFETY: the caller promises the XFLOAT record is writable.
        let user_fp_state = unsafe {
            (*xfstate).fpkind = fp_kind() as c_int;
            (*xfstate).exc_status = 0;
            (*xfstate).initialized = (*ifps).fp_valid;
            (*xfstate).fp_save_kind = save_kind() as c_int;
            (*xfstate).hw_state.as_mut_ptr().cast::<I386XfpSave>()
        };
        // SAFETY: as above; the C zeroed the image before filling it.
        unsafe {
            ptr::write_bytes(
                user_fp_state.cast::<u8>(),
                0,
                size_of::<I386XfpSave>(),
            )
        };
        // SAFETY: the flavor is not FLOAT and the kind is not FnSave, so the
        // object holds the XSAVE arm.
        let xfp = unsafe { &(*ifps).save.xfp_save_state };
        // SAFETY: both pointers name live records.
        unsafe {
            (*user_fp_state).fp_control = xfp.fp_control;
            (*user_fp_state).fp_status = xfp.fp_status;
            (*user_fp_state).fp_tag = xfp.fp_tag;
            (*user_fp_state).fp_eip = xfp.fp_eip;
            (*user_fp_state).fp_cs = xfp.fp_cs;
            (*user_fp_state).fp_opcode = xfp.fp_opcode;
            (*user_fp_state).fp_dp = xfp.fp_dp;
            (*user_fp_state).fp_ds = xfp.fp_ds;
            (*user_fp_state).fp_dp3 = xfp.fp_dp3;
            (*user_fp_state).fp_mxcsr = xfp.fp_mxcsr;
            (*user_fp_state).fp_mxcsr_mask = xfp.fp_mxcsr_mask;
        }
        for (slot, word) in xfp
            .fp_reg_word
            .iter()
            .zip(unsafe { (*user_fp_state).fp_reg_word.iter_mut() })
        {
            // SAFETY: both sides are the sixteen-byte xsave slots.
            unsafe {
                ptr::copy_nonoverlapping(
                    slot.as_ptr(),
                    word.as_mut_ptr(),
                    size_of::<[u8; 16]>(),
                )
            };
        }
        for (slot, word) in xfp
            .fp_xreg_word
            .iter()
            .zip(unsafe { (*user_fp_state).fp_xreg_word.iter_mut() })
        {
            // SAFETY: as above.
            unsafe {
                ptr::copy_nonoverlapping(
                    slot.as_ptr(),
                    word.as_mut_ptr(),
                    size_of::<[u8; 16]>(),
                )
            };
        }
        // SAFETY: both sides are the same packed header record.
        unsafe { (*user_fp_state).header = xfp.header };
        let xsave_size = xfp_save_size() as usize;
        if xsave_size > size_of::<I386XfpSave>() {
            // SAFETY: the caller's record is at least `xsave_size` bytes, and
            // the thread's area was allocated with the same size.
            unsafe {
                ptr::copy_nonoverlapping(
                    xfp.extended.as_ptr(),
                    (*user_fp_state).extended.as_mut_ptr(),
                    xsave_size - size_of::<I386XfpSave>(),
                )
            };
        }
    }

    // SAFETY: the lock taken for the lock-scoped copy above.
    unsafe { (*pcb).lock.unlock() };
    Ok(())
}

/// `fp_save()` of i386/i386/fpu.h, which i386/i386/fpu.c defined.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must not hold its
/// pcb lock the wrong way: the C's callers are the FPU traps and
/// `fpu_get_state()`.
pub(crate) unsafe fn fp_save(thread: *mut Thread) {
    // SAFETY: the caller promises a live thread.
    let pcb = unsafe { (*thread).pcb };
    // SAFETY: the pcb is live and the thread is not running elsewhere when
    // the caller saves from a trap or holds the pcb lock.
    unsafe {
        let ifps = (*pcb).ims.ifps;
        if !ifps.is_null() && (*ifps).fp_valid == 0 {
            fpu_save(ifps);
        }
    }
}

/// `fpinherit()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Both threads must be live, and the caller must hold no FPU state lock.
pub(crate) unsafe fn fpinherit(
    parent_thread: *mut Thread,
    thread: *mut Thread,
) {
    // SAFETY: the caller promises live threads.
    let pcb = unsafe { (*parent_thread).pcb };
    // SAFETY: the parent's pcb is live.
    let ifps = unsafe { (*pcb).ims.ifps };
    if !ifps.is_null() {
        // SAFETY: the parent's save area is live; `pcb_init()` built the
        // child's pcb.
        unsafe {
            if (*ifps).fp_valid == 1 {
                let native = &(*ifps).save.native;
                (*(*thread).pcb).init_control =
                    native.fp_save_state.fp_control;
            } else {
                let control = &raw mut (*(*thread).pcb).init_control;
                fnstcw_to(control);
            }
        }
    }
}

/// `fnstcw()` into a caller-named control word, the C's `fnstcw(&addr)`.
fn fnstcw_to(control: *mut c_ushort) {
    // SAFETY: `fnstcw` writes two bytes to the caller's live control word.
    unsafe {
        core::arch::asm!("fnstcw [{control}]", control = in(reg) control, options(nostack))
    };
}

/// `fpextovrflt()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Runs from the trap handler on the current thread, at spl0.
pub(crate) unsafe fn fpextovrflt() -> ! {
    let thread = current_thread();
    // SAFETY: the trap runs on the current thread, whose pcb `pcb_init()`
    // built.
    let pcb = unsafe { (*thread).pcb };
    // SAFETY: the lock protects `ims.ifps`.
    let ifps = unsafe {
        (*pcb).lock.lock();
        let ifps = (*pcb).ims.ifps;
        (*pcb).ims.ifps = ptr::null_mut();
        (*pcb).lock.unlock();
        ifps
    };

    clear_ts();
    fninit();
    set_ts();

    // SAFETY: the pointer was the thread's own save area, now detached.
    unsafe { free_fp_state(ifps) };

    // SAFETY: `i386_exception()` does not return, as the C annotated.
    unsafe {
        trap::i386_exception(EXC_BAD_ACCESS, VM_PROT_READ | VM_PROT_EXECUTE, 0)
    }
}

/// `EXC_BAD_ACCESS` of <mach/exception.h>.
const EXC_BAD_ACCESS: c_int = 1;
/// `EXC_ARITHMETIC` of <mach/exception.h>.
const EXC_ARITHMETIC: c_int = 2;
/// `EXC_I386_EXTERR` of <mach/i386/exception.h>.
const EXC_I386_EXTERR: c_int = 4;
/// `VM_PROT_READ` of <mach/vm_prot.h>.
const VM_PROT_READ: c_int = 1;
/// `VM_PROT_EXECUTE` of <mach/vm_prot.h>.
const VM_PROT_EXECUTE: c_int = 4;

/// `fphandleerr()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Runs on the current thread, at spl0.
unsafe fn fphandleerr() -> c_int {
    let thread = current_thread();
    clear_ts();
    // SAFETY: the current thread's pcb is live and its save area belongs to
    // it.
    unsafe { fp_save(thread) };
    fninit();
    set_ts();
    0
}

/// The FPU status word the C raised `EXC_I386_EXTERR` with.
///
/// # Safety
///
/// `thread` must be the current thread, whose save area is live.
unsafe fn fp_status_word(thread: *mut Thread) -> c_int {
    // SAFETY: the caller promises the current thread and a live pcb.
    unsafe {
        let ifps = (*(*thread).pcb).ims.ifps;
        if save_kind() != FpSaveKind::FnSave {
            (*ifps).save.xfp_save_state.fp_status as c_int
        } else {
            (*ifps).save.native.fp_save_state.fp_status as c_int
        }
    }
}

/// `fpexterrflt()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Runs from the exception handler on the current thread, at spl0.
pub(crate) unsafe fn fpexterrflt() {
    let thread = current_thread();
    // SAFETY: the caller runs on the current thread at spl0.
    if unsafe { fphandleerr() } != 0 {
        return;
    }
    // SAFETY: `i386_exception()` does not return.
    unsafe {
        trap::i386_exception(
            EXC_ARITHMETIC,
            EXC_I386_EXTERR,
            fp_status_word(thread) as c_long,
        )
    }
}

/// `fpastintr()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Runs from the AST on the current thread, at spl0.
pub(crate) unsafe fn fpastintr() {
    let thread = current_thread();
    // SAFETY: the caller runs on the current thread at spl0, with no lock.
    unsafe { fp_save(thread) };
    // SAFETY: `i386_exception()` does not return.
    unsafe {
        trap::i386_exception(
            EXC_ARITHMETIC,
            EXC_I386_EXTERR,
            fp_status_word(thread) as c_long,
        )
    }
}

/// `fp_load()` of i386/i386/fpu.c.
///
/// # Safety
///
/// `thread` must be the current thread, whose pcb is live, and the caller
/// must hold no pcb lock.
pub(crate) unsafe fn fp_load(thread: *mut Thread) {
    // SAFETY: the caller promises the current thread.
    let pcb = unsafe { (*thread).pcb };
    // SAFETY: the pcb is live; the C reads `ims.ifps` without the lock here
    // because no other CPU runs this thread.
    unsafe {
        let mut ifps = (*pcb).ims.ifps;
        if ifps.is_null() {
            ifps = alloc_fp_state();
            ptr::copy_nonoverlapping(
                FP_DEFAULT_STATE.cast::<u8>(),
                ifps.cast::<u8>(),
                offset_of!(I386FpSaveState, save) + xfp_save_size() as usize,
            );
            (*pcb).ims.ifps = ifps;
            fpinit(thread);
        } else if (*ifps).fp_valid == 2 {
            (*ifps).fp_valid = 1;
            set_ts();
            trap::i386_exception(
                EXC_ARITHMETIC,
                EXC_I386_EXTERR,
                fp_status_word(thread) as c_long,
            );
        } else if (*ifps).fp_valid == 0 {
            glue::printf(c"fp_load: invalid FPU state!\n".as_ptr());
            fninit();
        } else {
            fpu_rstor(ifps);
        }
        (*ifps).fp_valid = 0;
    }
}

/// `fpintr()` of i386/i386/fpu.c.
///
/// # Safety
///
/// Runs from the IRQ handler on the interrupt stack, at spl1.
pub(crate) unsafe fn fpintr(_unit: c_int) {
    // SAFETY: writing port 0xf0 clears the AT's coprocessor error latch.
    crate::arch::i386::pio::Port::new(0xf0).write_u8(0);
    // SAFETY: the caller runs on the current thread; `fphandleerr()` saves
    // its state.
    if unsafe { fphandleerr() } != 0 {
        return;
    }
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>.
    let s = unsafe { glue::splsched() };
    crate::kern::ast::ast_on(cpu_number(), AST_I386_FP);
    // SAFETY: `s` came from `splsched()`.
    unsafe { glue::splx(s) };
}

/// `AST_I386_FP` of <i386/ast.h>: the delayed FPU exception, an AST bit.
const AST_I386_FP: usize = 0x8000_0000;

/// `fpnoextflt()` of i386/i386/fpu.c: coprocessor not present.
///
/// # Safety
///
/// Runs from the trap handler on the current thread, at spl0.
pub(crate) unsafe fn fpnoextflt() {
    clear_ts();
    // SAFETY: the trap runs on the current thread, which is the thread
    // `fp_load()` wants; `fpu_module_init()` built the cache before any
    // thread can reach this trap.
    unsafe { fp_load(current_thread()) };
}

const _: () = assert!(
    FP_STATE_BYTES == 108,
    "i386_fp_save plus i386_fp_regs is the 108-byte FNSAVE image"
);
