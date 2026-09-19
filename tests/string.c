/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * memcpy for the test programs.
 *
 * The kernel's memcpy comes from libmach-rs.a.  These are user-mode binaries
 * with their own link, so they carry their own, as a plain loop rather than
 * the kernel's `rep movsb'.
 */

#include <stddef.h>

void *
memcpy(void *dest, const void *src, size_t n)
{
	unsigned char *d = dest;
	const unsigned char *s = src;

	while (n != 0) {
		*d++ = *s++;
		n--;
	}

	return dest;
}
