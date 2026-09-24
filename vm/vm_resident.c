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
 *	File:	vm/vm_resident.c
 *	Author:	Avadis Tevanian, Jr., Michael Wayne Young
 *
 *	Resident memory management module.
 */

#include <kern/printf.h>
#include <string.h>

#include <mach/vm_prot.h>
#include <kern/debug.h>
#include <kern/list.h>
#include <kern/sched_prim.h>
#include <kern/task.h>
#include <kern/thread.h>
#include <mach/vm_statistics.h>
#include <machine/vm_param.h>
#include <kern/slab.h>
#include <vm/pmap.h>
#include <vm/vm_map.h>
#include <vm/vm_page.h>
#include <vm/vm_pageout.h>
#include <vm/vm_kern.h>
#include <vm/vm_resident.h>

#include <mach/kern_return.h>
#include <mach_debug/hash_info.h>
#include <vm/vm_user.h>



/*
 *	Associated with each page of user-allocatable memory is a
 *	page structure.
 */

/*
 *	These variables record the values returned by vm_page_bootstrap,
 *	for debugging purposes.  The Rust pmap_steal_memory also uses them
 *	internally.
 */

vm_offset_t virtual_space_start;
vm_offset_t virtual_space_end;

/*
 *	The vm_page_lookup() routine, which provides for fast
 *	(virtual memory object, offset) to page lookup, employs
 *	the following hash table.  The vm_page_{insert,remove}
 *	routines install and remove associations in the table.
 *	[This table is often called the virtual-to-physical,
 *	or VP, table.]
 */
typedef struct {
	decl_simple_lock_data(,lock)
	vm_page_t pages;
} vm_page_bucket_t;

vm_page_bucket_t *vm_page_buckets;		/* Array of buckets */
unsigned long	vm_page_bucket_count = 0;	/* How big is array? */
unsigned long	vm_page_hash_mask;		/* Mask for hash function */

static struct list	vm_page_queue_fictitious;
def_simple_lock_data(,vm_page_queue_free_lock)
int		vm_page_fictitious_count;
int		vm_object_external_count;
int		vm_object_external_pages;

/*
 *	Occasionally, the virtual memory system uses
 *	resident page structures that do not refer to
 *	real pages, for example to leave a page with
 *	important state information in the VP table.
 *
 *	These page structures are allocated the way
 *	most other kernel structures are.
 */
struct kmem_cache	vm_page_cache;

/*
 *	Fictitious pages don't have a physical address,
 *	but we must initialize phys_addr to something.
 *	For debugging, this should be a strange value
 *	that the pmap module can recognize in assertions.
 */
phys_addr_t vm_page_fictitious_addr = (phys_addr_t) -1;

/*
 *	Resident page structures are also chained on
 *	queues that are used by the page replacement
 *	system (pageout daemon).  These queues are
 *	defined here, but are shared by the pageout
 *	module.
 */
def_simple_lock_data(,vm_page_queue_lock)
int	vm_page_active_count;
int	vm_page_inactive_count;
int	vm_page_wire_count;

/*
 *	Several page replacement parameters are also
 *	shared with this module, so that page allocation
 *	(done here in vm_page_alloc) can trigger the
 *	pageout daemon.
 */
int	vm_page_laundry_count = 0;
int	vm_page_external_laundry_count = 0;


/*
 *	The VM system has a couple of heuristics for deciding
 *	that pages are "uninteresting" and should be placed
 *	on the inactive queue as likely candidates for replacement.
 *	These variables let the heuristics be controlled at run-time
 *	to make experimentation easier.
 */

boolean_t vm_page_deactivate_behind = TRUE;
boolean_t vm_page_deactivate_hint = TRUE;

/*
 *	vm_page_bootstrap:
 *
 *	Initializes the resident memory module.
 *
 *	Allocates memory for the page cells, and
 *	for the object/offset-to-page hash table headers.
 *	Each page cell is initialized and placed on the free list.
 *	Returns the range of available kernel virtual memory.
 */

