# MIGRATE.md — C-to-Rust migration map

This file maps the C half of GNU Mach against the Rust port.  It covers
**all 33 translation units in `kern/`** file by file, then every other C
directory as a summary, and it states for each what has to exist in Rust
first, why the C dependency is there, and where the remaining C/Rust
boundary would sit.  It replaces the earlier leaf-only list: the method
there (`nm -u` survivors) still finds the easiest first steps, but it
cannot say anything about the 90% of the kernel that is coupled.

The port contract itself is in `AGENTS.md`; this file is the map,
not the rules.  Read the map top to bottom if you are choosing work,
or jump to `kern/<file>.c` for a specific file.

The user test tree (`tests/`) was removed in 2026-09: the correctness
gate is now the frozen binary pack in `abi-test/`, run against the
freshly built kernel by `mise run test`.  The per-file "Tests." notes
below name the old suite's test sources as the coverage that pinned the
C behaviour; that coverage is frozen in the pack, and no test source is
edited in this tree any more.

The mechanical data (the `Undef` counts in particular) is an
audit-time snapshot: it was produced from a clean tree with the first
six port commits applied, using `nm -g --defined-only` and `nm -u` over
`build-64/*.o`, source-level inspection of every macro, `static inline`
helper, function-pointer call and assembly escape, and a survey of MIG
inputs (`*.srv`, `*.cli`, `*.defs`) against their generated `*.server.c`
and `*.user.c`.

## 0. What a port is (the contract)

1. A Rust definition replaces a C definition of the **same symbol and
   signature**, and the C definition is deleted in the same commit.  Two
   definitions is a link error, not a fallback.
2. `libmach-rs.a` is linked between two passes over `libkernel.a`
   (`Makefile.am:149-152`), so Rust may call C and C may call Rust.
   `gnumach-undef-bad` rejects any symbol not covered by the allowlist.
3. No Cargo, no build script, no network.  `core` only, **no `alloc`**.
   Memory comes from the caller or from kernel allocators through shims.
4. clippy `-D warnings`, `rustfmt --check` at 79 columns and
   `--enable-queue-debug` are build gates (`rust/Makefrag.am:96-107`).
5. `mise run test` builds both kernels and runs the frozen ABI pack in
   `abi-test/` against them; it is the only correctness gate.  The pack
   is self-contained, so there is no in-tree user-test source to keep
   in sync.
6. `rust/src/` mirrors the C tree: `src/utils/`, `src/kern/`,
   `src/arch/<arch>/`, with C-call shims in `src/glue/` and — for
   macros, which cannot cross FFI — small C shim functions.

## 1. Friction scale

| # | Meaning |
|---|---|
| 1 | Leaf: no external calls or data; port is one module and a test copy if needed. |
| 2 | Near-leaf: one or two leaf dependencies (allocation, strings, one table). |
| 3 | Coupled to one subsystem (locks, a struct shared with C, or MIG signatures). |
| 4 | Deeply coupled: scheduler/IRQ context, several subsystems, or an asm-adjacent ABI. |
| 5 | Architectural anchor: context switch, trap path, MIG wire format, pmap, or boot. Port last. |

## 2. The layers — why files depend on each other

The C kernel's coupling is not accidental.  There are six layers of
shared machinery, and a file cannot move until the layer under it has a
Rust story.  This is the "why" behind every blocker in §4.

| Layer | C machinery | Rust must first provide | Blocks |
|---|---|---|---|
| **L0 pure** | string ops (already Rust), byte order (Rust), atoi (Rust), parser tables | nothing | — |
| **L1 types** | `struct thread`, `task`, `processor`, `processor_set`, `ipc_port`, `vm_map` read/written field-by-field, sometimes by asm (`i386asm.sym`) | `#[repr(C)]` mirror + offset/size `const` asserts, or C accessor shims; decision on who owns the layout | everything in `kern/` |
| **L2 locks/IRQ/percpu** | `simple_lock`/`_simple_lock` (thin macros over the Rust entry points), `spl*` (`spl.S`, per-CPU `curr_ipl`), `simple_lock_irq`, `percpu_get`/`current_thread()` (`%gs`), `__sync_synchronize`, `cpu_pause` | A `SpinLock` type `repr(transparent)` over `natural_t` so C macros keep working; an `IrqGuard` over `splx`; a per-CPU accessor in `src/arch/`; C shims for the lock/percpu/spl macros (first real shim customers) | `kmutex.c`, `eventcount.c`, `priority.c`, `timer.c`, scheduler/IPC/VM files |
| **L3 memory** | `kalloc`/`kfree`, `kmem_cache_*` (slab), `kmem_alloc_wired`, `vm_page_*` | the same C API behind thin shims; optionally later a `GlobalAlloc` over `kalloc` (an explicit design decision, not a quiet add) | `slab.c` itself, `rdxtree.c`, `syscall_emulation.c`, `processor.c`, `task.c` |
| **L4 runnable** | `thread_block`, `thread_wakeup`, `assert_wait`, `thread_setrun`, continuations (`extern "C" fn()` passed across `switch_context`), `set_timeout` | Rust `Thread`/`Task` mirror with locked accessors, a continuation type, and sleep/wake shims while `sched_prim.c` stays C | `ipc_sched.c`, `eventcount.c`, `syscall_subr.c`, `thread_swap.c`, `task.c` |
| **L5 IPC/VM** | `ipc_port`/`ipc_space`/`ipc_kmsg`/`vm_map` with `simple_lock` embedded and refcounts by convention; `copyin`/`copyout`; MIG wire formats | Rust `Port`/`Space`/`Kmsg`/`VmMap` types or opaque handles with C accessors; a safe copyin/copyout wrapper for slices | `exception.c`, `ipc_kobject.c`, `ipc_tt.c`, `ipc_mig.c`, `vm/*`, `device/*` |
| **L6 arch/MIG** | `switch_context`/`call_continuation`/`stack_handoff`, `pmap`, trap entry in `locore.S`, `mach_trap_table`, MIG-generated `_X*` unmarshallers | `src/arch/<arch>/` with `core::arch::asm!` or shims; `#[repr(C)]` trap/exec frames; acceptance that MIG and trap dispatch stay C | scheduler, exception, syscall, boot files |

### The two hard ABI walls

* **MIG** (`*.srv`/`*.cli` → `*.server.c`/`*.user.c` in `build-*/`).
  Routines are *not* generated: each generated server calls the
  hand-written definition (`build-64/ipc/mach_port.server.h:30`).
  A Rust port replaces only that definition — one `#[unsafe(no_mangle)]
  pub unsafe extern "C" fn` with the exact generated prototype.  The
  unmarshalling, `TypeCheck`, retcode packing and `*_server_routines[]`
  table stay C.  A whole generated file can only move when its protocol
  moves together.
* **Trap/asm ABI**.  `kern/syscall_sw.c`'s `mach_trap_table` is consumed
  raw by `locore.S` (16-byte stride on i386, 32 on x86_64); `struct
  eml_dispatch`, `struct machine_slot` and `struct thread` offsets are
  projected into `i386/i386/i386asm.sym`.  Symbol names, sizes and order
  must not change.  Rust can own the table only by reproducing the exact
  `#[repr(C)]` entry layout and the erased function-pointer type.

## 3. Boundary law — write Rust, keep the C edge thin

Distilled from the ports so far and `AGENTS.md`:

* **Safe core, unsafe edge.**  `queue.rs` is the template: safe
  operations over a `!Unpin` `QueueEntry` (`rust/src/kern/queue.rs:111`),
  and 13 one-to-three-statement `extern "C"` wrappers that do nothing but
  turn the C caller's address-stability contract into a `Pin`
  (`queue.rs:379-390`).  `smp.rs` has no pointer at all.  Every port
  should push contracts inward (`Pin`, `Option<NonNull<_>>`,
  `#[repr(transparent)]` newtypes) so the wrapper is the smallest part.
* **`#[repr(C)]` mirror + compile-time asserts** for anything C also
  touches, never a parallel definition with a comment.  `QueueEntry`
  asserts size, alignment and both offsets (`queue.rs:44-51`);
  `elf_load.rs` mirrors `exec_info_t` and `vm_prot_t` the same way.
* **C shims for macros.**  `spl*`, `simple_lock`, `percpu_get`,
  `current_thread()`, `thread_wakeup*`, `__builtin_offsetof` queue ops
  are macros or asm and cannot be declared in `glue`; the first Rust
  customer of each gets a one-line C shim beside the header that defines
  it.  `glue` declares `Panic` and the C functions the ports call
  (`rust/src/glue/mod.rs:30`).
* **At atomics, be explicit.**  `__sync_synchronize` becomes
  `fence(SeqCst)`; the `xchg` interlock becomes
  `AtomicU32::swap(AcqRel)`; `kmutex`'s CAS keeps `Acquire`/`Release`
  exactly.  `-C overflow-checks=off` means boundary arithmetic uses
  `wrapping_*`/`checked_*`/`saturating_*` deliberately (`timer.c`,
  `mach_factor.c`, `mach_clock.c`).
* **Typed views for bitfields.**  Rust cannot express C bitfields; expose
  the raw word and mask/construct through typed accessors while the C
  macros still read the same word (`thread.state`, `lock_data`'s
  `read_count:16/...`).
* **No hidden allocation, no unwinding.**  `panic=abort`, so a reachable
  `assert!` becomes a kernel `Panic`; `gnumach-undef-bad` means a port
  must not introduce new undefined symbols.
* **Keep the frozen names.**  MIG `intran`/`outtran` symbols
  (`include/mach/mach_types.defs:198-248`), trap entries and asm-read
  globals keep their exact C names and types; rename only after the C
  readers are gone.
* **Test copies.**  The user tests are frozen binaries in `abi-test/`
  and link their own copies of `kern/printf.c`, `util/atoi.c` and the
  string routines, so a port in this tree no longer needs a test copy.

What Rust still lacks (as of the rbtree port): an allocator over
`kalloc`/`kmem_cache`, an RAII lock/IRQ layer, per-CPU access, a struct
binding strategy beyond hand-written mirrors (no bindgen by design), a
`printf`/`log` glue, and a shared `src/arch/<arch>/` platform module
(drivers have opened `src/arch/i386/`, but there is no spl or per-CPU
layer yet).  §4's blockers name which missing piece each file needs.

## 4. `kern/` — file-by-file map

Undef = undefined symbols in the x86_64 object at audit time, after
the first six ports (queue and string routines are Rust now, so they
appear as calls into Rust).  Layer = the highest prerequisite layer
from §2.

### 4.0 Summary table

| File | LOC | Undef | Layer | Friction | Rust home |
|---|---:|---:|---|---:|---|
| `rbtree.c` | 463 | 0 | L0 | **2** | `src/kern/rbtree.rs` |
| `timer.c` | 236 | 0 | L0+L2 | **3** | `src/kern/timer.rs` |
| `thread_swap.c` | 196 | 14 | L4 | **2** | `src/kern/thread_swap.rs` |
| `mach_factor.c` | 150 | 6 | L1+L4 | **2** | `src/kern/mach_factor.rs` |
| `kmutex.c` | 75 | 2 | L2+L4 | **3** | `src/kern/kmutex.rs` |
| `boot_script.c` | 728 | 14 | L3 | **2** | `src/kern/boot_script.rs` |
| `rdxtree.c` | 799 | 4 | L3 | **3** | `src/kern/rdxtree.rs` |
| `ast.c` | 221 | 10 | L1+L2+L6 | **3** | `src/kern/ast.rs` + `src/arch/` |
| `debug.c` | 146 | 9 | L2+L5 | **3** | `src/kern/debug.rs` (partial) |
| `syscall_sw.c` | 220 | 27 | L6 | **3** | `src/kern/syscall_sw.rs` |
| `ipc_tt.c` | 1101 | 21 | L5 | **3** | `src/kern/ipc_tt.rs` |
| `ipc_host.c` | 506 | 12 | L5 | **3** | `src/kern/ipc_host.rs` |
| `host.c` | 418 | 25 | L1+L4+L5 | **3** | `src/kern/host.rs` |
| `syscall_emulation.c` | 453 | 9 | L3+L6 | **4** | `src/kern/syscall_emulation.rs` |
| `priority.c` | 196 | 10 | L2+L4 | **4** | `src/kern/priority.rs` |
| `eventcount.c` | 301 | 9 | L2+L4 | **4** | `src/kern/eventcount.rs` |
| `lock.c` | 463 | 4 | L2+L4 | **4** | `src/kern/lock.rs` |
| `gsync.c` | 537 | 15 | L3+L4+L5 | **4** | `src/kern/gsync.rs` |
| `ipc_sched.c` | 273 | 10 | L4 | **4** | `src/kern/ipc_sched.rs` |
| `ipc_kobject.c` | 362 | 23 | L5 | **4** | `src/kern/ipc_kobject.rs` |
| `mach_clock.c` | 751 | 25 | L2+L4+L6 | **4** | `src/kern/mach_clock.rs` |
| `syscall_subr.c` | 367 | 14 | L4+L6 | **4** | `src/kern/syscall_subr.rs` |
| `machine.c` | 651 | 35 | L4+L6 | **4** | `src/kern/machine.rs` |
| `processor.c` | 1007 | 36 | L1+L4+L5 | **4** | `src/kern/processor.rs` |
| `printf.c` | 656 | 3 | L2 | **5** | `src/kern/printf.rs` + C shims |
| `bootstrap.c` | 770 | 47 | L3+L6 | **5** | `src/kern/bootstrap.rs` |
| `startup.c` | 290 | 57 | L6 | **5** | `src/kern/startup.rs` |
| `slab.c` | 1280 | 25 | L3+L4+L5 | **5** | `src/kern/slab.rs` |
| `thread.c` | 2593 | 75 | L1+L4+L5+L6 | **5** | `src/kern/thread.rs` |
| `task.c` | 1408 | 72 | L1+L4+L5+L6 | **5** | `src/kern/task.rs` |
| `sched_prim.c` | 1912 | 47 | L1+L4+L6 | **5** | `src/kern/sched_prim.rs` |
| `ipc_mig.c` | 1019 | 58 | L5+L6 | **5** | `src/kern/ipc_mig.rs` |
| `exception.c` | 974 | 32 | L5+L6 | **5** | `src/kern/exception.rs` |

