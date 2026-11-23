#!/usr/bin/env python3
"""Build verification/logs/campaign-summary.txt from a hermes serial log.

Every count line is kept verbatim, including failures. The posixtest 1600
line and `SECTEST SUMMARY pass=91 fail=0 xfail=0 xpass=0` are required
exactly. A missing final, a failing count, a PASS/FAIL contradiction, or
notify-gate lates above 1 makes the process exit 1. The header is taken from
the serial log itself plus the sha256 of the file bytes, including when the
file is gzip. Running the script again on the same file is byte-identical.
"""

import gzip
import hashlib
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
COUNT_PREFIXES = (
    "NOTIFY-GATE SUMMARY ",
    "NOTIFY-GATE CAMPAIGN ",
    "posixtest: ",
    "SECTEST SUMMARY ",
    "Hermes phase ",
    "Hermes test-all summary ",
    "Hermes test-all result:",
    "Host stage ",
    "CASE notify-gate-sleep ",
)
POSIX_FINAL = (
    "posixtest: progress tests=1600/1600 (100.00%) "
    "apis-complete=185/185 (100.00%) apis-pass=185/185 (100.00%) "
    "groups-complete=9/9 (100.00%) groups-pass=9/9 (100.00%) "
    "pass=1600 fail=0 unresolved=0 unsupported=0 untested=0 "
    "launch-errors=0 timeout=0 scope=selected"
)
SECTEST_FINAL = "SECTEST SUMMARY pass=91 fail=0 xfail=0 xpass=0"
# One late sample may be retried. A larger total fails the campaign.
NOTIFY_GATE_LATE_MAX = 1


# A later boot may keep an older POSIX stage only when the commits differ by
# logs, docs, or the baseline host runners. The QEMU boot scripts are not in
# this set: a lag that touches them is rejected unless the preamble records
# their blob sha256 at git_head.
_LAG_ALLOW_EXACT = {
    "scripts/posix/baseline.py",
    "scripts/posix/cli.py",
}
_LAG_ALLOW_PREFIX = (
    "verification/logs/",
    "docs/",
)
_BOOT_SCRIPTS = (
    "scripts/smros-vm-launcher.py",
    "scripts/run-hermes-campaign.sh",
    "scripts/measure-notify-gate-sleep.sh",
)


