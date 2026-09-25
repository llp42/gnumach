// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ast.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/ast.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The per-CPU AST bits, which `kern/ast.h` declares as macros over
//! `need_ast[]` and `kern/ast.c` used to define.

use crate::arch::i386::percpu::{
    cpu_number, current_processor, current_thread, processor_ptr,
};
use crate::config::NCPUS;
use crate::glue;
use crate::kern::policy::POLICY_FIXEDPRI;
use crate::kern::processor::{
    PROCESSOR_ASSIGN, PROCESSOR_DISPATCHING, PROCESSOR_IDLE,
    PROCESSOR_OFF_LINE, PROCESSOR_RUNNING, PROCESSOR_SHUTDOWN,
};
use crate::kern::queue::{QueueEntry, queue_empty};
use crate::kern::sched::NRQS;
use crate::kern::sched_prim::thread_block;
use crate::kern::smp::smp_get_numcpus;
use crate::kern::thread::{TH_SUSP, Thread};
use crate::kern::thread_ffi::thread_halt_self;
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::c_int;

/// `AST_HALT` in <kern/ast.h>: the thread has been asked to halt at a clean
/// point.
pub const AST_HALT: c_int = 0x1;
/// `AST_TERMINATE` in <kern/ast.h>: the thread is terminating.
pub const AST_TERMINATE: c_int = 0x2;
/// `AST_BLOCK` in <kern/ast.h>: the scheduling AST reason.
pub const AST_BLOCK: usize = 0x4;
/// `AST_SCHEDULING` in <kern/ast.h>: the reasons the scheduler holds back
/// while the idle loop waits.
pub const AST_SCHEDULING: usize =
    (AST_HALT | AST_TERMINATE) as usize | AST_BLOCK;

/// `AST_NETWORK` in <kern/ast.h>: the reason the network code sets.
const AST_NETWORK: usize = 0x8;

/// `AST_PER_THREAD` in <kern/ast.h>: the reasons reset from the thread at a
/// context switch.
const AST_PER_THREAD: usize = (AST_HALT | AST_TERMINATE) as usize;

/// `need_ast[NCPUS]` of kern/ast.c: the reasons pending on each CPU.  The C
/// half reaches the same array through the <kern/ast.h> macros and
/// `i386/i386/locore.S` reads the symbol.
#[unsafe(export_name = "need_ast")]
static NEED_AST: SyncCell<[usize; NCPUS]> =
    SyncCell(UnsafeCell::new([0; NCPUS]));

/// The `ast_needed()` macro of <kern/ast.h>: the reasons pending on `cpu`.
pub fn ast_needed(cpu: c_int) -> usize {
    // SAFETY: `slot()` requires a live CPU number, as its caller must give it;
    // a read of the slot cannot invalidate anything.
    unsafe { slot(cpu).read_volatile() }
}

/// The `ast_on()` macro of <kern/ast.h>: set `reasons` on `cpu`.
pub fn ast_on(cpu: c_int, reasons: usize) {
    // SAFETY: as `ast_needed()`; the read-modify-write keeps the other reasons
    // of the same slot.
    unsafe {
        let cell = slot(cpu);
        cell.write_volatile(cell.read_volatile() | reasons);
    }
}

/// The `ast_off()` macro of <kern/ast.h>: clear `reasons` on `cpu`.
pub fn ast_off(cpu: c_int, reasons: usize) {
    // SAFETY: as `ast_needed()`.
    unsafe {
        let cell = slot(cpu);
        cell.write_volatile(cell.read_volatile() & !reasons);
    }
}

/// Whether CPU `cpu` has an AST other than the scheduling reasons, which the
/// idle loop handles itself.
pub fn ast_scheduling_pending(cpu: c_int) -> bool {
    ast_needed(cpu) & !AST_SCHEDULING != 0
}

/// The `need_ast[mycpu] &= ~AST_SCHEDULING` of kern/sched_prim.c.
pub fn ast_clear_scheduling(cpu: c_int) {
    // SAFETY: as `ast_needed()`.
    unsafe {
        let cell = slot(cpu);
        cell.write_volatile(cell.read_volatile() & !AST_SCHEDULING);
    }
}

