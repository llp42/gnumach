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
	feed("a\r\n\b");
	ASSERT(test_kd_ops() == 5, "plain: five events");
	ASSERT(test_kd_event(0)->op == TEST_KD_PUT, "plain: put");
	ASSERT(test_kd_event(0)->arg == 'a', "plain: put arg");
	ASSERT(test_kd_event(1)->op == TEST_KD_RIGHT, "plain: right");
	ASSERT(test_kd_event(2)->op == TEST_KD_CR, "plain: cr");
	ASSERT(test_kd_event(3)->op == TEST_KD_DOWN, "plain: down");
	ASSERT(test_kd_event(4)->op == TEST_KD_LEFT, "plain: left");

	test_kd_reset(0);
	feed("\x07");
	ASSERT(test_kd_ops() == 1, "plain: bell event");
	ASSERT(test_kd_event(0)->op == TEST_KD_BELL, "plain: bell");
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
test_cursor_more(void)
{
	test_kd_reset(0);
	feed("\x1b[2B");
	ASSERT(test_kd_ops() == 2, "cursor more: two downs");
	ASSERT(test_kd_event(1)->op == TEST_KD_DOWN, "cursor more: down");

	test_kd_reset(0);
	feed("\x1b[3C");
	ASSERT(test_kd_ops() == 3, "cursor more: three rights");
	ASSERT(test_kd_event(2)->op == TEST_KD_RIGHT, "cursor more: right");

	test_kd_reset(0);
	feed("\x1b[2E");
	ASSERT(test_kd_ops() == 3, "cursor more: cr and two downs");
	ASSERT(test_kd_event(0)->op == TEST_KD_CR, "cursor more: E cr");
	ASSERT(test_kd_event(1)->op == TEST_KD_DOWN, "cursor more: E down");

	test_kd_reset(0);
	feed("\x1b[2F");
	ASSERT(test_kd_ops() == 3, "cursor more: cr and two ups");
	ASSERT(test_kd_event(0)->op == TEST_KD_CR, "cursor more: F cr");
	ASSERT(test_kd_event(1)->op == TEST_KD_UP, "cursor more: F up");

	/* Zero counts run no command, as in the C `while (n--)`. */
	test_kd_reset(0);
	feed("\x1b[0A");
	ASSERT(test_kd_ops() == 0, "cursor more: zero count");
}

static void
test_position_more(void)
{
	/* 1-based column: "\e[4G" is column 4, i.e. 3 * 2. */
	test_kd_reset(0);
	feed("\x1b[4G");
	ASSERT(test_kd_ops() == 1, "position more: one setpos");
	ASSERT(test_kd_event(0)->arg == 6, "position more: column");

	/* Out-of-range row/column clamps to the lower right. */
	test_kd_reset(0);
	feed("\x1b[999;999H");
	ASSERT(test_kd_event(0)->arg == ONE_PAGE - ONE_SPACE,
	       "position more: clamped");

	/* Large counts repeat. */
	test_kd_reset(0);
	feed("\x1b[999A");
	ASSERT(test_kd_ops() == 999, "position more: 999 ups");
	ASSERT(test_kd_event(0)->op == TEST_KD_UP, "position more: first up");
	ASSERT(test_kd_event(63)->op == TEST_KD_UP, "position more: last up");
}

static void
test_unknown(void)
{
	/* `\e[?...` and `\e[<...` unsupported commands are dropped. */
	test_kd_reset(0);
	feed("\x1b[?25h");
	ASSERT(test_kd_ops() == 0, "unknown: question dropped");
	test_kd_reset(0);
	feed("\x1b[<1;2m");
	ASSERT(test_kd_ops() == 0, "unknown: angle dropped");
	/* A bare printable command byte is dropped too. */
	test_kd_reset(0);
	feed("\x1b[Z");
	ASSERT(test_kd_ops() == 0, "unknown: Z dropped");
	/* An incomplete sequence runs nothing until the byte arrives. */
	test_kd_reset(0);
	feed("\x1b[");
	ASSERT(test_kd_ops() == 0, "unknown: incomplete");
	feed("Z");
	ASSERT(test_kd_ops() == 0, "unknown: completed drop");
}

