# POSIX AArch64 Runtime Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate transient Linux process-state leakage between serialized AArch64 POSIX launches, correct the suite's obsolete AIO option-version guards, and produce a complete post-isolation failure inventory.

**Architecture:** A single idempotent process-reset boundary drains descriptor-owned handles, raw POSIX timer handles, Linux mappings, `brk` frames, attachments, allocators, container state, signals, and timers at matched `run_elf` lifecycle transitions. Persistent FxFS and named IPC state remain intact. The Open POSIX source correction is an assertion-preserving, SHA-256-bound patch applied by the existing source pipeline.

**Tech Stack:** Rust `no_std` kernel code, production-shared host tests, Python `unittest`, Git patch provenance, AArch64 GNU cross-toolchain, FxFS staging, and QEMU AArch64 system emulation.

---

### Task 1: Reset Transient Linux Process State

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/syscall/syscall.rs`

- [ ] **Step 1: Write the failing process-reset contract test**

Add this test near `posix_resource_snapshot_uses_authoritative_state_without_resetting_it`:

```rust
#[test]
fn linux_process_reset_reclaims_transient_state_without_reinitializing_global_state() {
    let syscall = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall.rs"
    ));
    let method_start = syscall
        .find("fn reset_linux_process_state(&mut self)")
        .expect("memory-state process reset");
    let method = braced_body(&syscall[method_start..]);
    let public_start = syscall
        .find("pub fn reset_linux_process_state()")
        .expect("public process reset");
    let public = braced_body(&syscall[public_start..]);

    for required in [
        "core::mem::take(&mut self.linux_mappings)",
        "MemorySyscallState::free_linux_pages(&mapping.pfns)",
        "core::mem::replace(&mut self.brk, BrkState::new())",
        "MemorySyscallState::free_linux_pages(&brk.pfns)",
        "record.attachments.clear()",
        "self.next_linux_addr = LINUX_MAPPING_BASE",
        "self.next_fd = COMPAT_FD_START",
        "self.reset_linux_container_state()",
    ] {
        assert!(method.contains(required), "missing reset operation: {required}");
    }
    assert!(public.contains("sys_close(fd)"));
    assert!(public.contains("linux_timer_handles"));
    assert!(public.contains("sys_handle_close(handle)"));
    assert!(public.contains("reset_linux_signal_timer_state()"));
    assert!(!method.contains("MemorySyscallState::new()"));
    assert!(!public.contains("MEMORY_SYSCALL_STATE = None"));
}
```

- [ ] **Step 2: Run the contract test and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_process_reset_reclaims_transient_state_without_reinitializing_global_state -- --exact
```

Expected: FAIL because `reset_linux_process_state` does not exist.

- [ ] **Step 3: Track raw Linux timer handles**

Add `linux_timer_handles: Vec<u32>` beside `linux_fds` in
`MemorySyscallState`, initialize it to `Vec::new()`, remove a handle from the
vector in `clear_external_handle_state`, register successful
`sys_linux_timer_create` handles, and unregister successful
`sys_linux_timer_delete` handles:

```rust
fn clear_external_handle_state(&mut self, handle: u32) {
    self.signals.retain(|signal| signal.handle != handle);
    self.linux_timer_handles.retain(|timer| *timer != handle);
}
```

```rust
let handle = compat::create_object(ObjectType::Timer).map_err(|_| SysError::ENOMEM)?;
memory_state().linux_timer_handles.push(handle.0);
```

```rust
if compat::close_handle(HandleValue(timerid as u32)) {
    memory_state()
        .linux_timer_handles
        .retain(|handle| *handle != timerid as u32);
    Ok(0)
} else {
    Err(SysError::EINVAL)
}
```

- [ ] **Step 4: Implement idempotent state teardown**

Add the memory-state method:

```rust
fn reset_linux_process_state(&mut self) {
    let mappings = core::mem::take(&mut self.linux_mappings);
    for mapping in mappings {
        MemorySyscallState::free_linux_pages(&mapping.pfns);
    }
    for record in &mut self.linux_shared_memory {
        record.attachments.clear();
    }

    let brk = core::mem::replace(&mut self.brk, BrkState::new());
    MemorySyscallState::free_linux_pages(&brk.pfns);
    self.linux_fxfs_files.clear();
    self.next_linux_addr = LINUX_MAPPING_BASE;
    self.next_fd = COMPAT_FD_START;
    self.reset_linux_container_state();
}
```

