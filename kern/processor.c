/*
 * Mach Operating System
 * Copyright (c) 1993-1988 Carnegie Mellon University
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
 *	processor.c: processor and processor_set manipulation routines.
 */

#include <string.h>

#include <mach/boolean.h>
#include <mach/policy.h>
#include <mach/processor_info.h>
#include <mach/vm_param.h>
#include <kern/cpu_number.h>
#include <kern/debug.h>
#include <kern/kalloc.h>
#include <kern/lock.h>
#include <kern/host.h>
#include <kern/ipc_tt.h>
#include <kern/machine.h>
#include <kern/processor.h>
#include <kern/sched.h>
#include <kern/task.h>
#include <kern/thread.h>
#include <kern/ipc_host.h>
#include <ipc/ipc_port.h>
#include <machine/mp_desc.h>

#if	MACH_HOST
#include <kern/slab.h>
struct kmem_cache pset_cache;
struct processor_set *slave_pset;
#endif	/* MACH_HOST */


/*
 *	Exported variables.
 */
int	master_cpu;

struct processor_set default_pset;

queue_head_t		all_psets;
int			all_psets_count;
def_simple_lock_data(, all_psets_lock);

processor_t	master_processor;

/*
 *	Bootstrap the processor/pset system so the scheduler can run.
 */
void pset_sys_bootstrap(void)
{
	int	i;

	pset_init(&default_pset);
	for (i = 0; i < NCPUS; i++) {
		/*
		 *	Initialize processor data structures.
		 */
		processor_init(processor_ptr(i), i);
	}
	master_processor = processor_ptr(master_cpu);
	queue_init(&all_psets);
	simple_lock_init(&all_psets_lock);
	queue_enter_tail(&all_psets, &default_pset,
	    __builtin_offsetof(typeof(default_pset), all_psets));
	all_psets_count = 1;
	default_pset.active = TRUE;

	/*
	 *	Note: the default_pset has a max_priority of BASEPRI_SYSTEM
	 *	so that privileged tasks can raise its priority later on.
	 */
}

#if	MACH_HOST
/*
 *	Rest of pset system initializations.
 */
void pset_sys_init(void)
{
	int		i;
	processor_t	processor;

	/*
	 * Allocate the cache for processor sets.
	 */
	kmem_cache_init(&pset_cache, "processor_set",
			sizeof(struct processor_set), 0, NULL, 0);

	/*
	 * Give each processor a control port.
	 * The master processor already has one.
	 */
	for (i = 0; i < NCPUS; i++) {
	    processor = processor_ptr(i);
	    if (processor != master_processor &&
		machine_slot[i].is_cpu)
	    {
		ipc_processor_init(processor);
	    }
	}

	processor_set_create(&realhost, &slave_pset, &slave_pset);
}
#endif	/* MACH_HOST */

/*
 *	pset_remove_processor() removes a processor from a processor_set.
 *	It can only be called on the current processor.  Caller must
 *	hold lock on current processor and processor set.
 */

void pset_remove_processor(
	processor_set_t	pset,
	processor_t	processor)
{
	if (pset != processor->processor_set)
		panic("pset_remove_processor: wrong pset");

	queue_remove_generic(&pset->processors, processor,
	    __builtin_offsetof(typeof(*processor), processors));
	processor->processor_set = PROCESSOR_SET_NULL;
	pset->processor_count--;
	quantum_set(pset);
}

/*
 *	pset_add_processor() adds a  processor to a processor_set.
 *	It can only be called on the current processor.  Caller must
 *	hold lock on curent processor and on pset.  No reference counting on
 *	processors.  Processor reference to pset is implicit.
 */

void pset_add_processor(
	processor_set_t	pset,
	processor_t	processor)
{
	queue_enter_tail(&pset->processors, processor,
	    __builtin_offsetof(typeof(*processor), processors));
	processor->processor_set = pset;
	pset->processor_count++;
	pset->empty = FALSE;
	quantum_set(pset);
}

/*
 *	pset_remove_task() removes a task from a processor_set.
 *	Caller must hold locks on pset and task.  Pset reference count
 *	is not decremented; caller must explicitly pset_deallocate.
 */

