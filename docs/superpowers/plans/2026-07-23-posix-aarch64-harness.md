# POSIX AArch64 Harness And Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a reproducible AArch64 Open POSIX Test Suite baseline, run selected binaries through the SMROS shell as real EL0 programs, and produce detailed per-API and per-group reports without counting modeled Rust tests as conformance.

**Architecture:** A Python standard-library host tool pins, audits, cross-builds, stages, runs, and reports the public GPLv2 suite. It emits a compact tab-separated guest manifest and a canonical JSON host manifest. A new no-std Rust service parses the guest manifest, filters tests, launches them through an observer-enabled `run_elf`, emits versioned serial events, and leaves process/VFS/signal/thread correctness to later milestones.

**Tech Stack:** Python 3 standard library, GNU AArch64 cross toolchain, qemu-user for the Linux reference, Rust `no_std` kernel services, QEMU system AArch64, glibc, FxFS `/shared`, JSON/NDJSON/JUnit XML/CSV/Markdown/HTML reports.

---

## File Structure

### Host Tooling

- Create `scripts/posix/__init__.py`: package marker and schema version.
- Create `scripts/posix/model.py`: immutable source, test, build, runtime, and summary records plus status constants.
- Create `scripts/posix/source.py`: source-lock loading, safe checkout, revision verification, and patch-series application.
- Create `scripts/posix/discovery.py`: suite test discovery, API/group classification, shell-test inventory, and stub-audit validation.
- Create `scripts/posix/build.py`: per-file cross compilation, architecture-correct `nm`/`readelf` use, runtime dependency staging, and deterministic manifests.
- Create `scripts/posix/baseline.py`: exact AArch64 artifact execution under qemu-user with per-test timeouts.
- Create `scripts/posix/events.py`: SMROS serial event parser and interrupted-run handling.
- Create `scripts/posix/report.py`: aggregation and JSON, NDJSON, JUnit, CSV, Markdown, and HTML renderers.
- Create `scripts/posix/qemu_runner.py`: QEMU lifecycle, shell command driving, watchdog/reboot logic, and raw serial capture.
- Create `scripts/posix/cli.py`: `fetch`, `audit`, `build`, `baseline`, `run-smros`, and `report` subcommands.
- Create `scripts/posix/tests/`: isolated standard-library unit tests for each host module.

### Provenance

- Create `third_party/posixtest/source.lock.json`: pinned repository and revision.
- Create `third_party/posixtest/README.md`: license, provenance, update, audit, and patch rules.
- Create `third_party/posixtest/patches/series`: ordered patch list, initially comments only.
- Create `third_party/posixtest/stub-review.tsv`: complete review of every source containing an executable `PTS_UNTESTED` path.
- Create `third_party/posixtest/shell-review.tsv`: classification of upstream shell files as test, generator, or helper.

### Guest Runtime

- Create `src/user_level/services/posix_test_logic_shared.rs`: host-testable manifest atom, filter, PTS status, and resource-delta rules.
- Create `src/user_level/services/posix_test.rs`: manifest parser, runner state machine, serial events, resource snapshots, and shell-facing API.
- Modify `src/user_level/services/run_elf.rs`: environment-aware launch requests and typed completion observers.
- Modify `src/user_level/services/user_shell.rs`: register and implement `posixtest`.
- Modify `src/user_level/services/mod.rs`: export the service and shared logic.
- Modify `src/syscall/syscall.rs`: expose a read-only POSIX resource snapshot.

### Integration And Documentation

- Modify `.gitignore`: ignore generated `host_shared/posixtest/` artifacts.
- Modify `Makefile`: add offline host-tool tests and explicit POSIX fetch/build/baseline/run/report targets.
- Modify `tests/host/src/lib.rs`: execute shared POSIX helper tests.
- Modify `tests/host/tests/integration_contracts.rs`: lock suite, Makefile, shell, and event-schema wiring.
- Create `docs/POSIX_CONFORMANCE.md`: operator guide, metric definitions, limitations, and milestone status.
- Modify `docs/TESTING.md`, `docs/USER_SHELL.md`, and `README.md`: link the new workflow and state that milestone 1 is a baseline, not conformance completion.

Generated files live under `target/posix/` and `host_shared/posixtest/`. They are never committed. Normal `make build` remains unchanged unless `make posix-stage` has populated the ignored staging directory.

---

### Task 1: Pin Suite Provenance And Validate Checkouts

**Files:**
- Create: `third_party/posixtest/source.lock.json`
- Create: `third_party/posixtest/README.md`
- Create: `third_party/posixtest/patches/series`
- Create: `scripts/posix/__init__.py`
- Create: `scripts/posix/model.py`
- Create: `scripts/posix/source.py`
- Create: `scripts/posix/cli.py`
- Create: `scripts/posix/tests/__init__.py`
- Create: `scripts/posix/tests/test_source.py`

- [ ] **Step 1: Write failing source-lock and checkout tests**

```python
# scripts/posix/tests/test_source.py
import json
import tempfile
import unittest
from pathlib import Path

from scripts.posix.source import SourceLock, load_source_lock, validate_checkout
from scripts.posix.cli import create_parser


class SourceTests(unittest.TestCase):
    def test_loads_exact_pinned_revision(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "source.lock.json"
            path.write_text(json.dumps({
                "schema": 1,
                "url": "https://github.com/emscripten-core/posixtestsuite.git",
                "revision": "85555325079ea362fa680bd2209c843cfe47e670",
                "license": "GPL-2.0-only",
                "standard": "IEEE Std 1003.1-2001 System Interfaces",
            }), encoding="utf-8")
            lock = load_source_lock(path)
            self.assertEqual(lock.revision, "85555325079ea362fa680bd2209c843cfe47e670")

    def test_rejects_non_commit_revision(self):
        with self.assertRaises(ValueError):
            SourceLock(1, "https://example.invalid/suite.git", "main", "GPL-2.0-only", "POSIX")

    def test_checkout_must_contain_license_and_expected_head(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "COPYING").write_text("GPL", encoding="utf-8")
            (root / ".smros-revision").write_text("bad\n", encoding="ascii")
            with self.assertRaises(ValueError):
                validate_checkout(root, "85555325079ea362fa680bd2209c843cfe47e670")

    def test_fetch_subcommand_is_registered(self):
        args = create_parser().parse_args(["fetch", "--work-dir", "target/posix"])
        self.assertEqual(args.command, "fetch")
```

