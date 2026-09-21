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
 * Macros to copy a vm_map_entry. We must be careful to correctly
 * manage the wired page count. vm_map_entry_copy() creates a new
 * map entry to the same memory - the wired count in the new entry
 * must be set to zero. vm_map_entry_copy_full() creates a new
 * entry that is identical to the old entry.  This preserves the
 * wire count; it's used for map splitting and cache changing in
 * vm_map_copyout.
 */
#define vm_map_entry_copy(NEW,OLD)			\
MACRO_BEGIN						\
                *(NEW) = *(OLD);			\
                (NEW)->is_shared = FALSE;		\
                (NEW)->needs_wakeup = FALSE;		\
                (NEW)->in_transition = FALSE;		\
                (NEW)->wired_count = 0;			\
                (NEW)->wired_access = VM_PROT_NONE;	\
MACRO_END

#define vm_map_entry_copy_full(NEW,OLD)        (*(NEW) = *(OLD))

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
 *	prototypes are unchanged in vm_map.h.
 */

/*
 *     Enforces the VM limit of a target map.
 */
static kern_return_t
vm_map_enforce_limit(
	vm_map_t map,
	vm_size_t size,
	const char *fn_name)
{
	/* Limit is ignored for the kernel map */
	if (vm_map_pmap(map) == kernel_pmap) {
		return KERN_SUCCESS;
	}

	/* Avoid taking into account the total VM_PROT_NONE virtual memory */
	vm_size_t really_allocated_size = map->size - map->size_none;
	vm_size_t new_size = really_allocated_size + size;
	/* Check for integer overflow */
	if (new_size < size) {
		return KERN_INVALID_ARGUMENT;
	}

	if (new_size > map->size_cur_limit) {
		return KERN_NO_SPACE;
	}

	return KERN_SUCCESS;
}

/*
 *	vm_map_entry_create:	[ internal use only ]
 *
 *	Allocates a VM map entry for insertion in the
 *	given map (or map copy).  No fields are filled.
 */
#define	vm_map_entry_create(map) \
	    _vm_map_entry_create(&(map)->hdr)

#define	vm_map_copy_entry_create(copy) \
	    _vm_map_entry_create(&(copy)->cpy_hdr)

static vm_map_entry_t
_vm_map_entry_create(const struct vm_map_header *map_header)
{
	vm_map_entry_t	entry;

	entry = (vm_map_entry_t) kmem_cache_alloc(&vm_map_entry_cache);
	if (entry == VM_MAP_ENTRY_NULL)
		panic("vm_map_entry_create");

	return(entry);
}

/*
 *	vm_map_entry_dispose:	[ internal use only ]
 *
 *	Inverse of vm_map_entry_create.
 */
#define	vm_map_entry_dispose(map, entry) \
	_vm_map_entry_dispose(&(map)->hdr, (entry))

#define	vm_map_copy_entry_dispose(map, entry) \
	_vm_map_entry_dispose(&(copy)->cpy_hdr, (entry))

static void
_vm_map_entry_dispose(const struct vm_map_header *map_header,
		vm_map_entry_t entry)
{
	(void)map_header;

	kmem_cache_free(&vm_map_entry_cache, (vm_offset_t) entry);
}

/*
 *	Red-black tree lookup/insert comparison functions
 */
static inline int vm_map_entry_cmp_lookup(vm_offset_t addr,
                                          const struct rbtree_node *node)
{
	struct vm_map_entry *entry;

	entry = rbtree_entry(node, struct vm_map_entry, tree_node);

	if (addr < entry->vme_start)
		return -1;
	else if (addr < entry->vme_end)
		return 0;
	else
		return 1;
}

static inline int vm_map_entry_cmp_insert(const struct rbtree_node *a,
                                          const struct rbtree_node *b)
{
	struct vm_map_entry *entry;

	entry = rbtree_entry(a, struct vm_map_entry, tree_node);
	return vm_map_entry_cmp_lookup(entry->vme_start, b);
}

/*
 *	Gap management functions
 */
static inline int vm_map_entry_gap_cmp_lookup(vm_size_t gap_size,
					      const struct rbtree_node *node)
{
	struct vm_map_entry *entry;

	entry = rbtree_entry(node, struct vm_map_entry, gap_node);

	if (gap_size < entry->gap_size)
		return -1;
	else if (gap_size == entry->gap_size)
		return 0;
	else
		return 1;
}

static inline int vm_map_entry_gap_cmp_insert(const struct rbtree_node *a,
					      const struct rbtree_node *b)
{
	struct vm_map_entry *entry;

	entry = rbtree_entry(a, struct vm_map_entry, gap_node);
	return vm_map_entry_gap_cmp_lookup(entry->gap_size, b);
}

static int
vm_map_gap_valid(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	return entry != (struct vm_map_entry *)&hdr->links;
}

static void
vm_map_gap_compute(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	struct vm_map_entry *next;

	next = entry->vme_next;

	if (vm_map_gap_valid(hdr, next)) {
		entry->gap_size = next->vme_start - entry->vme_end;
	} else {
		entry->gap_size = hdr->vme_end - entry->vme_end;
	}
}

static void
vm_map_gap_insert_single(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	struct vm_map_entry *tmp;
	struct rbtree_node *node;
	unsigned long slot;

	if (!vm_map_gap_valid(hdr, entry)) {
		return;
	}

	vm_map_gap_compute(hdr, entry);

	if (entry->gap_size == 0) {
		return;
	}

	node = rbtree_lookup_slot(&hdr->gap_tree, entry->gap_size,
				  vm_map_entry_gap_cmp_lookup, slot);

	if (node == NULL) {
		rbtree_insert_slot(&hdr->gap_tree, slot, &entry->gap_node);
		list_init(&entry->gap_list);
		entry->in_gap_tree = 1;
	} else {
		tmp = rbtree_entry(node, struct vm_map_entry, gap_node);
		list_insert_tail(&tmp->gap_list, &entry->gap_list);
		entry->in_gap_tree = 0;
	}
}

static void
vm_map_gap_remove_single(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	struct vm_map_entry *tmp;

	if (!vm_map_gap_valid(hdr, entry)) {
		return;
	}

	if (entry->gap_size == 0) {
		return;
	}

	if (!entry->in_gap_tree) {
		list_remove(&entry->gap_list);
		return;
	}

	rbtree_remove(&hdr->gap_tree, &entry->gap_node);

	if (list_empty(&entry->gap_list)) {
		return;
	}

	tmp = list_first_entry(&entry->gap_list, struct vm_map_entry, gap_list);
	list_remove(&tmp->gap_list);
	list_set_head(&tmp->gap_list, &entry->gap_list);
	rbtree_insert(&hdr->gap_tree, &tmp->gap_node,
		      vm_map_entry_gap_cmp_insert);
	tmp->in_gap_tree = 1;
}

static void
vm_map_gap_update(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	vm_map_gap_remove_single(hdr, entry);
	vm_map_gap_insert_single(hdr, entry);
}

static void
vm_map_gap_insert(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	vm_map_gap_remove_single(hdr, entry->vme_prev);
	vm_map_gap_insert_single(hdr, entry->vme_prev);
	vm_map_gap_insert_single(hdr, entry);
}

static void
vm_map_gap_remove(struct vm_map_header *hdr, struct vm_map_entry *entry)
{
	vm_map_gap_remove_single(hdr, entry);
	vm_map_gap_remove_single(hdr, entry->vme_prev);
	vm_map_gap_insert_single(hdr, entry->vme_prev);
}

/*
 *	vm_map_entry_{un,}link:
 *
 *	Insert/remove entries from maps (or map copies).
 *
 *	The start and end addresses of the entries must be properly set
 *	before using these macros.
 */
