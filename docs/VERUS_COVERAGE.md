# Verus Coverage Audit

This file classifies every `src/*.rs` and `src/*.S` file for verification.
`make verus-coverage` checks that this list stays in sync, that every
`*_logic_shared.rs` file is included by a Verus harness, and that every shared
macro is exercised by verification code except the one explicitly documented
unsupported case.

## Coverage Model

- Verified shared logic: pure helper macro bodies included from `src/` into a
  `verification/*/src/lib.rs` harness and tied to executable wrappers/specs.
- Modeled runtime: stateful kernel/runtime files whose pure obligations are
  mirrored in shared logic or modeled state machines under `verification/`.
- Runtime-only: hardware, inline assembly, MMIO, global mutable kernel state,
  generated build output, or interactive shell code that is validated by build,
  unit, smoke, and modeled helper proofs rather than direct Verus compilation.

## Explicit Exception

- `src/kernel_objects/object_logic_shared.rs::smros_ko_align_up_checked_body`
  calls Rust `usize::is_power_of_two`; current Verus reports that library method
  as unsupported. Adjacent alignment and range obligations remain covered by
  `ko_page_aligned`, VMAR range predicates, host unit tests, and runtime build
  checks. Remove this exception when the Verus toolchain can specify that method
  cleanly.

## Classified Source Files

- `src/kernel_lowlevel/context_switch.S`
- `src/kernel_lowlevel/drivers.rs`
- `src/kernel_lowlevel/interrupt.rs`
- `src/kernel_lowlevel/lowlevel_logic.rs`
- `src/kernel_lowlevel/lowlevel_logic_shared.rs`
- `src/kernel_lowlevel/memory.rs`
- `src/kernel_lowlevel/mmu.rs`
- `src/kernel_lowlevel/mod.rs`
- `src/kernel_lowlevel/serial.rs`
- `src/kernel_lowlevel/smp.rs`
- `src/kernel_lowlevel/thread.rs`
- `src/kernel_lowlevel/timer.rs`
- `src/kernel_objects/channel.rs`
- `src/kernel_objects/compat.rs`
- `src/kernel_objects/fifo.rs`
- `src/kernel_objects/fifo_logic.rs`
- `src/kernel_objects/fifo_logic_shared.rs`
- `src/kernel_objects/futex.rs`
- `src/kernel_objects/futex_logic.rs`
- `src/kernel_objects/futex_logic_shared.rs`
- `src/kernel_objects/handle.rs`
- `src/kernel_objects/hypervisor.rs`
- `src/kernel_objects/hypervisor_logic_shared.rs`
- `src/kernel_objects/job.rs`
- `src/kernel_objects/log.rs`
- `src/kernel_objects/log_logic_shared.rs`
- `src/kernel_objects/mod.rs`
- `src/kernel_objects/object_logic.rs`
- `src/kernel_objects/object_logic_shared.rs`
- `src/kernel_objects/port.rs`
- `src/kernel_objects/port_logic.rs`
- `src/kernel_objects/port_logic_shared.rs`
- `src/kernel_objects/process.rs`
- `src/kernel_objects/right.rs`
- `src/kernel_objects/scheduler.rs`
- `src/kernel_objects/scheduler_logic_shared.rs`
- `src/kernel_objects/socket.rs`
- `src/kernel_objects/socket_logic.rs`
- `src/kernel_objects/socket_logic_shared.rs`
- `src/kernel_objects/types.rs`
- `src/kernel_objects/vmar.rs`
- `src/kernel_objects/vmo.rs`
- `src/main.rs`
- `src/main_logic.rs`
- `src/main_logic_shared.rs`
- `src/syscall/address_logic.rs`
- `src/syscall/address_logic_shared.rs`
- `src/syscall/fuzz.rs`
- `src/syscall/mod.rs`
- `src/syscall/syscall.rs`
- `src/syscall/syscall_bridge.rs`
- `src/syscall/syscall_bridge_shared.rs`
- `src/syscall/syscall_dispatch.rs`
- `src/syscall/syscall_handler.rs`
- `src/syscall/syscall_logic.rs`
- `src/syscall/syscall_logic_shared.rs`
- `src/user_level/apps/mod.rs`
- `src/user_level/apps/user_process.rs`
- `src/user_level/apps/user_test.rs`
- `src/user_level/drivers/block.rs`
- `src/user_level/drivers/driver_logic.rs`
- `src/user_level/drivers/driver_logic_shared.rs`
- `src/user_level/drivers/mod.rs`
- `src/user_level/drivers/net.rs`
- `src/user_level/mod.rs`
- `src/user_level/services/compat_apps.rs`
- `src/user_level/services/component.rs`
- `src/user_level/services/docker_compat.rs`
- `src/user_level/services/elf.rs`
- `src/user_level/services/fxfs.rs`
- `src/user_level/services/gemma.rs`
- `src/user_level/services/hermes_agent.rs`
- `src/user_level/services/host_share.rs`
- `src/user_level/services/html_ui.rs`
- `src/user_level/services/lvgl.rs`
- `src/user_level/services/mod.rs`
- `src/user_level/services/net.rs`
- `src/user_level/services/qml_cluster.rs`
- `src/user_level/services/run_elf.rs`
- `src/user_level/services/svc.rs`
- `src/user_level/services/user_logic.rs`
- `src/user_level/services/user_logic_shared.rs`
- `src/user_level/services/user_shell.rs`
- `src/user_level/services/vm_host.rs`
