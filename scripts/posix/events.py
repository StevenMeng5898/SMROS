"""Strict parser for versioned SMROS POSIX serial events."""

from __future__ import annotations

from collections import Counter
import json
import re
from typing import Mapping

from .model import (
    OVERALL_STATUSES,
    ParsedEventRun,
    RAW_RUNTIME_STATUSES,
    ResourceDeltas,
    SerialAttempt,
    SerialEvent,
    validate_raw_attempt_semantics,
)


EVENT_PREFIX = "SMROS_POSIX_EVENT "
_DIGEST_RE = re.compile(r"[0-9a-f]{64}\Z")
_PREFLIGHT_RUN_ID_RE = re.compile(r"error-[0-9]+\Z")
_EMPTY_SHA256 = "0" * 64
_EVENT_NAMES = frozenset(
    {
        "suite_start",
        "test_start",
        "test_end",
        "suite_end",
        "infrastructure_error",
    }
)
_COMMON_FIELDS = {
    "schema",
    "seq",
    "event",
    "run_id",
    "manifest_sha256",
    "architecture",
}
_ALLOWED_FIELDS = {
    "suite_start": _COMMON_FIELDS
    | {
        "selected_count",
        "build_id",
        "build_results_sha256",
        "smros_commit",
        "revision",
        "patch_sha256",
        "boot_id",
        "filter",
        "started_ticks",
        "source",
    },
    "test_start": _COMMON_FIELDS
    | {
        "test_id",
        "group",
        "api",
        "build_status",
        "link_status",
        "binary_sha256",
        "source",
        "started_ticks",
    },
    "test_end": _COMMON_FIELDS
    | {
        "test_id",
        "group",
        "api",
        "status",
        "pts_status",
        "launch_status",
        "exit_code",
        "signal",
        "timed_out",
        "duration_ms",
        "elapsed_ticks",
        "stdout",
        "stderr",
        "launch_error",
        "infrastructure_error",
        "resource_deltas",
        "processes_delta",
        "scheduler_threads_delta",
        "linux_mappings_delta",
        "linux_fds_delta",
        "linux_shared_memory_delta",
        "kernel_handles_delta",
    },
    "suite_end": _COMMON_FIELDS
    | {
        "complete",
        "selected_count",
        "completed_count",
        "status_counts",
        "duration_ms",
        "elapsed_ticks",
    },
    "infrastructure_error": _COMMON_FIELDS
    | {"message", "detail", "test_id", "group", "api"},
}


def _reject_duplicate_keys(
    pairs: list[tuple[str, object]],
) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _require_string(value: Mapping[str, object], key: str) -> str:
    item = value.get(key)
    if not isinstance(item, str) or not item:
        raise ValueError(f"event {key} is invalid")
    return item


def _require_nonnegative_int(value: object, label: str) -> int:
    if type(value) is not int or value < 0:
        raise ValueError(f"event {label} is invalid")
    return value


def _infrastructure_error_detail(value: Mapping[str, object]) -> str:
    detail = value.get("detail", value.get("message"))
    if not isinstance(detail, str) or not detail:
        raise ValueError("infrastructure error detail is invalid")
    return detail


def _validate_common(value: object, line_number: int) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError(f"event at line {line_number} is not an object")
    event_name = value.get("event")
    if event_name not in _EVENT_NAMES:
        raise ValueError(f"unknown event at line {line_number}")
    if set(value) - _ALLOWED_FIELDS[str(event_name)]:
        raise ValueError(f"event schema has unknown fields at line {line_number}")
    if type(value.get("schema")) is not int or value.get("schema") != 1:
        raise ValueError(f"event schema is not 1 at line {line_number}")
    _require_nonnegative_int(value.get("seq"), "sequence")
    _require_string(value, "run_id")
    digest = _require_string(value, "manifest_sha256")
    if _DIGEST_RE.fullmatch(digest) is None:
        raise ValueError(f"event manifest checksum is invalid at line {line_number}")
    if _require_string(value, "architecture") != "aarch64":
        raise ValueError(f"event architecture is invalid at line {line_number}")
    return value


def _serial_event(value: dict[str, object]) -> SerialEvent:
    return SerialEvent(
        schema=1,
        seq=int(value["seq"]),
        event=str(value["event"]),
        run_id=str(value["run_id"]),
        manifest_sha256=str(value["manifest_sha256"]),
        architecture=str(value["architecture"]),
        values=dict(value),
    )


def _resource_deltas(value: Mapping[str, object]) -> ResourceDeltas:
    raw = value.get("resource_deltas")
    if not isinstance(raw, dict):
        raise ValueError("event lacks complete resource evidence")
    try:
        return ResourceDeltas.from_complete_mapping(raw)
    except ValueError as error:
        raise ValueError("event lacks complete resource evidence") from error