void vm_page_bootstrap(
	vm_offset_t *startp,
	vm_offset_t *endp)
{
	int i;

	/*
	 *	Initialize the page queues.
	 */

	simple_lock_init(&vm_page_queue_free_lock);
	simple_lock_init(&vm_page_queue_lock);

	list_init(&vm_page_queue_fictitious);

	/*
	 *	Allocate (and initialize) the virtual-to-physical
	 *	table hash buckets.
	 *
	 *	The number of buckets should be a power of two to
	 *	get a good hash function.  The following computation
	 *	chooses the first power of two that is greater
	 *	than the number of physical pages in the system.
	 */

	if (vm_page_bucket_count == 0) {
		unsigned long npages = vm_page_table_size();

		vm_page_bucket_count = 1;
		while (vm_page_bucket_count < npages)
			vm_page_bucket_count <<= 1;
	}

	vm_page_hash_mask = vm_page_bucket_count - 1;

	if (vm_page_hash_mask & vm_page_bucket_count)
		printf("vm_page_bootstrap: WARNING -- strange page hash\n");

	vm_page_buckets = (vm_page_bucket_t *)
		pmap_steal_memory(vm_page_bucket_count *
				  sizeof(vm_page_bucket_t));

	for (i = 0; i < vm_page_bucket_count; i++) {
		vm_page_bucket_t *bucket = &vm_page_buckets[i];

		bucket->pages = VM_PAGE_NULL;
		simple_lock_init(&bucket->lock);
	}

	vm_page_setup();

	virtual_space_start = round_page(virtual_space_start);
	virtual_space_end = trunc_page(virtual_space_end);

	*startp = virtual_space_start;
	*endp = virtual_space_end;
}

/*
 *	vm_page_hash:
 *
 *	Distributes the object/offset key pair among hash buckets.
 *
 *	NOTE:	To get a good hash function, the bucket count should
 *		be a power of two.
 */
#define vm_page_hash(object, offset) \
	(((unsigned int)(vm_offset_t)object + (unsigned int)atop(offset)) \
		& vm_page_hash_mask)

/*
 *	vm_page_insert:		[ internal use only ]
 *
 *	Inserts the given mem entry into the object/object-page
 *	table and object list.
 *
 *	The object and page must be locked.
 *	The free page queue must not be locked.
 */

void vm_page_insert(
	vm_page_t	mem,
	vm_object_t	object,
	vm_offset_t	offset)
{
	vm_page_bucket_t *bucket;


	VM_PAGE_CHECK(mem);


	if (!object->internal) {
		mem->external = TRUE;
		vm_object_external_pages++;
	}

	if (mem->tabled)
		panic("vm_page_insert");

	/*
	 *	Record the object/offset pair in this page
	 */

	mem->object = object;
	mem->offset = offset;

	/*
	 *	Insert it into the object_object/offset hash table
	 */

	bucket = &vm_page_buckets[vm_page_hash(object, offset)];
	simple_lock(&bucket->lock);
	mem->next = bucket->pages;
	bucket->pages = mem;
	simple_unlock(&bucket->lock);

	/*
	 *	Now link into the object's list of backed pages.
	 */

	queue_enter_tail(&object->memq, mem,
	    __builtin_offsetof(typeof(*mem), listq));
	mem->tabled = TRUE;

	/*
	 *	Show that the object has one more resident page.
	 */

	object->resident_page_count++;

	/*
	 *	Detect sequential access and inactivate previous page.
	 *	We ignore busy pages.
	 */

	if (vm_page_deactivate_behind &&
	    (offset == object->last_alloc + PAGE_SIZE)) {
		vm_page_t	last_mem;

		last_mem = vm_page_lookup(object, object->last_alloc);
		if ((last_mem != VM_PAGE_NULL) && !last_mem->busy)
			vm_page_deactivate(last_mem);
	}
	object->last_alloc = offset;
}

/*
 *	vm_page_replace:
 *
 *	Exactly like vm_page_insert, except that we first
 *	remove any existing page at the given offset in object
 *	and we don't do deactivate-behind.
 *
 *	The object and page must be locked.
 *	The free page queue must not be locked.
 */

void vm_page_replace(
	vm_page_t	mem,
	vm_object_t	object,
	vm_offset_t	offset)
{
	vm_page_bucket_t *bucket;


	VM_PAGE_CHECK(mem);


	if (!object->internal) {
		mem->external = TRUE;
		vm_object_external_pages++;
	}

	if (mem->tabled)
		panic("vm_page_replace");

	/*
	 *	Record the object/offset pair in this page
	 */

	mem->object = object;
	mem->offset = offset;

	/*
	 *	Insert it into the object_object/offset hash table,
	 *	replacing any page that might have been there.
	 */

	bucket = &vm_page_buckets[vm_page_hash(object, offset)];
	simple_lock(&bucket->lock);
	if (bucket->pages) {
		vm_page_t *mp = &bucket->pages;
		vm_page_t m = *mp;
		do {
			if (m->object == object && m->offset == offset) {
				/*
				 * Remove page from bucket and from object,
				 * and return it to the free list.
				 */
				*mp = m->next;
				queue_remove_generic(&object->memq, m,
				    __builtin_offsetof(typeof(*m), listq));
				m->tabled = FALSE;
				object->resident_page_count--;
				VM_PAGE_QUEUES_REMOVE(m);

				if (m->external) {
					m->external = FALSE;
					vm_object_external_pages--;
				}

				/*
				 * Return page to the free list.
				 * Note the page is not tabled now, so this
				 * won't self-deadlock on the bucket lock.
				 */

				vm_page_free(m);
				break;
			}
			mp = &m->next;
		} while ((m = *mp) != 0);
		mem->next = bucket->pages;
	} else {
		mem->next = VM_PAGE_NULL;
	}
	bucket->pages = mem;
	simple_unlock(&bucket->lock);

	/*
	 *	Now link into the object's list of backed pages.
	 */

	queue_enter_tail(&object->memq, mem,
	    __builtin_offsetof(typeof(*mem), listq));
	mem->tabled = TRUE;

	/*
	 *	And show that the object has one more resident
	 *	page.
	 */

	object->resident_page_count++;
}

