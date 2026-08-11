# AArch64 Clone Address-Space Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every AArch64 `CLONE_THREAD` child inherit and reinstall its owning Linux process translation root so glibc AIO workers execute instead of repeatedly instruction-faulting at the clone return address.

**Architecture:** Mirror the working AArch64 fork path at two boundaries. Configure the unpublished scheduler thread with the process root and identity during clone reservation, then carry the same root in `Aarch64CloneStart` and reinstall it with a TLB flush immediately before the final EL0 `eret`.

**Tech Stack:** Rust `no_std`, AArch64 exception/context-switch assembly, SMROS Linux task/process-memory and scheduler runtimes, host Rust integration contracts, Open POSIX Test Suite, QEMU, Tarpaulin, Coverity when available, and Verus.

---

## File Structure

- Modify `tests/host/tests/integration_contracts.rs`: add the RED source contract for clone process-root ownership, startup-image layout, and assembly ordering.
- Modify `src/syscall/linux_task.rs`: resolve the active process root, configure and bind the suspended scheduler thread, and carry `root_paddr` in the clone startup image.
- Modify `src/kernel_lowlevel/ARM64/context_switch.S`: install clone `TTBR0_EL1` and perform the required `dsb`/`tlbi`/`dsb`/`isb` sequence before register restore and `eret`.
- Restore `src/syscall/syscall.rs`: remove the temporary AIO syscall serial probes while retaining its pre-diagnostic behavior.
- Restore `src/user_level/services/user_shell.rs`: remove the temporary `ps` ELR display.
- Regenerate `host_shared/posixtest/`: replace the diagnostic AIO binary/stage with the pinned upstream stage. This is generated and is not committed.
- Generate `target/posix/aarch64/aio-clone-address-space-*`: retain fresh-disk run logs, NDJSON, coverage, quality evidence, and the detailed seven-artifact POSIX report. These files are generated and are not committed.

### Task 1: Add The Failing Clone Translation-Root Contract

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs` after `aarch64_clone_child_is_validated_before_publication`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add the focused failing integration contract**

Add this complete test:

```rust
#[test]
fn aarch64_clone_child_installs_process_translation_root_before_el0() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 context switch");

    let clone_layout_start = task
        .find("pub(crate) struct Aarch64CloneStart")
        .expect("clone startup image");
    let clone_layout_end = task[clone_layout_start..]
        .find("#[derive(Clone, Copy)]\n    struct TidDestination")
        .expect("end of clone startup layout");
    let clone_layout = &task[clone_layout_start..clone_layout_start + clone_layout_end];
    assert!(clone_layout.contains("pub root_paddr: u64"));
    assert!(clone_layout.contains(
        "assert!(core::mem::offset_of!(Aarch64CloneStart, root_paddr) == 0x330)"
    ));

    let reserve_start = task
        .find("pub(crate) fn reserve_clone(")
        .expect("clone reservation");
    let reserve = braced_body(&task[reserve_start..]);
    let root = reserve
        .find("linux_process_memory::current_root_paddr()")
        .expect("current process translation root");
    let task_reservation = reserve
        .find(".reserve_child(parent.tgid, scheduler_id.0)")
        .expect("Linux task reservation");
    let scheduler_context = reserve
        .find("thread.context.set_linux_process_start(")
        .expect("suspended scheduler context configuration");
    let process_binding = reserve
        .find("bind_thread_process(scheduler_id, parent.tgid)")
        .expect("scheduler process binding");
    let startup_slot = reserve
        .find("runtime.clone_slots[reservation.slot] = LinuxCloneSlot")
        .expect("clone startup publication");
    assert!(root < task_reservation);
    assert!(task_reservation < scheduler_context);
    assert!(scheduler_context < process_binding);
    assert!(process_binding < startup_slot);
    assert!(reserve.contains("root_paddr,"));
    assert!(reserve.contains("runtime.tasks.rollback(reservation)"));

    let clone_start = switch
        .find("start_linux_clone_child:")
        .expect("clone child assembly entry");
    let clone_end = switch[clone_start..]
        .find(".size start_linux_clone_child")
        .expect("end of clone child assembly entry");
    let clone = &switch[clone_start..clone_start + clone_end];
    let load_root = clone
        .find("ldr     x17, [x16, #0x330]")
        .expect("clone process root load");
    let install_root = clone
        .find("msr     ttbr0_el1, x17")
        .expect("clone process root install");
    let first_dsb = clone.find("dsb     ish").expect("pre-TLBI barrier");
    let tlbi = clone
        .find("tlbi    vmalle1is")
        .expect("clone TLB invalidation");
    let second_dsb = tlbi
        + clone[tlbi..]
            .find("dsb     ish")
            .expect("post-TLBI barrier");
    let isb = clone.find("isb").expect("clone instruction barrier");
    let register_restore = clone
        .find("b       start_linux_child_register_restore")
        .expect("shared child register restore");
    assert!(load_root < install_root);
    assert!(install_root < first_dsb);
    assert!(first_dsb < tlbi);
    assert!(tlbi < second_dsb);
    assert!(second_dsb < isb);
    assert!(isb < register_restore);
}
```

- [ ] **Step 2: Run the contract and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_clone_child_installs_process_translation_root_before_el0 -- --exact
```

