/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The kd device ABI as the tests see it: the ioctl flavors and the
 * user-visible event record.  <i386at/kd.h> and <device/input.h> are
 * kernel headers, so the tests mirror the values here.
 */

#ifndef TEST_KD_DEVICE_H
#define TEST_KD_DEVICE_H

/* <i386at/kd.h> ioctls. */
#define KDGSTATE	0x40046b03
#define KDGKBENT	0xc0056b01
#define KDSKBENT	0x80056b02
#define KDSETBELL	0x80046b04
#define KDGKBDTYPE	0x40044b02
#define KDSKBDMODE	0x80044b01
#define KB_VANILLAKB	0
#define KB_EVENT	1
#define KB_ASCII	2
#define KD_BELLON	1
#define KD_BELLOFF	0

/* <device/device_types.h>. */
#define DEV_GET_SIZE			0
#define DEV_GET_SIZE_DEVICE_SIZE	0
#define DEV_GET_SIZE_RECORD_SIZE	1
#define DEV_GET_SIZE_COUNT		2

/* <i386at/kd.h>: get/set a key map entry. */
struct kbentry {
	unsigned char kb_state;
	unsigned char kb_index;
	unsigned char kb_value[3];
};

/* <device/input.h> as user space sees it: the event record the kernel
 * copies out.  `rpc_time_value` is `time_value` for the user. */
struct test_kd_time {
	long seconds;
	int microseconds;
};
union test_kd_value {
	int up;
	unsigned char sc;
	struct {
		short mm_deltaX;
		short mm_deltaY;
	} mmotion;
};
struct test_kd_event {
	unsigned short type;
	struct test_kd_time unused_time;
	union test_kd_value value;
};

#define KEYBD_EVENT	5

#endif /* TEST_KD_DEVICE_H */
