"""Collect isolated SMROS POSIX results from a persistent QEMU campaign."""

from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Sequence
from dataclasses import dataclass, fields
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import shutil
import stat
import subprocess
import time
from typing import Protocol

from .baseline import (
    _atomic_write,
    _canonical_report,
    _load_stage_identity,
    _validate_selected,
    filter_runnable_tests,
)
from .build import ManifestMetadata
from .events import EVENT_PREFIX, parse_serial_log
from .model import (
    BuildResult,
    ResourceDeltas,
    RuntimeAttempt,
    SerialAttempt,
    SuiteTest,
    validate_host_watchdog_attempt_semantics,
    validate_raw_attempt_semantics,
)


PLATFORM = "smros-aarch64"
SOURCE = "smros-qemu"
WATCHDOG_SOURCE = "host-watchdog"
PROMPT = b"smros:/> "
_EVENT_PREFIX_BYTES = EVENT_PREFIX.encode("ascii")
_FATAL_PATTERNS = (
    b"!!! KERNEL PANIC !!!",
    b"[PANIC]",
    b"[FATAL]",
    b"kernel panic",
    b"Kernel panic",
)
_STREAM_TOKEN_BYTES = max(len(PROMPT), *(len(item) for item in _FATAL_PATTERNS))
_READ_INTERVAL_SECONDS = 0.1
_SHUTDOWN_SECONDS = 1.0
_EMPTY_SHA256 = "0" * 64


class ControllerError(RuntimeError):
    """The QEMU controller could not safely continue the campaign."""


class _Transport(Protocol):
    def read(self, timeout: float) -> bytes: ...
    def write(self, data: bytes) -> None: ...
    def poll(self) -> int | None: ...
    def terminate(self) -> None: ...
    def wait(self, timeout: float) -> int: ...
    def kill(self) -> None: ...


@dataclass(frozen=True)
class CampaignIdentity:
    metadata: ManifestMetadata
    build_id: str
    build_results: tuple[BuildResult, ...]
    runtime_snapshot_sha256: str = _EMPTY_SHA256


@dataclass(frozen=True)
class ControllerConfig:
    output_directory: Path
    qemu_argv: tuple[str, ...]
    boot_timeout_seconds: float = 30.0
    max_test_serial_bytes: int = 8 * 1024 * 1024


@dataclass(frozen=True)
class ControllerResult:
    attempts: tuple[RuntimeAttempt, ...]
    complete: bool
    restart_count: int
    result_path: Path
    raw_log_path: Path


class _PopenTransport:
    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self._process = process

    @classmethod
    def launch(cls, argv: tuple[str, ...]) -> _PopenTransport:
        process = subprocess.Popen(
            list(argv),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=0,
        )
        return cls(process)

    def read(self, timeout: float) -> bytes:
        stream = self._process.stdout
        if stream is None:
            raise ControllerError("QEMU stdout pipe is unavailable")
        with selectors.DefaultSelector() as selector:
            selector.register(stream, selectors.EVENT_READ)
            if not selector.select(max(0.0, timeout)):
                return b""
        return os.read(stream.fileno(), 65_536)

    def write(self, data: bytes) -> None:
        stream = self._process.stdin
        if stream is None:
            raise ControllerError("QEMU stdin pipe is unavailable")
        stream.write(data)
        stream.flush()

    def poll(self) -> int | None:
        return self._process.poll()

    def terminate(self) -> None:
        self._process.terminate()

    def wait(self, timeout: float) -> int:
        return self._process.wait(timeout=timeout)

    def kill(self) -> None:
        self._process.kill()


def _memory_mebibytes(value: str) -> int:
    match = re.fullmatch(r"([1-9][0-9]*)([MmGg]?)", value)
    if match is None:
        raise ValueError(f"invalid QEMU memory size: {value}")
    amount = int(match.group(1))
    suffix = match.group(2).upper()
    return amount * 1024 if suffix == "G" else amount