`rbtree.c` has since been ported; its entry below and the §9 table
are current.  `sched_prim.c` is partly ported: the wait/wake
primitives, `thread_dispatch` and `thread_setrun` are Rust now, and
its entry below and the §9 table record what moved.

### 4.1 Detailed entries

#### `kern/rbtree.c` — 463 lines — ported
* **Role.** Red-black tree over intrusive nodes; the color bit is
  packed in the parent pointer (2-bit masks, `rbtree_i.h:60-74`).
* **Rust home.** `src/kern/rbtree.rs`.  `RbtreeNode` and `Rbtree` are
  `#[repr(C)]` mirrors whose links are `Option<NonNull<_>>` (the null
  niche keeps the C layout); size, alignment and the children offset
  are asserted on the Rust side and mirrored with `_Static_assert`s in
  `rbtree_i.h`.  The parent/color bit encoding is Rust-only now (a
  private `Color` enum), as is the insertion-point slot, packed by
  `rbtree_slot()` and unpacked by `rbtree_insert_slot()`; the
  null-child index rule and the "stale node after remove" contract are
  preserved.  The child sides are a private `Side` enum — the ABI's
  `c_int` directions convert once at the boundary — and `Rbtree`/
  `RbtreeNode` have `new()` and `unlinked()`/`init()` constructors
  that the C entry points and the tests share.  The core is
  Rust-native: a copyable `NodeRef` handle and `Rbtree` methods carry
  the algorithms, and the nine exports are thin adapters.  Every
  `unsafe` block carries its own `// SAFETY:` note.
* **Boundary.** Nine `unsafe extern "C"` symbols:
  `rbtree_insert_rebalance`, `rbtree_remove`, `rbtree_nearest`,
  `rbtree_firstlast`, plus the leaf operations `rbtree_init`,
  `rbtree_node_init`, `rbtree_insert_slot`, `rbtree_slot` and
  `rbtree_d2i`.  `rbtree_d2i()` is called once per tree level by the
  lookup macros; the call is the price of keeping the index rule with
  the tree.  The generic macros stay C in `rbtree.h` for `slab.c`,
  and because only they embed `cmp_fn`, the Rust half takes no
  callbacks at all; `rbtree_entry`/`structof` stay macros.  (The
  `vm_map` trees were the other user until the port moved them to the
  Rust rbtree methods.)
* **Prune (cleanup).** `rbtree_lookup`, `rbtree_empty`,
  `rbtree_node_unlinked`, `rbtree_prev`/`rbtree_next`,
  `rbtree_for_each_remove`, `rbtree_check_alignment`,
  `rbtree_check_index` and the `rbtree_parent` inline had no callers,
  and went together with the three exports only they used
  (`rbtree_walk`, `rbtree_postwalk_deepest`, `rbtree_postwalk_unlink`).
  Moving the leaf inlines to Rust then pruned
  `rbtree_slot_parent`/`rbtree_slot_index` and the `RBTREE_COLOR_*`,
  `RBTREE_PARENT_MASK` and `RBTREE_SLOT_*` macros, so C no longer
  encodes a color or unpacks a slot.  The `rbtree_first`/`rbtree_last`
  macros are gone too; `vm_map.c` spells `rbtree_firstlast()` out.
* **Panics (cleanup).** `expect` remains only where the red-black
  rules re-derive a link (rotate's child, a red node's grandparent,
  the brother and its far child).  The insert and remove restructurings
  track the parent across a swap and the successor's parent down the
  descent, which removed three of them; a corrupt tree panics with a
  message where C would fault.
* **Tests.** `rbtree.rs` carries `#[cfg(test)]` tests that reimplement
  the macro protocols (insert, lookup_slot/insert_slot,
  lookup_nearest) and check the red-black rules after every mutation,
  plus the node colors and the slot round-trip.  The host runner was
  dropped with the old suite, so nothing compiles them today; re-add
  one before relying on them.  The `vm_map` trees (Rust since the port)
  and `kern/slab.c`'s active-slab tree (used by every non-direct cache,
  `slab.c:641-652`) exercise the functions through the ABI pack.

#### `kern/timer.c` — 236 lines — friction 3/5
* **Role.** Per-thread and per-CPU statistical timers (microseconds and
  seconds) with a seqlock-style read (`high_bits_check`) tolerant of
  concurrent normalization.
* **Exports/data.** `timer_init`, `init_timers`, `timer_normalize`,
  `timer_read`, `timer_delta`, `thread_read_times`,
  `db_thread_read_times`; owns `current_timer[NCPUS]` and
  `kernel_timer[NCPUS]` (`timer.c:39-40`).
* **Dependencies — why.** `cpu_number()` is the `percpu_get` macro over
  `%gs` (`i386/i386/cpu_number.h:54`), needed to index the per-CPU
  arrays; `__sync_synchronize()` makes the check/high publish order safe
  (`timer.c:93-116`).  `timer_bump` is a macro in `timer.h:105` that
  `mach_clock.c` applies directly to `struct timer` fields, so the
  fields must stay C-visible.  `TIMER_DELTA` moved to
  `src/kern/timer.rs` as `TimerSave::delta`, which calls `timer_delta()`
  for its coherency slow path.
* **Blockers.** A per-CPU accessor: either `src/arch/` asm or a C shim
  for `cpu_number()`; the `struct timer` mirror.
* **Boundary / notes.** `#[no_mangle] static mut` arrays with the same
  size/alignment; write order in `timer_normalize` (check first, high
  last) and `fence(SeqCst)` must match — a safe `Timer` API can exist
  internally, but the C macros keep poking the fields until
  `thread.c`/`mach_clock.c` move.  `timer_delta` is pure arithmetic and
  can be fully safe.

#### `kern/thread_swap.c` — 196 lines — friction 2/5
* **Role.** The swapin queue and the swapper kernel thread: allocate a
  kernel stack for a swapped-out thread and put it back on a run queue.
* **Exports/data.** `swapin_queue`, `swapper_init`, `thread_swapin`,
  `thread_doswapin`, `swapin_thread`.
* **Dependencies — why.** Scheduler primitives only: `thread_wakeup`,
  `assert_wait`/`thread_block`, `thread_setrun`, `thread_continue`,
  `stack_alloc`/`stack_privilege` (`thread_swap.c:97-192`).  Locks are
  the `simple_lock` macros; queues are already Rust functions.
* **Blockers.** The L4 sleep/wake/continuation story and `thread.state`/
  `links` access.  Small and regular once `Thread` has locked field
  accessors.
* **Boundary / notes.** `swapin_queue` is both a data symbol and the
  wakeup event address; keep the same static address.  `thread->links`
  is reused as the swapin chain, so the same `!Unpin` `QueueEntry`
  discipline applies.  `thread_doswapin` must clear
  `TH_SWAPPED|TH_SW_COMING_IN` before `stack_alloc` can expose the
  thread.

#### `kern/mach_factor.c` — 150 lines — friction 2/5
* **Role.** Periodic load averaging: publishes `avenrun[3]` and
  `mach_factor[3]` and updates each pset's `sched_load`.
* **Exports/data.** `compute_mach_factor`; owns `avenrun`,
  `mach_factor`, static `fract[3]`.
* **Dependencies — why.** Reaches into `processor_set` and `processor`
  run-queue counters under `simple_lock` and walks `all_psets`
  (`mach_factor.c` via `processor.h:116-118`).  Outputs are read by
  `host.c:179-190`; there is no asm, allocation or percpu.
* **Blockers.** `ProcessorSet`/`Processor` mirrors with locked
  accessors; otherwise it is pure arithmetic (fix-point shifts
  `SCHED_SCALE`/`SCHED_SHIFT`).
* **Boundary / notes.** Keep the three globals as `#[no_mangle]`
  statics until `host.c`/`processor.c` move, so there is one writer.

#### `kern/kmutex.c` — 75 lines — friction 3/5
* **Role.** Three-state sleepable mutex (unowned/locked/contended) with
  a lock-free fast path.
* **Exports.** `kmutex_init`, `kmutex_lock`, `kmutex_trylock`,
  `kmutex_unlock`; only caller is `gsync.c`.
* **Dependencies — why.** `atomic_cas_acq`/`atomic_swap_acq`/
  `atomic_cas_rel` (GCC `__atomic_*`, `kern/atomic.h:24-52`) implement
  the state machine; `thread_sleep` and `thread_wakeup_one` implement
  contention (`kmutex.c:50,69`).  `struct kmutex` is embedded in
  gsync's 512 hash buckets, so its layout is shared.
* **Blockers.** `SimpleLock` type and sleep/wake shims.
* **Boundary / notes.** Ideal early L2/L4 file: the fast path is one
  `compare_exchange(Acquire, Relaxed)`/`swap(Acquire)` pair; keep the
  `repr(C)` struct and the `state` offset.  No owner tracking exists in
  C; do not add one silently (gsync depends on lock ordering at the
  call site).

#### `kern/boot_script.c` — 728 lines — friction 2/5
* **Role.** Parser/executor for the Multiboot `$0`/`${var}`/`$(func)`
  boot-script DSL: symbol table, substitution, deferred commands.
* **Exports.** `boot_script_parse_line`, `boot_script_exec`,
  `boot_script_set_variable`, `boot_script_error_string`,
  `boot_script_define_function` (dead export, no callers).
* **Dependencies — why.** No IPC/VM/sched calls.  Allocation goes
  through the host callbacks `boot_script_malloc/free` implemented in
  `bootstrap.c:695-705`; libc calls are the already-Rust string
  routines.  The nine host callbacks (`boot_script.h:59-82`) are the
  only coupling, plus `struct cmd` (`boot_script.h:27-55`) which
  `bootstrap.c` fills.
* **Blockers.** A `BootScriptAlloc` trait backed by the `extern "C"`
  callbacks (no `alloc` in the crate); keep `struct cmd` `#[repr(C)]`
  while C callbacks remain.
* **Boundary / notes.** Parser over `&str`/slices inside, exact error
  codes outside.  Gotchas: it mutates the caller's module memory in
  place (`*q = '\0'`), and symbol values are `long`↔function-pointer
  puns that differ between i386 and x86_64 — use `usize`.

#### `kern/rdxtree.c` — 799 lines — friction 3/5
* **Role.** 64-bit-key radix-6 tree with insert/lookup/remove/walk and
  key allocation; used by the IPC space for port names.
* **Exports.** `rdxtree_cache_init`, `rdxtree_insert_common`,
  `rdxtree_insert_alloc_common`, `rdxtree_remove`,
  `rdxtree_lookup_common`, `rdxtree_replace_slot`, `rdxtree_walk`,
  `rdxtree_remove_all`; the header wraps these in `static inline`
  functions (`rdxtree_i.h`).
* **Dependencies — why.** Only `kmem_cache_alloc/free` and `memset`
  (`rdxtree.c:156,180,165`); the cache is a private
  `kmem_cache rdxtree_node_cache` (`rdxtree.c:118`).  The `llsync_*`
  macros are plain assignments here; safe only because callers hold the
  IPC space lock — do not present it as lock-free.
* **Blockers.** Slab itself or a minimal shim for the node cache (its
  `rbtree` dependency is ported).  The inline header API stays C,
  calling the Rust `_common` symbols.
* **Boundary / notes.** `struct rdxtree_node` is private, so a Rust
  `Node` enum (`Stored(NonNull<u8>)` / child) can be native.  Gotchas:
  low-bit node tagging, `void ***slotp` return, allocate/shrink root
  replacement (`rdxtree.c:320-322`), `ffs`-based allocation bitmap with
  `(unsigned)-1` sentinel (`:531-534`).

#### `kern/ast.c` — 221 lines — friction 3/5
* **Role.** Per-CPU AST state, `ast_check` decision, `ast_taken` on the
  user-return path.
* **Exports/data.** `need_ast[NCPUS]`, `ast_init`, `ast_taken`,
  `ast_check`; `need_ast` is read directly by `locore.S`
  (i386 `locore.S:359,573`; x86_64 `:504,731,1117`) and written by the
  `ast_on/off/context` macros in C (`net_io.c:541`, `fpu.c:856`,
  `trap.c:484`, `task.c:741`, `ipc_sched.c:221`).
* **Dependencies — why.** Scheduler (`csw_needed`, `thread_block`,
  `thread_halt_self`, `thread_exception_return`) because the AST handles
  thread halt/block at the user boundary; arch (`spl*`, `cpu_number`,
  `processor_ptr`) because it runs at IRQ level; `net_ast` for the
  network AST.  AST bits themselves are arch (`i386/i386/ast.h:38`).
* **Blockers.** L1 thread/processor mirrors, L2 percpu+IRQ, the exported
  `need_ast` global.  `cause_ast_check`/`init_ast_check`
  (`i386/i386/ast_check.c`, APIC IPI) stay arch C.
* **Boundary / notes.** Define `need_ast` as a `#[no_mangle] static mut
  [usize; NCPUS]` (or atomics), but C macros keep writing it; do not
  reorder or make it private.  `ast_taken` must clear `need_ast` before
  `spl0()` exactly as `ast.c:75-77` does.

#### `kern/debug.c` — 146 lines — friction 3/5
* **Role.** `Panic`, soft debugger stubs, `log`, stack-canary support.
* **Exports/data.** `SoftDebugger`, `Debugger`, `panic_init`, `Panic`,
  `log`, `panicstr`, `paniccpu`, `__stack_chk_guard`,
  `__stack_chk_fail`.
* **Dependencies — why.** `printf`/`_doprnt` for the varargs formatter,
  `cnputc` for the console, `halt_cpu`/`halt_all_cpus` for the halt
  path, `delay` (now Rust), `cpu_number`.  Locks are
  `simple_lock_irq`/`simple_unlock_irq` (`debug.c:65-94`).
