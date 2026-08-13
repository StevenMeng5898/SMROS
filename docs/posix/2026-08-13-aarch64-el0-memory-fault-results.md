# AArch64 EL0 Memory-Fault Results

Campaign date: 2026-08-13

## Scope And Outcome

This increment routes AArch64 EL0 instruction and data aborts through the
Linux-compatible synchronous signal and process-termination paths. It removes
the unchanged-`eret` fault loop that previously left `mmap/6-2.c` blocked in
`waitpid`, preserves file-backing metadata for `SIGBUS`, and bounds repeated
ordinary `mmap`/`munmap` metadata work.

The target regression passed three independent runs, adjacent `PROT_NONE` and
file-tail signal cases passed, and every focused run completed without a QEMU
restart or positive terminal resource delta. This is affected-surface evidence,
not full POSIX certification. The approved design is not fully accepted:
`mmap/6-1.c` still exposes the pre-existing fork-child execution defect, and
the 100,000-iteration `mmap/10-1.c` stress case still exceeds its 30-second
host watchdog.

No test assertion, exit status, timeout, group, or report disposition was
weakened or remapped. Every QEMU run used a private generated disk below
`target/posix/aarch64/`; the repository-root `smros-fxfs.img` and user-owned
QEMU PID `16622` were not accessed or signaled.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| Implementation commit | `5bb9ae97ac2e5b7dd8d5d357efb154f012c78bc8` |
| Approved design commit | `c9a3ad0219f428fe288110c1f2a1991569addca5` |
| Implementation plan commit | `2315ba42afcd28da5f582bbec53b968aff3e39f2` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Canonical patch SHA-256 | `e54668f51377ac9493d0bbbc607e4fae0d74ba60106dac11aeaa312b59c0f4f6` |
| Canonical manifest SHA-256 | `23dc823a798330899f60e7ac06c46cc809997156e719d734c490c0a237733044` |
| Manifest JSON file SHA-256 | `bfb593f98d97506634e3ee8c16bb8ebd2fa9d428e7761ff86ef0eb0053d7e32d` |
| Manifest TSV file SHA-256 | `2a9f1da80b34894e3877f2e72572873753461cae9227ef5adbb9e55e870ee027` |
| Canonical build-results SHA-256 | `e51e42f9f81bb3deb1d3a84b75390b1a5951d3767fe2f4c79f338c5ba491d901` |
| Build-results file SHA-256 | `23cf88f2ab0fe63290c0f3ac31d55266a2a7cc73b1cc95eabf00f2bf29ea2318` |
| Build ID | `4a2138c1e32dd99208dd26c5458cfe987afcd7e5af9de37ddfa92f671c2289e7` |
| Kernel image SHA-256 | `b273f99c4e2fd80784706902c9c5f3e7fda59c2a152d2cca111146ef2c3ce86e` |

The relevant source tree was clean when the stage and kernel were built. The
only worktree modification was the pre-existing, unrelated generated file
`scripts/__pycache__/smros-vm-launcher.cpython-312.pyc`; it was excluded from
the implementation and evidence commits.

The staged inventory reported 1,979 discovered C sources, 1,941 compile
passes, 38 compile failures, 1,680 link passes, 2 link failures, 169 unported
shell tests, and 119,396,628 staged bytes. The manifest contains 1,598 complete
runnable tests.

## Toolchain

| Tool | Version |
| --- | --- |
| Rust | `rustc 1.96.0-nightly (23903d01c 2026-03-26)`, LLVM 22.1.2 |
| Cargo | `1.96.0-nightly (e84cb639 2026-03-21)` |
| AArch64 GCC | `aarch64-linux-gnu-gcc 13.3.0` |
| QEMU | `qemu-system-aarch64 8.2.2` |
| Python | `3.12.3` |
| Git | `2.43.0` |
| Host | Linux 6.18.33.2 WSL2, `x86_64` |

## Focused Runtime Evidence

Each row is one fresh QEMU process and disk. All ten runs had one boot, zero
restarts, `launch_status=launched`, no watchdog, exact provenance, measured
resource evidence, and zero deltas for AIO requests, IPC objects, kernel
handles, Linux descriptors, mappings, processes, shared-memory objects,
zombies, private/shared pages, page-table pages, scheduler threads, native
processes, and timers.

| Test | Status | Exit | Duration | Guest diagnostic |
| --- | --- | ---: | ---: | --- |
| `mmap/6-2.c` run 1 | pass | 0 | 111 ms | observed `SIGSEGV` reading `PROT_NONE` |
| `mmap/6-2.c` run 2 | pass | 0 | 109 ms | observed `SIGSEGV` reading `PROT_NONE` |
| `mmap/6-2.c` run 3 | pass | 0 | 110 ms | observed `SIGSEGV` reading `PROT_NONE` |
| `mmap/6-1.c` | fail | 1 | 107 ms | child did not observe `SIGSEGV` writing a non-writable mapping |
| `mmap/6-3.c` | pass | 0 | 104 ms | observed `SIGSEGV` writing `PROT_NONE` |
| `mmap/11-2.c` | pass | 0 | 104 ms | installed handler observed `SIGBUS` |
| `mmap/11-3.c` | pass | 0 | 100 ms | installed handler observed `SIGBUS` |
| `WIFEXITED/1-1.c` | pass | 0 | 47 ms | `Test PASSED` |
| `WIFEXITED/1-2.c` | pass | 0 | 47 ms | `Test PASSED` |
| `fork/1-1.c` | fail | 1 | 414 ms | `The new process does not execute` |

