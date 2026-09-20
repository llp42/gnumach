/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The mouse packet decoders of the old i386/i386at/kd_mouse.c, for the
 * test programs' own link; the kernel's definitions are Rust now.  The
 * decoders are unchanged, but `mouse_moved'/`mouse_button' record into a
 * table instead of enqueueing events.
 */

#include <kern/printf.h>

#include "kd_mouse.h"

static u_char lastbuttons;
static int middlegitech;
static int mouse_packets;

#define MAX_EVENTS	8
static struct test_mouse_event events[MAX_EVENTS];
static int nevents;

static void
record(const struct test_mouse_event *ev)
{
	if (nevents < MAX_EVENTS)
		events[nevents] = *ev;
	nevents++;
}

void
mouse_moved(struct mouse_motion where)
{
	struct test_mouse_event ev;

	ev.kind = TEST_MOUSE_MOTION;
	ev.motion = where;
	ev.which = 0;
	ev.direction = 0;
	record(&ev);
}

void
mouse_button(kev_type which, u_char direction)
{
	struct test_mouse_event ev;

	ev.kind = TEST_MOUSE_BUTTON;
	ev.motion.mm_deltaX = 0;
	ev.motion.mm_deltaY = 0;
	ev.which = which;
	ev.direction = direction;
	record(&ev);
}

void
test_mouse_reset(u_char buttons, int middle)
{
	lastbuttons = buttons;
	middlegitech = middle;
	nevents = 0;
}

int
test_mouse_events(void)
{
	return nevents;
}

const struct test_mouse_event *
test_mouse_event(int index)
{
	if (index < 0 || index >= nevents)
		return 0;
	return &events[index];
}

void
mouse_packet_mouse_system_mouse(u_char mousebuf[MOUSEBUFSIZE])
{
	u_char buttons, buttonchanges;
	struct mouse_motion moved;

	buttons = mousebuf[0] & 0x7;	/* get current state of buttons */
	buttonchanges = buttons ^ lastbuttons;
	moved.mm_deltaX = (char)mousebuf[1] + (char)mousebuf[3];
	moved.mm_deltaY = (char)mousebuf[2] + (char)mousebuf[4];

	if (moved.mm_deltaX != 0 || moved.mm_deltaY != 0)
		mouse_moved(moved);

	if (buttonchanges != 0) {
		lastbuttons = buttons;
		if (buttonchanges & 1)
			mouse_button(MOUSE_RIGHT, buttons & 1);
		if (buttonchanges & 2)
			mouse_button(MOUSE_MIDDLE, (buttons & 2) >> 1);
		if (buttonchanges & 4)
			mouse_button(MOUSE_LEFT, (buttons & 4) >> 2);
	}
}

/* same as above for microsoft mouse */
void
mouse_packet_microsoft_mouse(u_char mousebuf[MOUSEBUFSIZE])
{
	u_char buttons, buttonchanges;
	struct mouse_motion moved;

	buttons = ((mousebuf[0] & 0x30) >> 4);
	buttons |= middlegitech;
	buttons = (~buttons) & 0x07;	/* convert to not pressed */

	buttonchanges = buttons ^ lastbuttons;
	moved.mm_deltaX = ((mousebuf[0] & 0x03) << 6) | (mousebuf[1] & 0x3F);
	moved.mm_deltaY = ((mousebuf[0] & 0x0c) << 4) | (mousebuf[2] & 0x3F);
	if (moved.mm_deltaX & 0x80)	/* negative, in fact */
		moved.mm_deltaX = moved.mm_deltaX - 0x100;
	if (moved.mm_deltaY & 0x80)	/* negative, in fact */
		moved.mm_deltaY = moved.mm_deltaY - 0x100;
	/* and finally the Y orientation is different for the microsoft mouse */
	moved.mm_deltaY = -moved.mm_deltaY;

	if (moved.mm_deltaX != 0 || moved.mm_deltaY != 0)
		mouse_moved(moved);

	if (buttonchanges != 0) {
		lastbuttons = buttons;
		if (buttonchanges & 1)
			mouse_button(MOUSE_RIGHT, (buttons & 1) ?
						MOUSE_UP : MOUSE_DOWN);
		if (buttonchanges & 2)
			mouse_button(MOUSE_LEFT, (buttons & 2) ?
						MOUSE_UP : MOUSE_DOWN);
		if (buttonchanges & 4)
			mouse_button(MOUSE_MIDDLE, (buttons & 4) ?
						MOUSE_UP : MOUSE_DOWN);
	}
}

void
mouse_packet_ibm_ps2_mouse(u_char mousebuf[MOUSEBUFSIZE])
{
	u_char buttons, buttonchanges;
	struct mouse_motion moved;

	buttons = mousebuf[0] & 0x7;	/* get current state of buttons */
	buttonchanges = buttons ^ lastbuttons;
	moved.mm_deltaX =
		((mousebuf[0] & 0x10) ? 0xffffff00 : 0) | (u_char)mousebuf[1];
	moved.mm_deltaY =
		((mousebuf[0] & 0x20) ? 0xffffff00 : 0) | (u_char)mousebuf[2];
	if (mouse_packets) {
		printf("(%x:%x:%x)", mousebuf[0], mousebuf[1], mousebuf[2]);
		return;
	}

	if (moved.mm_deltaX != 0 || moved.mm_deltaY != 0)
		mouse_moved(moved);

	if (buttonchanges != 0) {
		lastbuttons = buttons;
		if (buttonchanges & 1)
			mouse_button(MOUSE_LEFT,   !(buttons & 1));
		if (buttonchanges & 2)
			mouse_button(MOUSE_RIGHT,  !((buttons & 2) >> 1));
		if (buttonchanges & 4)
			mouse_button(MOUSE_MIDDLE, !((buttons & 4) >> 2));
	}
}
