/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * Send HMP "sendkey" commands to a QEMU monitor on a unix socket.
 * This runs on the host, not in the guest, so it is built with the
 * host compiler and is not part of the test programs' link.
 *
 * usage: tests/hmp-send <socket> <key>...
 */

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>

int
main(int argc, char **argv)
{
	struct sockaddr_un addr;
	int fd;
	int i;

	if (argc < 3) {
		fprintf(stderr, "usage: %s <socket> <key>...\n", argv[0]);
		return 2;
	}

	fd = socket(AF_UNIX, SOCK_STREAM, 0);
	if (fd < 0) {
		perror("socket");
		return 1;
	}

	memset(&addr, 0, sizeof addr);
	addr.sun_family = AF_UNIX;
	strncpy(addr.sun_path, argv[1], sizeof addr.sun_path - 1);
	if (connect(fd, (struct sockaddr *)&addr, sizeof addr) < 0) {
		perror("connect");
		return 1;
	}

	/* Let the monitor print its banner before the commands. */
	usleep(200000);

	for (i = 2; i < argc; i++) {
		char cmd[128];
		int len;

		snprintf(cmd, sizeof cmd, "sendkey %s\n", argv[i]);
		len = strlen(cmd);
		if (write(fd, cmd, len) != len) {
			perror("write");
			return 1;
		}
		/* Give QEMU time to deliver the press and release. */
		usleep(200000);
	}

	close(fd);
	return 0;
}