Add the public boundary after `reset_linux_container_state`:

```rust
pub fn reset_linux_process_state() {
    let fds = memory_state()
        .linux_fds
        .iter()
        .map(|record| record.fd)
        .collect::<Vec<_>>();
    for fd in fds {
        let _ = sys_close(fd);
    }

    let timer_handles = core::mem::take(&mut memory_state().linux_timer_handles);
    for handle in timer_handles {
        let _ = sys_handle_close(handle);
    }

    memory_state().reset_linux_process_state();
    reset_linux_signal_timer_state();
}
```

Do not reset `next_handle`, `root_vmar_handle`, FxFS contents,
`linux_shared_memory` objects, or the monotonic PID allocator.

- [ ] **Step 5: Run the contract and integration suite and verify GREEN**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_process_reset_reclaims_transient_state_without_reinitializing_global_state -- --exact
./scripts/run-host-unit-tests.sh --test integration_contracts
```

Expected: both commands PASS.

- [ ] **Step 6: Commit the process-state boundary**

```bash
git add src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "fix: reset Linux process state between ELF runs"
```

### Task 2: Connect Cleanup To Launch Identity

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/user_level/services/run_elf.rs`

- [ ] **Step 1: Make the lifecycle contract require the complete reset**

In `run_elf_terminal_outcomes_are_dispatched_once_after_state_is_cleared`,
replace the signal/timer assertion with:

```rust
assert!(launcher.contains("syscall::reset_linux_process_state()"));
assert_eq!(
    launcher
        .matches("syscall::reset_linux_process_state()")
        .count(),
    4,
    "start, prepare-return, completion, and explicit clear must share cleanup",
);
assert!(!launcher.contains("syscall::reset_linux_signal_timer_state()"));
```

- [ ] **Step 2: Run the lifecycle test and verify RED**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts run_elf_terminal_outcomes_are_dispatched_once_after_state_is_cleared -- --exact
```

Expected: FAIL because the launcher still calls the signal/timer-only hook.

- [ ] **Step 3: Replace all four matched lifecycle reset hooks**

In `spawn_observed`, `prepare_run_elf_return`, `take_active_request`, and
`clear_launch_state_without_outcome`, replace:

```rust
syscall::reset_linux_signal_timer_state()
```

with:

```rust
syscall::reset_linux_process_state()
```

Do not move the callbacks outside the launch-state transitions; their existing
launch-ID matching and lock ordering prevent stale cleanup.

- [ ] **Step 4: Verify lifecycle RED becomes GREEN**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts run_elf_terminal_outcomes_are_dispatched_once_after_state_is_cleared -- --exact
./scripts/run-host-unit-tests.sh --lib run_elf
```

Expected: both commands PASS, including stale/repeated launch-ID reset-count
tests.

- [ ] **Step 5: Commit lifecycle integration**

```bash
git add src/user_level/services/run_elf.rs tests/host/tests/integration_contracts.rs
git commit -m "fix: clean process resources on ELF lifecycle transitions"
```

### Task 3: Correct Obsolete AIO Option Guards

**Files:**
- Modify: `scripts/posix/tests/test_source.py`
- Create: `third_party/posixtest/patches/accept-newer-aio-option-versions.patch`
- Modify: `third_party/posixtest/patches/series`

- [ ] **Step 1: Add a failing repository-patch contract**

Add this test class before `DocumentationTests`:

```python
class RepositoryPatchTests(unittest.TestCase):
    def test_aio_patch_accepts_newer_option_versions_without_weakening_tests(self) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        series_entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        self.assertIn("accept-newer-aio-option-versions.patch", series_entries)

        patch = (patch_root / series_entries[0]).read_text(encoding="ascii")
        self.assertEqual(
            patch.count("-#if _POSIX_ASYNCHRONOUS_IO != 200112L"), 104
        )
        self.assertEqual(
            patch.count("+#if _POSIX_ASYNCHRONOUS_IO < 200112L"), 104
        )
        self.assertNotIn("PTS_PASS", patch)
        self.assertNotIn("PTS_UNSUPPORTED;\n+", patch)
```