def build_qemu_argv(
    *, qemu: str | Path, kernel: Path, disk: Path, memory: str
) -> tuple[str, ...]:
    """Build the AArch64 command line used by the normal smoke launcher."""
    configured_memory = memory if _memory_mebibytes(memory) >= 1024 else "1024M"
    return (
        str(qemu),
        "-M",
        "virt,gic-version=4,virtualization=on",
        "-cpu",
        "cortex-a710",
        "-smp",
        "4",
        "-m",
        configured_memory,
        "-nographic",
        "-kernel",
        str(kernel),
        "-drive",
        f"file={disk},if=none,format=raw,id=fxfs,cache=writethrough",
        "-device",
        "virtio-blk-device,drive=fxfs",
        "-netdev",
        "user,id=smrosnet",
        "-device",
        "virtio-net-device,netdev=smrosnet",
    )


def _json_bytes(value: object) -> bytes:
    return (
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        + "\n"
    ).encode("ascii")


def _attempt_record(attempt: RuntimeAttempt) -> dict[str, object]:
    return {"record_type": "attempt", **attempt.to_dict()}


def _reject_duplicate_keys(
    pairs: list[tuple[str, object]],
) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        if key in value:
            raise ControllerError(f"duplicate guest POSIX event key: {key}")
        value[key] = item
    return value


def _decode_attempt(value: object) -> RuntimeAttempt:
    if not isinstance(value, dict) or set(value) != {
        field.name for field in fields(RuntimeAttempt)
    }:
        raise ValueError("progress contains an invalid completed attempt")
    raw_resources = value.get("resource_deltas")
    if not isinstance(raw_resources, dict):
        raise ValueError("progress contains invalid resource evidence")
    payload = dict(value)
    payload["resource_deltas"] = ResourceDeltas.from_complete_mapping(raw_resources)
    return RuntimeAttempt(**payload)  # type: ignore[arg-type]


def _event_rows(data: bytes) -> tuple[tuple[dict[str, object], int], ...]:
    rows: list[tuple[dict[str, object], int]] = []
    offset = 0
    for raw_line in data.splitlines(keepends=True):
        offset += len(raw_line)
        if not raw_line.endswith(b"\n"):
            continue
        line = raw_line[:-1]
        if not line.startswith(_EVENT_PREFIX_BYTES):
            continue
        try:
            value = json.loads(
                line[len(_EVENT_PREFIX_BYTES) :].decode("utf-8"),
                object_pairs_hook=_reject_duplicate_keys,
            )
        except (UnicodeError, json.JSONDecodeError) as error:
            raise ControllerError("invalid guest POSIX event JSON") from error
        if not isinstance(value, dict):
            raise ControllerError("invalid guest POSIX event record")
        rows.append((value, offset))
    return tuple(rows)


_COMMON_EVENT_FIELDS = {
    "architecture",
    "event",
    "manifest_sha256",
    "run_id",
    "schema",
    "seq",
}
_SUITE_START_FIELDS = _COMMON_EVENT_FIELDS | {
    "boot_id",
    "build_id",
    "build_results_sha256",
    "filter",
    "patch_sha256",
    "revision",
    "selected_count",
    "smros_commit",
    "source",
    "started_ticks",
}
_TEST_START_FIELDS = _COMMON_EVENT_FIELDS | {
    "api",
    "binary_sha256",
    "build_status",
    "group",
    "link_status",
    "source",
    "started_ticks",
    "test_id",
}


def _event_common_is_valid(value: dict[str, object], event: str, seq: int) -> bool:
    return (
        type(value.get("schema")) is int
        and value.get("schema") == 1
        and type(value.get("seq")) is int
        and value.get("seq") == seq
        and value.get("event") == event
        and isinstance(value.get("run_id"), str)
        and bool(value.get("run_id"))
        and value.get("architecture") == "aarch64"
    )


