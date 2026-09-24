// SPDX-License-Identifier: CMU-Mach
// Derived from kern/thread.c and kern/thread.h:
//   Copyright (c) 1994-1987 Carnegie Mellon University.
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the thread module, one adapter per symbol
//! `kern/thread.c` used to define and `kern/thread.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::processor::ProcessorSet;
use crate::kern::task::Task;
use crate::kern::thread::{Continuation, StackResume, Thread, default_pset};
use crate::kern::types::KernError;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

/// `thread_init()` of kern/thread.c.
///
/// # Safety
///
/// Runs once, from the boot sequence, before the first thread exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_init() {
    // SAFETY: the caller's contract.
    unsafe { Thread::init() };
}

/// `thread_timer_delta()` of kern/sched.h, which used to be a macro.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds, at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timer_delta(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::timer_delta(thread) };
}

/// `thread_assign_default()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or a live thread the caller holds an extra reference
/// to, and the caller must hold no locks: `thread_assign()` may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_assign_default(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract is `thread_assign()`'s own, and the
    // default set is a live `struct processor_set` for the life of the kernel.
    match unsafe { Thread::assign(thread, default_pset()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `stack_alloc_try()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must point at a live thread whose lock the caller holds at
/// splsched, and `resume` must be a stack continuation.
#[unsafe(no_mangle)]
#[must_use]
pub unsafe extern "C" fn stack_alloc_try(
    thread: *mut Thread,
    resume: StackResume,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { (*thread).stack_alloc_try(resume) })
}

/// `stack_alloc()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must point at a live thread, the caller must hold no spin lock
/// because the allocation may block, and `resume` must be a stack
/// continuation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_alloc(
    thread: *mut Thread,
    resume: StackResume,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*thread).stack_alloc(resume) };
    0
}

/// `stack_free()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must point at a live thread whose lock the caller holds at
/// splsched, with a stack attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_free(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { (*thread).stack_free() };
}

/// `stack_collect()` of kern/thread.h.
#[unsafe(no_mangle)]
pub extern "C" fn stack_collect() {
    // SAFETY: `stack_collect()` takes its own splsched level and drops it
    // around each release; the caller holds no lock.
    unsafe { Thread::stack_collect() };
}

/// `stack_privilege()` of kern/thread.h.
///
/// # Safety
///
/// `thread` must be the current thread; the C halts the kernel otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_privilege(thread: *mut Thread) {
    // SAFETY: the caller's contract; the core compares `thread` with
    // `current_thread()` and halts when they differ.
    unsafe { (*thread).stack_privilege() };
}

/// `stack_init()` of kern/thread.c.
///
/// # Safety
///
/// `stack` must be the base address of a live `KERNEL_STACK_SIZE` kernel stack
/// object: the cache allocator's, or one the machine-dependent code is about
/// to install.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_init(stack: VmOffset) {
    // SAFETY: the caller's contract.
    unsafe { crate::kern::thread::stack_init(stack) };
}

/// `thread_reference()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_reference(thread: *mut Thread) {
    let Some(thread) = NonNull::new(thread) else {
        return;
    };
    // SAFETY: the caller's contract.
    unsafe { Thread::reference(thread.as_ptr()) };
}

/// `thread_force_terminate()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread that is not the current thread, as
/// `task_terminate()` guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_force_terminate(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::force_terminate(thread) };
}

/// `thread_hold()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_hold(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::hold(thread) };
}

/// `thread_release()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_release(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::release(thread) };
}

/// `thread_resume()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_resume(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; `resume()` checks the null the C checked.
    match unsafe { Thread::resume(thread) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_abort()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread; the routine takes the
/// thread's locks itself and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_abort(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; `abort()` checks the null and
    // current-thread arguments the C checked.
    match unsafe { Thread::abort(thread) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_start()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread; the C stores `start` in its
/// `swap_func` field.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_start(
    thread: *mut Thread,
    start: Continuation,
) {
    // SAFETY: the caller's contract.
    unsafe { (*thread).start(start) };
}

/// `thread_unfreeze()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_unfreeze(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::unfreeze(thread) };
}

