# AArch64 Zero-Warning Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the optimized `aarch64-unknown-none` SMROS kernel build succeed with zero warnings and preserve the result with an AArch64-only warning-as-error gate.

**Architecture:** Keep `kernel_lowlevel/mod.rs` as the single owner of the shared AArch64 VM module, delete obsolete kernel-only helpers, and compile pure host/Verus model APIs only when `target_os != "none"`. Enforce the boundary through a Make target that adds `-D warnings` only to AArch64 builds, plus host integration contracts that reject duplicate module inclusion and broad dead-code suppression.

**Tech Stack:** Rust `no_std`, Cargo nightly/build-std, GNU Make, host Rust integration tests, Verus, QEMU AArch64.

---

## File Map

- `Makefile`: apply `-D warnings` to AArch64 builds and expose the reusable
  `aarch64-warning-check` target without changing x86_64 or RISC-V64 policy.
- `docs/TESTING.md`: document the warning gate and its AArch64-only scope.
- `tests/host/tests/integration_contracts.rs`: lock the Make wiring, canonical
  VM module ownership, absence of broad suppression, and removal of obsolete
  runtime helpers.
- `src/kernel_lowlevel/memory.rs`: consume the canonical AArch64 VM module,
  remove the broad dead-code allowance, and delete the superseded legacy
  memory demo/shell surface.
- `src/kernel_lowlevel/aarch64_vm_logic_shared.rs`: retain the host-only root
  PFN inspection method outside bare-metal builds.
- `src/kernel_lowlevel/lowlevel_logic.rs`: remove runtime wrappers whose only
  callers were obsolete memory helpers while preserving the shared macros.
- `src/kernel_lowlevel/ARM64/cpu.rs`: remove the unconsumed TLS setter.
- `src/syscall/address_logic.rs`: remove unused wrapper functions while
  retaining the shared proof/test macros and active runtime wrappers.
- `src/syscall/linux_process_logic_shared.rs`: keep selected process-table
  model methods for host/Verus only.
- `src/syscall/linux_fork_logic_shared.rs`: keep failpoint configuration and
  combined clone/map model helpers for host/Verus only.
- `src/syscall/linux_process.rs`: remove unused resource-clone inspection
  methods.
- `src/syscall/linux_process_memory_logic_shared.rs`: retain live runtime
  primitives and gate only the pure host/Verus process-memory models.
- `src/syscall/linux_process_memory.rs`: remove two unused clone-record fields.
- `src/syscall/linux_task_logic_shared.rs`: keep legacy pure signal/task model
  inspection and transition helpers for host/Verus only.
- `src/syscall/linux_task.rs`: remove the unconsumed crate-private TID lookup
  wrapper.

### Task 1: Add The Strict AArch64 Warning Gate

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `Makefile`
- Modify: `docs/TESTING.md`

- [ ] **Step 1: Write the failing Make wiring contract**

Add this test beside `test_layer_commands_and_docs_are_wired`:

```rust
#[test]
fn aarch64_warning_gate_is_strict_and_target_scoped() {
    let makefile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"));
    let docs = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/TESTING.md"
    ));

    assert!(makefile.contains(
        "AARCH64_RUSTFLAGS = $(strip $(RUSTFLAGS) -D warnings)"
    ));
    assert!(makefile.contains(
        "aarch64-warning-check:\n\t@$(MAKE) build-test ARCH=aarch64-unknown-none"
    ));
    assert!(makefile.contains(
        "RUSTFLAGS='$(AARCH64_RUSTFLAGS)' SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET)"
    ));
    assert!(makefile.contains(
        "SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET)"
    ));
    assert!(docs.contains("make aarch64-warning-check"));
    assert!(docs.contains("x86_64 and RISC-V64 warning policy is unchanged"));
}
```

