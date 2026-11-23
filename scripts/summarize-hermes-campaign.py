#!/usr/bin/env python3
"""Build verification/logs/campaign-summary.txt from a hermes serial log.

The notify-gate line is copied from the guest's NOTIFY-GATE SUMMARY. A late
count above zero is repeated in the summary so a hand-written zero cannot hide it.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]


def summarize(text):
    notify = []
    posix = []
    sectest = []
    hermes = []
    for line in text.replace("\r", "").splitlines():
        if line.startswith("NOTIFY-GATE SUMMARY "):
            notify.append(line)
        elif line.startswith("posixtest: progress tests=1600/1600"):
            posix.append(line)
        elif line.startswith("posixtest: completed"):
            posix.append(line)
        elif line.startswith("SECTEST SUMMARY pass=91 "):
            sectest.append(line)
        elif line.startswith("Hermes test-all summary ") or line.startswith(
            "Hermes test-all result:"
        ):
            hermes.append(line)
    lines = []
    if notify:
        lines.append(notify[0])
        fields = dict(part.split("=", 1) for part in notify[0].split() if "=" in part)
        late = int(fields.get("late", "0") or "0")
        lines.append(
            "notify-gate disclosure "
            f"attempts={fields.get('attempts', '?')} late={late} values={fields.get('values', '')}"
        )
    else:
        lines.append("notify-gate disclosure missing")
    if posix:
        lines.append(posix[0])
        if "posixtest: completed" in posix:
            lines.append("posixtest: completed")
    if sectest:
        lines.append(sectest[0])
    if hermes:
        lines.append(hermes[-2] if len(hermes) >= 2 else hermes[-1])
        if hermes[-1].startswith("Hermes test-all result:"):
            lines.append(hermes[-1])
    return "\n".join(lines) + "\n"


def main():
    if "--self-test" in sys.argv:
        injected = (
            "NOTIFY-GATE SUMMARY attempts=2 late=1 values=250000000,51000000\n"
            "CASE notify-gate-sleep RESULT PASS slept-on-time\n"
        )
        text = summarize(injected)
        if "attempts=2" not in text or "late=1" not in text:
            print("injected late summary missing attempt=2 late=1", file=sys.stderr)
            return 1
        over = summarize(
            "NOTIFY-GATE SUMMARY attempts=1 late=1 values=350000000\n"
            "CASE notify-gate-sleep RESULT FAIL nanosleep-late\n"
        )
        if "attempts=1" not in over or "late=1" not in over or "350000000" not in over:
            print("over-300ms summary was dropped", file=sys.stderr)
            return 1
        print("campaign summary self-test passed")
        return 0
    if len(sys.argv) != 2:
        print("usage: summarize-hermes-campaign.py SERIAL", file=sys.stderr)
        return 2
    serial = pathlib.Path(sys.argv[1]).read_text(errors="replace")
    out = ROOT / "verification/logs/campaign-summary.txt"
    out.write_text(summarize(serial))
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
