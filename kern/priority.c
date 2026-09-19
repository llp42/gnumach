/*
 * Mach Operating System
 * Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
 * Copyright (c) 1993,1994 The University of Utah and
 * the Computer Systems Laboratory (CSL).
 * All rights reserved.
 *
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 *
 * CARNEGIE MELLON, THE UNIVERSITY OF UTAH AND CSL ALLOW FREE USE OF
 * THIS SOFTWARE IN ITS "AS IS" CONDITION, AND DISCLAIM ANY LIABILITY
 * OF ANY KIND FOR ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF
 * THIS SOFTWARE.
 *
 * Carnegie Mellon requests users of this software to return to
 *
 *  Software Distribution Coordinator  or  Software.Distribution@CS.CMU.EDU
 *  School of Computer Science
 *  Carnegie Mellon University
 *  Pittsburgh PA 15213-3890
 *
 * any improvements or extensions that they make and grant Carnegie Mellon
 * the rights to redistribute these changes.
 */
/*
 *	File:	priority.c
 *	Author:	Avadis Tevanian, Jr.
 *	Date:	1986
 *
 *	Clock primitives.
 */

#include <mach/boolean.h>
#include <mach/kern_return.h>
#include <mach/machine.h>
#include <kern/host.h>
#include <kern/mach_clock.h>
#include <kern/sched.h>
#include <kern/sched_prim.h>
#include <kern/thread.h>
#include <kern/priority.h>
#include <kern/processor.h>
#include <kern/timer.h>
#include <machine/spl.h>



/*
 *	USAGE_THRESHOLD is the amount by which usage must change to
 *	cause a priority shift that moves a thread between run queues.
 */

#define USAGE_THRESHOLD	(1 << (PRI_SHIFT + 2 + SCHED_SHIFT))

/*
 *	thread_quantum_update:
 *
 *	Recalculate the quantum and priority for a thread.
 *	The number of ticks that has elapsed since we were last called
 *	is passed as "nticks."
 *
 *	Called only from clock_interrupt().
 */

void thread_quantum_update(
	int			mycpu,
	thread_t		thread,
	int			nticks,
	int			state)
{
	int				quantum;
	processor_t			myprocessor;
	processor_set_t			pset;
	spl_t				s;

	myprocessor = processor_ptr(mycpu);
	pset = myprocessor->processor_set;
	if (pset == 0) {
	    /*
	     * Processor is being reassigned.
	     * Should rewrite processor assignment code to
	     * block clock interrupts.
	     */
	    return;
	}

	/*
	 *	Account for thread's utilization of these ticks.
	 *	This assumes that there is *always* a current thread.
	 *	When the processor is idle, it should be the idle thread.
	 */

	/*
	 *	Update set_quantum and calculate the current quantum.
	 */
	pset->set_quantum = pset->machine_quantum[
		((pset->runq.count > pset->processor_count) ?
		  pset->processor_count : pset->runq.count)];

	if (myprocessor->runq.count != 0)
		quantum = min_quantum;
	else
		quantum = pset->set_quantum;
		
	/*
	 *	Now recompute the priority of the thread if appropriate.
	 */

	if (state != CPU_STATE_IDLE) {
		myprocessor->quantum -= nticks;
		/*
		 *	Runtime quantum adjustment.  Use quantum_adj_index
		 *	to avoid synchronizing quantum expirations.
		 */
		if ((quantum != myprocessor->last_quantum) &&
		    (pset->processor_count > 1)) {
			myprocessor->last_quantum = quantum;
			s = simple_lock_irq(&pset->quantum_adj_lock);
			quantum = min_quantum + (pset->quantum_adj_index *
				(quantum - min_quantum)) / 
					(pset->processor_count - 1);
			if (++(pset->quantum_adj_index) >=
			    pset->processor_count)
				pset->quantum_adj_index = 0;
			simple_unlock_irq(s, &pset->quantum_adj_lock);
		}
		if (myprocessor->quantum <= 0) {
			s = splsched();
			_simple_lock(&(thread)->lock);
			if (thread->sched_stamp != sched_tick) {
				update_priority(thread);
			}
			else {
			    if (
				(thread->policy == POLICY_TIMESHARE) &&
				(thread->depress_priority < 0)) {
				    thread_timer_delta(thread);
				    thread->sched_usage +=
					thread->sched_delta;
				    thread->sched_delta = 0;
				    compute_my_priority(thread);
			    }
			}
			_simple_unlock(&(thread)->lock);
			(void) splx(s);
			/*
			 *	This quantum is up, give this thread another.
			 */
			myprocessor->first_quantum = FALSE;
			if (thread->policy == POLICY_TIMESHARE) {
				myprocessor->quantum += quantum;
			}
			else {
				/*
				 *    Fixed priority has per-thread quantum.
				 *    
				 */
				myprocessor->quantum += thread->sched_data;
			}
		}
		/*
		 *	Recompute priority if appropriate.
		 */
		else {
		    s = splsched();
		    _simple_lock(&(thread)->lock);
		    if (thread->sched_stamp != sched_tick) {
			update_priority(thread);
		    }
		    else {
			if (
			    (thread->policy == POLICY_TIMESHARE) &&
			    (thread->depress_priority < 0)) {
				thread_timer_delta(thread);
				if (thread->sched_delta >= USAGE_THRESHOLD) {
				    thread->sched_usage +=
					thread->sched_delta;
				    thread->sched_delta = 0;
				    compute_my_priority(thread);
				}
			}
		    }
		    _simple_unlock(&(thread)->lock);
		    (void) splx(s);
		}
		/*
		 * Check for and schedule ast if needed.
		 */
		ast_check();
	}
}