- [ ] **Step 2: Run the contract and verify it fails**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_warning_gate_is_strict_and_target_scoped --exact
```

Expected: FAIL at the first missing `AARCH64_RUSTFLAGS` assertion.

- [ ] **Step 3: Add target-scoped warning enforcement**

Add this variable after the existing build/test configuration variables:

```make
AARCH64_RUSTFLAGS = $(strip $(RUSTFLAGS) -D warnings)
```

Add `aarch64-warning-check` to `.PHONY`. Replace the Cargo invocation in
`build` with this architecture branch:

```make
	@if [ "$(TARGET)" = "aarch64-unknown-none" ]; then \
		RUSTFLAGS='$(AARCH64_RUSTFLAGS)' SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET); \
	else \
		SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET); \
	fi
```

Add the reusable target after `build-test`:

```make
# AArch64 release build with every Rust warning promoted to an error
aarch64-warning-check:
	@$(MAKE) build-test ARCH=aarch64-unknown-none
```

Add this help entry:

```make
	@echo "  aarch64-warning-check - Build and link-check AArch64 with Rust warnings denied"
```

- [ ] **Step 4: Document the warning gate**

Insert this subsection before `## System Smoke Test` in `docs/TESTING.md`:

````markdown
## AArch64 Warning Gate

Run:

```bash
make aarch64-warning-check
```

This performs the optimized AArch64 kernel build and link-layout check with
Rust warnings promoted to errors. Normal AArch64 `make build` invocations use
the same policy. x86_64 and RISC-V64 warning policy is unchanged until their
separate cleanup milestones.
````

- [ ] **Step 5: Run the contract and verify it passes**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_warning_gate_is_strict_and_target_scoped --exact
```

Expected: PASS.

- [ ] **Step 6: Prove the new warning gate detects the baseline**

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL with `function 'set_user_tls' is never used` promoted to an
error, followed by the remaining AArch64 warning inventory. This is the red
test for the cleanup tasks.

- [ ] **Step 7: Commit the gate**

```bash
git add Makefile docs/TESTING.md tests/host/tests/integration_contracts.rs
git commit -m "build: deny warnings in AArch64 builds"
```

### Task 2: Canonicalize AArch64 VM Ownership And Remove Legacy Memory Code

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/kernel_lowlevel/memory.rs`
- Modify: `src/kernel_lowlevel/aarch64_vm_logic_shared.rs`
- Modify: `src/kernel_lowlevel/lowlevel_logic.rs`

- [ ] **Step 1: Write the failing module-ownership contract**

Add this integration contract near the existing AArch64 allocator test:

```rust
#[test]
fn aarch64_memory_uses_one_vm_logic_module_without_broad_dead_code_suppression() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let modules = std::fs::read_to_string(repository.join("src/kernel_lowlevel/mod.rs"))
        .expect("read kernel low-level modules");
    let memory = std::fs::read_to_string(repository.join("src/kernel_lowlevel/memory.rs"))
        .expect("read memory module");

    assert!(modules.contains("pub(crate) mod aarch64_vm_logic_shared;"));
    assert!(memory.contains(
        "use super::aarch64_vm_logic_shared as aarch64_vm_logic;"
    ));
    assert!(!memory.contains("#![allow(dead_code)]"));
    assert!(!memory.contains("#[path = \"aarch64_vm_logic_shared.rs\"]"));
    assert!(!memory.contains("pub struct Shell"));
    assert!(!memory.contains("pub fn demo_processes()"));
}
```

- [ ] **Step 2: Run the contract and verify it fails**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_memory_uses_one_vm_logic_module_without_broad_dead_code_suppression --exact
```

Expected: FAIL because `memory.rs` still declares the file privately and
contains the broad allowance and legacy shell.

- [ ] **Step 3: Use the canonical module and remove the blanket allowance**

Delete `#![allow(dead_code)]` and the private `#[path] mod` declaration from
`memory.rs`. Keep the existing `lowlevel_logic` import and add the alias:

```rust
#[cfg(target_arch = "aarch64")]
use super::aarch64_vm_logic_shared as aarch64_vm_logic;
use super::lowlevel_logic;
```

- [ ] **Step 4: Delete the complete obsolete memory items**

Delete these complete definitions from `memory.rs`; this list is exhaustive:

