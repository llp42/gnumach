/* 
 * Mach Operating System
 * Copyright (c) 1993 Carnegie Mellon University
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

#include <kern/printf.h>
#include <stdarg.h>

#include "cpu_number.h"
#include <kern/lock.h>
#include <kern/thread.h>

#include <kern/debug.h>

#include <machine/loose_ends.h>
#include <machine/model_dep.h>

#include <device/cons.h>

simple_lock_irq_data_t Assert_print_lock;

static void
do_cnputc(char c, vm_offset_t offset)
{
	cnputc(c);
}

void
Assert(const char *exp, const char *file, int line, const char *fun)
{
	spl_t s = simple_lock_irq(&Assert_print_lock);
	printf("{cpu%d} %s:%d: %s: Assertion `%s' failed.",
	       cpu_number(), file, line, fun, exp);
	simple_unlock_irq(s, &Assert_print_lock);

	Debugger("assertion failure");
}

void SoftDebugger(const char *message)
{
	printf("Debugger invoked: %s\n", message);

	printf("But no debugger, continuing.\n");
}

void Debugger(const char *message)
{
	panic("Debugger invoked, but there isn't one!");
}

/* Be prepared to panic anytime,
   even before panic_init() gets called from the "normal" place in kern/startup.c.
   (panic_init() still needs to be called from there
   to make sure we get initialized before starting multiple processors.)  */
def_simple_lock_irq_data(static,	panic_lock)

const char     		*panicstr;
int			paniccpu;

void
panic_init(void)
{
	simple_lock_init_irq(&Assert_print_lock);
	simple_lock_init_irq(&panic_lock);
}

extern boolean_t reboot_on_panic;

/*VARARGS1*/
void
Panic(const char *file, int line, const char *fun, const char *s, ...)
{
	va_list	listp;
	spl_t spl;

	panic_init();

	spl = simple_lock_irq(&panic_lock);
	if (panicstr) {
	    if (cpu_number() != paniccpu) {
		simple_unlock_irq(spl, &panic_lock);
		halt_cpu();
		/* NOTREACHED */
	    }
	}
	else {
	    panicstr = s;
	    paniccpu = cpu_number();
	}
	simple_unlock_irq(spl, &panic_lock);
	printf("panic ");
	printf("{cpu%d} ", paniccpu);
	printf("%s:%d: %s: ",file, line, fun);
	va_start(listp, s);
	_doprnt(s, listp, do_cnputc, 16, 0);
	va_end(listp);
	printf("\n");

	/* Give the user time to see the message */
	{
	  int i = 1000;		/* seconds */
	  while (i--)
	    delay (1000000);	/* microseconds */
	}

	halt_all_cpus (reboot_on_panic);
}

/*
 * We'd like to use BSD's log routines here...
 */
/*VARARGS2*/
void
log(int level, const char *fmt, ...)
{
	va_list	listp;

	va_start(listp, fmt);
	_doprnt(fmt, listp, do_cnputc, 16, 0);
	va_end(listp);
}

/* GCC references this for stack protection.  */
unsigned char __stack_chk_guard [ sizeof (vm_offset_t) ] =
{
	[ sizeof (vm_offset_t) - 3 ] = '\r',
	[ sizeof (vm_offset_t) - 2 ] = '\n',
	[ sizeof (vm_offset_t) - 1 ] = 0xff,
};

void __stack_chk_fail (void);

void
__stack_chk_fail (void)
{
	panic("stack smashing detected");
}
