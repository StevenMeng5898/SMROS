"""Command-line entry point for Open POSIX Test Suite host tooling."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
from pathlib import Path

from .source import fetch_checkout, load_source_lock


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SOURCE_LOCK_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "source.lock.json"
PATCH_SERIES_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches" / "series"


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="smros-posixtest")
    subparsers = parser.add_subparsers(dest="command", required=True)
    fetch_parser = subparsers.add_parser("fetch", help="fetch the pinned source")
    fetch_parser.add_argument(
        "--work-dir", type=Path, default=Path("target/posix")
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = create_parser().parse_args(argv)
    if arguments.command == "fetch":
        lock = load_source_lock(SOURCE_LOCK_PATH)
        checkout = arguments.work_dir / "src" / lock.revision
        fetch_checkout(lock, checkout, PATCH_SERIES_PATH)
        return 0
    raise AssertionError(f"unhandled command: {arguments.command}")


if __name__ == "__main__":
    raise SystemExit(main())