/// The `ast_context()` macro of <kern/ast.h>: replace the per-thread reasons
/// of `cpu` with `thread`'s pending ones.
///
/// # Safety
///
/// `thread` must be a live thread, and `cpu` a CPU number below
/// [`smp_get_numcpus()`].
pub unsafe fn ast_context(thread: *mut Thread, cpu: c_int) {
    // SAFETY: the caller promises a live thread and CPU, and `slot()` serves
    // the same `need_ast` array the C macro touched.
    unsafe {
        let cell = slot(cpu);
        let per_thread = cell.read_volatile() & !AST_PER_THREAD;
        cell.write_volatile(per_thread | (*thread).ast as usize);
    }
}

/// `ast_init()` of kern/ast.c.
///
/// # Safety
///
/// The scheduler runs this during boot, before any CPU can take an AST.
pub(crate) unsafe fn init() {
    let slots = NEED_AST.0.get().cast::<usize>();
    for cpu in 0..NCPUS {
        // SAFETY: no other thread can set an AST before the boot reaches the
        // scheduler, and `cpu` indexes the `NCPUS` slots.
        unsafe { slots.add(cpu).write_volatile(0) };
    }
}

/// The address of `need_ast[cpu]`.
///
/// # Safety
///
/// `cpu` must be a CPU number of the machine, below [`smp_get_numcpus()`].
unsafe fn slot(cpu: c_int) -> *mut usize {
    debug_assert!(
        0 <= cpu && cpu < c_int::from(smp_get_numcpus()),
        "AST cpu {cpu} outside the {} probed CPUs",
        smp_get_numcpus(),
    );
    // SAFETY: the caller promises a live CPU number and the debug assertion
    // holds it below the probed count, at most `NCPUS`; the cast cannot wrap
    // because `cpu` is non-negative.
    unsafe { NEED_AST.0.get().cast::<usize>().add(cpu as usize) }
}

/// `ast_taken()` of kern/ast.c: act on the ASTs pending on the running CPU.
///
/// # Safety
///
/// Machine-dependent code calls this on return to user mode with interrupts
/// disabled; the CPU must have no AST action in progress.
pub(crate) unsafe fn taken() {
    let self_ = current_thread();
    let cpu = cpu_number();
    let reasons = ast_needed(cpu);
    // SAFETY: as `ast_needed()`; the C overwrote its own slot with
    // `AST_ZILCH` at this point.
    unsafe { slot(cpu).write_volatile(0) };
    // SAFETY: the C raised the interrupt level here.
    unsafe { glue::spl0() };

    if reasons & AST_NETWORK != 0 {
        // SAFETY: the network code owns the AST; interrupts are enabled, as
        // the C had them.
        unsafe { glue::net_ast() };
    }

    let myprocessor = current_processor();
    // SAFETY: the caller promises a live running thread and processor.
    unsafe {
        if self_ == (*myprocessor).idle_thread {
            return;
        }

        while should_halt(self_) {
            thread_halt_self(Some(glue::thread_exception_return));
        }

        if reasons & AST_BLOCK != 0 || csw_needed(self_, myprocessor) {
            thread_block(Some(glue::thread_exception_return));
        }
    }
}

/// `ast_check()` of kern/ast.c: check the running processor for AST
/// conditions at `splsched`.
///
/// # Safety
///
/// The caller must be the running thread's CPU, able to take `splsched`, and
/// must not hold the run-queue lock.
pub(crate) unsafe fn check() {
    let mycpu = cpu_number();
    let thread = current_thread();
    // SAFETY: the boot and scheduler code set `percpu_array` for this CPU.
    let myprocessor = unsafe { processor_ptr(mycpu) };
    // SAFETY: as the C; the level is the caller's.
    let s = unsafe { glue::splsched() };

    // SAFETY: the caller promises the running CPU and its live processor; the
    // run-queue lock is taken only around the hint update.
    unsafe {
        match (*myprocessor).state {
            PROCESSOR_OFF_LINE | PROCESSOR_IDLE | PROCESSOR_DISPATCHING => (),
            PROCESSOR_ASSIGN | PROCESSOR_SHUTDOWN => ast_on(mycpu, AST_BLOCK),
            PROCESSOR_RUNNING => {
                ast_on(mycpu, (*thread).ast as usize);
                if ast_needed(mycpu) == 0 {
                    check_running(mycpu, thread, myprocessor);
                }
            }
            state => panic_bad_state(mycpu, myprocessor, state),
        }

        glue::splx(s);
    }
}

