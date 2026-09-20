/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The `X_kdb` port-command interpreter, as the test programs' copy.
 *
 * The kernel's driver lives in rust/src/arch/i386/kd_event.rs now.
 * These are user-mode binaries with their own link, so they carry their
 * own C copy of the interpreter; the port I/O is recorded instead of
 * executed, so the tests can check the command stream.
 *
 * The original i386/i386at/kd_event.c checked the command count with
 * `count * sizeof > sizeof`, which wraps for a large `count`; the copy
 * keeps the same plain bound the Rust implementation uses, `count >
 * 512`.
 */

#ifndef TEST_KD_EVENT_H
#define TEST_KD_EVENT_H

/* <i386at/kd.h> command bits. */
#define K_X_IN		0x01000000
#define K_X_OUT		0x02000000
#define K_X_BYTE	0x00010000
#define K_X_WORD	0x00020000
#define K_X_LONG	0x00040000
#define K_X_TYPE	0x03070000
#define K_X_PORT	0x0000ffff

/* <device/device_types.h>. */
#define D_INVALID_OPERATION	2505

#define KDB_STR_MAX	512

enum test_kdb_op {
	TEST_KDB_INB,
	TEST_KDB_INW,
	TEST_KDB_INL,
	TEST_KDB_OUTB,
	TEST_KDB_OUTW,
	TEST_KDB_OUTL,
};

struct test_kdb_io {
	enum test_kdb_op op;
	unsigned int port;
	unsigned long value;
};

void test_kdb_reset(void);
int test_kdb_ops(void);
const struct test_kdb_io *test_kdb_op(int index);

int X_kdb_enter_init(unsigned int *data, unsigned int count);
int X_kdb_exit_init(unsigned int *data, unsigned int count);
void X_kdb_enter(void);
void X_kdb_exit(void);

#endif /* TEST_KD_EVENT_H */
