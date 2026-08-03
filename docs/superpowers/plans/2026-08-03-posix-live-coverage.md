# POSIX Guest Live Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show truthful selected-test, API, and group completion/pass percentages while `posixtest` runs on AArch64.

**Architecture:** A bounded tracker in the shared POSIX logic owns result counts and API/group state. The guest runner updates it after each `test_end` and emits human-readable selection/progress lines without changing structured event schema 1; the shell status command reads the same snapshot. Host contract and Python parser tests protect ordering and compatibility.

**Tech Stack:** Rust `no_std` plus `alloc::collections::BTreeMap`, SMROS serial output, host Rust tests, Python `unittest`, Open POSIX Test Suite staging, QEMU AArch64.

---

## File Structure

- Modify `src/user_level/services/posix_test_logic_shared.rs`: bounded coverage tracker, status counts, percentages, and emission-trigger decisions.
- Modify `tests/host/src/lib.rs`: host allocation shim and behavioral unit tests for the shared tracker.
- Modify `src/user_level/services/posix_test.rs`: runner ownership, tracker updates, serial formatting, invariant handling, and status snapshots.
- Modify `src/user_level/services/user_shell.rs`: detailed `posixtest status` coverage formatting.
- Modify `tests/host/tests/integration_contracts.rs`: source-level guest output and schema-stability contracts.
- Modify `scripts/posix/tests/test_events.py`: prove non-event progress text is ignored by the strict event parser.
- Modify `docs/USER_SHELL.md`: document live selected-scope fields.
- Modify `docs/POSIX_CONFORMANCE.md`: preserve the boundary between live selection coverage and full host compliance evidence.

### Task 1: Build The Pure Bounded Coverage Tracker

**Files:**
- Modify: `tests/host/src/lib.rs:4-14`
- Modify: `tests/host/src/lib.rs:2014-2196`
- Modify: `src/user_level/services/posix_test_logic_shared.rs:1-111`

- [ ] **Step 1: Write failing shared-logic tests**

Export `BTreeMap` from the host allocation shim:

```rust
pub mod collections {
    pub use std::collections::{BTreeMap, BTreeSet};
}
```

Add these tests inside `mod posix_test_logic_shared`:

```rust
#[test]
fn coverage_tracks_noncontiguous_apis_and_groups() {
    let mut tracker = PosixCoverageTracker::default();
    tracker.select("read", "base").unwrap();
    tracker.select("write", "base").unwrap();
    tracker.select("read", "io").unwrap();

    assert_eq!(
        tracker.snapshot(),
        PosixCoverageSnapshot {
            tests_completed: 0,
            tests_selected: 3,
            apis_complete: 0,
            apis_pass: 0,
            apis_selected: 2,
            groups_complete: 0,
            groups_pass: 0,
            groups_selected: 2,
            status_counts: PosixCoverageStatusCounts::default(),
        }
    );

    let first = tracker
        .record("read", "base", PosixCoverageResult::Pass)
        .unwrap();
    assert!(!first.api_completed);
    tracker
        .record("write", "base", PosixCoverageResult::Fail)
        .unwrap();
    let last = tracker
        .record("read", "io", PosixCoverageResult::Pass)
        .unwrap();
    assert!(last.api_completed);
    assert_eq!(last.snapshot.tests_completed, 3);
    assert_eq!(last.snapshot.apis_complete, 2);
    assert_eq!(last.snapshot.apis_pass, 1);
    assert_eq!(last.snapshot.groups_complete, 2);
    assert_eq!(last.snapshot.groups_pass, 1);
    assert_eq!(last.snapshot.status_counts.passed, 2);
    assert_eq!(last.snapshot.status_counts.failed, 1);
}

#[test]
fn every_nonpass_result_completes_but_does_not_pass_a_unit() {
    let cases = [
        PosixCoverageResult::Fail,
        PosixCoverageResult::Unresolved,
        PosixCoverageResult::Unsupported,
        PosixCoverageResult::Untested,
        PosixCoverageResult::LaunchError,
    ];
    for result in cases {
        let mut tracker = PosixCoverageTracker::default();
        tracker.select("api", "group").unwrap();
        let update = tracker.record("api", "group", result).unwrap();
        assert_eq!(update.snapshot.tests_completed, 1);
        assert_eq!(update.snapshot.apis_complete, 1);
        assert_eq!(update.snapshot.apis_pass, 0);
        assert_eq!(update.snapshot.groups_complete, 1);
        assert_eq!(update.snapshot.groups_pass, 0);
    }
}

#[test]
fn coverage_percentages_and_progress_triggers_are_exact() {
    assert_eq!(coverage_percent_hundredths(0, 0), 0);
    assert_eq!(coverage_percent_hundredths(25, 1598), 156);
    assert_eq!(coverage_percent_hundredths(3, 195), 153);
    assert_eq!(coverage_percent_hundredths(2, 195), 102);
    assert_eq!(coverage_percent_hundredths(1598, 1598), 10_000);

    assert!(!should_emit_progress(24, 1598, false));
    assert!(should_emit_progress(25, 1598, false));
    assert!(should_emit_progress(26, 1598, true));
    assert!(should_emit_progress(1598, 1598, true));
    assert!(should_emit_progress(1, 1, true));
}

#[test]
fn coverage_rejects_unknown_and_excess_completion_at_the_manifest_bound() {
    let mut tracker = PosixCoverageTracker::default();
    for _ in 0..4096 {
        tracker.select("api", "group").unwrap();
    }
    assert_eq!(tracker.snapshot().tests_selected, 4096);
    assert_eq!(
        tracker.record("missing", "group", PosixCoverageResult::Pass),
        Err(PosixCoverageError::UnknownUnit)
    );
    for _ in 0..4096 {
        tracker
            .record("api", "group", PosixCoverageResult::Pass)
            .unwrap();
    }
    assert_eq!(
        tracker.record("api", "group", PosixCoverageResult::Pass),
        Err(PosixCoverageError::TestOverComplete)
    );
}
```

