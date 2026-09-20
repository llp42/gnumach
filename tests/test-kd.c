/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Exercise the user-mode copy of the kd escape parser and modifier
 * state machine in tests/kd.c, pinning the behaviour of the kernel's
 * Rust implementation in rust/src/arch/i386/kd/.  The qemu suite uses
 * console=com0, so the kd parser is tested here instead.
 */

#include <testlib.h>

#include "kd.h"

static void
feed(const char *s)
{
	while (*s != '\0')
		kd_putc_esc((u_char)*s++);
}

static void
test_plain(void)
{
	test_kd_reset(0);
	feed("a\r\n\b\t\x07");
	ASSERT(test_kd_ops() == 7, "plain: seven events");
	ASSERT(test_kd_event(0)->op == TEST_KD_PUT, "plain: put");
	ASSERT(test_kd_event(0)->arg == 'a', "plain: put arg");
	ASSERT(test_kd_event(1)->op == TEST_KD_RIGHT, "plain: right");
	ASSERT(test_kd_event(2)->op == TEST_KD_CR, "plain: cr");
	ASSERT(test_kd_event(3)->op == TEST_KD_DOWN, "plain: down");
	ASSERT(test_kd_event(4)->op == TEST_KD_LEFT, "plain: left");
	ASSERT(test_kd_event(5)->op == TEST_KD_TAB, "plain: tab");
	ASSERT(test_kd_event(6)->op == TEST_KD_BELL, "plain: bell");
}

static void
test_cursor(void)
{
	test_kd_reset(0);
	feed("\x1b[2A");
	ASSERT(test_kd_ops() == 2, "cursor: two ups");
	ASSERT(test_kd_event(0)->op == TEST_KD_UP, "cursor: up 1");
	ASSERT(test_kd_event(1)->op == TEST_KD_UP, "cursor: up 2");

	test_kd_reset(0);
	feed("\x1b[3D");
	ASSERT(test_kd_ops() == 3, "cursor: three lefts");
	ASSERT(test_kd_event(2)->op == TEST_KD_LEFT, "cursor: left 3");

	test_kd_reset(0);
	feed("\x1b[B");
	ASSERT(test_kd_ops() == 1, "cursor: one down");
	ASSERT(test_kd_event(0)->op == TEST_KD_DOWN, "cursor: down");
}

static void
test_position(void)
{
	test_kd_reset(0);
	feed("\x1b[H");
	ASSERT(test_kd_ops() == 1, "position: home");
	ASSERT(test_kd_event(0)->op == TEST_KD_HOME, "position: home op");

	/* Row 10, column 5: (10-1)*160 + (5-1)*2. */
	test_kd_reset(0);
	feed("\x1b[10;5H");
	ASSERT(test_kd_ops() == 1, "position: one setpos");
	ASSERT(test_kd_event(0)->op == TEST_KD_SETPOS, "position: setpos");
	ASSERT(test_kd_event(0)->arg == 1448, "position: position");

	/* Column from 1: "\e[5G" lands at 4 * 2. */
	test_kd_reset(0);
	feed("\x1b[5G");
	ASSERT(test_kd_event(0)->arg == 8, "position: column");

	/* Not enough info yet: the command arrives in two writes. */
	test_kd_reset(0);
	feed("\x1b[");
	ASSERT(test_kd_ops() == 0, "position: incomplete");
	feed("A");
	ASSERT(test_kd_ops() == 1, "position: completed");
	ASSERT(test_kd_event(0)->op == TEST_KD_UP, "position: up");
}

static void
test_clear(void)
{
	test_kd_reset(0);
	feed("\x1b[J");
	ASSERT(test_kd_event(0)->op == TEST_KD_CLTOBCUR, "clear: to bottom");
	test_kd_reset(0);
	feed("\x1b[1J");
	ASSERT(test_kd_event(0)->op == TEST_KD_CLTOPCUR, "clear: to top");
	test_kd_reset(0);
	feed("\x1b[2J");
	ASSERT(test_kd_event(0)->op == TEST_KD_CLS, "clear: screen");
	test_kd_reset(0);
	feed("\x1b[K");
	ASSERT(test_kd_event(0)->op == TEST_KD_CLTOECUR, "clear: to eoln");
	test_kd_reset(0);
	feed("\x1b[1K");
	ASSERT(test_kd_event(0)->op == TEST_KD_CLFRBCUR, "clear: from bol");
	test_kd_reset(0);
	feed("\x1b[2K");
	ASSERT(test_kd_event(0)->op == TEST_KD_ERASELN, "clear: line");
}

