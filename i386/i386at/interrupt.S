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
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 */

#include <mach/machine/asm.h>

#include <i386/ipl.h>
#include <i386/apic.h>
#include <i386/i386asm.h>

/*
 * Generic interrupt handler.
 *
 * On entry, %eax contains the irq number.
 *
 */

#define S_REGS 28(%esp)
#define S_RET 24(%esp)
#define S_IRQ 20(%esp)
#define S_IPL 16(%esp)

ENTRY(interrupt)
#ifdef APIC
	cmpl	$255,%eax		/* was this a spurious intr? */
	jne	1f
	ret				/* if so, just return */
1:
#endif
	subl	$24,%esp		/* Two local variables + 4 parameters */
	movl	%eax,S_IRQ		/* save irq number */

	call	spl7			/* set ipl */
	movl	%eax,S_IPL		/* save previous ipl */

	movl	S_IRQ,%ecx		/* restore irq number */

	cmpl	$CALL_PMAP_UPDATE,%ecx	/* was this a SMP pmap_update request? */
	je	_call_single

	cmpl	$CALL_AST_CHECK,%ecx	/* was this a SMP remote -> local ast request? */
	je	_call_local_ast

	movb	EXT(irqinfo)(,%ecx,2),%al	/* look up irq_info[irq].trigger */
	testb	$1,%al			/* was this a level triggered interrupt? */
	jnz	_call_handler		/* yes: handle before eoi */

_eoi:
	movl	%ecx,(%esp)		/* load irq number as 1st arg */
	call	EXT(ioapic_irq_eoi)	/* ioapic irq specific EOI */

	movl	S_IRQ,%ecx		/* restore irq number */
	movb	EXT(irqinfo)(,%ecx,2),%al	/* look up irq_info[irq].trigger */
	testb	$1,%al			/* was this a level triggered interrupt? */
	jnz	_completed		/* yes: we are done */

_call_handler:
	movl	S_IPL,%eax
	movl	%eax,4(%esp)		/* previous ipl as 2nd arg */

	movl	S_RET,%eax
	movl	%eax,8(%esp)		/* return address as 3rd arg */

	movl	S_REGS,%eax
	movl	%eax,12(%esp)		/* address of interrupted registers as 4th arg */

	movl	S_IRQ,%eax		/* copy irq number */

	movl	EXT(iunit)(,%eax,4),%edx	/* get device unit number */
	movl	%edx,(%esp)		/* unit number as 1st arg */

	call	*EXT(ivect)(,%eax,4)	/* call interrupt handler */

	movl	S_IRQ,%ecx		/* restore irq number */
	movb	EXT(irqinfo)(,%ecx,2),%al	/* look up irq_info[irq].trigger */
	testb	$1,%al			/* was this a level triggered interrupt? */
	jnz	_eoi			/* yes: eoi */

_completed:
	movl	S_IPL,%eax		/* restore previous ipl */
	movl	%eax,(%esp)
	call	splx_cli		/* restore previous ipl */

	addl	$24,%esp		/* pop local variables */
	ret

_call_single:
	call	EXT(lapic_eoi)		/* lapic EOI before the handler to allow extra update */
	call	EXT(pmap_update_interrupt)
	jmp	_completed

_call_local_ast:
	call	EXT(lapic_eoi)		/* lapic EOI */
	call	EXT(ast_check)		/* AST check on this cpu */
	jmp	_completed

END(interrupt)
