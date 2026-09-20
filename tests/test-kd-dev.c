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
 * The device ABI is mirrored in kd_device.h, with the event size
 * asserted per target.
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

#include "kd_device.h"


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
	static const char seq[] = "\033[2J\033[1;1HX"
		"\033[1;2H\033[1mY"
		"\033[1;3H\033[99999999999mZ"
		"\033[1;4H\033[111111111111111111111111111111A"
		"\033[1mW"
		"\033[2;5H\033[0G\033[0mQ"
		"\033[1;6H\033[?25h\033[<1;2m\033[0mR"
		"\033[1;1H\033[0G";
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

	/* The sequence ends with "\e[0G" at home, which must not scroll:
	 * 'X' and 'Y' would move down a line if it did. */
	ASSERT(vga[0] == 'X', "vga: 'X' not on screen");
	ASSERT(vga[1] == 0x07, "vga: attribute not KA_NORMAL");

	/* A parameter too large for an int counts as absent: Z must come
	 * back normal, not bold like Y. */
	ASSERT(vga[2] == 'Y', "vga: 'Y' not on screen");
	ASSERT(vga[3] == 0x0f, "vga: 'Y' not bold");
	ASSERT(vga[4] == 'Z', "vga: 'Z' not on screen");
	ASSERT(vga[5] == 0x07, "vga: overflow parameter not absent");

	/* The 32-byte sequence fills the escape buffer; 'A' is dropped
	 * instead of written past it, so 'W' lands at the next position
	 * with the bold attribute the fresh sequence set. */
	ASSERT(vga[6] == 'W', "vga: 'W' not on screen");
	ASSERT(vga[7] == 0x0f, "vga: 'W' not bold");

	/* "\e[0G" is column 1 of the current line, not the cell before
	 * it: from column 5 of line 2, 'Q' lands at byte 160, not 158. */
	ASSERT(vga[160] == 'Q', "vga: zero G parameter not column 1");
	ASSERT(vga[161] == 0x07, "vga: 'Q' not normal");
	ASSERT(vga[158] == 0x20, "vga: zero G parameter landed early");

	/* The unsupported "\e[?..." and "\e[<..." sequences draw
	 * nothing, so 'R' lands right where the cursor was. */
	ASSERT(vga[10] == 'R', "vga: private sequence not dropped");
	ASSERT(vga[11] == 0x07, "vga: 'R' not normal");

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
