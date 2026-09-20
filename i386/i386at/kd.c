/*
 * Mach Operating System
 * Copyright (c) 1991,1990,1989 Carnegie Mellon University
 * All Rights Reserved.
 *
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 *
 * CARNEGIE MELLON ALLOWS FREE USE OF THIS SOFTWARE IN ITS "AS IS"
 * CONDITION.  CARNEGIE MELLON DISCLAIMS ANY LIABILITY OF ANY KIND FOR
 * ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF THIS SOFTWARE.
 *
 * Carnegie Mellon requests users of this software to return to
 *
 *  Software Distribution Coordinator  or  Software.Distribution@CS.CMU.EDU
 *  School of Computer Science
 *  Carnegie Mellon University
 *  Pittsburgh PA 15213-3890
 *
 * any improvements or extensions that they make and grant Carnegie Mellon
 * the rights to redistribute these changes.
 */
/*
 *	Olivetti Mach Console driver v0.0
 *	Copyright Ing. C. Olivetti & C. S.p.A. 1988, 1989
 *	All rights reserved.
 *
 */
/*
  Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.,
Cupertino, California.

		All Rights Reserved

  Permission to use, copy, modify, and distribute this software and
its documentation for any purpose and without fee is hereby
granted, provided that the above copyright notice appears in all
copies and that both the copyright notice and this permission notice
appear in supporting documentation, and that the name of Olivetti
not be used in advertising or publicity pertaining to distribution
of the software without specific, written prior permission.

  OLIVETTI DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE
INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS,
IN NO EVENT SHALL OLIVETTI BE LIABLE FOR ANY SPECIAL, INDIRECT, OR
CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
LOSS OF USE, DATA OR PROFITS, WHETHER IN ACTION OF CONTRACT,
NEGLIGENCE, OR OTHER TORTIOUS ACTION, ARISING OUR OF OR IN CONNECTION
WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
*/

/*
  Copyright 1988, 1989 by Intel Corporation, Santa Clara, California.

/*
 * The tty half of the keyboard/VGA console driver.  The keyboard,
 * display, escape and console code moved to
 * rust/src/arch/i386/kd/; this file keeps `kd_tty` and the device
 * entry points while the tty layer has no Rust layout.
 */

#include <mach/boolean.h>
#include <sys/types.h>
#include <kern/mach_clock.h>
#include <device/tty.h>
#include <device/io_req.h>
#include <i386/vm_param.h>
#include <i386/spl.h>
#include <i386at/kd.h>

struct tty       kd_tty;
extern boolean_t rebootflag;
extern int       kd_state;
extern vm_offset_t kd_bitmap_start;

/*
 * C shims for the Rust driver: the line discipline feed and the input
 * buffer allocation (`kd_tty_rint()`/`kd_tty_init()`), and the
 * `phystokv()`, `rebootflag` and `hz` values the Rust code cannot read.
 * See rust/src/glue.rs.
 */
void
kd_tty_rint (u_char c)
{
	(*linesw[kd_tty.t_line].l_rint)(c, &kd_tty);
}

void
kd_tty_init (void)
{
	ttychars(&kd_tty);
}

vm_offset_t
kd_phystokv (vm_offset_t addr)
{
	return phystokv(addr);
}

int
kd_rebootflag (void)
{
	return rebootflag;
}

int
kd_hz (void)
{
	return hz;
}