/*
 *	vm_page_remove:		[ internal use only ]
 *
 *	Removes the given mem entry from the object/offset-page
 *	table, the object page list, and the page queues.
 *
 *	The object and page must be locked.
 *	The free page queue must not be locked.
 */

void vm_page_remove(
	vm_page_t		mem)
{
	vm_page_bucket_t	*bucket;
	vm_page_t		this;



	VM_PAGE_CHECK(mem);

	/*
	 *	Remove from the object_object/offset hash table
	 */

	bucket = &vm_page_buckets[vm_page_hash(mem->object, mem->offset)];
	simple_lock(&bucket->lock);
	if ((this = bucket->pages) == mem) {
		/* optimize for common case */

		bucket->pages = mem->next;
	} else {
		vm_page_t	*prev;

		for (prev = &this->next;
		     (this = *prev) != mem;
		     prev = &this->next)
			continue;
		*prev = this->next;
	}
	simple_unlock(&bucket->lock);

	/*
	 *	Now remove from the object's list of backed pages.
	 */

	queue_remove_generic(&mem->object->memq, mem,
	    __builtin_offsetof(typeof(*mem), listq));

	/*
	 *	And show that the object has one fewer resident
	 *	page.
	 */

	mem->object->resident_page_count--;

	mem->tabled = FALSE;

	VM_PAGE_QUEUES_REMOVE(mem);

	if (mem->external) {
		mem->external = FALSE;
		vm_object_external_pages--;
	}
}

/*
 *	vm_page_lookup:
 *
 *	Returns the page associated with the object/offset
 *	pair specified; if none is found, VM_PAGE_NULL is returned.
 *
 *	The object must be locked.  No side effects.
 */

vm_page_t vm_page_lookup(
	vm_object_t		object,
	vm_offset_t		offset)
{
	vm_page_t		mem;
	vm_page_bucket_t 	*bucket;


	/*
	 *	Search the hash table for this object/offset pair
	 */

	bucket = &vm_page_buckets[vm_page_hash(object, offset)];

	simple_lock(&bucket->lock);
	for (mem = bucket->pages; mem != VM_PAGE_NULL; mem = mem->next) {
		VM_PAGE_CHECK(mem);
		if ((mem->object == object) && (mem->offset == offset))
			break;
	}
	simple_unlock(&bucket->lock);
	return mem;
}

/*
 *	vm_page_grab_fictitious:
 *
 *	Remove a fictitious page from the free list.
 *	Returns VM_PAGE_NULL if there are no free pages.
 */

vm_page_t vm_page_grab_fictitious(void)
{
	vm_page_t m;

	simple_lock(&vm_page_queue_free_lock);
	if (list_empty(&vm_page_queue_fictitious)) {
		m = VM_PAGE_NULL;
	} else {
		m = list_first_entry(&vm_page_queue_fictitious,
				     struct vm_page, node);
		list_remove(&m->node);
		m->free = FALSE;
		vm_page_fictitious_count--;
	}
	simple_unlock(&vm_page_queue_free_lock);

	return m;
}

/*
 *	vm_page_release_fictitious:
 *
 *	Release a fictitious page to the free list.
 */

static void vm_page_release_fictitious(
	vm_page_t m)
{
	simple_lock(&vm_page_queue_free_lock);
	if (m->free)
		panic("vm_page_release_fictitious");
	m->free = TRUE;
	list_insert_head(&vm_page_queue_fictitious, &m->node);
	vm_page_fictitious_count++;
	simple_unlock(&vm_page_queue_free_lock);
}

/*
 *	vm_page_more_fictitious:
 *
 *	Add more fictitious pages to the free list.
 *	Allowed to block.
 */

int vm_page_fictitious_quantum = 5;

void vm_page_more_fictitious(void)
{
	vm_page_t m;
	int i;

	for (i = 0; i < vm_page_fictitious_quantum; i++) {
		m = (vm_page_t) kmem_cache_alloc(&vm_page_cache);
		if (m == VM_PAGE_NULL)
			panic("vm_page_more_fictitious");

		vm_page_init(m);
		m->phys_addr = vm_page_fictitious_addr;
		m->fictitious = TRUE;
		vm_page_release_fictitious(m);
	}
}

