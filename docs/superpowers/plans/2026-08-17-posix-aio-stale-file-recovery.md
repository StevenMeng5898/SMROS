# POSIX AIO Stale-File Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove and close the repeated-run AIO `Error at open(): File exists` failure class without weakening the Open POSIX Test Suite or modifying the user's primary FxFS disk.

**Architecture:** Keep the existing FxFS-backed `unlink`/`unlinkat` behavior, verify host-level namespace and open-reference semantics, refresh the generated AArch64 POSIX stage so manifest provenance matches the current SMROS commit, then run same-disk AIO campaigns that reproduce the old failure boundary. Runtime validation checks logs and structured results for stale-name setup errors, restarts, timeouts, and resource growth while preserving unrelated PTS outcomes.

**Tech Stack:** Rust kernel/services, FxFS service, Linux syscall shim, Open POSIX Test Suite staging via `scripts.posix`, QEMU AArch64 system emulation, Makefile quality gates, cargo-tarpaulin host coverage when installed, Coverity when installed.

---

## File Structure

- Preserve: `src/user_level/services/fxfs.rs`
  - Owns real FxFS directory-entry removal, link-count updates, and unlinked-object reclamation.
- Preserve: `src/syscall/syscall.rs`
  - Owns `sys_unlink`, `sys_unlinkat`, and open-object lifetime handoff to FxFS.
- Preserve: `tests/host/src/lib.rs`
  - Contains focused host logic tests for FxFS link-count and unlinked-object reclaimability.
- Preserve: `tests/host/tests/integration_contracts.rs`
  - Contains source-level syscall integration contracts that prevent `unlinkat` from becoming a success-only stub again.
- Regenerate only: `host_shared/posixtest/`
  - Ignored generated stage. Rebuilt after all tracked commits and verified against current build inputs.
- Generate only: `target/posix/aarch64/aio-stale-file-*`
  - Private runtime disks, serial logs, structured results, quality evidence, and reports.
- Generate: `target/posix/aarch64/aio-stale-file-results.md`
  - Human-readable evidence summary for the stale-file recovery run.

## Acceptance Rules

- Do not edit upstream POSIX AIO tests to remove `O_EXCL`, change filenames, or clear `/tmp` between tests.
- Do not edit `/home/steven/workspace/SMROS/smros-fxfs.img`.
- Do not commit `host_shared/posixtest/`, `kernel8.img`, `target/`, or any disk image.
- A refreshed `host_shared/posixtest/manifest.json` and `manifest.tsv` must report the current tracked SMROS commit.
- Same-disk AIO runs must contain zero `Error at open(): File exists`, zero timeouts, zero QEMU restarts, and zero positive terminal resource deltas.
- `aio_cancel/7-1.c` or other unrelated PTS outcomes remain truthful; this plan proves stale-file recovery, not complete AIO conformance.

### Task 1: Commit This Plan

**Files:**
- Create: `docs/superpowers/plans/2026-08-17-posix-aio-stale-file-recovery.md`

- [ ] **Step 1: Confirm the workspace state**

Run:

```bash
git status --short --branch
git log --oneline -n 3
```

Expected: branch is `master`; unrelated generated files may be ignored, and no tracked source changes are mixed into this plan commit.

- [ ] **Step 2: Commit the plan**

Run:

```bash
git add docs/superpowers/plans/2026-08-17-posix-aio-stale-file-recovery.md
git commit -m "docs: plan POSIX AIO stale-file recovery"
```

Expected: one documentation-only commit. Do not add generated stage or runtime evidence.

### Task 2: Verify Existing Unlink Semantics

**Files:**
- Preserve: `src/user_level/services/fxfs.rs`
- Preserve: `src/syscall/syscall.rs`
- Test: `tests/host/src/lib.rs`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Run focused FxFS link-count lifetime test**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib user_logic::fxfs_hard_link_counts_retain_the_inode_until_the_last_name_is_removed -- --exact
```

Expected: PASS. This proves link counts do not underflow and an unlinked inode is reclaimable only at zero links and zero open references.

- [ ] **Step 2: Run focused syscall routing contract**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_named_semaphore_publication_uses_atomic_fxfs_links_and_inode_mmap_identity -- --exact
```

