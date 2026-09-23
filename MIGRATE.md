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

**`AGENTS.md`'s no-glue law governs this file.**  No port may write C,
so the order below is not a preference, it is the constraint.  Where a
file needs a macro, a lock or a field accessor that only C can reach,
the file that *defines* that thing is the next port and the dependent
file waits.  Every "Blockers" line below names a port to do first, not
a shim to write.  The glue already in the tree is pre-rule debt; §10
catalogues it and says what deletes each piece.

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
   Memory comes from the caller, or from the kernel allocators
   (`kalloc`, `kmem_cache_*`) declared in `glue` as the real C symbols
   they are.
4. clippy `-D warnings`, `rustfmt --check` at 79 columns and
   `--enable-queue-debug` are build gates (`rust/Makefrag.am:96-107`).
5. `mise run test` builds both kernels and runs the frozen ABI pack in
   `abi-test/` against them; it is the only correctness gate.  The pack
   is self-contained, so there is no in-tree user-test source to keep
   in sync.
6. `rust/src/` mirrors the C tree: `src/utils/`, `src/kern/`,
   `src/arch/<arch>/`, with the C functions Rust calls declared
   verbatim in `src/glue/`.  That declaration writes no C and is the
   whole permitted bridge inward.
7. **No new C.**  A C *macro* cannot be declared in `glue` and may not
   be wrapped in a shim, so the file that defines it is ported first.
   A port that cannot be done without writing C is not next; another
   one is.

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
| **L1 types** | `struct thread`, `task`, `processor`, `processor_set`, `ipc_port`, `vm_map` read/written field-by-field, sometimes by asm (`i386asm.sym`) | `#[repr(C)]` mirror + offset/size `const` asserts, and a decision on who owns the layout. Not a C accessor: a field Rust cannot name means the layout is not mirrored yet, and mirroring it is the work | everything in `kern/` |
| **L2 locks/IRQ/percpu** | `simple_lock`/`_simple_lock` (thin macros over the Rust entry points), `spl*` (`spl.S`, per-CPU `curr_ipl`), `simple_lock_irq`, `percpu_get`/`current_thread()` (`%gs`), `__sync_synchronize`, `cpu_pause` | A `SpinLock` type `repr(transparent)` over `natural_t` so C macros keep working; an `IrqGuard` over `splx` (the `spl*` entry points are real asm functions, so `glue` declares them); a per-CPU accessor in `src/arch/` — `percpu_get` and `current_thread()` are macros, so the accessor is Rust's own and its arrival is what unblocks the layer | `kmutex.c`, `eventcount.c`, `priority.c`, `timer.c`, scheduler/IPC/VM files |
| **L3 memory** | `kalloc`/`kfree`, `kmem_cache_*` (slab), `kmem_alloc_wired`, `vm_page_*` | nothing new: they are real symbols `glue` declares and Rust calls directly. Optionally later a `GlobalAlloc` over `kalloc` (an explicit design decision, not a quiet add) | `slab.c` itself, `rdxtree.c`, `syscall_emulation.c`, `processor.c`, `task.c` |
| **L4 runnable** | `thread_block`, `thread_wakeup`, `assert_wait`, `thread_setrun`, continuations (`extern "C" fn()` passed across `switch_context`), `set_timeout` | Rust `Thread`/`Task` mirror with locked accessors and a continuation type. The wait/wake primitives are Rust already (`src/kern/sched_prim.rs`), so `thread_wakeup*`'s macro wrapper is bypassed by calling `thread_wakeup_prim` directly, not shimmed | `ipc_sched.c`, `eventcount.c`, `syscall_subr.c`, `thread_swap.c`, `task.c` |
| **L5 IPC/VM** | `ipc_port`/`ipc_space`/`ipc_kmsg`/`vm_map` with `simple_lock` embedded and refcounts by convention; `copyin`/`copyout`; MIG wire formats | Rust `Port`/`Space`/`Kmsg`/`VmMap` mirrors asserted against the C layout, or opaque handles whose owner moves in the same step; a safe copyin/copyout wrapper for slices | `exception.c`, `ipc_kobject.c`, `ipc_tt.c`, `ipc_mig.c`, `vm/*`, `device/*` |
| **L6 arch/MIG** | `switch_context`/`call_continuation`/`stack_handoff`, `pmap`, trap entry in `locore.S`, `mach_trap_table`, MIG-generated `_X*` unmarshallers | `src/arch/<arch>/` with `core::arch::asm!`; `#[repr(C)]` trap/exec frames; acceptance that MIG and trap dispatch stay C | scheduler, exception, syscall, boot files |

The layers are an ordering, not a menu.  Under the no-glue law a file
in layer N is unportable until layer N-1 has a Rust definition, and
"unportable" is a fine answer: pick another file.  The layer that is
holding the most files is the one worth porting next, which is why §7
spends its early phases on L2 rather than on whatever looks small.

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
* **Macros are an ordering constraint, never a shim.**  A C macro
  cannot be declared in `glue`, and under the no-glue law it cannot be
  wrapped either, so whatever defines it is ported first.  The five
  that come up constantly, and their current state:

  | Macro | Owner | State |
  |---|---|---|
  | `simple_lock`, `simple_unlock`, `simple_lock_irq` | `kern/lock.h` | partly Rust (`src/kern/lock.rs`, `src/arch/i386/atomic_bits.rs`; `i386/i386/lock.h` is gone); finishing it is Phase 2 |
  | `spl*` | `i386/i386/spl.S` | **not a macro** — real asm functions, declared in `glue` already |
  | `percpu_get`, `current_thread()`, `cpu_number()` | `i386/i386/percpu.h`, `i386/i386/cpu_number.h`, `kern/thread.h` | Rust accessor started in `src/arch/i386/percpu.rs`; Phase 1 |
  | `thread_wakeup*` | `kern/sched_prim.h` | wrapper over `thread_wakeup_prim`, which is Rust — call it directly |
  | queue ops | `kern/queue.h` | Rust since `5fcbebe5`; the macros are gone |

  `glue` declares `Panic` and the plain C functions the ports call
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

What Rust still lacks, and therefore what the next ports are: an RAII
lock/IRQ layer over the finished `src/kern/lock.rs`, a complete per-CPU
accessor in `src/arch/`, the configure-time constants (`NCPUS`,
`NINTR`, `NCOM`) that C arrays are sized by, and a struct binding
strategy beyond hand-written mirrors (no bindgen by design).  An
allocator over `kalloc`/`kmem_cache` is a design conversation, not a
prerequisite: `kalloc` is a real symbol Rust can already call.  §4's
blockers name which missing piece each file needs, and each is a port
to do, not a shim to write.

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
| `thread_swap.c` | 196 | 14 | L4 | **2** | `src/kern/thread_swap.rs` — ported |
| `mach_factor.c` | 150 | 6 | L1+L4 | **2** | `src/kern/mach_factor.rs` |
| `kmutex.c` | 75 | 2 | L2+L4 | **3** | `src/kern/kmutex.rs` — ported |
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
| `printf.c` | 656 | 3 | L2 | **5** | blocked: see the entry |
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