static void
test_long_sequence(void)
{
	/* Thirty digits after "\e[" fill the 32-byte sequence buffer;
	 * the byte that would write the terminator past it is dropped,
	 * and the next sequence parses normally. */
	test_kd_reset(0);
	feed("\x1b[111111111111111111111111111111");
	ASSERT(test_kd_ops() == 0, "long: filled buffer");
	feed("A");
	ASSERT(test_kd_ops() == 0, "long: byte dropped");
	feed("\x1b[1m");
	ASSERT(test_kd_attr() == (KA_NORMAL ^ 0x08), "long: parser recovers");
}

static void
test_tab_more(void)
{
	/* At column 0, a tab is eight spaces. */
	test_kd_reset(0);
	feed("\t");
	ASSERT(test_kd_ops() == 16, "tab more: eight spaces");
	ASSERT(test_kd_event(0)->op == TEST_KD_PUT, "tab more: put");
	ASSERT(test_kd_event(15)->op == TEST_KD_RIGHT, "tab more: right");

	/* At column 74 the tab advances six spaces. */
	test_kd_reset(148);
	feed("\t");
	ASSERT(test_kd_ops() == 12, "tab more: six spaces");
}

static void
test_incomplete(void)
{
	test_kd_reset(0);
	feed("\x1b[");
	ASSERT(test_kd_ops() == 0, "incomplete: open");
	feed("1");
	ASSERT(test_kd_ops() == 0, "incomplete: parameter");
	feed(";");
	ASSERT(test_kd_ops() == 0, "incomplete: semicolon");
	feed("5");
	ASSERT(test_kd_ops() == 0, "incomplete: parameter");
	feed("m");
	ASSERT(test_kd_ops() == 0, "incomplete: attribute");
	/* 1 sets bold, 5 blinks: 7 ^ 0x08 ^ 0x80. */
	ASSERT(test_kd_attr() == (KA_NORMAL ^ 0x08 ^ 0x80),
	       "incomplete: bold and blink");
}

static void
test_attr_more(void)
{
	/* Bold, then foreground yellow (3 -> color_table[3] = 6) and
	 * background blue (4 -> color_table[4] = 1); bold flips bit 3. */
	test_kd_reset(0);
	feed("\x1b[1;33;44m");
	ASSERT(test_kd_attr() == (0x16 ^ 0x08), "attr more: bold + colors");

	/* 22 clears bold and dim, leaving the color. */
	feed("\x1b[22m");
	ASSERT(test_kd_attr() == 0x16, "attr more: 22 clears bold");

	/* 4 underlines with the bright foreground. */
	feed("\x1b[4m");
	ASSERT(test_kd_attr() == ((0x16 & 0xf0) | KAX_COL_UNDERLINE),
	       "attr more: underline");

	/* 24 clears underline. */
	feed("\x1b[24m");
	ASSERT(test_kd_attr() == 0x16, "attr more: 24 clears underline");

	/* 39 clears underline and resets the foreground. */
	feed("\x1b[39m");
	ASSERT(test_kd_attr() == ((0x16 & 0xf0) | (KA_NORMAL & 0x0f)),
	       "attr more: 39 resets fg");

	/* An unknown number leaves the attributes alone. */
	feed("\x1b[99m");
	ASSERT(test_kd_attr() == ((0x16 & 0xf0) | (KA_NORMAL & 0x0f)),
	       "attr more: unknown no-op");
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
	test_cursor_more();
	test_position();
	test_position_more();
	test_clear();
	test_unknown();
	test_long_sequence();
	test_tab_more();
	test_incomplete();
	test_attr_more();
	test_edit();
	test_scroll();
	test_attr();
	test_modifier();
	test_state2idx();
	return 0;
}
