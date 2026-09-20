/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Exercise the user-mode copy of the `X_kdb` interpreter in
 * tests/kd_event.c, pinning the behaviour of the kernel's Rust
 * implementation in rust/src/arch/i386/kd_event.rs.  The qemu suite
 * never opens /dev/kbd, so the interpreter is tested here instead.
 */

#include <testlib.h>

#include "kd_event.h"

static void
test_enter_out(void)
{
	unsigned int cmds[] = {
		K_X_OUT|K_X_BYTE|0x60, 0x42,
		K_X_OUT|K_X_WORD|0x40, 0x1234,
		K_X_OUT|K_X_LONG|0xcf8, 0x12345678,
	};

	test_kdb_reset();
	ASSERT(X_kdb_enter_init(cmds, 6) == 0, "enter init");
	X_kdb_enter();
	ASSERT(test_kdb_ops() == 3, "enter: three commands");

	ASSERT(test_kdb_op(0)->op == TEST_KDB_OUTB, "enter: outb");
	ASSERT(test_kdb_op(0)->port == 0x60, "enter: outb port");
	ASSERT(test_kdb_op(0)->value == 0x42, "enter: outb value");

	ASSERT(test_kdb_op(1)->op == TEST_KDB_OUTW, "enter: outw");
	ASSERT(test_kdb_op(1)->port == 0x40, "enter: outw port");
	ASSERT(test_kdb_op(1)->value == 0x1234, "enter: outw value");

	ASSERT(test_kdb_op(2)->op == TEST_KDB_OUTL, "enter: outl");
	ASSERT(test_kdb_op(2)->port == 0xcf8, "enter: outl port");
	ASSERT(test_kdb_op(2)->value == 0x12345678, "enter: outl value");
}

static void
test_exit_in(void)
{
	unsigned int cmds[] = {
		K_X_IN|K_X_BYTE|0x64, 0,
		K_X_IN|K_X_WORD|0x64, 0,
		K_X_IN|K_X_LONG|0xcf8, 0,
	};

	test_kdb_reset();
	ASSERT(X_kdb_exit_init(cmds, 6) == 0, "exit init");
	X_kdb_exit();
	ASSERT(test_kdb_ops() == 3, "exit: three commands");

	ASSERT(test_kdb_op(0)->op == TEST_KDB_INB, "exit: inb");
	ASSERT(test_kdb_op(0)->port == 0x64, "exit: inb port");
	ASSERT(test_kdb_op(1)->op == TEST_KDB_INW, "exit: inw");
	ASSERT(test_kdb_op(1)->port == 0x64, "exit: inw port");
	ASSERT(test_kdb_op(2)->op == TEST_KDB_INL, "exit: inl");
	ASSERT(test_kdb_op(2)->port == 0xcf8, "exit: inl port");
}

static void
test_bounds(void)
{
	static unsigned int cmds[KDB_STR_MAX + 1];

	ASSERT(X_kdb_enter_init(cmds, KDB_STR_MAX) == 0,
	       "enter: full list accepted");
	ASSERT(X_kdb_exit_init(cmds, KDB_STR_MAX) == 0,
	       "exit: full list accepted");
	ASSERT(X_kdb_enter_init(cmds, KDB_STR_MAX + 1) ==
	       D_INVALID_OPERATION,
	       "enter: oversized list rejected");
	ASSERT(X_kdb_exit_init(cmds, KDB_STR_MAX + 1) ==
	       D_INVALID_OPERATION,
	       "exit: oversized list rejected");
}

int
main(int argc, char *argv[], int envc, char *envp[])
{
	test_enter_out();
	test_exit_in();
	test_bounds();
	return 0;
}
