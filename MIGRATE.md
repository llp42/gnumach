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

- ~~`util/byteorder.c`~~ **ported** — now
  `rust/src/utils/byteorder.rs`: `ntohs()`, `ntohl()`, `htons()` and
  `htonl()` over `from_be()`/`to_be()`, which is what the C's
  `__builtin_bswap{16,32}` blocks compiled to.
- ~~`kern/queue.c`~~ **ported** — now `rust/src/kern/queue.rs`: a
  `#[repr(C)]`-compatible `QueueEntry` module behind the four `extern
  "C"` symbols (`dequeue_tail()` and `insque()` were dropped as
  uncalled), plus the `queue.h` accessor macros (`queue_init()`,
  `queue_first()`, `queue_next()`, `queue_prev()`, `queue_end()`,
  `queue_empty()`) and the generic `queue_enter()`, `queue_enter_first()`
  and `queue_remove()` operations, all of which are Rust functions now,
  reached by an explicit `__builtin_offsetof` at every former call site;
  `queue_iterate()` is the only macro left in `kern/queue.h`.
  `QueueEntry` is `!Unpin` with `Pin`-based mutators, and
  `--enable-queue-debug` compiles read-only link invariant checks.
- ~~`kern/smp.c`~~ **ported** — now `rust/src/kern/smp.rs`: the two
  routines over a private `AtomicU8` that starts at one and rejects
  zero, so the getter is a plain load.  `smp_info` was file-local (its
  struct is defined in the .c, in no header) and referenced by nothing,
  so the symbol is gone.
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
- ~~`i386/i386/loose_ends.c`~~ **ported** — now
  `rust/src/utils/delay.rs`: `delay()` over a private `CPU_SPEED`,
  with a `black_box` standing in for the C's volatile counter; the
  unread `cpuspeed` data symbol is gone.
- ~~`kern/elf-load.c`~~ **ported** — now `rust/src/kern/elf_load.rs`:
  `exec_load()` identifies the image and dispatches to `exec_load32()`
  or `exec_load64()`, two typed paths mirroring each other, in place
  of the C's compile-time `Elf_Ehdr`/`Elf_Phdr` typedef.  Both classes
  load on x86_64; i686 rejects ELF64 with `EX_WRONG_ARCH` rather than
  truncating its addresses.  The program headers are read one at a
  time in place of the C's `alloca()` table, so their count stays
  unbounded; a stride smaller than the header's size is rejected as
  `EX_CORRUPT`, which removes the C's out-of-bounds read, while a
  larger one is accepted (and a table cut short inside an entry's
  stride padding still loads, because only the header's size is read,
  where the C's one-shot `e_phnum * e_phentsize` read failed).
  Consequences of reading the table one entry at a time: an `e_phnum`
  of zero reads no table at all, where the C issued a zero-size read
  that `boot_read` could fail for an out-of-module `e_phoff`; the
  caller's `exec_info_t` is zeroed once the image is recognized, and
  its `entry`/`stack_prot` are written only after the whole image has
  loaded, where the C wrote `entry` before reading the table and
  `stack_prot` before applying the entries, so an error there left them
  set (a segment already applied is not undone by a later failure, as
  in the C, but a table read failing partway has no C counterpart: the
  C read the whole table first); and a file shorter than the header
  reports bad `EI_DATA`/unknown `EI_CLASS` as `EX_WRONG_ARCH` where the
  C's short header read reported `EX_NOT_EXECUTABLE`.

## Arch-asm candidate

- `i386/i386/db_interface.c` (103) — `db_get_debug_state()`,
  `db_set_debug_state()`, `db_load_context()`; owns `ddb_regs` and
  `zero_dr`.  Callees `set_dr0()`…`set_dr3()`/`set_dr7()` are
  `mov %dbN` inline-asm macros from `i386/i386/proc_reg.h`, and
  `current_thread()` is the `%gs` percpu macro.  The port belongs in
  `src/arch/<arch>/` with `core::arch::asm` or per-cpu glue shims.

## Excluded: dead code

- ~~`device/blkio.c`~~ — deleted: `minphys()` had no callers, and
  `block_io_mmap()` was never installed as a `d_mmap`, so the block
  pager path in `device/dev_pager.c` was unreachable.

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

The clean list is down to `i386/i386at/kd_queue.c`, `ipc/ipc_thread.c`
and `util/atoi.c`.  Then rbtree/timer, and leave `db_interface.c`
until `src/arch/` exists.
