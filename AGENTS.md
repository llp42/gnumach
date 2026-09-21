# AGENTS.md — GNU Mach: the C-to-Rust port, and the rules

GNU Mach is being migrated from C to **native Rust**, one routine at a
time.  This file is the whole contract: what the project is trying to
do, what the target imposes, how the Rust is written, and how the Rust
half is built.  It covers `rust/` too, which has no README of its own.

`MIGRATE.md` beside it is the map — every translation unit, its
blockers, and the six layers of coupling.  This file is the rules.

## The idea

The target is a Rust kernel, not a kernel with Rust in it.  Getting
there is incremental by construction, so at any moment the tree is
**mixed and noisy** — C files beside Rust modules, Rust modules wrapped
in `extern "C"` adapters, small C shims that exist only to let the two
halves meet.  That noise is expected and temporary.  It is the cost of
keeping the kernel booting and the test suite green at every commit,
and it is not a defect to tidy away by making the Rust look like the C.

Two rules hold the shape of it:

- **Always port to the purest Rust the routine allows.**  The point of
  moving a routine is to get Rust's guarantees; a line-for-line rewrite
  with the same raw pointers and the same `int` error codes has bought
  nothing.  Write the module as Rust would be written, and let the
  C-shaped signature live in a thin adapter at the edge of it.
- **Where C has not moved yet, write a C shim to keep compatibility.**
  The unported side keeps calling the symbol it always called, with the
  signature it always had.  If a macro, a lock, or a `struct` accessor
  cannot cross FFI, add a small C shim function next to its C file
  (`ipc/ipc_thread_glue.c` is the pattern) rather than dragging the C
  idiom inside the Rust module.

A shim is scaffolding.  When its last C caller moves, the shim goes in
the same commit.

### Where the noise is allowed to live

```
C caller ─► extern "C" adapter ─► safe Rust core ─► safe Rust helpers
           └── raw pointers, out-params, int error codes stop here
```

The adapter is the only place that speaks C: it validates what C handed
over, converts it into Rust types, calls the safe core, and converts the
answer back.  Everything behind it is written as if C did not exist.
`rust/src/kern/rbtree.rs` is the worked example — a `NodeRef` handle and
methods on `Rbtree` carry the algorithms, and nine `extern "C"`
functions adapt them for `kern/slab.c`.

Each step replaces a C definition with a Rust one of the same name and
the same signature, so the kernel links exactly as it did before and
the test suite says whether it still works.  There is no staging branch
and no parallel implementation kept beside the original: the C
definition goes away in the same commit the Rust one arrives, because
two definitions of one symbol is a link error, not a fallback.

## Hard rule: the tests are evidence, not an obstacle

**Weakening a test to make it pass is forbidden.**  The qemu suite is
the only correctness gate this project has.  A suite made green by
editing the suite proves nothing, and the next reader cannot tell it
from a suite that was green on merit.

Specifically, never:

- delete, rename away, `#if 0`, comment out or `#[ignore]` a test, a
  case, or an assertion;
- drop a `tests/module-*` from `tests/Makefrag.am` or from the set that
  gets booted;
- loosen an assertion — an exact value into a range, `assert_eq!` into
  `assert!`, a failure into a printed warning;
- shrink an input set, a loop count, an iteration bound or a table so
  that the failing case is no longer reached;
- widen a timeout, or re-run until it comes up green;
- skip an architecture: i386 and x86_64 are both the gate, always;
- delete a `debug_assert!` or an `--enable-queue-debug` invariant check
  that fires;
- silence the build gate instead of the test — `#[allow(...)]`, `-A` on
  the clippy line, dropping `-D warnings`, removing a `const` layout
  assertion, or adding a symbol to the `gnumach-undef` allowlist to
  hide an undefined reference.

**Coverage never goes down.**  A ported routine keeps every case its C
version was exercised through, and usually gains some.

