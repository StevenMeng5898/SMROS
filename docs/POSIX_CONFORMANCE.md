# POSIX Conformance Harness

This `docs/POSIX_CONFORMANCE.md` guide describes the reproducible Open POSIX
Test Suite workflow. The current milestone establishes infrastructure and a
failure baseline. It is not POSIX certification and does not claim conformance
completion.

## Scope And Order

The pinned suite targets **IEEE 1003.1-2001 System Interfaces**. Project
architecture work proceeds in this order: **AArch64, then x86_64, then RISC-V64**.
Only AArch64 is wired today. Every optional group is required for
the SMROS project target; an unsupported option is recorded as an incomplete
result, not removed from the target.

The source is the public Emscripten-maintained Open POSIX Test Suite mirror at
the immutable commit in `third_party/posixtest/source.lock.json`. The suite is
GPL-2.0-only; its fetched `COPYING` file is required. The patch series is
ordered by `third_party/posixtest/patches/series` and may not weaken assertions
or convert failures into passes.

## Prerequisites

The offline tool tests require Python 3 and the Rust host-test toolchain. The
full AArch64 workflow additionally requires Git, GNU Make,
`aarch64-linux-gnu-gcc`, `aarch64-linux-gnu-nm`,
`aarch64-linux-gnu-readelf`, `qemu-aarch64`, `qemu-system-aarch64`, and
`qemu-img`. On Ubuntu or Debian, the additional reference/cross tools are:

```bash
sudo apt-get install qemu-user gcc-aarch64-linux-gnu libc6-dev-arm64-cross
```

`AARCH64_SYSROOT` defaults to `/usr/aarch64-linux-gnu`.
`POSIX_QEMU_MEMORY` defaults to `1024M`.

## Exact Workflow

Run the offline host-tool suite:

```bash
make posix-tool-test
```

Fetch and audit the pinned source. These targets use the network only when the
validated checkout is not already present:

```bash
make posix-fetch
make posix-audit
```

Cross-build, publish, and then verify the guest stage:

```bash
make posix-build
make posix-stage
```

Run the AArch64 Linux reference and SMROS campaigns:

```bash
make posix-baseline
make posix-run
```

Render the report after both result inputs exist:

```bash
make posix-report
```

Useful overrides are explicit Make variables:

```bash
make posix-baseline AARCH64_SYSROOT=/usr/aarch64-linux-gnu
make posix-run POSIX_QEMU_MEMORY=1024M
make posix-report POSIX_QUALITY_EVIDENCE=target/quality/aarch64.json
```

