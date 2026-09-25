# AGENTS.md — GNU Mach: the C-to-Rust port, and the rules

> Guidance for AI coding agents working in this repo. Read this first.
> **Before building, changing, or debugging anything: verify live state.**
> Read the actual module, the actual `Makefrag.am`, the actual C header you
> are mirroring. `MIGRATE.md` beside this file is the map — every translation
> unit, its blockers, and the six layers of coupling. This file is the rules.

<!-- agents-md:begin id=communication -->
## Talking to the human

How to format replies. The reader works in a terminal and finds dense walls
of text hard to parse.

- Lead with the answer in one sentence. Put the detail after it.
- Short paragraphs, 2 to 3 sentences max, with a blank line between them.
- Bold at most one thing per message. If everything is bold, nothing stands out.
- Three or more items become a list, not a packed paragraph.
- No em-dashes. Use a period or a comma.
- No emojis in replies unless the human asks for them.
- Plain language over jargon. Explain as if the reader is smart but not deep
  in this stack.
- Keep replies short. If a reply would run long, give the headline and ask
  before expanding.
- End with at most one question.

Two project-specific additions:

- **Report test results verbatim.** "Both architectures green" is a claim
  about `mise run test`. If you did not run it, say you did not run it.
- **Name the file and line.** `rust/src/vm/vm_map.rs:412` is clickable; "the
  map code" is not.
<!-- agents-md:end id=communication -->

<!-- agents-md:begin id=overview -->
## Overview

GNU Mach is a microkernel being migrated from C to **native Rust**, one
routine at a time. The target is a Rust kernel, not a kernel with Rust in it.

The build is GNU Autotools plus a hand-written `rustc` invocation — no Cargo,
no lock file, no network. The Rust half compiles to `libmach-rs.a`, which is
linked between two passes over `libkernel.a`, so Rust may call C and C may
call Rust. As of 2026-09-25 the Rust half is 217 files and about 109,800
lines.
lines.

### The idea

Getting there is incremental by construction, so at any moment the tree is
**mixed and noisy** — C files beside Rust modules, Rust modules wrapped in
`extern "C"` adapters, symbols whose definition has crossed while their
callers have not. That noise is expected and temporary. It is the cost
of keeping the kernel booting and the test suite green at every commit,
and it is not a defect to tidy away by making the Rust look like the C.

Two rules hold the shape of it:

- **Always port to the purest Rust the routine allows.** The point of moving
  a routine is to get Rust's guarantees; a line-for-line rewrite with the same
  raw pointers and the same `int` error codes has bought nothing. Write the
  module as Rust would be written, and let the C-shaped signature live in a
  thin adapter at the edge of it.
- **Never write C to make a port fit.** The unported side keeps calling the
  symbol it always called, with the signature it always had, because Rust
  exports that exact symbol. If a macro, a lock, or a `struct` accessor
  cannot cross FFI, the answer is a different port order, never a shim. See
  "The no-glue law" below; it overrides anything else in this file.

### Where the noise is allowed to live

```
C caller ─► extern "C" adapter ─► safe Rust core ─► safe Rust helpers
           └── raw pointers, out-params, int error codes stop here
```

The adapter is the only place that speaks C: it validates what C handed over,
converts it into Rust types, calls the safe core, and converts the answer
back. Everything behind it is written as if C did not exist.

`rust/src/vm/` is the worked example at scale: `vm_map.rs` is the native core
and `vm_map_ffi.rs` is nothing but adapters, one per symbol `vm/vm_map.c` used
to define. `rust/src/utils/atoi.rs` is the same split in 80 lines — a private
`parse(&[u8]) -> (usize, Option<c_int>)` with the sentinel `MACH_ATOI_DEFAULT`
known only to the adapter above it.

Each step replaces a C definition with a Rust one of the same name and the
same signature, so the kernel links exactly as it did before and the test
suite says whether it still works. There is no staging branch and no parallel
implementation kept beside the original: the C definition goes away in the
same commit the Rust one arrives, because two definitions of one symbol is a
link error, not a fallback.

### The port so far

`MIGRATE.md` §9 is the authoritative table. Gone from C, in rough order:

| C file | Rust |
|---|---|
| `kern/strings.c`, `i386/i386/strings.c` | `src/utils/string.rs` |
| `kern/queue.c` | `src/kern/queue.rs` |
| `kern/smp.c` | `src/kern/smp.rs` |
| `i386/i386/loose_ends.c` (`delay`) | `src/utils/delay.rs` |
| `util/byteorder.c` | `src/utils/byteorder.rs` |
| `util/atoi.c` | `src/utils/atoi.rs` |
| `kern/elf-load.c` | `src/kern/elf_load.rs` |
| `kern/rbtree.c` | `src/kern/rbtree.rs` |
| `ipc/ipc_thread.c` | `src/ipc/ipc_thread.rs` |
| `i386/i386at/kd_queue.c` | `src/utils/kd_queue.rs` |
| `i386/i386at/kd_mouse.c` | `src/arch/i386/kd_mouse.rs` |
| `i386/i386at/kd_event.c` | `src/arch/i386/kd_event.rs` |
| `i386/i386at/kd.c` | `src/arch/i386/kd/` |
| `i386/i386at/mem.c` | `src/arch/i386/mem.rs` |
| `i386/i386at/mbinfo.c` | `src/arch/i386/mbinfo.rs` |
| `vm/vm_map.c` | `src/vm/vm_map.rs`, `src/vm/vm_map_ffi.rs` |
| `kern/lock.c` | `src/kern/lock.rs` |
| `i386/i386/lock.h` (simple lock, bit ops) | `src/kern/lock.rs`, `src/arch/i386/atomic_bits.rs` |
| `kern/kmutex.c` | `src/kern/kmutex.rs` |
| `ipc/ipc_table.c` | `src/ipc/ipc_table.rs` |
| `device/cirbuf.c` | `src/device/cirbuf.rs` |
| `kern/thread_swap.c` | `src/kern/thread_swap.rs` |
| `i386/i386/ast_check.c` | `src/arch/i386/ast_check.rs` |
| `i386/i386at/rtc.c` | `src/arch/i386/rtc.rs` |
| `i386/i386/pit.c` | `src/arch/i386/pit.rs` |
| `kern/slab.c` | `src/kern/slab.rs`, `src/kern/slab_ffi.rs` |
| `kern/rdxtree.c` | `src/kern/rdxtree.rs`, `src/kern/rdxtree_ffi.rs` |
| `vm/vm_page.c` | `src/vm/vm_page.rs`, `src/vm/vm_page_ffi.rs` |
| `vm/vm_object.c` | `src/vm/vm_object.rs`, `src/vm/vm_object_ffi.rs` |
| `kern/task.c` | `src/kern/task.rs`, `src/kern/task_ffi.rs` |
| `i386/intel/pmap.c` | `src/arch/i386/pmap.rs` |
| `i386/i386at/biosmem.c` | `src/arch/i386/biosmem.rs` |
| `kern/thread.c` | `src/kern/thread.rs`, `src/kern/thread_ffi.rs` |
| `kern/ipc_tt.c` | `src/kern/ipc_tt.rs`, `src/kern/ipc_tt_ffi.rs` |
| `ipc/ipc_port.c` | `src/ipc/ipc_port.rs`, `src/ipc/ipc_port_ffi.rs` |
| `ipc/ipc_init.c`, `ipc/ipc_target.c` | `src/ipc/ipc_init.rs`, `ipc_target.rs` |
| `ipc/ipc_space.c`, `ipc/ipc_entry.c` | `src/ipc/ipc_space.rs`, `ipc_space_ffi.rs`, `ipc_entry.rs`, `ipc_entry_ffi.rs` |
| `ipc/ipc_object.c` | `src/ipc/ipc_object.rs`, `ipc_object_ffi.rs` |
| `ipc/ipc_kmsg.c` | `src/ipc/ipc_kmsg.rs`, `ipc_kmsg_ffi.rs` |
| `ipc/mach_port.c` | `src/ipc/mach_port.rs`, `src/ipc/mach_port_ffi.rs` |
| `ipc/ipc_right.c` | `src/ipc/ipc_right.rs`, `src/ipc/ipc_right_ffi.rs` |
| `ipc/ipc_mqueue.c` | `src/ipc/ipc_mqueue.rs`, `src/ipc/ipc_mqueue_ffi.rs` |
| `ipc/ipc_pset.c` | `src/ipc/ipc_pset.rs`, `src/ipc/ipc_pset_ffi.rs` |
| `ipc/ipc_marequest.c` | `src/ipc/ipc_marequest.rs`, `src/ipc/ipc_marequest_ffi.rs` |
| `ipc/ipc_notify.c` | `src/ipc/ipc_notify.rs`, `src/ipc/ipc_notify_ffi.rs` |
| `ipc/mach_msg.c` | `src/ipc/mach_msg.rs`, `src/ipc/mach_msg_ffi.rs` |
| `ipc/mach_debug.c` | `src/ipc/mach_debug.rs`, `src/ipc/mach_debug_ffi.rs` |
| `kern/ipc_mig.c` | `src/kern/ipc_mig.rs`, `src/kern/ipc_mig_ffi.rs` |
| `i386/i386/apic.c` | `src/arch/i386/apic.rs` |
| `i386/i386at/acpi_parse_apic.c` | `src/arch/i386/acpi_parse_apic.rs` |
| `i386/i386/irq.c` | `src/arch/i386/irq.rs` |
| `i386/i386at/ioapic.c` | `src/arch/i386/ioapic.rs` |
| `device/ds_routines.c` | `src/device/ds_routines.rs`, `src/device/ds_routines_ffi.rs` |
| `device/dev_pager.c`, `device/dev_lookup.c` | `src/device/dev_pager.rs`, `src/device/dev_pager_ffi.rs`, `src/device/dev_lookup.rs`, `src/device/dev_lookup_ffi.rs` |
| `kern/ipc_host.c` | `src/kern/ipc_host.rs`, `src/kern/ipc_host_ffi.rs` |
| `kern/host.c` | `src/kern/host.rs`, `src/kern/host_ffi.rs` |
| `kern/exception.c` | `src/kern/exception.rs`, `src/kern/exception_ffi.rs` |
| `device/chario.c` | `src/device/chario.rs`, `src/device/chario_ffi.rs` |
| `device/net_io.c` | `src/device/net_io.rs`, `src/device/net_io_ffi.rs` |
| `ipc/copy_user.c` | `src/ipc/copy_user.rs`, `src/ipc/copy_user_ffi.rs` |
| `kern/mach_clock.c` | `src/kern/mach_clock.rs`, `src/kern/mach_clock_ffi.rs` |
| `kern/gsync.c` | `src/kern/gsync.rs`, `src/kern/gsync_ffi.rs` |
| `i386/i386/fpu.c` | `src/arch/i386/fpu.rs`, `src/arch/i386/fpu_ffi.rs` |
| `i386/i386/pcb.c` | `src/arch/i386/pcb.rs`, `src/arch/i386/pcb_ffi.rs` |
| `kern/sched_prim.c` | `src/kern/sched_prim.rs`, `src/kern/sched_prim_ffi.rs` |
| `kern/timer.c` | `src/kern/timer.rs`, `src/kern/timer_ffi.rs` |
| `i386/i386at/com.c` | `src/arch/i386/com.rs`, `src/arch/i386/com_ffi.rs` |
| `vm/memory_object.c` | `src/vm/memory_object.rs`, `src/vm/memory_object_ffi.rs` |
| `vm/vm_resident.c` | `src/vm/vm_resident.rs`, `src/vm/vm_resident_ffi.rs` |
| `vm/vm_kern.c` | `src/vm/vm_kern.rs`, `src/vm/vm_kern_ffi.rs` |
| `vm/vm_pageout.c` | `src/vm/vm_pageout.rs`, `src/vm/vm_pageout_ffi.rs` |
| `kern/boot_script.c`, `kern/bootstrap.c` | `src/kern/boot_script.rs`, `src/kern/boot_script_ffi.rs`, `src/kern/bootstrap.rs`, `src/kern/bootstrap_ffi.rs` |
| `i386/i386/io_perm.c` | `src/arch/i386/io_perm.rs`, `src/arch/i386/io_perm_ffi.rs` |
| `i386/i386/smp.c` | `src/arch/i386/smp.rs`, `src/arch/i386/smp_ffi.rs` |
| `i386/i386/trap.c` | `src/arch/i386/trap.rs`, `src/arch/i386/trap_ffi.rs` |
| `kern/processor.c` | `src/kern/processor.rs`, `src/kern/processor_ffi.rs` |
| `kern/machine.c` | `src/kern/machine.rs`, `src/kern/machine_ffi.rs` |
| `kern/eventcount.c` | `src/kern/eventcount.rs`, `src/kern/eventcount_ffi.rs` |
| `i386/i386at/model_dep.c` | `src/arch/i386/model_dep.rs`, `src/arch/i386/model_dep_ffi.rs` |
| `i386/i386/mp_desc.c` | `src/arch/i386/mp_desc.rs`, `src/arch/i386/mp_desc_ffi.rs` |
| `i386/i386/debug_i386.c` | `src/arch/i386/debug_i386.rs`, `src/arch/i386/debug_i386_ffi.rs` |
| `vm/memory_object_proxy.c` | `src/vm/memory_object_proxy.rs`, `src/vm/memory_object_proxy_ffi.rs` |
| `vm/vm_debug.c` | `src/vm/vm_debug.rs`, `src/vm/vm_debug_ffi.rs` |
| `vm/vm_user.c` | `src/vm/vm_user.rs`, `src/vm/vm_user_ffi.rs` |
| `kern/ast.c` | `src/kern/ast.rs`, `src/kern/ast_ffi.rs` |
| `kern/debug.c` (`__stack_chk_guard`) | `src/kern/debug.rs` |
| `kern/ipc_kobject.c` | `src/kern/ipc_kobject.rs`, `src/kern/ipc_kobject_ffi.rs` |
| `kern/startup.c` | `src/kern/startup.rs`, `src/kern/startup_ffi.rs` |
| `kern/syscall_emulation.c` | `src/kern/syscall_emulation.rs`, `src/kern/syscall_emulation_ffi.rs` |
| `kern/syscall_subr.c` | `src/kern/syscall_subr.rs`, `src/kern/syscall_subr_ffi.rs` |
| `i386/i386/gdt.c`, `i386/i386/idt.c`, `i386/i386/ktss.c`, `i386/i386/ldt.c` | `src/arch/i386/gdt.rs`, `gdt_ffi.rs`, `idt.rs`, `idt_ffi.rs`, `ktss.rs`, `ktss_ffi.rs`, `ldt.rs`, `ldt_ffi.rs`, `seg.rs` |
| `i386/i386at/int_init.c` | `src/arch/i386/int_init.rs`, `src/arch/i386/int_init_ffi.rs` |
| `i386/i386/user_ldt.c` | `src/arch/i386/user_ldt.rs`, `src/arch/i386/user_ldt_ffi.rs` |
| `i386/i386/db_interface.c` | `src/arch/i386/db_interface.rs`, `src/arch/i386/db_interface_ffi.rs` |
| `device/intr.c`, `device/kmsg.c`, `device/cons.c`, `device/dev_name.c`, `device/subrs.c` | `src/device/intr.rs`, `intr_ffi.rs`, `kmsg.rs`, `kmsg_ffi.rs`, `cons.rs`, `cons_ffi.rs`, `dev_name.rs`, `dev_name_ffi.rs`, `subrs.rs`, `subrs_ffi.rs` |
| `kern/syscall_sw.c`, `kern/mach_factor.c`, `kern/ipc_sched.c`, `kern/priority.c` | `src/kern/syscall_sw.rs`, `mach_factor.rs`, `mach_factor_ffi.rs`, `ipc_sched.rs`, `ipc_sched_ffi.rs`, `priority.rs`, `priority_ffi.rs` |
| `i386/i386/phys.c` | `src/arch/i386/phys.rs`, `src/arch/i386/phys_ffi.rs` |
| `i386/i386/machine_task.c` | `src/arch/i386/machine_task.rs`, `src/arch/i386/machine_task_ffi.rs` |
| `i386/i386/hardclock.c` | `src/arch/i386/hardclock.rs`, `src/arch/i386/hardclock_ffi.rs` |
| `i386/i386/percpu.c` | `src/arch/i386/percpu.rs`, `src/arch/i386/percpu_ffi.rs` |
| `i386/i386at/autoconf.c` | `src/arch/i386/autoconf.rs`, `src/arch/i386/autoconf_ffi.rs` |
| `chips/busses.c` | `src/arch/i386/busses.rs` |
| `device/device_init.c`, `i386/i386at/conf.c`, `i386/i386at/cons_conf.c` | `src/device/device_init.rs`, `dev_name.rs`, `cons.rs` |
| `vm/vm_fault.c` (`vm_fault_cleanup`, `vm_fault_unwire`, `vm_fault_wire_fast`, `vm_fault_copy`, `vm_fault_page`) | `src/vm/vm_fault.rs`, `vm_fault_ffi.rs` |

Also deleted as dead: `device/blkio.c`, the `#if 0` profiling facility
(`profil.h`, `profilparam.h`, `mpqueue`), `i386/intel/read_fault.c` (body
`#if`-ed out on every supported CPU), `ipc/copy_user.c`'s `USER32` half
(`copyoutmsg()` included), which never compiled in either configured
build, and `kern/debug.c`, whose `Panic` lost its last caller when
`vm_fault_unwire` moved to Rust.

**Next:** a good candidate is a leaf, needs no allocation, and has a C
definition that can be deleted in the same commit. `MIGRATE.md` §4 rates every
file in `kern/` by friction and names the blocker for each. The standing gaps
are an allocator over `kalloc`/`kmem_cache`, an RAII lock/IRQ layer, and a
per-CPU accessor — each is a design conversation, not something to add quietly
to land one patch.

**When you update this section, update `MIGRATE.md` §9 in the same commit.**
This table drifted badly once already.
<!-- agents-md:end id=overview -->

<!-- agents-md:begin id=noglue -->
## The no-glue law — this one is not negotiable

**The C half is never extended. The only bridge between the two halves is
`extern "C"` written in Rust.**

What that permits, exactly:

- Rust exports the symbol C already calls:
  `#[unsafe(no_mangle)] pub unsafe extern "C" fn vm_map_enter(...)`. The C
  caller keeps the prototype it already had, in the header it already had,
  and no C file is edited to make the call work.
- Rust declares the C functions it calls, in an `unsafe extern "C"` block in
  `rust/src/glue/`. Declaring a C symbol that already exists writes no C, so
  it is not glue.

What it forbids, with no exception and no "minimal" version:

- A new `*_glue.c` file.
- A new function in an existing `*_glue.c`, or a new Rust caller of one.
- Any C function, wrapper, accessor, macro-expander or bitfield getter
  written so that a Rust module can reach something. One line is still a C
  function.
- A new prototype, macro or `static inline` added to a C header for Rust's
  benefit.

### If a port needs glue, the port is out of order

Glue is not a cost to pay. It is the signal that the sequence is wrong, and
the fix is always the sequence:

- **Port the definer first.** `simple_lock`, `spl*`, `percpu_get`,
  `current_thread()` and `thread_wakeup*` are macros or assembly, so Rust
  cannot call them and no shim may be written for them. Whatever owns each
  one moves to Rust first and exports a real symbol; only then may its users
  move.
- **Move the whole unit.** If a port would need a C accessor for a field,
  the owner of that field moves in the same step, or the port waits.