def _attempt_from_end(
    start: SerialEvent,
    end: SerialEvent,
    output: list[str],
) -> SerialAttempt:
    start_value = start.values
    value = end.values
    for key in ("test_id", "group", "api"):
        if _require_string(value, key) != _require_string(start_value, key):
            label = "test ID" if key == "test_id" else key
            raise ValueError(f"event {label} mismatch")
    status = _require_string(value, "status")
    if status not in RAW_RUNTIME_STATUSES:
        raise ValueError(f"event status is invalid: {status}")
    pts_status = value.get("pts_status")
    if pts_status is not None and (
        not isinstance(pts_status, str)
        or pts_status not in OVERALL_STATUSES[:5]
    ):
        raise ValueError("event PTS status is invalid")
    launch_status = value.get("launch_status", "launched")
    if launch_status not in {
        "launched",
        "launch-error",
        "interrupted",
        "not-launched",
    }:
        raise ValueError("event launch status is invalid")
    exit_code = value.get("exit_code")
    signal = value.get("signal")
    if exit_code is not None and type(exit_code) is not int:
        raise ValueError("event exit code is invalid")
    if signal is not None and (type(signal) is not int or signal <= 0):
        raise ValueError("event signal is invalid")
    timed_out = value.get("timed_out", status == "timeout")
    if type(timed_out) is not bool:
        raise ValueError("event timeout flag is invalid")
    duration_ms = value.get("duration_ms", value.get("elapsed_ticks", 0))
    _require_nonnegative_int(duration_ms, "duration")
    stdout = value.get("stdout", "".join(output))
    stderr = value.get("stderr", "")
    if not isinstance(stdout, str) or not isinstance(stderr, str):
        raise ValueError("event output is invalid")
    launch_error = value.get("launch_error")
    infrastructure_error = value.get("infrastructure_error")
    if launch_error is not None and not isinstance(launch_error, str):
        raise ValueError("event launch error is invalid")
    if infrastructure_error is not None and not isinstance(
        infrastructure_error, str
    ):
        raise ValueError("event infrastructure error is invalid")
    validate_raw_attempt_semantics(
        status=status,
        pts_status=pts_status,
        launch_status=str(launch_status),
        exit_code=exit_code,
        signal=signal,
        timed_out=timed_out,
        launch_error=launch_error,
        infrastructure_error=infrastructure_error,
        label="event attempt",
    )
    return SerialAttempt(
        test_id=str(value["test_id"]),
        group=str(value["group"]),
        api=str(value["api"]),
        status=status,
        pts_status=pts_status,
        launch_status=str(launch_status),
        exit_code=exit_code,
        signal=signal,
        timed_out=timed_out,
        duration_ms=int(duration_ms),
        stdout=stdout,
        stderr=stderr,
        resource_deltas=_resource_deltas(value),
        resource_evidence="measured",
        run_id=end.run_id,
        manifest_sha256=end.manifest_sha256,
        architecture=end.architecture,
        launch_error=launch_error,
        infrastructure_error=infrastructure_error,
    )


def _interrupted_attempt(start: SerialEvent, output: list[str]) -> SerialAttempt:
    value = start.values
    return SerialAttempt(
        test_id=_require_string(value, "test_id"),
        group=_require_string(value, "group"),
        api=_require_string(value, "api"),
        status="interrupted",
        pts_status=None,
        launch_status="interrupted",
        exit_code=None,
        signal=None,
        timed_out=False,
        duration_ms=0,
        stdout="".join(output),
        stderr="",
        resource_deltas=ResourceDeltas(),
        resource_evidence="unavailable",
        run_id=start.run_id,
        manifest_sha256=start.manifest_sha256,
        architecture=start.architecture,
        infrastructure_error="serial event stream ended before test_end",
    )


