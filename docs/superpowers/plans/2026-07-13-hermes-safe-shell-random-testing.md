# Hermes Safe Shell and Random Testing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add strictly allowlisted Hermes guest command execution, deterministic random test campaigns, and fixed host-assisted `ut`/`it`/`st` jobs.

**Architecture:** Pure shared Rust logic classifies structured guest requests and selects deterministic campaign cases. The shell owns actual handler dispatch, Hermes owns bounded reports and persistence, and the existing host launcher gains a protocol that accepts only three enum-like test job names.

**Tech Stack:** `no_std` Rust kernel services, host-side Rust tests, Python 3 launcher, QEMU system tests, FxFS persistence.

---

### Task 1: Pure Hermes command policy and deterministic campaign selection

**Files:**
- Create: `src/user_level/services/hermes_shell_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `verification/services/src/lib.rs`

- [ ] **Step 1: Write failing host tests for the policy**

Add a `hermes_shell_logic` include module and tests asserting that read-only commands (`help`, `version`, `ps`, `meminfo`, `components`, `fxfs`, `drivers`, `ifconfig`, `pwd`, `ls`, `svc`, `uptime`, `sched`, `loglevel`, `echo`, `testsc`, `fuzzsc`) are accepted only with bounded valid arguments; `vm -s` and Docker read operations are accepted; and `rm`, `kill`, `reboot`, `exit`, `clear`, `vi`, `run`, `write`, `mkdir`, `mv`, `cp`, `mount`, `vm -k`, `docker rm`, and `docker stop` are denied.

```rust
assert_eq!(hermes_shell_logic::classify("vm", &["-s"]), HermesShellPolicy::Allowed);
assert_eq!(hermes_shell_logic::classify("vm", &["-k", "demo"]), HermesShellPolicy::Forbidden);
assert_eq!(hermes_shell_logic::classify("docker", &["images"]), HermesShellPolicy::Allowed);
assert_eq!(hermes_shell_logic::classify("docker", &["rm", "smros0001"]), HermesShellPolicy::Forbidden);
assert_eq!(hermes_shell_logic::classify("reboot", &[]), HermesShellPolicy::Forbidden);
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `./scripts/run-host-unit-tests.sh hermes_shell_policy -- --nocapture`

Expected: FAIL because `hermes_shell_logic_shared.rs` and its policy API do not exist.

- [ ] **Step 3: Implement the minimal shared policy and PRNG**

Define `HermesShellPolicy::{Allowed, Forbidden, Invalid}`, `smros_hermes_shell_policy_body!`, bounded numeric parsing, and a deterministic xorshift64 selector. Keep command/subcommand matching explicit; default every unknown request to `Forbidden`. Define a fixed random catalog containing bounded `testsc`, `hermes test`, `fuzzsc seed=<seed> iterations=<1..4>`, status, scheduler, VM-status, and Docker read-only cases.

```rust
pub const HERMES_MAX_ARGS: usize = 8;
pub const HERMES_MAX_ARG_LEN: usize = 96;
pub const HERMES_MAX_ITERATIONS: usize = 64;

pub fn next_random(state: &mut u64) -> u64 {
    let mut value = if *state == 0 { 0x9e37_79b9_7f4a_7c15 } else { *state };
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *state = value;
    value
}
```

- [ ] **Step 4: Add Verus wrapper assertions for permanent denials**

Wire the same macro into `verification/services/src/lib.rs` and prove representative safe and forbidden classifications, including nested VM and Docker cases.

- [ ] **Step 5: Run tests and verification**

Run: `./scripts/run-host-unit-tests.sh hermes_shell`

Run: `make verus-services`

Expected: all focused Rust tests and service proofs pass.

- [ ] **Step 6: Commit**

```bash
git add src/user_level/services/hermes_shell_logic_shared.rs tests/host/src/lib.rs verification/services/src/lib.rs
git commit -m "feat: add Hermes safe shell policy"
```

### Task 2: Shell-owned safe dispatch gateway

