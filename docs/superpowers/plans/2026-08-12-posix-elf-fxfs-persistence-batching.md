# POSIX ELF FxFS Persistence Batching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coalesce ordinary FxFS mutations across each launched POSIX ELF lifecycle while preserving explicit `sync`, `fsync`, and `fdatasync` durability so the complete AArch64 AIO group finishes without watchdog recovery.

**Architecture:** Add a fallible forced-commit path beneath the existing FxFS suspension guard, attach one guard to the exact accepted `run_elf` launch ID, and release it after process teardown but before outcome dispatch. Ordinary mutations defer to the lifecycle boundary; explicit synchronization bypasses the guard and reports `EIO` where the syscall contract permits.

**Tech Stack:** Rust `no_std`, SMROS FxFS and run-ELF lifecycle state, Linux syscall compatibility, host Rust integration contracts, pinned Open POSIX Test Suite, QEMU AArch64, Tarpaulin, Coverity when available, and Verus.

---

## File Structure

- Modify `tests/host/tests/integration_contracts.rs`: add three focused RED contracts for forced FxFS commits, exact-launch persistence ownership, and Linux synchronization syscall wiring.
- Modify `src/user_level/services/fxfs.rs`: add fallible forced persistence that bypasses suspension, preserves pending work after failure, and leaves the existing best-effort `flush_persist` API compatible.
- Modify `src/user_level/services/run_elf.rs`: make the active launch own an `FxfsPersistGuard`, attach it before thread publication, and release it outside the lifecycle lock before timing/outcome dispatch.
- Modify `src/syscall/syscall.rs`: route `sync`, `fsync`, `fdatasync`, and `sync_file_range` through forced FxFS persistence without changing current descriptor classification.
- Regenerate `host_shared/posixtest/`: verify the pinned upstream stage after the implementation commits. This directory is generated and is not committed.
- Generate `target/posix/aarch64/aio-fxfs-batching-*`: retain focused campaigns, the full AIO campaign, reboot evidence, quality evidence, and the seven-artifact report. These files are generated and are not committed.

### Task 1: Add Fallible Forced FxFS Persistence

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs` after `fxfs_bootstrap_provides_posix_shared_memory_directory`
- Modify: `src/user_level/services/fxfs.rs:598-631`
- Modify: `src/user_level/services/fxfs.rs:2051-2058`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add the focused forced-persistence contract**

Add this complete test:

```rust
#[test]
fn fxfs_forced_persist_bypasses_suspension_and_preserves_failed_pending_work() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS service");

    let persist_start = fxfs
        .find("fn persist(&mut self)")
        .expect("ordinary persistence path");
    let persist = braced_body(&fxfs[persist_start..]);
    assert!(persist.contains("if self.persist_suspended > 0"));
    assert!(persist.contains("self.persist_pending = true;"));
    assert!(persist.contains("let _ = self.force_persist();"));

    let force_start = fxfs
        .find("fn force_persist(&mut self) -> Result<(), FxfsError>")
        .expect("fallible forced persistence path");
    let force = braced_body(&fxfs[force_start..]);
    let pending = force
        .find("let pending = self.persist_pending;")
        .expect("pending state snapshot");
    let sync = force
        .find("self.sync_to_block()")
        .expect("full image commit");
    let clear = force
        .find("self.persist_pending = false;")
        .expect("successful commit clears pending work");
    assert!(pending < sync && sync < clear);
    assert!(force.contains("self.persist_pending = pending;"));
    assert!(force.contains("self.last_sync_ok = true;"));
    assert!(force.contains("self.last_sync_ok = false;"));
    assert!(force.contains("self.last_storage_error = Some(err);"));
    assert!(force.contains("Err(err)"));

    let public_force_start = fxfs
        .find("pub fn force_persist() -> Result<(), FxfsError>")
        .expect("public forced persistence API");
    let public_force = braced_body(&fxfs[public_force_start..]);
    assert!(public_force.contains("state().force_persist()"));

    let flush_start = fxfs
        .find("pub fn flush_persist()")
        .expect("best-effort compatibility flush");
    let flush = braced_body(&fxfs[flush_start..]);
    assert!(flush.contains("let _ = force_persist();"));
}
```

- [ ] **Step 2: Run RED and confirm the missing API is the failure**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts fxfs_forced_persist_bypasses_suspension_and_preserves_failed_pending_work -- --exact
```

