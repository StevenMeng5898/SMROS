# POSIX Fork Record-Lock Conformance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the defective Open POSIX `fork/11-1.c` test with its maintained record-lock assertion and implement process-owned `fcntl(F_GETLK/F_SETLK/F_SETLKW)` locks so the corrected test terminates truthfully on AArch64 SMROS.

**Architecture:** A tracked, provenance-audited patch ports Linux Test Project's maintained assertion into the pinned suite. A bounded pure Rust core normalizes `struct flock` ranges and transactionally manages process-owned locks, while a small synchronized runtime connects conflicts and waiters to the existing Linux task scheduler. `sys_fcntl`, FxFS object identity, descriptor close, process exit, signal interruption, and launch reset use that runtime without copying locks during fork.

**Tech Stack:** Rust `no_std`, fixed-capacity arrays, SMROS Linux syscall/task/FxFS runtimes, Python `unittest`, Open POSIX Test Suite C, AArch64 GNU cross tools, QEMU, Verus, Tarpaulin, Coverity when installed.

---

## File Map

- Create `third_party/posixtest/patches/replace-defective-fork-11-record-lock-test.patch`: port the maintained LTP assertion into the pinned suite.
- Modify `third_party/posixtest/patches/series`: apply that patch after the existing two corrections.
- Modify `third_party/posixtest/README.md`: record the exact LTP source commit, URL, checksum, and local entry-point adaptation.
- Modify `scripts/posix/tests/test_source.py`: enforce patch order, provenance, and non-weakening properties.
- Create `src/syscall/linux_record_lock_logic_shared.rs`: pure range normalization, conflict, transactional update, and waiter-table logic.
- Create `src/syscall/linux_record_lock.rs`: synchronized production table, blocking/waking, signal interruption, cleanup, and reset.
- Reuse `src/syscall/linux_runtime_lock_shared.rs`: include the established runtime spinlock inside the new production module; do not create a second lock primitive.
- Modify `src/syscall/mod.rs`: register the production record-lock module.
- Modify `src/syscall/linux_task_logic_shared.rs`: add `LinuxBlockReason::RecordLock`.
- Modify `src/syscall/linux_task.rs`: remove stale record-lock waiters during task retirement.
- Modify `src/syscall/syscall.rs`: marshal AArch64 `struct flock`, route record-lock commands, and connect close/exit/reset cleanup.
- Modify `src/syscall/syscall_logic_shared.rs` and `src/syscall/syscall_logic.rs`: recognize all three record-lock `fcntl` commands.
- Modify `src/user_level/services/fxfs.rs`: expose attributes by live `FxfsCursor` object identity for `SEEK_END` normalization.
- Modify `tests/host/src/lib.rs`: host unit tests for normalization, table atomicity, ownership, lifecycle, and waiters.
- Modify `tests/host/tests/integration_contracts.rs`: source-level contracts for ABI, syscall routing, wakeup ordering, and lifecycle wiring.
- Modify `verification/syscall/src/lib.rs`: wire the new shared logic into the Verus coverage/proof harness.
- Modify `docs/VERUS_COVERAGE.md`: classify both new syscall source files.
- Create `docs/posix/2026-08-12-aarch64-fork-record-lock-results.md`: record exact build, runtime, resource, coverage, and static-analysis evidence.

Generated checkouts below `target/posix/src`, staged files below `host_shared/posixtest`, private QEMU disks, and result directories are evidence only and are never committed as source behavior.

### Task 1: Audit And Port The Maintained Fork Assertion

**Files:**
- Modify: `scripts/posix/tests/test_source.py`
- Create: `third_party/posixtest/patches/replace-defective-fork-11-record-lock-test.patch`
- Modify: `third_party/posixtest/patches/series`
- Modify: `third_party/posixtest/README.md`

- [ ] **Step 1: Write the failing repository patch audit**

Add this test to `RepositoryPatchTests`:

```python
def test_fork_11_patch_ports_ltp_record_lock_assertion_without_weakening(self) -> None:
    patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
    entries = [
        line.strip()
        for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    name = "replace-defective-fork-11-record-lock-test.patch"
    self.assertEqual(entries.count(name), 1)
    self.assertEqual(entries[-1], name)

    patch = (patch_root / name).read_text(encoding="utf-8")
    self.assertIn("conformance/interfaces/fork/11-1.c", patch)
    removed_lines = [
        line[1:].strip()
        for line in patch.splitlines()
        if line.startswith("-") and not line.startswith("---")
    ]
    for removed in ("flockfile( stdout );", "ret = ftrylockfile( stdout );", '#include "testfrmw.c"'):
        self.assertIn(removed, removed_lines)
    for retained in (
        "fcntl(fd, F_GETLK, &fl)",
        "fcntl(fd, F_SETLK, &fl)",
        "errno == EACCES || errno == EAGAIN",
        ".l_start = 0",
        ".l_len = 100",
        ".l_start = 1",
        ".l_len = 99",
        "child_pid = fork()",
        "waitpid(child_pid, &child_stat, 0)",
        "result = WEXITSTATUS(child_stat)",
    ):
        self.assertIn(retained, patch)
    self.assertNotIn("PTS_ATTRIBUTE_UNUSED", patch)
    self.assertNotIn("timeout_ms", patch)
    self.assertNotIn("SMROS", patch)
    unconditional_pass = [
        line
        for line in patch.splitlines()
        if line.startswith("+") and "return PTS_PASS;" in line
    ]
    self.assertEqual(len(unconditional_pass), 1)
    self.assertIn("errno == EACCES || errno == EAGAIN", patch)

    readme = (patch_root.parent / "README.md").read_text(encoding="utf-8")
    self.assertIn("0b69550e055b5385822f001e2a27fedfbef31816", readme)
    self.assertIn("fcf9b794dd054586f65625ee6dd9a5daee61b98c1a43887de57e8c230a7d1626", readme)
```

