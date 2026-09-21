/* SPDX-License-Identifier: CMU-Mach
 * Derived from i386/i386at/kd_queue.c and i386/i386at/kd_queue.h:
 *   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
 *   Copyright Ing. C. Olivetti & C. S.p.A. 1989.
 *   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The five routines of the old i386/i386at/kd_queue.c, for the test
 * programs' own link; the kernel's definitions are Rust now.  The
 * `q_next` macro and the operations are unchanged, so the tests pin
 * the contract the Rust implementation has to keep.
 */

#include "kd_queue.h"

#define q_next(index)	(((index)+1) % KDQSIZE)

int
kdq_empty(const kd_event_queue *q)
{
	return(q->firstfree == q->firstout);
}

int
kdq_full(const kd_event_queue *q)
{
	return(q_next(q->firstfree) == q->firstout);
}

void
kdq_put(kd_event_queue *q, kd_event *ev)
{
	kd_event *qp = q->events + q->firstfree;

	qp->type = ev->type;
	qp->unused_time = ev->unused_time;
	qp->value = ev->value;
	q->firstfree = q_next(q->firstfree);
}

kd_event *
kdq_get(kd_event_queue *q)
{
	kd_event *result = q->events + q->firstout;

	q->firstout = q_next(q->firstout);
	return(result);
}

void
kdq_reset(kd_event_queue *q)
{
	q->firstout = q->firstfree = 0;
}