Expected: FAIL because `fn force_persist(&mut self) -> Result<(), FxfsError>` is absent. The failure must not be a compile error or unrelated assertion.

- [ ] **Step 3: Implement the minimal forced-commit state transition**

Replace `FxfsState::persist` and add `FxfsState::force_persist` immediately after it:

```rust
    fn persist(&mut self) {
        if self.persist_suspended > 0 {
            self.persist_pending = true;
            return;
        }
        let _ = self.force_persist();
    }

    fn force_persist(&mut self) -> Result<(), FxfsError> {
        let pending = self.persist_pending;
        if !self.block_backed {
            let err = FxfsError::StorageUnavailable;
            self.persist_pending = pending;
            self.last_sync_ok = false;
            self.last_storage_error = Some(err);
            return Err(err);
        }
        match self.sync_to_block() {
            Ok(()) => {
                self.persist_pending = false;
                self.last_sync_ok = true;
                self.last_storage_error = None;
                Ok(())
            }
            Err(err) => {
                self.persist_pending = pending;
                self.last_sync_ok = false;
                self.last_storage_error = Some(err);
                Err(err)
            }
        }
    }
```

Replace the public flush block with:

```rust
pub fn force_persist() -> Result<(), FxfsError> {
    state().force_persist()
}

pub fn flush_persist() {
    let _ = force_persist();
}
```

Do not change the two-slot image format, serialization order, block-write path, guard nesting, or existing best-effort callers.

- [ ] **Step 4: Run GREEN and the existing FxFS contract neighborhood**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts fxfs_forced_persist_bypasses_suspension_and_preserves_failed_pending_work -- --exact
./scripts/run-host-unit-tests.sh --test integration_contracts fxfs_bootstrap_provides_posix_shared_memory_directory -- --exact
git diff --check
```

Expected: both focused contracts pass and the diff check emits nothing.

- [ ] **Step 5: Commit the tested forced-persistence primitive**

```bash
git add tests/host/tests/integration_contracts.rs src/user_level/services/fxfs.rs
git commit -m "fix: expose forced FxFS persistence"
```

Expected: the commit contains one RED/GREEN contract and the minimal forced-commit implementation.

### Task 2: Own The Persistence Guard For The Exact ELF Launch

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs` after the Task 1 contract
- Modify: `src/user_level/services/run_elf.rs:124-125`
- Modify: `src/user_level/services/run_elf.rs:145-187`
- Modify: `src/user_level/services/run_elf.rs:290-335`
- Test: `tests/host/tests/integration_contracts.rs`
- Test: `tests/host/src/lib.rs:7848-8045`

- [ ] **Step 1: Add the focused lifecycle ownership contract**

Add this complete test:

```rust
#[test]
fn run_elf_batches_fxfs_persistence_for_the_exact_launch_lifecycle() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let launcher = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");

    let compact_launcher: String = launcher
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(compact_launcher.contains(
        "typeActiveRun=user_logic::RunElfActiveRequest<RunLaunchInputs,fxfs::FxfsPersistGuard>;"
    ));

    let spawn_start = launcher
        .find("pub fn spawn_observed(")
        .expect("observed ELF spawn");
    let spawn = braced_body(&launcher[spawn_start..]);
    let accept = spawn
        .find("run_elf_start_transition(state, request")
        .expect("accepted launch transition");
    let suspend = spawn
        .find("let persist_guard = fxfs::suspend_persist();")
        .expect("persistence suspension");
    let attach = spawn
        .find("run_elf_attach_resource_transition(state, launch_id, persist_guard)")
        .expect("launch-ID-aware guard attachment");
    let bind = spawn
        .find("RUN_CPU_BINDINGS.bind(cpu, launch_id)")
        .expect("CPU binding");
    let create = spawn
        .find("create_thread_on_cpu(")
        .expect("launcher thread publication");
    assert!(accept < suspend && suspend < attach && attach < bind && bind < create);
    assert!(spawn.contains("drop(error.into_resource());"));
    assert!(spawn.contains("clear_launch_state_without_outcome(LINUX_RUNTIME_CPU, launch_id);"));

    let clear_start = launcher
        .find("fn clear_launch_state_without_outcome(")
        .expect("launch cleanup");
    let clear = braced_body(&launcher[clear_start..]);
    assert!(clear.contains("let completion = with_run_state("));
    assert!(clear.contains("drop(completion);"));

    let complete_start = launcher
        .find("fn complete_active_run(")
        .expect("launch completion");
    let complete = braced_body(&launcher[complete_start..]);
    let parts = complete
        .find("active_request.into_parts()")
        .expect("owned launch decomposition");
    let release = complete
        .find("drop(resource);")
        .expect("guard release");
    let end_tick = complete
        .find("timer::get_tick_count()")
        .expect("completion timestamp");
    let dispatch = complete
        .find("dispatch_outcome(request.observer, outcome)")
        .expect("observer dispatch");
    assert!(parts < release && release < end_tick && end_tick < dispatch);
}
```

- [ ] **Step 2: Run RED and confirm lifecycle ownership is missing**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts run_elf_batches_fxfs_persistence_for_the_exact_launch_lifecycle -- --exact
```

Expected: FAIL on the `ActiveRun` resource type because it is still `()`.

- [ ] **Step 3: Attach the guard before CPU binding and thread creation**

Change the active request type to:

```rust
type ActiveRun =
    user_logic::RunElfActiveRequest<RunLaunchInputs, fxfs::FxfsPersistGuard>;
```

Immediately after the accepted `launch_id` match and before `let cpu = LINUX_RUNTIME_CPU;`, insert:

```rust
    let persist_guard = fxfs::suspend_persist();
    if let Err(error) = with_run_state(|state| {
        user_logic::run_elf_attach_resource_transition(state, launch_id, persist_guard)
    }) {
        drop(error.into_resource());
        clear_launch_state_without_outcome(LINUX_RUNTIME_CPU, launch_id);
        return Err(RunElfError::Thread);
    }
```

This uses the existing exact-launch attachment transition. Do not put the guard in a global, create it before launch acceptance, or let the launcher thread run before attachment.

- [ ] **Step 4: Make cleanup and completion release ordering explicit**

Replace `clear_launch_state_without_outcome` with:

```rust
fn clear_launch_state_without_outcome(cpu: usize, launch_id: user_logic::RunElfLaunchId) {
    let _ = RUN_CPU_BINDINGS.clear(cpu, launch_id);
    let completion = with_run_state(|state| {
        user_logic::run_elf_clear_transition(state, launch_id, || {
            syscall::reset_linux_process_state()
        })
    });
    drop(completion);
}
```

In `complete_active_run`, replace:

```rust
    let (request, resource) = active_request.into_parts();
    let _ = resource;
```

with:

```rust
    let (request, resource) = active_request.into_parts();
    drop(resource);
```

The explicit drop must remain before `timer::get_tick_count()` and `dispatch_outcome`.

- [ ] **Step 5: Run GREEN plus generic lifecycle ownership tests**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts run_elf_batches_fxfs_persistence_for_the_exact_launch_lifecycle -- --exact
./scripts/run-host-unit-tests.sh --lib run_elf_owned_resource_releases_on_post_allocation_failure -- --exact
./scripts/run-host-unit-tests.sh --lib run_elf_stale_launch_work_cannot_mutate_reentrant_successor -- --exact
./scripts/run-host-unit-tests.sh --lib run_elf_stack_ownership_balances_long_reentrant_campaign_before_callbacks -- --exact
git diff --check
```

Expected: the contract and all three lifecycle tests pass; resources release exactly once and before callbacks.

- [ ] **Step 6: Commit the tested lifecycle batching**

```bash
git add tests/host/tests/integration_contracts.rs src/user_level/services/run_elf.rs
git commit -m "fix: batch FxFS persistence across ELF runs"
```