* **Blockers.** `Panic` is variadic and must stay the C symbol; the
  Rust `#[panic_handler]` already calls it through `glue`.  The
  canary symbols are referenced by compiler-generated code and cannot
  change name or size.
* **Boundary / notes.** Port only the non-variadic state:
  `panicstr`/`paniccpu` to Rust atomics (`src/kern/debug.rs`) with the
  same symbols; keep `Panic`, `log`, `__stack_chk_guard` in C.  Gotcha:
  `panic_init` is called repeatedly and must stay idempotent.

#### `kern/syscall_sw.c` — 220 lines — friction 3/5
* **Role.** The `mach_trap_table`: ~130 entries mapping negated trap
  numbers to kernel routines and arg counts; read raw by `locore.S`
  (i386 `:715,743`; x86_64 `:873,928,1111`) and by the debugger.
* **Exports/data.** `mach_trap_table`, `mach_trap_count`,
  `kern_invalid_debug`; statics `null_port`, `kern_invalid`.
* **Dependencies — why.** Every entry names a trap body from another
  file (`ipc_mig.c`, `ipc_tt.c`, `ipc_host.c`, `syscall_subr.c`,
  `eventcount.c`, `ipc/mach_msg.c`, `debug.c`) — the table is the
  syscall ABI, so its order/size/names cannot change.  Entry layout:
  `{int arg_count; fptr; boolean_t stack; const char *name}`.
* **Blockers.** None of its own beyond `#[repr(C)]` entry type and
  `Option<extern "C" fn()>` construction; but each callee must already
  exist with its C name.  The table itself can move early if built as a
  native Rust static with exact layout — the biggest "cheap" ABI win.
* **Boundary / notes.** `mach_trap_count` must match; do not reorder or
  resize.  Trap 0-9 are reserved (Unix).  A `macro_rules!` entry
  constructor keeps the 130 lines readable and the argument counts
  auditable.

#### `kern/ipc_tt.c` — 1101 lines — friction 3/5
* **Role.** Task/thread IPC state: kernel self ports, exception/
  bootstrap/registered ports, special-port get/set, and the
  port↔task/thread/space/map conversions used by MIG `intran`/`outtran`.
* **Exports.** ~26: `ipc_task_init/enable/disable/terminate`,
  `ipc_thread_*`, `retrieve_task_self_fast`/`retrieve_thread_self_fast`,
  `mach_task_self`, `mach_thread_self`, `mach_reply_port`,
  `task/thread_get/set_special_port`, `mach_ports_register/lookup`,
  `convert_port_to_task/space/map/thread`,
  `convert_task_to_port`/`convert_thread_to_port`, `space_deallocate`.
* **Dependencies — why.** IPC (space create/destroy, port alloc/make/
  copy/release, kobject set, `ip_lock`); task/thread refs because the
  ports belong to those objects; `vm_map_reference` because
  `convert_port_to_map` hands one out; `kalloc` for the registered-port
  array.  Locks are `simple_lock`; `current_task/thread` are percpu.
* **Blockers.** L5 (`Port`/`Space` handles) and L1 (`task.itk_*`,
  `thread.ith_*` offsets).  MIG pins every symbol name.
* **Boundary / notes.** Port the four `convert_port_to_*` plus
  `space_deallocate` first (one shared fallible lookup); keep
  `ipc_task_init/terminate` for last.  `retrieve_*_fast` bumps
  `ip_srights` inline — exact behaviour required.  `mach_ports_register`
  consumes caller memory; keep the ownership contract.

#### `kern/ipc_host.c` — 506 lines — friction 3/5
* **Role.** Allocates/destroys the kernel ports for `realhost`,
  processors and processor sets, and converts between them.
* **Exports.** 19: `ipc_host_init`, `mach_host_self`,
  `ipc_processor_init`, `ipc_pset_init/enable/disable/terminate`,
  `processor_set_default`, 12 `convert_*` frozen by MIG.
* **Dependencies — why.** `ipc_port_alloc_special`/`dealloc_special`
  because these are kernel-owned special ports; `ipc_kobject_set`
  because ports carry object tags (`IKOT_HOST`, `IKOT_PROCESSOR`,
  `IKOT_PSET`, ...); `pset_reference/deallocate` because the port owns a
  pset ref; `default_pset`/`master_processor` because those are the
  objects.  No asm.
* **Blockers.** L5 port/kobject handles.  `ipc_pset_enable` must
  atomically set two kobjects and take refs under `pset->lock`
  (`ipc_host.c:532-545`) — preserve.
* **Boundary / notes.** Thin `extern "C"` conversion wrappers over a
  typed `port.kotype()` match once ports move; `ipc_host_init` is
  boot-single-threaded, so its `unsafe` can stay narrow.

#### `kern/host.c` — 418 lines — friction 3/5
* **Role.** Non-IPC host services behind `mach_host.defs`:
  `host_processors`, `host_info`, `host_kernel_version`,
  `host_processor_sets`, `host_processor_set_priv`,
  `processor_set_processors`.
* **Exports/data.** The above plus `realhost` (referenced by
  `ipc_host.c`, `processor.c`, `bootstrap.c`, `pcb.c`).
* **Dependencies — why.** `machine_slot`/`machine_info` (arch tables)
  for `host_info`; `all_psets`/`all_psets_lock` and `pset->lock` to walk
  processor sets; `convert_*` from `ipc_host.c` to hand out ports;
  `kalloc`/`kfree` for the arrays MIG copyouts consume; `tick`/
  `min_quantum`/`avenrun` for the info flavors.
* **Blockers.** L1 pset/processor mirrors, the arch machine table
  accessor, a `kalloc` shim; MIG signatures frozen.
* **Boundary / notes.** Start with `host_get_kernel_version` and
  `host_info` (bounded copies into caller arrays).  The `MACH_HOST`
  branches must be `#[cfg]`-selected from the same configure define.
  `host_processor_sets` retries allocation under `all_psets_lock`; keep
  the retry.

#### `kern/syscall_emulation.c` — 453 lines — friction 4/5
* **Role.** Per-task user syscall emulation dispatch vector; consulted
  by `locore.S` before the native trap table.
* **Exports.** `eml_init`, `eml_task_reference`, `eml_task_deallocate`,
  `task_set_emulation_vector`, `task_get_emulation_vector`,
  `task_set_emulation` (the last three are MIG server entries).
* **Dependencies — why.** `kalloc/kfree` (the vector is heap);
  `simple_lock` on task and vector; `vm_map_copyin/copyout` because an
  out-of-line `emulation_vector_t` travels as a `vm_map_copy_t`
  (`syscall_emulation.c:308-314`); `task->lock`/`eml_dispatch`.
* **Blockers.** L3 allocator shim and the L6 ABI: `struct
  eml_dispatch` offsets are baked into `i386asm.sym:69-73` and read by
  `i386/locore.S:680-689`, `x86_64/locore.S:841-850`.  Do not move the
  struct to Rust without regenerating matching offsets.
* **Boundary / notes.** `#[repr(C)]` struct with trailing
  `disp_vector[1]`; keep the `count_to_size` power-of-two allocation and
  the lock-protected allocate/race protocol (`:168-270`) exactly.  A
  good later file once the allocator is shimmed.

#### `kern/priority.c` — 196 lines — friction 4/5
* **Role.** Per-tick quantum accounting and lazy priority update for the
  running thread, called from the clock interrupt.
* **Exports.** `thread_quantum_update` (only caller `mach_clock.c`).
* **Dependencies — why.** Scheduler (`update_priority`,
  `compute_my_priority`, `thread_timer_delta`, `ast_check`,
  `sched_tick`/`min_quantum`) because it adjusts the running thread's
  quantum; `processor_ptr` percpu and
  `simple_lock_irq`/`splsched` because it runs in interrupt context.
* **Blockers.** L2 IRQ-safe lock + percpu, L4 `thread`/`processor`/
  `processor_set` mirrors, and `update_priority`/`compute_my_priority`
  must already exist (they are in `sched_prim.c`).
* **Boundary / notes.** The `quantum_adj_lock` is the `lock_irq` pair;
  mixing it with the scheduler spin lock breaks the `splsched`
  protocol.  Keep the `if (pset == 0) return;` re-check (`priority.c:82`)
  — it observes a processor being reassigned.

#### `kern/eventcount.c` — 301 lines — friction 4/5
* **Role.** User-visible event counters (`evc_wait`/`evc_wait_clear`,
  traps 17/18) with a global registry, plus `evc_notify_abort` from
  thread teardown.
* **Exports/data.** `evc_init`, `evc_destroy`, `evc_notify_abort`,
  `evc_wait`, `evc_wait_clear`, `evc_signal`; owns
  `all_eventcounters[MAX_EVCS]`.
* **Dependencies — why.** `assert_wait`/`thread_block` for blocking,
  `splsched` to guard the record, and — the sharp edge — it directly
  manipulates another thread's lock and state
  (`eventcount.c:245-247,260-285`) instead of using `clear_wait`.
* **Blockers.** L4 thread sleep/wake with locked field access; an IRQ
  guard for `splsched`.
* **Boundary / notes.** Model `waiting_thread: Option<ThreadRef>` behind
  the record lock and delete the cross-struct lock poking; until
  `thread.rs` exists, keep raw accessors.  Preserve the
  `-1`/`0`/`N` count protocol (`:149-160`) and the "wait_clear blocks
  forever if already signalled" semantics.

#### `kern/lock.c` — 463 lines — ported
* **Role.** Sleep-capable recursive reader/writer lock built around a
  `simple_lock` interlock; the lock state lives in caller-declared
  `lock_data_t` (`lock.h:110-126`).
* **Rust home.** `src/kern/lock.rs`.  `LockData` keeps its `#[repr(C)]`
  layout (16 bytes on x86_64, 12 on i386) with the size and offset
  assertions; `thread` and `state` are `UnsafeCell` so mutation through
  `&self` under the interlock is sound.  The bitfield word keeps the C
  packing (`read_count:16, want_upgrade:1, want_write:1, waiting:1,
  can_sleep:1, recursion_depth:12`, from the least significant bit),
  reached through named shift/mask accessors.  The eleven exports are
  thin `unsafe extern "C"` adapters over Rust methods; every algorithm
  (the 100-pause backoff, the interlock release before sleeping, the
  `waiting`/`read_count` wake policy, the failed-upgrade semantics of
  `lock_read_to_write`) is the C one, and the boolean-returning entry
  points are `#[must_use]`.  Every `unsafe` block carries its own
  `// SAFETY:` note.
* **Exports.** `lock_init`, `lock_write`, `lock_read`, `lock_done`,
  `lock_read_to_write`, `lock_write_to_read`, `lock_try_write`,
  `lock_try_read`, `lock_try_read_to_write`, `lock_set_recursive`,
  `lock_clear_recursive`; `lock_sleepable`, which had no caller, was
  dropped with the port.
* **Dependencies.** `current_thread` for recursion identity; the Rust
  `thread_sleep`/`thread_wakeup_prim` for the wait path, which releases
  the interlock after enqueue exactly as `lock.c:134` did; the
  `SimpleLock` interlock, which stays because it is part of the
  `struct lock` layout.
* **Boundary / notes.** `kern/lock.h` keeps the structs, the macros and
  the prototypes while C structs embed `struct lock`; the C file is
  gone.  The `waiting` read in `lock_done` stays unsynchronized by
  design (`:187`).  The port rode on `sched_prim.c`'s Rust sleep/wake
  and needs nothing further.
* **Machine half.** `i386/i386/lock.h` is gone too: its bit operations
  live in `src/arch/i386/atomic_bits.rs` and its simple-lock primitives
  are the `mach_simple_*` entry points in `src/kern/lock.rs`.  The
  header keeps only the ABI structs, the thin dispatch macros and the
  prototypes, so the C layouts survive until their embedders move.

#### `kern/gsync.c` — 537 lines — friction 4/5
* **Role.** Address-keyed wait/wake (futex analogue) over 512 sorted
  hash buckets, with shared/local keys, timeouts, broadcast and
  requeue; used by `tests/test-gsync.c` only.
* **Exports/data.** `gsync_setup`, `gsync_wait`, `gsync_wake`,
  `gsync_requeue`; owns `gsync_buckets[512]`.
* **Dependencies — why.** `kmutex` per bucket; `vm_map_lookup/enter/
  remove`, `vm_object_reference/deallocate` because a wait key may be a
  `{map,addr}` or `{object,offset}`; `copyin/copyout` to mutate remote
  keys; `thread_block`, `thread_will_wait[_with_timeout]`, `clear_wait`.
  It is the most cross-module leaf in `kern/`.
* **Blockers.** L3/L4/L5 all at once: list, kmutex, VM map/object
  wrappers, copyin/out abstraction.
* **Boundary / notes.** Typed key enum; manual list splice at
  `gsync.c:519-524` becomes a list operation; the in-place key rewrite
  while walking (`:507-515`) is the subtle part.  Object unlock must
  happen exactly once on every path.

#### `kern/ipc_sched.c` — 273 lines — friction 4/5
* **Role.** IPC-visible scheduler primitives: `thread_go`,
  `thread_will_wait[_with_timeout]`, `thread_handoff`, placed here
  because callers hold IPC locks.
* **Exports.** The four above; callers are `ipc/ipc_mqueue.c`,
  `ipc/ipc_port.c`, `ipc/mach_msg.c`, `gsync.c`, `syscall_subr.c`,
  `exception.c`.
* **Dependencies — why.** `splsched`/`_simple_lock` because it runs
  under IPC locks at splsched; `set_timeout`/`reset_timeout` for timed
  waits; `thread_setrun`, `stack_handoff`, `ast_context`,
  `current_processor`/`current_stack`.  `stack_handoff` is asm
  (`i386/i386/pcb.c`), and continuation identity is load-bearing
  (`ipc/mach_msg.c` compares `swap_func`).
* **Blockers.** L4 sleep/wake and a continuation representation; keep
  `stack_handoff` C/arch.
* **Boundary / notes.** Port `thread_go` and the `will_wait` pair first
  with opaque `Thread` handles; keep `thread_handoff` until stacks and
  continuations are modeled.  Keep `_simple_lock` (no spl bump) distinct
  from `simple_lock`.

