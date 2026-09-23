/*
 * Mach Operating System
 * Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
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
 *	kern/ipc_host.c
 *
 *	Routines to implement host ports.
 */

#include <mach/message.h>
#include <kern/debug.h>
#include <kern/host.h>
#include <kern/mach_host.server.h>
#include <kern/macros.h>
#include <kern/processor.h>
#include <kern/task.h>
#include <kern/thread.h>
#include <kern/ipc_host.h>
#include <kern/ipc_kobject.h>
#include <ipc/ipc_port.h>
#include <ipc/ipc_space.h>
#include <mach/mach_traps.h>

#include <machine/spl.h>	/* for spl */



/*
 *	ipc_host_init: set up various things.
 */

void ipc_host_init(void)
{
	ipc_port_t	port;
	/*
	 *	Allocate and set up the two host ports.
	 */
	port = ipc_port_alloc_kernel();
	if (port == IP_NULL)
		panic("ipc_host_init");

	ipc_kobject_set(port, (ipc_kobject_t) &realhost, IKOT_HOST);
	realhost.host_self = port;

	port = ipc_port_alloc_kernel();
	if (port == IP_NULL)
		panic("ipc_host_init");

	ipc_kobject_set(port, (ipc_kobject_t) &realhost, IKOT_HOST_PRIV);
	realhost.host_priv_self = port;

	/*
	 *	Set up ipc for default processor set.
	 */
	ipc_pset_init(&default_pset);
	ipc_pset_enable(&default_pset);

	/*
	 *	And for master processor
	 */
	ipc_processor_init(master_processor);
}

/*
 *	Routine:	mach_host_self [mach trap]
 *	Purpose:
 *		Give the caller send rights for his own host port.
 *	Conditions:
 *		Nothing locked.
 *	Returns:
 *		MACH_PORT_NULL if there are any resource failures
 *		or other errors.
 */

mach_port_name_t
mach_host_self(void)
{
	ipc_port_t sright;

	sright = ipc_port_make_send(realhost.host_self);
	return ipc_port_copyout_send(sright, current_space());
}

/*
 *	Routine:	convert_port_to_host
 *	Purpose:
 *		Convert from a port to a host.
 *		Doesn't consume the port ref; the host produced may be null.
 *	Conditions:
 *		Nothing locked.
 */

host_t
convert_port_to_host(
	ipc_port_t	port)
{
	host_t host = HOST_NULL;

	if (likely(IP_VALID(port))) {
		ip_lock(port);
		if (ip_active(port) &&
		    ((ip_kotype(port) == IKOT_HOST) ||
		     (ip_kotype(port) == IKOT_HOST_PRIV)))
			host = (host_t) port->ip_kobject;
		ip_unlock(port);
	}

	return host;
}

/*
 *	Routine:	convert_port_to_host_priv
 *	Purpose:
 *		Convert from a port to a host.
 *		Doesn't consume the port ref; the host produced may be null.
 *	Conditions:
 *		Nothing locked.
 */

host_t
convert_port_to_host_priv(
	ipc_port_t	port)
{
	host_t host = HOST_NULL;

	if (likely(IP_VALID(port))) {
		ip_lock(port);
		if (ip_active(port) &&
		    (ip_kotype(port) == IKOT_HOST_PRIV))
			host = (host_t) port->ip_kobject;
		ip_unlock(port);
	}

	return host;
}

/*
 *	Routine:	convert_port_to_processor
 *	Purpose:
 *		Convert from a port to a processor.
 *		Doesn't consume the port ref;
 *		the processor produced may be null.
 *	Conditions:
 *		Nothing locked.
 */

processor_t
convert_port_to_processor(
	ipc_port_t	port)
{
	processor_t processor = PROCESSOR_NULL;

	if (likely(IP_VALID(port))) {
		ip_lock(port);
		if (ip_active(port) &&
		    (ip_kotype(port) == IKOT_PROCESSOR))
			processor = (processor_t) port->ip_kobject;
		ip_unlock(port);
	}

	return processor;
}

/*
 *      Routine:        convert_port_to_processor_name
 *      Purpose:
 *              Convert from a port to a processor.
 *              Doesn't consume the port ref;
 *              the processor produced may be null.
 *      Conditions:
 *              Nothing locked.
 */

