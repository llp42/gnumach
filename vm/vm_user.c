/*
 * Mach Operating System
 * Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University
 * All Rights Reserved.
 *
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 *
 * CARNEGIE MELLON ALLOWS FREE USE OF THIS SOFTWARE IN ITS "AS IS"
 * CONDITION.  CARNEGIE MELLON DISCLAIMS ANY LIABILITY OF ANY KIND FOR
 * ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF THIS SOFTWARE.
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
 *	File:	vm/vm_user.c
 *	Author:	Avadis Tevanian, Jr., Michael Wayne Young
 *
 *	User-exported virtual memory functions.
 */

#include <mach/boolean.h>
#include <mach/kern_return.h>
#include <mach/mach_types.h>	/* to get vm_address_t */
#include <mach/memory_object.h>
#include <mach/std_types.h>	/* to get pointer_t */
#include <mach/vm_attributes.h>
#include <mach/vm_param.h>
#include <mach/vm_statistics.h>
#include <mach/vm_cache_statistics.h>
#include <mach/vm_sync.h>
#include <kern/gnumach.server.h>
#include <kern/host.h>
#include <kern/mach.server.h>
#include <kern/mach_host.server.h>
#include <kern/task.h>
#include <vm/vm_fault.h>
#include <vm/vm_kern.h>
#include <vm/vm_map.h>
#include <vm/vm_object.h>
#include <vm/memory_object_proxy.h>
#include <vm/vm_page.h>



vm_statistics_data_t	vm_stat;

kern_return_t vm_statistics(
	vm_map_t		map,
	vm_statistics_data_t	*stat)
{
	if (map == VM_MAP_NULL)
		return(KERN_INVALID_ARGUMENT);

	*stat = vm_stat;

	stat->pagesize = PAGE_SIZE;
	stat->free_count = vm_page_mem_free();
	stat->active_count = vm_page_active_count;
	stat->inactive_count = vm_page_inactive_count;
	stat->wire_count = vm_page_wire_count;

	return(KERN_SUCCESS);
}

kern_return_t vm_cache_statistics(
	vm_map_t			map,
	vm_cache_statistics_data_t	*stats)
{
	if (map == VM_MAP_NULL)
		return KERN_INVALID_ARGUMENT;

	stats->cache_object_count = vm_object_external_count;
	stats->cache_count = vm_object_external_pages;

	/* XXX Not implemented yet */
	stats->active_tmp_count = 0;
	stats->inactive_tmp_count = 0;
	stats->active_perm_count = 0;
	stats->inactive_perm_count = 0;
	stats->dirty_count = 0;
	stats->laundry_count = 0;
	stats->writeback_count = 0;
	stats->slab_count = 0;
	stats->slab_reclaim_count = 0;
	return KERN_SUCCESS;
}

/*
 *	Routine:	vm_map
 */
kern_return_t vm_map(
	vm_map_t	target_map,
	vm_offset_t	*address,
	vm_size_t	size,
	vm_offset_t	mask,
	boolean_t	anywhere,
	ipc_port_t	memory_object,
	vm_offset_t	offset,
	boolean_t	copy,
	vm_prot_t	cur_protection,
	vm_prot_t	max_protection,
	vm_inherit_t	inheritance)
{
	vm_object_t	object;
	kern_return_t	result;

	if ((target_map == VM_MAP_NULL) ||
	    (cur_protection & ~VM_PROT_ALL) ||
	    (max_protection & ~VM_PROT_ALL))
		return(KERN_INVALID_ARGUMENT);

        switch (inheritance) {
        case VM_INHERIT_NONE:
        case VM_INHERIT_COPY:
        case VM_INHERIT_SHARE:
                break;
        default:
                return(KERN_INVALID_ARGUMENT);
        }

	if (size == 0)
		return KERN_INVALID_ARGUMENT;

#ifdef USER32
        if (mask & 0x80000000)
            mask |= 0xffffffff00000000;
#endif

	*address = trunc_page(*address);
	size = round_page(size);

	if (!IP_VALID(memory_object)) {
		object = VM_OBJECT_NULL;
		offset = 0;
		copy = FALSE;
	} else if ((object = vm_object_enter(memory_object, size, FALSE))
			== VM_OBJECT_NULL)
	  {
	    ipc_port_t real_memobj;
	    vm_prot_t prot;
	    vm_offset_t start;
	    vm_offset_t len;

	    result = memory_object_proxy_lookup (memory_object, &real_memobj,
						 &prot, &start, &len);
	    if (result != KERN_SUCCESS)
	      return result;

           if (!copy)
             {
		/* Reduce the allowed access to the memory object.  */
		max_protection &= prot;
		cur_protection &= prot;
             }
           else
             {
               /* Disallow making a copy unless the proxy allows reading.  */
               if (!(prot & VM_PROT_READ))
                 return KERN_PROTECTION_FAILURE;
             }

	    /* Reduce the allowed range */
	    if ((start + offset + size) > (start + len))
	      return KERN_INVALID_ARGUMENT;

	    offset += start;

	    if ((object = vm_object_enter(real_memobj, size, FALSE))
		== VM_OBJECT_NULL)
	      return KERN_INVALID_ARGUMENT;
	  }

	/*
	 *	Perform the copy if requested
	 */

	if (copy) {
		vm_object_t	new_object;
		vm_offset_t	new_offset;

		result = vm_object_copy_strategically(object, offset, size,
				&new_object, &new_offset,
				&copy);

		/*
		 *	Throw away the reference to the
		 *	original object, as it won't be mapped.
		 */

		vm_object_deallocate(object);

		if (result != KERN_SUCCESS)
			return (result);

		object = new_object;
		offset = new_offset;
	}

	if ((result = vm_map_enter(target_map,
				address, size, mask, anywhere,
				object, offset,
				copy,
				cur_protection, max_protection, inheritance
				)) != KERN_SUCCESS)
		vm_object_deallocate(object);
	return(result);
}

