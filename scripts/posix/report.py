"""Canonical POSIX result aggregation and multi-format reporting."""

from __future__ import annotations

from collections import Counter
import csv
import ctypes
from dataclasses import asdict, dataclass, fields
import errno
import html
import io
import json
import os
from pathlib import Path, PurePosixPath
import secrets
import stat
from typing import Iterable, Mapping, Sequence
import xml.etree.ElementTree as ET

from .build import (
    CHECKSUM_DEFINITION,
    MAX_BUILD_RESULTS_BYTES,
    MAX_HOST_MANIFEST_BYTES,
    MAX_MANIFEST_BYTES,
    ManifestMetadata,
    _build_results_digest,
    _load_build_results,
    parse_manifest,
)
from .model import (
    OVERALL_STATUSES,
    BuildResult,
    CoverageMetric,
    RuntimeAttempt,
    SuiteTest,
)


OUTPUT_NAMES = (
    "events.ndjson",
    "summary.json",
    "junit.xml",
    "groups.csv",
    "apis.csv",
    "report.md",
    "index.html",
)
_MAX_RUNTIME_RESULTS_BYTES = 128 * 1024 * 1024
_MAX_RUNTIME_RESULT_LINE_BYTES = 512 * 1024
_MAX_RUNTIME_ROWS = 32_768
_MAX_FAILURE_OUTPUT_BYTES = 16_384
_DIGEST_LENGTH = 64
_COMMIT_LENGTH = 40
_COMPLETE_DISPOSITIONS = frozenset(
    {"complete", "compile-failed", "link-failed", "not-built-shell-test"}
)
_RUNTIME_TERMINAL_REQUIRED_FIELDS = {
    "build_id",
    "build_results_sha256",
    "complete",
    "completed_count",
    "manifest_sha256",
    "patch_sha256",
    "platform",
    "record_type",
    "revision",
    "run_id",
    "selected_count",
    "smros_commit",
    "source",
    "status_counts",
}
_RUNTIME_TERMINAL_OPTIONAL_FIELDS = {
    "boot_count",
    "infrastructure_error",
    "qemu",
    "raw_log",
    "restart_count",
    "runtime_snapshot_sha256",
    "sysroot",
}


@dataclass(frozen=True)
class _ManifestInput:
    metadata: ManifestMetadata
    tests: tuple[SuiteTest, ...]
    build_results: tuple[BuildResult, ...]
    runtime: tuple[tuple[str, str], ...]


@dataclass(frozen=True)
class _RuntimeInput:
    path: str
    attempts: tuple[RuntimeAttempt, ...]
    terminal: Mapping[str, object]
    rows: tuple[Mapping[str, object], ...]


def _directory_flags() -> int:
    return (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_DIRECTORY", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )


def _open_directory_at(parent: int, name: str, label: str) -> int:
    try:
        before = os.stat(name, dir_fd=parent, follow_symlinks=False)
    except OSError as error:
        raise ValueError(f"missing {label}: {name}") from error
    if stat.S_ISLNK(before.st_mode):
        raise ValueError(f"{label} must not be a symlink: {name}")
    if not stat.S_ISDIR(before.st_mode):
        raise ValueError(f"{label} is not a directory: {name}")
    try:
        descriptor = os.open(name, _directory_flags(), dir_fd=parent)
    except OSError as error:
        raise ValueError(f"{label} could not be opened safely: {name}") from error
    opened = os.fstat(descriptor)
    if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
        os.close(descriptor)
        raise ValueError(f"{label} changed while being opened: {name}")
    return descriptor


def _open_directory_chain(path: Path, label: str, *, create: bool) -> int:
    absolute = Path(os.path.abspath(path))
    descriptor = os.open(absolute.anchor, _directory_flags())
    current = Path(absolute.anchor)
    try:
        for part in absolute.parts[1:]:
            current /= part
            try:
                child = _open_directory_at(
                    descriptor, part, f"{label} component {current}"
                )
            except ValueError as original:
                try:
                    os.stat(part, dir_fd=descriptor, follow_symlinks=False)
                except FileNotFoundError:
                    if not create:
                        raise original
                    try:
                        os.mkdir(part, 0o755, dir_fd=descriptor)
                    except FileExistsError:
                        pass
                    child = _open_directory_at(
                        descriptor, part, f"{label} component {current}"
                    )
                else:
                    raise
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _read_regular(path: Path, label: str, maximum: int) -> bytes:
    parent = _open_directory_chain(path.parent, f"{label} parent", create=False)
    descriptor: int | None = None
    try:
        try:
            before = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        except OSError as error:
            raise ValueError(f"missing {label}: {path}") from error
        if stat.S_ISLNK(before.st_mode):
            raise ValueError(f"{label} must not be a symlink: {path}")
        if not stat.S_ISREG(before.st_mode):
            raise ValueError(f"{label} is not a regular file: {path}")
        if before.st_size > maximum:
            raise ValueError(f"{label} exceeds its size limit")
        descriptor = os.open(
            path.name,
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=parent,
        )
        opened = os.fstat(descriptor)
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise ValueError(f"{label} changed while being opened")
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(descriptor, min(65_536, maximum + 1 - total))
            if not chunk:
                break
            chunks.append(chunk)
            total += len(chunk)
            if total > maximum:
                raise ValueError(f"{label} exceeds its size limit")
        after = os.fstat(descriptor)
        fingerprint = lambda info: (
            info.st_dev,
            info.st_ino,
            info.st_mode,
            info.st_size,
            info.st_mtime_ns,
            info.st_ctime_ns,
        )
        if fingerprint(opened) != fingerprint(after):
            raise ValueError(f"{label} changed while being read")
        return b"".join(chunks)
    finally:
        if descriptor is not None:
            os.close(descriptor)
        os.close(parent)


def _reject_duplicate_keys(
    pairs: list[tuple[str, object]],
) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _canonical_json(value: object) -> str:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    )