Expected: PASS. This proves `sys_unlinkat` calls `fxfs::unlink_file`, checks `linux_fxfs_object_is_open(object_id)`, and calls `fxfs::release_unlinked_file(object_id)` instead of returning stub success.

- [ ] **Step 3: Inspect the implementation surface**

Run:

```bash
sed -n '1542,1592p' src/user_level/services/fxfs.rs
sed -n '6338,6358p' src/syscall/syscall.rs
```

Expected: `unlink_file` removes the selected dirent before returning the object id; `sys_unlinkat` releases the object only when no Linux open description still references it.

### Task 3: Refresh And Verify The POSIX Stage

**Files:**
- Regenerate: `host_shared/posixtest/`
- Verify: `third_party/posixtest/source.lock.json`
- Verify: `third_party/posixtest/patches/series`

- [ ] **Step 1: Confirm stale stage metadata is rejected or absent**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
```

Expected before regeneration: either success if another worker already refreshed the stage, or failure with `manifest metadata does not match current build inputs`. A stale failure is acceptable RED evidence.

- [ ] **Step 2: Rebuild the AArch64 POSIX stage**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
```

Expected: build summary reports the reviewed runnable inventory and writes `manifest.json`, `manifest.tsv`, `build-results.ndjson`, staged binaries, staged libraries, and runtime snapshot files under `host_shared/posixtest/`.

- [ ] **Step 3: Verify the rebuilt stage**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
```

Expected: success. The verifier must compare manifest metadata, test inventory, binary hashes, runtime snapshot, and build results against current pinned inputs.

- [ ] **Step 4: Check manifest provenance**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import subprocess
from pathlib import Path

head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
manifest = json.loads(Path("host_shared/posixtest/manifest.json").read_text())
tsv = Path("host_shared/posixtest/manifest.tsv").read_text().splitlines()
json_commit = manifest["metadata"]["smros_commit"]
tsv_commit = next(line.split("\t", 2)[2] for line in tsv if line.startswith("meta\tsmros_commit\t"))
print(f"HEAD={head}")
print(f"manifest.json smros_commit={json_commit}")
print(f"manifest.tsv smros_commit={tsv_commit}")
if json_commit != head or tsv_commit != head:
    raise SystemExit("manifest provenance mismatch")
PY
```

Expected: both manifests identify the current `HEAD`.

### Task 4: Build Warning-Denied AArch64 Kernel

**Files:**
- Regenerate: `kernel8.img`
- Verify: `target/aarch64-unknown-none/release/smros`

- [ ] **Step 1: Build with warnings denied**

Run:

```bash
make aarch64-warning-check
```

Expected: optimized AArch64 kernel build succeeds, emits no warnings, validates the link layout, and writes `kernel8.img`.

### Task 5: Run Same-Disk AIO Regression Campaigns

**Files:**
- Generate: `target/posix/aarch64/aio-stale-file-same-disk.img`
- Generate: `target/posix/aarch64/aio-stale-file-same-disk-first/`
- Generate: `target/posix/aarch64/aio-stale-file-same-disk-second/`
- Read only: `host_shared/posixtest/`
- Read only: `kernel8.img`

- [ ] **Step 1: Create a fresh private disk**

Run:

```bash
mkdir -p target/posix/aarch64
qemu-img create -f raw target/posix/aarch64/aio-stale-file-same-disk.img 128M
```

Expected: a new private raw disk exists below `target/posix/aarch64/`.

- [ ] **Step 2: Run complete `aio_cancel` once on the private disk**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/aio-stale-file-same-disk-first"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/aio-stale-file-same-disk.img"),
    memory="1024M",
    api="aio_cancel",
)
print(f"attempts={len(result.attempts)} restarts={result.restart_count} results={result.result_path}")
if result.restart_count != 0:
    raise SystemExit("first same-disk campaign restarted QEMU")
