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
`make posix-tool-test` is the current syscall/POSIX unit/integration gate while
`tests/host` is uncompilable, `make skt` is the fast guest smoke test (SKT),
and official POSIX remains host-controlled `make posix-run`. `make st` remains
a compatibility alias for `make skt`.

Run the constrained host-launcher protocol tests separately with:

```bash
make launcher-test
```

The protocol accepts only named `ut` and `it` jobs. `hermes test-all` does not
request ST/SKT: a nested command-line SMROS boot from the guest cannot run.
`make skt` remains the host-only QEMU smoke test. `hermes test-all` runs its
native check once, then one random operation and the `ut`/`it` host jobs in
every iteration. Its positive `iterations=<n>` value therefore controls both
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

Run all currently wired Verus proof harnesses:

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