Expected: the commit contains the lifecycle RED/GREEN contract and exact-launch guard ownership only.

### Task 3: Force Persistence From Linux Synchronization Syscalls

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs` after the Task 2 contract
- Modify: `src/syscall/syscall.rs:6295-6314`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add the focused synchronization syscall contract**

Add this complete test:

```rust
#[test]
fn linux_sync_syscalls_force_fxfs_persistence() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let sync_start = syscall.find("pub fn sys_sync()").expect("sync syscall");
    let sync = braced_body(&syscall[sync_start..]);
    assert!(sync.contains("let _ = fxfs::force_persist();"));
    assert!(sync.contains("Ok(0)"));

    let fsync_start = syscall
        .find("pub fn sys_fsync(fd: usize)")
        .expect("fsync syscall");
    let fsync = braced_body(&syscall[fsync_start..]);
    let validate = fsync
        .find("if !linux_fd_is_file_or_pipe(fd)")
        .expect("descriptor validation");
    let force = fsync
        .find("fxfs::force_persist()")
        .expect("forced FxFS commit");
    assert!(validate < force);
    assert!(fsync.contains("map_err(|_| SysError::EIO)?"));
    assert!(fsync.contains("Err(SysError::ENODEV)"));

    let fdatasync_start = syscall
        .find("pub fn sys_fdatasync(fd: usize)")
        .expect("fdatasync syscall");
    let fdatasync = braced_body(&syscall[fdatasync_start..]);
    assert!(fdatasync.contains("sys_fsync(fd)"));

    let sync_range_start = syscall
        .find("pub fn sys_sync_file_range(")
        .expect("sync_file_range syscall");
    let sync_range = braced_body(&syscall[sync_range_start..]);
    assert!(sync_range.contains("sys_fsync(fd)"));
}
```

- [ ] **Step 2: Run RED and confirm `sync` is still a no-op**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_sync_syscalls_force_fxfs_persistence -- --exact
```

Expected: FAIL because `sys_sync` does not call `fxfs::force_persist()`.

- [ ] **Step 3: Implement the minimal syscall wiring**

Replace `sys_sync` and `sys_fsync` with:

```rust
pub fn sys_sync() -> SysResult {
    let _ = fxfs::force_persist();
    Ok(0)
}

pub fn sys_fsync(fd: usize) -> SysResult {
    if !linux_fd_is_file_or_pipe(fd) {
        return Err(SysError::ENODEV);
    }
    fxfs::force_persist().map_err(|_| SysError::EIO)?;
    Ok(0)
}
```

Keep these existing delegations unchanged:

```rust
pub fn sys_fdatasync(fd: usize) -> SysResult {
    sys_fsync(fd)
}

pub fn sys_sync_file_range(fd: usize, _offset: usize, _nbytes: usize, _flags: usize) -> SysResult {
    sys_fsync(fd)
}
```

Do not expand this performance repair into descriptor-type reclassification.

- [ ] **Step 4: Run GREEN and all three new contracts together**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_sync_syscalls_force_fxfs_persistence -- --exact
./scripts/run-host-unit-tests.sh --test integration_contracts fxfs_forced_persist_bypasses_suspension_and_preserves_failed_pending_work -- --exact
./scripts/run-host-unit-tests.sh --test integration_contracts run_elf_batches_fxfs_persistence_for_the_exact_launch_lifecycle -- --exact
make it
git diff --check
```

Expected: all focused contracts and the complete integration suite pass with no diff errors.

- [ ] **Step 5: Commit explicit POSIX synchronization durability**

```bash
git add tests/host/tests/integration_contracts.rs src/syscall/syscall.rs
git commit -m "fix: force FxFS persistence from sync syscalls"
```

Expected: the commit contains only syscall wiring and its focused contract.

### Task 4: Run Host Gates And Rebuild The Pinned AArch64 Stage

**Files:**
- Regenerate: `host_shared/posixtest/`
- Regenerate: `kernel8.img`
- Test: repository host suites and AArch64 warning gate

- [ ] **Step 1: Run the immediate repository gates**

```bash
make host-fmt-check
make script-check
make ut
make it
make posix-tool-test
git diff --check
```

Expected: formatting, shell syntax, all host unit/integration tests, POSIX host tooling, and diff hygiene exit zero.

- [ ] **Step 2: Rebuild and verify the pinned upstream test stage**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
rg "conformance/interfaces/aio_cancel/5-1.c.*41aa55510d10fa8ce5b14e0327ef87916fd16ae354da5d5c8b82f693105eabc9" host_shared/posixtest/manifest.tsv
! rg -a "AIO_DIAG|AIO_KERNEL" host_shared/posixtest
```

