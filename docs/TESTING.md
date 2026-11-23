# Testing SMROS

SMROS uses layered tests because the kernel is a bare-metal multi-architecture
binary while much of its policy logic is pure Rust. The default production
target is ARM64/AArch64, and the same Makefile flow also builds and smokes the
RISC-V64 and x86_64 targets.

## POSIX Harness

The Open POSIX Test Suite workflow is documented in
`docs/POSIX_CONFORMANCE.md`. Its current milestone is infrastructure and a
failure baseline, not a POSIX certification or a claim of conformance
completion. Run the offline host-tool checks with:

```bash
make posix-tool-test
```

The network fetch, selected-target cross-build, qemu-user reference,
QEMU/SMROS run, and seven-artifact report are explicit `make posix-*` targets
and are not pulled into ordinary offline testing. The architecture order is
AArch64, then x86_64, then RISC-V64.

## Fast Unit Tests

Run:

```bash
make ut
```

This executes the host-side crate in `tests/host`. It tests pure shared logic
from the `*_logic_shared.rs` files on the Rust host target, including address
range validation, syscall guard logic, syscall bridge register/errno helpers,
kernel-object helpers, FIFO/socket/port/futex arithmetic, scheduler policy
helpers, low-level page-table helpers, hypervisor state helpers, log-level
helpers, and user-service/ELF metadata checks.

The host target is selected explicitly so the root `.cargo/config.toml` can keep
pointing normal builds at the bare-metal default target.

## Integration Tests

Run:

```bash
make it
```

This executes the Cargo integration tests in `tests/host/tests`. These tests
lock cross-module contracts that should agree across subsystems, such as
overflow-safe range arithmetic, FIFO/socket ring-buffer math, shared signal
updates, Linux/Zircon syscall-number routing boundaries, ELF mapping ranges
feeding fixed mmap checks, and Makefile/docs wiring for the test layers.

## Coverage Reports

Run:

```bash
make coverage-host
```

This runs `cargo tarpaulin --out Html --fail-under 100 --include-tests` for the
host-side test crate and writes the source-highlighted HTML coverage report to:

```text
target/coverage/host/tarpaulin-report.html
```

Use narrower reports when you want to inspect only one host layer:

```bash
make coverage-ut
make coverage-it
make coverage-st
```

Those write to `target/coverage/ut/tarpaulin-report.html` and
`target/coverage/it/tarpaulin-report.html`. `make coverage-st` writes the QEMU
serial smoke report to `target/coverage/st/index.html` and the raw serial log to
`target/coverage/st/smros-smoke-qemu.log`.

Run the full coverage/smoke view with:

```bash
make coverage
```

`make coverage` generates the combined host UT/IT Tarpaulin HTML report and the
QEMU `skt` smoke HTML/log report. The host reports are hard-gated at 100%.
Tarpaulin measures the Rust host tests; the QEMU `skt` layer is reported as
100% required-serial-milestone coverage when every boot milestone is found, not
guest line coverage. To get bare-metal guest line coverage, SMROS would need a
separate kernel/QEMU instrumentation path.

If `cargo-tarpaulin` is missing, install it with:

```bash
cargo install --locked cargo-tarpaulin
```

## Hygiene Checks

Run:

```bash
make host-fmt-check
make script-check
```

`host-fmt-check` checks formatting for the host-side unit-test crate.
`script-check` runs `bash -n` over the shell scripts in `scripts/`.

## Kernel build parallelism

Kernel builds use `SMROS_BUILD_JOBS` for both Cargo's jobserver (`cargo -j`)
and the kernel crate's LLVM codegen units
(`CARGO_PROFILE_RELEASE_CODEGEN_UNITS` and `CARGO_PROFILE_DEV_CODEGEN_UNITS`).
The default is 4. `panic = "abort"`, `opt-level = "s"`, `lto = false`, and
incremental compilation stay as they are. Host tests in `tests/host` keep
their own Cargo profile; `make ut` and `make it` only pass `-j`.

`make build` prints the chosen value once:

```text
SMROS build jobs: 4 (cargo -j 4, codegen-units 4)
```

Override it for a build or a QEMU run:

```bash
make build SMROS_BUILD_JOBS=8
make run SMROS_BUILD_JOBS=8
```

`./scripts/build.sh` reads the same variable. `./scripts/run.sh` and
`./scripts/run-simple.sh` boot a kernel image that is already built.

## Build Test

Run:

```bash
make build-test
```

This checks that the production kernel still builds and emits `kernel8.img`.
By default that means `ARCH=aarch64-unknown-none`. To check the RISC-V64 or
x86_64 kernel build instead, run:

```bash
make build-test ARCH=riscv64gc-unknown-none-elf
make build-test ARCH=x86_64-unknown-none
```

The RISC-V64 build emits `target/riscv64gc-unknown-none-elf/release/smros`,
which QEMU loads directly as an ELF payload.
The x86_64 build emits `target/x86_64-unknown-none/release/smros`, which QEMU
loads as a PVH ELF payload.

