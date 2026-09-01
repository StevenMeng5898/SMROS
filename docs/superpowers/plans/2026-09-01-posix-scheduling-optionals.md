# POSIX Scheduling Optional Groups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the SMROS AArch64 POSIX runtime execute the 20 currently unsupported process-scope and sporadic-server scheduling tests, while preserving the exact failure evidence needed to fix the two aggregate failures.

**Architecture:** Keep the Linux scheduler policy state in the kernel and extend its policy table with the POSIX sporadic-server policy. Extend the POSIX test compile header with the standard sporadic fields, and interpose the compatibility runtime at the libc boundary to validate those fields and retain process-contention thread metadata. Process-scope threads remain backed by the existing SMROS thread objects; their metadata is updated when the process policy changes.

**Tech Stack:** Rust kernel syscall logic, C POSIX preload runtime, AArch64 cross-compilation, host Rust/Python contract tests, focused QEMU POSIX runs.

---

### Task 1: Capture scheduler policy contracts

**Files:**
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`
- Test: existing host test targets

- [ ] **Step 1: Write failing tests** for sporadic priority bounds, kernel-priority mapping, sporadic parameter validation, and process-scope metadata propagation.
- [ ] **Step 2: Run the focused host tests** and confirm they fail because policy `4`, extended parameters, and process scope are absent.
- [ ] **Step 3: Keep the tests as the executable contract** for the following implementation tasks.

### Task 2: Add POSIX scheduling definitions to the cross-build

**Files:**
- Create: `scripts/posix/runtime/include/sched.h`
- Modify: `scripts/posix/build.py`
- Modify: `scripts/posix/tests/test_build.py`

- [ ] **Step 1: Add a wrapper header** that preserves the target libc declarations, defines `SCHED_SPORADIC`, `_POSIX_SPORADIC_SERVER`, `_POSIX_THREAD_SPORADIC_SERVER`, `SS_REPL_MAX`, and the four POSIX sporadic fields in `struct sched_param`.
- [ ] **Step 2: Add the wrapper include directory to test and preload compile commands** without changing link behavior.
- [ ] **Step 3: Run build-command and header compilation tests** and verify the previously conditional sources compile their real sporadic branches.

### Task 3: Implement sporadic policy and validation

**Files:**
- Modify: `src/syscall/syscall_logic_shared.rs`
- Modify: `src/syscall/linux_task_logic_shared.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `scripts/posix/runtime/smros_posix_compat.c`
- Modify: `scripts/posix/runtime/smros_posix_compat.map`

- [ ] **Step 1: Extend policy bounds and kernel-priority mapping** for `SCHED_SPORADIC`.
- [ ] **Step 2: Validate sporadic low priority, positive normalized timespecs, budget not exceeding replenishment period, and `SS_REPL_MAX` bounds before mutating scheduler state.
- [ ] **Step 3: Interpose `sched_setparam`/`sched_setscheduler` and zero extended output fields** so libc’s four-byte Linux ABI remains compatible while POSIX fields are enforced by SMROS.
- [ ] **Step 4: Run host scheduler tests and focused sporadic QEMU tests**; ensure invalid requests leave policy and priority unchanged.

### Task 4: Implement process-contention scope

**Files:**
- Modify: `scripts/posix/runtime/smros_posix_compat.c`
- Modify: `scripts/posix/runtime/smros_posix_compat.map`
- Modify: `scripts/posix/tests/test_build.py`

- [ ] **Step 1: Interpose `pthread_attr_setscope`/`pthread_attr_getscope`** and retain scope in the existing attribute lifecycle records.
- [ ] **Step 2: Record each thread’s scope and process identity** and update process-scope records after process scheduler changes.
- [ ] **Step 3: Run all seven process-scope tests** and existing thread scheduling tests.

### Task 5: Diagnose and fix the two aggregate failures

**Files:** determined from the current run’s `results.ndjson` and serial log after Tasks 2-4.

- [ ] **Step 1: Re-run the aggregate or inspect the complete `results.ndjson`** to identify both exact failed test IDs.
- [ ] **Step 2: Reproduce each failure in an isolated QEMU run and add a focused regression test before changing code.
- [ ] **Step 3: Implement one root-cause fix at a time and verify no scheduler optional tests regress.

### Task 6: Full verification and artifact hygiene

- [ ] **Step 1: Run AArch64 release build, host Rust contracts, Python POSIX tests, and all 20 focused scheduler tests.
- [ ] **Step 2: Run `posixtest all` with bounded output and confirm no unsupported or failed records remain for the addressed cases.
- [ ] **Step 3: Record result paths and keep workspace usage below 10 GB.
- [ ] **Step 4: Commit only source, test, and plan changes; leave unrelated bytecode cache changes untouched.