#### `kern/timer.c` — 236 lines — partly ported
* **Role.** Per-thread and per-CPU statistical timers (microseconds and
  seconds) with a seqlock-style read (`high_bits_check`) tolerant of
  concurrent normalization.
* **Rust home.** `src/kern/timer.rs`: the port moved `timer_init`,
  `timer_normalize`, `timer_grab` (private), `timer_delta` and
  `timer_read`, the first two and the last two behind their same-named
  adapters.  `timer_grab` is `static` in C, so it could not stay behind;
  `timer_read` came along because it is the other caller, and leaving
  it C would have needed that static helper exported, which the no-glue
  law forbids.
* **Exports/data.** Still C: `init_timers`, `thread_read_times`,
  `db_thread_read_times`, and the debugger's `db_timer_grab` and
  `nonblocking_timer_read` (both `static`); owns `current_timer[NCPUS]`
  and `kernel_timer[NCPUS]` (`timer.c:39-40`).  `thread_read_times`
  keeps calling `timer_read` through `timer.h:96`, unchanged;
  `db_thread_read_times` stays on its own `nonblocking_timer_read`
  path.
* **Dependencies — why.** `cpu_number()` is the `percpu_get` macro over
  `%gs` (`i386/i386/cpu_number.h:54`), needed to index the per-CPU
  arrays; `__sync_synchronize()` became `fence(SeqCst)` in the Rust
  `grab`/`normalize` pair, which keeps the check-first, high-last
  publish order.  `timer_bump` is a macro in `timer.h:105` that
  `mach_clock.c` applies directly to `struct timer` fields, so the
  fields must stay C-visible.  `TIMER_DELTA` moved to
  `src/kern/timer.rs` as `TimerSave::delta`, which now calls the Rust
  `delta` helper for its coherency slow path instead of
  `glue::timer_delta`.
* **Blockers.** `init_timers` alone: it indexes `current_timer[NCPUS]`,
  so the per-CPU accessor in `src/arch/` (Phase 1) is still the
  prerequisite.
* **Boundary / notes.** `#[no_mangle] static mut` arrays with the same
  size/alignment; write order in `timer_normalize` (check first, high
  last) and `fence(SeqCst)` must match — a safe `Timer` API can exist
  internally, but the C macros keep poking the fields until
  `thread.c`/`mach_clock.c` move.

#### `kern/thread_swap.c` — 196 lines — ported
* **Role.** The swapin queue and the swapper kernel thread: allocate a
  kernel stack for a swapped-out thread and put it back on a run queue.
* **Rust home.** `src/kern/thread_swap.rs`.  `swapin_queue` keeps its
  exact C symbol and its `QueueEntry` type, because it is both the
  queue head and the wakeup event; `swapper_lock_data` is a private
  `SimpleLock`.  The four exported routines keep their symbols and
  signatures, and the continuation stays private:
  `swapper_init` heads the queue and initializes the lock,
  `thread_swapin` switches on `TH_SWAP_STATE` and enqueues the
  thread's `links` at the tail under the swapper lock before waking
  through the Rust `thread_wakeup_prim` the C macro expands to, and
  `thread_doswapin` is a thin adapter over a private `doswapin` that
  calls `stack_alloc(thread, thread_continue)` and then takes
  splsched, locks the thread, clears `TH_SWAPPED|TH_SW_COMING_IN` and
  runs `thread_setrun` when `TH_RUN` survives.  The private noreturn
  continuation alternates `doswapin` (which may block) with the
  spl/queue protocol, re-enqueues at the head on failure, and blocks
  through `assert_wait` and `thread_block`.  Every `unsafe` block
  carries its own `// SAFETY:` note.
* **Boundary / notes.** `stack_alloc`, `stack_privilege` and
  `thread_continue` are the only new `glue` declarations; `splsched`,
  `splx` and `thread_block` were already there, and `assert_wait`,
  `thread_setrun`, `thread_wakeup_prim`, `current_thread` and the
  `SimpleLock` are Rust.  No C was added.  The queue operations use
  the pinned `QueueEntry` API, not the C-shaped queue adapters.
* **Tests.** Qemu only: `swapper_init()` and `thread_doswapin()` run
  on every boot (`kern/startup.c:129,160,202` and `kern/thread.c:1567`);
  `thread_swapin()`/`swapin_thread()` need a real swap and are not
  reached by the pack.

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

#### `kern/kmutex.c` — 75 lines — ported
* **Role.** Three-state sleepable mutex (unowned/locked/contended) with
  a lock-free fast path.
* **Rust home.** `src/kern/kmutex.rs`.  `KMutex` keeps the C layout
  (`AtomicU32` state at 0, `SimpleLock` interlock at 4; size, alignment
  and both offsets asserted), and the three states are a private
  `repr(u32)` `State` enum.  `try_lock` is the C
  `atomic_cas_acq(AVAIL, LOCKED)`; `lock` adds the interlock, the
  `swap(CONTENDED)` recheck and the `thread_sleep` on the mutex's own
  address; `unlock` is the `atomic_cas_rel(LOCKED, AVAIL)` fast path
  before taking the interlock and `thread_wakeup_prim`.  The
  `interruptible` argument crosses as a `c_int` and becomes a `bool`
  inside the adapter.  Every `unsafe` block carries its own `// SAFETY:`
  note.
* **Exports.** `kmutex_init`, `kmutex_lock`, `kmutex_trylock`,
  `kmutex_unlock`; only caller is `gsync.c`, which never touches `state`
  or `lock` directly, so `kern/kmutex.h` keeps the record and the
  constants for the 512 hash buckets that embed it.
* **Boundary / notes.** The C `atomic_*` macros become
  `compare_exchange`/`swap` with the same orderings; the plain
  `state = KMUTEX_AVAIL` under the interlock is a `Relaxed` store,
  since the interlock carries the ordering.  No owner tracking exists
  in C and none was added — gsync depends on lock ordering at the call
  site.  A null mutex, which C would fault on, reports
  `KERN_INVALID_ARGUMENT` from the integer-returning entries and is
  ignored by the two void ones.
* **Tests.** Qemu only: `test-gsync` drives the mutex through
  `gsync_wait`/`gsync_wake`; `kmutex_init` runs at boot from
  `gsync_setup` (`kern/startup.c:141`).
* **No-glue precedent.** The port added no C: every dependency was
  already a real symbol or already Rust.  It is the worked example of
  §7's ordering, and the model for `eventcount.c` next.

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
* **Blockers.** None that need new C: `kmem_cache_alloc`/`_free` are
  real symbols `glue` can declare, and the `rbtree` dependency is
  ported.  Porting `kern/slab.c` first is cleaner but is not required.
  The inline header API stays C, calling the Rust `_common` symbols.
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
  `ast_on/off/context` macros in C (`net_io.c:541`, `fpu.c:845`,
  `trap.c:484`, `task.c:741`, `ipc_sched.c:221`).