void pset_remove_task(
	processor_set_t	pset,
	task_t		task)
{
	if (pset != task->processor_set)
		return;

	queue_remove_generic(&pset->tasks, task,
	    __builtin_offsetof(typeof(*task), pset_tasks));
	task->processor_set = PROCESSOR_SET_NULL;
	pset->task_count--;
}

/*
 *	pset_add_task() adds a task to a processor_set.
 *	Caller must hold locks on pset and task.  Pset references to
 *	tasks are implicit.
 */

void pset_add_task(
	processor_set_t	pset,
	task_t		task)
{
	queue_enter_tail(&pset->tasks, task,
	    __builtin_offsetof(typeof(*task), pset_tasks));
	task->processor_set = pset;
	pset->task_count++;
}

kern_return_t
processor_info(
	processor_t		processor,
	int			flavor,
	host_t			*host,
	processor_info_t	info,
	natural_t		*count)
{
	int				slot_num, state;
	processor_basic_info_t		basic_info;

	if (processor == PROCESSOR_NULL)
		return KERN_INVALID_ARGUMENT;

	if (flavor != PROCESSOR_BASIC_INFO ||
		*count < PROCESSOR_BASIC_INFO_COUNT)
			return KERN_FAILURE;

	basic_info = (processor_basic_info_t) info;

	slot_num = processor->slot_num;
	basic_info->cpu_type = machine_slot[slot_num].cpu_type;
	basic_info->cpu_subtype = machine_slot[slot_num].cpu_subtype;
	state = processor->state;
	if (state == PROCESSOR_SHUTDOWN || state == PROCESSOR_OFF_LINE)
		basic_info->running = FALSE;
	else
		basic_info->running = TRUE;
	basic_info->slot_num = slot_num;
	if (processor == master_processor)
		basic_info->is_master = TRUE;
	else
		basic_info->is_master = FALSE;

	*count = PROCESSOR_BASIC_INFO_COUNT;
	*host = &realhost;
	return KERN_SUCCESS;
}

/*
 *	Precalculate the appropriate system quanta based on load.  The
 *	index into machine_quantum is the number of threads on the
 *	processor set queue.  It is limited to the number of processors in
 *	the set.
 */

void quantum_set(
	processor_set_t	pset)
{
	int	i, ncpus;

	ncpus = pset->processor_count;

	for ( i=1 ; i <= ncpus ; i++) {
		pset->machine_quantum[i] =
			((min_quantum * ncpus) + (i/2)) / i ;
	}
	pset->machine_quantum[0] = 2 * pset->machine_quantum[1];

	i = ((pset->runq.count > pset->processor_count) ?
		pset->processor_count : pset->runq.count);
	pset->set_quantum = pset->machine_quantum[i];
}

#if	MACH_HOST
/*
 *	processor_set_create:
 *
 *	Create and return a new processor set.
 */

kern_return_t
processor_set_create(
	host_t		host,
	processor_set_t *new_set,
	processor_set_t *new_name)
{
	processor_set_t	pset;

	if (host == HOST_NULL)
		return KERN_INVALID_ARGUMENT;

	pset = (processor_set_t) kmem_cache_alloc(&pset_cache);
	pset_init(pset);
	pset_reference(pset);	/* for new_set out argument */
	pset_reference(pset);	/* for new_name out argument */
	ipc_pset_init(pset);
	pset->active = TRUE;

	simple_lock(&all_psets_lock);
	queue_enter_tail(&all_psets, pset,
	    __builtin_offsetof(typeof(*pset), all_psets));
	all_psets_count++;
	simple_unlock(&all_psets_lock);

	ipc_pset_enable(pset);

	*new_set = pset;
	*new_name = pset;
	return KERN_SUCCESS;
}

/*
 *	processor_set_destroy:
 *
 *	destroy a processor set.  Any tasks, threads or processors
 *	currently assigned to it are reassigned to the default pset.
 */
