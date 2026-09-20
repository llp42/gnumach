/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * C shims for the `inb'/`outb' port I/O macros of <i386/pio.h>, which
 * Rust cannot call.  A shim cannot be named `inb'/`outb' beside their
 * function-like macros, hence the `pio_' prefix.  See rust/src/glue.rs.
 */

#include <i386/pio.h>

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
