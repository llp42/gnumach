// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/machine.h:
//   Copyright (C) 2008 Free Software Foundation, Inc.
// Derived from kern/machine.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from include/mach/machine.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct machine_slot` of `include/mach/machine.h`, and the machine
//! abstraction of `kern/machine.c`.

use crate::arch::i386::ast_check::init_ast_check;
use crate::arch::i386::percpu::{current_thread, percpu_at};
use crate::arch::i386::pmap;
use crate::config::NCPUS;
use crate::glue;
use crate::kern::debug;
use crate::kern::lock::SimpleLock;
use crate::kern::processor::{
    PROCESSOR_ASSIGN, PROCESSOR_DISPATCHING, PROCESSOR_IDLE,
    PROCESSOR_OFF_LINE, PROCESSOR_RUNNING, PROCESSOR_SHUTDOWN, Processor,
    ProcessorSet, default_pset, master_processor, slave_pset,
};
use crate::kern::queue::{
    QueueEntry, queue_empty, queue_end, queue_enter_tail, queue_first,
    queue_next, queue_remove_generic,
};
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_bind, thread_block,
    thread_wakeup_prim,
};
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr::{self, NonNull};

/// `CPU_STATE_MAX` in <mach/machine.h>: the per-state tick counters every
/// machine slot carries.
pub const CPU_STATE_MAX: usize = 3;

/// `struct machine_slot` of <mach/machine.h>: what the arch probe records
/// about each possible CPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSlot {
    pub is_cpu: c_int,
    pub cpu_type: c_int,
    pub cpu_subtype: c_int,
    pub running: c_int,
    /// `cpu_ticks`: the ticks accumulated per `CPU_STATE_*`.
    pub cpu_ticks: [c_int; CPU_STATE_MAX],
    /// `clock_freq`: the clock interrupt frequency.
    pub clock_freq: c_int,
}

impl MachineSlot {
    /// The all-zero image a C `static struct machine_slot` began with.
    const fn zeroed() -> Self {
        Self {
            is_cpu: 0,
            cpu_type: 0,
            cpu_subtype: 0,
            running: 0,
            cpu_ticks: [0; CPU_STATE_MAX],
            clock_freq: 0,
        }
    }
}

const _: () = assert!(size_of::<MachineSlot>() == 32);
const _: () = assert!(align_of::<MachineSlot>() == align_of::<c_int>());
const _: () = assert!(offset_of!(MachineSlot, is_cpu) == 0);
const _: () = assert!(offset_of!(MachineSlot, cpu_type) == 4);
const _: () = assert!(offset_of!(MachineSlot, cpu_subtype) == 8);
const _: () = assert!(offset_of!(MachineSlot, running) == 12);
const _: () = assert!(offset_of!(MachineSlot, cpu_ticks) == 16);
const _: () = assert!(offset_of!(MachineSlot, clock_freq) == 28);

/// `struct machine_info` of <mach/machine.h>: what `kern/startup.c` records
/// about the machine as a whole.
#[repr(C)]
pub struct MachineInfo {
    pub major_version: c_int,
    pub minor_version: c_int,
    pub max_cpus: c_int,
    pub avail_cpus: c_int,
    /// `memory_size`: a `vm_size_t`, four bytes on i386 and eight on x86_64.
    pub memory_size: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<MachineInfo>() == 24);
    assert!(align_of::<MachineInfo>() == 8);
    assert!(offset_of!(MachineInfo, major_version) == 0);
    assert!(offset_of!(MachineInfo, minor_version) == 4);
    assert!(offset_of!(MachineInfo, max_cpus) == 8);
    assert!(offset_of!(MachineInfo, avail_cpus) == 12);
    assert!(offset_of!(MachineInfo, memory_size) == 16);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<MachineInfo>() == 20);
    assert!(align_of::<MachineInfo>() == 4);
    assert!(offset_of!(MachineInfo, major_version) == 0);
    assert!(offset_of!(MachineInfo, minor_version) == 4);
    assert!(offset_of!(MachineInfo, max_cpus) == 8);
    assert!(offset_of!(MachineInfo, avail_cpus) == 12);
    assert!(offset_of!(MachineInfo, memory_size) == 16);
};

