"""Shared immutable records for POSIX suite host tooling."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Mapping


PTS_PASS = 0
PTS_FAIL = 1
PTS_UNRESOLVED = 2
PTS_UNSUPPORTED = 4
PTS_UNTESTED = 5

OVERALL_STATUSES = (
    "pass",
    "fail",
    "unresolved",
    "unsupported",
    "untested",
    "interrupted",
    "timeout",
    "crash",
    "launch-error",
    "build-fail",
    "not-built",
    "flaky",
)
RAW_RUNTIME_STATUSES = OVERALL_STATUSES[:9]
HOST_WATCHDOG_ERROR_MAX_BYTES = 4096
MAX_RUN_ID_BYTES = 256


def is_valid_run_id(value: object) -> bool:
    """Return whether a run ID is nonempty strict UTF-8 within its byte cap."""
    if not isinstance(value, str) or not value:
        return False
    try:
        return len(value.encode("utf-8")) <= MAX_RUN_ID_BYTES
    except UnicodeEncodeError:
        return False


def validate_raw_attempt_semantics(
    *,
    status: str,
    pts_status: str | None,
    launch_status: str,
    exit_code: int | None,
    signal: int | None,
    timed_out: bool,
    launch_error: str | None,
    infrastructure_error: str | None,
    label: str,
) -> None:
    """Reject cross-field contradictions in one observed runtime attempt."""
    if status not in RAW_RUNTIME_STATUSES:
        raise ValueError(f"{label} raw runtime status is invalid: {status}")
    if launch_status == "not-launched":
        coherent = (
            status == "untested"
            and pts_status is None
            and exit_code is None
            and signal is None
            and timed_out is False
            and launch_error is None
            and infrastructure_error is None
        )
    elif status == "interrupted":
        coherent = (
            launch_status == "interrupted"
            and pts_status is None
            and exit_code is None
            and signal is None
            and timed_out is False
            and launch_error is None
            and bool(infrastructure_error)
        )
    elif status == "launch-error":
        coherent = (
            launch_status == "launch-error"
            and pts_status is None
            and exit_code is None
            and signal is None
            and timed_out is False
            and bool(launch_error)
        )
    elif status == "timeout":
        coherent = (
            launch_status == "launched"
            and pts_status is None
            and exit_code is None
            and timed_out is True
            and launch_error is None
            and infrastructure_error is None
        )
    elif status == "crash":
        coherent = (
            launch_status == "launched"
            and pts_status is None
            and exit_code is None
            and signal is not None
            and signal > 0
            and timed_out is False
            and launch_error is None
            and infrastructure_error is None
        )
    else:
        expected_exit = {
            "pass": 0,
            "unresolved": 2,
            "unsupported": 4,
            "untested": 5,
        }.get(status)
        exit_matches = (
            exit_code == expected_exit
            if expected_exit is not None
            else (
                exit_code is not None
                and exit_code >= 0
                and exit_code not in {0, 2, 4, 5}
            )
        )
        coherent = (
            launch_status == "launched"
            and pts_status == status
            and exit_matches
            and signal is None
            and timed_out is False
            and launch_error is None
            and infrastructure_error is None
        )
    if not coherent:
        raise ValueError(f"{label} {status} dimensions are invalid")


def validate_host_watchdog_attempt_semantics(
    *,
    status: str,
    pts_status: str | None,
    launch_status: str,
    exit_code: int | None,
    signal: int | None,
    timed_out: bool,
    launch_error: str | None,
    infrastructure_error: str | None,
    label: str,
) -> None:
    """Reject guest-derived dimensions on a host watchdog result."""
    bounded_error = (
        isinstance(infrastructure_error, str)
        and bool(infrastructure_error)
        and len(infrastructure_error.encode("utf-8", errors="replace"))
        <= HOST_WATCHDOG_ERROR_MAX_BYTES
    )
    coherent = (
        status in {"timeout", "crash"}
        and launch_status in {"launched", "interrupted"}
        and pts_status is None
        and exit_code is None
        and signal is None
        and timed_out is (status == "timeout")
        and launch_error is None
        and bounded_error
    )
    if not coherent:
        raise ValueError(f"{label} host-watchdog dimensions are invalid")


@dataclass(frozen=True)
class SuiteTest:
    test_id: str
    group: str
    api: str
    kind: str
    disposition: str
    source: str
    binary: str | None
    sha256: str | None
    timeout_ms: int


@dataclass(frozen=True)
class BuildResult:
    test_id: str
    stage: str
    status: str
    argv: tuple[str, ...]
    returncode: int | None
    stdout: str
    stderr: str
    duration_ms: int
    artifact_sha256: str | None


@dataclass(frozen=True)
class BuildSummary:
    discovered: int
    compile_pass: int
    compile_fail: int
    link_pass: int
    link_fail: int
    shell_unported: int
    staged_bytes: int

    def format_counts(self) -> str:
        return (
            f"discovered={self.discovered} build-pass={self.compile_pass} "
            f"build-fail={self.compile_fail} link-pass={self.link_pass} "
            f"link-fail={self.link_fail} shell-unported={self.shell_unported} "
            f"staged-bytes={self.staged_bytes}"
        )


RESOURCE_DELTA_NAMES = (
    "aio_requests",
    "ipc_objects",
    "kernel_handles",
    "linux_fds",
    "linux_mappings",
    "linux_processes",
    "linux_shared_memory",
    "linux_zombies",
    "page_table_pages",
    "private_pages",
    "processes",
    "scheduler_threads",
    "shared_pages",
    "timers",
)


@dataclass(frozen=True)
class ResourceDeltas:
    aio_requests: int = 0
    ipc_objects: int = 0
    kernel_handles: int = 0
    linux_fds: int = 0
    linux_mappings: int = 0
    linux_processes: int = 0
    linux_shared_memory: int = 0
    linux_zombies: int = 0
    page_table_pages: int = 0
    private_pages: int = 0
    processes: int = 0
    scheduler_threads: int = 0
    shared_pages: int = 0
    timers: int = 0

    def __post_init__(self) -> None:
        for name in RESOURCE_DELTA_NAMES:
            value = getattr(self, name)
            if type(value) is not int or not -(2**63) <= value < 2**63:
                raise ValueError(f"resource delta {name} is outside signed 64-bit range")

    @classmethod
    def from_mapping(cls, value: Mapping[str, object]) -> ResourceDeltas:
        unknown = set(value) - set(RESOURCE_DELTA_NAMES)
        if unknown:
            raise ValueError(f"unknown resource delta: {sorted(unknown)[0]}")
        arguments = {name: value.get(name, 0) for name in RESOURCE_DELTA_NAMES}
        return cls(**arguments)  # type: ignore[arg-type]

    @classmethod
    def from_complete_mapping(cls, value: Mapping[str, object]) -> ResourceDeltas:
        if set(value) != set(RESOURCE_DELTA_NAMES):
            raise ValueError("resource evidence does not contain every resource class")
        return cls.from_mapping(value)

    def to_dict(self) -> dict[str, int]:
        return {name: getattr(self, name) for name in RESOURCE_DELTA_NAMES}

    def has_nonzero(self) -> bool:
        return any(getattr(self, name) != 0 for name in RESOURCE_DELTA_NAMES)

    def has_positive(self) -> bool:
        return any(getattr(self, name) > 0 for name in RESOURCE_DELTA_NAMES)


@dataclass(frozen=True)
class RuntimeAttempt:
    test_id: str
    group: str
    api: str
    platform: str
    build_status: str
    link_status: str
    launch_status: str
    pts_status: str | None
    status: str
    exit_code: int | None
    signal: int | None
    timed_out: bool
    duration_ms: int
    stdout: str
    stderr: str
    source: str
    launch_error: str | None = None
    infrastructure_error: str | None = None
    stdout_bytes: int = 0
    stderr_bytes: int = 0
    stdout_truncated: bool = False
    stderr_truncated: bool = False
    manifest_sha256: str = ""
    build_results_sha256: str = ""
    build_id: str = ""
    revision: str = ""
    patch_sha256: str = ""
    smros_commit: str = ""
    binary_sha256: str = ""
    runtime_snapshot_sha256: str = ""
    run_id: str = ""
    resource_deltas: ResourceDeltas = ResourceDeltas()
    resource_evidence: str = "unavailable"
    raw_log_start: int | None = None
    raw_log_end: int | None = None

    def to_dict(self) -> dict[str, object]:
        """Return every core and finalized runtime field for persistence."""
        return {
            "api": self.api,
            "binary_sha256": self.binary_sha256,
            "build_id": self.build_id,
            "build_results_sha256": self.build_results_sha256,
            "build_status": self.build_status,
            "duration_ms": self.duration_ms,
            "exit_code": self.exit_code,
            "group": self.group,
            "infrastructure_error": self.infrastructure_error,
            "launch_error": self.launch_error,
            "launch_status": self.launch_status,
            "link_status": self.link_status,
            "manifest_sha256": self.manifest_sha256,
            "patch_sha256": self.patch_sha256,
            "platform": self.platform,
            "pts_status": self.pts_status,
            "raw_log_end": self.raw_log_end,
            "raw_log_start": self.raw_log_start,
            "revision": self.revision,
            "resource_deltas": self.resource_deltas.to_dict(),
            "resource_evidence": self.resource_evidence,
            "runtime_snapshot_sha256": self.runtime_snapshot_sha256,
            "run_id": self.run_id,
            "signal": self.signal,
            "smros_commit": self.smros_commit,
            "source": self.source,
            "status": self.status,
            "stderr": self.stderr,
            "stderr_bytes": self.stderr_bytes,
            "stderr_truncated": self.stderr_truncated,
            "stdout": self.stdout,
            "stdout_bytes": self.stdout_bytes,
            "stdout_truncated": self.stdout_truncated,
            "test_id": self.test_id,
            "timed_out": self.timed_out,
        }


@dataclass(frozen=True)
class RunMetadata:
    run_id: str
    platform: str
    manifest_sha256: str
    build_id: str
    complete: bool


@dataclass(frozen=True)
class SerialEvent:
    schema: int
    seq: int
    event: str
    run_id: str
    manifest_sha256: str
    architecture: str
    values: Mapping[str, object]

    def to_dict(self) -> dict[str, object]:
        return dict(self.values)


@dataclass(frozen=True)
class SerialAttempt:
    test_id: str
    group: str
    api: str
    status: str
    pts_status: str | None
    launch_status: str
    exit_code: int | None
    signal: int | None
    timed_out: bool
    duration_ms: int
    stdout: str
    stderr: str
    resource_deltas: ResourceDeltas
    resource_evidence: str
    run_id: str
    manifest_sha256: str
    architecture: str
    launch_error: str | None = None
    infrastructure_error: str | None = None


@dataclass(frozen=True)
class ParsedEventRun:
    events: tuple[SerialEvent, ...]
    attempts: tuple[SerialAttempt, ...]
    run_id: str
    manifest_sha256: str
    architecture: str
    complete: bool
    status: str
    terminal_event: SerialEvent | None
    infrastructure_error: str | None = None


@dataclass(frozen=True)
class CoverageMetric:
    numerator: int
    denominator: int
    test_ids: tuple[str, ...]
    numerator_test_ids: tuple[str, ...]

    def to_dict(self) -> dict[str, object]:
        return {
            "denominator": self.denominator,
            "fraction": f"{self.numerator}/{self.denominator}",
            "numerator": self.numerator,
            "numerator_test_ids": list(self.numerator_test_ids),
            "test_ids": list(self.test_ids),
        }