- **Leave the file for later.** This is the normal answer and it costs
  nothing. A file that cannot move without glue is simply not next.

**Choosing that order is the planner's whole job.** A plan whose steps
include "add a small shim" is not a plan, it is a deferred problem: reorder
it until every step is one C definition deleted and one Rust definition
arriving in its place. If no glue-free ordering exists, say so and stop —
what is missing is a design conversation (an allocator, an RAII lock layer,
a per-CPU accessor), not a shim to add quietly.

### The glue already in the tree

No `*_glue.c` file and no shim pair remains in the C tree: the last
were `vm/vm_map_glue.c`, `vm/vm_external_glue.c` and
`i386/i386at/com.c`'s `com_base_addr`/`com_irq`, all deleted with their
last callers.  The list may never grow, and no new glue ever joins it.
`MIGRATE.md` §10 records what each deletion took.

**Not every row is waiting on a phase.** `i386/i386at/kd_glue.c` was listed
as blocked on the lock phase long after that phase had landed, and it came
out whole: its lock shims called `mach_simple_lock`, which Rust defines, and
its array shims read C statics Rust can declare. Before believing a row,
check what the shim actually calls. A shim that calls a Rust symbol, an
empty macro, a constant macro or a plain C global is removable today.

`MIGRATE.md`'s ordering was rewritten around this rule: §6 is the test
that decides whether a function can move, §7 the phases, §10 the debt.
Where any older note there still names a shim as a step, the step is wrong.

### Take the free ports first

A **free port** is one that needs nothing which does not already exist:
no new C, no new `#[repr(C)]` mirror, no configure-time constant brought
into Rust, no design conversation. `MIGRATE.md` §6 is the seven-question
test that decides this mechanically, and §6.1 is the current list, split
into what is confirmed, what waits on one decision, and what is only a
candidate until someone reads it.

**An empty §6.1 does not mean there are no free ports left.** It was
emptied once, and a mechanical re-derivation immediately found forty
more the first pass had never been pointed at. Re-derive by function
across the whole tree, the way §6.3 describes, before concluding the
list is exhausted; a file's friction rating says nothing about its
leaves.

**Free ports are worked to exhaustion before any infrastructure is
proposed.** They are the only kind of port that cannot be blocked, they
pay off the glue debt fastest, and each one is a commit that needs a
decision from nobody.

So before proposing a mirror, a constant, an allocator or a lock layer:
check whether §6.1 is empty. If it is not, the thing being proposed is
not next, and the honest answer to "what should I port?" is a name from
that list. If a listed function turns out not to be free, say which of
the five questions it fails and fix the entry rather than leaving it.

None of this weakens the no-glue law. A free port is free because it
needs no glue, not the other way round.
<!-- agents-md:end id=noglue -->

<!-- agents-md:begin id=commands -->
## Commands

Everything goes through `mise`, which wraps configure + make + qemu. Tasks are
defined in `mise.toml`.

- Check prerequisites: `mise run deps`
- Build both kernels: `mise run build`
- Build one: `mise run build:x86_64` · `mise run build:i386`
- **Test (the gate): `mise run test`** — builds both kernels and runs the
  frozen ABI pack in `abi-test/` against them on x86_64 **and** i386
- Test one arch: `mise run test:x86_64` · `mise run test:i386`
- Clean: `mise run clean` (removes `build-64` and `build-32`)

There is no separate lint step. `rustfmt --check` and `clippy-driver -D
warnings` run as `rust/lint.stamp`, which `libmach-rs.a` depends on, so an
ordinary `make` runs them and a warning fails the build.

Without mise, the equivalent is `autoreconf -fi`, then `../configure` in
`build-64` / `build-32` with the flags `mise.toml` passes, then `make`, and
finally `abi-test/run-all.sh build-32/gnumach build-64/gnumach`. Read the
task before hand-rolling it: the i386 build needs
`CC='gcc -m32' LD='ld -m elf_i386'`, and both builds reconfigure when
`RUST_LIB_SRC` has gone stale.
<!-- agents-md:end id=commands -->

<!-- agents-md:begin id=testing -->
## Testing

`mise run test` — building both kernels and running the frozen ABI pack in
`abi-test/` against them, on x86_64 and i386 — is what decides whether a port
is correct. The pack is 26 binaries per architecture taken from the old
gnumach ABI, so a ported routine is verified by the running kernel still
satisfying them; a behaviour the pack does not reach stays unverified, and
the frozen pack is not edited to make a failure go away.

`.githooks/pre-commit` builds and runs the pack on every commit, deliberately
and without a file-extension filter. Enable it with
`git config core.hooksPath .githooks`.

There is **no test harness inside the kernel**. Rule 20 still applies: a
module that needs nothing from the kernel and keeps out of `crate::` can
carry `#[cfg(test)]` tests that `rustc --test` compiles for the host.
`src/kern/rbtree.rs` has such tests, but nothing runs them since the host
runner was dropped with the old suite.

### Hard rule: the tests are evidence, not an obstacle

**Weakening a test to make it pass is forbidden.** The ABI pack is the only
correctness gate this project has. A suite made green by editing the suite
proves nothing, and the next reader cannot tell it from a suite that was green
on merit.

Specifically, never:

- delete, rename away, `#if 0`, comment out or `#[ignore]` a test, a case, or
  an assertion;
- edit a test in the frozen pack, or replace a pack tarball with a rebuilt
  one, so that a failing case is no longer reached or no longer fails;
- loosen an assertion — an exact value into a range, `assert_eq!` into
  `assert!`, a failure into a printed warning;
- shrink an input set, a loop count, an iteration bound or a table so that the
  failing case is no longer reached;
- widen a timeout, or re-run until it comes up green;
- skip an architecture: i386 and x86_64 are both the gate, always;
- delete a `debug_assert!` or an `--enable-queue-debug` invariant check that
  fires;
- silence the build gate instead of the test — `#[allow(...)]`, `-A` on the
  clippy line, dropping `-D warnings`, removing a `const` layout assertion, or
  adding a symbol to the `gnumach-undef` allowlist to hide an undefined
  reference.

**Coverage never goes down.** A ported routine keeps every case its C version
was exercised through, and usually gains some.

If a test fails, the port is wrong until proved otherwise. Read the failure,
fix the code. If the test itself is genuinely wrong, say so out loud and fix
it as *its own commit*, with the reasoning in the message — not folded into
the change it was blocking.

The one narrow exception is a deliberate change of observable behaviour.
Then: a separate commit, a message naming the behaviour that changed and why,
a new assertion at least as strict as the old one, and no net loss of
coverage. Adding tests needs no ceremony at all and is always welcome.

### Writing tests

**Never `#[ignore]` a test. If the behaviour is currently wrong, assert the
wrong behaviour and add a `FIXME` saying why.** You then notice when it is
fixed, and you have confirmed the wrong behaviour is at least not a panic.

**Never use `#[should_panic]`.** Check explicitly for `None` or `Err`. Kernel
code must handle all input without panicking, and the panic output pollutes
the log the harness is parsing.

**Name tests as sentences and group them in `mod` blocks; one behaviour per
test.**

```rust
#[cfg(test)]
mod tests {
    mod insert {
        #[test]
        fn returns_err_when_the_key_is_already_present() { /* ... */ }
        #[test]
        fn test_insert() { /* WRONG: says nothing */ }
    }
}
```

**Assert on the error variant with `matches!` when the payload does not
matter.**

```rust
assert!(matches!(err, Error::NoSpace), "expected NoSpace, got {err:?}");
```

**Tolerate duplication in tests.** Share setup; keep the action and the
assertion inline where the reader can see them.
<!-- agents-md:end id=testing -->

<!-- agents-md:begin id=structure -->
## Project structure

`rust/src/` mirrors the C tree, so a routine's Rust home is recognisable from
the C file it came out of.

- `rust/src/utils/` — code that is the same on every machine.
- `rust/src/kern/` — machine-independent facilities, mirroring `kern/`.
- `rust/src/ipc/`, `rust/src/vm/` — mirroring `ipc/` and `vm/`.
- `rust/src/arch/<arch>/` — code written twice, for i686 and x86_64.
- `rust/src/ffi/` — the `extern "C"` entry points C still calls, one
  module per interface definition file (`ffi/mach_host.rs` holds the
  `mach_host.defs` server entries). Adapters only: the cores stay in
  `kern/`, `ipc/` or `vm/`. Not to be confused with `glue/`, which
  points the other way.
- `rust/src/glue/` — the C functions Rust calls, declared with the C
  signature exactly, inside an `unsafe extern "C"` block. A C *macro* cannot
  come through here, and no shim may be written for it: whatever defines the
  macro is ported first, so that there is a real symbol to declare.
- `rust/src/panic.rs` — `#[panic_handler]`, routed into the kernel's `Panic()`.

No `*_glue.c` file remains in the C tree, and no shim pair either.  The
last were `vm/vm_map_glue.c`, `vm/vm_external_glue.c` and
`i386/i386at/com.c`'s `com_base_addr`/`com_irq`, deleted with their last
callers and listed under "The no-glue law".  Nothing adds to them and
nothing joins them.

The C half is unchanged Mach: `kern/`, `ipc/`, `vm/`, `device/`, `i386/`,
`x86_64/`, `chips/`, `util/`, with `include/` for the public interfaces.
`abi-test/` holds the frozen ABI pack that gates every commit. `build-64/`
and `build-32/` are the out-of-tree build directories and hold the
MIG-generated `*.server.c` / `*.user.c`.

### Every new `.rs` file goes in `MACH_RS_SRCS`

`rust/Makefrag.am` lists the Rust sources. That list is what `rustfmt --check`
formats, what `rust/lint.stamp` depends on, and what gets distributed. A file
reached only by a `mod` declaration still compiles, but it is not
format-checked and the lint stamp does not rebuild when it changes.

