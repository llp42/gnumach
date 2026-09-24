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
| **L1 types** | structs read field-by-field, sometimes by asm | `Thread`, `Processor`, `ProcessorSet`, `RunQueue`, `Timer`, `Timeout`, `QueueEntry`, `SimpleLock`, `TimeValue`/`TimeValue64`, `VmMap`/`VmMapEntry`/`VmMapHeader`/`VmMapLinks`, `VmPage`, `VmObject`, `Task`/`MachineTask`, `KmemCache`, `MachineSlot`, `struct ipc_port` (with its `ipc_target` and `ipc_mqueue`), `struct ipc_space`, `struct ipc_kmsg` and `struct ipc_entry` are `#[repr(C)]` mirrors with size, alignment and offset asserts.  `struct pcb`, the APIC structs and the driver structs have no field mirror. |
| **L2 locks/IRQ/percpu** | `simple_lock`, `spl*`, `percpu_get`, `current_thread()` | done: `kern/lock.c` and `i386/i386/lock.h` are gone, `SimpleLock` is `src/kern/lock.rs`, `spl*` are real asm functions in `glue`, and `current_thread()`, `cpu_number()` and `percpu_get` live in `src/arch/i386/percpu.rs`.  An RAII `IrqGuard` is a Rust-side type to write when wanted. |
| **L3 memory** | `kalloc`/`kfree`, `kmem_cache_*` | done: `kern/slab.c` is gone, `src/kern/slab.rs` owns the allocator and `src/kern/slab_ffi.rs` exports its C symbols.  A `GlobalAlloc` over `kalloc` remains a design conversation. |
| **L4 runnable** | `thread_block`, `assert_wait`, `set_timeout`, continuations | the wait/wake primitives are Rust; `thread_block`, `assert_wait` and `set_timeout` are real C symbols in `glue`; `switch_context`, `call_continuation` and `stack_handoff` stay C. |
| **L5 IPC/VM** | ports, spaces, kmsgs, maps, objects, pages | `vm_map` and `vm_object` are Rust-native, and `struct task` is mirrored; the rest have no field mirrors, and `vm/vm_map_glue.c` exists for the page and task fields the map's C edges still read. |
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
| `ast.c` | 215 | 3 | 0 | `ast_taken`/`ast_check` need `net_ast()` and the run-queue walk |
| `boot_script.c` | 696 | 2 | 0 | `struct cmd` fields; static helpers |
| `bootstrap.c` | 751 | 5 | 0 | bootstrap data; static helpers |
| `debug.c` | 121 | 3 | 0 | C variadics (`log`) |
| `eventcount.c` | 305 | 4 | 0 | `struct eventcounter` has no mirror |
| `exception.c` | 934 | 5 | 0 | `struct exception` and `ipc_port` fields |
| `gsync.c` | 537 | 4 | 0 | `struct gsync_node` internals |
| `host.c` | 290 | 3 | 0 | `host_info`'s `machine_info` and load-average globals |
| `ipc_host.c` | 390 | 3 | 0 | `ipc_port`/`ipc_space` fields |
| `ipc_kobject.c` | 362 | 4 | 0 | `ipc_port` fields |
| `ipc_mig.c` | 856 | 5 | 0 | `port_name_to_*` are static; wire-type structs |
| `ipc_sched.c` | 163 | 4 | 0 | — |
| `mach_clock.c` | 648 | 4 | 0 | `__sync_synchronize` wrappers, static `time_value64_add_hpc`, `clock_boottime_update` |
| `mach_factor.c` | 150 | 2 | 0 | `mach_factor[]`/`load_average[]` are NCPUS-sized |
| `machine.c` | 630 | 4 | 0 | `machine_info` and NCPUS loops |
| `printf.c` | 592 | 5 | 0 | C-variadic definitions; blocked (see §8) |
| `priority.c` | 196 | 4 | 0 | pset tail and `struct slock_irq` |
| `processor.c` | 465 | 4 | 0 | `processor_set_things`'s allocation and port conversions |
| `sched_prim.c` | 1238 | 5 | 0 | static `thread_select`/`do_runq_scan`; continuations |
| `startup.c` | 290 | 5 | 0 | `machine_info`, NCPUS loops, boot |
| `syscall_emulation.c` | 446 | 4 | 0 | `struct eml_dispatch` and task fields |
| `syscall_subr.c` | 251 | 4 | 0 | static continuations (`swtch_continue`, ...) |
| `syscall_sw.c` | 220 | 3 | 0 | trap table ABI; static stubs |
| `timer.c` | 93 | 3 | 0 | `db_thread_read_times` and its static helpers |

