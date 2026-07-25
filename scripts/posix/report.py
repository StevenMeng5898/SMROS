"""Canonical POSIX result aggregation and multi-format reporting."""

from __future__ import annotations

from collections import Counter
import csv
import ctypes
from dataclasses import asdict, dataclass, fields
import errno
import fcntl
import hashlib
import html
import io
import json
import os
from pathlib import Path, PurePosixPath
import stat
from typing import Iterable, Mapping, Sequence
import xml.etree.ElementTree as ET

from .events import EVENT_PREFIX, parse_serial_log
from .build import (
    CHECKSUM_DEFINITION,
    MAX_BUILD_RESULTS_BYTES,
    MAX_HOST_MANIFEST_BYTES,
    MAX_MANIFEST_BYTES,
    MAX_TESTS,
    ManifestMetadata,
    _build_results_digest,
    _load_build_results,
    parse_manifest,
)
from .model import (
    OVERALL_STATUSES,
    BuildResult,
    CoverageMetric,
    RAW_RUNTIME_STATUSES,
    RESOURCE_DELTA_NAMES,
    ResourceDeltas,
    RuntimeAttempt,
    SuiteTest,
    is_valid_run_id,
    validate_host_watchdog_attempt_semantics,
    validate_raw_attempt_semantics,
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
LINUX_REFERENCE_PLATFORM = "aarch64-linux-reference"
LINUX_REFERENCE_SOURCE = "qemu-user"
SMROS_PLATFORM = "smros-aarch64"
SMROS_SOURCES = frozenset({"host-watchdog", "smros-qemu", "smros-serial"})
SMROS_SERIAL_SOURCE = "smros-serial"
_REPORT_QUARANTINE_NAME = ".smros-posix-report-quarantine"
_REPORT_WORK_ROOT_NAME = "generation"
_MAX_RUNTIME_RESULTS_BYTES = 128 * 1024 * 1024
_MAX_RUNTIME_RESULT_LINE_BYTES = 512 * 1024
_MAX_RUNTIME_ROWS = 32_768
_MAX_RUNTIME_INPUTS = 4 * MAX_TESTS
_MAX_RUNTIME_INPUT_BYTES = 4 * _MAX_RUNTIME_RESULTS_BYTES
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
    byte_count: int
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


def _is_strict_utf8_text(value: object, *, nonempty: bool = False) -> bool:
    if not isinstance(value, str) or (nonempty and not value):
        return False
    try:
        value.encode("utf-8")
    except UnicodeEncodeError:
        return False
    return True


def _require_optional_text(value: object, label: str) -> str | None:
    if value is not None and not _is_strict_utf8_text(value):
        raise ValueError(f"runtime attempt {label} is invalid")
    return value


def _validate_attempt(
    value: Mapping[str, object],
    tests_by_id: Mapping[str, SuiteTest],
    build_by_identity: Mapping[tuple[str, str], BuildResult],
    metadata: ManifestMetadata,
    line_number: int,
    *,
    role: str,
) -> RuntimeAttempt:
    expected = {field.name for field in fields(RuntimeAttempt)} | {"record_type"}
    without_evidence = expected - {"resource_evidence"}
    without_deltas = expected - {"resource_deltas"}
    legacy = without_evidence - {"resource_deltas"}
    schema_variants = {
        frozenset(expected),
        frozenset(without_evidence),
        frozenset(without_deltas),
    }
    if role == "linux":
        schema_variants.add(frozenset(legacy))
    allowed_schemas = schema_variants | {
        schema - {"raw_log_start", "raw_log_end"}
        for schema in schema_variants
    }
    if set(value) not in allowed_schemas or value.get("record_type") != "attempt":
        raise ValueError(f"invalid runtime attempt schema at line {line_number}")
    test_id = value["test_id"]
    if not isinstance(test_id, str) or test_id not in tests_by_id:
        raise ValueError(f"unknown runtime attempt test ID at line {line_number}")
    test = tests_by_id[test_id]
    for key, expected_value in (("group", test.group), ("api", test.api)):
        if value[key] != expected_value:
            raise ValueError(f"runtime attempt {key} mismatch at line {line_number}")
    if value["binary_sha256"] != test.sha256:
        raise ValueError(
            f"runtime attempt binary checksum mismatch at line {line_number}"
        )
    compile_result = build_by_identity.get((test_id, "compile"))
    expected_build_status = (
        compile_result.status if compile_result is not None else "not-built"
    )
    if value["build_status"] != expected_build_status:
        raise ValueError(f"runtime attempt build status mismatch at line {line_number}")
    link_result = build_by_identity.get((test_id, "link"))
    expected_link_status = (
        link_result.status if link_result is not None else "not-linked"
    )
    if value["link_status"] != expected_link_status:
        raise ValueError(f"runtime attempt link status mismatch at line {line_number}")
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
        valid = (
            is_valid_run_id(value[key])
            if key == "run_id"
            else _is_strict_utf8_text(
                value[key],
                nonempty=key not in {"stdout", "stderr"},
            )
        )
        if not valid:
            raise ValueError(f"runtime attempt {key} is invalid at line {line_number}")
    if role == "linux":
        if (
            value["platform"] != LINUX_REFERENCE_PLATFORM
            or value["source"] != LINUX_REFERENCE_SOURCE
        ):
            raise ValueError(
                "Linux-reference platform/source does not match its input role"
            )
    elif role == "smros":
        if value["platform"] != SMROS_PLATFORM or value["source"] not in SMROS_SOURCES:
            raise ValueError("SMROS platform/source does not match its input role")
    else:
        raise AssertionError(f"unknown runtime input role: {role}")
    if value["status"] not in RAW_RUNTIME_STATUSES:
        raise ValueError(
            f"runtime attempt raw runtime status is invalid at line {line_number}"
        )
    if value["build_status"] not in {"passed", "failed", "not-built"}:
        raise ValueError(f"runtime attempt build status is invalid at line {line_number}")
    if value["link_status"] not in {"passed", "failed", "not-linked", "not-built"}:
        raise ValueError(f"runtime attempt link status is invalid at line {line_number}")
    if value["launch_status"] not in {
        "launched",
        "launch-error",
        "interrupted",
        "not-launched",
    }:
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
    semantics = (
        validate_host_watchdog_attempt_semantics
        if role == "smros" and value["source"] == "host-watchdog"
        else validate_raw_attempt_semantics
    )
    semantics(
        status=str(value["status"]),
        pts_status=pts_status,
        launch_status=str(value["launch_status"]),
        exit_code=value["exit_code"],  # type: ignore[arg-type]
        signal=value["signal"],  # type: ignore[arg-type]
        timed_out=bool(value["timed_out"]),
        launch_error=value["launch_error"],  # type: ignore[arg-type]
        infrastructure_error=value["infrastructure_error"],  # type: ignore[arg-type]
        label="runtime attempt",
    )
    raw_log_start = value.get("raw_log_start")
    raw_log_end = value.get("raw_log_end")
    offset_label = (
        "host-watchdog raw log offsets"
        if value["source"] == "host-watchdog"
        else "runtime attempt raw log offsets"
    )
    if (raw_log_start is None) != (raw_log_end is None) or (
        raw_log_start is not None
        and (
            type(raw_log_start) is not int
            or type(raw_log_end) is not int
            or raw_log_start < 0
            or raw_log_end < raw_log_start
        )
    ):
        raise ValueError(f"{offset_label} are invalid at line {line_number}")
    if value["source"] == "host-watchdog" and raw_log_start is None:
        raise ValueError(
            f"host-watchdog raw log offsets are required at line {line_number}"
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
    raw_resources = value.get("resource_deltas")
    raw_evidence = value.get("resource_evidence")
    if role == "smros":
        if not isinstance(raw_resources, dict):
            raise ValueError(
                f"SMROS attempt lacks complete resource evidence at line {line_number}"
            )
        try:
            resource_deltas = ResourceDeltas.from_complete_mapping(raw_resources)
        except ValueError as error:
            raise ValueError(
                f"SMROS attempt lacks complete resource evidence at line {line_number}"
            ) from error
        resource_evidence = raw_evidence
        if value["source"] == "host-watchdog":
            if resource_evidence != "unavailable" or resource_deltas.has_nonzero():
                raise ValueError(
                    f"host-watchdog resource evidence is invalid at line {line_number}"
                )
        elif value["status"] == "interrupted":
            if resource_evidence != "unavailable" or resource_deltas.has_nonzero():
                raise ValueError(
                    f"interrupted resource evidence is invalid at line {line_number}"
                )
        elif resource_evidence != "measured":
            raise ValueError(
                f"SMROS attempt resource evidence is invalid at line {line_number}"
            )
    elif role == "linux":
        if raw_resources is None:
            resource_deltas = ResourceDeltas()
        elif isinstance(raw_resources, dict):
            try:
                resource_deltas = ResourceDeltas.from_complete_mapping(raw_resources)
            except ValueError as error:
                raise ValueError(
                    f"Linux attempt resource evidence is invalid at line {line_number}"
                ) from error
        else:
            raise ValueError(
                f"Linux attempt resource evidence is invalid at line {line_number}"
            )
        resource_evidence = raw_evidence or "unavailable"
        if resource_evidence != "unavailable":
            raise ValueError(
                f"Linux attempt resource evidence is invalid at line {line_number}"
            )
    else:
        raise AssertionError(f"unknown runtime input role: {role}")
    payload = {key: value[key] for key in value if key != "record_type"}
    payload.setdefault("raw_log_start", None)
    payload.setdefault("raw_log_end", None)
    payload["resource_deltas"] = resource_deltas
    payload["resource_evidence"] = resource_evidence
    return RuntimeAttempt(**payload)  # type: ignore[arg-type]


def _validate_terminal(
    value: Mapping[str, object],
    metadata: ManifestMetadata,
    attempts: Sequence[RuntimeAttempt],
    line_number: int,
    *,
    allow_unknown_provenance: bool = False,
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
        valid = (
            is_valid_run_id(value[key])
            if key == "run_id"
            else _is_strict_utf8_text(value[key], nonempty=True)
        )
        if not valid:
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
    if (
        value["complete"] is True
        and value["completed_count"] != value["selected_count"]
    ):
        raise ValueError(
            "runtime complete record does not cover every selected attempt "
            f"at line {line_number}"
        )
    if value["complete"] is True and len(
        {attempt.test_id for attempt in attempts}
    ) != value["selected_count"]:
        raise ValueError(
            "runtime complete record does not contain unique selected attempts "
            f"at line {line_number}"
        )
    if value["complete"] is True and any(
        attempt.status == "interrupted" for attempt in attempts
    ):
        raise ValueError(
            "runtime complete record contains an interrupted attempt "
            f"at line {line_number}"
        )
    if value["complete"] is True and (
        value.get("infrastructure_error")
        or any(
            attempt.infrastructure_error and attempt.source != "host-watchdog"
            for attempt in attempts
        )
    ):
        raise ValueError(
            "runtime complete record contains an infrastructure error "
            f"at line {line_number}"
        )
    expected_counts = dict(sorted(Counter(item.status for item in attempts).items()))
    if value["status_counts"] != expected_counts:
        raise ValueError(f"runtime terminal status counts mismatch at line {line_number}")
    if any(
        attempt.run_id != value["run_id"]
        or attempt.platform != value["platform"]
        or (
            attempt.source != value["source"]
            and not (
                value["source"] == "smros-qemu"
                and attempt.source == "host-watchdog"
            )
        )
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
    provenance = {
        "manifest_sha256": metadata.manifest_sha256,
        "build_results_sha256": metadata.build_results_sha256,
        "revision": metadata.revision,
        "patch_sha256": metadata.patch_sha256,
        "smros_commit": metadata.smros_commit,
    }
    if allow_unknown_provenance:
        provenance = {
            key: "0" * (40 if key in {"revision", "smros_commit"} else 64)
            for key in provenance
        }
    if (
        any(value[key] != expected for key, expected in provenance.items())
        or (
            allow_unknown_provenance
            and value["build_id"] != "0" * 64
        )
    ):
        raise ValueError(f"runtime terminal provenance mismatch at line {line_number}")
    for key in _RUNTIME_TERMINAL_OPTIONAL_FIELDS & fields_present:
        item = value[key]
        if key in {"boot_count", "restart_count"}:
            if type(item) is not int or item < 0:
                raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")
        elif item is not None and not _is_strict_utf8_text(item):
            raise ValueError(f"runtime terminal {key} is invalid at line {line_number}")


def _load_runtime_results(
    path: Path,
    tests: Sequence[SuiteTest],
    build_results: Sequence[BuildResult],
    metadata: ManifestMetadata,
    *,
    role: str,
    maximum_bytes: int = _MAX_RUNTIME_RESULTS_BYTES,
    data: bytes | None = None,
) -> _RuntimeInput:
    maximum = min(_MAX_RUNTIME_RESULTS_BYTES, maximum_bytes)
    if data is None:
        data = _read_regular(path, "runtime results", maximum)
    elif len(data) > maximum:
        raise ValueError("runtime results exceeds its size limit")
    if not data or not data.endswith(b"\n") or b"\r" in data:
        raise ValueError("runtime results must be nonempty canonical LF NDJSON")
    tests_by_id = {test.test_id: test for test in tests}
    build_by_identity = {
        (result.test_id, result.stage): result for result in build_results
    }
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
            attempts.append(
                _validate_attempt(
                    value,
                    tests_by_id,
                    build_by_identity,
                    metadata,
                    line_number,
                    role=role,
                )
            )
        elif value.get("record_type") == "run":
            _validate_terminal(value, metadata, attempts, line_number)
            terminal = value
        else:
            raise ValueError(f"unknown runtime result record at line {line_number}")
        rows.append(value)
    if terminal is None:
        raise ValueError("runtime results lack a terminal run record")
    if role == "linux":
        if (
            terminal["platform"] != LINUX_REFERENCE_PLATFORM
            or terminal["source"] != LINUX_REFERENCE_SOURCE
        ):
            raise ValueError(
                "Linux-reference platform/source does not match its input role"
            )
    elif role == "smros":
        if (
            terminal["platform"] != SMROS_PLATFORM
            or terminal["source"] not in SMROS_SOURCES
        ):
            raise ValueError("SMROS platform/source does not match its input role")
    else:
        raise AssertionError(f"unknown runtime input role: {role}")
    return _RuntimeInput(
        path=str(Path(os.path.abspath(path))),
        byte_count=len(data),
        attempts=tuple(attempts),
        terminal=dict(terminal),
        rows=tuple(dict(row) for row in rows),
    )


def _load_serial_results(
    path: Path,
    text: str,
    manifest: _ManifestInput,
    *,
    byte_count: int,
) -> _RuntimeInput:
    parsed = parse_serial_log(text)
    if not is_valid_run_id(parsed.run_id):
        raise ValueError("serial run ID is invalid")
    if parsed.infrastructure_error is not None and not _is_strict_utf8_text(
        parsed.infrastructure_error,
        nonempty=True,
    ):
        raise ValueError("serial terminal infrastructure error is invalid")
    preflight_error = (
        len(parsed.events) == 1
        and parsed.events[0].event == "infrastructure_error"
        and parsed.manifest_sha256 == "0" * 64
        and not parsed.attempts
    )
    if (
        not preflight_error
        and parsed.manifest_sha256 != manifest.metadata.manifest_sha256
    ):
        raise ValueError("serial event manifest provenance mismatch")
    suite_start = None if preflight_error else parsed.events[0].values
    required_provenance = {
        "build_id": _is_digest,
        "build_results_sha256": _is_digest,
        "revision": _is_commit,
        "patch_sha256": _is_digest,
        "smros_commit": _is_commit,
    }
    if suite_start is not None:
        for key, validator in required_provenance.items():
            if not validator(suite_start.get(key)):
                raise ValueError(f"serial suite_start {key} is invalid")
    metadata = manifest.metadata
    if suite_start is not None and (
        suite_start["build_results_sha256"] != metadata.build_results_sha256
        or suite_start["revision"] != metadata.revision
        or suite_start["patch_sha256"] != metadata.patch_sha256
        or suite_start["smros_commit"] != metadata.smros_commit
    ):
        raise ValueError("serial suite_start provenance mismatch")
    tests_by_id = {test.test_id: test for test in manifest.tests}
    build_by_identity = {
        (result.test_id, result.stage): result for result in manifest.build_results
    }
    attempts: list[RuntimeAttempt] = []
    for serial_attempt in parsed.attempts:
        for key in ("stdout", "stderr"):
            if not _is_strict_utf8_text(getattr(serial_attempt, key)):
                raise ValueError(f"serial attempt {key} is invalid")
        for key in ("launch_error", "infrastructure_error"):
            item = getattr(serial_attempt, key)
            if item is not None and not _is_strict_utf8_text(item):
                raise ValueError(f"serial attempt {key} is invalid")
        test = tests_by_id.get(serial_attempt.test_id)
        if test is None:
            raise ValueError(f"unknown serial event test ID: {serial_attempt.test_id}")
        if serial_attempt.group != test.group or serial_attempt.api != test.api:
            raise ValueError(f"serial event dimensions mismatch: {test.test_id}")
        compile_result = build_by_identity.get((test.test_id, "compile"))
        link_result = build_by_identity.get((test.test_id, "link"))
        attempts.append(
            RuntimeAttempt(
                test_id=test.test_id,
                group=test.group,
                api=test.api,
                platform=SMROS_PLATFORM,
                build_status=(
                    compile_result.status if compile_result is not None else "not-built"
                ),
                link_status=(
                    link_result.status if link_result is not None else "not-linked"
                ),
                launch_status=serial_attempt.launch_status,
                pts_status=serial_attempt.pts_status,
                status=serial_attempt.status,
                exit_code=serial_attempt.exit_code,
                signal=serial_attempt.signal,
                timed_out=serial_attempt.timed_out,
                duration_ms=serial_attempt.duration_ms,
                stdout=serial_attempt.stdout,
                stderr=serial_attempt.stderr,
                source=SMROS_SERIAL_SOURCE,
                launch_error=serial_attempt.launch_error,
                infrastructure_error=serial_attempt.infrastructure_error,
                stdout_bytes=len(serial_attempt.stdout.encode("utf-8")),
                stderr_bytes=len(serial_attempt.stderr.encode("utf-8")),
                manifest_sha256=metadata.manifest_sha256,
                build_results_sha256=metadata.build_results_sha256,
                build_id=str(suite_start["build_id"]),
                revision=metadata.revision,
                patch_sha256=metadata.patch_sha256,
                smros_commit=metadata.smros_commit,
                binary_sha256=test.sha256 or "0" * 64,
                runtime_snapshot_sha256="0" * 64,
                run_id=parsed.run_id,
                resource_deltas=serial_attempt.resource_deltas,
                resource_evidence=serial_attempt.resource_evidence,
            )
        )
    selected_count = 0 if suite_start is None else suite_start["selected_count"]
    assert type(selected_count) is int
    unknown_digest = "0" * 64
    unknown_commit = "0" * 40
    terminal: dict[str, object] = {
        "build_id": unknown_digest if suite_start is None else suite_start["build_id"],
        "build_results_sha256": (
            unknown_digest if suite_start is None else metadata.build_results_sha256
        ),
        "complete": parsed.complete,
        "completed_count": len(attempts),
        "manifest_sha256": (
            unknown_digest if suite_start is None else metadata.manifest_sha256
        ),
        "patch_sha256": (
            unknown_digest if suite_start is None else metadata.patch_sha256
        ),
        "platform": SMROS_PLATFORM,
        "record_type": "run",
        "revision": unknown_commit if suite_start is None else metadata.revision,
        "run_id": parsed.run_id,
        "selected_count": selected_count,
        "smros_commit": (
            unknown_commit if suite_start is None else metadata.smros_commit
        ),
        "source": SMROS_SERIAL_SOURCE,
        "status_counts": dict(
            sorted(Counter(attempt.status for attempt in attempts).items())
        ),
    }
    if parsed.infrastructure_error is not None:
        terminal["infrastructure_error"] = parsed.infrastructure_error
    _validate_terminal(
        terminal,
        metadata,
        attempts,
        len(parsed.events) + 1,
        allow_unknown_provenance=preflight_error,
    )
    return _RuntimeInput(
        path=str(Path(os.path.abspath(path))),
        byte_count=byte_count,
        attempts=tuple(attempts),
        terminal=terminal,
        rows=tuple(event.to_dict() for event in parsed.events),
    )


def _load_smros_results(
    path: Path,
    manifest: _ManifestInput,
    *,
    maximum_bytes: int = _MAX_RUNTIME_RESULTS_BYTES,
) -> _RuntimeInput:
    maximum = min(_MAX_RUNTIME_RESULTS_BYTES, maximum_bytes)
    data = _read_regular(path, "SMROS results", maximum)
    try:
        text = data.decode("utf-8")
    except UnicodeError as error:
        raise ValueError("SMROS results are not UTF-8") from error
    if any(line.startswith(EVENT_PREFIX) for line in text.splitlines()):
        return _load_serial_results(
            path,
            text,
            manifest,
            byte_count=len(data),
        )
    return _load_runtime_results(
        path,
        manifest.tests,
        manifest.build_results,
        manifest.metadata,
        role="smros",
        maximum_bytes=maximum,
        data=data,
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
    program = [
        test
        for test in tests
        if _complete_test(test) or test.disposition == "definition-only"
    ]
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
        for test in program
        if _test_status(
            test, build_by_identity, primary_attempts.get(test.test_id, ())
        )
        == "pass"
    ]
    statuses = Counter(
        _test_status(test, build_by_identity, primary_attempts.get(test.test_id, ()))
        for test in tests
    )
    resource_leaks = {
        name: sum(
            any(
                getattr(attempt.resource_deltas, name) > 0
                for attempt in primary_attempts.get(test.test_id, ())
            )
            for test in tests
        )
        for name in RESOURCE_DELTA_NAMES
    }
    leaked = sum(
        any(
            attempt.resource_deltas.has_positive()
            for attempt in primary_attempts.get(test.test_id, ())
        )
        for test in tests
    )
    resource_deltas = _sum_resource_deltas(
        attempt.resource_deltas
        for test in tests
        for attempt in primary_attempts.get(test.test_id, ())
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
            (test.test_id for test in program),
            (test.test_id for test in passed),
        ),
    }
    return {
        "counts": counts,
        "metrics": metrics,
        "resource_deltas": resource_deltas.to_dict(),
        "resource_leaks": resource_leaks,
    }


def _sum_resource_deltas(values: Iterable[ResourceDeltas]) -> ResourceDeltas:
    totals = {name: 0 for name in RESOURCE_DELTA_NAMES}
    for value in values:
        for name in RESOURCE_DELTA_NAMES:
            totals[name] += getattr(value, name)
    return ResourceDeltas.from_mapping(totals)


def _resource_text(value: Mapping[str, object]) -> str:
    parts = [
        f"{name}={value[name]}"
        for name in RESOURCE_DELTA_NAMES
        if value.get(name) != 0
    ]
    return ", ".join(parts) if parts else "none"


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
    runtime_inputs = (*linux_inputs, *smros_inputs)
    run_identities: set[tuple[str, str]] = set()
    for source in runtime_inputs:
        platform = source.terminal["platform"]
        run_id = source.terminal["run_id"]
        assert isinstance(platform, str) and isinstance(run_id, str)
        identity = (platform, run_id)
        if identity in run_identities:
            raise ValueError(
                "duplicate runtime run identity: "
                f"platform={identity[0]} run_id={identity[1]}"
            )
        run_identities.add(identity)
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
        primary_resources = _sum_resource_deltas(
            attempt.resource_deltas for attempt in primary_attempts
        )
        evidence_values = {
            attempt.resource_evidence for attempt in primary_attempts
        }
        if evidence_values == {"measured"}:
            resource_evidence = "measured"
        elif not evidence_values or evidence_values == {"unavailable"}:
            resource_evidence = "unavailable"
        else:
            resource_evidence = "mixed"
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
                "resource_deltas": primary_resources.to_dict(),
                "resource_evidence": resource_evidence,
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
    complete = bool(runtime_inputs) and all(
        source.terminal["complete"] is True
        and not source.terminal.get("infrastructure_error")
        and not any(
            attempt.status == "interrupted"
            or (
                attempt.infrastructure_error
                and attempt.source != "host-watchdog"
            )
            for attempt in source.attempts
        )
        for source in runtime_inputs
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
        "resource_deltas": global_summary["resource_deltas"],
        "resource_leaks": global_summary["resource_leaks"],
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


def _xml_text(value: object) -> str:
    result: list[str] = []
    for character in str(value):
        codepoint = ord(character)
        if (
            codepoint in {0x09, 0x0A, 0x0D}
            or 0x20 <= codepoint <= 0xD7FF
            or 0xE000 <= codepoint <= 0xFFFD
            or 0x10000 <= codepoint <= 0x10FFFF
        ):
            result.append(character)
        else:
            result.append("\ufffd")
    return "".join(result)


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
                "classname": _xml_text(f"{row['group']}.{row['api']}"),
                "name": _xml_text(row["test_id"]),
                "time": f"{int(durations['runtime']) / 1000:.3f}",
            },
        )
        if row["disposition"] == "excluded-upstream-stub":
            ET.SubElement(case, "skipped", {"message": "audited upstream stub"})
        elif row["status"] != "pass":
            failure = ET.SubElement(
                case, "failure", {"message": _xml_text(row["status"])}
            )
            failure.text = _xml_text(row["failure_output"])
        attempts = row["attempts"]
        assert isinstance(attempts, list)
        output = "".join(str(attempt.get("stdout", "")) for attempt in attempts)
        if output:
            ET.SubElement(case, "system-out").text = _xml_text(output)
        errors = "".join(str(attempt.get("stderr", "")) for attempt in attempts)
        if errors:
            ET.SubElement(case, "system-err").text = _xml_text(errors)
        resource_deltas = row["resource_deltas"]
        assert isinstance(resource_deltas, dict)
        nonzero = {
            name: resource_deltas[name]
            for name in RESOURCE_DELTA_NAMES
            if resource_deltas.get(name) != 0
        }
        if nonzero:
            properties = ET.SubElement(case, "properties")
            for name, value in nonzero.items():
                ET.SubElement(
                    properties,
                    "property",
                    {
                        "name": _xml_text(f"resource_delta.{name}"),
                        "value": _xml_text(value),
                    },
                )
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
        *(f"resource_delta_{name}" for name in RESOURCE_DELTA_NAMES),
        *(f"resource_leak_{name}" for name in RESOURCE_DELTA_NAMES),
    ]
    stream = io.StringIO(newline="")
    writer = csv.DictWriter(stream, fieldnames=columns, lineterminator="\n")
    writer.writeheader()
    for name, scope in sorted(scopes.items()):
        assert isinstance(scope, dict)
        counts = scope["counts"]
        metrics = scope["metrics"]
        resources = scope["resource_deltas"]
        resource_leaks = scope["resource_leaks"]
        assert (
            isinstance(counts, dict)
            and isinstance(metrics, dict)
            and isinstance(resources, dict)
            and isinstance(resource_leaks, dict)
        )
        row: dict[str, object] = {label: name}
        for column in columns[1:]:
            if column in metrics:
                metric = metrics[column]
                assert isinstance(metric, dict)
                row[column] = metric["fraction"]
            elif column.startswith("resource_delta_"):
                row[column] = resources[column.removeprefix("resource_delta_")]
            elif column.startswith("resource_leak_"):
                row[column] = resource_leaks[column.removeprefix("resource_leak_")]
            else:
                row[column] = counts.get(column, 0)
        writer.writerow(row)
    return stream.getvalue().encode("utf-8")


def _markdown_escape(value: object) -> str:
    text = str(value).replace("\r\n", "\n").replace("\r", "\n")
    text = text.replace("\\", "\\\\")
    for character in "`*{}_[]()#+-.!|":
        text = text.replace(character, "\\" + character)
    return html.escape(text, quote=True).replace("\n", "<br>")


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
            "| Test ID | Group | API | Disposition | Status | Linux delta | Resource evidence | Resource deltas | Failure output |",
            "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
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
                    "resource_evidence",
                )
            )
            + " | "
            + _markdown_escape(_resource_text(row["resource_deltas"]))
            + " | "
            + _markdown_escape(row["failure_output"])
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
        "Resource evidence",
        "Resource deltas",
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
            "resource_evidence",
        ):
            _append_text(tr, "td", row[key])
        _append_text(tr, "td", _resource_text(row["resource_deltas"]))
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


