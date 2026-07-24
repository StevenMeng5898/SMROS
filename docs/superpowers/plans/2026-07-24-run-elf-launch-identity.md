# Run ELF Launch Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every asynchronous ELF lifecycle operation target its originating launch and carry that identity safely through the EL0-to-EL1 resume path.

**Architecture:** `RunElfLifecycleState` allocates a nonzero monotonic typed launch ID and classifies every ID-aware mutation as matched, repeated, stale, or missing. A bounded atomic binding table pins that ID to the launcher's physical CPU before thread creation; matched `sys_exit` returns the raw ID in the existing syscall result register, which becomes the resume function's C ABI argument.

**Tech Stack:** Rust `no_std`, core atomics, AArch64 exception ABI, Cargo host tests, Make-based AArch64 build checks.

---

### Task 1: Add Launch-Identity Regression Tests

**Files:**
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write stale A/B lifecycle tests**

Add a production-shared test that admits A, attaches and completes A, releases A before its callback, admits and attaches B, then submits stale A completion, attachment, loader-failure completion, and prepare-return operations. Assert all stale results are explicit, no reset/callback occurs, B remains active, only the unattached stale resource is released, and matched B prepare/completion releases B before its callback.

```rust
let a = started_id(run_elf_start_transition(&mut state, Request::new(1), || resets += 1));
assert!(run_elf_attach_resource_transition(&mut state, a, allocation(&a_releases)).is_ok());
let completed_a = run_elf_take_completion_transition(&mut state, a, || resets += 1);
drop(requested_resource(completed_a));
callbacks += 1;

let b = started_id(run_elf_start_transition(&mut state, Request::new(2), || resets += 1));
assert_ne!(a, b);
assert!(run_elf_attach_resource_transition(&mut state, b, allocation(&b_releases)).is_ok());
assert!(matches!(
    run_elf_take_completion_transition(&mut state, a, || resets += 1).completion,
    RunElfCompletion::Stale
));
assert_eq!(state.active_id(), Some(b));
```

- [ ] **Step 2: Write exhaustion and CPU-binding tests**

Initialize state at `u64::MAX`, verify the last ID is issued once, complete it, and verify subsequent admission is `Exhausted` without reset or reuse. Test zero/raw conversion, CPU bounds, occupied binding rejection, stale compare-exchange clear, and successful matched clear/rebind.

```rust
let max = RunElfLaunchId::from_raw(u64::MAX).unwrap();
let mut state = RunElfLifecycleState::with_next_launch_id(max);
assert_eq!(started_id(run_elf_start_transition(&mut state, 1, || resets += 1)), max);
let _ = run_elf_take_completion_transition(&mut state, max, || resets += 1);
assert!(matches!(
    run_elf_start_transition(&mut state, 2, || resets += 1),
    RunElfStart::Exhausted(2)
));
```

- [ ] **Step 3: Write syscall-token integration contracts**

Require production to bind before `create_thread_on_cpu`, pass IDs to all async transitions, return the matched ID from `sys_exit`, accept `id_raw: usize` in `run_elf_launcher_resume`, reject zero/non-64-bit conversion, and preserve syscall result `x0` through the AArch64 exception restore before `eret`.

- [ ] **Step 4: Run RED**

Run: `make ut it`

Expected: compilation fails because `RunElfLaunchId`, `RunElfStart`, ID-aware transition signatures, and `RunElfCpuBindings` do not exist.

### Task 2: Implement the Tokenized Shared State

**Files:**
- Modify: `src/user_level/services/user_logic_shared.rs`

- [ ] **Step 1: Add typed monotonic identity allocation**

Add `RunElfLaunchId(u64)` with nonzero `from_raw`, 64-bit `from_usize`/`to_usize`, and `raw`. Add `RunElfStart<T> { Started(id), Busy(T), Exhausted(T) }`. Store `active_id`, `next_launch_id`, and `last_terminal_id` in `RunElfLifecycleState`; consume IDs only on successful non-busy admission and fail closed after `u64::MAX`.

- [ ] **Step 2: Make all transitions ID-aware**