This has drifted before: `rust/src/arch/i386/kd/tty.rs` was compiled through
`pub mod tty;` in `kd/mod.rs` for three commits without ever being
format-checked. Check the list when you add a file, not when something breaks.

### What gets built

| | |
|---|---|
| `rust/libcore.rlib` | `core`, compiled from the toolchain's own sources for our target. A release toolchain ships `core` only for targets it knows about, and none of them is a freestanding kernel. |
| `rust/libcompiler_builtins.rlib` | An empty crate. rustc requires one for a `staticlib`, but the kernel already links libgcc for these intrinsics. |
| `libmach-rs.a` | The kernel code itself, from `rust/src/`. |

### The toolchain contract

Pinned in `rust-toolchain.toml` (channel `stable`, with `rust-src`, `clippy`
and `rustfmt`), driven from `rust/Makefrag.am`.

| | |
|---|---|
| **Edition** | **2024** — `--edition 2024` in `AM_RUSTFLAGS`, and `edition = "2024"` in `rust/rustfmt.toml`. The two move together or rustfmt parses a different language from rustc. |
| Target | `rust/targets/{i686,x86_64}-gnumach.json` — `"os": "none"`, `"std": false`, `panic-strategy: abort`, no MMX/SSE, soft float, no red zone, kernel code model, static relocation. |
| Codegen | `-C panic=abort -C opt-level=2 -C force-frame-pointers=yes -C overflow-checks=off`, and `-C lto=fat` for `libmach-rs.a`. |
| Nightly knobs | `RUSTC_BOOTSTRAP=1`: custom target JSONs and building `core` out of tree are nightly-only on a stable channel. |
| Gates | `rustfmt --check` at 79 columns and `clippy-driver -D warnings` run as `rust/lint.stamp`, which `libmach-rs.a` depends on. A warning fails the build. |
| Correctness | `mise run test` — the qemu suite on x86_64 **and** i386. |

Neither lint is an extra thing to install: `rust-toolchain.toml` lists
`clippy` and `rustfmt` among the toolchain's components, and `configure`
checks for both and fails early with a hint if either is missing. They are
cheap — `core` is already built by the time they run, so clippy has only this
crate left to look at.
<!-- agents-md:end id=structure -->

<!-- agents-md:begin id=conventions -->
## Code style & conventions

Formatting is not a matter of taste here: `rustfmt --check` at
`max_width = 79` is a build gate, and the C half is written to 79 columns too.
Run `rustfmt` before you think about anything else. Note that rustfmt checks
neither comments nor documentation, so everything below about doc comments and
`// SAFETY:` is enforced by review, not tooling.

Rules 1–20 are the port's idiom rules. Rules 21–45 are the standing Rust
practice this target needs, drawn from the Linux kernel Rust coding
guidelines, the rust-analyzer style guide, the Rust API guidelines and
Apollo's Rust best practices, and filtered down to what holds in `#![no_std]`
with no allocator.

Rule numbers are stable. Add new rules at the end rather than renumbering.

### The comment budget

**Four kinds of comment are mandatory. Every other comment is a defect
until it has earned its line.**

Mandatory, and never trimmed:

1. The SPDX header block (see "License headers").
2. The module's `//!` doc comment, naming the C file it replaces.
3. `# Safety` on every exported `unsafe extern "C" fn` — clippy fails the
   build without it.
4. `// SAFETY:` on every `unsafe { ... }` block, naming the invariant and
   who guarantees it.

Everything else is written to this standard: **the code carries its own
meaning through names, types and small functions, and a comment appears
only where a reader who understands Rust and has the C original in front
of them would still be unable to work out *why*.**

A comment earns its line only by recording something the code cannot
state: a hardware quirk, a lock ordering, a deliberate divergence from
the C, a constant's provenance, a wrapping that is intended. Restating
the name, the signature, the control flow or the arithmetic is not one
of those. Neither is announcing a step (`// Take the lock.`,
`// Now walk the list.`, `// Convert back for C.`).

**Output that carries explanatory comments is rejected.** When a step
seems to need a narrative line, the answer is a named helper variable or
a named function, not the line. Write the code so the line is not
missed.

This overrides the density of the file you are editing. About a third of
the lines in `rust/src/` are comments today, and much of that is exactly
what this section forbids: it is what is being cleaned up, not the house
style to match. Rules 18, 36, 37 and 39 are the detail; this is the budget they spend
against.

### What `no_std` costs

`#![no_std]`, `core` only, **no `alloc`**. This is the constraint that shapes
every design here, so it is worth stating the consequences rather than
rediscovering them per port:

- **No `Box`, `Vec`, `String`, no collections.** Containers are *intrusive*:
  the node lives inside the caller's structure and the container links it
  (`src/kern/queue.rs`, `src/kern/rbtree.rs`). Code works in memory the caller
  supplies. When a routine genuinely needs to allocate, that is a design
  conversation about a `GlobalAlloc` over `kalloc` — not something added
  quietly to land one patch.
- **Intrusive means pinned.** A linked node must not move; `QueueEntry` is
  `!Unpin` and linking takes `Pin<&mut _>` so that safe Rust cannot move it.
  New intrusive types follow that pattern.
- **Panic is a halt.** `-C panic=abort`, `panic-strategy: abort` in the
  target, and `src/panic.rs` routes `#[panic_handler]` into the kernel's
  `Panic()`. There is no unwinding, nothing to catch, and no unwinding across
  FFI. `Result` is the only error channel — see rule 12.
- **Arithmetic wraps silently** (`-C overflow-checks=off`). There is no
  overflow panic here to catch anything — see rule 13.
- **No `std::sync`.** No `Mutex`, `RwLock`, `Once`, `OnceLock`, and no
  `thread_local!`. Shared state is `core::sync::atomic` or the kernel's own
  `simple_lock` behind a Rust guard type (`src/kern/lock.rs`); per-CPU data
  goes through the `%gs` accessors in `src/arch/`.
- **Atomic width.** `max-atomic-width` is 64 on both targets, but prefer
  `AtomicUsize`/`AtomicU32`/`AtomicU8`: a 64-bit atomic on i686 lowers to a
  `cmpxchg8b` loop and can reach for libatomic, which the kernel does not
  link. `gnumach-undef-bad` is what catches it, at link time, on one
  architecture only.
- **No floating point.** The targets disable MMX/SSE and use soft float;
  `f32`/`f64` do not belong in kernel code. `-C lto=fat` is what keeps
  `core`'s float formatting — and the soft-float intrinsics libgcc does not
  provide — out of the archive.
- **Two pointer widths.** i686 is 32-bit and x86_64 is 64-bit, from one source
  tree. Never assume `usize` is 64 bits; spell the FFI boundary in `core::ffi`
  types (`c_int`, `c_uint`, `c_char`) and the Mach types in
  `src/arch/types.rs`.
- **Stacks are small.** A kernel stack is a few KiB. Do not pass anything over
  a few hundred bytes by value; take `&T`. There is no `Box` to move it to the
  heap, so the only options are a reference or a smaller type.
- **`#![no_builtins]`.** LLVM may rewrite a byte-copy loop into a call to
  `memcpy`, which in this crate is a call to itself. The attribute is not
  unused: nothing tests for it.
- **C strings.** `core::ffi::CStr` and `c"..."` literals, not a hand-rolled
  NUL walk. Printing goes through `kprint!`/`kprintln!` in
  `src/kern/console.rs`, which formats with `core::fmt` and writes through
  the `cnputc()` core; `CStrArg` is how a NUL-terminated C string becomes a
  `{}` argument, `write_cstr` is the `snprintf()` replacement, and
  `kpanic!` is the Rust `Panic()`. No C caller of the C `printf` path is
  left; `kern/printf.c` remains only until its deletion pass.

### Edition 2024

The crate is edition 2024 and new code is written in its idiom.

- **`unsafe extern "C" { ... }`.** Extern blocks are unsafe in 2024;
  `rust/src/glue/mod.rs` declares the C side inside one. A bare
  `extern "C" { }` no longer compiles.
- **Unsafe attributes.** `#[unsafe(no_mangle)]`, `#[unsafe(export_name)]` and
  `#[unsafe(link_section)]` — the bare spellings are an error.
- **`static_mut_refs` is a hard error.** Taking a reference to a `static mut`
  no longer compiles, so rule 5 is enforced by the compiler rather than by
  review.
- **`unsafe_op_in_unsafe_fn` warns by default**, and `src/lib.rs` raises it to
  `deny`. The body of an `unsafe fn` is not an implicit unsafe block: "this
  function's contract is unsafe" and "this line dereferences a pointer" stay
  separate statements.
- **Never-type fallback changed.** `!` no longer falls back to `()`, which
  matters around diverging FFI (`Panic()` is `-> !`) and match arms that never
  return. Annotate rather than rely on inference.
- **`if let` and tail-expression temporary scopes changed.** Temporaries in an
  `if let` scrutinee drop before the `else` arm. Where the scrutinee takes a
  lock or a guard, say what the scope is instead of leaning on the old timing.
- **RPIT lifetime capture.** `impl Trait` in return position captures every
  in-scope lifetime; use `+ use<>` to opt out when an iterator must not borrow
  its argument.
- **`gen` is a reserved keyword**, and `macro_rules!`'s `expr` fragment now
  matches `const` blocks and `_`.

---

## The rules

These apply to all work in this repository, to new ports and to cleanups of
ports already landed.

### 1. Error handling

No integer return codes and no `NULL`-as-failure. A fallible insertion returns
`Result<(), Error>`; a lookup returns `Option<&Node>` or `Option<&mut Node>`.
The `extern "C"` adapter is what turns those back into the `0` / `-1` or the
possibly-null pointer C expects.

`src/vm/error.rs` is the pattern: an `Error` enum named after
`mach/kern_return.h`, plus `kern_return()` and `error_from_kern_return()`
const functions that the adapters use and nothing else does.