The unconditional-pass regex is intentionally line anchored. The maintained test may return `PTS_PASS` only inside the `EACCES || EAGAIN` conflict branch.

- [ ] **Step 2: Run the test and verify RED**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  scripts.posix.tests.test_source.RepositoryPatchTests.test_fork_11_patch_ports_ltp_record_lock_assertion_without_weakening -v
```

Expected: FAIL because the patch is absent from `series`.

- [ ] **Step 3: Create the tracked port and provenance record**

Use the exact upstream source at:

```text
https://raw.githubusercontent.com/linux-test-project/ltp/0b69550e055b5385822f001e2a27fedfbef31816/testcases/open_posix_testsuite/conformance/interfaces/fork/11-1.c
sha256: fcf9b794dd054586f65625ee6dd9a5daee61b98c1a43887de57e8c230a7d1626
```

Generate the patch against pinned `conformance/interfaces/fork/11-1.c`, but adapt the LTP entry point to the older pinned harness:

```c
int main(int argc, char **argv)
{
    (void)argc;
    (void)argv;
    /* The remaining body is the maintained LTP test_main body unchanged. */
}
```

Do not import `PTS_ATTRIBUTE_UNUSED`, because the pinned `include/posixtest.h` does not define it. Preserve all maintained `F_GETLK`, `F_SETLK`, overlapping-range, error, fork, wait, and result-propagation branches. Append the patch name exactly once to `series`. In `README.md`, record the commit, raw URL, source SHA-256, and the `test_main` to `main` compatibility adaptation.

- [ ] **Step 4: Run the audit and patch-application tests**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  scripts.posix.tests.test_source.RepositoryPatchTests.test_fork_11_patch_ports_ltp_record_lock_assertion_without_weakening -v
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_source -v
```

Expected: PASS, including the existing Git-tree and patch-digest reuse checks.

- [ ] **Step 5: Materialize a fresh patched checkout and run the corrected test natively**

```bash
work="target/posix/fork-record-lock-source-check"
test ! -e "$work"
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli fetch --work-dir "$work"
src="$work/src/85555325079ea362fa680bd2209c843cfe47e670"
cc -std=c11 -D_GNU_SOURCE -Wall -Wextra -Werror \
  -I"$src/include" "$src/conformance/interfaces/fork/11-1.c" \
  -o "$work/fork-11-1-native"
timeout 5s "$work/fork-11-1-native"
```

Expected: compile exits zero; the native run prints the maintained pass diagnostic and exits `0` before five seconds. This confirms the port is executable independently of SMROS.

- [ ] **Step 6: Commit the audited suite correction**

```bash
git add scripts/posix/tests/test_source.py \
  third_party/posixtest/patches/replace-defective-fork-11-record-lock-test.patch \
  third_party/posixtest/patches/series third_party/posixtest/README.md
git commit -m "test: replace defective fork record-lock assertion"
```

### Task 2: Build The Pure Transactional Lock Core

**Files:**
- Create: `src/syscall/linux_record_lock_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `verification/syscall/src/lib.rs`

- [ ] **Step 1: Add the shared module and failing normalization tests**

In `tests/host/src/lib.rs`, include the new file in a `linux_record_lock_logic` module and add tests with this public shape:

```rust
mod linux_record_lock_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_record_lock_logic_shared.rs"
    ));

    #[test]
    fn record_lock_ranges_normalize_all_whence_and_length_forms() {
        assert_eq!(
            normalize_linux_record_lock_range(0, 10, 20, 40, 80),
            Ok(LinuxRecordLockRange::finite(10, 30).unwrap())
        );
        assert_eq!(
            normalize_linux_record_lock_range(1, -5, 10, 40, 80),
            Ok(LinuxRecordLockRange::finite(35, 45).unwrap())
        );
        assert_eq!(
            normalize_linux_record_lock_range(2, -20, 0, 40, 80),
            Ok(LinuxRecordLockRange::to_eof(60))
        );
        assert_eq!(
            normalize_linux_record_lock_range(0, 30, -10, 40, 80),
            Ok(LinuxRecordLockRange::finite(20, 30).unwrap())
        );
    }

    #[test]
    fn record_lock_range_errors_distinguish_invalid_from_overflow() {
        assert_eq!(
            normalize_linux_record_lock_range(3, 0, 1, 0, 0),
            Err(LinuxRecordLockRangeError::Invalid)
        );
        assert_eq!(
            normalize_linux_record_lock_range(0, -1, 1, 0, 0),
            Err(LinuxRecordLockRangeError::Invalid)
        );
        assert_eq!(
            normalize_linux_record_lock_range(0, i64::MAX, 1, 0, 0),
            Err(LinuxRecordLockRangeError::Overflow)
        );
        assert_eq!(
            normalize_linux_record_lock_range(1, 0, 1, u64::MAX, 0),
            Err(LinuxRecordLockRangeError::Overflow)
        );
    }
}
```

- [ ] **Step 2: Run the normalization tests and verify RED**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock_range -- --nocapture
```

