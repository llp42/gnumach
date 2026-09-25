# MIGRATE.md — C-to-Rust migration map

This map was rebuilt on 2026-09-23 from a fresh derivation over the whole
C tree.  The previous map had drifted: its Tier 0 list named functions
that were already ported, called others free that were not, and its
"already moved" table still called landed ports "pending".  That file is
frozen beside this one as `migrate.md.old`; nothing in it is updated any
more.

The port contract is in `AGENTS.md`; this file is the map, not the rules.
The no-glue law governs everything here: no port may write C, so a file
moves only when everything it needs is either a real symbol `glue` can
declare or Rust already.  Where that is not true, the answer is a
different port, and §6.2 names the unlock.

The free list in §6.1 was derived by parsing every function definition in
the eight C directories with clang, filtering against `nm` on `build-64`
and `build-32`, and reading every survivor against the seven questions of
§6.  §6.3 records the method, including the two traps that made the
earlier lists wrong.

## 0. What a port is (the contract)

1. A Rust definition replaces a C definition of the **same symbol and
   signature**, and the C definition is deleted in the same commit.  Two
   definitions is a link error, not a fallback.
2. `libmach-rs.a` is linked between two passes over `libkernel.a`, so
   Rust may call C and C may call Rust.  `gnumach-undef-bad` rejects any
   symbol not covered by the allowlist.
3. No Cargo, no build script, no network.  `core` only, **no `alloc`**.
   Memory comes from the caller, or from the kernel allocators
   (`kalloc`, `kmem_cache_*`) declared in `glue` as the real C symbols
   they are.
4. clippy `-D warnings` and `rustfmt --check` at 79 columns are build
   gates; the target and codegen flags are in `rust/Makefrag.am`.
5. `mise run test` builds both kernels and runs the frozen ABI pack in
   `abi-test/` against them, on x86_64 **and** i386.  It is the only
   correctness gate.
6. `rust/src/` mirrors the C tree; the C functions Rust calls are
   declared verbatim in `rust/src/glue/`.  That declaration writes no C
   and is the whole permitted bridge inward.
7. **No new C.**  A C macro cannot cross FFI and may not be shimmed, so
   whatever defines it is ported first.  A port that cannot be done
   without writing C is not next; another one is.

## 2. The layers — why files depend on each other

A file cannot move until the layer under it has a Rust story.  The state
column is what exists in the tree today, not a plan.

| Layer | What it is | State today |
|---|---|---|
| **L0 pure** | strings, byte order, atoi, parser tables | done |
| **L1 types** | structs read field-by-field, sometimes by asm | `Thread`, `Processor`, `ProcessorSet`, `RunQueue`, `Timer`, `Timeout`, `QueueEntry`, `SimpleLock`, `TimeValue`/`TimeValue64`, `VmMap`/`VmMapEntry`/`VmMapHeader`/`VmMapLinks`, `VmPage`, `VmObject`, `Task`/`MachineTask`, `KmemCache`, `MachineSlot`, `struct ipc_port` (with its `ipc_target` and `ipc_mqueue`), `struct ipc_space`, `struct ipc_kmsg`, `struct ipc_entry`, `struct ipc_marequest`, `ApicLocalUnit`, `ApicIoUnit`, `ApicInfo`, `IoApicData` and the packed ACPI tables are `#[repr(C)]` mirrors with size, alignment and offset asserts.  `struct pcb` has no field mirror; the `struct bus_device`/`bus_ctlr`/`bus_driver` trio it listed as missing landed with `src/arch/i386/com.rs`. |
| **L2 locks/IRQ/percpu** | `simple_lock`, `spl*`, `percpu_get`, `current_thread()` | done: `kern/lock.c` and `i386/i386/lock.h` are gone, `SimpleLock` is `src/kern/lock.rs`, `spl*` are real asm functions in `glue`, and `current_thread()`, `cpu_number()` and `percpu_get` live in `src/arch/i386/percpu.rs`.  An RAII `IrqGuard` is a Rust-side type to write when wanted. |
| **L3 memory** | `kalloc`/`kfree`, `kmem_cache_*` | done: `kern/slab.c` is gone, `src/kern/slab.rs` owns the allocator and `src/kern/slab_ffi.rs` exports its C symbols.  A `GlobalAlloc` over `kalloc` remains a design conversation. |
| **L4 runnable** | `thread_block`, `assert_wait`, continuations | the scheduler is Rust whole: `sched_prim.rs` owns the wait/wake primitives, `thread_block`/`thread_invoke`/`thread_select`/`thread_run`, the run-queue and stuck-thread scans, and `timer.rs` owns the statistical timers, with `sched_prim_ffi.rs`/`timer_ffi.rs` exporting the C symbols; only the asm entry points (`call_continuation`, `Switch_context`) stay C. |
| **L5 IPC/VM** | ports, spaces, kmsgs, maps, objects, pages | `vm_map` and `vm_object` are Rust-native, and `struct task` is mirrored; the rest have no field mirrors.  The page field shims and the map and external slab caches are Rust statics, so both VM glue files are deleted. |
| **L6 arch/MIG** | MIG output, trap table, pmap, locore | stays C.  MIG routines are not generated: the generated server calls the hand-written definition, so a Rust port replaces only that definition. |

The two hard ABI walls are unchanged.  **MIG** (`*.srv`/`*.cli` →
`*.server.c`/`*.user.c`) keeps its unmarshalling, `TypeCheck` and
`*_server_routines[]` table in C.  **Trap/asm** fixes
`mach_trap_table`'s stride and `i386asm.sym`'s symbol names, sizes and
order.

## 4. `kern/` — file-by-file

LOC is the current file; friction is carried from the 2026-09 audit and
is a guide, not a measurement.  "Free" is the number of functions in
§6.1; "holds the rest" names the thing that blocks the next port in the
file, or `—` when the rest is ready too.

| File | LOC | Friction | Free | Holds the rest |
|---|---:|---:|---:|---|
| `debug.c` | 114 | 3 | 0 | C variadics (`Panic`, `log`) |
| `ipc_sched.c` | 163 | 4 | 0 | — |
| `mach_factor.c` | 150 | 2 | 0 | `mach_factor[]`/`load_average[]` are NCPUS-sized |
| `printf.c` | 592 | 5 | 0 | C-variadic definitions; blocked (see §8) |
| `priority.c` | 196 | 4 | 0 | pset tail and `struct slock_irq` |
| `syscall_sw.c` | 220 | 3 | 0 | trap table ABI; static stubs |

## 5. Outside `kern/`

### `ipc/` (0 files)

`copy_user.c` was the last one.  Its only live definition was the LP64
kernel's `copyinmsg()`, now `src/ipc/copy_user.rs`; the i386 kernel takes
that entry point from `i386/i386/locore.S`, and the file's `USER32` half
never compiled in either configured build (§8, §9).