def _parse_canonical_json(data: bytes, label: str) -> object:
    try:
        text = data.decode("utf-8")
        value = json.loads(text, object_pairs_hook=_reject_duplicate_keys)
    except (UnicodeError, json.JSONDecodeError, ValueError) as error:
        raise ValueError(f"{label} is invalid JSON") from error
    if text != _canonical_json(value) + "\n":
        raise ValueError(f"{label} is not canonical JSON")
    return value


def _validate_runtime_inventory(value: object) -> tuple[tuple[str, str], ...]:
    if not isinstance(value, list):
        raise ValueError("manifest runtime is invalid")
    result: list[tuple[str, str]] = []
    seen: set[str] = set()
    for entry in value:
        if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
            raise ValueError("manifest runtime entry is invalid")
        path = entry["path"]
        digest = entry["sha256"]
        if not isinstance(path, str) or not isinstance(digest, str):
            raise ValueError("manifest runtime entry is invalid")
        parsed = PurePosixPath(path)
        if (
            parsed.is_absolute()
            or parsed.as_posix() != path
            or len(parsed.parts) != 2
            or parsed.parts[0] != "lib"
            or any(part in {"", ".", ".."} for part in path.split("/"))
            or path in seen
            or not _is_digest(digest)
        ):
            raise ValueError(f"manifest runtime entry is invalid: {path!r}")
        seen.add(path)
        result.append((path, digest))
    return tuple(result)


def _load_manifest(path: Path) -> _ManifestInput:
    if path.name != "manifest.json":
        raise ValueError("report manifest input must be manifest.json")
    host_value = _parse_canonical_json(
        _read_regular(path, "manifest.json", MAX_HOST_MANIFEST_BYTES),
        "manifest.json",
    )
    if not isinstance(host_value, dict) or set(host_value) != {
        "schema",
        "checksum_definition",
        "metadata",
        "runtime",
        "tests",
    }:
        raise ValueError("manifest.json schema is invalid")
    if (
        host_value["schema"] != 1
        or host_value["checksum_definition"] != CHECKSUM_DEFINITION
    ):
        raise ValueError("manifest.json schema is invalid")
    manifest_data = _read_regular(
        path.with_name("manifest.tsv"), "manifest.tsv", MAX_MANIFEST_BYTES
    )
    metadata, tests = parse_manifest(manifest_data)
    if host_value["metadata"] != asdict(metadata) or host_value["tests"] != [
        asdict(test) for test in tests
    ]:
        raise ValueError("manifest.json differs from manifest.tsv")
    runtime = _validate_runtime_inventory(host_value["runtime"])
    build_path = path.with_name("build-results.ndjson")
    build_parent = _open_directory_chain(
        build_path.parent, "build-results.ndjson parent", create=False
    )
    descriptor: int | None = None
    try:
        before = os.stat(
            build_path.name, dir_fd=build_parent, follow_symlinks=False
        )
        if stat.S_ISLNK(before.st_mode):
            raise ValueError("build-results.ndjson must not be a symlink")
        if not stat.S_ISREG(before.st_mode):
            raise ValueError("build-results.ndjson is not a regular file")
        if before.st_size > MAX_BUILD_RESULTS_BYTES:
            raise ValueError("build-results.ndjson exceeds its size limit")
        descriptor = os.open(
            build_path.name,
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=build_parent,
        )
        opened = os.fstat(descriptor)
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise ValueError("build-results.ndjson changed while being opened")
        build_results = _load_build_results(
            descriptor, tests, revision=metadata.revision
        )
    except FileNotFoundError as error:
        raise ValueError(f"missing build-results.ndjson: {build_path}") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
        os.close(build_parent)
    if _build_results_digest(build_results) != metadata.build_results_sha256:
        raise ValueError("build results checksum mismatch")
    return _ManifestInput(metadata, tests, build_results, runtime)


def _is_digest(value: object) -> bool:
    return isinstance(value, str) and len(value) == _DIGEST_LENGTH and all(
        character in "0123456789abcdef" for character in value
    )


def _is_commit(value: object) -> bool:
    return isinstance(value, str) and len(value) == _COMMIT_LENGTH and all(
        character in "0123456789abcdef" for character in value
    )


def _require_optional_text(value: object, label: str) -> str | None:
    if value is not None and not isinstance(value, str):
        raise ValueError(f"runtime attempt {label} is invalid")
    return value


