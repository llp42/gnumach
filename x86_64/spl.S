/* 
 * Copyright (c) 1995 Shantanu Goel
 * All Rights Reserved.
 * 
 * Permission to use, copy, modify and distribute this software and its
 * documentation is hereby granted, provided that both the copyright
 * notice and this permission notice appear in all copies of the
 * software, derivative works or modified versions, and any portions
 * thereof, and that both notices appear in supporting documentation.
 * 
 * THE AUTHOR ALLOWS FREE USE OF THIS SOFTWARE IN ITS "AS IS"
 * CONDITION.  THE AUTHOR DISCLAIMS ANY LIABILITY OF ANY KIND FOR
 * ANY DAMAGES WHATSOEVER RESULTING FROM THE USE OF THIS SOFTWARE.
 */

/*
 * spl routines for the i386at.
 */

#include <mach/machine/asm.h>
#include <i386/i386/ipl.h>
#include <i386/i386/i386asm.h>
#include <i386/cpu_number.h>

#if NCPUS > 1
#define mb lock; addl $0,(%rsp)
#else
#define mb
#endif

ENTRY(spl0)
	mb;
	CPU_NUMBER(%edx)
	movl	CX(EXT(curr_ipl),%rdx),%eax	/* save current ipl */
	pushq	%rax
	cli				/* disable interrupts */
	cmpl	$0,softclkpending	/* softclock pending? */
	je	1f			/* no, skip */
	movl	$0,softclkpending	/* clear flag */
	call	EXT(spl1)		/* block further interrupts */
	call	EXT(softclock)		/* go handle interrupt */
	cli				/* disable interrupts */
1:
	CPU_NUMBER(%edx)
	cmpl	$(SPL0),CX(EXT(curr_ipl),%rdx)	/* are we at spl0? */
	je	1f				/* yes, all done */
	movl	$(SPL0),CX(EXT(curr_ipl),%rdx)	/* set ipl */
1:
	sti				/* enable interrupts */
	popq	%rax			/* return previous mask */
	ret


/*
 * Historically, SETIPL(level) was called
 * for spl levels 1-6, now we have combined
 * all the intermediate levels into the highest level
 * such that interrupts are either on or off,
 * since modern hardware can handle it.
 * This simplifies the interrupt handling
 * especially for the linux drivers.
 */
Entry(splsoftclock)
ENTRY(spl1)
ENTRY(spl2)
ENTRY(spl3)
Entry(splnet)
Entry(splhdw)
ENTRY(spl4)
Entry(splbio)
Entry(spldcm)
ENTRY(spl5)
Entry(spltty)
Entry(splimp)
Entry(splvm)
ENTRY(spl6)
Entry(splclock)
Entry(splsched)
Entry(splhigh)
Entry(splhi)
ENTRY(spl7)
	mb;
	/* just clear IF */
	cli
	CPU_NUMBER(%edx)
	movl    $SPL7,%eax
	xchgl   CX(EXT(curr_ipl),%rdx),%eax
	ret

ENTRY(splx)
	movq	S_ARG0,%rdx		/* get ipl */
	CPU_NUMBER(%eax)
	testl	%edx,%edx		/* spl0? */
	jz	EXT(spl0)		/* yes, handle specially */
	CPU_NUMBER(%eax)
	cmpl	CX(EXT(curr_ipl),%rax),%edx	/* same ipl as current? */
	jne	spl			/* no */
	cmpl	$SPL7,%edx		/* spl7? */
	je	1f			/* to ipl7, don't enable interrupts */
	sti				/* ensure interrupts are enabled */
1:
	movl	%edx,%eax		/* return previous ipl */
	ret

/*
 * Like splx() but returns with interrupts disabled and does
 * not return the previous ipl.  This should only be called
 * when returning from an interrupt.
 */
	.align	TEXT_ALIGN
	.globl	splx_cli
splx_cli:
	movq	S_ARG0,%rdx		/* get ipl */
	cli				/* disable interrupts */
	testl	%edx,%edx		/* spl0? */
	jnz	2f			/* no, skip */
	cmpl	$0,softclkpending	/* softclock pending? */
	je	1f			/* no, skip */
	movl	$0,softclkpending	/* clear flag */
	call	EXT(spl1)		/* block further interrupts */
	call	EXT(softclock)		/* go handle interrupt */
	cli				/* disable interrupts */
1:
	xorl	%edx,%edx		/* edx = ipl 0 */
2:
	CPU_NUMBER(%eax)
	cmpl	CX(EXT(curr_ipl),%rax),%edx	/* same ipl as current? */
	je	1f				/* yes, all done */
	movl	%edx,CX(EXT(curr_ipl),%rax)	/* set ipl */
1:
	ret

/*
 * NOTE: This routine must *not* use %ecx, otherwise
 * the interrupt code will break.
 */
	.align	TEXT_ALIGN
	.globl	spl
spl:
	CPU_NUMBER(%eax)
	cmpl	$SPL7,%edx		/* spl7? */
	je	EXT(spl7)		/* yes, handle specially */
	cli				/* disable interrupts */
	CPU_NUMBER(%eax)
	xchgl	CX(EXT(curr_ipl),%rax),%edx	/* set ipl */
	sti				/* enable interrupts */
	movl	%edx,%eax		/* return previous ipl */
	ret

ENTRY(sploff)
	pushfq
	popq	%rax
	cli
	ret

ENTRY(splon)
	pushq	S_ARG0
	popfq
	ret

	.data
	.align	DATA_ALIGN
softclkpending:
	.long	0
	.text

ENTRY(setsoftclock)
	incl	softclkpending
	ret