def _matching_start_seen(
    data: bytes,
    test: SuiteTest,
    identity: CampaignIdentity,
) -> bool:
    suite_start: dict[str, object] | None = None
    for value, _offset in _event_rows(data):
        if value.get("event") == "suite_start" and suite_start is None:
            suite_start = value
            continue
        if value.get("event") != "test_start":
            continue
        if any(
            value.get(key) != expected
            for key, expected in (
                ("test_id", test.test_id),
                ("group", test.group),
                ("api", test.api),
                ("binary_sha256", test.sha256),
            )
        ):
            raise ControllerError("guest test identity does not match the command")
        if suite_start is None:
            continue
        suite_is_valid = (
            set(suite_start) <= _SUITE_START_FIELDS
            and _event_common_is_valid(suite_start, "suite_start", 1)
            and suite_start.get("manifest_sha256")
            == identity.metadata.manifest_sha256
            and suite_start.get("selected_count") == 1
            and suite_start.get("build_id") == identity.build_id
            and suite_start.get("build_results_sha256")
            == identity.metadata.build_results_sha256
            and suite_start.get("smros_commit") == identity.metadata.smros_commit
            and suite_start.get("revision") == identity.metadata.revision
            and suite_start.get("patch_sha256") == identity.metadata.patch_sha256
            and suite_start.get("filter") == f"test={test.test_id}"
            and suite_start.get("source") == "smros-serial"
            and type(suite_start.get("started_ticks")) is int
            and int(suite_start["started_ticks"]) >= 0
        )
        test_is_valid = (
            set(value) <= _TEST_START_FIELDS
            and _event_common_is_valid(value, "test_start", 2)
            and value.get("run_id") == suite_start.get("run_id")
            and value.get("manifest_sha256")
            == identity.metadata.manifest_sha256
            and value.get("source") == "smros-serial"
            and type(value.get("started_ticks")) is int
            and int(value["started_ticks"]) >= 0
        )
        if suite_is_valid and test_is_valid:
            return True
    return False


def _suite_end_offset(data: bytes) -> int | None:
    for value, offset in _event_rows(data):
        if value.get("event") == "suite_end":
            return offset
    return None


def _bounded_reason(value: str, maximum: int = 4096) -> str:
    marker = b"...[truncated]"
    data = value.encode("utf-8", errors="replace")
    if len(data) <= maximum:
        return data.decode("utf-8")
    prefix = data[: maximum - len(marker)]
    return prefix.decode("utf-8", errors="ignore") + marker.decode("ascii")