def _directory_entry_matches(
    parent: int,
    name: str,
    descriptor: int,
) -> bool:
    try:
        entry = os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return False
    held = os.fstat(descriptor)
    return stat.S_ISDIR(entry.st_mode) and (entry.st_dev, entry.st_ino) == (
        held.st_dev,
        held.st_ino,
    )


def _require_private_directory(descriptor: int, label: str) -> None:
    info = os.fstat(descriptor)
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError(f"{label} ownership or mode is unsafe")


def _open_or_create_private_directory(
    parent: int,
    name: str,
    label: str,
) -> int:
    try:
        os.mkdir(name, 0o700, dir_fd=parent)
    except FileExistsError:
        pass
    descriptor = _open_directory_at(parent, name, label)
    try:
        _require_private_directory(descriptor, label)
        if not _directory_entry_matches(parent, name, descriptor):
            raise ValueError(f"{label} changed while being opened")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _require_empty_directory(descriptor: int, label: str) -> None:
    with os.scandir(descriptor) as entries:
        if next(entries, None) is not None:
            raise ValueError(f"{label} must be empty before reuse")


def _report_work_slot_name(destination_name: str) -> str:
    digest = hashlib.sha256(os.fsencode(destination_name)).hexdigest()
    return f"report-{digest}"


