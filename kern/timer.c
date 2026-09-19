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

#include <mach/kern_return.h>
#include <mach/port.h>
#include <kern/queue.h>
#include <kern/thread.h>
#include <mach/time_value.h>
#include <kern/timer.h>
#include <kern/cpu_number.h>

#include <kern/assert.h>
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

/*
 *	timer_init initializes a single timer.
 */
void timer_init(timer_t this_timer)
{
	this_timer->low_bits = 0;
	this_timer->high_bits = 0;
	this_timer->tstamp = 0;
	this_timer->high_bits_check = 0;
}

/*
 *	timer_normalize normalizes the value of a timer.  It is
 *	called only rarely, to make sure low_bits never overflows.
 */
void timer_normalize(timer_t timer)
{
	unsigned int	high_increment;

	/*
	 *	Calculate high_increment, then write high check field first
	 *	followed by low and high.  timer_grab() reads these fields in
	 *	reverse order so if high and high check match, we know
	 *	that the values read are ok.
	 */

	high_increment = timer->low_bits/TIMER_HIGH_UNIT;
	timer->high_bits_check += high_increment;
	__sync_synchronize();
	timer->low_bits %= TIMER_HIGH_UNIT;
	__sync_synchronize();
	timer->high_bits += high_increment;
}

/*
 *	timer_grab() retrieves the value of a timer.
 *
 *	Critical scheduling code uses TIMER_DELTA macro in timer.h
 *	(called from thread_timer_delta in sched.h).
 *
 *      Keep coherent with db_time_grab below.
 */

static void timer_grab(
	timer_t		timer,
	timer_save_t	save)
{
  unsigned int passes=0;
	do {
		(save)->high = (timer)->high_bits;
		__sync_synchronize ();
		(save)->low = (timer)->low_bits;
		__sync_synchronize ();
	/*
	 *	If the timer was normalized while we were doing this,
	 *	the high_bits value read above and the high_bits check
	 *	value will not match because high_bits_check is the first
	 *	field touched by the normalization procedure, and
	 *	high_bits is the last.
	 *
	 *	Additions to timer only touch low bits and
	 *	are therefore atomic with respect to this.
	 */
		passes++;
		assert((passes < 10000) ? (1) : ((timer->high_bits_check = save->high), 0));
	} while ( (save)->high != (timer)->high_bits_check);
}

#define TIMER_TO_TIME_VALUE64(tv, timer) 				\
MACRO_BEGIN								\
		(tv)->seconds = (timer)->high + (timer)->low / 1000000;	\
		(tv)->nanoseconds = (timer)->low % 1000000 * 1000;	\
MACRO_END

/*
 *	timer_read reads the value of a timer into a time_value64_t.  If the
 *	timer was modified during the read, retry.  The value returned
 *	is accurate to the last update; time accumulated by a running
 *	timer since its last timestamp is not included.
 */

void
timer_read(
	timer_t 	timer,
	time_value64_t 	*tv)
{
	timer_save_data_t	temp;

	timer_grab(timer,&temp);
	TIMER_TO_TIME_VALUE64(tv, &temp);
}

/*
 *	thread_read_times reads the user and system times from a thread.
 *	Time accumulated since last timestamp is not included.  Should
 *	be called at splsched() to avoid having user and system times
 *	be out of step.  Doesn't care if caller locked thread.
 *
 *      Needs to be kept coherent with thread_read_times ahead.
 */
void	thread_read_times(
	thread_t 	thread,
	time_value64_t	*user_time_p,
	time_value64_t	*system_time_p)
{
	timer_read(&thread->user_timer, user_time_p);
	timer_read(&thread->system_timer, system_time_p);
}

/*
 *
 * 	Db_timer_grab(): used by db_thread_read_times. An nonblocking
 *      version of db_thread_get_times. Keep coherent with timer_grab
 *      above.
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

/*
 *	timer_delta takes the difference of a saved timer value
 *	and the current one, and updates the saved value to current.
 *	The difference is returned as a function value.  See
 *	TIMER_DELTA macro (timer.h) for optimization to this.
 */

unsigned
timer_delta(
	timer_t		timer,
	timer_save_t	save)
{
	timer_save_data_t	new_save;
	unsigned		result;

	timer_grab(timer,&new_save);
	result = (new_save.high - save->high) * TIMER_HIGH_UNIT +
		new_save.low - save->low;
	save->high = new_save.high;
	save->low = new_save.low;
	return(result);
}