**Files:**
- Modify: `src/user_level/services/user_shell.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write a failing wiring contract**

Assert that `user_shell.rs` exposes a structured `execute_hermes_command(ctx, command, args)` gateway, calls the shared policy before searching `SHELL_COMMANDS`, and never routes Hermes input through the interactive input buffer.

```rust
assert!(shell.contains("fn execute_hermes_command("));
assert!(shell.contains("hermes_shell_policy("));
assert!(shell.contains("HermesShellPolicy::Allowed"));
```

- [ ] **Step 2: Run the focused contract and verify RED**

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts hermes_safe_gateway -- --exact`

Expected: FAIL because the gateway is absent.

- [ ] **Step 3: Implement structured dispatch**

Add `HermesCommandStatus::{Completed, Denied, Invalid, Unknown}` and invoke the existing handler only after policy returns `Allowed`. Add a recursion guard that forbids `hermes exec hermes ...`; permit only the fixed `hermes test` catalog entry through an internal direct test call. Return structured status while normal command output continues to serial.

- [ ] **Step 4: Wire `hermes exec`**

Extend `cmd_hermes` parsing with `exec`, require at least one command token, enforce argument bounds before dispatch, and print an explicit denial for forbidden requests.

- [ ] **Step 5: Verify gateway tests and x86/ARM builds**

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts`

Run: `make build ARCH=x86_64-unknown-none`

Run: `make build ARCH=aarch64-unknown-none`

Expected: integration contracts and both kernel builds pass.

- [ ] **Step 6: Commit**

```bash
git add src/user_level/services/user_shell.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: add Hermes safe shell gateway"
```

### Task 3: Hermes random campaigns and persisted reports

**Files:**
- Modify: `src/user_level/services/hermes_agent.rs`
- Modify: `src/user_level/services/user_shell.rs`
- Modify: `tests/host/src/lib.rs`

- [ ] **Step 1: Write failing campaign accounting tests**

Test fixed-seed reproducibility, `iterations=0` rejection, maximum 64 iterations, completed/pass/fail/denied accounting, and bounded report formatting.

```rust
let first = hermes_shell_logic::campaign_indices(1234, 8);
let second = hermes_shell_logic::campaign_indices(1234, 8);
assert_eq!(first, second);
assert_eq!(first.len(), 8);
assert!(hermes_shell_logic::campaign_iterations_valid(64));
assert!(!hermes_shell_logic::campaign_iterations_valid(65));
```

- [ ] **Step 2: Run focused campaign tests and verify RED**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign -- --nocapture`

Expected: FAIL because campaign/report functions are absent.

- [ ] **Step 3: Implement campaign types and persistence**

Add `HermesCampaignReport` and `HermesCommandReport`, generate concrete bounded commands from the shared catalog, dispatch each through a callback supplied by `user_shell.rs`, and persist `/data/hermes/tests/latest.log` plus at most eight numbered historical reports. Record seed, requested/completed count, status totals, command display, and bounded failure reason; never record unbounded serial output.

- [ ] **Step 4: Wire `hermes random`**

Parse `seed=<u64>` and `iterations=<usize>` in any order, reject duplicates/unknown keys, default to a runtime-derived seed and eight iterations, run sequentially, print the replay command, and persist partial reports on failure.

- [ ] **Step 5: Run tests and kernel build**

Run: `./scripts/run-host-unit-tests.sh hermes_campaign`

Run: `make build ARCH=x86_64-unknown-none`

Expected: campaign tests and kernel build pass.

- [ ] **Step 6: Commit**

```bash
git add src/user_level/services/hermes_agent.rs src/user_level/services/user_shell.rs tests/host/src/lib.rs
git commit -m "feat: add Hermes random test campaigns"
```

### Task 4: Fixed host test-job protocol

**Files:**
- Modify: `scripts/smros-vm-launcher.py`
- Create: `scripts/test-smros-vm-launcher.py`
- Modify: `src/user_level/services/vm_host.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `Makefile`

- [ ] **Step 1: Write failing Python protocol tests**

Load the launcher module with `importlib`, inject a fake subprocess runner, and assert `SMROS_TEST_RUN 1\njob=ut\n\n` maps to the fixed `make ut` argv while `job=verify`, `command=make clean`, extra fields, and malformed jobs are denied.

```python
assert module.parse_test_job({"job": "ut"}) == ("make", "ut")
with self.assertRaises(ValueError):
    module.parse_test_job({"job": "ut", "command": "make clean"})