If a test fails, the port is wrong until proved otherwise.  Read the
failure, fix the code.  If the test itself is genuinely wrong, say so
out loud and fix it as *its own commit*, with the reasoning in the
message — not folded into the change it was blocking.

The one narrow exception is a deliberate change of observable
behaviour.  Then: a separate commit, a message naming the behaviour
that changed and why, a new assertion at least as strict as the old
one, and no net loss of coverage.  Adding tests needs no ceremony at
all and is always welcome.

## The port so far

The string routines moved first: all of `i386/i386/strings.c` and
`kern/strings.c` — `memcpy()`, `memmove()`, `memcmp()`, `memset()` and
the `str*()` family.  Both C files are gone, so the kernel's own calls
and the ones the C compiler inserts for struct and array copies land in
Rust now.

The Mach queue package followed: `kern/queue.c` is gone too, replaced by
`src/kern/queue.rs` — an idiomatic module (NonNull, Option, an iterator)
behind thirteen `extern "C"` wrappers keeping the old symbols: the four
live routines of `kern/queue.c` (`dequeue_tail()` and `insque()` were
dropped as uncalled), the former accessor macros `queue_init()`,
`queue_first()`, `queue_next()`, `queue_prev()`, `queue_end()` and
`queue_empty()`, and the generic `queue_enter()`, `queue_enter_first()`
and `queue_remove()` operations themselves, which take the chain-field
offset as a value.  Those three macros are gone too: every former call
site spells the `__builtin_offsetof` out, so `kern/queue.h` keeps only
`queue_iterate()`.  `QueueEntry` is `#[repr(C)]`-identical to `struct
queue_entry`, so that one macro keeps working on the same layout.
(`mpqueue` is gone altogether: its only user was the dead `#if 0`
profiling facility.)  `QueueEntry` is `!Unpin`, and linking takes
`Pin<&mut _>`: the C callers pin their objects by contract, the
`extern "C"` wrappers turn that contract into a `Pin` at the boundary,
and safe Rust can no longer move an entry once it is linked.  A
development build configured with `--enable-queue-debug` compiles
read-only link-invariant checks around every mutation; they never
change the links, so the queue behaves identically.

The generic SMP controller followed: `kern/smp.c` is gone too, replaced
by `src/kern/smp.rs`, which keeps `smp_set_numcpus()` and
`smp_get_numcpus()` over a private `AtomicU8`.  The count starts at one
and the setter rejects zero, so the getter is a plain load; the probe
publishes the value on the boot processor and every CPU reads it
afterwards, so it is not plain state.  Its former `smp_info` symbol
appeared in no header and was referenced by nothing, and so is gone.

**Next:** work outward from the string routines.  A good candidate is a
leaf, needs no allocation, and has a C definition that can be deleted in
the same commit.  The audited list of self-contained translation units
lives in `MIGRATE.md`.

## The toolchain contract

Pinned in `rust-toolchain.toml` (channel `stable`, with `rust-src`,
`clippy` and `rustfmt`), driven from `rust/Makefrag.am`.  No Cargo, no
lock file, no build script, nothing fetched from the network.

| | |
|---|---|
| **Edition** | **2024** — `--edition 2024` in `AM_RUSTFLAGS`, and `edition = "2024"` in `rust/rustfmt.toml`. The two move together or rustfmt parses a different language from rustc. |
| Target | `rust/targets/{i686,x86_64}-gnumach.json` — `"os": "none"`, `"std": false`, `panic-strategy: abort`, no MMX/SSE, soft float, no red zone, kernel code model, static relocation. |
| Codegen | `-C panic=abort -C opt-level=2 -C force-frame-pointers=yes -C overflow-checks=off`, and `-C lto=fat` for `libmach-rs.a`. |
| Nightly knobs | `RUSTC_BOOTSTRAP=1`: custom target JSONs and building `core` out of tree are nightly-only on a stable channel. |
| Gates | `rustfmt --check` at 79 columns and `clippy-driver -D warnings` run as `rust/lint.stamp`, which `libmach-rs.a` depends on. A warning fails the build. |
| Correctness | `mise run test` — the qemu suite on x86_64 **and** i386. `.githooks/pre-commit` runs it on every commit. |