def _validate_attempt(
    value: Mapping[str, object],
    tests_by_id: Mapping[str, SuiteTest],
    metadata: ManifestMetadata,
    line_number: int,
) -> RuntimeAttempt:
    expected = {field.name for field in fields(RuntimeAttempt)} | {"record_type"}
    if set(value) != expected or value.get("record_type") != "attempt":
        raise ValueError(f"invalid runtime attempt schema at line {line_number}")
    test_id = value["test_id"]
    if not isinstance(test_id, str) or test_id not in tests_by_id:
        raise ValueError(f"unknown runtime attempt test ID at line {line_number}")
    test = tests_by_id[test_id]
    for key, expected_value in (("group", test.group), ("api", test.api)):
        if value[key] != expected_value:
            raise ValueError(f"runtime attempt {key} mismatch at line {line_number}")
    for key in (
        "platform",
        "build_status",
        "link_status",
        "launch_status",
        "status",
        "stdout",
        "stderr",
        "source",
        "run_id",
    ):
        if not isinstance(value[key], str) or not value[key]:
            if key not in {"stdout", "stderr"} or value[key] != "":
                raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    if value["status"] not in OVERALL_STATUSES:
        raise ValueError(f"runtime attempt status is invalid at line {line_number}")
    if value["build_status"] not in {"passed", "failed", "not-built"}:
        raise ValueError(f"runtime attempt build status is invalid at line {line_number}")
    if value["link_status"] not in {"passed", "failed", "not-linked", "not-built"}:
        raise ValueError(f"runtime attempt link status is invalid at line {line_number}")
    if value["launch_status"] not in {"launched", "launch-error", "interrupted"}:
        raise ValueError(f"runtime attempt launch status is invalid at line {line_number}")
    pts_status = value["pts_status"]
    if pts_status is not None and pts_status not in OVERALL_STATUSES[:5]:
        raise ValueError(f"runtime attempt PTS status is invalid at line {line_number}")
    for key in ("exit_code", "signal"):
        if value[key] is not None and type(value[key]) is not int:
            raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    if value["signal"] is not None and int(value["signal"]) <= 0:
        raise ValueError(f"runtime attempt signal is invalid at line {line_number}")
    if type(value["timed_out"]) is not bool:
        raise ValueError(f"runtime attempt timeout flag is invalid at line {line_number}")
    for key in ("duration_ms", "stdout_bytes", "stderr_bytes"):
        if type(value[key]) is not int or int(value[key]) < 0:
            raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    for key in ("stdout_truncated", "stderr_truncated"):
        if type(value[key]) is not bool:
            raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    for key in ("launch_error", "infrastructure_error"):
        _require_optional_text(value[key], key)
    if value["status"] == "pass" and (
        value["launch_status"] != "launched"
        or value["pts_status"] != "pass"
        or value["exit_code"] != 0
        or value["signal"] is not None
        or value["timed_out"] is not False
        or value["launch_error"] is not None
        or value["infrastructure_error"] is not None
    ):
        raise ValueError(
            f"runtime attempt pass dimensions are invalid at line {line_number}"
        )
    for key in (
        "manifest_sha256",
        "build_results_sha256",
        "build_id",
        "patch_sha256",
        "binary_sha256",
        "runtime_snapshot_sha256",
    ):
        if not _is_digest(value[key]):
            raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    for key in ("revision", "smros_commit"):
        if not _is_commit(value[key]):
            raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    if (
        value["manifest_sha256"] != metadata.manifest_sha256
        or value["build_results_sha256"] != metadata.build_results_sha256
        or value["revision"] != metadata.revision
        or value["patch_sha256"] != metadata.patch_sha256
        or value["smros_commit"] != metadata.smros_commit
    ):
        raise ValueError(f"runtime attempt provenance mismatch at line {line_number}")
    payload = {key: value[key] for key in value if key != "record_type"}
    return RuntimeAttempt(**payload)  # type: ignore[arg-type]


def _validate_terminal(
    value: Mapping[str, object],
    metadata: ManifestMetadata,
    attempts: Sequence[RuntimeAttempt],
    line_number: int,
) -> None:
    fields_present = set(value)
    if (
        not _RUNTIME_TERMINAL_REQUIRED_FIELDS <= fields_present
        or fields_present
        - _RUNTIME_TERMINAL_REQUIRED_FIELDS
        - _RUNTIME_TERMINAL_OPTIONAL_FIELDS
        or value.get("record_type") != "run"
    ):
        raise ValueError(f"invalid runtime terminal schema at line {line_number}")
    for key in ("platform", "run_id", "source"):
        if not isinstance(value[key], str) or not value[key]:
            raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")
    if type(value["complete"]) is not bool:
        raise ValueError(f"runtime terminal completion is invalid at line {line_number}")
    for key in ("selected_count", "completed_count"):
        if type(value[key]) is not int or int(value[key]) < 0:
            raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")
    if value["completed_count"] != len(attempts):
        raise ValueError(f"runtime terminal completed count mismatch at line {line_number}")
    if value["completed_count"] > value["selected_count"]:
        raise ValueError(f"runtime terminal selected count mismatch at line {line_number}")
    expected_counts = dict(sorted(Counter(item.status for item in attempts).items()))
    if value["status_counts"] != expected_counts:
        raise ValueError(f"runtime terminal status counts mismatch at line {line_number}")
    if any(
        attempt.run_id != value["run_id"]
        or attempt.platform != value["platform"]
        or attempt.source != value["source"]
        for attempt in attempts
    ):
        raise ValueError(f"runtime terminal run identity mismatch at line {line_number}")
    if any(attempt.build_id != value["build_id"] for attempt in attempts):
        raise ValueError(
            f"runtime terminal build identity mismatch at line {line_number}"
        )
    for key in ("manifest_sha256", "build_results_sha256", "build_id", "patch_sha256"):
        if not _is_digest(value[key]):
            raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")
    for key in ("revision", "smros_commit"):
        if not _is_commit(value[key]):
            raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")
    if (
        value["manifest_sha256"] != metadata.manifest_sha256
        or value["build_results_sha256"] != metadata.build_results_sha256
        or value["revision"] != metadata.revision
        or value["patch_sha256"] != metadata.patch_sha256
        or value["smros_commit"] != metadata.smros_commit
    ):
        raise ValueError(f"runtime terminal provenance mismatch at line {line_number}")
    for key in _RUNTIME_TERMINAL_OPTIONAL_FIELDS & fields_present:
        item = value[key]
        if key in {"boot_count", "restart_count"}:
            if type(item) is not int or item < 0:
                raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")
        elif item is not None and not isinstance(item, str):
            raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")