/// `machine_info` of kern/machine.c.
#[unsafe(export_name = "machine_info")]
static mut MACHINE_INFO: MachineInfo = MachineInfo {
    major_version: 0,
    minor_version: 0,
    max_cpus: 0,
    avail_cpus: 0,
    memory_size: 0,
};

/// `machine_slot[NCPUS]` of <mach/machine.h>.
#[unsafe(export_name = "machine_slot")]
static mut MACHINE_SLOT: [MachineSlot; NCPUS] =
    [const { MachineSlot::zeroed() }; NCPUS];

/// `action_queue` of kern/machine.c: the assign/shutdown queue.
#[unsafe(export_name = "action_queue")]
static mut ACTION_QUEUE: QueueEntry = QueueEntry::unlinked();

/// `action_lock` of kern/machine.c.
#[unsafe(export_name = "action_lock")]
static mut ACTION_LOCK: SimpleLock = SimpleLock::new();

/// The C `machine_slot[cpu]` of <mach/machine.h>.
///
/// # Safety
///
/// `cpu` must be below the configured `NCPUS`, the C array's length.
pub(crate) unsafe fn slot(cpu: usize) -> *mut MachineSlot {
    // SAFETY: the caller promises `cpu < NCPUS`; `.cast()` keeps the element
    // pointer and `.add()` stays inside the array.
    unsafe {
        ptr::addr_of_mut!(MACHINE_SLOT)
            .cast::<MachineSlot>()
            .add(cpu)
    }
}

/// The live `machine_info`.
pub(crate) fn info() -> *mut MachineInfo {
    ptr::addr_of_mut!(MACHINE_INFO)
}

/// The live `action_queue`.
pub(crate) fn action_queue() -> *mut QueueEntry {
    ptr::addr_of_mut!(ACTION_QUEUE)
}

/// The live `action_lock`.
pub(crate) fn action_lock() -> *mut SimpleLock {
    ptr::addr_of_mut!(ACTION_LOCK)
}

/// The `RB_*` flag word of <sys/reboot.h>, the `host_reboot()` options.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RebootOptions(c_int);

impl RebootOptions {
    /// `RB_DEBUGGER`: enter the kernel debugger from user level instead of
    /// rebooting.
    const DEBUGGER: Self = Self(0x1000);
    /// `RB_HALT`: do not reboot, just halt.
    const HALT: Self = Self(0x08);

