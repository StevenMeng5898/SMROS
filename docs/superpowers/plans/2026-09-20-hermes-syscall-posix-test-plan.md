# Hermes Syscall and POSIX Test Plan Implementation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire Hermes skills `ut`, `it`, `st`, and `fuzzing` so syscall ABI and POSIX API checks run through the existing UT / IT / ST / fuzzsc layers without weakening the Hermes allowlist or launching unsolicited full POSIX campaigns.

**Architecture:** Keep Hermes as the only autonomous control plane. Guest actions stay behind `hermes_shell_logic_shared::classify`. Host jobs stay a closed launcher enum. Official POSIX remains host-controlled `run-smros`. Python StagingTests remain the TDD gate while `tests/host` does not compile.

**Tech Stack:** SMROS kernel (AArch64, RISC-V64), Hermes agent, `src/syscall/fuzz.rs`, Open POSIX Test Suite harness in `scripts/posix`, Make targets `ut`/`it`/`st`/`posix-tool-test`/`posix-run`.

**Spec:** `docs/superpowers/specs/2026-09-20-hermes-syscall-posix-test-plan-design.md`

## Global Constraints

- Dual-arch POSIX: AArch64 and RISC-V64; no hardcoded architecture-specific paths or values.
- Official POSIX campaign is host-controlled `posixtest test {id}` with watchdog; guest `posixtest all` is never a Hermes command.
- Quality bar: pass ≥99%, timeout 0, unresolved 0, untested 0, fail < 2 on both arches.
- Do not start a full ~1600-test campaign unless focused canaries regress.
- TDD: RED then GREEN. Prefer `scripts/posix/tests/test_build.py` over the broken `tests/host` crate.
- Do not git commit unless the user asks. `posix-build` refuses a dirty tree.
- Workspace/disk under 10 GB; isolated `target/posix/<arch>/smros-fxfs.img`.
- Permanent Hermes forbidden list unchanged (`rm`, `kill`, `reboot`, ...).
- Gemma text is never executed.

---

### Task 1: Install Hermes skills `ut`, `it`, `st`, `fuzzing`

**Files:**
- Modify: `src/user_level/services/hermes_agent.rs`
- Modify: `tests/host/src/lib.rs` only if that crate is compiling; otherwise lock skill strings from `scripts/posix/tests/test_build.py`
- Modify: `docs/USER_SHELL.md`, `docs/TESTING.md`

**Interfaces:**
- Consumes: existing `HermesSkillDefinition` / `HERMES_SKILLS`
- Produces: four new slugs `ut`, `it`, `st`, `fuzzing` with keyword lists that match syscall/POSIX prompts

- [ ] **Step 1: Write the failing skill-presence test**

Add a StagingTest that reads `hermes_agent.rs` and requires slugs `ut`, `it`, `st`, `fuzzing`, paths under `/data/hermes/skills/<slug>/SKILL.md`, and bodies that mention the Make targets / `fuzzsc` from the spec.

- [ ] **Step 2: Run it and confirm RED**

Run: `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.posix.tests.test_build.StagingTests.test_hermes_syscall_skills_are_installed -v`

Expected: FAIL because those slugs are absent.

- [ ] **Step 3: Add the four skill definitions**

Keep existing `smros-kernel`, `hermes-web-ui`, `smros-ops`, `hermes-memory`. Append the four new skills. Bodies are short Markdown that point at the spec and the exact commands in the design (no free-form shell).

- [ ] **Step 4: Re-run the test and `make posix-tool-test`**

Expected: GREEN. `hermes test` skill-count assertion may need to accept `HERMES_SKILLS.len()` rather than a hardcoded `4`.

- [ ] **Step 5: Document the skills in `docs/USER_SHELL.md` and `docs/TESTING.md`**

---

### Task 2: Syscall-mode campaign catalog

**Files:**
- Modify: `src/user_level/services/hermes_shell_logic_shared.rs`
- Modify: `src/user_level/services/user_shell.rs` (`run_hermes_test_all` / random catalog)
- Test: `scripts/posix/tests/test_build.py` or existing host hermes campaign tests if compiling

**Interfaces:**
- Consumes: `HermesCampaignCase`, `campaign_case_const`, `classify`
- Produces: a syscall catalog that includes `testsc`, `fuzzsc seed=1 iterations=1`, and `hermes test`; still 12-or-so bounded cases

