# Verus Coverage Audit

This file classifies every `src/*.rs` and `src/*.S` file for verification.
`make verus-coverage` checks that this list stays in sync, that every
`*_logic_shared.rs` file plus `syscall_bridge_shared.rs` and
`linux_runtime_lock_shared.rs` is included by a Verus harness, that each
include is classified as inside or outside `verus!`, that every shared macro
is exercised by verification code except the one explicitly documented
unsupported case, that every inventory line has exactly one label, and that
every macro in a file labeled 已证明 expands inside a `func-details` function
with a non-empty spec in `verification/logs/verus-*.log`.

## Labels

- 已证明: every macro in the file is expanded inside a `verus!` function that appears in that harness's `func-details` and has an `ensures` that constrains the result value. `out.is_some()` alone does not. The tautology check is syntactic: `true`, `x == x`, `x ==> x`, `x ==> true`, `x <= x`, `x >= x`, and `x || !x` either way, including a disjunction that contains one of those or a conjunction made only of them. It does not decide general arithmetic or boolean equivalence, such as `x < x + 1` or `!(x && !x)`. A plain function in the same file is not covered by that label unless it too appears in func-details. Raw JSON is `verification/logs/verus-*.log`. `make verus-coverage` checks this against the JSON; a coverage pass is a proof gate only for files labeled 已证明.
- 待证明: in scope, but the file itself is not fully verified. A shared model may already cover part of it.
- 公理或可信计算基: assembly, exception entry, or a device register path that Verus does not compile.
- 规范假设: a stated assumption of the specification rather than an implementation proof.
- 超出范围: not part of the kernel TCB for this Verus audit.
- 未覆盖: present in the tree and not yet given one of the other labels. This inventory has none.

## Inventory counts

Counted from the classified `src/*.rs` and `src/*.S` entries below.

- 已证明: 17
- 待证明: 110
- 公理或可信计算基: 28
- 规范假设: 0 file entries. The handle-generation wrap clause under `handle_namespace_logic_shared.rs` stays 规范假设 and is not a source file.
- 超出范围: 1 source file (`src/user_level/apps/epoll_coro.rs`). `third_party/posixtest/**` and `target/posix/**` are 超出范围 and are not kernel sources.
- 未覆盖: 0

The three files for scheduling, lifecycle, and syscall dispatch stay 已证明. Counted rows are only macros in those files whose harness postcondition is the full spec or the successor state. Constant unhandled tails, `sched_highest_ready_priority`, and the constant-input reap-once lemmas are not coverage. `linux_critical_handler` and `linux_handler_class` are `dead_code`: the dispatcher does not call them. The audit still lists those two reference rows inside the 120, and they are not kernel critical coverage. Kernel critical coverage is the other 118 rows. The 待证明 count did not drop: the live scheduler, process table, task table, `stopped`, and the syscall `match` stay 待证明. `scheduler_queue_logic_shared.rs` dropped `smros_sched_tick_charge_body` (11 lines). The kernel-objects harness dropped the matching `sched_tick_charge` function (12 lines). The returned priority was never stored.

## 签署（待证明）

签署不是已证明。`verification/verus-mutation-signoff.txt` 里的每个 `lib::` 函数都要在下面逐条写出。mutation 摘要把它们记为 signed-off，不计入 `functions` 和 `killed`。只要还有签署，`make verus-mutation` 的退出码就是 1。

当前没有签署函数。

## Coverage Model

- Verified shared logic: pure helper macro bodies included from `src/` into a
  `verification/*/src/lib.rs` harness and tied to executable wrappers/specs.
- Modeled runtime: stateful kernel/runtime files whose pure obligations are
  mirrored in shared logic or modeled state machines under `verification/`.
  Those files stay 待证明 until the implementation itself is verified.
- Runtime-only: hardware, inline assembly, MMIO, global mutable kernel state,
  or the reset path. Those files are 公理或可信计算基.

## Explicit Exception