* **Dependencies — why.** Scheduler (`csw_needed`, `thread_block`,
  `thread_halt_self`, `thread_exception_return`) because the AST handles
  thread halt/block at the user boundary; arch (`spl*`, `cpu_number`,
  `processor_ptr`) because it runs at IRQ level; `net_ast` for the
  network AST.  AST bits themselves are arch (`i386/i386/ast.h:38`).
* **Blockers.** L1 thread/processor mirrors, L2 percpu+IRQ, the exported
  `need_ast` global.  `cause_ast_check`/`init_ast_check` are ported in
  `src/arch/i386/ast_check.rs`.
* **Boundary / notes.** Define `need_ast` as a `#[no_mangle] static mut
  [usize; NCPUS]` (or atomics), but C macros keep writing it; do not
  reorder or make it private.  `ast_taken` must clear `need_ast` before
  `spl0()` exactly as `ast.c:75-77` does.

#### `kern/debug.c` — 146 lines — partly ported
* **Role.** `Panic`, soft debugger stubs, `log`, stack-canary support.
* **Rust home.** `src/kern/debug.rs`: the port moved `SoftDebugger`,
  `Debugger` and `panic_init`, each behind its same-named adapter.
  `panic_lock` is that module's `SimpleLock` static, exported as the
  C symbol `panic_lock`; `def_simple_lock_irq_data`'s `struct
  slock_irq` is layout-identical to a bare `struct slock`, so
  `kern/debug.c` now carries `decl_simple_lock_irq_data(extern,
  panic_lock)` and the C `Panic()` keeps taking the same object with
  `simple_lock_irq()`.
* **Exports/data.** Still C: `Panic`, `log`, `do_cnputc` (static),
  `panicstr`, `paniccpu`, `__stack_chk_guard`, `__stack_chk_fail`.
  `Panic` and `log` stay C because they are variadic (question 4).
* **Dependencies — why.** `printf`/`_doprnt` for the varargs formatter,
  `cnputc` for the console, `halt_cpu`/`halt_all_cpus` for the halt
  path, `delay` (now Rust), `cpu_number`.  Locks are
  `simple_lock_irq`/`simple_unlock_irq` (`debug.c:70-99`).
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
* **Blockers.** L1 pset/processor mirrors and the arch machine table
  accessor; MIG signatures frozen.  `kalloc` is a real symbol and needs
  no shim.
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
* **Blockers.** The L6 ABI: `struct
  eml_dispatch` offsets are baked into `i386asm.sym:69-73` and read by
  `i386/locore.S:680-689`, `x86_64/locore.S:841-850`.  Do not move the
  struct to Rust without regenerating matching offsets.
* **Boundary / notes.** `#[repr(C)]` struct with trailing
  `disp_vector[1]`; keep the `count_to_size` power-of-two allocation and
  the lock-protected allocate/race protocol (`:168-270`) exactly.
  `kalloc` is callable from Rust today, so the L3 side is not what
  holds this file up.

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
* **Ported so far.** `host_reboot` is `src/kern/machine.rs` now, its
  adapter keeping the <mach/mach_host.defs> prototype; the file stays
  C for the rest.
* **Exports/data.** `cpu_up`, `processor_assign`,
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
* **Ported so far.** `processor_init`, `pset_init`, `processor_start`,
  `processor_exit`, `processor_control`, `processor_get_assignment`,
  `processor_info`, `processor_set_info`,
  `pset_add_processor`/`pset_remove_processor`/`quantum_set`,
  `pset_add_thread`/`pset_remove_thread`/`thread_change_psets`,
  `processor_set_max_priority` and
  `processor_set_policy_enable`/`processor_set_policy_disable`, plus the
  `MACH_HOST` branch of `pset_reference`/`pset_deallocate`, are in
  `src/kern/processor.rs`.  The NCPUS-sized pset tail
  (`machine_quantum` through `sched_load`) is still reached through
  `kern/processor_glue.c`, which is pre-rule debt (§10): bringing
  `NCPUS` into Rust lets the `ProcessorSet` mirror carry the tail and
  deletes the file.  The rest of `processor.c` stays C.
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
  `kalloc` for the temporary port arrays; the `cpu_control` hook is
  `src/arch/i386/mp_desc.rs` now.
* **Blockers.** L1 pset/processor/runq layout, L2 locks, L5 port
  conversions.  No asm in the file.
* **Boundary / notes.** `processor_set_tasks/threads` build raw arrays
  with `kalloc` and convert in place — call `kalloc` through `glue` and
  keep the raw arrays, no slices.  Refcount restoration in
  `pset_deallocate` (`:342-402`) is lock-order sensitive.  `master_cpu`
  is written once and read widely.

#### `kern/printf.c` — 656 lines — blocked (friction 5/5)
* **Role.** The whole console formatting engine (`printnum`, `_doprnt`,
  `printf`/`iprintf`, `sprintf`/`snprintf`/`vsnprintf`, `safe_gets`,
  the `%b` bit-field format).
* **Exports.** The above plus `printnum`, `indent`, `vprintf`.
* **Dependencies — why.** `cnputc`/`cngetc` (console), `strlen`;
  three output sinks through function pointers.  ~223 call sites; the
  frozen test modules in `abi-test/` link their own copy, so a port in
  this tree touches only the kernel side.
* **Blockers.** Defining a C-variadic function needs the unstable
  `c_variadic` feature; on the pinned rustc 1.98.1 it is still
  `error[E0658]`.  So `printf`, `vprintf`, `_doprnt`, `sprintf` and
  `Panic` have no Rust definition available at all.
* **Verdict under the no-glue law: blocked, not partially portable.**
  The old plan here was to port the engine and keep the variadic entry
  points as C shims calling it.  That is exactly the glue the law
  forbids, and it would also leave two formatting implementations in
  the tree.  The file moves whole or not at all, and moving it whole
  needs one of two decisions, both "ask first" in `AGENTS.md`:
  enabling `c_variadic` (the build already sets `RUSTC_BOOTSTRAP=1`,
  so the knob exists), or changing the ~223 call sites off the
  variadic ABI.  Until one is taken, `printf.c` stays C in full and no
  part of it is ported.
* **Boundary / notes.** When it does move: a `core`-only formatter over
  `&mut dyn FnMut(char)` in `src/kern/printf.rs` under the variadic
  entry points, preserving `%b` (`:257-314`) and the truncation flag
  exactly.  Rust-side printing keeps going through the `printf` symbol
  declared in `glue` in the meantime.

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
* **Blockers.** Almost everything: L6
  user-stack/`set_user_regs`/`thread_bootstrap_return` asm, dual
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
* **Blockers.** `vm_page_*` must be callable, which it is (real
  symbols), and the lock wrapper of Phase 2.
  Metadata is pointer arithmetic in caller memory — needs raw pointers
  and deliberate bounds, not slices.