- [ ] **Step 2: Run the tests and verify the module is missing**

Run: `python3 -m unittest scripts.posix.tests.test_source -v`

Expected: `ERROR` with `ModuleNotFoundError: No module named 'scripts.posix.source'`.

- [ ] **Step 3: Add the immutable source lock and provenance documentation**

```json
{
  "schema": 1,
  "url": "https://github.com/emscripten-core/posixtestsuite.git",
  "revision": "85555325079ea362fa680bd2209c843cfe47e670",
  "license": "GPL-2.0-only",
  "standard": "IEEE Std 1003.1-2001 System Interfaces"
}
```

Document that the mirror contains Emscripten changes, the commit is intentionally immutable, fetched source remains GPLv2, `COPYING` must exist, patches cannot weaken assertions, and updates require regenerating both review TSV files.

- [ ] **Step 4: Implement strict lock parsing, shared records, and checkout validation**

```python
# scripts/posix/source.py
from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")


@dataclass(frozen=True)
class SourceLock:
    schema: int
    url: str
    revision: str
    license: str
    standard: str

    def __post_init__(self) -> None:
        if self.schema != 1 or not self.url.startswith("https://"):
            raise ValueError("unsupported POSIX source lock")
        if not COMMIT_RE.fullmatch(self.revision):
            raise ValueError("revision must be a full lowercase Git commit")
        if self.license != "GPL-2.0-only":
            raise ValueError("unexpected suite license")


def load_source_lock(path: Path) -> SourceLock:
    data = json.loads(path.read_text(encoding="utf-8"))
    if set(data) != {"schema", "url", "revision", "license", "standard"}:
        raise ValueError("source lock fields do not match schema 1")
    return SourceLock(**data)


def validate_checkout(root: Path, revision: str) -> None:
    if not (root / "COPYING").is_file():
        raise ValueError("suite checkout is missing COPYING")
    marker = root / ".smros-revision"
    if not marker.is_file() or marker.read_text(encoding="ascii").strip() != revision:
        raise ValueError("suite checkout revision does not match source lock")


def fetch_checkout(lock: SourceLock, root: Path, patch_series: Path) -> None:
    root.parent.mkdir(parents=True, exist_ok=True)
    if root.exists():
        validate_checkout(root, lock.revision)
        return
    subprocess.run(["git", "clone", "--no-checkout", lock.url, str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "fetch", "--depth", "1", "origin", lock.revision], check=True)
    subprocess.run(["git", "-C", str(root), "checkout", "--detach", lock.revision], check=True)
    (root / ".smros-revision").write_text(lock.revision + "\n", encoding="ascii")
    for line in patch_series.read_text(encoding="utf-8").splitlines():
        patch = line.strip()
        if patch and not patch.startswith("#"):
            subprocess.run(["git", "-C", str(root), "apply", str(patch_series.parent / patch)], check=True)
    validate_checkout(root, lock.revision)
```

Add immutable `SuiteTest`, `BuildResult`, `RuntimeAttempt`, and `RunMetadata`
dataclasses to `model.py`. Their common keys are stable test ID, group, API,
kind, disposition, source path, binary path/checksum, timeout, status, duration,
and provenance IDs. Define the PTS exit constants in this module so baseline,
event, and report code cannot drift.

```python
# scripts/posix/model.py
from __future__ import annotations

from dataclasses import dataclass

PTS_PASS = 0
PTS_FAIL = 1
PTS_UNRESOLVED = 2
PTS_UNSUPPORTED = 4
PTS_UNTESTED = 5


@dataclass(frozen=True)
class SuiteTest:
    test_id: str
    group: str
    api: str
    kind: str
    disposition: str
    source: str
    binary: str | None
    sha256: str | None
    timeout_ms: int


@dataclass(frozen=True)
class BuildResult:
    test_id: str
    stage: str
    status: str
    argv: tuple[str, ...]
    returncode: int | None
    stdout: str
    stderr: str
    duration_ms: int
    artifact_sha256: str | None


@dataclass(frozen=True)
class RuntimeAttempt:
    test_id: str
    platform: str
    status: str
    exit_code: int | None
    signal: int | None
    timed_out: bool
    duration_ms: int
    stdout: str
    stderr: str
    source: str


@dataclass(frozen=True)
class RunMetadata:
    run_id: str
    platform: str
    manifest_sha256: str
    build_id: str
    complete: bool
```

- [ ] **Step 5: Implement the initial CLI with lazy subcommand dispatch**