Expected: stage build and verification pass, the upstream `aio_cancel/5-1.c` checksum is unchanged, and no diagnostic binary is staged.

- [ ] **Step 3: Build the commit-matched warning-free AArch64 kernel**

```bash
make aarch64-warning-check
```

Expected: optimized AArch64 compile, link, and layout validation finish with warnings denied and produce `kernel8.img` containing the verified stage.

### Task 5: Run Focused Clone And Previously Slow AIO Regressions

**Files:**
- Generate: `target/posix/aarch64/aio-fxfs-batching-clone-*`
- Generate: `target/posix/aarch64/aio-fxfs-batching-focused-*`

- [ ] **Step 1: Preserve the repaired clone worker with three fresh-disk runs**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

test_id = "conformance/interfaces/aio_cancel/5-1.c"
for number in range(1, 4):
    disk = Path(f"target/posix/aarch64/aio-fxfs-batching-clone-{number}.img")
    output = Path(f"target/posix/aarch64/aio-fxfs-batching-clone-{number}")
    assert not disk.exists(), disk
    assert not output.exists(), output
    subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M", test_id=test_id,
    )
    assert result.complete
    assert result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.test_id == test_id
    assert attempt.pts_status == "pass"
    assert attempt.exit_code == 0
    assert not attempt.timed_out
    assert attempt.duration_ms < 30_000
    assert not attempt.resource_deltas.has_positive()
    print(number, attempt.status, attempt.duration_ms, result.raw_log_path)
PY
```

Expected: all three independent runs pass without timeout, restart, or positive residual resource count.

- [ ] **Step 2: Run the four previously slow or timed-out programs on fresh disks**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

tests = (
    ("conformance/interfaces/aio_read/7-1.c", "pass"),
    ("conformance/interfaces/aio_error/2-1.c", "pass"),
    ("conformance/interfaces/aio_return/3-1.c", "pass"),
    ("conformance/interfaces/aio_return/3-2.c", "fail"),
)
for number, (test_id, expected_pts) in enumerate(tests, start=1):
    disk = Path(f"target/posix/aarch64/aio-fxfs-batching-focused-{number}.img")
    output = Path(f"target/posix/aarch64/aio-fxfs-batching-focused-{number}")
    assert not disk.exists(), disk
    assert not output.exists(), output
    subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M", test_id=test_id,
    )
    assert result.complete
    assert result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.test_id == test_id
    assert attempt.pts_status == expected_pts
    assert not attempt.timed_out
    assert attempt.duration_ms < 30_000
    assert not attempt.resource_deltas.has_positive()
    print(test_id, attempt.pts_status, attempt.duration_ms, result.raw_log_path)
PY
```

Expected: every test reaches its truthful terminal PTS result within the watchdog, including the upstream `aio_return/3-2.c` assertion failure.

### Task 6: Run Complete AIO Selections Without Recovery

**Files:**
- Generate: `target/posix/aarch64/aio-fxfs-batching-api.img`
- Generate: `target/posix/aarch64/aio-fxfs-batching-api/`
- Generate: `target/posix/aarch64/aio-fxfs-batching-group.img`
- Generate: `target/posix/aarch64/aio-fxfs-batching-group/`

- [ ] **Step 1: Run the complete `aio_cancel` API on a fresh disk**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from collections import Counter
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