/*
 *	Specify that the range of the virtual address space
 *	of the target task must not cause page faults for
 *	the indicated accesses.
 *
 *	[ To unwire the pages, specify VM_PROT_NONE. ]
 */
kern_return_t vm_wire(const ipc_port_t port,
		vm_map_t map,
		vm_offset_t start,
		vm_size_t size,
		vm_prot_t access)
{
	boolean_t priv;

	if (!IP_VALID(port))
		return KERN_INVALID_HOST;

	ip_lock(port);
	if (!ip_active(port) ||
		  (ip_kotype(port) != IKOT_HOST_PRIV
		&& ip_kotype(port) != IKOT_HOST))
	{
		ip_unlock(port);
		return KERN_INVALID_HOST;
	}

	priv = ip_kotype(port) == IKOT_HOST_PRIV;
	ip_unlock(port);

	if (map == VM_MAP_NULL)
		return KERN_INVALID_TASK;

	if (access & ~VM_PROT_ALL)
		return KERN_INVALID_ARGUMENT;

	/*Check if range includes projected buffer;
	  user is not allowed direct manipulation in that case*/
	if (projected_buffer_in_range(map, start, start+size))
		return(KERN_INVALID_ARGUMENT);

	/* TODO: make it tunable */
	if (!priv && access != VM_PROT_NONE && map->size_wired + size > (8<<20))
		return KERN_NO_ACCESS;

	return vm_map_pageable(map, trunc_page(start), round_page(start+size),
			       access, TRUE, TRUE);
}

kern_return_t vm_wire_all(const ipc_port_t port, vm_map_t map, vm_wire_t flags)
{
	if (!IP_VALID(port))
		return KERN_INVALID_HOST;

	ip_lock(port);

	if (!ip_active(port)
	    || (ip_kotype(port) != IKOT_HOST_PRIV)) {
		ip_unlock(port);
		return KERN_INVALID_HOST;
	}

	ip_unlock(port);

	if (map == VM_MAP_NULL) {
		return KERN_INVALID_TASK;
	}

	if (flags & ~VM_WIRE_ALL) {
		return KERN_INVALID_ARGUMENT;
	}

	/*Check if range includes projected buffer;
	  user is not allowed direct manipulation in that case*/
	if (projected_buffer_in_range(map, map->min_offset, map->max_offset)) {
		return KERN_INVALID_ARGUMENT;
	}

	return vm_map_pageable_all(map, flags);
}

/*
 *	vm_allocate_contiguous allocates "zero fill" physical memory and maps
 *	it into in the specfied map.
 */
/* TODO: respect physical alignment (palign)
 *       and minimum physical address (pmin)
 */