def _load_runtime_results(
    path: Path,
    tests: Sequence[SuiteTest],
    metadata: ManifestMetadata,
) -> _RuntimeInput:
    data = _read_regular(path, "runtime results", _MAX_RUNTIME_RESULTS_BYTES)
    if not data or not data.endswith(b"\n") or b"\r" in data:
        raise ValueError("runtime results must be nonempty canonical LF NDJSON")
    tests_by_id = {test.test_id: test for test in tests}
    attempts: list[RuntimeAttempt] = []
    rows: list[Mapping[str, object]] = []
    terminal: Mapping[str, object] | None = None
    for line_number, raw_line in enumerate(data.splitlines(), start=1):
        if line_number > _MAX_RUNTIME_ROWS:
            raise ValueError("runtime result row count exceeds its limit")
        if len(raw_line) > _MAX_RUNTIME_RESULT_LINE_BYTES:
            raise ValueError("runtime result line exceeds its size limit")
        try:
            line = raw_line.decode("utf-8")
            value = json.loads(line, object_pairs_hook=_reject_duplicate_keys)
        except (UnicodeError, json.JSONDecodeError, ValueError) as error:
            raise ValueError(f"invalid runtime result JSON at line {line_number}") from error
        if not isinstance(value, dict) or line != _canonical_json(value):
            raise ValueError(f"noncanonical runtime result JSON at line {line_number}")
        if terminal is not None:
            raise ValueError("runtime result row appears after terminal record")
        if value.get("record_type") == "attempt":
            attempts.append(_validate_attempt(value, tests_by_id, metadata, line_number))
        elif value.get("record_type") == "run":
            _validate_terminal(value, metadata, attempts, line_number)
            terminal = value
        else:
            raise ValueError(f"unknown runtime result record at line {line_number}")
        rows.append(value)
    if terminal is None:
        raise ValueError("runtime results lack a terminal run record")
    return _RuntimeInput(
        path=str(Path(os.path.abspath(path))),
        attempts=tuple(attempts),
        terminal=dict(terminal),
        rows=tuple(dict(row) for row in rows),
    )


def _complete_test(test: SuiteTest) -> bool:
    return test.disposition in _COMPLETE_DISPOSITIONS


def _buildable_complete_test(test: SuiteTest) -> bool:
    return _complete_test(test) and test.kind != "shell"


def _successful_build(
    test: SuiteTest,
    build_by_identity: Mapping[tuple[str, str], BuildResult],
) -> bool:
    compile_result = build_by_identity.get((test.test_id, "compile"))
    if compile_result is None or compile_result.status != "passed":
        return False
    if test.kind == "runnable":
        link_result = build_by_identity.get((test.test_id, "link"))
        return link_result is not None and link_result.status == "passed"
    return True


def _attempt_executed(attempt: RuntimeAttempt) -> bool:
    return attempt.launch_status == "launched" and attempt.status not in {
        "interrupted",
        "launch-error",
    }


def _overall_attempt_status(attempts: Sequence[RuntimeAttempt]) -> str | None:
    if not attempts:
        return None
    outcomes = {attempt.status for attempt in attempts}
    if "flaky" in outcomes or len(outcomes) > 1:
        return "flaky"
    return attempts[0].status


def _test_status(
    test: SuiteTest,
    build_by_identity: Mapping[tuple[str, str], BuildResult],
    attempts: Sequence[RuntimeAttempt],
) -> str:
    if test.disposition == "excluded-upstream-stub":
        return "not-built"
    if test.kind == "definition":
        return "pass" if _successful_build(test, build_by_identity) else "build-fail"
    if test.kind == "shell" or test.disposition == "not-built-shell-test":
        return "not-built"
    if not _successful_build(test, build_by_identity):
        return "build-fail"
    return _overall_attempt_status(attempts) or "untested"


def _metric(
    denominator: Iterable[str], numerator: Iterable[str]
) -> dict[str, object]:
    denominator_ids = tuple(sorted(set(denominator)))
    numerator_ids = tuple(sorted(set(numerator)))
    if not set(numerator_ids) <= set(denominator_ids):
        raise AssertionError("coverage numerator is not a denominator subset")
    return CoverageMetric(
        numerator=len(numerator_ids),
        denominator=len(denominator_ids),
        test_ids=denominator_ids,
        numerator_test_ids=numerator_ids,
    ).to_dict()