### `vm/` (1 file, 2,024 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `vm_fault.c` | 2024 | 0 | the pinned toolchain folds the copy-object null test (§9) |

`memory_object.c`, `vm_resident.c`, `vm_kern.c`, `vm_pageout.c`,
`vm_user.c`, `vm_debug.c` and `memory_object_proxy.c` are whole: the
`memory_manager_default` port and its lock, the `vm_page_bucket_t` hash
table, the fictitious-page list, `virtual_space_start`/`virtual_space_end`,
the kernel map globals, the pageout daemon's statics, the `vm_stat` block,
the VM-debug info records and the proxy slab cache all moved with them
(§9).  `vm_fault.c` was attempted and put back whole; the blocker is
recorded in §9.

### `device/` (6 files, 1,610 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `cons.c` | 176 | 0 | `cn_tab` static table |
| `device_init.c` | 49 | 0 | — |
| `dev_name.c` | 166 | 0 | `dev_ops`/`dev_indirect` fields |
| `intr.c` | 375 | 0 | `struct irqdev`/`user_intr_t` fields |
| `kmsg.c` | 237 | 0 | — (the rest is message plumbing) |
| `subrs.c` | 53 | 0 | `ifnet` fields |

### `i386/` (10 files, 1,157 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `i386/hardclock.c` | 69 | 0 | `machine_slot` and interrupt plumbing |
| `i386/machine_task.c` | 70 | 0 | `task.machine` fields |
| `i386/percpu.c` | 31 | 0 | `struct percpu.self` field |
| `i386/phys.c` | 164 | 0 | mapped-window internals for `pmap_copy_page` etc. |
| `i386/pic.c` | 270 | 0 | not compiled in the APIC configuration |
| `i386at/autoconf.c` | 127 | 0 | `bus_device`/`bus_ctlr` fields |
| `i386at/conf.c` | 144 | 0 | static tables |
| `i386at/cons_conf.c` | 48 | 0 | static tables |
| `i386at/pic_isa.c` | 56 | 0 | not compiled in the APIC configuration |
| `intel/read_fault.c` | 178 | 0 | dead: body is `#if`-ed out on every supported CPU |

`chips/busses.c` (232 LOC) is still C, but the `struct bus_device`,
`struct bus_ctlr` and `struct bus_driver` mirrors it reads now exist in
`src/arch/i386/com.rs`, asserts included, so its field gap is closed.
There are no C files under `x86_64/`.

## 6. What to port next — the objective test

Choosing work is a seven-question test, applied to a single C **function**,
not to a file.  Questions 1–5 ask what the function does; 6 and 7 ask
whether it can be replaced at all.  §6.3 decides both from the built
objects rather than the source.

| # | Question | If the answer is "no" |
|---|---|---|
| 1 | Is every function it calls a real linker symbol, not a `#define` or `static inline`? | Port the definer first, or pick another function.  A shim is not available. |
| 2 | Is every struct field it touches covered by a Rust mirror that exists **today**, laid out by `const` asserts?  `sizeof` and by-value passing count as field uses. | Mirror that struct first; §6.2. |
| 3 | Does it avoid every array sized by a configure-time constant (`NCPUS`, `NINTR`, `NCOM`, `NIPL`)? | Blocked; §6.2. |
| 4 | Is it non-variadic and free of `va_list`? | Blocked.  See `kern/printf.c`. |
| 5 | Is its inline assembly expressible with `core::arch::asm!`? | Blocked on the arch layer. |
| 6 | Is it visible outside its own translation unit — non-`static`, with a prototype in a header? | Its callers move with it, or it waits.  A Rust definition of a `static` C function is unreachable, and adding the `extern` declaration that would reach it is writing C. |
| 7 | Is the definition live in a buildable configuration, not a dead `#if` branch? | It is a deletion, not a port.  §8. |

These exemptions are settled and are not re-decided per port:

* **Locks pass question 1.**  `simple_lock`/`simple_unlock`/
  `simple_lock_try`/`simple_lock_irq` expand to
  `mach_simple_lock`/`mach_simple_unlock`/`mach_simple_lock_try`
  (`rust/src/kern/lock.rs`) or `splhigh()` plus one of them.  All real.
* **`spl*` pass.**  Every one is a real asm function, already declared in
  `glue`.
* **`current_thread()`, `cpu_number()` and `percpu_get` pass.**
  `rust/src/arch/i386/percpu.rs` is the Rust equivalent; Rust never
  invokes the macro.
* **`thread_wakeup*` pass.**  The macro expands to `thread_wakeup_prim`,
  which is Rust; Rust calls it directly.
* **`inb`/`outb`/`cpuid`/`rdmsr`/`wrmsr` pass question 5.**  They are
  statement-expression macros, but each is one instruction `asm!` emits
  directly.
* **A C global of unmirrored type passes question 2 when only its address
  is used.**  Declare it in `glue` as an opaque `extern` static and take
  `&raw mut`.  Reading a *field* of one is a different thing and still
  fails.

### 6.1 Free ports today (Tier 0) — empty

The last six entries moved in one pass (see §9).  An empty list is not a
finished file or a finished tree: §6.3's re-derivation is what refills
it, and the previous mechanical pass found forty entries the list had
never been pointed at.  Re-derive before reading a file's "Free: 0" as
final.

### 6.2 Blocked with one unlock

**NCPUS/NINTR/NCOM (15).**  The configure constants are in
`rust/src/config.rs`, the `processor_set` tail they sized is mirrored, and
the four `processor_glue.c` shims plus `thread_glue_pset_sched_load` are
deleted (§9, §10).  Five of the 20 moved in a follow-up pass:
`init_timers`, `ast_init`, `host_processors`, `pset_sys_init` and
`chario_init` (§9).  Of the 15 this unlock freed, `pmap_virtual_space`
went with the `pmap.c` port, `picdisable` and the four `i386/i386/irq.c`
accessors went with the whole-file `irq.c`/`ioapic.c` ports (§9), and the
eight `i386/i386at/com.c` entries went with that whole-file port.  The
last of the NCPUS-sized functions, `interrupt_stack_alloc`, went with the
whole-file `i386/i386/mp_desc.c` port, which also carried the two arrays
it sized (§9).