kern_return_t vm_allocate_contiguous(
	host_t			host_priv,
	vm_map_t		map,
	vm_address_t		*result_vaddr,
	rpc_phys_addr_t		*result_paddr,
	vm_size_t		size,
	rpc_phys_addr_t		pmin,
	rpc_phys_addr_t		pmax,
	rpc_phys_addr_t		palign)
{
	vm_size_t		alloc_size;
	unsigned int		npages;
	unsigned int		i;
	unsigned int		order;
	unsigned int		selector;
	vm_page_t		pages;
	vm_object_t		object;
	kern_return_t		kr;
	vm_address_t		vaddr;

	if (host_priv == HOST_NULL)
		return KERN_INVALID_HOST;

	if (map == VM_MAP_NULL)
		return KERN_INVALID_TASK;

	/* FIXME */
	if (pmin != 0)
		return KERN_INVALID_ARGUMENT;

	if (palign == 0)
		palign = PAGE_SIZE;

	/* FIXME: Allows some small alignments less than page size */
	if ((palign < PAGE_SIZE) && (PAGE_SIZE % palign == 0))
		palign = PAGE_SIZE;

	/* FIXME */
	if (palign != PAGE_SIZE)
		return KERN_INVALID_ARGUMENT;

#ifdef USER32
	if (pmax > 0x100000000ULL)
		pmax = 0x100000000ULL;
#endif

	selector = VM_PAGE_SEL_DMA;
	if (pmax > VM_PAGE_DMA_LIMIT)
#ifdef VM_PAGE_DMA32_LIMIT
#if VM_PAGE_DMA32_LIMIT < VM_PAGE_DIRECTMAP_LIMIT
		if (pmax <= VM_PAGE_DMA32_LIMIT)
			selector = VM_PAGE_SEL_DMA32;
	if (pmax > VM_PAGE_DMA32_LIMIT)
#endif
#endif
		if (pmax <= VM_PAGE_DIRECTMAP_LIMIT)
			selector = VM_PAGE_SEL_DIRECTMAP;
	if (pmax > VM_PAGE_DIRECTMAP_LIMIT)
#ifdef VM_PAGE_DMA32_LIMIT
#if VM_PAGE_DMA32_LIMIT > VM_PAGE_DIRECTMAP_LIMIT
		if (pmax <= VM_PAGE_DMA32_LIMIT)
			selector = VM_PAGE_SEL_DMA32;
	if (pmax > VM_PAGE_DMA32_LIMIT)
#endif
#endif
		/* Do not even try to check for largest limit, we will truncate anyway */
		/* if (pmax <= VM_PAGE_HIGHMEM_LIMIT) */
			selector = VM_PAGE_SEL_HIGHMEM;

	size = vm_page_round(size);

	if (size == 0)
		return KERN_INVALID_ARGUMENT;

	object = vm_object_allocate(size);

	if (object == NULL)
		return KERN_RESOURCE_SHORTAGE;

	/*
	 * XXX The page allocator returns blocks with a power-of-two size.
	 * The requested size may not be a power-of-two, requiring some
	 * work to release back the pages that aren't needed.
	 */
	order = vm_page_order(size);
	alloc_size = (1 << (order + PAGE_SHIFT));
	npages = vm_page_atop(alloc_size);

	pages = vm_page_grab_contig(alloc_size, selector);

	if (pages == NULL) {
		vm_object_deallocate(object);
		return KERN_RESOURCE_SHORTAGE;
	}

	simple_lock(&(object)->Lock);
	simple_lock(&vm_page_queue_lock);

	for (i = 0; i < vm_page_atop(size); i++) {
		/*
		 * XXX We can safely handle contiguous pages as an array,
		 * but this relies on knowing the implementation of the
		 * page allocator.
		 */
		pages[i].busy = FALSE;
		vm_page_insert(&pages[i], object, vm_page_ptoa(i));
		vm_page_wire(&pages[i]);
	}

	simple_unlock(&vm_page_queue_lock);
	simple_unlock(&(object)->Lock);

	for (i = vm_page_atop(size); i < npages; i++) {
		vm_page_release(&pages[i], FALSE, FALSE);
	}

	vaddr = 0;
	kr = vm_map_enter(map, &vaddr, size, 0, TRUE, object, 0, FALSE,
			  VM_PROT_READ | VM_PROT_WRITE,
			  VM_PROT_READ | VM_PROT_WRITE, VM_INHERIT_DEFAULT);

	if (kr != KERN_SUCCESS) {
		vm_object_deallocate(object);
		return kr;
	}

	kr = vm_map_pageable(map, vaddr, vaddr + size,
			     VM_PROT_READ | VM_PROT_WRITE,
			     TRUE, TRUE);

	if (kr != KERN_SUCCESS) {
		vm_map_remove(map, vaddr, vaddr + size);
		return kr;
	}

	simple_lock(&(object)->Lock);
	simple_lock(&vm_page_queue_lock);
	for (i = 0; i < vm_page_atop(size); i++)
		vm_page_unwire(&pages[i]);
	simple_unlock(&vm_page_queue_lock);
	simple_unlock(&(object)->Lock);

	*result_vaddr = vaddr;
	*result_paddr = pages->phys_addr;


	return KERN_SUCCESS;
}