/*
 *	vm_page_convert:
 *
 *	Attempt to convert a fictitious page into a real page.
 *
 *	The object referenced by *MP must be locked.
 */

boolean_t vm_page_convert(struct vm_page **mp)
{
	struct vm_page *real_m, *fict_m;
	vm_object_t object;
	vm_offset_t offset;

	fict_m = *mp;


	real_m = vm_page_grab(VM_PAGE_HIGHMEM);
	if (real_m == VM_PAGE_NULL)
		return FALSE;

	object = fict_m->object;
	offset = fict_m->offset;
	simple_lock(&vm_page_queue_lock);
	vm_page_remove(fict_m);

	memcpy(&real_m->vm_page_header,
	       &fict_m->vm_page_header,
	       VM_PAGE_BODY_SIZE);
	real_m->fictitious = FALSE;

	vm_page_insert(real_m, object, offset);
	simple_unlock(&vm_page_queue_lock);


	vm_page_release_fictitious(fict_m);
	*mp = real_m;
	return TRUE;
}

/*
 *	vm_page_grab_contig:
 *
 *	Remove a block of contiguous pages from the free list.
 *	Returns VM_PAGE_NULL if the request fails.
 */

vm_page_t vm_page_grab_contig(
	vm_size_t size,
	unsigned int selector)
{
	unsigned int i, order, nr_pages;
	vm_page_t mem;

	order = vm_page_order(size);
	nr_pages = 1 << order;

	/* TODO Allow caller to pass type */
	mem = vm_page_alloc_pa(order, selector, VM_PT_KERNEL);

	if (mem == NULL) {
		simple_unlock(&vm_page_queue_free_lock);
		return NULL;
	}

	for (i = 0; i < nr_pages; i++) {
		mem[i].free = FALSE;
	}

	simple_unlock(&vm_page_queue_free_lock);

	return mem;
}

/*
 *	vm_page_free_contig:
 *
 *	Return a block of contiguous pages to the free list.
 */

void vm_page_free_contig(vm_page_t mem, vm_size_t size)
{
	unsigned int i, order, nr_pages;

	order = vm_page_order(size);
	nr_pages = 1 << order;

	simple_lock(&vm_page_queue_free_lock);

	for (i = 0; i < nr_pages; i++) {
		if (mem[i].free)
			panic("vm_page_free_contig");

		mem[i].free = TRUE;
	}

	vm_page_free_pa(mem, order);

	simple_unlock(&vm_page_queue_free_lock);
}

/*
 *	vm_page_free:
 *
 *	Returns the given page to the free list,
 *	disassociating it with any VM object.
 *
 *	Object and page queues must be locked prior to entry.
 */
void vm_page_free(
	vm_page_t	mem)
{
	if (mem->free)
		panic("vm_page_free");

	if (mem->tabled) {
		vm_page_remove(mem);
	}



	if (mem->wire_count != 0) {
		if (!mem->private && !mem->fictitious)
			vm_page_wire_count--;
		mem->wire_count = 0;
	}

	PAGE_WAKEUP_DONE(mem);

	if (mem->absent)
		vm_object_absent_release(mem->object);

	/*
	 *	XXX The calls to vm_page_init here are
	 *	really overkill.
	 */

	if (mem->private || mem->fictitious) {
		vm_page_init(mem);
		mem->phys_addr = vm_page_fictitious_addr;
		mem->fictitious = TRUE;
		vm_page_release_fictitious(mem);
	} else {
		boolean_t laundry = mem->laundry;
		boolean_t external_laundry = mem->external_laundry;
		vm_page_init(mem);
		vm_page_release(mem, laundry, external_laundry);
	}
}

/*
 *	Routine:	vm_page_info
 *	Purpose:
 *		Return information about the global VP table.
 *		Fills the buffer with as much information as possible
 *		and returns the desired size of the buffer.
 *	Conditions:
 *		Nothing locked.  The caller should provide
 *		possibly-pageable memory.
 */

unsigned int
vm_page_info(
	hash_info_bucket_t *info,
	unsigned int	count)
{
	int i;

	if (vm_page_bucket_count < count)
		count = vm_page_bucket_count;

	for (i = 0; i < count; i++) {
		vm_page_bucket_t *bucket = &vm_page_buckets[i];
		unsigned int bucket_count = 0;
		vm_page_t m;

		simple_lock(&bucket->lock);
		for (m = bucket->pages; m != VM_PAGE_NULL; m = m->next)
			bucket_count++;
		simple_unlock(&bucket->lock);

		/* don't touch pageable memory while holding locks */
		info[i].hib_count = bucket_count;
	}

	return vm_page_bucket_count;
}