## AArch64 Warning Gate

Run:

```bash
make aarch64-warning-check
```

This performs the optimized AArch64 kernel build and link-layout check with
Rust warnings promoted to errors. Normal AArch64 `make build` invocations use
the same policy. x86_64 and RISC-V64 warning policy is unchanged until their
separate cleanup milestones.

## System Smoke Test

Run:

```bash
make skt
```

This builds the kernel, starts QEMU in non-interactive mode, captures serial
output in `target/smros-smoke-qemu.log`, sends `testsc`, `fuzzsc seed=1
iterations=1`, `hermes random seed=1 iterations=1`, and `hermes exec kill`,
and passes when the syscall smoke and bounded fuzz complete, the safe campaign
completes, `kill` is denied, and the required boot milestones are seen.
Hermes owns the `reboot` skill (`hermes exec reboot`) and the `cmd` skill for
VM/Docker lifecycle commands; smoke does not execute them. When a Hermes campaign
does reboot, it persists `/data/hermes/tests/resume.cfg` and continues remaining
iterations after reset. Reboot-round host `ut`/`it` jobs run after that resume
once the guest network can reach the host launcher. The ops catalog includes
`vm -c /shared/vm-demo.xml` so a nested GTK QEMU Linux window can appear during
`hermes test-all`. Campaign create waits until guest Linux boot OK, then
`vm -k linux-demo` recycles the nested QEMU process. If nested QEMU never
launched, the campaign skips the boot wait and still recycles. The Hermes skills
`ut`, `it`, `skt`, `fuzzing`, `reboot`, and `cmd` map onto this stack:
`make ut` and `make it` compile and run `tests/host`. `make posix-tool-test`
is the offline POSIX host-tool gate and does not replace those host tests.
`make skt` is the fast guest smoke test (SKT),
and official POSIX remains host-controlled `make posix-run`. `make st` remains
a compatibility alias for `make skt`.

Run the constrained host-launcher protocol tests separately with:

```bash
make launcher-test
```

The protocol accepts a closed set of named jobs: per-iteration `ut` and `it`, plus once-per-campaign `el0`, `skt`, `st`, `posix-tool-test`, `posix-fetch`, `posix-audit`, `posix-build`, `posix-stage`, `posix-baseline`, `launcher-test`, and `verus`. `make posix-run` and `make posix-report` stay host commands outside this set. It still rejects arbitrary command text. `SMROS_TEST_RUN` returns a token immediately and `SMROS_TEST_STATUS` is polled until the host job finishes, so a long `make ut` cannot stall the guest TCP stack. Guest `tcp_connect` always honors its deadline even when leftover virtio RX frames keep arriving, and it RSTs leftover IPv4 TCP frames (including unmatched SYN-ACKs) so ARM64 slirp delayed packets cannot hang a later host-job handshake. ARM64 `arm_at_nanoseconds` rewrites CNTP_CVAL when the cached compare is already behind CNTPCT, because GICv3/TCG will not raise a new edge from a stale past deadline. Lone-thread `yield_now()` programs a 1ms CNTP wakeup, restores IRQs with ISB, then DSB/ISB/WFI so ARM64 TCG can retire virtio completions and advance CNTPCT. Memory Syscall Test component launchers keep a live TTBR0 instead of installing 0, leftover exception VMARs are closed, leftover virtio RX is drained before host `ut`/`it`, leftover launchers ERET into a user-mapped SVC trampoline at `0x10000000` instead of kernel `.text` (ARM64 UXN/`ESR=8200000e`), and ARM64 `wait_for_event()` WFI so leftover EL0 threads cannot freeze CNTPCT after `testsc`. Failed `it` summaries ignore Make `Leaving directory` lines, including as the last fallback line, and read combined stdout/stderr. `hermes test-all` runs its
native check once, then mandatory `testsc` while the shell thread still owns a Linux brk, then `posixtest all` and `sectest all` (including `sectest undef`, one case id, and a check that every sectest id ran) once while `/dev/shm`
is mode `040755`, then the seven guest ELFs under `tests/guest/` (`f1_seccomp`, `f2_landlock`, `f3_pkey`, `f4_iouring`, `f5_ptrace`, `g1_stopcont`, `g1b_stop`) with exit code 0 required. It then runs one random operation and the `ut`/`it` host jobs in
every iteration. The full POSIX suite is not repeated inside the iteration loop
because it is about 127k scheduler ticks. After the iterations, host stages run once through the launcher: `tests/security/aarch64/run_el0_cases.py`, `make skt`, `make st`, `make posix-tool-test`, `posix-fetch`, `posix-audit`, `posix-build`, `posix-stage`, `posix-baseline`, `make launcher-test`, and `make verus`. `skt` and `st` use `SMROS_HERMES_ISOLATED=1` and a private FxFS image so they do not rebuild over the kernel the live guest has mapped. `make posix-run` and `make posix-report` are not campaign stages: each boots another SMROS to execute the same manifest the opening `posixtest all` already ran, and that second guest's serial protocol cannot share the campaign VM. `posix-baseline` copies `lib/libsmros-posix-compat.so` from the stage `posix-build` produced into its runtime snapshot; the toolchain sysroot is required for `ld-linux`, `libc`, and `libgcc`. The runner does not set `LD_PRELOAD`. The published `pthread_cond_init/1-3` `LD_DEBUG=bindings` log binds `pthread_cond_*` to `libc.so.6`, so this host baseline does not cover the compat library's `pthread_cond_*` and must not be cited as that coverage. The campaign stage passes when that qemu-user reference run finishes and publishes every selected test. Non-passing reference results are counted as `posix-baseline=<passed>/<nonpass>/0` and do not fail the campaign: this milestone is a failure baseline, and the SMROS gate remains in-guest `posixtest all` at 1600/1600. `make posix-baseline` itself still exits non-zero unless every reference test passes. A harness error, including a missing staged compat library, still fails the stage. Bounded `fuzzsc seed=1 iterations=1 time=1` then runs in the guest. The
`shm-mode` phase runs another bounded `fuzzsc seed=1 iterations=1 time=1` and requires
`/dev/shm` to still be mode `040755`. Setattr of `/`, `/dev`, and `/dev/shm`
rejects any other mode, so `fuzzsc` cannot persist mode `0777` and leave a
later `posixtest all` unresolved. `posixtest=skip`, `sectest=skip`, and `<stage>=skip` omit that stage; the default runs every stage. A missing guest ELF or a launcher that cannot run a host stage is `UNAVAILABLE` and fails the campaign, except Verus: a missing Verus toolchain, or a coverage audit that stops `make verus` before any harness runs, is `UNAVAILABLE` in the summary and does not by itself fail the campaign, while a Verus proof failure does. Phase counts are stored in the reboot checkpoint
and in `/data/hermes/tests/latest.log`. Any new test item must also be orchestrated into `hermes test-all` in the same change.
The default catalog already includes LVGL, QML, syscall
smoke, POSIX canaries, and ops APIs. Its positive `iterations=<n>` value therefore controls both
the guest operations and the number of `ut` and `it` requests. Reports keep
aggregate totals and no more than 64 round details. Host logs are bounded
under `target/hermes-tests/`.

