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
 */

#include <kern/thread.h>
#include <mach/vm_attributes.h>
#include <vm/pmap.h>

void vm_map_glue_privilege_inc(void);
void vm_map_glue_privilege_dec(void);
kern_return_t vm_map_glue_pmap_attribute(
	pmap_t pmap,
	vm_offset_t address,
	vm_size_t size,
	vm_machine_attribute_t attribute,
	vm_machine_attribute_val_t *value);

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
