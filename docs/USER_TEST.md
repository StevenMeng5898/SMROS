# User Test Harness: Current Behavior

This document explains what the current user test code actually validates.

## Relevant Files

- `src/user_level/apps/user_test.rs`
- `src/user_level/apps/user_process.rs`
- `src/user_level/services/user_shell.rs`
- `src/main.rs`

## Two Different Test Layers Exist

The tree currently contains both:

1. an explicit EL0 syscall smoke helper
2. additional shell-level syscall smoke tests

Those are not the same thing.

## Explicit EL0 Smoke Helper

The normal boot path does not call:

```rust
crate::user_level::user_test::run_user_test();
```

That keeps the shell prompt on the fast path. `run_user_test()` remains available as an explicit helper and currently:

- prints `[EL0]`-prefixed log lines
- prepares a small EL0 stack
- drops into EL0 with `switch_to_el0()`
- runs `user_test_process_entry()`
- issues Linux-style `svc #0` calls for `write`, `getpid`, `mmap`, and `exit`
- resumes at `el0_test_resume()` and validates the EL0-observed and EL1-observed syscall results

This validates the real EL0-to-EL1 syscall trap path when the helper is run. It still uses a lightweight `ttbr0 = 0` setup, so it is not yet a fully isolated userspace process.

## EL0 Helpers

`src/user_level/apps/user_test.rs` contains:

- `linux_syscall()`
- `test_getpid()`
- `test_mmap()`
- `test_write()`
- `test_exit()`
- `user_test_process_entry()`
- `user_busy_loop_entry()`

These helpers back the explicit EL0 smoke path and remain useful for expanding the EL0 coverage.

## What The Explicit EL0 Test Proves

When run, the EL0 helper proves:

- the active exception vector can enter EL1 from EL0 via `svc #0`
- Linux syscall numbers for `write`, `getpid`, `mmap`, and `exit` route through `handle_syscall_simple()`
- syscall results are observed consistently by the EL0 code and the EL1 validation hook
- control can return to EL1 through the validation hook

## What The Explicit EL0 Test Does Not Prove

The explicit test does not prove:

- fully isolated user page tables
- a real per-process userspace address space
- a complete Linux ABI
- a complete Zircon ABI
- complete user-space memory isolation

## The Shell's `testsc` Command

The shell exposes a `testsc` command that acts as an additional smoke test.

It currently:

- performs a lightweight write-style syscall helper call
- directly exercises Linux process/time and memory syscall helpers
- directly exercises Zircon VMO/VMAR, handle/object, signal/wait, port, channel, socket, FIFO, futex, process/thread, time/debug/system/exception, and hypervisor helpers
- directly exercises Linux signal, SysV IPC, socket/networking, misc, file, directory, fd, vector I/O, poll, and stat helpers
- directly checks the minimal component framework, ELF loader metadata, FxFS-shaped object store, and `/svc` fixed-message IPC
- runs ported compatibility smoke targets for a Linux `cat`-style FxFS reader and a Fuchsia `/svc` client
- checks Docker/runc compatibility surfaces for OCI-style config parsing, namespace/mount/seccomp/cgroup syscall modeling, and built-in image metadata
- checks the Gemma, Hermes, LVGL workbench, and Qt/QML cluster service ports

Treat it as a developer smoke test, not as a full syscall compliance suite.

Current successful shell runs include these group completion markers:

```text
[OK] object signal tests completed
[OK] port tests completed
[OK] socket kernel object tests completed
[OK] FIFO kernel object tests completed
[OK] futex tests completed
[OK] time/debug/system/exception tests completed
[OK] hypervisor tests completed
[OK] Linux signal, IPC, misc, and net tests completed
[OK] Linux file, dir, fd, poll, and stat tests completed
[OK] component framework, FxFS, and /svc IPC returned
```

The exact command output can also include compatibility-app, Docker/runc, Gemma,
Hermes, LVGL, and Qt/QML cluster group messages depending on the current smoke
path. The UI service groups render bounded PPM previews so they fit the default
QEMU heap profile.

## VM Launch Smoke Path

The shell `vm -c <config.xml>` path creates a modeled SMROS VM process and,
when the XML includes `<linux kernel="...">`, asks the host-side
`scripts/smros-vm-launcher.py` daemon to open a separate QEMU process/window
for that Linux kernel. This validates the guest-to-host launch bridge and the
SMROS hypervisor monitor bookkeeping. It does not prove that SMROS itself is
executing a hardware-virtualized guest kernel inside EL1.

The normal Makefile and run-script QEMU paths auto-start the launcher through
`scripts/start-smros-vm-launcher.sh`. If `vm -c` reports a timeout, the guest
cannot reach TCP port `7070` on the host. If it reports `launcher denied
request`, check `smros-vm-launcher.log` for the missing Linux kernel/initrd/disk
path.

To run two host-assisted demo VMs at once, use configs with different VM names,
for example `vm -c /config/vm-demo.xml` and `vm -c /config/vm-demo2.xml`.

## Why The Logs Still Say `[EL0]`

The prefixes in `run_user_test()` reflect the intended direction of the project, not the current execution mode.

As the code stands today:

- the kernel initializes user-process scaffolding
- the boot-time syscall test is available as an explicit EL0 helper
- the shell remains in EL1

## What Is Needed For A Full EL0 Process

To convert the current smoke test into a fully isolated user process, the kernel still needs to:

1. build or place executable user code into a user mapping
2. create a real `UserProcess`
3. install TTBR0 page tables for that process
4. set up `SP_EL0`, `ELR_EL1`, and `SPSR_EL1`
5. call `switch_to_el0()`
6. return syscall results through a fully correct EL0 register-frame path

## Bottom Line

The current user test code is useful, but it should be described accurately:

- active boot path: fast shell startup without the EL0 syscall smoke helper
- explicit EL0 helper: real EL0 syscall smoke test with lightweight address-space setup
- shell `testsc`: broader EL1 developer smoke test for syscall helper behavior
- shell `components`/`fxfs`/`svc`: visibility into boot ELF load metadata, FxFS object attributes, directory entries, journal replay state, and fixed-message service IPC counters
- shell `run`: dynamic PIE launch smoke path for FxFS-hosted AArch64 binaries with `/shared/lib` dependencies
- shell `lvgl`/`qmlcluster`/`hui`: native LVGL-style workbench, Qt/QML cluster preview, and Hermes terminal UI surfaces

That distinction matters when evaluating boot logs or shell output.