disk = Path("target/posix/aarch64/aio-fxfs-batching-api.img")
output = Path("target/posix/aarch64/aio-fxfs-batching-api")
assert not disk.exists(), disk
assert not output.exists(), output
subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
result = run_smros(
    Path("host_shared/posixtest"), output,
    kernel=Path("kernel8.img"), disk=disk, memory="1024M", api="aio_cancel",
)
assert result.complete
assert result.restart_count == 0
assert result.attempts
assert len({attempt.test_id for attempt in result.attempts}) == len(result.attempts)
assert all(not attempt.timed_out for attempt in result.attempts)
assert all(attempt.launch_status != "launched" or attempt.duration_ms < 30_000 for attempt in result.attempts)
assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
focused = [
    attempt for attempt in result.attempts
    if attempt.test_id == "conformance/interfaces/aio_cancel/5-1.c"
]
assert len(focused) == 1 and focused[0].pts_status == "pass"
print(Counter(attempt.status for attempt in result.attempts))
PY
```

Expected: the complete API selection finishes in one boot with no timeout, duplicate result, or positive resource delta.

- [ ] **Step 2: Run the complete 80-test AIO group on another fresh disk**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from collections import Counter
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

disk = Path("target/posix/aarch64/aio-fxfs-batching-group.img")
output = Path("target/posix/aarch64/aio-fxfs-batching-group")
assert not disk.exists(), disk
assert not output.exists(), output
subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
result = run_smros(
    Path("host_shared/posixtest"), output,
    kernel=Path("kernel8.img"), disk=disk, memory="1024M", group="aio",
)
assert result.complete
assert result.restart_count == 0
assert len(result.attempts) == 80
assert len({attempt.test_id for attempt in result.attempts}) == 80
assert all(not attempt.timed_out for attempt in result.attempts)
assert all(attempt.launch_status != "launched" or attempt.duration_ms < 30_000 for attempt in result.attempts)
assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
focused = [
    attempt for attempt in result.attempts
    if attempt.test_id == "conformance/interfaces/aio_cancel/5-1.c"
]
assert len(focused) == 1 and focused[0].pts_status == "pass"
serial = result.raw_log_path.read_text(errors="replace")
for forbidden in ("Kernel panic", "AIO_DIAG", "AIO_KERNEL", "0x82000006"):
    assert forbidden not in serial
print(Counter(attempt.status for attempt in result.attempts))
print("max_launched_duration_ms", max(
    attempt.duration_ms for attempt in result.attempts
    if attempt.launch_status == "launched"
))
PY
```

Expected: all 80 selected rows have unique terminal results, no watchdog timeout or restart occurs, and genuine upstream failures, unresolved results, and untested build rows remain visible.

### Task 7: Run Lifecycle Canaries And Reboot Readback

**Files:**
- Generate: `target/posix/aarch64/aio-fxfs-batching-canary.img`
- Generate: `target/posix/aarch64/aio-fxfs-batching-canary-*`
- Generate: `target/posix/aarch64/aio-fxfs-batching-reboot.img`
- Generate: `target/posix/aarch64/aio-fxfs-batching-reboot-*.log`

- [ ] **Step 1: Run clone, TLS, join, fork, and exit canaries**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

tests = (
    "conformance/interfaces/pthread_create/1-1.c",
    "conformance/interfaces/pthread_getspecific/1-1.c",
    "conformance/interfaces/pthread_join/1-1.c",
    "conformance/interfaces/fork/1-1.c",
    "conformance/behavior/WIFEXITED/1-1.c",
)
disk = Path("target/posix/aarch64/aio-fxfs-batching-canary.img")
assert not disk.exists(), disk
subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
for number, test_id in enumerate(tests, start=1):
    output = Path(f"target/posix/aarch64/aio-fxfs-batching-canary-{number}")
    assert not output.exists(), output
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M", test_id=test_id,
    )
    assert result.complete
    assert result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.pts_status == "pass"
    assert not attempt.timed_out
    assert attempt.duration_ms < 30_000
    assert not attempt.resource_deltas.has_positive()
    print(test_id, attempt.status, attempt.duration_ms)
