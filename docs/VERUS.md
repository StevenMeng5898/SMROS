# Verus

Verus verification is kept separate from the `smros` kernel crate so the ARM64 `no_std` build stays unchanged.

The current verified syscall slice is the standalone proof file at `verification/syscall/src/lib.rs`. It models the overflow-safe address-range helpers and multi-mapping availability predicates used by `src/syscall/syscall.rs`, the pure syscall bridge rules shared by `src/syscall/syscall_handler.rs` and `src/syscall/syscall_dispatch.rs`, and the shared syscall helper logic for Zircon routing, handle/buffer validation, signal updates, wait satisfaction, supported Linux clock IDs, Linux signal/IPC/socket/misc validation, Linux file/dir/fd/poll/stat validation, Zircon time/debug/system/exception validation, and Zircon hypervisor argument validation.

`verification/kernel_objects/src/lib.rs` verifies pure helper logic and modeled state transitions for every `src/kernel_objects/` file: shared types/page rounding, handle lookup/rights masking, VMO range checks, VMAR range availability, channel limits/signals, thread state predicates, scheduler selection, compatibility-object table predicates, and the no-algorithm module wiring in `mod.rs`.

`verification/kernel_lowlevel/src/lib.rs` verifies pure helper logic for every `src/kernel_lowlevel/` Rust file: memory segment/page arithmetic, process lookup predicates, bitmap allocator indexing, page-table-entry bit updates, UART/GIC/timer register computations, SMP CPU-id/PSCI predicates, and the no-algorithm driver/module wiring.

`verification/user_level/src/lib.rs` verifies pure helper logic for `src/main.rs` and every `src/user_level/` Rust file: the kernel bump allocator alignment/end computation, EL0 process memory-layout arithmetic, user process lookup predicates, shell input/decimal parsing/time and memory summaries, minimal ELF header/program-header/segment bounds checks, `/svc` fixed-message protocol predicates, and the boot-time EL0 syscall-test decision rules.

Commands:

- `make verus-setup`
- `make verus-syscall`
- `make verus-kernel-objects`
- `make verus-kernel-lowlevel`
- `make verus-user-level`

The setup script downloads the pinned Verus release into `.tools/verus/current` and installs the Rust toolchain requested by that Verus release.

Recent verification smoke coverage includes:

- syscall bridge layout and routing predicates
- Linux memory range and mapping predicates
- Linux signal, sigset, SysV IPC, socket, memfd, getrandom, and close-range predicates
- Linux open flags, directory detection, fd targets, pipe/dup/fcntl flags, unlink/rename/stat/statx masks, lseek whence, iov bounds, poll bounds/events, and copy flags
- Zircon signal/wait/object predicates
- Zircon clock, timer, debuglog, system event, and exception option predicates
- Zircon guest trap, VCPU entry, interrupt vector, and VCPU state buffer predicates
- user-level component/FxFS predicates, including FxFS append/write-end/seek/replay-count checks, `/svc` service/protocol predicates, and minimal ELF loader bounds predicates
