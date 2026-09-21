/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com> */

/*
 * Shims between vm/vm_map.c and its Rust port, for what cannot cross
 * FFI.
 *
 * The privilege bump is `current_thread()`, a per-CPU macro; it moves
 * into kern/thread.rs when the thread structure does.
 *
 * `pmap_attribute` is a macro on this machine (it is the constant
 * KERN_INVALID_ADDRESS), so Rust cannot declare it.
 *
 * `thread_wakeup` is a macro over `thread_wakeup_prim()`; it goes when
 * the scheduler's wait/wake interface is callable from Rust.
 *
 * The object lock, the page-release probe and the submap-placeholder
 * probe read `struct vm_object`, which stays C until vm/vm_object.c
 * moves; they go then.
 *
 * The page shims (`vm_map_glue_page_*`, `vm_map_glue_pmap_enter`) read
 * `struct vm_page` bitfields and expand PMAP_ENTER/PAGE_WAKEUP_DONE,
 * which stay C until vm/vm_page.c moves.
 */

#include <kern/thread.h>
#include <mach/vm_attributes.h>
#include <vm/pmap.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>

void vm_map_glue_privilege_inc(void);
void vm_map_glue_privilege_dec(void);
boolean_t vm_map_glue_object_is_pristine_submap(vm_object_t object);
void vm_map_glue_object_paging_begin(vm_object_t object);
void vm_map_glue_object_paging_end(vm_object_t object);
boolean_t vm_map_glue_page_is_absent(vm_page_t page);
void vm_map_glue_page_set_busy(vm_page_t page);
void vm_map_glue_page_wakeup_done(vm_page_t page);
void vm_map_glue_page_activate_if_idle(vm_page_t page);
void vm_map_glue_pmap_enter(
	pmap_t pmap,
	vm_offset_t addr,
	vm_page_t page,
	vm_prot_t protection);
kern_return_t vm_map_glue_pmap_attribute(
	pmap_t pmap,
	vm_offset_t address,
	vm_size_t size,
	vm_machine_attribute_t attribute,
	vm_machine_attribute_val_t *value);
void vm_map_glue_thread_wakeup(void *event);
void vm_map_glue_object_lock(vm_object_t object);
void vm_map_glue_object_unlock(vm_object_t object);
boolean_t vm_map_glue_object_can_release(vm_object_t object);

void
vm_map_glue_privilege_inc(void)
{
	struct thread *thread = current_thread();

	if (thread != THREAD_NULL)
		thread->vm_privilege++;
}

void
vm_map_glue_privilege_dec(void)
{
	struct thread *thread = current_thread();

	if (thread != THREAD_NULL)
		thread->vm_privilege--;
}

kern_return_t
vm_map_glue_pmap_attribute(
	pmap_t pmap,
	vm_offset_t address,
	vm_size_t size,
	vm_machine_attribute_t attribute,
	vm_machine_attribute_val_t *value)
{
	return pmap_attribute(pmap, address, size, attribute, value);
}

void
vm_map_glue_thread_wakeup(void *event)
{
	thread_wakeup((event_t) event);
}

void
vm_map_glue_object_lock(vm_object_t object)
{
	simple_lock(&object->Lock);
}

void
vm_map_glue_object_unlock(vm_object_t object)
{
	simple_unlock(&object->Lock);
}

boolean_t
vm_map_glue_object_can_release(vm_object_t object)
{
	return !object->pager_created &&
	       object->ref_count == 1 &&
	       object->paging_in_progress == 0;
}

boolean_t
vm_map_glue_object_is_pristine_submap(vm_object_t object)
{
	return object->resident_page_count == 0 &&
	       object->copy == VM_OBJECT_NULL &&
	       object->shadow == VM_OBJECT_NULL &&
	       !object->pager_created;
}

void
vm_map_glue_object_paging_begin(vm_object_t object)
{
	vm_object_paging_begin(object);
}

void
vm_map_glue_object_paging_end(vm_object_t object)
{
	vm_object_paging_end(object);
}

boolean_t
vm_map_glue_page_is_absent(vm_page_t page)
{
	return page->absent;
}

void
vm_map_glue_page_set_busy(vm_page_t page)
{
	page->busy = TRUE;
}

void
vm_map_glue_page_wakeup_done(vm_page_t page)
{
	PAGE_WAKEUP_DONE(page);
}

void
vm_map_glue_page_activate_if_idle(vm_page_t page)
{
	simple_lock(&vm_page_queue_lock);
	if (!page->active && !page->inactive)
		vm_page_activate(page);
	simple_unlock(&vm_page_queue_lock);
}

void
vm_map_glue_pmap_enter(
	pmap_t pmap,
	vm_offset_t addr,
	vm_page_t page,
	vm_prot_t protection)
{
	PMAP_ENTER(pmap, addr, page, protection, FALSE);
}