class QemuController:
    def __init__(
        self,
        *,
        identity: CampaignIdentity,
        selected: Sequence[SuiteTest],
        config: ControllerConfig,
        transport_factory: Callable[
            [tuple[str, ...]], _Transport
        ] = _PopenTransport.launch,
        monotonic: Callable[[], float] = time.monotonic,
        run_id_factory: Callable[[], str] = lambda: secrets.token_hex(16),
    ) -> None:
        if not selected:
            raise ValueError("QEMU campaign selected no tests")
        if config.boot_timeout_seconds <= 0:
            raise ValueError("QEMU boot timeout must be positive")
        if (
            type(config.max_test_serial_bytes) is not int
            or config.max_test_serial_bytes <= 0
        ):
            raise ValueError("QEMU per-test serial byte limit must be positive")
        self.identity = identity
        self.selected = tuple(selected)
        self.config = config
        self._transport_factory = transport_factory
        self._monotonic = monotonic
        self._run_id_factory = run_id_factory
        self._output = Path(os.path.abspath(config.output_directory))
        self._progress_path = self._output / "progress.json"
        self._result_path = self._output / "results.ndjson"
        self._raw_log_path = self._output / "qemu-serial.log"
        self._attempts: list[RuntimeAttempt] = []
        self._current_test: str | None = None
        self._restart_count = 0
        self._boot_count = 0
        self._run_id = ""

    def _progress(self) -> dict[str, object]:
        return {
            "build_id": self.identity.build_id,
            "boot_count": self._boot_count,
            "completed_attempts": [attempt.to_dict() for attempt in self._attempts],
            "current_test": self._current_test,
            "manifest_sha256": self.identity.metadata.manifest_sha256,
            "raw_log": str(self._raw_log_path),
            "restart_count": self._restart_count,
            "run_id": self._run_id,
            "selected_ids": [test.test_id for test in self.selected],
        }

    def _persist_progress(self) -> None:
        _atomic_write(self._progress_path, _json_bytes(self._progress()))

    def _load_progress(self) -> None:
        try:
            value = json.loads(self._progress_path.read_text(encoding="utf-8"))
        except OSError as error:
            raise ValueError("resume progress is unavailable") from error
        except (UnicodeError, json.JSONDecodeError) as error:
            raise ValueError("resume progress is invalid") from error
        if not isinstance(value, dict):
            raise ValueError("resume progress is invalid")
        if set(value) != {
            "boot_count",
            "build_id",
            "completed_attempts",
            "current_test",
            "manifest_sha256",
            "raw_log",
            "restart_count",
            "run_id",
            "selected_ids",
        }:
            raise ValueError("resume progress schema is invalid")
        expected_ids = [test.test_id for test in self.selected]
        if (
            value.get("manifest_sha256") != self.identity.metadata.manifest_sha256
            or value.get("build_id") != self.identity.build_id
        ):
            raise ValueError("resume build identity does not match the staged campaign")
        if value.get("selected_ids") != expected_ids:
            raise ValueError("resume test selection does not match the campaign")
        if value.get("raw_log") != str(self._raw_log_path):
            raise ValueError("resume raw log identity does not match the campaign")
        run_id = value.get("run_id")
        boot_count = value.get("boot_count")
        restart_count = value.get("restart_count")
        completed = value.get("completed_attempts")
        current_test = value.get("current_test")
        if (
            not isinstance(run_id, str)
            or not run_id
            or type(boot_count) is not int
            or boot_count < 0
            or type(restart_count) is not int
            or restart_count < 0
            or not isinstance(completed, list)
            or (
                current_test is not None
                and current_test not in expected_ids
            )
        ):
            raise ValueError("resume progress is invalid")
        attempts = [_decode_attempt(item) for item in completed]
        selected_ids = set(expected_ids)
        completed_ids = [attempt.test_id for attempt in attempts]
        if (
            len(completed_ids) != len(set(completed_ids))
            or any(test_id not in selected_ids for test_id in completed_ids)
            or completed_ids != expected_ids[: len(completed_ids)]
        ):
            raise ValueError("resume progress contains invalid completed tests")
        tests_by_id = {test.test_id: test for test in self.selected}
        for attempt in attempts:
            self._validate_resumed_attempt(
                attempt, tests_by_id[attempt.test_id], run_id
            )
        try:
            raw_info = self._raw_log_path.lstat()
        except OSError as error:
            raise ValueError("resume raw log is unavailable") from error
        if stat.S_ISLNK(raw_info.st_mode) or not stat.S_ISREG(raw_info.st_mode):
            raise ValueError("resume raw log is not a regular file")
        required_size = max(
            (attempt.raw_log_end or 0 for attempt in attempts), default=0
        )
        if raw_info.st_size < required_size:
            raise ValueError("resume raw log is truncated")
        self._attempts = attempts
        self._boot_count = boot_count
        self._restart_count = restart_count
        self._run_id = run_id
        self._current_test = None

    def _validate_resumed_attempt(
        self, attempt: RuntimeAttempt, test: SuiteTest, run_id: str
    ) -> None:
        build_status, link_status = self._build_statuses(test)
        identity_matches = (
            attempt.test_id == test.test_id
            and attempt.group == test.group
            and attempt.api == test.api
            and attempt.binary_sha256 == test.sha256
            and attempt.platform == PLATFORM
            and attempt.build_status == build_status
            and attempt.link_status == link_status
            and attempt.manifest_sha256 == self.identity.metadata.manifest_sha256
            and attempt.build_results_sha256
            == self.identity.metadata.build_results_sha256
            and attempt.build_id == self.identity.build_id
            and attempt.revision == self.identity.metadata.revision
            and attempt.patch_sha256 == self.identity.metadata.patch_sha256
            and attempt.smros_commit == self.identity.metadata.smros_commit
            and attempt.runtime_snapshot_sha256
            == self.identity.runtime_snapshot_sha256
            and attempt.run_id == run_id
        )
        offsets_match = (
            type(attempt.raw_log_start) is int
            and type(attempt.raw_log_end) is int
            and attempt.raw_log_start >= 0
            and attempt.raw_log_end >= attempt.raw_log_start
        )
        scalar_types_match = (
            type(attempt.duration_ms) is int
            and attempt.duration_ms >= 0
            and isinstance(attempt.stdout, str)
            and isinstance(attempt.stderr, str)
            and type(attempt.stdout_bytes) is int
            and attempt.stdout_bytes >= 0
            and type(attempt.stderr_bytes) is int
            and attempt.stderr_bytes >= 0
            and type(attempt.stdout_truncated) is bool
            and type(attempt.stderr_truncated) is bool
        )
        try:
            if attempt.source == WATCHDOG_SOURCE:
                validate_host_watchdog_attempt_semantics(
                    status=attempt.status,
                    pts_status=attempt.pts_status,
                    launch_status=attempt.launch_status,
                    exit_code=attempt.exit_code,
                    signal=attempt.signal,
                    timed_out=attempt.timed_out,
                    launch_error=attempt.launch_error,
                    infrastructure_error=attempt.infrastructure_error,
                    label="resume completed attempt",
                )
                evidence_matches = (
                    attempt.resource_evidence == "unavailable"
                    and not attempt.resource_deltas.has_nonzero()
                )
            elif attempt.source == SOURCE:
                validate_raw_attempt_semantics(
                    status=attempt.status,
                    pts_status=attempt.pts_status,
                    launch_status=attempt.launch_status,
                    exit_code=attempt.exit_code,
                    signal=attempt.signal,
                    timed_out=attempt.timed_out,
                    launch_error=attempt.launch_error,
                    infrastructure_error=attempt.infrastructure_error,
                    label="resume completed attempt",
                )
                evidence_matches = attempt.resource_evidence == "measured"
            else:
                evidence_matches = False
        except ValueError as error:
            raise ValueError(f"resume completed attempt is invalid: {error}") from error
        if not (
            identity_matches
            and offsets_match
            and scalar_types_match
            and evidence_matches
        ):
            raise ValueError("resume completed attempt identity is invalid")

    def _append_raw(self, stream, data: bytes) -> None:
        if not data:
            return
        stream.write(data)
        stream.flush()

    def _wait_for_prompt(self, transport: _Transport, raw) -> bool:
        deadline = self._monotonic() + self.config.boot_timeout_seconds
        tail = b""
        while self._monotonic() < deadline:
            remaining = deadline - self._monotonic()
            data = transport.read(min(_READ_INTERVAL_SECONDS, remaining))
            self._append_raw(raw, data)
            combined = tail + data
            if transport.poll() is not None:
                return False
            if any(pattern in combined for pattern in _FATAL_PATTERNS):
                return False
            if PROMPT in combined:
                return True
            tail = combined[-(_STREAM_TOKEN_BYTES - 1) :]
        raise ControllerError("QEMU boot prompt deadline exceeded")

    def _stop(self, transport: _Transport) -> None:
        if transport.poll() is None:
            transport.terminate()
        try:
            transport.wait(_SHUTDOWN_SECONDS)
        except subprocess.TimeoutExpired:
            transport.kill()
            transport.wait(_SHUTDOWN_SECONDS)

    def _launch_ready(self, raw) -> _Transport:
        transport = self._transport_factory(self.config.qemu_argv)
        self._boot_count += 1
        if self._boot_count > 1:
            self._restart_count += 1
            self._persist_progress()
        try:
            if not self._wait_for_prompt(transport, raw):
                status = transport.poll()
                raise ControllerError(
                    f"QEMU failed before the exact shell prompt (status {status})"
                )
            return transport
        except BaseException:
            self._stop(transport)
            raise

    def _validate_guest(
        self, data: bytes, test: SuiteTest
    ) -> tuple[SerialAttempt, int, bool]:
        end_offset = _suite_end_offset(data)
        if end_offset is None:
            raise ControllerError("guest POSIX suite has no terminal event")
        try:
            parsed = parse_serial_log(
                data[:end_offset].decode("utf-8", errors="replace")
            )
        except ValueError as error:
            raise ControllerError(
                f"invalid guest POSIX event stream: {error}"
            ) from error
        if not parsed.complete or len(parsed.attempts) != 1:
            raise ControllerError("guest POSIX suite did not complete exactly one test")
        attempt = parsed.attempts[0]
        if (
            attempt.test_id != test.test_id
            or attempt.group != test.group
            or attempt.api != test.api
        ):
            raise ControllerError("guest test identity does not match the command")
        start = parsed.events[0].values
        expected_start = {
            "selected_count": 1,
            "build_id": self.identity.build_id,
            "build_results_sha256": self.identity.metadata.build_results_sha256,
            "smros_commit": self.identity.metadata.smros_commit,
            "revision": self.identity.metadata.revision,
            "patch_sha256": self.identity.metadata.patch_sha256,
            "filter": f"test={test.test_id}",
        }
        if any(start.get(key) != value for key, value in expected_start.items()):
            raise ControllerError("guest suite identity or provenance does not match")
        if parsed.manifest_sha256 != self.identity.metadata.manifest_sha256:
            raise ControllerError("guest manifest checksum does not match")
        test_starts = [
            event.values for event in parsed.events if event.event == "test_start"
        ]
        if len(test_starts) != 1 or test_starts[0].get("binary_sha256") != test.sha256:
            raise ControllerError(
                "guest test identity or binary checksum does not match"
            )
        prompt_after = PROMPT in data[end_offset:]
        return attempt, end_offset, prompt_after

    def _build_statuses(self, test: SuiteTest) -> tuple[str, str]:
        by_stage = {
            result.stage: result.status
            for result in self.identity.build_results
            if result.test_id == test.test_id
        }
        return by_stage.get("compile", "not-built"), by_stage.get(
            "link", "not-linked"
        )

    def _guest_attempt(
        self,
        guest: SerialAttempt,
        test: SuiteTest,
        *,
        raw_log_start: int,
        raw_log_end: int,
    ) -> RuntimeAttempt:
        build_status, link_status = self._build_statuses(test)
        return RuntimeAttempt(
            test_id=test.test_id,
            group=test.group,
            api=test.api,
            platform=PLATFORM,
            build_status=build_status,
            link_status=link_status,
            launch_status=guest.launch_status,
            pts_status=guest.pts_status,
            status=guest.status,
            exit_code=guest.exit_code,
            signal=guest.signal,
            timed_out=guest.timed_out,
            duration_ms=guest.duration_ms,
            stdout=guest.stdout,
            stderr=guest.stderr,
            source=SOURCE,
            launch_error=guest.launch_error,
            infrastructure_error=guest.infrastructure_error,
            stdout_bytes=len(guest.stdout.encode("utf-8")),
            stderr_bytes=len(guest.stderr.encode("utf-8")),
            manifest_sha256=self.identity.metadata.manifest_sha256,
            build_results_sha256=self.identity.metadata.build_results_sha256,
            build_id=self.identity.build_id,
            revision=self.identity.metadata.revision,
            patch_sha256=self.identity.metadata.patch_sha256,
            smros_commit=self.identity.metadata.smros_commit,
            binary_sha256=test.sha256 or _EMPTY_SHA256,
            runtime_snapshot_sha256=self.identity.runtime_snapshot_sha256,
            run_id=self._run_id,
            resource_deltas=guest.resource_deltas,
            resource_evidence=guest.resource_evidence,
            raw_log_start=raw_log_start,
            raw_log_end=raw_log_end,
        )

    def _watchdog_attempt(
        self,
        test: SuiteTest,
        *,
        status: str,
        started: bool,
        timed_out: bool,
        reason: str,
        duration_ms: int,
        raw_log_start: int,
        raw_log_end: int,
    ) -> RuntimeAttempt:
        build_status, link_status = self._build_statuses(test)
        return RuntimeAttempt(
            test_id=test.test_id,
            group=test.group,
            api=test.api,
            platform=PLATFORM,
            build_status=build_status,
            link_status=link_status,
            launch_status="launched" if started else "interrupted",
            pts_status=None,
            status=status,
            exit_code=None,
            signal=None,
            timed_out=timed_out,
            duration_ms=duration_ms,
            stdout="",
            stderr="",
            source=WATCHDOG_SOURCE,
            infrastructure_error=_bounded_reason(reason),
            manifest_sha256=self.identity.metadata.manifest_sha256,
            build_results_sha256=self.identity.metadata.build_results_sha256,
            build_id=self.identity.build_id,
            revision=self.identity.metadata.revision,
            patch_sha256=self.identity.metadata.patch_sha256,
            smros_commit=self.identity.metadata.smros_commit,
            binary_sha256=test.sha256 or _EMPTY_SHA256,
            runtime_snapshot_sha256=self.identity.runtime_snapshot_sha256,
            run_id=self._run_id,
            resource_deltas=ResourceDeltas(),
            resource_evidence="unavailable",
            raw_log_start=raw_log_start,
            raw_log_end=raw_log_end,
        )

    def _run_test(self, transport: _Transport, raw, test: SuiteTest):
        command_started = self._monotonic()
        deadline = command_started + test.timeout_ms / 1000.0
        raw_start = raw.tell()
        try:
            transport.write(f"posixtest test {test.test_id}\n".encode("utf-8"))
        except OSError as error:
            attempt = self._watchdog_attempt(
                test,
                status="crash",
                started=False,
                timed_out=False,
                reason=f"QEMU command write failed: {error}",
                duration_ms=max(
                    0, int((self._monotonic() - command_started) * 1000)
                ),
                raw_log_start=raw_start,
                raw_log_end=raw.tell(),
            )
            return attempt, False, True
        data = bytearray()
        matching_start = False
        while True:
            remaining = deadline - self._monotonic()
            if remaining <= 0:
                raw_end = raw.tell()
                attempt = self._watchdog_attempt(
                    test,
                    status="timeout",
                    started=matching_start,
                    timed_out=True,
                    reason=f"host watchdog deadline exceeded for {test.test_id}",
                    duration_ms=max(
                        0, int((self._monotonic() - command_started) * 1000)
                    ),
                    raw_log_start=raw_start,
                    raw_log_end=raw_end,
                )
                return attempt, False, True
            chunk = transport.read(min(_READ_INTERVAL_SECONDS, remaining))
            self._append_raw(raw, chunk)
            remaining_capacity = self.config.max_test_serial_bytes + 1 - len(data)
            if remaining_capacity > 0:
                data.extend(chunk[:remaining_capacity])
            matching_start = matching_start or _matching_start_seen(
                bytes(data), test, self.identity
            )
            if (
                len(data) > self.config.max_test_serial_bytes
                or len(chunk) > remaining_capacity
            ):
                attempt = self._watchdog_attempt(
                    test,
                    status="crash",
                    started=matching_start,
                    timed_out=False,
                    reason=(
                        "host watchdog serial byte limit exceeded for "
                        f"{test.test_id}"
                    ),
                    duration_ms=max(
                        0, int((self._monotonic() - command_started) * 1000)
                    ),
                    raw_log_start=raw_start,
                    raw_log_end=raw.tell(),
                )
                return attempt, False, True
            terminal_offset = _suite_end_offset(bytes(data))
            if terminal_offset is not None:
                guest, _end_offset, prompt_after = self._validate_guest(
                    bytes(data), test
                )
                return (
                    self._guest_attempt(
                        guest,
                        test,
                        raw_log_start=raw_start,
                        raw_log_end=raw.tell(),
                    ),
                    prompt_after,
                    False,
                )
            fatal = next((item for item in _FATAL_PATTERNS if item in data), None)
            returncode = transport.poll()
            if fatal is not None or returncode is not None:
                if fatal is not None:
                    reason = f"host watchdog observed fatal serial pattern {fatal!r}"
                else:
                    reason = f"QEMU exited with status {returncode}"
                attempt = self._watchdog_attempt(
                    test,
                    status="crash",
                    started=matching_start,
                    timed_out=False,
                    reason=reason,
                    duration_ms=max(
                        0, int((self._monotonic() - command_started) * 1000)
                    ),
                    raw_log_start=raw_start,
                    raw_log_end=raw.tell(),
                )
                return attempt, False, True

    def _terminal(self) -> dict[str, object]:
        return {
            "boot_count": self._boot_count,
            "build_id": self.identity.build_id,
            "build_results_sha256": self.identity.metadata.build_results_sha256,
            "complete": True,
            "completed_count": len(self._attempts),
            "manifest_sha256": self.identity.metadata.manifest_sha256,
            "patch_sha256": self.identity.metadata.patch_sha256,
            "platform": PLATFORM,
            "qemu": " ".join(self.config.qemu_argv),
            "raw_log": str(self._raw_log_path),
            "record_type": "run",
            "restart_count": self._restart_count,
            "revision": self.identity.metadata.revision,
            "run_id": self._run_id,
            "selected_count": len(self.selected),
            "smros_commit": self.identity.metadata.smros_commit,
            "source": SOURCE,
            "status_counts": dict(
                sorted(Counter(attempt.status for attempt in self._attempts).items())
            ),
        }

    def _publish(self) -> None:
        rows = [_attempt_record(attempt) for attempt in self._attempts]
        rows.append(self._terminal())
        _atomic_write(self._result_path, _canonical_report(rows))

    def run(self, resume: bool = False) -> ControllerResult:
        self._output.mkdir(parents=True, exist_ok=True)
        if resume:
            self._load_progress()
            raw_mode = "ab"
        else:
            self._run_id = self._run_id_factory()
            self._attempts = []
            self._restart_count = 0
            self._boot_count = 0
            raw_mode = "wb"
        transport: _Transport | None = None
        prompt_ready = False
        completed_ids = {attempt.test_id for attempt in self._attempts}
        with self._raw_log_path.open(raw_mode) as raw:
            self._persist_progress()
            try:
                for test in self.selected:
                    if test.test_id in completed_ids:
                        continue
                    while transport is None or not prompt_ready:
                        if transport is None:
                            transport = self._launch_ready(raw)
                            prompt_ready = True
                        elif self._wait_for_prompt(transport, raw):
                            prompt_ready = True
                        else:
                            self._stop(transport)
                            transport = None
                    self._current_test = test.test_id
                    self._persist_progress()
                    attempt, prompt_ready, restart = self._run_test(
                        transport, raw, test
                    )
                    self._attempts.append(attempt)
                    completed_ids.add(test.test_id)
                    self._current_test = None
                    self._persist_progress()
                    if restart:
                        self._stop(transport)
                        transport = None
                        prompt_ready = False
                self._publish()
                self._progress_path.unlink()
            finally:
                if transport is not None:
                    self._stop(transport)
        return ControllerResult(
            attempts=tuple(self._attempts),
            complete=True,
            restart_count=self._restart_count,
            result_path=self._result_path,
            raw_log_path=self._raw_log_path,
        )


