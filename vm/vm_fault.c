/*
 * Mach Operating System
 * Copyright (c) 1994,1990,1989,1988,1987 Carnegie Mellon University.
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
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 */
/*
 *	File:	vm_fault.c
 *	Author:	Avadis Tevanian, Jr., Michael Wayne Young
 *
 *	Page fault handling module.
 */

#include <kern/printf.h>
#include <vm/vm_fault.h>
#include <mach/kern_return.h>
#include <mach/message.h>	/* for error codes */
#include <kern/debug.h>
#include <kern/thread.h>
#include <kern/sched_prim.h>
#include <vm/vm_map.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>
#include <vm/pmap.h>
#include <mach/vm_statistics.h>
#include <vm/vm_pageout.h>
#include <mach/vm_param.h>
#include <mach/memory_object.h>
#include <vm/memory_object_user.user.h>
				/* For memory_object_data_{request,unlock} */
#include <kern/macros.h>
#include <kern/slab.h>


/*
 *	State needed by vm_fault_continue.
 *	This is a little hefty to drop directly
 *	into the thread structure.
 */
typedef struct vm_fault_state {
	struct vm_map *vmf_map;
	vm_offset_t vmf_vaddr;
	vm_prot_t vmf_fault_type;
	boolean_t vmf_change_wiring;
	vm_fault_continuation_t vmf_continuation;
	vm_map_version_t vmf_version;
	boolean_t vmf_wired;
	struct vm_object *vmf_object;
	vm_offset_t vmf_offset;
	vm_prot_t vmf_prot;

	boolean_t vmfp_backoff;
	struct vm_object *vmfp_object;
	vm_offset_t vmfp_offset;
	struct vm_page *vmfp_first_m;
	vm_prot_t vmfp_access;
} vm_fault_state_t;

struct kmem_cache	vm_fault_state_cache;

int		vm_object_absent_max = 50;

boolean_t	vm_fault_dirty_handling = FALSE;
boolean_t	vm_fault_interruptible = TRUE;

boolean_t	software_reference_bits = TRUE;


/*
 *	Routine:	vm_fault_init
 *	Purpose:
 *		Initialize our private data structures.
 */
void vm_fault_init(void)
{
	kmem_cache_init(&vm_fault_state_cache, "vm_fault_state",
			sizeof(vm_fault_state_t), 0, NULL, 0);
}

/*
 *	Routine:	vm_fault
 *	Purpose:
 *		Handle page faults, including pseudo-faults
 *		used to change the wiring status of pages.
 *	Returns:
 *		If an explicit (expression) continuation is supplied,
 *		then we call the continuation instead of returning.
 *	Implementation:
 *		Explicit continuations make this a little icky,
 *		because it hasn't been rewritten to embrace CPS.
 *		Instead, we have resume arguments for vm_fault and
 *		vm_fault_page, to let continue the fault computation.
 *
 *		vm_fault and vm_fault_page save mucho state
 *		in the moral equivalent of a closure.  The state
 *		structure is allocated when first entering vm_fault
 *		and deallocated when leaving vm_fault.
 */

static void
vm_fault_continue(void)
{
	vm_fault_state_t *state =
		(vm_fault_state_t *) current_thread()->ith_other;

	(void) vm_fault(state->vmf_map,
			state->vmf_vaddr,
			state->vmf_fault_type,
			state->vmf_change_wiring,
			TRUE, state->vmf_continuation);
	/*NOTREACHED*/
}