PY
```

Expected: all selected `aio_cancel` rows reach structured terminal results with no QEMU restart.

- [ ] **Step 3: Run complete `aio_cancel` again on the same private disk**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/aio-stale-file-same-disk-second"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/aio-stale-file-same-disk.img"),
    memory="1024M",
    api="aio_cancel",
)
print(f"attempts={len(result.attempts)} restarts={result.restart_count} results={result.result_path}")
if result.restart_count != 0:
    raise SystemExit("second same-disk campaign restarted QEMU")
PY
```

Expected: the second run sees the persisted namespace from the first run and still completes without stale `O_EXCL` setup failures.

- [ ] **Step 4: Validate both same-disk logs and results**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
from pathlib import Path

roots = [
    Path("target/posix/aarch64/aio-stale-file-same-disk-first"),
    Path("target/posix/aarch64/aio-stale-file-same-disk-second"),
]
required_pass = {
    "conformance/interfaces/aio_cancel/1-1.c",
    "conformance/interfaces/aio_cancel/2-1.c",
    "conformance/interfaces/aio_cancel/2-2.c",
    "conformance/interfaces/aio_cancel/4-1.c",
    "conformance/interfaces/aio_cancel/5-1.c",
    "conformance/interfaces/aio_cancel/6-1.c",
}
for root in roots:
    serial = (root / "qemu-serial.log").read_text(errors="replace")
    if "Error at open(): File exists" in serial:
        raise SystemExit(f"{root}: stale File exists setup error")
    for forbidden in ("posixtest: timeout", "infrastructure_error", "panic", "fatal"):
        if forbidden in serial.lower():
            raise SystemExit(f"{root}: forbidden marker {forbidden}")
    attempts = [json.loads(line) for line in (root / "results.ndjson").read_text().splitlines() if line.strip()]
    by_id = {attempt["test_id"]: attempt for attempt in attempts}
    missing = required_pass.difference(by_id)
    if missing:
        raise SystemExit(f"{root}: missing required tests {sorted(missing)}")
    not_pass = {test_id: by_id[test_id]["status"] for test_id in required_pass if by_id[test_id]["status"] != "pass"}
    if not_pass:
        raise SystemExit(f"{root}: required stale-file canaries did not pass {not_pass}")
    leaking = [
        attempt["test_id"]
        for attempt in attempts
        for value in attempt.get("resource_deltas", {}).values()
        if isinstance(value, int) and value > 0
    ]
    if leaking:
        raise SystemExit(f"{root}: positive resource deltas {leaking[:5]}")
    counts = {}
    for attempt in attempts:
        counts[attempt["status"]] = counts.get(attempt["status"], 0) + 1
    print(f"{root}: attempts={len(attempts)} counts={counts}")
PY
```

Expected: each run prints the exact attempt count and status counts, with the stale-file canaries passing and no stale `File exists` setup error.

### Task 6: Run Affected-Disk Recovery Campaign

**Files:**
- Generate: `target/posix/aarch64/aio-stale-file-affected-copy.img`
- Generate: `target/posix/aarch64/aio-stale-file-affected-copy/`
- Read only: `target/posix/aarch64/aio-eexist-current-disk.img` if present

- [ ] **Step 1: Copy the previously affected disk when available**

Run:

```bash
if [ -f target/posix/aarch64/aio-eexist-current-disk.img ]; then
  cp target/posix/aarch64/aio-eexist-current-disk.img target/posix/aarch64/aio-stale-file-affected-copy.img
else
  qemu-img create -f raw target/posix/aarch64/aio-stale-file-affected-copy.img 128M
fi
```

Expected: the source disk is never modified; a private disposable recovery disk is available.

- [ ] **Step 2: Run complete `aio_cancel` on the private recovery disk**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/aio-stale-file-affected-copy"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/aio-stale-file-affected-copy.img"),
    memory="1024M",
    api="aio_cancel",
)
print(f"attempts={len(result.attempts)} restarts={result.restart_count} results={result.result_path}")
if result.restart_count != 0:
    raise SystemExit("affected-copy campaign restarted QEMU")
PY
```