PY
```

Expected: all five canaries pass without timeout, restart, or residual resource growth.

- [ ] **Step 2: Write a persistent file, reboot, and read it back**

```bash
qemu-img create -f raw target/posix/aarch64/aio-fxfs-batching-reboot.img 128M
SMROS_ST_COMMANDS='write /data/posix-batch-reboot.txt posix-batch-persisted' \
SMROS_ST_REQUIRED_PATTERNS='smros:/>|wrote 21 bytes to /data/posix-batch-reboot.txt' \
SMROS_ST_TIMEOUT=60 \
SMROS_ST_LOG=target/posix/aarch64/aio-fxfs-batching-reboot-write.log \
FXFS_DISK=target/posix/aarch64/aio-fxfs-batching-reboot.img \
QEMU_MEMORY=1024M \
./scripts/smoke-qemu.sh
SMROS_ST_COMMANDS='cat /data/posix-batch-reboot.txt' \
SMROS_ST_REQUIRED_PATTERNS='smros:/>|posix-batch-persisted' \
SMROS_ST_TIMEOUT=60 \
SMROS_ST_LOG=target/posix/aarch64/aio-fxfs-batching-reboot-read.log \
FXFS_DISK=target/posix/aarch64/aio-fxfs-batching-reboot.img \
QEMU_MEMORY=1024M \
./scripts/smoke-qemu.sh
```

Expected: the first boot reports an exact 21-byte write, and the second boot prints `posix-batch-persisted` from the same two-slot FxFS image.

### Task 8: Run Final Gates And Publish Detailed Quality Evidence

**Files:**
- Generate: `target/posix/aarch64/aio-fxfs-batching-quality/`
- Generate: `target/posix/aarch64/aio-fxfs-batching-quality.json`
- Generate: `target/posix/aarch64/report-aio-fxfs-batching/`

- [ ] **Step 1: Run every applicable repository gate from the implementation commit**

```bash
make host-fmt-check
make script-check
make ut
make it
make posix-tool-test
make aarch64-warning-check
make verus
git diff --check
```

Expected: all repository gates exit zero, the AArch64 build emits no warnings, and all wired Verus proof harnesses verify.

- [ ] **Step 2: Capture host coverage and Coverity availability honestly**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import shutil
import subprocess
from pathlib import Path

root = Path("target/posix/aarch64/aio-fxfs-batching-quality")
root.mkdir(parents=True, exist_ok=False)
checks = []

coverage_log = root / "coverage-host.log"
with coverage_log.open("wb") as output:
    coverage = subprocess.run(
        ["make", "coverage-host"], stdout=output, stderr=subprocess.STDOUT
    )
tarpaulin = shutil.which("cargo-tarpaulin")
coverage_artifact = Path("target/coverage/host/tarpaulin-report.html")
if coverage.returncode == 0 and coverage_artifact.is_file():
    checks.append({
        "artifact": str(coverage_artifact),
        "command": "make coverage-host",
        "coverage_percent": 100.0,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "passed",
        "summary": "Tarpaulin met the repository's exact 100 percent host gate",
        "version": subprocess.check_output(
            ["cargo", "tarpaulin", "--version"], text=True
        ).strip(),
    })
elif tarpaulin is None:
    checks.append({
        "artifact": str(coverage_log),
        "command": "make coverage-host",
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "unavailable",
        "summary": "cargo-tarpaulin is not installed",
        "version": None,
    })
else:
    checks.append({
        "artifact": str(coverage_log),
        "command": "make coverage-host",
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "failed",
        "summary": f"Host coverage exited with status {coverage.returncode}",
        "version": subprocess.check_output(
            ["cargo", "tarpaulin", "--version"], text=True
        ).strip(),
    })

coverity_names = ("cov-build", "cov-analyze", "cov-format-errors")
coverity_tools = [shutil.which(name) for name in coverity_names]
coverity_json = root / "coverity-results.json"
coverity_log = root / "coverity.log"
if all(coverity_tools):
    capture = root / "coverity-capture"
    commands = (
        [coverity_tools[0], "--dir", str(capture), "make", "aarch64-warning-check"],
        [coverity_tools[1], "--dir", str(capture), "--all"],
        [coverity_tools[2], "--dir", str(capture), "--json-output-v7", str(coverity_json)],
    )
    with coverity_log.open("wb") as output:
        completed = [
            subprocess.run(command, stdout=output, stderr=subprocess.STDOUT)
            for command in commands
        ]
    if all(item.returncode == 0 for item in completed) and coverity_json.is_file():
        payload = json.loads(coverity_json.read_text())
        issues = payload.get("issues")
        if not isinstance(issues, list):
            raise ValueError("Coverity JSON does not contain an issues list")
        checks.append({
            "artifact": str(coverity_json),
            "command": "cov-build; cov-analyze --all; cov-format-errors --json-output-v7",
            "coverage_percent": None,
            "findings": len(issues),
            "kind": "static-analysis",
            "name": "coverity",
            "status": "passed" if not issues else "failed",
            "summary": f"Coverity completed with {len(issues)} findings",
            "version": subprocess.check_output(
                [coverity_tools[0], "--version"], text=True, stderr=subprocess.STDOUT
            ).splitlines()[0],
        })
    else:
        checks.append({
            "artifact": str(coverity_log),
            "command": "cov-build; cov-analyze --all; cov-format-errors --json-output-v7",
            "coverage_percent": None,
            "findings": None,
            "kind": "static-analysis",
            "name": "coverity",
            "status": "failed",
            "summary": "One or more Coverity commands failed or produced no JSON artifact",
            "version": subprocess.check_output(
                [coverity_tools[0], "--version"], text=True, stderr=subprocess.STDOUT
            ).splitlines()[0],
        })
else:
    missing = [
        name for name, tool in zip(coverity_names, coverity_tools) if tool is None
    ]
    checks.append({
        "artifact": None,
        "command": None,
        "coverage_percent": None,
        "findings": None,
        "kind": "static-analysis",
        "name": "coverity",
        "status": "unavailable",
        "summary": "Missing Coverity commands: " + ", ".join(missing),
        "version": None,
    })

commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
evidence = {
    "architecture": "aarch64",
    "checks": checks,
    "schema": 1,
    "smros_commit": commit,
}
path = Path("target/posix/aarch64/aio-fxfs-batching-quality.json")
path.write_text(json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n")
print(path)
PY
```

