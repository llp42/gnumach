/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The kd escape parser and modifier state machine of the old
 * i386/i386at/kd.c, for the test programs' own link; the kernel's
 * definitions are Rust now.  The parser is unchanged, but the display
 * operations record into a table instead of drawing.
 */

#include <util/atoi.h>

#include "kd.h"

static const unsigned char color_table[] = {
	0, 4, 2, 6, 1, 5, 3, 7, 8, 12, 10, 14, 9, 13, 11, 15
};

static u_char esc_seq[K_MAXESC];
static int esc_spt;
static csrpos_t kd_curpos;
static u_char kd_attr;
static u_char kd_attrflags;
static u_char kd_color;

#define MAX_EVENTS 64
static struct test_kd_event events[MAX_EVENTS];
static int nevents;

static void
record(enum test_kd_op op, long arg)
{
	if (nevents < MAX_EVENTS) {
		events[nevents].op = op;
		events[nevents].arg = arg;
	}
	nevents++;
}

void
test_kd_reset(csrpos_t curpos)
{
	kd_curpos = curpos;
	kd_attr = KA_NORMAL;
	kd_attrflags = 0;
	kd_color = KA_NORMAL;
	esc_spt = 0;
	nevents = 0;
}

int
test_kd_ops(void)
{
	return nevents;
}

const struct test_kd_event *
test_kd_event(int index)
{
	if (index < 0 || index >= nevents)
		return 0;
	return &events[index];
}

int
test_kd_attr(void)
{
	return kd_attr;
}

#define CHARIDX(sidx) ((sidx) * NUMOUTPUT)
#define BEG_OF_LINE(pos) ((pos) - (pos) % ONE_LINE)
#define CURRENT_COLUMN(pos) (((pos) % ONE_LINE) / ONE_SPACE)

/* Display operations, recorded. */

static void
kd_setpos(csrpos_t newpos)
{
	if (newpos > ONE_PAGE)
		newpos = BOTTOM_LINE;
	if (newpos < 0)
		newpos = 0;
	kd_curpos = newpos;
	record(TEST_KD_SETPOS, newpos);
}

static void kd_up(void) { record(TEST_KD_UP, 0); }
static void kd_down(void) { record(TEST_KD_DOWN, 0); }
static void kd_right(void) { record(TEST_KD_RIGHT, 0); }
static void kd_left(void) { record(TEST_KD_LEFT, 0); }
static void kd_cr(void) { record(TEST_KD_CR, 0); }
static void kd_home(void) { record(TEST_KD_HOME, 0); }
static void kd_tab(void) { record(TEST_KD_TAB, 0); }
static void kd_cls(void) { record(TEST_KD_CLS, 0); }
static void kd_scrollup(void) { record(TEST_KD_SCROLLUP, 0); }
static void kd_scrolldn(void) { record(TEST_KD_SCROLLDN, 0); }
static void kd_cltobcur(void) { record(TEST_KD_CLTOBCUR, 0); }
static void kd_cltopcur(void) { record(TEST_KD_CLTOPCUR, 0); }
static void kd_cltoecur(void) { record(TEST_KD_CLTOECUR, 0); }
static void kd_clfrbcur(void) { record(TEST_KD_CLFRBCUR, 0); }
static void kd_eraseln(void) { record(TEST_KD_ERASELN, 0); }

static void
kd_insch(int number)
{
	record(TEST_KD_INSCH, number);
}

static void
kd_delch(int number)
{
	record(TEST_KD_DELCH, number);
}

static void
kd_delln(int number)
{
	record(TEST_KD_DELLN, number);
}

static void
kd_insln(int number)
{
	record(TEST_KD_INSLN, number);
}

static void
kd_erase(int number)
{
	record(TEST_KD_ERASE, number);
}

/* The output primitive, with the control specials. */

static void
kd_putc(u_char ch)
{
	switch (ch) {
	case (K_LF):
		kd_down();
		break;
	case (K_CR):
		kd_cr();
		break;
	case (K_BS):
		kd_left();
		break;
	case (K_HT):
		kd_tab();
		break;
	case (K_BEL):
		record(TEST_KD_BELL, 0);
		break;
	default:
		record(TEST_KD_PUT, ch);
		kd_right();
		break;
	}
}

#define reverse_video_char(a) \
	(((a) & 0x88) | ((((a) >> 4) | ((a) << 4)) & 0x77))