Expected: compile failure because the module and types do not exist.

- [ ] **Step 3: Implement exact range types and normalization**

Create these core declarations:

```rust
pub(crate) const LINUX_RECORD_LOCK_END_OF_FILE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockRangeError { Invalid, Overflow }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRecordLockRange { pub start: u64, pub end: u64 }

impl LinuxRecordLockRange {
    pub(crate) fn finite(start: u64, end: u64) -> Option<Self> {
        (start < end && end != LINUX_RECORD_LOCK_END_OF_FILE).then_some(Self { start, end })
    }
    pub(crate) const fn to_eof(start: u64) -> Self {
        Self { start, end: LINUX_RECORD_LOCK_END_OF_FILE }
    }
}
```

`normalize_linux_record_lock_range(whence, l_start, l_len, cursor, file_size)` must:

1. convert only the base selected by `whence` to `i64`, returning `Overflow` on failure;
2. map whence `0/1/2` to bases `0/cursor/file_size`, returning `Invalid` otherwise;
3. checked-add `l_start` to the base, returning `Overflow` on arithmetic failure;
4. reject a negative resolved anchor as `Invalid`;
5. map positive lengths to `[anchor, anchor + len)`, zero to `[anchor, EOF)`, and negative lengths to `[anchor + len, anchor)`;
6. return `Overflow` for checked-add failure and `Invalid` when the negative-length start is below zero.

Define and use `smros_linux_record_lock_ranges_overlap_body!` and `smros_linux_record_lock_types_conflict_body!` macros so the arithmetic predicates remain available to the Verus harness.

- [ ] **Step 4: Verify GREEN for range normalization**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock_range -- --nocapture
```

Expected: both range tests pass.

- [ ] **Step 5: Write failing ownership, replacement, split, coalescing, and capacity tests**

Add independent tests using:

```rust
let mut locks = LinuxRecordLockTable::<4>::new();
locks.set(7, 100, LinuxRecordLockKind::Write, range(0, 100)).unwrap();
```

Cover these exact outcomes:

- owner `100` does not conflict with itself, but owner `101` sees a write conflict with PID `100`;
- a read lock permits another read lock but conflicts with another owner's write lock;
- replacing `[20, 80)` in an owner's `[0, 100)` write lock with read produces write `[0,20)`, read `[20,80)`, write `[80,100)`;
- unlocking `[20,80)` splits `[0,100)` without touching another file or owner;
- adjacent same-owner/same-file/same-type records coalesce;
- a zero-length range remains open through future growth;
- a capacity error returns `LinuxRecordLockTableError::Capacity` and leaves `snapshot()` byte-for-byte equal to the pre-call snapshot;
- `release_owner_file(owner, file)` and `release_owner(owner)` remove only matching records;
- a child owner starts with no records while the parent's records remain visible as conflicts.

- [ ] **Step 6: Run the table tests and verify RED**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock_table -- --nocapture
```

Expected: compile failure because the table API is absent.

- [ ] **Step 7: Implement the bounded table transactionally**