## Edition 2024

The crate is edition 2024 and new code is written in its idiom.  What
it changes here, and what it buys:

- **`unsafe extern "C" { ... }`.**  Extern blocks are unsafe in 2024;
  `rust/src/glue.rs` declares the C side inside one.  A bare
  `extern "C" { }` no longer compiles.
- **Unsafe attributes.**  `#[unsafe(no_mangle)]`,
  `#[unsafe(export_name)]` and `#[unsafe(link_section)]` — the bare
  spellings are an error.  Every exported adapter carries the wrapped
  form.
- **`static_mut_refs` is a hard error.**  Taking a reference to a
  `static mut` no longer compiles, so rule 5 is enforced by the
  compiler rather than by review.
- **`unsafe_op_in_unsafe_fn` warns by default**, and `src/lib.rs`
  raises it to `deny`.  The body of an `unsafe fn` is not an implicit
  unsafe block: "this function's contract is unsafe" and "this line
  dereferences a pointer" stay separate statements.
- **Never-type fallback changed.**  `!` no longer falls back to `()`,
  which matters around diverging FFI (`Panic()` is `-> !`) and match
  arms that never return.  Annotate rather than rely on inference.
- **`if let` and tail-expression temporary scopes changed.**
  Temporaries in an `if let` scrutinee drop before the `else` arm.
  Where the scrutinee takes a lock or a guard, say what the scope is
  instead of leaning on the old timing.
- **RPIT lifetime capture.**  `impl Trait` in return position captures
  every in-scope lifetime; use `+ use<>` to opt out when an iterator
  must not borrow its argument.
- **`gen` is a reserved keyword**, and `macro_rules!`'s `expr` fragment
  now matches `const` blocks and `_`.

## no_std: what the target costs

`#![no_std]`, `core` only, **no `alloc`**.  This is the constraint that
shapes every design here, so it is worth stating the consequences
rather than rediscovering them per port:

- **No `Box`, `Vec`, `String`, no collections.**  Containers are
  *intrusive*: the node lives inside the caller's structure and the
  container links it (`src/kern/queue.rs`, `src/kern/rbtree.rs`).  Code
  works in memory the caller supplies.  When a routine genuinely needs
  to allocate, that is a design conversation about a `GlobalAlloc` over
  `kalloc` — not something added quietly to land one patch.
- **Intrusive means pinned.**  A linked node must not move;
  `QueueEntry` is `!Unpin` and linking takes `Pin<&mut _>` so that safe
  Rust cannot move it.  New intrusive types follow that pattern.
- **Panic is a halt.**  `-C panic=abort`, `panic-strategy: abort` in
  the target, and `src/panic.rs` routes `#[panic_handler]` into the
  kernel's `Panic()`.  There is no unwinding, nothing to catch, and no
  unwinding across FFI.  `Result` is the only error channel — see rule
  12.
- **Arithmetic wraps silently** (`-C overflow-checks=off`).  There is
  no overflow panic here to catch anything — see rule 13.
- **No `std::sync`.**  No `Mutex`, `RwLock`, `Once`, `OnceLock`, and no
  `thread_local!`.  Shared state is `core::sync::atomic` or the
  kernel's own `simple_lock` behind a Rust guard type; per-CPU data
  goes through the `%gs` accessors in `src/arch/`.
- **Atomic width.**  `max-atomic-width` is 64 on both targets, but
  prefer `AtomicUsize`/`AtomicU32`/`AtomicU8`: a 64-bit atomic on i686
  lowers to a `cmpxchg8b` loop and can reach for libatomic, which the
  kernel does not link.  `gnumach-undef-bad` is what catches it, at
  link time, on one architecture only.