def _regular_file(path: Path, label: str) -> Path:
    absolute = Path(os.path.abspath(path))
    if not absolute.is_file():
        raise ValueError(f"{label} is unavailable: {absolute}")
    return absolute


def run_smros(
    stage: Path,
    output_directory: Path,
    *,
    kernel: Path,
    disk: Path,
    memory: str = "1024M",
    api: str | None = None,
    group: str | None = None,
    test_id: str | None = None,
    resume: bool = False,
    qemu: str = "qemu-system-aarch64",
) -> ControllerResult:
    qemu_path = shutil.which(qemu)
    if qemu_path is None:
        raise ValueError(f"required QEMU executable is unavailable: {qemu}")
    stage = Path(os.path.abspath(stage))
    loaded = _load_stage_identity(stage)
    selected = filter_runnable_tests(
        loaded.tests, api=api, group=group, test_id=test_id
    )
    _validate_selected(stage, selected)
    identity = CampaignIdentity(
        metadata=loaded.metadata,
        build_id=loaded.build_id,
        build_results=loaded.build_results,
        runtime_snapshot_sha256=loaded.runtime_snapshot_sha256 or _EMPTY_SHA256,
    )
    argv = build_qemu_argv(
        qemu=qemu_path,
        kernel=_regular_file(kernel, "kernel image"),
        disk=_regular_file(disk, "FxFS disk image"),
        memory=memory,
    )
    return QemuController(
        identity=identity,
        selected=selected,
        config=ControllerConfig(
            output_directory=output_directory,
            qemu_argv=argv,
        ),
    ).run(resume=resume)