kern_return_t vm_fault(
	vm_map_t	map,
	vm_offset_t	vaddr,
	vm_prot_t	fault_type,
	boolean_t	change_wiring,
	boolean_t	resume,
	vm_fault_continuation_t	continuation)
{
	vm_map_version_t	version;	/* Map version for verificiation */
	boolean_t		wired;		/* Should mapping be wired down? */
	vm_object_t		object;		/* Top-level object */
	vm_offset_t		offset;		/* Top-level offset */
	vm_prot_t		prot;		/* Protection for mapping */
	vm_object_t		old_copy_object; /* Saved copy object */
	vm_page_t		result_page;	/* Result of vm_fault_page */
	vm_page_t		top_page;	/* Placeholder page */
	kern_return_t		kr;

	vm_page_t		m;	/* Fast access to result_page */

	if (resume) {
		vm_fault_state_t *state =
			(vm_fault_state_t *) current_thread()->ith_other;

		/*
		 *	Retrieve cached variables and
		 *	continue vm_fault_page.
		 */

		object = state->vmf_object;
		if (object == VM_OBJECT_NULL)
			goto RetryFault;
		version = state->vmf_version;
		wired = state->vmf_wired;
		offset = state->vmf_offset;
		prot = state->vmf_prot;

		kr = vm_fault_page(object, offset, fault_type,
				(change_wiring && !wired), !change_wiring,
				&prot, &result_page, &top_page,
				TRUE, vm_fault_continue);
		goto after_vm_fault_page;
	}

	if (continuation != vm_fault_no_continuation) {
		/*
		 *	We will probably need to save state.
		 */

		char *	state;

		/*
		 * if this assignment stmt is written as
		 * 'active_threads[cpu_number()] = kmem_cache_alloc()',
		 * cpu_number may be evaluated before kmem_cache_alloc;
		 * if kmem_cache_alloc blocks, cpu_number will be wrong
		 */

		state = (char *) kmem_cache_alloc(&vm_fault_state_cache);
		current_thread()->ith_other = state;

	}

    RetryFault: ;

	/*
	 *	Find the backing store object and offset into
	 *	it to begin the search.
	 */

	if ((kr = vm_map_lookup(&map, vaddr, fault_type, FALSE, &version,
				&object, &offset,
				&prot, &wired)) != KERN_SUCCESS) {
		goto done;
	}

	/*
	 *	If the page is wired, we must fault for the current protection
	 *	value, to avoid further faults.
	 */

	if (wired)
		fault_type = prot;

   	/*
	 *	Make a reference to this object to
	 *	prevent its disposal while we are messing with
	 *	it.  Once we have the reference, the map is free
	 *	to be diddled.  Since objects reference their
	 *	shadows (and copies), they will stay around as well.
	 */

	object->ref_count++;
	vm_object_paging_begin(object);

	if (continuation != vm_fault_no_continuation) {
		vm_fault_state_t *state =
			(vm_fault_state_t *) current_thread()->ith_other;

		/*
		 *	Save variables, in case vm_fault_page discards
		 *	our kernel stack and we have to restart.
		 */

		state->vmf_map = map;
		state->vmf_vaddr = vaddr;
		state->vmf_fault_type = fault_type;
		state->vmf_change_wiring = change_wiring;
		state->vmf_continuation = continuation;

		state->vmf_version = version;
		state->vmf_wired = wired;
		state->vmf_object = object;
		state->vmf_offset = offset;
		state->vmf_prot = prot;

		kr = vm_fault_page(object, offset, fault_type,
				   (change_wiring && !wired), !change_wiring,
				   &prot, &result_page, &top_page,
				   FALSE, vm_fault_continue);
	} else
	{
		kr = vm_fault_page(object, offset, fault_type,
				   (change_wiring && !wired), !change_wiring,
				   &prot, &result_page, &top_page,
				   FALSE, (void (*)()) 0);
	}
    after_vm_fault_page:

	/*
	 *	If we didn't succeed, lose the object reference immediately.
	 */

	if (kr != VM_FAULT_SUCCESS)
		vm_object_deallocate(object);

	/*
	 *	See why we failed, and take corrective action.
	 */

	switch (kr) {
		case VM_FAULT_SUCCESS:
			break;
		case VM_FAULT_RETRY:
			goto RetryFault;
		case VM_FAULT_INTERRUPTED:
			kr = KERN_SUCCESS;
			goto done;
		case VM_FAULT_MEMORY_SHORTAGE:
			if (continuation != vm_fault_no_continuation) {
				vm_fault_state_t *state =
					(vm_fault_state_t *) current_thread()->ith_other;

				/*
				 *	Save variables in case VM_PAGE_WAIT
				 *	discards our kernel stack.
				 */

				state->vmf_map = map;
				state->vmf_vaddr = vaddr;
				state->vmf_fault_type = fault_type;
				state->vmf_change_wiring = change_wiring;
				state->vmf_continuation = continuation;
				state->vmf_object = VM_OBJECT_NULL;

				VM_PAGE_WAIT(vm_fault_continue);
			} else
				VM_PAGE_WAIT((void (*)()) 0);
			goto RetryFault;
		case VM_FAULT_FICTITIOUS_SHORTAGE:
			vm_page_more_fictitious();
			goto RetryFault;
		case VM_FAULT_MEMORY_ERROR:
			kr = KERN_MEMORY_ERROR;
			goto done;
	}

	m = result_page;


	/*
	 *	How to clean up the result of vm_fault_page.  This
	 *	happens whether the mapping is entered or not.
	 */

#define UNLOCK_AND_DEALLOCATE				\
	MACRO_BEGIN					\
	vm_fault_cleanup(m->object, top_page);		\
	vm_object_deallocate(object);			\
	MACRO_END

	/*
	 *	What to do with the resulting page from vm_fault_page
	 *	if it doesn't get entered into the physical map:
	 */

#define RELEASE_PAGE(m)					\
	MACRO_BEGIN					\
	PAGE_WAKEUP_DONE(m);				\
	simple_lock(&vm_page_queue_lock);				\
	if (!m->active && !m->inactive)			\
		vm_page_activate(m);			\
	simple_unlock(&vm_page_queue_lock);			\
	MACRO_END

	/*
	 *	We must verify that the maps have not changed
	 *	since our last lookup.
	 */

	old_copy_object = m->object->copy;

	simple_unlock(&(m->object)->Lock);
	while (!vm_map_verify(map, &version)) {
		vm_object_t	retry_object;
		vm_offset_t	retry_offset;
		vm_prot_t	retry_prot;

		/*
		 *	To avoid trying to write_lock the map while another
		 *	thread has it read_locked (in vm_map_pageable), we
		 *	do not try for write permission.  If the page is
		 *	still writable, we will get write permission.  If it
		 *	is not, or has been marked needs_copy, we enter the
		 *	mapping without write permission, and will merely
		 *	take another fault.
		 */
		kr = vm_map_lookup(&map, vaddr,
				   fault_type & ~VM_PROT_WRITE, FALSE, &version,
				   &retry_object, &retry_offset, &retry_prot,
				   &wired);

		if (kr != KERN_SUCCESS) {
			simple_lock(&(m->object)->Lock);
			RELEASE_PAGE(m);
			UNLOCK_AND_DEALLOCATE;
			goto done;
		}

		simple_unlock(&(retry_object)->Lock);
		simple_lock(&(m->object)->Lock);

		if ((retry_object != object) ||
		    (retry_offset != offset)) {
			RELEASE_PAGE(m);
			UNLOCK_AND_DEALLOCATE;
			goto RetryFault;
		}

		/*
		 *	Check whether the protection has changed or the object
		 *	has been copied while we left the map unlocked.
		 */
		prot &= retry_prot;
		simple_unlock(&(m->object)->Lock);
	}
	simple_lock(&(m->object)->Lock);

	/*
	 *	If the copy object changed while the top-level object
	 *	was unlocked, then we must take away write permission.
	 */

	if (m->object->copy != old_copy_object)
		prot &= ~VM_PROT_WRITE;

	/*
	 *	If we want to wire down this page, but no longer have
	 *	adequate permissions, we must start all over.
	 */

	if (wired && (prot != fault_type)) {
		vm_map_verify_done(map, &version);
		RELEASE_PAGE(m);
		UNLOCK_AND_DEALLOCATE;
		goto RetryFault;
	}

	/*
	 *	It's critically important that a wired-down page be faulted
	 *	only once in each map for which it is wired.
	 */

	simple_unlock(&(m->object)->Lock);

	/*
	 *	Put this page into the physical map.
	 *	We had to do the unlock above because pmap_enter
	 *	may cause other faults.  The page may be on
	 *	the pageout queues.  If the pageout daemon comes
	 *	across the page, it will remove it from the queues.
	 */

	PMAP_ENTER(map->pmap, vaddr, m, prot, wired);

	/*
	 *	If the page is not wired down and isn't already
	 *	on a pageout queue, then put it where the
	 *	pageout daemon can find it.
	 */
	simple_lock(&(m->object)->Lock);
	simple_lock(&vm_page_queue_lock);
	if (change_wiring) {
		if (wired)
			vm_page_wire(m);
		else
			vm_page_unwire(m);
	} else if (software_reference_bits) {
		if (!m->active && !m->inactive)
			vm_page_activate(m);
		m->reference = TRUE;
	} else {
		vm_page_activate(m);
	}
	simple_unlock(&vm_page_queue_lock);

	/*
	 *	Unlock everything, and return
	 */

	vm_map_verify_done(map, &version);
	PAGE_WAKEUP_DONE(m);
	kr = KERN_SUCCESS;
	UNLOCK_AND_DEALLOCATE;

#undef	UNLOCK_AND_DEALLOCATE
#undef	RELEASE_PAGE

    done:
	if (continuation != vm_fault_no_continuation) {
		vm_fault_state_t *state =
			(vm_fault_state_t *) current_thread()->ith_other;

		kmem_cache_free(&vm_fault_state_cache, (vm_offset_t) state);
		(*continuation)(kr);
		/*NOTREACHED*/
	}

	return(kr);
}
