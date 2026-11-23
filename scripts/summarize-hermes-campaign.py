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


def _meta(text, key):
    match = re.search(rf"^{key}=(.*)$", text, re.M)
    return match.group(1) if match else "missing"


def _late_total(lines):
    total = 0
    for line in lines:
        if not line.startswith("NOTIFY-GATE SUMMARY "):
            continue
        for part in line.split():
            if part.startswith("late="):
                total += int(part.split("=", 1)[1] or "0")
    return total


def _problems(kept):
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
    if any(
        line.startswith("SECTEST SUMMARY ") and "fail=" in line and "fail=0" not in line
        for line in kept
    ):
        problems.append("sectest-fail")
    if _late_total(kept) > NOTIFY_GATE_LATE_MAX:
        problems.append("notify-gate-late")
    return problems


def summarize(serial_text, serial_bytes):
    text = serial_text.replace("\r", "")
    kept = []
    for line in text.splitlines():
        if line.startswith(COUNT_PREFIXES):
            kept.append(line)
    problems = _problems(kept)
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
        f"{POSIX_FINAL}\n"
        "NOTIFY-GATE SUMMARY attempts=1 late=0 values=1\n"
        f"{SECTEST_FINAL}\n"
        "Hermes test-all summary overall=PASS\n"
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