#define vm_map_entry_link(map, after_where, entry)	\
	_vm_map_entry_link(&(map)->hdr, after_where, entry, 1)

#define vm_map_copy_entry_link(copy, after_where, entry)	\
	_vm_map_entry_link(&(copy)->cpy_hdr, after_where, entry, 0)

#define _vm_map_entry_link(hdr, after_where, entry, link_gap)	\
	MACRO_BEGIN					\
	(hdr)->nentries++;				\
	(entry)->vme_prev = (after_where);		\
	(entry)->vme_next = (after_where)->vme_next;	\
	(entry)->vme_prev->vme_next =			\
	 (entry)->vme_next->vme_prev = (entry);		\
	rbtree_insert(&(hdr)->tree, &(entry)->tree_node,	\
		      vm_map_entry_cmp_insert);		\
	if (link_gap)					\
		vm_map_gap_insert((hdr), (entry));	\
	MACRO_END

#define vm_map_entry_unlink(map, entry)			\
	_vm_map_entry_unlink(&(map)->hdr, entry, 1)

#define vm_map_copy_entry_unlink(copy, entry)			\
	_vm_map_entry_unlink(&(copy)->cpy_hdr, entry, 0)

#define _vm_map_entry_unlink(hdr, entry, unlink_gap)	\
	MACRO_BEGIN					\
	(hdr)->nentries--;				\
	(entry)->vme_next->vme_prev = (entry)->vme_prev; \
	(entry)->vme_prev->vme_next = (entry)->vme_next; \
	rbtree_remove(&(hdr)->tree, &(entry)->tree_node);	\
	if (unlink_gap)					\
		vm_map_gap_remove((hdr), (entry));	\
	MACRO_END

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
 * Find a range of available space from the specified map.
 *
 * If successful, this function returns the map entry immediately preceding
 * the range, and writes the range address in startp. If the map contains
 * no entry, the entry returned points to the map header.
 * Otherwise, NULL is returned.
 *
 * If map_locked is true, this function will not wait for more space in case
 * of failure. Otherwise, the map is locked.
 */
static struct vm_map_entry *
vm_map_find_entry_anywhere(struct vm_map *map,
			   vm_size_t size,
			   vm_offset_t mask,
			   boolean_t map_locked,
			   vm_offset_t *startp)
{
	struct vm_map_entry *entry;
	struct rbtree_node *node;
	vm_size_t max_size;
	vm_offset_t start, end;
	vm_offset_t max;


	max = map->max_offset;
	if (((mask + 1) & mask) != 0) {
		/* We have high bits in addition to the low bits */

		int first0 = __builtin_ffs(~mask);		/* First zero after low bits */
		vm_offset_t lowmask = (1UL << (first0-1)) - 1;		/* low bits */
		vm_offset_t himask = mask - lowmask;			/* high bits */
		int second1 = __builtin_ffs(himask);		/* First one after low bits */

		max = 1UL << (second1-1);

		if (himask + max != 0) {
			/* high bits do not continue up to the end */
			printf("invalid mask %zx\n", mask);
			return NULL;
		}

		mask = lowmask;
	}

	if (!map_locked) {
		vm_map_lock(map);
	}

restart:
	if (map->hdr.nentries == 0) {
		entry = vm_map_to_entry(map);
		start = (map->min_offset + mask) & ~mask;
		end = start + size;

		if ((start < map->min_offset) || (end <= start) || (end > max)) {
			goto error;
		}

		*startp = start;
		return entry;
	}

	entry = map->first_free;

	if (entry != vm_map_to_entry(map)) {
		start = (entry->vme_end + mask) & ~mask;
		end = start + size;

		if ((start >= entry->vme_end)
		    && (end > start)
		    && (end <= max)
		    && (end <= (entry->vme_end + entry->gap_size))) {
			*startp = start;
			return entry;
		}
	}

	max_size = size + mask;

	if (max_size < size) {
		printf("max_size %zd got smaller than size %zd with mask %zd\n",
		       max_size, size, mask);
		goto error;
	}

	node = rbtree_lookup_nearest(&map->hdr.gap_tree, max_size,
				     vm_map_entry_gap_cmp_lookup, RBTREE_RIGHT);

	if (node == NULL) {
		if (map_locked || !map->wait_for_space) {
			goto error;
		}

		assert_wait((event_t)map, TRUE);
		vm_map_unlock(map);
		thread_block(NULL);
		vm_map_lock(map);
		goto restart;
	}

	entry = rbtree_entry(node, struct vm_map_entry, gap_node);

	if (!list_empty(&entry->gap_list)) {
		entry = list_last_entry(&entry->gap_list,
					struct vm_map_entry, gap_list);
	}

	start = (entry->vme_end + mask) & ~mask;
	end = start + size;
	if (end > max) {
		/* Does not respect the allowed maximum */
		printf("%lx does not respect %lx\n", (unsigned long) end, (unsigned long) max);
		return NULL;
	}
	*startp = start;
	return entry;

error:
	printf("no more room in %p (%s)\n", map, map->name);
	return NULL;
}

/*
 *	vm_map_clip_start:	[ internal use only ]
 *
 *	Asserts that the given entry begins at or after
 *	the specified address; if necessary,
 *	it splits the entry into two.
 */
#define vm_map_clip_start(map, entry, startaddr) \
	MACRO_BEGIN \
	if ((startaddr) > (entry)->vme_start) \
		_vm_map_clip_start(&(map)->hdr,(entry),(startaddr),1); \
	MACRO_END

#define vm_map_copy_clip_start(copy, entry, startaddr) \
	MACRO_BEGIN \
	if ((startaddr) > (entry)->vme_start) \
		_vm_map_clip_start(&(copy)->cpy_hdr,(entry),(startaddr),0); \
	MACRO_END


/*
 *	vm_map_clip_end:	[ internal use only ]
 *
 *	Asserts that the given entry ends at or before
 *	the specified address; if necessary,
 *	it splits the entry into two.
 */
#define vm_map_clip_end(map, entry, endaddr) \
	MACRO_BEGIN \
	if ((endaddr) < (entry)->vme_end) \
		_vm_map_clip_end(&(map)->hdr,(entry),(endaddr),1); \
	MACRO_END

#define vm_map_copy_clip_end(copy, entry, endaddr) \
	MACRO_BEGIN \
	if ((endaddr) < (entry)->vme_end) \
		_vm_map_clip_end(&(copy)->cpy_hdr,(entry),(endaddr),0); \
	MACRO_END

static void
vm_map_entry_inc_wired(vm_map_t map, vm_map_entry_t entry)
{
	/*
	 * This member is a counter to indicate whether an entry
	 * should be faulted in (first time it is wired, wired_count
	 * goes from 0 to 1) or not (other times, wired_count goes
	 * from 1 to 2 or remains 2).
	 */
	if (entry->wired_count > 1) {
		return;
	}

	if (entry->wired_count == 0) {
		map->size_wired += entry->vme_end - entry->vme_start;
	}

	entry->wired_count++;
}

static void
vm_map_entry_reset_wired(vm_map_t map, vm_map_entry_t entry)
{
	if (entry->wired_count != 0) {
		map->size_wired -= entry->vme_end - entry->vme_start;
		entry->wired_count = 0;
	}
}

/*
 *	vm_map_copy_steal_pages, vm_map_copy_page_discard,
 *	vm_map_copy_discard, vm_map_copy_copy,
 *	vm_map_copy_discard_cont and vm_map_copy_overwrite live in
 *	rust/src/vm/vm_map_ffi.rs now.  The copyin/copyout routines
 *	below still call vm_map_copy_steal_pages, whose prototype is in
 *	vm/vm_map.h.
 */