## 5. Outside `kern/`

### `ipc/` (10 files, 9,969 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `copy_user.c` | 540 | 0 | `mach_msg_header` fields; `copyoutmsg` absent from both builds |
| `ipc_kmsg.c` | 2600 | 0 | `struct ipc_kmsg` fields |
| `ipc_marequest.c` | 415 | 0 | `struct ipc_marequest` fields |
| `ipc_mqueue.c` | 659 | 0 | `struct ipc_mqueue` fields |
| `ipc_notify.c` | 448 | 0 | `ipc_kmsg` and message fields |
| `ipc_pset.c` | 309 | 0 | `ipc_pset`/`ipc_mqueue` fields |
| `ipc_right.c` | 1844 | 0 | `ipc_entry`/`ipc_port` fields |
| `mach_debug.c` | 286 | 0 | `hash_info_bucket_t` has no mirror |
| `mach_msg.c` | 1648 | 0 | `ipc_kmsg` fields |
| `mach_port.c` | 1220 | 0 | `ipc_space`/`ipc_port` fields |

### `vm/` (10 files, 6,776 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `memory_object.c` | 1079 | 0 | `ipc_port` fields |
| `memory_object_proxy.c` | 227 | 0 | cache statics |
| `vm_debug.c` | 541 | 0 | `hash_info_bucket_t` mirror |
| `vm_external_glue.c` | 21 | 0 | three `kmem_cache` storage symbols; the `KmemCache` mirror exists now, so a follow-up moves the definitions to Rust statics and deletes the file |
| `vm_fault.c` | 2024 | 0 | `vm_page`/task fields |
| `vm_kern.c` | 812 | 0 | — |
| `vm_map_glue.c` | 197 | 0 | the page field shims; the task shims went with `kern/task.c`, the object shims with `vm_object.c` |
| `vm_pageout.c` | 505 | 0 | `vm_page` fields |
| `vm_resident.c` | 948 | 0 | the `vm_page_bucket_t` table, the fictitious-page statics and `vm_page_order` |
| `vm_user.c` | 602 | 0 | `vm_page` fields for the rest |

### `device/` (11 files, 7,057 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `chario.c` | 998 | 0 | `struct tty` fields |
| `cons.c` | 176 | 0 | `cn_tab` static table |
| `device_init.c` | 49 | 0 | — |
| `dev_lookup.c` | 365 | 0 | `mach_device` fields |
| `dev_name.c` | 166 | 0 | `dev_ops`/`dev_indirect` fields |
| `dev_pager.c` | 565 | 0 | hash and device fields |
| `ds_routines.c` | 1854 | 0 | `struct io_req` fields |
| `intr.c` | 375 | 0 | `struct irqdev`/`user_intr_t` fields |
| `kmsg.c` | 237 | 0 | — (the rest is message plumbing) |
| `net_io.c` | 2168 | 0 | `ifnet`/`net_hash_entry` fields |
| `subrs.c` | 53 | 0 | `ifnet` fields |

### `i386/` (30 files, 8,413 LOC)