def _scope_summary(
    tests: Sequence[SuiteTest],
    build_by_identity: Mapping[tuple[str, str], BuildResult],
    primary_attempts: Mapping[str, Sequence[RuntimeAttempt]],
) -> dict[str, object]:
    complete = [test for test in tests if _complete_test(test)]
    buildable = [test for test in complete if _buildable_complete_test(test)]
    built = [test for test in buildable if _successful_build(test, build_by_identity)]
    attempted = [test for test in complete if primary_attempts.get(test.test_id)]
    executed = [
        test
        for test in built
        if any(_attempt_executed(item) for item in primary_attempts.get(test.test_id, ()))
    ]
    passed = [
        test
        for test in complete
        if _test_status(
            test, build_by_identity, primary_attempts.get(test.test_id, ())
        )
        == "pass"
    ]
    statuses = Counter(
        _test_status(test, build_by_identity, primary_attempts.get(test.test_id, ()))
        for test in tests
    )
    leaked = sum(
        any(
            bool(getattr(attempt, "resource_deltas", {}))
            and any(getattr(attempt, "resource_deltas", {}).values())
            for attempt in primary_attempts.get(test.test_id, ())
        )
        for test in tests
    )
    counts: dict[str, int] = {
        "build_attempted": sum(
            (test.test_id, "compile") in build_by_identity for test in buildable
        ),
        "built": len(built),
        "complete": len(complete),
        "definition_only": sum(test.kind == "definition" for test in tests),
        "discovered": len(tests),
        "excluded_upstream_stub": sum(
            test.disposition == "excluded-upstream-stub" for test in tests
        ),
        "executed": len(executed),
        "execution_attempted": len(attempted),
        "leaked_resources": leaked,
        "passed": len(passed),
    }
    counts.update({status: statuses.get(status, 0) for status in OVERALL_STATUSES})
    metrics = {
        "build_coverage": _metric(
            (test.test_id for test in buildable),
            (test.test_id for test in built),
        ),
        "execution_coverage": _metric(
            (test.test_id for test in built),
            (test.test_id for test in executed),
        ),
        "pass_coverage": _metric(
            (test.test_id for test in executed),
            (test.test_id for test in passed if test in executed),
        ),
        "program_completion": _metric(
            (test.test_id for test in complete),
            (test.test_id for test in passed),
        ),
    }
    return {"counts": counts, "metrics": metrics}


def _bounded_failure_output(parts: Iterable[str]) -> str:
    text = "\n".join(part for part in parts if part)
    data = text.encode("utf-8", errors="replace")
    suffix = b"\n...[truncated]"
    if len(data) <= _MAX_FAILURE_OUTPUT_BYTES:
        return text
    body = data[: _MAX_FAILURE_OUTPUT_BYTES - len(suffix)]
    while True:
        try:
            return body.decode("utf-8") + suffix.decode("ascii")
        except UnicodeDecodeError:
            body = body[:-1]


def _attempt_dict(attempt: RuntimeAttempt) -> dict[str, object]:
    return attempt.to_dict()


def _reference_delta(primary: str, reference: str | None, has_smros: bool) -> str:
    if not has_smros:
        return "not-applicable"
    if reference is None:
        return "missing-reference"
    if primary == reference:
        return "match"
    if primary == "pass":
        return "improvement"
    if reference == "pass":
        return "regression"
    return "different"


def _aggregate(
    manifest: _ManifestInput,
    linux_inputs: Sequence[_RuntimeInput],
    smros_inputs: Sequence[_RuntimeInput],
) -> dict[str, object]:
    build_by_identity = {
        (result.test_id, result.stage): result for result in manifest.build_results
    }
    linux_by_test: dict[str, list[RuntimeAttempt]] = {}
    smros_by_test: dict[str, list[RuntimeAttempt]] = {}
    for source_inputs, destination in (
        (linux_inputs, linux_by_test),
        (smros_inputs, smros_by_test),
    ):
        for source in source_inputs:
            for attempt in source.attempts:
                destination.setdefault(attempt.test_id, []).append(attempt)
    primary_by_test = smros_by_test if smros_inputs else linux_by_test
    test_rows: list[dict[str, object]] = []
    for test in manifest.tests:
        primary_attempts = tuple(primary_by_test.get(test.test_id, ()))
        linux_attempts = tuple(linux_by_test.get(test.test_id, ()))
        smros_attempts = tuple(smros_by_test.get(test.test_id, ()))
        status = _test_status(test, build_by_identity, primary_attempts)
        reference_status = _overall_attempt_status(linux_attempts)
        build_rows = [
            asdict(result)
            for result in manifest.build_results
            if result.test_id == test.test_id
        ]
        for row in build_rows:
            row["argv"] = list(row["argv"])
        failure_parts: list[str] = []
        for result in manifest.build_results:
            if result.test_id == test.test_id and result.status == "failed":
                failure_parts.extend((result.stdout, result.stderr))
        for attempt in (*linux_attempts, *smros_attempts):
            if attempt.status != "pass":
                failure_parts.extend(
                    (
                        attempt.stdout,
                        attempt.stderr,
                        attempt.launch_error or "",
                        attempt.infrastructure_error or "",
                    )
                )
        test_rows.append(
            {
                "api": test.api,
                "attempts": [
                    _attempt_dict(attempt)
                    for attempt in (*linux_attempts, *smros_attempts)
                ],
                "build_results": build_rows,
                "disposition": test.disposition,
                "duration_ms": {
                    "build": sum(result.duration_ms for result in manifest.build_results if result.test_id == test.test_id),
                    "runtime": sum(attempt.duration_ms for attempt in (*linux_attempts, *smros_attempts)),
                },
                "exclusion_evidence": (
                    {"disposition": test.disposition, "source": test.source}
                    if test.disposition == "excluded-upstream-stub"
                    else None
                ),
                "failure_output": _bounded_failure_output(failure_parts),
                "group": test.group,
                "kind": test.kind,
                "linux_attempts": [_attempt_dict(attempt) for attempt in linux_attempts],
                "linux_reference_delta": _reference_delta(
                    status, reference_status, bool(smros_inputs)
                ),
                "linux_reference_status": reference_status,
                "resource_deltas": {},
                "smros_attempts": [_attempt_dict(attempt) for attempt in smros_attempts],
                "source": test.source,
                "status": status,
                "test_id": test.test_id,
            }
        )
    global_summary = _scope_summary(
        manifest.tests, build_by_identity, primary_by_test
    )
    groups: dict[str, dict[str, object]] = {}
    for group in sorted({test.group for test in manifest.tests}):
        scoped = tuple(test for test in manifest.tests if test.group == group)
        groups[group] = _scope_summary(scoped, build_by_identity, primary_by_test)
    apis: dict[str, dict[str, object]] = {}
    for api in sorted({test.api for test in manifest.tests}):
        scoped = tuple(test for test in manifest.tests if test.api == api)
        apis[api] = _scope_summary(scoped, build_by_identity, primary_by_test)
    runtime_inputs = (*linux_inputs, *smros_inputs)
    complete = bool(runtime_inputs) and all(
        source.terminal["complete"] is True for source in runtime_inputs
    )
    provenance = {
        "build_results_sha256": manifest.metadata.build_results_sha256,
        "compiler": manifest.metadata.compiler,
        "libc": manifest.metadata.libc,
        "linux_runs": [dict(source.terminal) for source in linux_inputs],
        "manifest_sha256": manifest.metadata.manifest_sha256,
        "patch_sha256": manifest.metadata.patch_sha256,
        "revision": manifest.metadata.revision,
        "smros_commit": manifest.metadata.smros_commit,
        "smros_runs": [dict(source.terminal) for source in smros_inputs],
    }
    return {
        "apis": apis,
        "complete": complete,
        "counts": global_summary["counts"],
        "groups": groups,
        "metrics": global_summary["metrics"],
        "primary_runtime": "smros" if smros_inputs else "linux-reference",
        "provenance": provenance,
        "run_status": "complete" if complete else "incomplete",
        "schema": 1,
        "statuses": list(OVERALL_STATUSES),
        "tests": test_rows,
    }