kern_return_t processor_set_destroy(
	processor_set_t pset)
{
	queue_entry_t	elem;
	queue_head_t	*list;

	if (pset == PROCESSOR_SET_NULL || pset == &default_pset)
		return KERN_INVALID_ARGUMENT;

	/*
	 *	Handle multiple termination race.  First one through sets
	 *	active to FALSE and disables ipc access.
	 */
	simple_lock(&(pset)->lock);
	if (!(pset->active)) {
		simple_unlock(&(pset)->lock);
		return KERN_FAILURE;
	}

	pset->active = FALSE;
	ipc_pset_disable(pset);


	/*
	 *	Now reassign everything in this set to the default set.
	 */

	if (pset->task_count > 0) {
	    list = &pset->tasks;
	    while (!queue_empty(list)) {
		elem = queue_first(list);
		task_reference((task_t) elem);
		simple_unlock(&(pset)->lock);
		task_assign((task_t) elem, &default_pset, FALSE);
		task_deallocate((task_t) elem);
		simple_lock(&(pset)->lock);
	    }
	}

	if (pset->thread_count > 0) {
	    list = &pset->threads;
	    while (!queue_empty(list)) {
		elem = queue_first(list);
		thread_reference((thread_t) elem);
		simple_unlock(&(pset)->lock);
		thread_assign((thread_t) elem, &default_pset);
		thread_deallocate((thread_t) elem);
		simple_lock(&(pset)->lock);
	    }
	}

	if (pset->processor_count > 0) {
	    list = &pset->processors;
	    while(!queue_empty(list)) {
		elem = queue_first(list);
		simple_unlock(&(pset)->lock);
		processor_assign((processor_t) elem, &default_pset, TRUE);
		simple_lock(&(pset)->lock);
	    }
	}

	simple_unlock(&(pset)->lock);

	/*
	 *	Destroy ipc state.
	 */
	ipc_pset_terminate(pset);

	/*
	 *	Deallocate pset's reference to itself.
	 */
	pset_deallocate(pset);
	return KERN_SUCCESS;
}

#else	/* MACH_HOST */

kern_return_t
processor_set_create(
	host_t		host,
	processor_set_t *new_set,
	processor_set_t *new_name)
{
	return KERN_FAILURE;
}

kern_return_t processor_set_destroy(
	processor_set_t pset)
{
	return KERN_FAILURE;
}

#endif	/* MACH_HOST */

kern_return_t
processor_set_info(
	processor_set_t		pset,
	int			flavor,
	host_t			*host,
	processor_set_info_t	info,
	natural_t		*count)
{
	if (pset == PROCESSOR_SET_NULL)
		return KERN_INVALID_ARGUMENT;

	if (flavor == PROCESSOR_SET_BASIC_INFO) {
		processor_set_basic_info_t	basic_info;

		if (*count < PROCESSOR_SET_BASIC_INFO_COUNT)
			return KERN_FAILURE;

		basic_info = (processor_set_basic_info_t) info;

		simple_lock(&(pset)->lock);
		basic_info->processor_count = pset->processor_count;
		basic_info->task_count = pset->task_count;
		basic_info->thread_count = pset->thread_count;
		basic_info->mach_factor = pset->mach_factor;
		basic_info->load_average = pset->load_average;
		simple_unlock(&(pset)->lock);

		*count = PROCESSOR_SET_BASIC_INFO_COUNT;
		*host = &realhost;
		return KERN_SUCCESS;
	}
	else if (flavor == PROCESSOR_SET_SCHED_INFO) {
		processor_set_sched_info_t	sched_info;

		if (*count < PROCESSOR_SET_SCHED_INFO_COUNT)
			return KERN_FAILURE;

		sched_info = (processor_set_sched_info_t) info;

		simple_lock(&(pset)->lock);
		sched_info->policies = pset->policies;
		sched_info->max_priority = pset->max_priority;
		simple_unlock(&(pset)->lock);

		*count = PROCESSOR_SET_SCHED_INFO_COUNT;
		*host = &realhost;
		return KERN_SUCCESS;
	}

	*host = HOST_NULL;
	return KERN_INVALID_ARGUMENT;
}

#define THING_TASK	0
#define THING_THREAD	1

/*
 *	processor_set_things:
 *
 *	Common internals for processor_set_{threads,tasks}
 */