For a filtered diagnostic canary or a resumed QEMU campaign, invoke the CLI
directly:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli baseline --api getpid --sysroot /usr/aarch64-linux-gnu
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros --api getpid --qemu-memory 1024M
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros --qemu-memory 1024M --resume
```

## Guest Live Selection Coverage

The guest shell reports live selection coverage without changing structured
`SMROS_POSIX_EVENT` schema 1. It prints the selected test, API, and group totals
after `suite_start`, then prints progress after every 25 completed tests, when
an API completes, and at suite completion.

`tests` is completed selected tests divided by all selected tests.
`apis-complete` and `groups-complete` count units for which every selected test
has a terminal result. `apis-pass` and `groups-pass` count only complete units
whose selected tests all passed. Fail, unresolved, unsupported, untested, and
launch-error results complete selection work but prevent the containing API and
group from passing. Percentages are truncated to two decimal places.

Every live line carries `scope=selected`. These values do not include source
inventory or tests that failed to build. Live selection coverage
does not prove POSIX compliance. The host report remains authoritative for
build coverage, execution coverage, pass coverage, optional-group completion,
provenance, program completion, and the final compliance decision.

## Source Audit And Updates

`posix-audit` discovers every pinned C source and shell file and checks the
review ledgers. An executable upstream `PTS_UNTESTED` path is excluded only
when its source appears in the reviewed file allowlist
`third_party/posixtest/stub-review.tsv`. Every audited upstream stub remains
listed in reports with exclusion evidence. It is removed from coverage and
program-completion denominators only by that reviewed disposition. It is never
silently treated as a pass.

Shell files are classified in `third_party/posixtest/shell-review.tsv` as
tests, generators, or helpers. Unported shell tests remain in the program
target and block completion. To update the suite, change the pinned full commit
deliberately, review the new license and patch result, regenerate both candidate
inventories, review every changed path, update the ledgers and their pinned
checksums, and rerun the complete build/reference workflow. Do not reuse review
decisions by filename without reviewing the new contents.

## Build And Staging

Each source has a stable test ID. Compilation, symbol inspection, and linking
are recorded independently, so a compiler or linker failure stays visible in
`build-results.ndjson` rather than aborting into a false clean result. The stage
contains the manifest, runnable test binaries, and the resolved AArch64 dynamic
runtime closure. Publication verifies paths, checksums, ELF architecture,
runtime closure, and current source/build identity.

Generated build data lives below `target/posix/aarch64/`. The guest stage lives
at `host_shared/posixtest/` and is embedded into `/shared/posixtest/` on the next
kernel build. The generated staging tree has a hard **256 MiB** aggregate bound;
crossing it fails staging. Generated data is ignored and must not be committed.

## Reference, Watchdog, And Resume Semantics

The Linux reference runs the exact staged AArch64 binaries through
`qemu-aarch64` with the selected sysroot and per-test deadlines. It is a
behavioral reference, not a substitute build and not a POSIX pass for SMROS.

The SMROS controller boots QEMU, waits for the shell prompt, and runs one
manifest test at a time. Its host watchdog enforces the boot deadline, each
manifest timeout, bounded serial output, fatal serial patterns, and QEMU exit.
A watchdog timeout or crash is recorded truthfully with unavailable resource
evidence; it is never translated to a guest PTS result or zero resource use.
The controller restarts QEMU where recovery is possible and retains raw serial
bytes.

Progress is durably checkpointed after each completed test. `--resume` accepts
only canonical, bounded checkpoints whose selection, manifest, build, patch,
runtime snapshot, raw-log offsets, completed prefix, and SMROS commit match the
current campaign. It appends to the bound raw log and skips only the validated
completed prefix. Conflicting, changed, truncated, or terminal checkpoints fail
closed rather than rerunning or inventing results.

## PTS Status Meanings

- `PTS_PASS` / exit 0: the upstream assertion passed for that execution.
- `PTS_FAIL` / exit 1: the assertion failed.
- `PTS_UNRESOLVED` / exit 2: the test could not determine a result; this is not a pass.
- `PTS_UNSUPPORTED` / exit 4: the required facility was unavailable; all optional groups remain required, so this is not completion.
- `PTS_UNTESTED` / exit 5: the assertion did not run; this is not a pass.
- Other nonzero exits are failures. Host-side interruption, timeout, crash,
  launch error, build failure, and flaky repeated outcomes remain distinct
  report statuses.

## Metrics And Evidence

Metrics are calculated globally, for every group, and for every API:

- **build coverage** = successfully linked runnable complete C tests / all
  buildable complete C tests.
- **execution coverage** = built tests with a real launched execution / built
  tests.
- **pass coverage** = tests with a PTS pass / executed tests.
- **program completion** = passed complete programs plus successfully compiled
  definition-only checks / every required complete program and definition-only
  check.

The denominator includes compile failures, link failures, unported shell
assertions, unsupported optional groups, and missing executions. Only an
audited upstream stub on the reviewed file allowlist is excluded, and it is
still listed. Direct Rust and model tests never count as POSIX passes.

Each test row retains its group/API, source and disposition, build commands and
diagnostics, every runtime attempt, duration, PTS/aggregate status, bounded
failure output, Linux-reference delta, and exclusion evidence. Per-test
resource evidence is `measured`, `unavailable`, or `mixed`; measured deltas cover file
descriptors, mappings, shared memory, processes, scheduler threads, handles,
IPC objects, timers, and AIO requests. Positive residuals remain visible as
leaks and are not canceled by later cleanup.

## Inputs, Provenance, And Artifacts

The raw input set is the staged `manifest.json`, `manifest.tsv`,
`build-results.ndjson`, Linux reference NDJSON, SMROS NDJSON or raw serial log,
and staged runtime files. Report provenance binds the source revision, patch
digest, compiler and libc identity, manifest/build digests, runtime snapshot,
SMROS commit, run IDs, QEMU details, and raw-log paths.

Default paths are:

```text
target/posix/src/<pinned-revision>/
target/posix/aarch64/linux-reference/results.ndjson
target/posix/aarch64/smros-run/results.ndjson
target/posix/aarch64/report/
host_shared/posixtest/
```

The report directory contains exactly seven artifacts:

```text
events.ndjson
summary.json
junit.xml
groups.csv
apis.csv
report.md
index.html
```

`events.ndjson` preserves validated runtime records. `summary.json` is the full
normalized aggregation. JUnit describes POSIX testcases; the CSV files contain
group/API coverage; Markdown and HTML are human-readable views. All are one
atomically published generation.

Optional quality evidence is strict canonical LF JSON, at most 1 MiB, with at
most 128 uniquely named checks. Names are at most 128 UTF-8 bytes; other text
fields are at most 4096 UTF-8 bytes.
Quality evidence text rejects all Unicode C0/C1 control characters,
including tab, newline, and carriage return.
The exact input schema is:

```json
{"architecture":"aarch64","checks":[{"artifact":null,"command":null,"coverage_percent":null,"findings":null,"kind":"static-analysis","name":"coverity","status":"unavailable","summary":"Coverity tools are not installed","version":null}],"schema":1,"smros_commit":"0000000000000000000000000000000000000000"}
```

The architecture and commit must equal manifest provenance. Check status is
`passed`, `failed`, `unavailable`, or `not-run`; unavailable/not-run checks must
have null findings and coverage. Overall quality is failed if any check failed,
incomplete if any check is unavailable/not-run, and otherwise passed. The data
appears only in `summary.json`, suite-level JUnit properties, Markdown, and
HTML: quality evidence never changes POSIX denominators, `complete`, testcase
failures, `events.ndjson`, `groups.csv`, or `apis.csv`.

## Limitations And Claims

Current test programs use identity-mapped execution. SMROS has modeled process state
and incomplete VFS, signals, and threads. File-descriptor behavior,
process isolation, signal delivery, thread semantics, and filesystem behavior
therefore remain insufficient for a conformance claim even where individual
assertions pass.

Open POSIX Test Suite evidence is not IEEE or Open Group certification. It is
also not a POSIX certification, an Open Group conformance mark, or proof that
unexecuted interfaces work. This milestone is infrastructure and a failure
baseline for closing those gaps honestly.