Expected: FAIL at `clone_layout.contains("pub root_paddr: u64")` or earlier compilation of the new assertions, proving the current clone startup image does not own a process translation root. A pass is not acceptable; revise the test until it observes the reproduced defect.

### Task 2: Configure And Reinstall The Clone Process Root

**Files:**
- Modify: `src/syscall/linux_task.rs:797-923`
- Modify: `src/kernel_lowlevel/ARM64/context_switch.S:202-217`
- Modify: `src/syscall/syscall.rs:6557-6565,7583-7614,7953-7967`
- Modify: `src/user_level/services/user_shell.rs:9344-9349`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Extend the clone startup ABI**

Replace the clone startup structure and layout assertions with:

```rust
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub(crate) struct Aarch64CloneStart {
    pub frame: Aarch64ExceptionFrame,
    pub user_sp: u64,
    pub return_pc: u64,
    pub pstate: u64,
    pub tls: u64,
    pub root_paddr: u64,
}

const _: () = {
    assert!(core::mem::offset_of!(Aarch64CloneStart, frame) == 0x000);
    assert!(core::mem::offset_of!(Aarch64CloneStart, user_sp) == 0x310);
    assert!(core::mem::offset_of!(Aarch64CloneStart, return_pc) == 0x318);
    assert!(core::mem::offset_of!(Aarch64CloneStart, pstate) == 0x320);
    assert!(core::mem::offset_of!(Aarch64CloneStart, tls) == 0x328);
    assert!(core::mem::offset_of!(Aarch64CloneStart, root_paddr) == 0x330);
};
```

- [ ] **Step 2: Replace `reserve_clone` with process-root configuration before publication**

Use this implementation:

```rust
pub(crate) fn reserve_clone(
    scheduler_id: ThreadId,
    request: LinuxCloneRequest,
    context: LinuxSyscallFrameRef,
) -> Result<LinuxTaskReservation, SysError> {
    let root_paddr = crate::syscall::linux_process_memory::current_root_paddr()?;
    if root_paddr == 0 {
        return Err(SysError::EAGAIN);
    }
    with_runtime(|runtime| {
        let current = scheduler::scheduler().current();
        let Some(parent) = runtime.tasks.by_scheduler(current.0) else {
            return Err(SysError::EAGAIN);
        };
        if scheduler_id == ThreadId::IDLE
            || scheduler_id == current
            || scheduler::scheduler()
                .get_thread(scheduler_id)
                .map(|thread| thread.state)
                != Some(ThreadState::Blocked)
        {
            return Err(SysError::EAGAIN);
        }

        let reservation = runtime
            .tasks
            .reserve_child(parent.tgid, scheduler_id.0)
            .ok_or(SysError::EAGAIN)?;
        if !runtime.tasks.inherit_signal_mask(reservation, current.0) {
            let _ = runtime.tasks.rollback(reservation);
            return Err(SysError::EAGAIN);
        }
        let mut frame = unsafe { context.frame.read() };
        frame.regs[0] = 0;
        let tls = request
            .tls
            .map(|tls| tls as u64)
            .unwrap_or_else(crate::kernel_lowlevel::cpu::read_user_tls);
        let configured = scheduler::scheduler()
            .get_thread_mut(scheduler_id)
            .map(|thread| {
                thread.context.set_linux_process_start(
                    request.user_sp as u64,
                    tls,
                    root_paddr,
                )
            })
            .unwrap_or(false)
            && scheduler::scheduler().bind_thread_process(scheduler_id, parent.tgid);
        if !configured {
            let _ = runtime.tasks.rollback(reservation);
            return Err(SysError::EAGAIN);
        }
        runtime.clone_slots[reservation.slot] = LinuxCloneSlot {
            reservation,
            start: Some(Aarch64CloneStart {
                frame,
                user_sp: request.user_sp as u64,
                return_pc: context.return_pc,
                pstate: context.pstate,
                tls,
                root_paddr,
            }),
            parent_tid: TidDestination {
                address: request.parent_tid.unwrap_or(0),
                ..TidDestination::EMPTY
            },
            child_tid: TidDestination {
                address: if request.flags & CLONE_CHILD_SETTID != 0 {
                    request.child_tid.unwrap_or(0)
                } else {
                    0
                },
                ..TidDestination::EMPTY
            },
            clear_child_tid: if request.clear_child_tid {
                request.child_tid.unwrap_or(0)
            } else {
                0
            },
            committed: false,
        };
        Ok(reservation)
    })
}
```

Resolving `current_root_paddr()` before `with_runtime` is mandatory: the memory lookup resolves the current Linux task and must not recursively enter the task runtime lock.

- [ ] **Step 3: Install and flush the clone root before shared register restore**

Replace `start_linux_clone_child` with:

```asm
.globl start_linux_clone_child
.type start_linux_clone_child, %function
start_linux_clone_child:
    msr     daifset, #2
    mov     x16, x0

    ldr     x17, [x16, #0x310]
    msr     sp_el0, x17
    ldr     x17, [x16, #0x318]
    msr     elr_el1, x17
    ldr     x17, [x16, #0x320]
    msr     spsr_el1, x17
    ldr     x17, [x16, #0x328]
    msr     tpidr_el0, x17
    ldr     x17, [x16, #0x330]
    msr     ttbr0_el1, x17
    dsb     ish
    tlbi    vmalle1is
    dsb     ish
    isb
    b       start_linux_child_register_restore
.size start_linux_clone_child, . - start_linux_clone_child
```

- [ ] **Step 4: Remove every temporary diagnostic source probe**

Restore these syscall bodies exactly:

```rust
pub fn sys_pwrite(fd: usize, buf: usize, len: usize, _offset: u64) -> SysResult {
    sys_write(fd, buf, len)
}

pub fn sys_sched_getparam(_pid: usize, param: usize) -> SysResult {
    linux_zero_user(param, core::mem::size_of::<i32>())
}

pub fn sys_sched_getscheduler(_pid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_sched_setscheduler(_pid: usize, _policy: usize, param: usize) -> SysResult {
    if param == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_futex(
    uaddr: usize,
    op: u32,
    val: u32,
    val2: usize,
    uaddr2: usize,
    val3: u32,
) -> SysResult {
    linux_futex::sys_futex(uaddr, op, val, val2, uaddr2, val3)
}
```

Remove only these two temporary `cmd_ps` statements and leave the following newline write intact:

```rust
ctx.serial.write_str("  pc=0x");
print_hex(&mut ctx.serial, thread.context.elr_el1);
```

Verify no probe remains:

```bash
! rg -n "AIO_KERNEL|AIO_DIAG|pc=0x" src/syscall/syscall.rs src/user_level/services/user_shell.rs
```

Expected: `rg` finds no diagnostic marker and the negated command exits zero.