```rust
// Core: Rust errors.
fn enter(&mut self, req: &EnterRequest) -> Result<(), Error> { /* ... */ }

// Adapter: the C caller's kern_return_t.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_enter(/* ... */) -> c_int {
    kern_return(map.enter(&req))
}
```

### 2. Out-parameters

Return the data. `void rb_search(key, struct node **out)` becomes
`fn search(&self, key: K) -> Option<&Node>`. Multiple results come back as a
tuple or a named struct, never as a caller-supplied slot to fill.

```rust
// RIGHT -- src/utils/atoi.rs: count and value both returned.
fn parse(bytes: &[u8]) -> (usize, Option<c_int>)

// WRONG
fn parse(bytes: &[u8], number: &mut c_int) -> usize
```

The one exception is filling a buffer the caller owns, which is a different
thing: `fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error>`.

### 3. Memory slices

A pointer and a length that belong together are one `&[u8]` or `&mut [u8]`.
They are fused at the FFI boundary, inside the adapter, and no function behind
it takes the two separately.

### 4. Pointers versus references in tree linkage

Rust's aliasing rules mean `&mut` usually cannot express parent/child links —
the tree's own edges alias the nodes. Where raw pointers are genuinely needed,
they are `NonNull<Node>`, never a bare `*mut Node` documented as non-null. All
pointer arithmetic and link surgery is encapsulated in safe methods on the
container; the module boundary exposes safe references only.

### 5. Global state

No `static mut` — edition 2024 makes a reference to one a hard error anyway. A
scalar becomes an atomic of the right width with an explicitly chosen
ordering; compound state is wrapped in a GNU Mach-appropriate lock (the
kernel's own `simple_lock` behind a Rust guard type — see `src/kern/lock.rs`
and the L2 layer in `MIGRATE.md`), not in a `Mutex` this environment does not
have.

```rust
// src/kern/smp.rs -- the whole of the module's state.
static NUM_CPUS: AtomicU8 = AtomicU8::new(1);
```

### 6. Type-state for values with a fixed domain

`#define RB_RED` / `RB_BLACK` and similar integer constants become a Rust
`enum` (`NodeColor { Red, Black }`), and the logic that consumes them uses
exhaustive `match` — rebalancing especially, where the compiler checking that
every case is handled is most of the value.

### 7. Initialization

No two-step init. Implement `Default`, or provide `Self::new()`, so an empty
tree or a fresh node is constructed already valid. A C `rbtree_init(&tree)`
becomes `Rbtree::new()` behind its adapter.

Never invent a dummy state just to have a `Default`. If a type has no sensible
empty value, make the caller state the initial state.

### 8. Bitflags

Flag sets are a `bitflags!`-style type, not raw bitwise arithmetic on an
integer. **This build has no Cargo and fetches nothing from the network**, so
use an in-tree equivalent under `rust/src/utils/` rather than adding the
`bitflags` crate as a dependency.

`src/vm/types.rs`'s `VmProt` is the hand-written form: a
`#[repr(transparent)]` newtype over `c_int`, associated `const`s for the bits,
`contains()`, and `BitOr`/`BitAnd` impls.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct VmProt(c_int);

impl VmProt {
    pub const READ: Self = Self(0x1);
    pub const WRITE: Self = Self(0x2);
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}
```

### 9. Iteration

Replace `while` loops that walk internal links in the caller's face with a
real iterator: implement `Iterator` yielding `&T` or `&mut T` for in-order
traversal, and let callers use `for`, `find`, `any`. Internal node pointers do
not leave the module.

Name the methods exactly `iter`, `iter_mut`, `into_iter`, and name the types
they return `Iter`, `IterMut`, `IntoIter`. Not `nodes()`, not `TreeIterator`.

### 10. Strings

Validate UTF-8 at the FFI boundary and convert to `&str` there, so the Rust
core works in `&str`. Use `&[u8]` strictly for raw binary IPC payloads and
other byte buffers that are not text.

### 11. Safety documentation, and the smallest unsafe block

`unsafe` marks the operation that needs it and nothing else — not the
function, not the loop around it. Cast once, compute offsets in safe code, and
touch memory at the last possible moment.

Every `unsafe { ... }` block carries a `// SAFETY:` comment naming the
invariant it relies on and who guarantees it. Every exported
`unsafe extern "C" fn` documents its contract in a `# Safety` section;
clippy's `missing_safety_doc` fails the build without one.

Keep the two straight. **`# Safety` states the contract the caller must
uphold. `// SAFETY:` says why this particular call respects it.**

```rust
/// # Safety
///
/// `s` must be readable up to and including the first non-digit byte --
/// a NUL-terminated string satisfies this; `nump` must be valid for a
/// write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_atoi(s: *const u8, nump: *mut c_int) -> c_int {
    // SAFETY: the caller promises a non-digit byte is reachable, so the
    // walk stops inside the readable region.
    while unsafe { *s.add(len) }.is_ascii_digit() { len += 1; }
```

"Node is non-null and uniquely accessed for the length of this rotation" is a
safety comment. "raw pointer" is not. Write the comment even when the reason
looks trivial: it is the record that the obligation was considered, and it is
where a reviewer finds out whether an implicit constraint is hiding.

### 12. No panics

Avoid `unwrap()`, `expect()` and direct indexing whose bounds are not
statically guaranteed. Use `get()` / `get_mut()`, `match`, `?` and the
`checked_*` family. A panic here reaches `src/panic.rs` and the kernel's
`Panic()`: it is a kernel halt, not an error message.

`let ... else` is the idiom when the failure path just diverges:

```rust
// RIGHT
let Some(entry) = map.lookup(addr) else {
    return Err(Error::InvalidAddress);
};

// WRONG -- a panic in ring 0.
let entry = map.lookup(addr).unwrap();
```

A deliberate `assert!` is allowed where the condition is a kernel invariant
whose violation means the machine is already lost, and it is documented in a
`# Panics` section. `smp_set_numcpus` is the precedent.

### 13. Casts and arithmetic

No `as` where `From`, `TryFrom` or `try_into()` will do; `as` is for a
deliberate truncation, with a comment saying why it cannot lose anything that
matters. Overflow checks are off, so say `wrapping_*`, `checked_*` or
`saturating_*` whenever the behaviour at the boundary is part of what the
routine means.

```rust
// src/utils/atoi.rs -- both halves of this rule in four lines.
number = number
    .wrapping_mul(10)
    .wrapping_add(c_int::from(byte - b'0')); // From, not `as`.

// The C original converted `cp - original` to an `int`; `used` is
// bounded by the readable byte count, so only a multi-gigabyte digit
// run could differ.
used as c_int
```

### 14. Newtypes, not aliases

A handle, an index or a unit gets a `#[repr(transparent)]` newtype. A `type`
alias stops nothing: it will not keep a `vm_offset_t` out of a parameter that
wants a `vm_size_t`.

```rust
// RIGHT -- map_page(pa, va) is a compile error.
pub struct PhysAddr(u64);
pub struct VirtAddr(u64);
fn map_page(va: VirtAddr, pa: PhysAddr);

// WRONG -- map_page(pa, va) compiles and corrupts the page table.
type phys_addr_t = u64;
fn map_page(va: u64, pa: u64);
```

### 15. Layout mirrors are asserted, not described

Every `#[repr(C)]` type shared with C carries `const` assertions on
`size_of`, `align_of` and `offset_of` against the C layout it mirrors, so
drift is a build error rather than a corrupted field at run time.

```rust
// src/kern/queue.rs
const _: () =
    assert!(size_of::<QueueEntry>() == 2 * size_of::<*mut QueueEntry>());
const _: () = assert!(core::mem::offset_of!(QueueEntry, next) == 0);
const _: () = assert!(
    core::mem::offset_of!(QueueEntry, prev) == size_of::<*mut QueueEntry>()
);
```

Where the layout differs per pointer width, assert both under `#[cfg]`, as
`src/kern/lock.rs` does for `LockData`.

### 16. `#[must_use]`

On `Result`-returning functions that C would have let you ignore, on
constructors, and on handles whose value must not be dropped silently.

### 17. Atomic orderings are chosen, not defaulted

Name the ordering and say in a comment what it publishes or acquires. `SeqCst`
is a decision to justify, not a fallback; `Relaxed` needs to say why no other
thread depends on the order.

```rust
// src/kern/lock.rs -- three orderings, three reasons.

/// The store is `Relaxed` because the caller's proof, not an
/// ordering, is what makes it safe.
pub fn init(&self) { self.lock_data.store(0, Ordering::Relaxed); }

/// The outer `swap` is the `xchg` of the C macro: it takes the lock
/// and acquires the releasing unlock's publishes.  The inner load is
/// its test-and-test-and-set read and may be `Relaxed`.
pub fn lock(&self) {
    while self.lock_data.swap(1, Ordering::AcqRel) != 0 {
        while self.lock_data.load(Ordering::Relaxed) != 0 {
            core::hint::spin_loop();
        }
    }
}
```

### 18. Doc comments and visibility

Every module's doc comment names the C file it replaces and the header it
mirrors. Every `unsafe extern "C" fn` has `# Safety`. **Outside those two,
an item has no doc comment by default.** It gains one only where the
contract is not evident from the name and the signature: an error
condition, an invariant the constructor establishes, a unit, a lock the
caller must already hold, the C name when it differs. `/// Returns the
port.` above `fn port(&self) -> &Port` is a defect; delete it rather than
reword it. A doc that names a C function, macro or header keeps that name
when it differs from the Rust name or is needed to find the original.
Visibility is the smallest that works — internals stay private so that the
module boundary *is* the safe API.

```rust
//! The generic SMP controller, which `kern/smp.c` used to define.
//!
//! Just the CPU count the architecture probe reports.  The count is
//! always at least one: a machine running Mach has a CPU, so the state
//! starts at one and the setter treats zero as a forbidden argument.
```

