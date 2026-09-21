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
 * KERN_INVALID_ADDRESS), and `pmap_copy` is a no-op macro over the
 * pmap type, so Rust cannot declare either.
 *
 * `thread_wakeup` is a macro over `thread_wakeup_prim()`; it goes when
 * the scheduler's wait/wake interface is callable from Rust.
 *
 * The object lock, the page-release probe and the submap-placeholder
 * probe read `struct vm_object`, which stays C until vm/vm_object.c
 * moves; they go then.
 *
 * The page shims (`vm_map_glue_page_*`, `vm_map_glue_pmap_enter`) read
 * `struct vm_page` bitfields and expand PMAP_ENTER/PAGE_WAKEUP_DONE/
 * VM_PAGE_FREE/VM_PAGE_QUEUES_REMOVE and the page-locked
 * `pmap_page_protect` of the page-list copyin, which stay C until
 * vm/vm_page.c moves.
 *
 * The fork shims read `struct vm_object`'s sharing fields
 * (`shadowed`, `temporary`, `size`, `use_shared_copy`, `ref_count`);
 * they go when vm/vm_object.c moves.  The overwrite's
 * `vm_map_glue_object_is_temporary` reads the same `temporary` bit, the
 * page-list copyin's `vm_map_glue_object_is_shadowed` the same
 * `shadowed` bit, and the page-list copyout's
 * `vm_map_glue_object_can_coalesce` /
 * `vm_map_glue_object_extend_size` read and write the same structure;
 * they go with them.
 */

#include <kern/thread.h>
#include <mach/vm_attributes.h>
#include <vm/pmap.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>

void vm_map_glue_privilege_inc(void);
void vm_map_glue_privilege_dec(void);
boolean_t vm_map_glue_object_is_pristine_submap(vm_object_t object);
boolean_t vm_map_glue_object_needs_shadow(
	vm_object_t object,
	vm_size_t size,
	boolean_t needs_copy,
	boolean_t is_shared);
boolean_t vm_map_glue_object_is_temporary(vm_object_t object);
boolean_t vm_map_glue_object_is_shadowed(vm_object_t object);
boolean_t vm_map_glue_object_use_shared_copy(vm_object_t object);
void vm_map_glue_object_make_shared(vm_object_t object);
void vm_map_glue_object_paging_begin(vm_object_t object);
void vm_map_glue_object_paging_end(vm_object_t object);
boolean_t vm_map_glue_page_is_absent(vm_page_t page);
boolean_t vm_map_glue_page_is_tabled(vm_page_t page);
boolean_t vm_map_glue_page_is_busy(vm_page_t page);
boolean_t vm_map_glue_page_is_fictitious(vm_page_t page);
boolean_t vm_map_glue_page_is_error(vm_page_t page);
boolean_t vm_map_glue_page_is_precious(vm_page_t page);
vm_object_t vm_map_glue_page_object(vm_page_t page);
void vm_map_glue_page_free(vm_page_t page);
void vm_map_glue_page_steal(vm_page_t page);
void vm_map_glue_page_protect(vm_page_t page, vm_prot_t protection);
void vm_map_glue_page_set_busy(vm_page_t page);
void vm_map_glue_page_clear_busy(vm_page_t page);
void vm_map_glue_page_set_dirty(vm_page_t page);
void vm_map_glue_page_wakeup_done(vm_page_t page);
void vm_map_glue_page_activate_if_idle(vm_page_t page);
int vm_map_glue_page_wire_count(vm_page_t page);
vm_offset_t vm_map_glue_page_offset(vm_page_t page);
void vm_map_glue_page_queue_lock(void);
void vm_map_glue_page_queue_unlock(void);
boolean_t vm_map_glue_object_can_coalesce(vm_object_t object);
void vm_map_glue_object_extend_size(vm_object_t object, vm_size_t size);
void vm_map_glue_pmap_enter(
	pmap_t pmap,
	vm_offset_t addr,
	vm_page_t page,
	vm_prot_t protection,
	boolean_t wired);
kern_return_t vm_map_glue_pmap_attribute(
	pmap_t pmap,
	vm_offset_t address,
	vm_size_t size,
	vm_machine_attribute_t attribute,
	vm_machine_attribute_val_t *value);