| File | LOC | Free | Holds the rest |
|---|---:|---:|---|
| `i386/apic.c` | 354 | 0 | `ApicInfo`/`IoApicData`/`ApicLocalUnit` fields |
| `i386/db_interface.c` | 103 | 0 | `struct pcb` fields |
| `i386/debug_i386.c` | 178 | 0 | `i386_saved_state` fields |
| `i386/fpu.c` | 830 | 0 | `struct pcb` and FPU save-area fields |
| `i386/gdt.c` | 141 | 0 | static `gdt_fill`, `reload_segs` |
| `i386/hardclock.c` | 69 | 0 | `machine_slot` and interrupt plumbing |
| `i386/idt.c` | 80 | 0 | static `idt_fill` |
| `i386/io_perm.c` | 325 | 0 | `struct io_perm`; static bitmap helpers |
| `i386/irq.c` | 136 | 0 | `ivect`/`iunit` are NINTR-sized |
| `i386/ktss.c` | 86 | 0 | static `ktss_fill` |
| `i386/ldt.c` | 100 | 0 | static `ldt_fill` |
| `i386/machine_task.c` | 70 | 0 | `task.machine` fields |
| `i386/mp_desc.c` | 296 | 0 | `int_stack_base`/`int_stack_top` are NCPUS-sized |
| `i386/pcb.c` | 882 | 0 | `struct pcb`/`i386_saved_state` fields |
| `i386/percpu.c` | 31 | 0 | `struct percpu.self` field |
| `i386/phys.c` | 164 | 0 | mapped-window internals for `pmap_copy_page` etc. |
| `i386/pic.c` | 270 | 0 | not compiled in the APIC configuration |
| `i386/smp.c` | 214 | 0 | static `smp_send_ipi` |
| `i386/trap.c` | 532 | 0 | trap frames; `trap_type[]` static |
| `i386/user_ldt.c` | 422 | 0 | `struct pcb` and descriptor structs |
| `i386at/acpi_parse_apic.c` | 635 | 0 | static ACPI helpers |
| `i386at/autoconf.c` | 127 | 0 | `bus_device`/`bus_ctlr` fields |
| `i386at/com.c` | 909 | 0 | `com_*` arrays are NCOM-sized |
| `i386at/conf.c` | 144 | 0 | static tables |
| `i386at/cons_conf.c` | 48 | 0 | static tables |
| `i386at/int_init.c` | 78 | 0 | static `int_fill` |
| `i386at/ioapic.c` | 487 | 0 | `curr_ipl` is NCPUS-sized; `ioapic_*` statics |
| `i386at/model_dep.c` | 468 | 0 | init/boot state |
| `i386at/pic_isa.c` | 56 | 0 | not compiled in the APIC configuration |
| `intel/read_fault.c` | 178 | 0 | dead: body is `#if`-ed out on every supported CPU |

`chips/busses.c` (232 LOC) is wholly blocked on `bus_device`/`bus_ctlr`
fields.  There are no C files under `x86_64/`.

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
went with the `pmap.c` port and the other 14 are still C:
`interrupt_stack_alloc`, `picdisable`, the four `i386/i386/irq.c`
accessors and the eight `i386/i386at/com.c` entries.

**Mirror gaps.**
`host_ipc_marequest_info` and `host_virtual_physical_table_info` need a
`hash_info_bucket_t` mirror.  The `struct pmap` story now exists: the
whole of `i386/intel/pmap.c`, its `static` `phys_attribute_*` helpers
included, moved to `src/arch/i386/pmap.rs` (§9).
The `vm/vm_object.c` port completed the `struct vm_object` field mirror
(`src/vm/types.rs`) and moved the module to `src/vm/vm_object.rs`, so the
object's lock, flags and page list are Rust; the C files that still call
it keep the `vm/vm_object.h` prototypes.  A follow-up added the
`struct task` mirror (`src/kern/task.rs`, with `struct machine_task` in
`src/arch/i386/machine_task.rs`) with the offsets read from both built
kernels, so the eviction path bumps `current_task()->reactivations` as
the C did.