- [ ] **Step 2: Run the source test and verify RED**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_source.RepositoryPatchTests.test_aio_patch_accepts_newer_option_versions_without_weakening_tests -v
```

Expected: ERROR or FAIL because the patch is absent from `series`.

- [ ] **Step 3: Generate the mechanical 104-file patch from a temporary clone**

Clone the pinned generated checkout into a temporary directory. Replace only
the exact guard line in C sources:

```text
#if _POSIX_ASYNCHRONOUS_IO != 200112L
```

with:

```text
#if _POSIX_ASYNCHRONOUS_IO < 200112L
```

Capture the temporary clone's unified Git diff with no changes to assertion
bodies, use `apply_patch` to create
`third_party/posixtest/patches/accept-newer-aio-option-versions.patch`, and add
that filename as the only non-comment entry in `patches/series`.

- [ ] **Step 4: Verify the patch contract becomes GREEN**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_source.RepositoryPatchTests.test_aio_patch_accepts_newer_option_versions_without_weakening_tests -v
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_source -v
git diff --check
```

Expected: all source tests PASS; the patch contains exactly 104 removals and
104 replacements and no PTS-result edits.

- [ ] **Step 5: Commit the reviewed source correction**

```bash
git add scripts/posix/tests/test_source.py third_party/posixtest/patches
git commit -m "test: accept newer POSIX AIO option versions"
```

### Task 4: Rebind Source And Stage Provenance

**Files:**
- Generated: `target/posix/src/85555325079ea362fa680bd2209c843cfe47e670/`
- Generated: `host_shared/posixtest/`
- Generated: `target/posix/aarch64/`

- [ ] **Step 1: Preserve and replace the old generated checkout**

First verify the old checkout has the pinned clean tree. Rename it under
`target/posix/src/` rather than deleting it, then fetch the patched checkout:

```bash
git -C target/posix/src/85555325079ea362fa680bd2209c843cfe47e670 status --short
mv target/posix/src/85555325079ea362fa680bd2209c843cfe47e670 target/posix/src/85555325079ea362fa680bd2209c843cfe47e670.unpatched-c51660f
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli fetch
```

Expected: the status output is empty and fetch applies the reviewed patch.

- [ ] **Step 2: Audit the patched checkout**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli audit --check
```

Expected: pinned source/review inventory counts remain unchanged.

- [ ] **Step 3: Cross-build and verify the AArch64 stage**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
```

Expected: 1,598 runnable tests build and the manifest records the nonempty AIO
patch digest plus the current SMROS commit.

### Task 5: Run Offline And Kernel Gates

**Files:**
- No tracked changes expected

- [ ] **Step 1: Run all offline quality gates**

```bash
make host-fmt-check script-check launcher-test linker-layout-test ut it posix-tool-test
git diff --check
```

Expected: every target exits zero and the diff check is silent.

- [ ] **Step 2: Build and validate the AArch64 kernel**

```bash
make build-test ARCH=aarch64-unknown-none
```

Expected: the release kernel builds, `kernel8.img` is refreshed, and the
AArch64 link-layout check passes.

### Task 6: Run A Private-Disk Mapping Canary

**Files:**
- Generated: `target/posix/aarch64/smros-fxfs-runtime-isolation-mmap.img`
- Generated: `target/posix/aarch64/smros-run-runtime-isolation-mmap/`

- [ ] **Step 1: Create and stage a fresh private disk**

```bash
test ! -e target/posix/aarch64/smros-fxfs-runtime-isolation-mmap.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-runtime-isolation-mmap.img 128M
python3 scripts/sync-host-shared.py target/posix/aarch64/smros-fxfs-runtime-isolation-mmap.img host_shared
```

Expected: the fresh image contains the current staged manifest and binaries.
Do not access `smros-fxfs.img`.