/// `thread_should_halt()` of <kern/thread.h>: the halt and terminate ASTs.
///
/// # Safety
///
/// `thread` must be a live thread.
unsafe fn should_halt(thread: *mut Thread) -> bool {
    // SAFETY: the caller promises the live thread.
    unsafe {
        (*thread).ast as usize & (AST_HALT | AST_TERMINATE) as usize != 0
    }
}

/// The `PROCESSOR_RUNNING` arm of `ast_check()`, after the thread's own ASTs
/// have been propagated.
///
/// # Safety
///
/// `myprocessor` must be the running CPU's processor, `thread` its current
/// thread, and the caller must hold `splsched` with the run-queue lock free.
unsafe fn check_running(
    mycpu: c_int,
    thread: *mut Thread,
    myprocessor: *mut crate::kern::processor::Processor,
) {
    // SAFETY: the caller's contract; the run queue is the processor set's.
    unsafe {
        if (*thread).state() & TH_SUSP != 0 || (*myprocessor).runq.count > 0 {
            ast_on(mycpu, AST_BLOCK);
            return;
        }

        let pset = (*myprocessor).processor_set;
        if (*pset).policies & POLICY_FIXEDPRI != 0 {
            if csw_needed(thread, myprocessor) {
                ast_on(mycpu, AST_BLOCK);
            } else if (*thread).policy == POLICY_FIXEDPRI {
                (*myprocessor).first_quantum = 1;
            }
            return;
        }

        let rq = &raw mut (*pset).runq;
        if (*myprocessor).first_quantum != 0 || (*rq).count == 0 {
            return;
        }

        let low = (*rq).low;
        if queue_empty(queue_at(rq, low)) != 0 {
            (*rq).lock.lock();
            if (*rq).count > 0 {
                let mut i = (*rq).low;
                while i < NRQS as c_int {
                    if queue_empty(queue_at(rq, i)) == 0 {
                        break;
                    }
                    i += 1;
                }
                (*rq).low = i;
            }
            (*rq).lock.unlock();
        }

        if (*rq).low <= (*thread).sched_pri {
            ast_on(mycpu, AST_BLOCK);
        }
    }
}

/// `rq->runq[low]` of the C run-queue hint walk.
///
/// # Safety
///
/// `rq` must be a live run queue and `low` a valid queue index.
unsafe fn queue_at(
    rq: *mut crate::kern::sched::RunQueue,
    low: c_int,
) -> *mut crate::kern::queue::QueueEntry {
    // SAFETY: the caller promises the index; `NRQS` is the array's length.
    unsafe { (&raw mut (*rq).runq).cast::<QueueEntry>().add(low as usize) }
}

/// The `csw_needed()` macro of <kern/sched.h>.
///
/// # Safety
///
/// `thread` must be live and `processor` the running CPU's processor.
unsafe fn csw_needed(
    thread: *mut Thread,
    processor: *mut crate::kern::processor::Processor,
) -> bool {
    // SAFETY: the caller's contract; the fields are readable and the
    // processor set pointer is live while the processor runs.
    unsafe {
        (*thread).state() & TH_SUSP != 0
            || (*processor).runq.count > 0
            || (*(*processor).processor_set).runq.count > 0
    }
}

/// The `default: panic()` arm of `ast_check()`.
fn panic_bad_state(
    mycpu: c_int,
    processor: *mut crate::kern::processor::Processor,
    state: c_int,
) -> ! {
    // SAFETY: `Panic` does not return; the file and function are the C
    // panic macro's, and the arguments match its format string.
    unsafe {
        glue::Panic(
            c"kern/ast.c".as_ptr(),
            // Only `c_int` widths can reach `Panic`'s varargs.
            line!() as c_int,
            c"ast_check".as_ptr(),
            c"ast_check: Bad processor state (cpu %d processor %p) state: %d"
                .as_ptr(),
            mycpu,
            processor,
            state,
        )
    }
}
