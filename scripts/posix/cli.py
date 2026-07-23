"""Command-line entry point for Open POSIX Test Suite host tooling."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
from pathlib import Path
import shutil
import sys

from .baseline import BaselinePrerequisiteError, run_baseline
from .build import (
    ManifestMetadata,
    build_campaign,
    compiler_query,
    run_bounded_command,
    sha256_file,
    validate_build_checkout,
    verify_stage,
)
from .discovery import audit_reviews
from .model import BuildSummary, SuiteTest
from .source import fetch_checkout, load_source_lock


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SOURCE_LOCK_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "source.lock.json"
PATCH_SERIES_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches" / "series"
STUB_REVIEW_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "stub-review.tsv"
SHELL_REVIEW_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "shell-review.tsv"
PINNED_C_SOURCE_COUNT = 1_979
PINNED_SHELL_SOURCE_COUNT = 176
PINNED_STUB_REVIEW_SHA256 = (
    "d0cab4333fcb6f0dfc3238485e61b086ed4863bf7a1eebc8e7a756947ba82f7e"
)
PINNED_SHELL_REVIEW_SHA256 = (
    "be5f388dbf4768769a503a6ce58e5642ac8fbf9ed705c03093338b25c0afe7b5"
)
BASELINE_STAGE_PATH = REPOSITORY_ROOT / "host_shared" / "posixtest"
BASELINE_RESULTS_PATH = (
    REPOSITORY_ROOT
    / "target"
    / "posix"
    / "aarch64"
    / "linux-reference"
    / "results.ndjson"
)
BASELINE_PREREQUISITE = (
    "sudo apt-get install qemu-user gcc-aarch64-linux-gnu "
    "libc6-dev-arm64-cross"
)


def _print_exception_notes(error: BaseException) -> None:
    for note in getattr(error, "__notes__", ()):
        if isinstance(note, str):
            print(note, file=sys.stderr)


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
    build_parser = subparsers.add_parser(
        "build", help="cross-build and stage the reviewed POSIX suite"
    )
    build_parser.add_argument("--arch", required=True)
    build_parser.add_argument("--stage", required=True, type=Path)
    build_parser.add_argument("--verify-only", action="store_true")
    baseline_parser = subparsers.add_parser(
        "baseline", help="run the staged suite under qemu-user"
    )
    filters = baseline_parser.add_mutually_exclusive_group()
    filters.add_argument("--api")
    filters.add_argument("--group")
    filters.add_argument("--test")
    baseline_parser.add_argument("--sysroot", required=True, type=Path)
    return parser


def _required_tool(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise ValueError(f"required AArch64 tool is unavailable: {name}")
    return name


def _compiler_identity(compiler: str) -> str:
    try:
        completed = run_bounded_command([compiler, "--version"])
    except OSError as error:
        raise ValueError(f"required tool is unavailable: {compiler}: {error}") from error
    if completed.returncode != 0 or not completed.stdout.strip():
        raise ValueError(f"compiler version query failed: {compiler}")
    return completed.stdout.strip().splitlines()[0]


def _smros_commit() -> str:
    try:
        completed = run_bounded_command(
            ["git", "rev-parse", "HEAD"],
            cwd=REPOSITORY_ROOT,
        )
    except OSError as error:
        raise ValueError(f"cannot identify the SMROS commit: {error}") from error
    commit = completed.stdout.strip()
    if completed.returncode != 0 or len(commit) != 40 or any(
        character not in "0123456789abcdef" for character in commit
    ):
        raise ValueError("cannot identify the SMROS commit")
    status = run_bounded_command(
        [
            "git",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude,glob)**/__pycache__/**",
            ":(exclude,glob)**/*.pyc",
        ],
        cwd=REPOSITORY_ROOT,
    )
    if status.returncode != 0:
        raise ValueError("cannot inspect the SMROS worktree")
    if status.stdout.strip():
        raise ValueError("SMROS worktree is dirty; commit relevant sources first")
    return commit


def _libc_identity(compiler: str) -> str:
    value = compiler_query(compiler, "-print-file-name=libc.so.6")
    path = Path(value)
    if value == "libc.so.6" or not path.is_file():
        raise ValueError("AArch64 libc.so.6 could not be resolved by the compiler")
    return f"libc.so.6:{sha256_file(path.resolve())}"


def _validate_build_inventory(
    c_sources: int,
    shell_files: int,
    excluded_stubs: int,
    shell_tests: int,
) -> None:
    actual = (c_sources, shell_files, excluded_stubs, shell_tests)
    expected = (
        PINNED_C_SOURCE_COUNT,
        PINNED_SHELL_SOURCE_COUNT,
        94,
        169,
    )
    if actual != expected:
        raise ValueError(
            "pinned build inventory mismatch: "
            f"C={c_sources}, shell={shell_files}, "
            f"excluded={excluded_stubs}, shell-tests={shell_tests}"
        )


def _validate_review_ledgers(stub_path: Path, shell_path: Path) -> None:
    actual = (sha256_file(stub_path), sha256_file(shell_path))
    expected = (PINNED_STUB_REVIEW_SHA256, PINNED_SHELL_REVIEW_SHA256)
    if actual != expected:
        raise ValueError(
            "review ledger checksum mismatch; review identity pins must be "
            "updated deliberately with the reviewed path dispositions"
        )


def _current_build_inputs() -> tuple[
    ManifestMetadata,
    Path,
    tuple[SuiteTest, ...],
    tuple[str, ...],
]:
    lock = load_source_lock(SOURCE_LOCK_PATH)
    checkout = Path("target/posix") / "src" / lock.revision
    expected_patch = validate_build_checkout(
        checkout, lock.revision, PATCH_SERIES_PATH
    )
    _validate_review_ledgers(STUB_REVIEW_PATH, SHELL_REVIEW_PATH)
    audit = audit_reviews(checkout, STUB_REVIEW_PATH, SHELL_REVIEW_PATH)
    shell_tests = tuple(
        path
        for path, review in audit.shell_reviews.items()
        if review.disposition == "test"
    )
    _validate_build_inventory(
        audit.c_sources,
        audit.shell_files,
        sum(
            test.disposition == "excluded-upstream-stub"
            for test in audit.tests
        ),
        len(shell_tests),
    )
    metadata = ManifestMetadata(
        source=lock.url,
        revision=lock.revision,
        architecture="aarch64",
        compiler=_compiler_identity("aarch64-linux-gnu-gcc"),
        libc=_libc_identity("aarch64-linux-gnu-gcc"),
        patch_sha256=expected_patch,
        smros_commit=_smros_commit(),
    )
    return metadata, checkout, audit.tests, shell_tests


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
    if arguments.command == "build":
        if arguments.arch != "aarch64":
            print(
                f"build failed: unsupported architecture: {arguments.arch}",
                file=sys.stderr,
            )
            return 1
        try:
            _required_tool("aarch64-linux-gnu-gcc")
            _required_tool("aarch64-linux-gnu-nm")
            _required_tool("aarch64-linux-gnu-readelf")
            if arguments.verify_only:
                (
                    expected_metadata,
                    _checkout,
                    expected_tests,
                    expected_shell_tests,
                ) = _current_build_inputs()
                summary = verify_stage(
                    arguments.stage,
                    expected_metadata=expected_metadata,
                    expected_tests=expected_tests,
                    expected_shell_tests=expected_shell_tests,
                    strict_command_paths=True,
                )
            else:
                metadata, checkout, tests, shell_tests = _current_build_inputs()
                summary = build_campaign(
                    checkout,
                    tests,
                    shell_tests,
                    metadata,
                    arguments.stage,
                    Path("target/posix/aarch64"),
                )
        except (OSError, ValueError) as error:
            print(f"build failed: {error}", file=sys.stderr)
            return 1
        print(summary.format_counts())
        return 0
    if arguments.command == "baseline":
        qemu = shutil.which("qemu-aarch64")
        if qemu is None:
            print("baseline failed: qemu-aarch64 is unavailable", file=sys.stderr)
            print(BASELINE_PREREQUISITE, file=sys.stderr)
            return 1
        try:
            (
                expected_metadata,
                _checkout,
                expected_tests,
                expected_shell_tests,
            ) = _current_build_inputs()

            def strict_verifier(stage: Path) -> BuildSummary:
                return verify_stage(
                    stage,
                    expected_metadata=expected_metadata,
                    expected_tests=expected_tests,
                    expected_shell_tests=expected_shell_tests,
                    strict_command_paths=True,
                )

            result = run_baseline(
                BASELINE_STAGE_PATH,
                arguments.sysroot,
                BASELINE_RESULTS_PATH,
                api=arguments.api,
                group=arguments.group,
                test_id=arguments.test,
                qemu=qemu,
                verifier=strict_verifier,
            )
        except BaselinePrerequisiteError as error:
            print(f"baseline failed: {error}", file=sys.stderr)
            _print_exception_notes(error)
            print(BASELINE_PREREQUISITE, file=sys.stderr)
            return 1
        except (OSError, ValueError) as error:
            print(f"baseline failed: {error}", file=sys.stderr)
            _print_exception_notes(error)
            return 1
        print(
            f"selected={len(result.attempts)} "
            f"passed={sum(attempt.status == 'pass' for attempt in result.attempts)} "
            f"results={result.result_path}"
        )
        return 0 if result.all_passed else 1
    raise AssertionError(f"unhandled command: {arguments.command}")


if __name__ == "__main__":
    raise SystemExit(main())