def _git_ok(args):
    try:
        completed = subprocess.run(
            ["git", *args],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if completed.returncode != 0:
        return None
    return completed.stdout


def _boot_script_sha_ok(text, git_head, path):
    """Preamble hash must be the blob of this path at git_head."""
    head = git_head[: -len("-dirty")] if git_head.endswith("-dirty") else git_head
    if not head or head == "missing":
        return False
    try:
        completed = subprocess.run(
            ["git", "show", f"{head}:{path}"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    if completed.returncode != 0:
        return False
    digest = hashlib.sha256(completed.stdout).hexdigest()
    return re.search(rf"^script_sha256 {re.escape(path)}={digest}$", text, re.M) is not None


def _paths_allow_lag(paths, text, git_head):
    """True when every lagged path is logs, docs, baseline/cli, or a re-run boot script."""
    for path in paths:
        if not path:
            continue
        if path.startswith(_LAG_ALLOW_PREFIX) or path in _LAG_ALLOW_EXACT:
            continue
        if path in _BOOT_SCRIPTS and _boot_script_sha_ok(text, git_head, path):
            continue
        return False
    return True


def _event_commit_allowed(event_sha, git_head, text):
    """Prefer an equal commit. Other ancestors must stay inside the lag whitelist."""
    head = git_head[: -len("-dirty")] if git_head.endswith("-dirty") else git_head
    if not head or head == "missing":
        return False
    event_full = _git_ok(["rev-parse", "--verify", event_sha + "^{commit}"])
    head_full = _git_ok(["rev-parse", "--verify", head + "^{commit}"])
    if not event_full or not head_full:
        return False
    if event_full.strip() == head_full.strip():
        return True
    if _git_ok(["merge-base", "--is-ancestor", event_sha, head]) is None:
        return False
    names = _git_ok(["diff", "--name-only", event_sha, head])
    if names is None:
        return False
    return _paths_allow_lag(names.splitlines(), text, head)


def _published_results_sha256():
    """Raw ndjson hash, and the hash recorded in cases.txt when that file exists."""
    found = []
    archive = ROOT / "verification/logs/posix-baseline-results.ndjson.gz"
    if archive.is_file():
        found.append(hashlib.sha256(gzip.decompress(archive.read_bytes())).hexdigest())
    cases = ROOT / "verification/logs/posix-baseline-cases.txt"
    if cases.is_file():
        match = re.search(r"^results_sha256=([0-9a-f]{64})$", cases.read_text(), re.M)
        if match:
            found.append(match.group(1))
    return found


def _meta(text, key):
    match = re.search(rf"^{key}=(.*)$", text, re.M)
    return match.group(1) if match else "missing"


FAIL_VALUE = re.compile(r"\bfail=(\d+)")
LATE_VALUE = re.compile(r"\blate=(\d+)")


def _fail_values(line):
    """Word-bounded fail= counts. `xfail=0` does not match."""
    return [int(match.group(1)) for match in FAIL_VALUE.finditer(line)]


def _late_total(lines):
    total = 0
    for line in lines:
        if not line.startswith("NOTIFY-GATE SUMMARY "):
            continue
        for match in LATE_VALUE.finditer(line):
            total += int(match.group(1))
    return total


def _campaign_late(lines):
    for line in lines:
        if not line.startswith("NOTIFY-GATE CAMPAIGN "):
            continue
        match = LATE_VALUE.search(line)
        if match:
            return int(match.group(1))
    return None


def _posix_event_problems(text, diff_paths=None):
    shas = sorted(set(re.findall(r'"smros_commit":"([0-9a-f]{40})"', text)))
    if diff_paths is not None:
        if not shas or not _paths_allow_lag(diff_paths, text, _meta(text, "git_head")):
            return ["posix-event-commit"]
        return []
    problems = []
    for sha in shas:
        if not _event_commit_allowed(sha, _meta(text, "git_head"), text):
            problems.append("posix-event-commit")
            break
    return problems


def _problems(kept, text, diff_paths=None):
    """Missing finals, a failing count, or a contradiction. Empty means pass."""
    problems = []
    if POSIX_FINAL not in kept:
        problems.append("missing-posixtest-final")
    if SECTEST_FINAL not in kept:
        problems.append("missing-sectest-91")
    if not any(line.startswith("NOTIFY-GATE SUMMARY ") for line in kept):
        problems.append("missing-notify-gate")
    summaries = [line for line in kept if line.startswith("Hermes test-all summary ")]
    results = [line for line in kept if line.startswith("Hermes test-all result:")]
    if not summaries:
        problems.append("missing-hermes-summary")
    if not results:
        problems.append("missing-hermes-result")
    if any("overall=FAIL" in line for line in summaries):
        problems.append("overall-fail")
    if any(line.endswith("FAIL") for line in results):
        problems.append("result-fail")
    if any("overall=PASS" in line for line in summaries) and any(
        line.endswith("FAIL") for line in results
    ):
        problems.append("contradiction")
    if any("overall=FAIL" in line for line in summaries) and any(
        line.endswith("PASS") for line in results
    ):
        problems.append("contradiction")
    for line in kept:
        if line.startswith("SECTEST SUMMARY ") and any(value != 0 for value in _fail_values(line)):
            problems.append("sectest-fail")
            break
    for line in kept:
        # posix-baseline's fail count is the qemu-user reference miss total.
        # The campaign stays PASS when that stage publishes every selected test.
        if line.startswith("Hermes phase posix-baseline "):
            continue
        if line.startswith("Hermes phase ") and any(value != 0 for value in _fail_values(line)):
            problems.append("phase-fail")
            break
    for line in summaries:
        if "overall=PASS" not in line:
            continue
        if "posixtest=1600/1600" not in line or "sectest=91/0/0/0" not in line or any(
            value != 0 for value in _fail_values(line)
        ):
            problems.append("summary-mismatch")
            break
    if any("overall=PASS" in line for line in summaries):
        phase_ok = False
        for line in kept:
            if not line.startswith("Hermes phase sectest "):
                continue
            if "pass=91" in line and re.search(r"\bfail=0\b", line):
                phase_ok = True
        if SECTEST_FINAL not in kept or not phase_ok:
            problems.append("sectest-mismatch")
    claimed_hashes = []
    for line in kept:
        if not line.startswith("Host stage "):
            continue
        match = re.search(r"(?:^|_)results_sha256=([0-9a-f]{64})(?:_|$)", line)
        run_id = re.search(r"(?:^|_)run_id=[0-9a-f]{16,64}(?:_|$)", line)
        if match and run_id:
            claimed_hashes.append(match.group(1))
    if not claimed_hashes:
        problems.append("missing-baseline-identity")
    published = _published_results_sha256()
    if claimed_hashes and published and (
        len(set(published)) != 1 or any(value != published[0] for value in claimed_hashes)
    ):
        problems.append("baseline-sha-mismatch")
    problems.extend(_posix_event_problems(text, diff_paths))
    if _late_total(kept) > NOTIFY_GATE_LATE_MAX:
        problems.append("notify-gate-late")
    campaign_late = _campaign_late(kept)
    if campaign_late is not None and campaign_late > NOTIFY_GATE_LATE_MAX:
        problems.append("notify-gate-campaign-late")
    commit = _meta(text, "smros_commit")
    head = _meta(text, "git_head")
    commits = re.findall(r"^smros_commit=(.*)$", text, re.M)
    if commit == "missing" or head == "missing":
        problems.append("missing-commit")
    elif any(item != head for item in commits):
        problems.append("commit-mismatch")
    if commit.endswith("-dirty") or head.endswith("-dirty"):
        problems.append("commit-dirty")
    return problems


def summarize(serial_text, serial_bytes, diff_paths=None):
    text = serial_text.replace("\r", "")
    kept = []
    for line in text.splitlines():
        if line.startswith(COUNT_PREFIXES):
            kept.append(line)
    problems = _problems(kept, text, diff_paths)
    if "missing-posixtest-final" in problems:
        kept.append("posixtest final missing")
    if "missing-sectest-91" in problems:
        kept.append(SECTEST_FINAL + " missing")
    if "missing-notify-gate" in problems:
        kept.append("NOTIFY-GATE SUMMARY missing")
    if "missing-hermes-summary" in problems:
        kept.append("Hermes test-all summary missing")
    if "missing-hermes-result" in problems:
        kept.append("Hermes test-all result: missing")
    header = [
        f"utc={_meta(text, 'utc')}",
        f"git_head={_meta(text, 'git_head')}",
        f"git_dirty={_meta(text, 'git_dirty')}",
        "serial_sha256=" + hashlib.sha256(serial_bytes).hexdigest(),
    ]
    return "\n".join(header + kept) + "\n", problems


def load_serial_bytes(raw, gzipped):
    if gzipped:
        text = gzip.decompress(raw).decode("utf-8", "replace")
    else:
        text = raw.decode("utf-8", "replace")
    return text, raw


def load_serial(path):
    raw = path.read_bytes()
    return load_serial_bytes(raw, path.suffix == ".gz" or raw.startswith(b"\x1f\x8b"))


def _passing_serial():
    return (
        "utc=2026-01-01T00:00:00Z\n"
        "git_head=abc\n"
        "git_dirty=false\n"
        "smros_commit=abc\n"
        f"{POSIX_FINAL}\n"
        "NOTIFY-GATE SUMMARY attempts=1 late=0 values=1\n"
        "NOTIFY-GATE CAMPAIGN late=0 threshold=1\n"
        f"{SECTEST_FINAL}\n"
        "Hermes phase guest phase=done pass=7 fail=0 unavailable=0\n"
        "Hermes phase posixtest phase=done selected=1600 pass=1600 fail=0 unresolved=0 timeout=0 launch_errors=0\n"
        "Hermes phase sectest phase=done pass=91 fail=0 xfail=0 xpass=0 ids=91/91 undef_pass=1 undef_fail=0 one_pass=1 one_fail=0\n"
        "Hermes phase posix-baseline phase=done pass=1392 fail=208 unavailable=0\n"
        "Host stage PASS reference_selected=1600_reference_passed=1392_reference_nonpass=208_run_id=0123456789abcdef_results_sha256="
        + (
            (_published_results_sha256() or ["0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"])[0]
        )
        + "\n"
        "Hermes test-all summary posixtest=1600/1600 fail=0 sectest=91/0/0/0 overall=PASS\n"
        "Hermes test-all result: PASS\n"
    )


def main():
    if "--self-test" in sys.argv:
        injected = (
            "utc=2026-01-01T00:00:00Z\n"
            "git_head=abc\n"
            "git_dirty=false\n"
            "posixtest: progress tests=3/1600 pass=3 fail=0 unresolved=0 timeout=0\n"
            "NOTIFY-GATE SUMMARY attempts=1 late=0 values=1\n"
            "SECTEST SUMMARY pass=1 fail=0 xfail=0 xpass=0\n"
            "SECTEST SUMMARY pass=90 fail=1 xfail=0 xpass=0\n"
            "Hermes test-all summary overall=FAIL\n"
            "Hermes test-all result: FAIL\n"
        )
        raw = injected.encode()
        text, problems = summarize(injected, raw)
        if "fail=1" not in text or "pass=90" not in text or "overall=FAIL" not in text:
            print("injected failure count was dropped", file=sys.stderr)
            return 1
        if "posixtest final missing" not in text or "SECTEST SUMMARY pass=91" not in text:
            print("deleted final lines were not reported missing", file=sys.stderr)
            return 1
        if not problems:
            print("failure and missing finals still exit 0", file=sys.stderr)
            return 1
        if "serial_sha256=" + hashlib.sha256(raw).hexdigest() not in text:
            print("serial sha256 mismatch", file=sys.stderr)
            return 1
        if summarize(injected, raw) != (text, problems):
            print("summarize is not deterministic", file=sys.stderr)
            return 1
        packed = gzip.compress(raw)
        gz_text, gz_raw = load_serial_bytes(packed, gzipped=True)
        gz_summary, gz_problems = summarize(gz_text, gz_raw)
        if "fail=1" not in gz_summary or not gz_problems:
            print("gzipped failure count was dropped", file=sys.stderr)
            return 1
        ok_text, ok_problems = summarize(_passing_serial(), _passing_serial().encode())
        if ok_problems or "missing" in ok_text:
            print(f"passing serial was rejected: {ok_problems}", file=sys.stderr)
            return 1
        contradicted = _passing_serial().replace(
            "Hermes test-all result: PASS", "Hermes test-all result: FAIL"
        )
        _text, contra = summarize(contradicted, contradicted.encode())
        if "contradiction" not in contra:
            print("PASS/FAIL contradiction was accepted", file=sys.stderr)
            return 1
        one_late = _passing_serial().replace("late=0", "late=1", 1)
        _text, one_problems = summarize(one_late, one_late.encode())
        if one_problems:
            print(f"a single late was rejected: {one_problems}", file=sys.stderr)
            return 1
        late = _passing_serial() + "NOTIFY-GATE SUMMARY attempts=2 late=2 values=2\n"
        _text, late_problems = summarize(late, late.encode())
        if "notify-gate-late" not in late_problems:
            print("cumulative late above the threshold was accepted", file=sys.stderr)
            return 1
        xfail = _passing_serial() + "SECTEST SUMMARY pass=0 fail=1 xfail=0 xpass=0\n"
        _text, xfail_problems = summarize(xfail, xfail.encode())
        if "sectest-fail" not in xfail_problems:
            print("fail=1 xfail=0 was accepted", file=sys.stderr)
            return 1
        checks = (
            (
                _passing_serial() + "Hermes phase guest phase=done pass=6 fail=1 unavailable=0\n",
                "phase-fail",
                "guest phase fail=1",
            ),
            (
                _passing_serial().replace(
                    "Hermes phase posixtest phase=done selected=1600 pass=1600 fail=0 unresolved=0 timeout=0 launch_errors=0",
                    "Hermes phase posixtest phase=done selected=1600 pass=1599 fail=1 unresolved=0 timeout=0 launch_errors=0",
                ),
                "phase-fail",
                "posixtest phase fail=1",
            ),
            (
                _passing_serial().replace("posixtest=1600/1600", "posixtest=1599"),
                "summary-mismatch",
                "posixtest=1599 overall=PASS",
            ),
            (
                _passing_serial().replace("sectest=91/0/0/0", "sectest=90/1/0/0"),
                "summary-mismatch",
                "sectest=90/1",
            ),
            (
                _passing_serial().replace("sectest=91/0/0/0", "sectest=80/0/0/0"),
                "summary-mismatch",
                "sectest=80/0",
            ),
            (
                "\n".join(
                    line
                    for line in _passing_serial().splitlines()
                    if not line.startswith("Host stage ")
                )
                + "\n",
                "missing-baseline-identity",
                "deleted Host stage line",
            ),
            (
                re.sub(
                    r"results_sha256=[0-9a-f]{64}",
                    "results_sha256=" + ("ab" * 32),
                    _passing_serial(),
                ),
                "baseline-sha-mismatch",
                "wrong results_sha256",
            ),
            (
                _passing_serial()
                .replace("git_head=abc", "git_head=479b0b5")
                .replace("smros_commit=abc", "smros_commit=479b0b5")
                + 'SMROS_POSIX_EVENT {"smros_commit":"3bd7e4162ab2481a3674f7844d40364516163e24"}\n',
                "posix-event-commit",
                "3bd7e41 under git_head=479b0b5",
            ),
            (
                _passing_serial().replace(
                    "NOTIFY-GATE CAMPAIGN late=0 threshold=1",
                    "NOTIFY-GATE CAMPAIGN late=5 threshold=1",
                ),
                "notify-gate-campaign-late",
                "CAMPAIGN late=5",
            ),
            (
                _passing_serial().replace("smros_commit=abc", "smros_commit=abc-dirty"),
                "commit-dirty",
                "dirty smros_commit",
            ),
            (
                _passing_serial().replace("smros_commit=abc", "smros_commit=other"),
                "commit-mismatch",
                "smros_commit differs from git_head",
            ),
        )
        for sample, needle, label in checks:
            _text, found = summarize(sample, sample.encode())
            if needle not in found:
                print(f"{label} was accepted: {found}", file=sys.stderr)
                return 1

        def lag_rc(paths, extra="", git_head="head"):
            sample = (
                _passing_serial()
                .replace("git_head=abc", f"git_head={git_head}")
                .replace("smros_commit=abc", f"smros_commit={git_head}")
                + 'SMROS_POSIX_EVENT {"smros_commit":"0123456789abcdef0123456789abcdef01234567"}\n'
                + extra
            )
            _text, found = summarize(sample, sample.encode(), diff_paths=paths)
            return 0 if not found else 1

        launcher_rc = lag_rc(["scripts/smros-vm-launcher.py"])
        campaign_rc = lag_rc(["scripts/run-hermes-campaign.sh"])
        measure_rc = lag_rc(["scripts/measure-notify-gate-sleep.sh"])
        baseline_rc = lag_rc(["scripts/posix/baseline.py"])
        cli_rc = lag_rc(["docs/NOTIFY_GATE_SLEEP.md", "scripts/posix/cli.py"])
        print(f"launcher-only lag rc={launcher_rc}")
        print(f"campaign-script lag rc={campaign_rc}")
        print(f"measure-script lag rc={measure_rc}")
        print(f"baseline-only lag rc={baseline_rc}")
        if launcher_rc != 1 or campaign_rc != 1 or measure_rc != 1 or baseline_rc != 0 or cli_rc != 0:
            print(
                "boot-script lag was allowed or a baseline/docs lag was rejected",
                file=sys.stderr,
            )
            return 1
        head = (_git_ok(["rev-parse", "HEAD"]) or "").strip()
        shown = subprocess.run(
            ["git", "show", f"{head}:scripts/smros-vm-launcher.py"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            timeout=30,
        )
        if shown.returncode != 0:
            print("cannot hash the launcher at HEAD", file=sys.stderr)
            return 1
        digest = hashlib.sha256(shown.stdout).hexdigest()
        rerun_rc = lag_rc(
            ["scripts/smros-vm-launcher.py"],
            extra=f"script_sha256 scripts/smros-vm-launcher.py={digest}\n",
            git_head=head,
        )
        print(f"launcher re-run hash rc={rerun_rc}")
        if rerun_rc != 0:
            print("a launcher lag with the HEAD blob hash was rejected", file=sys.stderr)
            return 1
        print("campaign summary self-test passed")
        return 0
    if len(sys.argv) != 2:
        print("usage: summarize-hermes-campaign.py SERIAL", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    text, raw = load_serial(path)
    out = ROOT / "verification/logs/campaign-summary.txt"
    rendered, problems = summarize(text, raw)
    out.write_text(rendered)
    print(out)
    if problems:
        print("campaign summary rejected: " + ",".join(problems), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
