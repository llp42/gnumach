/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Read keyboard events from /dev/kbd in event mode while the test
 * runner injects keystrokes through the qemu monitor.  This is the
 * only way the suite reaches kdintr() and the scan-code queue.
 *
 * The test prints KD_READY, then blocks in device_read; the runner
 * waits for the marker and sends "shift-a", producing the four
 * scancodes below (shift down, a down, a up, shift up).
 */

#include <syscalls.h>
#include <testlib.h>

#include <mach/machine/vm_param.h>
#include <mach/std_types.h>
#include <mach/mach_types.h>

#include <device.user.h>
#include <mach.user.h>
#include <mach_port.user.h>

#include "kd_device.h"

static void
test_kd_intr(void)
{
	mach_port_t kbd;
	dev_status_data_t data;
	struct test_kd_event events[8];
	int have = 0;
	kern_return_t err;

	err = device_open(device_priv(), D_READ | D_WRITE, "kbd", &kbd);
	ASSERT_RET(err, "device_open kbd");

	data[0] = KB_EVENT;
	err = device_set_status(kbd, KDSKBDMODE, data, 1);
	ASSERT_RET(err, "kbd event mode");

	printf("KD_READY\n");

	/* The four scancodes arrive as the injected key is pressed and
	 * released, possibly in several batches; read until they are in. */
	while (have < 4) {
		io_buf_ptr_t buf = 0;
		mach_msg_type_number_t cnt = 0;
		int got;

		err = device_read(kbd, 0, 0,
				  (int)(sizeof events -
					have * sizeof(events[0])),
				  &buf, &cnt);
		ASSERT_RET(err, "device_read kbd");
		ASSERT(buf != 0, "device_read no data");
		got = (int)(cnt / sizeof(events[0]));
		ASSERT(got > 0, "kd intr: no events");
		memcpy(&events[have], buf, got * sizeof(events[0]));
		have += got;
	}

	ASSERT(events[0].type == KEYBD_EVENT, "kd intr: first is an event");
	ASSERT(events[0].value.sc == 0x2a, "kd intr: shift down");
	ASSERT(events[1].type == KEYBD_EVENT, "kd intr: second is an event");
	ASSERT(events[1].value.sc == 0x1e, "kd intr: a down");
	ASSERT(events[2].type == KEYBD_EVENT, "kd intr: third is an event");
	ASSERT(events[2].value.sc == 0x9e, "kd intr: a up");
	ASSERT(events[3].type == KEYBD_EVENT, "kd intr: fourth is an event");
	ASSERT(events[3].value.sc == 0xaa, "kd intr: shift up");

	data[0] = KB_ASCII;
	err = device_set_status(kbd, KDSKBDMODE, data, 1);
	ASSERT_RET(err, "kbd ascii mode");

	err = device_close(kbd);
	ASSERT_RET(err, "device_close kbd");
}

int
main(int argc, char *argv[], int envc, char *envp[])
{
	test_kd_intr();
	return 0;
}
