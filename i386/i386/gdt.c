/* 
 * Mach Operating System
 * Copyright (c) 1991,1990 Carnegie Mellon University
 * Copyright (c) 1991 IBM Corporation 
 * All Rights Reserved.
 * 
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation,
 * and that the name IBM not be used in advertising or publicity 
 * pertaining to distribution of the software without specific, written
 * prior permission.
 * 
 * CARNEGIE MELLON AND IBM ALLOW FREE USE OF THIS SOFTWARE IN ITS "AS IS"
 * CONDITION.  CARNEGIE MELLON AND IBM DISCLAIM ANY LIABILITY OF ANY KIND FOR
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
 * Global descriptor table.
 */
#include <mach/machine/vm_types.h>

#include <intel/pmap.h>
#include <kern/cpu_number.h>
#include <machine/percpu.h>

#include "vm_param.h"
#include "seg.h"
#include "msr.h"
#include "gdt.h"
#include "mp_desc.h"

struct real_descriptor gdt[GDTSZ];

static void
gdt_fill(int cpu, struct real_descriptor *mygdt)
{
	/* Initialize the kernel code and data segment descriptors.  */
#ifdef __x86_64__
	_fill_gdt_descriptor(mygdt, KERNEL_CS, 0, 0, ACC_PL_K|ACC_CODE_R, SZ_64);
	_fill_gdt_descriptor(mygdt, KERNEL_DS, 0, 0, ACC_PL_K|ACC_DATA_W, SZ_64);
	_fill_gdt_descriptor(mygdt, LINEAR_DS, 0, 0, ACC_PL_K|ACC_DATA_W, SZ_64);
#else
	_fill_gdt_descriptor(mygdt, KERNEL_CS,
			    LINEAR_MIN_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS,
			    LINEAR_MAX_KERNEL_ADDRESS - (LINEAR_MIN_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS) - 1,
			    ACC_PL_K|ACC_CODE_R, SZ_32);
	_fill_gdt_descriptor(mygdt, KERNEL_DS,
			    LINEAR_MIN_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS,
			    LINEAR_MAX_KERNEL_ADDRESS - (LINEAR_MIN_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS) - 1,
			    ACC_PL_K|ACC_DATA_W, SZ_32);
	_fill_gdt_descriptor(mygdt, LINEAR_DS,
			    0,
			    0xffffffff,
			    ACC_PL_K|ACC_DATA_W, SZ_32);
	vm_offset_t thiscpu = kvtolin(&percpu_array[cpu]);
	_fill_gdt_descriptor(mygdt, PERCPU_DS,
			    thiscpu,
			    thiscpu + sizeof(struct percpu) - 1,
			    ACC_PL_K|ACC_DATA_W, SZ_32);
#endif


	/* Load the new GDT.  */
	{
		struct pseudo_descriptor pdesc;

		pdesc.limit = (GDTSZ * sizeof(struct real_descriptor))-1;
		pdesc.linear_base = kvtolin(mygdt);
		lgdt(&pdesc);
	}
}

#ifdef __x86_64__
static void
reload_gs_base(int cpu)
{
	/* KGSBASE is kernels gs base while in userspace,
	 * but when in kernel, GSBASE must point to percpu area. */
	wrmsr(MSR_REG_GSBASE, (uint64_t)&percpu_array[cpu]);
	wrmsr(MSR_REG_KGSBASE, 0);
}
#endif

static void
reload_segs(void)
{
	/* Reload all the segment registers from the new GDT.
	   We must load ds and es with 0 before loading them with KERNEL_DS
	   because some processors will "optimize out" the loads
	   if the previous selector values happen to be the same.  */
#ifndef __x86_64__
	asm volatile("ljmp	%0,$1f\n"
		     "1:\n"
		     "movw	%w2,%%ds\n"
		     "movw	%w2,%%es\n"
		     "movw	%w2,%%fs\n"
		     "movw	%w2,%%gs\n"
		     
		     "movw	%w1,%%ds\n"
		     "movw	%w1,%%es\n"
		     "movw	%w3,%%gs\n"
		     "movw	%w1,%%ss\n"
		     : : "i" (KERNEL_CS), "r" (KERNEL_DS), "r" (0), "r" (PERCPU_DS));
#endif
}

void
gdt_init(void)
{
	gdt_fill(0, gdt);

	reload_segs();
#ifdef __x86_64__
	reload_gs_base(0);
#endif

}

void
ap_gdt_init(int cpu)
{
	gdt_fill(cpu, mp_gdt[cpu]);

	reload_segs();
#ifdef __x86_64__
	reload_gs_base(cpu);
#endif
}