Add these types and methods:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockKind { Read, Write }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRecordLock {
    pub file_id: u64,
    pub owner: usize,
    pub kind: LinuxRecordLockKind,
    pub range: LinuxRecordLockRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockTableError { Capacity }

#[derive(Clone, Copy)]
pub(crate) struct LinuxRecordLockTable<const N: usize> {
    records: [Option<LinuxRecordLock>; N],
}
```

Implement `const fn new`, `first_conflict`, `set`, `unlock`, `release_owner_file`, `release_owner`, `snapshot`, and `reset`. `set` and `unlock` must edit a copied table, preserve the left and right pieces of each overlapping owner record, insert or remove the requested range, sort by `(file_id, owner, start, end, kind)`, coalesce compatible adjacent records, and assign `*self = candidate` only after all inserts succeed. Never mutate the published table before a capacity check completes.

- [ ] **Step 8: Verify GREEN and wire shared logic into Verus**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock -- --nocapture
```

Include `linux_record_lock_logic_shared.rs` in a dead-code-allowed module in `verification/syscall/src/lib.rs`, and add Verus assertions for touching versus overlapping ranges and read/read versus read/write conflicts. Then run:

```bash
make verus-syscall
```

Expected: all focused host tests and the syscall proof harness pass.

- [ ] **Step 9: Commit the pure core**

```bash
git add src/syscall/linux_record_lock_logic_shared.rs tests/host/src/lib.rs \
  verification/syscall/src/lib.rs
git commit -m "feat: add transactional POSIX record-lock core"
```

### Task 3: Add Bounded Waiter State And Production Blocking

**Files:**
- Modify: `src/syscall/linux_record_lock_logic_shared.rs`
- Create: `src/syscall/linux_record_lock.rs`
- Modify: `src/syscall/mod.rs`
- Modify: `src/syscall/linux_task_logic_shared.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing pure waiter tests**

Add tests proving:

```rust
let waiter = LinuxRecordLockWaiter::new(7, 101, LinuxRecordLockKind::Write,
    range(20, 40), 11, 12);
assert_eq!(waiters.push(waiter), Ok(()));
assert_eq!(waiters.interrupt(11, 12), true);
assert_eq!(waiters.take_outcome(11, 12), Some(LinuxRecordLockWaitOutcome::Interrupted));
```

Also prove FIFO insertion, duplicate task registration rejection, `Capacity`, wake only after `first_conflict` becomes `None`, cleanup by task identity, and reset to an empty table.

- [ ] **Step 2: Run waiter tests and verify RED**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock_waiter -- --nocapture
```

Expected: compile failure because waiter state is absent.

- [ ] **Step 3: Implement fixed-capacity waiter state**

Define:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockWaitOutcome { Waiting, Woken, Interrupted }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRecordLockWaiter {
    pub file_id: u64,
    pub owner: usize,
    pub kind: LinuxRecordLockKind,
    pub range: LinuxRecordLockRange,
    pub tid: usize,
    pub scheduler_thread: usize,
    pub sequence: u64,
    pub outcome: LinuxRecordLockWaitOutcome,
}

pub(crate) struct LinuxRecordLockState<const L: usize, const W: usize> {
    pub locks: LinuxRecordLockTable<L>,
    waiters: [Option<LinuxRecordLockWaiter>; W],
    next_sequence: u64,
}
```

Implement `const fn new` for the state and the waiter constructor shown in Step 1. `wake_ready` marks waiters `Woken` only when their exact request no longer conflicts, returns `[Option<(usize, usize)>; W]`, and preserves FIFO sequence. `interrupt`, `remove_task`, and `take_outcome` address waiters by both TID and scheduler-thread identity.

- [ ] **Step 4: Verify GREEN for pure waiter behavior**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock_waiter -- --nocapture
```

- [ ] **Step 5: Write the failing production integration contract**

Add `linux_record_lock_runtime_blocks_without_missed_wakeups` to `integration_contracts.rs`. It must require:

- `LinuxBlockReason::RecordLock` exists;
- `linux_record_lock.rs` uses `LinuxRuntimeLock<LinuxRecordLockState<...>>`;
- blocking record-lock operations reject execution away from Linux runtime CPU 0;
- the conflicting request is rechecked while holding the record runtime lock;
- waiter publication precedes `linux_task::block_current(LinuxBlockReason::RecordLock)`;
- the runtime guard is dropped before `block_current`, avoiding record-lock/task-lock inversion;
- `block_current` and `scheduler::schedule()` occur before the saved interrupt state is restored;
- wake identities are collected under the runtime lock and passed to `linux_task::wake_blocked` only after it is released;
- signal delivery routes `RecordLock` to `linux_record_lock::interrupt_task`;
- task retirement calls `linux_record_lock::remove_task_waiters` alongside futex cleanup.

- [ ] **Step 6: Run the contract and verify RED**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts \
  linux_record_lock_runtime_blocks_without_missed_wakeups -- --exact
```

Expected: FAIL because the production module and block reason do not exist.

- [ ] **Step 7: Implement the production runtime**

At the top of `linux_record_lock.rs`, include the established runtime lock exactly as the futex runtime does:

```rust
include!("linux_record_lock_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");
```

Use limits `LINUX_RECORD_LOCK_LIMIT = 64` and `LINUX_RECORD_LOCK_WAITER_LIMIT = linux_task::LINUX_TASK_LIMIT`. Expose:

```rust
pub(crate) fn first_conflict(file_id: u64, owner: usize, kind: LinuxRecordLockKind,
    range: LinuxRecordLockRange) -> Option<LinuxRecordLock>;
pub(crate) fn set_nonblocking(file_id: u64, owner: usize, kind: Option<LinuxRecordLockKind>,
    range: LinuxRecordLockRange) -> Result<(), LinuxRecordLockRuntimeError>;
pub(crate) fn set_blocking(file_id: u64, owner: usize, kind: LinuxRecordLockKind,
    range: LinuxRecordLockRange) -> Result<(), SysError>;
pub(crate) fn release_owner_file(owner: usize, file_id: u64);
pub(crate) fn release_owner(owner: usize);
pub(crate) fn interrupt_task(tid: usize, scheduler_thread: usize) -> bool;
pub(crate) fn remove_task_waiters(tid: usize, scheduler_thread: usize) -> usize;
pub(crate) fn reset();
```

Define production error mapping explicitly:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockRuntimeError { Conflict, Capacity }
```

Every successful `set_nonblocking` mutation, including `F_UNLCK` and an owner-range replacement, calls `wake_ready` while the new table is committed, drops the record runtime guard, and only then wakes returned task identities. Cleanup operations use the same post-commit wake path.

For blocking set, first reject a caller away from Linux runtime CPU 0. Mask interrupts, lock the record runtime, recheck the conflict, publish the waiter, and drop the runtime guard before entering the task runtime. Call `block_current` and `scheduler::schedule()` while interrupts remain masked as in the futex path, consume the outcome after resumption, and only then restore the saved interrupt state. If blocking fails, reacquire the record runtime and remove the waiter before restoring interrupts. On `Woken`, retry the complete lock operation; map `Interrupted` to `EINTR`. Map waiter or lock capacity to `ENOLCK`. Because all Linux tasks and record-lock wake sources are CPU0-bound, interrupt masking closes the publication-to-block window without nesting the record and task runtime locks.

Add the module to `mod.rs`, add the block reason, interrupt it from `interrupt_linux_signal_target`, and remove waiters from `complete_task_retirements`.

- [ ] **Step 8: Verify production wiring and commit**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock -- --nocapture
./scripts/run-host-unit-tests.sh --test integration_contracts \
  linux_record_lock_runtime_blocks_without_missed_wakeups -- --exact
make verus-syscall
```

Expected: PASS.

```bash
git add src/syscall/linux_record_lock_logic_shared.rs src/syscall/linux_record_lock.rs \
  src/syscall/mod.rs src/syscall/linux_task_logic_shared.rs src/syscall/linux_task.rs \
  src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: block and wake POSIX record-lock waiters"
```

### Task 4: Expose Stable FxFS Cursor Attributes

**Files:**
- Modify: `src/user_level/services/fxfs.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write a failing object-identity contract**

Add `fxfs_cursor_identity_drives_record_lock_size_lookup`. Require `FxfsCursor::object_id()`, a public `cursor_attrs(cursor: FxfsCursor)` function, and implementation lookup through `cursor.object_id` rather than the stored pathname. Require `sys_fcntl` to use `file.cursor.object_id()` and `fxfs::cursor_attrs(file.cursor)`.

- [ ] **Step 2: Run it and verify RED**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts \
  fxfs_cursor_identity_drives_record_lock_size_lookup -- --exact
```

Expected: FAIL because `cursor_attrs` does not exist.

- [ ] **Step 3: Add cursor-based attribute lookup**

Inside `FxfsState`, add:

```rust
fn cursor_attrs(&mut self, cursor: FxfsCursor) -> Result<FxfsAttributes, FxfsError> {
    let index = self
        .objects
        .iter()
        .position(|object| object.object_id == cursor.object_id)
        .ok_or(FxfsError::NotFound)?;
    Ok(self.objects[index].attrs)
}
```

Expose `pub fn cursor_attrs(cursor: FxfsCursor)`. Do not resolve the saved path: descriptor numbers and pathnames can be reused, while an open cursor already carries the stable object identity.

- [ ] **Step 4: Verify and commit**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts \
  fxfs_cursor_identity_drives_record_lock_size_lookup -- --exact
make aarch64-warning-check
```

Expected: PASS with no warnings.

```bash
git add src/user_level/services/fxfs.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: expose FxFS attributes by cursor identity"
```

### Task 5: Marshal `struct flock` And Route `fcntl`

**Files:**
- Modify: `src/syscall/syscall.rs`
- Modify: `src/syscall/syscall_logic.rs`
- Modify: `src/syscall/syscall_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing command and ABI tests**

Extend the existing `linux_fcntl_cmd_supported` unit tests to require commands `5`, `6`, and `7`. Add an integration contract requiring these AArch64 wire constants:

```rust
const LINUX_FLOCK_BYTES: usize = 32;
const LINUX_FLOCK_TYPE_OFFSET: usize = 0;
const LINUX_FLOCK_WHENCE_OFFSET: usize = 2;
const LINUX_FLOCK_START_OFFSET: usize = 8;
const LINUX_FLOCK_LEN_OFFSET: usize = 16;
const LINUX_FLOCK_PID_OFFSET: usize = 24;
```

Require native-endian field parsing through `linux_wire_field`/`linux_put_wire_field`, `linux_copy_from_user`, and `linux_copy_to_user`, rather than casting an untrusted pointer to `LinuxFlock`.

- [ ] **Step 2: Run and verify RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_fcntl -- --nocapture
./scripts/run-host-unit-tests.sh --test integration_contracts \
  linux_fcntl_marshals_aarch64_record_locks -- --exact
```

Expected: the new commands and ABI contract fail.

- [ ] **Step 3: Add exact errors, wire helpers, and command recognition**

Add `EBADF = 9` and `ENOLCK = 37` to `SysError`. Define a private decoded structure:

```rust
#[derive(Clone, Copy)]
struct LinuxFlock {
    l_type: i16,
    l_whence: i16,
    l_start: i64,
    l_len: i64,
    l_pid: i32,
}
```

Implement `linux_read_flock` and `linux_write_flock` against a 32-byte local array and the exact offsets above. Extend `linux_fcntl_cmd_supported` and its shared macro to accept `F_GETLK=5`, `F_SETLK=6`, and `F_SETLKW=7` while preserving existing commands.

- [ ] **Step 4: Implement record-lock routing in `sys_fcntl`**

For all three commands:

1. look up the fd and return `EBADF` if absent;
2. require `ObjectType::LinuxFile` and an FxFS record, otherwise `EINVAL`;
3. copy in the complete flock, returning `EFAULT` on any bad read;
4. validate type `F_RDLCK=0` or `F_WRLCK=1` for `F_GETLK`, and `F_RDLCK`, `F_WRLCK`, or `F_UNLCK=2` for the set commands;
5. return `EBADF` when installing a read lock on a write-only description or a write lock on a read-only description;
6. obtain cursor offset, `cursor_attrs(...).size`, and cursor object ID;
7. normalize the range, mapping `Invalid -> EINVAL` and `Overflow -> EOVERFLOW`.

Collect the descriptor access mode, cursor offset, file size, object ID, and owner PID into copied locals inside a short `memory_state()` scope. End that borrow before calling `set_blocking`; a task must never schedule while retaining access to global syscall descriptor state.

`F_GETLK` searches another owner's conflict. On no conflict, preserve every caller field except copy out `l_type=F_UNLCK`; on conflict, copy out its type, `SEEK_SET`, normalized start, zero length for EOF or exact finite length, and owner PID. Reject a non-writable destination with `EFAULT` before returning success.

`F_SETLK` calls `set_nonblocking`; map conflict to `EAGAIN` and capacity to `ENOLCK`. `F_UNLCK` never conflicts. `F_SETLKW` uses the blocking runtime for read/write and performs a nonblocking unlock for `F_UNLCK`.

- [ ] **Step 5: Write and run focused syscall lifecycle contracts**

Add contracts proving `sys_fcntl` routes all three commands, uses PID/TGID ownership, does not special-case `fork/11-1.c`, never lowers a timeout, and maps every error above. Run:

```bash
./scripts/run-host-unit-tests.sh --lib linux_fcntl -- --nocapture
./scripts/run-host-unit-tests.sh --test integration_contracts linux_fcntl_ -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit the syscall API**

```bash
git add src/syscall/syscall.rs src/syscall/syscall_logic.rs \
  src/syscall/syscall_logic_shared.rs tests/host/src/lib.rs \
  tests/host/tests/integration_contracts.rs
git commit -m "feat: implement POSIX fcntl record locks"
```

### Task 6: Connect Close, Exit, Fork, Signal, And Reset Lifecycle

**Files:**
- Modify: `src/syscall/syscall.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing lifecycle contracts**

Add `linux_record_locks_follow_process_associated_lifecycle` and require:

- `sys_close` resolves the FxFS object ID before descriptor removal, then releases all current-owner locks for that file even if another duplicate remains;
- `dup3` replacement performs the same implicit-close release for the replaced target fd;
- `close_range` inherits behavior by calling `sys_close`;
- `release_linux_process_resources(pid)` calls `linux_record_lock::release_owner(pid)` before handles disappear;
- fork reservation/installation never clones lock records or changes parent ownership;
- signal and normal exit both pass through process-resource release;
- reset clears record locks and waiters;
- task retirement removes record-lock waiter state;
- no close-on-exec claim is added to the current stub `sys_execve` path.

- [ ] **Step 2: Run and verify RED**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts \
  linux_record_locks_follow_process_associated_lifecycle -- --exact
```

Expected: FAIL because close/exit/reset cleanup is not yet wired.

- [ ] **Step 3: Centralize current-process descriptor close**

Extract one internal close helper that obtains the current PID and target object's cursor ID before removing the descriptor. After successful removal, call `release_owner_file(pid, file_id)` regardless of the open-description reference count, then release the open description and handle as today. Use it from `sys_close` and the target replacement branch of `sys_dup3`. Preserve fd `0..2` shell behavior only when no descriptor entry exists.

- [ ] **Step 4: Wire process and launch cleanup**

At the start of `release_linux_process_resources(pid)`, call `linux_record_lock::release_owner(pid)` and wake every waiter made ready by that committed removal. In `reset_linux_process_state`, call `linux_record_lock::reset()` while tasks are retired and before a new launch can publish work. Do not touch lock state in `reserve_process_resources`, `install_process_resources`, or fork rollback.

- [ ] **Step 5: Add behavioral table tests for close/fork/exit semantics**

Use the pure table to prove:

```rust
locks.set(file, parent, LinuxRecordLockKind::Write, range(0, 100)).unwrap();
assert!(locks.first_conflict(file, child, LinuxRecordLockKind::Write, range(1, 100)).is_some());
locks.release_owner_file(parent, file);
assert!(locks.first_conflict(file, child, LinuxRecordLockKind::Write, range(1, 100)).is_none());
```

Add separate cases for duplicate close, unrelated-file close, child exit preserving parent locks, parent exit waking child, and fork rollback preserving parent records.

- [ ] **Step 6: Verify and commit lifecycle behavior**

```bash
./scripts/run-host-unit-tests.sh --lib record_lock -- --nocapture
./scripts/run-host-unit-tests.sh --test integration_contracts \
  linux_record_locks_follow_process_associated_lifecycle -- --exact
make aarch64-warning-check
```

Expected: PASS with no Rust warning.

```bash
git add src/syscall/syscall.rs src/syscall/linux_task.rs \
  tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: release POSIX record locks by process lifecycle"
```

### Task 7: Rebuild The Signed Stage And Run Offline Gates

**Files:**
- Modify: `docs/VERUS_COVERAGE.md`
- Generated: `host_shared/posixtest/`
- Generated: `kernel8.img`

- [ ] **Step 1: Classify new source files and run formatting**

Add both new Rust paths in sorted position under the syscall section of `docs/VERUS_COVERAGE.md`.

```bash
rustfmt --edition 2021 --check \
  src/syscall/linux_record_lock_logic_shared.rs src/syscall/linux_record_lock.rs \
  src/syscall/linux_task_logic_shared.rs src/syscall/linux_task.rs \
  src/syscall/syscall_logic_shared.rs src/syscall/syscall_logic.rs \
  src/syscall/syscall.rs src/user_level/services/fxfs.rs \
  tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git diff --check
```

Expected: zero exit and no whitespace errors.

- [ ] **Step 2: Run all offline repository gates**

```bash
make host-fmt-check
make script-check
make launcher-test
make linker-layout-test
make ut
make it
make posix-tool-test
make verus
```

Expected: every suite passes. Record actual counts; do not reuse counts from an earlier commit.

- [ ] **Step 3: Rebuild and verify the exact pinned AArch64 stage**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build \
  --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build \
  --arch aarch64 --stage host_shared/posixtest --verify-only
```

Expected: `fork/11-1.c` is complete, its binary hash changes from the defective version, metadata binds the current commit and new nonzero patch checksum, and all 1,598 reviewed entries remain represented.

- [ ] **Step 4: Build the warning-free production kernel**

```bash
make aarch64-warning-check
```

Expected: optimized build, link, and layout validation exit zero with `-D warnings` and no warning output.

- [ ] **Step 5: Commit verification metadata only if tracked files changed**

```bash
git add docs/VERUS_COVERAGE.md
git commit -m "docs: classify POSIX record-lock verification"
```

Do not add `host_shared/posixtest`, `kernel8.img`, or `target` evidence.

### Task 8: Run Three Fresh-Disk Focused SMROS Attempts

**Files:**
- Generated: `target/posix/aarch64/fork-record-lock-${commit}-run-{1,2,3}/`
- Generated: `target/posix/aarch64/fork-record-lock-${commit}-disk-{1,2,3}.img`

- [ ] **Step 1: Create one private disk per attempt**

```bash
commit=$(git rev-parse --short=12 HEAD)
for run in 1 2 3; do
  disk="target/posix/aarch64/fork-record-lock-${commit}-disk-${run}.img"
  test ! -e "$disk"
  qemu-img create -f raw "$disk" 128M
done
```

Never read, modify, stop, or reuse the user-owned QEMU process or its disk. In particular, do not signal PID `191326`; re-check ownership at execution time rather than assuming that PID remains current.

- [ ] **Step 2: Run the corrected test three times**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
import subprocess

from scripts.posix.qemu_runner import run_smros

commit = subprocess.check_output(
    ["git", "rev-parse", "--short=12", "HEAD"], text=True
).strip()
for run in range(1, 4):
    disk = Path(f"target/posix/aarch64/fork-record-lock-{commit}-disk-{run}.img")
    output = Path(f"target/posix/aarch64/fork-record-lock-{commit}-run-{run}")
    assert disk.is_file() and not output.exists()
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M",
        test_id="conformance/interfaces/fork/11-1.c",
    )
    assert result.complete and len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.status == "pass", attempt
    assert not attempt.timed_out
    assert attempt.launch_status == "launched"
    print(run, attempt.status, attempt.duration_ms)
