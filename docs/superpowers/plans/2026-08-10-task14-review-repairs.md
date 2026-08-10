# Task 14 Review Repairs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve every confirmed Critical and Important full-range review finding before fast-forwarding the AArch64 fork process runtime to local `master`.

**Architecture:** Keep the approved eager-copy process design. Move every fallible mapping-metadata allocation ahead of hardware page-table changes, make the eventual metadata swap allocation-free, and carry detached SysV attachment references out of the process-memory lock for release. Keep process lifecycle policy in the shared host-testable logic, and append merge-head gate provenance without changing the immutable `c0a513e` campaign outcomes.

**Tech Stack:** Rust `no_std` kernel runtime, shared Rust host logic, AArch64 translation tables, Cargo host/integration tests, Verus, QEMU smoke tests, Markdown evidence.

---

### Task 1: Process terminal and copy-fault semantics

**Files:**
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/syscall/linux_process_logic_shared.rs`
- Modify: `src/syscall/linux_process.rs`
- Modify: `src/syscall/linux_process_memory_logic_shared.rs`
- Modify: `src/syscall/linux_process_memory.rs`

- [ ] **Step 1: Write failing host tests for exit-signal routing and visible orphan parents**

Add tests that require terminal transitions to carry `Option<usize>` instead of a Boolean notification, with `None` for clone exit signal zero, `Some(SIGCHLD)` for fork, and the exact custom signal for non-thread clone. Add a test that maps `LINUX_LAUNCH_REAPER_PID` to `LINUX_ROOT_PID` while retaining parent zero for the launch root.

```rust
assert_eq!(linux_child_exit_notification(0), None);
assert_eq!(linux_child_exit_notification(17), Some(17));
assert_eq!(linux_visible_parent_pid(LINUX_ROOT_PID, 0), 0);
assert_eq!(
    linux_visible_parent_pid(42, LINUX_LAUNCH_REAPER_PID),
    LINUX_ROOT_PID,
);
```

- [ ] **Step 2: Run the focused host tests and verify RED**

Run:

```bash
cargo test --manifest-path tests/host/Cargo.toml \
  process_exit_signal_routes_exact_notification -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml \
  orphan_parent_identity_is_user_visible -- --nocapture
```

Expected: compilation or assertion failure because the helpers and signal-bearing transition do not exist.

- [ ] **Step 3: Implement exact terminal notification and parent identity**

Replace `LinuxTerminalChildTransition::notify_parent` with `notification_signal: Option<usize>`. Pass the exact signal to the notification closure, use `None` for exit signal zero, and apply ignored/`SA_NOCLDWAIT` policy only to `SIGCHLD`. Translate the internal launch-reaper sentinel to PID 1 in `current_parent_pid()`.

```rust
pub(crate) struct LinuxTerminalChildTransition {
    pub parent_pid: usize,
    pub notification_signal: Option<usize>,
}

pub(crate) const fn linux_child_exit_notification(exit_signal: usize) -> Option<usize> {
    if exit_signal == 0 { None } else { Some(exit_signal) }
}
```

- [ ] **Step 4: Write and run a failing copy-error classification test**

Add a shared error-class helper test requiring invalid address, invalid permission, already mapped, not mapped, and permission denied to classify as `Fault` for copy operations; only allocation failure remains `OutOfMemory`.

Run:

```bash
cargo test --manifest-path tests/host/Cargo.toml copy_address_errors_are_efault -- --nocapture
```

Expected: FAIL because mapping and copy operations currently share one errno mapper.

- [ ] **Step 5: Implement copy-specific error mapping and verify GREEN**

Use a copy-specific AArch64 error mapper for `copy_to_user` and `copy_from_user`. Map user-address and translation errors to `EFAULT` while preserving `ENOMEM` for allocation failure. Run:

```bash
cargo test --manifest-path tests/host/Cargo.toml \
  process_exit_signal_routes_exact_notification -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml \
  orphan_parent_identity_is_user_visible -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml \
  copy_address_errors_are_efault -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts \
  linux_wait_reaps_one_real_child_status -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 6: Commit Task 1**

```bash
git add tests/host/src/lib.rs tests/host/tests/integration_contracts.rs \
  src/syscall/linux_process_logic_shared.rs src/syscall/linux_process.rs \
  src/syscall/linux_process_memory_logic_shared.rs src/syscall/linux_process_memory.rs
git commit -m "fix: preserve Linux process terminal semantics"
```

### Task 2: Allocation-free mapping metadata commit

**Files:**
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/syscall/linux_process_memory_logic_shared.rs`
- Modify: `src/syscall/linux_process_memory.rs`

- [ ] **Step 1: Write failing allocation-order and attachment tests**

Add integration contracts requiring fallible metadata planning before `map_unmapped_pages`, `protect_pages_transactionally`, or `unmap_pages_transactionally`; forbid `push`, `extend`, `to_vec`, `collect::<Vec<_>>()`, source `clone`, and unchecked capacity growth after those hardware mutations. Add host coverage for full and partial shared-attachment replacement, requiring a full replacement to emit one detached reference and a partial replacement to retain it.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts \
  linux_process_memory_metadata_commit_is_allocation_free -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml \
  shared_attachment_replacement_reconciles_final_mappings -- --nocapture
```

Expected: FAIL on the current post-page-table `push`, `extend`, `to_vec`, `collect`, and stale-attachment paths.