def _open_report_work_slot(parent: int, destination_name: str) -> tuple[int, int]:
    quarantine = _open_or_create_private_directory(
        parent,
        _REPORT_QUARANTINE_NAME,
        "report quarantine",
    )
    slot: int | None = None
    work: int | None = None
    try:
        slot_name = _report_work_slot_name(destination_name)
        slot = _open_or_create_private_directory(
            quarantine,
            slot_name,
            "report work slot",
        )
        fcntl.flock(slot, fcntl.LOCK_EX)
        if not _directory_entry_matches(
            parent, _REPORT_QUARANTINE_NAME, quarantine
        ):
            raise ValueError("report quarantine changed while locking work slot")
        if not _directory_entry_matches(quarantine, slot_name, slot):
            raise ValueError("report work slot changed while being locked")
        work = _open_or_create_private_directory(
            slot,
            _REPORT_WORK_ROOT_NAME,
            "report work root",
        )
        _require_empty_directory(work, "report work root")
        os.fsync(parent)
        os.fsync(quarantine)
        os.fsync(slot)
        return slot, work
    except BaseException:
        if work is not None:
            os.close(work)
        if slot is not None:
            os.close(slot)
        raise
    finally:
        os.close(quarantine)


def _create_report_work_root(slot: int) -> int:
    try:
        os.mkdir(_REPORT_WORK_ROOT_NAME, 0o700, dir_fd=slot)
    except FileExistsError as error:
        raise ValueError("concurrent report work root appeared") from error
    work = _open_directory_at(slot, _REPORT_WORK_ROOT_NAME, "report work root")
    try:
        _require_private_directory(work, "report work root")
        _require_empty_directory(work, "report work root")
        if not _directory_entry_matches(slot, _REPORT_WORK_ROOT_NAME, work):
            raise ValueError("report work root changed while being created")
        os.fsync(slot)
        return work
    except BaseException:
        os.close(work)
        raise


