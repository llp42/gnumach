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
/* Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com> */

#include <kern/printf.h>
#include <stdarg.h>

#include "cpu_number.h"
#include <kern/lock.h>
#include <kern/thread.h>

#include <kern/debug.h>

#include <machine/loose_ends.h>
#include <machine/model_dep.h>

#include <device/cons.h>


static void
do_cnputc(char c, vm_offset_t offset)
{
	cnputc(c);
}


/* Be prepared to panic anytime,
   even before panic_init() gets called from the "normal" place in kern/startup.c.
   (panic_init() still needs to be called from there
   to make sure we get initialized before starting multiple processors.)  */
decl_simple_lock_irq_data(extern,	panic_lock)

const char     		*panicstr;
int			paniccpu;

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

