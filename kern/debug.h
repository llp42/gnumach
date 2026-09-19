/*
 * Copyright (c) 1993,1994 The University of Utah and
 * the Computer Systems Laboratory (CSL).  All rights reserved.
 *
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 *
 * THE UNIVERSITY OF UTAH AND CSL ALLOW FREE USE OF THIS SOFTWARE IN ITS "AS
 * IS" CONDITION.  THE UNIVERSITY OF UTAH AND CSL DISCLAIM ANY LIABILITY OF
 * ANY KIND FOR ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF THIS SOFTWARE.
 *
 * CSL requests users of this software to return to csl-dist@cs.utah.edu any
 * improvements that they make and grant CSL redistribution rights.
 *
 *      Author: Bryan Ford, University of Utah CSL
 */
/*
 *	File:	debug.h
 *	Author:	Bryan Ford
 *
 *	This file contains definitions for kernel debugging,
 *	which are compiled in on the DEBUG symbol.
 *
 */
#ifndef _mach_debug__debug_
#define _mach_debug__debug_

#include <kern/assert.h> /*XXX*/


extern void log (int level, const char *fmt, ...);

extern void panic_init(void);
extern void Panic (const char *file, int line, const char *fun,
		   const char *s, ...)
	__attribute__ ((noreturn, format (printf, 4, 5)));
#define panic(s, ...)							\
	Panic (__FILE__, __LINE__, __FUNCTION__, s, ##__VA_ARGS__)

extern void SoftDebugger (const char *message);
extern void Debugger (const char *message) __attribute__ ((noreturn));

#endif /* _mach_debug__debug_ */
