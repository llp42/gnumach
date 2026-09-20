/* 
 * Mach Operating System
 * Copyright (c) 1991,1990,1989 Carnegie Mellon University
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
 */
/*
 *	File:	ipc/ipc_thread.h
 *	Author:	Rich Draves
 *	Date:	1989
 *
 *	Definitions for the IPC component of threads.
 */

#ifndef	_IPC_IPC_THREAD_H_
#define _IPC_IPC_THREAD_H_

#include <kern/thread.h>

typedef thread_t ipc_thread_t;

/*
 *	Note that this isn't a queue, but rather a stack. This causes
 *	threads that were recently running to be reused earlier, which
 *	helps improve locality of reference.
 */
typedef struct ipc_thread_queue {
	ipc_thread_t ithq_base;
} *ipc_thread_queue_t;


/*
 * The operations are Rust now (rust/src/ipc/ipc_thread.rs); ipc_thread.c
 * is gone and the former header macros are functions.  The unused
 * ipc_thread_queue_empty() was not carried over.
 */

/* Make a thread's links point at itself, i.e. unlinked */
extern void ipc_thread_links_init(
	ipc_thread_t		thread);

/* Empty a queue */
extern void ipc_thread_queue_init(
	ipc_thread_queue_t	queue);

/* The first thread of a queue, or THREAD_NULL */
extern ipc_thread_t ipc_thread_queue_first(
	ipc_thread_queue_t	queue);

/* Enqueue a thread on a message queue */
extern void ipc_thread_enqueue(
	ipc_thread_queue_t	queue,
	ipc_thread_t		thread);

/* Dequeue a thread from a message queue */
extern ipc_thread_t ipc_thread_dequeue(
	ipc_thread_queue_t	queue);

/* Remove a thread from a message queue */
extern void ipc_thread_rmqueue(
	ipc_thread_queue_t	queue,
	ipc_thread_t		thread);

/* Remove the first thread from a message queue */
extern void ipc_thread_rmqueue_first(
	ipc_thread_queue_t	queue,
	ipc_thread_t		thread);

#endif	/* _IPC_IPC_THREAD_H_ */