static void
kd_update_kd_attr(void)
{
	kd_attr = kd_color;
	if (kd_attrflags & KAX_UNDERLINE)
		kd_attr = (kd_attr & 0xf0) | KAX_COL_UNDERLINE;
	else if (kd_attrflags & KAX_DIM)
		kd_attr = (kd_attr & 0xf0) | KAX_COL_DIM;
	if (kd_attrflags & KAX_REVERSE)
		kd_attr = reverse_video_char(kd_attr);
	if (kd_attrflags & KAX_BLINK)
		kd_attr ^= 0x80;
	if (kd_attrflags & KAX_BOLD)
		kd_attr ^= 0x08;
}

/* The escape parser, copied. */

void
kd_putc_esc(u_char c)
{
	if (c == (K_ESC)) {
		if (esc_spt == 0) {
			esc_seq[esc_spt++] = (K_ESC);
			esc_seq[esc_spt] = '\0';
		} else {
			kd_putc((K_ESC));
			esc_spt = 0;
		}
	} else {
		if (esc_spt) {
			if (esc_spt > K_MAXESC - 1)
				esc_spt = 0;
			else {
				esc_seq[esc_spt++] = c;
				esc_seq[esc_spt] = '\0';
				kd_parseesc();
			}
		} else {
			kd_putc(c);
		}
	}
}

void
kd_parseesc(void)
{
	u_char *escp;

	escp = esc_seq + 1;
	switch (*escp) {
	case 'c':
		kd_cls();
		kd_home();
		esc_spt = 0;
		break;
	case '[':
		escp++;
		kd_parserest(escp);
		break;
	case '\0':
		break;
	default:
		kd_putc(*escp);
		esc_spt = 0;
		break;
	}
}

static void repeat(int n, void (*f)(void))
{
	int i;

	for (i = 0; i < n; i++)
		f();
}