**Gated decisions.**
`i386/i386/pcb.c:857 user_stack_low` needs the `--enable-user32`
`--cfg`, because `VM_MAX_USER_ADDRESS` takes a third value there.
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
`i386/i386/smp.c`'s `smp_send_ipi` is the same case.

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
  `task_ras_control`, the `apic.c` accessors); (b) the nine deletable
  `vm_map_glue.c` shims, which are Rust-side edits and C deletions, not
  ports; (c) `vm_user.c` wrappers;
  (d) the ipc wrappers (`ipc_init`, `ipc_object_destroy`, `ipc_port_*`,
  `ipc_thread_*`, `ipc_pset_*`, `ipc_host`); (e) `vm_kern.c` and
  `vm_resident.c`; (f) the `kern/thread.c`/`sched_prim.c` scheduler
  batch; (g) the `model_dep.c` clock and console leaves.

* **Phase B — unlock work.**  The constants are in `rust/src/config.rs`;
  the 20 functions they freed are next, then mirror `hash_info_bucket_t`
  and `struct vm_page`; `struct pmap` followed with the whole-file
  `pmap.c` port.

* **Phase C — the coupled files.**  `eventcount`, `priority`, `gsync`,
  `ipc_tt`, `ipc_host`, `host`, `processor`, `machine`, `mach_clock` once
  their struct stories exist; then the anchors (`sched_prim`,
  `ipc_mig`, `exception`, `startup`, `bootstrap`, `trap`, `pcb`,
  `ipc_kmsg`, `mach_msg`).

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
* `#if 0` blocks in `kern/{boot_script,bootstrap,exception,ipc_kobject}.c`,
  `device/intr.c`, `i386/i386/{fpu,smp,pcb,trap}.c`,
  `i386/i386at/{kd,com}.c`.  Delete before porting the surrounding code.
  `kern/ipc_tt.c`'s four `#if 0` `retrieve_*` bodies went with the file.
* Dead `#else /* MACH_HOST */` halves of `kern/machine.c:309`;
  `MACH_HOST` is 1 in both configured builds.  The `kern/task.c` and
  `kern/thread.c` halves went with their files.
