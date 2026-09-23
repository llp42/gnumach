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

#include <mach/boolean.h>
#include <mach/thread_switch.h>
#include <ipc/ipc_port.h>
#include <ipc/ipc_space.h>
#include <kern/ipc_kobject.h>
#include <kern/mach_clock.h>
#include <kern/processor.h>
#include <kern/sched.h>
#include <kern/sched_prim.h>
#include <kern/syscall_subr.h>
#include <kern/ipc_sched.h>
#include <kern/task.h>
#include <kern/thread.h>
#include <machine/spl.h>	/* for splsched */

#include <mach/policy.h>

/*
 *	swtch and swtch_pri both attempt to context switch (logic in
 *	thread_block no-ops the context switch if nothing would happen).
 *	A boolean is returned that indicates whether there is anything
 *	else runnable.
 *
 *	This boolean can be used by a thread waiting on a
 *	lock or condition:  If FALSE is returned, the thread is justified
 *	in becoming a resource hog by continuing to spin because there's
 *	nothing else useful that the processor could do.  If TRUE is
 *	returned, the thread should make one more check on the
 *	lock and then be a good citizen and really suspend.
 */
static void swtch_continue(void)
{
	processor_t	myprocessor;

	myprocessor = current_processor();
	thread_syscall_return(myprocessor->runq.count > 0 ||
			      myprocessor->processor_set->runq.count > 0);
	/*NOTREACHED*/
}

boolean_t swtch(void)
{
	processor_t	myprocessor;

	myprocessor = current_processor();
	if (myprocessor->runq.count == 0 &&
	    myprocessor->processor_set->runq.count == 0)
		return(FALSE);

	thread_block(swtch_continue);
	myprocessor = current_processor();
	return(myprocessor->runq.count > 0 ||
	       myprocessor->processor_set->runq.count > 0);
}

static void swtch_pri_continue(void)
{
	thread_t	thread = current_thread();
	processor_t	myprocessor;

	if (thread->depress_priority >= 0)
		(void) thread_depress_abort(thread);
	myprocessor = current_processor();
	thread_syscall_return(myprocessor->runq.count > 0 ||
			      myprocessor->processor_set->runq.count > 0);
	/*NOTREACHED*/
}

boolean_t  swtch_pri(int pri)
{
	thread_t	thread = current_thread();
	processor_t	myprocessor;

	myprocessor = current_processor();
	if (myprocessor->runq.count == 0 &&
	    myprocessor->processor_set->runq.count == 0)
		return(FALSE);

	/*
	 *	XXX need to think about depression duration.
	 *	XXX currently using min quantum.
	 */
	thread_depress_priority(thread, min_quantum);

	thread_block(swtch_pri_continue);

	if (thread->depress_priority >= 0)
		(void) thread_depress_abort(thread);
	myprocessor = current_processor();
	return(myprocessor->runq.count > 0 ||
	       myprocessor->processor_set->runq.count > 0);
}

static void thread_switch_continue(void)
{
	thread_t	cur_thread = current_thread();

	/*
	 *  Restore depressed priority
	 */
	if (cur_thread->depress_priority >= 0)
		(void) thread_depress_abort(cur_thread);
	thread_syscall_return(KERN_SUCCESS);
	/*NOTREACHED*/
}

/*
 *	thread_switch:
 *
 *	Context switch.  User may supply thread hint.
 *
 *	Fixed priority threads that call this get what they asked for
 *	even if that violates priority order.
 */
kern_return_t thread_switch(
	mach_port_name_t 	thread_name,
	int					option,
	mach_msg_timeout_t 	option_time)
{
    thread_t			cur_thread = current_thread();
    processor_t			myprocessor;
    ipc_port_t			port;

    /*
     *	Process option.
     */
    switch (option) {
	case SWITCH_OPTION_NONE:
	    /*
	     *	Nothing to do.
	     */
	    break;

	case SWITCH_OPTION_DEPRESS:
	    /*
	     *	Depress priority for given time.
	     */
	    thread_depress_priority(cur_thread, option_time);
	    break;

	case SWITCH_OPTION_WAIT:
	    thread_will_wait_with_timeout(cur_thread, option_time);
	    break;

	default:
	    return(KERN_INVALID_ARGUMENT);
    }

    /*
     *	Check and act on thread hint if appropriate.
     */
    if ((thread_name != 0) &&
	(ipc_port_translate_send(cur_thread->task->itk_space,
				 thread_name, &port) == KERN_SUCCESS)) {
	    /* port is locked, but it might not be active */

	    /*
	     *	Get corresponding thread.
	     */
	    if (ip_active(port) && (ip_kotype(port) == IKOT_THREAD)) {
		thread_t thread;
		spl_t s;

		thread = (thread_t) port->ip_kobject;
		/*
		 *	Check if the thread is in the right pset. Then
		 *	pull it off its run queue.  If it
		 *	doesn't come, then it's not eligible.
		 */
		s = splsched();
		_simple_lock(&(thread)->lock);
		if ((thread->processor_set == cur_thread->processor_set)
		    && (rem_runq(thread) != RUN_QUEUE_NULL)) {
			/*
			 *	Hah, got it!!
			 */
			_simple_unlock(&(thread)->lock);
			(void) splx(s);
			ip_unlock(port);
			/* XXX thread might disappear on us now? */
			if (thread->policy == POLICY_FIXEDPRI) {
			    myprocessor = current_processor();
			    myprocessor->quantum = thread->sched_data;
			    myprocessor->first_quantum = TRUE;
			}
			thread_run(thread_switch_continue, thread);
			/*
			 *  Restore depressed priority
			 */
			if (cur_thread->depress_priority >= 0)
				(void) thread_depress_abort(cur_thread);

			return(KERN_SUCCESS);
		}
		_simple_unlock(&(thread)->lock);
		(void) splx(s);
	    }
	    ip_unlock(port);
    }

    /*
     *	No handoff hint supplied, or hint was wrong.  Call thread_block() in
     *	hopes of running something else.  If nothing else is runnable,
     *	thread_block will detect this.  WARNING: thread_switch with no
     *	option will not do anything useful if the thread calling it is the
     *	highest priority thread (can easily happen with a collection
     *	of timesharing threads).
     */
    myprocessor = current_processor();
    if (myprocessor->processor_set->runq.count > 0 ||
	myprocessor->runq.count > 0)
    {
	thread_block(thread_switch_continue);
    }

    /*
     *  Restore depressed priority
     */
    if (cur_thread->depress_priority >= 0)
	(void) thread_depress_abort(cur_thread);
    return(KERN_SUCCESS);
}
