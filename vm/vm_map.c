/*
 * Mach Operating System
 * Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
 * Copyright (c) 1993,1994 The University of Utah and
 * the Computer Systems Laboratory (CSL).
 * All rights reserved.
 *
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 *
 * CARNEGIE MELLON, THE UNIVERSITY OF UTAH AND CSL ALLOW FREE USE OF
 * THIS SOFTWARE IN ITS "AS IS" CONDITION, AND DISCLAIM ANY LIABILITY
 * OF ANY KIND FOR ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF
 * THIS SOFTWARE.
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
 *	File:	vm/vm_map.c
 *	Author:	Avadis Tevanian, Jr., Michael Wayne Young
 *	Date:	1985
 *
 *	Virtual memory mapping module.
 */

#include <kern/macros.h>
#include <kern/printf.h>
#include <mach/kern_return.h>
#include <mach/port.h>
#include <mach/vm_attributes.h>
#include <mach/vm_param.h>
#include <mach/vm_wire.h>
#include <kern/debug.h>
#include <kern/kalloc.h>
#include <kern/mach.server.h>
#include <kern/list.h>
#include <kern/rbtree.h>
#include <kern/slab.h>
#include <kern/mach4.server.h>
#include <vm/pmap.h>
#include <vm/vm_fault.h>
#include <vm/vm_map.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>
#include <vm/vm_resident.h>
#include <vm/vm_kern.h>
#include <vm/memory_object_proxy.h>
#include <ipc/ipc_port.h>
#include <string.h>


/*
 *	Virtual memory maps provide for the mapping, protection,
 *	and sharing of virtual memory objects.  In addition,
 *	this module provides for an efficient virtual copy of
 *	memory from one map to another.
 *
 *	Synchronization is required prior to most operations.
 *
 *	Maps consist of an ordered doubly-linked list of simple
 *	entries; a hint and a red-black tree are used to speed up lookups.
 *
 *	Sharing maps have been deleted from this version of Mach.
 *	All shared objects are now mapped directly into the respective
 *	maps.  This requires a change in the copy on write strategy;
 *	the asymmetric (delayed) strategy is used for shared temporary
 *	objects instead of the symmetric (shadow) strategy.  This is
 *	selected by the (new) use_shared_copy bit in the object.  See
 *	vm_object_copy_temporary in vm_object.c for details.  All maps
 *	are now "top level" maps (either task map, kernel map or submap
 *	of the kernel map).
 *
 *	Since portions of maps are specified by start/end addresses,
 *	which may not align with existing map entries, all
 *	routines merely "clip" entries to these start/end values.
 *	[That is, an entry is split into two, bordering at a
 *	start or end value.]  Note that these clippings may not
 *	always be necessary (as the two resulting entries are then
 *	not changed); however, the clipping is done for convenience.
 *	The entries can later be "glued back together" (coalesced).
 *
 *	The symmetric (shadow) copy strategy implements virtual copy
 *	by copying VM object references from one map to
 *	another, and then marking both regions as copy-on-write.
 *	It is important to note that only one writeable reference
 *	to a VM object region exists in any map when this strategy
 *	is used -- this means that shadow object creation can be
 *	delayed until a write operation occurs.  The asymmetric (delayed)
 *	strategy allows multiple maps to have writeable references to
 *	the same region of a vm object, and hence cannot delay creating
 *	its copy objects.  See vm_object_copy_temporary() in vm_object.c.
 *	Copying of permanent objects is completely different; see
 *	vm_object_copy_strategically() in vm_object.c.
 */

struct kmem_cache    vm_map_cache;		/* cache for vm_map structures */
struct kmem_cache    vm_map_entry_cache;	/* cache for vm_map_entry structures */
struct kmem_cache    vm_map_copy_cache; 	/* cache for vm_map_copy structures */

/*
 *	Placeholder object for submap operations.  This object is dropped
 *	into the range by a call to vm_map_find, and removed when
 *	vm_map_submap creates the submap.
 */

static struct vm_object	vm_submap_object_store;
vm_object_t		vm_submap_object = &vm_submap_object_store;

/*
 *	vm_map_init:
 *
 *	Initialize the vm_map module.  Must be called before
 *	any other vm_map routines.
 *
 *	Map and entry structures are allocated from caches -- we must
 *	initialize those caches.
 *
 *	There are two caches of interest:
 *
 *	vm_map_cache:		used to allocate maps.
 *	vm_map_entry_cache:	used to allocate map entries.
 *
 *	We make sure the map entry cache allocates memory directly from the
 *	physical allocator to avoid recursion with this module.
 */

void vm_map_init(void)
{
	kmem_cache_init(&vm_map_cache, "vm_map", sizeof(struct vm_map), 0,
			NULL, 0);
	kmem_cache_init(&vm_map_entry_cache, "vm_map_entry",
			sizeof(struct vm_map_entry), 0, NULL,
			KMEM_CACHE_NOOFFSLAB | KMEM_CACHE_PHYSMEM);
	kmem_cache_init(&vm_map_copy_cache, "vm_map_copy",
			sizeof(struct vm_map_copy), 0, NULL, 0);

	/*
	 *	Submap object is initialized by vm_object_init.
	 */
}

/*
 *	vm_map_setup, vm_map_create, vm_map_lock, vm_map_unlock,
 *	vm_map_copy_limits, vm_map_reference, vm_map_deallocate,
 *	vm_map_lookup_entry, vm_map_verify, vm_map_machine_attribute
 *	and vm_map_msync live in rust/src/vm/vm_map_ffi.rs now; their
 *	prototypes are unchanged in vm_map.h.  The whole copy family,
 *	including the page-list copyin and its continuation, and the
 *	region family (`vm_region`, `vm_region_create_proxy`) are Rust
 *	too.  Only vm_map_init and the caches remain here.
 */

/*
 *	SAVE_HINT:
 *
 *	Saves the specified entry as the hint for
 *	future lookups.  Performs necessary interlocks.
 */
#define	SAVE_HINT(map,value) \
	MACRO_BEGIN \
		simple_lock(&(map)->hint_lock); \
		(map)->hint = (value); \
		simple_unlock(&(map)->hint_lock); \
	MACRO_END

/*
 *	vm_map_verify:
 *
 *	Verifies that the map in question has not changed
 *	since the given version.  If successful, the map
 *	will not change until vm_map_verify_done() is called.
 */
/*
 *	vm_map_verify_done:
 *
 *	Releases locks acquired by a vm_map_verify.
 *
 *	This is now a macro in vm/vm_map.h.  It does a
 *	vm_map_unlock_read on the map.
 */

/*
 *	Routine:	vm_map_machine_attribute, vm_map_msync,
 *	vm_map_lookup, vm_map_submap, vm_map_pmap_enter, vm_map_enter
 *	and vm_map_fork live in rust/src/vm/vm_map_ffi.rs; their
 *	prototypes are unchanged in vm_map.h.  The pmap-enter
 *	debugging switches moved to rust/src/vm/vm_map.rs with the one
 *	routine that could reach them, keeping their C symbol names.
 */