/*
 *	Routine:	vm_map_copy_insert
 *
 *	Description:
 *		Link a copy chain ("copy") into a map at the
 *		specified location (after "where").
 *	Side effects:
 *		The copy chain is destroyed.
 *
 *	Not static because Rust's vm_map_fork calls it; it goes back to
 *	internal linkage when the copyin/copyout routines move in M6.
 */
void
vm_map_copy_insert(struct vm_map *map, struct vm_map_entry *where,
		   struct vm_map_copy *copy)
{
	struct vm_map_entry *entry;


	for (;;) {
		entry = vm_map_copy_first_entry(copy);

		if (entry == vm_map_copy_to_entry(copy)) {
			break;
		}

		/*
		 * TODO Turn copy maps into their own type so they don't
		 * use any of the tree operations.
		 */
		vm_map_copy_entry_unlink(copy, entry);
		vm_map_entry_link(map, where, entry);
		where = entry;
	}

	kmem_cache_free(&vm_map_copy_cache, (vm_offset_t)copy);
}

/*
 *	vm_map_copyout lives in rust/src/vm/vm_map_ffi.rs now; the
 *	page-list half it still calls is below, until the next slice
 *	moves it too.
 */

/*
 *
 *	vm_map_copyout_page_list:
 *
 *	Version of vm_map_copyout() for page list vm map copies.
 *
 */
kern_return_t vm_map_copyout_page_list(
	vm_map_t	dst_map,
	vm_offset_t	*dst_addr,	/* OUT */
	vm_map_copy_t	copy)
{
	vm_size_t	size;
	vm_offset_t	start;
	vm_offset_t	end;
	vm_offset_t	offset;
	vm_map_entry_t	last;
	vm_object_t	object;
	vm_page_t	*page_list, m;
	vm_map_entry_t	entry;
	vm_offset_t	old_last_offset;
	boolean_t	cont_invoked, needs_wakeup = FALSE;
	kern_return_t	result = KERN_SUCCESS;
	vm_map_copy_t	orig_copy;
	vm_offset_t	dst_offset;
	boolean_t	must_wire;

	/*
	 *	Make sure the pages are stolen, because we are
	 *	going to put them in a new object.  Assume that
	 *	all pages are identical to first in this regard.
	 */

	page_list = &copy->cpy_page_list[0];
	if ((*page_list)->tabled)
		vm_map_copy_steal_pages(copy);

	/*
	 *	Find space for the data
	 */

	size =	round_page(copy->offset + copy->size) -
		trunc_page(copy->offset);

	vm_map_lock(dst_map);

	last = vm_map_find_entry_anywhere(dst_map, size, 0, TRUE, &start);

	if (last == NULL) {
		vm_map_unlock(dst_map);
		return KERN_NO_SPACE;
	}

	if ((result = vm_map_enforce_limit(dst_map, size, "vm_map_copyout_page_lists")) != KERN_SUCCESS) {
		vm_map_unlock(dst_map);
		return result;
	}

	end = start + size;

	must_wire = dst_map->wiring_required;

	/*
	 *	See whether we can avoid creating a new entry (and object) by
	 *	extending one of our neighbors.  [So far, we only attempt to
	 *	extend from below.]
	 *
	 *	The code path below here is a bit twisted.  If any of the
	 *	extension checks fails, we branch to create_object.  If
	 *	it all works, we fall out the bottom and goto insert_pages.
	 */
	if (last == vm_map_to_entry(dst_map) ||
	    last->vme_end != start ||
	    last->is_shared != FALSE ||
	    last->is_sub_map != FALSE ||
	    last->inheritance != VM_INHERIT_DEFAULT ||
	    last->protection != VM_PROT_DEFAULT ||
	    last->max_protection != VM_PROT_ALL ||
	    last->in_transition ||
	    (must_wire ? (last->wired_count == 0)
		       : (last->wired_count != 0))) {
		    goto create_object;
	}

	/*
	 * If this entry needs an object, make one.
	 */
	if (last->object.vm_object == VM_OBJECT_NULL) {
		object = vm_object_allocate(
			(vm_size_t)(last->vme_end - last->vme_start + size));
		last->object.vm_object = object;
		last->offset = 0;
		simple_lock(&(object)->Lock);
	}
	else {
	    vm_offset_t	prev_offset = last->offset;
	    vm_size_t	prev_size = start - last->vme_start;
	    vm_size_t	new_size;

	    /*
	     *	This is basically vm_object_coalesce.
	     */

	    object = last->object.vm_object;
	    simple_lock(&(object)->Lock);

	    /*
	     *	Try to collapse the object first
	     */
	    vm_object_collapse(object);

	    /*
	     *	Can't coalesce if pages not mapped to
	     *	last may be in use anyway:
	     *	. more than one reference
	     *	. paged out
	     *	. shadows another object
	     *	. has a copy elsewhere
	     *	. paging references (pages might be in page-list)
	     */

	    if ((object->ref_count > 1) ||
		object->pager_created ||
		(object->shadow != VM_OBJECT_NULL) ||
		(object->copy != VM_OBJECT_NULL) ||
		(object->paging_in_progress != 0)) {
		    simple_unlock(&(object)->Lock);
		    goto create_object;
	    }

	    /*
	     *	Extend the object if necessary.  Don't have to call
	     *  vm_object_page_remove because the pages aren't mapped,
	     *	and vm_page_replace will free up any old ones it encounters.
	     */
	    new_size = prev_offset + prev_size + size;
	    if (new_size > object->size)
		object->size = new_size;
        }

	/*
	 *	Coalesced the two objects - can extend
	 *	the previous map entry to include the
	 *	new range.
	 */
	dst_map->size += size;
	/*
	 *	dst_map->size_none need no updating because the protection
	 *	of `last` entry is VM_PROT_DEFAULT / VM_PROT_ALL (otherwise
	 *	the flow would have jumped to create_object).
	 */
	last->vme_end = end;
	vm_map_gap_update(&dst_map->hdr, last);

	SAVE_HINT(dst_map, last);

	goto insert_pages;

create_object:

	/*
	 *	Create object
	 */
	object = vm_object_allocate(size);

	/*
	 *	Create entry
	 */

	entry = vm_map_entry_create(dst_map);

	entry->object.vm_object = object;
	entry->offset = 0;

	entry->is_shared = FALSE;
	entry->is_sub_map = FALSE;
	entry->needs_copy = FALSE;
	entry->wired_count = 0;

	if (must_wire) {
		vm_map_entry_inc_wired(dst_map, entry);
		entry->wired_access = VM_PROT_DEFAULT;
	} else {
		entry->wired_access = VM_PROT_NONE;
	}

	entry->in_transition = TRUE;
	entry->needs_wakeup = FALSE;

	entry->vme_start = start;
	entry->vme_end = start + size;

	entry->inheritance = VM_INHERIT_DEFAULT;
	entry->protection = VM_PROT_DEFAULT;
	entry->max_protection = VM_PROT_ALL;
	entry->projected_on = 0;

	simple_lock(&(object)->Lock);

	/*
	 *	Update the hints and the map size
	 */
	if (dst_map->first_free == last) {
		dst_map->first_free = entry;
	}
	SAVE_HINT(dst_map, entry);
	dst_map->size += size;
	/*
	 *	dst_map->size_none need no updating because the protection
	 *	of `entry` is VM_PROT_DEFAULT / VM_PROT_ALL
	 */

	/*
	 *	Link in the entry
	 */
	vm_map_entry_link(dst_map, last, entry);
	last = entry;

	/*
	 *	Transfer pages into new object.
	 *	Scan page list in vm_map_copy.
	 */
insert_pages:
	dst_offset = copy->offset & PAGE_MASK;
	cont_invoked = FALSE;
	orig_copy = copy;
	last->in_transition = TRUE;
	old_last_offset = last->offset
	    + (start - last->vme_start);

	simple_lock(&vm_page_queue_lock);

	for (offset = 0; offset < size; offset += PAGE_SIZE) {
		m = *page_list;

		/*
		 *	Must clear busy bit in page before inserting it.
		 *	Ok to skip wakeup logic because nobody else
		 *	can possibly know about this page.
		 *	The page is dirty in its new object.
		 */


		m->busy = FALSE;
		m->dirty = TRUE;
		vm_page_replace(m, object, old_last_offset + offset);
		if (must_wire) {
			vm_page_wire(m);
			PMAP_ENTER(dst_map->pmap,
				   last->vme_start + m->offset - last->offset,
				   m, last->protection, TRUE);
		} else {
			vm_page_activate(m);
		}

		*page_list++ = VM_PAGE_NULL;
		if (--(copy->cpy_npages) == 0 &&
		    vm_map_copy_has_cont(copy)) {
			vm_map_copy_t	new_copy;

			/*
			 *	Ok to unlock map because entry is
			 *	marked in_transition.
			 */
			cont_invoked = TRUE;
			simple_unlock(&vm_page_queue_lock);
			simple_unlock(&(object)->Lock);
			vm_map_unlock(dst_map);
			vm_map_copy_invoke_cont(copy, &new_copy, &result);

			if (result == KERN_SUCCESS) {

				/*
				 *	If we got back a copy with real pages,
				 *	steal them now.  Either all of the
				 *	pages in the list are tabled or none
				 *	of them are; mixtures are not possible.
				 *
				 *	Save original copy for consume on
				 *	success logic at end of routine.
				 */
				if (copy != orig_copy)
					vm_map_copy_discard(copy);

				if ((copy = new_copy) != VM_MAP_COPY_NULL) {
					page_list = &copy->cpy_page_list[0];
					if ((*page_list)->tabled)
				    		vm_map_copy_steal_pages(copy);
				}
			}
			else {
				/*
				 *	Continuation failed.
				 */
				vm_map_lock(dst_map);
				goto error;
			}

			vm_map_lock(dst_map);
			simple_lock(&(object)->Lock);
			simple_lock(&vm_page_queue_lock);
		}
	}

	simple_unlock(&vm_page_queue_lock);
	simple_unlock(&(object)->Lock);

	*dst_addr = start + dst_offset;

	/*
	 *	Clear the in transition bits.  This is easy if we
	 *	didn't have a continuation.
	 */
error:
	if (!cont_invoked) {
		/*
		 *	We didn't unlock the map, so nobody could
		 *	be waiting.
		 */
		last->in_transition = FALSE;
		needs_wakeup = FALSE;
	}
	else {
		if (!vm_map_lookup_entry(dst_map, start, &entry))
			panic("vm_map_copyout_page_list: missing entry");

                /*
                 * Clear transition bit for all constituent entries that
                 * were in the original entry.  Also check for waiters.
                 */
                while((entry != vm_map_to_entry(dst_map)) &&
                      (entry->vme_start < end)) {
                        entry->in_transition = FALSE;
                        if(entry->needs_wakeup) {
                                entry->needs_wakeup = FALSE;
                                needs_wakeup = TRUE;
                        }
                        entry = entry->vme_next;
                }
	}

	if (result != KERN_SUCCESS)
		vm_map_delete(dst_map, start, end);

	vm_map_unlock(dst_map);

	if (needs_wakeup)
		vm_map_entry_wakeup(dst_map);

	/*
	 *	Consume on success logic.
	 */
	if (copy != orig_copy) {
		kmem_cache_free(&vm_map_copy_cache, (vm_offset_t) copy);
	}
	if (result == KERN_SUCCESS) {
		kmem_cache_free(&vm_map_copy_cache, (vm_offset_t) orig_copy);
	}

	return(result);
}

