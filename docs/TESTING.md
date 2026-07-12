# Testing SMROS

SMROS uses layered tests because the kernel is a bare-metal multi-architecture
binary while much of its policy logic is pure Rust. The default production
target is ARM64/AArch64, and the same Makefile flow also builds and smokes the
RISC-V64 and x86_64 targets.

## Fast Unit Tests

Run:

```bash
make ut
```

This executes the host-side crate in `tests/host`. It tests pure shared logic
from the `*_logic_shared.rs` files on the Rust host target, including address
range validation, syscall guard logic, kernel-object helpers, FIFO arithmetic,
scheduler policy helpers, low-level page-table helpers, and user-service/ELF
metadata checks.

The host target is selected explicitly so the root `.cargo/config.toml` can keep
pointing normal builds at the bare-metal default target.

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

## System Smoke Test

Run:

```bash
make st
```

This builds the kernel, starts QEMU in non-interactive mode, captures serial
output in `target/smros-smoke-qemu.log`, and passes when the `smros:/>` prompt is
seen.

Useful overrides:

```bash
SMROS_ST_TIMEOUT=90 make st
SMOKE_QEMU_SMP=1 SMOKE_QEMU_MEMORY=256M make st
SMROS_ST_LOG=/tmp/smros.log make st
make st ARCH=riscv64gc-unknown-none-elf
make st ARCH=x86_64-unknown-none
make st ARCH=aarch64-unknown-none QEMU_CPU_AARCH64=cortex-a57
```

`make st` requires `qemu-img` plus the QEMU system binary for the selected
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

`make test` runs scoped formatting checks, script syntax checks, unit tests, and
the kernel build test. It intentionally does not boot QEMU, so it stays suitable
for quick local and CI checks. Use `make st` for the boot-level smoke test, or
`make verify` for unit tests, build, system smoke, and Verus verification.

## Test Layers

- Hygiene: host-test formatting and shell syntax checks.
- UT: host unit tests for deterministic pure logic.
- Build test: production `aarch64-unknown-none` release build plus raw image by
  default; use `ARCH=riscv64gc-unknown-none-elf` for the RISC-V64 ELF payload or
  `ARCH=x86_64-unknown-none` for the x86_64 PVH ELF payload.
- ST: QEMU boot smoke test that validates the selected architecture's serial
  boot path reaches the shell.
- Verus: proof harnesses for selected syscall, kernel-object, low-level,
  user-level, and service logic.

Future higher-value additions are a serial command runner that sends `testsc`
and `lvgl test` inside QEMU, fixture-based ELF loader tests, and a small CI
workflow that runs `make test` on every change and `make verify` on scheduled
or protected-branch runs.