def _events_bytes(inputs: Sequence[_RuntimeInput]) -> bytes:
    return "".join(
        _canonical_json(row) + "\n"
        for source in inputs
        for row in source.rows
    ).encode("ascii")


def _summary_bytes(summary: Mapping[str, object]) -> bytes:
    return (_canonical_json(summary) + "\n").encode("ascii")


def _junit_bytes(summary: Mapping[str, object]) -> bytes:
    tests = summary["tests"]
    assert isinstance(tests, list)
    failures = sum(
        row["status"] != "pass"
        and row["disposition"] != "excluded-upstream-stub"
        for row in tests
        if isinstance(row, dict)
    )
    skipped = sum(
        isinstance(row, dict)
        and row["disposition"] == "excluded-upstream-stub"
        for row in tests
    )
    suite = ET.Element(
        "testsuite",
        {
            "errors": "0",
            "failures": str(failures),
            "name": "SMROS POSIX coverage",
            "skipped": str(skipped),
            "tests": str(len(tests)),
        },
    )
    for row in tests:
        assert isinstance(row, dict)
        durations = row["duration_ms"]
        assert isinstance(durations, dict)
        case = ET.SubElement(
            suite,
            "testcase",
            {
                "classname": f"{row['group']}.{row['api']}",
                "name": str(row["test_id"]),
                "time": f"{int(durations['runtime']) / 1000:.3f}",
            },
        )
        if row["disposition"] == "excluded-upstream-stub":
            ET.SubElement(case, "skipped", {"message": "audited upstream stub"})
        elif row["status"] != "pass":
            failure = ET.SubElement(
                case, "failure", {"message": str(row["status"])}
            )
            failure.text = str(row["failure_output"])
        attempts = row["attempts"]
        assert isinstance(attempts, list)
        output = "".join(str(attempt.get("stdout", "")) for attempt in attempts)
        if output:
            ET.SubElement(case, "system-out").text = output
        errors = "".join(str(attempt.get("stderr", "")) for attempt in attempts)
        if errors:
            ET.SubElement(case, "system-err").text = errors
    tree = ET.ElementTree(suite)
    ET.indent(tree, space="  ")
    stream = io.BytesIO()
    tree.write(stream, encoding="utf-8", xml_declaration=True)
    return stream.getvalue() + b"\n"


def _csv_bytes(scopes: Mapping[str, object], label: str) -> bytes:
    columns = [
        label,
        "discovered",
        "complete",
        "excluded_upstream_stub",
        "build_attempted",
        "built",
        "execution_attempted",
        "executed",
        "passed",
        *OVERALL_STATUSES,
        "build_coverage",
        "execution_coverage",
        "pass_coverage",
        "program_completion",
    ]
    stream = io.StringIO(newline="")
    writer = csv.DictWriter(stream, fieldnames=columns, lineterminator="\n")
    writer.writeheader()
    for name, scope in sorted(scopes.items()):
        assert isinstance(scope, dict)
        counts = scope["counts"]
        metrics = scope["metrics"]
        assert isinstance(counts, dict) and isinstance(metrics, dict)
        row: dict[str, object] = {label: name}
        for column in columns[1:]:
            if column in metrics:
                metric = metrics[column]
                assert isinstance(metric, dict)
                row[column] = metric["fraction"]
            else:
                row[column] = counts.get(column, 0)
        writer.writerow(row)
    return stream.getvalue().encode("utf-8")