def _reset_report_work_root(slot: int, work: int) -> None:
    _clear_directory(work)
    os.fchmod(work, 0o700)
    os.fsync(work)
    if not _directory_entry_matches(slot, _REPORT_WORK_ROOT_NAME, work):
        raise ValueError("report work root changed during cleanup")
    os.fsync(slot)
    if not _directory_entry_matches(slot, _REPORT_WORK_ROOT_NAME, work):
        raise ValueError("report work root changed during cleanup")


def _rename_between(
    source_parent: int,
    source_name: str,
    destination_parent: int,
    destination_name: str,
    flags: int,
) -> None:
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
    if renameat2(
        source_parent,
        os.fsencode(source_name),
        destination_parent,
        os.fsencode(destination_name),
        flags,
    ) != 0:
        error_number = ctypes.get_errno()
        if error_number in {errno.ENOSYS, errno.EINVAL, errno.ENOTSUP}:
            raise ValueError(
                "atomic report directory replacement is unavailable"
            )
        raise OSError(error_number, os.strerror(error_number), destination_name)


def _rename_noreplace(
    source_parent: int,
    source_name: str,
    destination_parent: int,
    destination_name: str,
) -> None:
    _rename_between(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
        1,
    )


def _rename_exchange(
    source_parent: int,
    source_name: str,
    destination_parent: int,
    destination_name: str,
) -> None:
    _rename_between(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
        2,
    )