* Macro-shadowed definitions: `i386/intel/pmap.c`'s `pmap_copy` and
  `pmap_kernel` were unreachable behind `i386/intel/pmap.h`'s macros and
  went with the file's port.
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
| `device/net_io.c` (`bpf_hash`) | `src/device/net_io.rs` | `c2d49b23` |
| `ipc/ipc_object.c` (`ipc_object_copyin_type`) | `src/ipc/ipc_object.rs` | `8c8c697f` |
| `ipc/ipc_port.c` (`ipc_port_timestamp`) | `src/ipc/ipc_port.rs` | `54dfe7cd` |
| `ipc/ipc_port.c` whole, with the `ipc_port_multiple_lock_data` static and the `ipc_port_request`, `ipc_entry`, `ipc_space` and `ipc_kmsg` field mirrors it reads | `src/ipc/ipc_port.rs`, `src/ipc/ipc_port_ffi.rs`, `src/ipc/mod.rs` | pending |
| `ipc/mach_port.c` (four right routines) | `src/ipc/mach_port.rs` | `6c47e6b4`, `1690113f` |
| `kern/thread.c` (`thread_init`), `kern/sched.h` (`thread_timer_delta`) | `src/kern/thread.rs`, `src/kern/timer.rs` | `16581252` |
| `kern/timer.c` (read/normalize/delta) | `src/kern/timer.rs` | `6858b08c` |
| `kern/processor.c` (`processor_init`, `pset_init`, info and pset entries) | `src/kern/processor.rs` | `498525e5` |
| `kern/machine.c` (`host_reboot`) | `src/kern/machine.rs` | `32ca71ed` |
| `i386/i386at/rtc.c` | `src/arch/i386/rtc.rs` | `5afeaa94` |
| `i386/i386at/pit.c` | `src/arch/i386/pit.rs` | `896ae703` |
| `i386/i386/pcb.c` (`stack_detach`, `load_context`, `pcb_collect`), `i386/i386/phys.c` (`kvtophys`) | `src/arch/i386/pcb.rs`, `phys.rs` | pending |
| `i386/i386/apic.c` (`apic_lapic_init`, `apic_get_cpu_kernel_id`, `apic_get_lapic`, `apic_get_current_cpu`, `hpet_init`, `hpet_udelay`, `hpet_mdelay`, `hpclock_read_counter`, `hpclock_get_counter_period_nsec`) | `src/arch/i386/apic.rs` | pending |
| `i386/i386at/acpi_parse_apic.c` (`acpi_print_info`), `i386/i386at/ioapic.c` (`intnull`) | `src/arch/i386/acpi_parse_apic.rs`, `ioapic.rs` | pending |
| `device/chario.c` (`tty_queue_completion`), `device/device_init.c` (`device_service_create`), `device/ds_routines.c` (`ds_device_open_new`), `device/intr.c` (`irqgetstat`), `device/kmsg.c` (`kmsggetstat`) | `src/device/chario.rs`, `device_init.rs`, `ds_routines.rs`, `intr.rs`, `kmsg.rs` | pending |
| `kern/host.c` (`host_processor_set_priv`, `processor_set_processors`) | `src/kern/host.rs` | pending |
| `kern/ipc_host.c` (`ipc_processor_init`, `ipc_pset_init`, `ipc_pset_enable`, `ipc_pset_disable`, `ipc_pset_terminate`, `processor_set_default`) | `src/kern/ipc_host.rs` | pending |
| `kern/ipc_mig.c` (`mach_msg_abort_rpc`, `mig_get_reply_port`, `mig_deallocate`, `thread_set_self_state`) | `src/kern/ipc_mig.rs` | pending |
| `kern/ipc_sched.c` (`thread_go`, `thread_will_wait`, `thread_will_wait_with_timeout`) | `src/kern/ipc_sched.rs` | pending |
| `kern/ipc_tt.c` whole, with the `struct ipc_port`/`ipc_target`/`ipc_mqueue` field mirror its `ip_srights` bump reads | `src/kern/ipc_tt.rs`, `src/kern/ipc_tt_ffi.rs`, `src/ipc/mod.rs` | pending |
| `kern/mach_clock.c` (`read_time_stamp`, `host_set_time`, `host_adjust_time`, `host_adjust_time64`) | `src/kern/mach_clock.rs` | pending |
| `kern/machine.c` (`action_thread`) | `src/kern/machine.rs` | pending |
| `kern/task.c` (`task_create`, `task_ras_control`, `register_new_task_notification`) | `src/kern/task.rs` | pending |
| `kern/bootstrap.c` (`boot_script_free_task`) | `src/kern/bootstrap.rs` | pending |
| `kern/exception.c` (`exception_no_server`) | `src/kern/exception.rs` | pending |
| `kern/printf.c` (`printnum`, `safe_gets`) | `src/kern/printf.rs` | pending |
| `kern/rdxtree.c` with the `struct rdxtree`/`rdxtree_iter` mirrors | `src/kern/rdxtree.rs`, `rdxtree_ffi.rs` | pending |
| `kern/timer.c` (`thread_read_times`) | `src/kern/timer.rs` | pending |
| `kern/sched_prim.c` (`thread_set_timeout`, `thread_bind`, `thread_continue`, `compute_priority`, `compute_my_priority`, `recompute_priorities`, `set_pri`, `choose_pset_thread`) and `kern/syscall_subr.c` (`thread_depress_priority`, `thread_depress_timeout`, `thread_depress_abort`) | `src/kern/sched_prim.rs`, `src/kern/syscall_subr.rs` | pending |
| `kern/thread.c` (`stack_alloc_try`, `stack_alloc`, `stack_free`, `stack_collect`, `stack_privilege`) | `src/kern/thread.rs` | pending |
| `kern/thread.c` (`thread_reference`, `thread_force_terminate`, `thread_hold`, `thread_release`, `thread_resume`, `thread_abort`, `thread_start`, `thread_unfreeze`, `thread_get_assignment`) | `src/kern/thread.rs` | pending |
| `kern/thread.c` (`thread_get_state`, `thread_set_state`, `thread_priority`, `thread_set_own_priority`, `thread_max_priority`, `thread_policy`, `thread_wire`, `stack_init`, `thread_stats`, `thread_set_name`, `thread_get_name`) | `src/kern/thread.rs` | pending |
| `i386/i386/fpu.c` (`fpnoextflt`) | `src/arch/i386/fpu.rs` | pending |
| `i386/i386/mp_desc.c` (`interrupt_processor`) | `src/arch/i386/mp_desc.rs` | pending |
| `vm/vm_kern.c` (`projected_buffer_collect`, `projected_buffer_in_range`, `kmem_alloc_wired_flags`, `kmem_alloc_wired`, `kmem_map_aligned_table`, `kmem_alloc_pageable`, `kmem_free`, `kmem_submap`, `kmem_init`, `kmem_io_map_deallocate`) | `src/vm/vm_kern.rs`, `src/vm/vm_kern_ffi.rs` | pending |
| `vm/vm_user.c` (`vm_allocate`, `vm_deallocate`, `vm_inherit`, `vm_protect`, `vm_machine_attribute`, `vm_read`, `vm_write`, `vm_copy`, `vm_object_sync`, `vm_msync`, `vm_get_size_limit`) | `src/vm/vm_user.rs`, `src/vm/vm_user_ffi.rs` | pending |
| `vm/vm_fault.c` (`vm_fault_wire`), `vm/vm_page.c` (`vm_page_seg_name`), `vm/vm_resident.c` (`pmap_steal_memory`, `vm_page_rename`, `vm_page_alloc_flags`, `vm_page_alloc`) | `src/vm/vm_fault.rs`, `vm_fault_ffi.rs`, `vm_page.rs`, `vm_page_ffi.rs`, `vm_resident.rs`, `vm_resident_ffi.rs` | pending |
| `kern/processor_glue.c` (4 shims), `kern/sched_prim.c` (`thread_glue_pset_sched_load`) | `src/config.rs` (`NCPUS`, `NCOM`, `NINTR`), `src/kern/processor.rs`, `src/kern/thread.rs` | pending |
| `kern/ast.c` (`ast_init`), `kern/timer.c` (`init_timers`), `kern/host.c` (`host_processors`), `kern/processor.c` (`pset_sys_init`), `device/chario.c` (`chario_init`) | `src/kern/ast.rs`, `src/kern/timer.rs`, `src/kern/host.rs`, `src/kern/processor.rs`, `src/device/chario.rs` | pending |
| `vm/vm_page.c` (`vm_page_set_type`, `vm_page_wire`), `vm/vm_resident.c` (`vm_page_init`, `vm_page_module_init`, `vm_page_grab`, `vm_page_grab_phys_addr`, `vm_page_release`, `vm_page_zero_fill`, `vm_page_copy`) with the `struct vm_page` mirror | `src/vm/vm_page.rs`, `vm_page_ffi.rs`, `vm_resident.rs`, `vm_resident_ffi.rs` | pending |
| `vm/vm_page.c` whole, with the `struct vm_page_seg`, `struct vm_page_boot_seg` and file-private statics it owned, and the `struct vm_object` field mirror its evictor reads | `src/vm/vm_page.rs`, `src/vm/vm_page_ffi.rs`, `src/vm/types.rs` | pending |
| `kern/slab.c` with the `struct kmem_cache` mirror | `src/kern/slab.rs`, `src/kern/slab_ffi.rs` | pending |
| `vm/vm_object.c` whole, with the file-private statics it owned and the `vm_submap_object` placeholder | `src/vm/vm_object.rs`, `src/vm/vm_object_ffi.rs` | pending |
| `kern/task.c` whole, with the file-private statics it owned and the `struct task` mirror | `src/kern/task.rs`, `src/kern/task_ffi.rs` | pending |
| `i386/intel/pmap.c` whole, with the file-private statics it owned and the `struct pmap`, `struct pv_entry`, `pmap_update_list` and `pmap_mapwindow_t` mirrors | `src/arch/i386/pmap.rs` | pending |
| `i386/i386at/biosmem.c` whole, with the file-private statics it owned | `src/arch/i386/biosmem.rs` | pending |
| `kern/thread.c` whole, with the file-private `walking_zombie`, `reaper_thread_continue`, `thread_collect_scan`, `stack_usage` and `stack_statistics`, and the globals it owned | `src/kern/thread.rs`, `src/kern/thread_ffi.rs` | pending |
| `ipc/ipc_init.c`, `ipc/ipc_target.c`, `ipc/ipc_space.c`, `ipc/ipc_entry.c` and `ipc/ipc_object.c` whole, with the `ipc_space_cache`, `ipc_entry_cache`, `ipc_object_caches`, `ipc_space_kernel`, `ipc_space_reply`, `ipc_kernel_map` and `ipc_kernel_map_size` globals | `src/ipc/ipc_init.rs`, `ipc_target.rs`, `ipc_space.rs`, `ipc_space_ffi.rs`, `ipc_entry.rs`, `ipc_entry_ffi.rs`, `ipc_object.rs`, `ipc_object_ffi.rs` | pending |