/// `thread_get_assignment()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `pset` must be valid
/// for a write; the MIG server passes the address of its own
/// `processor_set_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_assignment(
    thread: *mut Thread,
    pset: *mut *mut ProcessorSet,
) -> c_int {
    // SAFETY: the caller's contract; `assignment()` checks the null the C
    // checked.
    match unsafe { Thread::assignment(thread) } {
        Ok(assignment) => {
            // SAFETY: the caller promises the out-parameter.
            unsafe { *pset = assignment };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `thread_get_state()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread; `old_state` must be
/// writable for the words `*old_state_count` names, and `old_state_count` must
/// be valid for a read and a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_state(
    thread: *mut Thread,
    flavor: c_int,
    old_state: *mut c_uint,
    old_state_count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract; `get_status()` checks the null and
    // current-thread arguments the C checked.
    match unsafe {
        Thread::get_status(thread, flavor, old_state, old_state_count)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_set_state()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `new_state` must be
/// readable for `new_state_count` words.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_state(
    thread: *mut Thread,
    flavor: c_int,
    new_state: *mut c_uint,
    new_state_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract; `set_status()` checks the null and
    // current-thread arguments the C checked.
    match unsafe {
        Thread::set_status(thread, flavor, new_state, new_state_count)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_priority()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_priority(
    thread: *mut Thread,
    priority: c_int,
    set_max: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `set_max` is the C boolean.
    match unsafe { Thread::priority(thread, priority, set_max != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_set_own_priority()` of kern/thread.c.
///
/// # Safety
///
/// The caller must be the current thread and hold no thread lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_own_priority(priority: c_int) {
    // SAFETY: the caller's contract.
    unsafe { Thread::set_own_priority(priority) };
}

/// `thread_max_priority()` of kern/thread.c.
///
/// # Safety
///
/// `thread` and `pset` must be null or point at live objects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_max_priority(
    thread: *mut Thread,
    pset: *mut ProcessorSet,
    max_priority: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `max_priority()` checks the null
    // arguments the C checked.
    match unsafe { Thread::max_priority(thread, pset, max_priority) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_policy()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_policy(
    thread: *mut Thread,
    policy: c_int,
    data: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `policy()` checks the null argument the C
    // checked.
    match unsafe { Thread::policy(thread, policy, data) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_wire()` of kern/thread.c.
///
/// # Safety
///
/// `host` must be null or a live `struct host`; `thread` must be null or point
/// at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_wire(
    host: *mut c_void,
    thread: *mut Thread,
    wired: c_int,
) -> c_int {
    if host.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    // SAFETY: the caller's contract; `wire()` checks the null and
    // current-thread arguments the C checked.
    match unsafe { Thread::wire(thread, wired != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_stats()` of kern/thread.c.
///
/// # Safety
///
/// Reached from the debugger; the queue walk takes no locks, exactly as the C
/// did, so every link must be a live thread for the walk.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_stats() {
    // SAFETY: the caller's contract.
    unsafe { Thread::stats() };
}

/// `thread_set_name()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `name` must be
/// readable up to `TASK_NAME_SIZE - 1` bytes or a NUL inside them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_name(
    thread: *mut Thread,
    name: *const c_char,
) -> c_int {
    // SAFETY: the caller's contract; `set_name()` checks the null argument the
    // C checked.
    match unsafe { Thread::set_name(thread, name) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_get_name()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `name` must be
/// writable for `TASK_NAME_SIZE` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_name(
    thread: *mut Thread,
    name: *mut c_char,
) -> c_int {
    // SAFETY: the caller's contract; `get_name()` checks the null argument the
    // C checked.
    match unsafe { Thread::get_name(thread, name) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_create()` of kern/thread.c.
///
/// # Safety
///
/// `parent_task` must be null or a live task, and `child_thread` must be
/// writable for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_create(
    parent_task: *mut Task,
    child_thread: *mut *mut Thread,
) -> c_int {
    // SAFETY: the null check and the writable slot are the caller's contract;
    // `create()` writes the slot's value on success only.
    match unsafe { Thread::create(parent_task) } {
        Ok(thread) => {
            // SAFETY: the caller promises `child_thread` is writable.
            unsafe { child_thread.write(thread) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `thread_deallocate()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or a live thread the caller holds a reference to,
/// and the caller must hold no locks: the teardown may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_deallocate(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::deallocate(thread) };
}

/// `thread_terminate()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or a live thread, and the caller must hold no locks:
/// the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_terminate(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; `terminate()` checks the null the C
    // checked.
    match unsafe { Thread::terminate(thread) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_terminate_release()` of kern/thread.c.
///
/// # Safety
///
/// `thread` and `task` must be null or point at live objects, and the caller
/// must hold no locks: the routine deallocates and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_terminate_release(
    thread: *mut Thread,
    task: *mut Task,
    thread_name: c_uint,
    reply_port: c_uint,
    address: VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller's contract; `terminate_release()` checks the null
    // arguments the C checked.
    match unsafe {
        Thread::terminate_release(
            thread,
            task,
            thread_name,
            reply_port,
            address,
            size,
        )
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_halt()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be a live thread other than the current one, and the caller
/// must hold no locks: the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_halt(
    thread: *mut Thread,
    must_halt: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `must_halt` is the C boolean.
    match unsafe { Thread::halt(thread, must_halt != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_halt_self()` of kern/thread.c.
///
/// # Safety
///
/// Runs on the current thread, which must be at a clean kernel point; the
/// continuation runs when the thread is released to halt.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_halt_self(continuation: Continuation) {
    // SAFETY: the caller's contract.
    unsafe { Thread::halt_self(continuation) };
}

/// `thread_dowait()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be a live thread other than the current one, and the caller
/// must hold no locks: the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_dowait(
    thread: *mut Thread,
    must_halt: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `must_halt` is the C boolean.
    match unsafe { Thread::dowait(thread, must_halt != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_suspend()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or a live thread, and the caller must hold no locks:
/// the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_suspend(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; `suspend()` checks the null the C
    // checked.
    match unsafe { Thread::suspend(thread) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_info()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or a live thread; `thread_info` must be writable for
/// `*thread_info_count` `integer_t`s, and `thread_info_count` must be readable
/// and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_info(
    thread: *mut Thread,
    flavor: c_int,
    thread_info: *mut c_int,
    thread_info_count: *mut c_uint,
) -> c_int {
    if thread.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises the count pointer is readable.
    let count = unsafe { *thread_info_count };

    // SAFETY: the caller's contract.
    match unsafe { Thread::info(thread, flavor, thread_info, count) } {
        Ok(count) => {
            // SAFETY: the caller promises the count pointer is writable.
            unsafe { *thread_info_count = count };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `kernel_thread()` of kern/thread.c.
///
/// # Safety
///
/// `task` must point at a live task, `name` must be a NUL-terminated string,
/// `start` must be a continuation, and the caller must hold no locks: the
/// routine may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel_thread(
    task: *mut Task,
    name: *const c_char,
    start: Continuation,
    arg: *mut c_void,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { crate::kern::thread::kernel_thread(task, name, start, arg) }
}

/// `reaper_thread()` of kern/thread.c.
///
/// # Safety
///
/// Runs as the reaper kernel thread, which the boot starts once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reaper_thread() {
    // SAFETY: the caller's contract; the loop never returns.
    unsafe { crate::kern::thread::reaper_thread_continue() };
}

/// `thread_assign()` of kern/thread.c, the `MACH_HOST` arm both configured
/// builds take.
///
/// # Safety
///
/// `thread` must be null or a live thread the caller holds an extra reference
/// to, `new_pset` must be null or a live processor set, and the caller must
/// hold no locks: the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_assign(
    thread: *mut Thread,
    new_pset: *mut ProcessorSet,
) -> c_int {
    // SAFETY: the caller's contract; `assign()` checks the null arguments the
    // C checked.
    match unsafe { Thread::assign(thread, new_pset) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_freeze()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must hold no locks:
/// the wait may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_freeze(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::freeze(thread) };
}

/// `thread_doassign()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread and `new_pset` at a live processor
/// set, and the caller must hold no locks: the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_doassign(
    thread: *mut Thread,
    new_pset: *mut ProcessorSet,
    release_freeze: c_int,
) {
    // SAFETY: the caller's contract; `release_freeze` is the C boolean.
    unsafe { Thread::doassign(thread, new_pset, release_freeze != 0) };
}

/// `consider_thread_collect()` of kern/thread.c.
///
/// # Safety
///
/// The pageout daemon calls this with nothing locked, as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn consider_thread_collect() {
    // SAFETY: the caller's contract.
    unsafe { crate::kern::thread::consider_collect() };
}

/// `stack_finalize()` of kern/thread.c.
///
/// # Safety
///
/// `stack` must be the base address of a live `KERNEL_STACK_SIZE` kernel stack
/// object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_finalize(stack: VmOffset) {
    // SAFETY: the caller's contract.
    unsafe { crate::kern::thread::stack_finalize(stack) };
}

/// `host_stack_usage()` of kern/thread.c.
///
/// # Safety
///
/// `host` must be null or the live host the MIG stub converted, and every out
/// pointer must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_stack_usage(
    host: *mut c_void,
    reservedp: *mut VmSize,
    totalp: *mut c_uint,
    spacep: *mut VmSize,
    residentp: *mut VmSize,
    maxusagep: *mut VmSize,
    maxstackp: *mut VmOffset,
) -> c_int {
    // SAFETY: the caller's contract.
    let usage = match unsafe { crate::kern::thread::host_stack_usage(host) } {
        Ok(usage) => usage,
        Err(error) => return c_int::from(error),
    };

    // SAFETY: the caller promises every out pointer is writable; the C wrote
    // them only on success.
    unsafe {
        reservedp.write(0);
        totalp.write(usage.total);
        spacep.write(usage.space);
        residentp.write(usage.space);
        maxusagep.write(usage.maxusage);
        maxstackp.write(usage.maxstack);
    }
    0
}

/// `processor_set_stack_usage()` of kern/thread.c.
///
/// # Safety
///
/// `pset` must be null or point at a live processor set, and the caller must
/// hold no locks: the routine allocates.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_stack_usage(
    pset: *mut ProcessorSet,
    totalp: *mut c_uint,
    spacep: *mut VmSize,
    residentp: *mut VmSize,
    maxusagep: *mut VmSize,
    maxstackp: *mut VmOffset,
) -> c_int {
    // SAFETY: the caller's contract.
    let usage = match unsafe {
        crate::kern::thread::processor_set_stack_usage(pset)
    } {
        Ok(usage) => usage,
        Err(error) => return c_int::from(error),
    };

    // SAFETY: the caller promises every out pointer is writable; the C wrote
    // them only on success.
    unsafe {
        totalp.write(usage.total);
        spacep.write(usage.space);
        residentp.write(usage.space);
        maxusagep.write(usage.maxusage);
        maxstackp.write(usage.maxstack);
    }
    0
}