- `src/kernel_objects/object_logic_shared.rs::smros_ko_align_up_checked_body`
  - 待证明
  calls Rust `usize::is_power_of_two`; current Verus reports that library method
  as unsupported. Adjacent alignment and range obligations remain covered by
  `ko_page_aligned`, VMAR range predicates, host unit tests, and runtime build
  checks. Remove this exception when the Verus toolchain can specify that method
  cleanly. The file stays 待证明 because of that gap.


## Upstream POSIX suite

These trees are the pinned Open POSIX Test Suite and its generated checkout.
They are not kernel TCB, they are not modified by this repository's proof
work, and the 16 patches under `third_party/posixtest/patches/` stay as they are.

- `third_party/posixtest/**`
  - 超出范围
    Pinned upstream Open POSIX Test Suite sources and the existing patch series. Not kernel TCB. Do not modify or prove these files.
- `target/posix/**`
  - 超出范围
    Generated checkout of that pinned suite. Not kernel TCB. Do not modify or prove these files.

## Classified Source Files

- `src/kernel_lowlevel/ARM64/boot.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/context_shared.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/context_switch.S`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/cpu.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/drivers.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/interrupt.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/mod.rs`
  - 待证明
- `src/kernel_lowlevel/ARM64/serial.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/smp.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/thread.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/timer.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/ARM64/user_address_space.rs`
  - 待证明
- `src/kernel_lowlevel/RISCV64/boot.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/context_switch.S`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/cpu.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/drivers.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/interrupt.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/mod.rs`
  - 待证明