processor_t
convert_port_to_processor_name(
	ipc_port_t      port)
{
	processor_t processor = PROCESSOR_NULL;

	if (likely(IP_VALID(port))) {
		ip_lock(port);
		if (ip_active(port) &&
		    ((ip_kotype(port) == IKOT_PROCESSOR) ||
		     (ip_kotype(port) == IKOT_PROCESSOR_NAME))) {
		        processor = (processor_t) port->ip_kobject;
		}
		ip_unlock(port);
	}

	return processor;
}

/*
 *	Routine:	convert_port_to_pset
 *	Purpose:
 *		Convert from a port to a pset.
 *		Doesn't consume the port ref; produces a pset ref,
 *		which may be null.
 *	Conditions:
 *		Nothing locked.
 */

processor_set_t
convert_port_to_pset(
	ipc_port_t	port)
{
	processor_set_t pset = PROCESSOR_SET_NULL;

	if (likely(IP_VALID(port))) {
		ip_lock(port);
		if (ip_active(port) &&
		    (ip_kotype(port) == IKOT_PSET)) {
			pset = (processor_set_t) port->ip_kobject;
			pset_reference(pset);
		}
		ip_unlock(port);
	}

	return pset;
}

/*
 *	Routine:	convert_port_to_pset_name
 *	Purpose:
 *		Convert from a port to a pset.
 *		Doesn't consume the port ref; produces a pset ref,
 *		which may be null.
 *	Conditions:
 *		Nothing locked.
 */

processor_set_t
convert_port_to_pset_name(
	ipc_port_t	port)
{
	processor_set_t pset = PROCESSOR_SET_NULL;

	if (likely(IP_VALID(port))) {
		ip_lock(port);
		if (ip_active(port) &&
		    ((ip_kotype(port) == IKOT_PSET) ||
		     (ip_kotype(port) == IKOT_PSET_NAME))) {
			pset = (processor_set_t) port->ip_kobject;
			pset_reference(pset);
		}
		ip_unlock(port);
	}

	return pset;
}

/*
 *	Routine:	convert_host_to_port
 *	Purpose:
 *		Convert from a host to a port.
 *		Produces a naked send right which may be invalid.
 *	Conditions:
 *		Nothing locked.
 */

ipc_port_t
convert_host_to_port(
	host_t		host)
{
	ipc_port_t port;

	port = ipc_port_make_send(host->host_self);

	return port;
}

/*
 *	Routine:	convert_processor_to_port
 *	Purpose:
 *		Convert from a processor to a port.
 *		Produces a naked send right which is always valid.
 *	Conditions:
 *		Nothing locked.
 */

ipc_port_t
convert_processor_to_port(processor_t processor)
{
	ipc_port_t port;

	port = ipc_port_make_send(processor->processor_self);

	return port;
}

/*
 *      Routine:        convert_processor_name_to_port
 *      Purpose:
 *              Convert from a processor to a port.
 *              Produces a naked send right which is always valid.
 *      Conditions:
 *              Nothing locked.
 */

ipc_port_t
convert_processor_name_to_port(processor_t processor)
{
        ipc_port_t port;

        port = ipc_port_make_send(processor->processor_name_self);

        return port;
}

/*
 *	Routine:	convert_pset_to_port
 *	Purpose:
 *		Convert from a pset to a port.
 *		Consumes a pset ref; produces a naked send right
 *		which may be invalid.
 *	Conditions:
 *		Nothing locked.
 */

ipc_port_t
convert_pset_to_port(
	processor_set_t		pset)
{
	ipc_port_t port;

	simple_lock(&(pset)->lock);
	if (pset->active)
		port = ipc_port_make_send(pset->pset_self);
	else
		port = IP_NULL;
	simple_unlock(&(pset)->lock);

	pset_deallocate(pset);
	return port;
}

/*
 *	Routine:	convert_pset_name_to_port
 *	Purpose:
 *		Convert from a pset to a port.
 *		Consumes a pset ref; produces a naked send right
 *		which may be invalid.
 *	Conditions:
 *		Nothing locked.
 */

ipc_port_t
convert_pset_name_to_port(
	processor_set_t		pset)
{
	ipc_port_t port;

	simple_lock(&(pset)->lock);
	if (pset->active)
		port = ipc_port_make_send(pset->pset_name_self);
	else
		port = IP_NULL;
	simple_unlock(&(pset)->lock);

	pset_deallocate(pset);
	return port;
}