#### `kern/ipc_kobject.c` — 362 lines — friction 4/5
* **Role.** Routes messages sent to kernel ports to the matching MIG
  server routine; owns port→kobject binding and kobject teardown.
* **Exports.** `ipc_kobject_server`, `ipc_kobject_set`,
  `ipc_kobject_set_locked`, `ipc_kobject_destroy`,
  `ipc_kobject_notify`.
* **Dependencies — why.** `ipc_kmsg` allocation/copy because it builds
  and forwards messages; `ipc_port_release_*`; ten MIG
  `*_server_routine()` selectors plus the machine selector; VM/device/
  proxy notify hooks per kobject type.  The dispatch table is generated
  C function pointers (`ipc_kobject.c:160-170`).
* **Blockers.** L5 `Kmsg`/`Port` types and the decision to leave MIG's
  routine tables C.  `ipc_kobject_set_locked` is a 3-line function and
  a fine first slice.
* **Boundary / notes.** Keep `ipc_kobject_server` `extern "C"`;
  preserve the manual destination-release dance (`:196-209`) and the
  `MACH_NOTIFY_*` fallback dispatch.

#### `kern/mach_clock.c` — 751 lines — friction 4/5
* **Role.** Timekeeping and the timeout wheel: `clock_interrupt`,
  `softclock`, `set_timeout`/`reset_timeout`, wallclock/uptime with HPET
  interpolation, the mmap-able time page, and the `host_*time*` MIG
  bodies.
* **Exports/data.** The above plus `hz`, `tick`, `time`, `uptime`,
  `elapsed_ticks`, `softticks`, `timeoutwheel[256]`,
  `timeout_timers[20]`, `mtime`, `clock_boottime_offset`.
* **Dependencies — why.** Per-CPU (`cpu_number`, `machine_slot`,
  `cpu_idle`) because tick accounting is per CPU; thread/sched
  (`current_thread`, `timer_bump`, `thread_quantum_update`,
  `thread_bind`/`thread_block` for time-set migration); IRQ-safe locks
  (`simple_lock_irq`, `splclock`/`splhigh`/`splsoftclock`); VM
  (`kmem_alloc_wired`) for the time page; machine clock
  (`hpclock_read_counter`, `resettodr`).  `struct timeout` is embedded
  in `thread.h:206-207` and pageout/chario timers.
* **Blockers.** L2 IRQ+percpu, L1 `struct timeout` mirror, and a
  `extern "C"` `set_timeout` while C callers remain.  Split it: the
  wheel first, then stamps, then `host_*`.
* **Boundary / notes.** `elapsed_ticks + interval + 1` off-by-one
  (`:426`); master-CPU-only updates (`:241`); `mtime` can be 0 before
  `mapable_time_init`; the `#warning` at `:112` (32-bit `last_hpc_read`)
  should be fixed deliberately, not preserved by accident.

#### `kern/syscall_subr.c` — 367 lines — friction 4/5
* **Role.** Native scheduling traps: `swtch`, `swtch_pri`,
  `thread_switch`, depression timeout/abort, `mach_print`.
* **Exports.** The five above plus `thread_depress_timeout`,
  `thread_depress_abort`.
* **Dependencies — why.** Deep scheduler internals
  (`thread_block`/`thread_run`/`rem_runq`, run-queue counts,
  `compute_priority`, `thread_depress_*`, `min_quantum`, `NRQS`);
  `set_timeout`/`reset_timeout` on `thread->depress_timer`; IPC
  (`ipc_port_translate_send`, `ip_active`, `ip_kotype`, `IKOT_THREAD`)
  for `thread_switch`'s hint; `splsched` and raw `_simple_lock` on
  `thread->lock`.
* **Blockers.** L4 scheduler API and continuation model; trap ABI
  (`thread_syscall_return`) stays.
* **Boundary / notes.** Continuations stay `extern "C" fn()` passed to
  `thread_block`; preserve the port-unlock-before-`thread_run` order
  (`:209-216`) and the documented softclock race around the depress
  timeout (`:316-335`); depression stays clamped to `NRQS-1`.

#### `kern/machine.c` — 651 lines — friction 4/5
* **Role.** Machine-independent CPU/processor control: `cpu_up`/
  `cpu_down`, `processor_assign`/`processor_shutdown`, reboot, and the
  `action_thread` that performs them.
* **Exports/data.** `cpu_up`, `host_reboot`, `processor_assign`,
  `processor_shutdown`, `action_thread_continue`, `action_thread`,
  `processor_doshutdown`; owns `machine_info`, `machine_slot[NCPUS]`,
  `action_queue`.
* **Dependencies — why.** Processor-set and thread primitives to
  unplug/replug CPUs; arch `switch_to_shutdown_context`,
  `halt_cpu`/`halt_all_cpus`, `PMAP_DEACTIVATE_KERNEL`, `init_ast_check`,
  per-CPU `percpu_array`.  Locking is `simple_lock`/`splsched`, queues
  are Rust.
* **Blockers.** L1 machine tables, L4 thread/pset, L6 asm-bound
  shutdown.  `machine_slot` layout is read by `host.c` and asm, so it
  stays C for now.
* **Boundary / notes.** Volatile spin at `:480-482` is a lock-free wait
  for pset emptiness — use `read_volatile` + `spin_loop`.  `MACH_HOST`
  blocks need `#[cfg]`.  Shutdown path stays C/arch.

#### `kern/processor.c` — 1007 lines — friction 4/5
* **Role.** Processor and processor-set lifecycle, refcounts, info MIG
  calls, policy setters, task/thread port listing.
* **Exports/data.** `pset_sys_bootstrap`, `pset_init`, `processor_init`,
  `pset_add/remove_processor`, `pset_add/remove_task`,
  `pset_add/remove_thread`, `thread_change_psets`,
  `pset_reference`/`pset_deallocate`, `processor_info`,
  `processor_start`/`exit`/`control`,
  `processor_set_info/max_priority/policy_*`,
  `processor_set_tasks/threads`, `pset_sys_init`,
  `processor_set_create/destroy`; owns `default_pset`, `all_psets`,
  `master_cpu`, `master_processor`, `pset_cache`, `slave_pset`.
* **Dependencies — why.** IPC (`ipc_processor_init`,
  `ipc_pset_*`, `convert_*`) because psets and processors are kobjects;
  task/thread (`task_assign`, `thread_assign`, refs) because it owns
  their membership; `kmem_cache` for pset/processor caches and
  `kalloc` for the temporary port arrays; `cpu_control` arch stub.
* **Blockers.** L1 pset/processor/runq layout, L2 locks, L5 port
  conversions.  No asm in the file.
* **Boundary / notes.** `processor_set_tasks/threads` build raw arrays
  with `kalloc` and convert in place — keep allocation as shims, no
  slices.  Refcount restoration in `pset_deallocate` (`:342-402`) is
  lock-order sensitive.  `master_cpu` is written once and read widely.

#### `kern/printf.c` — 656 lines — friction 5/5
* **Role.** The whole console formatting engine (`printnum`, `_doprnt`,
  `printf`/`iprintf`, `sprintf`/`snprintf`/`vsnprintf`, `safe_gets`,
  the `%b` bit-field format).
* **Exports.** The above plus `printnum`, `indent`, `vprintf`.
* **Dependencies — why.** `cnputc`/`cngetc` (console), `strlen`;
  three output sinks through function pointers.  ~223 call sites; the
  frozen test modules in `abi-test/` link their own copy, so a port in
  this tree touches only the kernel side.
* **Blockers.** `va_list` cannot be implemented in stable Rust.
  The engine can.
* **Boundary / notes.** Split engine from ABI: a `core`-only formatter
  over `&mut dyn FnMut(char)` in `src/kern/printf.rs`, and keep the
  variadic entry points (`printf`, `_doprnt`, `Panic`) as C shims that
  call it.  Preserve `%b` (`:257-314`) and the truncation flag exactly.

#### `kern/bootstrap.c` — 770 lines — friction 5/5
* **Role.** Builds the first user task/thread from Multiboot modules;
  implements the `boot_script.h` host callbacks, `boot_read`/`read_exec`
  for the Rust `exec_load`, and the argv/stack handoff.
* **Exports.** `bootstrap_create`, `boot_script_exec_cmd` and the seven
  other `boot_script_*` host callbacks.
* **Dependencies — why.** Boot data (`boot_info`, `kernel_cmdline`,
  `phystokv`) because it reads modules; task/thread/VM because it
  creates and starts the user task; IPC to hand it host/device ports;
  boot-script parser; the Rust `exec_load` via C callbacks;
  `alloca` for the argument/stack staging.
* **Blockers.** Almost everything: L3 shim (it leaks but allocates),
  L6 user-stack/`set_user_regs`/`thread_bootstrap_return` asm, dual
  Multiboot layouts, and `alloca` has no Rust equivalent — replace with
  a fixed-size buffer or a `kmem_alloc` staging area explicitly.
* **Boundary / notes.** Port `boot_read`/`read_exec` as safe Rust over a
  module slice and build argv with slices/copyout first; keep
  `bootstrap_create` `extern "C"`.  The `boot_script_*` callback surface
  stays C until the parser moves.  Leaks are intentional.

#### `kern/startup.c` — 290 lines — friction 5/5
* **Role.** Master boot sequence (`setup_main`), kernel thread
  bootstrap, per-CPU bring-up, and the final context switch.
* **Exports.** `setup_main`, `start_kernel_threads`,
  `cpu_launch_first_thread`, `reboot_on_panic`.
* **Dependencies — why.** Every subsystem's init in strict order
  (`panic_init` → `sched_init` → `vm_mem_bootstrap` →
  `rdxtree_cache_init` → `ipc_bootstrap` → `vm_mem_init` →
  `ipc_init` → `task_init`/`thread_init`/`swapper_init` → ... →
  `bootstrap_create`); PMAP macros and percpu assignment because it
  activates the first thread; `load_context` because it never returns.
* **Blockers.** All of them.  It is the integration point and its order
  encodes hidden state (no current thread before `:176`, manual
  `TH_RUN` at `:170`).
* **Boundary / notes.** Port last.  Keep `setup_main` and
  `cpu_launch_first_thread` `extern "C"` for `model_dep.c` and the AP
  entry; arch helpers under `src/arch/`.  `startrtclock` must follow an
  active thread (`:285-286`).

#### `kern/slab.c` — 1280 lines — friction 5/5
* **Role.** Bonwick slab allocator plus `kalloc`/`kfree` general caches,
  with active/free/partial lists and an rbtree for buffer→slab
  resolution.
* **Exports/data.** `kmem_cache_init/alloc/free`, `slab_collect`,
  `slab_bootstrap`, `slab_init`, `kalloc_init`, `kalloc`, `kfree`,
  `slab_info`, `host_slab_info`; owns `kalloc_caches[13]`,
  `kmem_cache_list`, `kmem_gc_last_tick`.
* **Dependencies — why.** VM page primitives (`vm_page_grab/release`,
  `vm_page_lookup_pa`, `set_priv`/`get_priv`) because slabs are made of
  pages and the bufctl/buftag live at computed offsets *inside* the
  buffers (`slab.c:298,310`); `kmem_alloc_wired` for bootstrap;
  `rbtree.rs`; simple locks; `elapsed_ticks`/`hz` for GC.
* **Blockers.** L3 VM page shims, a lock wrapper.
  Metadata is pointer arithmetic in caller memory — needs raw pointers
  and deliberate bounds, not slices.
* **Boundary / notes.** `cache->lock` must be dropped before
  `kmem_slab_create` and emptiness revalidated (`:411,734-736`);
  `cache->ctor` callbacks stay C.  This is the file that unlocks `kalloc`
  for everyone; port it after the page shims but before the larger
  consumers.

#### `kern/thread.c` — 2593 lines — friction 5/5
* **Role.** Thread object lifecycle (create/suspend/resume/halt/terminate/
  reaper), thread state/info MIG entries, priorities/policies,
  processor-set assignment, kernel-stack cache.
* **Ported so far.** `thread_init` — `Thread::new()` and the
  `thread_init()` adapter in `src/kern/thread.rs`, beside the full
  `struct thread` mirror the scheduler port landed.  `thread_deallocate`
  and the rest of the file stay C.
* **Exports/data.** ~58 exports: `thread_init`, `thread_create`,
  `thread_terminate[_release]`, `thread_deallocate/reference`,
  `thread_force_terminate`, `thread_halt[_self]`, `thread_hold/release/
  dowait`, `thread_suspend/resume/abort`, `thread_get/set_state`,
  `thread_info`, `kernel_thread`, `reaper_thread`,
  `thread_priority/max_priority/policy/wire/assign/freeze/unfreeze`,
  `stack_alloc/free/privilege/collect`, `host_stack_usage`,
  `processor_set_stack_usage`, `thread_set/get_name`, `thread_stats`;
  owns `thread_cache`, `thread_stack_cache`, `thread_template`,
  `reaper_queue`, stack free-list state.
* **Dependencies — why.** Scheduler (block/sleep/wake/setrun/
  `assert_wait`/`update_priority`) because thread state transitions are
  the scheduler's; IPC (`ipc_thread_*`, `mach_msg_*`) because threads
  own reply ports and RPC state; VM/alloc (`kmem_cache`, `vm_deallocate`
  for stacks); arch (`pcb_*`, `stack_attach/detach`,
  `thread_get/setstatus`, `splsched`); MIG server entries.
* **Blockers.** L1 `Thread` mirror (including the
  `state:16/wake_active:1/active:1` union with `event_key` — no Rust
  bitfields, so expose the word with typed accessors), L2 locks/percpu/
  spl, L4 scheduler primitives, L6 pcb/status asm.
* **Boundary / notes.** The template copy at `:388` cannot be `Copy`;
  use field-wise init. `thread_info` copies `system_time64 =
  user_time` (`:1492`) — existing behaviour, keep unless deliberately
  fixed.  Document the lock order (`processor.h:120-157`) in the module
  and encode the `thread_dowait` state machine (`:1208-1264`) as a
  closed enum, not ad-hoc bit tests.