PY
```

Expected: all three attempts reach a genuine guest `test_end` with `pts_status=pass`, exit code `0`, no watchdog timeout, and elapsed time below 30 seconds.

- [ ] **Step 3: Audit raw evidence and resource deltas**

Parse each `results.ndjson` with `json.loads`. Require exact manifest/build/patch provenance; zero restarts; no `Kernel panic`, fatal glibc, translation fault, allocator corruption, or host-watchdog marker; and non-positive terminal deltas for Linux fds, mappings, processes, zombies, page-table pages, private/shared pages, scheduler threads, handles, IPC objects, AIO requests, and timers.

If any attempt fails an assertion, retain its truthful status and return to the smallest failing unit or integration test. Never convert it to pass or raise the watchdog.

### Task 9: Run Fork API And Adjacent Canaries

**Files:**
- Generated: `target/posix/aarch64/fork-record-lock-${commit}-api/`
- Generated: fresh private disk images per selection

- [ ] **Step 1: Run the complete staged `fork` API selection**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from collections import Counter
import subprocess

from scripts.posix.qemu_runner import run_smros

commit = subprocess.check_output(
    ["git", "rev-parse", "--short=12", "HEAD"], text=True
).strip()
disk = Path(f"target/posix/aarch64/fork-record-lock-{commit}-api.img")
output = Path(f"target/posix/aarch64/fork-record-lock-{commit}-api")
assert not disk.exists() and not output.exists()
subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
result = run_smros(
    Path("host_shared/posixtest"), output,
    kernel=Path("kernel8.img"), disk=disk, memory="1024M", api="fork",
)
assert result.complete
assert result.restart_count == 0
assert len(result.attempts) == 19
assert all(not attempt.timed_out for attempt in result.attempts)
assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
corrected = next(
    attempt for attempt in result.attempts
    if attempt.test_id == "conformance/interfaces/fork/11-1.c"
)
assert corrected.status == "pass" and corrected.exit_code == 0
print(Counter(attempt.status for attempt in result.attempts))
print(result.result_path, result.raw_log_path)
PY
```