```

- [ ] **Step 2: Run Python tests and verify RED**

Run: `python3 scripts/test-smros-vm-launcher.py`

Expected: FAIL because the test protocol parser is absent.

- [ ] **Step 3: Implement constrained launcher jobs**

Accept header `SMROS_TEST_RUN 1`; require exactly one `job` field in `{ut,it,st}`; map internally to `("make", job)`; serialize execution with a lock; use a configurable bounded timeout defaulting to 300 seconds; write capped output to `target/hermes-tests/<job>.log`; and respond `OK job=<job> status=0 summary=<bounded>` or `ERROR ...`. Never invoke `shell=True`.

- [ ] **Step 4: Add Rust client and parser tests**

Define `HermesHostTestJob::{Ut, It, St}`, build the fixed request without arbitrary strings, reuse the VM launcher TCP transport, and parse structured success/error responses into `HermesHostTestResult`.

- [ ] **Step 5: Wire launcher tests into repository checks**

Add a `launcher-test` Make target and include it in `test`/`verify` without changing the existing `ut`, `it`, or `st` definitions.

- [ ] **Step 6: Verify protocol suites**

Run: `python3 scripts/test-smros-vm-launcher.py`

Run: `./scripts/run-host-unit-tests.sh vm_host`

Run: `make script-check`

Expected: Python, Rust parser, and script checks pass.

- [ ] **Step 7: Commit**

```bash
git add scripts/smros-vm-launcher.py scripts/test-smros-vm-launcher.py src/user_level/services/vm_host.rs tests/host/src/lib.rs Makefile
git commit -m "feat: add fixed Hermes host test jobs"
```

### Task 5: `test-all`, system coverage, and documentation

**Files:**
- Modify: `src/user_level/services/hermes_agent.rs`
- Modify: `src/user_level/services/user_shell.rs`
- Modify: `scripts/smoke-qemu.sh`
- Modify: `README.md`
- Modify: `docs/USER_SHELL.md`
- Modify: `docs/TESTING.md`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing command/documentation contracts**

Require help and docs for `hermes exec`, `hermes random`, and `hermes test-all`; require the smoke script to run a fixed-seed one-iteration campaign and probe `hermes exec reboot` for a denial marker.

- [ ] **Step 2: Run integration contracts and verify RED**

Run: `./scripts/run-host-unit-tests.sh --test integration_contracts`

Expected: FAIL because `test-all`, smoke probes, and docs are absent.

- [ ] **Step 3: Implement `hermes test-all` orchestration**

Run native `hermes test`, then a guest campaign, then sequential fixed host jobs `ut`, `it`, and `st`. Persist a combined report. Continue after individual failures so all requested stages report status, but return an overall failure unless every stage passes.

- [ ] **Step 4: Extend smoke coverage**

Feed `hermes random seed=1 iterations=1` and `hermes exec reboot` into the smoke serial input. Require a campaign completion marker and `Hermes denied forbidden command: reboot`; do not issue a real reboot.

- [ ] **Step 5: Document commands and safety boundary**

Add examples, replay behavior, report paths, fixed host-job mapping, launcher prerequisite, permanent forbidden list, iteration/timeout limits, and the rule that Gemma text is never executed directly.

- [ ] **Step 6: Run full fresh verification**

Run: `python3 scripts/test-smros-vm-launcher.py`

Run: `make ut`

Run: `make it`

Run: `make build ARCH=x86_64-unknown-none`

Run: `make build ARCH=aarch64-unknown-none`

Run: `make st ARCH=x86_64-unknown-none`

Run: `git diff --check`

Expected: all tests, both builds, x86 system smoke, and whitespace checks pass. If host networking or QEMU prerequisites prevent `st`, report the exact external failure rather than claiming it passed.

- [ ] **Step 7: Commit**

```bash
git add src/user_level/services/hermes_agent.rs src/user_level/services/user_shell.rs scripts/smoke-qemu.sh README.md docs/USER_SHELL.md docs/TESTING.md tests/host/tests/integration_contracts.rs
git commit -m "feat: complete Hermes safe test orchestration"
```
