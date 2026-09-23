// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/percpu.h and i386/i386/cpu_number.h:
//   Copyright (c) 2023 Free Software Foundation, Inc.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The per-CPU block, which `i386/i386/percpu.h` declares and
//! `i386/i386/cpu_number.h` and `kern/processor.h` read.
//!
//! The C accessors are `%gs`-relative: the segment base points at the
//! running CPU's [`Percpu`], so a field is one `mov` and the block's
//! address is the `self` pointer at offset zero.  The asm here mirrors
//! `percpu_get()` and `percpu_ptr()` instruction for instruction; the
//! first four fields of `Percpu` are the C ones, and the rest is the
//! embedded `struct processor`.

use crate::arch::types::VmOffset;
use crate::kern::processor::Processor;
use crate::kern::thread::Thread;
use core::arch::asm;
use core::ffi::c_int;
use core::mem::offset_of;

/// `struct percpu` of <i386/percpu.h>.
#[repr(C)]
pub struct Percpu {
    /// `self`: the block's own address; `%gs:0`.
    pub self_ptr: *mut Percpu,
    pub apic_id: c_int,
    pub cpu_id: c_int,
    pub processor: Processor,
    pub active_thread: *mut Thread,
    pub active_stack: VmOffset,
}

/// The C compiler's `sizeof(struct percpu)` for each kernel.
#[cfg(target_pointer_width = "64")]
const PERCPU_SIZE: usize = 1208;
#[cfg(target_pointer_width = "32")]
const PERCPU_SIZE: usize = 620;

/// The number of the current CPU.  `percpu_get(int, cpu_id)` in
/// <i386/percpu.h>, which the inline `cpu_number()` wraps.
pub fn cpu_number() -> c_int {
    let cpu: c_int;
    // SAFETY: the kernel keeps `%gs` based at the running CPU's
    // `struct percpu` from the first context switch on, so the
    // `cpu_id` field is readable.  The load does not write memory,
    // touch the stack or change the flags.
    unsafe {
        asm!(
            "mov {cpu:e}, gs:[{off}]",
            cpu = out(reg) cpu,
            off = const offset_of!(Percpu, cpu_id),
            options(nostack, preserves_flags, readonly),
        );
    }
    cpu
}

/// The processor record of the current CPU.  `percpu_ptr(struct
/// processor, processor)` in <kern/processor.h>: the `self` pointer at
/// `%gs:0` plus the field offset.
pub fn current_processor() -> *mut Processor {
    let base: usize;
    // SAFETY: as `cpu_number()`; `%gs:0` is the block's `self` field.
    unsafe {
        asm!(
            "mov {base}, gs:[0]",
            base = out(reg) base,
            options(nostack, preserves_flags, readonly),
        );
    }
    // The C adds the offset to the self pointer; a valid `struct
    // processor` lies inside the block.
    (base + offset_of!(Percpu, processor)) as *mut Processor
}

/// The thread running on the current CPU.  `current_thread()` in
/// <kern/thread.h>, which is `percpu_get(thread_t, active_thread)`.
pub fn current_thread() -> *mut Thread {
    let thread: *mut Thread;
    // SAFETY: as `cpu_number()`; `active_thread` is always set for a
    // CPU that is executing code.
    unsafe {
        asm!(
            "mov {thread}, gs:[{off}]",
            thread = out(reg) thread,
            off = const offset_of!(Percpu, active_thread),
            options(nostack, preserves_flags, readonly),
        );
    }
    thread
}

/// The kernel stack of the thread running on the current CPU.
/// `current_stack()` in <kern/thread.h>, which is
/// `percpu_get(vm_offset_t, active_stack)`.
pub fn current_stack() -> VmOffset {
    let stack: VmOffset;
    // SAFETY: as `cpu_number()`; `active_stack` is set by the context
    // switch on the way to the thread that is running.
    unsafe {
        asm!(
            "mov {stack}, gs:[{off}]",
            stack = out(reg) stack,
            off = const offset_of!(Percpu, active_stack),
            options(nostack, preserves_flags, readonly),
        );
    }
    stack
}

// `percpu_array` in <i386/percpu.h>: one block per CPU.  The C
// declares the whole `NCPUS`-element array; the mirror names the first
// element because `NCPUS` is a C constant, and the pointer arithmetic
// below strides one block at a time.
//
// The FFI lint treats the `PhantomPinned` marker at the end of
// `QueueEntry` as poison even embedded in a `#[repr(C)]` mirror; the C
// side hands over the same block, and the offsets are asserted below.
#[expect(improper_ctypes)]
unsafe extern "C" {
    static mut percpu_array: Percpu;
}

/// The per-CPU block of CPU `cpu` in `percpu_array`.
///
/// # Safety
///
/// `cpu` must be a CPU number the machine reports, below
/// `smp_get_numcpus()`.
pub unsafe fn percpu_at(cpu: c_int) -> *mut Percpu {
    // SAFETY: the caller promises a live CPU number, and the C array
    // holds one block for each CPU the probe counted.  `cpu` is a
    // non-negative CPU number, and widening it to `isize` is exact.
    unsafe { (&raw mut percpu_array).offset(cpu as isize) }
}

// `struct percpu` is the C block; the embedded `Processor` must match
// `struct processor` exactly for `active_thread`'s offset to be right.
const _: () = assert!(size_of::<Percpu>() == PERCPU_SIZE);
const _: () = assert!(offset_of!(Percpu, self_ptr) == 0);
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(Percpu, apic_id) == 8);
    assert!(offset_of!(Percpu, cpu_id) == 12);
    assert!(offset_of!(Percpu, processor) == 16);
    assert!(offset_of!(Percpu, active_thread) == 1192);
    assert!(offset_of!(Percpu, active_stack) == 1200);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(offset_of!(Percpu, apic_id) == 4);
    assert!(offset_of!(Percpu, cpu_id) == 8);
    assert!(offset_of!(Percpu, processor) == 12);
    assert!(offset_of!(Percpu, active_thread) == 612);
    assert!(offset_of!(Percpu, active_stack) == 616);
};
