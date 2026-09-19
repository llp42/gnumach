/* 
 * Mach Operating System
 * Copyright (c) 1991,1990,1989 Carnegie Mellon University
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
 *	Author: David B. Golub, Carnegie Mellon University
 *	Date: 	3/90
 *
 * 	Definitions to make new IO structures look like old ones
 */

#ifndef _DEVICE_BUF_H_
#define _DEVICE_BUF_H_

/*
 * io_req and fields
 */
#include <device/io_req.h>

#define	buf	io_req

/*
 * Redefine flags
 */
#define	B_DONE		IO_DONE
#define	B_ERROR		IO_ERROR
#define	B_BUSY		IO_BUSY
#define	B_WANTED	IO_WANTED
#define	B_BAD		IO_BAD
#define	B_CALL		IO_CALL

#define	B_MD1		IO_SPARE_START

/*
 * Export standard minphys routine.
 */
extern void minphys(io_req_t);

/*
 * Alternate name for iodone
 */
#define	biodone	iodone
#define biowait iowait

#endif /* _DEVICE_BUF_H_ */