#### `kern/task.c` — 1408 lines — friction 5/5
* **Role.** Task object: create/fork, refcounts, terminate, suspend/
  resume, info, pset assignment, names/priorities, resource collection;
  also the `task_*` MIG server bodies.
* **Exports/data.** `task_init`, `task_create[_kernel]`,
  `task_deallocate/reference`, `task_terminate`, `task_hold[_locked]`,
  `task_dowait`, `task_release`, `task_threads`,
  `task_suspend/resume`, `task_info`, `task_assign[_default]`,
  `task_get_assignment`, `task_priority/max_priority`, `task_set_name`,
  `task_set_essential`, `task_ras_control`,
  `register_new_task_notification`, `consider_task_collect`; owns
  `kernel_task`, `task_cache`, `new_task_notification`.
* **Dependencies — why.** Thread primitives because a task is a thread
  container and termination waits on/holds every thread; IPC because it
  owns `itk_space` and the task port; VM because it owns `map`;
  pmap (`pmap_create/destroy/collect`) because a task owns a pmap;
  pset because it is a pset member (`pset_tasks` link); allocator for
  the slab and the temp arrays; EML; arch `machine_task_*`.
* **Blockers.** L1 `Task`/`TaskIpc`/`machine_task` layout (embedded arch
  `iopb_lock`, `i386/i386/task.h:30-41`), L2 locks/percpu, L3 shims,
  L4 thread wait, L5 space/map handles.  MIG pins twelve signatures;
  `task_priority`/`task_get_assignment`/`task_set_essential` have no
  prototype in `task.h`.
* **Boundary / notes.** `task_create_kernel:96-212` and
  `task_terminate:267-451` are the state machines to preserve: publish
  last on create; on terminate excise the current thread first
  (`:313`), lock two tasks in address order (`:331-338`), never block
  with `task->lock`/`pset->lock` held, and reinsert the self thread last
  (`:440-448`).  Reference transfers (e.g. `convert_thread_to_port`
  takes a ref) are conventions, not types — keep them in shims.  First
  slices: `task_ras_control`, `task_set_name`, `task_set_essential`,
  `task_get_assignment`, `task_priority`.

#### `kern/sched_prim.c` — 1912 lines — friction 5/5 — partly ported
* **Role.** Scheduler core: wait-event hash, wakeup/clear-wait, run
  queues, `thread_invoke`/`thread_block`/`thread_run`, priority/aging,
  idle and scheduler threads, stuck-thread scan.
* **Ported so far.** The wait/wake primitives (`thread_timeout`,
  `thread_timeout_setup`, `assert_wait`, `clear_wait`,
  `thread_wakeup_prim`, `thread_sleep`), `thread_dispatch` and
  `thread_setrun`, with `wait_hash` byte-identical.  The state
  transitions are copied line for line except in `clear_wait()`,
  whose wake path owns the `TH_RUN | TH_WAIT` transition and enqueues
  the thread itself, so a bypassed resumer dispatch cannot strand it;
  `thread_setrun` leaves a thread that is already scheduled alone and
  a stale `thread_dispatch` is a no-op.  The `struct thread` mirror is
  full (`src/kern/thread.rs`), as are the processor and run-queue
  mirrors (`src/kern/processor.rs`) and the `%gs` accessors
  (`src/arch/i386/percpu.rs`).  `wait_queue`/`wait_lock` stay C and are
  reached as externs (`wait_lock` lost its `static`); `state_panic`
  stays a C macro for `thread_invoke`, and Rust has its own copy.
* **Exports/data.** ~35: `sched_init`, `assert_wait`, `clear_wait`,
  `thread_sleep`, `thread_wakeup_prim`, `thread_invoke`,
  `thread_block`, `thread_run`, `thread_set_timeout`, `thread_setrun`,
  `thread_dispatch`, `thread_continue`, `thread_bind`,
  `compute_priority`/`compute_my_priority`, `recompute_priorities`,
  `update_priority`, `set_pri`, `choose_thread`, `rem_runq`,
  `idle_thread`, `sched_thread`; owns `min_quantum`, `sched_tick`,
  `wait_queue[1031]`, `wait_shift[32]`, `recompute_priorities_timer`,
  `stuck_threads[]`.
* **Dependencies — why.** It *is* the dependency for most files: sleep/
  wake/block, run-queue selection and priority.  Below it are only
  timeout primitives (`set_timeout`/`reset_timeout`), pset/processor
  bootstrap, arch context-switch entry points (`switch_context`,
  `call_continuation`, `stack_handoff`, `machine_idle`,
  `MARK_CPU_IDLE/ACTIVE`) and per-CPU/spl.
* **Blockers (the rest).** L6 context-switch and continuation ABI;
  `thread_invoke`'s swap hand-off and `machine_idle` paths.
* **Boundary / notes.** The idle loop's `volatile` reads of
  `next_thread`/`runq.count` (`:1526-1527`) must stay `read_volatile`;
  `wait_shift` arithmetic wraps and `wait_hash` must be identical or
  wakeups miss; `sched_tick` is read unlocked by design.  Context-switch
  symbols stay C/asm; Rust can own the pure priority computation before
  the wait/run machinery.

#### `kern/ipc_mig.c` — 1019 lines — friction 5/5
* **Role.** Kernel-side MIG client runtime (`mach_msg`,
  `mach_msg_send/rpc_from_kernel`, `mig_*` helpers), fast port-name
  lookups, and the 15 `syscall_*`/`thread_set_self_state` mach traps.
* **Exports.** 24: the traps plus `mach_msg_send_from_kernel`,
  `mach_msg_rpc_from_kernel`, `mach_msg_abort_rpc`,
  `mig_get_reply_port`, `mig_put_reply_port`,
  `mig_dealloc_reply_port`, `mig_strncpy`, `mig_deallocate`,
  `mach_msg`.
* **Dependencies — why.** The whole IPC core (`ipc_kmsg_*`,
  `ipc_mqueue_*`, `ipc_object_*`, `mach_port_*`), task/thread/VM/device
  because each trap reaches one of them, and `copyin/copyout` because
  user pointers arrive raw.  `copyin_address`/`copyout_address` are
  `ipc/copy_user.h` macros; the 32-bit `rpc_vm_offset_t` ABI holds on
  x86_64.
* **Blockers.** L5 types and a safe copyin/out boundary; trap ABI
  (`mach_trap_table` order, `MACH_SEND_INTERRUPTED` fallback semantics).
* **Boundary / notes.** Port the five `port_name_to_*` helpers as one
  shared fallible lookup first, then read-only traps
  (`syscall_vm_deallocate`, `syscall_task_suspend`); leave
  copyin-heavy traps (`vm_map`, `task_create`, `device_write*`,
  `thread_set_self_state`) last.  `mach_msg_rpc_from_kernel` panics by
  design (`:97-101`); keep that.

#### `kern/exception.c` — 974 lines — friction 5/5
* **Role.** The exception up-call: choose thread/task exception port,
  synthesize an `exception_raise` kmsg, run the RPC with handoff, parse
  the reply and resume or kill the thread.
* **Exports/data.** `exception`, `exception_try_task`,
  `exception_no_server`, `exception_raise`, `exception_parse_reply`,
  `exception_raise_continue[_slow/_fast]`; owns the four
  `mach_msg_type_t` prototypes and `exception_raise_misses`.
* **Dependencies — why.** IPC kmsg/port/mqueue/entry because it sends
  a typed message and receives the reply; thread/sched because it
  hands off the thread and resumes it; the message body is hand-built
  to match `mach/exc.defs` (reply id 2500) on both word sizes.
* **Blockers.** L5 `Kmsg`/`Port`/`Mqueue` types and continuations;
  arch exception-return path stays C.
* **Boundary / notes.** `exception_parse_reply` is self-contained and
  the natural first slice; then the raise chain.  Do not reorder locks
  or simplify the `abort_copyout` paths (`:531-572`); preserve
  `MACH_RCV_TOO_LARGE`/`MACH_RCV_BODY_ERROR` and the
  `exception_raise_misses` symbol; `ip_srights`/`ip_sorights` accounting
  is exact.

## 5. Outside `kern/`

Every `.c` in the rest of the tree, with the same friction scale.
MIG-generated `.c` live only under `build-*/` and are not ported.

### vm/ (11 files, 17,819 LOC)

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `memory_object_proxy.c` | 227 | proxy port for memory objects | 3 | `mach4.server.h`, ports/slab |
| `vm_debug.c` | 541 | `mach_vm_*` info server routines | 3 | MIG-S, map/object walks |
| `vm_pageout.c` | 505 | page daemon | 4 | MIG-U, `thread_block`, pmap |
| `vm_user.c` | 882 | VM user/server entry points | 4 | MIG-S, map/kern |
| `memory_object.c` | 1079 | pager protocol core | 4 | MIG-S/U, pmap, locks |
| `vm_fault.c` | 2060 | page-fault resolution | 5 | pmap, MIG-U, scheduler |
| `vm_kern.c` | 1112 | kernel map, `kmem_alloc` | 5 | `kernel_map`, pmap, kalloc |
| `vm_map.c` | 5241 | address-space map (anchor) | 5 | ported — see §9 |
| `vm_object.c` | 2887 | VM objects/pagers (anchor) | 5 | lock/refcount, pager ports, pmap |
| `vm_page.c` | 2214 | page allocation/queues | 5 | pmap, percpu, page lock |
| `vm_resident.c` | 1071 | resident page table/free lists | 5 | pmap, queues, slab |

The `vm_map` port is complete (M1--M6b); `vm/vm_map.c` is gone and
§9 records it.  `rust/src/vm/vm_map.rs` mirrors
`vm_map_links`, `vm_map_entry`, `vm_map_header`, `vm_map`,
`vm_map_version`, `vm_map_copy` (all three variants) and
`vm_map_copyin_args_data` `#[repr(C)]`, with size, alignment and
field offsets pinned by `const` assertions; the structs C names get
paired `_Static_assert`s in `vm/vm_map.h`, while the copy variants
and `vm_map_version` are pinned Rust-side only.  The bitfield words
are one `u32` each with named bit constants, and `projected_on` has a
`Projection` view.  `rust/src/kern/list.rs` mirrors `kern/list.h` and
`rust/src/kern/lock.rs` adds `SimpleLock` (the `struct slock` word)
and the `LockData` layout; `VmProt` moved to `rust/src/vm/types.rs`,
where `VmInherit` also lives.

Eleven exported routines have moved to the native core in
`rust/src/vm/vm_map.rs`, behind the adapters of
`rust/src/vm/vm_map_ffi.rs`: `vm_map_setup`, `vm_map_create`,
`vm_map_lock`, `vm_map_unlock`, `vm_map_copy_limits`,
`vm_map_reference`, `vm_map_deallocate`, `vm_map_lookup_entry`,
`vm_map_verify`, `vm_map_machine_attribute` and `vm_map_msync`.  The
C definitions are deleted.  `vm/vm_map_glue.c` carries the two shims
Rust cannot reach (`current_thread()->vm_privilege`, the
`pmap_attribute` macro) and `rust/src/kern/rbtree.rs` gained the
`init`/`lookup_nearest` methods the map uses.  `tests/test-vm.c` pins
the machine-attribute bounds check and the msync flag/rounding
behavior.

M2 adds `vm_map_find_entry` and, behind it, the Rust-native gap
machinery and entry lifecycle: `vm_map_gap_*`, `_vm_map_entry_link`,
`_vm_map_entry_create`, `vm_map_enforce_limit` and
`vm_map_find_entry_anywhere` are methods in `vm/vm_map.rs` now, with
the entry tree through `Rbtree::insert_by`/`lookup_slot`/`lookup_nearest`
and the gap lists through `List`.  The same statics stay in
`vm/vm_map.c` for `vm_map_enter` until M4; the exported
`vm_map_find_entry` no longer exists there.

M3 adds the deletion and protection core: `_vm_map_clip_start`,
`_vm_map_clip_end`, `vm_map_entry_delete`, `vm_map_delete`,
`vm_map_remove`, `vm_map_coalesce_entry`, `vm_map_protect`,
`vm_map_inherit`, `vm_map_pageable` and `vm_map_pageable_all` are
Rust now, with the pageability scan, the protected-range pass, and
the object cleanup through the new `vm_map_glue.c` shims
(`vm_map_glue_object_lock/unlock/can_release`, thread wakeup).
`VmMap::deallocate` calls the Rust delete directly.

M4 ports the lookup and range-editing core.  It starts with
`vm_map_lookup`: `VmMap::lookup` follows submaps, fixes up a
copy-on-write or empty entry under the upgraded write lock, and
returns the object locked with the map's timestamp.  The upgrade is
`VmMap::lock_read_to_write`, which mirrors the
`vm_map_lock_read_to_write()` macro's timestamp bump; `Error` grew
`KERN_WRITE_PROTECTION_FAILURE` for a notified write fault.
`vm_map_submap` followed, replacing a pristine `vm_submap_object`
placeholder with the subordinate map through
`vm_map_glue_object_is_pristine_submap()`, the first
`struct vm_object` probe that will die with `vm/vm_object.c`; its
`VM_MAP_RANGE_CHECK` was the macro's last C caller, so it went too.
`vm_map_pmap_enter` is Rust too: the scan owns the loop, and the
`struct vm_page` bitfields (`absent`, `busy`, `active`, `inactive`),
the `PMAP_ENTER`/`PAGE_WAKEUP_DONE` macros and the object's paging
in-progress count are one-line shims in `vm_map_glue.c` until
`vm/vm_page.c` and `vm/vm_object.c` move.  The scan's only caller is
gated by `vm_map_pmap_enter_enable`, which defaults to 0 and only a
debugger flips, so the Rust loop and its shims are build-verified but
not boot-exercised; coverage is unchanged from the C.  `vm_map_enter`
is the largest slice: `EnterRequest` carries the C's eleven arguments,
`VmMap::enter` owns the single unlock on every path, and the private
`EnterOutcome` enum is the C's `RETURN`/`BailOut`.  It drives the Rust
`find_entry_anywhere`, `enforce_limit`, `coalesce_entry`, `pageable`
and `pmap_enter`, and the two pmap-enter debugging switches moved with
it as `AtomicU32` statics that keep their C symbol names.
`find_entry_anywhere` now validates the mask after taking the lock,
closing the C path that returned unlocked to a caller whose cleanup
unlocked again.  `vm_map_fork` closes M4: it walks the old map under
its write lock, clones SHARE entries (marking both shared and the
object `use_shared_copy` through `vm_map_glue_object_make_shared`),
asks `vm_object_copy_temporary` for COPY entries, and falls back to
`vm_map_copyin`/`vm_map_copy_insert` for the rest.  Its remaining
shims are `vm_map_glue_object_needs_shadow` and
`vm_map_glue_pmap_copy` (`pmap_copy` is an empty macro on i386);
`vm_map_copy_insert` lost its `static` for the one Rust caller and
returns to internal linkage in M5.  What remains in `vm/vm_map.c` is
the copy family, `vm_region`/`vm_region_create_proxy`, the caches and
`vm_map_init`; M5/M6 follow.

