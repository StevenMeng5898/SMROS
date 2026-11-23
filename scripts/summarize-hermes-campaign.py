#!/usr/bin/env python3
"""Build verification/logs/campaign-summary.txt from a hermes serial log.

Every count line is kept verbatim, including failures. A missing family is
written as `missing`. The header is taken from the serial log itself plus the
sha256 of the file bytes, including when the file is gzip. Running the
script again on the same file is byte-identical.
"""

import gzip
import hashlib
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
COUNT_PREFIXES = (
    "NOTIFY-GATE SUMMARY ",
    "posixtest: ",
    "SECTEST SUMMARY ",
    "Hermes phase ",
    "Hermes test-all summary ",
    "Hermes test-all result:",
    "CASE notify-gate-sleep ",
)
REQUIRED = (
    "NOTIFY-GATE SUMMARY ",
    "posixtest: ",
    "SECTEST SUMMARY ",
    "Hermes test-all summary ",
    "Hermes test-all result:",
)


def _meta(text, key):
    match = re.search(rf"^{key}=(.*)$", text, re.M)
    return match.group(1) if match else "missing"


def summarize(serial_text, serial_bytes):
    text = serial_text.replace("\r", "")
    kept = []
    for line in text.splitlines():
        if line.startswith(COUNT_PREFIXES):
            kept.append(line)
    for prefix in REQUIRED:
        if not any(line.startswith(prefix) for line in kept):
            kept.append(prefix.rstrip() + " missing")
    header = [
        f"utc={_meta(text, 'utc')}",
        f"git_head={_meta(text, 'git_head')}",
        f"git_dirty={_meta(text, 'git_dirty')}",
        "serial_sha256=" + hashlib.sha256(serial_bytes).hexdigest(),
    ]
    return "\n".join(header + kept) + "\n"


def load_serial_bytes(raw, gzipped):
    if gzipped:
        text = gzip.decompress(raw).decode("utf-8", "replace")
    else:
        text = raw.decode("utf-8", "replace")
    return text, raw


def load_serial(path):
    raw = path.read_bytes()
    return load_serial_bytes(raw, path.suffix == ".gz" or raw.startswith(b"\x1f\x8b"))


def main():
    if "--self-test" in sys.argv:
        injected = (
            "utc=2026-01-01T00:00:00Z\n"
            "git_head=abc\n"
            "git_dirty=false\n"
            "posixtest: progress tests=1600/1600 pass=1599 fail=1 unresolved=0 timeout=0\n"
            "NOTIFY-GATE SUMMARY attempts=1 late=0 values=1\n"
            "SECTEST SUMMARY pass=90 fail=1 xfail=0 xpass=0\n"
            "Hermes test-all summary overall=FAIL\n"
            "Hermes test-all result: FAIL\n"
        )
        raw = injected.encode()
        text = summarize(injected, raw)
        if "fail=1" not in text or "pass=90" not in text or "overall=FAIL" not in text:
            print("injected failure count was dropped", file=sys.stderr)
            return 1
        if "serial_sha256=" + hashlib.sha256(raw).hexdigest() not in text:
            print("serial sha256 mismatch", file=sys.stderr)
            return 1
        if summarize(injected, raw) != text:
            print("summarize is not deterministic", file=sys.stderr)
            return 1
        packed = gzip.compress(raw)
        gz_text, gz_raw = load_serial_bytes(packed, gzipped=True)
        gz_summary = summarize(gz_text, gz_raw)
        if "fail=1" not in gz_summary or hashlib.sha256(packed).hexdigest() not in gz_summary:
            print("gzipped failure count was dropped", file=sys.stderr)
            return 1
        print("campaign summary self-test passed")
        return 0
    if len(sys.argv) != 2:
        print("usage: summarize-hermes-campaign.py SERIAL", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    text, raw = load_serial(path)
    out = ROOT / "verification/logs/campaign-summary.txt"
    rendered = summarize(text, raw)
    out.write_text(rendered)
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
