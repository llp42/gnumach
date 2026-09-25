// SPDX-License-Identifier: CMU-Mach
// Derived from kern/processor.c and kern/processor.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries `kern/processor.c` used to define.

use crate::kern::processor::{Processor, ProcessorSet, ProcessorSetInfo};
use crate::kern::processor_info::{
    PROCESSOR_BASIC_INFO_COUNT, PROCESSOR_SET_BASIC_INFO_COUNT,
    PROCESSOR_SET_SCHED_INFO_COUNT, ProcessorBasicInfo, ProcessorSetBasicInfo,
    ProcessorSetSchedInfo,
};
use crate::kern::task::Task;
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::{self, NonNull};
use core::slice;

/// `pset_sys_bootstrap()` of kern/processor.c.
///
/// # Safety
///
/// `kern/sched_prim.c`'s `sched_init()` is the only caller; it runs during the
/// single-threaded boot before any other CPU starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_sys_bootstrap() {
    // SAFETY: the caller's contract.
    unsafe { crate::kern::processor::bootstrap() };
}

/// `processor_init()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must point at writable storage for a [`Processor`] that no other
/// thread can see yet, as `pset_sys_bootstrap()` guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_init(pr: *mut Processor, slot_num: c_int) {
    // SAFETY: the caller's contract.
    unsafe { Processor::init(pr, slot_num) };
}

/// `pset_init()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at writable storage for a full `struct processor_set`
/// that no other thread can see yet; `pset_sys_bootstrap()` and
/// `processor_set_create()` are the callers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_init(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ProcessorSet::init(pset) };
}

/// `pset_sys_init()` of kern/processor.c.
///
/// # Safety
///
/// `kern/startup.c` is the only caller; it runs this during boot after
/// `pset_sys_bootstrap()` and before any other CPU is started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_sys_init() {
    // SAFETY: the caller's contract.
    unsafe { crate::kern::processor::system_init() };
}

/// `processor_start()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_start(pr: *mut Processor) -> c_int {
    let Some(pr) = NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).start() } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_exit()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_exit(pr: *mut Processor) -> c_int {
    let Some(pr) = NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).exit() } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_control()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`, and `info` must be
/// readable for `count` integers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_control(
    pr: *mut Processor,
    info: *mut c_int,
    count: c_uint,
) -> c_int {
    let Some(pr) = NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    let info: &[c_int] = if count == 0 {
        &[]
    } else {
        // SAFETY: the caller promises `count` readable integers.
        unsafe { slice::from_raw_parts(info, count as usize) }
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).control(info) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_get_assignment()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`, and `pset` must be
/// a valid out-parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_get_assignment(
    pr: *mut Processor,
    pset: *mut *mut ProcessorSet,
) -> c_int {
    let Some(pr) = NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).get_assignment() } {
        Ok(assignment) => {
            // SAFETY: the caller passed the out-parameter the C signature
            // requires.
            unsafe { *pset = assignment };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `pset_reference()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_reference(pset: *mut ProcessorSet) {
    // SAFETY: the caller promises a live set.
    unsafe { (*pset).reference() };
}

/// `pset_deallocate()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set` that the
/// caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_deallocate(pset: *mut ProcessorSet) {
    let Some(pset) = NonNull::new(pset) else {
        return;
    };

    // SAFETY: the caller's contract.
    unsafe { (*pset.as_ptr()).deallocate() };
}

/// `pset_add_thread()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `thread` at a live
/// thread that is not linked into a set; the caller must hold both locks, as
/// the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_thread(
    pset: *mut ProcessorSet,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_thread(thread) };
}

/// `pset_remove_thread()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `thread` at a live
/// thread linked into it; the caller must hold both locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_thread(
    pset: *mut ProcessorSet,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_thread(thread) };
}

/// `pset_add_task()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `task` at a live task
/// that is not linked into a set; the caller must hold both locks, as the C
/// requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_task(
    pset: *mut ProcessorSet,
    task: *mut Task,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_task(task) };
}

/// `pset_remove_task()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `task` at a live task
/// linked into it; the caller must hold both locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_task(
    pset: *mut ProcessorSet,
    task: *mut Task,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_task(task) };
}

/// `quantum_set()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn quantum_set(pset: *mut ProcessorSet) {
    // SAFETY: the caller promises a live set.
    unsafe { (*pset).quantum_set() };
}

/// `pset_add_processor()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `processor` at a
/// live processor that is not linked into a set; the caller must hold both
/// locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_processor(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_processor(processor) };
}

/// `pset_remove_processor()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `processor` at a
/// live processor linked into it; the caller must hold both locks, as the C
/// requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_processor(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_processor(processor) };
}

/// `thread_change_psets()` of kern/processor.c.
///
/// # Safety
///
/// `thread` must point at a live thread linked into the live set `old_pset`,
/// and `new_pset` at a live set; the caller must hold the locks of both sets
/// and of the thread, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_change_psets(
    thread: *mut Thread,
    old_pset: *mut ProcessorSet,
    new_pset: *mut ProcessorSet,
) {
    // SAFETY: the caller's contract.
    unsafe { Thread::change_psets(thread, old_pset, new_pset) };
}

