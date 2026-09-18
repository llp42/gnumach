/*
 * Mach Operating System
 * Copyright (c) 1993,1992,1991,1990 Carnegie Mellon University
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
 * Interface to new debugger.
 */

#include <string.h>
#include <sys/reboot.h>
#include <vm/pmap.h>

#include <i386/thread.h>
#include <i386/db_machdep.h>
#include <i386/seg.h>
#include <i386/trap.h>
#include <i386/pmap.h>
#include <i386/proc_reg.h>
#include <i386/locore.h>
#include <i386at/biosmem.h>
#include "gdt.h"
#include "trap.h"

#include "vm_param.h"
#include <vm/vm_map.h>
#include <vm/vm_fault.h>
#include <kern/cpu_number.h>
#include <kern/printf.h>
#include <kern/thread.h>
#include <kern/task.h>
#include <machine/db_interface.h>
#include <machine/spl.h>

/* Whether the current debug registers are zero.  */
static boolean_t zero_dr;

db_regs_t	ddb_regs;

void db_load_context(pcb_t pcb)
{
	/* Else set user debug registers, if any */
	unsigned int *dr = pcb->ims.ids.dr;
	boolean_t will_zero_dr = !dr[0] && !dr[1] && !dr[2] && !dr[3] && !dr[7];

	if (!(zero_dr && will_zero_dr))
	{
		set_dr0(dr[0]);
		set_dr1(dr[1]);
		set_dr2(dr[2]);
		set_dr3(dr[3]);
		set_dr7(dr[7]);
		zero_dr = will_zero_dr;
	}

}

void db_get_debug_state(
	pcb_t pcb,
	struct i386_debug_state *state)
{
	*state = pcb->ims.ids;
}

kern_return_t db_set_debug_state(
	pcb_t pcb,
	const struct i386_debug_state *state)
{
	int i;

	for (i = 0; i <= 3; i++)
		if (state->dr[i] < VM_MIN_USER_ADDRESS
		 || state->dr[i] >= VM_MAX_USER_ADDRESS)
			return KERN_INVALID_ARGUMENT;

	pcb->ims.ids = *state;

	if (pcb == current_thread()->pcb)
		db_load_context(pcb);

	return KERN_SUCCESS;
}