/*
 *	Routine:	vm_map_copyin
 *
 *	Description:
 *		Copy the specified region (src_addr, len) from the
 *		source address space (src_map), possibly removing
 *		the region from the source address space (src_destroy).
 *
 *	Returns:
 *		A vm_map_copy_t object (copy_result), suitable for
 *		insertion into another address space (using vm_map_copyout),
 *		copying over another address space region (using
 *		vm_map_copy_overwrite).  If the copy is unused, it
 *		should be destroyed (using vm_map_copy_discard).
 *
 *	In/out conditions:
 *		The source map should not be locked on entry.
 */
kern_return_t vm_map_copyin(
	vm_map_t	src_map,
	vm_offset_t	src_addr,
	vm_size_t	len,
	boolean_t	src_destroy,
	vm_map_copy_t	*copy_result)	/* OUT */
{
	vm_map_entry_t	tmp_entry;	/* Result of last map lookup --
					 * in multi-level lookup, this
					 * entry contains the actual
					 * vm_object/offset.
					 */

	vm_offset_t	src_start;	/* Start of current entry --
					 * where copy is taking place now
					 */
	vm_offset_t	src_end;	/* End of entire region to be
					 * copied */

	vm_map_copy_t	copy;		/* Resulting copy */

	/*
	 *	Check for copies of zero bytes.
	 */

	if (len == 0) {
		*copy_result = VM_MAP_COPY_NULL;
		return(KERN_SUCCESS);
	}

	/*
	 *	Check that the end address doesn't overflow
	 */

	if ((src_addr + len) <= src_addr) {
		return KERN_INVALID_ADDRESS;
	}

	/*
	 *	Compute start and end of region
	 */

	src_start = trunc_page(src_addr);
	src_end = round_page(src_addr + len);

	/*
	 *	XXX VM maps shouldn't end at maximum address
	 */

	if (src_end == 0) {
		return KERN_INVALID_ADDRESS;
	}

	/*
	 *	Allocate a header element for the list.
	 *
	 *	Use the start and end in the header to
	 *	remember the endpoints prior to rounding.
	 */

	copy = (vm_map_copy_t) kmem_cache_alloc(&vm_map_copy_cache);
	vm_map_copy_first_entry(copy) =
	 vm_map_copy_last_entry(copy) = vm_map_copy_to_entry(copy);
	copy->type = VM_MAP_COPY_ENTRY_LIST;
	copy->cpy_hdr.nentries = 0;
	rbtree_init(&copy->cpy_hdr.tree);
	rbtree_init(&copy->cpy_hdr.gap_tree);

	copy->offset = src_addr;
	copy->size = len;

#define	RETURN(x)						\
	MACRO_BEGIN						\
	vm_map_unlock(src_map);					\
	vm_map_copy_discard(copy);				\
	MACRO_RETURN(x);					\
	MACRO_END

	/*
	 *	Find the beginning of the region.
	 */

 	vm_map_lock(src_map);

	if (!vm_map_lookup_entry(src_map, src_start, &tmp_entry))
		RETURN(KERN_INVALID_ADDRESS);
	vm_map_clip_start(src_map, tmp_entry, src_start);

	/*
	 *	Go through entries until we get to the end.
	 */

	while (TRUE) {
		vm_map_entry_t	src_entry = tmp_entry;	/* Top-level entry */
		vm_size_t	src_size;		/* Size of source
							 * map entry (in both
							 * maps)
							 */

		vm_object_t	src_object;		/* Object to copy */
		vm_offset_t	src_offset;

		boolean_t	src_needs_copy;		/* Should source map
							 * be made read-only
							 * for copy-on-write?
							 */

		vm_map_entry_t	new_entry;		/* Map entry for copy */
		boolean_t	new_entry_needs_copy;	/* Will new entry be COW? */

		boolean_t	was_wired;		/* Was source wired? */
		vm_map_version_t version;		/* Version before locks
							 * dropped to make copy
							 */

		/*
		 *	Verify that the region can be read.
		 */

		if (! (src_entry->protection & VM_PROT_READ))
			RETURN(KERN_PROTECTION_FAILURE);

		/*
		 *	Clip against the endpoints of the entire region.
		 */

		vm_map_clip_end(src_map, src_entry, src_end);

		src_size = src_entry->vme_end - src_start;
		src_object = src_entry->object.vm_object;
		src_offset = src_entry->offset;
		was_wired = (src_entry->wired_count != 0);

		/*
		 *	Create a new address map entry to
		 *	hold the result.  Fill in the fields from
		 *	the appropriate source entries.
		 */

		new_entry = vm_map_copy_entry_create(copy);
		vm_map_entry_copy(new_entry, src_entry);

		/*
		 *	Attempt non-blocking copy-on-write optimizations.
		 */

		if (src_destroy &&
		    (src_object == VM_OBJECT_NULL ||
		     (src_object->temporary && !src_object->use_shared_copy)))
		{
		    /*
		     * If we are destroying the source, and the object
		     * is temporary, and not shared writable,
		     * we can move the object reference
		     * from the source to the copy.  The copy is
		     * copy-on-write only if the source is.
		     * We make another reference to the object, because
		     * destroying the source entry will deallocate it.
		     */
		    vm_object_reference(src_object);

		    /*
		     * Copy is always unwired.  vm_map_copy_entry
		     * set its wired count to zero.
		     */

		    goto CopySuccessful;
		}

		if (!was_wired &&
		    vm_object_copy_temporary(
				&new_entry->object.vm_object,
				&new_entry->offset,
				&src_needs_copy,
				&new_entry_needs_copy)) {

			new_entry->needs_copy = new_entry_needs_copy;

			/*
			 *	Handle copy-on-write obligations
			 */

			if (src_needs_copy && !tmp_entry->needs_copy) {
				vm_object_pmap_protect(
					src_object,
					src_offset,
					src_size,
			      		(src_entry->is_shared ? PMAP_NULL
						: src_map->pmap),
					src_entry->vme_start,
					src_entry->protection &
						~VM_PROT_WRITE);

				tmp_entry->needs_copy = TRUE;
			}

			/*
			 *	The map has never been unlocked, so it's safe to
			 *	move to the next entry rather than doing another
			 *	lookup.
			 */

			goto CopySuccessful;
		}

		new_entry->needs_copy = FALSE;

		/*
		 *	Take an object reference, so that we may
		 *	release the map lock(s).
		 */

		vm_object_reference(src_object);

		/*
		 *	Record the timestamp for later verification.
		 *	Unlock the map.
		 */

		version.main_timestamp = src_map->timestamp;
		vm_map_unlock(src_map);

		/*
		 *	Perform the copy
		 */

		if (was_wired) {
			simple_lock(&(src_object)->Lock);
			(void) vm_object_copy_slowly(
					src_object,
					src_offset,
					src_size,
					FALSE,
					&new_entry->object.vm_object);
			new_entry->offset = 0;
			new_entry->needs_copy = FALSE;
		} else {
			kern_return_t	result;

			result = vm_object_copy_strategically(src_object,
				src_offset,
				src_size,
				&new_entry->object.vm_object,
				&new_entry->offset,
				&new_entry_needs_copy);

			new_entry->needs_copy = new_entry_needs_copy;


			if (result != KERN_SUCCESS) {
				vm_map_copy_entry_dispose(copy, new_entry);

				vm_map_lock(src_map);
				RETURN(result);
			}

		}

		/*
		 *	Throw away the extra reference
		 */

		vm_object_deallocate(src_object);

		/*
		 *	Verify that the map has not substantially
		 *	changed while the copy was being made.
		 */

		vm_map_lock(src_map);	/* Increments timestamp once! */

		if ((version.main_timestamp + 1) == src_map->timestamp)
			goto CopySuccessful;

		/*
		 *	Simple version comparison failed.
		 *
		 *	Retry the lookup and verify that the
		 *	same object/offset are still present.
		 *
		 *	[Note: a memory manager that colludes with
		 *	the calling task can detect that we have
		 *	cheated.  While the map was unlocked, the
		 *	mapping could have been changed and restored.]
		 */

		if (!vm_map_lookup_entry(src_map, src_start, &tmp_entry)) {
			vm_map_copy_entry_dispose(copy, new_entry);
			RETURN(KERN_INVALID_ADDRESS);
		}

		src_entry = tmp_entry;
		vm_map_clip_start(src_map, src_entry, src_start);

		if ((src_entry->protection & VM_PROT_READ) == VM_PROT_NONE)
			goto VerificationFailed;

		if (src_entry->vme_end < new_entry->vme_end)
			src_size = (new_entry->vme_end = src_entry->vme_end) - src_start;

		if ((src_entry->object.vm_object != src_object) ||
		    (src_entry->offset != src_offset) ) {

			/*
			 *	Verification failed.
			 *
			 *	Start over with this top-level entry.
			 */

		 VerificationFailed: ;

			vm_object_deallocate(new_entry->object.vm_object);
			vm_map_copy_entry_dispose(copy, new_entry);
			tmp_entry = src_entry;
			continue;
		}

		/*
		 *	Verification succeeded.
		 */

	 CopySuccessful: ;

		/*
		 *	Link in the new copy entry.
		 */

		vm_map_copy_entry_link(copy, vm_map_copy_last_entry(copy),
				       new_entry);

		/*
		 *	Determine whether the entire region
		 *	has been copied.
		 */
		src_start = new_entry->vme_end;
		if ((src_start >= src_end) && (src_end != 0))
			break;

		/*
		 *	Verify that there are no gaps in the region
		 */

		tmp_entry = src_entry->vme_next;
		if (tmp_entry->vme_start != src_start)
			RETURN(KERN_INVALID_ADDRESS);
	}

	/*
	 * If the source should be destroyed, do it now, since the
	 * copy was successful.
	 */
	if (src_destroy)
	    (void) vm_map_delete(src_map, trunc_page(src_addr), src_end);

	vm_map_unlock(src_map);

	*copy_result = copy;
	return(KERN_SUCCESS);

#undef	RETURN
}