def parse_serial_log(log: str) -> ParsedEventRun:
    """Parse one serial log, retaining output while making truncation explicit."""
    if not isinstance(log, str):
        raise TypeError("serial log must be text")
    events: list[SerialEvent] = []
    attempts: list[SerialAttempt] = []
    active: SerialEvent | None = None
    active_output: list[str] = []
    terminal: SerialEvent | None = None
    suite_start: SerialEvent | None = None
    previous_seq: int | None = None
    infrastructure_error: str | None = None

    for line_number, line in enumerate(log.splitlines(), start=1):
        if not line.startswith(EVENT_PREFIX):
            if active is not None and terminal is None:
                active_output.append(line + "\n")
            continue
        try:
            value = json.loads(
                line[len(EVENT_PREFIX) :],
                object_pairs_hook=_reject_duplicate_keys,
            )
        except (json.JSONDecodeError, ValueError) as error:
            raise ValueError(f"invalid event JSON at line {line_number}") from error
        value = _validate_common(value, line_number)
        event = _serial_event(value)
        if previous_seq is not None and event.seq <= previous_seq:
            raise ValueError(f"event sequence is not monotonically increasing at line {line_number}")
        previous_seq = event.seq
        if suite_start is not None and (
            event.run_id != suite_start.run_id
            or event.manifest_sha256 != suite_start.manifest_sha256
            or event.architecture != suite_start.architecture
        ):
            raise ValueError(f"event run ID or provenance mismatch at line {line_number}")
        if terminal is not None:
            if event.event in {"suite_end", "infrastructure_error"}:
                raise ValueError("duplicate terminal event")
            raise ValueError("event appears after terminal event")
        if event.event == "suite_start":
            if suite_start is not None or events:
                raise ValueError("duplicate or misplaced suite_start event")
            _require_nonnegative_int(value.get("selected_count"), "selected count")
            suite_start = event
        elif suite_start is None:
            if (
                event.event != "infrastructure_error"
                or events
                or event.seq != 1
                or _PREFLIGHT_RUN_ID_RE.fullmatch(event.run_id) is None
                or event.manifest_sha256 != _EMPTY_SHA256
            ):
                raise ValueError("event stream does not start with suite_start")
            infrastructure_error = _infrastructure_error_detail(value)
            terminal = event
        elif event.event == "test_start":
            if active is not None:
                raise ValueError("test_start appears while another test is active")
            for key in ("test_id", "group", "api"):
                _require_string(value, key)
            active = event
            active_output = []
        elif event.event == "test_end":
            if active is None:
                raise ValueError("test_end has no matching test_start")
            attempts.append(_attempt_from_end(active, event, active_output))
            active = None
            active_output = []
        elif event.event == "suite_end":
            if active is not None:
                raise ValueError("suite_end appears while a test is active")
            if type(value.get("complete")) is not bool:
                raise ValueError("event completion flag is invalid")
            selected = _require_nonnegative_int(
                value.get("selected_count"), "selected count"
            )
            completed = _require_nonnegative_int(
                value.get("completed_count"), "completed count"
            )
            if selected != suite_start.values["selected_count"]:
                raise ValueError("event selected count mismatch")
            if completed != len(attempts) or completed > selected:
                raise ValueError("event completed count mismatch")
            if value["complete"] is True and completed != selected:
                raise ValueError(
                    "complete event does not cover every selected attempt"
                )
            if value["complete"] is True and len(
                {attempt.test_id for attempt in attempts}
            ) != selected:
                raise ValueError(
                    "complete event does not contain unique selected attempts"
                )
            if value["complete"] is True and any(
                attempt.status == "interrupted" for attempt in attempts
            ):
                raise ValueError("complete event contains an interrupted attempt")
            if value["complete"] is True and any(
                attempt.infrastructure_error for attempt in attempts
            ):
                raise ValueError("complete event contains an infrastructure error")
            raw_counts = value.get("status_counts")
            expected_counts = dict(
                sorted(Counter(attempt.status for attempt in attempts).items())
            )
            if raw_counts != expected_counts:
                raise ValueError("event status counts mismatch")
            terminal = event
        elif event.event == "infrastructure_error":
            if active is not None:
                attempts.append(_interrupted_attempt(active, active_output))
                active = None
                active_output = []
            infrastructure_error = _infrastructure_error_detail(value)
            terminal = event
        events.append(event)

    if suite_start is None:
        if terminal is None:
            raise ValueError("serial log contains no POSIX suite_start event")
        return ParsedEventRun(
            events=tuple(events),
            attempts=(),
            run_id=terminal.run_id,
            manifest_sha256=terminal.manifest_sha256,
            architecture=terminal.architecture,
            complete=False,
            status="incomplete",
            terminal_event=terminal,
            infrastructure_error=infrastructure_error,
        )
    if active is not None:
        attempts.append(_interrupted_attempt(active, active_output))
    complete = bool(
        terminal is not None
        and terminal.event == "suite_end"
        and terminal.values.get("complete") is True
    )
    return ParsedEventRun(
        events=tuple(events),
        attempts=tuple(attempts),
        run_id=suite_start.run_id,
        manifest_sha256=suite_start.manifest_sha256,
        architecture=suite_start.architecture,
        complete=complete,
        status="complete" if complete else "incomplete",
        terminal_event=terminal,
        infrastructure_error=infrastructure_error,
    )
