# GNU Mach ABI test suite -- reusable binary pack

This directory contains a frozen, reusable binary pack of the GNU Mach ABI
test suite, taken from the `abi-tests` branch at commit `2fbf950f`:

* `i386.tar.gz` -- i386 pack, extracts to `abi-test-i386/`;
* `x86_64.tar.gz` -- x86_64 pack, extracts to `abi-test-x86_64/`;
* `run-all.sh` -- convenience wrapper that runs both architectures.

Each pack is self-contained and holds the 25 freestanding user-space ELF
test modules of that architecture plus a `grub.cfg.template`, a per-arch
runner and a `VERSION` file.  The suite has 26 tests per architecture:
`test-multiboot` (a `grub-file --is-x86-multiboot` check of the kernel
image, run first as a gate) and `test-X` for each `modules/module-X`.

The pack does not depend on the GNU Mach source tree, so it can be copied
to any other branch, worktree or machine.  The only inputs are the pack
directory and a GNU Mach kernel image.

## What a run does

For each selected test the runner builds a tiny bootable ISO containing
`boot/gnumach`, `boot/module-X` and a rendered `boot/grub/grub.cfg`, then
boots it in QEMU and looks for the test markers on the serial console:

* `booting-start-of-test` -- start marker;
* `gnumach-test-success-and-reboot` -- success marker;
* `gnumach-test-failure` -- failure marker.

The QEMU options match the ones the old `tests/user-qemu.mk` used:

| Arch     | QEMU binary             | Extra option        |
| -------- | ----------------------- | ------------------- |
| `i386`   | `qemu-system-i386`      | `-cpu pentium3-v1`  |
| `x86_64` | `qemu-system-x86_64`    | `-cpu core2duo-v1`  |

Common options: `-m 2047 -nographic -no-reboot -monitor none -serial stdio
-boot d`; each test has a 120 s timeout by default.  Because the pack is
validated with NCPUS=2 kernels, each QEMU run is also given `-smp 2`
(`tests/user-qemu.mk` adds `-smp 2` when SMP is enabled).  The number of
QEMU CPUs must match the kernel's `NCPUS`; `--smp N` overrides it for
other kernel configurations, e.g. `--smp 1` for an NCPUS=1 kernel.

## Requirements

* `qemu-system-i386` and `qemu-system-x86_64`
* `grub-mkrescue` or `grub2-mkrescue`, plus `xorriso`
* `grub-file` or `grub2-file`
* GNU `timeout` (coreutils)
* GNU Mach `i386` and `x86_64` kernel images built with `--enable-ncpus=2`

The suite is validated with NCPUS=2 kernels.  Other kernel configurations
should work as well, but NCPUS=2 is what the pack was tested with.  See
the `VERSION` file in each tarball for details.

## Usage

Run both architectures (recommended):

```
./run-all.sh /path/to/gnumach-32 /path/to/gnumach-64
```

The wrapper extracts the two tarballs into a temporary directory, runs
`run-i386.sh` and `run-x86_64.sh`, prints a combined summary and removes
the temporary directory.  Options after the two kernel paths are forwarded
verbatim to both runners, e.g.:

```
./run-all.sh /path/to/gnumach-32 /path/to/gnumach-64 --test hello
./run-all.sh /path/to/gnumach-32 /path/to/gnumach-64 --timeout 60
./run-all.sh /path/to/gnumach-32 /path/to/gnumach-64 --keep
```

Use already extracted packs instead of the tarballs:

```
./run-all.sh /path/to/gnumach-32 /path/to/gnumach-64 \
    --i386-dir /tmp/abi-test-i386 --x86_64-dir /tmp/abi-test-x86_64
```

Run a single architecture directly:

```
tar xzf i386.tar.gz
./abi-test-i386/run-i386.sh /path/to/gnumach-32
./abi-test-i386/run-i386.sh /path/to/gnumach-32 --list
./abi-test-i386/run-i386.sh /path/to/gnumach-32 --test hello --test vm-abi
./abi-test-i386/run-i386.sh /path/to/gnumach-32 --log-dir /tmp/abi-logs
```

Both runners accept `test-foo`, `module-foo` and `foo` spelling for
`--test`.  `test-multiboot` always runs first as a gate; if it fails the
runner aborts without booting anything and exits nonzero with a clear
message.  The exit status is 0 if all selected tests passed, 1 if a test
failed and 2 on usage errors (missing kernel, missing tools, unknown
test).  `--help` documents all options.

## Repository pre-commit hook

The repository's `.githooks/pre-commit` builds both kernels with `mise` and
runs `./run-all.sh` against them, refusing the commit when any test fails.
Enable it once per clone:

```
git config core.hooksPath .githooks
```

Bypass a single commit deliberately with `git commit --no-verify` or
`SKIP_TESTS=1 git commit ...`.  The no-mise fallback builds whatever
`build-64`/`build-32` directories exist and then runs the same pack.

## Using the pack from another branch or worktree

The pack is plain files, so any of these work:

```
# copy it next to another worktree
cp -r abi-test /path/to/other-worktree/

# or extract a tarball anywhere
mkdir -p /tmp/abipack && tar xzf i386.tar.gz -C /tmp/abipack
/tmp/abipack/abi-test-i386/run-i386.sh /path/to/other-worktree/build-abi-32/gnumach
```

No environment variables, no generated files and no repository paths are
required; everything is resolved relative to the runner script.