```text
PAGE_MASK
MemorySegment::size
ProcessAddressSpace::heap_alloc
ProcessAddressSpace::stack_alloc
ProcessAddressSpace::page_to_vaddr
ProcessAddressSpace::find_segment_for_vaddr
ProcessAddressSpace::is_valid_vaddr
ProcessAddressSpace::print_info
ProcessManager::get_process_by_pid
ProcessManager::print_status
print_hex
```

Remove `ProcessState::Blocked` and its `as_str` match arm so the active enum is:

```rust
pub enum ProcessState {
    Empty = 0,
    Ready = 1,
    Running = 2,
    Terminated = 4,
}

impl ProcessState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessState::Empty => "Empty     ",
            ProcessState::Ready => "Ready     ",
            ProcessState::Running => "Running   ",
            ProcessState::Terminated => "Terminated",
        }
    }
}
```

Delete the contiguous legacy debug surface beginning with
`/// Demo: Create multiple processes and show memory layout` and ending at EOF.
This removes `demo_processes`, `ShellCommand`, `Shell`, the `Shell` impl, all
legacy `cmd_*` handlers, `print_process_tree`, `print_hex_u64`,
`print_padded_number`, and `start_shell`. The active user shell remains in
`src/user_level/services/user_shell.rs`.

- [ ] **Step 5: Remove wrappers whose only consumers were deleted**

Delete these complete wrappers from `lowlevel_logic.rs`; keep every macro body
in `lowlevel_logic_shared.rs` for host tests and Verus:

```text
segment_size
segment_contains
heap_alloc
stack_alloc
page_to_vaddr
pfn_valid
bitmap_word_index
bitmap_bit_index
bitmap_mask
```

Keep `segment_end`, `memory_capacity_ok`, both permission helpers,
`process_index_valid`, and all architecture helpers unchanged.

- [ ] **Step 6: Retain root-PFN inspection only for model builds**

Add this attribute immediately before
`Aarch64AddressSpaceCore::root_pfn`:

```rust
#[cfg(not(target_os = "none"))]
```

Do not gate `aarch64_frame_range` or `aarch64_frame_range_cap`; after the
canonical import they are active AArch64 runtime functions.

- [ ] **Step 7: Run focused checks**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_memory_uses_one_vm_logic_module_without_broad_dead_code_suppression --exact
```

Expected: PASS.

Run:

```bash
make ut
```

Expected: PASS, including AArch64 VM and low-level shared-logic tests.

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL on a later category such as the unused `set_user_tls`; no
warning may originate from the duplicate AArch64 VM module or removed memory
surface.

- [ ] **Step 8: Commit canonical ownership**

```bash
git add src/kernel_lowlevel/memory.rs src/kernel_lowlevel/aarch64_vm_logic_shared.rs src/kernel_lowlevel/lowlevel_logic.rs tests/host/tests/integration_contracts.rs
git commit -m "refactor: canonicalize AArch64 VM logic ownership"
```

### Task 3: Remove Genuinely Obsolete Runtime Helpers And Fields

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/kernel_lowlevel/ARM64/cpu.rs`
- Modify: `src/syscall/address_logic.rs`
- Modify: `src/syscall/linux_process.rs`
- Modify: `src/syscall/linux_process_memory.rs`
- Modify: `src/syscall/linux_task.rs`

- [ ] **Step 1: Write the failing obsolete-surface contract**

Add this integration test:

```rust
#[test]
fn aarch64_runtime_has_no_obsolete_warning_only_surface() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cpu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/cpu.rs"))
        .expect("read AArch64 CPU module");
    let address = std::fs::read_to_string(repository.join("src/syscall/address_logic.rs"))
        .expect("read address wrappers");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read Linux process memory runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");

    assert!(!cpu.contains("pub fn set_user_tls("));
    for helper in [
        "pub(crate) fn range_overlaps(",
        "pub(crate) fn range_within_window(",
        "pub(crate) fn linux_user_range_writable(",
        "pub(crate) fn linux_user_range_readable(",
    ] {
        assert!(!address.contains(helper), "obsolete address wrapper {helper}");
    }
    assert!(!process.contains("pub(crate) fn descriptors(&self)"));
    assert!(!process.contains("pub(crate) fn shared_attachments("));
    assert!(!task.contains("pub(crate) fn lookup_tid("));

    let clone_start = memory
        .find("pub(crate) struct LinuxSharedAttachmentClone")
        .expect("shared attachment clone");
    let clone_body = braced_body(&memory[clone_start..]);
    assert!(!clone_body.contains("pub prot:"));
    assert!(!clone_body.contains("pub flags:"));
}
```

