# TODO

Open items from the NCPUS=2 ABI-test work that fixed the i386
`test-host-abi` hang (commits `c4498541`, `6a6281be`, `04a6d343` on the
`cleanup` branch).  Most of the kernel bugs below are documented in the
header comment of the test that found them; this file is the index.

## Scheduler

- [ ] Republish the action thread when a CPU shuts down.  The idempotence
  guard `already_scheduled()` in `rust/src/kern/sched_prim.rs` counts a
  thread active on the current CPU, so the `thread_dispatch(this_thread)`
  in `switch_to_shutdown_context` (`i386/i386/cswitch.S:131`) no longer
  reaches `thread_setrun`, and the action thread is lost after
  `halt_cpu()`.  Unreachable with 2 CPUs, where no valid action follows
  the exit; reachable with 3 or more: the next
  `processor_exit`/`processor_assign` request queues on `action_queue`
  and never runs.  Fix idea: skip only when the thread is active on a CPU
  other than the caller's.  An earlier attempt to exclude the current CPU
  reintroduced roughly 10% i386 `test-host-abi` timeouts, so it needs a
  careful retry; verify with a 20x i386 soak and a 3-vCPU pack run.
- [ ] Implement `x86_64/cswitch.S:switch_to_shutdown_context`; it is a
  `ud2` today, so a CPU shutdown traps and the action thread is lost on
  x86_64 too.  The suite passes only because nothing checks that CPU1
  went offline or issues a second action.
- [ ] Finish `kern/sched_prim.c`.  The wait/wake set, `thread_setrun` and
  `thread_dispatch` are Rust now (`rust/src/kern/sched_prim.rs`,
  `rust/src/kern/thread.rs`, `rust/src/kern/processor.rs`);
  `thread_invoke`, `thread_block`, `thread_select`, continuations and
  stack handoff remain C (see `MIGRATE.md` section 4).  `clear_wait` and
  `thread_setrun` deliberately diverge from the C transitions there; keep
  that divergence documented.

## Kernel bugs found by the ABI suite

- [ ] `vm_allocate_contiguous` panics above 4 MiB: `1 << (order +
  PAGE_SHIFT)` in `int` (`vm/vm_user.c:651`) and the buddy assert at
  `vm/vm_page.c:714`; it also asserts paddr against pmin/pmax instead of
  returning an error.
- [ ] `vm_region_create_proxy` dereferences `entry->object.vm_object`
  without a null check (`vm/vm_map.c:5109`, an untouched anonymous mapping
  panics); `vm_object_pager_create` panics rather than failing, so the
  error returns below it are unreachable (`vm/vm_map.c:5122`).
- [ ] `device_intr_register` reports a duplicate registration as
  `D_NO_MEMORY` (`rust/src/device/ds_routines.rs:979`, `device/intr.c:186`)
  and queues the entry before `install_user_intr_handler` can fail
  (`device/intr.c:201`), risking an unbalanced reference.
- [ ] i386 user LDT: installing descriptors on a running x86_64 thread
  truncates the base; an unaligned out-of-line `i386_set_ldt` installs
  zeroed descriptors; an out-of-line `i386_get_ldt` with more than 256
  descriptors returns uninitialized memory and overruns the inline buffer
  (`i386/i386/user_ldt.c:335,363`).
- [ ] `i386_io_perm_modify`: the generated server stub deallocates a null
  or non-io_perm port, which panics on a device port.
- [ ] `task_set_emulation_vector`: count 0 panics in `vm_map_delete`
  (`kern/syscall_emulation.c:330`); an unbounded count is `memset`
  without a null check (`kern/syscall_emulation.c:261`); `EML_BAD_CNT`
  is never returned.
- [ ] `gsync_requeue` miss path reads an uninitialized `int exact`
  (`kern/gsync.c:493,496`).

## Test harness

- [ ] `tests/include/mach/mig_support.h:66` `mig_strncpy` returns 0,
  which truncates long MIG name arguments; the tests work around it with
  short names.
- [ ] `abi-test/` (untracked) holds the frozen per-arch packs.  Refresh
  it after any kernel ABI change; the runners default to `-smp 2` and are
  validated with `--enable-ncpus=2`.
