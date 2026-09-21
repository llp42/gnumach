/* SPDX-License-Identifier: CMU-Mach */
/* Derived from vm/vm_external.c and vm/vm_external.h: */
/*   Copyright (c) 1991,1990,1989 Carnegie Mellon University. */
/* Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com> */

/*
 * Storage for the Rust port of vm/vm_external.c.
 *
 * The three slab caches of the external-page bookkeeping are storage
 * rather than shims: `struct kmem_cache` is still C, so they stay here
 * -- statically allocated, with the same symbols and types -- until
 * kern/slab.c moves.  Nothing else of the module remains in C; the
 * routines are in rust/src/vm/vm_external.rs behind the adapters of
 * the same file.
 */

#include <kern/slab.h>

struct kmem_cache	vm_external_cache;
struct kmem_cache	vm_object_small_existence_map_cache;
struct kmem_cache	vm_object_large_existence_map_cache;