/// `processor_set_max_priority()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_max_priority(
    pset: *mut ProcessorSet,
    max_priority: c_int,
    change_threads: c_int,
) -> c_int {
    let Some(pset) = NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe {
        (*pset.as_ptr()).max_priority(max_priority, change_threads)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_policy_enable()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_policy_enable(
    pset: *mut ProcessorSet,
    policy: c_int,
) -> c_int {
    let Some(pset) = NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).policy_enable(policy) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_policy_disable()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_policy_disable(
    pset: *mut ProcessorSet,
    policy: c_int,
    change_threads: c_int,
) -> c_int {
    let Some(pset) = NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).policy_disable(policy, change_threads) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_create()` of kern/processor.c.
///
/// # Safety
///
/// `host` must be null or the live host privilege object; `new_set` and
/// `new_name` must be valid out-parameters, and the caller owns one reference
/// through each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_create(
    host: *mut c_void,
    new_set: *mut *mut ProcessorSet,
    new_name: *mut *mut ProcessorSet,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { crate::kern::processor::create(host) } {
        Ok(pset) => {
            // SAFETY: the caller passed the two out-parameters the C signature
            // requires; `create()` took one reference for each.
            unsafe {
                *new_set = pset;
                *new_name = pset;
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_destroy()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set` that the
/// caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_destroy(
    pset: *mut ProcessorSet,
) -> c_int {
    let Some(pset) = NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).destroy() } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_tasks()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`; `task_list`
/// must be a valid out-parameter for the port array, and `count` for its
/// length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_tasks(
    pset: *mut ProcessorSet,
    task_list: *mut *mut Task,
    count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { crate::kern::processor::tasks(pset) } {
        Ok((list, length)) => {
            // SAFETY: the caller passed the out-parameters the C signature
            // requires; the array holds the ports MIG sends.
            unsafe {
                *task_list = list.cast::<Task>();
                *count = length;
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_threads()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`;
/// `thread_list` must be a valid out-parameter for the port array, and
/// `count` for its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_threads(
    pset: *mut ProcessorSet,
    thread_list: *mut *mut Thread,
    count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { crate::kern::processor::threads(pset) } {
        Ok((list, length)) => {
            // SAFETY: the caller passed the out-parameters the C signature
            // requires; the array holds the ports MIG sends.
            unsafe {
                *thread_list = list.cast::<Thread>();
                *count = length;
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `processor_info()` of kern/processor.c.
///
/// # Safety
///
/// `processor` must be null or point at a live `struct processor`; `host` and
/// `count` must be valid out-parameters, and `info` must be writable for the
/// `processor_basic_info` that `*count` reports.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_info(
    processor: *mut Processor,
    flavor: c_int,
    host: *mut *mut c_void,
    info: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    let Some(processor) = NonNull::new(processor) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a valid `count` out-parameter.
    let capacity = unsafe { *count };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*processor.as_ptr()).info(flavor, capacity) } {
        Ok(basic) => {
            // SAFETY: the count check inside `info()` guarantees the caller's
            // buffer is at least a `processor_basic_info`, and the caller
            // promises the other two out-parameters.
            unsafe {
                ptr::write(info.cast::<ProcessorBasicInfo>(), basic);
                *count = PROCESSOR_BASIC_INFO_COUNT;
                *host = crate::kern::host::realhost().cast::<c_void>();
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_info()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`; `host` and
/// `count` must be valid out-parameters, and `info` must be writable for the
/// record the flavor and `*count` call for.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_info(
    pset: *mut ProcessorSet,
    flavor: c_int,
    host: *mut *mut c_void,
    info: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    let Some(pset) = NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a valid `count` out-parameter.
    let capacity = unsafe { *count };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).info(flavor, capacity) } {
        Ok(ProcessorSetInfo::Basic(basic)) => {
            // SAFETY: the count check inside `info()` guarantees the caller's
            // buffer is at least a `processor_set_basic_info`, and the caller
            // promises the other two out-parameters.
            unsafe {
                ptr::write(info.cast::<ProcessorSetBasicInfo>(), basic);
                *count = PROCESSOR_SET_BASIC_INFO_COUNT;
                *host = crate::kern::host::realhost().cast::<c_void>();
            }
            0
        }
        Ok(ProcessorSetInfo::Sched(sched)) => {
            // SAFETY: the flavor's count check guarantees the caller's buffer
            // is at least a `processor_set_sched_info`, and the caller
            // promises the other two out-parameters.
            unsafe {
                ptr::write(info.cast::<ProcessorSetSchedInfo>(), sched);
                *count = PROCESSOR_SET_SCHED_INFO_COUNT;
                *host = crate::kern::host::realhost().cast::<c_void>();
            }
            0
        }
        Err(KernError::InvalidArgument) => {
            // SAFETY: the caller promises a valid `host` out-parameter.
            unsafe {
                *host = ptr::null_mut();
            }
            c_int::from(KernError::InvalidArgument)
        }
        Err(error) => c_int::from(error),
    }
}