**Mirror gaps.**
`host_ipc_marequest_info` and `host_virtual_physical_table_info` needed a
`hash_info_bucket_t` mirror; the `HashInfoBucket` in `src/ipc/mod.rs`
landed with the `ipc_marequest.c` port, and the `mach_debug.c` batch
moved `host_ipc_marequest_info`; `host_virtual_physical_table_info` went
with the whole-file `vm_debug.c` port.  The `struct pmap` story now
exists: the whole of `i386/intel/pmap.c`, its `static` `phys_attribute_*`
helpers included, moved to `src/arch/i386/pmap.rs` (§9).
The `vm/vm_object.c` port completed the `struct vm_object` field mirror
(`src/vm/types.rs`) and moved the module to `src/vm/vm_object.rs`, so the
object's lock, flags and page list are Rust; the C files that still call
it keep the `vm/vm_object.h` prototypes.  A follow-up added the
`struct task` mirror (`src/kern/task.rs`, with `struct machine_task` in
`src/arch/i386/machine_task.rs`) with the offsets read from both built
kernels, so the eviction path bumps `current_task()->reactivations` as
the C did.

**Gated decisions.**
`i386/i386/pcb.c`'s whole-file port carried `user_stack_low` with it for
the two gate configurations; the `--enable-user32` third value of
`VM_MAX_USER_ADDRESS` remains out of scope for the Rust half.
`pmap_make_temporary_mapping` and `pmap_remove_temporary_mapping` moved
with the whole-file `pmap.c` port for the non-PAE i386 and PAE x86_64
builds; a PAE i386 build would still need a `--cfg` plumbed through
`rust/configfrag.ac` before `src/arch/i386/pmap.rs` covers it.

### 6.3 How the list was derived

**The built objects are the oracle for questions 6 and 7.**  `nm
--defined-only` over `build-64` and `build-32`:

| `nm` says | Meaning | Verdict |
|---|---|---|
| `T` | externally visible and compiled | passes 6 and 7 |
| `t` | compiled, but local to its unit | fails 6: its callers move with it, or it waits |
| absent | not compiled, or a `static inline` | fails 7 if a definition, a macro if not |

Apply it to the function **and to every callee**.  A `t` callee is the
most common hidden blocker: the four `pmap_*attribute*` functions look
free until `nm` shows their `phys_attribute_*` helpers are `static`, and
`i386/i386/ktss.c`'s `ktss_fill` is the same case.

**Then parse.**  clang's AST dump over each C file with the build's own
flags finds, per definition, its calls, its member accesses and its
locals; two traps matter:

* Inline asm is `GCCAsmStmt` in the dump, not `GNUAsmStmt`.  A pass that
  looks for the wrong node reports every asm function as clean.
* Field accesses must be resolved to their record (`FieldDecl` → parent
  `RecordDecl`) and checked against that record's Rust mirror.  A check
  against the union of all mirror field names passes any struct that
  happens to share one field name, which is how the earlier lists
  collected false positives.

**Then read the survivors.**  Questions 2 and 3 still hide outside the
body: a `.` access, a struct passed by value, a local struct variable, a
`sizeof`, and an array whose size is in its declaration.  The scan is a
filter, not an answer.

**Re-derive after any batch of ports.**  Every mirror that lands and
every constant that becomes visible moves functions out of the rejection
classes.  A derivation is a snapshot of one afternoon's tree.

## 7. Recommended phasing

* **Phase A — the free ports, in file-sized batches.**  (a) the zero-risk
  leaves (`machine_idle`, `machine_relax`, `pcb_collect`,
  `task_ras_control`, the `apic.c` accessors); (b) `vm_user.c` wrappers;
  (c) the ipc wrappers (`ipc_init`, `ipc_object_destroy`, `ipc_port_*`,
  `ipc_thread_*`, `ipc_pset_*`, `ipc_host`); (d) `vm_resident.c`;
  (e) the `kern/thread.c`/`sched_prim.c` scheduler
  batch; (f) the `model_dep.c` clock and console leaves.

* **Phase B — unlock work.**  The constants are in `rust/src/config.rs`;
  the 20 functions they freed are next, then the `struct vm_page` mirror;
  `struct pmap` followed with the whole-file `pmap.c` port.  The
  `hash_info_bucket_t` mirror landed with `ipc_marequest.c`.

* **Phase C — the coupled files.**  `priority`, `ipc_tt`, `ipc_host`
  and `host` once their struct stories exist, then the anchors
  (`exception`, `bootstrap`, `trap`, `pcb`, `ipc_kmsg`).
  `eventcount`, `processor`, `machine` and the whole of `sched_prim`
  and `timer` are done.

Exit criterion for every step: both qemu architectures green, `rustfmt`
and clippy clean, no new undefined symbols, and no new C.

**Not in any phase: `kern/printf.c`.**  Its variadic definitions
(`printf`, `iprintf`, `snprintf`, `vprintf`, `_doprnt`) cannot be written
in the pinned toolchain.  The two non-variadic leaves, `printnum` and
`safe_gets`, are Rust now (see §9).

## 8. Deletions

* `i386/intel/read_fault.c`: the body is
  `#if (__i386__ && !(__i486__ || __i586__ || __i686__))`, compiled out
  on every supported CPU.  Delete, do not port.
* `#if 0` blocks in `kern/exception.c`, `device/intr.c`,
  and `i386/i386at/kd.c`.  Delete before porting the surrounding code.
  The `#if 0` bodies of `kern/{boot_script,bootstrap}.c`,
  `i386/i386at/com.c`, `kern/ipc_tt.c` (four `retrieve_*` bodies) and
  `i386/i386/{fpu,pcb,smp,trap}.c` went with their whole-file ports.
* Dead `#else /* MACH_HOST */` halves of `kern/machine.c` and
  `kern/processor.c`; `MACH_HOST` is 1 in both configured builds, so only
  the live halves were translated.  The `kern/task.c` and
  `kern/thread.c` halves went with their files.
* Macro-shadowed definitions: `i386/intel/pmap.c`'s `pmap_copy` and
  `pmap_kernel` were unreachable behind `i386/intel/pmap.h`'s macros and
  went with the file's port.
* `ipc/copy_user.c`'s `#ifdef USER32` half: `copyoutmsg()`, `msg_usize()`
  and the twelve user-type conversion helpers are compiled out of both
  configured builds, so they were deleted with the file rather than ported.
  The header beside them had already lost its last C caller and went with
  it.
* `i386/i386/pic.c` and `i386/i386at/pic_isa.c` are not compiled in the
  APIC configuration.  They stay until the non-APIC configuration is
  either built or dropped; they are not port targets.

## 9. Already moved (for reference)

