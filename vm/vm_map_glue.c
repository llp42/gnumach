/* SPDX-License-Identifier: CMU-Mach */
/* Derived from vm/vm_map.c and vm/vm_map.h: */
/*   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University. */
/*   Copyright (c) 1993,1994 The University of Utah and the Computer */
/*   Systems Laboratory (CSL). */
/* Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com> */

/*
 * Shims for the Rust port of vm/vm_map.c, for what cannot cross
 * FFI.
 *
 * The page shims (`vm_map_glue_page_*`, `vm_map_glue_pmap_enter`) read
 * `struct vm_page` bitfields and expand PMAP_ENTER/PAGE_WAKEUP_DONE/
 * VM_PAGE_QUEUES_REMOVE and the page-locked `pmap_page_protect` of the
 * page-list copyin; the map module's C edges still take them.
 *
 * The region shims read `struct task`'s `map` and `itk_space` fields;
 * they die with kern/task.c.
 *
 * The object shims and the `vm_submap_object` placeholder that used to
 * live here went with vm/vm_object.c; the object fields and the
 * placeholder are Rust now.
 *
 * The three caches of the map module are storage rather than shims: the
 * `struct kmem_cache` mirror exists, so a follow-up moves the definitions
 * to Rust statics and deletes them from here.
 */

#include <kern/slab.h>
#include <kern/task.h>
#include <ipc/ipc_port.h>
#include <vm/pmap.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>

boolean_t vm_map_glue_page_is_absent(vm_page_t page);
boolean_t vm_map_glue_page_is_tabled(vm_page_t page);
boolean_t vm_map_glue_page_is_busy(vm_page_t page);
boolean_t vm_map_glue_page_is_fictitious(vm_page_t page);
boolean_t vm_map_glue_page_is_error(vm_page_t page);
boolean_t vm_map_glue_page_is_precious(vm_page_t page);
vm_object_t vm_map_glue_page_object(vm_page_t page);
void vm_map_glue_page_steal(vm_page_t page);
void vm_map_glue_page_protect(vm_page_t page, vm_prot_t protection);
void vm_map_glue_page_set_busy(vm_page_t page);
void vm_map_glue_page_clear_busy(vm_page_t page);
void vm_map_glue_page_set_dirty(vm_page_t page);
void vm_map_glue_page_wakeup_done(vm_page_t page);
void vm_map_glue_page_activate_if_idle(vm_page_t page);
int vm_map_glue_page_wire_count(vm_page_t page);
vm_offset_t vm_map_glue_page_offset(vm_page_t page);
void vm_map_glue_pmap_enter(
	pmap_t pmap,
	vm_offset_t addr,
	vm_page_t page,
	vm_prot_t protection,
	boolean_t wired);
struct vm_map *vm_map_glue_task_map(struct task *task);
ipc_space_t vm_map_glue_task_space(struct task *task);

/*
 * The map module's slab caches.
 */

struct kmem_cache    vm_map_cache;		/* cache for vm_map structures */
struct kmem_cache    vm_map_entry_cache;	/* cache for vm_map_entry structures */
struct kmem_cache    vm_map_copy_cache; 	/* cache for vm_map_copy structures */

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

struct vm_map *
vm_map_glue_task_map(struct task *task)
{
	return task->map;
}

ipc_space_t
vm_map_glue_task_space(struct task *task)
{
	return task->itk_space;
}