- [ ] **Step 5: Run GREEN and the surrounding host/build gates**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_clone_child_installs_process_translation_root_before_el0 -- --exact
make it
make ut
make aarch64-warning-check
git diff --check
```

Expected: the focused contract passes, all host tests report zero failures, the AArch64 release/link-layout build exits zero with warnings denied, and the diff check emits nothing.

- [ ] **Step 6: Commit the tested production repair**

```bash
git add tests/host/tests/integration_contracts.rs src/syscall/linux_task.rs src/kernel_lowlevel/ARM64/context_switch.S
git commit -m "fix: inherit AArch64 clone process translation root"
git status --short
```

Expected: the commit contains the RED test and minimal production fix. `src/syscall/syscall.rs` and `src/user_level/services/user_shell.rs` are clean because their temporary edits were removed, not included in the commit.

### Task 3: Restore The Upstream Stage And Prove The Focused Fix

**Files:**
- Regenerate: `host_shared/posixtest/`
- Generate: `target/posix/aarch64/aio-clone-address-space-focused-*/`
- Generate: `target/posix/aarch64/aio-clone-address-space-focused-*.img`

- [ ] **Step 1: Rebuild and verify the pinned upstream stage**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
! rg -a "AIO_DIAG|AIO_KERNEL" host_shared/posixtest
rg "conformance/interfaces/aio_cancel/5-1.c.*41aa55510d10fa8ce5b14e0327ef87916fd16ae354da5d5c8b82f693105eabc9" host_shared/posixtest/manifest.tsv
```

Expected: stage build and verification exit zero, no diagnostic marker is embedded, and the upstream `aio_cancel/5-1.c` binary checksum is restored to `41aa55510d10fa8ce5b14e0327ef87916fd16ae354da5d5c8b82f693105eabc9`.

- [ ] **Step 2: Build the commit-matched warning-free kernel**

Run:

```bash
make aarch64-warning-check
```

Expected: optimized AArch64 compilation and link-layout validation exit zero, print no Rust warnings, and produce `kernel8.img` embedding the restored upstream stage.

- [ ] **Step 3: Run the upstream reproducer three times on three fresh disks**

Run this exact controller script:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

test_id = "conformance/interfaces/aio_cancel/5-1.c"
for run_number in range(1, 4):
    disk = Path(f"target/posix/aarch64/aio-clone-address-space-focused-{run_number}.img")
    output = Path(f"target/posix/aarch64/aio-clone-address-space-focused-{run_number}")
    assert not disk.exists(), disk
    assert not output.exists(), output
    subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
    result = run_smros(
        Path("host_shared/posixtest"),
        output,
        kernel=Path("kernel8.img"),
        disk=disk,
        memory="1024M",
        test_id=test_id,
    )
    assert result.complete
    assert result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.test_id == test_id
    assert attempt.status == "pass"
    assert attempt.pts_status == "pass"
    assert attempt.exit_code == 0
    assert not attempt.timed_out
    assert not attempt.resource_deltas.has_positive()
    serial = result.raw_log_path.read_text(errors="replace")
    assert "Test PASSED" in serial
    for forbidden in (
        "AIO_DIAG",
        "AIO_KERNEL",
        "Kernel panic",
        "timeout",
        "0x82000006",
    ):
        assert forbidden not in serial
    print(run_number, attempt.status, result.raw_log_path)
PY
```

Expected: all three independent fresh-disk campaigns produce one matching `test_end` with PTS pass, exit code zero, no restart/timeout, and no positive resource delta.

### Task 4: Run Complete AIO And Clone/Thread Canaries

**Files:**
- Generate: `target/posix/aarch64/aio-clone-address-space-api-*`
- Generate: `target/posix/aarch64/aio-clone-address-space-group-*`
- Generate: `target/posix/aarch64/aio-clone-address-space-canary-*`

- [ ] **Step 1: Run the complete `aio_cancel` API on a fresh disk**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from collections import Counter
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

disk = Path("target/posix/aarch64/aio-clone-address-space-api.img")
output = Path("target/posix/aarch64/aio-clone-address-space-api")
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
assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
focused = [
    attempt for attempt in result.attempts
    if attempt.test_id == "conformance/interfaces/aio_cancel/5-1.c"
]
assert len(focused) == 1 and focused[0].pts_status == "pass"
print(Counter(attempt.status for attempt in result.attempts))
PY
```