Useful overrides:

```bash
SMROS_ST_TIMEOUT=180 make skt
SMOKE_QEMU_SMP=1 SMOKE_QEMU_MEMORY=2G make skt
SMROS_ST_LOG=/tmp/smros.log make skt
SMROS_ST_REQUIRED_PATTERNS='[OK] Kernel initialized successfully!|smros:/>' make skt
make skt ARCH=riscv64gc-unknown-none-elf
make skt ARCH=x86_64-unknown-none
make skt ARCH=aarch64-unknown-none QEMU_CPU_AARCH64=cortex-a57
```

`make skt` requires `qemu-img` plus the QEMU system binary for the selected
architecture: `qemu-system-aarch64` for ARM64 or `qemu-system-riscv64` for
RISC-V64, or `qemu-system-x86_64` for x86_64.

## Verification Harnesses

Run all currently wired Verus proof harnesses, the coverage audit, and the
mutation gate. The gate is also `make verus-mutation`. `hermes test-all` runs
it as the existing `verus` stage.

```bash
make verus
```

Run the fast local confidence suite:

```bash
make test
```

`make test` runs scoped formatting checks, script syntax checks, unit tests,
integration tests, the offline `posix-tool-test`, and the kernel build test. It
intentionally does not fetch sources, cross-build the POSIX suite, run
qemu-user, or boot QEMU, so it stays suitable for quick local and CI checks.
Use `make skt` for the boot-level smoke test, or `make verify` for unit tests,
integration tests, build, system smoke, and Verus verification.

## Test Layers

- Hygiene: host-test formatting and shell syntax checks.
- UT: host unit tests for deterministic pure logic.
- IT: host integration tests for cross-module contracts and test-layer wiring.
- POSIX tool tests: offline Python tests for source, audit, build, runner, and
  report contracts. See `docs/POSIX_CONFORMANCE.md` for the full workflow.
- Coverage: `cargo-tarpaulin` HTML heatmaps for host UT/IT coverage plus an
  optional QEMU smoke run through `make coverage`.
- Build test: production `aarch64-unknown-none` release build plus raw image by
  default; use `ARCH=riscv64gc-unknown-none-elf` for the RISC-V64 ELF payload or
  `ARCH=x86_64-unknown-none` for the x86_64 PVH ELF payload.
- SKT: QEMU boot smoke test that validates the selected architecture's serial
  boot path reaches required milestones and the shell.
- Verus: proof harnesses for selected syscall, kernel-object, low-level,
  user-level, and service logic.

`make skt` now sends `testsc` and bounded `fuzzsc` in addition to the Hermes
safe campaign. Future higher-value additions are fixture-based ELF loader
tests and a small CI workflow that runs `make test` on every change and
`make verify` on scheduled or protected-branch runs.
