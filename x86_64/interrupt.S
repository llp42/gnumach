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

#define S_REGS 24(%rsp)
#define S_RET 16(%rsp)
#define S_IRQ 8(%rsp)
#define S_IPL 0(%rsp)

ENTRY(interrupt)
#ifdef APIC
	cmpl	$255,%eax		/* was this a spurious intr? */
	jne	1f
	ret				/* if so, just return */
1:
#endif
	subq	$16,%rsp		/* Two local variables */
	movl	%eax,S_IRQ		/* save irq number */

	call	spl7			/* set ipl */
	movl	%eax,S_IPL		/* save previous ipl */

	movl	S_IRQ,%ecx		/* restore irq number */

	cmpl	$CALL_PMAP_UPDATE,%ecx	/* was this a SMP pmap_update request? */
	je	_call_single

	cmpl	$CALL_AST_CHECK,%ecx	/* was this a SMP remote -> local ast request? */
	je	_call_local_ast

	movb	EXT(irqinfo)(,%rcx,2),%al	/* look up irq_info[irq].trigger */
	testb	$1,%al			/* was this a level triggered interrupt? */
	jnz	_call_handler		/* yes: call handler before eoi */

_eoi:
	movl	%ecx,%edi		/* load irq number as 1st arg */
	call	EXT(ioapic_irq_eoi)	/* ioapic irq specific EOI */

	movl	S_IRQ,%ecx
	movb	EXT(irqinfo)(,%rcx,2),%al	/* look up irq_info[irq].trigger */
	testb	$1,%al			/* was this a level triggered interrupt? */
	jnz	_completed		/* yes: we are done */

_call_handler:
	;
	movq	S_IPL,S_ARG1		/* previous ipl as 2nd arg */

	;
	movq	S_RET,S_ARG2		/* return address as 3th arg */

	;
	movq	S_REGS,S_ARG3		/* address of interrupted registers as 4th arg */

	movl	S_IRQ,%eax		/* copy irq number */
	movl	EXT(iunit)(,%rax,4),%edi	/* get device unit number as 1st arg */

	call	*EXT(ivect)(,%rax,8)	/* call interrupt handler */

	movl	S_IRQ,%ecx
	movb	EXT(irqinfo)(,%rcx,2),%al	/* look up irq_info[irq].trigger */
	testb	$1,%al			/* was this a level triggered interrupt? */
	jnz	_eoi			/* yes: eoi */

_completed:
	movl	S_IPL,%edi		/* restore previous ipl */
	call	splx_cli		/* restore previous ipl */

	addq	$16,%rsp		/* pop local variables */
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