void
kd_parserest(u_char *cp)
{
	int number[16], npar = 0, i;
	csrpos_t newpos;
	int question = 0;
	int angle = 0;

	if (*cp == '?') {
		question = 1;
		cp++;
	} else if (*cp == '<') {
		angle = 1;
		cp++;
	}

	for (i = 0; i <= 15; i++)
		number[i] = MACH_ATOI_DEFAULT;

	do {
		cp += mach_atoi(cp, &number[npar]);
	} while (*cp == ';' && ++npar <= 15 && cp++);

	if (question || angle) {
		switch (*cp) {
		case '\0':
			break;
		default:
			if (*cp >= '@' && *cp <= '~') {
				/* drop */
			} else {
				kd_putc(*cp);
			}
			esc_spt = 0;
			break;
		}
		return;
	}

	switch (*cp) {
	case 'm':
		for (i = 0; i <= npar; i++)
			switch (number[i]) {
			case MACH_ATOI_DEFAULT:
			case 0:
				kd_attrflags = 0;
				kd_color = KA_NORMAL;
				break;
			case 1:
				kd_attrflags |= KAX_BOLD;
				kd_attrflags &= ~KAX_DIM;
				break;
			case 2:
				kd_attrflags |= KAX_DIM;
				kd_attrflags &= ~KAX_BOLD;
				break;
			case 4:
				kd_attrflags |= KAX_UNDERLINE;
				break;
			case 5:
				kd_attrflags |= KAX_BLINK;
				break;
			case 7:
				kd_attrflags |= KAX_REVERSE;
				break;
			case 8:
				kd_attrflags |= KAX_INVISIBLE;
				break;
			case 21:
			case 22:
				kd_attrflags &= ~(KAX_BOLD | KAX_DIM);
				break;
			case 24:
				kd_attrflags &= ~KAX_UNDERLINE;
				break;
			case 25:
				kd_attrflags &= ~KAX_BLINK;
				break;
			case 27:
				kd_attrflags &= ~KAX_REVERSE;
				break;
			case 38:
				kd_attrflags |= KAX_UNDERLINE;
				kd_color = (kd_color & 0xf0) | (KA_NORMAL & 0x0f);
				break;
			case 39:
				kd_attrflags &= ~KAX_UNDERLINE;
				kd_color = (kd_color & 0xf0) | (KA_NORMAL & 0x0f);
				break;
			default:
				if (number[i] >= 30 && number[i] <= 37) {
					kd_color = (kd_color & 0xf0) |
						color_table[(number[i] - 30)];
				} else if (number[i] >= 40 && number[i] <= 47) {
					kd_color = (kd_color & 0x0f) |
						(color_table[(number[i] - 40)] << 4);
				}
				break;
			}
		kd_update_kd_attr();
		esc_spt = 0;
		break;
	case '@':
		kd_insch(number[0] == MACH_ATOI_DEFAULT ? 1 : number[0]);
		esc_spt = 0;
		break;
	case 'A':
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_up();
		else
			repeat(number[0], kd_up);
		esc_spt = 0;
		break;
	case 'B':
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_down();
		else
			repeat(number[0], kd_down);
		esc_spt = 0;
		break;
	case 'C':
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_right();
		else
			repeat(number[0], kd_right);
		esc_spt = 0;
		break;
	case 'D':
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_left();
		else
			repeat(number[0], kd_left);
		esc_spt = 0;
		break;
	case 'E':
		kd_cr();
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_down();
		else
			repeat(number[0], kd_down);
		esc_spt = 0;
		break;
	case 'F':
		kd_cr();
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_up();
		else
			repeat(number[0], kd_up);
		esc_spt = 0;
		break;
	case 'G':
		if (number[0] == MACH_ATOI_DEFAULT)
			number[0] = 0;
		else if (number[0] > 0)
			--number[0];
		kd_setpos(BEG_OF_LINE(kd_curpos) + number[0] * ONE_SPACE);
		esc_spt = 0;
		break;
	case 'f':
	case 'H':
		if (number[0] == MACH_ATOI_DEFAULT &&
		    number[1] == MACH_ATOI_DEFAULT) {
			kd_home();
			esc_spt = 0;
			break;
		}
		if (number[0] == MACH_ATOI_DEFAULT)
			number[0] = 0;
		else if (number[0] > 0)
			--number[0];
		newpos = (number[0] * ONE_LINE);
		if (number[1] == MACH_ATOI_DEFAULT)
			number[1] = 0;
		else if (number[1] > 0)
			number[1]--;
		newpos += (number[1] * ONE_SPACE);
		if (newpos < 0)
			newpos = 0;
		if (newpos > ONE_PAGE)
			newpos = (ONE_PAGE - ONE_SPACE);
		kd_setpos(newpos);
		esc_spt = 0;
		break;
	case 'J':
		switch (number[0]) {
		case MACH_ATOI_DEFAULT:
		case 0:
			kd_cltobcur();
			break;
		case 1:
			kd_cltopcur();
			break;
		case 2:
			kd_cls();
			break;
		}
		esc_spt = 0;
		break;
	case 'K':
		switch (number[0]) {
		case MACH_ATOI_DEFAULT:
		case 0:
			kd_cltoecur();
			break;
		case 1:
			kd_clfrbcur();
			break;
		case 2:
			kd_eraseln();
			break;
		}
		esc_spt = 0;
		break;
	case 'L':
		kd_insln(number[0] == MACH_ATOI_DEFAULT ? 1 : number[0]);
		esc_spt = 0;
		break;
	case 'M':
		kd_delln(number[0] == MACH_ATOI_DEFAULT ? 1 : number[0]);
		esc_spt = 0;
		break;
	case 'P':
		kd_delch(number[0] == MACH_ATOI_DEFAULT ? 1 : number[0]);
		esc_spt = 0;
		break;
	case 'S':
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_scrollup();
		else
			repeat(number[0], kd_scrollup);
		esc_spt = 0;
		break;
	case 'T':
		if (number[0] == MACH_ATOI_DEFAULT)
			kd_scrolldn();
		else
			repeat(number[0], kd_scrolldn);
		esc_spt = 0;
		break;
	case 'X':
		kd_erase(number[0] == MACH_ATOI_DEFAULT ? 1 : number[0]);
		esc_spt = 0;
		break;
	case '\0':
		break;
	default:
		if (*cp >= '@' && *cp <= '~') {
			/* drop */
		} else {
			kd_putc(*cp);
		}
		esc_spt = 0;
		break;
	}
}

/* The modifier state machine, copied. */

int
do_modifier(int state, u_char c, int up)
{
	switch (c) {
	case (K_ALTSC):
		if (up)
			state &= ~KS_ALTED;
		else
			state |= KS_ALTED;
		break;
	case (K_CLCKSC):
	case (K_CTLSC):
		if (up)
			state &= ~KS_CTLED;
		else
			state |= KS_CTLED;
		break;
	case (K_NLCKSC):
		if (!up)
			state ^= KS_NLKED;
		break;
	case (K_LSHSC):
	case (K_RSHSC):
		if (up)
			state &= ~KS_SHIFTED;
		else
			state |= KS_SHIFTED;
		break;
	}
	return (state);
}

unsigned int
kdstate2idx(unsigned int state, int extended)
{
	int state_idx = NORM_STATE;

	if ((!extended) && state != KS_NORMAL) {
		if ((state & (KS_SHIFTED | KS_ALTED)) ==
		    (KS_SHIFTED | KS_ALTED))
			state_idx = SHIFT_ALT;
		else if (state & KS_CTLED)
			state_idx = CTRL_STATE;
		else if (state & KS_SHIFTED)
			state_idx = SHIFT_STATE;
		else if (state & KS_ALTED)
			state_idx = ALT_STATE;
	}
	return (CHARIDX(state_idx));
}