- **No floating point.**  The targets disable MMX/SSE and use soft
  float; `f32`/`f64` do not belong in kernel code.  `-C lto=fat` is
  what keeps `core`'s float formatting — and the soft-float intrinsics
  libgcc does not provide — out of the archive.
- **Two pointer widths.**  i686 is 32-bit and x86_64 is 64-bit, from
  one source tree.  Never assume `usize` is 64 bits; spell the FFI
  boundary in `core::ffi` types (`c_int`, `c_uint`, `c_char`) and the
  Mach types in `src/arch/types.rs`.
- **`#![no_builtins]`.**  LLVM may rewrite a byte-copy loop into a call
  to `memcpy`, which in this crate is a call to itself.  The attribute
  is not unused: nothing tests for it.
- **C strings.**  `core::ffi::CStr` and `c"..."` literals, not a
  hand-rolled NUL walk.  `core::fmt` exists but drags in machinery;
  printing goes through the `printf` shim.
- **No test harness in the kernel.**  The qemu suite is the gate.  Only
  a module that needs nothing from the kernel and keeps out of
  `crate::` can carry `#[cfg(test)]` tests for the host (rule 20).

## The rules

These apply to all work in this repository, to new ports and to
cleanups of ports already landed.  Rules 1–12 are the port's idiom
rules; 13–20 are the standing Rust practice this target needs.

### 1. Error handling

No integer return codes and no `NULL`-as-failure.  A fallible insertion
returns `Result<(), Error>`; a lookup returns `Option<&Node>` or
`Option<&mut Node>`.  The `extern "C"` adapter is what turns those back
into the `0` / `-1` or the possibly-null pointer C expects.

### 2. Out-parameters

Return the data.  `void rb_search(key, struct node **out)` becomes
`fn search(&self, key: K) -> Option<&Node>`.  Multiple results come back
as a tuple or a named struct, never as a caller-supplied slot to fill.

### 3. Memory slices

A pointer and a length that belong together are one `&[u8]` or
`&mut [u8]`.  They are fused at the FFI boundary, inside the adapter,
and no function behind it takes the two separately.

### 4. Pointers versus references in tree linkage

Rust's aliasing rules mean `&mut` usually cannot express parent/child
links — the tree's own edges alias the nodes.  Where raw pointers are
genuinely needed, they are `NonNull<Node>`, never a bare `*mut Node`
documented as non-null.  All pointer arithmetic and link surgery is
encapsulated in safe methods on the container; the module boundary
exposes safe references only.

### 5. Global state

No `static mut` — edition 2024 makes a reference to one a hard error
anyway.  A scalar becomes `AtomicUsize` / `AtomicBool` / an atomic of
the right width with an explicitly chosen ordering; compound state is
wrapped in a GNU Mach-appropriate lock (the kernel's own `simple_lock`
behind a Rust guard type — see the L2 layer in `MIGRATE.md`), not in a
`Mutex` this environment does not have.

### 6. Type-state for values with a fixed domain

`#define RB_RED` / `RB_BLACK` and similar integer constants become a
Rust `enum` (`NodeColor { Red, Black }`), and the logic that consumes
them uses exhaustive `match` — rebalancing especially, where the
compiler checking that every case is handled is most of the value.

### 7. Initialization

No two-step init.  Implement `Default`, or provide `Self::new()`, so an
empty tree or a fresh node is constructed already valid.  A C
`rbtree_init(&tree)` becomes `Rbtree::new()` behind its adapter.

### 8. Bitflags

Flag sets are a `bitflags!`-style type, not raw bitwise arithmetic on an
integer.  **This build has no Cargo and fetches nothing from the
network**, so use an in-tree equivalent macro under `rust/src/utils/`
rather than adding the `bitflags` crate as a dependency.

### 9. Iteration