Remove `"pub(crate) fn lookup_tid("` from the required task API list in
`linux_task_runtime_is_locked_and_pinned_to_one_cpu`; the new test explicitly
locks its removal instead of silently weakening the contract.

- [ ] **Step 2: Run the contract and verify it fails**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_runtime_has_no_obsolete_warning_only_surface --exact
```

Expected: FAIL because all listed helpers and fields still exist.

- [ ] **Step 3: Delete the unused helpers**

Delete these complete definitions:

```text
src/kernel_lowlevel/ARM64/cpu.rs: set_user_tls
src/syscall/address_logic.rs: range_overlaps
src/syscall/address_logic.rs: range_within_window
src/syscall/address_logic.rs: linux_user_range_writable
src/syscall/address_logic.rs: linux_user_range_readable
src/syscall/linux_process.rs: LinuxResourceClone::descriptors
src/syscall/linux_process.rs: LinuxResourceClone::shared_attachments
src/syscall/linux_task.rs: lookup_tid
```

Retain the active `checked_end`, `page_aligned`, and
`fixed_linux_mmap_request_ok` wrappers in `address_logic.rs`. Retain
`LinuxResourceClone::take_shared_attachments` because the fork transaction
consumes it.

- [ ] **Step 4: Remove the unused clone fields**

Make `LinuxSharedAttachmentClone` exactly:

```rust
#[derive(Clone)]
pub(crate) struct LinuxSharedAttachmentClone {
    pub object_id: u32,
    pub attachment_addr: usize,
    pub attachment_len: usize,
    pub addr: usize,
    pub len: usize,
    pub pages: Vec<LinuxPageBacking>,
    owns_attachment_reference: bool,
}
```

Remove `prot: mapping.prot` and `flags: mapping.flags` from its sole
initializer in `clone_shared_attachments`.

- [ ] **Step 5: Run focused checks**

Run the new contract and then all integration contracts:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_runtime_has_no_obsolete_warning_only_surface --exact
./scripts/run-host-unit-tests.sh --test integration_contracts
```

Expected: both PASS.

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL only on the remaining host/Verus model methods; none of the
symbols removed in this task may appear.

- [ ] **Step 6: Commit obsolete runtime cleanup**

```bash
git add src/kernel_lowlevel/ARM64/cpu.rs src/syscall/address_logic.rs src/syscall/linux_process.rs src/syscall/linux_process_memory.rs src/syscall/linux_task.rs tests/host/tests/integration_contracts.rs
git commit -m "refactor: remove unused AArch64 runtime helpers"
```

### Task 4: Scope Process And Fork Model APIs To Host And Verus Builds

**Files:**
- Modify: `src/syscall/linux_process_logic_shared.rs`
- Modify: `src/syscall/linux_fork_logic_shared.rs`

- [ ] **Step 1: Confirm the warning gate is red for this category**

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL and name `LinuxProcessTable::{reserve_child, publish,
by_scheduler, launch_reaper_active}`, `configure_fork_failure`, and
`clone_and_map_linux_fork_pages` among the remaining errors.

- [ ] **Step 2: Gate only the host/proof process-table methods**

Insert this exact attribute immediately before each of
`LinuxProcessTable::reserve_child`, `LinuxProcessTable::publish`,
`LinuxProcessTable::by_scheduler`, and
`LinuxProcessTable::launch_reaper_active`:

```rust
#[cfg(not(target_os = "none"))]
```