- `src/kernel_lowlevel/RISCV64/serial.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/smp.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/thread.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/RISCV64/timer.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/boot.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/context_switch.S`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/cpu.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/drivers.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/interrupt.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/mod.rs`
  - 待证明
- `src/kernel_lowlevel/X86_64/serial.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/smp.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/thread.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/X86_64/timer.rs`
  - 公理或可信计算基
- `src/kernel_lowlevel/aarch64_exception_logic_shared.rs`
  - 待证明
    Plain functions `aarch64_saved_state_is_el0`, `aarch64_lower_el_sync`, `aarch64_lower_el_action`, and `el0_signal_handler_may_run` are absent from `verification/logs/verus-kernel-lowlevel.log` func-details. The include does not put them in the Verus AST.
- `src/kernel_lowlevel/aarch64_vm_logic_shared.rs`
  - 待证明
    Page-table helpers such as `aarch64_table_descriptor` and `allocate_table` are absent from func-details. The include does not put this file in the Verus AST.
- `src/kernel_lowlevel/kernel_budget.rs`
  - 待证明
    Global charge table and interrupt masking. Pure admission is proven in kernel_budget_logic_shared.rs; this file is not compiled by Verus.
- `src/kernel_lowlevel/kernel_budget_logic_shared.rs`
  - 已证明
    The four macros expand inside `verified_kernel_budget_*`, which are `lib::` func-details entries with postconditions. `scripts/audit-verus-proofs.py` lists `admit_heap`, `admit_process`, `mmap_metadata_bytes`, and `uncharge` in `UNVERIFIED_WRAPPERS`: those wrappers are not the proof. mmap metadata's specified success path is the checked multiply; overflow returns None from that macro.
- `src/kernel_lowlevel/lowlevel_logic.rs`
  - 待证明
- `src/kernel_lowlevel/lowlevel_logic_shared.rs`
  - 待证明
    The file stays 待证明. `ll_pfn_index` and `ll_pfn_address` expand `smros_ll_pfn_index_body` and `smros_ll_pfn_address_body`, which `pfn_index` and `pfn_address` also call. The address postcondition is `pfn_address_spec`, including `None` when `checked_mul` overflows, with no precondition that assumes the product fits. `smros_ll_bitmap_mask_body` is compiled for the kernel target and used by `alloc` and `free`. `verified_page_bit_free` and `verified_page_range_admitted` are the bit test and the `init_range` admission check.
- `src/kernel_lowlevel/memory.rs`
  - 待证明
- `src/kernel_lowlevel/mmu.rs`
  - 待证明
- `src/kernel_lowlevel/mod.rs`
  - 待证明
- `src/kernel_objects/campaign_watch.rs`
  - 待证明
    Deadline predicate is proven in the shared file. Serial printing and system_reset remain the trusted reset path and are not out of scope.
- `src/kernel_objects/campaign_watch_logic_shared.rs`
  - 已证明
    Armed deadline comparison. Serial output and system_reset are not in this file.
- `src/kernel_objects/channel.rs`
  - 待证明
- `src/kernel_objects/compat.rs`
  - 待证明
- `src/kernel_objects/fifo.rs`
  - 待证明
- `src/kernel_objects/fifo_logic.rs`
  - 待证明
- `src/kernel_objects/fifo_logic_shared.rs`
  - 已证明
- `src/kernel_objects/futex.rs`
  - 待证明
- `src/kernel_objects/futex_logic.rs`
  - 待证明
- `src/kernel_objects/futex_logic_shared.rs`
  - 已证明
- `src/kernel_objects/handle.rs`
  - 待证明
- `src/kernel_objects/hypervisor.rs`
  - 待证明
- `src/kernel_objects/hypervisor_logic_shared.rs`
  - 已证明
- `src/kernel_objects/job.rs`
  - 待证明
- `src/kernel_objects/log.rs`
  - 待证明
- `src/kernel_objects/log_logic_shared.rs`
  - 已证明
- `src/kernel_objects/mod.rs`
  - 待证明
- `src/kernel_objects/object_logic.rs`
  - 待证明
- `src/kernel_objects/object_logic_shared.rs`
  - 待证明
    Included by the kernel_objects harness except smros_ko_align_up_checked_body, which calls usize::is_power_of_two and is the documented unsupported exception.
- `src/kernel_objects/port.rs`
  - 待证明
- `src/kernel_objects/port_logic.rs`
  - 待证明
- `src/kernel_objects/port_logic_shared.rs`
  - 已证明
- `src/kernel_objects/process.rs`
  - 待证明
- `src/kernel_objects/right.rs`
  - 待证明
- `src/kernel_objects/scheduler.rs`
  - 待证明
    Live ready bitmap, priority counts, and `pick_*` loops. The bit update, count step, and best-priority candidate test they call are proved in `scheduler_queue_logic_shared.rs`. The timer path calls `smros_sched_time_slice_after_elapsed_body` directly. A returned priority that the tick never stores is not counted.
- `src/kernel_objects/scheduler_logic_shared.rs`
  - 待证明
    Scheduler arithmetic macros expand inside specified harness functions. `record_before_switch`, `confirm_after_switch`, and `take_reclaimable` still perform the atomic CAS. Their accept predicates, and `scheduler_retired_slot_reuse_action`, are proved in `scheduler_queue_logic_shared.rs`.
- `src/kernel_objects/scheduler_queue_logic_shared.rs`
  - 已证明
    Ready-bit word and mask, the single-bit update that leaves other bits alone, the saturating ready-count step, the best-priority candidate test, retirement accept/confirm/take, and retired-slot reuse. `smros_sched_tick_charge_body` was removed (11 lines); the kernel-objects harness removed the corresponding 12-line `sched_tick_charge` function. The timer path uses `smros_sched_time_slice_after_elapsed_body` and does not keep the unused priority word. `sched_highest_ready_priority` is a harness lemma over a vector; the live picker uses the priority counts and is not that function, so the lemma is not coverage. The atomic slots and the `Scheduler` struct stay in the files above.
- `src/kernel_objects/socket.rs`
  - 待证明
- `src/kernel_objects/socket_logic.rs`
  - 待证明
- `src/kernel_objects/socket_logic_shared.rs`
  - 已证明
- `src/kernel_objects/types.rs`
  - 待证明
- `src/kernel_objects/vmar.rs`
  - 待证明
- `src/kernel_objects/vmo.rs`
  - 待证明
- `src/main.rs`
  - 待证明
- `src/main_logic.rs`
  - 待证明
- `src/main_logic_shared.rs`
  - 已证明
- `src/syscall/address_logic.rs`
  - 待证明
- `src/syscall/address_logic_shared.rs`
  - 待证明
    `checked_end`, overlap, window, alignment, and mmap-span macros expand inside specified syscall functions. `user_range_writable` and `user_range_readable` are invoked only outside `verus!` and have no spec.
- `src/syscall/fuzz.rs`
  - 待证明
- `src/syscall/handle_namespace_logic_shared.rs`
  - 待证明
    The Vec slot table is not in the Verus AST. `verified_handle_next_generation` ensures the bumped generation differs from the old one while `gen_max > 1`. `handle_encode_decode_round_trip` checks the prefix `0x4000_0000`, the index, and the generation. `handle_encode_injective` is the injectivity lemma. `lookup` and `close` call `smros_handle_generation_matches_body`.
- `src/syscall/handle_namespace_logic_shared.rs generation wrap`
  - 规范假设
    `next_generation` returns 1 once the value exceeds `0x000f_ffff`. A stale handle can match a later occupant of the same slot after that wrap. Single-step inequality is proved; the wrap alias is not.
- `src/syscall/linux_fork_logic_shared.rs`
  - 待证明
    Included as ordinary Rust for the fork ledger. It is not a Verus proof.
- `src/syscall/linux_futex.rs`
  - 待证明
- `src/syscall/linux_futex_logic_shared.rs`
  - 待证明
    The file's plain futex helpers and waiter methods are absent from func-details. `futex_value_matches` is not counted: its spec is the same equality as the body.
- `src/syscall/linux_mqueue.rs`
  - 待证明
    Syscall wrapper around the queue state machine. Attribute and name predicates are proven in the shared file; the wrapper is not compiled by Verus.
- `src/syscall/linux_mqueue_logic_shared.rs`
  - 待证明
    Attribute bounds and name shape are proven. The Vec/String queue state machine is compiled beside the harness and is not a Verus proof.
- `src/syscall/linux_process.rs`
  - 待证明
- `src/syscall/linux_process_logic_shared.rs`
  - 待证明
    The process table, wait scan, and reparent loops are absent from func-details. Reserve, publish, fork publish, rollback, begin/abort exit, exit, reap, root/dispatch install (Empty to Running), dispatch abandon, and launch-descendant clear call the proved edges in `process_lifecycle_logic_shared.rs`. `stopped` is a bool beside the state enum. Stop and continue do not change that enum and are not in the proved model. `reset` fills every slot and stays 待证明. `process_wait_reaps_once` is a constant-input lemma, not coverage.
- `src/syscall/linux_process_memory.rs`
  - 待证明
- `src/syscall/linux_process_memory_logic_shared.rs`
  - 待证明
    Mapping and shared-page functions in this file are absent from func-details.
- `src/syscall/linux_record_lock.rs`
  - 待证明
- `src/syscall/linux_record_lock_logic_shared.rs`
  - 待证明
    Included beside the syscall harness, not inside `verus!`. Only the overlap and conflict macros are smoked. `LinuxRecordLockTable`, waiters, and normalize are not in the Verus AST.
- `src/syscall/linux_riscv_syscall_context.rs`
  - 待证明
    Unsafe trap-frame read/write stays trusted. Alignment, return PC, and SSTATUS.SPP are proven in the shared predicate file.
- `src/syscall/linux_riscv_syscall_context_logic_shared.rs`
  - 已证明
    Frame alignment, sepc+4 with saturation, and SSTATUS.SPP. Unsafe trap-frame loads and stores are not in this file.
- `src/syscall/linux_runtime_lock_shared.rs`
  - 待证明
    Included beside the syscall harness, not inside `verus!`. The atomic spinlock and `UnsafeCell` guard are not in the Verus AST.
- `src/syscall/linux_shm_cache_logic_shared.rs`
  - 待证明
    Generic &str binary search stays ordinary Rust. The syscall harness proves the same insertion-point search for a strictly increasing u64 key vector.
- `src/syscall/linux_syscall_context.rs`
  - 待证明
- `src/syscall/linux_syscall_context_logic_shared.rs`
  - 待证明
    Included as ordinary Rust for the frame owner table. It is not a Verus proof.
- `src/syscall/linux_task.rs`
  - 待证明
- `src/syscall/linux_task_logic_shared.rs`
  - 待证明
    Task, signal, and sleep functions in this file are absent from func-details. Reserve, publish, block, wake, exit, retire, root/dispatch install (Empty to Runnable), and rollback call the proved edges in `process_lifecycle_logic_shared.rs`. `task_retire_reclaims_once` is a constant-input lemma, not coverage. `smros_linux_task_publish_transition_allowed_body` remains a harness smoke of the same Starting to Runnable edge.
- `src/syscall/mod.rs`
  - 待证明
- `src/syscall/syscall.rs`
  - 待证明
    The handler `match` is not compiled by Verus and does not route on `linux_syscall_handler_class`. The audited-stub arm calls `linux_audited_stub_errno`. The default arms call `linux_unhandled_errno` and `zircon_unhandled_rejected`, which are constant tails in the 待证明 shared file and are not coverage.
- `src/syscall/syscall_bridge.rs`
  - 待证明
- `src/syscall/syscall_bridge_shared.rs`
  - 待证明
    Register and errno macros expand inside specified syscall functions. `smros_saved_regs_ptr_from_el0_sp_body` builds a raw pointer and is only smoked outside `verus!`.
- `src/syscall/process_lifecycle_logic_shared.rs`
  - 已证明
    Process edges include Empty to Running for root and dispatch install, the Reserved/Publishing/Running/Exiting/Zombie path, occupied-slot clear, and visible launch-descendant clear. Task edges include Empty to Runnable, Starting or Runnable rollback to Empty, and the block/wake/exit/retire path. Each counted function's postcondition names the successor. `process_wait_reaps_once` and `task_retire_reclaims_once` take constant tokens and are lemmas, not coverage. `stopped` is outside the enum and is not proved here. The table that stores the tokens stays 待证明.
- `src/syscall/syscall_dispatch.rs`
  - 待证明
    Assembly entry and the register spill. Number classification is proved in `syscall_dispatch_logic_shared.rs`.
- `src/syscall/syscall_handler.rs`
  - 待证明
- `src/syscall/syscall_logic.rs`
  - 待证明
- `src/syscall/syscall_dispatch_logic_shared.rs`
  - 已证明
    `linux_audited_stub_class` ensures `out == linux_audited_stub_class_spec(n)` for the whole stub table (1 ENOSYS, 2 EPERM, 3 harmless, 4 personality, 0 otherwise). `linux_audited_stub_errno` is the live fail-closed errno for that table. `linux_critical_handler` and `linux_handler_class` are `dead_code` reference proofs: nothing in `dispatch_linux_syscall` calls them, so they are not kernel critical coverage. Their ensures are still set equality and `out == linux_handler_class_spec(n)`. The set is exit 93, exit_group 94, waitid 95, futex 98, sched 118-127, kill 129, tkill 130, tgkill 131, setpriority 140, getpriority 141, clone 220, execve 221, wait4 260, sched_getattr 275, clone3 435, fork 1000, vfork 1001. tkill and tgkill are the other live signal-delivery arms next to kill. sched_getattr is the live scheduler read next to sched_getscheduler. nanosleep 101 stays out: it has a real handler and is not in the lifecycle, signal, scheduler, or futex set. The giant match stays 待证明. The constant unhandled tails are not in this file and are not coverage.
- `src/syscall/syscall_logic_shared.rs`
  - 待证明
    Dozens of plain helpers (`linux_exit_status`, clock-id predicates, epoll reject, and others) are absent from func-details. `linux_audited_stub` and `linux_audited_stub_errno` wrap the proved table. `linux_syscall_handler_class` wraps the reference classifier and is not used by the dispatcher. `linux_unhandled_errno` and `zircon_unhandled_rejected` are constant tails defined here; they are not coverage.
- `src/syscall/uaccess.rs`
  - 待证明
    Per-thread SPSR slot and EL0 detection. The range predicate is proven in uaccess_logic_shared.rs.
- `src/syscall/uaccess_logic_shared.rs`
  - 已证明
    `smros_uaccess_el0_range_body` expands inside `verified_el0_user_range`. That proof is only the numeric window: zero length is accepted, and a non-zero length must not overflow and must sit inside `[user_base, user_limit)`. It does not prove that the range is mapped or that the access is permitted. `el0_user_range_plausible` is in `UNVERIFIED_WRAPPERS` and is not the proof.
- `src/user_level/apps/epoll_coro.rs`
  - 超出范围
    EL0 epoll demo. Kernel epoll proofs stay on the syscall side.
- `src/user_level/apps/mod.rs`
  - 待证明
- `src/user_level/apps/user_process.rs`
  - 待证明
- `src/user_level/apps/user_test.rs`
  - 待证明
- `src/user_level/drivers/block.rs`
  - 待证明
- `src/user_level/drivers/ddk.rs`
  - 待证明
- `src/user_level/drivers/demos.rs`
  - 待证明
- `src/user_level/drivers/driver_logic.rs`
  - 待证明
- `src/user_level/drivers/driver_logic_shared.rs`
  - 已证明
    Each macro expands inside a user_level function that is in func-details and has a non-empty spec. `driver_mmio_slot_base_direct` has no overflow precondition. Its postcondition is `Some(base + slot * stride)` when that sum fits and `None` when the multiply or the add overflows. `smros_driver_block_capacity_bytes_body` and `smros_driver_block_range_valid_body` are specified by `driver_block_capacity_bytes_direct` and `driver_block_range_valid_direct`, whose postconditions restate the checked multiply and checked add, including the overflow results.
- `src/user_level/drivers/linux.rs`
  - 待证明
- `src/user_level/drivers/linux_logic.rs`
  - 待证明
- `src/user_level/drivers/linux_logic_shared.rs`
  - 已证明
- `src/user_level/drivers/linux_pci.rs`
  - 待证明
- `src/user_level/drivers/linuxcompat/mod.rs`
  - 待证明
- `src/user_level/drivers/mod.rs`
  - 待证明
- `src/user_level/drivers/net.rs`
  - 待证明
- `src/user_level/drivers/pci.rs`
  - 待证明
- `src/user_level/mod.rs`
  - 待证明
- `src/user_level/services/compat_apps.rs`
  - 待证明
- `src/user_level/services/component.rs`
  - 待证明
- `src/user_level/services/docker_compat.rs`
  - 待证明
- `src/user_level/services/elf.rs`
  - 待证明
- `src/user_level/services/elf_load_logic_shared.rs`
  - 待证明
    Page rounding and segment admission are exercised by Verus. The Vec mapping plan is not a Verus proof.
- `src/user_level/services/fxfs.rs`
  - 待证明
- `src/user_level/services/gemma.rs`
  - 待证明
- `src/user_level/services/hermes_agent.rs`
  - 待证明
- `src/user_level/services/hermes_shell_logic_shared.rs`
  - 待证明
    Included as ordinary Rust. Campaign iteration checks in the services harness are a separate model.
- `src/user_level/services/host_share.rs`
  - 待证明
- `src/user_level/services/html_ui.rs`
  - 待证明
- `src/user_level/services/lvgl.rs`
  - 待证明
- `src/user_level/services/mod.rs`
  - 待证明
- `src/user_level/services/net.rs`
  - 待证明
- `src/user_level/services/perfetto.rs`
  - 待证明
- `src/user_level/services/posix_test.rs`
  - 待证明
- `src/user_level/services/posix_test_logic_shared.rs`
  - 待证明
    Numeric POSIX helpers are proven. The string suffix filter is compiled in the harness module and is not a Verus string proof.
- `src/user_level/services/qml_cluster.rs`
  - 待证明
- `src/user_level/services/run_elf.rs`
  - 待证明
- `src/user_level/services/security_case.rs`
  - 待证明
    Address-window, timeout, and trap-verdict predicates are proven in security_case_logic_shared.rs. The EL0 probe runner is not fully proven and is not out of scope.
- `src/user_level/services/security_case_logic_shared.rs`
  - 已证明
    Trap verdict priority, case timeout, and half-open address window.
- `src/user_level/services/svc.rs`
  - 待证明
- `src/user_level/services/user_logic.rs`
  - 待证明
- `src/user_level/services/user_logic_shared.rs`
  - 待证明
    FxFS and run_elf functions in this file are absent from func-details in both the user_level and services logs.
- `src/user_level/services/user_shell.rs`
  - 待证明
- `src/user_level/services/vm_host.rs`
  - 待证明