* **Boundary / notes.** `cache->lock` must be dropped before
  `kmem_slab_create` and emptiness revalidated (`:411,734-736`);
  `cache->ctor` callbacks stay C.  This is the file that puts the
  allocator in Rust; port it after `vm_page`'s layout is mirrored and
  before the larger consumers.

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
  `iopb_lock`, `i386/i386/task.h:30-41`), L2 locks/percpu,
  L4 thread wait, L5 space/map handles.  MIG pins twelve signatures;
  `task_priority`/`task_get_assignment`/`task_set_essential` have no
  prototype in `task.h`.
* **Boundary / notes.** `task_create_kernel:96-212` and
  `task_terminate:267-451` are the state machines to preserve: publish
  last on create; on terminate excise the current thread first
  (`:313`), lock two tasks in address order (`:331-338`), never block
  with `task->lock`/`pset->lock` held, and reinsert the self thread last
  (`:440-448`).  Reference transfers (e.g. `convert_thread_to_port`
  takes a ref) are conventions, not types — carry them in the Rust
  adapter's contract, documented, since there is no C to put them in.
  First
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
storage is every shim the narrative above names — all of it pre-rule
debt now, catalogued in §10 and closed by Phases 1, 4 and 5 rather
than by anything new: the
`current_thread()` privilege pair and the `pmap_attribute`/`pmap_copy`
/`thread_wakeup` macro shims, the `struct vm_object` and
`struct vm_page` probes, the `struct task` field accessors, and the
proxy cast.  Each dies with its owner, as its comment says, and no
more may be added: a routine still in C here moves when its struct
mirror does, not when a shim is written for it.

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
moves (§10), and the dead `existence_count` `#if 0` block went with the
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
| `ipc_table.c` | 134 | space table sizing/alloc | 2 | ported; see §9 |
| `ipc_entry.c` | 187 | entry allocation | 3 | slab, rdxtree C inlines |
| `ipc_init.c` | 115 | IPC bootstrap | 3 | slab, host/port init ordering |
| `ipc_notify.c` | 448 | port-death notifications | 3 | kmsg/mqueue, ports |
| `mach_debug.c` | 286 | mach_debug server routines | 3 | MIG-S, host/vm introspection |
| `ipc_space.c` | 213 | IPC spaces | 4 | entry/table, refcounts |
| `copy_user.c` | 540 | user↔kernel field copy | 4 | LP64-only, `copyin/out`, USER32 |
| `ipc_marequest.c` | 415 | msg-accepted bookkeeping | 4 | slab, locks, notify |
| `ipc_mqueue.c` | 659 | message queues | 4 | `sched_prim`, `ipc_sched` |
| `ipc_object.c` | 852 | generic object refcounts | 4 | slab, rights/notify; `ipc_object_copyin_type` is ported, the rest stays C |
| `ipc_pset.c` | 309 | port sets | 4 | mqueue/right, space |
| `mach_port.c` | 1437 | `mach_port_*` server routines | 4 | MIG-S, rights/space, vm; `mach_port_rename`, `mach_port_insert_right`, `mach_port_extract_right` and `mach_port_request_notification` are ported, the rest stays C |
| `ipc_kmsg.c` | 2600 | kernel message buffers (anchor) | 5 | map copyin/out, slab, locks |
| `ipc_port.c` | 1172 | ports (anchor) | 5 | space/object locks, kobjects; `ipc_port_timestamp` and its two globals are ported, the rest stays C |
| `ipc_right.c` | 1844 | rights translation (anchor) | 5 | entry/space/table/marequest |
| `mach_msg.c` | 1648 | `mach_msg_trap` (anchor) | 5 | copyin/out, locore/pcb, sched |

`ipc_table.c` and `ipc_thread.c` are ported; §9 records them.  The
entry below keeps the detail §4.1 gives the `kern/` files.

#### `ipc/ipc_thread.c` — 103 lines — ported
* **Role.** The LIFO stack of threads waiting on a message queue or
  blocked on a port; the links are `ith_next`/`ith_prev` inside
  `struct thread`.
* **Rust home.** `src/ipc/ipc_thread.rs`: `IpcThreadQueue` with
  `init`/`first`/`enqueue`/`dequeue`/`rmqueue`/`rmqueue_first`, and
  `ThreadRef` for a thread and its links.  The queue is Rust-native and
  the C side is seven adapters.
* **Bridges.** The module reads the `ith_next`/`ith_prev` pair from
  the `Thread` mirror (`rust/src/kern/thread.rs:264`); the former
  `ipc/ipc_thread_glue.c` view is gone.
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
| `subrs.c` | 85 | `ether_sprintf`, `sleep`, `wakeup` | 2 | three ported (see §9); `if_init_queues` stays C |
| `cirbuf.c` | 277 | circular char buffer | 2 | ported; see §9 |
| `dev_name.c` | 242 | name/indirection tables | 2 | `name_equal` and the eleven `nulldev_*`/`nodev_*`/`nomap` stubs ported (§9); `dev_name_lookup`/`dev_set_indirection` stay C, over the tables and `strcmp` |
| `device_init.c` | 63 | device bring-up | 3 | kernel ports, io/net threads |
| `dev_lookup.c` | 365 | device registry | 3 | ipc kobject, slab |
| `kmsg.c` | 251 | kernel message device | 3 | lock, device server port |
| `dev_pager.c` | 629 | device pager server | 4 | MIG-S/U, `vm_page` |
| `intr.c` | 395 | user interrupt delivery | 4 | irq threads, ipc, spl |
| `chario.c` | 1060 | tty line discipline | 5 | spl, MIG-U, vm_map |
| `ds_routines.c` | 1859 | `device_*` server routines | 5 | MIG-S/U, spl, vm |
| `net_io.c` | 2178 | network filter/IPC; `bpf_hash` ported (§9), rest stays C | 5 | spl, kmsg/mqueue, sched |

### i386/i386/ (22 files, 5,698 LOC — arch-shared i686/x86_64)

| File | LOC | Role | Friction | Blockers |
|---|---:|---|---:|---|
| `ast_check.c` | 52 | AST IPI dispatch | 2 | ported; see §9 |
| `hardclock.c` | 69 | tick | 2 | `clock_interrupt`, trap return |
| `irq.c` | 95 | IRQ ack/enable | 2 | ioapic EOI, spl |
| `pit.c` | 144 | 8254 timer | 2 | ported; see §9 |
| `db_interface.c` | 103 | debug-register access | 3 | `%dbN` asm, percpu |
| `debug_i386.c` | 178 | trace/print debug | 3 | trap frames, console |
| `idt.c` | 80 | IDT construction | 3 | gate tables, gdt |
| `io_perm.c` | 325 | I/O permission bitmap | 3 | MIG-S, pcb/gdt |
| `ktss.c` | 86 | per-CPU TSS | 3 | GDT slots, seg.h |
| `ldt.c` | 100 | LDT management | 3 | `lldt` asm, gdt/pmap |
| `machine_task.c` | 80 | task iopb hooks | 3 | `kmem_cache_*`, lock |
| `pic.c` | 270 | 8259 PIC | 3 | `cli` asm, spl |
| `apic.c` | 501 | local APIC | 4 | lapic MMIO, kalloc, idt |
| `fpu.c` | 848 | FPU save/restore | 4 | `fp_free` ported, rest C; inline asm, trap, percpu |
| `gdt.c` | 141 | per-CPU GDT | 4 | `ljmp` asm, percpu |
| `mp_desc.c` | 329 | SMP per-CPU descriptors | 4 | `cpu_control`/`simple_lock_pause` ported, rest C; lapic/idt/gdt, pmap |
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
| `rtc.c` | 242 | CMOS clock | 3 | ported; see §9 |
| `biosmem.c` | 1027 | boot memory/direct map | 3 | multiboot, VM boot |
| `acpi_parse_apic.c` | 651 | ACPI MADT parser | 3 | acpi tables, kernel VM |
| `int_init.c` | 78 | IDT gate fill | 3 | asm stubs |
| `ioapic.c` | 493 | IOAPIC | 3 | `cli` asm, irq routing |
| `pic_isa.c` | 56 | ISA IRQ tables | 3 | pic/ipl |
| `com.c` | 893 | 8250 serial | 4 | tty, spl |
| `model_dep.c` | 545 | machine init/bootstrap (anchor) | 5 | asm, pmap, percpu |

