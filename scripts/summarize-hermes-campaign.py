#!/usr/bin/env python3
"""Build verification/logs/campaign-summary.txt from a hermes serial log.

The notify-gate line is copied from the guest's NOTIFY-GATE SUMMARY. A late
count above zero is repeated in the summary so a hand-written zero cannot hide it.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]


def summarize(text):
    lines = []
    late_notes = []
    for line in text.replace("\r", "").splitlines():
        if line.startswith("NOTIFY-GATE SUMMARY "):
            lines.append(line)
            fields = dict(
                part.split("=", 1) for part in line.split() if "=" in part
            )
            late = int(fields.get("late", "0") or "0")
            if late:
                late_notes.append(
                    f"notify-gate late={late} attempts={fields.get('attempts', '?')} values={fields.get('values', '')}"
                )
        elif line.startswith("posixtest: progress tests=1600/1600"):
            lines.append(line)
        elif line.startswith("posixtest: completed") and not any(
            item == "posixtest: completed" for item in lines
        ):
            lines.append(line)
        elif line.startswith("SECTEST SUMMARY "):
            lines.append(line)
        elif line.startswith("Hermes test-all summary ") or line.startswith(
            "Hermes test-all result:"
        ):
            lines.append(line)
    if not late_notes:
        lines.append("notify-gate late=0")
    else:
        lines.extend(late_notes)
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
