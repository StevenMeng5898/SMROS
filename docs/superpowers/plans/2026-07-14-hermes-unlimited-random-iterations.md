# Hermes Unlimited Random Iterations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Accept every positive, platform-representable Hermes random iteration count while ensuring `test-all` runs `ut`, `it`, and `st` once each rather than once per random iteration.

**Architecture:** Keep campaign option parsing in the shared pure Rust helper and remove only its policy ceiling. Preserve the existing `test-all` sequence, then protect that sequence with an integration source contract and update user-facing syntax everywhere it currently advertises `1..64`.

**Tech Stack:** Rust `no_std` shared logic, Rust host tests, source-level integration contracts, Markdown documentation.

---

### Task 1: Remove the Campaign Iteration Ceiling

**Files:**
- Modify: `tests/host/src/lib.rs:597`
- Modify: `src/user_level/services/hermes_shell_logic_shared.rs:1`

- [ ] **Step 1: Write the failing parser tests**

Replace the former 65-is-invalid assertion with positive values above the old ceiling and the largest platform value:

```rust
assert!(!campaign_iterations_valid(0));
assert!(campaign_iterations_valid(64));
assert!(campaign_iterations_valid(65));
assert!(campaign_iterations_valid(usize::MAX));
assert_eq!(
    parse_campaign_options(&["seed=9393", "iterations=65"]),
    Some(HermesCampaignOptions {
        seed: Some(9393),
        iterations: 65,
    })
);
```

Keep the existing zero and duplicate-key rejection checks. On platforms where `u64` exceeds `usize`, add an overflow string check guarded by `usize::BITS < u64::BITS`.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign_selection_is_reproducible_and_bounded -- --exact --nocapture`

Expected: FAIL because `campaign_iterations_valid(65)` still returns false.

- [ ] **Step 3: Implement positive-only validation**

Remove `HERMES_MAX_ITERATIONS` and change validation to:

```rust
pub fn campaign_iterations_valid(iterations: usize) -> bool {
    iterations > 0
}
```

Keep the parser's checked decimal conversion and `number <= usize::MAX as u64` guard unchanged.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign_selection_is_reproducible_and_bounded -- --exact --nocapture`

Expected: PASS.

### Task 2: Lock Test-All Sequencing and Update the Interface

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs:395`
- Modify: `src/user_level/services/user_shell.rs:5175`
- Modify: `README.md:331`
- Modify: `docs/USER_SHELL.md:313`
- Modify: `docs/TESTING.md:140`

- [ ] **Step 1: Write the failing interface and sequencing contract**

In `hermes_test_orchestration_is_documented_and_smoke_wired`, isolate the `run_hermes_test_all` source and assert that the random campaign call precedes the single host-job loop, the loop contains all three fixed enum values, and no help or docs text contains `iterations=<1..64>`:

```rust
let test_all = shell
    .split("fn run_hermes_test_all")
    .nth(1)
    .and_then(|text| text.split("fn run_hermes_random_campaign").next())
    .expect("test-all function source");
let random_pos = test_all.find("run_hermes_random_campaign").expect("random campaign");
let jobs_pos = test_all.find("for job in [").expect("single host job loop");
assert!(random_pos < jobs_pos);
assert_eq!(test_all.matches("for job in [").count(), 1);
for job in ["HermesHostTestJob::Ut", "HermesHostTestJob::It", "HermesHostTestJob::St"] {
    assert_eq!(test_all.matches(job).count(), 1);
}
assert!(!shell.contains("iterations=<1..64>"));
assert!(!docs.contains("iterations=<1..64>"));
```

- [ ] **Step 2: Run the integration contract and verify RED**

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts hermes_test_orchestration_is_documented_and_smoke_wired -- --exact --nocapture`

Expected: FAIL because shell help and user documentation still advertise `1..64`.

- [ ] **Step 3: Update syntax and documentation**

Change all Hermes random/test-all usage strings from `iterations=<1..64>` to `iterations=<positive-n>`. Document that the iteration count affects only the random campaign and that native, `ut`, `it`, and `st` stages each run once. Keep examples finite and small.

- [ ] **Step 4: Run focused and full host verification**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign -- --nocapture`

Expected: PASS.

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts hermes_test_orchestration_is_documented_and_smoke_wired -- --exact --nocapture`

Expected: PASS.

Run: `./scripts/run-host-unit-tests.sh`

Expected: all host unit and integration tests PASS.

- [ ] **Step 5: Check formatting and stale limits**

Run: `cargo fmt --all -- --check`

Expected: exit 0.

Run: `rg -n 'HERMES_MAX_ITERATIONS|iterations=<1\.\.64>|maximum 64 iterations' src tests README.md docs`

Expected: no matches outside historical superseded implementation plans.