| C unit | Rust | Commit |
|---|---|---|
| `kern/strings.c`, `i386/i386/strings.c` | `src/utils/string.rs` | `c4a7f57c` |
| `kern/queue.c` | `src/kern/queue.rs` | `5fcbebe5` … `1441f06a` |
| `kern/smp.c` | `src/kern/smp.rs` | `c2375cdc` |
| `i386/i386/loose_ends.c` (`delay`) | `src/utils/delay.rs` | `87d85e0c` |
| `util/byteorder.c` | `src/utils/byteorder.rs` | `7ad91b7b` |
| `kern/elf-load.c` | `src/kern/elf_load.rs` | `308594ca` |
| `i386/i386at/kd_queue.c` | `src/utils/kd_queue.rs` | `ed2502e9` |
| `i386/i386at/kd_mouse.c` | `src/arch/i386/kd_mouse.rs` | `64f44fa8` |
| `i386/i386at/kd_event.c` | `src/arch/i386/kd_event.rs` | `5d6a289a` |
| `i386/i386at/kd.c` | `src/arch/i386/kd/` | `009b05af` … `f1512f88` |
| `i386/i386at/mem.c` | `src/arch/i386/mem.rs` | `548186d3` |
| `i386/i386at/mbinfo.c` | `src/arch/i386/mbinfo.rs` | `21fcbe0b` |
| `kern/rbtree.c` | `src/kern/rbtree.rs` | `9445e08b` … `e2b04831` |
| `ipc/ipc_thread.c` | `src/ipc/ipc_thread.rs` | `417ba80a` |
| `util/atoi.c` | `src/utils/atoi.rs` | `c289337f` |
| `vm/vm_external.c`, `vm/vm_init.c` | `src/vm/vm_external.rs`, `src/vm/vm_init.rs` | `727275e7` |
| `vm/vm_map.c` | `src/vm/vm_map.rs`, `src/vm/vm_map_ffi.rs` | `d32c7253` … `170e6104` |
| `kern/lock.c`, `i386/i386/lock.h` | `src/kern/lock.rs`, `src/arch/i386/atomic_bits.rs` | `9a9ced86`, `f0a3cb2c`, `102c4926` |
| `kern/sched_prim.c` (wait/wake, `thread_dispatch`, `thread_setrun`) | `src/kern/sched_prim.rs`, `thread.rs`, `timer.rs`, `processor.rs`, `src/arch/i386/percpu.rs` | `c4498541` |
| `kern/ast.h` (`ast_on`, `ast_off`, `ast_needed`) | `src/kern/ast.rs` | `6a6281be` |
| `kern/ipc_mig.c`, `kern/host.c`, `kern/syscall_subr.c` (leaves) | `src/kern/ipc_mig.rs`, `host.rs`, `syscall_subr.rs` | `c1cf99d4` |
| `device/dev_pager.c`, `kern/debug.c`, `kern/boot_script.c`, `kern/bootstrap.c` (leaves) | `src/device/dev_pager.rs`, `src/kern/debug.rs`, `boot_script.rs`, `bootstrap.rs` | `eecdf229` |
| `device/dev_lookup.c` and `device/dev_pager.c` whole, with the device-number table, the two pager hash tables, the `dev_hdr_cache`/`dev_pager_cache`/`dev_device_hash_cache` slab caches and the `mach_device_reference`/`mach_device_deallocate`/`dev_port_*` entries they owned; `device/dev_pager.h` deleted with them | `src/device/dev_lookup.rs`, `dev_lookup_ffi.rs`, `dev_pager.rs`, `dev_pager_ffi.rs` | pending |
| `kern/mach_clock.c`, `syscall_emulation.c`, `ipc/ipc_target.c`, `kern/task.c`, `kern/thread.c` (stubs) | `src/kern/mach_clock.rs`, `syscall_emulation.rs`, `src/ipc/ipc_target.rs`, `src/kern/task.rs`, `thread.rs` | `5bc0c535` |
| `i386/i386/machine_task.c`, `i386/i386at/model_dep.c`, `i386/intel/pmap.c` (leaves) | `src/arch/i386/machine_task.rs`, `model_dep.rs`, `pmap.rs` | `8dbc4e8e` |
| `i386/i386at/model_dep.c` (`machine_idle`, `machine_relax`, `timemmap`, `inittodr`, `resettodr`, `init_alloc_aligned`, `pmap_grab_page`) | `src/arch/i386/model_dep.rs` | pending |
| `kern/kmutex.c` | `src/kern/kmutex.rs` | `d4fe54dc` |
| `ipc/ipc_table.c` | `src/ipc/ipc_table.rs` | `bd582ec6` |
| `device/cirbuf.c` | `src/device/cirbuf.rs` | `ed2151aa` |
| `device/subrs.c` (partial) | `src/device/subrs.rs` | `85edf33a` |
| `kern/thread_swap.c` | `src/kern/thread_swap.rs` | `f80a3253` |
| `i386/i386/ast_check.c` | `src/arch/i386/ast_check.rs` | `5274f324` |
| `kern/debug.c` (`SoftDebugger`, `Debugger`, `panic_init`) | `src/kern/debug.rs` | `6dcb3aa2` |
| `i386/i386/mp_desc.c` (`simple_lock_pause`, `cpu_control`) | `src/arch/i386/mp_desc.rs` | `8fd43a5f` |
| `i386/i386/fpu.c` (`fp_free`) | `src/arch/i386/fpu.rs` | `14fbd933` |
| `device/dev_name.c` (`name_equal`, stubs) | `src/device/dev_name.rs` | `9187f3bb` |
| `device/net_io.c` whole, with the four `def_simple_lock_data(static, ...)` locks (`net_queue_lock`, `net_queue_free_lock`, `net_kmsg_total_lock`, `net_hash_header_lock`), the `net_rcv_cache`/`net_hash_entry_cache` caches, the filter lists, and the `struct ifnet`, `struct ifqueue` and `struct net_rcv_msg` mirrors the receive paths read | `src/device/net_io.rs`, `net_io_ffi.rs` | pending |
| `ipc/ipc_object.c` (`ipc_object_copyin_type`) | `src/ipc/ipc_object.rs` | `8c8c697f` |
| `ipc/ipc_port.c` (`ipc_port_timestamp`) | `src/ipc/ipc_port.rs` | `54dfe7cd` |
| `ipc/ipc_port.c` whole, with the `ipc_port_multiple_lock_data` static and the `ipc_port_request`, `ipc_entry`, `ipc_space` and `ipc_kmsg` field mirrors it reads | `src/ipc/ipc_port.rs`, `src/ipc/ipc_port_ffi.rs`, `src/ipc/mod.rs` | pending |
| `ipc/mach_port.c` whole, with the `mach_port_deallocate_debug` global it owned and the `mach_port_status_t` view its receive-status call fills | `src/ipc/mach_port.rs`, `src/ipc/mach_port_ffi.rs` | pending |
| `kern/thread.c` (`thread_init`), `kern/sched.h` (`thread_timer_delta`) | `src/kern/thread.rs`, `src/kern/timer.rs` | `16581252` |
| `kern/timer.c` (read/normalize/delta) | `src/kern/timer.rs` | `6858b08c` |
| `kern/processor.c` (`processor_init`, `pset_init`, info and pset entries) | `src/kern/processor.rs` | `498525e5` |
| `kern/machine.c` (`host_reboot`) | `src/kern/machine.rs` | `32ca71ed` |
| `i386/i386at/rtc.c` | `src/arch/i386/rtc.rs` | `5afeaa94` |
| `i386/i386at/pit.c` | `src/arch/i386/pit.rs` | `896ae703` |
| `i386/i386/pcb.c` (`stack_detach`, `load_context`, `pcb_collect`), `i386/i386/phys.c` (`kvtophys`) | `src/arch/i386/pcb.rs`, `phys.rs` | pending |
| `i386/i386/apic.c` whole, with the `ApicLocalUnit`, `ApicIoUnit`, `ApicReg`, `IoApicData`, `IrqOverrideData` and `ApicInfo` mirrors and the `lapic`, `cpu_id_lut`, `apic_data`, `apic_id_mask` and `hpet_period_nsec` globals it owned | `src/arch/i386/apic.rs` | pending |
| `i386/i386at/acpi_parse_apic.c` whole, with the packed ACPI table mirrors, the `lapic_addr` and `hpet_addr` globals and the static MADT it owned | `src/arch/i386/acpi_parse_apic.rs` | pending |
| `i386/i386at/ioapic.c` whole, with the `irqinfo`, `ivect`, `iunit`, `curr_ipl`, `spl_init`, `pic_mode`, `timer_pin`, `calibrated_ticks`, `lapic_timer_val` and `ioapic_lock` statics it owned | `src/arch/i386/ioapic.rs` | pending |
| `i386/i386/irq.c` whole, with the `struct irqdev`/`user_intr_t` mirrors its `irqtab` is read through and the `nested_irqs` static, and the six accessor shims §10 listed | `src/arch/i386/irq.rs` | pending |
| `device/ds_routines.c` whole, with the `struct io_req`, `struct device`, `struct mach_device`, `struct dev_ops` and `struct device_emulation_ops` mirrors it owned, and its `device_io_map`, `io_inband_cache`, `io_trap_cache`, `io_done_list` and `mach_device_emulation_ops` globals | `src/device/ds_routines.rs`, `ds_routines_ffi.rs`, `src/arch/i386/io_req.rs` | pending |
| `device/device_init.c` (`device_service_create`), `device/intr.c` (`irqgetstat`), `device/kmsg.c` (`kmsggetstat`) | `src/device/device_init.rs`, `intr.rs`, `kmsg.rs` | pending |
| `device/chario.c` whole, with the `struct tty`, `struct ldisc_switch` and `struct tty_status` mirrors and the `tthiwat`, `ttlowat`, `linesw`, `tty_inq_size`, `tty_outq_size`, `pdma_default`, `pdma_timeouts` and `pdma_water_mark` globals it owned | `src/device/chario.rs`, `chario_ffi.rs` | pending |
| `kern/ipc_mig.c` whole, with the `mach_msg`/`syscall_*` RPC stubs and the `port_name_to_*` send-right lookups | `src/kern/ipc_mig.rs`, `src/kern/ipc_mig_ffi.rs` | pending |
| `kern/ipc_sched.c` (`thread_go`, `thread_will_wait`, `thread_will_wait_with_timeout`) | `src/kern/ipc_sched.rs` | pending |
| `kern/ipc_tt.c` whole, with the `struct ipc_port`/`ipc_target`/`ipc_mqueue` field mirror its `ip_srights` bump reads | `src/kern/ipc_tt.rs`, `src/kern/ipc_tt_ffi.rs`, `src/ipc/mod.rs` | pending |
| `kern/mach_clock.c` whole, with the `hz`, `tick`, `time`, `uptime`, `elapsed_ticks`, `softticks`, `mtime`, `clock_boottime_offset`, `timedelta`, `tickdelta`, `tickadj`, `bigadj`, `last_hpc_read`, `timeoutwheel`, `timeout_timers`, `nextsoftcheck`, `timeout_lock` and `twheel_lock` globals it owned, and the `read_time_stamp`, `host_set_time`, `host_adjust_time` and `host_adjust_time64` entries that had already moved | `src/kern/mach_clock.rs`, `src/kern/mach_clock_ffi.rs` | pending |
| `kern/gsync.c` whole, with the `gsync_buckets` table, the `union gsync_key`, `struct gsync_waiter` and `struct vm_args` it owned | `src/kern/gsync.rs`, `src/kern/gsync_ffi.rs` | pending |
| `kern/machine.c` (`action_thread`) | `src/kern/machine.rs` | pending |
| `kern/task.c` (`task_create`, `task_ras_control`, `register_new_task_notification`) | `src/kern/task.rs` | pending |
| `kern/bootstrap.c` (`boot_script_malloc`, `boot_script_free`, `boot_script_free_task`) | `src/kern/bootstrap.rs`, `bootstrap_ffi.rs` | pending |
| `kern/printf.c` (`printnum`, `safe_gets`) | `src/kern/printf.rs` | pending |
| `kern/rdxtree.c` with the `struct rdxtree`/`rdxtree_iter` mirrors | `src/kern/rdxtree.rs`, `rdxtree_ffi.rs` | pending |
| `kern/sched_prim.c` whole, with the wait hash table, `wait_shift`, the stuck-thread scan statics, `sched_tick`/`min_quantum` and the continuations it owned, and `kern/timer.c` whole, with `current_timer`/`kernel_timer` and the nonblocking debug reads | `src/kern/sched_prim.rs`, `sched_prim_ffi.rs`, `src/kern/timer.rs`, `timer_ffi.rs` | pending |
| `kern/syscall_subr.c` (`thread_depress_priority`, `thread_depress_timeout`, `thread_depress_abort`) | `src/kern/syscall_subr.rs` | pending |
| `kern/thread.c` (`stack_alloc_try`, `stack_alloc`, `stack_free`, `stack_collect`, `stack_privilege`) | `src/kern/thread.rs` | pending |
| `kern/thread.c` (`thread_reference`, `thread_force_terminate`, `thread_hold`, `thread_release`, `thread_resume`, `thread_abort`, `thread_start`, `thread_unfreeze`, `thread_get_assignment`) | `src/kern/thread.rs` | pending |
| `kern/ipc_host.c`, `kern/host.c` and `kern/exception.c` whole, with the `Host`/`realhost` object, the host-info record views, the `struct mach_exception` record, the four `mach_msg_type_t` protos and the `exception_raise_misses` counter | `src/kern/ipc_host.rs`, `ipc_host_ffi.rs`, `host.rs`, `host_ffi.rs`, `exception.rs`, `exception_ffi.rs` | pending |
| `kern/thread.c` (`thread_get_state`, `thread_set_state`, `thread_priority`, `thread_set_own_priority`, `thread_max_priority`, `thread_policy`, `thread_wire`, `stack_init`, `thread_stats`, `thread_set_name`, `thread_get_name`) | `src/kern/thread.rs` | pending |
| `i386/i386/fpu.c` (`fpnoextflt`) | `src/arch/i386/fpu.rs` | pending |
| `i386/i386/mp_desc.c` (`interrupt_processor`) | `src/arch/i386/mp_desc.rs` | pending |
| `vm/vm_kern.c` (`projected_buffer_collect`, `projected_buffer_in_range`, `kmem_alloc_wired_flags`, `kmem_alloc_wired`, `kmem_map_aligned_table`, `kmem_alloc_pageable`, `kmem_free`, `kmem_submap`, `kmem_init`, `kmem_io_map_deallocate`) | `src/vm/vm_kern.rs`, `src/vm/vm_kern_ffi.rs` | pending |
| `vm/vm_user.c` (`vm_allocate`, `vm_deallocate`, `vm_inherit`, `vm_protect`, `vm_machine_attribute`, `vm_read`, `vm_write`, `vm_copy`, `vm_object_sync`, `vm_msync`, `vm_get_size_limit`) | `src/vm/vm_user.rs`, `src/vm/vm_user_ffi.rs` | pending |
| `vm/vm_fault.c` (`vm_fault_wire`), `vm/vm_page.c` (`vm_page_seg_name`), `vm/vm_resident.c` (`pmap_steal_memory`, `vm_page_rename`, `vm_page_alloc_flags`, `vm_page_alloc`) | `src/vm/vm_fault.rs`, `vm_fault_ffi.rs`, `vm_page.rs`, `vm_page_ffi.rs`, `vm_resident.rs`, `vm_resident_ffi.rs` | pending |
| `kern/processor_glue.c` (4 shims), `kern/sched_prim.c` (`thread_glue_pset_sched_load`) | `src/config.rs` (`NCPUS`, `NCOM`, `NINTR`), `src/kern/processor.rs`, `src/kern/thread.rs` | pending |
| `kern/ast.c` (`ast_init`), `kern/timer.c` (`init_timers`), `kern/host.c` (`host_processors`), `kern/processor.c` (`pset_sys_init`) | `src/kern/ast.rs`, `src/kern/timer.rs`, `src/kern/host.rs`, `src/kern/processor.rs` | pending |
| `vm/vm_page.c` (`vm_page_set_type`, `vm_page_wire`), `vm/vm_resident.c` (`vm_page_init`, `vm_page_module_init`, `vm_page_grab`, `vm_page_grab_phys_addr`, `vm_page_release`, `vm_page_zero_fill`, `vm_page_copy`) with the `struct vm_page` mirror | `src/vm/vm_page.rs`, `vm_page_ffi.rs`, `vm_resident.rs`, `vm_resident_ffi.rs` | pending |
| `vm/vm_page.c` whole, with the `struct vm_page_seg`, `struct vm_page_boot_seg` and file-private statics it owned, and the `struct vm_object` field mirror its evictor reads | `src/vm/vm_page.rs`, `src/vm/vm_page_ffi.rs`, `src/vm/types.rs` | pending |
| `kern/slab.c` with the `struct kmem_cache` mirror | `src/kern/slab.rs`, `src/kern/slab_ffi.rs` | pending |
| `vm/vm_object.c` whole, with the file-private statics it owned and the `vm_submap_object` placeholder | `src/vm/vm_object.rs`, `src/vm/vm_object_ffi.rs` | pending |
| `kern/task.c` whole, with the file-private statics it owned and the `struct task` mirror | `src/kern/task.rs`, `src/kern/task_ffi.rs` | pending |
| `i386/intel/pmap.c` whole, with the file-private statics it owned and the `struct pmap`, `struct pv_entry`, `pmap_update_list` and `pmap_mapwindow_t` mirrors | `src/arch/i386/pmap.rs` | pending |
| `i386/i386at/biosmem.c` whole, with the file-private statics it owned | `src/arch/i386/biosmem.rs` | pending |
| `kern/thread.c` whole, with the file-private `walking_zombie`, `reaper_thread_continue`, `thread_collect_scan`, `stack_usage` and `stack_statistics`, and the globals it owned | `src/kern/thread.rs`, `src/kern/thread_ffi.rs` | pending |
| `ipc/ipc_init.c`, `ipc/ipc_target.c`, `ipc/ipc_space.c`, `ipc/ipc_entry.c` and `ipc/ipc_object.c` whole, with the `ipc_space_cache`, `ipc_entry_cache`, `ipc_object_caches`, `ipc_space_kernel`, `ipc_space_reply`, `ipc_kernel_map` and `ipc_kernel_map_size` globals | `src/ipc/ipc_init.rs`, `ipc_target.rs`, `ipc_space.rs`, `ipc_space_ffi.rs`, `ipc_entry.rs`, `ipc_entry_ffi.rs`, `ipc_object.rs`, `ipc_object_ffi.rs` | pending |
| `ipc/ipc_kmsg.c` whole, with the `ipc_kmsg_cache` per-CPU array it owned and the `mach_msg_type_t`/`mach_msg_type_long_t` descriptor view its body walks use | `src/ipc/ipc_kmsg.rs`, `ipc_kmsg_ffi.rs`, `src/ipc/mod.rs` | pending |
| `ipc/ipc_right.c` whole, with the `ipc_reverse_insert`/`ipc_reverse_remove` inlines its capability switches used | `src/ipc/ipc_right.rs`, `ipc_right_ffi.rs`, `ipc_space.rs` | pending |
| `ipc/ipc_mqueue.c` whole, with the `struct ipc_marequest` view its receive path tears down | `src/ipc/ipc_mqueue.rs`, `ipc_mqueue_ffi.rs` | pending |
| `ipc/ipc_pset.c` whole | `src/ipc/ipc_pset.rs`, `ipc_pset_ffi.rs` | pending |
| `ipc/ipc_marequest.c` whole, with the `ipc_marequest_cache`, `ipc_marequest_size`, `ipc_marequest_mask` and `ipc_marequest_table` globals it owned and the `struct ipc_marequest`, `struct ipc_marequest_bucket` and `hash_info_bucket_t` mirrors its bodies read | `src/ipc/ipc_marequest.rs`, `ipc_marequest_ffi.rs`, `src/ipc/mod.rs` | pending |
| `ipc/ipc_notify.c` whole, with the six notification templates it initialized and the `mach_msg_type_t`/notification layouts its senders wrote | `src/ipc/ipc_notify.rs`, `ipc_notify_ffi.rs` | pending |
| `ipc/mach_msg.c` whole, with the `mach_msg_continue`/`mach_msg_receive_continue` continuations whose addresses `kern/thread.c` and `kern/exception.c` compare | `src/ipc/mach_msg.rs`, `mach_msg_ffi.rs` | pending |
| `ipc/mach_debug.c` whole, `host_ipc_marequest_info` included | `src/ipc/mach_debug.rs`, `mach_debug_ffi.rs` | pending |
| `device/ds_routines.c` whole, with the `struct io_req`, `struct device`, `struct mach_device`, `struct dev_ops` and `struct device_emulation_ops` mirrors it owned, and its `device_io_map`, `io_inband_cache`, `io_trap_cache`, `io_done_list` and `mach_device_emulation_ops` globals | `src/device/ds_routines.rs`, `ds_routines_ffi.rs`, `src/arch/i386/io_req.rs` | pending |
| `ipc/copy_user.c`, whose one live definition was the LP64 `copyinmsg()`; the `USER32` half is deleted as dead (§8) | `src/ipc/copy_user.rs`, `copy_user_ffi.rs` | pending |
| `i386/i386/fpu.c` whole, with the `fp_kind`, `fp_save_kind`, `fp_xsave_support`, `fp_xsave_size`, `fp_default_state`, `ifps_cache` and `mxcsr_feature_mask` globals it owned and the `I386FpSave`, `I386FpRegs`, `I386XfpSave` and save-state mirrors its bodies read | `src/arch/i386/fpu.rs`, `fpu_ffi.rs` | pending |
| `i386/i386/pcb.c` whole, with the `pcb_cache` and `kernel_stack` globals it owned and the `Pcb`, `I386SavedState`, `I386InterruptState`, `I386MachineState`, `TaskTss`, `UserLdt` and thread-status mirrors its bodies read | `src/arch/i386/pcb.rs`, `pcb_ffi.rs` | pending |
| `i386/i386at/com.c` whole, with the NCOM-sized `cominfo`/`com_tty`/`commodom`/`comcarrier`/`comfifo`/`comtimer_state`/`com_std` arrays, the `comdriver` bus record and the `BusDevice`/`BusCtlr`/`BusDriver` mirrors its body reads, and the two `com_base_addr`/`com_irq` shims §10 listed | `src/arch/i386/com.rs`, `src/arch/i386/com_ffi.rs` | pending |
| `vm/memory_object.c` whole, with the `memory_manager_default` port and its lock | `src/vm/memory_object.rs`, `memory_object_ffi.rs`, `src/vm/error.rs` | pending |
| `vm/vm_resident.c` whole, with the `vm_page_bucket_t` hash table, the fictitious-page list, the file-private counters and the `virtual_space_start`/`virtual_space_end` globals | `src/vm/vm_resident.rs`, `vm_resident_ffi.rs` | pending |
| `kern/boot_script.c` and `kern/bootstrap.c` whole, with the `struct cmd` mirror of <kern/boot_script.h>, the `struct multiboot_raw_info`/`struct multiboot_raw_module` mirrors of <mach/machine/multiboot.h>, the parser's `cmds`/`symtab` statics and the `boot_host_port`/`boot_device_port` globals they owned | `src/kern/boot_script.rs`, `boot_script_ffi.rs`, `bootstrap.rs`, `bootstrap_ffi.rs` | pending |
| `i386/i386/io_perm.c` whole, with the `IoPerm` mirror, the `taken_pci_cfg` static, the `device_emulation_ops` instance and the `no_senders()` handler it owned | `src/arch/i386/io_perm.rs`, `src/arch/i386/io_perm_ffi.rs` | pending |
| `i386/i386/smp.c` whole, with the static `smp_send_ipi()` and the `smp_data_init()`, `wait_for_ipi()`, `smp_send_ipi_init()` and `smp_send_ipi_startup_twice()` helpers it owned | `src/arch/i386/smp.rs`, `src/arch/i386/smp_ffi.rs` | pending |
| `i386/i386/trap.c` whole, with the `trap_type[]` table, the `user_page_fault_continue()` continuation and the `struct recovery` mirror its two table walks read | `src/arch/i386/trap.rs`, `src/arch/i386/trap_ffi.rs` | pending |
| `vm/vm_kern.c` whole, with the `kernel_map_store`, `kernel_map` and `kernel_pageable_map` globals it owned and the eleven definitions left in C | `src/vm/vm_kern.rs`, `vm_kern_ffi.rs` | pending |
| `vm/vm_pageout.c` whole, with the file-private `vm_pageout_requested` and `vm_pageout_continue` statics it owned | `src/vm/vm_pageout.rs`, `vm_pageout_ffi.rs` | pending |
| `kern/processor.c` whole, with the `master_cpu`, `default_pset`, `all_psets`, `all_psets_count`, `all_psets_lock`, `master_processor`, `pset_cache` and `slave_pset` globals it owned and the `processor_set_things` allocation and port conversions | `src/kern/processor.rs`, `processor_ffi.rs` | pending |
| `kern/machine.c` whole, with the `machine_info`, `machine_slot`, `action_queue` and `action_lock` globals it owned and the static `cpu_down`, `processor_request_action` and `processor_doaction` helpers | `src/kern/machine.rs`, `machine_ffi.rs` | pending |
| `kern/eventcount.c` whole, with the `all_eventcounters[MAX_EVCS]` table and the file-private `struct evc` | `src/kern/eventcount.rs`, `eventcount_ffi.rs` | pending |
| `i386/i386at/model_dep.c` whole, with the `boot_info`, `kernel_cmdline` and `rebootflag` globals and the `ElfShdr` and `GdtDescrTmp` mirrors its boot path reads | `src/arch/i386/model_dep.rs`, `model_dep_ffi.rs` | pending |
| `i386/i386/mp_desc.c` whole, with the NCPUS-sized `int_stack_base`, `int_stack_top`, `solid_intstack`, `mp_desc_table`, `mp_ktss` and `mp_gdt` it owned, the `apboot_addr` global, and the `RealGate` and `MpDescTable` mirrors `idt.c`, `int_init.c`, `gdt.c`, `ldt.c` and `ktss.c` still read | `src/arch/i386/mp_desc.rs`, `mp_desc_ffi.rs` | pending |
| `i386/i386/debug_i386.c` whole, with the `debug_trace_buf`/`debug_trace_pos`, `syscall_trace`/`syscall_trace_task` globals and the `DebugTraceEntry` and `MachTrap` mirrors, and `dump_ss` re-homed out of `glue` for `trap.rs` | `src/arch/i386/debug_i386.rs`, `debug_i386_ffi.rs` | pending |
| `vm/memory_object_proxy.c` whole, with the `memory_object_proxy_cache` slab cache and the `struct memory_object_proxy` record it owned | `src/vm/memory_object_proxy.rs`, `memory_object_proxy_ffi.rs` | pending |
| `vm/vm_debug.c` whole, with the `VmRegionInfo`, `VmObjectInfo`, `VmPageInfo` and `VmPagePhysInfo` mirrors of `mach_debug/vm_info.h` its bodies fill | `src/vm/vm_debug.rs`, `vm_debug_ffi.rs` | pending |
| `vm/vm_user.c` whole, with the `vm_stat` block it owned and the `VmCacheStatistics` mirror of `mach/vm_cache_statistics.h` | `src/vm/vm_user.rs`, `vm_user_ffi.rs` | pending |
| `kern/ast.c` (`ast_taken`, `ast_check`) whole, with the `need_ast[NCPUS]` array the macros and `locore.S` read | `src/kern/ast.rs`, `ast_ffi.rs` | pending |
| `kern/debug.c` (`__stack_chk_guard`) | `src/kern/debug.rs` | pending |
| `kern/ipc_kobject.c` whole, with the `IKOT_*` type values, the ten generated MIG server tables it dispatches through and the `ipc_port` bits/kobject update | `src/kern/ipc_kobject.rs`, `ipc_kobject_ffi.rs` | pending |
| `kern/startup.c` whole, with the `reboot_on_panic` global and the empty `start_timer`/`timer_switch` macros | `src/kern/startup.rs`, `startup_ffi.rs` | pending |
| `kern/syscall_emulation.c` whole, with the `struct eml_dispatch` mirror its bodies and the `i386asm.sym` offsets read | `src/kern/syscall_emulation.rs`, `syscall_emulation_ffi.rs` | pending |
| `kern/syscall_subr.c` whole, with the `swtch_continue`, `swtch_pri_continue` and `thread_switch_continue` statics and the `thread_switch` hint path | `src/kern/syscall_subr.rs`, `syscall_subr_ffi.rs` | pending |
| `i386/i386/gdt.c`, `i386/i386/idt.c`, `i386/i386/ktss.c` and `i386/i386/ldt.c` whole, with the `gdt`, `idt`, `ktss` and `ldt` tables and the `gdt_fill`/`idt_fill`/`ktss_fill`/`ldt_fill` statics they owned, and the `seg.h` descriptor fillers, loaders and selector constants in a new `src/arch/i386/seg.rs`; adds the `RealDescriptor64` and `PseudoDescriptor` mirrors and the `IdtInitEntry` mirror with size, align and offset asserts from both built kernels | `src/arch/i386/gdt.rs`, `gdt_ffi.rs`, `idt.rs`, `idt_ffi.rs`, `ktss.rs`, `ktss_ffi.rs`, `ldt.rs`, `ldt_ffi.rs`, `seg.rs` | pending |
| `i386/i386at/int_init.c` whole, with the static `int_fill` and the `int_entry_table` walk | `src/arch/i386/int_init.rs`, `int_init_ffi.rs` | pending |
| `i386/i386/user_ldt.c` whole, with the `struct descriptor` mirror and the `user_ldt_free` entry `pcb.rs` calls | `src/arch/i386/user_ldt.rs`, `user_ldt_ffi.rs` | pending |
| `i386/i386/db_interface.c` whole, with the `zero_dr` static and the `ddb_regs` global | `src/arch/i386/db_interface.rs`, `db_interface_ffi.rs` | pending |