- [ ] **Step 2: Run all 33 selected `mmap` tests**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/smros-run-runtime-isolation-mmap"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/smros-fxfs-runtime-isolation-mmap.img"),
    memory="1024M",
    api="mmap",
)
assert result.complete
assert len(result.attempts) == 33
print(f"attempts={len(result.attempts)} passes={sum(a.status == 'pass' for a in result.attempts)}")
PY
```

Expected: 33 terminal attempts complete, crossing the old eight-launch
exhaustion threshold. API assertion failures are retained.

- [ ] **Step 3: Reject loader exhaustion and mapping leakage**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
from pathlib import Path

root = Path("target/posix/aarch64/smros-run-runtime-isolation-mmap")
attempts = [json.loads(line) for line in (root / "results.ndjson").read_text().splitlines()]
raw = (root / "qemu-serial.log").read_text(errors="replace")
assert len(attempts) == 33
assert "cannot create shared object descriptor" not in raw
assert "failed to map segment" not in raw
assert all(a["resource_deltas"]["linux_mappings"] == 0 for a in attempts)
print("mmap canary: no cross-launch mapping leak")
PY
```

Expected: the assertions pass.

### Task 7: Run And Classify All 1,598 Tests

**Files:**
- Generated: `target/posix/aarch64/smros-fxfs-runtime-isolation-all.img`
- Generated: `target/posix/aarch64/smros-run-runtime-isolation-all/`
- Generated: `target/posix/aarch64/report-runtime-isolation/`
- Create after evidence: `docs/posix/2026-08-03-aarch64-post-isolation-results.md`

- [ ] **Step 1: Create a separate full-run private disk**

```bash
test ! -e target/posix/aarch64/smros-fxfs-runtime-isolation-all.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-runtime-isolation-all.img 128M
python3 scripts/sync-host-shared.py target/posix/aarch64/smros-fxfs-runtime-isolation-all.img host_shared
```

- [ ] **Step 2: Execute the complete staged selection**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/smros-run-runtime-isolation-all"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/smros-fxfs-runtime-isolation-all.img"),
    memory="1024M",
)
assert result.complete
assert len(result.attempts) == 1598
print(f"attempts={len(result.attempts)} passes={sum(a.status == 'pass' for a in result.attempts)} restarts={result.restart_count}")
PY
```

Expected: the campaign has 1,598 terminal attempts and a valid `suite_end`.
The command may take substantial time because real timeouts are no longer
hidden by loader failures.

- [ ] **Step 3: Strictly reject the original cascade and summarize real results**

Parse `results.ndjson` and `qemu-serial.log`. Assert that neither loader error
signature occurs and every attempt has zero `linux_mappings` delta. Group
non-passes by `pts_status`, API, group, exit code, and normalized diagnostic
signature. Record totals, API/group coverage, provenance digests, resource
deltas, timeouts, and restart count in
`docs/posix/2026-08-03-aarch64-post-isolation-results.md`.

- [ ] **Step 4: Render detailed report artifacts**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --smros-results target/posix/aarch64/smros-run-runtime-isolation-all/results.ndjson \
  --out target/posix/aarch64/report-runtime-isolation
```

Expected: canonical JSON, JUnit, CSV, Markdown, and HTML artifacts agree on the
1,598-attempt topology and retain every genuine non-pass.

- [ ] **Step 5: Commit only the evidence summary**

```bash
git add docs/posix/2026-08-03-aarch64-post-isolation-results.md
git commit -m "docs: record AArch64 post-isolation POSIX results"
```

Generated disks, stages, logs, and reports remain uncommitted.

### Task 8: Select The Next Semantic Cluster

**Files:**
- No code changes in this task

- [ ] **Step 1: Verify repository and provenance state**

```bash
git status --short --branch
git log -8 --oneline --decorate
```

Expected: only intentional commits are ahead of `origin/master`, the tracked
worktree is clean, and generated evidence remains ignored.

- [ ] **Step 2: Start the next bounded conformance design**

Use the post-isolation inventory to select the highest-impact shared semantic
root cause. The known `fork`/signal/`wait` foundation takes priority unless the
new inventory proves another cluster blocks more tests. Preserve the final gate:
all 1,598 tests must genuinely return `PTS_PASS`.