    /// Whether every bit of `other` is set in `self`.
    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Reboot or halt the host, or enter the debugger.
fn reboot(
    host: Option<NonNull<c_void>>,
    options: RebootOptions,
) -> Result<(), KernError> {
    if host.is_none() {
        return Err(KernError::InvalidHost);
    }

    if options.contains(RebootOptions::DEBUGGER) {
        // SAFETY: `Debugger` takes a readable NUL-terminated message; this one
        // is a literal, and the call never returns.
        unsafe { debug::Debugger(c"Debugger".as_ptr()) };
    } else {
        // `halt_all_cpus` never returns.
        let reboot = c_int::from(!options.contains(RebootOptions::HALT));
        crate::arch::i386::model_dep::halt_all_cpus(reboot);
    }

    Ok(())
}

/// `cpu_up()` of kern/machine.c: flag `cpu` as up and running.  Called when a
/// processor comes online.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reports, and the boot or the hot-plug path
/// must call this with nothing locked on the processor.
pub(crate) unsafe fn cpu_up(cpu: c_int) {
    // SAFETY: `cpu` is a live CPU number, so its block exists.
    let processor = unsafe { ptr::addr_of_mut!((*percpu_at(cpu)).processor) };
    let default = default_pset();
    let slave = slave_pset();

    // SAFETY: the caller promises the processor is unshared; the boot's
    // `pset_sys_init()` built the two sets and the CPU record.
    unsafe {
        (*default).lock.lock();
        (*slave).lock.lock();

        let s = glue::splsched();
        (*processor).lock.lock();
        init_ast_check(processor);

        // `cpu` is a live CPU number below `NCPUS`.
        (*slot(cpu as usize)).running = 1;
        let info = info();
        (*info).avail_cpus = (*info).avail_cpus.wrapping_add(1);

        if cpu != 0 {
            (*slave).add_processor(processor);
        } else {
            (*default).add_processor(processor);
        }
        (*processor).state = PROCESSOR_RUNNING;

        (*processor).lock.unlock();
        glue::splx(s);
        (*slave).lock.unlock();
        (*default).lock.unlock();
    }
}

/// `cpu_down()` of kern/machine.c: flag `cpu` as down.  Called when a
/// processor is about to go offline.
fn cpu_down(cpu: c_int) {
    // SAFETY: `cpu` is a live CPU number, so its block exists; the processor
    // has already been removed from its set.
    unsafe {
        let s = glue::splsched();
        let processor = ptr::addr_of_mut!((*percpu_at(cpu)).processor);
        (*processor).lock.lock();

        // `cpu` is a live CPU number below `NCPUS`.
        (*slot(cpu as usize)).running = 0;
        let info = info();
        (*info).avail_cpus = (*info).avail_cpus.wrapping_sub(1);
        (*processor).processor_set_next = ptr::null_mut();
        (*processor).state = PROCESSOR_OFF_LINE;

        (*processor).lock.unlock();
        glue::splx(s);
    }
}

/// `processor_request_action()` of kern/machine.c: queue `processor` for
/// assignment to `new_pset`, or for shutdown when it is null.
///
/// # Safety
///
/// `processor` must be a live processor in a live set with its lock held by
/// the caller, as `processor_assign()` and `processor_shutdown()` arrange.
unsafe fn request_action(
    processor: *mut Processor,
    new_pset: *mut ProcessorSet,
) {
    // SAFETY: the caller promises the processor is in a live set; the idle
    // lock is the C lock for the state below.
    unsafe {
        let pset = (*processor).processor_set;
        (*pset).idle_lock.lock();

        loop {
            // Another CPU's action thread clears the state under this lock;
            // the volatile read is the C's.
            let state = ptr::read_volatile(ptr::addr_of!((*processor).state));
            if state != PROCESSOR_DISPATCHING {
                break;
            }
            core::hint::spin_loop();
        }

        (*action_lock()).lock();

        match (*processor).state {
            PROCESSOR_IDLE => {
                queue_remove_generic(
                    &raw mut (*pset).idle_queue,
                    processor.cast::<c_void>(),
                    offset_of!(Processor, processor_queue),
                );
                (*pset).idle_count = (*pset).idle_count.wrapping_sub(1);
                queue_enter_tail(
                    action_queue(),
                    processor.cast::<c_void>(),
                    offset_of!(Processor, processor_queue),
                );
                set_action_state(processor, new_pset);
            }
            PROCESSOR_RUNNING => {
                queue_enter_tail(
                    action_queue(),
                    processor.cast::<c_void>(),
                    offset_of!(Processor, processor_queue),
                );
                set_action_state(processor, new_pset);
            }
            PROCESSOR_ASSIGN => {
                set_action_state(processor, new_pset);
            }
            state => {
                glue::printf(c"state: %d\n".as_ptr(), state);
                // SAFETY: `Panic` does not return; the message and the
                // function tag are the C `panic()` call's.
                glue::Panic(
                    c"kern/machine.c".as_ptr(),
                    line!() as c_int,
                    c"processor_request_action".as_ptr(),
                    c"processor_request_action: bad state".as_ptr(),
                );
            }
        }

        (*action_lock()).unlock();
        (*pset).idle_lock.unlock();

        let _ = thread_wakeup_prim(
            action_queue().cast::<c_void>(),
            0,
            THREAD_AWAKENED,
        );
    }
}

/// The state store the three action cases share.
///
/// # Safety
///
/// `processor` must be a live processor whose lock the caller holds.
unsafe fn set_action_state(
    processor: *mut Processor,
    new_pset: *mut ProcessorSet,
) {
    // SAFETY: the caller holds the processor lock and the action lock.
    unsafe {
        if new_pset.is_null() {
            (*processor).state = PROCESSOR_SHUTDOWN;
        } else {
            (*processor).state = PROCESSOR_ASSIGN;
            (*processor).processor_set_next = new_pset;
        }
    }
}

/// `processor_assign()` of kern/machine.c: change the set `processor` is
/// assigned to.
///
/// # Safety
///
/// `processor` must be null or a live processor, `new_pset` null or a live
/// set, and the caller must hold no lock: the routine waits and may block.
pub(crate) unsafe fn assign(
    processor: *mut Processor,
    new_pset: *mut ProcessorSet,
    wait: bool,
) -> Result<(), KernError> {
    if processor.is_null()
        || new_pset.is_null()
        || processor == master_processor()
    {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: `new_pset` is live; the reference is the one the action takes.
    unsafe { (*new_pset).reference() };

    loop {
        // SAFETY: the caller promises a live processor; the C takes the
        // processor lock at splsched.
        unsafe {
            let mut s = glue::splsched();
            (*processor).lock.lock();

            let state = (*processor).state;
            if state == PROCESSOR_OFF_LINE || state == PROCESSOR_SHUTDOWN {
                (*processor).lock.unlock();
                glue::splx(s);
                (*new_pset).deallocate();
                return Err(KernError::Failure);
            }

            if state == PROCESSOR_ASSIGN {
                assert_wait(processor.cast::<c_void>(), 1);
                (*processor).lock.unlock();
                glue::splx(s);
                thread_block(None);
                continue;
            }

            if (*processor).processor_set == new_pset {
                (*processor).lock.unlock();
                glue::splx(s);
                (*new_pset).deallocate();
                return Ok(());
            }

            request_action(processor, new_pset);

            if wait {
                loop {
                    let state = (*processor).state;
                    if state != PROCESSOR_ASSIGN && state != PROCESSOR_SHUTDOWN
                    {
                        break;
                    }
                    assert_wait(processor.cast::<c_void>(), 1);
                    (*processor).lock.unlock();
                    glue::splx(s);
                    thread_block(None);
                    s = glue::splsched();
                    (*processor).lock.lock();
                }
            }

            (*processor).lock.unlock();
            glue::splx(s);
            return Ok(());
        }
    }
}

/// `processor_shutdown()` of kern/machine.c: queue `processor` for shutdown.
///
/// # Safety
///
/// `processor` must be null or a live processor; the routine takes the
/// processor lock itself and may be called from interrupt level.
pub(crate) unsafe fn shutdown(
    processor: *mut Processor,
) -> Result<(), KernError> {
    if processor.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live processor; the C takes the processor
    // lock at splsched and leaves the shutdown queueing to the action thread.
    unsafe {
        let s = glue::splsched();
        (*processor).lock.lock();

        let state = (*processor).state;
        if state == PROCESSOR_OFF_LINE || state == PROCESSOR_SHUTDOWN {
            (*processor).lock.unlock();
            glue::splx(s);
            return Ok(());
        }

        request_action(processor, ptr::null_mut());
        (*processor).lock.unlock();
        glue::splx(s);

        Ok(())
    }
}

/// `processor_doaction()` of kern/machine.c: perform the shutdown or the
/// reassignment the action queue recorded.
///
/// # Safety
///
/// `processor` must be a live processor queued on `action_queue` with a state
/// of assign or shutdown, and the caller must be the action thread.
unsafe fn doaction(processor: *mut Processor) {
    let this_thread = current_thread();
    // SAFETY: the action thread is the current thread and the processor is
    // live; the bind only stores the pairing.
    unsafe { thread_bind(this_thread, processor) };
    // SAFETY: the action thread has no wait state set, so the block returns
    // immediately when this thread is the one selected to run.
    unsafe { thread_block(None) };

    let pset = unsafe { (*processor).processor_set };
    let mut prev_thread: *mut Thread = ptr::null_mut();
    let mut have_pset_ref = false;

    // SAFETY: `pset` is the set the processor belongs to, and this is the
    // action thread, so the set lock serializes the thread list.
    unsafe {
        (*pset).lock.lock();
        if (*pset).processor_count == 1 {
            let mut thread =
                queue_first(&raw mut (*pset).threads).cast::<Thread>();
            while queue_end(&raw mut (*pset).threads, thread.cast()) == 0 {
                Thread::hold(thread);
                thread = queue_next(&raw mut (*thread).pset_threads)
                    .cast::<Thread>();
            }
            (*pset).empty = 1;
            (*pset).ref_count = (*pset).ref_count.wrapping_add(1);
            have_pset_ref = true;

            'restart_thread: loop {
                prev_thread = ptr::null_mut();
                let mut thread =
                    queue_first(&raw mut (*pset).threads).cast::<Thread>();
                while queue_end(&raw mut (*pset).threads, thread.cast()) == 0 {
                    Thread::reference(thread);
                    (*pset).lock.unlock();
                    if !prev_thread.is_null() {
                        Thread::deallocate(prev_thread);
                    }

                    Thread::freeze(thread);
                    if (*thread).processor_set != pset {
                        Thread::unfreeze(thread);
                        Thread::deallocate(thread);
                        (*pset).lock.lock();
                        continue 'restart_thread;
                    }

                    let _ = Thread::dowait(thread, true);
                    prev_thread = thread;
                    (*pset).lock.lock();
                    Thread::unfreeze(prev_thread);
                    thread = queue_next(&raw mut (*thread).pset_threads)
                        .cast::<Thread>();
                }
                break;
            }
        }
        (*pset).lock.unlock();
    }

    let mut new_pset = unsafe { (*processor).processor_set_next };

    if !new_pset.is_null() {
        'restart_pset: loop {
            // SAFETY: both sets are live, and the C locks them in address
            // order to avoid deadlock.
            unsafe {
                if (pset as usize) < (new_pset as usize) {
                    (*pset).lock.lock();
                    (*new_pset).lock.lock();
                } else {
                    (*new_pset).lock.lock();
                    (*pset).lock.lock();
                }

                if (*new_pset).active == 0 {
                    (*new_pset).lock.unlock();
                    (*pset).lock.unlock();
                    (*new_pset).deallocate();
                    new_pset = default_pset();
                    (*new_pset).reference();
                    continue 'restart_pset;
                }

                if (*new_pset).empty != 0 && (*new_pset).processor_count > 0 {
                    (*new_pset).lock.unlock();
                    (*pset).lock.unlock();
                    loop {
                        // Another action thread clears the race under the set
                        // lock; the volatile reads are the C's.
                        let empty = ptr::read_volatile(ptr::addr_of!(
                            (*new_pset).empty
                        ));
                        let count = ptr::read_volatile(ptr::addr_of!(
                            (*new_pset).processor_count
                        ));
                        if empty == 0 || count == 0 {
                            break;
                        }
                        core::hint::spin_loop();
                    }
                    continue 'restart_pset;
                }

                let s = glue::splsched();
                (*processor).lock.lock();

                if (*processor).state == PROCESSOR_SHUTDOWN {
                    (*processor).processor_set_next = ptr::null_mut();
                    (*new_pset).lock.unlock();
                    shutdown_tail(
                        pset,
                        processor,
                        new_pset,
                        this_thread,
                        have_pset_ref,
                        prev_thread,
                        s,
                    );
                    return;
                }

                (*pset).remove_processor(processor);
                (*pset).lock.unlock();
                (*new_pset).add_processor(processor);
                if (*new_pset).empty != 0 {
                    let mut thread = queue_first(&raw mut (*new_pset).threads)
                        .cast::<Thread>();
                    while queue_end(
                        &raw mut (*new_pset).threads,
                        thread.cast(),
                    ) == 0
                    {
                        Thread::release(thread);
                        thread = queue_next(&raw mut (*thread).pset_threads)
                            .cast::<Thread>();
                    }
                    (*new_pset).empty = 0;
                }
                (*processor).processor_set_next = ptr::null_mut();
                (*processor).state = PROCESSOR_RUNNING;
                let _ = thread_wakeup_prim(
                    processor.cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
                (*processor).lock.unlock();
                glue::splx(s);
                (*new_pset).lock.unlock();

                (*new_pset).deallocate();
                if have_pset_ref {
                    (*pset).deallocate();
                }
                if !prev_thread.is_null() {
                    Thread::deallocate(prev_thread);
                }
                thread_bind(this_thread, ptr::null_mut());
                thread_block(None);
                return;
            }
        }
    }

    // SAFETY: the fall-through shutdown takes the processor lock at
    // splsched, as the C did after its `shutdown:` label.
    unsafe {
        if (*processor).state != PROCESSOR_SHUTDOWN {
            glue::printf(c"state: %d\n".as_ptr(), (*processor).state);
            // SAFETY: `Panic` does not return; the message and the function
            // tag are the C `panic()` call's.
            glue::Panic(
                c"kern/machine.c".as_ptr(),
                line!() as c_int,
                c"processor_doaction".as_ptr(),
                c"action_thread -- bad processor state".as_ptr(),
            );
        }

        let s = glue::splsched();
        (*processor).lock.lock();
        shutdown_tail(
            pset,
            processor,
            new_pset,
            this_thread,
            have_pset_ref,
            prev_thread,
            s,
        );
    }
}

/// The tail the assignment and shutdown paths share: drop the processor from
/// its set, release every reference, and leave through the shutdown context.
///
/// # Safety
///
/// `processor` must be live, its lock held and its set's lock held at
/// splsched `s`, and `pset` must be the set it is leaving.
unsafe fn shutdown_tail(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
    new_pset: *mut ProcessorSet,
    this_thread: *mut Thread,
    have_pset_ref: bool,
    prev_thread: *mut Thread,
    s: c_int,
) {
    // SAFETY: the caller's contract; the cleanup releases the references the
    // C released and hands the thread to the shutdown context.
    unsafe {
        (*pset).remove_processor(processor);
        (*processor).lock.unlock();
        (*pset).lock.unlock();
        glue::splx(s);

        if !new_pset.is_null() {
            (*new_pset).deallocate();
        }
        if have_pset_ref {
            (*pset).deallocate();
        }
        if !prev_thread.is_null() {
            Thread::deallocate(prev_thread);
        }

        thread_bind(this_thread, ptr::null_mut());
        glue::switch_to_shutdown_context(
            this_thread,
            Some(processor_doshutdown),
            processor,
        );
    }
}

/// `action_thread_continue()` of kern/machine.c: drain the action queue,
/// shutting processors down or reassigning them.
///
/// # Safety
///
/// The thread that runs this must be the action thread, and nothing else may
/// drain `action_queue`.
pub(crate) unsafe extern "C" fn action_thread_continue() -> ! {
    // SAFETY: `engine()` never returns.
    unsafe { engine() }
}

/// The action loop itself, separate so the continuation pointer can be a
/// plain `extern "C" fn()`.
///
/// # Safety
///
/// As [`action_thread_continue()`].
unsafe fn engine() -> ! {
    loop {
        // SAFETY: the action thread is the only drainer of the queue, and the
        // action lock serializes the walk.
        unsafe {
            let mut s = glue::splsched();
            (*action_lock()).lock();

            while queue_empty(action_queue()) == 0 {
                let processor =
                    queue_first(action_queue()).cast::<Processor>();
                queue_remove_generic(
                    action_queue(),
                    processor.cast::<c_void>(),
                    offset_of!(Processor, processor_queue),
                );
                (*action_lock()).unlock();
                glue::splx(s);

                doaction(processor);

                s = glue::splsched();
                (*action_lock()).lock();
            }

            assert_wait(action_queue().cast::<c_void>(), 0);
            (*action_lock()).unlock();
            glue::splx(s);

            // The continuation must be typed `extern "C" fn()`; the
            // trampoline re-enters the loop above on the resumed stack.
            unsafe extern "C" fn resume() {
                // SAFETY: `engine()` never returns.
                unsafe { engine() }
            }
            thread_block(Some(resume));
        }
    }
}

/// `processor_doshutdown()` of kern/machine.c: take `processor` out of the
/// system, running on its shutdown stack.
///
/// # Safety
///
/// `processor` must be the live processor whose shutdown context invoked
/// this, and the call must come from `switch_to_shutdown_context()`.
pub(crate) unsafe extern "C" fn processor_doshutdown(
    processor: *mut Processor,
) {
    // SAFETY: the processor owns the CPU it runs on, and `halt_cpu()` never
    // returns, as the C required.
    unsafe {
        let cpu = (*processor).slot_num;

        // `timer_switch()` is the empty macro of <kern/timer.h>, so the C
        // statement compiled to nothing.

        pmap::deactivate_kernel(cpu);
        (*percpu_at(cpu)).active_thread = ptr::null_mut();
        cpu_down(cpu);
        let _ =
            thread_wakeup_prim(processor.cast::<c_void>(), 0, THREAD_AWAKENED);
        glue::halt_cpu();
    }
}

/// `host_reboot()` of kern/machine.c, the routine <mach/mach_host.defs>
/// declares.
pub(crate) unsafe fn host_reboot(
    host_priv: *mut c_void,
    options: c_int,
) -> Result<(), KernError> {
    reboot(NonNull::new(host_priv), RebootOptions(options))
}
