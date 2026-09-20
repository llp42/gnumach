/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The keyboard/mouse event ring buffer, as the test programs' copy.
 *
 * The kernel's queue lives in rust/src/utils/kd_queue.rs now.  These
 * are user-mode binaries with their own link, so they carry their own
 * C copy, kept semantically identical to the old
 * i386/i386at/kd_queue.c: one slot stays free, events leave in the
 * order they arrived, and kdq_get() hands back a pointer into the
 * queue.
 */

#ifndef TEST_KD_QUEUE_H
#define TEST_KD_QUEUE_H

#define KDQSIZE	100

typedef struct {
	unsigned short type;
	struct {
		long seconds;
		int microseconds;
	} unused_time;
	union {
		int up;
		unsigned char sc;
		struct {
			short mm_deltaX;
			short mm_deltaY;
		} mmotion;
	} value;
} kd_event;

typedef struct {
	kd_event events[KDQSIZE];
	int firstfree, firstout;
} kd_event_queue;

void kdq_put(kd_event_queue *, kd_event *);
void kdq_reset(kd_event_queue *);
int kdq_empty(const kd_event_queue *);
int kdq_full(const kd_event_queue *);
kd_event *kdq_get(kd_event_queue *);

#endif /* TEST_KD_QUEUE_H */
