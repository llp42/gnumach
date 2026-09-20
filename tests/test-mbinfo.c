/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Read the multiboot information block through /dev/mbinfo.  This is
 * the only way the suite reaches mbinforead() and the registration the
 * boot path does in model_dep.c; the size below pins the raw block.
 */

#include <syscalls.h>
#include <testlib.h>

#include <mach/machine/vm_param.h>
#include <mach/std_types.h>
#include <mach/mach_types.h>

#include <device.user.h>
#include <mach.user.h>
#include <mach_port.user.h>

/* sizeof(struct multiboot_raw_info), mirrored in Rust. */
#define MBINFO_SIZE		116
#define MULTIBOOT_LOADER_MEMORY	0x01

int
main(int argc, char *argv[], int envc, char *envp[])
{
	mach_port_t dev;
	kern_return_t err;
	io_buf_ptr_t buf = 0;
	mach_msg_type_number_t cnt = 0;
	unsigned char info[MBINFO_SIZE];
	uint32_t flags, mem_upper;

	err = device_open(device_priv(), D_READ, "mbinfo", &dev);
	ASSERT_RET(err, "device_open mbinfo");

	err = device_read(dev, 0, 0, MBINFO_SIZE, &buf, &cnt);
	ASSERT_RET(err, "device_read mbinfo");
	ASSERT(cnt == MBINFO_SIZE, "mbinfo: one raw block");
	ASSERT(buf != 0, "mbinfo: no data");
	memcpy(info, buf, MBINFO_SIZE);

	/* The loader always reports memory; parse the packed prefix. */
	memcpy(&flags, info, sizeof flags);
	memcpy(&mem_upper, info + 8, sizeof mem_upper);
	ASSERT(flags & MULTIBOOT_LOADER_MEMORY, "mbinfo: memory flag");
	ASSERT(mem_upper > 0, "mbinfo: mem_upper set");

	/* A read larger than the raw block is refused. */
	err = device_read(dev, 0, 0, MBINFO_SIZE + 1, &buf, &cnt);
	ASSERT(err != KERN_SUCCESS, "mbinfo: oversized read refused");

	err = device_close(dev);
	ASSERT_RET(err, "device_close mbinfo");

	return 0;
}
