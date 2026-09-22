/* SPDX-License-Identifier: BSD-2-Clause
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 *
 * The ast_on() macro of <kern/ast.h> in a form Rust can call; a
 * function-like macro cannot be declared in rust/src/glue.rs.  Delete
 * this file once kern/ast.c's need_ast[] belongs to Rust.  See
 * rust/src/kern/sched_prim.rs and rust/src/glue.rs.
 */

#include <kern/ast.h>

void ast_on_cpu(int cpu, ast_t reasons);

void
ast_on_cpu(int cpu, ast_t reasons)
{
	ast_on(cpu, reasons);
}