`kd_queue.c`, `kd_event.c`, `kd_mouse.c`, `kd.c`, `mem.c`, `mbinfo.c`
and `rtc.c` are ported; §9 records them.  The entries below keep the
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
  `spltty`/`splx` (asm functions, not macros), `printf`, `thread_block`,
  `iodone`, `device_read_alloc`,
  `ds_read_done`, `comgetc`, `kd_sendcmd`, `kd_cmdreg_write`,
  `kd_mouse_drain`, `kdintr`.  The wait and wake primitives go to
  Rust: `assert_wait`, `thread_wakeup_prim` and the `subrs::wakeup`
  wrapper are Rust.  Port I/O is `src/arch/i386/pio.rs` now; the
  `pio_glue.c` shims it replaced were deleted with that port.  The
  remaining macros and config-shaped
  data got C shims instead — pre-rule debt now, and §10 says what
  deletes each: `irq_mask`/`irq_unmask`
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
* **Shared pieces.** The `IoReq` prefix mirror and the request drain
  live in `src/arch/i386/io_req.rs`; the device return codes are
  `src/device/return.rs`.  The 16/32-bit port access the
  `X_kdb` interpreter needs is `src/arch/i386/pio.rs`; `kb_mode` moved
  into the Rust kd module, so its `kbd_set_mode()` shim is gone.
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
  and `usize` internal returns, device return codes from
  `src/device/return.rs`, `Option<NonNull<_>>` for the tty's inert
  pointers, and
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

## 6. What to port next — the objective test

Choosing work is a seven-question test, not a judgement call.  Apply
it to a single C **function**, not to a file; a file is ready when all
of its functions pass.  Questions 1 to 5 ask what the function does;
6 and 7 ask whether it can be replaced at all, and §6.3 answers both
from the built objects rather than the source.

| # | Question | If the answer is "no" |
|---|---|---|
| 1 | Is every function it calls a real linker symbol, rather than a `#define` or a `static inline`? | Port the definer first, or pick another function.  A shim is not available (`AGENTS.md`, the no-glue law). |
| 2 | Is every struct field it touches covered by a Rust mirror that exists **today**?  `sizeof(struct X)` counts as a field: a size Rust cannot compute is the same dependency by another route. | Mirror that struct first: §7 Phase 4. |
| 3 | Does it avoid every array sized by a configure-time constant (`NCPUS`, `NINTR`, `NCOM`, `NIPL`)? | Blocked on §7 Phase 3. |
| 4 | Is it non-variadic, and free of `va_list`? | Blocked.  See the `kern/printf.c` entry in §4. |
| 5 | Is its inline assembly, if any, expressible with `core::arch::asm!`? | Blocked on the arch layer. |
| 6 | Is it visible outside its own translation unit — non-`static`, with a prototype in a header? | Its callers move with it, or it waits.  A Rust definition of a `static` C function is unreachable, and adding the `extern` declaration that would reach it is writing C. |
| 7 | Is the definition live in a buildable configuration, rather than a dead `#if` branch? | It is a deletion, not a port.  §8. |

These exemptions are settled, and are not re-decided per port:

* **Locks pass question 1.**  `simple_lock`/`simple_unlock`/
  `simple_lock_try` expand to `mach_simple_lock`/`mach_simple_unlock`/
  `mach_simple_lock_try` (`kern/lock.h:171-186`), which Rust defines
  (`rust/src/kern/lock.rs:774`).  `simple_lock_init` is a plain field
  write over the mirrored `SimpleLock`, and `simple_lock_irq` is
  `splhigh()` plus `mach_simple_lock`, both real.
* **`spl*` passes question 1.**  Every one is a real asm function
  (`i386/i386/spl.h:35-69`), not a macro.
* **`current_thread()`, `cpu_number()` and `percpu_get` pass.**  They
  are macros, but `rust/src/arch/i386/percpu.rs` is the Rust
  equivalent, so Rust never invokes the macro.
* **`thread_wakeup*` passes.**  The macro expands to
  `thread_wakeup_prim`, which is Rust already
  (`rust/src/kern/sched_prim.rs:504`); Rust calls it directly with the
  literal arguments the macro would have supplied.
* **`inb`/`outb` pass question 5.**  They are statement-expression
  macros, but port I/O is one instruction and `asm!` emits it
  directly, as `percpu.rs` already does.
* **A C global of unmirrored type passes question 2 when only its
  address is used.**  Declare it in `glue` as an opaque `extern`
  static and take `&raw mut`: that writes no C and needs no layout.
  `rust/src/kern/sched_prim.rs` already does it with
  `glue::wait_queue`.  Reading a *field* of one is a different thing
  and still fails.

Six exemptions, then, and the tiers below are just the number of "no"
answers.

Questions 6 and 7 were added after the second list: eleven of its
thirteen refusals failed one of them, and every one of those eleven
had passed questions 1 to 5.  Neither is about what a function does,
which is why a reader looking only at bodies misses both.

### 6.1 Tier 0 — current status

**Zero "no" answers: needs nothing that does not exist today.**  No new
C, no new mirror, no new constant, no design conversation.  Tier 0 is
worked to exhaustion before any infrastructure is proposed
(`AGENTS.md`, "Take the free ports first").

Tier 0 has three parts, and the difference between them matters more
than the count.  **Confirmed** is verified by hand and ready to port.
**Gated** is free but waits on one decision that is not the porter's.
**Candidates** came out of the scan and are not yet checked; §6.3 says
why the scan cannot finish the job.

#### Confirmed — port these now

Eight functions, each read against the C, its callees' linkage and the
mirrors that exist.

