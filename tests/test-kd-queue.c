/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Exercise the user-mode copy of the keyboard/mouse ring buffer in
 * tests/kd_queue.c, pinning the behaviour of the kernel's Rust
 * implementation in rust/src/utils/kd_queue.rs.
 */

#include <testlib.h>

#include "kd_queue.h"

static kd_event_queue q;

/* Enqueue an event whose value is `value`; the caller has room. */
static void
put(int value)
{
	kd_event ev;

	ASSERT(!kdq_full(&q), "put into a full queue");

	ev.type = 5;
	ev.unused_time.seconds = 11;
	ev.unused_time.microseconds = 22;
	ev.value.up = value;
	kdq_put(&q, &ev);
}

/* Dequeue one event and return its value; the caller has events. */
static int
get(void)
{
	kd_event *ev;

	ASSERT(!kdq_empty(&q), "get from an empty queue");

	ev = kdq_get(&q);
	ASSERT(ev->type == 5, "event type not copied");
	ASSERT(ev->unused_time.seconds == 11, "event seconds not copied");
	ASSERT(ev->unused_time.microseconds == 22,
	       "event microseconds not copied");
	return ev->value.up;
}

static void
test_reset(void)
{
	kdq_reset(&q);
	ASSERT(kdq_empty(&q), "reset queue is not empty");
	ASSERT(!kdq_full(&q), "reset queue is full");
	ASSERT(q.firstfree == 0 && q.firstout == 0, "reset indices");
}

static void
test_put_get(void)
{
	kd_event *ev;

	kdq_reset(&q);

	put(42);
	ASSERT(!kdq_empty(&q), "queue empty after one put");
	ASSERT(!kdq_full(&q), "one-event queue is full");

	/* The event landed in the free slot, not somewhere else. */
	ASSERT(q.firstfree == 1, "firstfree not advanced");
	ASSERT(q.events[0].value.up == 42, "event not copied to events[0]");

	ev = kdq_get(&q);
	ASSERT(ev == &q.events[0], "get did not return the first slot");
	ASSERT(ev->type == 5, "type not copied");
	ASSERT(ev->unused_time.microseconds == 22, "time not copied");
	ASSERT(ev->value.up == 42, "value not copied");
	ASSERT(q.firstout == 1, "firstout not advanced");
	ASSERT(kdq_empty(&q), "queue not empty after get");
}

static void
test_fifo(void)
{
	int i;

	kdq_reset(&q);
	for (i = 0; i < KDQSIZE - 1; i++) {
		put(i);
		ASSERT(!kdq_full(&q) || i == KDQSIZE - 2,
		       "queue full before its last slot");
	}
	ASSERT(kdq_full(&q), "queue not full at KDQSIZE - 1 events");

	/* A get frees the oldest slot again; the queue refills. */
	ASSERT(get() == 0, "first event out of order");
	ASSERT(!kdq_full(&q), "queue still full after a get");
	put(KDQSIZE - 1);
	ASSERT(kdq_full(&q), "queue not full again");

	for (i = 1; i < KDQSIZE - 1; i++)
		ASSERT(get() == i, "events out of order");
	ASSERT(get() == KDQSIZE - 1, "wrapped event out of order");
	ASSERT(kdq_empty(&q), "queue not empty after draining");
}

static void
test_wrap(void)
{
	int i;

	/* Each put/get pair advances both indices, wrapping many times. */
	kdq_reset(&q);
	for (i = 0; i < 3 * KDQSIZE; i++) {
		put(i);
		ASSERT(get() == i, "event out of order across the wrap");
	}
	ASSERT(kdq_empty(&q), "queue not empty after the wrap cycle");
	ASSERT(q.firstfree < KDQSIZE && q.firstout < KDQSIZE,
	       "index out of range");
}

static void
test_get_pointer(void)
{
	kd_event *first;

	kdq_reset(&q);
	put(1);
	put(2);

	first = kdq_get(&q);
	ASSERT(first == &q.events[0], "get returned the wrong slot");
	ASSERT(first->value.up == 1, "wrong event");
	ASSERT(kdq_get(&q) == &q.events[1], "next slot not returned");
	ASSERT(first->value.up == 1, "slot changed under the reader");
}

static void
test_reset_nonempty(void)
{
	kdq_reset(&q);
	put(1);
	put(2);
	kdq_reset(&q);
	ASSERT(kdq_empty(&q), "reset queue is not empty");
	ASSERT(!kdq_full(&q), "reset queue is full");
	ASSERT(q.firstfree == 0 && q.firstout == 0, "reset indices");
}

int
main(int argc, char *argv[], int envc, char *envp[])
{
	test_reset();
	test_put_get();
	test_fifo();
	test_wrap();
	test_get_pointer();
	test_reset_nonempty();
	return 0;
}