### 19. `const` over macros

A C `#define` becomes a `const` or a `const fn`, not a Rust macro. Reach for a
macro only when a function or a generic genuinely cannot express it.

### 20. Host-testable modules stay `crate::`-free

A module that needs nothing from the kernel keeps out of `crate::` so that
`rustc --test` can compile it for the host (the rbtree's `#[cfg(test)]` tests
were compiled that way until the host runner was dropped with the old suite).
It is still compiled into `libmach-rs.a` like any other module — that is not a
second build path for the kernel.

---

## Standing Rust practice (21–44)

**Naming — rules 21 to 25.**

### 21. Casing, and acronyms are one word

`UpperCamelCase` types, `snake_case` modules and functions,
`SCREAMING_SNAKE_CASE` consts. Acronyms are capitalised as words: `IpcPort`,
`TlbEntry`, `VmMap` — never `IPCPort` or `VMMap`. In snake_case they are
lowercase: `flush_tlb_range`.

A single letter never becomes its own snake_case word unless it is the last
one: `btree_map`, not `b_tree_map`.

### 22. Conversion prefixes mean what they say

`as_` is a free borrowed-to-borrowed view, `to_` does real work, `into_`
consumes an owned value. Put `mut` where it appears in the return type:
`as_mut_slice`, not `as_slice_mut`.

```rust
impl PageFrame {
    pub fn as_bytes(&self) -> &[u8; 4096] { &self.0 }       // free view
    pub fn to_phys_addr(&self) -> PhysAddr { /* computes */ }
    pub fn into_raw(self) -> *mut u8 { /* consumes self */ }
}
```

The unwrapping accessor of a single-value wrapper is `into_inner()`.

### 23. No `get_` prefix on getters, and no setters

A getter is named for the field; the mutable form takes a `_mut` suffix.

```rust
// RIGHT
impl Task {
    pub fn name(&self) -> &TaskName { &self.name }
    pub fn name_mut(&mut self) -> &mut TaskName { &mut self.name }
}
// WRONG: get_name, get_mut_name, mut_name
```

Getters return borrowed data: `&str` not `String`, `Option<&T>` not
`&Option<T>`.

If a field has an invariant, document it, enforce it in the constructor, keep
the field private and add a getter. **Do not add a setter** — a setter is how
an invariant gets broken from outside. The exception is a `#[repr(C)]` mirror
of a C struct, which is a passive record by design and may have `pub` fields.

### 24. Bounds-checked and unchecked pairs have fixed signatures

```rust
fn get(&self, i: PortIndex) -> Option<&Port>;
fn get_mut(&mut self, i: PortIndex) -> Option<&mut Port>;
/// # Safety
/// `i` must be a live index in this space.
unsafe fn get_unchecked(&self, i: PortIndex) -> &Port;
```

### 25. Keep C names recognisable, adjusted to Rust casing

When a Rust item wraps a C concept, keep the name as close to the C name as
Rust conventions allow, so switching between the two halves is not confusing.
Do not repeat the module's namespacing in the item name.

```rust
// RIGHT: vm::Prot::Read
pub mod vm {
    pub enum Prot { Read, Write, Execute }
}
// WRONG: vm::vm_prot::VM_PROT_READ
```

The exception is a symbol C still links against: those keep their exact C
name, because the linker is the contract. MIG `intran`/`outtran` symbols, trap
entries and asm-read globals do not get renamed until their C readers are gone.

**Types and type-safety — rules 26 to 31.**

### 26. Derive the common traits eagerly

`Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`,
`Default` all live in `core`, so they cost nothing here. Deriving them is
cheap; adding them later from another module is impossible under the orphan
rule.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct VmInherit(c_int);
```

Derive `Copy` only when every field is `Copy`, the type is plain data with no
ownership, and it is small — say three words. And **never put `Copy` and
`Iterator` on the same type**: copying an iterator silently forks its position.

Do not repeat derivable bounds on the struct definition. `struct Slab<T>`,
not `struct Slab<T: Clone + Debug>`.

### 27. A `bool` parameter is almost always the wrong type

Pass meaning through an enum. `vm_allocate(n, true)` says nothing at the call
site.

```rust
// RIGHT
pub enum Wiring { Wired, Pageable }
pub fn vm_allocate(size: usize, wiring: Wiring);

// WRONG
pub fn vm_allocate(size: usize, wired: bool);
```

If a `bool` or `Option` parameter is passed as the same literal at every call
site, split the function in two instead.

### 28. Implement `From`/`TryFrom`, never `Into`/`TryInto`

The blanket impls give you `Into` for free. Use a named `from_*` constructor
instead of `From` when the source type alone does not determine the meaning —
`from_raw_bits(u32)` rather than `From<u32>`.

### 29. `Send` and `Sync` on FFI wrappers are asserted, not assumed

A type holding a raw pointer to a C object is neither by default, and that is
usually correct. If it genuinely is one, say why in a safety comment and pin
it with a compile-time assertion.

```rust
/// SAFETY: every access to the underlying `ipc_port` goes through the
/// port lock.
unsafe impl Sync for PortRef {}
const _: () = {
    const fn assert_sync<T: Sync>() {}
    assert_sync::<PortRef>();
};
```

### 30. Validity is enforced in this order

Static types first, then a runtime check returning `Result`, then
`debug_assert!` on a hot path, then an `unsafe` `_unchecked` opt-out. Reach
for the weakest one only when the stronger one is measurably too expensive.

```rust
pub fn write_page(a: PageAligned);                        // best
pub fn write_page(a: VmOffset) -> Result<(), Error>;      // fine
pub fn write_page(a: VmOffset) { debug_assert!(/*...*/) } // hot paths
/// # Safety
/// `a` must be page-aligned.
pub unsafe fn write_page_unchecked(a: VmOffset);          // opt-out
```

### 31. Destructors never fail and never block

Fallible teardown goes in an explicit `destroy()` returning `Result`; `Drop`
does best-effort cleanup and never panics. A panic in a destructor here is a
kernel halt during cleanup, which is the worst possible time for one.

```rust
// RIGHT
impl PortRef {
    pub fn destroy(self) -> Result<(), Error> { /* reports failure */ }
}
impl Drop for PortRef {
    fn drop(&mut self) { let _ = self.release_best_effort(); }
}
// WRONG
impl Drop for PortRef {
    fn drop(&mut self) { self.release().expect("release failed"); }
}
```

**Errors — rules 32 to 35.**

### 32. One error enum per subsystem, never `()`

Name the variants after the C codes they map to, and put the mapping in one
place. `src/vm/error.rs` is the model. Nest errors from a lower layer with
`From` so `?` converts across the boundary for you.

```rust
pub enum IpcError { Vm(VmError), Port(PortError) }
impl From<VmError> for IpcError {
    fn from(e: VmError) -> Self { IpcError::Vm(e) }
}
fn send(m: &Msg) -> Result<(), IpcError> { copy_in(m)?; Ok(()) }
```

### 33. `return Err(e)`, never `Err(e)?`

`return` has type `!`, so the compiler can see the dead code after it.
`Err(e)?` is a generic `T` and defeats that.

### 34. Use the `_else` variants when the fallback does work

```rust
x.ok_or_else(|| Error::describe(name))  // closure runs only on None
x.ok_or(Error::describe(name))          // builds the error every call
```

`.ok()` / `.ok_or()` convert between `Result` and `Option`; do not write the
`match` by hand.

### 35. Error message strings are lowercase and unpunctuated

"invalid port name", not "Invalid port name." This is the opposite of the
comment rule below, and both are deliberate: prose gets periods, error strings
do not.

**Comments and documentation — rules 36 to 39.**

### 36. Comments are sentences

Capitalised, ending in a period, written in Markdown even though `//` is never
rendered. This applies to tagged comments too: `// SAFETY: The caller holds
the map lock.` Writing a sentence rather than a fragment is what makes you
write down the context you are holding in your head.

```rust
// RIGHT
// Only simple single-segment paths are allowed.

// WRONG
// only simple single segment paths allowed
```

Comments say **why**, and only where the why is not already on the page.
`// increment i by 1` above `i += 1` is noise. So is a line that names the
next step, restates a condition, or translates the code into English. If a
function needs a narrative comment per step, it wants named helper
functions and named helper variables instead, and the comments go away with
the rewrite.

Before finishing any change, re-read the diff and **delete every comment
that is not one of the four mandatory kinds and does not record a why the
code cannot carry**. This pass is part of writing the code, not a cleanup
for later.

Use `///` even on private items you are documenting: it keeps the style
uniform and survives a visibility change.

### 37. Never leave a bare `TODO`

Say what will resolve it and, where one exists, name the tracked item. The
same rule covers a comment on pre-rule glue: name the C caller whose move
deletes it.

```rust
// TODO: Drop this declaration when ipc_thread.c's last caller moves. <- RIGHT
// TODO: fix this                                                     <- WRONG
```

### 38. The first line of a doc comment is one sentence saying what it does

Everything else goes in later paragraphs. Sections in order: `# Safety`,
`# Errors`, `# Panics`, `# Invariants`, `# Examples`. Link other items with
intra-doc links so rustdoc resolves them.

```rust
/// Returns the [`Task`] that owns this port, or [`None`] if it was
/// destroyed.
```

`# Invariants` is not a rustdoc-standard section, but it is the Rust-for-Linux
convention and this tree uses it. Put it on any type whose fields are private
because they have a relationship the constructor establishes.

```rust
/// A reference to a live IPC port.
///
/// # Invariants
///
/// `self.0` always points at a valid `ipc_port`, and this wrapper owns
/// one reference to it.
pub struct PortRef(NonNull<IpcPort>);
```

### 39. Commentary about the docs goes inside the doc block

A `FIXME` about the code goes between the doc block and the item.