Expected: every selected `aio_cancel` program reaches a terminal result without watchdog timeout or restart; the repaired `5-1.c` row passes. Other upstream assertion results remain truthful.

- [ ] **Step 2: Run the complete AIO group on another fresh disk**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import subprocess
from collections import Counter
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

disk = Path("target/posix/aarch64/aio-clone-address-space-group.img")
output = Path("target/posix/aarch64/aio-clone-address-space-group")
assert not disk.exists(), disk
assert not output.exists(), output
subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
result = run_smros(
    Path("host_shared/posixtest"), output,
    kernel=Path("kernel8.img"), disk=disk, memory="1024M", group="aio",
)
assert result.complete
assert result.restart_count == 0
assert result.attempts
assert len({attempt.test_id for attempt in result.attempts}) == len(result.attempts)
assert all(not attempt.timed_out for attempt in result.attempts)
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
PY
```

Expected: the complete required AIO selection terminates without this hang, and all real non-pass statuses remain visible in `results.ndjson`.

- [ ] **Step 3: Run clone, TLS, join, fork, and process-exit canaries**

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
disk = Path("target/posix/aarch64/aio-clone-address-space-canary.img")
assert not disk.exists(), disk
subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
for index, test_id in enumerate(tests, start=1):
    output = Path(f"target/posix/aarch64/aio-clone-address-space-canary-{index}")
    assert not output.exists(), output
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
    assert not attempt.timed_out
    assert not attempt.resource_deltas.has_positive()
    print(test_id, attempt.status)
PY
```

Expected: all five canaries pass without timeout, restart, or positive resource delta, covering executable clone entry, TLS, futex join, distinct fork address-space entry, and child status observation.

### Task 5: Run Repository Gates And Publish Detailed Quality Evidence

**Files:**
- Generate: `target/posix/aarch64/aio-clone-address-space-quality/`
- Generate: `target/posix/aarch64/aio-clone-address-space-quality.json`
- Generate: `target/posix/aarch64/report-aio-clone-address-space/`

- [ ] **Step 1: Run all applicable repository gates**

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

Expected: formatting, scripts, host suites, POSIX tools, warning-as-error AArch64 build, all wired Verus proofs, and diff hygiene exit zero. Preserve the complete output counts in the verification record.

- [ ] **Step 2: Capture host coverage and Coverity honestly**

Run this script after Task 3 has staged the implementation commit:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import shutil
import subprocess
from pathlib import Path

root = Path("target/posix/aarch64/aio-clone-address-space-quality")
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
path = Path("target/posix/aarch64/aio-clone-address-space-quality.json")
path.write_text(json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n")
print(path)
PY
```

Expected in the current environment: coverage and Coverity are likely recorded as `unavailable` because `cargo-tarpaulin`, `cov-build`, `cov-analyze`, and `cov-format-errors` were absent during planning. Recheck at execution time; never invent a percentage or finding count.

- [ ] **Step 3: Render the detailed POSIX and quality report**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --smros-results target/posix/aarch64/aio-clone-address-space-group/results.ndjson \
  --quality-evidence target/posix/aarch64/aio-clone-address-space-quality.json \
  --out target/posix/aarch64/report-aio-clone-address-space
```

Expected: the seven report artifacts agree on source/build provenance, selected AIO tests, API/group coverage, each non-pass, resource deltas, and coverage/Coverity availability. The report must not claim overall POSIX compliance from this filtered campaign.

- [ ] **Step 4: Verify final repository and evidence state**

```bash
git status --short --branch
git log -3 --oneline --decorate
! rg -n "AIO_KERNEL|AIO_DIAG|pc=0x" src/syscall/syscall.rs src/user_level/services/user_shell.rs
! rg -a "AIO_KERNEL|AIO_DIAG" host_shared/posixtest
find target/posix/aarch64/report-aio-clone-address-space -maxdepth 1 -type f -printf '%f\n' | sort
```

Expected: tracked source is clean; the design, plan, and tested implementation commits are visible; no diagnostic marker is present; and the report contains exactly the documented seven artifacts. Generated stages, images, logs, quality JSON, coverage, and reports remain uncommitted.