/*
 *	vm_map_copyin_object:
 *
 *	Create a copy object from an object.
 *	Our caller donates an object reference.
 */

kern_return_t vm_map_copyin_object(
	vm_object_t	object,
	vm_offset_t	offset,		/* offset of region in object */
	vm_size_t	size,		/* size of region in object */
	vm_map_copy_t	*copy_result)	/* OUT */
{
	vm_map_copy_t	copy;		/* Resulting copy */

	/*
	 *	We drop the object into a special copy object
	 *	that contains the object directly.  These copy objects
	 *	are distinguished by links.
	 */

	copy = (vm_map_copy_t) kmem_cache_alloc(&vm_map_copy_cache);
	vm_map_copy_first_entry(copy) =
	 vm_map_copy_last_entry(copy) = VM_MAP_ENTRY_NULL;
	copy->type = VM_MAP_COPY_OBJECT;
	copy->cpy_object = object;
	copy->offset = offset;
	copy->size = size;

	*copy_result = copy;
	return(KERN_SUCCESS);
}

/*
 *	vm_map_copyin_page_list_cont:
 *
 *	Continuation routine for vm_map_copyin_page_list.
 *
 *	If vm_map_copyin_page_list can't fit the entire vm range
 *	into a single page list object, it creates a continuation.
 *	When the target of the operation has used the pages in the
 *	initial page list, it invokes the continuation, which calls
 *	this routine.  If an error happens, the continuation is aborted
 *	(abort arg to this routine is TRUE).  To avoid deadlocks, the
 *	pages are discarded from the initial page list before invoking
 *	the continuation.
 *
 *	NOTE: This is not the same sort of continuation used by
 *	the scheduler.
 */