```rust
/// Returns a new [`Foo`].
///
/// # Examples
///
// TODO: Find a better example.
/// ```
/// let foo = f(42);
/// ```
// FIXME: Use the fallible approach once vm_map_enter returns Result.
pub fn f(x: c_int) -> Foo { /* ... */ }
```

**Control flow and organization — rules 40 to 45.**

### 40. Early returns, and do not hide control flow

Push the condition to the caller rather than starting the callee with a guard
that returns.

```rust
// RIGHT
if cond { f() }

// WRONG
fn f() { if !cond { return; } /* ... */ }
```

Express preconditions in the type system and let the caller satisfy them, and
**never split a check from its use across two functions** — return the proof,
not a `bool`. This is the rule that matters most around `unsafe`: a
`is_valid()` in one function and a dereference in another is a safety argument
that no longer typechecks.

```rust
// RIGHT -- validity and use are one expression.
if let Some(contents) = string_literal_contents(s) { /* ... */ }

// WRONG -- the 1..len-1 slice is justified in a different function.
if is_string_literal(s) { let contents = &s[1..s.len() - 1]; }
```

### 41. Avoid single-use helper functions; use a block

A block delineates the logic just as well and has all the context. Single-use
functions churn as parameters come and go, and get reused where they do not
fit. The exception is when you need `return` or `?`.

Where a nested helper is right, put it at the end of the enclosing function,
after a `return`, and never nest more than one level.

Introduce helper *variables* freely, especially to name a multiline condition.
They are free, they read well, and they are easier to inspect.

### 42. Order items for a first-time reader

Types before functions, parent types before child types, most important first.
With bodies folded, the file should read as documentation of its API. Order
`mod` declarations in the order a newcomer should read them.

Prefer `use crate::foo::bar` over `use super::bar`. When implementing a trait
from `core::fmt` or `core::ops`, import the module and write `fmt::Display`,
so it is obvious a trait is being implemented rather than used. Never write
`use MyEnum::*`.

### 43. Prefer `match` over `if let ... else`, and `<` over `>`

`match` is more compact and gives the else-branch a real pattern (`None`,
`Err(_)`) instead of `_`. Comparisons read as a number line: `lo <= x && x <=
hi`, not `x >= lo && x <= hi`. An intentionally empty arm is `=> ()`, not
`=> {}`. Never use the `ref` keyword.

Use combinators when they are the natural choice, not as a matter of principle;
`Option::filter` and `bool::then` are usually an indirection for an `if` that
does no work of its own.

### 44. `#[expect]`, not `#[allow]`

`#[expect]` warns once the lint stops firing, so a stale suppression removes
itself. `#[allow]` rots silently.

```rust
// RIGHT -- and the reason belongs above it.
// Read by the C side only; the Rust module never constructs one.
#[expect(dead_code)]
struct ElfPhdr { /* ... */ }
```

Use `#[allow]` only where conditional compilation makes the lint fire in one
configuration and not another — an `as` cast whose width depends on the
target, a `#[cfg]`-gated caller. That is common here, so it is a real
exception, not an escape hatch. Where only one or two configurations are
involved, a conditional `expect` is more precise:

```rust
#[cfg_attr(not(target_pointer_width = "64"), expect(dead_code))]
fn wide_only() { /* ... */ }
```

**There are 13 `#[allow(dead_code)]` in the tree and no `#[expect]`.**
Converting one is a welcome drive-by when you are already in the file.

Never add a suppression to silence a lint that is telling the truth. See the
hard rule under Testing.

### 45. New locks come from `rust/src/spin`, and guards are `#[must_use]`

New lock requirements use the vendored primitives in `rust/src/spin` —
`crate::spin::{Mutex, RwLock}` and their guards — never a fresh
hand-rolled spinlock over `core::sync::atomic`, and never an unguarded
spin on a C `simple_lock` this tree has not ported.  Every guard carries
`#[must_use]`, so taking a lock and dropping it on the same statement is
a warning: bind it (`let guard = lock.lock();`) or discard it
deliberately (`let _ = lock.lock();`).

This is the rule for new lock sites, not a migration order.  `kern/lock.h`
still owns `struct slock` and `struct lock` while C structs embed them,
so those C layouts go on being the ABI; a lock site inside a C file moves
to the vendored primitives when that file moves, not before.

---

## Deliberately not adopted

These are real rules from the sources above that this project does **not**
follow, so that nobody "fixes" the tree toward them.

- **The Linux kernel's vertical import style** (one item per line with a
  trailing `//`). Every file here uses condensed imports —
  `use core::ffi::{c_int, c_uint, c_void};` — and rustfmt at 79 columns is
  already the gate. Adopting it would reflow the whole tree for a
  merge-conflict benefit this repo does not have.
- **`thiserror`, `anyhow`, `bitflags`, `smallvec`, any crate at all.** No
  Cargo, no network. Rule 8 is why the bitflags macro is written in tree.
  `rust/src/spin` is the one exception, and not a dependency: it is
  MIT-licensed source vendored in the tree, and rule 45 requires new locks
  to use it.
- **`std::error::Error`, `Send + Sync` bounds on errors, serde, async.** No
  `std`, no allocator, no executor.
- **`Cargo.toml` `[lints]` tables.** Lints are set in `src/lib.rs` and on the
  clippy command line in `rust/Makefrag.am`.
- **Nightly rustfmt options** (`group_imports`, `imports_granularity`). The
  channel is stable; `rust/rustfmt.toml` sets `edition` and `max_width` only.
- **`#[non_exhaustive]` on error enums.** There are no downstream crates —
  every match on `Error` is in this tree, and an exhaustive match is what
  catches the arm you forgot when a variant is added.
- **MIT/Apache dual licensing.** See the license header rules under Git.
<!-- agents-md:end id=conventions -->

<!-- agents-md:begin id=git -->
## Git & PR workflow

Upstream is `savannah` (`git.savannah.gnu.org/git/hurd/gnumach.git`); `origin`
is the fork this work lands on. Work happens on topic branches, not on the
upstream default branch.

Commit messages are `subsystem: imperative summary`, lowercase after the
colon, wrapped at 72 columns, with the why in the body:

```
vm: port vm_map_enter to Rust
ipc: fix the M5d2 review findings
docs: stop naming the deleted vm_map.c as a live rbtree user
```

One logical change per commit. A port and its review fixes are separate
commits, and a test fix is never folded into the change it was blocking.

`.githooks/pre-commit` runs the whole qemu suite on every commit. Enable it
with `git config core.hooksPath .githooks`. `SKIP_TESTS=1` and `--no-verify`
bypass it; both are a deliberate statement that you ran the suite another way.

### Adding a routine

1. Write it under `rust/src/utils/`, `rust/src/kern/`, `rust/src/ipc/`,
   `rust/src/vm/` or `rust/src/arch/<arch>/`, name the C file it replaces in
   the module doc comment, and **add it to `MACH_RS_SRCS` in
   `rust/Makefrag.am`**.
2. Write the core as native Rust, by the rules — safe types, no
   out-parameters, no integer error codes — and keep the C-shaped signature in
   a thin `extern "C"` adapter over it.
3. Give the adapter the C signature exactly, `#[unsafe(no_mangle)]` and
   `extern "C"`, and a `# Safety` section saying what the caller has to
   guarantee.
4. Delete the C definition in the same commit. Two definitions of one symbol
   is a link error, not a fallback. If the step cannot be taken without
   writing C, it is the wrong step: pick a different one, by the no-glue
   law.
5. The test programs are the frozen binaries in `abi-test/`, so they need no
   copy here; a port must keep the behaviour their ABI pins.
6. Record the move in `MIGRATE.md` §9, and in the overview table above.
7. `mise run test` — both architectures green — before committing.

### Checklist before a commit

1. No test was weakened, skipped, shortened or deleted, and no lint, assertion
   or allowlist was loosened to get green. Coverage is the same or better.
2. The C definition of every symbol the Rust now defines is deleted in the
   same commit.
3. No C was written. No new `*_glue.c`, no new function in an existing one,
   no accessor or prototype added to a C header for Rust's benefit.
4. Every new `.rs` file is in `MACH_RS_SRCS`.
5. The rules hold for the new module, not only the parts clippy can check.
6. The comment pass was run over the diff: every comment is an SPDX
   header, a module `//!`, a `# Safety` section, a `// SAFETY:` block, or
   a recorded why the code cannot carry. Anything else was deleted and
   the code renamed instead.
7. New code is edition 2024 idiom: `unsafe extern "C"` blocks,
   `#[unsafe(no_mangle)]`, no `static mut`.
8. `mise run test` — x86_64 **and** i386 — is green.
9. `MIGRATE.md` records what moved.

### Review

Compiling and passing the suite is necessary, not sufficient. Re-read the diff
against the rules above before reporting a task complete, paying particular
attention to every `// SAFETY:` comment: is the invariant it names actually
guaranteed by the code that calls it?

`/code-review` runs a review of the pending diff at a chosen effort level. Do
not block a commit on style nits — note them and clean them up in a follow-up.
When a review turns up a rule that is not written down here, add it to this
file rather than leaving the comment on one diff.

### License headers

**Source files carry an SPDX header. Build files do not.**

New code, and a new implementation of a public interface, are
BSD-2-Clause.  A public interface is one a standard defines, POSIX first
among them: the routines in `rust/src/utils/string.rs` and the byte
swaps in `rust/src/utils/byteorder.rs` are theirs.  A genuinely new
design is BSD too, for the same fair-use reason that only the
expression is protected: `rust/src/kern/smp.rs` is an `AtomicU8` where
the C had a plain byte.

```
// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
```

Everything else is a translation of Mach's own code, and a translation
is a derivative work.  The SPDX line names the source's license, the
source's copyright notice follows, and the new copyright line goes
last:

```
// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_map.c and vm/vm_map.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
```

