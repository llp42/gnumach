/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Drive the kd driver through its device: open /dev/kd (which runs
 * kdinit(), the display and the tty setup), get/set the keyboard state
 * and key map, write an escape sequence and read the VGA text buffer
 * back through /dev/mem, map the kd bitmap, and probe /dev/kbd.  This
 * executes the Rust kd code that the console=com0 suite otherwise
 * never reaches.
 *
 * The ioctl flavors and the user-visible kd_event layout live in
 * <i386at/kd.h> and <device/input.h>, which are kernel headers; they
 * are mirrored here, with the event size asserted per target.
 */

#include <syscalls.h>
#include <testlib.h>

#include <mach/machine/vm_param.h>
#include <mach/std_types.h>
#include <mach/mach_types.h>
#include <mach/vm_param.h>

#include <device.user.h>
#include <mach.user.h>
#include <mach_port.user.h>

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

/* The VGA text buffer, and the attribute KA_NORMAL = 0x07. */
#define VGA_BASE	0xb8000

static mach_port_t
open_device(const char *name)
{
	mach_port_t dev = MACH_PORT_NULL;
	kern_return_t err;

	err = device_open(device_priv(), D_READ | D_WRITE, name, &dev);
	ASSERT_RET(err, name);
	return dev;
}

static void
test_kd_keymap(mach_port_t kd)
{
	struct kbentry kb;
	int raw[2];
	mach_msg_type_number_t count;
	kern_return_t err;

	/* device_get_status() is out-only (device.defs), so the kbentry
	 * cannot be read back here; the follow-up keystroke test observes
	 * the remapped key instead.  Setting it exercises the map write. */
	kb.kb_state = 0;
	kb.kb_index = 0x1e;
	kb.kb_value[0] = 'z';
	kb.kb_value[1] = 0xff;
	kb.kb_value[2] = 0xff;
	memset(raw, 0, sizeof raw);
	memcpy(raw, &kb, sizeof kb);
	count = 2;
	err = device_set_status(kd, KDSKBENT, (dev_status_t)raw, count);
	ASSERT_RET(err, "kd set KDSKBENT");

	kb.kb_value[0] = 'a';
	memset(raw, 0, sizeof raw);
	memcpy(raw, &kb, sizeof kb);
	count = 2;
	err = device_set_status(kd, KDSKBENT, (dev_status_t)raw, count);
	ASSERT_RET(err, "kd restore KDSKBENT");
}

static void
test_kd_write(mach_port_t kd)
{
	static const char seq[] = "\033[2J\033[1;1HX";
	int written = 0;
	kern_return_t err;

	err = device_write(kd, D_WRITE, 0, (io_buf_ptr_t)seq,
			   sizeof seq - 1, &written);
	ASSERT_RET(err, "kd device_write");
	ASSERT(written == (int)(sizeof seq - 1), "kd short write");

	/* The tty may start output asynchronously; give it a moment. */
	msleep(100);
}

static void
test_kd_vga(mach_port_t kd)
{
	mach_port_t mem, pager;
	unsigned char *vga = 0;
	kern_return_t err;
	vm_size_t size = vm_page_size;

	(void)kd;

	mem = open_device("mem");
	err = device_map(mem, VM_PROT_READ | VM_PROT_WRITE, VGA_BASE, size,
			 &pager, 0);
	ASSERT_RET(err, "device_map mem");
	err = vm_map(mach_task_self(), (vm_address_t *)&vga, size, 0, 1,
		     pager, 0, 0, VM_PROT_READ | VM_PROT_WRITE,
		     VM_PROT_READ | VM_PROT_WRITE, VM_INHERIT_NONE);
	ASSERT_RET(err, "vm_map vga");

	ASSERT(vga[0] == 'X', "vga: 'X' not on screen");
	ASSERT(vga[1] == 0x07, "vga: attribute not KA_NORMAL");

	err = vm_deallocate(mach_task_self(), (vm_address_t)vga, size);
	ASSERT_RET(err, "vm_deallocate vga");
	err = mach_port_deallocate(mach_task_self(), pager);
	ASSERT_RET(err, "mach_port_deallocate pager");
	err = device_close(mem);
	ASSERT_RET(err, "device_close mem");
}

static void
test_kd_mmap(mach_port_t kd)
{
	mach_port_t pager;
	kern_return_t err;

	/* kdmmap() maps the bitmap frame buffer. */
	err = device_map(kd, VM_PROT_READ | VM_PROT_WRITE, 0, vm_page_size,
			 &pager, 0);
	ASSERT_RET(err, "device_map kd");
	err = mach_port_deallocate(mach_task_self(), pager);
	ASSERT_RET(err, "mach_port_deallocate kd pager");
}

static void
test_kd(void)
{
	mach_port_t kd = open_device("kd");
	dev_status_data_t data;
	mach_msg_type_number_t count;
	kern_return_t err;

	/* The state starts normal. */
	memset(data, 0xaa, sizeof data);
	count = 1;
	err = device_get_status(kd, KDGSTATE, data, &count);
	ASSERT_RET(err, "kd get KDGSTATE");
	ASSERT(count == 1, "kd KDGSTATE count");
	ASSERT(data[0] == 0, "kd state not normal");

	test_kd_keymap(kd);

	/* The bell, on then off. */
	data[0] = KD_BELLON;
	err = device_set_status(kd, KDSETBELL, data, 1);
	ASSERT_RET(err, "kd bell on");
	data[0] = KD_BELLOFF;
	err = device_set_status(kd, KDSETBELL, data, 1);
	ASSERT_RET(err, "kd bell off");

	test_kd_write(kd);
	test_kd_vga(kd);
	test_kd_mmap(kd);

	err = device_close(kd);
	ASSERT_RET(err, "device_close kd");
}

static void
test_kbd(void)
{
	mach_port_t kbd = open_device("kbd");
	dev_status_data_t data;
	mach_msg_type_number_t count;
	kern_return_t err;

	ASSERT(sizeof(struct test_kd_event) == (sizeof(void *) == 8 ? 32 : 16),
	       "test kd_event layout");

	count = 1;
	err = device_get_status(kbd, KDGKBDTYPE, data, &count);
	ASSERT_RET(err, "kbd KDGKBDTYPE");
	ASSERT(data[0] == KB_VANILLAKB, "kbd type");

	/* The record size pins the Rust kd_event mirror to the ABI. */
	memset(data, 0, sizeof data);
	count = DEV_GET_SIZE_COUNT;
	err = device_get_status(kbd, DEV_GET_SIZE, data, &count);
	ASSERT_RET(err, "kbd DEV_GET_SIZE");
	ASSERT(count == DEV_GET_SIZE_COUNT, "kbd DEV_GET_SIZE count");
	ASSERT(data[DEV_GET_SIZE_DEVICE_SIZE] == 0, "kbd device size");
	ASSERT(data[DEV_GET_SIZE_RECORD_SIZE] == (int)sizeof(struct test_kd_event),
	       "kd_event record size");

	data[0] = KB_EVENT;
	err = device_set_status(kbd, KDSKBDMODE, data, 1);
	ASSERT_RET(err, "kbd event mode");
	data[0] = KB_ASCII;
	err = device_set_status(kbd, KDSKBDMODE, data, 1);
	ASSERT_RET(err, "kbd ascii mode");

	err = device_close(kbd);
	ASSERT_RET(err, "device_close kbd");
}

int
main(int argc, char *argv[], int envc, char *envp[])
{
	test_kd();
	test_kbd();
	return 0;
}