Expected: all 19 currently reviewed runnable `fork` tests terminate. If the reviewed inventory changes before execution, derive the expected selected count from the verified manifest instead of silently accepting a mismatch. Preserve unrelated pass/fail/unresolved/unsupported/untested outcomes exactly.

- [ ] **Step 2: Run adjacent descriptor/process canaries**

Run each of these in a separate fresh QEMU process and disk:

```text
conformance/interfaces/fork/1-1.c
conformance/interfaces/fork/12-1.c
conformance/behavior/WIFEXITED/1-3.c
```

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
import subprocess

from scripts.posix.qemu_runner import run_smros

tests = (
    "conformance/interfaces/fork/1-1.c",
    "conformance/interfaces/fork/12-1.c",
    "conformance/behavior/WIFEXITED/1-3.c",
    "conformance/interfaces/pthread_kill/1-1.c",
)
commit = subprocess.check_output(
    ["git", "rev-parse", "--short=12", "HEAD"], text=True
).strip()
for index, test_id in enumerate(tests, 1):
    disk = Path(f"target/posix/aarch64/fork-record-lock-{commit}-canary-{index}.img")
    output = Path(f"target/posix/aarch64/fork-record-lock-{commit}-canary-{index}")
    assert not disk.exists() and not output.exists()
    subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M", test_id=test_id,
    )
    assert result.complete and result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert not attempt.timed_out and not attempt.resource_deltas.has_positive()
    print(test_id, attempt.status, attempt.duration_ms, result.raw_log_path)