The test is the code, not the header comment.  Tracking the C routine,
its control flow, its case order, its constants, its comments, makes a
derivative; writing the same interface with a design of one's own does
not.  `rust/src/utils/atoi.rs` translates `util/atoi.c` and carries
`CMU-Mach`, and `rust/src/kern/elf_load.rs` reworks the loader until
nothing of the C's structure remains and stays BSD.

The source licenses to name in the translation case:

- Carnegie Mellon Mach code, with the Utah and Olivetti notices that
  travel with it: `CMU-Mach`.
- Richard Braun's `kern/rbtree.*` and `kern/list.h`: `BSD-2-Clause`.
- FSF additions under the GNU GPL, among them `i386/i386at/mbinfo.c`:
  `GPL-2.0-or-later`.  A translation of GPL code stays
  `GPL-2.0-or-later`; copyleft does not allow relicensing.

A module that merges translations from several sources carries the
strictest and reproduces every notice.

Every file that is changed carries the new copyright line, with the
year of the change; a file touched in several years lists them,
`2026, 2027`, and a file that lacks the line gains it in the same
commit.

Use `//` in `.rs`, `dnl` in `.ac`, `/* */` in `.c` and `.h`.  Keep the
new copyright line exactly as above.

**No header** on `AGENTS.md`, `MIGRATE.md`, `rust/Makefrag.am`,
`rust/configfrag.ac`, `rust/rustfmt.toml`, or `rust/targets/*.json` (JSON has
no comment syntax, so it is left bare). This matches the C half, where
`Makefrag.am` and `configfrag.ac` have none either.
<!-- agents-md:end id=git -->

<!-- agents-md:begin id=gotchas -->
## Gotchas & hard-won lessons

Things that break the build, silently or confusingly, if forgotten.

- **A new `.rs` file not in `MACH_RS_SRCS` still compiles.** It is reached
  through its `mod` declaration, so nothing fails — but it is never
  format-checked, the lint stamp does not rebuild when it changes, and it is
  not distributed. `rust/src/arch/i386/kd/tty.rs` sat like that from
  `b313678b` until it was noticed.
- **`#![no_builtins]` in `rust/src/lib.rs` is load-bearing.** It guards
  against LLVM rewriting a byte-copy loop into a call to `memcpy` — in this
  crate, a call to itself. It was *not* observed to do so at
  `-C opt-level=2 -C lto=fat` on rustc 1.98; the attribute is there so that a
  change of pass, level or toolchain cannot make it happen. Do not drop it as
  "unused": nothing tests for it.
- **`Makefile.am` names `libkernel.a` twice around `libmach-rs.a`.** The two
  archives call into each other, and one pass over each cannot resolve that.
  `--start-group` would say the same thing, but Automake rejects linker flags
  in a `_LDADD`.
- **`-C lto=fat` is not an optimisation.** Without it the archive keeps
  `core`'s float formatting and the soft-float intrinsics libgcc does not
  provide, which `gnumach-undef-bad` rejects. The same check is what catches a
  64-bit atomic on i686 reaching for libatomic — at link time, on one
  architecture only, which is why both architectures are always the gate.
- **`RUSTC_BOOTSTRAP=1`, custom target JSONs, and compiling `core` out of tree
  are all nightly-only knobs** on a stable channel, so the recipes set it.
- **`rust-toolchain.toml` pins the toolchain, and an absolute `RUSTC` path is
  not enough.** With rustup, a path means "whatever toolchain is default this
  week", and a build that mixes two of them fails with E0514 ("compiled by an
  incompatible version of rustc") rather than saying so.
- **mise does not simply obey `rust-toolchain.toml`.** It exports
  `RUSTUP_TOOLCHAIN`, which outranks the file. `mise.toml` sets
  `idiomatic_version_file_enable_tools = ["rust"]` so that mise reads the file
  instead. Never add a `rust` entry to a `[tools]` section there: the second
  pin wins and then drifts.
- **`RUST_LIB_SRC` comes from `configure`, so it goes stale when the toolchain
  updates.** It then points at the old toolchain's `core` sources while `make`
  uses the new rustc. Reconfigure after a toolchain update; the `mise run
  build` and `mise run test` tasks compare it against `rustc --print sysroot`
  and reconfigure when it has moved.
- **The test programs are frozen user-mode binaries.** They live in
  `abi-test/` and link their own copies of the string, `printf` and `atoi`
  routines rather than `libmach-rs.a`, which is built for the kernel's
  target. Moving a routine they also contain changes only the kernel side;
  the pack is the ABI those binaries pin.
- **A C macro cannot be declared in `glue/`, and may not be shimmed.**
  `spl*`, `simple_lock`, `percpu_get`, `current_thread()` and
  `thread_wakeup*` are macros or assembly. Under the no-glue law that makes
  them an ordering constraint, not a shim: their definer is ported first and
  exports a real symbol, and until it has, every routine that needs one of
  them is blocked and stays C.
- **Rust cannot express C bitfields.** Expose the raw word and mask through
  typed accessors while the C macros keep reading the same word.
  `src/kern/lock.rs`'s `LockData` documents the packing order in its doc
  comment and leaves the word as one `u32`.
<!-- agents-md:end id=gotchas -->

<!-- agents-md:begin id=security -->
## Security & secrets

This is a kernel; the security surface that matters day to day is the FFI
boundary, and it is covered by the rules above. Two housekeeping items:

- **`mise.local.toml` contains live API credentials and must never be
  committed, quoted, logged, or pasted into a commit message, an issue, or a
  reply.** It is currently excluded through `.git/info/exclude`, which is
  local to this clone and does not travel — a fresh clone has no protection
  for it. Treat any file matching `*.local.toml` as secret.
- **Nothing in the build fetches from the network.** No Cargo, no lock file,
  no build script, no `curl` in a recipe. If a change would add a network
  fetch to the build, it is the wrong change.

There is no `.env` and no secret in the tracked tree. If you find one, say so
rather than working around it.

Untrusted input in the code itself is anything crossing the user/kernel
boundary: `copyin`/`copyout` buffers, MIG message payloads, and the multiboot
command line. Validate it in the adapter, before it reaches safe Rust, and
never trust a length C handed you without checking it against the mapping.
<!-- agents-md:end id=security -->

<!-- agents-md:begin id=boundaries -->
## Boundaries

The most important section. Keep it current.

- ✅ **Always**: run `mise run test` on **both** architectures before claiming
  a change works; delete the C definition in the same commit as the Rust one
  that replaces it; add every new `.rs` file to `MACH_RS_SRCS`; write a
  `// SAFETY:` comment on every `unsafe` block and a `# Safety` section on
  every exported `unsafe extern "C" fn`; record the move in `MIGRATE.md`;
  carry the correct SPDX header (BSD-2-Clause on new code and public
  interfaces, the source's license on a translation); pick a port order in
  which no glue is needed, and stop rather than write C when none exists;
  take the free ports of `MIGRATE.md` §6.1 before proposing any
  infrastructure.

- ⚠️ **Ask first**: adding an allocator (`GlobalAlloc` over `kalloc`) or
  anything that allocates; changing a `#[repr(C)]` layout, a MIG signature, an
  asm-read symbol name, or an entry in `mach_trap_table`; changing the
  toolchain pin, the target JSONs, or the codegen flags; touching
  `Makefile.am`'s link order; changing observable kernel behaviour; adding to
  the `gnumach-undef` allowlist; renaming an exported symbol C still calls.

- 🚫 **Never**: write a comment that restates the code, narrates a step, or
  documents an item whose name and signature already say it — the four
  mandatory kinds are the whole budget (see "The comment budget");
  weaken, skip, shorten or delete a test to get green, in any of
  the ways listed under Testing; add a Cargo manifest, a lock file, a build
  script, an external crate, or anything that fetches from the network; add
  `#[allow]` or `-A` to silence a lint that is telling the truth; drop
  `-D warnings`, `#![no_builtins]`, `-C lto=fat`, or a `const` layout
  assertion; commit `mise.local.toml` or quote its contents; use `static mut`;
  write any new C — a `*_glue.c`, a function in an existing one, a shim, an
  accessor or a header prototype for Rust's benefit — where the answer is a
  different port order; leave two definitions of one symbol in the tree;
  edit `build-64/`, `build-32/`, `configure`, `Makefile.in`, or any other
  generated file by hand;
  force-push a shared branch.
<!-- agents-md:end id=boundaries -->

## Notes
<!-- Human-owned. Anything you write here is never touched by re-runs of agents-md. -->

- Sources for rules 21–44: the Linux kernel Rust coding guidelines, the
  rust-analyzer style guide, the Rust API guidelines, and Apollo's Rust best
  practices. Rules that assume `std`, an allocator, async, serde or crates.io
  publishing were dropped; the survivors are listed above with kernel-shaped
  examples. See "Deliberately not adopted" for the ones rejected on purpose.
- `## The no-glue law` is project-specific and has no counterpart in the
  external agents-md template. It must survive a regeneration; check it is
  still there afterwards.
- The `## Commands` and `## Testing` text was rewritten on 2026-09 for the
  frozen ABI pack in `abi-test/`; the external agents-md template still
  describes `make check` and the deleted `tests/` tree, so it needs the same
  edit before the next regeneration.
- `--enable-user32` is out of scope.  The Rust half builds for the two ABI
  gate configurations, i686 and x86_64, and no Rust declaration uses the
  `rpc_vm_*` 32-bit types.  Do not add `--cfg user32` plumbing or a
  user32-shaped declaration for a shim: the deleted
  `memory_object_create_proxy` shim was the last one, and reviving the
  configuration is a project decision, not a port.

---
<!-- Content INSIDE `agents-md:begin/end` markers is regenerated on re-run.
     Everything OUTSIDE the markers (including ## Notes) is preserved.
     Template: agents-md (https://github.com/eugeniughelbur/agents-md). -->