static kern_return_t
processor_set_things(
	processor_set_t	pset,
	mach_port_t	**thing_list,
	natural_t	*count,
	int		type)
{
	unsigned int actual;	/* this many things */
	unsigned i;

	vm_size_t size, size_needed;
	vm_offset_t addr;

	if (pset == PROCESSOR_SET_NULL)
		return KERN_INVALID_ARGUMENT;

	size = 0; addr = 0;

	for (;;) {
		simple_lock(&(pset)->lock);
		if (!pset->active) {
			simple_unlock(&(pset)->lock);
			return KERN_FAILURE;
		}

		if (type == THING_TASK)
			actual = pset->task_count;
		else
			actual = pset->thread_count;

		/* do we have the memory we need? */

		size_needed = actual * sizeof(mach_port_t);
		if (size_needed <= size)
			break;

		/* unlock the pset and allocate more memory */
		simple_unlock(&(pset)->lock);

		if (size != 0)
			kfree(addr, size);

		size = size_needed;

		addr = kalloc(size);
		if (addr == 0)
			return KERN_RESOURCE_SHORTAGE;
	}

	/* OK, have memory and the processor_set is locked & active */

	switch (type) {
	    case THING_TASK: {
		task_t *tasks = (task_t *) addr;
		task_t task;

		for (i = 0, task = (task_t) queue_first(&pset->tasks);
		     i < actual;
		     i++, task = (task_t) queue_next(&task->pset_tasks)) {
			/* take ref for convert_task_to_port */
			task_reference(task);
			tasks[i] = task;
		}
		break;
	    }

	    case THING_THREAD: {
		thread_t *threads = (thread_t *) addr;
		thread_t thread;

		for (i = 0, thread = (thread_t) queue_first(&pset->threads);
		     i < actual;
		     i++,
		     thread = (thread_t) queue_next(&thread->pset_threads)) {
			/* take ref for convert_thread_to_port */
			thread_reference(thread);
			threads[i] = thread;
		}
		break;
	    }
	}

	/* can unlock processor set now that we have the task/thread refs */
	simple_unlock(&(pset)->lock);

	if (actual == 0) {
		/* no things, so return null pointer and deallocate memory */
		*thing_list = 0;
		*count = 0;

		if (size != 0)
			kfree(addr, size);
	} else {
		/* if we allocated too much, must copy */

		if (size_needed < size) {
			vm_offset_t newaddr;

			newaddr = kalloc(size_needed);
			if (newaddr == 0) {
				switch (type) {
				    case THING_TASK: {
					task_t *tasks = (task_t *) addr;

					for (i = 0; i < actual; i++)
						task_deallocate(tasks[i]);
					break;
				    }

				    case THING_THREAD: {
					thread_t *threads = (thread_t *) addr;

					for (i = 0; i < actual; i++)
						thread_deallocate(threads[i]);
					break;
				    }
				}
				kfree(addr, size);
				return KERN_RESOURCE_SHORTAGE;
			}

			memcpy((void *) newaddr, (void *) addr, size_needed);
			kfree(addr, size);
			addr = newaddr;
		}

		*thing_list = (mach_port_t *) addr;
		*count = actual;

		/* do the conversion that Mig should handle */

		switch (type) {
		    case THING_TASK: {
			task_t *tasks = (task_t *) addr;

			for (i = 0; i < actual; i++)
			    ((mach_port_t *) tasks)[i] =
				(mach_port_t)convert_task_to_port(tasks[i]);
			break;
		    }

		    case THING_THREAD: {
			thread_t *threads = (thread_t *) addr;

			for (i = 0; i < actual; i++)
			    ((mach_port_t *) threads)[i] =
				(mach_port_t)convert_thread_to_port(threads[i]);
			break;
		    }
		}
	}

	return KERN_SUCCESS;
}


/*
 *	processor_set_tasks:
 *
 *	List all tasks in the processor set.
 */
kern_return_t
processor_set_tasks(
	processor_set_t	pset,
	task_array_t	*task_list,
	natural_t	*count)
{
	return processor_set_things(pset, task_list, count, THING_TASK);
}

/*
 *	processor_set_threads:
 *
 *	List all threads in the processor set.
 */
kern_return_t
processor_set_threads(
	processor_set_t	pset,
	thread_array_t	*thread_list,
	natural_t	*count)
{
	return processor_set_things(pset, thread_list, count, THING_THREAD);
}