Deleted dead code: `device/blkio.c`, the `#if 0` profiling facility
(`profil.h`, `profilparam.h`, `mpqueue`), and `i386/i386at/kd_glue.c`
(`018c9cd8`).  `kern/rdxtree.c`'s `rdxtree_check_alignment`, never called
by either build, went with that file's port.

## 10. The glue debt

Every piece of C in this tree that exists only so Rust can reach
something.  All of it predates the no-glue law, none of it is precedent,
and nothing may be added.  Each row says what deletes it.

| Glue | What it provides | Deleted by |
|---|---|---|
| `vm/vm_map_glue.c` — page field shims | `vm_page` bit probes and the `PMAP_ENTER`/`PAGE_WAKEUP_DONE` macros | the page-list copyin macros moving to Rust |
| `vm/vm_external_glue.c` | three `kmem_cache` storage symbols | Follow-up to the `kern/slab.c` port: `struct kmem_cache` is `src/kern/slab.rs`'s `KmemCache` now, so the definitions move to Rust statics in a pass of their own |
| `i386/i386/irq.c` — `irq_mask`, `irq_unmask`, `irq_{set,get}_{handler,unit}` | `mask_irq`/`unmask_irq` static inlines and the NINTR-sized `ivect`/`iunit` | Phase B: `NINTR`, plus a Rust `mask_irq` |
| `i386/i386at/com.c` — `com_base_addr`, `com_irq` | `cominfo` is NCOM-sized | Phase B (`NCOM`) or porting `com.c` |