Add `RunElfTransition::{Matched, Repeated, Stale, Missing}` and `RunElfCompletion<T>::Stale`. Require expected IDs for request cloning, prepare-return, clear, attachment, and completion. Invoke reset hooks only for `Started`, `Matched`, or `Requested` results. Add `RunElfAttachError<R> { Stale(R), Missing(R), Occupied(R) }` with `into_resource`.

```rust
pub(crate) fn run_elf_prepare_return_transition<T>(
    state: &mut RunElfLifecycleState<T>,
    expected: RunElfLaunchId,
    exit_code: i32,
    reset: impl FnOnce(),
) -> RunElfTransition;

pub(crate) fn run_elf_take_completion_transition<T>(
    state: &mut RunElfLifecycleState<T>,
    expected: RunElfLaunchId,
    reset: impl FnOnce(),
) -> RunElfTaken<T>;
```

- [ ] **Step 3: Add bounded atomic CPU bindings**

Implement `RunElfCpuBindings<const N: usize>` over `[AtomicU64; N]`. `bind` validates bounds and compare-exchanges EMPTY to the ID; `get` rejects out-of-range/zero; `clear` compare-exchanges only the expected ID to EMPTY.

- [ ] **Step 4: Run GREEN for host unit tests**

Run: `make ut`

Expected: 0 failures, including stale A/B, exhaustion, and binding tests.

### Task 3: Wire Production Identity and Resume ABI

**Files:**
- Modify: `src/user_level/services/run_elf.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Bind before pinned thread creation**

Have successful admission return `RunElfLaunchId`, validate the physical CPU, bind the ID before calling `create_thread_on_cpu(..., Some(cpu))`, and clear by compare-exchange plus matched state clear on binding/thread-creation failure.

- [ ] **Step 2: Carry the ID through every launcher path**

Launcher entry reads its pinned CPU binding, clones only `state.request_for(id)`, and supplies the ID to attachment, loader failure, clear, and completion. Rejected stale attachment drops only `error.into_resource()` and terminates the stale launcher without completing the active request. Every synchronous failure clears only `(cpu, expected_id)`.

- [ ] **Step 3: Carry the ID through normal EL0 return**

Change `prepare_run_elf_return` to return `Option<usize>`. It validates current CPU binding and active lifecycle ID, performs matched prepare-return, installs the resume address, and returns the nonzero raw ID. Change `sys_exit` to return that value as the syscall result. Change resume to `run_elf_launcher_resume(id_raw: usize)`, reconstruct the typed ID, and complete only that ID.

```rust
if let Some(launch_id) = crate::user_level::run_elf::prepare_run_elf_return(exit_code) {
    return Ok(launch_id);
}
```

- [ ] **Step 4: Run GREEN for unit and integration tests**

Run: `make ut it`

Expected: all host unit and integration tests pass.

### Task 4: Verify, Review, and Commit

**Files:**
- Modify only if review finds a Task 8 blocker.

- [ ] **Step 1: Run complete verification**

```bash
make ut it build-test
cargo fmt --manifest-path tests/host/Cargo.toml --check
rustfmt --edition 2021 --check \
  src/kernel_objects/scheduler.rs \
  src/kernel_objects/scheduler_logic_shared.rs \
  src/user_level/services/run_elf.rs \
  src/user_level/services/user_logic_shared.rs \
  src/user_level/services/posix_test.rs
bash -n scripts/run-host-unit-tests.sh
make script-check
git diff --check
```

Expected: exit 0 for every command; only known include-harness dead-code warnings may remain.

- [ ] **Step 2: Request independent review**

Review the complete diff from `f0f964b`, focusing on monotonic identity allocation, matched-only side effects, stale A/B resource safety, binding-before-eligibility, compare-exchange clearing, and the syscall-result resume ABI.

- [ ] **Step 3: Stage only intended files and commit**

```bash
git add \
  src/user_level/services/run_elf.rs \
  src/user_level/services/user_logic_shared.rs \
  src/syscall/syscall.rs \
  tests/host/src/lib.rs \
  tests/host/tests/integration_contracts.rs
git commit -m "fix: bind ELF lifecycle to launch identity"
```