Expected: complete `aio_cancel` run reaches terminal results.

- [ ] **Step 3: Validate the affected-copy recovery log**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
from pathlib import Path

root = Path("target/posix/aarch64/aio-stale-file-affected-copy")
serial = (root / "qemu-serial.log").read_text(errors="replace")
if "Error at open(): File exists" in serial:
    raise SystemExit("affected-copy stale File exists setup error")
attempts = [json.loads(line) for line in (root / "results.ndjson").read_text().splitlines() if line.strip()]
counts = {}
for attempt in attempts:
    counts[attempt["status"]] = counts.get(attempt["status"], 0) + 1
    if attempt.get("timed_out"):
        raise SystemExit(f"timeout in {attempt['test_id']}")
    for value in attempt.get("resource_deltas", {}).values():
        if isinstance(value, int) and value > 0:
            raise SystemExit(f"positive resource delta in {attempt['test_id']}")
print(f"affected-copy: attempts={len(attempts)} counts={counts}")
PY
```

Expected: no stale `File exists` setup error and no runtime infrastructure regression.

### Task 7: Run Repository Gates And Quality Evidence

**Files:**
- Generate: `target/posix/aarch64/aio-stale-file-quality/`
- Generate: `target/posix/aarch64/aio-stale-file-quality.json`
- Generate: `target/posix/aarch64/report-aio-stale-file/`

- [ ] **Step 1: Run core repository gates with logs**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import re
import subprocess
import time
from pathlib import Path

root = Path("target/posix/aarch64/aio-stale-file-quality")
root.mkdir(parents=True, exist_ok=True)
commands = [
    ("cargo fmt --check", ["cargo", "fmt", "--check"]),
    ("make host-fmt-check", ["make", "host-fmt-check"]),
    ("make script-check", ["make", "script-check"]),
    ("make launcher-test", ["make", "launcher-test"]),
    ("make linker-layout-test", ["make", "linker-layout-test"]),
    ("make ut", ["make", "ut"]),
    ("make it", ["make", "it"]),
    ("make posix-tool-test", ["make", "posix-tool-test"]),
    ("make verus", ["make", "verus"]),
    ("make aarch64-warning-check", ["make", "aarch64-warning-check"]),
]
records = []
for label, argv in commands:
    slug = re.sub(r"[^a-z0-9]+", "-", label.lower()).strip("-")
    log = root / f"gate-{slug}.log"
    start = time.monotonic()
    with log.open("wb") as output:
        completed = subprocess.run(argv, stdout=output, stderr=subprocess.STDOUT)
    records.append({
        "command": label,
        "duration_seconds": round(time.monotonic() - start, 3),
        "log": str(log),
        "returncode": completed.returncode,
        "status": "passed" if completed.returncode == 0 else "failed",
    })
    print(f"{label}: {records[-1]['status']} ({completed.returncode}) log={log}")
summary = {"commands": records, "schema": 1}
(root / "gates.json").write_text(
    json.dumps(summary, sort_keys=True, separators=(",", ":")) + "\n"
)
failed = [record["command"] for record in records if record["returncode"] != 0]
if failed:
    raise SystemExit("gate failures: " + ", ".join(failed))
PY
```

Expected: every command exits zero. If a gate is unavailable because a required local tool is absent, record the exact command and missing tool in the results document instead of marking it as passed.

- [ ] **Step 2: Capture coverage and Coverity availability**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import shutil
import subprocess
from pathlib import Path

root = Path("target/posix/aarch64/aio-stale-file-quality")
root.mkdir(parents=True, exist_ok=True)
checks = []

coverage_log = root / "coverage-host.log"
with coverage_log.open("wb") as output:
    coverage = subprocess.run(["make", "coverage-host"], stdout=output, stderr=subprocess.STDOUT)