| Function | Why it is free |
|---|---|
| `i386/i386at/model_dep.c:187 machine_idle` | one `asm volatile ("hlt")`; question 5 |
| `i386/i386at/model_dep.c:192 machine_relax` | one `asm volatile ("rep; nop")`; question 5 |
| `i386/i386/pcb.c:387 pcb_collect` | empty body |
| `i386/i386/apic.c:498 hpclock_get_counter_period_nsec` | returns `hpet_period_nsec`, a `uint32_t` global read as a scalar |
| `device/intr.c:26 irqgetstat` | a `switch` over the flavour, two out-parameters, and the `pic_mode` global.  Its `D_SUCCESS`/`D_INVALID_OPERATION` are now `DeviceError` (`rust/src/device/return.rs`), so it lands in the type the tree already has |
| `i386/intel/pmap.c:2276 pmap_clear_modify`, `:2288 pmap_is_modified`, `:2299 pmap_clear_reference`, `:2311 pmap_is_referenced` | one call each to `phys_attribute_clear`/`_test`, both externally visible, plus a plain constant |

#### Gated — free, but one decision first

| Function | The decision |
|---|---|
| `i386/i386/trap.c:106 trap_name` | `trap_type[]` has a second reader in the same file, so moving the table changes the kernel-trap report for an unknown vector from `Kernel trap 42` to `Kernel (unknown) trap`.  Observable behaviour, so it wants its own commit under the narrow exception in `AGENTS.md`, or the second reader moves with it |
| `i386/i386/pcb.c:894 user_stack_low` | `VM_MAX_USER_ADDRESS` takes a third value on a 64-bit kernel built `--enable-user32`, so Rust needs a `--cfg user32` plumbed into `AM_RUSTFLAGS`.  That is an "ask first" build change |

#### Candidates — derived, not verified

The scan leaves about a hundred functions that pass every question it
can decide mechanically.  **They are a shortlist to check, not a list
to port.**  Two questions the scan cannot answer are exactly the ones
that sink most candidates:

* **Question 2, struct fields.**  The scan sees `->` and stops there.
  It cannot see `.` access, a struct passed by value, or a global of
  unmirrored type.
* **Question 3, configure-sized arrays.**  The size is in the
  declaration, not the body.  `i386/i386at/com.c:703 fix_modem_state`
  reads clean and indexes `commodem[NCOM]` (`com.c:75`); the four
  `irq_*` accessors in `i386/i386/irq.c` index `ivect`/`iunit`, both
  `NINTR`-sized.  Both look free to any body-only reader.

So the candidate pool is where to look next, in this order: the
remaining `i386/intel/pmap.c` leaves, `device/intr.c` and
`device/kmsg.c`'s device entries, and the `i386/i386/apic.c`
accessors.  Check each against all seven questions before porting it.

#### What emptying the list twice has taught

The list has been emptied twice and refilled twice.  The first time it
looked like completion; it was the limit of a sampling method.  The
second time eleven of thirteen refusals were functions this file had
called free, which is where questions 6 and 7 came from.  An empty
Tier 0 is a statement about the last derivation, never about the tree.

### 6.2 The five ways a candidate fails

Every rejection so far falls into one of five classes, and each is a
concrete piece of work rather than a vague difficulty.  The class is
also the fix.

1. **A configure-time array** (question 3).  `init_timers`,
   `thread_quantum_update`, `compute_mach_factor`, `cpu_up`,
   `ast_init`, `fix_modem_state` and everything in `i386/i386/irq.c`
   index `NCPUS`-, `NINTR`- or `NCOM`-sized storage.  §7 Phase 3
   clears the whole class at once.
2. **An unmirrored struct field** (question 2).  Everything in
   `kern/eventcount.c` (`struct eventcounter`), most of
   `vm/vm_pageout.c` and `device/net_io.c` (`vm_page`, `vm_object`,
   `net_hash_entry`), and `i386/i386/machine_task.c`'s remaining
   entries (`task->machine`).  §7 Phase 4.
3. **A lock macro over an unmirrored struct** (question 2, indirectly).
   This is what gates `ipc/`: `ip_lock`, `ip_unlock`,
   `is_write_unlock` and `ips_lock` inline a dereference of `struct
   ipc_port` or `ipc_space` into the caller, so the caller fails
   question 2 even though the lock underneath is Rust.  It is why a
   directory of 18 files has yielded six functions.
4. **Not externally visible** (question 6).  `itoa`, `null_port`,
   `kern_invalid` and the three `acpi_parse_apic.c` helpers are
   `static` with no prototype.  The fix is to port the caller in the
   same commit so the function becomes a private Rust helper, never to
   add the `extern` declaration.
5. **Not compiled** (question 7).  The `#else /* MACH_HOST */` halves
   and the macro-shadowed `pmap_copy`/`pmap_kernel`.  These are
   deletions, §8.

The two cheapest ways to grow Tier 0 are Rust changes, not C ones:

* **Mirror `mach_msg_header_t` and `mach_msg_type_t`.**  That alone
  clears class 2 for the six `ipc_notify_init_*` functions in
  `ipc/ipc_notify.c`.
* **Bring `NCPUS` into Rust** (§7 Phase 3).  That clears class 1 and
  deletes `kern/processor_glue.c` at the same time (§10).

### 6.3 How to derive the list, and what the derivation cannot do

**The built objects are the oracle for questions 6 and 7.**  Run `nm
--defined-only` over `build-64` and `build-32` and read the symbol:

| `nm` says | Meaning | Verdict |
|---|---|---|
| `T` | externally visible and compiled | passes 6 and 7 |
| `t` | compiled, but local to its unit | fails 6: its callers move with it or it waits |
| absent | not compiled in this configuration | fails 7: it is a deletion, §8 |

That is exact and needs no parsing, and it is the check the first two
derivations did not have.  Apply it to the function *and to every
callee*: a same-file callee is only safe if it is `T` itself or moves
in the same commit.  `i386/i386/smp.c:52 smp_send_ipi` is the case
that taught this, and it cost two entries.

**Then parse, to shortlist.**  Extract every function definition in
the eight C directories and keep the ones whose callees are all in
`glue`, already Rust, or a settled exemption.  This is a filter, not
an answer.

**Then read the survivors, because two questions stay manual.**
Question 2 and question 3 both hide outside the function body: a `.`
field access or a struct passed by value, and an array whose
configure-time size is in its declaration.  `fix_modem_state` and the
`irq_*` accessors pass every mechanical check and fail both.

**Redo it after any Phase 3 or Phase 4 work**, and after any batch of
ports: every mirror that lands and every constant that becomes
visible to Rust moves functions out of the rejection classes.  A
derivation is a snapshot of one afternoon's tree.

### 6.4 Tiers 1 to 4

* **Tier 1 — one "no", and the fix is small.**  `ipc/ipc_target.c`,
  `kern/boot_script.c`, `i386/i386/hardclock.c`.
* **Tier 2 — one "no", cleared by a numbered phase.**
  `kern/mach_factor.c`, `kern/syscall_sw.c`, `kern/rdxtree.c`,
  `i386/i386/irq.c`, `i386/i386/machine_task.c`, `chips/busses.c`.
* **Tier 3 — the layers themselves.**  `slab`, `eventcount`,
  `priority`, `ipc_tt`, `ipc_host`, `host`.
