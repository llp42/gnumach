/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The one shim the Rust ipc_thread port needs: a view of the
 * ith_next/ith_prev link pair inside `struct thread`.  The cast below
 * relies on the kernel's -fno-strict-aliasing (Makefile.am).  Delete
 * this file once `struct thread` is mirrored in Rust and the links can
 * be read directly.  See rust/src/ipc/ipc_thread.rs and
 * rust/src/glue.rs.
 */

#include <stddef.h>

#include <kern/thread.h>
#include <ipc/ipc_thread.h>

/*
 * The link pair as one record.  The Rust side mirrors this layout; the
 * static assertion below keeps the view honest.
 */
struct ipc_thread_links {
	thread_t next;
	thread_t prev;
};

struct ipc_thread_links *ipc_thread_glue_links (thread_t thread);

_Static_assert(offsetof(struct thread, ith_prev)
	       == offsetof(struct thread, ith_next) + sizeof(thread_t),
	       "the thread IPC links are not adjacent");
_Static_assert(sizeof(struct ipc_thread_queue) == sizeof(thread_t),
	       "the Rust IpcThreadQueue mirror changed size");

struct ipc_thread_links *
ipc_thread_glue_links (thread_t thread)
{
	return (struct ipc_thread_links *)&thread->ith_next;
}