```python
# scripts/posix/cli.py
from __future__ import annotations

import argparse
import sys
from pathlib import Path

from scripts.posix.source import fetch_checkout, load_source_lock


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="smros-posixtest")
    subparsers = parser.add_subparsers(dest="command", required=True)
    fetch = subparsers.add_parser("fetch")
    fetch.add_argument("--work-dir", default="target/posix")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = create_parser().parse_args(argv)
    if args.command == "fetch":
        lock = load_source_lock(Path("third_party/posixtest/source.lock.json"))
        fetch_checkout(
            lock,
            Path(args.work_dir) / "src" / lock.revision,
            Path("third_party/posixtest/patches/series"),
        )
        return 0
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 6: Run source tests**

Run: `python3 -m unittest scripts.posix.tests.test_source -v`

Expected: all four tests pass.

- [ ] **Step 7: Commit provenance support**

```bash
git add third_party/posixtest scripts/posix
git commit -m "test: pin Open POSIX Test Suite source"
```

---

### Task 2: Discover Tests And Enforce Complete Reviews

**Files:**
- Create: `scripts/posix/discovery.py`
- Create: `scripts/posix/tests/test_discovery.py`
- Create: `third_party/posixtest/stub-review.tsv`
- Create: `third_party/posixtest/shell-review.tsv`
- Modify: `scripts/posix/cli.py`

- [ ] **Step 1: Write failing discovery and review-completeness tests**

Use a temporary fixture with `conformance/interfaces/mmap/1-1.c`,
`conformance/definitions/aio_h/1-1.c`, a `PTS_UNTESTED` stub, a conditional
`PTS_UNTESTED` path, and one `.sh` file. Assert that IDs are relative POSIX
paths, API is the parent directory, group is derived from the API catalog,
definition tests are build-only, and every stub/shell candidate requires one
review row.

```python
def test_missing_stub_review_is_rejected(self):
    tests = discover_tests(self.root)
    with self.assertRaisesRegex(ValueError, "missing stub review"):
        apply_reviews(tests, {}, {})

def test_duplicate_review_is_rejected(self):
    with self.assertRaisesRegex(ValueError, "duplicate review"):
        load_review(self.write_review("path\texclude-stub\treason\npath\truntime-path\treason\n"))
```

- [ ] **Step 2: Run discovery tests and verify failure**

Run: `python3 -m unittest scripts.posix.tests.test_discovery -v`

Expected: import failure for `scripts.posix.discovery`.

- [ ] **Step 3: Implement deterministic discovery and strict TSV review parsing**

Discover the 1,979 buildable C sources selected by upstream naming rules and
inventory all 176 shell files. Never infer an exclusion merely because a file
mentions `PTS_UNTESTED`; only `exclude-stub` rows are excluded. Allowed stub
review dispositions are `exclude-stub` and `runtime-path`. Allowed shell
review dispositions are `test`, `generator`, and `helper`.

```python
def api_group(api: str) -> str:
    if api.startswith("pthread_"):
        return "threads"
    if api.startswith("mq_"):
        return "message-queues"
    if api.startswith("sem_"):
        return "semaphores"
    if api.startswith("aio_") or api == "lio_listio":
        return "aio"
    if api.startswith("sched_"):
        return "scheduling"
    if api.startswith("sig") or api in {"kill", "killpg", "raise", "signal"}:
        return "signals"
    if api.startswith("clock") or api.startswith("timer_") or api in {"nanosleep", "time"}:
        return "time"
    if api in {"mmap", "munmap", "mlock", "mlockall", "munlock", "munlockall", "shm_open", "shm_unlink"}:
        return "memory"
    return "base"
```

Extend `create_parser()` with `audit --write-candidates <path>` and
`audit --check`. Dispatch both through `discovery.audit_reviews`, returning
nonzero for incomplete, duplicate, stale, or invalid review rows.

- [ ] **Step 4: Generate review candidates from the pinned checkout**

Run:

```bash
python3 -m scripts.posix.cli fetch
python3 -m scripts.posix.cli audit --write-candidates target/posix/review
```

Expected: `stub-candidates.tsv` includes every C source with an executable
`PTS_UNTESTED` reference and `shell-candidates.tsv` includes every discovered
shell file. The command exits nonzero until committed review files cover every
candidate exactly once.

- [ ] **Step 5: Audit definition stubs and shell helpers**

For each definition candidate that only prints “not implemented” and returns
`PTS_UNTESTED`, record `exclude-stub` with that exact evidence. Record
conditional runtime paths as `runtime-path`. Classify cleanup-only `cln.sh`
files as `helper`; classify source-generating scripts as `generator`; classify
actual assertions as `test`. Do not classify a complete assertion as a helper.

- [ ] **Step 6: Audit interface stubs and validate full coverage**

Review every remaining candidate using the same rule, then run:

```bash
python3 -m scripts.posix.cli audit --check
```

Expected: exit 0 with discovered, stub-review, and shell-review counts; no
missing, stale, or duplicate rows.

- [ ] **Step 7: Run discovery tests**

Run: `python3 -m unittest scripts.posix.tests.test_discovery -v`

Expected: all tests pass.

- [ ] **Step 8: Commit discovery and audited classifications**

```bash
git add scripts/posix third_party/posixtest
git commit -m "test: audit POSIX suite test inventory"
```

---

### Task 3: Cross-Build Tests And Stage AArch64 Runtime Files

**Files:**
- Create: `scripts/posix/build.py`
- Create: `scripts/posix/tests/test_build.py`
- Modify: `scripts/posix/model.py`
- Modify: `scripts/posix/cli.py`
- Modify: `.gitignore`

- [ ] **Step 1: Write failing build-command and staging tests**

Test that compile commands use `aarch64-linux-gnu-gcc`, inspect objects with
`aarch64-linux-gnu-nm` rather than host `nm`, use `-std=gnu99`,
`-D_POSIX_C_SOURCE=200112L`, `-D_XOPEN_SOURCE=600`, `-pthread`, `-lrt`, and
`-lm`, and never run target objects during the build. Test that manifest order
and SHA-256 values are deterministic and staged paths cannot escape the stage
root.

```python
def test_link_detection_uses_target_nm(self):
    command = nm_command("aarch64-linux-gnu-nm", Path("case.o"))
    self.assertEqual(command, ["aarch64-linux-gnu-nm", "-g", "--defined-only", "case.o"])

