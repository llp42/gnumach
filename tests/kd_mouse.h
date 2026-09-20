/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The mouse packet decoders, as the test programs' copy.
 *
 * The kernel's driver lives in rust/src/arch/i386/kd_mouse.rs now.
 * These are user-mode binaries with their own link, so they carry their
 * own C copy of the pure decoders; the copy records the events they
 * produce instead of queueing them, so the tests can check them.
 */

#ifndef TEST_KD_MOUSE_H
#define TEST_KD_MOUSE_H

/* <i386at/kd_mouse.h> and <device/input.h>. */
#define MOUSEBUFSIZE	5

#define MOUSE_UP	1
#define MOUSE_DOWN	0
#define MOUSE_LEFT	1
#define MOUSE_MIDDLE	2
#define MOUSE_RIGHT	3

typedef unsigned char u_char;
typedef unsigned short kev_type;

struct mouse_motion {
	short mm_deltaX;
	short mm_deltaY;
};

enum test_mouse_kind {
	TEST_MOUSE_MOTION,
	TEST_MOUSE_BUTTON,
};

struct test_mouse_event {
	enum test_mouse_kind kind;
	struct mouse_motion motion;
	kev_type which;
	u_char direction;
};

void mouse_moved(struct mouse_motion where);
void mouse_button(kev_type which, u_char direction);
void mouse_packet_mouse_system_mouse(u_char mousebuf[MOUSEBUFSIZE]);
void mouse_packet_microsoft_mouse(u_char mousebuf[MOUSEBUFSIZE]);
void mouse_packet_ibm_ps2_mouse(u_char mousebuf[MOUSEBUFSIZE]);

void test_mouse_reset(u_char lastbuttons, int middlegitech);
int test_mouse_events(void);
const struct test_mouse_event *test_mouse_event(int index);

#endif /* TEST_KD_MOUSE_H */