The `mmap/6-1.c` diagnostic and independent `fork/1-1.c` canary both leave
fork/process behavior open: `6-1.c` reports a zero child exit without the
required protection signal, while `fork/1-1.c` reports that its child did not
post the expected semaphore. Neither is a fault-loop timeout, signal-delivery
crash, or resource leak. Host wire-layout tests separately verify the AArch64
`siginfo_t` signal/code/fault address and saved `ucontext_t` register state
supplied to `SA_SIGINFO` handlers.

Focused artifacts are under:

```text
target/posix/aarch64/el0-memory-fault-5bb9ae97ac2e-run-6-2-final-*/
target/posix/aarch64/el0-memory-fault-5bb9ae97ac2e-run-canary-final-*/
```

## Complete Mmap Selection

The complete selection terminated all 33 manifest-selected tests in two boots
with one controlled watchdog restart:

| Status | Count |
| --- | ---: |
| pass | 15 |
| fail | 16 |
| timeout | 1 |
| untested | 1 |
| selected / terminal | 33 / 33 |

All 32 guest-terminal attempts had measured resource evidence and no positive
delta. `mmap/10-1.c` timed out at 30,001 ms and therefore has unavailable
terminal non-leak evidence, as required by the result schema. Its retained
profiling run reached approximately 40,000 of 100,000 map/unmap iterations in
30 seconds, with time distributed across source lookup, mapping, and unmapping;
it showed forward progress and no repeated abort. The timeout triggered the
single runner restart. The optimized `mmap/24-1.c` now passes in 1,948 ms after
mapping 45,664,256 bytes, replacing its earlier address-space-exhaustion
timeout.

The 16 failures cover fork-child execution, file lifetime and timestamp
semantics, error-code differences, shared-file writeback, access checks, and
large-range overflow handling. They remain genuine Open POSIX Test Suite
failures. `mmap/27-1.c` remains genuinely untested. The serial log contains no
kernel panic, fatal synchronous exception, stale-root diagnostic, allocator
corruption, or unchanged-fault-loop marker.

| Artifact | SHA-256 |
| --- | --- |
| `target/posix/aarch64/el0-memory-fault-5bb9ae97ac2e-run-mmap-full-final/results.ndjson` | `00e5236e692931838d616e968d6ac97e41895ee4504e3c2f65d90f04444f1c2f` |
| `target/posix/aarch64/el0-memory-fault-5bb9ae97ac2e-run-mmap-full-final/qemu-serial.log` | `57878a87c34076536a888efa17e42373debbde0853cfcc86bcb41fd043a5bb41` |
| `target/posix/aarch64/el0-memory-fault-diagnostic-profile-run-10-1/qemu-serial.log` | `56ab7ea7500bbfb164c91ddc539bcae21e91c54939079d1fafaae7ec315fa1e1` |

## Deterministic And Formal Gates

| Gate | Result |
| --- | --- |
| Formatting, shell syntax, launcher, linker layout | pass; launcher 4/4 and linker-layout 8/8 |
| Host Rust unit tests | pass; 282/282 |
| Host Rust integration contracts | pass; 103/103 |
| POSIX Python tooling tests | pass; 474/474 |
| AArch64 warning-denied release build | pass; zero Rust warnings, layout check pass |
| Kernel-lowlevel Verus | pass; 132 verified, 0 errors |
| Syscall Verus | pass; 279 verified, 0 errors |
| Verus source-coverage audit | pass |
| `git diff --check` | pass |

Host test crates intentionally include shared modules broader than each test
binary and therefore emit dead-code warnings. The production AArch64 release
gate promotes every Rust warning to an error and completed with zero warnings.
Logs and exit markers are retained under
`target/posix/aarch64/el0-memory-fault-quality/`.

## Coverage And Static Analysis

Host line coverage is **unavailable** because `cargo-tarpaulin` is not
installed. No percentage is claimed. The exact status is in
`el0-memory-fault-quality/coverage-host.exit` and the diagnostic is in
`coverage-host.log`.

Coverity is **unavailable** because `cov-build`, `cov-analyze`, and
`cov-format-errors` are not installed. No defect count or zero-defect claim is
possible. The exact status is in `el0-memory-fault-quality/coverity.status` and
the missing-command list is in `coverity.log`.

## Acceptance Audit

| Approved criterion | Evidence | Disposition |
| --- | --- | --- |
| `mmap/6-2.c` terminates and observes child `SIGSEGV` | three passes, 109-111 ms, no restarts | met |
| `mmap/6-1.c` and `6-3.c` report protection faults | `6-3.c` passes; `6-1.c` reaches a truthful fork-related failure | not fully met |
| Correct handler signal, address, and saved context | `11-2.c`/`11-3.c` runtime passes plus host wire-layout tests | met for affected surface |
| File-tail `SIGBUS`; mapping/permission `SIGSEGV` | runtime canaries plus pure policy tests | met |
| Parent wait completes and measured resources return | target and wait canaries terminate; every measured focused delta is zero | met |
| Current-EL remains fatal; no unchanged fault-loop return | warning build, vector contracts, and runtime log audit | met |
| All specified repository and AArch64 runtime gates pass | repository/formal gates pass; full mmap selection retains one timeout | not met |

The remaining acceptance failures are carried forward. This document records a
verified AArch64 EL0 fault-delivery increment and its limitations; it does not
claim that SMROS is 100% POSIX compliant.
