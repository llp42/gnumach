/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * C shims for the Rust kd driver: `struct tty`'s lock macros and line
 * discipline switch and `ttlowat[]`, none of which Rust can call
 * directly.  See rust/src/glue.rs.
 */

#include <kern/lock.h>
#include <machine/spl.h>
#include <device/tty.h>
#include <i386/vm_param.h>

spl_t kd_simple_lock_irq (simple_lock_irq_t l);
void kd_simple_unlock_irq (spl_t s, simple_lock_irq_t l);
void kd_simple_lock (simple_lock_t l);
void kd_simple_unlock (simple_lock_t l);
int kd_ldisc_read (int line, struct tty *tp, io_req_t ior);
int kd_ldisc_write (int line, struct tty *tp, io_req_t ior);
void kd_ldisc_rint (int line, unsigned int c, struct tty *tp);
short kd_ttlowat (int speed);

spl_t
kd_simple_lock_irq (simple_lock_irq_t l)
{
	return simple_lock_irq(l);
}

void
kd_simple_unlock_irq (spl_t s, simple_lock_irq_t l)
{
	simple_unlock_irq(s, l);
}

void
kd_simple_lock (simple_lock_t l)
{
	_simple_lock(l);
}

void
kd_simple_unlock (simple_lock_t l)
{
	_simple_unlock(l);
}

int
kd_ldisc_read (int line, struct tty *tp, io_req_t ior)
{
	return (*linesw[line].l_read)(tp, ior);
}

int
kd_ldisc_write (int line, struct tty *tp, io_req_t ior)
{
	return (*linesw[line].l_write)(tp, ior);
}

void
kd_ldisc_rint (int line, unsigned int c, struct tty *tp)
{
	(*linesw[line].l_rint)(c, tp);
}

short
kd_ttlowat (int speed)
{
	return ttlowat[speed];
}