- [ ] **Step 2: Run the focused tests and verify red**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib posix_test_logic_shared::coverage -- --nocapture
```

Expected: compilation fails because the coverage types and functions are not defined.

- [ ] **Step 3: Implement the tracker and pure decisions**

Add `BTreeMap`/`String` imports and the following public model to
`posix_test_logic_shared.rs`:

```rust
use crate::alloc::collections::BTreeMap;
use crate::alloc::string::String;

pub const POSIX_PROGRESS_INTERVAL: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixCoverageResult {
    Pass,
    Fail,
    Unresolved,
    Unsupported,
    Untested,
    LaunchError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixCoverageError {
    CounterOverflow,
    UnknownUnit,
    TestOverComplete,
    UnitOverComplete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PosixCoverageStatusCounts {
    pub passed: usize,
    pub failed: usize,
    pub unresolved: usize,
    pub unsupported: usize,
    pub untested: usize,
    pub launch_errors: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PosixCoverageSnapshot {
    pub tests_completed: usize,
    pub tests_selected: usize,
    pub apis_complete: usize,
    pub apis_pass: usize,
    pub apis_selected: usize,
    pub groups_complete: usize,
    pub groups_pass: usize,
    pub groups_selected: usize,
    pub status_counts: PosixCoverageStatusCounts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PosixCoverageUpdate {
    pub snapshot: PosixCoverageSnapshot,
    pub api_completed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PosixCoverageUnit {
    selected: usize,
    completed: usize,
    all_pass: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosixCoverageTracker {
    tests_selected: usize,
    tests_completed: usize,
    apis: BTreeMap<String, PosixCoverageUnit>,
    groups: BTreeMap<String, PosixCoverageUnit>,
    status_counts: PosixCoverageStatusCounts,
}
```

Implement the tracker with validation before mutation, so an error cannot leave
only one of the API/group counters advanced:

```rust
impl PosixCoverageStatusCounts {
    fn with_result(
        self,
        result: PosixCoverageResult,
    ) -> Result<Self, PosixCoverageError> {
        let mut next = self;
        let counter = match result {
            PosixCoverageResult::Pass => &mut next.passed,
            PosixCoverageResult::Fail => &mut next.failed,
            PosixCoverageResult::Unresolved => &mut next.unresolved,
            PosixCoverageResult::Unsupported => &mut next.unsupported,
            PosixCoverageResult::Untested => &mut next.untested,
            PosixCoverageResult::LaunchError => &mut next.launch_errors,
        };
        *counter = counter
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        Ok(next)
    }
}

impl PosixCoverageTracker {
    pub fn select(&mut self, api: &str, group: &str) -> Result<(), PosixCoverageError> {
        let tests_selected = self
            .tests_selected
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        let api_selected = Self::next_selected(&self.apis, api)?;
        let group_selected = Self::next_selected(&self.groups, group)?;

        self.tests_selected = tests_selected;
        Self::set_selected(&mut self.apis, api, api_selected);
        Self::set_selected(&mut self.groups, group, group_selected);
        Ok(())
    }

    pub fn record(
        &mut self,
        api: &str,
        group: &str,
        result: PosixCoverageResult,
    ) -> Result<PosixCoverageUpdate, PosixCoverageError> {
        if self.tests_completed >= self.tests_selected {
            return Err(PosixCoverageError::TestOverComplete);
        }
        let tests_completed = self
            .tests_completed
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        let (api_completed, api_now_complete) = Self::next_completed(&self.apis, api)?;
        let (group_completed, _) = Self::next_completed(&self.groups, group)?;
        let status_counts = self.status_counts.with_result(result)?;

        self.tests_completed = tests_completed;
        self.status_counts = status_counts;
        Self::set_completed(&mut self.apis, api, api_completed, result);
        Self::set_completed(&mut self.groups, group, group_completed, result);
        Ok(PosixCoverageUpdate {
            snapshot: self.snapshot(),
            api_completed: api_now_complete,
        })
    }

    pub fn snapshot(&self) -> PosixCoverageSnapshot {
        let (apis_complete, apis_pass, apis_selected) = unit_summary(&self.apis);
        let (groups_complete, groups_pass, groups_selected) = unit_summary(&self.groups);
        PosixCoverageSnapshot {
            tests_completed: self.tests_completed,
            tests_selected: self.tests_selected,
            apis_complete,
            apis_pass,
            apis_selected,
            groups_complete,
            groups_pass,
            groups_selected,
            status_counts: self.status_counts,
        }
    }

    fn next_selected(
        units: &BTreeMap<String, PosixCoverageUnit>,
        name: &str,
    ) -> Result<usize, PosixCoverageError> {
        units
            .get(name)
            .map_or(Some(1), |unit| unit.selected.checked_add(1))
            .ok_or(PosixCoverageError::CounterOverflow)
    }

    fn set_selected(
        units: &mut BTreeMap<String, PosixCoverageUnit>,
        name: &str,
        selected: usize,
    ) {
        match units.get_mut(name) {
            Some(unit) => unit.selected = selected,
            None => {
                units.insert(
                    String::from(name),
                    PosixCoverageUnit {
                        selected,
                        completed: 0,
                        all_pass: true,
                    },
                );
            }
        }
    }

    fn next_completed(
        units: &BTreeMap<String, PosixCoverageUnit>,
        name: &str,
    ) -> Result<(usize, bool), PosixCoverageError> {
        let unit = units.get(name).ok_or(PosixCoverageError::UnknownUnit)?;
        let completed = unit
            .completed
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        if completed > unit.selected {
            return Err(PosixCoverageError::UnitOverComplete);
        }
        Ok((completed, completed == unit.selected))
    }

    fn set_completed(
        units: &mut BTreeMap<String, PosixCoverageUnit>,
        name: &str,
        completed: usize,
        result: PosixCoverageResult,
    ) {
        let unit = units
            .get_mut(name)
            .expect("coverage unit was validated before mutation");
        unit.completed = completed;
        if result != PosixCoverageResult::Pass {
            unit.all_pass = false;
        }
    }
}

fn unit_summary(units: &BTreeMap<String, PosixCoverageUnit>) -> (usize, usize, usize) {
    let mut complete = 0usize;
    let mut pass = 0usize;
    for unit in units.values() {
        if unit.completed == unit.selected {
            complete += 1;
            if unit.all_pass {
                pass += 1;
            }
        }
    }
    (complete, pass, units.len())
}

pub fn coverage_percent_hundredths(numerator: usize, denominator: usize) -> usize {
    if denominator == 0 {
        0
    } else {
        numerator.saturating_mul(10_000) / denominator
    }
}

pub fn should_emit_progress(
    completed: usize,
    selected: usize,
    api_completed: bool,
) -> bool {
    completed > 0
        && (completed % POSIX_PROGRESS_INTERVAL == 0
            || api_completed
            || completed == selected)
}
```

- [ ] **Step 4: Run focused and complete host unit tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib posix_test_logic_shared::coverage -- --nocapture
./scripts/run-host-unit-tests.sh --lib
```

Expected: all new coverage tests pass, followed by the complete host library test suite.

- [ ] **Step 5: Commit the pure tracker**

```bash
git add src/user_level/services/posix_test_logic_shared.rs tests/host/src/lib.rs
git commit -m "feat: add bounded POSIX coverage tracker"
```

### Task 2: Integrate Coverage With The Guest Runner

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs:1070-1280`
- Modify: `src/user_level/services/posix_test.rs:190-1035`

- [ ] **Step 1: Write failing guest integration contracts**

Extend `posix_guest_runner_is_serialized_bounded_and_fail_closed` with:

```rust
for contract in [
    "coverage: PosixCoverageTracker",
    "emit_selection_summary(state);",
    "fn emit_progress(",
    "posixtest: selection tests=",
    " apis=",
    " groups=",
    " interval=",
    " scope=selected",
    "posixtest: progress tests=",
    " apis-complete=",
    " apis-pass=",
    " groups-complete=",
    " groups-pass=",
    " launch-errors=",
    "should_emit_progress(",
] {
    assert!(runner.contains(contract), "missing live coverage contract {contract}");
}
assert_eq!(runner.matches("pub const POSIX_EVENT_SCHEMA: u32 = 1;").count(), 1);
```

Also assert ordering inside both `record_run_outcome` and
`record_unlaunched_test`: `emit_*test_end` appears before `.record(`, and
`emit_progress` appears after `.record(`. Assert `finish_suite` verifies
`tests_completed == selected.len()` before `emit_suite_end`.

- [ ] **Step 2: Run the focused integration contract and verify red**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_guest_runner_is_serialized_bounded_and_fail_closed -- --exact --nocapture
```

Expected: FAIL on the missing live coverage contracts.

- [ ] **Step 3: Replace duplicate runner counts with the tracker snapshot**

Import/re-export the shared types:

```rust
use super::posix_test_logic_shared::{
    PosixCoverageResult, PosixCoverageSnapshot, PosixCoverageTracker,
};
pub type PosixStatusCounts = posix_test_logic_shared::PosixCoverageStatusCounts;
```

Add `pub coverage: PosixCoverageSnapshot` to `PosixRunnerStatus`. Replace
`RunnerState.completed` and `RunnerState.status_counts` with:

```rust
coverage: PosixCoverageTracker,
```

During selection, register every selected test before publishing runner state:

```rust
let mut coverage = PosixCoverageTracker::default();
for test in &selected {
    if coverage.select(test.api.as_str(), test.group.as_str()).is_err() {
        emit_unbound_infrastructure_error("coverage-selection-invariant");
        return Err(PosixTestError::InfrastructureError);
    }
}
```

Build `status_snapshot` from `state.coverage.snapshot()`, preserving the
existing `completed`, `selected`, and `status_counts` public fields. The test
configuration and idle path use `PosixCoverageSnapshot::default()` and never
retain a previous run.

- [ ] **Step 4: Record every terminal outcome and fail closed**

Map `PosixRuntimeStatus` to the shared result enum:

```rust
fn coverage_result(status: PosixRuntimeStatus) -> PosixCoverageResult {
    match status {
        PosixRuntimeStatus::Pass => PosixCoverageResult::Pass,
        PosixRuntimeStatus::Fail => PosixCoverageResult::Fail,
        PosixRuntimeStatus::Unresolved => PosixCoverageResult::Unresolved,
        PosixRuntimeStatus::Unsupported => PosixCoverageResult::Unsupported,
        PosixRuntimeStatus::Untested => PosixCoverageResult::Untested,
        PosixRuntimeStatus::LaunchError => PosixCoverageResult::LaunchError,
    }
}
```

In each terminal path, emit `test_end` first, then call:

```rust
let update = match state.coverage.record(
    test.api.as_str(),
    test.group.as_str(),
    coverage_result(status),
) {
    Ok(update) => update,
    Err(_) => return false,
};
state.current_index = None;
if posix_test_logic_shared::should_emit_progress(
    update.snapshot.tests_completed,
    update.snapshot.tests_selected,
    update.api_completed,
) {
    emit_progress(update.snapshot);
}
```

Use `PosixCoverageResult::Untested` in the reviewed not-launched path. In
`finish_suite`, compare snapshot completion to `selected.len()`; emit
`infrastructure_error` instead of `suite_end` on mismatch.

- [ ] **Step 5: Emit bounded selection and progress lines**

Call `emit_selection_summary(state)` immediately after `emit_suite_start`.
Add integer-only serial helpers:

```rust
#[cfg(not(test))]
fn write_coverage_ratio(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    numerator: usize,
    denominator: usize,
) {
    write_u64(serial, numerator as u64);
    serial.write_byte(b'/');
    write_u64(serial, denominator as u64);
    serial.write_str(" (");
    let percent = posix_test_logic_shared::coverage_percent_hundredths(
        numerator,
        denominator,
    );
    write_u64(serial, (percent / 100) as u64);
    serial.write_byte(b'.');
    serial.write_byte(b'0' + ((percent / 10) % 10) as u8);
    serial.write_byte(b'0' + (percent % 10) as u8);
    serial.write_str("%)");
}
```

`emit_selection_summary` prints exactly:

```text
posixtest: selection tests=<n> apis=<n> groups=<n> interval=25 scope=selected
```

`emit_progress` prints exactly these fields in this order:

```text
posixtest: progress tests=<done>/<selected> (<pct>%) apis-complete=<done>/<selected> (<pct>%) apis-pass=<pass>/<selected> (<pct>%) groups-complete=<done>/<selected> (<pct>%) groups-pass=<pass>/<selected> (<pct>%) pass=<n> fail=<n> unresolved=<n> unsupported=<n> untested=<n> launch-errors=<n> scope=selected
```

Use one initialized `Serial` per line, `write_coverage_ratio` for every ratio,
and the tracker snapshot's status counts. Plain lines never call `begin_event`
and never increment event `seq`.

- [ ] **Step 6: Run contracts, unit tests, and formatting**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_guest_runner_is_serialized_bounded_and_fail_closed -- --exact --nocapture
./scripts/run-host-unit-tests.sh --lib
cargo fmt --manifest-path tests/host/Cargo.toml -- --check
```

Expected: the focused contract and all host unit tests pass; formatting reports no changes.

- [ ] **Step 7: Commit guest integration**

```bash
git add src/user_level/services/posix_test.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: emit POSIX live coverage progress"
```

### Task 3: Extend Shell Status And Protect Parser Compatibility

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs:1282-1355`
- Modify: `src/user_level/services/user_shell.rs:8332-8425`
- Modify: `scripts/posix/tests/test_events.py:50-140`

- [ ] **Step 1: Write failing shell and parser tests**

Extend `posix_test_shell_command_is_strictly_wired_to_the_runner` to require
these handler fragments:

```rust
for field in [
    " tests=",
    " apis-complete=",
    " apis-pass=",
    " groups-complete=",
    " groups-pass=",
    " scope=selected",
] {
    assert!(handler.contains(field), "status omits {field}");
}
assert!(handler.contains("status.coverage"));
```

In `scripts/posix/tests/test_events.py`, add a valid schema-1 stream with
ordinary selection/progress lines between `suite_start`, `test_start`,
`test_end`, and `suite_end`. Parse it with `parse_serial_log` and assert the
event names remain exactly:

```python
["suite_start", "test_start", "test_end", "suite_end"]
```

- [ ] **Step 2: Run both focused tests and verify red**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_test_shell_command_is_strictly_wired_to_the_runner -- --exact --nocapture
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_events -v
```

Expected: the Rust contract fails on missing status fields; the Python test passes only after its new fixture is syntactically valid, proving no parser production change is needed.

- [ ] **Step 3: Format the status snapshot with percentages**

Add a local integer ratio formatter beside `cmd_posix_test` using
`print_usize` and `coverage_percent_hundredths`. Change the active status body
to include the same ordered ratios as progress output:

```text
tests=<done>/<selected> (<pct>%) apis-complete=<done>/<selected> (<pct>%) apis-pass=<pass>/<selected> (<pct>%) groups-complete=<done>/<selected> (<pct>%) groups-pass=<pass>/<selected> (<pct>%)
```

Retain run ID, filter, current test, all result totals, and append
`scope=selected`. The idle snapshot prints zero ratios and no stale run data.

- [ ] **Step 4: Run focused and full compatibility tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_test_shell_command_is_strictly_wired_to_the_runner -- --exact --nocapture
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_events -v
./scripts/run-host-unit-tests.sh --test integration_contracts
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/posix/tests -v
```

Expected: all Rust contracts and all POSIX Python tests pass; schema 1 parsing accepts the ordinary progress lines without changing allowed structured fields.

- [ ] **Step 5: Commit status and compatibility coverage**

```bash
git add src/user_level/services/user_shell.rs tests/host/tests/integration_contracts.rs scripts/posix/tests/test_events.py
git commit -m "feat: show POSIX coverage in runner status"
```

### Task 4: Document Truthful Coverage Semantics

**Files:**
- Modify: `docs/USER_SHELL.md:112-132`
- Modify: `docs/POSIX_CONFORMANCE.md:80-118`
- Modify: `tests/host/tests/integration_contracts.rs:720-760`

- [ ] **Step 1: Write a failing documentation contract**

Extend the POSIX documentation contract to require all of:

```rust
for phrase in [
    "selection coverage",
    "apis-complete",
    "apis-pass",
    "groups-complete",
    "groups-pass",
    "every 25 completed tests",
    "does not prove POSIX compliance",
] {
    assert!(docs.contains(phrase), "missing coverage documentation: {phrase}");
}
```

- [ ] **Step 2: Run the documentation contract and verify red**

Run the exact documentation contract:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_conformance_workflow_and_limitations_are_documented -- --exact --nocapture
```

Expected: FAIL on the new live-coverage phrases.

- [ ] **Step 3: Document console behavior and evidence boundaries**

In `USER_SHELL.md`, add the selection line, complete progress line, trigger
rules, field definitions, and `posixtest status` behavior. In
`POSIX_CONFORMANCE.md`, state that these ratios cover only the current manifest
selection; build failures, source inventory, optional-group completion,
provenance, and final compliance remain host report responsibilities. State
that a passing selected API is complete only when all of its selected tests
pass and that live selection coverage does not prove POSIX compliance.

- [ ] **Step 4: Run the contract and documentation checks**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts
git diff --check
```

Expected: all integration contracts pass and no whitespace errors are reported.

- [ ] **Step 5: Commit documentation**

```bash
git add docs/USER_SHELL.md docs/POSIX_CONFORMANCE.md tests/host/tests/integration_contracts.rs
git commit -m "docs: explain POSIX selection coverage"
```

### Task 5: Complete AArch64 Verification

**Files:**
- Verify only; no planned source changes.

- [ ] **Step 1: Run all offline quality gates**

Run:

```bash
make host-fmt-check script-check launcher-test linker-layout-test ut it posix-tool-test build-test
git diff --check
```

Expected: every target exits zero, `kernel8.img` is rebuilt, the AArch64 link-layout check passes, and the diff check is silent.

- [ ] **Step 2: Verify the staged POSIX payload identity**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
```

Expected: the existing staged manifest, binaries, runtime closure, checksums, and AArch64 ELF identity validate without rewriting the stage.

- [ ] **Step 3: Run the automated SMROS AArch64 canary**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros --api WIFEXITED --qemu-memory 1024M
```

Expected: QEMU boots, each selected canary produces schema-1 start/end events,
ordinary selection/final progress text does not break parsing, and the campaign
writes parseable results under `target/posix/aarch64/smros-run/`.

- [ ] **Step 4: Inspect a direct live run for periodic and API output**

Run:

```bash
make run POSIX_QEMU_MEMORY=1024M
```

At the `smros:/>` prompt enter:

```text
posixtest all
```

Expected: a selection line immediately follows `suite_start`; progress appears
at test 25, at each API completion, and at final completion; each progress line
follows its `test_end` and precedes the next `test_start`; the final line shows
`tests=1598/1598 (100.00%)` for the current staged selection and precedes
`suite_end`. The API/group pass ratios may truthfully be below 100 percent.

- [ ] **Step 5: Review repository state and final evidence**

Run:

```bash
git status --short --branch
git log -6 --oneline --decorate
```

Expected: only intentional commits are ahead of `origin/master`; generated
POSIX stage data remains ignored; no unrelated user files were modified.
