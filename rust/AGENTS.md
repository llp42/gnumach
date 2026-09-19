# rust/ — conventions

This directory is the Rust half of GNU Mach.  It builds into `libmach-rs.a`,
which is linked into `gnumach.o` alongside `libkernel.a`.

There is no separate README for this directory; this file is it.

## The goal

Move as much of the C kernel into Rust as it will take, one routine at a time.

The port is incremental by construction.  Each step replaces a C definition
with a Rust one of the same name and the same signature, so the kernel links
exactly as it did before and the test suite says whether it still works.
There is no staging branch and no parallel implementation kept beside the
original: the C definition goes away in the same commit the Rust one arrives,
because two definitions of one symbol is a link error, not a fallback.

So far the string routines have moved: all of `i386/i386/strings.c` and
`kern/strings.c` — `memcpy()`, `memmove()`, `memcmp()`, `memset()` and the
`str*()` family.  Both C files are gone, so the kernel's own calls and the
ones the C compiler inserts for struct and array copies land in Rust now.
The Mach queue package followed: `kern/queue.c` is gone too, replaced by
`src/kern/queue.rs` — an idiomatic module (NonNull, Option, an iterator)
behind six `extern "C"` wrappers keeping the old symbols.  `QueueEntry`
is `#[repr(C)]`-identical to `struct queue_entry`, so the `kern/queue.h`
macros keep working on the same layout.

**Next:** work outward from the string routines.  A good candidate is a
leaf, needs no allocation, and has a C definition that can be deleted in
the same commit.  The audited list of self-contained translation units
lives in `MIGRATE.md` at the top of the tree.

## Rules

### Write Rust, not transliterated C

The point of moving a routine is to get Rust's guarantees, so the Rust is
written the way Rust is written — slices and iterators rather than pointer
arithmetic, `NonNull` rather than a raw pointer that is merely documented as
non-null, types that make an invariant unrepresentable rather than a comment
asking the reader to maintain it.  A line-for-line rewrite of the C, with the
same pointers and the same loop, has bought nothing and reads worse than the
original.

clippy runs as part of the build with `-D warnings` (see below), so most of
this argument is settled mechanically rather than in review.

Two things this target imposes that are easy to forget:

- **`-C overflow-checks=off`**, so arithmetic wraps silently — there is no
  overflow panic here to catch anything.  Say `wrapping_*`, `checked_*` or
  `saturating_*` when the behaviour at the boundary is part of what the
  routine means.
- **`core` only.  There is no `alloc`.**  No `Box`, no `Vec`, no `String`, no
  collections.  Code here works in memory the caller supplies.  When a routine
  genuinely needs to allocate, that is a design conversation about putting a
  `GlobalAlloc` over the kernel's own allocator — not something to add quietly
  in order to land one patch.

### `unsafe` gets the smallest block that will hold it

`unsafe` marks the operation that needs it and nothing else.  Not the
function, not the loop around the operation, not whatever block was
convenient to write.

- `#![deny(unsafe_op_in_unsafe_fn)]` is set in `src/lib.rs`, so the body of an
  `unsafe fn` is *not* implicitly an unsafe block.  Leave it that way: it is
  what forces the distinction between "this function's contract is unsafe" and
  "this line dereferences a pointer".
- Every `unsafe` block carries a `// SAFETY:` comment naming the invariant it
  relies on and who guarantees it.  "The caller promises both are valid for
  `n` bytes" is a safety comment; "raw pointer" is not.
- Every exported `unsafe extern "C"` function documents its contract in a
  `# Safety` section of its doc comment.  clippy's `missing_safety_doc`
  enforces this, and will fail the build.
- Shrink the unsafe surface before widening the block.  Cast once, compute the
  offsets in safe code, and touch memory at the last possible moment, so that
  the block is one line rather than the whole routine.

### The qemu suite is the gate

`mise run test` — the project's own `make check`, booting every
`tests/module-*` under qemu on x86_64 and i386 — is what decides whether a
port is correct.  There are no host-side Rust unit tests and no second build
path to maintain.  A ported routine is verified by being exercised through the
running kernel; if it has behaviour the suite does not reach, the test to add
is a C one under `tests/`, beside the others.

`.githooks/pre-commit` runs the whole suite on every commit, deliberately and
without a file-extension filter.

## Where things go

- `src/utils/` — code that is the same on every machine.
- `src/kern/` — machine-independent kernel facilities, mirroring `kern/`.
- `src/arch/<arch>/` — code that has to be written twice, for i686 and x86_64.
- `src/glue.rs` — the C functions Rust calls, declared with the C signature
  exactly.  A C *macro* cannot come through here: it needs a shim written in
  C, so that the C compiler still expands it with this build's configuration.

`src/` mirrors the C tree, so a routine's Rust home should be recognisable
from the C file it came out of.

## No Cargo

`rustc` is driven straight from `rust/Makefrag.am`.  Do not add a
`Cargo.toml`, a lock file, a build script, or anything that fetches from the
network — the kernel's own build system stays the single source of truth.