coverage_artifact = Path("target/coverage/host/tarpaulin-report.html")
if coverage.returncode == 0 and coverage_artifact.is_file():
    checks.append({
        "artifact": str(coverage_artifact),
        "command": "make coverage-host",
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "passed",
        "summary": "Host Rust coverage completed; see artifact for exact line and branch metrics",
        "version": None,
    })
else:
    checks.append({
        "artifact": str(coverage_log),
        "command": "make coverage-host",
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "unavailable" if coverage.returncode == 127 else "failed",
        "summary": f"Host coverage exited with status {coverage.returncode}; see log",
        "version": None,
    })

coverity_names = ("cov-build", "cov-analyze", "cov-format-errors")
coverity_tools = [shutil.which(name) for name in coverity_names]
coverity_log = root / "coverity.log"
coverity_json = root / "coverity-results.json"
if all(coverity_tools):
    capture = root / "coverity-capture"
    commands = [
        [coverity_tools[0], "--dir", str(capture), "make", "aarch64-warning-check"],
        [coverity_tools[1], "--dir", str(capture), "--all"],
        [coverity_tools[2], "--dir", str(capture), "--json-output-v7", str(coverity_json)],
    ]
    with coverity_log.open("wb") as output:
        completed = [subprocess.run(command, stdout=output, stderr=subprocess.STDOUT) for command in commands]
    if all(item.returncode == 0 for item in completed) and coverity_json.is_file():
        payload = json.loads(coverity_json.read_text())
        issues = payload.get("issues", [])
        version = subprocess.check_output([coverity_tools[0], "--version"], text=True, stderr=subprocess.STDOUT).strip()
        checks.append({
            "artifact": str(coverity_json),
            "command": " && ".join(" ".join(command) for command in commands),
            "coverage_percent": None,
            "findings": len(issues),
            "kind": "static-analysis",
            "name": "coverity",
            "status": "passed" if len(issues) == 0 else "failed",
            "summary": f"Coverity reported {len(issues)} issues",
            "version": version,
        })
    else:
        checks.append({
            "artifact": str(coverity_log),
            "command": " && ".join(" ".join(command) for command in commands),
            "coverage_percent": None,
            "findings": None,
            "kind": "static-analysis",
            "name": "coverity",
            "status": "failed",
            "summary": "Coverity command failed; see log",
            "version": subprocess.check_output([coverity_tools[0], "--version"], text=True, stderr=subprocess.STDOUT).strip(),
        })
else:
    missing = [name for name, tool in zip(coverity_names, coverity_tools) if tool is None]
    checks.append({
        "artifact": None,
        "command": "cov-build --dir target/posix/aarch64/aio-stale-file-quality/coverity-capture make aarch64-warning-check && cov-analyze --dir target/posix/aarch64/aio-stale-file-quality/coverity-capture --all && cov-format-errors --dir target/posix/aarch64/aio-stale-file-quality/coverity-capture --json-output-v7 target/posix/aarch64/aio-stale-file-quality/coverity-results.json",
        "coverage_percent": None,
        "findings": None,
        "kind": "static-analysis",
        "name": "coverity",
        "status": "unavailable",
        "summary": "Missing Coverity tools: " + ", ".join(missing),
        "version": None,
    })

commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
evidence = {
    "architecture": "aarch64",
    "checks": checks,
    "schema": 1,
    "smros_commit": commit,
}
Path("target/posix/aarch64/aio-stale-file-quality.json").write_text(
    json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n"
)
print(json.dumps(evidence, indent=2, sort_keys=True))
PY
```

Expected: quality evidence JSON exists. Coverity unavailability is reported honestly if Coverity is not installed.

- [ ] **Step 3: Render the filtered POSIX report**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --smros-results target/posix/aarch64/aio-stale-file-same-disk-second/results.ndjson \
  --quality-evidence target/posix/aarch64/aio-stale-file-quality.json \
  --out target/posix/aarch64/report-aio-stale-file
```

Expected: report artifacts render atomically and include coverage, API/group metrics, non-pass rows, resource deltas, and quality evidence.

### Task 8: Record Final Evidence