void vm_map_glue_pmap_copy(
	pmap_t dst,
	pmap_t src,
	vm_offset_t dst_addr,
	vm_size_t len,
	vm_offset_t src_addr);
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
vm_map_glue_pmap_copy(
	pmap_t dst,
	pmap_t src,
	vm_offset_t dst_addr,
	vm_size_t len,
	vm_offset_t src_addr)
{
	pmap_copy(dst, src, dst_addr, len, src_addr);
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

boolean_t
vm_map_glue_object_needs_shadow(
	vm_object_t object,
	vm_size_t size,
	boolean_t needs_copy,
	boolean_t is_shared)
{
	return needs_copy || object->shadowed ||
	       (object->temporary && !is_shared && object->size > size);
}

boolean_t
vm_map_glue_object_is_temporary(vm_object_t object)
{
	return object->temporary;
}

boolean_t
vm_map_glue_object_is_shadowed(vm_object_t object)
{
	return object->shadowed;
}

boolean_t
vm_map_glue_object_use_shared_copy(vm_object_t object)
{
	return object->use_shared_copy;
}

boolean_t
vm_map_glue_object_can_coalesce(vm_object_t object)
{
	return object->ref_count <= 1 &&
	       !object->pager_created &&
	       object->shadow == VM_OBJECT_NULL &&
	       object->copy == VM_OBJECT_NULL &&
	       object->paging_in_progress == 0;
}

void
vm_map_glue_object_extend_size(vm_object_t object, vm_size_t size)
{
	if (size > object->size)
		object->size = size;
}

void
vm_map_glue_object_make_shared(vm_object_t object)
{
	simple_lock(&object->Lock);
	object->use_shared_copy = TRUE;
	object->ref_count++;
	simple_unlock(&object->Lock);
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

boolean_t
vm_map_glue_page_is_tabled(vm_page_t page)
{
	return page->tabled;
}

boolean_t
vm_map_glue_page_is_busy(vm_page_t page)
{
	return page->busy;
}

boolean_t
vm_map_glue_page_is_fictitious(vm_page_t page)
{
	return page->fictitious;
}

boolean_t
vm_map_glue_page_is_error(vm_page_t page)
{
	return page->error;
}

boolean_t
vm_map_glue_page_is_precious(vm_page_t page)
{
	return page->precious;
}

vm_object_t
vm_map_glue_page_object(vm_page_t page)
{
	return page->object;
}

void
vm_map_glue_page_free(vm_page_t page)
{
	VM_PAGE_FREE(page);
}

void
vm_map_glue_page_steal(vm_page_t page)
{
	simple_lock(&vm_page_queue_lock);
	vm_page_remove(page);
	if (page->wire_count > 0) {
		page->wire_count = 0;
		vm_page_wire_count--;
	} else {
		VM_PAGE_QUEUES_REMOVE(page);
	}
	simple_unlock(&vm_page_queue_lock);
}

void
vm_map_glue_page_protect(vm_page_t page, vm_prot_t protection)
{
	pmap_page_protect(page->phys_addr,
			  protection & ~page->page_lock & ~VM_PROT_WRITE);
}

void
vm_map_glue_page_set_busy(vm_page_t page)
{
	page->busy = TRUE;
}

void
vm_map_glue_page_clear_busy(vm_page_t page)
{
	page->busy = FALSE;
}

void
vm_map_glue_page_set_dirty(vm_page_t page)
{
	page->dirty = TRUE;
}

void
vm_map_glue_page_wakeup_done(vm_page_t page)
{
	PAGE_WAKEUP_DONE(page);
}

vm_offset_t
vm_map_glue_page_offset(vm_page_t page)
{
	return page->offset;
}

void
vm_map_glue_page_queue_lock(void)
{
	simple_lock(&vm_page_queue_lock);
}

void
vm_map_glue_page_queue_unlock(void)
{
	simple_unlock(&vm_page_queue_lock);
}

void
vm_map_glue_page_activate_if_idle(vm_page_t page)
{
	simple_lock(&vm_page_queue_lock);
	if (!page->active && !page->inactive)
		vm_page_activate(page);
	simple_unlock(&vm_page_queue_lock);
}

int
vm_map_glue_page_wire_count(vm_page_t page)
{
	return page->wire_count;
}

void
vm_map_glue_pmap_enter(
	pmap_t pmap,
	vm_offset_t addr,
	vm_page_t page,
	vm_prot_t protection,
	boolean_t wired)
{
	PMAP_ENTER(pmap, addr, page, protection, wired);
}
