# Testing SMROS

SMROS uses layered tests because the kernel is a bare-metal AArch64 binary while
much of its policy logic is pure Rust.

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
pointing normal builds at `aarch64-unknown-none`.

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
```

`make st` requires `qemu-system-aarch64` and `qemu-img`.

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
- Build test: production `aarch64-unknown-none` release build plus raw image.
- ST: QEMU boot smoke test that validates the serial boot path reaches the shell.
- Verus: proof harnesses for selected syscall, kernel-object, low-level,
  user-level, and service logic.

Future higher-value additions are a serial command runner that sends `testsc`
inside QEMU, fixture-based ELF loader tests, and a small CI workflow that runs
`make test` on every change and `make verify` on scheduled or protected-branch
runs.