static kern_return_t	vm_map_copyin_page_list_cont(
	vm_map_copyin_args_t	cont_args,
	vm_map_copy_t		*copy_result)	/* OUT */
{
	kern_return_t	result = 0; /* '=0' to quiet gcc warnings */
	boolean_t	do_abort, src_destroy, src_destroy_only;

	/*
	 *	Check for cases that only require memory destruction.
	 */
	do_abort = (copy_result == (vm_map_copy_t *) 0);
	src_destroy = (cont_args->destroy_len != (vm_size_t) 0);
	src_destroy_only = (cont_args->src_len == (vm_size_t) 0);

	if (do_abort || src_destroy_only) {
		if (src_destroy)
			result = vm_map_remove(cont_args->map,
			    cont_args->destroy_addr,
			    cont_args->destroy_addr + cont_args->destroy_len);
		if (!do_abort)
			*copy_result = VM_MAP_COPY_NULL;
	}
	else {
		result = vm_map_copyin_page_list(cont_args->map,
			cont_args->src_addr, cont_args->src_len, src_destroy,
			cont_args->steal_pages, copy_result, TRUE);

		if (src_destroy && !cont_args->steal_pages &&
			vm_map_copy_has_cont(*copy_result)) {
			    vm_map_copyin_args_t	new_args;
		    	    /*
			     *	Transfer old destroy info.
			     */
			    new_args = (vm_map_copyin_args_t)
			    		(*copy_result)->cpy_cont_args;
		            new_args->destroy_addr = cont_args->destroy_addr;
		            new_args->destroy_len = cont_args->destroy_len;
		}
	}

	vm_map_deallocate(cont_args->map);
	kfree((vm_offset_t)cont_args, sizeof(vm_map_copyin_args_data_t));

	return(result);
}

/*
 *	vm_map_copyin_page_list:
 *
 *	This is a variant of vm_map_copyin that copies in a list of pages.
 *	If steal_pages is TRUE, the pages are only in the returned list.
 *	If steal_pages is FALSE, the pages are busy and still in their
 *	objects.  A continuation may be returned if not all the pages fit:
 *	the recipient of this copy_result must be prepared to deal with it.
 */