**Files:**
- Generate: `target/posix/aarch64/aio-stale-file-results.md`

- [ ] **Step 1: Generate the evidence summary from artifacts**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import subprocess
from pathlib import Path

ROOT = Path("target/posix/aarch64")
campaigns = [
    ("same-disk first", ROOT / "aio-stale-file-same-disk-first", ROOT / "aio-stale-file-same-disk.img"),
    ("same-disk second", ROOT / "aio-stale-file-same-disk-second", ROOT / "aio-stale-file-same-disk.img"),
    ("affected-copy", ROOT / "aio-stale-file-affected-copy", ROOT / "aio-stale-file-affected-copy.img"),
]
required_pass = {
    "conformance/interfaces/aio_cancel/1-1.c",
    "conformance/interfaces/aio_cancel/2-1.c",
    "conformance/interfaces/aio_cancel/2-2.c",
    "conformance/interfaces/aio_cancel/4-1.c",
    "conformance/interfaces/aio_cancel/5-1.c",
    "conformance/interfaces/aio_cancel/6-1.c",
}

def load_attempts(root: Path) -> list[dict]:
    return [json.loads(line) for line in (root / "results.ndjson").read_text().splitlines() if line.strip()]

def status_counts(attempts: list[dict]) -> str:
    counts = {}
    for attempt in attempts:
        counts[attempt["status"]] = counts.get(attempt["status"], 0) + 1
    return ", ".join(f"{key}={counts[key]}" for key in sorted(counts))

def positive_resource_delta_count(attempts: list[dict]) -> int:
    return sum(
        1
        for attempt in attempts
        if any(
            isinstance(value, int) and value > 0
            for value in attempt.get("resource_deltas", {}).values()
        )
    )

def timeout_count(attempts: list[dict]) -> int:
    return sum(1 for attempt in attempts if attempt.get("timed_out"))

def serial_text(root: Path) -> str:
    return (root / "qemu-serial.log").read_text(errors="replace")

def restart_count(serial: str) -> int:
    return max(0, serial.count('"event":"suite_start"') - 1)

stage_verify = subprocess.run(
    [
        "python3",
        "-m",
        "scripts.posix.cli",
        "build",
        "--arch",
        "aarch64",
        "--stage",
        "host_shared/posixtest",
        "--verify-only",
    ],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
)
stage_verify_status = f"passed ({stage_verify.stdout.strip()})" if stage_verify.returncode == 0 else f"failed rc={stage_verify.returncode}: {stage_verify.stdout.strip()}"
if stage_verify.returncode != 0:
    raise SystemExit(stage_verify_status)

head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
metadata = json.loads(Path("host_shared/posixtest/manifest.json").read_text())["metadata"]
quality = json.loads((ROOT / "aio-stale-file-quality.json").read_text())
gates = json.loads((ROOT / "aio-stale-file-quality" / "gates.json").read_text())

summaries = []
all_attempts = []
for name, root, disk in campaigns:
    attempts = load_attempts(root)
    all_attempts.extend(attempts)
    serial = serial_text(root)
    stale_errors = serial.count("Error at open(): File exists")
    timeouts = timeout_count(attempts)
    positive_deltas = positive_resource_delta_count(attempts)
    restarts = restart_count(serial)
    if stale_errors or timeouts or positive_deltas or restarts:
        raise SystemExit(
            f"{name}: stale_errors={stale_errors} timeouts={timeouts} positive_deltas={positive_deltas} restarts={restarts}"
        )
    summaries.append({
        "attempts": len(attempts),
        "counts": status_counts(attempts),
        "disk": str(disk),
        "name": name,
        "positive_deltas": positive_deltas,
        "restarts": restarts,
        "stale_errors": stale_errors,
        "timeouts": timeouts,
    })

