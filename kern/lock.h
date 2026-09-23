/* 
 * Mach Operating System
 * Copyright (c) 1993-1987 Carnegie Mellon University
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
 *	File:	kern/lock.h
 *	Author:	Avadis Tevanian, Jr., Michael Wayne Young
 *	Date:	1985
 *
 *	Locking primitives definitions
 */

#ifndef	_KERN_LOCK_H_
#define	_KERN_LOCK_H_

#include <mach/boolean.h>
#include <mach/machine/vm_types.h>
#include <machine/spl.h>

/*
 * Note: we cannot blindly use simple locks in interrupt handlers, otherwise one
 * may try to acquire a lock while already having the lock, thus a deadlock.
 *
 * When locks are needed in interrupt handlers, the _irq versions of the calls
 * should be used, which disable interrupts (by calling splhigh) before acquiring
 * the lock, thus preventing the deadlock. They need to be used this way:
 *
 * spl_t s = simple_lock_irq(&mylock);
 * [... critical section]
 * simple_unlock_irq(s, &mylock);
 *
 * In the following, the _irq versions disable interrupts and the plain
 * versions are the ones to use in ordinary code.  Both are built on
 * the machine entry points below, which src/kern/lock.rs defines.
 */

/*
 *	A simple spin lock.
 */

struct slock {
	volatile natural_t lock_data;	/* in general 1 bit is sufficient */
	struct {} is_a_simple_lock;
};

/*
 *	Used by macros to assert that the given argument is a simple
 *	lock.
 */
#define simple_lock_assert(l)	(void) &(l)->is_a_simple_lock

typedef struct slock	simple_lock_data_t;
typedef struct slock	*simple_lock_t;

/*
 *	Reset a simple lock to unlocked.  Both spellings came from the
 *	machine header, which is gone.
 */
#define SIMPLE_LOCK_INITIALIZER(l) \
	{.lock_data = 0}
#define simple_lock_init(l) \
	((l)->lock_data = 0)

/*
 *	The machine half of a simple lock, defined in src/kern/lock.rs.
 *	The macros below are its only callers.
 */
extern void mach_simple_lock(simple_lock_t);
extern void mach_simple_unlock(simple_lock_t);
extern int mach_simple_lock_try(simple_lock_t);

extern void simple_lock_pause(void);

/*
 *	Use the locks.
 */

#define	decl_simple_lock_data(class,name) \
class	simple_lock_data_t	name;
#define	def_simple_lock_data(class,name) \
class	simple_lock_data_t	name = SIMPLE_LOCK_INITIALIZER(&name);
#define	def_simple_lock_irq_data(class,name) \
class	simple_lock_irq_data_t	name = { SIMPLE_LOCK_INITIALIZER(&name.lock) };

#define	simple_lock_addr(lock)	(simple_lock_assert(&(lock)),	\
				 &(lock))
#define	simple_lock_irq_addr(l)	(simple_lock_irq_assert(&(l)),	\
				&(l)->lock)



/*
 *	The general lock structure.  Provides for multiple readers,
 *	upgrading from read to write, and sleeping until the lock
 *	can be gained.
 *
 *	On some architectures, assembly language code in the 'inline'
 *	program fiddles the lock structures.  It must be changed in
 *	concert with the structure layout.
 *
 *	Only the "interlock" field is used for hardware exclusion;
 *	other fields are modified with normal instructions after
 *	acquiring the interlock bit.
 */
struct lock {
	struct thread	*thread;	/* Thread that has lock, if
					   recursive locking allowed */
	unsigned int	read_count:16,	/* Number of accepted readers */
	/* boolean_t */	want_upgrade:1,	/* Read-to-write upgrade waiting */
	/* boolean_t */	want_write:1,	/* Writer is waiting, or
					   locked for write */
	/* boolean_t */	waiting:1,	/* Someone is sleeping on lock */
	/* boolean_t */	can_sleep:1,	/* Can attempts to lock go to sleep? */
			recursion_depth:12, /* Depth of recursion */
			:0; 
	decl_simple_lock_data(,interlock)
					/* Hardware interlock field.
					   Last in the structure so that
					   field offsets are the same whether
					   or not it is present. */
};

typedef struct lock	lock_data_t;
typedef struct lock	*lock_t;

/* Sleep locks must work even if no multiprocessing */

extern void		lock_init(lock_t, boolean_t);
extern void		lock_write(lock_t);
extern void		lock_read(lock_t);
extern void		lock_done(lock_t);
extern boolean_t	lock_read_to_write(lock_t);
extern void		lock_write_to_read(lock_t);
extern boolean_t	lock_try_write(lock_t);
extern boolean_t	lock_try_read(lock_t);
extern boolean_t	lock_try_read_to_write(lock_t);

#define	lock_read_done(l)	lock_done(l)
#define	lock_write_done(l)	lock_done(l)

extern void		lock_set_recursive(lock_t);
extern void		lock_clear_recursive(lock_t);

/* Lock debugging support.  */
#define lock_check_no_interrupts()

#define simple_lock(l)		\
MACRO_BEGIN \
	lock_check_no_interrupts(); \
	mach_simple_lock(l); \
MACRO_END
#define simple_lock_try(l)	({ \
	mach_simple_lock_try(l); \
})
#define simple_unlock(l)	\
MACRO_BEGIN \
	mach_simple_unlock(l); \
MACRO_END

/*
 *	The machine header's spellings, kept for the C callers that
 *	still use them directly.  Each goes when its last caller moves
 *	to the plain macros above or to the Rust SimpleLock.
 */
#define _simple_lock(l)		mach_simple_lock(l)
#define _simple_unlock(l)	mach_simple_unlock(l)

/* _irq variants */

struct slock_irq {
	struct slock slock;
};

#define simple_lock_irq_assert(l)	simple_lock_assert(&(l)->slock)

typedef struct slock_irq	simple_lock_irq_data_t;
typedef struct slock_irq	*simple_lock_irq_t;

#define	decl_simple_lock_irq_data(class,name) \
class	simple_lock_irq_data_t	name;

#define simple_lock_init_irq(l) simple_lock_init(&(l)->slock)

#define simple_lock_irq(l)	({ \
	spl_t __s = splhigh(); \
	mach_simple_lock(&(l)->slock); \
	__s; \
})
#define simple_unlock_irq(s, l)	\
MACRO_BEGIN \
	mach_simple_unlock(&(l)->slock); \
	splx(s); \
MACRO_END


#endif	/* _KERN_LOCK_H_ */