def test_stage_path_rejects_parent_component(self):
    with self.assertRaises(ValueError):
        safe_stage_path(Path("stage"), "../libc.so.6")
```

- [ ] **Step 2: Run build tests and verify failure**

Run: `python3 -m unittest scripts.posix.tests.test_build -v`

Expected: import failure for `scripts.posix.build`.

- [ ] **Step 3: Implement per-source compile/link result capture**

Compile every C source separately into `target/posix/aarch64/obj/`. Use target
`nm` to detect `main`; link only runnable objects. Capture argv, return code,
bounded stdout/stderr, object/executable checksum, and duration in
`build-results.ndjson`. A compile or link failure remains a manifest result and
does not abort other tests.

Definition-only tests pass through successful compilation. Reviewed upstream
stubs retain their build results but are tagged `excluded-upstream-stub`.
Unported shell assertions are tagged `not-built-shell-test` and block program
completion; generators/helpers do not become guest tests.

- [ ] **Step 4: Implement dependency closure and deterministic manifests**

Use `aarch64-linux-gnu-readelf -l -d` to collect the interpreter and `NEEDED`
entries. Resolve each against the configured AArch64 sysroot, copy it into
`host_shared/posixtest/lib/`, and fail on unresolved or basename-colliding
libraries. Write:

```text
host_shared/posixtest/manifest.tsv
host_shared/posixtest/manifest.json
host_shared/posixtest/build-results.ndjson
host_shared/posixtest/bin/<test-id>.test
host_shared/posixtest/lib/<runtime-file>
```

Guest manifest rows use exactly nine tab-separated fields:

```text
test<TAB>id<TAB>group<TAB>api<TAB>kind<TAB>disposition<TAB>path<TAB>timeout_ms<TAB>sha256
```

The header is `SMROS_POSIX_MANIFEST<TAB>1`; metadata rows carry source,
revision, architecture, compiler, libc, patch checksum, manifest checksum, and
SMROS commit. Reject tabs, control bytes, `.`/`..` path components, duplicate
IDs, duplicate paths, nondecimal timeouts, and manifests over 4,096 tests or 2
MiB.

Extend `model.py` with the finalized build fields and extend `cli.py` with
`build --arch --stage [--verify-only]`. The command returns zero when the build
campaign completes and records individual failures; it returns nonzero for
toolchain, provenance, manifest, staging, or dependency-closure failures.

- [ ] **Step 5: Keep generated artifacts out of normal source control**

Add these lines to `.gitignore`:

```gitignore
/host_shared/posixtest/
/target/posix/
```

Normal builds do not fetch or generate the directory. Dedicated POSIX builds
use at least 1 GiB QEMU RAM because the current `/shared` mechanism embeds
generated files into the kernel image. Record total staged bytes and fail the
stage command above 256 MiB so growth is explicit.

- [ ] **Step 6: Run unit tests and a real AArch64 build**

Run:

```bash
python3 -m unittest scripts.posix.tests.test_build -v
python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
```

Expected: unit tests pass; the build command completes even when individual
upstream tests fail to compile, writes deterministic manifests, and prints
discovered/build-pass/build-fail/link-pass/link-fail/shell-unported counts.

- [ ] **Step 7: Verify staged ELF architecture and runtime closure**

Run:

```bash
find host_shared/posixtest/bin -type f -name '*.test' -print0 | xargs -0 file
python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
```

Expected: every executable is AArch64 ELF; verification reports no missing
binary, checksum mismatch, unresolved interpreter/library, or unsafe path.

- [ ] **Step 8: Commit build tooling**

```bash
git add .gitignore scripts/posix
git commit -m "test: cross-build POSIX suite for AArch64"
```

---

### Task 4: Run The AArch64 Linux Reference Baseline

**Files:**
- Create: `scripts/posix/baseline.py`
- Create: `scripts/posix/tests/test_baseline.py`
- Modify: `scripts/posix/model.py`
- Modify: `scripts/posix/cli.py`

- [ ] **Step 1: Write failing result-classification and timeout tests**

Cover exit codes 0/1/2/4/5, values above 5, signal termination, launch error,
and `subprocess.TimeoutExpired`. Assert output truncation records original byte
count and keeps separate stdout/stderr.

- [ ] **Step 2: Run baseline tests and verify failure**

Run: `python3 -m unittest scripts.posix.tests.test_baseline -v`

Expected: import failure for `scripts.posix.baseline`.

- [ ] **Step 3: Implement exact-artifact qemu-user execution**

Run each staged executable as:

```text
qemu-aarch64 -L <aarch64-sysroot> <absolute-test-path>
```

Use a fresh temporary working directory, a minimal deterministic environment
(`PATH`, `LANG=C`, `LC_ALL=C`, `TMPDIR`, `LD_LIBRARY_PATH`), manifest timeout,
and a new process group. On timeout, terminate the group, wait briefly, then
kill it. Write one canonical result object per test to
`target/posix/aarch64/linux-reference/results.ndjson`.

- [ ] **Step 4: Add prerequisite diagnostics**

Fail before running if `qemu-aarch64`, the configured sysroot, interpreter, or
any manifest runtime file is missing. Print the Debian/Ubuntu prerequisite
without attempting privileged installation:

```text
sudo apt-get install qemu-user gcc-aarch64-linux-gnu libc6-dev-arm64-cross
```

Extend `model.py` with signal/timeout/launch-error attempt fields and register
`baseline --api --group --test --sysroot` in `cli.py`. The filters are mutually
exclusive and use the same exact-match rules as the guest.

- [ ] **Step 5: Run baseline tests and a focused baseline**

Run:

```bash
python3 -m unittest scripts.posix.tests.test_baseline -v
python3 -m scripts.posix.cli baseline --api getpid --sysroot /usr/aarch64-linux-gnu
```

Expected: unit tests pass. When qemu-user is installed, reference results are
written with provenance and a terminal run record. If it is absent, the command
exits nonzero with the exact prerequisite diagnostic and does not create a
partial passing report.

- [ ] **Step 6: Commit reference runner**

```bash
git add scripts/posix
git commit -m "test: add AArch64 Linux POSIX baseline runner"
```

---

### Task 5: Aggregate Results And Render Detailed API Coverage

**Files:**
- Create: `scripts/posix/events.py`
- Create: `scripts/posix/report.py`
- Create: `scripts/posix/tests/test_events.py`
- Create: `scripts/posix/tests/test_report.py`
- Modify: `scripts/posix/model.py`
- Modify: `scripts/posix/cli.py`

- [ ] **Step 1: Write failing serial-event parser tests**

Use logs with ordinary kernel lines, program output between start/end events,
valid `SMROS_POSIX_EVENT {json}`, a duplicate terminal event, malformed JSON,
and a missing suite-end event. Require schema 1, monotonically increasing event
sequence numbers, matching run/test IDs, and explicit incomplete-run status.

- [ ] **Step 2: Write failing denominator and renderer tests**

Use a fixture containing complete pass/fail/unbuilt tests, one definition
compile pass, and one audited stub. Assert:

```python
build = summary["metrics"]["build_coverage"]
completion = summary["metrics"]["program_completion"]
self.assertEqual((build["numerator"], build["denominator"]), (3, 4))
self.assertEqual((completion["numerator"], completion["denominator"]), (2, 4))
self.assertNotIn("stub-case", completion["test_ids"])
```

Also parse every JSON/NDJSON/JUnit/CSV output and assert Markdown/HTML escape
test output rather than interpreting it.

- [ ] **Step 3: Run report tests and verify failure**

Run: `python3 -m unittest scripts.posix.tests.test_events scripts.posix.tests.test_report -v`

Expected: imports fail for the missing modules.

- [ ] **Step 4: Implement strict event parsing and canonical aggregation**

Statuses are `pass`, `fail`, `unresolved`, `unsupported`, `untested`,
`interrupted`, `timeout`, `crash`, `launch-error`, `build-fail`, `not-built`,
and `flaky`. Merge build, Linux reference, and SMROS attempts by stable test ID.
Retain every attempt. Mark differing repeated outcomes flaky. Never infer a
missing test as pass.

Calculate build coverage, execution coverage, pass coverage, and program
completion exactly as defined in the approved design. Emit the same counts and
fractions globally, by group, and by API. Include provenance, durations,
failure output, reference delta, resource deltas, and exclusion evidence.

- [ ] **Step 5: Implement all report formats**

Write atomically into the requested output directory:

```text
events.ndjson
summary.json
junit.xml
groups.csv
apis.csv
report.md
index.html
```

Use `json`, `csv`, `html`, and `xml.etree.ElementTree`; do not build structured
formats with string concatenation. The HTML is a static table-based report with
status filters and no external assets.

Extend `model.py` with the canonical summary records and register `report`
inputs for manifest, Linux results, SMROS results, and output directory in
`cli.py`. At least one runtime-result input is required.

- [ ] **Step 6: Run report tests**

Run: `python3 -m unittest scripts.posix.tests.test_events scripts.posix.tests.test_report -v`

Expected: all tests pass and parse their generated artifacts.

- [ ] **Step 7: Generate the Linux reference report**

Run:

```bash
python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --linux-results target/posix/aarch64/linux-reference/results.ndjson \
  --out target/posix/aarch64/linux-reference/report