kern_return_t vm_map_copyin_page_list(
	vm_map_t	src_map,
	vm_offset_t	src_addr,
	vm_size_t	len,
	boolean_t	src_destroy,
	boolean_t	steal_pages,
	vm_map_copy_t	*copy_result,	/* OUT */
	boolean_t	is_cont)
{
	vm_map_entry_t	src_entry;
	vm_page_t 	m;
	vm_offset_t	src_start;
	vm_offset_t	src_end;
	vm_size_t	src_size;
	vm_object_t	src_object;
	vm_offset_t	src_offset;
	vm_offset_t	src_last_offset;
	vm_map_copy_t	copy;		/* Resulting copy */
	kern_return_t	result = KERN_SUCCESS;
	boolean_t	need_map_lookup;
        vm_map_copyin_args_t	cont_args;

	/*
	 * 	If steal_pages is FALSE, this leaves busy pages in
	 *	the object.  A continuation must be used if src_destroy
	 *	is true in this case (!steal_pages && src_destroy).
	 *
	 * XXX	Still have a more general problem of what happens
	 * XXX	if the same page occurs twice in a list.  Deadlock
	 * XXX	can happen if vm_fault_page was called.  A
	 * XXX	possible solution is to use a continuation if vm_fault_page
	 * XXX	is called and we cross a map entry boundary.
	 */

	/*
	 *	Check for copies of zero bytes.
	 */

	if (len == 0) {
		*copy_result = VM_MAP_COPY_NULL;
		return(KERN_SUCCESS);
	}

	/*
	 *	Check that the end address doesn't overflow
	 */

	if ((src_addr + len) <= src_addr) {
		return KERN_INVALID_ADDRESS;
	}

	/*
	 *	Compute start and end of region
	 */

	src_start = trunc_page(src_addr);
	src_end = round_page(src_addr + len);

	/*
	 *	XXX VM maps shouldn't end at maximum address
	 */

	if (src_end == 0) {
		return KERN_INVALID_ADDRESS;
	}

	/*
	 *	Allocate a header element for the page list.
	 *
	 *	Record original offset and size, as caller may not
	 *      be page-aligned.
	 */

	copy = (vm_map_copy_t) kmem_cache_alloc(&vm_map_copy_cache);
	copy->type = VM_MAP_COPY_PAGE_LIST;
	copy->cpy_npages = 0;
	copy->offset = src_addr;
	copy->size = len;
	copy->cpy_cont = (vm_map_copy_cont_fn) 0;
	copy->cpy_cont_args = VM_MAP_COPYIN_ARGS_NULL;

	/*
	 *	Find the beginning of the region.
	 */

do_map_lookup:

 	vm_map_lock(src_map);

	if (!vm_map_lookup_entry(src_map, src_start, &src_entry)) {
		result = KERN_INVALID_ADDRESS;
		goto error;
	}
	need_map_lookup = FALSE;

	/*
	 *	Go through entries until we get to the end.
	 */

	while (TRUE) {

		if (! (src_entry->protection & VM_PROT_READ)) {
			result = KERN_PROTECTION_FAILURE;
			goto error;
		}

		if (src_end > src_entry->vme_end)
			src_size = src_entry->vme_end - src_start;
		else
			src_size = src_end - src_start;

		src_object = src_entry->object.vm_object;
		src_offset = src_entry->offset +
				(src_start - src_entry->vme_start);

		/*
		 *	If src_object is NULL, allocate it now;
		 *	we're going to fault on it shortly.
		 */
		if (src_object == VM_OBJECT_NULL) {
			src_object = vm_object_allocate((vm_size_t)
				src_entry->vme_end -
				src_entry->vme_start);
			src_entry->object.vm_object = src_object;
		}

		/*
		 * Iterate over pages.  Fault in ones that aren't present.
		 */
		src_last_offset = src_offset + src_size;
		for (; (src_offset < src_last_offset && !need_map_lookup);
		       src_offset += PAGE_SIZE, src_start += PAGE_SIZE) {

			if (copy->cpy_npages == VM_MAP_COPY_PAGE_LIST_MAX) {
make_continuation:
			    /*
			     *	At this point we have the max number of
			     *  pages busy for this thread that we're
			     *  willing to allow.  Stop here and record
			     *  arguments for the remainder.  Note:
			     *  this means that this routine isn't atomic,
			     *  but that's the breaks.  Note that only
			     *  the first vm_map_copy_t that comes back
			     *  from this routine has the right offset
			     *  and size; those from continuations are
			     *  page rounded, and short by the amount
			     *	already done.
			     *
			     *	Reset src_end so the src_destroy
			     *	code at the bottom doesn't do
			     *	something stupid.
			     */

			    cont_args = (vm_map_copyin_args_t)
			    	    kalloc(sizeof(vm_map_copyin_args_data_t));
			    cont_args->map = src_map;
			    vm_map_reference(src_map);
			    cont_args->src_addr = src_start;
			    cont_args->src_len = len - (src_start - src_addr);
			    if (src_destroy) {
			    	cont_args->destroy_addr = cont_args->src_addr;
				cont_args->destroy_len = cont_args->src_len;
			    }
			    else {
			    	cont_args->destroy_addr = (vm_offset_t) 0;
				cont_args->destroy_len = (vm_offset_t) 0;
			    }
			    cont_args->steal_pages = steal_pages;

			    copy->cpy_cont_args = cont_args;
			    copy->cpy_cont = vm_map_copyin_page_list_cont;

			    src_end = src_start;
			    vm_map_clip_end(src_map, src_entry, src_end);
			    break;
			}

			/*
			 *	Try to find the page of data.
			 */
			simple_lock(&(src_object)->Lock);
			vm_object_paging_begin(src_object);
			if (((m = vm_page_lookup(src_object, src_offset)) !=
			    VM_PAGE_NULL) && !m->busy && !m->fictitious &&
			    !m->absent && !m->error) {

				/*
				 *	This is the page.  Mark it busy
				 *	and keep the paging reference on
				 *	the object whilst we do our thing.
				 */
				m->busy = TRUE;

				/*
				 *	Also write-protect the page, so
				 *	that the map`s owner cannot change
				 *	the data.  The busy bit will prevent
				 *	faults on the page from succeeding
				 *	until the copy is released; after
				 *	that, the page can be re-entered
				 *	as writable, since we didn`t alter
				 *	the map entry.  This scheme is a
				 *	cheap copy-on-write.
				 *
				 *	Don`t forget the protection and
				 *	the page_lock value!
				 *
				 *	If the source is being destroyed
				 *	AND not shared writable, we don`t
				 *	have to protect the page, since
				 *	we will destroy the (only)
				 *	writable mapping later.
				 */
				if (!src_destroy ||
				    src_object->use_shared_copy)
				{
				    pmap_page_protect(m->phys_addr,
						  src_entry->protection
						& ~m->page_lock
						& ~VM_PROT_WRITE);
				}

			}
			else {
				vm_prot_t result_prot;
				vm_page_t top_page;
				kern_return_t kr;

				/*
				 *	Have to fault the page in; must
				 *	unlock the map to do so.  While
				 *	the map is unlocked, anything
				 *	can happen, we must lookup the
				 *	map entry before continuing.
				 */
				vm_map_unlock(src_map);
				need_map_lookup = TRUE;
retry:
				result_prot = VM_PROT_READ;

				kr = vm_fault_page(src_object, src_offset,
						   VM_PROT_READ, FALSE, FALSE,
						   &result_prot, &m, &top_page,
						   FALSE, (void (*)()) 0);
				/*
				 *	Cope with what happened.
				 */
				switch (kr) {
				case VM_FAULT_SUCCESS:
					break;
				case VM_FAULT_INTERRUPTED: /* ??? */
			        case VM_FAULT_RETRY:
					simple_lock(&(src_object)->Lock);
					vm_object_paging_begin(src_object);
					goto retry;
				case VM_FAULT_MEMORY_SHORTAGE:
					VM_PAGE_WAIT((void (*)()) 0);
					simple_lock(&(src_object)->Lock);
					vm_object_paging_begin(src_object);
					goto retry;
				case VM_FAULT_FICTITIOUS_SHORTAGE:
					vm_page_more_fictitious();
					simple_lock(&(src_object)->Lock);
					vm_object_paging_begin(src_object);
					goto retry;
				case VM_FAULT_MEMORY_ERROR:
					/*
					 *	Something broke.  If this
					 *	is a continuation, return
					 *	a partial result if possible,
					 *	else fail the whole thing.
					 *	In the continuation case, the
					 *	next continuation call will
					 *	get this error if it persists.
					 */
					vm_map_lock(src_map);
					if (is_cont &&
					    copy->cpy_npages != 0)
						goto make_continuation;

					result = KERN_MEMORY_ERROR;
					goto error;
				}

				if (top_page != VM_PAGE_NULL) {
					simple_lock(&(src_object)->Lock);
					VM_PAGE_FREE(top_page);
					vm_object_paging_end(src_object);
					simple_unlock(&(src_object)->Lock);
				 }

				 /*
				  *	We do not need to write-protect
				  *	the page, since it cannot have
				  *	been in the pmap (and we did not
				  *	enter it above).  The busy bit
				  *	will protect the page from being
				  *	entered as writable until it is
				  *	unlocked.
				  */

			}

			/*
			 *	The page is busy, its object is locked, and
			 *	we have a paging reference on it.  Either
			 *	the map is locked, or need_map_lookup is
			 *	TRUE.
			 *
			 *	Put the page in the page list.
			 */
			copy->cpy_page_list[copy->cpy_npages++] = m;
			simple_unlock(&(m->object)->Lock);
		}

		/*
		 *	DETERMINE whether the entire region
		 *	has been copied.
		 */
		if (src_start >= src_end && src_end != 0) {
			if (need_map_lookup)
				vm_map_lock(src_map);
			break;
		}

		/*
		 *	If need_map_lookup is TRUE, have to start over with
		 *	another map lookup.  Note that we dropped the map
		 *	lock (to call vm_fault_page) above only in this case.
		 */
		if (need_map_lookup)
			goto do_map_lookup;

		/*
		 *	Verify that there are no gaps in the region
		 */

		src_start = src_entry->vme_end;
		src_entry = src_entry->vme_next;
		if (src_entry->vme_start != src_start) {
			result = KERN_INVALID_ADDRESS;
			goto error;
		}
	}

	/*
	 *	If steal_pages is true, make sure all
	 *	pages in the copy are not in any object
	 *	We try to remove them from the original
	 *	object, but we may have to copy them.
	 *
	 *	At this point every page in the list is busy
	 *	and holds a paging reference to its object.
	 *	When we're done stealing, every page is busy,
	 *	and in no object (m->tabled == FALSE).
	 */
	src_start = trunc_page(src_addr);
	if (steal_pages) {
		int 		i;
		vm_offset_t	unwire_end;

		unwire_end = src_start;
		for (i = 0; i < copy->cpy_npages; i++) {

			/*
			 *	Remove the page from its object if it
			 *	can be stolen.  It can be stolen if:
 			 *
			 *	(1) The source is being destroyed,
			 *	      the object is temporary, and
			 *	      not shared.
			 *	(2) The page is not precious.
			 *
			 *	The not shared check consists of two
			 *	parts:  (a) there are no objects that
			 *	shadow this object.  (b) it is not the
			 *	object in any shared map entries (i.e.,
			 *	use_shared_copy is not set).
			 *
			 *	The first check (a) means that we can't
			 *	steal pages from objects that are not
			 *	at the top of their shadow chains.  This
			 *	should not be a frequent occurrence.
			 *
			 *	Stealing wired pages requires telling the
			 *	pmap module to let go of them.
			 *
			 *	NOTE: stealing clean pages from objects
			 *  	whose mappings survive requires a call to
			 *	the pmap module.  Maybe later.
 			 */
			m = copy->cpy_page_list[i];
			src_object = m->object;
			simple_lock(&(src_object)->Lock);

			if (src_destroy &&
			    src_object->temporary &&
			    (!src_object->shadowed) &&
			    (!src_object->use_shared_copy) &&
			    !m->precious) {
				vm_offset_t	page_vaddr;

				page_vaddr = src_start + (i * PAGE_SIZE);
				if (m->wire_count > 0) {

				    /*
				     *	In order to steal a wired
				     *	page, we have to unwire it
				     *	first.  We do this inline
				     *	here because we have the page.
				     *
				     *	Step 1: Unwire the map entry.
				     *		Also tell the pmap module
				     *		that this piece of the
				     *		pmap is pageable.
				     */
				    simple_unlock(&(src_object)->Lock);
				    if (page_vaddr >= unwire_end) {
				        if (!vm_map_lookup_entry(src_map,
				            page_vaddr, &src_entry))
		    panic("vm_map_copyin_page_list: missing wired map entry");

				        vm_map_clip_start(src_map, src_entry,
						page_vaddr);
				    	vm_map_clip_end(src_map, src_entry,
						src_start + src_size);

					vm_map_entry_reset_wired(src_map, src_entry);
					unwire_end = src_entry->vme_end;
				        pmap_pageable(vm_map_pmap(src_map),
					    page_vaddr, unwire_end, TRUE);
				    }

				    /*
				     *	Step 2: Unwire the page.
				     *	pmap_remove handles this for us.
				     */
				    simple_lock(&(src_object)->Lock);
				}

				/*
				 *	Don't need to remove the mapping;
				 *	vm_map_delete will handle it.
				 *
				 *	Steal the page.  Setting the wire count
				 *	to zero is vm_page_unwire without
				 *	activating the page.
  				 */
				simple_lock(&vm_page_queue_lock);
	 			vm_page_remove(m);
				if (m->wire_count > 0) {
				    m->wire_count = 0;
				    vm_page_wire_count--;
				} else {
				    VM_PAGE_QUEUES_REMOVE(m);
				}
				simple_unlock(&vm_page_queue_lock);
			}
			else {
			        /*
				 *	Have to copy this page.  Have to
				 *	unlock the map while copying,
				 *	hence no further page stealing.
				 *	Hence just copy all the pages.
				 *	Unlock the map while copying;
				 *	This means no further page stealing.
				 */
				simple_unlock(&(src_object)->Lock);
				vm_map_unlock(src_map);

				vm_map_copy_steal_pages(copy);

				vm_map_lock(src_map);
				break;
		        }

			vm_object_paging_end(src_object);
			simple_unlock(&(src_object)->Lock);
	        }

		/*
		 * If the source should be destroyed, do it now, since the
		 * copy was successful.
		 */

		if (src_destroy) {
		    (void) vm_map_delete(src_map, src_start, src_end);
		}
	}
	else {
		/*
		 *	!steal_pages leaves busy pages in the map.
		 *	This will cause src_destroy to hang.  Use
		 *	a continuation to prevent this.
		 */
	        if (src_destroy && !vm_map_copy_has_cont(copy)) {
			cont_args = (vm_map_copyin_args_t)
				kalloc(sizeof(vm_map_copyin_args_data_t));
			vm_map_reference(src_map);
			cont_args->map = src_map;
			cont_args->src_addr = (vm_offset_t) 0;
			cont_args->src_len = (vm_size_t) 0;
			cont_args->destroy_addr = src_start;
			cont_args->destroy_len = src_end - src_start;
			cont_args->steal_pages = FALSE;

			copy->cpy_cont_args = cont_args;
			copy->cpy_cont = vm_map_copyin_page_list_cont;
		}

	}

	vm_map_unlock(src_map);

	*copy_result = copy;
	return(result);

error:
	vm_map_unlock(src_map);
	vm_map_copy_discard(copy);
	return(result);
}

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
 *	vm_region:
 *
 *	User call to obtain information about a region in
 *	a task's address map.
 */

