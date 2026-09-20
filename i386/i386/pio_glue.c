/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * C shims for the `inb'/`outb' port I/O macros of <i386/pio.h>, which
 * Rust cannot call.  A shim cannot be named `inb'/`outb' beside their
 * function-like macros, hence the `pio_' prefix.  See rust/src/glue.rs.
 */

#include <i386/pio.h>

unsigned char pio_inb(unsigned short port);
unsigned short pio_inw(unsigned short port);
unsigned int pio_inl(unsigned short port);
void pio_outb(unsigned short port, unsigned char value);
void pio_outw(unsigned short port, unsigned short value);
void pio_outl(unsigned short port, unsigned int value);

unsigned char
pio_inb(unsigned short port)
{
	return inb(port);
}

void
pio_outb(unsigned short port, unsigned char value)
{
	outb(port, value);
}

unsigned short
pio_inw(unsigned short port)
{
	return inw(port);
}

unsigned int
pio_inl(unsigned short port)
{
	return inl(port);
}

void
pio_outw(unsigned short port, unsigned short value)
{
	outw(port, value);
}

void
pio_outl(unsigned short port, unsigned int value)
{
	outl(port, value);
}