M5a is the copy destruction family: `vm_map_copy_steal_pages`,
`vm_map_copy_page_discard`, `vm_map_copy_discard`,
`vm_map_copy_copy` and `vm_map_copy_discard_cont` are Rust now,
over the copy cache and the `VmMapCopy` union.
`vm_map_copy_steal_pages` was static and lived behind an adapter only
while the copyin/copyout C callers lasted; the adapter went in the
M5d2 review fix, its prototype in M5e, and the core calls
`VmMapCopy::steal_pages` directly.
`vm_map_copy_discard_cont` keeps its exact symbol and signature,
which `vm_kern.c` stores in `cpy_cont`; `vm_map_copy_discard()`
recognizes that function through `vm_map_ffi::is_discard_cont()` and
follows its chain iteratively, exactly as the C special case does.
The new `vm_map_glue.c` shims `vm_map_glue_page_is_tabled`,
`vm_map_glue_page_object` and `vm_map_glue_page_free` read the page
fields and expand `VM_PAGE_FREE`; they go when `vm/vm_page.c` moves.
The copyin, copyout and overwrite routines, `vm_region` and the
caches remain for M5b/M6.

M5b ports `vm_map_copy_overwrite`: `VmMap::copy_overwrite` walks the
destination under the map lock for a writeable, contiguous range, then
walks the copy's entries, either swapping a temporary entry's object in
place (protecting the old object out of the pmap) or copying through
`vm_fault_copy` with the map unlocked and revalidating through the
saved version.  The C forces its `interruptible` argument to `FALSE`
before use, so the port drops the argument and the permanent-object
branch it alone gated.  The one new shim,
`vm_map_glue_object_is_temporary`, reads the object's `temporary` bit
until `vm/vm_object.c` moves; `vm_map_copy_insert` stays in C and
non-static for its Rust `vm_map_copyout` and `vm_map_fork` callers, to
return to Rust with the copyin routines in M6.

M5c ports `vm_map_copyout` and its page-list half
`vm_map_copyout_page_list`.  The null copy is the adapter's; the core
dispatches three ways: an `OBJECT` copy goes through `VmMap::enter`, an
entry-list copy has its addresses adjusted and is linked through the C
`vm_map_copy_insert`, and a page-list copy runs
`VmMap::copyout_page_list`, which steals tabled pages, extends the
entry below or creates an object and an entry, and drains continuation
chains with the map, object and page-queue locks dropped around each
continuation.  When a continuation returns no copy the C passes
NULL to `kmem_cache_free`, which its slab layer cannot accept (the
free path derives a bogus slab address and writes through it); the
port skips that call.
The C's private `vm_map_enforce_limit`, `vm_map_find_entry_anywhere`,
`vm_map_gap_update` and `vm_map_entry_inc_wired`, whose last caller was
this routine, go with it.  New shims read and write `struct vm_page`
(`busy`, `dirty`, `offset`, `wire_count`, the active/inactive queue
lock) and probe `struct vm_object` (`can_coalesce`, `extend_size`);
`vm_map_glue_pmap_enter` now carries the wired flag.  They go when
`vm/vm_page.c` and `vm/vm_object.c` move.  What remains in
`vm/vm_map.c` is `vm_map_init`, `vm_map_copy_insert`, the copyin
family, `vm_region`/`vm_region_create_proxy` and the caches; M6
follows.

M5d ports the copyin half.  `vm_map_copyin` is `VmMap::copyin`: the
source map lock, the clip-and-verify loop, the temporary-object move
and copy-on-write shortcuts, the two `vm_object_copy_*` strategies and
the optional `vm_map_delete` are Rust now, with
`vm_object_copy_slowly` and `vm_object_copy_strategically` behind new
`glue` declarations and `vm_map_glue_object_use_shared_copy` the
one new `struct vm_object` probe.  The zero-length copy stays the
adapter's, which answers it with a null copy object as the C does.
`Error` gained `SendInterrupted`, because the object copy strategies
under the copyin can return `MACH_SEND_INTERRUPTED`, which the C
passed through verbatim.
`vm_map_copyin_object` is `VmMapCopy::copyin_object`.  With the last C
caller gone, `vm_map_copy_insert` is the private
`VmMap::copy_insert`; `vm_map_fork` and the entry-list copyout call it
directly, its C definition, prototype and glue declaration are gone,
and it has its original internal linkage back.  The C helpers whose
last caller was the copyin go with it:
`_vm_map_entry_create`/`_vm_map_entry_dispose`, the entry copy macros,
the entry link/unlink macros and their comparison and gap machinery.
What remains in `vm/vm_map.c` is `vm_map_init`,
`vm_map_copyin_page_list` with its continuation,
`vm_region`/`vm_region_create_proxy` and the caches; M6 follows.

M5e ports the page-list copyin.  `vm_map_copyin_page_list` is
`VmMap::copyin_page_list`, and its continuation is a Rust
`unsafe extern "C"` routine keeping the exact `vm_map_copy_cont_fn`
signature, so a copy the port builds is drained by the C
`vm_map_copy_invoke_cont`/`vm_map_copy_abort_cont` macros and by the
Rust `VmMap::copyout_page_list` alike.  The routine faults pages
through `vm_fault_page` with the map unlocked, keeps the taken pages
busy and write-protects them, and then either steals them inline
(unwiring and clipping the map entry through the already-Rust
`entry_reset_wired`) or copies the ones that remain with the
already-Rust `VmMapCopy::steal_pages`.  The C `vm_map_copy_steal_pages`
prototype, the last caller's `vm_map_entry_reset_wired` static and the
four dead `vm_map_clip_*`/`vm_map_copy_clip_*` macros go with the two
functions.  New shims read `struct vm_page` (`busy`, `fictitious`,
`error`, `precious`) and `struct vm_object` (`shadowed`) and expand
`VM_PAGE_QUEUES_REMOVE` and the page-locked `pmap_page_protect`; they
go when `vm/vm_page.c` and `vm/vm_object.c` move.  Where the C reaches
its completion check with the map already locked on the `is_cont`
memory-error path and locks it a second time, the port records the
lock it took instead of deadlocking on the write lock.  What remains
in `vm/vm_map.c` is `vm_map_init`, `vm_region`/`vm_region_create_proxy`
and the caches; M6 follows.

M5f ports the two region queries.  `vm_region` is `VmMap::region`:
the read lock, the containing-or-next entry lookup, and the fields
the C copies out, with the object name taken through
`vm_object_name()` while the map lock keeps the entry's object alive.
`vm_region_create_proxy` is `VmMap::region_create_proxy`, split into
the locked half that limits the arguments and copies the entry
pager's send right, and the unlocked half that asks
`memory_object_create_proxy` for the proxy port.  The proxy call's
`rpc_vm_*` arguments are a guarded type (`uint32_t` under USER32,
pointer-sized otherwise), so the cast stays in a `vm_map_glue.c` shim;
the same file gains the `struct task` field and `struct vm_object`
pager accessors, which die with `kern/task.c` and `vm/vm_object.c`.
The new core uses the `IpcPort`/`IpcSpace` handles of
`rust/src/ipc/`, and `Error` gained `InvalidName` and `InvalidTask`
for the two `kern_return_t`s the proxy call can pass through.  With
the C definitions deleted, what remained in `vm/vm_map.c` was
`vm_map_init` and the three caches.

M6b moves that last storage out: the three caches and
`vm_submap_object` live in `vm/vm_map_glue.c` now -- C, because
`struct kmem_cache` and `struct vm_object` are, and they move to Rust
with kern/slab.c and vm/vm_object.c.  `vm_map_init` is
`VmMap::init_module()` behind its adapter in `vm_map_ffi.rs`, with the
same three names, sizes and flags.  `vm/vm_map.c` held nothing but
comments after that -- the `SAVE_HINT` macro, which no longer had a
caller, and the `vm_map_verify`/`vm_map_verify_done` comment blocks --
and it is deleted.  `vm/vm_map.h` stays for its C readers, minus the
macros and prototypes whose last C user was that file: the
`KENTRY_DATA_SIZE`, `vm_map_last_entry`, `vm_map_copy_*_entry`,
`vm_map_lock_init`, `vm_map_lock_write_to_read`,
`vm_map_lock_read_to_write` and `vm_map_entry_wait`/`_wakeup` macros,
and the `vm_map_coalesce_entry`, `vm_map_delete` and
`vm_map_copyout_page_list` prototypes.  Their core methods stay in
Rust; M7-pre then deletes the exported adapters no C caller named
(`vm_map_delete`, `vm_map_pmap_enter`, `vm_map_coalesce_entry` and
`vm_map_copyout_page_list`).  What remains in
`vm_map_glue.c` beside the
storage is every shim the narrative above names: the
`current_thread()` privilege pair and the `pmap_attribute`/`pmap_copy`
/`thread_wakeup` macro shims, the `struct vm_object` and
`struct vm_page` probes, the `struct task` field accessors, and the
proxy cast.  Each dies with its owner, as its comment says.

M7-pre is a cleanup pass over the finished port.  The body of
`vm_map_copy_discard_cont` moves out of the FFI edge and behind
`VmMapCopy::discard_cont`, leaving the exported symbol `vm/vm_kern.c`
stores in `cpy_cont` a thin call, with `is_discard_cont` still
recognizing it by address.  The four exported adapters no C, header,
asm or test caller named -- `vm_map_delete`, `vm_map_pmap_enter`,
`vm_map_coalesce_entry` and `vm_map_copyout_page_list` -- are
deleted; their core methods stay in `vm_map.rs`, where `remove`,
`enter`, `copyout` and the rest call them directly.

M7a ports the external-page bookkeeping.  `vm/vm_external.c` is gone:
`struct vm_external` is the `VmExternal` mirror in
`rust/src/vm/vm_external.rs` (the header now names only the opaque
`vm_external_t`), the bitmap is a Rust slice behind the struct's
accessors, and the page state is the `ExternalState` enum, with the C
`int` converted at the edge.  `vm_external_create`,
`vm_external_destroy`, `_vm_external_state_get`,
`vm_external_state_set` and `vm_external_module_initialize` are exact
symbols behind adapters in that file, with the same bitmap sizes,
`atop` arithmetic, zeroing and `existence_size` checks as the C.
One deliberate divergence: every allocation failure returns a null
record where the C would fault, the header allocation included, so the
kernel gets no new panic.  `vm_object.c` and `memory_object.c` already
test for `VM_EXTERNAL_NULL` and accept the record.  The three
slab caches stay in `vm/vm_external_glue.c` until kern/slab.c
moves, and the dead `existence_count` `#if 0` block went with the
port.  Coverage is unchanged: only `vm_external_module_initialize` is
reached at boot; create/destroy and the state pair are
external-paging paths the suite does not set up.

M7b moves the bootstrap itself.  `vm/vm_init.c` is gone; the two entry
points live in `rust/src/vm/vm_init.rs` as exact symbols behind
adapters, with `kern/startup.c` unchanged.  `vm_mem_bootstrap`'s
eleven calls run in the C's order, `vm_page_bootstrap`'s two
out-parameters are locals of the Rust core, and the map module is
entered as `VmMap::init_module` directly rather than through its
C-shaped adapter.  `vm_mem_init` keeps its three calls.  Both entry
points are boot-exercised on x86_64 and i386.

### ipc/ (18 files, 13,002 LOC)

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `ipc_target.c` | 40 | target-port set init/term | 1 | one call: `ipc_mqueue_init` |
| `ipc_thread.c` | 103 | thread linkage helpers | 1 | its own header macros |
| `ipc_table.c` | 134 | space table sizing/alloc | 2 | `kalloc/kfree` only |
| `ipc_entry.c` | 187 | entry allocation | 3 | slab, rdxtree C inlines |
| `ipc_init.c` | 115 | IPC bootstrap | 3 | slab, host/port init ordering |
| `ipc_notify.c` | 448 | port-death notifications | 3 | kmsg/mqueue, ports |
| `mach_debug.c` | 286 | mach_debug server routines | 3 | MIG-S, host/vm introspection |
| `ipc_space.c` | 213 | IPC spaces | 4 | entry/table, refcounts |
| `copy_user.c` | 540 | user↔kernel field copy | 4 | LP64-only, `copyin/out`, USER32 |
| `ipc_marequest.c` | 415 | msg-accepted bookkeeping | 4 | slab, locks, notify |
| `ipc_mqueue.c` | 659 | message queues | 4 | `sched_prim`, `ipc_sched` |
| `ipc_object.c` | 852 | generic object refcounts | 4 | slab, rights/notify |
| `ipc_pset.c` | 309 | port sets | 4 | mqueue/right, space |
| `mach_port.c` | 1437 | `mach_port_*` server routines | 4 | MIG-S, rights/space, vm |
| `ipc_kmsg.c` | 2600 | kernel message buffers (anchor) | 5 | map copyin/out, slab, locks |
| `ipc_port.c` | 1172 | ports (anchor) | 5 | space/object locks, kobjects |
| `ipc_right.c` | 1844 | rights translation (anchor) | 5 | entry/space/table/marequest |
| `mach_msg.c` | 1648 | `mach_msg_trap` (anchor) | 5 | copyin/out, locore/pcb, sched |