PY
```

Before running, read these sources and record what each canary covers. Also search the verified manifest/source at execution time for any newly reviewed test that directly exercises descriptor duplication, ordinary descriptor close, `fcntl`, thread signal interruption, or wait/exit. Run each discovered candidate on a fresh disk and record it; do not infer a pass from a missing API directory.

- [ ] **Step 3: Validate campaign completeness**

Require selected count to equal terminal attempt count, no host watchdog, no restart, and no positive resource delta. Report every non-pass by exact test ID and PTS diagnostic. The filtered campaign proves this affected surface only; it must not be labeled full POSIX compliance.

### Task 10: Capture Coverage, Coverity, And Final Results

**Files:**
- Generate: `target/posix/aarch64/fork-record-lock-quality/`
- Generate: `target/posix/aarch64/fork-record-lock-quality.json`
- Create: `docs/posix/2026-08-12-aarch64-fork-record-lock-results.md`

- [ ] **Step 1: Re-run final gates at the evidence commit**

```bash
make host-fmt-check script-check launcher-test linker-layout-test
make ut it posix-tool-test aarch64-warning-check verus
git diff --check
```

Expected: all exit zero, with no AArch64 warning.

- [ ] **Step 2: Capture host coverage honestly**

```bash
quality="target/posix/aarch64/fork-record-lock-quality"
test ! -e "$quality"
mkdir -p "$quality"
if command -v cargo-tarpaulin >/dev/null 2>&1; then
  set +e
  make coverage-host >"$quality/coverage-host.log" 2>&1
  printf '%s\n' "$?" >"$quality/coverage-host.exit"
  set -e
