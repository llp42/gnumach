/* SPDX-License-Identifier: CMU-Mach */
/* Derived from kern/processor.c and kern/processor.h: */
/*   Copyright (c) 1993-1988 Carnegie Mellon University. */
/*   Copyright (c) 1991,1990,1989 Carnegie Mellon University. */
/*   Copyright (c) 1993,1994 The University of Utah and the Computer */
/*   Systems Laboratory (CSL). */
/* Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com> */

/*
 * The shims the Rust port of kern/processor.c needs: the tail of
 * `struct processor_set`, from `machine_quantum` through `sched_load`,
 * whose offset depends on the configure-time NCPUS and so cannot be
 * named in Rust.  They die when NCPUS is visible to Rust and the
 * `ProcessorSet` mirror can carry the tail.  See
 * rust/src/kern/processor.rs and rust/src/glue/mod.rs.
 */

#include <kern/processor.h>
#include <kern/sched.h>

void processor_glue_pset_tail_init(processor_set_t pset, int quantum);

void
processor_glue_pset_tail_init(
	processor_set_t	pset,
	int		quantum)
{
	int i;

	for (i = 0; i <= NCPUS; i++) {
	    pset->machine_quantum[i] = quantum;
	}
	pset->mach_factor = 0;
	pset->load_average = 0;
	pset->sched_load = SCHED_SCALE;		/* i.e. 1 */
}

int *processor_glue_pset_machine_quantum(processor_set_t pset);

/*
 * The head of the tail, `machine_quantum`, which the Rust
 * `quantum_set()` writes through.  It dies when NCPUS is visible to
 * Rust and the `ProcessorSet` mirror can carry the tail.
 */
int *
processor_glue_pset_machine_quantum(
	processor_set_t	pset)
{
	return pset->machine_quantum;
}