def _markdown_escape(value: object) -> str:
    return (
        html.escape(str(value), quote=True)
        .replace("\\", "\\\\")
        .replace("|", "\\|")
        .replace("\r", "")
        .replace("\n", "<br>")
    )


def _markdown_bytes(summary: Mapping[str, object]) -> bytes:
    metrics = summary["metrics"]
    tests = summary["tests"]
    assert isinstance(metrics, dict) and isinstance(tests, list)
    lines = [
        "# SMROS POSIX API Coverage",
        "",
        f"Run status: **{_markdown_escape(summary['run_status'])}**",
        "",
        "## Coverage",
        "",
        "| Metric | Result |",
        "| --- | ---: |",
    ]
    for name in (
        "build_coverage",
        "execution_coverage",
        "pass_coverage",
        "program_completion",
    ):
        metric = metrics[name]
        assert isinstance(metric, dict)
        lines.append(
            f"| {_markdown_escape(name.replace('_', ' ').title())} | "
            f"{_markdown_escape(metric['fraction'])} |"
        )
    lines.extend(
        (
            "",
            "## Tests",
            "",
            "| Test ID | Group | API | Disposition | Status | Linux delta | Failure output |",
            "| --- | --- | --- | --- | --- | --- | --- |",
        )
    )
    for row in tests:
        assert isinstance(row, dict)
        lines.append(
            "| "
            + " | ".join(
                _markdown_escape(row[key])
                for key in (
                    "test_id",
                    "group",
                    "api",
                    "disposition",
                    "status",
                    "linux_reference_delta",
                    "failure_output",
                )
            )
            + " |"
        )
    return ("\n".join(lines) + "\n").encode("utf-8")


def _append_text(parent: ET.Element, tag: str, text: object, **attrs: str) -> ET.Element:
    node = ET.SubElement(parent, tag, attrs)
    node.text = str(text)
    return node


def _html_bytes(summary: Mapping[str, object]) -> bytes:
    root = ET.Element("html", {"lang": "en"})
    head = ET.SubElement(root, "head")
    ET.SubElement(head, "meta", {"charset": "utf-8"})
    ET.SubElement(
        head,
        "meta",
        {"content": "width=device-width, initial-scale=1", "name": "viewport"},
    )
    _append_text(head, "title", "SMROS POSIX API Coverage")
    _append_text(
        head,
        "style",
        "body{font-family:system-ui,sans-serif;margin:24px;color:#171717;background:#fff}"
        "table{border-collapse:collapse;width:100%;font-size:14px}"
        "th,td{border:1px solid #bbb;padding:6px;text-align:left;vertical-align:top}"
        "th{background:#eee;position:sticky;top:0}pre{white-space:pre-wrap;margin:0;max-width:48rem}"
        "label{display:inline-flex;gap:8px;align-items:center;margin:12px 0}"
        ".metrics{display:flex;gap:20px;flex-wrap:wrap}.metric{border-left:4px solid #333;padding-left:8px}",
    )
    body = ET.SubElement(root, "body")
    _append_text(body, "h1", "SMROS POSIX API Coverage")
    _append_text(body, "p", f"Run status: {summary['run_status']}")
    metrics = summary["metrics"]
    assert isinstance(metrics, dict)
    metrics_node = ET.SubElement(body, "div", {"class": "metrics"})
    for name in (
        "build_coverage",
        "execution_coverage",
        "pass_coverage",
        "program_completion",
    ):
        metric = metrics[name]
        assert isinstance(metric, dict)
        block = ET.SubElement(metrics_node, "div", {"class": "metric"})
        _append_text(block, "strong", name.replace("_", " ").title())
        _append_text(block, "div", metric["fraction"])
    label = ET.SubElement(body, "label", {"for": "status-filter"})
    label.text = "Status"
    select = ET.SubElement(
        label,
        "select",
        {"id": "status-filter", "onchange": "filterStatus(this.value)"},
    )
    _append_text(select, "option", "All", value="all")
    for status_name in OVERALL_STATUSES:
        _append_text(select, "option", status_name, value=status_name)
    table = ET.SubElement(body, "table", {"id": "results"})
    header = ET.SubElement(ET.SubElement(table, "thead"), "tr")
    for column in (
        "Test ID",
        "Group",
        "API",
        "Disposition",
        "Status",
        "Linux delta",
        "Failure output",
    ):
        _append_text(header, "th", column)
    tbody = ET.SubElement(table, "tbody")
    tests = summary["tests"]
    assert isinstance(tests, list)
    for row in tests:
        assert isinstance(row, dict)
        tr = ET.SubElement(tbody, "tr", {"data-status": str(row["status"])})
        for key in (
            "test_id",
            "group",
            "api",
            "disposition",
            "status",
            "linux_reference_delta",
        ):
            _append_text(tr, "td", row[key])
        cell = ET.SubElement(tr, "td")
        _append_text(cell, "pre", row["failure_output"])
    _append_text(
        body,
        "script",
        "function filterStatus(wanted){document.querySelectorAll('#results tbody tr').forEach(function(row){row.hidden=wanted!=='all'&&row.dataset.status!==wanted;});}",
    )
    return b"<!doctype html>\n" + ET.tostring(
        root, encoding="utf-8", method="html"
    ) + b"\n"