- [ ] **Step 1: RED — catalog must include `testsc` and `fuzzsc`**

Assert `HERMES_CAMPAIGN_CASES` (or a new `HERMES_SYSCALL_CAMPAIGN_CASES`) contains `testsc` with zero args and `fuzzsc` with `seed=1`, `iterations=1`, and that `classify` returns `Allowed` for every catalog entry.

- [ ] **Step 2: Confirm RED**

Current catalog has `fuzzsc` but not `testsc`.

- [ ] **Step 3: GREEN — add `testsc` and keep every entry allowlisted**

Do not add `posixtest all`. Optional later canary is Task 3.

- [ ] **Step 4: Wire `hermes test-all mode=syscall`**

Parse optional `mode=syscall` next to `seed` / `iterations`. Unknown modes are invalid. Default mode remains the existing ops catalog so current smoke does not change behavior except where Task 4 extends `make st` input.

---

### Task 3: Allowlist `posixtest test <canary>` only

**Files:**
- Modify: `src/user_level/services/hermes_shell_logic_shared.rs`
- Modify: `src/user_level/services/user_shell.rs` only if classify is the single gate (it should be)
- Test: StagingTests + existing `hermes_shell_policy_*` tests if host crate compiles

**Interfaces:**
- Consumes: `classify(command, args) -> HermesShellPolicy`
- Produces: `posixtest` + `["test", "getpid/1-1.c"]` Allowed; `posixtest all` Forbidden; `posixtest test ../x` Forbidden

- [ ] **Step 1: RED**

```python
# Allowed
classify("posixtest", ["test", "getpid/1-1.c"]) == Allowed
classify("posixtest", ["status"]) == Allowed
# Forbidden
classify("posixtest", ["all"]) == Forbidden
classify("posixtest", ["group", "conformance"]) == Forbidden
classify("posixtest", ["test", "mq_send/5-1.c"]) == Forbidden  # not in canary table yet
```

Canary table starts with `getpid/1-1.c` only. Additional ids are a later, explicit table edit.

- [ ] **Step 2: Confirm RED** (today every `posixtest` is Forbidden)

- [ ] **Step 3: GREEN — `posix_policy(args)` closed table**

Exact match only. No globs, no `all`, no `api`, no `group`.

- [ ] **Step 4: Do not add this canary to `make st` by default**

Guest POSIX still depends on an embedded stage. Fast ST stays `testsc`/`fuzzsc`. Document that Hermes may `exec posixtest test getpid/1-1.c` only on kernels built with a POSIX stage.

---

### Task 4: Fast ST probes `testsc` and `fuzzsc`

**Files:**
- Modify: `scripts/smoke-qemu.sh` (default `SMROS_ST_COMMANDS` / required patterns)
- Modify: `tests/host/tests/integration_contracts.rs` `hermes_test_orchestration_is_documented_and_smoke_wired` **only if host IT compiles**; otherwise StagingTest on the smoke script text
- Modify: `docs/TESTING.md`

**Interfaces:**
- Consumes: existing smoke serial runner
- Produces: extra commands and required markers from the spec

- [ ] **Step 1: RED — smoke script must send `testsc` and `fuzzsc seed=1 iterations=1`**

- [ ] **Step 2: Confirm RED** (today defaults are `hermes random` + `hermes exec reboot`)

- [ ] **Step 3: GREEN — extend `SMROS_ST_COMMANDS` and required patterns**

Keep reboot denial. Do not require POSIX suite markers. Keep overrides via `SMROS_ST_REQUIRED_PATTERNS`.

- [ ] **Step 4: Run `make st` on one arch after kernel build**

Only when implementing, not during design. Use the arch already being built; do not spawn a second full kernel tree. Expected: existing milestones plus testsc/fuzzsc markers.

---

### Task 5: Optional host job `posix-tool`

**Files:**
- Modify: `scripts/smros-vm-launcher.py`
- Modify: `scripts/test-smros-vm-launcher.py`
- Modify: `src/user_level/services/vm_host.rs`
- Modify: launcher protocol version in `scripts/start-smros-vm-launcher.sh`