Replace `while` loops that walk internal links in the caller's face with
a real iterator: implement `Iterator` yielding `&T` or `&mut T` for
in-order traversal, and let callers use `for`, `find`, `any`.  Internal
node pointers do not leave the module.

### 10. Strings

Validate UTF-8 at the FFI boundary and convert to `&str` there, so the
Rust core works in `&str`.  Use `&[u8]` strictly for raw binary IPC
payloads and other byte buffers that are not text.

### 11. Safety documentation, and the smallest unsafe block

`unsafe` marks the operation that needs it and nothing else — not the
function, not the loop around it.  Cast once, compute offsets in safe
code, and touch memory at the last possible moment.

Every `unsafe { ... }` block — frequent in rotations and pointer
dereferences — carries a `// SAFETY:` comment naming the invariant it
relies on and who guarantees it.  "Node is non-null and uniquely
accessed for the length of this rotation" is a safety comment; "raw
pointer" is not.  Every exported `unsafe extern "C" fn` documents its
contract in a `# Safety` section; clippy's `missing_safety_doc` fails
the build without one.

### 12. No panics

Avoid `unwrap()`, `expect()` and direct indexing whose bounds are not
statically guaranteed.  Use `get()` / `get_mut()`, `match`, `?` and the
`checked_*` family.  A panic here reaches `src/panic.rs` and the
kernel's `Panic()`: it is a kernel halt, not an error message.

### 13. Casts and arithmetic

No `as` where `From`, `TryFrom` or `try_into()` will do; `as` is for a
deliberate truncation, with a comment saying why it cannot lose
anything that matters.  Overflow checks are off, so say `wrapping_*`,
`checked_*` or `saturating_*` whenever the behaviour at the boundary is
part of what the routine means.

### 14. Newtypes, not aliases

A handle, an index or a unit gets a `#[repr(transparent)]` newtype.  A
`type` alias stops nothing: it will not keep a `vm_offset_t` out of a
parameter that wants a `vm_size_t`.

### 15. Layout mirrors are asserted, not described

Every `#[repr(C)]` type shared with C carries `const` assertions on
`size_of`, `align_of` and `offset_of` against the C layout it mirrors,
so drift is a build error rather than a corrupted field at run time
(`src/kern/rbtree.rs` is the precedent).

### 16. `#[must_use]`

On `Result`-returning functions that C would have let you ignore, on
constructors, and on handles whose value must not be dropped silently.

### 17. Atomic orderings are chosen, not defaulted

Name the ordering and say in a comment what it publishes or acquires.
`SeqCst` is a decision to justify, not a fallback; `Relaxed` needs to
say why no other thread depends on the order.

### 18. Doc comments and visibility

Every module's doc comment names the C file it replaces and the header
it mirrors.  Every public item has a doc line; every
`unsafe extern "C" fn` has `# Safety`.  Visibility is the smallest that
works — internals stay private so that the module boundary *is* the
safe API.

### 19. `const` over macros

A C `#define` becomes a `const` or a `const fn`, not a Rust macro.
Reach for a macro only when a function or a generic genuinely cannot
express it.

### 20. Host-testable modules stay `crate::`-free

A module that needs nothing from the kernel keeps out of `crate::` so
that `rustc --test` can compile it for the host (`tests/test-rbtree-rs`
does this for the rbtree as part of `make check`).  It is still
compiled into `libmach-rs.a` like any other module — that is not a
second build path for the kernel.

## The qemu suite is the gate

`mise run test` — the project's own `make check`, booting every
`tests/module-*` under qemu on x86_64 and i386 — is what decides whether
a port is correct.  A ported routine is verified by being exercised
through the running kernel; if it has behaviour the suite does not
reach, the test to add is a C one under `tests/`, beside the others.
The one exception is the host-compiled module of rule 20.

`.githooks/pre-commit` runs the whole suite on every commit,
deliberately and without a file-extension filter.