Three things get built:

| | |
|---|---|
| `rust/libcore.rlib` | `core`, compiled from the toolchain's own sources for our target.  A release toolchain ships `core` only for targets it knows about, and none of them is a freestanding kernel. |
| `rust/libcompiler_builtins.rlib` | An empty crate.  rustc requires one for a `staticlib`, but the kernel already links libgcc for these intrinsics. |
| `libmach-rs.a` | The kernel code itself, from `rust/src/`. |

The kernel is compiled for a target of our own, `rust/targets/*.json`, rather
than one of rustc's, so that code generation matches what the C half is told
to do: no MMX/SSE, no red zone, the kernel code model, no position-independent
code.

## What the build checks

Both lints run from `rust/Makefrag.am` as `rust/lint.stamp`, which
`libmach-rs.a` depends on — so they are part of an ordinary `make`, not a
separate step anyone can forget:

- **`rustfmt --check`** over `MACH_RS_SRCS`.  `rust/rustfmt.toml` sets
  `max_width = 79`, because the C half is written to 79 columns and rustfmt's
  default of 100 would reflow every signature in the tree on its first run.
- **`clippy-driver … -D warnings`** over the crate.  A clippy warning fails
  the build.

Neither is an extra thing to install: `rust-toolchain.toml` lists `clippy` and
`rustfmt` among the toolchain's components, and `configure` checks for both
and fails early with a hint if either is missing.  They are cheap — `core` is
already built by the time they run, so clippy has only this crate left to look
at, and the pair costs about a tenth of a second.

## Constraints that break the build if forgotten

- `#![no_builtins]` in `src/lib.rs` guards against LLVM rewriting a byte-copy
  loop into a call to `memcpy` — in this crate, a call to itself.  It was
  *not* observed to do so at `-C opt-level=2 -C lto=fat` on rustc 1.98; the
  attribute is there so that a change of pass, level or toolchain cannot make
  it happen.  Do not drop it as "unused": nothing tests for it.
- `Makefile.am` names `libkernel.a` twice around `libmach-rs.a`.  The two
  archives call into each other, and one pass over each cannot resolve that.
  `--start-group` would say the same thing, but Automake rejects linker flags
  in a `_LDADD`.
- `-C lto=fat` is not an optimisation.  Without it the archive keeps `core`'s
  float formatting and the soft-float intrinsics libgcc does not provide,
  which `gnumach-undef-bad` rejects.
- `RUSTC_BOOTSTRAP=1`, custom target JSONs, and compiling `core` out of tree
  are all nightly-only knobs, so the recipes set it.
- `rust-toolchain.toml` at the top of the tree pins the toolchain and its
  components.  An absolute `RUSTC` path is not enough on its own: with rustup
  it means "whatever toolchain is default this week", and a build that mixes
  two of them fails with E0514 ("compiled by an incompatible version of
  rustc") rather than saying so.
- mise does not simply obey `rust-toolchain.toml`: it exports
  `RUSTUP_TOOLCHAIN`, which outranks the file.  `mise.toml` sets
  `idiomatic_version_file_enable_tools = ["rust"]` so that mise reads the file
  instead; never add a `rust` entry to a `[tools]` section there, because the
  second pin wins and then drifts.
- `RUST_LIB_SRC` comes from `configure`, so it goes stale when the toolchain
  updates — it then points at the old toolchain's `core` sources while `make`
  uses the new rustc.  Reconfigure after a toolchain update; the `mise run
  build` and `mise run test` tasks compare it against `rustc --print sysroot`
  and reconfigure when it has moved.
- The test programs are user-mode binaries with their own link, and the
  string routines they use live in `tests/string.c` rather than in
  `libmach-rs.a`, which is built for the kernel's target.  Moving a
  routine the tests call means giving them a copy of their own there.

## Adding a routine

1. Write it under `src/utils/` or `src/arch/<arch>/`, name the C file it
   replaces in the module doc comment, and add it to `MACH_RS_SRCS` in
   `rust/Makefrag.am`.
2. Give it the C signature exactly, `#[unsafe(no_mangle)]` and `extern "C"`,
   and a `# Safety` section saying what the caller has to guarantee.
3. Delete the C definition in the same commit.  Two definitions of one symbol
   is a link error, not a fallback.
4. If the routine also lives in the test programs, give them their own copy in
   `tests/string.c`.
5. `mise run test` — both architectures green — before committing.

## License headers

**Source files carry an SPDX header.  Build files do not.**

```
// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
```

Use `//` in `.rs`, `dnl` in `.ac`.  Keep the copyright line exactly as above.

**No header** on `rust/AGENTS.md`, `rust/Makefrag.am`, `rust/configfrag.ac`,
`rust/rustfmt.toml`, or `rust/targets/*.json` (JSON has no comment syntax, so
it is left bare).  This matches the C half, where `Makefrag.am` and
`configfrag.ac` have none either.