static void
test_edit(void)
{
	test_kd_reset(0);
	feed("\x1b[3L");
	ASSERT(test_kd_event(0)->op == TEST_KD_INSLN, "edit: insln");
	ASSERT(test_kd_event(0)->arg == 3, "edit: insln arg");
	test_kd_reset(0);
	feed("\x1b[M");
	ASSERT(test_kd_event(0)->op == TEST_KD_DELLN, "edit: delln");
	ASSERT(test_kd_event(0)->arg == 1, "edit: delln arg");
	test_kd_reset(0);
	feed("\x1b[2P");
	ASSERT(test_kd_event(0)->op == TEST_KD_DELCH, "edit: delch");
	ASSERT(test_kd_event(0)->arg == 2, "edit: delch arg");
	test_kd_reset(0);
	feed("\x1b[4X");
	ASSERT(test_kd_event(0)->op == TEST_KD_ERASE, "edit: erase");
	ASSERT(test_kd_event(0)->arg == 4, "edit: erase arg");
	test_kd_reset(0);
	feed("\x1b[2@");
	ASSERT(test_kd_event(0)->op == TEST_KD_INSCH, "edit: insch");
	ASSERT(test_kd_event(0)->arg == 2, "edit: insch arg");
}

static void
test_scroll(void)
{
	test_kd_reset(0);
	feed("\x1b[2S");
	ASSERT(test_kd_ops() == 2, "scroll: two scrollups");
	ASSERT(test_kd_event(1)->op == TEST_KD_SCROLLUP, "scroll: scrollup");
	test_kd_reset(0);
	feed("\x1b[2T");
	ASSERT(test_kd_ops() == 2, "scroll: two scrolldns");
	ASSERT(test_kd_event(1)->op == TEST_KD_SCROLLDN, "scroll: scrolldn");
}

static void
test_attr(void)
{
	test_kd_reset(0);
	feed("\x1b[0m");
	ASSERT(test_kd_attr() == KA_NORMAL, "attr: normal");
	feed("\x1b[31m");
	ASSERT(test_kd_attr() == 4, "attr: red");
	feed("\x1b[0m");
	feed("\x1b[1m");
	ASSERT(test_kd_attr() == (KA_NORMAL ^ 0x08), "attr: bold");
	feed("\x1b[0m");
	feed("\x1b[7m");
	ASSERT(test_kd_attr() == 0x70, "attr: reverse");
	feed("\x1b[0m");
	feed("\x1b[2m");
	ASSERT(test_kd_attr() == 0x08, "attr: dim");
}

static void
test_modifier(void)
{
	int st = KS_NORMAL;

	st = do_modifier(st, K_LSHSC, 0);
	ASSERT(st == KS_SHIFTED, "modifier: shift down");
	st = do_modifier(st, K_LSHSC, 1);
	ASSERT(st == KS_NORMAL, "modifier: shift up");

	st = do_modifier(st, K_CTLSC, 0);
	ASSERT(st == KS_CTLED, "modifier: ctrl down");
	st = do_modifier(st, K_CTLSC, 1);
	ASSERT(st == KS_NORMAL, "modifier: ctrl up");

	st = do_modifier(st, K_ALTSC, 0);
	ASSERT(st == KS_ALTED, "modifier: alt down");
	st = do_modifier(st, K_ALTSC, 1);
	ASSERT(st == KS_NORMAL, "modifier: alt up");

	st = do_modifier(st, K_NLCKSC, 0);
	ASSERT(st == KS_NLKED, "modifier: numlock on");
	st = do_modifier(st, K_NLCKSC, 0);
	ASSERT(st == KS_NORMAL, "modifier: numlock off");

	st = do_modifier(st, K_CLCKSC, 0);
	ASSERT(st == KS_CTLED, "modifier: caps acts as ctrl");

	st = do_modifier(st, 0x10, 0);
	ASSERT(st == KS_CTLED, "modifier: other keys leave state");
}

static void
test_state2idx(void)
{
	ASSERT(kdstate2idx(KS_NORMAL, 0) == NORM_STATE * NUMOUTPUT,
	       "state2idx: normal");
	ASSERT(kdstate2idx(KS_SHIFTED, 0) == SHIFT_STATE * NUMOUTPUT,
	       "state2idx: shift");
	ASSERT(kdstate2idx(KS_CTLED | KS_SHIFTED, 0) == CTRL_STATE * NUMOUTPUT,
	       "state2idx: ctrl beats shift");
	ASSERT(kdstate2idx(KS_ALTED, 0) == ALT_STATE * NUMOUTPUT,
	       "state2idx: alt");
	ASSERT(kdstate2idx(KS_SHIFTED | KS_ALTED, 0) == SHIFT_ALT * NUMOUTPUT,
	       "state2idx: shift-alt");
	ASSERT(kdstate2idx(KS_SHIFTED, 1) == NORM_STATE * NUMOUTPUT,
	       "state2idx: extended is normal");
}

int
main(int argc, char *argv[], int envc, char *envp[])
{
	test_plain();
	test_cursor();
	test_position();
	test_clear();
	test_edit();
	test_scroll();
	test_attr();
	test_modifier();
	test_state2idx();
	return 0;
}
