/*
 * Mach Operating System
 * Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University
 * All Rights Reserved.
 *
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 *
 * CARNEGIE MELLON ALLOWS FREE USE OF THIS SOFTWARE IN ITS "AS IS"
 * CONDITION.  CARNEGIE MELLON DISCLAIMS ANY LIABILITY OF ANY KIND FOR
 * ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF THIS SOFTWARE.
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
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 */

#include <mach/kern_return.h>
#include <mach/port.h>
#include <kern/queue.h>
#include <kern/thread.h>
#include <mach/time_value.h>
#include <kern/timer.h>
#include <kern/cpu_number.h>

#include <kern/macros.h>



timer_t		current_timer[NCPUS];
timer_data_t	kernel_timer[NCPUS];

/*
 *	init_timers initializes all non-thread timers and puts the
 *	service routine on the callout queue.  All timers must be
 *	serviced by the callout routine once an hour.
 */
void init_timers(void)
{
	int	i;
	timer_t	this_timer;

	/*
	 *	Initialize all the kernel timers and start the one
	 *	for this cpu (master) slaves start theirs later.
	 */
	this_timer = &kernel_timer[0];
	for ( i=0 ; i<NCPUS ; i++, this_timer++) {
		timer_init(this_timer);
		current_timer[i] = (timer_t) 0;
	}

	start_timer(&kernel_timer[cpu_number()]);
}

#define TIMER_TO_TIME_VALUE64(tv, timer) 				\
MACRO_BEGIN								\
		(tv)->seconds = (timer)->high + (timer)->low / 1000000;	\
		(tv)->nanoseconds = (timer)->low % 1000000 * 1000;	\
MACRO_END

/*
 *
 * 	Db_timer_grab(): used by db_thread_read_times. An nonblocking
 *      version of db_thread_get_times. Keep coherent with timer_grab
 *      in rust/src/kern/timer.rs.
 *
 */
static void db_timer_grab(
	timer_t		timer,
	timer_save_t	save)
{
  /* Don't worry about coherency */

  (save)->high = (timer)->high_bits;
  (save)->low = (timer)->low_bits;
}

static void
nonblocking_timer_read(
	timer_t 	timer,
	time_value64_t 	*tv)
{
	timer_save_data_t	temp;

	db_timer_grab(timer, &temp);
	TIMER_TO_TIME_VALUE64(tv, &temp);
}

/*
 *      Db_thread_read_times: A version of thread_read_times that
 *      can be called by the debugger. This version does not call
 *      timer_grab, which can block. Please keep it up to date with
 *      thread_read_times above.
 *
 */
void	db_thread_read_times(
	thread_t 	thread,
	time_value64_t	*user_time_p,
	time_value64_t	*system_time_p)
{
	nonblocking_timer_read(&thread->user_timer, user_time_p);
	nonblocking_timer_read(&thread->system_timer, system_time_p);
}