second_attempts = load_attempts(ROOT / "aio-stale-file-same-disk-second")
second_by_id = {attempt["test_id"]: attempt for attempt in second_attempts}
bad_canaries = {
    test_id: second_by_id.get(test_id, {}).get("status", "missing")
    for test_id in required_pass
    if second_by_id.get(test_id, {}).get("status") != "pass"
}
if bad_canaries:
    raise SystemExit("required stale-file canaries failed: " + json.dumps(bad_canaries, sort_keys=True))

manifest_values = sorted({attempt["manifest_sha256"] for attempt in all_attempts})
build_values = sorted({attempt["build_results_sha256"] for attempt in all_attempts})
commit_values = sorted({attempt["smros_commit"] for attempt in all_attempts})
if len(manifest_values) != 1 or len(build_values) != 1 or commit_values != [head]:
    raise SystemExit("runtime provenance mismatch")

gate_status = "passed" if all(record["returncode"] == 0 for record in gates["commands"]) else "failed"
kernel_gate = next(record for record in gates["commands"] if record["command"] == "make aarch64-warning-check")

lines = [
    "# AArch64 POSIX AIO Stale-File Recovery Results",
    "",
    "## Scope",
    "",
    "This generated evidence verifies that repeated `aio_cancel` campaigns no longer fail setup with `Error at open(): File exists` on a persistent FxFS disk. It does not claim complete AIO conformance.",
    "",
    "## Provenance",
    "",
    "| Field | Value |",
    "| --- | --- |",
    f"| SMROS commit | `{head}` |",
    f"| Manifest metadata SMROS commit | `{metadata['smros_commit']}` |",
    f"| POSIX manifest SHA-256 | `{manifest_values[0]}` |",
    f"| POSIX build-results SHA-256 | `{build_values[0]}` |",
    f"| POSIX stage verify-only | `{stage_verify_status}` |",
    f"| Kernel build | `{kernel_gate['status']} rc={kernel_gate['returncode']} log={kernel_gate['log']}` |",
    "",
    "## Runtime Evidence",
    "",
    "| Campaign | Disk | Attempts | Status counts | Restarts | Stale `File exists` | Timeouts | Positive resource deltas |",
    "| --- | --- | ---: | --- | ---: | ---: | ---: | ---: |",
]
for item in summaries:
    lines.append(
        f"| {item['name']} | `{item['disk']}` | {item['attempts']} | `{item['counts']}` | {item['restarts']} | {item['stale_errors']} | {item['timeouts']} | {item['positive_deltas']} |"
    )
lines.extend([
    "",
    "## Required Stale-File Canaries",
    "",
    "The following tests passed in the same-disk second campaign:",
    "",
])
for test_id in sorted(required_pass):
    lines.append(f"- `{test_id}`")
lines.extend([
    "",
    "## Quality Evidence",
    "",
    "| Check | Status | Artifact | Notes |",
    "| --- | --- | --- | --- |",
    f"| repository gates | `{gate_status}` | `target/posix/aarch64/aio-stale-file-quality/gates.json` | `{len(gates['commands'])} commands recorded` |",
])
for check in quality["checks"]:
    artifact = check["artifact"] if check["artifact"] is not None else "none"
    lines.append(
        f"| {check['name']} | `{check['status']}` | `{artifact}` | `{check['summary']}` |"
    )
lines.extend([
    "",
    "## Conclusion",
    "",
    "The stale-file setup failure class is closed for the tested AArch64 `aio_cancel` boundary when the refreshed stage and kernel at the recorded SMROS commit are used. Unrelated POSIX AIO assertion outcomes are preserved as genuine test results.",
    "",
])
(ROOT / "aio-stale-file-results.md").write_text("\n".join(lines))
print(ROOT / "aio-stale-file-results.md")
PY
```

Expected: `target/posix/aarch64/aio-stale-file-results.md` is generated from exact observed artifacts and contains no fabricated coverage or Coverity success.

- [ ] **Step 2: Final clean check**

Run:

```bash
git status --short --branch
git log --oneline -n 5
git diff --stat HEAD
```

Expected: tracked tree has no unintended source changes after the plan commit. Ignored generated runtime evidence may remain under `target/`, `host_shared/posixtest/`, and `kernel8.img`.