`i386/i386at/kd_glue.c`, `kern/processor_glue.c`, the
`thread_glue_pset_sched_load` shim in `kern/sched_prim.c`,
`vm/vm_map_glue.c`'s `vm_map_glue_object_*` shims with the
`vm_submap_object` placeholder, and its `vm_map_glue_task_map`/
`vm_map_glue_task_space` pair are deleted; nothing joined the list since.
The `i386/intel/pmap.c` port declared the C routines it still calls
(`splvm`, `kmem_alloc_wired`, `cpu_features`, `_start`, `etext`) in
`rust/src/glue/`, which writes no C and is not debt.  The
`i386/i386at/biosmem.c` port moved `biosmem_directmap_end` out of that
list and into `src/arch/i386/biosmem.rs`.  The `kern/thread.c` port
declared the C routines it still calls (`pcb_init`, `pcb_terminate`,
`ipc_thread_init`, `mach_port_deallocate`, `mach_port_destroy`,
`mach_msg_continue`, `mach_msg_receive_continue` and
`mach_msg_interrupt`) in the same block, which is not debt either.

`--enable-user32` is out of scope for the Rust half: the build targets
the i686 and x86_64 configurations the ABI pack gates.  The removed
`memory_object_create_proxy` shim was the one place that assumed
otherwise.  A user32 build would need `--cfg user32` plumbed through
`rust/configfrag.ac` before that native-width declaration is correct
again.
