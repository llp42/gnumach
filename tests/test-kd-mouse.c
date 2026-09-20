/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Exercise the user-mode copy of the mouse packet decoders in
 * tests/kd_mouse.c, pinning the behaviour of the kernel's Rust
 * implementation in rust/src/arch/i386/kd_mouse.rs.  The qemu suite
 * never opens /dev/mouse, so the decoders are tested here instead.
 */

#include <testlib.h>

#include "kd_mouse.h"

static void
test_system_motion(void)
{
	u_char buf[MOUSEBUFSIZE] = {0x80, 0x7f, 0x02, 0x01, 0x00};
	const struct test_mouse_event *ev;

	test_mouse_reset(0, 0);
	mouse_packet_mouse_system_mouse(buf);
	ASSERT(test_mouse_events() == 1, "system motion: one event");
	ev = test_mouse_event(0);
	ASSERT(ev->kind == TEST_MOUSE_MOTION, "system motion: kind");
	ASSERT(ev->motion.mm_deltaX == 128, "system motion: dx");
	ASSERT(ev->motion.mm_deltaY == 2, "system motion: dy");
}

static void
test_system_buttons(void)
{
	u_char buf[MOUSEBUFSIZE] = {0x87, 0x00, 0x00, 0x00, 0x00};
	const struct test_mouse_event *ev;

	test_mouse_reset(0x5, 0);
	mouse_packet_mouse_system_mouse(buf);
	ASSERT(test_mouse_events() == 1, "system buttons: one event");
	ev = test_mouse_event(0);
	ASSERT(ev->kind == TEST_MOUSE_BUTTON, "system buttons: kind");
	ASSERT(ev->which == MOUSE_MIDDLE, "system buttons: which");
	ASSERT(ev->direction == MOUSE_UP, "system buttons: direction");
}

static void
test_microsoft_motion(void)
{
	u_char buf[MOUSEBUFSIZE] = {0xc3, 0x00, 0x00, 0x00, 0x00};
	const struct test_mouse_event *ev;

	test_mouse_reset(0x7, 0);
	mouse_packet_microsoft_mouse(buf);
	ASSERT(test_mouse_events() == 1, "microsoft motion: one event");
	ev = test_mouse_event(0);
	ASSERT(ev->kind == TEST_MOUSE_MOTION, "microsoft motion: kind");
	ASSERT(ev->motion.mm_deltaX == -64, "microsoft motion: dx");
	ASSERT(ev->motion.mm_deltaY == 0, "microsoft motion: dy");
}

static void
test_microsoft_buttons(void)
{
	u_char buf[MOUSEBUFSIZE] = {0xf0, 0x00, 0x00, 0x00, 0x00};

	test_mouse_reset(0x7, 0);
	mouse_packet_microsoft_mouse(buf);
	ASSERT(test_mouse_events() == 2, "microsoft buttons: two events");
	ASSERT(test_mouse_event(0)->which == MOUSE_RIGHT,
	       "microsoft buttons: right");
	ASSERT(test_mouse_event(0)->direction == MOUSE_DOWN,
	       "microsoft buttons: right direction");
	ASSERT(test_mouse_event(1)->which == MOUSE_LEFT,
	       "microsoft buttons: left");
	ASSERT(test_mouse_event(1)->direction == MOUSE_DOWN,
	       "microsoft buttons: left direction");
}

static void
test_ibm_motion(void)
{
	u_char positive[MOUSEBUFSIZE] = {0x08, 0x01, 0x02, 0x00, 0x00};
	u_char negative[MOUSEBUFSIZE] = {0x10, 0x01, 0x00, 0x00, 0x00};
	const struct test_mouse_event *ev;

	test_mouse_reset(0, 0);
	mouse_packet_ibm_ps2_mouse(positive);
	ASSERT(test_mouse_events() == 1, "ibm motion: one event");
	ev = test_mouse_event(0);
	ASSERT(ev->kind == TEST_MOUSE_MOTION, "ibm motion: kind");
	ASSERT(ev->motion.mm_deltaX == 1, "ibm motion: dx");
	ASSERT(ev->motion.mm_deltaY == 2, "ibm motion: dy");

	test_mouse_reset(0, 0);
	mouse_packet_ibm_ps2_mouse(negative);
	ASSERT(test_mouse_events() == 1, "ibm negative: one event");
	ev = test_mouse_event(0);
	ASSERT(ev->motion.mm_deltaX == -255, "ibm negative: dx");
	ASSERT(ev->motion.mm_deltaY == 0, "ibm negative: dy");
}

static void
test_ibm_buttons(void)
{
	u_char buf[MOUSEBUFSIZE] = {0x00, 0x00, 0x00, 0x00, 0x00};

	test_mouse_reset(0x7, 0);
	mouse_packet_ibm_ps2_mouse(buf);
	ASSERT(test_mouse_events() == 3, "ibm buttons: three events");
	ASSERT(test_mouse_event(0)->which == MOUSE_LEFT,
	       "ibm buttons: left");
	ASSERT(test_mouse_event(0)->direction == MOUSE_UP,
	       "ibm buttons: left direction");
	ASSERT(test_mouse_event(1)->which == MOUSE_RIGHT,
	       "ibm buttons: right");
	ASSERT(test_mouse_event(1)->direction == MOUSE_UP,
	       "ibm buttons: right direction");
	ASSERT(test_mouse_event(2)->which == MOUSE_MIDDLE,
	       "ibm buttons: middle");
	ASSERT(test_mouse_event(2)->direction == MOUSE_UP,
	       "ibm buttons: middle direction");
}

int
main(int argc, char *argv[], int envc, char *envp[])
{
	test_system_motion();
	test_system_buttons();
	test_microsoft_motion();
	test_microsoft_buttons();
	test_ibm_motion();
	test_ibm_buttons();
	return 0;
}
