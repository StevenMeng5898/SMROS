"""Dedicated Linux subreaper process for one POSIX baseline attempt."""

from __future__ import annotations

from pathlib import Path
import sys


if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from scripts.posix.baseline import launch_runtime, supervise_runtime


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    if len(arguments) < 2:
        return 125
    mode = "supervise"
    if arguments[0] == "launch":
        mode = "launch"
        arguments = arguments[1:]
        if len(arguments) < 2:
            return 125
    try:
        control_descriptor = int(arguments[0])
    except ValueError:
        return 125
    runner = launch_runtime if mode == "launch" else supervise_runtime
    return runner(arguments[1:], control_descriptor)


if __name__ == "__main__":
    raise SystemExit(main())