kern_return_t	vm_region(
	vm_map_t	map,
	vm_offset_t	*address,		/* IN/OUT */
	vm_size_t	*size,			/* OUT */
	vm_prot_t	*protection,		/* OUT */
	vm_prot_t	*max_protection,	/* OUT */
	vm_inherit_t	*inheritance,		/* OUT */
	boolean_t	*is_shared,		/* OUT */
	ipc_port_t	*object_name,		/* OUT */
	vm_offset_t	*offset_in_object)	/* OUT */
{
	vm_map_entry_t	tmp_entry;
	vm_map_entry_t	entry;
	vm_offset_t	tmp_offset;
	vm_offset_t	start;

	if (map == VM_MAP_NULL)
		return(KERN_INVALID_ARGUMENT);

	start = *address;

	vm_map_lock_read(map);
	if (!vm_map_lookup_entry(map, start, &tmp_entry)) {
		if ((entry = tmp_entry->vme_next) == vm_map_to_entry(map)) {
			vm_map_unlock_read(map);
		   	return(KERN_NO_SPACE);
		}
	} else {
		entry = tmp_entry;
	}

	start = entry->vme_start;
	*protection = entry->protection;
	*max_protection = entry->max_protection;
	*inheritance = entry->inheritance;
	*address = start;
	*size = (entry->vme_end - start);

	tmp_offset = entry->offset;


	if (entry->is_sub_map) {
		*is_shared = FALSE;
		*object_name = IP_NULL;
		*offset_in_object = tmp_offset;
	} else {
		*is_shared = entry->is_shared;
		*object_name = vm_object_name(entry->object.vm_object);
		*offset_in_object = tmp_offset;
	}

	vm_map_unlock_read(map);

	return(KERN_SUCCESS);
}

/*
 *	vm_region_create_proxy:
 *
 *	Gets a proxy to the region that ADDRESS belongs to, starting at the
 *	region start, with MAX_PROTECTION and LEN limited by the region ones,
 *	and returns it in *PORT.
 */
kern_return_t
vm_region_create_proxy (task_t task, vm_address_t address,
			vm_prot_t max_protection, vm_size_t len,
			ipc_port_t *port)
{
  kern_return_t ret;
  vm_map_entry_t entry, tmp_entry;
  vm_object_t object;
  rpc_vm_offset_t rpc_offset, rpc_start;
  rpc_vm_size_t rpc_len = (rpc_vm_size_t) len;
  ipc_port_t pager;

  if (task == TASK_NULL)
    return(KERN_INVALID_ARGUMENT);

  vm_map_lock_read(task->map);
  if (!vm_map_lookup_entry(task->map, address, &tmp_entry)) {
    if ((entry = tmp_entry->vme_next) == vm_map_to_entry(task->map)) {
      vm_map_unlock_read(task->map);
      return(KERN_NO_SPACE);
    }
  } else {
    entry = tmp_entry;
  }

  if (entry->is_sub_map) {
    vm_map_unlock_read(task->map);
    return(KERN_INVALID_ARGUMENT);
  }

  /* Limit the allowed protection and range to the entry ones */
  if (len > entry->vme_end - entry->vme_start) {
    vm_map_unlock_read(task->map);
    return(KERN_INVALID_ARGUMENT);
  }
  max_protection &= entry->max_protection;

  object = entry->object.vm_object;
  simple_lock(&(object)->Lock);
  /* Create a pager in case this is an internal object that does
     not yet have one. */
  vm_object_pager_create(object);
  pager = ipc_port_copy_send(object->pager);
  simple_unlock(&(object)->Lock);

  rpc_start = (address - entry->vme_start) + entry->offset;
  rpc_offset = 0;

  vm_map_unlock_read(task->map);

  ret = memory_object_create_proxy(task->itk_space, max_protection,
				    &pager, 1,
				    &rpc_offset, 1,
				    &rpc_start, 1,
				    &rpc_len, 1, port);
  if (ret)
    ipc_port_release_send(pager);

  return ret;
}




/*
 *	Routine:	vm_map_machine_attribute, vm_map_msync,
 *	vm_map_lookup, vm_map_submap, vm_map_pmap_enter, vm_map_enter
 *	and vm_map_fork live in rust/src/vm/vm_map_ffi.rs; their
 *	prototypes are unchanged in vm_map.h.  The pmap-enter
 *	debugging switches moved to rust/src/vm/vm_map.rs with the one
 *	routine that could reach them, keeping their C symbol names.
 */