* **Tier 4 — the anchors.**  `thread`, `task`, `sched_prim`,
  `ipc_mig`, `exception`, `mach_clock`, `startup`, `bootstrap`,
  `pmap`, `trap`, `pcb`, `ipc_kmsg`, `mach_msg`.  `kern/printf.c` is
  not in any tier: see §4.

## 7. Recommended phasing

The no-glue law fixes the shape of this list: a file moves only when
everything it needs from C is either a real symbol `glue` can declare
or Rust already.  Each phase exists to make the next one legal, and no
phase contains a shim.  Where the old phasing said "add the shim", the
replacement says which file to port instead.

* **Phase 0 — Tier 0 (done).**  The whole of `i386/i386/pit.c` was the
  last free file: `clkstart` and the four sleep and delay functions are
  ported, §6.1 is empty and §9 records it.  Each function needed
  nothing that did not exist today, so the phase finished without a
  single decision from any later one.

* **Phase 1 — per-CPU.**  Finish the accessor in `src/arch/<arch>/`
  (`src/arch/i386/percpu.rs` is the start).  `percpu_get`,
  `current_thread()` and `cpu_number()` are `%gs` macros, so there is
  nothing to declare and nothing that may be shimmed: the Rust
  accessor *is* the unblocking work.  `spl*` needs nothing — real asm
  functions, in `glue` already — so the `IrqGuard` over them is a
  Rust-side type written whenever it is wanted.
  *Unblocks:* `timer.c`, `priority.c`, `ast.c`, and the per-CPU half
  of everything later.

* **Phase 2 — locks: already done.**  This phase is recorded as
  complete because the audit that named locks a blocker predates the
  `kern/lock.c` port.  `kern/lock.c` and `i386/i386/lock.h` are gone,
  `src/kern/lock.rs` defines all fourteen `lock_*` symbols plus
  `mach_simple_lock`/`mach_simple_unlock`/`mach_simple_lock_try`, and
  the macros left in `kern/lock.h` expand to exactly those symbols
  (`kern/lock.h:171-186`).  `simple_lock_init` is a plain field write
  over the mirrored `SimpleLock`, and `simple_lock_irq` is
  `splhigh()` plus `mach_simple_lock`, both real.
  **So taking a lock from Rust needs no glue and no further work**, and
  any "blocked on locks" note elsewhere in this file is stale.
  `kern/kmutex.c` is the precedent: it moved whole in `d4fe54dc`,
  taking `SimpleLock` from `src/kern/lock.rs`, `current_thread()`
  from `src/arch/i386/percpu.rs` and
  `thread_sleep`/`thread_wakeup_prim` from `src/kern/sched_prim.rs`,
  and it added no C.  `kern/eventcount.c` is the next file of the
  same shape.
  *Unblocked already:* every file whose only C need was a lock — the
  largest single group in §4.

* **Phase 3 — the configure-time constants.**  `NCPUS`, `NINTR`,
  `NCOM` and friends size C arrays and shift struct tails, and Rust
  cannot name them today.  That single gap is what created
  `kern/processor_glue.c`, the `ivect`/`iunit` accessors in
  `i386/i386/irq.c` and `com_base_addr`/`com_irq` in
  `i386/i386at/com.c` (§10).  Generate them into Rust from the same
  `config.h` the C half uses, with layout asserts.
  *Unblocks:* the `ProcessorSet` tail, the mouse driver's remaining C,
  and every NCPUS-indexed array.

* **Phase 4 — the shared structs.**  Mirror and assert the layouts C
  and Rust both touch: `struct thread`'s `ith_next`/`ith_prev`,
  `struct vm_object` and `struct vm_page`'s flag bits, `struct task`'s
  fields, `struct timer`.  Each mirror lands with the deletion of the
  accessors it replaces, in the same commit.
  *Unblocks:* almost all of `vm_map_glue.c` goes away here; `thread.c`
  and `task.c` become portable at all.

* **Phase 5 — memory.**  `kalloc`, `kfree` and `kmem_cache_*` are real
  symbols Rust already calls, so this phase is about moving the
  allocator itself, not reaching it: `kern/slab.c` after `vm_page`'s
  mirror (Phase 4), then `kern/rdxtree.c`, then the slab caches still
  parked in `vm/vm_external_glue.c` and `vm/vm_map_glue.c`.  A
  `GlobalAlloc` over `kalloc` remains a separate design decision.

* **Phase 6 — scheduler surface.**  `mach_factor`,
  `priority`, `syscall_sw`, then the rest of `sched_prim.c`, then
  `ipc_sched.c`'s `thread_go`/`will_wait`, then `ast.c`.  Keep
  `switch_context`, `call_continuation` and `stack_handoff` C.

* **Phase 7 — objects.**  `ipc_tt`/`ipc_host`/`host` conversions →
  `processor`/`machine` → `task` → `thread` (the state machines last).

* **Phase 8 — IPC/VM/arch anchors.**  `ipc_kmsg`, `ipc_port`,
  `mach_msg`, `vm_object`, `vm_page`, then `pmap`, `trap`, `pcb`,
  `startup`/`bootstrap`, then the drivers.

**Not in any phase: `kern/printf.c`.**  It needs a C-variadic
definition, which the pinned toolchain rejects, and the old plan to
keep its entry points as C shims is exactly what the law forbids.  It
is blocked pending a toolchain or call-site decision; see its §4
entry.

Exit criterion for every step is unchanged, plus one: both qemu
architectures green, `rustfmt`/`clippy` clean, no new undefined
symbols, **and no new C**.  A module that needs nothing from the
kernel may add host tests like the rbtree's; see §8.

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
* Dead `#else /* MACH_HOST */` halves.  `MACH_HOST` cannot be 0 in
  this tree: `configfrag-first.ac:33` makes fewer than two CPUs a hard
  configure error and `configfrag.ac:38` defines `MACH_HOST` to 1
  above one CPU, so both `config.h` files carry 1.  The stub halves of
  `kern/machine.c:309 processor_assign`, `kern/task.c:1081
  task_assign`, `kern/thread.c:1832 thread_assign` and
  `kern/processor.c:291,299 processor_set_create`/`_destroy` are
  therefore never compiled; `nm` on `build-64/kern/processor.o` shows
  the 166-byte `#if` implementation.  Delete the `#else` halves rather
  than porting them.
* Macro-shadowed definitions, found by the §6.3 mechanical pass:
  `i386/intel/pmap.c:1902 pmap_copy` and `:2068 pmap_kernel` are real
  function definitions that no caller can reach, because
  `i386/intel/pmap.h:447` defines `pmap_copy` as an empty macro and
  `:443` defines `pmap_kernel()` as `(kernel_pmap)`.  Every caller
  including the header gets the macro.  Delete the two functions
  rather than porting them; they are not Tier 0 entries.