Expected in the planning environment: Tarpaulin and all three Coverity commands are absent, so both checks are recorded as `unavailable` with null percentages/findings. Recheck at execution time and record real results if tools have become available.

- [ ] **Step 3: Render the canonical seven-artifact filtered report**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --smros-results target/posix/aarch64/aio-fxfs-batching-group/results.ndjson \
  --quality-evidence target/posix/aarch64/aio-fxfs-batching-quality.json \
  --out target/posix/aarch64/report-aio-fxfs-batching
```

Expected: `events.ndjson`, `summary.json`, `junit.xml`, `groups.csv`, `apis.csv`, `report.md`, and `index.html` are published atomically and agree on provenance, all 80 selected rows, API/group coverage, every non-pass, resource evidence, coverage, and Coverity availability. The filtered report must not claim overall POSIX compliance.

- [ ] **Step 4: Verify the final repository and evidence state**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path

expected = {
    "events.ndjson",
    "summary.json",
    "junit.xml",
    "groups.csv",
    "apis.csv",
    "report.md",
    "index.html",
}
root = Path("target/posix/aarch64/report-aio-fxfs-batching")
actual = {path.name for path in root.iterdir() if path.is_file()}
assert actual == expected, (sorted(expected), sorted(actual))
print("\n".join(sorted(actual)))
PY
git status --short --branch
git log -6 --oneline --decorate
! rg -n "AIO_KERNEL|AIO_DIAG|pc=0x" src/syscall/syscall.rs src/user_level/services/user_shell.rs
! rg -a "AIO_KERNEL|AIO_DIAG" host_shared/posixtest
```

Expected: exactly seven report files exist, tracked source is clean, the design/plan and three implementation commits are visible, and no diagnostic marker remains in source or the staged suite.