See the hard rule above: the suite is changed to cover more, never to
demand less.

## Where things go

- `rust/src/utils/` — code that is the same on every machine.
- `rust/src/kern/` — machine-independent facilities, mirroring `kern/`.
- `rust/src/arch/<arch>/` — code written twice, for i686 and x86_64.
- `rust/src/glue.rs` — the C functions Rust calls, declared with the C
  signature exactly, inside an `unsafe extern "C"` block (edition
  2024).  A C *macro* cannot come through here: it needs a shim written
  in C, so that the C compiler still expands it with this build's
  configuration.

`rust/src/` mirrors the C tree, so a routine's Rust home should be
recognisable from the C file it came out of.  A C shim written for an
unported caller lives beside its C file instead, named `*_glue.c`
(`ipc/ipc_thread_glue.c`), with a comment saying what will delete it.

## No Cargo

`rustc` is driven straight from `rust/Makefrag.am`.  Do not add a
`Cargo.toml`, a lock file, a build script, or anything that fetches from
the network — the kernel's own build system stays the single source of
truth.  This is also why rule 8's bitflags macro is written in tree.

Three things get built:

| | |
|---|---|
| `rust/libcore.rlib` | `core`, compiled from the toolchain's own sources for our target.  A release toolchain ships `core` only for targets it knows about, and none of them is a freestanding kernel. |
| `rust/libcompiler_builtins.rlib` | An empty crate.  rustc requires one for a `staticlib`, but the kernel already links libgcc for these intrinsics. |
| `libmach-rs.a` | The kernel code itself, from `rust/src/`. |

The kernel is compiled for a target of our own, `rust/targets/*.json`,
rather than one of rustc's, so that code generation matches what the C
half is told to do: no MMX/SSE, no red zone, the kernel code model, no
position-independent code.

## What the build checks

Both lints run from `rust/Makefrag.am` as `rust/lint.stamp`, which
`libmach-rs.a` depends on — so they are part of an ordinary `make`, not
a separate step anyone can forget:

- **`rustfmt --check`** over `MACH_RS_SRCS`.  `rust/rustfmt.toml` sets
  `max_width = 79`, because the C half is written to 79 columns and
  rustfmt's default of 100 would reflow every signature in the tree on
  its first run.
- **`clippy-driver … -D warnings`** over the crate.  A clippy warning
  fails the build, and is fixed rather than allowed.

Neither is an extra thing to install: `rust-toolchain.toml` lists
`clippy` and `rustfmt` among the toolchain's components, and `configure`
checks for both and fails early with a hint if either is missing.  They
are cheap — `core` is already built by the time they run, so clippy has
only this crate left to look at, and the pair costs about a tenth of a
second.

## Constraints that break the build if forgotten

- `#![no_builtins]` in `rust/src/lib.rs` guards against LLVM rewriting a
  byte-copy loop into a call to `memcpy` — in this crate, a call to
  itself.  It was *not* observed to do so at `-C opt-level=2 -C lto=fat`
  on rustc 1.98; the attribute is there so that a change of pass, level
  or toolchain cannot make it happen.  Do not drop it as "unused":
  nothing tests for it.
- `Makefile.am` names `libkernel.a` twice around `libmach-rs.a`.  The two
  archives call into each other, and one pass over each cannot resolve
  that.  `--start-group` would say the same thing, but Automake rejects
  linker flags in a `_LDADD`.
- `-C lto=fat` is not an optimisation.  Without it the archive keeps
  `core`'s float formatting and the soft-float intrinsics libgcc does
  not provide, which `gnumach-undef-bad` rejects.  The same check is
  what catches a 64-bit atomic on i686 reaching for libatomic.
- `RUSTC_BOOTSTRAP=1`, custom target JSONs, and compiling `core` out of
  tree are all nightly-only knobs, so the recipes set it.