* `i386/i386at/rtc.h` keeps `struct rtc_st`, the `load_rtc`/`save_rtc`
  macros and the `RTCRTIME`/`RTCSTIME` ioctl numbers with no C user
  left: they are the driver interface, retained until that interface is
  retired, and only the two prototypes feed `model_dep.c` now.
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
| `kern/ipc_mig.c` (`mig_strncpy`, `mig_put_reply_port`, `mig_dealloc_reply_port`, `mach_msg_rpc_from_kernel`), `kern/host.c` (`host_get_kernel_version`, `host_kernel_version`), `kern/syscall_subr.c` (`mach_print`) | `src/kern/ipc_mig.rs`, `src/kern/host.rs`, `src/kern/syscall_subr.rs` | `c1cf99d4` |
| `device/dev_pager.c` (six `device_pager_*` entries), `kern/debug.c` (`__stack_chk_fail`), `kern/boot_script.c` (`boot_script_error_string`), `kern/bootstrap.c` (`boot_script_malloc`, `boot_script_free`) | `src/device/dev_pager.rs`, `src/kern/debug.rs`, `src/kern/boot_script.rs`, `src/kern/bootstrap.rs` | `eecdf229` |
| `kern/mach_clock.c` (`timeopen`, `timeclose`), `kern/syscall_emulation.c` (`eml_init`), `ipc/ipc_target.c` (`ipc_target_terminate`), `kern/task.c` (`task_assign_default`), `kern/thread.c` (`thread_assign_default`) | `src/kern/mach_clock.rs`, `src/kern/syscall_emulation.rs`, `src/ipc/ipc_target.rs`, `src/kern/task.rs`, `src/kern/thread.rs` | `5bc0c535` |
| `i386/i386/machine_task.c` (`machine_task_module_init`), `i386/i386at/model_dep.c` (`db_halt_cpu`, `db_reset_cpu`), `i386/intel/pmap.c` (`pmap_pageable`) | `src/arch/i386/machine_task.rs`, `src/arch/i386/model_dep.rs`, `src/arch/i386/pmap.rs` | `8dbc4e8e` |
| `kern/kmutex.c` | `src/kern/kmutex.rs` | `d4fe54dc` |
| `ipc/ipc_table.c` | `src/ipc/ipc_table.rs` | `bd582ec6` |
| `device/cirbuf.c` | `src/device/cirbuf.rs` | `pending` |
| `device/subrs.c` (`ether_sprintf`, `sleep`, `wakeup`; `if_init_queues` stays C) | `src/device/subrs.rs` | `pending` |
| `kern/thread.c` (`thread_init`) | `src/kern/thread.rs` | `pending` |
| `kern/sched.h` (`thread_timer_delta`) | `src/kern/thread.rs`, `src/kern/timer.rs` | `pending` |
| `kern/timer.c` (the five read/normalize/init functions) | `src/kern/timer.rs` | `pending` |
| `kern/debug.c` (`SoftDebugger`, `Debugger`, `panic_init`) | `src/kern/debug.rs` | `pending` |
| `kern/processor.c` (`processor_init`, `pset_init`, `processor_start/exit/control`, `processor_get_assignment`, `processor_info`, `processor_set_info`, `pset_reference`, `pset_deallocate`, `pset_add/remove_thread`, `thread_change_psets`, `processor_set_max_priority`, `processor_set_policy_enable/disable`) | `src/kern/processor.rs` | `pending` |
| `kern/machine.c` (`host_reboot`; rest stays C) | `src/kern/machine.rs` | `pending` |
| `kern/thread_swap.c` | `src/kern/thread_swap.rs` | `pending` |
| `i386/i386/ast_check.c` | `src/arch/i386/ast_check.rs` | `pending` |
| `i386/i386/mp_desc.c` (`simple_lock_pause`, `cpu_control`; rest stays C) | `src/arch/i386/mp_desc.rs` | `pending` |
| `i386/i386/fpu.c` (`fp_free`; rest stays C) | `src/arch/i386/fpu.rs` | `pending` |
| `device/dev_name.c` (`name_equal` and the eleven `nulldev_*`/`nodev_*`/`nomap` stubs; `dev_name_lookup`/`dev_set_indirection` stay C) | `src/device/dev_name.rs` | `pending` |
| `device/net_io.c` (`bpf_hash`; rest stays C) | `src/device/net_io.rs` | `pending` |
| `ipc/ipc_object.c` (`ipc_object_copyin_type`; rest stays C) | `src/ipc/ipc_object.rs` | `pending` |
| `ipc/ipc_port.c` (`ipc_port_timestamp` and its two globals; rest stays C) | `src/ipc/ipc_port.rs` | `pending` |
| `ipc/mach_port.c` (`mach_port_rename`, `mach_port_insert_right`, `mach_port_extract_right`, `mach_port_request_notification`; rest stays C) | `src/ipc/mach_port.rs` | `pending` |
| `i386/i386at/rtc.c` | `src/arch/i386/rtc.rs` | `pending` |
| `i386/i386/pit.c` (the whole file, `clkstart` and its four globals included) | `src/arch/i386/pit.rs` | `pending` |

Deleted dead code: `device/blkio.c` (unreachable block pager path) and
the `#if 0` profiling facility (`profil.h`, `profilparam.h`,
`mpqueue`).

## 10. The glue debt

Every piece of C in this tree that exists only so Rust can reach
something.  All of it predates `AGENTS.md`'s no-glue law, none of it
is a precedent, and nothing may be added to it.  Each row says what
deletes it.

| Glue | What it provides | Deleted by |
|---|---|---|
| `vm/vm_map_glue.c` (476 lines, 42 functions) | `current_thread()->vm_privilege`; the `pmap_attribute`, `pmap_copy` and `thread_wakeup` macros; `struct vm_object` and `struct vm_page` bit probes; `struct task` field accessors; the memory-object proxy cast; the three `kmem_cache` storage symbols | Phase 1 (percpu, for `current_thread()`), Phase 4 (the `vm_object`/`vm_page`/`task` mirrors — most of the file), Phase 5 (the caches, when `slab.c` moves) |
| `vm/vm_external_glue.c` | Three `kmem_cache` symbols as storage, not as shims | Phase 5: `kern/slab.c` |
| `kern/processor_glue.c` | The NCPUS-sized `struct processor_set` tail (`machine_quantum` … `sched_load`) and its two load accessors | Phase 3: `NCPUS` visible to Rust, so the `ProcessorSet` mirror carries the tail |
| `kern/sched_prim.c:716` — `thread_glue_pset_sched_load` | The same pset tail, read from the scheduler | Phase 3, with the row above |
| `i386/i386/irq.c` — `irq_mask`, `irq_unmask`, `irq_{set,get}_handler`, `irq_{set,get}_unit` | `ivect`/`iunit` are `NINTR`-sized arrays and `mask_irq` is `static inline` under APIC | Phase 3 (`NINTR`), plus a Rust `mask_irq` equivalent |
| `i386/i386at/com.c` — `com_base_addr`, `com_irq` | `cominfo` is an `NCOM`-sized array | Phase 3 (`NCOM`), or porting `com.c` |
| `i386/i386at/kd_glue.c` | `struct tty`'s lock macros, the line-discipline switch, `ttlowat[]` | Phase 2 (locks) for the first four; the `tty`/`ldisc` port for the rest |
