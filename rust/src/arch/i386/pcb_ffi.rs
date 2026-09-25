// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/pcb.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of the PCB unit, which `i386/i386/pcb.c` used to
//! define and `i386/i386/pcb.h` declares.

use crate::arch::i386::pcb;
use crate::arch::i386::pcb::{ExecInfo, Pcb};
use crate::arch::types::{VmOffset, VmSize};
use crate::kern::task::Task;
use crate::kern::thread::{Continuation, StackResume, Thread};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_ushort};

/// The `kern_return_t` a Rust result stands for.
fn kern_return(result: Result<(), KernError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error.as_u8()),
    }
}

/// `stack_attach()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `thread` must be a live thread whose stack is not attached, `stack` must
/// be a whole kernel stack, and `continuation` a stack continuation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_attach(
    thread: *mut Thread,
    stack: VmOffset,
    continuation: StackResume,
) {
    // SAFETY: the caller's contract.
    unsafe { pcb::stack_attach(thread, stack, continuation) };
}

/// `switch_ktss()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `pcb` must be the live pcb of a thread about to run on this CPU.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn switch_ktss(pcb: *mut Pcb) {
    // SAFETY: the caller's contract.
    unsafe { pcb::switch_ktss(pcb) };
}

/// `update_ktss_iopb()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// The caller must hold the task's `iopb_lock`, and `new_iopb` must be
/// readable for `size` bytes when it is non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn update_ktss_iopb(new_iopb: *mut u8, size: c_ushort) {
    // SAFETY: the caller's contract.
    unsafe { pcb::update_ktss_iopb(new_iopb, size) };
}

/// `stack_handoff()` of <kern/sched_prim.h>.
///
/// # Safety
///
/// `old` must be the running thread and `new` the thread about to run, both
/// live and not running on any other CPU.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_handoff(old: *mut Thread, new: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { pcb::stack_handoff(old, new) };
}

/// `switch_context()` of <kern/sched_prim.h>.
///
/// # Safety
///
/// `old` must be the running thread and `new` the thread about to run, both
/// live and not running on any other CPU; `continuation` is where `old`
/// resumes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn switch_context(
    old: *mut Thread,
    continuation: Continuation,
    new: *mut Thread,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { pcb::switch_context(old, continuation, new) }
}

/// `pcb_module_init()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// Called once at startup, before any thread is created.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pcb_module_init() {
    // SAFETY: the caller's contract.
    unsafe { pcb::pcb_module_init() };
}

/// `pcb_init()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `parent_task` must be the task `thread` is being created in, both live,
/// called before the thread can run.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pcb_init(
    parent_task: *mut Task,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { pcb::pcb_init(parent_task, thread) };
}

/// `pcb_terminate()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `thread` must be a live thread that will not run again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pcb_terminate(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { pcb::pcb_terminate(thread) };
}

/// `thread_setstatus()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `thread` must point at a live thread, and `tstate` must be readable for
/// `count` words of the record `flavor` names.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_setstatus(
    thread: *mut Thread,
    flavor: c_int,
    tstate: *mut c_uint,
    count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe {
        pcb::thread_setstatus(thread, flavor, tstate, count)
    })
}

/// `thread_getstatus()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `thread` must point at a live thread, `tstate` must be writable for
/// `count` words of the record `flavor` names, and `count` must be valid for
/// a read and a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_getstatus(
    thread: *mut Thread,
    flavor: c_int,
    tstate: *mut c_uint,
    count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe {
        pcb::thread_getstatus(thread, flavor, tstate, count)
    })
}

/// `thread_set_syscall_return()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_syscall_return(
    thread: *mut Thread,
    retval: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { pcb::thread_set_syscall_return(thread, retval) };
}

/// `user_stack_low()` of `i386/i386/pcb.h`.
#[unsafe(no_mangle)]
pub extern "C" fn user_stack_low(stack_size: VmSize) -> VmOffset {
    pcb::user_stack_low(stack_size)
}

/// `set_user_regs()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// Runs on the current thread, whose pcb is live, and `exec_info` points at
/// a live record.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_user_regs(
    stack_base: VmOffset,
    stack_size: VmOffset,
    exec_info: *const ExecInfo,
    arg_size: VmSize,
) -> VmOffset {
    // SAFETY: the caller's contract.
    unsafe { pcb::set_user_regs(stack_base, stack_size, exec_info, arg_size) }
}

/// `stack_detach()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must own its
/// `kernel_stack` field for the duration.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_detach(thread: *mut Thread) -> VmOffset {
    // SAFETY: the caller's contract.
    unsafe { pcb::stack_detach(thread) }
}

/// `load_context()` of `i386/i386/pcb.h`.
///
/// # Safety
///
/// `new` must point at a live thread whose kernel stack and saved context
/// are ready to resume, and no other CPU may be running it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn load_context(new: *mut Thread) -> ! {
    // SAFETY: the caller's contract.
    unsafe { pcb::load_context(new) }
}

/// `pcb_collect()` of `i386/i386/pcb.h`, whose body the C already emptied.
#[unsafe(no_mangle)]
pub extern "C" fn pcb_collect(_thread: *mut Thread) {}
