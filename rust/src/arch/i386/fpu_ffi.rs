// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/fpu.c and i386/i386/fpu.h:
//   Copyright (c) 1992-1990 Carnegie Mellon University
//   Copyright (C) 1994 Linus Torvalds
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of the FPU unit, which `i386/i386/fpu.c` used to
//! define and `i386/i386/fpu.h` declares.

use crate::arch::i386::fpu;
use crate::arch::i386::fpu::I386FpSaveState;
use crate::arch::types::VmSize;
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};

/// The `kern_return_t` a Rust result stands for.
fn kern_return(result: Result<(), KernError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error.as_u8()),
    }
}

/// `init_fpu()` of i386/i386/fpu.h, called on each CPU at boot.
///
/// # Safety
///
/// Runs before the calling CPU schedules any thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_fpu() {
    // SAFETY: the caller's contract.
    unsafe { fpu::init_fpu() };
}

/// `i386_get_xstate_size()` of i386/i386/fpu.h.
///
/// # Safety
///
/// `size` must be valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_get_xstate_size(
    host: *mut c_void,
    size: *mut VmSize,
) -> c_int {
    // SAFETY: the caller promises the size pointer.
    kern_return(unsafe { fpu::i386_get_xstate_size(host, size) })
}

/// `fpu_module_init()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Called once at startup, before any thread can save FPU state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpu_module_init() {
    // SAFETY: the caller's contract.
    unsafe { fpu::fpu_module_init() };
}

/// `fpu_set_state()` of i386/i386/fpu.h.
///
/// # Safety
///
/// `thread` must point at a live thread, and `state` must hold the record
/// `flavor` names, readable for the C's `i386_FLOAT_STATE_COUNT` or the
/// XFLOAT record's current size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpu_set_state(
    thread: *mut Thread,
    state: *mut c_void,
    flavor: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { fpu::fpu_set_state(thread, state, flavor) })
}

/// `fpu_get_state()` of i386/i386/fpu.h.
///
/// # Safety
///
/// `thread` must point at a live thread, and `state` must be writable for
/// the record `flavor` names, at the C's `i386_FLOAT_STATE_COUNT` or the
/// XFLOAT record's current size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpu_get_state(
    thread: *mut Thread,
    state: *mut c_void,
    flavor: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { fpu::fpu_get_state(thread, state, flavor) })
}

/// `fp_save()` of i386/i386/fpu.h.
///
/// # Safety
///
/// `thread` must point at a live thread whose FPU state may be saved from
/// the current context, as the FPU traps and `fpu_get_state()` do.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fp_save(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { fpu::fp_save(thread) };
}

/// `fp_load()` of i386/i386/fpu.h.
///
/// # Safety
///
/// `thread` must be the current thread, and the caller must hold no pcb
/// lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fp_load(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { fpu::fp_load(thread) };
}

/// `fpinherit()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Both threads must be live, and the caller must hold no FPU state lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpinherit(
    parent_thread: *mut Thread,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { fpu::fpinherit(parent_thread, thread) };
}

/// `fpextovrflt()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Runs from the trap handler on the current thread, at spl0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpextovrflt() {
    // SAFETY: the caller's contract; the core never returns.
    unsafe { fpu::fpextovrflt() };
}

/// `fpexterrflt()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Runs from the exception handler on the current thread, at spl0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpexterrflt() {
    // SAFETY: the caller's contract; the core never returns.
    unsafe { fpu::fpexterrflt() };
}

/// `fpastintr()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Runs from the AST on the current thread, at spl0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpastintr() {
    // SAFETY: the caller's contract; the core never returns.
    unsafe { fpu::fpastintr() };
}

/// `fpintr()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Runs from the IRQ handler on the interrupt stack, at spl1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpintr(unit: c_int) {
    // SAFETY: the caller's contract.
    unsafe { fpu::fpintr(unit) };
}

/// `fpnoextflt()` of i386/i386/fpu.h.
///
/// # Safety
///
/// Runs from the trap handler on the current thread, at spl0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpnoextflt() {
    // SAFETY: the caller's contract.
    unsafe { fpu::fpnoextflt() };
}

/// `fp_free()` of i386/i386/fpu.h.
///
/// # Safety
///
/// `fps` must be a live save area from the FPU cache, and the caller must
/// give it up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fp_free(fps: *mut I386FpSaveState) {
    // SAFETY: the caller's contract.
    unsafe { fpu::free_fp_state(fps) };
}