def _publish_generation(output_directory: Path, outputs: Mapping[str, bytes]) -> None:
    if tuple(outputs) != OUTPUT_NAMES:
        raise AssertionError("report generation is missing an output")
    if not output_directory.name or output_directory.name in {".", ".."}:
        raise ValueError("report output directory is invalid")
    parent = _open_directory_chain(
        output_directory.parent, "report output parent", create=True
    )
    slot: int | None = None
    work: int | None = None
    generated_in_slot = False
    operation_error: BaseException | None = None
    try:
        slot, work = _open_report_work_slot(parent, output_directory.name)
        generated_in_slot = True
        for name in OUTPUT_NAMES:
            descriptor = os.open(
                name,
                os.O_WRONLY
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                0o644,
                dir_fd=work,
            )
            try:
                _write_all(descriptor, outputs[name])
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        os.fsync(work)
        try:
            destination = os.stat(
                output_directory.name, dir_fd=parent, follow_symlinks=False
            )
        except FileNotFoundError:
            destination = None
        if destination is None:
            try:
                _rename_noreplace(
                    slot,
                    _REPORT_WORK_ROOT_NAME,
                    parent,
                    output_directory.name,
                )
            except FileExistsError as error:
                raise ValueError(
                    "concurrent destination appeared during report publication"
                ) from error
            except BaseException as publication_error:
                if _directory_entry_matches(parent, output_directory.name, work):
                    generated_in_slot = False
                    try:
                        os.fsync(slot)
                        os.fsync(parent)
                        replacement_work = _create_report_work_root(slot)
                        os.close(replacement_work)
                    except BaseException as cleanup_error:
                        raise BaseExceptionGroup(
                            "report publication and cleanup both failed",
                            [publication_error, cleanup_error],
                        )
                raise
            generated_in_slot = False
            os.fsync(slot)
            os.fsync(parent)
            if not _directory_entry_matches(parent, output_directory.name, work):
                raise ValueError("report output directory changed during publication")
            replacement_work = _create_report_work_root(slot)
            os.close(replacement_work)
        else:
            if stat.S_ISLNK(destination.st_mode):
                raise ValueError("report output directory must not be a symlink")
            if not stat.S_ISDIR(destination.st_mode):
                raise ValueError("report output destination is not a directory")
            held_destination = _open_directory_at(
                parent, output_directory.name, "report output directory"
            )
            try:
                if not _directory_entry_matches(
                    parent, output_directory.name, held_destination
                ):
                    raise ValueError("report output directory changed before publication")
                try:
                    _rename_exchange(
                        slot,
                        _REPORT_WORK_ROOT_NAME,
                        parent,
                        output_directory.name,
                    )
                    os.fsync(slot)
                    os.fsync(parent)
                    if not _directory_entry_matches(
                        parent, output_directory.name, work
                    ):
                        raise ValueError(
                            "report output directory changed during publication"
                        )
                    if not _directory_entry_matches(
                        slot, _REPORT_WORK_ROOT_NAME, held_destination
                    ):
                        publication_error = ValueError(
                            "report output directory changed during publication"
                        )
                        displaced = os.stat(
                            _REPORT_WORK_ROOT_NAME,
                            dir_fd=slot,
                            follow_symlinks=False,
                        )
                        displaced_identity = (displaced.st_dev, displaced.st_ino)
                        try:
                            if not _directory_entry_matches(
                                parent, output_directory.name, work
                            ):
                                raise ValueError(
                                    "generated report changed before publication rollback"
                                )
                            _rename_exchange(
                                slot,
                                _REPORT_WORK_ROOT_NAME,
                                parent,
                                output_directory.name,
                            )
                            os.fsync(slot)
                            os.fsync(parent)
                            if not _directory_entry_matches(
                                slot, _REPORT_WORK_ROOT_NAME, work
                            ):
                                raise ValueError(
                                    "publication rollback did not restore the generated report"
                                )
                            restored = os.stat(
                                output_directory.name,
                                dir_fd=parent,
                                follow_symlinks=False,
                            )
                            if (
                                restored.st_dev,
                                restored.st_ino,
                            ) != displaced_identity:
                                raise ValueError(
                                    "publication rollback did not restore the raced destination"
                                )
                            generated_in_slot = True
                        except BaseException as rollback_error:
                            raise BaseExceptionGroup(
                                "report publication and rollback both failed",
                                [publication_error, rollback_error],
                            )
                        raise publication_error
                    _reset_report_work_root(slot, held_destination)
                except BaseException as publication_error:
                    if _directory_entry_matches(
                        parent, output_directory.name, work
                    ):
                        generated_in_slot = False
                        if _directory_entry_matches(
                            slot,
                            _REPORT_WORK_ROOT_NAME,
                            held_destination,
                        ):
                            try:
                                os.fsync(slot)
                                os.fsync(parent)
                                if not _directory_entry_matches(
                                    parent, output_directory.name, work
                                ):
                                    raise ValueError(
                                        "generated report changed during publication recovery"
                                    )
                                if not _directory_entry_matches(
                                    slot,
                                    _REPORT_WORK_ROOT_NAME,
                                    held_destination,
                                ):
                                    raise ValueError(
                                        "prior report changed during publication recovery"
                                    )
                                _reset_report_work_root(
                                    slot, held_destination
                                )
                            except BaseException as cleanup_error:
                                raise BaseExceptionGroup(
                                    "report publication and cleanup both failed",
                                    [publication_error, cleanup_error],
                                )
                    raise
                generated_in_slot = False
            finally:
                os.close(held_destination)
        os.fsync(parent)
    except BaseException as error:
        operation_error = error
    cleanup_error: BaseException | None = None
    if work is not None and slot is not None:
        try:
            if _directory_entry_matches(slot, _REPORT_WORK_ROOT_NAME, work):
                _reset_report_work_root(slot, work)
            elif _directory_entry_matches(parent, output_directory.name, work):
                os.fsync(slot)
                os.fsync(parent)
                if not _directory_entry_matches(
                    parent, output_directory.name, work
                ):
                    raise ValueError(
                        "published report changed during cleanup reconciliation"
                    )
                try:
                    os.stat(
                        _REPORT_WORK_ROOT_NAME,
                        dir_fd=slot,
                        follow_symlinks=False,
                    )
                except FileNotFoundError:
                    replacement_work = _create_report_work_root(slot)
                    os.close(replacement_work)
            else:
                raise ValueError(
                    "held generated report inode is not at output or work slot"
                )
        except BaseException as error:
            cleanup_error = error
    for descriptor in (work, slot, parent):
        if descriptor is not None:
            try:
                os.close(descriptor)
            except BaseException as error:
                if cleanup_error is None:
                    cleanup_error = error
    if operation_error is not None:
        if cleanup_error is not None:
            raise BaseExceptionGroup(
                "report publication and cleanup both failed",
                [operation_error, cleanup_error],
            )
        raise operation_error
    if cleanup_error is not None:
        raise cleanup_error


