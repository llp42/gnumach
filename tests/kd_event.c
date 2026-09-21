/* SPDX-License-Identifier: CMU-Mach
 * Derived from i386/i386at/kd_event.c:
 *   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
 *   Copyright Ing. C. Olivetti & C. S.p.A. 1989.
 *   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The `X_kdb` interpreter of the old i386/i386at/kd_event.c, for the
 * test programs' own link; the kernel's definitions are Rust now.  The
 * interpreter is unchanged, but `kdb_in_out' records the operation
 * instead of touching a port.
 */

#include <string.h>

#include "kd_event.h"

static unsigned int x_kdb_enter_str[KDB_STR_MAX];
static unsigned int x_kdb_exit_str[KDB_STR_MAX];
static unsigned int x_kdb_enter_len;
static unsigned int x_kdb_exit_len;

#define MAX_OPS	16
static struct test_kdb_io ops[MAX_OPS];
static int nops;

static void
record(enum test_kdb_op op, unsigned int port, unsigned long value)
{
	if (nops < MAX_OPS) {
		ops[nops].op = op;
		ops[nops].port = port;
		ops[nops].value = value;
	}
	nops++;
}

void
test_kdb_reset(void)
{
	nops = 0;
}

int
test_kdb_ops(void)
{
	return nops;
}

const struct test_kdb_io *
test_kdb_op(int index)
{
	if (index < 0 || index >= nops)
		return 0;
	return &ops[index];
}

static void
kdb_in_out(const unsigned int *p)
{
	unsigned int t = p[0];

	switch (t & K_X_TYPE) {
		case K_X_IN|K_X_BYTE:
			record(TEST_KDB_INB, t & K_X_PORT, 0);
			break;

		case K_X_IN|K_X_WORD:
			record(TEST_KDB_INW, t & K_X_PORT, 0);
			break;

		case K_X_IN|K_X_LONG:
			record(TEST_KDB_INL, t & K_X_PORT, 0);
			break;

		case K_X_OUT|K_X_BYTE:
			record(TEST_KDB_OUTB, t & K_X_PORT,
			       (unsigned char)p[1]);
			break;

		case K_X_OUT|K_X_WORD:
			record(TEST_KDB_OUTW, t & K_X_PORT,
			       (unsigned short)p[1]);
			break;

		case K_X_OUT|K_X_LONG:
			record(TEST_KDB_OUTL, t & K_X_PORT, p[1]);
			break;
	}
}

void
X_kdb_enter(void)
{
	unsigned int *u_ip, *endp;

	for (u_ip = x_kdb_enter_str, endp = &x_kdb_enter_str[x_kdb_enter_len];
	     u_ip < endp;
	     u_ip += 2)
		kdb_in_out(u_ip);
}

void
X_kdb_exit(void)
{
	unsigned int *u_ip, *endp;

	for (u_ip = x_kdb_exit_str, endp = &x_kdb_exit_str[x_kdb_exit_len];
	     u_ip < endp;
	     u_ip += 2)
		kdb_in_out(u_ip);
}

int
X_kdb_enter_init(
    unsigned int *data,
    unsigned int count)
{
    if (count > KDB_STR_MAX)
	return D_INVALID_OPERATION;

    memcpy(x_kdb_enter_str, data, count * sizeof x_kdb_enter_str[0]);
    x_kdb_enter_len = count;
    return 0;
}

int
X_kdb_exit_init(
    unsigned int *data,
    unsigned int count)
{
    if (count > KDB_STR_MAX)
	return D_INVALID_OPERATION;

    memcpy(x_kdb_exit_str, data, count * sizeof x_kdb_exit_str[0]);
    x_kdb_exit_len = count;
    return 0;
}