```

Expected: all seven artifacts exist; summary explicitly lists build failures,
unported shell tests, runtime failures, and audited stubs.

- [ ] **Step 8: Commit reporting**

```bash
git add scripts/posix
git commit -m "test: report POSIX coverage by API group"
```

---

### Task 6: Add Host-Testable Guest Manifest And Status Rules

**Files:**
- Create: `src/user_level/services/posix_test_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`

- [ ] **Step 1: Add failing shared-logic tests**

Include the new shared file in `tests/host/src/lib.rs` and test valid/invalid
manifest atoms, safe staged binary paths, filter matching, PTS exit mapping,
and signed resource deltas.

```rust
assert!(posix_test_logic::manifest_atom_valid("conformance/interfaces/mmap/1-1"));
assert!(!posix_test_logic::manifest_atom_valid("../mmap"));
assert_eq!(posix_test_logic::pts_status(0), 0);
assert_eq!(posix_test_logic::pts_status(5), 4);
assert_eq!(posix_test_logic::pts_status(9), 1);
assert_eq!(posix_test_logic::resource_delta(4, 7), 3);
```

- [ ] **Step 2: Run host unit tests and verify failure**

Run: `make ut`

Expected: compile failure because `posix_test_logic_shared.rs` does not exist.

- [ ] **Step 3: Implement pure shared macros and thin wrappers**

Define numeric PTS categories `0=pass`, `1=fail`, `2=unresolved`,
`3=unsupported`, `4=untested`, `5=interrupted`. Reject empty atoms, tabs,
control bytes, backslashes, `//`, and `.`/`..` path segments. Filters match
exact IDs/APIs/groups; `all` matches all runnable complete tests.
Unrecognized normal process exits are failures; interrupted is reserved for
runner infrastructure failures.

- [ ] **Step 4: Run unit and format checks**