def _runtime_paths(
    linux_results: Sequence[Path] | Path | None,
    smros_results: Sequence[Path] | Path | None,
) -> tuple[tuple[str, Path], ...]:
    entries: list[tuple[str, Path]] = []
    seen: set[Path] = set()
    for role, value in (
        ("linux", linux_results),
        ("smros", smros_results),
    ):
        if value is None:
            continue
        paths: Iterable[Path] = (value,) if isinstance(value, Path) else value
        for item in paths:
            if len(entries) >= _MAX_RUNTIME_INPUTS:
                raise ValueError("runtime result input count exceeds its limit")
            path = Path(os.path.abspath(Path(item)))
            if path in seen:
                raise ValueError(f"duplicate runtime result path: {path}")
            seen.add(path)
            entries.append((role, path))
    return tuple(entries)


def generate_report(
    manifest_path: Path,
    *,
    linux_results: Sequence[Path] | Path | None = None,
    smros_results: Sequence[Path] | Path | None = None,
    output_directory: Path,
) -> dict[str, object]:
    """Validate all inputs, aggregate them, and publish one report generation."""
    runtime_paths = _runtime_paths(linux_results, smros_results)
    if not runtime_paths:
        raise ValueError("at least one runtime-result input is required")
    manifest = _load_manifest(Path(manifest_path))
    inputs: dict[str, list[_RuntimeInput]] = {"linux": [], "smros": []}
    run_identities: set[tuple[object, object]] = set()
    total_bytes = 0
    for role, path in runtime_paths:
        remaining = _MAX_RUNTIME_INPUT_BYTES - total_bytes
        if remaining <= 0:
            raise ValueError("runtime result inputs exceed cumulative byte limit")
        maximum = min(_MAX_RUNTIME_RESULTS_BYTES, remaining)
        try:
            if role == "linux":
                source = _load_runtime_results(
                    path,
                    manifest.tests,
                    manifest.build_results,
                    manifest.metadata,
                    role=role,
                    maximum_bytes=maximum,
                )
            else:
                source = _load_smros_results(
                    path,
                    manifest,
                    maximum_bytes=maximum,
                )
        except ValueError as error:
            if remaining < _MAX_RUNTIME_RESULTS_BYTES and str(error) in {
                "runtime results exceeds its size limit",
                "SMROS results exceeds its size limit",
            }:
                raise ValueError(
                    "runtime result inputs exceed cumulative byte limit"
                ) from error
            raise
        total_bytes += source.byte_count
        identity = (source.terminal["platform"], source.terminal["run_id"])
        if identity in run_identities:
            raise ValueError(
                "duplicate runtime run identity: "
                f"platform={identity[0]} run_id={identity[1]}"
            )
        run_identities.add(identity)
        inputs[role].append(source)
    linux_inputs = tuple(inputs["linux"])
    smros_inputs = tuple(inputs["smros"])
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
