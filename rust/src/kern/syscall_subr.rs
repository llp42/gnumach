// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_subr.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The console-print trap and the scheduling entries of `kern/syscall_subr.c`,
//! declared in <kern/syscall_subr.h>.

use crate::arch::i386::percpu::{current_processor, current_thread};
use crate::glue;
use crate::ipc::{IpcPort, IpcSpace, ipc_object};
use crate::kern::ipc_kobject::IKOT_THREAD;
use crate::kern::ipc_sched::{
    ipc_timeout_to_ticks, thread_will_wait_with_timeout,
};
use crate::kern::mach_clock::{self, reset_timeout_check};
use crate::kern::policy::POLICY_FIXEDPRI;
use crate::kern::processor::Processor;
use crate::kern::sched::{NRQS, RUN_QUEUE_NULL};
use crate::kern::sched_prim::{
    compute_priority, min_quantum, rem_runq, thread_block, thread_run,
};
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_char, c_int, c_uint, c_void};

/// `SWITCH_OPTION_NONE` of <mach/thread_switch.h>.
const SWITCH_OPTION_NONE: c_int = 0;
/// `SWITCH_OPTION_DEPRESS` of <mach/thread_switch.h>.
const SWITCH_OPTION_DEPRESS: c_int = 1;
/// `SWITCH_OPTION_WAIT` of <mach/thread_switch.h>.
const SWITCH_OPTION_WAIT: c_int = 2;

/// `MACH_PORT_RIGHT_SEND` of <mach/port.h>: the right
/// `ipc_port_translate_send()` looks up.
const MACH_PORT_RIGHT_SEND: c_uint = 0;

/// `mach_print()` in C: write a kernel string to the console.
///
/// # Safety
///
/// `s` must point at a NUL-terminated string that stays readable for the
/// duration of the call.
pub(crate) unsafe fn print(s: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `s`, and the
    // format string is the C call's literal, which matches it.
    unsafe { glue::printf(c"%s".as_ptr(), s) };
}

