"""Command-line entry point for Open POSIX Test Suite host tooling."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
from pathlib import Path
import sys

from .discovery import audit_reviews
from .source import fetch_checkout, load_source_lock


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SOURCE_LOCK_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "source.lock.json"
PATCH_SERIES_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches" / "series"
STUB_REVIEW_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "stub-review.tsv"
SHELL_REVIEW_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "shell-review.tsv"
PINNED_C_SOURCE_COUNT = 1_979
PINNED_SHELL_SOURCE_COUNT = 176


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="smros-posixtest")
    subparsers = parser.add_subparsers(dest="command", required=True)
    fetch_parser = subparsers.add_parser("fetch", help="fetch the pinned source")
    fetch_parser.add_argument(
        "--work-dir", type=Path, default=Path("target/posix")
    )
    audit_parser = subparsers.add_parser(
        "audit", help="audit discovered tests and human review coverage"
    )
    audit_parser.add_argument(
        "--work-dir", type=Path, default=Path("target/posix")
    )
    actions = audit_parser.add_mutually_exclusive_group(required=True)
    actions.add_argument("--write-candidates", type=Path)
    actions.add_argument("--check", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = create_parser().parse_args(argv)
    if arguments.command == "fetch":
        lock = load_source_lock(SOURCE_LOCK_PATH)
        checkout = arguments.work_dir / "src" / lock.revision
        fetch_checkout(lock, checkout, PATCH_SERIES_PATH)
        return 0
    if arguments.command == "audit":
        lock = load_source_lock(SOURCE_LOCK_PATH)
        checkout = arguments.work_dir / "src" / lock.revision
        try:
            fetch_checkout(lock, checkout, PATCH_SERIES_PATH)
            result = audit_reviews(
                checkout,
                STUB_REVIEW_PATH,
                SHELL_REVIEW_PATH,
                write_directory=arguments.write_candidates,
            )
            if arguments.check and (
                result.c_sources != PINNED_C_SOURCE_COUNT
                or result.shell_files != PINNED_SHELL_SOURCE_COUNT
            ):
                raise ValueError(
                    "pinned inventory count mismatch: "
                    f"C={result.c_sources} (expected {PINNED_C_SOURCE_COUNT}), "
                    f"shell={result.shell_files} "
                    f"(expected {PINNED_SHELL_SOURCE_COUNT})"
                )
        except ValueError as error:
            print(f"audit failed: {error}", file=sys.stderr)
            return 1
        print(result.format_counts())
        return 0
    raise AssertionError(f"unhandled command: {arguments.command}")


if __name__ == "__main__":
    raise SystemExit(main())