Do not gate `reserve_child_with_pid`, `publish_fork`,
`complete_fork_publish`, `rollback_fork`, or any wait/exit method; those are
live kernel paths.

- [ ] **Step 3: Gate only the host/proof fork helpers**

Insert the same exact attribute immediately before
`configure_fork_failure` and `clone_and_map_linux_fork_pages`:

```rust
#[cfg(not(target_os = "none"))]
```

Keep the failpoint atomics, `clear_fork_failure`, `fork_failpoint`, separate
clone/map/release functions, and transaction engine in the bare-metal build.

- [ ] **Step 4: Prove model consumers still compile and pass**

Run:

```bash
make ut
make verus-syscall
```

Expected: PASS. Host tests still call every gated model API and Verus still
includes both shared files on its host target.

- [ ] **Step 5: Re-run the warning gate**

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL on later process-memory/task model warnings, with no error from
the six APIs gated in this task.

- [ ] **Step 6: Commit process model ownership**

```bash
git add src/syscall/linux_process_logic_shared.rs src/syscall/linux_fork_logic_shared.rs
git commit -m "refactor: scope process models to host builds"
```

### Task 5: Scope Process-Memory Models To Host And Verus Builds

**Files:**
- Modify: `src/syscall/linux_process_memory_logic_shared.rs`

- [ ] **Step 1: Confirm the warning gate is red for the model block**

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL naming the model types beginning with
`LinuxProcessAttributesCore`, `LinuxOpenDescriptionTableCore`,
`LinuxProcessResourceCore`, `LinuxSharedPageTableCore`, and
`LinuxProcessMemoryCore`.

- [ ] **Step 2: Gate the independent host/proof items**

Insert this exact attribute immediately before every item in the following
list:

```rust
#[cfg(not(target_os = "none"))]
```

```text
LINUX_MAP_PRIVATE
LinuxProcessAttributesCore struct
LinuxProcessAttributesCore impl
LinuxForkFailureSchedule struct
LinuxForkFailureSchedule impl
LinuxForkAcquisitionLedger::release
linux_clone_page_backing
LinuxDescriptorEntry::EMPTY
LinuxPageBacking::is_shared
linux_mapping_range_covered
linux_process_memory_remove_index
```

Leave the containing `LinuxForkAcquisitionLedger`, `LinuxDescriptorEntry`, and
`LinuxPageBacking` types compiled for the kernel because their other methods
and values are used by live runtime transactions.

- [ ] **Step 3: Gate each complete host/proof model family**

Insert `#[cfg(not(target_os = "none"))]` before both the struct and its impl
for every family below:

```text
LinuxOpenDescriptionTableCore
LinuxProcessResourceCore
LinuxResourceCloneCore
LinuxSharedPageTableCore
LinuxProcessMemoryCore
```

Insert the same attribute before each standalone dependent model type:

```text
LinuxProcessMappingCore
LinuxMappingRange
LinuxBrkCore
```

Do not gate `LinuxOpenDescription`, `LinuxSharedPageRecord`,
`LinuxSharedAttachmentRecord`, `LinuxSharedMappingRange`, or
`LinuxMappingAccessRange`; the AArch64 runtime consumes those types directly.

- [ ] **Step 4: Prove host and proof behavior is unchanged**

Run:

```bash
make ut
make verus-syscall
```

Expected: PASS, including resource-clone rollback, process-memory range,
shared-page, fork failpoint, and Verus proof coverage.

- [ ] **Step 5: Re-run the warning gate**

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL only on the remaining Linux task-model category. No
process-memory model item from this task may be reported.

- [ ] **Step 6: Commit process-memory model ownership**

```bash
git add src/syscall/linux_process_memory_logic_shared.rs
git commit -m "refactor: scope process memory models to host builds"
```

### Task 6: Scope Linux Task Model APIs And Reach Zero Warnings

**Files:**
- Modify: `src/syscall/linux_task_logic_shared.rs`

- [ ] **Step 1: Confirm the final warning category**

Run:

```bash
make aarch64-warning-check
```

Expected: FAIL naming only the unused signal/task model methods in
`linux_task_logic_shared.rs`.

