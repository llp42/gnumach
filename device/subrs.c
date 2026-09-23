/* 
 * Mach Operating System
 * Copyright (c) 1993,1991,1990,1989,1988 Carnegie Mellon University
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
 * Random device subroutines and stubs.
 */

#include <kern/debug.h>
#include <kern/printf.h>
#include <vm/vm_kern.h>
#include <vm/vm_user.h>
#include <device/if_hdr.h>
#include <device/if_ether.h>
#include <device/subrs.h>



/*
 * Initialize send and receive queues on an interface.
 */
void if_init_queues(struct ifnet *ifp)
{
	IFQ_INIT(&ifp->if_snd);
	queue_init(&ifp->if_rcv_port_list);
	queue_init(&ifp->if_snd_port_list);
	simple_lock_init(&ifp->if_rcv_port_list_lock);
	simple_lock_init(&ifp->if_snd_port_list_lock);
}