else
  printf '%s\n' 'cargo-tarpaulin is not installed' >"$quality/coverage-host.log"
  printf '%s\n' 'unavailable' >"$quality/coverage-host.exit"
fi
```

If Tarpaulin is absent, record `status="unavailable"`, `coverage_percent=null`, and the missing command. If it runs, extract the actual coverage percentage from its log/report and retain `target/coverage/host/tarpaulin-report.html`; do not hard-code 100 percent unless the generated report proves it. A nonzero installed-tool run is `failed`.

- [ ] **Step 3: Capture Coverity honestly**

```bash
quality="target/posix/aarch64/fork-record-lock-quality"
missing=()
for tool in cov-build cov-analyze cov-format-errors; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -eq 0 ]; then
  covdir="$quality/coverity-capture"
  cov-build --dir "$covdir" make aarch64-warning-check \
    >"$quality/coverity.log" 2>&1
  cov-analyze --dir "$covdir" --all >>"$quality/coverity.log" 2>&1
  cov-format-errors --dir "$covdir" --json-output-v7 \
    "$quality/coverity-results.json" >>"$quality/coverity.log" 2>&1
else
  printf 'missing Coverity commands: %s\n' "${missing[*]}" \
    >"$quality/coverity.log"
fi
```

If all tools exist, parse the JSON `issues` array and record the exact finding count. If any command is absent, record `status="unavailable"`, `findings=null`, and every missing command. A failed capture is `failed`, never `passed` or `unavailable`.

Use this evidence schema:

```json
{
  "schema": 1,
  "architecture": "aarch64",
  "smros_commit": "read with git rev-parse HEAD",
  "checks": [
    {"name":"host-rust-coverage","kind":"coverage","status":"passed|failed|unavailable","coverage_percent":null,"findings":null,"artifact":null,"command":"make coverage-host","version":null,"summary":"..."},
    {"name":"coverity","kind":"static-analysis","status":"passed|failed|unavailable","coverage_percent":null,"findings":null,"artifact":null,"command":null,"version":null,"summary":"..."}
  ]
}
```

- [ ] **Step 4: Write the final results document**

Record:

- branch and full commit;
- pinned suite revision, patch SHA-256, manifest SHA-256, corrected binary SHA-256, and LTP provenance;
- native corrected-test result;
- every offline command and actual count/status;
- all three focused attempt durations and resource deltas;
- complete `fork` API counts and every non-pass;
- adjacent canary IDs and outcomes;
- AArch64 warning result;
- Verus result;
- Tarpaulin percentage or explicit unavailability;
- Coverity finding count or explicit unavailability;
- the remaining limitation that close-on-exec lock cleanup awaits a real `execve` transition.

Do not claim that one filtered API run establishes whole-suite POSIX compliance.

- [ ] **Step 5: Commit the evidence document and perform final review**

```bash
git add docs/posix/2026-08-12-aarch64-fork-record-lock-results.md
git commit -m "docs: record AArch64 fork record-lock results"
git status --short --branch
git log --oneline --decorate -12
```

Expected: only intentionally ignored/generated runtime artifacts remain outside Git. Compare implementation and evidence against every completion criterion in `docs/superpowers/specs/2026-08-12-posix-fork-record-lock-conformance-design.md`. Any missing gate, timeout, synthetic result, unexplained positive resource delta, or stale diagnostic asset keeps the work incomplete.