`vm/vm_fault.c` was ported whole and rolled back in the same pass: the pinned
toolchain turns the copy-object loop's `first_object->copy` null test into an
`llvm.assume` (the optimized IR carries `!nonnull` on the load), so a null copy
dereferences `copy_object->Lock` at offset 0x10 and panics.  The retry inside
that loop is the trigger; `vm_fault.c` stays C until the toolchain or the loop
shape changes.

Deleted dead code: `device/blkio.c`, the `#if 0` profiling facility
(`profil.h`, `profilparam.h`, `mpqueue`), and `i386/i386at/kd_glue.c`
(`018c9cd8`).  `kern/rdxtree.c`'s `rdxtree_check_alignment`, never called
by either build, went with that file's port.

## 10. The glue debt

Every piece of C in this tree that exists only so Rust can reach
something.  All of it predates the no-glue law, none of it is precedent,
and nothing may be added.  Each row says what deletes it.

There is nothing left to list.  The last piece,
`i386/i386at/com.c`'s `com_base_addr`/`com_irq` pair, came out with the
`com.c` port.  Before it, `i386/i386at/kd_glue.c`,
`kern/processor_glue.c`, the `thread_glue_pset_sched_load` shim in
`kern/sched_prim.c`, `vm/vm_map_glue.c` with its
`vm_map_glue_object_*` shims, the `vm_submap_object` placeholder, the
`vm_map_glue_task_map`/`vm_map_glue_task_space` pair, the page field
shims and three slab caches, `vm/vm_external_glue.c` with its three
slab caches, `i386/i386/irq.c`'s six accessors, and
`i386/i386at/com.c`'s `com_base_addr`/`com_irq` pair were deleted.
Nothing joined the list since.
The `i386/intel/pmap.c` port declared the C routines it still calls
(`splvm`, `kmem_alloc_wired`, `cpu_features`, `_start`, `etext`) in
`rust/src/glue/`, which writes no C and is not debt.  The
`i386/i386at/biosmem.c` port moved `biosmem_directmap_end` out of that
list and into `src/arch/i386/biosmem.rs`.  The `kern/thread.c` port
declared the C routines it still calls (`ipc_thread_init`,
`mach_port_deallocate`, `mach_port_destroy` and the three `mach_msg_*`
entries) in the same block, which is not debt either; the
`mach_msg_*` declarations came out when `ipc/mach_msg.c` moved, and the
`pcb_init`/`pcb_terminate` and `fp_load`/`ifps_cache`/`fpintr`
declarations when `i386/i386/pcb.c` and `i386/i386/fpu.c` moved.  The
`ipc/ipc_mqueue.c` port declared `ipc_kobject_server`, and the
`ipc/ipc_marequest.c` port declared `ipc_notify_msg_accepted`, in that
same block; declaring C symbols that already exist writes no C, the
notify declarations came out with `ipc/ipc_notify.c`, and the
`ipc_kobject_server` declaration went with the `kern/ipc_kobject.c`
port.

`--enable-user32` is out of scope for the Rust half: the build targets
the i686 and x86_64 configurations the ABI pack gates.  The removed
`memory_object_create_proxy` shim was the one place that assumed
otherwise.  A user32 build would need `--cfg user32` plumbed through
`rust/configfrag.ac` before that native-width declaration is correct
again.

The `gdt.c`/`idt.c`/`ktss.c`/`ldt.c`/`int_init.c` port declared
`idt_inittab`, `int_entry_table`, `syscall` and `syscall64` in the same
block; those are the generated tables and asm entries the port reads,
which writes no C and is not debt.
