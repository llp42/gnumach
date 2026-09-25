// SPDX-License-Identifier: CMU-Mach
// Derived from kern/sched_prim.c:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the scheduler, one adapter per symbol
//! `kern/sched_prim.c` used to define and `kern/sched_prim.h` declares.

use crate::kern::lock::SimpleLock;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::sched::RunQueue;
use crate::kern::sched_prim;
use crate::kern::thread::{Continuation, Thread};
use core::ffi::{c_int, c_void};

/// `sched_init()` of kern/sched_prim.c.
///
/// # Safety
///
/// `kern/startup.c` calls this once during the boot, before any other CPU or
/// thread can reach the scheduler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_init() {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::sched_init() };
}

/// `thread_timeout_setup()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live, freshly created thread that no other CPU can see
/// yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timeout_setup(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_timeout_setup(thread) };
}

/// `thread_timeout()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be the value stored in a live thread's `timer.param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timeout(thread: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_timeout(thread) };
}

/// `assert_wait()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller must be the current thread, must not already be waiting, and
/// must prevent the wakeup from being lost until `thread_block()` or
/// `thread_sleep()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn assert_wait(
    event: *mut c_void,
    interruptible: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::assert_wait(event, interruptible) };
}

/// `clear_wait()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes its own locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clear_wait(
    thread: *mut Thread,
    result: c_int,
    interrupt_only: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::clear_wait(thread, result, interrupt_only) };
}

/// `thread_wakeup_prim()` of kern/sched_prim.h.
///
/// # Safety
///
/// `event` is an opaque wait key that any thread may wait on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_wakeup_prim(
    event: *mut c_void,
    one_thread: c_int,
    result: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_wakeup_prim(event, one_thread, result) }
}

/// `thread_sleep()` of kern/sched_prim.h.
///
/// # Safety
///
/// Same contract as `assert_wait()`, and `lock` must be a live simple lock the
/// current thread holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_sleep(
    event: *mut c_void,
    lock: *mut SimpleLock,
    interruptible: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_sleep(event, lock, interruptible) };
}

/// `thread_dispatch()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must be a live thread that is not on a run queue, and the caller
/// must be at splsched; the context switch calls this directly.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_dispatch(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_dispatch(thread) };
}

/// `thread_setrun()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must be a live thread locked by the caller at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_setrun(
    thread: *mut Thread,
    may_preempt: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_setrun(thread, may_preempt) };
}

/// `thread_invoke()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller must be at splsched, hold no run-queue lock, and both threads
/// must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_invoke(
    old_thread: *mut Thread,
    continuation: Continuation,
    new_thread: *mut Thread,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe {
        sched_prim::thread_invoke(old_thread, continuation, new_thread)
    })
}

/// `thread_block()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller must be the current thread, must not hold a spin lock, and must
/// have set its wait state first when it means to block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_block(continuation: Continuation) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_block(continuation) };
}

/// `thread_run()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller must be the current thread, must not hold a spin lock, and
/// `new_thread` must be live and runnable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_run(
    continuation: Continuation,
    new_thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_run(continuation, new_thread) };
}

/// `thread_set_timeout()` of kern/sched_prim.h.
///
/// # Safety
///
/// Must be called between `assert_wait()` and `thread_block()` for the current
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_timeout(t: c_int) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_set_timeout(t) };
}

/// `update_priority()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn update_priority(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::update_priority(thread) };
}

/// `compute_priority()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compute_priority(
    thread: *mut Thread,
    resched: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::compute_priority(thread, resched) };
}

/// `compute_my_priority()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller must hold the thread lock and know the thread is timesharing and
/// not depressed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compute_my_priority(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::compute_my_priority(thread) };
}

/// `thread_bind()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must be a live thread, and `processor` a live processor or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_bind(
    thread: *mut Thread,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_bind(thread, processor) };
}

/// `thread_continue()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller runs this on the current thread, at splsched, after a stack
/// swap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_continue(old_thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::thread_continue(old_thread) };
}

/// `recompute_priorities()` of kern/sched_prim.h.
///
/// # Safety
///
/// Called by the timeout machinery at splsoftclock, and once at boot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recompute_priorities(param: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::recompute_priorities(param) };
}

/// `set_pri()` of kern/sched_prim.h.
///
/// # Safety
///
/// `th` must be a live thread whose lock the caller holds at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_pri(th: *mut Thread, pri: c_int, resched: c_int) {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::set_pri(th, pri, resched) };
}

/// `choose_pset_thread()` of kern/sched_prim.h.
///
/// # Safety
///
/// The caller must be at splsched and must hold `pset`'s run-queue lock;
/// `myprocessor` must be the current processor and `pset` its processor set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn choose_pset_thread(
    myprocessor: *mut Processor,
    pset: *mut ProcessorSet,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::choose_pset_thread(myprocessor, pset) }
}

/// `choose_thread()` of kern/sched.h.
///
/// # Safety
///
/// The caller must be at splsched and hold no run-queue lock; `myprocessor`
/// must be the current processor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn choose_thread(
    myprocessor: *mut Processor,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::choose_thread(myprocessor) }
}

/// `rem_runq()` of kern/sched.h.
///
/// # Safety
///
/// `th` must be a live thread whose lock the caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rem_runq(th: *mut Thread) -> *mut RunQueue {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::rem_runq(th) }
}

/// `idle_thread()` of kern/sched_prim.h.
///
/// # Safety
///
/// `kern/startup.c` starts this as the processor's idle thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idle_thread() {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::idle_thread() };
}

/// `sched_thread()` of kern/sched_prim.h.
///
/// # Safety
///
/// `kern/startup.c` starts this as the "sched" kernel thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_thread() {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::sched_thread() };
}

/// `do_thread_scan()` of kern/sched_prim.h.
///
/// # Safety
///
/// Runs in thread context with no lock held; the scan takes its own.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_thread_scan() {
    // SAFETY: the caller's contract.
    unsafe { sched_prim::do_thread_scan() };
}