/*
 *	vm_pages_phys returns information about a region of memory
 */
kern_return_t vm_pages_phys(
	host_t				host,
	vm_map_t			map,
	vm_address_t			address,
	vm_size_t			size,
	rpc_phys_addr_array_t		*pagespp,
	mach_msg_type_number_t		*countp)
{
	if (host == HOST_NULL)
		return KERN_INVALID_HOST;
	if (map == VM_MAP_NULL)
		return KERN_INVALID_TASK;

	if (!page_aligned(address))
		return KERN_INVALID_ARGUMENT;
	if (!page_aligned(size))
		return KERN_INVALID_ARGUMENT;

	mach_msg_type_number_t count = atop(size), cur;
	rpc_phys_addr_array_t pagesp = *pagespp;
	kern_return_t kr;

	if (*countp < count) {
		vm_offset_t allocated;
		/* Avoid faults while we keep vm locks */
		kr = kmem_alloc(ipc_kernel_map, &allocated,
				count * sizeof(pagesp[0]));
		if (kr != KERN_SUCCESS)
			return KERN_RESOURCE_SHORTAGE;
		pagesp = (rpc_phys_addr_array_t) allocated;
	}

	for (cur = 0; cur < count; cur++) {
		vm_map_t cmap;		/* current map in traversal */
		rpc_phys_addr_t paddr;
		vm_map_entry_t entry;	/* entry in current map */

		/* find the entry containing (or following) the address */
		vm_map_lock_read(map);
		for (cmap = map;;) {
			/* cmap is read-locked */

			if (!vm_map_lookup_entry(cmap, address, &entry)) {
				entry = VM_MAP_ENTRY_NULL;
				break;
			}

			if (entry->is_sub_map) {
				/* move down to the sub map */

				vm_map_t nmap = entry->object.sub_map;
				vm_map_lock_read(nmap);
				vm_map_unlock_read(cmap);
				cmap = nmap;
				continue;
			} else {
				/* Found it */
				break;
			}
			/*NOTREACHED*/
		}

		paddr = 0;
		if (entry) {
			vm_offset_t offset = address - entry->vme_start + entry->offset;
			vm_object_t object = entry->object.vm_object;

			if (object) {
				simple_lock(&(object)->Lock);
				vm_page_t page = vm_page_lookup(object, offset);
				if (page) {
					if (page->phys_addr != (typeof(pagesp[cur])) page->phys_addr)
						printf("warning: physical address overflow in vm_pages_phys!!\n");
					else
						paddr = page->phys_addr;
				}
				simple_unlock(&(object)->Lock);
			}
		}
		vm_map_unlock_read(cmap);
		pagesp[cur] = paddr;

		address += PAGE_SIZE;
	}

	if (pagesp != *pagespp) {
		vm_map_copy_t copy;
		kr = vm_map_copyin(ipc_kernel_map, (vm_offset_t) pagesp,
				   count * sizeof(pagesp[0]), TRUE, &copy);
		*pagespp = (rpc_phys_addr_array_t) copy;
	}

	*countp = count;

	return KERN_SUCCESS;
}

/*
 *	vm_set_size_limit
 *
 *	Sets the current/maximum virtual adress space limits
 *	of the `target_task`.
 *
 *	The host privileged port must be provided to increase
 *	the max limit.
 */
kern_return_t
vm_set_size_limit(
	const ipc_port_t host_port,
	vm_map_t         map,
	vm_size_t        current_limit,
	vm_size_t        max_limit)
{
	ipc_kobject_type_t ikot_host = IKOT_NONE;

	if (current_limit > max_limit)
		return KERN_INVALID_ARGUMENT;
	if (map == VM_MAP_NULL)
		return KERN_INVALID_TASK;

	if (!IP_VALID(host_port))
		return KERN_INVALID_HOST;
	ip_lock(host_port);
	if (ip_active(host_port))
		ikot_host = ip_kotype(host_port);
	ip_unlock(host_port);

	if (ikot_host != IKOT_HOST && ikot_host != IKOT_HOST_PRIV)
		return KERN_INVALID_HOST;

	vm_map_lock(map);
	if (max_limit > map->size_max_limit && ikot_host != IKOT_HOST_PRIV) {
		vm_map_unlock(map);
		return KERN_NO_ACCESS;
	}

	map->size_cur_limit = current_limit;
	map->size_max_limit = max_limit;
	vm_map_unlock(map);

	return KERN_SUCCESS;
}
