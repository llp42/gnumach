/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The kd escape parser and modifier state machine, as the test
 * programs' copy.
 *
 * The kernel's driver lives in rust/src/arch/i386/kd/ now.  These are
 * user-mode binaries with their own link, so they carry their own C
 * copy of the parser and the modifier logic; the display operations
 * are recorded instead of drawing, so the tests can check them.
 */

#ifndef TEST_KD_H
#define TEST_KD_H

typedef unsigned char u_char;
typedef short csrpos_t;

/* <device/input.h>/<i386at/kd.h> values used by the parser. */
#define NUMOUTPUT	3

#define K_ESC		0x1b
#define K_HT		0x09
#define K_BS		0x08
#define K_LF		0x0a
#define K_CR		0x0d
#define K_BEL		0x07
#define K_SPACE		0x20

#define K_CTLSC		0x1d
#define K_LSHSC		0x2a
#define K_RSHSC		0x36
#define K_ALTSC		0x38
#define K_CLCKSC	0x3a
#define K_NLCKSC	0x45

#define KS_NORMAL	0x00
#define KS_NLKED	0x02
#define KS_CLKED	0x04
#define KS_ALTED	0x08
#define KS_SHIFTED	0x10
#define KS_CTLED	0x20

#define NORM_STATE	0
#define SHIFT_STATE	1
#define CTRL_STATE	2
#define ALT_STATE	3
#define SHIFT_ALT	4

#define ONE_SPACE	2
#define ONE_LINE	160
#define ONE_PAGE	4000
#define BOTTOM_LINE	3840

#define KAX_REVERSE	0x01
#define KAX_UNDERLINE	0x02
#define KAX_BLINK	0x04
#define KAX_BOLD	0x08
#define KAX_DIM	0x10
#define KAX_INVISIBLE	0x20
#define KA_NORMAL	0x07
#define KAX_COL_UNDERLINE 0x0f
#define KAX_COL_DIM	0x08

#define K_MAXESC	32

enum test_kd_op {
	TEST_KD_PUT,
	TEST_KD_SETPOS,
	TEST_KD_UP,
	TEST_KD_DOWN,
	TEST_KD_RIGHT,
	TEST_KD_LEFT,
	TEST_KD_CR,
	TEST_KD_HOME,
	TEST_KD_TAB,
	TEST_KD_CLS,
	TEST_KD_SCROLLUP,
	TEST_KD_SCROLLDN,
	TEST_KD_INSCH,
	TEST_KD_DELCH,
	TEST_KD_DELLN,
	TEST_KD_INSLN,
	TEST_KD_ERASE,
	TEST_KD_CLTOBCUR,
	TEST_KD_CLTOPCUR,
	TEST_KD_CLTOECUR,
	TEST_KD_CLFRBCUR,
	TEST_KD_ERASELN,
	TEST_KD_BELL,
};

struct test_kd_event {
	enum test_kd_op op;
	long arg;
};

void test_kd_reset(csrpos_t curpos);
int test_kd_ops(void);
const struct test_kd_event *test_kd_event(int index);
int test_kd_attr(void);

void kd_putc_esc(u_char c);
void kd_parseesc(void);
void kd_parserest(u_char *cp);
int do_modifier(int state, u_char c, int up);
unsigned int kdstate2idx(unsigned int state, int extended);

#endif /* TEST_KD_H */