def _write_all(descriptor: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        written = os.write(descriptor, view)
        if written <= 0:
            raise OSError("report write made no progress")
        view = view[written:]


def _clear_directory(descriptor: int) -> None:
    with os.scandir(descriptor) as iterator:
        entries = list(iterator)
    for entry in entries:
        info = os.stat(entry.name, dir_fd=descriptor, follow_symlinks=False)
        if stat.S_ISDIR(info.st_mode):
            child = _open_directory_at(descriptor, entry.name, "report cleanup directory")
            try:
                _clear_directory(child)
            finally:
                os.close(child)
            os.rmdir(entry.name, dir_fd=descriptor)
        else:
            os.unlink(entry.name, dir_fd=descriptor)


def _rename_exchange(parent: int, first: str, second: str) -> None:
    renameat2 = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
    if renameat2 is None:
        raise ValueError("atomic report directory replacement is unavailable")
    renameat2.argtypes = (
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    )
    renameat2.restype = ctypes.c_int
    if renameat2(parent, os.fsencode(first), parent, os.fsencode(second), 2) != 0:
        error_number = ctypes.get_errno()
        if error_number in {errno.ENOSYS, errno.EINVAL, errno.ENOTSUP}:
            raise ValueError(
                "atomic report directory replacement is unavailable"
            )
        raise OSError(error_number, os.strerror(error_number), second)


def _publish_generation(output_directory: Path, outputs: Mapping[str, bytes]) -> None:
    if tuple(outputs) != OUTPUT_NAMES:
        raise AssertionError("report generation is missing an output")
    if not output_directory.name or output_directory.name in {".", ".."}:
        raise ValueError("report output directory is invalid")
    parent = _open_directory_chain(
        output_directory.parent, "report output parent", create=True
    )
    temporary_name = f".{output_directory.name}.{secrets.token_hex(8)}.tmp"
    temporary: int | None = None
    temporary_exists = False
    try:
        os.mkdir(temporary_name, 0o700, dir_fd=parent)
        temporary_exists = True
        temporary = _open_directory_at(parent, temporary_name, "temporary report")
        for name in OUTPUT_NAMES:
            descriptor = os.open(
                name,
                os.O_WRONLY
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                0o644,
                dir_fd=temporary,
            )
            try:
                _write_all(descriptor, outputs[name])
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        os.fsync(temporary)
        try:
            destination = os.stat(
                output_directory.name, dir_fd=parent, follow_symlinks=False
            )
        except FileNotFoundError:
            destination = None
        if destination is None:
            os.rename(
                temporary_name,
                output_directory.name,
                src_dir_fd=parent,
                dst_dir_fd=parent,
            )
            temporary_exists = False
        else:
            if stat.S_ISLNK(destination.st_mode):
                raise ValueError("report output directory must not be a symlink")
            if not stat.S_ISDIR(destination.st_mode):
                raise ValueError("report output destination is not a directory")
            held_destination = _open_directory_at(
                parent, output_directory.name, "report output directory"
            )
            try:
                held = os.fstat(held_destination)
                current = os.stat(
                    output_directory.name,
                    dir_fd=parent,
                    follow_symlinks=False,
                )
                if (held.st_dev, held.st_ino) != (current.st_dev, current.st_ino):
                    raise ValueError("report output directory changed before publication")
                _rename_exchange(parent, temporary_name, output_directory.name)
                os.fsync(parent)
            finally:
                os.close(held_destination)
            old = _open_directory_at(parent, temporary_name, "previous report")
            try:
                _clear_directory(old)
            finally:
                os.close(old)
            os.rmdir(temporary_name, dir_fd=parent)
            temporary_exists = False
        os.fsync(parent)
    finally:
        if temporary is not None:
            os.close(temporary)
        if temporary_exists:
            try:
                leftover = _open_directory_at(parent, temporary_name, "temporary report")
            except ValueError:
                leftover = None
            if leftover is not None:
                try:
                    _clear_directory(leftover)
                finally:
                    os.close(leftover)
                try:
                    os.rmdir(temporary_name, dir_fd=parent)
                except FileNotFoundError:
                    pass
        os.close(parent)


def _paths(value: Sequence[Path] | Path | None) -> tuple[Path, ...]:
    if value is None:
        return ()
    if isinstance(value, Path):
        return (value,)
    return tuple(Path(path) for path in value)


def generate_report(
    manifest_path: Path,
    *,
    linux_results: Sequence[Path] | Path | None = None,
    smros_results: Sequence[Path] | Path | None = None,
    output_directory: Path,
) -> dict[str, object]:
    """Validate all inputs, aggregate them, and publish one report generation."""
    linux_paths = _paths(linux_results)
    smros_paths = _paths(smros_results)
    if not linux_paths and not smros_paths:
        raise ValueError("at least one runtime-result input is required")
    manifest = _load_manifest(Path(manifest_path))
    linux_inputs = tuple(
        _load_runtime_results(path, manifest.tests, manifest.metadata)
        for path in linux_paths
    )
    smros_inputs = tuple(
        _load_runtime_results(path, manifest.tests, manifest.metadata)
        for path in smros_paths
    )
    summary = _aggregate(manifest, linux_inputs, smros_inputs)
    outputs = {
        "events.ndjson": _events_bytes((*linux_inputs, *smros_inputs)),
        "summary.json": _summary_bytes(summary),
        "junit.xml": _junit_bytes(summary),
        "groups.csv": _csv_bytes(summary["groups"], "group"),  # type: ignore[arg-type]
        "apis.csv": _csv_bytes(summary["apis"], "api"),  # type: ignore[arg-type]
        "report.md": _markdown_bytes(summary),
        "index.html": _html_bytes(summary),
    }
    _publish_generation(Path(output_directory), outputs)
    return summary
