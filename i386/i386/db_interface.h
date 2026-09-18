/*
 * Copyright (C) 2007 Free Software Foundation, Inc.
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2, or (at your option)
 * any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program; if not, write to the Free Software
 * Foundation, 675 Mass Ave, Cambridge, MA 02139, USA.
 *
 * Author: Barry deFreese.
 */

#ifndef _I386_DB_INTERFACE_H_
#define _I386_DB_INTERFACE_H_

#include <sys/types.h>
#include <kern/task.h>
#include <machine/thread.h>

extern void db_get_debug_state(
	pcb_t pcb,
	struct i386_debug_state *state);
extern kern_return_t db_set_debug_state(
	pcb_t pcb,
	const struct i386_debug_state *state);

extern void db_load_context(pcb_t pcb);

extern void cnpollc(boolean_t on);

#endif /* _I386_DB_INTERFACE_H_ */