/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
int
kdopen(
	dev_t	 dev,
	int	 flag,
	io_req_t ior)
{
	struct 	tty	*tp;
	spl_t	o_pri;

	tp = &kd_tty;
	o_pri = simple_lock_irq(&tp->t_lock);
	if (!(tp->t_state & (TS_ISOPEN|TS_WOPEN))) {
		/* XXX ttychars allocates memory */
		_simple_unlock(&tp->t_lock.slock);
		ttychars(tp);
		_simple_lock(&tp->t_lock.slock);
		/*
		 *	Special support for boot-time rc scripts, which don't
		 *	stty the console.
		 */
		tp->t_start = kdstart;
		tp->t_stop = kdstop;
		tp->t_ospeed = tp->t_ispeed = B115200;
		tp->t_flags = TF_ODDP|TF_EVENP|TF_ECHO|TF_CRMOD|TF_XTABS|TF_LITOUT;
		kdinit();
	}
	tp->t_state |= TS_CARR_ON;
	simple_unlock_irq(o_pri, &tp->t_lock);
	return (char_open(dev, tp, flag, ior));
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
void
kdclose(dev_t dev, int flag)
{
	struct	tty	*tp;

	tp = &kd_tty;
	{
	    spl_t s;
	    s = simple_lock_irq(&tp->t_lock);
	    ttyclose(tp);
	    simple_unlock_irq(s, &tp->t_lock);
	}

	return;
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
int
kdread(dev_t dev, io_req_t uio)
{
	struct	tty	*tp;

	tp = &kd_tty;
	tp->t_state |= TS_CARR_ON;
	return((*linesw[kd_tty.t_line].l_read)(tp, uio));
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
int
kdwrite(dev_t dev, io_req_t uio)
{
	return((*linesw[kd_tty.t_line].l_write)(&kd_tty, uio));
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
vm_offset_t
kdmmap(dev_t dev, vm_offset_t off, vm_prot_t prot)
{
	if (off >= (128*1024))
		return(-1);

	/* Get page frame number for the page to be mapped. */
	return(i386_btop(kd_bitmap_start+off));
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
int
kdportdeath(
	dev_t		dev,
	mach_port_t	port)
{
	return (tty_portdeath(&kd_tty, (ipc_port_t)port));
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
io_return_t kdgetstat(
	dev_t		dev,
	dev_flavor_t	flavor,
	dev_status_t	data,		/* pointer to OUT array */
	mach_msg_type_number_t	*count)		/* OUT */
{
	io_return_t	result;

	switch (flavor) {
	    case KDGSTATE:
		if (*count < 1)
		    return (D_INVALID_OPERATION);
		*data = kd_state;
		*count = 1;
		result = D_SUCCESS;
		break;

	    case KDGKBENT:
		result = kdgetkbent((struct kbentry *)data);
		*count = sizeof(struct kbentry)/sizeof(int);
		break;

	    default:
		result = tty_get_status(&kd_tty, flavor, data, count);
		break;
	}
	return (result);
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
io_return_t kdsetstat(
	dev_t		dev,
	dev_flavor_t	flavor,
	dev_status_t	data,
	mach_msg_type_number_t	count)
{
	io_return_t	result;

	switch (flavor) {
	    case KDSKBENT:
		if (count < sizeof(struct kbentry)/sizeof(int)) {
		    return (D_INVALID_OPERATION);
		}
		result = kdsetkbent((struct kbentry *)data, 0);
		break;

	    case KDSETBELL:
		if (count < 1)
		    return (D_INVALID_OPERATION);
		result = kdsetbell(*data, 0);
		break;

	    default:
		result = tty_set_status(&kd_tty, flavor, data, count);
	}
	return (result);
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
void
kdstart(struct tty *tp)
{
	spl_t	o_pri;
	int	ch;

	if (tp->t_state & TS_TTSTOP)
		return;
	for ( ; ; ) {
		tp->t_state &= ~TS_BUSY;
		if (tp->t_state & TS_TTSTOP)
			break;
		if ((tp->t_outq.c_cc <= 0) || (ch = getc(&tp->t_outq)) == -1)
			break;
		/*
		 * Drop priority for long screen updates. ttstart() calls us at
		 * spltty.
		 */
		o_pri = splsoftclock();		/* block timeout */
		kd_putc_esc(ch);
		splx(o_pri);
	}
	if (tp->t_outq.c_cc <= TTLOWAT(tp)) {
		tt_write_wakeup(tp);
	}
}

/* Ported to rust/src/arch/i386/kd/: see MIGRATE.md. */
void
kdstop(
	struct tty 	*tp,
	int		flags)
{
	/*
	 * do nothing - all characters are output by one call to
	 * kdstart.
	 */
}