**Interfaces:**
- Consumes: `parse_test_job`
- Produces: `job=posix-tool` → `("make", "posix-tool-test")`

- [ ] **Step 1: RED Python protocol test**

Unknown job still denied. `posix-tool` maps to `make posix-tool-test`. Extra fields still denied. No `shell=True`.

- [ ] **Step 2: Confirm RED**

- [ ] **Step 3: GREEN parser + Rust enum + version bump**

If this protocol bump is too invasive for the milestone, skip the enum and document that operators run `make posix-tool-test` directly. Do not overload `job=it` with POSIX tool tests without updating docs and timeouts.

- [ ] **Step 4: Do not add `posix-run` as a job**

---

### Task 6: Fuzz corpus for mq, kill, yield, sleep

**Files:**
- Modify: `src/syscall/fuzz.rs`
- Test: StagingTest that the fuzz match arms include the Linux numbers from the spec, **or** a host unit test of a extracted corpus table if that can live in `*_logic_shared.rs`

**Interfaces:**
- Consumes: `fuzz_syscalls_with_config`
- Produces: structured cases for sched_yield (124), kill (129), clock_nanosleep (115), mq_timedsend, futex already present

- [ ] **Step 1: RED — corpus table names those families**

Do not require a live kernel. Assert source contains the success-path argument shapes (scratch pointers, O_NONBLOCK mq send, yield with zero args).

- [ ] **Step 2: Confirm RED** if mq/kill/yield are missing

- [ ] **Step 3: GREEN — add named cases; keep clone out of the default round**

- [ ] **Step 4: Manual/guest verify with `fuzzsc seed=1 iterations=8 time=5` when a kernel is already running**

Do not boot a dedicated fuzz VM as part of UT.

---

### Task 7: StagingTests for the standing syscall/POSIX contracts

**Files:**
- Test: `scripts/posix/tests/test_build.py` (already contains several)
- No production change unless a contract is missing

Keep these GREEN and treat them as the UT skill's required set:

- `test_sched_yield_enqueues_current_before_reschedule`
- `test_mq_open_16_1_counts_exclusive_create_race_as_success`
- `test_aarch64_sleep_uses_sigtimedwait_for_abort` (dual-arch sigtimedwait)
- `test_aarch64_mq_send_polls_instead_of_parking` (dual-arch poll, SIGUSR1, `time(`)

- [ ] **Step 1: Run `make posix-tool-test`**

Expected: PASS. If RED, fix the contract or the production code with TDD; do not weaken the assertion.

- [ ] **Step 2: Add one StagingTest that `classify` still forbids `posixtest all` after Task 3**

---

### Task 8: Documentation and operator index

**Files:**
- Modify: `docs/TESTING.md` — point UT/IT/ST/fuzzing at the spec; note host crate blocked
- Modify: `docs/USER_SHELL.md` — skills list, `mode=syscall`, posixtest canary allowlist
- Modify: `docs/POSIX_CONFORMANCE.md` — one paragraph: Hermes does not run `posixtest all`; official path remains `make posix-run`
- Modify: `README.md` only if it already lists `hermes test-all` behavior

- [ ] **Step 1: RED docs contracts** (string presence in StagingTest or host IT)

- [ ] **Step 2: GREEN docs**

- [ ] **Step 3: Do not claim a new 1600-test campaign was run**

---

### Task 9: Focused POSIX canary (implementation-time only)

**Files:** none in-tree except report dirs under `target/posix/<arch>/focused-hermes-canary/`

- [ ] **Step 1: Host-controlled canary on the arch under test**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros --api getpid --qemu-memory 1024M
```

Use isolated disk. Do not overwrite campaign-keep.

- [ ] **Step 2: If getpid fails, stop and debug with the systematic-debugging skill. Do not open the full suite.**

- [ ] **Step 3: Optional second canary API from the family matrix after a production change, never a speculative 1600.**

---

## Execution Notes

- Implement tasks in order. Task 5 is optional. Task 9 is verification, not wiring.
- If `tests/host` is still uncompilable, every host-crate test in this plan is replaced by a StagingTest that reads the same source strings.
- Do not `posix-build`. Do not commit unless asked.
- Do not kill an in-use AArch64 QEMU using `kernel8.img` and the repo-root disk.