Run:

```bash
make ut
cargo fmt --manifest-path tests/host/Cargo.toml --check
```

Expected: both pass.

- [ ] **Step 5: Commit guest rules**

```bash
git add src/user_level/services/posix_test_logic_shared.rs tests/host/src/lib.rs
git commit -m "test: define POSIX guest manifest rules"
```

---

### Task 7: Parse Guest Manifests And Snapshot Resources

**Files:**
- Create: `src/user_level/services/posix_test.rs`
- Modify: `src/user_level/services/mod.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing integration contract assertions**

Assert the service module is exported, the manifest path is
`/shared/posixtest/manifest.tsv`, the parser enforces schema 1 and the 2 MiB /
4,096-test limits, and `PosixResourceSnapshot` reads process, thread, mapping,
fd, shared-memory, and handle counts.

- [ ] **Step 2: Run integration tests and verify failure**

Run: `make it`

Expected: the new contract test fails because the service and snapshot do not
exist.

- [ ] **Step 3: Add a read-only resource snapshot**

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PosixResourceSnapshot {
    pub processes: usize,
    pub scheduler_threads: usize,
    pub linux_mappings: usize,
    pub linux_fds: usize,
    pub linux_shared_memory: usize,
    pub kernel_handles: usize,
}

pub fn posix_resource_snapshot() -> PosixResourceSnapshot {
    let state = memory_state();
    PosixResourceSnapshot {
        processes: crate::kernel_lowlevel::memory::process_manager().active_processes(),
        scheduler_threads: crate::kernel_objects::scheduler::scheduler().active_threads(),
        linux_mappings: state.linux_mappings.len(),
        linux_fds: state.linux_fds.len(),
        linux_shared_memory: state.linux_shared_memory.len(),
        kernel_handles: state.handles.len(),
    }
}
```

This function is diagnostic only and must not mutate or reset state.

- [ ] **Step 4: Implement the bounded guest manifest parser**

Load via `fxfs::ensure_host_share()` and `fxfs::read_file`. Parse the header,
metadata, and nine-field test rows. Reject invalid UTF-8, unknown row types,
unknown kind/disposition, duplicates, unsafe paths, checksum width other than
64 lowercase hex characters, zero/overflowing timeouts, and any runnable path
outside `/shared/posixtest/bin/`.

Expose:

```rust
pub enum PosixFilter { All, Group(String), Api(String), Test(String) }
pub fn parse_filter(args: &[&str]) -> Result<PosixFilter, PosixTestError>;
pub fn load_manifest() -> Result<PosixManifest, PosixTestError>;
pub fn status_snapshot() -> PosixRunnerStatus;
```

- [ ] **Step 5: Run host and integration tests**

Run: `make ut it`

Expected: all tests pass.

- [ ] **Step 6: Commit parser and resource diagnostics**

```bash
git add src/user_level/services/posix_test.rs src/user_level/services/mod.rs src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: parse POSIX test manifests"
```

---

### Task 8: Make The ELF Launcher Observable And Environment-Aware

**Files:**
- Modify: `src/user_level/services/run_elf.rs`
- Modify: `src/user_level/services/user_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing environment and observer contract tests**

Test environment validation (`KEY=value`, no NUL, no empty key), observer
completion classification, and source wiring that preserves the old `spawn`
API while adding `spawn_observed`.

- [ ] **Step 2: Run tests and verify failure**

Run: `make ut it`

Expected: failures for missing environment validation and observer API.

- [ ] **Step 3: Add typed launch requests and outcomes**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunObserver { Shell, PosixTest }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunTermination { Exit(i32), LaunchError(RunElfError) }

#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub path: String,
    pub termination: RunTermination,
    pub elapsed_ticks: u64,
}

pub fn spawn(path: String, argv: Vec<String>) -> Result<(), RunElfError> {
    spawn_observed(path, argv, Vec::new(), RunObserver::Shell)
}
```

`spawn_observed` stores validated environment entries and an observer in the
single active request. `build_initial_stack` uses those entries, adding the
default `LD_LIBRARY_PATH=/shared/posixtest/lib:/shared/lib:/lib` only when the
caller did not supply `LD_LIBRARY_PATH`.

Prepend `/shared/posixtest/lib/` in `resolve_library_path` so the test toolchain's
pinned interpreter is used before the general `/shared/lib` and `/lib` copies.
The ordinary shell `run` wrapper keeps its existing behavior and arguments.

- [ ] **Step 4: Deliver every terminal outcome exactly once**

On loader preparation failure or normal `exit`/`exit_group`, clear active
state, calculate duration, and dispatch one `RunOutcome`. `Shell` retains the
existing human-readable output. `PosixTest` calls
`posix_test::on_run_outcome(outcome)` and does not print the shell summary.
Reset signal/timer state in both paths.

- [ ] **Step 5: Search the tree for bypasses**

Run:

```bash
rg -n "run_elf::spawn|prepare_run_elf_return|RunObserver|RunOutcome" src tests/host
```

Expected: `cmd_run` still uses the compatibility wrapper; only the POSIX
service uses `PosixTest`; every launch-error and exit path dispatches an
outcome.

- [ ] **Step 6: Run tests and production build**

Run: `make ut it build-test`

Expected: all pass.

- [ ] **Step 7: Commit launcher observer support**

```bash
git add src/user_level/services/run_elf.rs src/user_level/services/user_logic_shared.rs tests/host
git commit -m "feat: observe user ELF completion"
```

---

### Task 9: Implement The Guest Runner And Serial Event Protocol