/// `thread_depress_priority()` in C.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the thread
/// lock itself.
pub(crate) unsafe fn depress_priority(
    thread: *mut Thread,
    depress_time: c_uint,
) {
    let ticks = ipc_timeout_to_ticks(depress_time);
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the priority
    // fields and the timer element.
    unsafe {
        (*thread).lock.lock();

        reset_timeout_check(&raw mut (*thread).depress_timer);

        (*thread).depress_priority = (*thread).priority;
        (*thread).priority = NRQS as c_int - 1;
        (*thread).sched_pri = NRQS as c_int - 1;
        if ticks != 0 {
            mach_clock::set_timeout(&raw mut (*thread).depress_timer, ticks);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_depress_timeout()` in C, in the shape the timer callback calls.
///
/// # Safety
///
/// `param` must be the live thread the timer was armed for.
pub(crate) unsafe extern "C" fn depress_timeout(param: *mut c_void) {
    let thread = param.cast::<Thread>();
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the priority
    // fields.
    unsafe {
        (*thread).lock.lock();

        if (*thread).depress_priority >= 0 {
            (*thread).priority = (*thread).depress_priority;
            (*thread).depress_priority = -1;
            compute_priority(thread, 0);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_depress_abort()` in C.
///
/// # Safety
///
/// `thread` must be null or a live thread; the routine takes splsched and the
/// thread lock itself.
pub(crate) unsafe fn depress_abort(thread: *mut Thread) -> c_int {
    if thread.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    let s = unsafe { glue::splsched() };
    // SAFETY: the null check above and the caller's contract; the thread lock
    // protects the priority fields and the timer element.
    unsafe {
        (*thread).lock.lock();

        if (*thread).depress_priority >= 0 {
            reset_timeout_check(&raw mut (*thread).depress_timer);
            (*thread).priority = (*thread).depress_priority;
            (*thread).depress_priority = -1;
            compute_priority(thread, 0);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }

    0
}

/// Whether `processor` or its set has a runnable thread, the predicate the
/// `swtch*` routines share.
///
/// # Safety
///
/// `processor` must be the running CPU's live processor.
unsafe fn runnable(processor: *mut Processor) -> bool {
    // SAFETY: the caller promises the live processor and its set.
    unsafe {
        (*processor).runq.count > 0
            || (*(*processor).processor_set).runq.count > 0
    }
}

/// `swtch_continue()` of kern/syscall_subr.c.
unsafe extern "C" fn swtch_continue() {
    let myprocessor = current_processor();
    // SAFETY: the continuation runs on the CPU whose processor it reads.
    let runnable = unsafe { runnable(myprocessor) };
    // SAFETY: the machine's syscall return never comes back, and the C
    // passed the boolean as an `int`.
    unsafe { glue::thread_syscall_return(c_int::from(runnable)) };
}

/// `swtch()` in C.
///
/// # Safety
///
/// Must run on the current thread with no lock held and no wait state set.
pub(crate) unsafe fn swtch() -> c_int {
    let myprocessor = current_processor();
    // SAFETY: the running CPU's processor is live.
    if !unsafe { runnable(myprocessor) } {
        return 0;
    }

    // SAFETY: the caller's contract.
    unsafe { thread_block(Some(swtch_continue)) };

    let myprocessor = current_processor();
    // SAFETY: as above, after the context switch.
    c_int::from(unsafe { runnable(myprocessor) })
}

/// `swtch_pri_continue()` of kern/syscall_subr.c.
unsafe extern "C" fn swtch_pri_continue() {
    let thread = current_thread();
    // SAFETY: the continuation runs on its own thread, whose lock it takes.
    unsafe {
        if (*thread).depress_priority >= 0 {
            let _ = depress_abort(thread);
        }
    }

    let myprocessor = current_processor();
    // SAFETY: the continuation runs on the CPU whose processor it reads.
    let runnable = unsafe { runnable(myprocessor) };
    // SAFETY: the machine's syscall return never comes back.
    unsafe { glue::thread_syscall_return(c_int::from(runnable)) };
}

/// `swtch_pri()` in C.  The C ignores its priority argument.
///
/// # Safety
///
/// Must run on the current thread with no lock held and no wait state set.
pub(crate) unsafe fn swtch_pri() -> c_int {
    let thread = current_thread();
    let myprocessor = current_processor();
    // SAFETY: the running CPU's processor is live.
    if !unsafe { runnable(myprocessor) } {
        return 0;
    }

    // The C converted the non-negative `min_quantum` to the depression time.
    let quantum = min_quantum() as c_uint;
    // SAFETY: the caller's contract; both routines lock the thread as their C
    // versions did.
    unsafe {
        depress_priority(thread, quantum);
        thread_block(Some(swtch_pri_continue));
        if (*thread).depress_priority >= 0 {
            let _ = depress_abort(thread);
        }
    }

    let myprocessor = current_processor();
    // SAFETY: as above, after the context switch.
    c_int::from(unsafe { runnable(myprocessor) })
}

/// `thread_switch_continue()` of kern/syscall_subr.c.
unsafe extern "C" fn thread_switch_continue() {
    let cur_thread = current_thread();
    // SAFETY: the continuation runs on its own thread, whose lock it takes.
    unsafe {
        if (*cur_thread).depress_priority >= 0 {
            let _ = depress_abort(cur_thread);
        }
    }
    // SAFETY: the machine's syscall return never comes back; the C returned
    // `KERN_SUCCESS`.
    unsafe { glue::thread_syscall_return(0) };
}

/// `thread_switch()` in C.
///
/// # Safety
///
/// Must run on the current thread with no lock held and no wait state set.
pub(crate) unsafe fn thread_switch(
    thread_name: c_uint,
    option: c_int,
    option_time: c_uint,
) -> c_int {
    let cur_thread = current_thread();

    match option {
        SWITCH_OPTION_NONE => (),
        SWITCH_OPTION_DEPRESS => {
            // SAFETY: the caller's contract.
            unsafe { depress_priority(cur_thread, option_time) }
        }
        SWITCH_OPTION_WAIT => {
            // SAFETY: the caller's contract.
            unsafe { thread_will_wait_with_timeout(cur_thread, option_time) }
        }
        _ => return c_int::from(KernError::InvalidArgument),
    }

    if let Some(thread) = unsafe { hint_thread(cur_thread, thread_name) } {
        // SAFETY: `hint_thread` removed `thread` from its run queue under the
        // thread lock, which it left held.
        unsafe {
            if (*thread).policy == POLICY_FIXEDPRI {
                let myprocessor = current_processor();
                (*myprocessor).quantum = (*thread).sched_data;
                (*myprocessor).first_quantum = 1;
            }
            thread_run(Some(thread_switch_continue), thread);

            if (*cur_thread).depress_priority >= 0 {
                let _ = depress_abort(cur_thread);
            }
        }
        return 0;
    }

    let myprocessor = current_processor();
    // SAFETY: the running CPU's processor is live.
    if unsafe { runnable(myprocessor) } {
        // SAFETY: the caller's contract.
        unsafe { thread_block(Some(thread_switch_continue)) };
    }

    // SAFETY: the thread is the current one, whose lock the routine takes.
    unsafe {
        if (*cur_thread).depress_priority >= 0 {
            let _ = depress_abort(cur_thread);
        }
    }

    0
}

/// The thread-hint arm of `thread_switch()`: translate `thread_name`, check
/// that it is an eligible thread of the current set still on its run queue,
/// and pull it off.
///
/// Returns the thread with its lock held and interrupts at `splsched` when
/// the hint is usable; the port, thread and run-queue lock are released
/// otherwise.
///
/// # Safety
///
/// `cur_thread` must be the current thread and nothing may be locked.
unsafe fn hint_thread(
    cur_thread: *mut Thread,
    thread_name: c_uint,
) -> Option<*mut Thread> {
    if thread_name == 0 {
        return None;
    }

    // SAFETY: the current thread's task and space are live.
    let space = unsafe { IpcSpace::from_raw((*(*cur_thread).task).itk_space) };
    // SAFETY: the space is live and nothing is locked; the C looked the name
    // up as a send right.
    let object = unsafe {
        ipc_object::translate(space, thread_name, MACH_PORT_RIGHT_SEND)
    }
    .ok()?;
    // SAFETY: a successful translate returns a live, locked object.
    let port = unsafe { IpcPort::from_raw(object) };

    // SAFETY: the port is live and locked.
    let eligible = unsafe { port.is_active() && port.kotype() == IKOT_THREAD };
    if !eligible {
        // SAFETY: the port is live and locked.
        unsafe { port.unlock() };
        return None;
    }

    // SAFETY: an active port of type `IKOT_THREAD` names a live thread.
    let thread = unsafe { port.kobject().cast::<Thread>() };
    // SAFETY: interrupts are blocked while the thread lock is taken.
    let s = unsafe { glue::splsched() };
    // SAFETY: `thread` is live and the thread lock protects its run-queue
    // link and processor set.
    unsafe {
        (*thread).lock.lock();
        let got_it = (*thread).processor_set == (*cur_thread).processor_set
            && rem_runq(thread) != RUN_QUEUE_NULL;
        if got_it {
            // SAFETY: the C released the lock and the interrupt level before
            // switching to the thread, and the port after.
            (*thread).lock.unlock();
            glue::splx(s);
            port.unlock();
            return Some(thread);
        }
        (*thread).lock.unlock();
        glue::splx(s);
    }

    // SAFETY: the port is live and locked.
    unsafe { port.unlock() };
    None
}