`ipc_thread.c` is ported; §9 records it.  The entry below keeps the
detail §4.1 gives the `kern/` files.

#### `ipc/ipc_thread.c` — 103 lines — ported
* **Role.** The LIFO stack of threads waiting on a message queue or
  blocked on a port; the links are `ith_next`/`ith_prev` inside
  `struct thread`.
* **Rust home.** `src/ipc/ipc_thread.rs`: `IpcThreadQueue` with
  `init`/`first`/`enqueue`/`dequeue`/`rmqueue`/`rmqueue_first`, and
  `ThreadRef` for a thread and its links.  The queue is Rust-native and
  the C side is seven adapters.
* **Bridges.** `struct thread` is still C, so the module asks the new
  `ipc/ipc_thread_glue.c` for a view of the `ith_next`/`ith_prev` pair
  (asserted adjacent in C); `glue` declares the shim.
* **Header.** `ipc_thread.h` keeps the struct and the prototypes only:
  every macro became a function, the dead `ipc_thread_queue_empty()`
  is gone, and the 18 former-macro call sites use the functions.
* **Tests.** Qemu only: `test-machmsg`, `test-mach_port`,
  `test-syscalls`, `test-task` and `test-threads` move rights and
  messages through the queues.

### device/ (12 files, 7,580 LOC)

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `cons.c` | 176 | console dispatch | 2 | `constab`, `kmsg_putchar` |
| `subrs.c` | 85 | `ether_sprintf`, `sleep`, `wakeup` | 2 | thread primitives |
| `cirbuf.c` | 277 | circular char buffer | 2 | kalloc/kfree |
| `dev_name.c` | 242 | name/indirection tables | 2 | static tables + strcmp |
| `device_init.c` | 63 | device bring-up | 3 | kernel ports, io/net threads |
| `dev_lookup.c` | 365 | device registry | 3 | ipc kobject, slab |
| `kmsg.c` | 251 | kernel message device | 3 | lock, device server port |
| `dev_pager.c` | 629 | device pager server | 4 | MIG-S/U, `vm_page` |
| `intr.c` | 395 | user interrupt delivery | 4 | irq threads, ipc, spl |
| `chario.c` | 1060 | tty line discipline | 5 | spl, MIG-U, vm_map |
| `ds_routines.c` | 1859 | `device_*` server routines | 5 | MIG-S/U, spl, vm |
| `net_io.c` | 2178 | network filter/IPC | 5 | spl, kmsg/mqueue, sched |

### i386/i386/ (22 files, 5,709 LOC — arch-shared i686/x86_64)

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `ast_check.c` | 52 | AST IPI dispatch | 2 | one callee `smp_remote_ast` |
| `hardclock.c` | 69 | tick | 2 | `clock_interrupt`, trap return |
| `irq.c` | 95 | IRQ ack/enable | 2 | ioapic EOI, spl |
| `pit.c` | 144 | 8254 timer | 2 | `splon/sploff`, hz |
| `db_interface.c` | 103 | debug-register access | 3 | `%dbN` asm, percpu |
| `debug_i386.c` | 178 | trace/print debug | 3 | trap frames, console |
| `idt.c` | 80 | IDT construction | 3 | gate tables, gdt |
| `io_perm.c` | 325 | I/O permission bitmap | 3 | MIG-S, pcb/gdt |
| `ktss.c` | 86 | per-CPU TSS | 3 | GDT slots, seg.h |
| `ldt.c` | 100 | LDT management | 3 | `lldt` asm, gdt/pmap |
| `machine_task.c` | 80 | task iopb hooks | 3 | `kmem_cache_*`, lock |
| `pic.c` | 270 | 8259 PIC | 3 | `cli` asm, spl, pio |
| `apic.c` | 501 | local APIC | 4 | lapic MMIO, kalloc, idt |
| `fpu.c` | 859 | FPU save/restore | 4 | inline asm, trap, percpu |
| `gdt.c` | 141 | per-CPU GDT | 4 | `ljmp` asm, percpu |
| `mp_desc.c` | 329 | SMP per-CPU descriptors | 4 | lapic/idt/gdt, pmap |
| `percpu.c` | 31 | per-CPU base init (anchor) | 4 | `%gs` layout, apic |
| `phys.c` | 179 | physical address access | 4 | pmap |
| `smp.c` | 214 | AP bring-up | 4 | `wbinvd`, lapic/ioapic |
| `user_ldt.c` | 422 | i386 LDT server routines | 4 | MIG-S, kalloc, pcb |
| `pcb.c` | 919 | PCB/context (anchor) | 5 | `switch_context` asm, fpu |
| `trap.c` | 532 | trap entry bodies (anchor) | 5 | `alltraps`/`all_intrs` |

### i386/i386at/ (13 files, 4,395 LOC)

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `mem.c` | 42 | `/dev/mem` mmap hook | 1 | `biosmem_addr_available` |
| `mbinfo.c` | 49 | multiboot info device | 2 | `device_read_alloc` |
| `cons_conf.c` | 48 | console table | 2 | `constab` entries |
| `autoconf.c` | 127 | bus probe/attach | 2 | bus tables, spl |
| `conf.c` | 144 | driver switch tables | 3 | kd/com/mem wiring |
| `rtc.c` | 242 | CMOS clock | 3 | spl, pio, mach_clock |
| `biosmem.c` | 1027 | boot memory/direct map | 3 | multiboot, VM boot |
| `acpi_parse_apic.c` | 651 | ACPI MADT parser | 3 | acpi tables, kernel VM |
| `int_init.c` | 78 | IDT gate fill | 3 | asm stubs |
| `ioapic.c` | 493 | IOAPIC | 3 | `cli` asm, irq routing |
| `pic_isa.c` | 56 | ISA IRQ tables | 3 | pic/ipl |
| `com.c` | 893 | 8250 serial | 4 | tty, spl, pio |
| `model_dep.c` | 545 | machine init/bootstrap (anchor) | 5 | asm, pmap, percpu |

`kd_queue.c`, `kd_event.c`, `kd_mouse.c`, `kd.c`, `mem.c` and
`mbinfo.c` are ported; §9 records them.  The entries below keep the
detail §4.1 gives the `kern/` files.

#### `i386/i386at/mbinfo.c` — 49 lines — ported
* **Role.** `/dev/mbinfo`: the boot path hands the multiboot
  information block to `mbinfo_register_boot_data()`, and
  `mbinforead()` serves it back raw.
* **Rust home.** `src/arch/i386/mbinfo.rs`, shared by both x86
  kernels.  `struct multiboot_raw_info` is mirrored `#[repr(C,
  packed)]` with its size asserted; the register function and
  `mbinforead()` keep their names and signatures, and `mbinfo.h` stays
  for `conf.c` and `model_dep.c`.
* **Shared cell.** `SyncCell` moved to `utils/cell.rs`; the kd driver
  and mbinfo both use it.
* **Tests.** `tests/test-mbinfo.c` opens `/dev/mbinfo`, reads the
  block, checks the loader memory flag and `mem_upper`, and that a
  count larger than the block is refused.

#### `i386/i386at/mem.c` — 42 lines — ported
* **Role.** `/dev/mem`: the mmap hook hands out pages of memory that
  is not main RAM, so a user can look at the BIOS areas, the VGA
  window and the like.
* **Rust home.** `src/arch/i386/mem.rs`, shared by both x86 kernels.
  `memmmap()` keeps its name and signature; `mem.h` stays as the C
  declaration `conf.c`'s device switch sees.
* **Bridges.** `biosmem_addr_available()` comes through `glue`;
  `i386_btop()` is the shift by `I386_PGSHIFT` that `vm_param.h`
  defines.
* **Tests.** No new test: `tests/test-kd-dev.c` opens `/dev/mem` and
  maps the VGA text through it, so the mmap path runs on both arches.

#### `i386/i386at/kd_queue.c` — 109 lines — ported
* **Rust home.** `src/utils/kd_queue.rs`, shared by both x86 kernels.
  A `#[repr(C)]` mirror of `kd_event` and `kd_event_queue` with size and
  offset asserts; the embedded `rpc_time_value` comes from
  `src/glue/time_value.rs`, whose `c_long` mirrors `rpc_long_integer_t`,
  so only the default configuration is covered (`--enable-user32` makes
  the C field 32 bits, which Rust cannot see).
* **Boundary.** Pure Rust now: the drivers use its safe `clear`/
  `push_back`/`pop_front`/`is_empty`/`is_full` operations.  The five
  `kdq_*` `extern "C"` wrappers and `kd_queue.h` went when `kd_event.c`
  stopped being their last C caller.  `tests/kd_queue.c` and
  `tests/test-kd-queue.c` pin the contract, since the suite never opens
  `/dev/kbd`.

#### `i386/i386at/kd_mouse.c` — 799 lines — ported
* **Role.** `/dev/mouse`: the Mouse Systems 5-byte, Microsoft and
  Logitech 3-byte and IBM PS/2 3-byte protocols, on COM1 or the
  keyboard controller, decoded into `kd_event`s and queued.
* **Rust home.** `src/arch/i386/kd_mouse.rs`, shared by both x86
  kernels.  Every name in `kd_mouse.h` is unchanged, and `kd.c`'s two
  direct uses (`mouse_in_use`, `mouse_handle_byte()`) stay exported.
* **Bridges.** `glue` declares the plain C functions: `splhi`/
  `spltty`/`splx` (asm functions, not macros), `printf`, `wakeup`,
  `assert_wait`, `thread_block`, `iodone`, `device_read_alloc`,
  `ds_read_done`, `comgetc`, `kd_sendcmd`, `kd_cmdreg_write`,
  `kd_mouse_drain`, `kdintr`.  The macros and config-shaped data got C
  shims instead: `pio_inb`/`pio_outb` in a new `i386/i386/pio_glue.c`
  over the `inb`/`outb` statement expressions; `irq_mask`/`irq_unmask`
  and `ivect`/`iunit` accessors added to `i386/i386/irq.c` (`mask_irq`
  is inline under APIC and the arrays are `NINTR`-sized); and
  `com_base_addr`/`com_irq` added to `i386/i386at/com.c` for the
  `NCOM`-sized `cominfo`.  `minor()` and `printf_once` are Rust-side.
* **`io_req_t`.** The four device entry points read a `#[repr(C)]`
  prefix mirror of `struct io_req` (`IoReq`) through `io_done`; the two
  leading chain fields are asserted, and are what let an IO request
  double as its own `queue_entry_t` for the read queue.
* **Notes.** `STATE` holds the old globals, including the event queue
  and the `mouse_read_queue` head, which is self-linked on first use
  where C linked it at compile time.  `kd_queue`'s API was widened with
  constructors and public methods, with no layout or ABI change.
  `tests/kd_mouse.c` and `tests/test-kd-mouse.c` pin the decoders,
  since the suite never opens `/dev/mouse`.

#### `i386/i386at/kd_event.c` — 392 lines — ported
* **Role.** `/dev/kbd`: `kd.c` calls `kd_enqsc()` for every scan code,
  the events queue up, and `kbdread()` drains them; the same file
  carries the `X_kdb` port-command escape that `kd.c`'s `cnpollc()`
  replays.
* **Rust home.** `src/arch/i386/kd_event.rs`.  Every name in
  `kd_event.h` stays a symbol: `conf.c`'s four device entries and
  `kd.c`'s `X_kdb_enter()`/`X_kdb_exit()`/`kd_enqsc()` calls are
  unchanged.
* **Shared pieces.** The `IoReq` prefix mirror, the request drain and
  the device return codes moved to `src/arch/i386/io_req.rs`, whose
  future home is a `src/device/` module.  `pio_glue.c` gained the
  16/32-bit shims the `X_kdb` interpreter needs; `kb_mode` moved into
  the Rust kd module, so its `kbd_set_mode()` shim is gone.
* **Notes.** The ioctl flavors are mirrored as computed values;
  `K_X_KDB_ENTER`/`EXIT` differ per target because their ioctl length
  field carries `sizeof(struct X_kdb)`.  The C bound
  `count * sizeof > sizeof` overflows for a huge `count`, so the port
  and its test use `count > 512` instead.  `STATE` holds the two
  command lists and the queue; the read-queue head self-links on first
  use.  `tests/kd_event.c` and `tests/test-kd-event.c` pin the
  interpreter, since the suite never opens `/dev/kbd`.

#### `i386/i386at/kd.c` — 3033 lines — ported
* **Role.** The keyboard/VGA console: the scan-code interrupt and
  modifier state machine, the escape parser that draws the console,
  the EGA text and bitmap display backends, the console entry points,
  the key map and the tty device entry points.
* **Rust home.** `src/arch/i386/kd/` split by role: `keyboard.rs`,
  `esc.rs`, `display.rs`, `console.rs`, `tty.rs` (the `struct tty`
  mirror and kdopen/close/read/write, get/set status, mmap,
  portdeath, kdstart), `keymap.rs` (the 89-row map, generated from the
  C table) and `mod.rs` (state and `kdinit()`).
* **The tty wall.** `tty.rs` mirrors `struct tty` `#[repr(C)]` field
  for field, with the offsets pinned per target (`t_lock`, `t_inq`,
  `t_outq`, `t_state`, `t_line`, the delayed queues, `t_timeout` and
  the size); `kd_tty` is Rust storage now, and `ttychars()` initializes
  its queues.  The lock macros, the `linesw[]` switch, `ttlowat[]` and
  `phystokv()` are the shims in the new `i386/i386at/kd_glue.c`;
  `char_open`/`ttychars`/`ttyclose`/`tty_get_status`/`tty_set_status`/
  `tty_portdeath`/`tty_queue_completion`/`getc` and the `hz`/
  `rebootflag` data come through `glue`.
* **`kd.c` is gone.** The file, its Makefrag entries and the five
  shims it briefly hosted (`kd_tty_rint`, `kd_tty_init`, `kd_phystokv`,
  `kd_rebootflag`, `kd_hz`) are deleted.