**Files:**
- Modify: `src/user_level/services/posix_test.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing runner-state and event-schema contract tests**

Lock the event prefix `SMROS_POSIX_EVENT `, schema 1, sequence field, and event
names `suite_start`, `test_start`, `test_end`, `suite_end`, and
`infrastructure_error`. Assert only one run can be active, definition tests are
not launched, stub exclusions are emitted but not executed, and no missing
binary can become a pass.

- [ ] **Step 2: Run integration tests and verify failure**

Run: `make it`

Expected: runner/event contract test fails.

- [ ] **Step 3: Implement a bounded asynchronous runner state machine**

Maintain one `UnsafeCell<Option<RunnerState>>` using the repository's existing
single-scheduler serialization rule. `start(filter)` loads and validates the
manifest, selects matching tests, records a run ID from build ID plus timer
tick, snapshots resources, emits `suite_start`, and launches the first runnable
test through `RunObserver::PosixTest`.

`on_run_outcome` maps PTS exit codes, takes a second resource snapshot, emits
signed deltas, advances counters, and launches the next case. After the final
case it emits `suite_end` with complete counts and clears state. Launch errors
emit `test_end` with `launch-error` and continue. A manifest or invariant error
emits `infrastructure_error`, marks the run incomplete, and clears state.

- [ ] **Step 4: Emit JSON safely without a JSON dependency**

Only manifest atoms validated as printable ASCII enter JSON strings. Implement
one JSON-string writer that escapes quote and backslash anyway. Numeric fields
are decimal. Every event contains `schema`, `seq`, `event`, `run_id`,
`manifest_sha256`, and architecture. Test events also contain ID/group/API,
status, exit code or launch error, elapsed ticks, and all resource deltas.

- [ ] **Step 5: Implement status and filter behavior**

`status_snapshot()` returns idle/running, run ID, filter, current test,
completed/selected counts, and status counts. `All`, `Group`, `Api`, and `Test`
must select exact matches only. An empty selection is an error, not a successful
empty suite.

- [ ] **Step 6: Run tests and build**

Run: `make ut it build-test`

Expected: all pass and the kernel links without allocator or format-macro
errors.

- [ ] **Step 7: Commit guest runner**

```bash
git add src/user_level/services/posix_test.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: run POSIX manifest tests from SMROS"
```

---

### Task 10: Register The `posixtest` Shell Command

**Files:**
- Modify: `src/user_level/services/user_shell.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing shell wiring tests**

Assert the command table contains `posixtest`, dispatches to `cmd_posix_test`,
and supports only:

```text
posixtest all
posixtest group <group>
posixtest api <api>
posixtest test <test-id>
posixtest status
```

- [ ] **Step 2: Run integration tests and verify failure**

Run: `make it`

Expected: shell wiring test fails.

- [ ] **Step 3: Implement strict shell parsing and output**

Register:

```rust
ShellCommand {
    name: "posixtest",
    description: "Run Open POSIX Test Suite manifest cases",
    handler: cmd_posix_test,
},
```

`status` prints the bounded status snapshot. Other forms call `parse_filter`
and `start`. Invalid forms print one usage line. Busy, missing manifest,
checksum/schema, empty selection, and launch errors have distinct messages.
The handler yields after a successful asynchronous start, like `cmd_run`.

- [ ] **Step 4: Build and inspect help wiring**

Run:

```bash
make it build-test
rg -n 'name: "posixtest"|fn cmd_posix_test|posixtest all' src/user_level/services/user_shell.rs
```

Expected: tests/build pass and all three wiring points are present.

- [ ] **Step 5: Commit shell command**

```bash
git add src/user_level/services/user_shell.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: add posixtest shell command"
```

---

### Task 11: Add QEMU Collection, Watchdog, And Reboot Continuation

**Files:**
- Create: `scripts/posix/qemu_runner.py`
- Create: `scripts/posix/tests/test_qemu_runner.py`
- Modify: `scripts/posix/model.py`
- Modify: `scripts/posix/cli.py`

- [ ] **Step 1: Write failing controller-state tests**

Mock the process transport and cover prompt discovery, sending one
`posixtest test <id>` at a time, waiting for matching start/end events, refusing
an early next command, per-test deadline, QEMU exit, fatal kernel pattern,
restart, completed-ID skipping, and terminal controller record.

- [ ] **Step 2: Run controller tests and verify failure**

Run: `python3 -m unittest scripts.posix.tests.test_qemu_runner -v`

Expected: import failure for `scripts.posix.qemu_runner`.

- [ ] **Step 3: Implement QEMU lifecycle and serial collection**

Use argument arrays with `subprocess.Popen`; never use `shell=True`. Mirror the
selected AArch64 QEMU machine/CPU/block/network options from `smoke-qemu.sh`,
use at least 1,024 MiB memory, capture combined serial output, and wait for the
exact `smros:/> ` prompt before sending commands.

Drive tests individually so the host watchdog can recover from a hung identity-
mapped process. After a timeout or kernel-fatal pattern, terminate QEMU, append
a host-watchdog timeout/crash attempt with raw-log offsets, reboot, validate the
same build/manifest IDs, and continue at the next uncompleted test. Do not
convert a watchdog result into a guest PTS result.

- [ ] **Step 4: Persist resumable controller state atomically**

Store `target/posix/aarch64/smros-run/progress.json` after each attempt with
manifest/build IDs, selected IDs, completed attempts, current test, restart
count, and raw-log path. `--resume` rejects changed manifest/build IDs. A clean
terminal run writes `results.ndjson` and removes only the in-progress marker,
retaining provenance and attempts.

Register `run-smros --api --group --test --qemu-memory [--resume]` in `cli.py`.
The filters are mutually exclusive; when no filter option is present, select
all complete runnable tests. Store watchdog attempts through the same `RuntimeAttempt` model used by
the Linux baseline and report merger.