- `rust-toolchain.toml` at the top of the tree pins the toolchain and its
  components.  An absolute `RUSTC` path is not enough on its own: with
  rustup it means "whatever toolchain is default this week", and a build
  that mixes two of them fails with E0514 ("compiled by an incompatible
  version of rustc") rather than saying so.
- mise does not simply obey `rust-toolchain.toml`: it exports
  `RUSTUP_TOOLCHAIN`, which outranks the file.  `mise.toml` sets
  `idiomatic_version_file_enable_tools = ["rust"]` so that mise reads the
  file instead; never add a `rust` entry to a `[tools]` section there,
  because the second pin wins and then drifts.
- `RUST_LIB_SRC` comes from `configure`, so it goes stale when the
  toolchain updates — it then points at the old toolchain's `core`
  sources while `make` uses the new rustc.  Reconfigure after a toolchain
  update; the `mise run build` and `mise run test` tasks compare it
  against `rustc --print sysroot` and reconfigure when it has moved.
- The test programs are user-mode binaries with their own link, and the
  string routines they use live in `tests/string.c` rather than in
  `libmach-rs.a`, which is built for the kernel's target.  Moving a
  routine the tests call means giving them a copy of their own there.

## Adding a routine

1. Write it under `rust/src/utils/`, `rust/src/kern/` or
   `rust/src/arch/<arch>/`, name the C file it replaces in the module
   doc comment, and add it to `MACH_RS_SRCS` in `rust/Makefrag.am`.
2. Write the core as native Rust, by the twenty rules — safe types, no
   out-parameters, no integer error codes — and keep the C-shaped
   signature in a thin `extern "C"` adapter over it.
3. Give the adapter the C signature exactly, `#[unsafe(no_mangle)]` and
   `extern "C"`, and a `# Safety` section saying what the caller has to
   guarantee.
4. Delete the C definition in the same commit.  Two definitions of one
   symbol is a link error, not a fallback.  Where an unported caller
   needs a macro or an accessor, add a minimal `*_glue.c` beside it.
5. If the routine also lives in the test programs, give them their own
   copy in `tests/string.c`.
6. Record the move in `MIGRATE.md`.
7. `mise run test` — both architectures green — before committing.

## Review before done

Compiling and passing the suite is necessary, not sufficient.  Before
reporting a task complete, run the `rust-reviewer` agent (`/review`)
over the pending diff and resolve every BLOCKER it reports.  A
`READY WITH NITS` verdict may be committed, with the nits left for a
follow-up; `NOT READY` may not.  The reviewer only reports — it never
edits — so the author fixes what it finds.

## Checklist before a commit

1. No test was weakened, skipped, shortened or deleted, and no lint,
   assertion or allowlist was loosened to get green.  Coverage is the
   same or better.
2. The C definition of every symbol the Rust now defines is deleted in
   the same commit.
3. Any C shim added is minimal, named `*_glue.c` beside its C file, and
   says what will delete it.
4. The twenty rules hold for the new module, not only the parts clippy
   can check.
5. New code is edition 2024 idiom: `unsafe extern "C"` blocks,
   `#[unsafe(no_mangle)]`, no `static mut`.
6. `mise run test` — x86_64 **and** i386 — is green.
7. `MIGRATE.md` records what moved.
8. The diff has passed `rust-reviewer` (`/review`) with no unresolved
   BLOCKER.

## License headers

**Source files carry an SPDX header.  Build files do not.**

```
// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
```

Use `//` in `.rs`, `dnl` in `.ac`, `/* */` in `.c` and `.h`.  Keep the
copyright line exactly as above.

**No header** on `AGENTS.md`, `MIGRATE.md`, `rust/Makefrag.am`,
`rust/configfrag.ac`, `rust/rustfmt.toml`, or `rust/targets/*.json`
(JSON has no comment syntax, so it is left bare).  This matches the C
half, where `Makefrag.am` and `configfrag.ac` have none either.