* **Narrow boundary (cleanup).** Only 23 symbols stay
  `#[no_mangle] extern "C"`: the 14 kd entries (8 conf.c device hooks,
  4 console hooks, `kdintr`, `kdreboot`), the 5 kbd entries and the 4
  mouse entries; `cnpollc` stays exported too because
  `i386/i386/db_interface.h` declares it.  Everything else is
  `pub(crate)` Rust, the Rust-to-Rust glue declarations are gone, and
  `kd.h`/`kd_mouse.h`/`kd_event.h` no longer declare the retired
  symbols.
* **Idioms (cleanup).** Named scancode/controller constants, `bool`
  and `usize` internal returns, device return codes shared from
  `io_req.rs`, `Option<NonNull<_>>` for the tty's inert pointers, and
  private names that read as Rust (`cn_set_leds`, `char_to_bit`,
  `fb_ptr`, `motion`, ...).
* **State (cleanup).** The driver's globals live in one `Kd` object
  behind a `SyncCell<UnsafeCell<_>>` singleton, `kd()`; `state()`,
  `kb_mode()`/`set_kb_mode()`, the modifier bits and `mouse_in_use()`
  are accessors on it.  No `static mut` remains in the kd family.
* **Backend (cleanup).** `display.rs` calls the EGA text routines
  directly.  The bitmap backend and `kdsoft.h`'s function-pointer
  table were never selected by the ported `kdinit()` and had no C
  callers, so they, the header and its Makefrag entries are gone.
* **Parameters (deliberate change).** `esc.rs` parses its `\e[...]`
  numbers itself with `core::str::parse::<c_int>()` over the leading
  digit run; a parameter too large for an `int` counts as absent
  instead of wrapping into a repeat count, an SGR attribute or a
  row/column as `mach_atoi()`'s accumulator did.  `test-kd-dev` pins
  the reset with an 11-digit parameter.
* **Escape buffer (fix).** `esc_seq` is `K_MAXESC + 1` bytes.  The C
  sized the array for the sequence bytes and wrote the NUL terminator
  one past it; the port turned that write into a Rust bounds panic,
  so a 32-byte sequence halted the kernel.  The extra byte carries the
  terminator, and the byte after a full buffer is dropped and the
  parser starts over, as the C's guard intended.  `tests/kd.c`'s copy
  got the same byte, and `test-kd-dev` writes a full sequence.
* **Tests.** `tests/kd.c` and `tests/test-kd.c` pin the escape parser
  (command dispatch, positions, attributes, a full buffer) and the
  modifier state machine.  `tests/test-kd-dev.c` drives the driver
  through its device: it opens `/dev/kd` (running `kdinit()`, the
  display and the tty setup), sets the keyboard mode and key map,
  writes an escape sequence -- one that fills the buffer, a zero
  parameter and an unsupported private sequence -- and reads the VGA
  text back through `/dev/mem`, maps the kd bitmap,
  and checks `/dev/kbd`'s record size against the `KdEvent` mirror;
  the qemu suite runs it on both arches.  `tests/test-kd-intr.c`
  goes one step further: the runner waits for its ready marker, injects
  a keystroke through a qemu monitor socket (`tests/hmp_send.c`), and
  the test reads the resulting scan codes back from `/dev/kbd`, so
  `kdintr()` and the event queue are executed too.

### i386/intel/, x86_64/, util/, chips/

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `util/atoi.c` | 106 | `mach_atoi` | 1 | ported — see §9 |
| `i386/intel/read_fault.c` | 178 | pre-486 workaround | 1 | dead on i686/x86_64 — delete |
| `chips/busses.c` | 232 | bus config tables | 2 | `bus_*_init` hooks |
| `i386/intel/pmap.c` | 2599 | x86 page tables (anchor) | 5 | everything; see below |
| `x86_64/` | — | **no C at all** | — | only `.S` + headers |

#### `util/atoi.c` — 106 lines — ported
* **Role.** Parse the leading decimal digits of a byte string; the C
  interface stores the number -- or `MACH_ATOI_DEFAULT` when there is
  none -- and returns the bytes consumed.
* **Rust home.** `src/utils/atoi.rs`: a safe `parse()` over `&[u8]`
  returning `(usize, Option<c_int>)`, with the C accumulator's
  wrapping, and the `mach_atoi()` adapter at the edge.
* **Callers.** `i386/i386at/com.c` parses the `console=com<n>` unit
  through the adapter; `src/arch/i386/kd/esc.rs` parses its
  escape-sequence parameters with `core`'s integer parser instead.
* **Tests.** Qemu only: boot parses `console=com0`, and
  `test-kd-dev` drives the escape parser on both arches.  `util/atoi.c`
  was compiled into the user tests; their frozen copies live in the
  `abi-test/` modules now.

### Hard anchors outside `kern/`

* `i386/intel/pmap.c` — Rust `PhysAddr`/`VirtAddr` newtypes, a `Pmap`
  with interior locking, TLB shootdown tied to `smp_remote_ast`, and the
  `kernel_pmap` global.  Everything in `vm/` and `device/` calls it.
* `i386/i386/trap.c` + `locore.S` (`alltraps`, `all_intrs`) — the C/asm
  user-transition boundary; implies `#[repr(C)]` trap frames exported to
  asm and Rust-visible exception dispatch.
* `i386/i386/pcb.c` + `cswitch.S` — context switch and FPU save;
  implies a Rust context structure with exact asm layout and
  `Switch_context` as an `extern "C"` callee.
* `i386/i386/percpu.c` + `percpu.h` — the `%gs` base of
  `current_thread()`, `current_map()`, `cpu_number()`.  Until a
  `PerCpu<T>` layer exists, every L2 consumer stays C.
* `i386/i386/spl.S` + `spl.h`/`ipl.h` — interrupt masking used by ~30
  files; implies RAII `Spl<Level>` guards over the asm symbols.
* `ipc/mach_msg.c`, `ipc/ipc_kmsg.c`, `ipc/ipc_port.c`,
  `ipc/ipc_right.c`, `vm/vm_object.c`,
  `vm/vm_page.c`, `vm/vm_resident.c` — the IPC/VM cores; they gate the
  `kern/` IPC files and most of `device/`.
* `i386/i386at/model_dep.c` — the boot order and `c_boot_entry`; the
  Rust entry contract.

### The MIG story in one paragraph

Source specs are server `.srv` (`device/device.srv`,
`device/device_pager.srv`, `ipc/mach_port.srv`,
`i386/i386/mach_i386.srv`, `kern/{mach,mach4,gnumach,experimental,
mach_debug,mach_host}.srv`), client `.cli` (`device/device_reply.cli`,
`device/memory_object_reply.cli`, `kern/task_notify.cli`,
`vm/memory_object_default.cli`, `vm/memory_object_user.cli`) and
msgid-only `.defs` (`kern/exc.defs`, `ipc/notify.defs`).
`Makerules.mig.am:74-98` pipes them through `MIGCOM` into
`*.server.{h,c}` and `*.user.{h,c}` under `build-*/`.  The generated
code calls hand-written definitions: `ipc/mach_port.c`,
`device/ds_routines.c`, `device/dev_pager.c`, `vm/vm_user.c`,
`rust/src/vm/vm_map_ffi.rs`, `vm/memory_object.c`, `vm/vm_debug.c`,
`ipc/mach_debug.c`, `i386/i386/{io_perm,user_ldt}.c`, and the trap
bodies in `kern/ipc_mig.c` and `ipc/mach_msg.c`.  A Rust port replaces
exactly one hand-written definition, with the exact prototype from the
generated `.server.h`; the unmarshalling, `TypeCheck` and
`*_server_routines[]` table remain C.

## 6. Least-friction candidates, in order

Ported from this list so far: `kern/rbtree.c`, `i386/i386at/kd_queue.c`,
`i386/i386at/mem.c`, `i386/i386at/mbinfo.c`, `ipc/ipc_thread.c` and
`util/atoi.c` (see §9).

Tier 1 — no new infrastructure:

1. `ipc/ipc_target.c` — one call to `ipc_mqueue_init` (shim or defer).
2. `i386/i386/ast_check.c`, `i386/i386/hardclock.c` — tiny, asm-free.
3. `kern/boot_script.c` — isolated, allocation callbacks only.

Tier 2 — after the first shims (percpu, locks, `struct` mirrors):

5. `kern/timer.c` — needs `cpu_number` accessor only.
6. `kern/kmutex.c`, `kern/mach_factor.c`, `kern/thread_swap.c` — need
    the lock/sleep layer.
7. `kern/syscall_sw.c` — the trap table can move once entry layout is
    `#[repr(C)]`; the routines it names need not have moved.
8. `device/cirbuf.c`, `device/dev_name.c`, `chips/busses.c`,
    `ipc/ipc_table.c`, `i386/i386/pit.c`,
    `i386/i386/irq.c`, `i386/i386/machine_task.c` — small, one or two
    leaf dependencies.

Tier 3 — the lock/allocator/IPC layers (`rdxtree`, `slab`, `lock`,
`eventcount`, `priority`, `ipc_tt`, `ipc_host`, `host`).

Tier 4 — the anchors (`thread`, `task`, `sched_prim`, `ipc_mig`,
`exception`, `mach_clock`, `startup`, `bootstrap`, `printf` engine,
`pmap`, `trap`, `pcb`, `ipc_kmsg`, `mach_msg`).

## 7. Recommended phasing

* **Phase 0 (now).** Tier-1 files.  No new infrastructure; each port
  establishes only its own module.  Add the first C shim when a Tier-1
  file needs a macro (none should).
* **Phase 1 — the Rust platform.**  In one coherent push, add
  `src/arch/<arch>/` with per-CPU access and spl guards; a
  `SpinLock`/`IrqLock` `repr(transparent)` over the C `lock_data` word;
  `glue` declarations for `thread_sleep`/`thread_wakeup`,
  `kalloc`/`kmem_cache`, and `copyin`/`copyout`; and the `#[repr(C)]`
  mirror pattern with compile-time asserts for the first shared struct.
* **Phase 2 — infrastructure.**  `rbtree` (ported), `timer`,
  `kmutex`, then `slab`/`kalloc` shims, then `rdxtree` → `lock.c` →
  `eventcount.c`.
* **Phase 3 — scheduler surface.**  `mach_factor`, `thread_swap`,
  `priority`, `syscall_sw`, then the pure priority computation inside
  `sched_prim.c`, then `ipc_sched.c`'s `thread_go`/`will_wait`, then
  `ast.c`.  Keep `switch_context`, `call_continuation` and
  `stack_handoff` C.
* **Phase 4 — objects.**  `ipc_tt`/`ipc_host`/`host` conversions →
  `processor`/`machine` → `task` → `thread` (the state machines last).
* **Phase 5 — IPC/VM/arch anchors.**  `ipc_kmsg`, `ipc_port`,
  `mach_msg`, `vm_page`, then `pmap`, `trap`, `pcb`,
  `startup`/`bootstrap`, then `printf`'s engine and the drivers.

Exit criterion for every step is unchanged: both qemu architectures
green, `rustfmt`/`clippy` clean, no new undefined symbols.  A module
that needs nothing from the kernel may add host tests like the
rbtree's; see §8.

## 8. Deletions and test-side copies

* Dead code found in this audit: `i386/intel/read_fault.c` (body is
  `#if (__i386__ && !(__i486__ || __i586__ || __i686__))`, compiled out
  on every supported CPU) and the `#if 0` blocks in
  `kern/{boot_script,bootstrap,exception,ipc_kobject}.c`,
  `device/intr.c`, `i386/i386/{fpu,smp,pcb,trap}.c`,
  `i386/i386at/{kd,com}.c`, `i386/intel/pmap.c`.  Delete before porting
  the surrounding code.  `rdxtree.h:49` has an `#if 0` block to check
  the same way.  `kern/boot_script.c`'s
  `boot_script_define_function` has no callers.
* Host-side Rust tests: `rust/src/kern/rbtree.rs` is free of kernel
  calls and `crate::` imports, so it carries `#[cfg(test)]` tests, but
  the host runner went away with the old suite and nothing compiles
  them today.  It is the exception, not a second build path; every
  other port is exercised through the running kernel.
* Test-linked routines: the user tests are the frozen binaries in
  `abi-test/`; they link their own copies of `kern/printf.c`,
  `util/atoi.c`, `i386/i386/strings.c`, `kern/strings.c` and the
  i386at keyboard files, so a port in this tree changes no test copy.

## 9. Already moved (for reference)

| C file | Rust | Commit |
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
| `vm/vm_external.c` | `src/vm/vm_external.rs` | `727275e7` |
| `vm/vm_init.c` | `src/vm/vm_init.rs` | `727275e7` |
| `vm/vm_map.c` | `src/vm/vm_map.rs`, `src/vm/vm_map_ffi.rs` | `d32c7253` … `170e6104` |
| `kern/lock.c` | `src/kern/lock.rs` | `9a9ced86` |
| `i386/i386/lock.h` (bit ops) | `src/arch/i386/atomic_bits.rs` | `f0a3cb2c` |
| `i386/i386/lock.h` (simple lock) | `src/kern/lock.rs` | `102c4926` |
| `kern/sched_prim.c` (wait/wake, `thread_dispatch`, `thread_setrun`) | `src/kern/sched_prim.rs` + `src/kern/thread.rs`, `src/kern/timer.rs`, `src/kern/processor.rs`, `src/arch/i386/percpu.rs` | `c4498541` |
| `kern/ast.h` (`ast_on`, `ast_off`, `ast_needed`) | `src/kern/ast.rs` | `6a6281be` |
| `kern/thread.c` (`thread_init`) | `src/kern/thread.rs` | `pending` |
| `kern/sched.h` (`thread_timer_delta`) | `src/kern/thread.rs`, `src/kern/timer.rs` | `pending` |

Deleted dead code: `device/blkio.c` (unreachable block pager path) and
the `#if 0` profiling facility (`profil.h`, `profilparam.h`,
`mpqueue`).
