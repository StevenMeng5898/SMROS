# Verus

Verus verification is kept separate from the `smros` kernel crate so the ARM64 `no_std` build stays unchanged.

The current verified slice is the standalone proof file at `verification/syscall/src/lib.rs`. It models the overflow-safe address-range helpers and multi-mapping availability predicates used by `src/syscall/syscall.rs`, plus the pure syscall bridge rules shared by `src/syscall/syscall_handler.rs` and `src/syscall/syscall_dispatch.rs`.

`verification/kernel_objects/src/lib.rs` verifies pure helper logic and modeled state transitions for every `src/kernel_objects/` file: shared types/page rounding, handle lookup/rights masking, VMO range checks, VMAR range availability, channel limits/signals, thread state predicates, scheduler selection, and the no-algorithm module wiring in `mod.rs`.

`verification/kernel_lowlevel/src/lib.rs` verifies pure helper logic for every `src/kernel_lowlevel/` Rust file: memory segment/page arithmetic, process lookup predicates, bitmap allocator indexing, page-table-entry bit updates, UART/GIC/timer register computations, SMP CPU-id/PSCI predicates, and the no-algorithm driver/module wiring.

Commands:

- `make verus-setup`
- `make verus-syscall`
- `make verus-kernel-objects`
- `make verus-kernel-lowlevel`

The setup script downloads the pinned Verus release into `.tools/verus/current` and installs the Rust toolchain requested by that Verus release.