- [ ] **Step 2: Gate pending-signal inspection helpers**

Insert this exact attribute immediately before each listed method:

```rust
#[cfg(not(target_os = "none"))]
```

```text
LinuxPendingSignals::take_eligible
LinuxPendingSignals::take_matching
LinuxTaskSignalState::take_unblocked
LinuxTaskSignalState::take_matching
LinuxTaskSignalState::take_suspend_restore_mask
```

Keep the reservation-based variants compiled for the kernel:
`take_eligible_reserved`, `take_matching_reserved`, and
`take_unblocked_reserved`.

- [ ] **Step 3: Gate pure task-table model methods**

Insert `#[cfg(not(target_os = "none"))]` immediately before each method:

```text
LinuxTaskTable::signal_state
LinuxTaskTable::signal_wait_target
LinuxTaskTable::accepting_signal_wait_target
LinuxTaskTable::handoff_process_pending_signal
LinuxTaskTable::process_signal_target
LinuxTaskTable::discard_signal
LinuxTaskTable::begin_child_exit_by_scheduler
```

Do not gate the corresponding process-filtered runtime helpers implemented in
`linux_task.rs`; those remain active and are used by signal delivery.

- [ ] **Step 4: Prove host and proof behavior is unchanged**

Run:

```bash
make ut
make verus-syscall
```

Expected: PASS, including pending-signal ordering, signal-wait handoff,
process target selection, signal discard, and child-exit tests.

- [ ] **Step 5: Run the authoritative warning gate**

Run:

```bash
make aarch64-warning-check
```

Expected: PASS. Cargo must report a successful optimized
`aarch64-unknown-none` build with no warning lines, and the AArch64 link-layout
checker must pass.

- [ ] **Step 6: Check that suppression was not introduced**

Run:

```bash
git diff 74fd0b9 -- '*.rs' | rg '^\+.*allow\((dead_code|unused|warnings)'
```

Expected: no matches and `rg` exit status 1.

- [ ] **Step 7: Commit task-model ownership**

```bash
git add src/syscall/linux_task_logic_shared.rs
git commit -m "refactor: scope task models to host builds"
```

### Task 7: Run Full AArch64 Regression Verification

**Files:**
- Verify only; modify files only if a failing check exposes a defect in the
  changes above.

- [ ] **Step 1: Format both Rust crates**

Run:

```bash
cargo fmt
cargo fmt --manifest-path tests/host/Cargo.toml
```

Expected: success.

- [ ] **Step 2: Re-run the strict AArch64 build from the final tree**

Run:

```bash
make aarch64-warning-check
```

Expected: PASS with zero warning lines.

- [ ] **Step 3: Run the complete local test suite**

Run:

```bash
make test
```

Expected: PASS for formatting, shell syntax, launcher tests, link-layout tests,
host unit tests, host integration contracts, POSIX host-tool tests, and the
strict AArch64 production build.

- [ ] **Step 4: Run all Verus proof harnesses**

Run:

```bash
make verus
```

Expected: PASS for coverage audit plus syscall, kernel-object,
kernel-low-level, user-level, and services harnesses.

- [ ] **Step 5: Run the AArch64 QEMU smoke test**

Run:

```bash
make st ARCH=aarch64-unknown-none
```

Expected: PASS after required boot milestones, the safe Hermes campaign, and
the `smros:/>` shell prompt are observed.

- [ ] **Step 6: Run final repository hygiene checks**

Run:

```bash
git diff --check
git status --short
```

Expected: `git diff --check` prints nothing. `git status --short` is empty
after any formatting-only changes are committed.

- [ ] **Step 7: Commit any formatting-only changes**

Only when Step 1 changed files:

```bash
git add -u
git commit -m "style: format AArch64 warning cleanup"
```

- [ ] **Step 8: Record final evidence in the handoff**

Report the final commit, exact strict-build command, zero-warning result, host
test result, Verus result, QEMU smoke result, and `git diff --check` result.
State explicitly that x86_64 and RISC-V64 cleanup remains deferred.
