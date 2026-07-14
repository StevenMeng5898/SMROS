# Hermes Iterated Host Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every `hermes test-all` iteration execute one deterministic random guest operation followed by `ut`, `it`, and `st`, while keeping 1000-iteration reports persistable.

**Architecture:** Add pure report-detail limit helpers to the shared Hermes campaign logic. Extract one shell helper that selects, records, and executes a single deterministic random round; both standalone random campaigns and `test-all` use it, while `test-all` owns the outer loop containing the three fixed host jobs and aggregates bounded totals.

**Tech Stack:** Rust `no_std` guest code, shared pure Rust campaign logic, Rust host unit tests, source-level integration contracts, Markdown documentation.

---

### Task 1: Bound Campaign Report Details

**Files:**
- Modify: `tests/host/src/lib.rs:597`
- Modify: `src/user_level/services/hermes_shell_logic_shared.rs:1`

- [ ] **Step 1: Write failing report-limit tests**

Extend the Hermes campaign unit test with:

```rust
assert!(campaign_report_includes_round(0));
assert!(campaign_report_includes_round(63));
assert!(!campaign_report_includes_round(64));
assert_eq!(campaign_report_omitted_rounds(64), 0);
assert_eq!(campaign_report_omitted_rounds(1000), 936);
```

Import `campaign_report_includes_round` and `campaign_report_omitted_rounds` from the included shared module.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign_selection_is_reproducible_and_bounded -- --exact --nocapture`

Expected: FAIL to compile because the two report-limit helpers do not exist.

- [ ] **Step 3: Implement the detail-limit helpers**

Add to `hermes_shell_logic_shared.rs`:

```rust
pub const HERMES_REPORT_DETAIL_LIMIT: usize = 64;

pub fn campaign_report_includes_round(round: usize) -> bool {
    round < HERMES_REPORT_DETAIL_LIMIT
}

pub fn campaign_report_omitted_rounds(iterations: usize) -> usize {
    iterations.saturating_sub(HERMES_REPORT_DETAIL_LIMIT)
}
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign_selection_is_reproducible_and_bounded -- --exact --nocapture`

Expected: PASS.

### Task 2: Put Host Jobs Inside Every Test-All Iteration

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs:395`
- Modify: `src/user_level/services/user_shell.rs:5190`

- [ ] **Step 1: Replace the old sequencing contract with the failing nested-loop contract**

Add this brace-matching test helper, then require the `run_hermes_test_all` source to have the shown structure:

```rust
fn braced_body(source: &str) -> &str {
    let open = source.find('{').expect("opening brace");
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("closing brace");
}

let round_loop = "for round in 0..options.iterations {";
let round_pos = test_all.find(round_loop).expect("test-all iteration loop");
let round_body = braced_body(&test_all[round_pos..]);
assert!(test_all[..round_pos].contains("run_hermes_agent_tests(ctx)"));
assert!(round_body.contains("execute_hermes_campaign_round"));
assert_eq!(round_body.matches("for (job_index, job) in jobs.iter().copied().enumerate()").count(), 1);
for job in ["HermesHostTestJob::Ut", "HermesHostTestJob::It", "HermesHostTestJob::St"] {
    assert_eq!(test_all.matches(job).count(), 1);
}
assert!(test_all.contains("campaign_report_omitted_rounds(options.iterations)"));
```

Remove the obsolete assertions that require the random campaign call to precede one host loop.

- [ ] **Step 2: Run the integration contract and verify RED**

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts hermes_test_orchestration_is_documented_and_smoke_wired -- --exact --nocapture`

Expected: FAIL because `run_hermes_test_all` has no outer iteration loop.

- [ ] **Step 3: Extract single-round random execution**

Add this single-round helper:

```rust
fn execute_hermes_campaign_round(
    ctx: &mut ShellContext,
    seed: u64,
    round: usize,
    report: &mut String,
) -> HermesCommandStatus {
    use crate::user_level::services::hermes_shell_logic_shared::{
        campaign_case, campaign_case_index, campaign_report_includes_round,
    };

    let index = campaign_case_index(seed, round);
    let Some(case) = campaign_case(index, seed, round) else {
        return HermesCommandStatus::Unknown;
    };
    if campaign_report_includes_round(round) {
        report.push_str("case=");
        append_usize_shell(report, round);
        report.push(' ');
        report.push_str(case.command);
        for arg in &case.args[..case.arg_count] {
            report.push(' ');
            report.push_str(arg);
        }
        report.push('\n');
    }
    execute_hermes_command(ctx, case.command, &case.args[..case.arg_count])
}
```

Replace the duplicated selection, case formatting, and dispatch block in `run_hermes_random_campaign` with this helper. After the loop, append:

```rust
let omitted = campaign_report_omitted_rounds(options.iterations);
if omitted > 0 {
    report.push_str("details_omitted=");
    append_usize_shell(&mut report, omitted);
    report.push('\n');
}
```

- [ ] **Step 4: Implement per-iteration test-all orchestration**

Keep `run_hermes_agent_tests(ctx)` before the loop. Define the fixed job array once, random status counters, and `[usize; 3]` host pass/failure counters. Use this shape:

```rust
for round in 0..options.iterations {
    let status = execute_hermes_campaign_round(ctx, seed, round, &mut report);
    count_hermes_command_status(
        status,
        &mut random_completed,
        &mut random_denied,
        &mut random_invalid,
        &mut random_unknown,
    );

    for (job_index, job) in jobs.iter().copied().enumerate() {
        match crate::user_level::services::vm_host::run_hermes_test(job) {
            Ok(result) if result.passed => host_passes[job_index] += 1,
            Ok(_) | Err(_) => host_failures[job_index] += 1,
        }
    }
}
```

Print each host result with its one-based iteration number, append only aggregate random and per-job totals to the persisted report, add `details_omitted` when needed, and define overall success as the native check passing, every random status being `Completed`, and every job pass count equaling `options.iterations`. Report persistence errors to serial instead of discarding them.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign -- --nocapture`

Expected: PASS.

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts hermes_test_orchestration_is_documented_and_smoke_wired -- --exact --nocapture`

Expected: PASS.

### Task 3: Correct Documentation and Verify the Complete Change

**Files:**
- Modify: `README.md:380`
- Modify: `docs/USER_SHELL.md:414`
- Modify: `docs/TESTING.md:140`
- Modify: `tests/host/tests/integration_contracts.rs:395`

- [ ] **Step 1: Add failing documentation assertions**

Require README and user-shell documentation to contain `each host job once per iteration` and require the shell source to contain `details_omitted=`.

- [ ] **Step 2: Run the documentation contract and verify RED**

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts hermes_test_orchestration_is_documented_and_smoke_wired -- --exact --nocapture`

Expected: FAIL because the current docs say each host job runs once overall.

- [ ] **Step 3: Update documentation**

State that `hermes test-all iterations=1000` runs the native check once and then performs 1000 rounds, each containing one random guest operation plus one `ut`, `it`, and `st` request. State that reports retain aggregate totals and at most 64 round details.

- [ ] **Step 4: Run complete verification**

Run: `./scripts/run-host-unit-tests.sh`

Expected: 44 host unit tests and 11 integration contracts PASS.

Run: `git diff --check`

Expected: exit 0.

Run: `rustfmt --edition 2021 --check tests/host/src/lib.rs tests/host/tests/integration_contracts.rs`

Expected: exit 0.

Run: `rg -n 'only to the random campaign|host jobs once each|each host job once;' README.md docs/USER_SHELL.md docs/TESTING.md`

Expected: no matches.
