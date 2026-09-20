# MIGRATE.md — candidates for the Rust port

The kernel translation units that depend on nothing but themselves:
no external function calls, no external data.  They are the next
candidates for the incremental Rust port described in `rust/AGENTS.md`;
a port deletes the C definition in the same commit the Rust one
arrives, and `mise run test` decides whether it is correct.

## Method

1. `nm -u` over every kernel object in `build-64` (x86_64) and
   `build-32` (i386), keeping only objects whose undefined symbols are
   a subset of the twelve string routines already in Rust (`memcpy`,
   `memmove`, `memcmp`, `memset`, `strchr`, `strcpy`, `strncpy`,
   `strsep`, `strcmp`, `strncmp`, `strlen`, `strstr`).  In fact every
   survivor has no undefined symbols at all.
2. An ast-grep audit (`--kind call_expression`) of each survivor for
   the callees `nm` cannot see: macros, `static inline` header helpers
   and compiler builtins.  Every callee found is named below and
   classed as local, macro, builtin or asm.
3. Indirect (function-pointer) calls are invisible to `nm`; the audit
   found none in these files.

## Clean candidates: pure C, no asm, no allocation

- `util/byteorder.c` (53 lines) — `htonl()`, `htons()`, `ntohl()`,
  `ntohs()`.  Only callee: `__builtin_bswap{16,32}`, which is
  `u16::to_be()` & co. in Rust.  The simplest file in the tree.
- ~~`kern/queue.c`~~ **ported** — now `rust/src/kern/queue.rs`: a
  `#[repr(C)]`-compatible `QueueEntry` module behind the four `extern
  "C"` symbols (`dequeue_tail()` and `insque()` were dropped as
  uncalled), plus the `queue.h` accessor macros (`queue_init()`,
  `queue_first()`, `queue_next()`, `queue_prev()`, `queue_end()`,
  `queue_empty()`) and the generic `queue_enter()` and `queue_remove()`
  operations, all of which are Rust functions now; `queue_iterate()` is
  the only macro left.  `queue_enter_first()` is gone: its sole caller
  now expands to `queue_enter_head()` and its `__builtin_offsetof`.
  `QueueEntry` is `!Unpin` with `Pin`-based mutators, and
  `--enable-queue-debug` compiles read-only link invariant checks.
- `device/blkio.c` (66) — `block_io_mmap()`, `minphys()`.  Zero calls.
- `kern/smp.c` (49) — `smp_get_numcpus()`, `smp_set_numcpus()`.  Zero
  calls; owns the data symbol `smp_info`.
- `i386/i386at/kd_queue.c` (109) — `kdq_empty()`, `kdq_full()`,
  `kdq_get()`, `kdq_put()`, `kdq_reset()`.  Only callee: the file-local
  `q_next` macro.  A ring buffer for the keyboard/mouse event queue.
- `ipc/ipc_thread.c` (103) — `ipc_thread_enqueue()`,
  `ipc_thread_dequeue()`, `ipc_thread_rmqueue()`.  Callees are the
  `ipc/ipc_thread.h` macros (`ipc_thread_enqueue_macro()`, ...), which
  expand to pure struct pointer manipulation; Rust reimplements them.
- `util/atoi.c` (106) — `mach_atoi()`.  Zero calls, but the file is
  also linked into the test programs (`tests/user-qemu.mk`), so the
  port needs a test-side copy in `tests/`, like `tests/string.c` for
  the string routines.

## Candidates with one wrinkle

- `kern/rbtree.c` (463) — `rbtree_insert_rebalance()`, `rbtree_remove()`,
  `rbtree_nearest()`, `rbtree_firstlast()`, `rbtree_walk()`,
  `rbtree_postwalk_deepest()`, `rbtree_postwalk_unlink()` (plus the
  statics `rbtree_rotate()` and `rbtree_find_deepest()`).  Callees are
  `static inline` helpers in `kern/rbtree_i.h` (parent/color
  bit-packing) plus the `unlikely` hint; the Rust port reimplements the
  helpers as private functions.  The headers stay: other translation
  units use the same inlines.
- `kern/timer.c` (236) — `timer_init()`, `init_timers()`,
  `timer_read()`, `timer_normalize()`, `timer_delta()`,
  `thread_read_times()`, `db_thread_read_times()`; owns the data
  symbols `kernel_timer[NCPUS]` and `current_timer`.  Wrinkles:
  `__sync_synchronize()` (→ `core::sync::atomic::fence(Ordering::
  SeqCst)`) and `cpu_number()`, which is a `%gs`-segment asm macro
  (`percpu_get`) — a C macro needs a glue shim in `src/glue.rs` per
  `rust/AGENTS.md`, or the percpu read lands in `src/arch/` as asm.
- `i386/i386/loose_ends.c` (44) — `delay()`, plus the exported data
  symbol `cpuspeed`.  The only callee is the file-local `DELAY`
  busy-loop macro.  The port exports a `#[unsafe(no_mangle)] static`.
- `kern/elf-load.c` (104) — `exec_load()`.  The only external facility
  used is `alloca()` for the program-header table; Rust has no alloca,
  so the port first replaces it with a bounded on-stack array (and a
  check of `e_phnum` against that bound).

## Arch-asm candidate

- `i386/i386/db_interface.c` (103) — `db_get_debug_state()`,
  `db_set_debug_state()`, `db_load_context()`; owns `ddb_regs` and
  `zero_dr`.  Callees `set_dr0()`…`set_dr3()`/`set_dr7()` are
  `mov %dbN` inline-asm macros from `i386/i386/proc_reg.h`, and
  `current_thread()` is the `%gs` percpu macro.  The port belongs in
  `src/arch/<arch>/` with `core::arch::asm` or per-cpu glue shims.

## Excluded: self-contained by accident

Objects that pass the `nm` filter only because their code is compiled
out — nothing to port:

- `i386/intel/read_fault.c` — body is `#if (__i386__ && !(__i486__ ||
  __i586__ || __i686__))`; empty on i686 and x86_64.
- ~~`kern/profile.c`~~ — was entirely `#if 0`; deleted with the rest of
  the dead profiling facility (`profil.h`, `profilparam.h`, `mpqueue`).
- `ipc/copy_user.c` — empty on i386 (`#ifdef __LP64__`); on x86_64 it
  calls `copyin`, so it is not a candidate.
- `i386/i386at/kdasm.S`, `i386/i386/debug_trace.S` — assembly, not C.
- `version.c` — a data string, no code.
- MIG-generated `*.defs.o` / `*.server.o` stubs.

## Suggested order

`util/byteorder.c` is the next port — zero calls, no state.  Then the
rest of the clean list, then rbtree/timer, and leave `db_interface.c`
until `src/arch/` exists.
