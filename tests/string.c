/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * String routines for the test programs.
 *
 * The kernel's string routines come from libmach-rs.a.  These are
 * user-mode binaries with their own link, so they carry their own, as
 * plain loops.  Only the routines the tests actually use live here.
 */

#include <stddef.h>

void *
memcpy(void *s1, const void *s2, size_t n)
{
	unsigned char *d = s1;
	const unsigned char *s = s2;

	while (n != 0) {
		*d++ = *s++;
		n--;
	}

	return s1;
}

void *
memset(void *s, int c, size_t n)
{
	unsigned char *p = s;

	while (n != 0) {
		*p++ = (unsigned char)c;
		n--;
	}

	return s;
}

char *
strcpy(char *s1, const char *s2)
{
	char *ret = s1;

	while ((*s1++ = *s2++) != '\0')
		continue;

	return ret;
}

char *
strncpy(char *s1, const char *s2, size_t n)
{
	char *ret = s1;

	while (n != 0) {
		n--;
		if ((*s1++ = *s2++) == '\0')
			break;
	}

	while (n != 0) {
		*s1++ = '\0';
		n--;
	}

	return ret;
}

size_t
strlen(const char *s)
{
	const char *ret = s;

	while (*s++ != '\0')
		continue;

	return s - 1 - ret;
}
