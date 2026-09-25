// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/priority.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/priority.h:
//   Copyright (c) 2013 Free Software Foundation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The quantum recalculation `kern/priority.c` used to define for
//! <kern/priority.h>.

use crate::arch::i386::percpu::processor_ptr;
use crate::glue;
use crate::kern::ast;
use crate::kern::mach_clock::CPU_STATE_IDLE;
use crate::kern::policy::POLICY_TIMESHARE;
use crate::kern::sched::{PRI_SHIFT, SCHED_SHIFT};
use crate::kern::sched_prim::{
    compute_my_priority, min_quantum, sched_tick, update_priority,
};
use crate::kern::thread::Thread;
use core::ffi::{c_int, c_uint};

/// `USAGE_THRESHOLD` in kern/priority.c: the change that moves a thread
/// between run queues.
const USAGE_THRESHOLD: c_uint = 1 << (PRI_SHIFT + 2 + SCHED_SHIFT);

/// `thread_quantum_update()` of kern/priority.c.
///
/// # Safety
///
/// `mycpu` must be the running CPU, `thread` its live interrupted thread or
/// the thread the clock charged, and the caller must hold no lock the thread
/// or processor-set lock would nest under.
pub(crate) unsafe fn thread_quantum_update(
    mycpu: c_int,
    thread: *mut Thread,
    nticks: c_int,
    state: c_int,
) {
    // SAFETY: the caller promises a live CPU number, so its per-CPU slot holds
    // a live processor.
    let myprocessor = unsafe { processor_ptr(mycpu) };
    // SAFETY: the processor record is live; its set is null only while the
    // assignment code is moving it.
    let pset = unsafe { (*myprocessor).processor_set };
    if pset.is_null() {
        return;
    }

    // SAFETY: the set is live; both counts are non-negative and
    // `processor_count` is at most `NCPUS`, so the index is inside
    // `machine_quantum`.  The fallback is the value the array itself starts
    // at, so a corrupt count cannot read foreign state.
    unsafe {
        (*pset).set_quantum =
            usize::try_from(if (*pset).runq.count > (*pset).processor_count {
                (*pset).processor_count
            } else {
                (*pset).runq.count
            })
            .ok()
            .and_then(|index| (*pset).machine_quantum.get(index))
            .copied()
            .unwrap_or_else(min_quantum);

        let mut quantum = if (*myprocessor).runq.count != 0 {
            min_quantum()
        } else {
            (*pset).set_quantum
        };

        if state != CPU_STATE_IDLE {
            (*myprocessor).quantum =
                (*myprocessor).quantum.wrapping_sub(nticks);

            if quantum != (*myprocessor).last_quantum
                && (*pset).processor_count > 1
            {
                (*myprocessor).last_quantum = quantum;
                let level = glue::splhigh();
                (*pset).quantum_adj_lock.lock();
                let index = (*pset).quantum_adj_index;
                quantum = min_quantum().wrapping_add(
                    index.wrapping_mul(quantum.wrapping_sub(min_quantum()))
                        / (*pset).processor_count.wrapping_sub(1),
                );
                let next = index.wrapping_add(1);
                (*pset).quantum_adj_index = if next >= (*pset).processor_count
                {
                    0
                } else {
                    next
                };
                (*pset).quantum_adj_lock.unlock();
                glue::splx(level);
            }

            if (*myprocessor).quantum <= 0 {
                let level = glue::splsched();
                (*thread).lock.lock();
                if (*thread).sched_stamp != sched_tick() {
                    update_priority(thread);
                } else if (*thread).policy == POLICY_TIMESHARE
                    && (*thread).depress_priority < 0
                {
                    Thread::timer_delta(thread);
                    (*thread).sched_usage = (*thread)
                        .sched_usage
                        .wrapping_add((*thread).sched_delta);
                    (*thread).sched_delta = 0;
                    compute_my_priority(thread);
                }
                (*thread).lock.unlock();
                glue::splx(level);

                (*myprocessor).first_quantum = 0;
                if (*thread).policy == POLICY_TIMESHARE {
                    (*myprocessor).quantum =
                        (*myprocessor).quantum.wrapping_add(quantum);
                } else {
                    (*myprocessor).quantum = (*myprocessor)
                        .quantum
                        .wrapping_add((*thread).sched_data);
                }
            } else {
                let level = glue::splsched();
                (*thread).lock.lock();
                if (*thread).sched_stamp != sched_tick() {
                    update_priority(thread);
                } else if (*thread).policy == POLICY_TIMESHARE
                    && (*thread).depress_priority < 0
                {
                    Thread::timer_delta(thread);
                    if (*thread).sched_delta >= USAGE_THRESHOLD {
                        (*thread).sched_usage = (*thread)
                            .sched_usage
                            .wrapping_add((*thread).sched_delta);
                        (*thread).sched_delta = 0;
                        compute_my_priority(thread);
                    }
                }
                (*thread).lock.unlock();
                glue::splx(level);
            }

            ast::check();
        }
    }
}