- [ ] **Step 5: Run controller tests**

Run: `python3 -m unittest scripts.posix.tests.test_qemu_runner -v`

Expected: all tests pass.

- [ ] **Step 6: Run one SMROS canary test**

Run:

```bash
python3 -m scripts.posix.cli run-smros --api getpid --qemu-memory 1024M
```

Expected: QEMU boots, the shell accepts `posixtest test ...`, raw serial and
result files are written, and the outcome is reported honestly even if current
SMROS behavior fails or crashes. Absence of a PASS does not fail the harness;
missing/corrupt events or an unrecoverable controller error does.

- [ ] **Step 7: Commit QEMU controller**

```bash
git add scripts/posix
git commit -m "test: collect POSIX results from QEMU"
```

---

### Task 12: Wire Make Targets, Documentation, And Final Verification

**Files:**
- Modify: `Makefile`
- Create: `docs/POSIX_CONFORMANCE.md`
- Modify: `docs/TESTING.md`
- Modify: `docs/USER_SHELL.md`
- Modify: `README.md`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing Makefile/documentation contract tests**

Assert `.PHONY` and recipes exist for `posix-tool-test`, `posix-fetch`,
`posix-audit`, `posix-build`, `posix-stage`, `posix-baseline`, `posix-run`, and
`posix-report`; `test` includes only offline `posix-tool-test`; docs state the
suite standard, architecture order, optional-group requirement, stub
denominator rule, output paths, and current milestone limitations.

- [ ] **Step 2: Run integration tests and verify failure**

Run: `make it`

Expected: new wiring assertions fail.

- [ ] **Step 3: Add explicit Make targets**

```make
posix-tool-test:
	@python3 -m unittest discover -s scripts/posix/tests -v

posix-fetch:
	@python3 -m scripts.posix.cli fetch

posix-audit: posix-fetch
	@python3 -m scripts.posix.cli audit --check

posix-build: posix-audit
	@python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest

posix-stage: posix-build
	@python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only

posix-baseline: posix-stage
	@python3 -m scripts.posix.cli baseline --sysroot "$${AARCH64_SYSROOT:-/usr/aarch64-linux-gnu}"

posix-run: posix-stage $(FXFS_DISK)
	@$(MAKE) build ARCH=aarch64-unknown-none
	@python3 -m scripts.posix.cli run-smros --qemu-memory "$${POSIX_QEMU_MEMORY:-1024M}"

posix-report:
	@python3 -m scripts.posix.cli report --manifest host_shared/posixtest/manifest.json --smros-results target/posix/aarch64/smros-run/results.ndjson --linux-results target/posix/aarch64/linux-reference/results.ndjson --out target/posix/aarch64/report
```

Add `posix-tool-test` to `test`; do not add network, cross-build, qemu-user, or
full QEMU targets to the fast default suite.

- [ ] **Step 4: Document operation and honest limitations**

`docs/POSIX_CONFORMANCE.md` must include prerequisites, exact commands, report
schema/metrics, PTS status meanings, group coverage, audit/update workflow,
generated artifact size behavior, watchdog semantics, and architecture order.
State prominently that milestone 1 establishes infrastructure and a failure
baseline: current identity-mapped execution, modeled processes, and incomplete
signals/threads/VFS prevent a conformance claim.

- [ ] **Step 5: Run all offline checks**

Run:

```bash
make posix-tool-test
make host-fmt-check script-check launcher-test ut it build-test
git diff --check
```

Expected: all commands pass with no formatting or whitespace errors.

- [ ] **Step 6: Run suite provenance/build checks**

Run:

```bash
make posix-fetch posix-audit posix-stage
```

Expected: pinned revision and reviews validate; staged manifest and dependency
closure verify; individual upstream build failures remain recorded rather than
causing false harness success.

- [ ] **Step 7: Run available reference and SMROS canaries**

Run:

```bash
python3 -m scripts.posix.cli baseline --api getpid --sysroot /usr/aarch64-linux-gnu
python3 -m scripts.posix.cli run-smros --api getpid --qemu-memory 1024M
python3 -m scripts.posix.cli report --manifest host_shared/posixtest/manifest.json --smros-results target/posix/aarch64/smros-run/results.ndjson --linux-results target/posix/aarch64/linux-reference/results.ndjson --out target/posix/aarch64/report
```

Expected: with qemu-user installed, both canaries produce parseable results and
all seven report artifacts. If qemu-user is unavailable, record that external
prerequisite explicitly; still run the SMROS canary and offline verification.

- [ ] **Step 8: Inspect the final report for false positives**

Verify `summary.json` shows every discovered complete test in the program-
completion denominator, excludes only reviewed stubs, lists unported shell
assertions/build failures, and never uses host Rust tests as POSIX passes.

- [ ] **Step 9: Commit milestone documentation and wiring**

```bash
git add Makefile docs README.md tests/host/tests/integration_contracts.rs
git commit -m "docs: add POSIX conformance harness workflow"
```

---

## Milestone Exit Criteria

- The source commit, patch series, toolchain, runtime files, manifest, and SMROS
  build are reproducibly identified.
- Every upstream C and shell candidate has an explicit reviewed disposition.
- Cross-build failures and unported shell assertions are visible and block
  program completion.
- The exact staged AArch64 ELF artifacts can run under the Linux reference and
  can be selected from the SMROS `posixtest` shell command.
- Guest serial events survive ordinary output interleaving and produce all
  report formats with exact per-group/per-API denominators.
- Timeout, crash, launch-error, unsupported, unresolved, untested, flaky, and
  incomplete runs cannot be reported as passes.
- The report states that process isolation and POSIX subsystem correctness are
  work for the following implementation milestones.
