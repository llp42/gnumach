// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/percpu.h and i386/i386/cpu_number.h:
//   Copyright (c) 2023 Free Software Foundation, Inc.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The per-CPU block, which `i386/i386/percpu.h` declares and
//! `i386/i386/cpu_number.h` and `kern/processor.h` read.

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

/// The number of the current CPU.
pub fn cpu_number() -> c_int {
    let cpu: c_int;
    // SAFETY: the kernel keeps `%gs` based at the running CPU's `struct
    // percpu` from the first context switch on, so the `cpu_id` field is
    // readable.
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

/// The processor record of the current CPU.
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
    (base + offset_of!(Percpu, processor)) as *mut Processor
}

/// `current_thread()` in <kern/thread.h>, which is `percpu_get(thread_t,
/// active_thread)`.
pub fn current_thread() -> *mut Thread {
    let thread: *mut Thread;
    // SAFETY: as `cpu_number()`; `active_thread` is always set for a CPU that
    // is executing code.
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

/// `current_stack()` in <kern/thread.h>, which is `percpu_get(vm_offset_t,
/// active_stack)`.
pub fn current_stack() -> VmOffset {
    let stack: VmOffset;
    // SAFETY: as `cpu_number()`; `active_stack` is set by the context switch
    // on the way to the thread that is running.
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

/// `percpu_assign(active_thread, thread)` of <i386/percpu.h>: make `thread`
/// the running thread of the current CPU.
///
/// # Safety
///
/// The caller must be the context switcher for this CPU: it is about to
/// resume `thread`, and no other CPU may run it.
pub unsafe fn set_active_thread(thread: *mut Thread) {
    // SAFETY: `%gs` is based at the running CPU's block from the first
    // context switch on, and `thread` becomes the thread that block names.
    unsafe {
        asm!(
            "mov gs:[{off}], {src}",
            src = in(reg) thread,
            off = const offset_of!(Percpu, active_thread),
            options(nostack, preserves_flags),
        );
    }
}

// `percpu_array` in <i386/percpu.h>: one block per CPU.
#[expect(improper_ctypes)]
unsafe extern "C" {
    static mut percpu_array: Percpu;
}

/// The `processor_ptr()` macro of <kern/processor.h>: CPU `cpu`'s processor
/// record.
///
/// # Safety
///
/// `cpu` must be a CPU number the machine reports, below `smp_get_numcpus()`.
pub(crate) unsafe fn processor_ptr(cpu: c_int) -> *mut Processor {
    // SAFETY: the caller promises a live CPU number, so the block the C array
    // holds for it has a live processor.
    unsafe { &raw mut (*percpu_at(cpu)).processor }
}

/// The per-CPU block of CPU `cpu` in `percpu_array`.
///
/// # Safety
///
/// `cpu` must be a CPU number the machine reports, below `smp_get_numcpus()`.
pub unsafe fn percpu_at(cpu: c_int) -> *mut Percpu {
    // SAFETY: the caller promises a live CPU number, and the C array holds one
    // block for each CPU the probe counted.
    unsafe { (&raw mut percpu_array).offset(cpu as isize) }
}

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