- [ ] **Step 3: Build metadata plans before page-table mutation**

Introduce a private `LinuxMappingMetadataPlan` that fallibly clones the current metadata, fallibly creates split pieces and removed-backing lists, and fallibly creates the final shared-attachment and detached-reference lists. It must reserve every destination vector with `try_reserve_exact`, use a fallible mapping-source slice helper, and expose only allocation-free commit operations.

```rust
struct LinuxMappingMetadataPlan {
    mappings: Vec<LinuxProcessMapping>,
    removed: Vec<LinuxPageBacking>,
    shared_attachments: Vec<LinuxSharedAttachmentRecord>,
    detached: Vec<(u32, usize)>,
}

fn commit_mapping_metadata(&mut self, plan: LinuxMappingMetadataPlan) {
    self.mappings = plan.mappings;
    self.shared_attachments = plan.shared_attachments;
}
```

The real implementation must keep any new replacement mapping outside the plan until every fallible step succeeds, reserve the final insertion slot, then move it into the plan with an infallible `push`.

- [ ] **Step 4: Apply plans to every VM mutation**

Use prebuilt plans for non-fixed and fixed `mmap`, `mprotect`, `munmap`, shrinking/growing/moving/fixed `mremap`, and `brk`. Pre-reserve `mark_shared` page, acquisition, and attachment vectors before acquiring shared references or changing page tables. Precompute `next_addr` before commit. Replace rollback-only vector construction with prebuilt page lists or allocation-free loops.

After a committed replacement, return detached `(object_id, address)` records from the locked operation and release their attachment references after `with_current` returns.

- [ ] **Step 5: Verify focused GREEN and full host regression**

Run:

```bash
cargo test --manifest-path tests/host/Cargo.toml \
  shared_attachment_replacement_reconciles_final_mappings -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts \
  linux_process_memory_metadata_commit_is_allocation_free -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml
```

Expected: focused tests and all host tests pass.

- [ ] **Step 6: Commit Task 2**

```bash
git add tests/host/src/lib.rs tests/host/tests/integration_contracts.rs \
  src/syscall/linux_process_memory_logic_shared.rs src/syscall/linux_process_memory.rs
git commit -m "fix: make Linux VM metadata commits allocation free"
```

### Task 3: Merge-head evidence without historical mutation

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `docs/posix/2026-08-06-aarch64-fork-process-runtime-results.md`

- [ ] **Step 1: Add a failing documentation contract**

Extend the existing results-document integration contract to require a `Task 14 merge-head verification` section containing the merge candidate commit, proof counts, host test counts, AArch64 build/layout result, QEMU smoke result, focused canaries, the repair-head campaign summary, and an explicit statement that the earlier full campaign remains immutably bound to `c0a513e` as a historical baseline.

- [ ] **Step 2: Run the contract and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts \
  posix_process_runtime_results_separate_campaign_and_merge_head_evidence -- --nocapture
```

Expected: FAIL because the addendum is absent.

- [ ] **Step 3: Run current-head gates, canaries, and campaign**

Run `make test`, `make verus`, `make coverage-host`, `SMOKE_QEMU_SMP=4 SMOKE_QEMU_MEMORY=512M make st ARCH=aarch64-unknown-none`, `make posix-stage`, the three focused canaries, complete `fork`, base, memory, and all-test selections, fatal/resource audits, Coverity availability, report generation, `git diff --check`, and retained-artifact hash checks. Use fresh private disk copies and result directories below `target/posix/aarch64/`; do not access either checkout's repository-root disk.

Record exact counts, hashes, and commit identity in an additive merge-head section. Do not change any status, denominator, quality result, hash, or `c0a513e` provenance field in the historical campaign sections.

- [ ] **Step 4: Verify GREEN and commit**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts \
  posix_process_runtime_results_separate_campaign_and_merge_head_evidence -- --nocapture
git add tests/host/tests/integration_contracts.rs \
  docs/posix/2026-08-06-aarch64-fork-process-runtime-results.md
git commit -m "docs: record Task 14 merge-head verification"
```

Expected: documentation contract passes.

### Task 4: Review, gates, and local merge

**Files:**
- Review: `master..HEAD`
- Preserve: every registered worktree and retained POSIX artifact

- [ ] **Step 1: Request independent follow-up review**

Review all repairs against `docs/superpowers/specs/2026-08-06-posix-aarch64-fork-process-runtime-design.md`. Resolve every Critical and Important finding before continuing.

- [ ] **Step 2: Run the complete pre-merge gate**

```bash
make test
make verus
SMOKE_QEMU_SMP=4 SMOKE_QEMU_MEMORY=512M make st ARCH=aarch64-unknown-none
git diff --check
git status --short --branch
```

Expected: all commands exit zero and the feature worktree is clean after restoring only test-generated bytecode.

- [ ] **Step 3: Fast-forward local master and preserve the feature worktree**

```bash
git -C /home/steven/workspace/SMROS merge --ff-only feat/posix-aarch64-fork-process-runtime
```

Do not pull, push, delete the branch, remove a worktree, or rewrite retained evidence.

- [ ] **Step 4: Verify the merged result**

Run the complete gate again from `/home/steven/workspace/SMROS`, verify `master` and the feature branch resolve to the same commit, and verify every worktree remains registered.
