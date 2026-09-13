# RISC-V POSIX ABI follow-up — 2026-09-14

This is focused regression evidence, not a complete POSIX conformance claim.

## Changes

- `605e7cd`: retain GLIBC_2.17 aliases while exporting GLIBC_2.27 identity,
  process-scope attribute, and sysconf APIs. The native dynamic-loader regression
  failed for missing bindings before the change and passes afterward.
- `dac6445`, `a633822`: synchronize two stale discovery assertions with the
  existing 600-second clock-volume timeouts; runtime timeout policy is unchanged.
- `06c5a07`: export the existing sigqueue interposer for both target versions.

The current RISC-V stage is `/tmp/smros-riscv-identity-stage`, with manifest digest
`4e811f1d48aed51d79f72eade8d0904f8201fba953d27a5a9750e872cb8f6096`.
The RISC-V kernel at `target/riscv64gc-unknown-none-elf/release/smros` embeds it.
`host_shared/posixtest` remains the preserved AArch64 stage; running `make build`
for RISC-V without swapping the stage would embed that AArch64 stage instead.

## Evidence

- `target/posix/riscv64-dualabi-remaining/results.ndjson`: 26 previously
  untested/unsupported/unresolved cases rerun with `605e7cd`; 23 pass, two
  sigqueue cases fail, one mmap case remains untested. All seven previously
  unsupported process-scope scheduler cases pass.
- `target/posix/riscv64-sigqueue-focus/{0,1}/results.ndjson`: both sigqueue cases
  pass with `06c5a07` (QEMU SMP 4). These are separate focused runs, not an
  updated full-campaign total.
- `target/posix/riscv64-sem-focus/1/results.ndjson`: sem_unlink/2-2.c still
  fails when the child attempts `execl("/bin/ls", ...)`.
- `target/posix/arm64-identity-regression/results.ndjson`: mlock/speculative/12-1.c
  passes with the preserved ARM64 stage. This does not validate an ARM64 stage
  rebuilt from the new C source.
- `make aarch64-warning-check`: passes, including linker-layout verification.
- Combined QEMU-runner, discovery, and dual-ABI host tests: 139 pass.
- Both versions of the new aliases are present in an AArch64 cross-compiled
  compatibility library, compiled with `-Wall -Wextra -Werror`.
- Workspace usage at handoff: about 3.9 GiB; no new worktrees created.

## Remaining work and limits of the evidence

The full post-ABI campaign is being collected in
`target/posix/riscv64-post-abi-all`. It runs the complete selection through the
host controller's per-test shell commands; it does **not** exercise one
uninterrupted guest `posixtest all` command.

Do not infer that generic signal permission checks are implemented from the
two sigqueue passes. The existing C interposer has a PID-1 permission shortcut,
and `sys_rt_sigqueueinfo` currently returns success for signal zero after only
validating process existence. Generic credential ownership checks and tests
that bypass the preload are still needed. An earlier progress message saying
the kernel path was not involved was too strong.

Likewise, generic execve image replacement and a real executable utility are
needed for sem_unlink/2-2.c. Do not add another pathname-specific success stub.
The mmap/27-1.c test explicitly reports PTS_UNTESTED when MAP_FIXED is defined;
do not relabel that result as a runtime pass.

The staged build also records 34 compile failures and 169 unported shell tests
outside the 1,600 runnable selection. Passing that selection alone would not
constitute 100% POSIX compliance.
