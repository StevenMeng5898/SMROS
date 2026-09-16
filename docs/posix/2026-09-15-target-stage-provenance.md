# Target-specific POSIX stage selection

## Reported failure and root cause

RISC-V `posixtest all` rejected its manifest with `invalid-provenance`.
The RISC-V executable itself contained `meta architecture aarch64`:
`build.rs` embedded `host_shared/posixtest` without considering `TARGET`.
Temporarily swapping stage directories only worked until the next kernel
rebuild. The failure was not evidence of FxFS corruption.

The build now embeds exactly one POSIX subtree:

| Target | Default host stage |
| --- | --- |
| AArch64 | `host_shared/posixtest` (unchanged) |
| RISC-V64 | `target/posix/riscv64/stage` |
| x86-64 | `target/posix/x86_64/stage` |

`SMROS_POSIX_STAGE` overrides the stage for both kernel builds and POSIX
CLI commands. Relative overrides are resolved from the repository root.
The selected stage replaces the entire embedded `posixtest/` subtree;
unrelated host-share files remain unchanged. A missing, malformed-header,
or wrong-architecture stage fails the build rather than reaching the guest.
Cargo watches the selection environment and the selected stage files.
Guest checksum/provenance validation remains unchanged.

`build --arch` can now omit `--stage` to use the matching default.
`make posix-build`, `posix-stage`, `posix-baseline`, and `posix-run` follow
`ARCH`. Use `POSIX_SYSROOT` for a nonstandard baseline sysroot. The existing
ARM64 `AARCH64_SYSROOT` override still works. `posix-report` remains ARM64
by default; use the report CLI with explicit paths for RISC-V reports.

The preserved RISC-V stage was moved (not rebuilt or copied) from
`target/posix/riscv64-patched-stage` to `target/posix/riscv64/stage`.
Its manifest SHA-256 is
`fd46bc677eaae24d97e70e0daec9247dfba840b8c1aa4b4408df7e5785029e31`.
No disk reset, persisted-file deletion, or default disk-path change is part
of this fix.

## Verification

- Seven new behavioral regression tests pass. They execute the real build
  script and compile its generated `include_bytes!` snapshot; the old build
  reproduced the wrong-architecture bytes before implementation.
- All 192 Rust host integration contracts pass, including the RISC-V
  floating-point return-state regression.
- RISC-V builds successfully (21 existing warnings). ARM64 passes
  `make aarch64-warning-check`, including the linker-layout check.
- Both rebuilt binaries contain only their expected manifest architecture.
- On one isolated 128 MiB FxFS disk, WIFEXITED/1-1 passes through four
  successive boots: RISC-V, RISC-V, ARM64, RISC-V (four QEMU CPUs).
  Logs: `target/posix/stage-selection-boot-{0,1,2,3}`.
- Workspace usage: 4.6 GiB, below the user's 10 GB limit.

## Remaining work (not passing-suite evidence)

The full Python tooling run passed 575 of 576 tests. The unchanged
`test_riscv_timer_trap_checks_preemption_after_accounting` still expects
two assembly calls instead of the existing Rust timer wrapper.

The first RISC-V campaign also exposed a separate user-state bug. The
reported `scause=2` words decode to valid `fsd`/`fld` instructions in the
RISC-V dynamic loader. SMROS was returning to user mode with
`sstatus.FS=Off`, so the loader was terminated with SIGILL-derived exit 139.
RISC-V user entry, trap return, and fork/clone return now mark the floating
point state usable. Focused guest checks pass for WIFEXITED/1-2 and
aio_return/3-2, and a native SMP=4 `posixtest api WIFEXITED` suite passes
all three cases.

A bounded native RISC-V `posixtest all` checkpoint then completed 297 test
results before the six-minute safety limit: 296 pass and one upstream
untested, with zero fail, unresolved, unsupported, or illegal-instruction
events. It has no `suite_end` and is not a completed 1600-case campaign.
Evidence: `target/posix/riscv64/fp-native-all/serial.log`.

The separate host-controlled, single-CPU campaign was also stopped; its
progress checkpoint contains 421 attempts (419 pass, one untested, one
crash). Its `results.ndjson` is empty and must not be reported as a complete
run. Checkpoint: `target/posix/riscv64/provenance-all/progress.json`.
These runtime failures need investigation independently of stage selection.
