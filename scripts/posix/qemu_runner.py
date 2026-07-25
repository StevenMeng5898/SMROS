"""Collect isolated SMROS POSIX results from a persistent QEMU campaign."""

from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Sequence
import ctypes
from dataclasses import dataclass, fields
from enum import Enum
import errno
import fcntl
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
    _open_parent,
    _validate_selected,
    filter_runnable_tests,
)
from .build import MAX_TESTS, ManifestMetadata
from .events import EVENT_PREFIX, parse_serial_log
from .model import (
    BuildResult,
    ResourceDeltas,
    RuntimeAttempt,
    SerialAttempt,
    SuiteTest,
    is_valid_run_id,
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
)
_STREAM_TOKEN_BYTES = max(len(PROMPT), *(len(item) for item in _FATAL_PATTERNS))
_READ_INTERVAL_SECONDS = 0.1
_SHUTDOWN_SECONDS = 1.0
_EMPTY_SHA256 = "0" * 64
_TRUNCATION_MARKER = b"\n...[truncated]"
_MAX_PERSISTED_STREAM_BYTES = 16 * 1024
_MAX_PERSISTED_ERROR_BYTES = 4 * 1024
_MAX_PROGRESS_BYTES = 128 * 1024 * 1024
_MAX_RESULT_LINE_BYTES = 512 * 1024
_PERSISTED_ATTEMPTS_BUDGET = 120 * 1024 * 1024
_ATTEMPT_FIXED_BUDGET = 8 * 1024
_JSON_ESCAPE_EXPANSION = 6
_RESULT_QUARANTINE_NAME = ".smros-posix-qemu-quarantine"
_RESULT_QUARANTINE_SLOT = "cleanup"
_PROGRESS_RETIREMENT_NAME = ".progress.json.retiring"
_CAMPAIGN_LOCK_NAME = ".smros-posix-qemu.lock"


class ControllerError(RuntimeError):
    """The QEMU controller could not safely continue the campaign."""


class _ResumeCheckpointState(Enum):
    ACTIVE = "active"
    INCOMPLETE = "incomplete"
    TERMINAL = "terminal"


class _ResumeResultState(Enum):
    ABSENT = "absent"
    MARKER = "marker"
    EXACT = "exact"


@dataclass
class _ResumeCheckpoint:
    state: _ResumeCheckpointState
    progress_descriptor: int | None
    progress_fingerprint: tuple[int, ...]
    progress_bytes: bytes
    result_state: _ResumeResultState = _ResumeResultState.ABSENT
    result_descriptor: int | None = None
    result_fingerprint: tuple[int, ...] | None = None

    def take_progress_descriptor(self) -> int:
        if self.progress_descriptor is None:
            raise RuntimeError("resume progress descriptor was already transferred")
        descriptor = self.progress_descriptor
        self.progress_descriptor = None
        return descriptor

    def take_result_descriptor(self) -> int:
        if self.result_descriptor is None:
            raise RuntimeError("resume result descriptor was already transferred")
        descriptor = self.result_descriptor
        self.result_descriptor = None
        return descriptor

    def close(self) -> None:
        errors: list[BaseException] = []
        for field_name in ("result_descriptor", "progress_descriptor"):
            descriptor = getattr(self, field_name)
            setattr(self, field_name, None)
            if descriptor is None:
                continue
            try:
                os.close(descriptor)
            except BaseException as error:
                errors.append(error)
        if len(errors) == 1:
            raise errors[0]
        if errors:
            raise BaseExceptionGroup(
                "resume checkpoint descriptor cleanup failed",
                errors,
            )

    def __enter__(self) -> _ResumeCheckpoint:
        return self

    def __exit__(self, exc_type, exc, traceback) -> bool:
        del exc_type, traceback
        try:
            self.close()
        except BaseException as cleanup_error:
            if exc is not None:
                raise cleanup_error from exc
            raise
        return False


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


@dataclass(frozen=True)
class _AttemptFieldLimits:
    stream_bytes: int
    error_bytes: int


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
        and is_valid_run_id(value.get("run_id"))
        and value.get("architecture") == "aarch64"
    )


def _suite_start_is_valid(
    suite_start: dict[str, object],
    test: SuiteTest,
    identity: CampaignIdentity,
) -> bool:
    return (
        set(suite_start) <= _SUITE_START_FIELDS
        and _event_common_is_valid(suite_start, "suite_start", 1)
        and suite_start.get("manifest_sha256")
        == identity.metadata.manifest_sha256
        and type(suite_start.get("selected_count")) is int
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


def _start_pair_is_valid(
    suite_start: dict[str, object],
    test_start: dict[str, object],
    test: SuiteTest,
    identity: CampaignIdentity,
) -> bool:
    suite_is_valid = _suite_start_is_valid(suite_start, test, identity)
    test_is_valid = (
        set(test_start) <= _TEST_START_FIELDS
        and _event_common_is_valid(test_start, "test_start", 2)
        and test_start.get("run_id") == suite_start.get("run_id")
        and test_start.get("manifest_sha256")
        == identity.metadata.manifest_sha256
        and test_start.get("test_id") == test.test_id
        and test_start.get("group") == test.group
        and test_start.get("api") == test.api
        and test_start.get("binary_sha256") == test.sha256
        and test_start.get("source") == "smros-serial"
        and type(test_start.get("started_ticks")) is int
        and int(test_start["started_ticks"]) >= 0
    )
    return suite_is_valid and test_is_valid


def _matching_start_seen(
    data: bytes,
    test: SuiteTest,
    identity: CampaignIdentity,
) -> bool:
    rows = _event_rows(data)
    for value, _offset in rows:
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
    if len(rows) != 2:
        return False
    suite_start, test_start = (value for value, _offset in rows)
    return _start_pair_is_valid(suite_start, test_start, test, identity)


def _terminal_event_offset(data: bytes) -> tuple[str, int] | None:
    for value, offset in _event_rows(data):
        event = value.get("event")
        if event in {"suite_end", "infrastructure_error"}:
            return str(event), offset
    return None


def _bounded_reason(value: str, maximum: int = 4096) -> str:
    marker = b"...[truncated]"
    data = value.encode("utf-8", errors="replace")
    if len(data) <= maximum:
        return data.decode("utf-8")
    prefix = data[: maximum - len(marker)]
    return prefix.decode("utf-8", errors="ignore") + marker.decode("ascii")


def _persisted_attempt_field_limits(selected_count: int) -> _AttemptFieldLimits:
    # Keep all attempt rows within 120 MiB, leaving 8 MiB under the report cap
    # for progress inventory and the terminal row. Each attempt reserves its
    # LF before assigning the remaining variable bytes across two streams and
    # both errors allowed on a launch-error attempt.
    attempt_budget = _PERSISTED_ATTEMPTS_BUDGET // selected_count
    line_budget = min(_MAX_RESULT_LINE_BYTES, attempt_budget - 1)
    variable_budget = (line_budget - _ATTEMPT_FIXED_BUDGET) // (
        _JSON_ESCAPE_EXPANSION
    )
    error_bytes = min(_MAX_PERSISTED_ERROR_BYTES, variable_budget // 4)
    stream_bytes = min(
        _MAX_PERSISTED_STREAM_BYTES,
        (variable_budget - 2 * error_bytes) // 2,
    )
    return _AttemptFieldLimits(
        stream_bytes=stream_bytes,
        error_bytes=error_bytes,
    )


def _bounded_stream(value: str, maximum: int) -> tuple[str, int, bool]:
    data = value.encode("utf-8")
    if len(data) <= maximum:
        return value, len(data), False
    prefix_bytes = maximum - len(_TRUNCATION_MARKER)
    prefix = data[:prefix_bytes].decode("utf-8", errors="ignore")
    return prefix + _TRUNCATION_MARKER.decode("ascii"), len(data), True


def _strict_utf8_size(value: str) -> int | None:
    try:
        return len(value.encode("utf-8"))
    except UnicodeEncodeError:
        return None


def _persisted_stream_is_valid(
    value: str, byte_count: int, truncated: bool, maximum: int
) -> bool:
    stored_bytes = _strict_utf8_size(value)
    return stored_bytes is not None and stored_bytes <= maximum and (
        (
            truncated
            and byte_count > stored_bytes
            and value.endswith(_TRUNCATION_MARKER.decode("ascii"))
        )
        or (not truncated and byte_count == stored_bytes)
    )


def _persisted_errors_are_valid(
    launch_error: object,
    infrastructure_error: object,
    maximum: int,
) -> bool:
    values = (launch_error, infrastructure_error)
    return all(
        value is None
        or (
            isinstance(value, str)
            and (size := _strict_utf8_size(value)) is not None
            and size <= maximum
        )
        for value in values
    )


def _reject_progress_duplicate_keys(
    pairs: list[tuple[str, object]],
) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate progress JSON key: {key}")
        value[key] = item
    return value


def _rename_result_between(
    source_parent: int,
    source_name: str,
    destination_parent: int,
    destination_name: str,
    flags: int,
) -> None:
    renameat2 = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
    if renameat2 is None:
        raise ControllerError("atomic QEMU result transitions are unavailable")
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
            raise ControllerError(
                "atomic QEMU result transitions are unavailable"
            )
        raise OSError(error_number, os.strerror(error_number), source_name)


def _rename_result_entry(
    parent: int,
    first: str,
    second: str,
    flags: int,
) -> None:
    _rename_result_between(parent, first, parent, second, flags)


def _rename_noreplace(parent: int, first: str, second: str) -> None:
    _rename_result_entry(parent, first, second, 1)


def _rename_noreplace_between(
    source_parent: int,
    source_name: str,
    destination_parent: int,
    destination_name: str,
) -> None:
    _rename_result_between(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
        1,
    )


def _rename_exchange(parent: int, first: str, second: str) -> None:
    _rename_result_entry(parent, first, second, 2)


def _entry_matches(parent: int, name: str, descriptor: int) -> bool:
    try:
        entry = os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return False
    held = os.fstat(descriptor)
    return (entry.st_dev, entry.st_ino) == (held.st_dev, held.st_ino)


def _stat_fingerprint(info: os.stat_result) -> tuple[int, ...]:
    return (
        info.st_dev,
        info.st_ino,
        info.st_mode,
        info.st_nlink,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )


def _open_result_entry(parent: int, name: str) -> int:
    return os.open(
        name,
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NONBLOCK", 0)
        | getattr(os, "O_NOFOLLOW", 0),
        dir_fd=parent,
    )


def _validate_campaign_lock(
    output_descriptor: int,
    lock_descriptor: int,
) -> None:
    info = os.fstat(lock_descriptor)
    if (
        not stat.S_ISREG(info.st_mode)
        or info.st_uid != os.geteuid()
        or stat.S_IMODE(info.st_mode) != 0o600
        or info.st_nlink != 1
    ):
        raise ControllerError("QEMU campaign lock is unsafe")
    if not _entry_matches(
        output_descriptor,
        _CAMPAIGN_LOCK_NAME,
        lock_descriptor,
    ):
        raise ControllerError("QEMU campaign lock changed while being opened")


def _open_campaign_lock(output_descriptor: int) -> int:
    flags = (
        os.O_RDWR
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NONBLOCK", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    descriptor: int | None = None
    created = False
    try:
        try:
            descriptor = os.open(
                _CAMPAIGN_LOCK_NAME,
                flags | os.O_CREAT | os.O_EXCL,
                0o600,
                dir_fd=output_descriptor,
            )
            created = True
        except FileExistsError:
            descriptor = os.open(
                _CAMPAIGN_LOCK_NAME,
                flags,
                dir_fd=output_descriptor,
            )
        if created:
            os.fchmod(descriptor, 0o600)
            os.fsync(descriptor)
            os.fsync(output_descriptor)
        _validate_campaign_lock(output_descriptor, descriptor)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ControllerError(
                "QEMU campaign is already active for this output directory"
            ) from error
        _validate_campaign_lock(output_descriptor, descriptor)
        return descriptor
    except ControllerError:
        if descriptor is not None:
            os.close(descriptor)
        raise
    except OSError as error:
        if descriptor is not None:
            os.close(descriptor)
        raise ControllerError(
            "QEMU campaign lock could not be opened safely"
        ) from error
    except BaseException:
        if descriptor is not None:
            os.close(descriptor)
        raise


def _write_all(descriptor: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        written = os.write(descriptor, view)
        if written <= 0:
            raise OSError("QEMU result write made no progress")
        view = view[written:]


def _recover_result_quarantine_slot(quarantine_descriptor: int) -> None:
    slot_descriptor: int | None = None
    try:
        slot_descriptor = _open_result_entry(
            quarantine_descriptor,
            _RESULT_QUARANTINE_SLOT,
        )
        info = os.fstat(slot_descriptor)
        if (
            not stat.S_ISREG(info.st_mode)
            or info.st_nlink != 1
            or not _entry_matches(
                quarantine_descriptor,
                _RESULT_QUARANTINE_SLOT,
                slot_descriptor,
            )
        ):
            raise ControllerError("QEMU result quarantine cleanup slot is unsafe")
        os.unlink(_RESULT_QUARANTINE_SLOT, dir_fd=quarantine_descriptor)
        os.fsync(quarantine_descriptor)
    except ControllerError:
        raise
    except OSError as error:
        raise ControllerError(
            "QEMU result quarantine cleanup slot could not be recovered safely"
        ) from error
    finally:
        if slot_descriptor is not None:
            os.close(slot_descriptor)


def _open_result_quarantine(output_descriptor: int) -> int:
    descriptor: int | None = None
    try:
        try:
            os.mkdir(_RESULT_QUARANTINE_NAME, 0o700, dir_fd=output_descriptor)
        except FileExistsError:
            pass
        descriptor = os.open(
            _RESULT_QUARANTINE_NAME,
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_DIRECTORY", 0)
            | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=output_descriptor,
        )
        info = os.fstat(descriptor)
        if (
            not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.geteuid()
            or stat.S_IMODE(info.st_mode) != 0o700
            or not _entry_matches(
                output_descriptor,
                _RESULT_QUARANTINE_NAME,
                descriptor,
            )
        ):
            raise ControllerError("QEMU result quarantine is unsafe")
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        if not _entry_matches(
            output_descriptor,
            _RESULT_QUARANTINE_NAME,
            descriptor,
        ):
            raise ControllerError("QEMU result quarantine changed while locking")
        with os.scandir(descriptor) as entries:
            first = next(entries, None)
            second = next(entries, None)
        if first is not None:
            if first.name != _RESULT_QUARANTINE_SLOT or second is not None:
                raise ControllerError("QEMU result quarantine content is unsafe")
            _recover_result_quarantine_slot(descriptor)
        os.fsync(output_descriptor)
        return descriptor
    except ControllerError:
        if descriptor is not None:
            os.close(descriptor)
        raise
    except OSError as error:
        if descriptor is not None:
            os.close(descriptor)
        raise ControllerError(
            "QEMU result quarantine could not be opened safely"
        ) from error
    except BaseException:
        if descriptor is not None:
            os.close(descriptor)
        raise


def _remove_owned_result_entry(
    output_descriptor: int,
    name: str,
    expected_descriptor: int,
) -> None:
    if not _entry_matches(output_descriptor, name, expected_descriptor):
        raise ControllerError("QEMU results changed during cleanup")
    quarantine_descriptor = _open_result_quarantine(output_descriptor)
    try:
        try:
            _rename_noreplace_between(
                output_descriptor,
                name,
                quarantine_descriptor,
                _RESULT_QUARANTINE_SLOT,
            )
        except FileExistsError as error:
            raise ControllerError("QEMU result quarantine is not empty") from error
        os.fsync(output_descriptor)
        os.fsync(quarantine_descriptor)
        if not _entry_matches(
            quarantine_descriptor,
            _RESULT_QUARANTINE_SLOT,
            expected_descriptor,
        ):
            try:
                _rename_noreplace_between(
                    quarantine_descriptor,
                    _RESULT_QUARANTINE_SLOT,
                    output_descriptor,
                    name,
                )
            except FileExistsError as error:
                raise ControllerError(
                    "QEMU results changed and cleanup restoration was blocked"
                ) from error
            os.fsync(quarantine_descriptor)
            os.fsync(output_descriptor)
            raise ControllerError("QEMU results changed during cleanup")
        os.unlink(_RESULT_QUARANTINE_SLOT, dir_fd=quarantine_descriptor)
        os.fsync(quarantine_descriptor)
    except ControllerError:
        raise
    except OSError as error:
        raise ControllerError("QEMU results could not be cleaned safely") from error
    finally:
        os.close(quarantine_descriptor)


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
        if len(selected) > MAX_TESTS:
            raise ValueError("QEMU campaign selected too many tests")
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
        self._infrastructure_error: str | None = None
        self._prompt_tail = b""
        self._resume_output_descriptor: int | None = None
        self._resume_progress_descriptor: int | None = None
        self._resume_progress_fingerprint: tuple[int, ...] | None = None

    def _progress(self) -> dict[str, object]:
        return {
            "build_id": self.identity.build_id,
            "boot_count": self._boot_count,
            "completed_attempts": [attempt.to_dict() for attempt in self._attempts],
            "current_test": self._current_test,
            "infrastructure_error": self._infrastructure_error,
            "manifest_sha256": self.identity.metadata.manifest_sha256,
            "raw_log": str(self._raw_log_path),
            "restart_count": self._restart_count,
            "run_id": self._run_id,
            "selected_ids": [test.test_id for test in self.selected],
        }

    def _persist_progress(self) -> None:
        data = _json_bytes(self._progress())
        if (
            self._resume_output_descriptor is None
            or self._resume_progress_descriptor is None
            or self._resume_progress_fingerprint is None
        ):
            _atomic_write(self._progress_path, data)
            return
        previous = self._resume_progress_descriptor
        replacement, replacement_fingerprint = self._replace_progress(
            self._resume_output_descriptor,
            previous,
            self._resume_progress_fingerprint,
            data,
        )
        self._resume_progress_descriptor = replacement
        self._resume_progress_fingerprint = replacement_fingerprint
        os.close(previous)

    def _replace_progress(
        self,
        output_descriptor: int,
        expected_descriptor: int,
        expected_fingerprint: tuple[int, ...],
        data: bytes,
    ) -> tuple[int, tuple[int, ...]]:
        temporary_name = (
            f".{self._progress_path.name}.{secrets.token_hex(8)}.resume"
        )
        generated_descriptor: int | None = None
        keep_generated = False
        try:
            generated_descriptor = os.open(
                temporary_name,
                os.O_RDWR
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                0o600,
                dir_fd=output_descriptor,
            )
            _write_all(generated_descriptor, data)
            os.fsync(generated_descriptor)
            if (
                _stat_fingerprint(os.fstat(expected_descriptor))
                != expected_fingerprint
                or not _entry_matches(
                    output_descriptor,
                    self._progress_path.name,
                    expected_descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU progress changed before checkpoint rewrite"
                )
            _rename_exchange(
                output_descriptor,
                self._progress_path.name,
                temporary_name,
            )
            os.fsync(output_descriptor)
            if not _entry_matches(
                output_descriptor,
                self._progress_path.name,
                generated_descriptor,
            ):
                raise ControllerError(
                    "resume QEMU progress changed during checkpoint rewrite"
                )
            if not _entry_matches(
                output_descriptor,
                temporary_name,
                expected_descriptor,
            ):
                displaced = os.stat(
                    temporary_name,
                    dir_fd=output_descriptor,
                    follow_symlinks=False,
                )
                displaced_identity = (displaced.st_dev, displaced.st_ino)
                try:
                    _rename_exchange(
                        output_descriptor,
                        self._progress_path.name,
                        temporary_name,
                    )
                    os.fsync(output_descriptor)
                except BaseException as rollback_error:
                    raise ControllerError(
                        "resume QEMU progress changed and rollback failed"
                    ) from rollback_error
                restored = os.stat(
                    self._progress_path.name,
                    dir_fd=output_descriptor,
                    follow_symlinks=False,
                )
                if (
                    (restored.st_dev, restored.st_ino) != displaced_identity
                    or not _entry_matches(
                        output_descriptor,
                        temporary_name,
                        generated_descriptor,
                    )
                ):
                    raise ControllerError(
                        "resume QEMU progress rewrite rollback is inconsistent"
                    )
                _remove_owned_result_entry(
                    output_descriptor,
                    temporary_name,
                    generated_descriptor,
                )
                temporary_name = ""
                raise ControllerError(
                    "resume QEMU progress changed during checkpoint rewrite"
                )
            _remove_owned_result_entry(
                output_descriptor,
                temporary_name,
                expected_descriptor,
            )
            temporary_name = ""
            if not _entry_matches(
                output_descriptor,
                self._progress_path.name,
                generated_descriptor,
            ):
                raise ControllerError(
                    "resume QEMU progress changed after checkpoint rewrite"
                )
            keep_generated = True
            return (
                generated_descriptor,
                _stat_fingerprint(os.fstat(generated_descriptor)),
            )
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU progress could not be rewritten safely"
            ) from error
        finally:
            cleanup_error: ControllerError | None = None
            if temporary_name and generated_descriptor is not None:
                try:
                    owned_descriptor: int | None = None
                    if _entry_matches(
                        output_descriptor,
                        temporary_name,
                        generated_descriptor,
                    ):
                        owned_descriptor = generated_descriptor
                    elif _entry_matches(
                        output_descriptor,
                        temporary_name,
                        expected_descriptor,
                    ):
                        owned_descriptor = expected_descriptor
                    if owned_descriptor is not None:
                        _remove_owned_result_entry(
                            output_descriptor,
                            temporary_name,
                            owned_descriptor,
                        )
                except ControllerError as error:
                    cleanup_error = error
                except OSError:
                    pass
            if generated_descriptor is not None and not keep_generated:
                os.close(generated_descriptor)
            if cleanup_error is not None:
                raise cleanup_error

    def _retire_progress(
        self,
        output_descriptor: int,
        expected_descriptor: int | None = None,
        expected_fingerprint: tuple[int, ...] | None = None,
        *,
        expected_bytes: bytes | None = None,
        result_descriptor: int | None = None,
        result_fingerprint: tuple[int, ...] | None = None,
    ) -> None:
        if result_descriptor is not None:
            if (
                expected_descriptor is None
                or expected_fingerprint is None
                or expected_bytes is None
            ):
                raise ControllerError(
                    "resume QEMU progress retirement lacks a validated checkpoint"
                )
            self._validate_retirement_result(
                output_descriptor,
                result_descriptor,
                result_fingerprint,
                "before progress retirement",
            )
            self._stage_progress_retirement(
                output_descriptor,
                expected_descriptor,
                expected_fingerprint,
            )
            try:
                try:
                    os.stat(
                        self._progress_path.name,
                        dir_fd=output_descriptor,
                        follow_symlinks=False,
                    )
                except FileNotFoundError:
                    pass
                else:
                    raise ControllerError(
                        "resume QEMU progress appeared during retirement"
                    )
                self._validate_retirement_result(
                    output_descriptor,
                    result_descriptor,
                    result_fingerprint,
                    "after progress staging",
                )
                self._validate_retirement_result(
                    output_descriptor,
                    result_descriptor,
                    result_fingerprint,
                    "before progress retirement commit",
                )
                try:
                    os.stat(
                        self._progress_path.name,
                        dir_fd=output_descriptor,
                        follow_symlinks=False,
                    )
                except FileNotFoundError:
                    pass
                else:
                    raise ControllerError(
                        "resume QEMU progress appeared before retirement commit"
                    )
            except BaseException as operation_error:
                try:
                    self._restore_staged_progress(
                        output_descriptor,
                        expected_descriptor,
                    )
                except BaseException as restore_error:
                    raise restore_error from operation_error
                raise
            try:
                _remove_owned_result_entry(
                    output_descriptor,
                    _PROGRESS_RETIREMENT_NAME,
                    expected_descriptor,
                )
            except BaseException as operation_error:
                try:
                    staged_entry_remains = _entry_matches(
                        output_descriptor,
                        _PROGRESS_RETIREMENT_NAME,
                        expected_descriptor,
                    )
                except FileNotFoundError:
                    staged_entry_remains = False
                except OSError as inspection_error:
                    raise ControllerError(
                        "resume QEMU progress retirement failure could not be inspected"
                    ) from inspection_error
                if staged_entry_remains:
                    try:
                        self._restore_staged_progress(
                            output_descriptor,
                            expected_descriptor,
                        )
                    except BaseException as restore_error:
                        raise restore_error from operation_error
                raise
            try:
                self._validate_retirement_result(
                    output_descriptor,
                    result_descriptor,
                    result_fingerprint,
                    "after progress retirement commit",
                )
                try:
                    os.stat(
                        self._progress_path.name,
                        dir_fd=output_descriptor,
                        follow_symlinks=False,
                    )
                except FileNotFoundError:
                    pass
                else:
                    raise ControllerError(
                        "resume QEMU progress appeared after retirement commit"
                    )
            except BaseException as operation_error:
                try:
                    self._recreate_retired_progress(
                        output_descriptor,
                        expected_bytes,
                    )
                except BaseException as restoration_error:
                    raise operation_error from restoration_error
                raise
            return
        if expected_descriptor is not None:
            if (
                expected_fingerprint is None
                or _stat_fingerprint(os.fstat(expected_descriptor))
                != expected_fingerprint
                or not _entry_matches(
                    output_descriptor,
                    self._progress_path.name,
                    expected_descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU progress changed before retirement"
                )
            _remove_owned_result_entry(
                output_descriptor,
                self._progress_path.name,
                expected_descriptor,
            )
            return
        os.unlink(self._progress_path.name, dir_fd=output_descriptor)
        try:
            os.fsync(output_descriptor)
        except OSError as error:
            raise ControllerError(
                "QEMU progress removal could not be synchronized"
            ) from error

    def _validate_retirement_result(
        self,
        output_descriptor: int,
        result_descriptor: int,
        result_fingerprint: tuple[int, ...] | None,
        phase: str,
    ) -> None:
        try:
            if (
                result_fingerprint is None
                or _stat_fingerprint(os.fstat(result_descriptor))
                != result_fingerprint
                or not _entry_matches(
                    output_descriptor,
                    self._result_path.name,
                    result_descriptor,
                )
            ):
                raise ControllerError(f"resume QEMU results changed {phase}")
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                f"resume QEMU results could not be revalidated {phase}"
            ) from error

    def _stage_progress_retirement(
        self,
        output_descriptor: int,
        expected_descriptor: int,
        expected_fingerprint: tuple[int, ...],
    ) -> None:
        try:
            if (
                _stat_fingerprint(os.fstat(expected_descriptor))
                != expected_fingerprint
                or not _entry_matches(
                    output_descriptor,
                    self._progress_path.name,
                    expected_descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU progress changed before retirement staging"
                )
            try:
                _rename_noreplace_between(
                    output_descriptor,
                    self._progress_path.name,
                    output_descriptor,
                    _PROGRESS_RETIREMENT_NAME,
                )
            except FileExistsError as error:
                raise ControllerError(
                    "resume QEMU progress retirement slot is already occupied"
                ) from error
            os.fsync(output_descriptor)
            if not _entry_matches(
                output_descriptor,
                _PROGRESS_RETIREMENT_NAME,
                expected_descriptor,
            ):
                raise ControllerError(
                    "resume QEMU progress changed during retirement staging"
                )
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU progress could not be staged for retirement"
            ) from error

    def _restore_staged_progress(
        self,
        output_descriptor: int,
        expected_descriptor: int,
    ) -> None:
        try:
            try:
                if _entry_matches(
                    output_descriptor,
                    self._progress_path.name,
                    expected_descriptor,
                ):
                    return
            except FileNotFoundError:
                pass
            if not _entry_matches(
                output_descriptor,
                _PROGRESS_RETIREMENT_NAME,
                expected_descriptor,
            ):
                raise ControllerError(
                    "resume QEMU staged progress changed before restoration"
                )
            try:
                _rename_noreplace_between(
                    output_descriptor,
                    _PROGRESS_RETIREMENT_NAME,
                    output_descriptor,
                    self._progress_path.name,
                )
            except FileExistsError as error:
                raise ControllerError(
                    "resume QEMU progress restoration is blocked; "
                    f"checkpoint retained at {_PROGRESS_RETIREMENT_NAME}"
                ) from error
            os.fsync(output_descriptor)
            if not _entry_matches(
                output_descriptor,
                self._progress_path.name,
                expected_descriptor,
            ):
                raise ControllerError(
                    "resume QEMU progress restoration is inconsistent"
                )
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU staged progress could not be restored safely"
            ) from error

    def _recreate_retired_progress(
        self,
        output_descriptor: int,
        data: bytes,
    ) -> None:
        temporary_name = (
            f".{self._progress_path.name}.{secrets.token_hex(8)}.restore"
        )
        descriptor: int | None = None
        installed_name: str | None = None
        try:
            descriptor = os.open(
                temporary_name,
                os.O_RDWR
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                0o600,
                dir_fd=output_descriptor,
            )
            _write_all(descriptor, data)
            os.fsync(descriptor)
            try:
                _rename_noreplace_between(
                    output_descriptor,
                    temporary_name,
                    output_descriptor,
                    self._progress_path.name,
                )
                installed_name = self._progress_path.name
            except FileExistsError:
                try:
                    _rename_noreplace_between(
                        output_descriptor,
                        temporary_name,
                        output_descriptor,
                        _PROGRESS_RETIREMENT_NAME,
                    )
                except FileExistsError as error:
                    raise ControllerError(
                        "resume QEMU progress restoration destinations are occupied"
                    ) from error
                installed_name = _PROGRESS_RETIREMENT_NAME
            temporary_name = ""
            os.fsync(output_descriptor)
            info = os.fstat(descriptor)
            if (
                not stat.S_ISREG(info.st_mode)
                or info.st_nlink != 1
                or info.st_size != len(data)
                or installed_name is None
                or not _entry_matches(
                    output_descriptor,
                    installed_name,
                    descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU recreated progress checkpoint is inconsistent"
                )
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU progress checkpoint could not be recreated safely"
            ) from error
        finally:
            cleanup_error: ControllerError | None = None
            if temporary_name and descriptor is not None:
                try:
                    if _entry_matches(
                        output_descriptor,
                        temporary_name,
                        descriptor,
                    ):
                        _remove_owned_result_entry(
                            output_descriptor,
                            temporary_name,
                            descriptor,
                        )
                except ControllerError as error:
                    cleanup_error = error
                except OSError:
                    pass
            if descriptor is not None:
                os.close(descriptor)
            if cleanup_error is not None:
                raise cleanup_error

    def _recover_interrupted_progress_retirement(
        self,
        output_descriptor: int,
    ) -> None:
        descriptor: int | None = None
        try:
            try:
                descriptor = os.open(
                    _PROGRESS_RETIREMENT_NAME,
                    os.O_RDONLY
                    | getattr(os, "O_CLOEXEC", 0)
                    | getattr(os, "O_NONBLOCK", 0)
                    | getattr(os, "O_NOFOLLOW", 0),
                    dir_fd=output_descriptor,
                )
            except FileNotFoundError:
                return
            info = os.fstat(descriptor)
            if (
                not stat.S_ISREG(info.st_mode)
                or info.st_nlink != 1
                or not _entry_matches(
                    output_descriptor,
                    _PROGRESS_RETIREMENT_NAME,
                    descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU interrupted progress retirement is unsafe"
                )
            self._restore_staged_progress(output_descriptor, descriptor)
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU interrupted progress retirement could not be recovered"
            ) from error
        finally:
            if descriptor is not None:
                os.close(descriptor)

    def _invalidate_results(
        self,
        output_descriptor: int,
        validated_prior_descriptor: int | None = None,
    ) -> int:
        prior_descriptor = validated_prior_descriptor
        marker_descriptor: int | None = None
        keep_marker = False
        marker_name = (
            f".{self._result_path.name}.{secrets.token_hex(8)}.invalid"
        )
        try:
            marker_descriptor = os.open(
                marker_name,
                os.O_RDWR
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                0o600,
                dir_fd=output_descriptor,
            )
            os.fsync(marker_descriptor)
            if prior_descriptor is None:
                try:
                    prior_descriptor = _open_result_entry(
                        output_descriptor,
                        self._result_path.name,
                    )
                except FileNotFoundError:
                    try:
                        _rename_noreplace(
                            output_descriptor,
                            marker_name,
                            self._result_path.name,
                        )
                    except FileExistsError as error:
                        raise ControllerError(
                            "QEMU results changed during invalidation"
                        ) from error
                    marker_name = ""
                    os.fsync(output_descriptor)
                    if not _entry_matches(
                        output_descriptor,
                        self._result_path.name,
                        marker_descriptor,
                    ):
                        raise ControllerError(
                            "QEMU results changed during invalidation"
                        )
                    keep_marker = True
                    return marker_descriptor
                except OSError as error:
                    raise ControllerError(
                        "existing QEMU results could not be opened safely"
                    ) from error
            prior_info = os.fstat(prior_descriptor)
            if not stat.S_ISREG(prior_info.st_mode) or prior_info.st_nlink != 1:
                raise ControllerError(
                    "existing QEMU results are not a regular single-link file"
                )
            if not _entry_matches(
                output_descriptor,
                self._result_path.name,
                prior_descriptor,
            ):
                raise ControllerError(
                    "existing QEMU results changed while being opened"
                )
            _rename_exchange(
                output_descriptor,
                self._result_path.name,
                marker_name,
            )
            os.fsync(output_descriptor)
            if not _entry_matches(
                output_descriptor,
                self._result_path.name,
                marker_descriptor,
            ):
                if _entry_matches(
                    output_descriptor,
                    marker_name,
                    prior_descriptor,
                ):
                    _remove_owned_result_entry(
                        output_descriptor,
                        marker_name,
                        prior_descriptor,
                    )
                    marker_name = ""
                raise ControllerError("QEMU results changed during invalidation")
            if not _entry_matches(
                output_descriptor,
                marker_name,
                prior_descriptor,
            ):
                displaced = os.stat(
                    marker_name,
                    dir_fd=output_descriptor,
                    follow_symlinks=False,
                )
                displaced_identity = (displaced.st_dev, displaced.st_ino)
                try:
                    _rename_exchange(
                        output_descriptor,
                        self._result_path.name,
                        marker_name,
                    )
                    os.fsync(output_descriptor)
                except BaseException as rollback_error:
                    raise ControllerError(
                        "QEMU results changed and rollback failed during invalidation"
                    ) from rollback_error
                try:
                    restored = os.stat(
                        self._result_path.name,
                        dir_fd=output_descriptor,
                        follow_symlinks=False,
                    )
                except OSError as rollback_error:
                    raise ControllerError(
                        "QEMU results invalidation rollback lost the replacement"
                    ) from rollback_error
                if (
                    (restored.st_dev, restored.st_ino) != displaced_identity
                    or not _entry_matches(
                        output_descriptor,
                        marker_name,
                        marker_descriptor,
                    )
                ):
                    raise ControllerError(
                        "QEMU results invalidation rollback is inconsistent"
                    )
                _remove_owned_result_entry(
                    output_descriptor,
                    marker_name,
                    marker_descriptor,
                )
                marker_name = ""
                raise ControllerError("QEMU results changed during invalidation")
            _remove_owned_result_entry(
                output_descriptor,
                marker_name,
                prior_descriptor,
            )
            marker_name = ""
            if not _entry_matches(
                output_descriptor,
                self._result_path.name,
                marker_descriptor,
            ):
                raise ControllerError("QEMU results changed during invalidation")
            keep_marker = True
            return marker_descriptor
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "QEMU results could not be invalidated safely"
            ) from error
        finally:
            cleanup_error: ControllerError | None = None
            if marker_name:
                try:
                    owned_descriptor: int | None = None
                    if marker_descriptor is not None and _entry_matches(
                        output_descriptor,
                        marker_name,
                        marker_descriptor,
                    ):
                        owned_descriptor = marker_descriptor
                    elif prior_descriptor is not None and _entry_matches(
                        output_descriptor,
                        marker_name,
                        prior_descriptor,
                    ):
                        owned_descriptor = prior_descriptor
                    if owned_descriptor is not None:
                        _remove_owned_result_entry(
                            output_descriptor,
                            marker_name,
                            owned_descriptor,
                        )
                except ControllerError as error:
                    cleanup_error = error
                except OSError:
                    pass
            try:
                if prior_descriptor is not None:
                    os.close(prior_descriptor)
            finally:
                if marker_descriptor is not None and not keep_marker:
                    os.close(marker_descriptor)
            if cleanup_error is not None:
                raise cleanup_error

    def _bind_result_marker(
        self,
        output_descriptor: int,
        validated_descriptor: int | None = None,
        *,
        expect_missing: bool = False,
    ) -> int:
        descriptor = validated_descriptor
        keep_descriptor = False
        try:
            if descriptor is None and expect_missing:
                try:
                    descriptor = os.open(
                        self._result_path.name,
                        os.O_RDWR
                        | os.O_CREAT
                        | os.O_EXCL
                        | getattr(os, "O_CLOEXEC", 0)
                        | getattr(os, "O_NOFOLLOW", 0),
                        0o600,
                        dir_fd=output_descriptor,
                    )
                except FileExistsError as error:
                    raise ControllerError(
                        "QEMU results changed while binding the resume marker"
                    ) from error
                os.fsync(descriptor)
                os.fsync(output_descriptor)
            elif descriptor is None:
                try:
                    descriptor = _open_result_entry(
                        output_descriptor,
                        self._result_path.name,
                    )
                except FileNotFoundError:
                    try:
                        descriptor = os.open(
                            self._result_path.name,
                            os.O_RDWR
                            | os.O_CREAT
                            | os.O_EXCL
                            | getattr(os, "O_CLOEXEC", 0)
                            | getattr(os, "O_NOFOLLOW", 0),
                            0o600,
                            dir_fd=output_descriptor,
                        )
                    except FileExistsError as error:
                        raise ControllerError(
                            "QEMU results changed while binding the resume marker"
                        ) from error
                    os.fsync(descriptor)
                    os.fsync(output_descriptor)
            info = os.fstat(descriptor)
            if (
                not stat.S_ISREG(info.st_mode)
                or info.st_nlink != 1
                or info.st_size != 0
                or not _entry_matches(
                    output_descriptor,
                    self._result_path.name,
                    descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU results are not the expected empty marker"
                )
            keep_descriptor = True
            return descriptor
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU result marker could not be opened safely"
            ) from error
        finally:
            if descriptor is not None and not keep_descriptor:
                os.close(descriptor)

    def _resume_result_is_committed(
        self,
        output_descriptor: int,
        *,
        checkpoint: _ResumeCheckpoint,
    ) -> bool:
        descriptor: int | None = None
        try:
            try:
                descriptor = _open_result_entry(
                    output_descriptor,
                    self._result_path.name,
                )
            except FileNotFoundError:
                return False
            opened = os.fstat(descriptor)
            if not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1:
                raise ControllerError(
                    "resume QEMU results are not a regular single-link file"
                )
            if not _entry_matches(
                output_descriptor,
                self._result_path.name,
                descriptor,
            ):
                raise ControllerError("resume QEMU results changed while being opened")
            if opened.st_size == 0:
                checkpoint.result_state = _ResumeResultState.MARKER
                checkpoint.result_descriptor = descriptor
                checkpoint.result_fingerprint = _stat_fingerprint(opened)
                descriptor = None
                return False
            if checkpoint.state is _ResumeCheckpointState.ACTIVE:
                raise ControllerError(
                    "resume QEMU results conflict with an active test checkpoint"
                )
            expected = self._result_bytes()
            if opened.st_size != len(expected):
                raise ControllerError(
                    "resume QEMU results do not match committed progress"
                )
            chunks: list[bytes] = []
            remaining = len(expected)
            while remaining:
                chunk = os.read(descriptor, min(65_536, remaining))
                if not chunk:
                    break
                chunks.append(chunk)
                remaining -= len(chunk)
            after = os.fstat(descriptor)
            if (
                _stat_fingerprint(opened) != _stat_fingerprint(after)
                or not _entry_matches(
                    output_descriptor,
                    self._result_path.name,
                    descriptor,
                )
            ):
                raise ControllerError("resume QEMU results changed while being read")
            if b"".join(chunks) != expected:
                raise ControllerError(
                    "resume QEMU results do not match committed progress"
                )
            checkpoint.result_state = _ResumeResultState.EXACT
            checkpoint.result_descriptor = descriptor
            checkpoint.result_fingerprint = _stat_fingerprint(after)
            descriptor = None
            return checkpoint.state is _ResumeCheckpointState.TERMINAL
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU results could not be validated safely"
            ) from error
        finally:
            if descriptor is not None:
                os.close(descriptor)

    def _validate_resume_checkpoint(
        self,
        output_descriptor: int,
        checkpoint: _ResumeCheckpoint,
    ) -> None:
        try:
            progress_descriptor = checkpoint.progress_descriptor
            if (
                progress_descriptor is None
                or _stat_fingerprint(os.fstat(progress_descriptor))
                != checkpoint.progress_fingerprint
                or not _entry_matches(
                    output_descriptor,
                    self._progress_path.name,
                    progress_descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU progress changed after validation"
                )
            if checkpoint.result_state is _ResumeResultState.ABSENT:
                try:
                    os.stat(
                        self._result_path.name,
                        dir_fd=output_descriptor,
                        follow_symlinks=False,
                    )
                except FileNotFoundError:
                    return
                raise ControllerError(
                    "resume QEMU results appeared after validation"
                )
            result_descriptor = checkpoint.result_descriptor
            if (
                result_descriptor is None
                or checkpoint.result_fingerprint is None
                or _stat_fingerprint(os.fstat(result_descriptor))
                != checkpoint.result_fingerprint
                or not _entry_matches(
                    output_descriptor,
                    self._result_path.name,
                    result_descriptor,
                )
            ):
                raise ControllerError(
                    "resume QEMU results changed after validation"
                )
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "resume QEMU checkpoint could not be revalidated safely"
            ) from error

    def _read_progress(
        self, output_descriptor: int
    ) -> tuple[bytes, int, tuple[int, ...]]:
        descriptor: int | None = None
        keep_descriptor = False
        try:
            descriptor = os.open(
                self._progress_path.name,
                os.O_RDONLY
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NONBLOCK", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=output_descriptor,
            )
            opened = os.fstat(descriptor)
            if not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1:
                raise ValueError("resume progress is not a regular single-link file")
            if opened.st_size > _MAX_PROGRESS_BYTES:
                raise ValueError("resume progress exceeds its size limit")
            chunks: list[bytes] = []
            total = 0
            while True:
                chunk = os.read(
                    descriptor,
                    min(65_536, _MAX_PROGRESS_BYTES + 1 - total),
                )
                if not chunk:
                    break
                chunks.append(chunk)
                total += len(chunk)
                if total > _MAX_PROGRESS_BYTES:
                    raise ValueError("resume progress exceeds its size limit")
            after = os.fstat(descriptor)
            if _stat_fingerprint(opened) != _stat_fingerprint(after):
                raise ValueError("resume progress changed while being read")
            keep_descriptor = True
            return b"".join(chunks), descriptor, _stat_fingerprint(after)
        except OSError as error:
            raise ValueError("resume progress is unavailable") from error
        finally:
            if descriptor is not None and not keep_descriptor:
                os.close(descriptor)

    def _load_progress(
        self, output_descriptor: int, raw_info: os.stat_result
    ) -> _ResumeCheckpoint:
        data, progress_descriptor, progress_fingerprint = self._read_progress(
            output_descriptor
        )
        try:
            state = self._apply_progress(data, raw_info)
        except BaseException:
            os.close(progress_descriptor)
            raise
        return _ResumeCheckpoint(
            state=state,
            progress_descriptor=progress_descriptor,
            progress_fingerprint=progress_fingerprint,
            progress_bytes=data,
        )

    def _apply_progress(
        self, data: bytes, raw_info: os.stat_result
    ) -> _ResumeCheckpointState:
        try:
            value = json.loads(
                data.decode("utf-8"),
                object_pairs_hook=_reject_progress_duplicate_keys,
            )
        except (UnicodeError, json.JSONDecodeError, ValueError) as error:
            raise ValueError("resume progress is invalid") from error
        if data != _json_bytes(value):
            raise ValueError("resume progress is not canonical LF JSON")
        if not isinstance(value, dict):
            raise ValueError("resume progress is invalid")
        if set(value) != {
            "boot_count",
            "build_id",
            "completed_attempts",
            "current_test",
            "infrastructure_error",
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
        infrastructure_error = value.get("infrastructure_error")
        if (
            not is_valid_run_id(run_id)
            or type(boot_count) is not int
            or boot_count < 0
            or type(restart_count) is not int
            or restart_count < 0
            or not isinstance(completed, list)
            or (
                infrastructure_error is not None
                and (
                    not isinstance(infrastructure_error, str)
                    or not infrastructure_error
                    or not _persisted_errors_are_valid(
                        None,
                        infrastructure_error,
                        _MAX_PERSISTED_ERROR_BYTES,
                    )
                )
            )
            or (
                current_test is not None
                and current_test not in expected_ids
            )
            or (infrastructure_error is not None and current_test is not None)
        ):
            raise ValueError("resume progress is invalid")
        if restart_count != max(boot_count - 1, 0):
            raise ValueError("resume progress is invalid")
        if boot_count == 0 and (
            completed
            or current_test is not None
            or infrastructure_error is not None
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
        first_incomplete = (
            expected_ids[len(completed_ids)]
            if len(completed_ids) < len(expected_ids)
            else None
        )
        if current_test is not None and current_test != first_incomplete:
            raise ValueError("resume progress is invalid")
        tests_by_id = {test.test_id: test for test in self.selected}
        for attempt in attempts:
            self._validate_resumed_attempt(
                attempt,
                tests_by_id[attempt.test_id],
                run_id,
                infrastructure_error,
            )
        required_size = max(
            (attempt.raw_log_end or 0 for attempt in attempts), default=0
        )
        if raw_info.st_size < required_size:
            raise ValueError("resume raw log is truncated")
        self._attempts = attempts
        self._boot_count = boot_count
        self._restart_count = restart_count
        self._run_id = run_id
        self._infrastructure_error = infrastructure_error
        self._current_test = None
        if current_test is not None:
            return _ResumeCheckpointState.ACTIVE
        if infrastructure_error is not None or completed_ids == expected_ids:
            return _ResumeCheckpointState.TERMINAL
        return _ResumeCheckpointState.INCOMPLETE

    def _validate_resumed_attempt(
        self,
        attempt: RuntimeAttempt,
        test: SuiteTest,
        run_id: str,
        terminal_infrastructure_error: str | None,
    ) -> None:
        build_status, link_status = self._build_statuses(test)
        field_limits = _persisted_attempt_field_limits(len(self.selected))
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
            and _persisted_stream_is_valid(
                attempt.stdout,
                attempt.stdout_bytes,
                attempt.stdout_truncated,
                field_limits.stream_bytes,
            )
            and _persisted_stream_is_valid(
                attempt.stderr,
                attempt.stderr_bytes,
                attempt.stderr_truncated,
                field_limits.stream_bytes,
            )
            and _persisted_errors_are_valid(
                attempt.launch_error,
                attempt.infrastructure_error,
                field_limits.error_bytes,
            )
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
                if attempt.status == "interrupted":
                    evidence_matches = (
                        terminal_infrastructure_error is not None
                        and attempt.infrastructure_error
                        == terminal_infrastructure_error
                        and attempt.resource_evidence == "unavailable"
                        and not attempt.resource_deltas.has_nonzero()
                    )
                else:
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

    def _open_raw_log(self, output_descriptor: int, *, resume: bool):
        descriptor: int | None = None
        flags = (
            os.O_WRONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NONBLOCK", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        if resume:
            flags |= os.O_APPEND
        else:
            flags |= os.O_CREAT
        try:
            try:
                descriptor = os.open(
                    self._raw_log_path.name,
                    flags,
                    0o644,
                    dir_fd=output_descriptor,
                )
            except OSError as error:
                raise ControllerError(
                    "QEMU raw log could not be opened safely"
                ) from error
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
                raise ControllerError(
                    "QEMU raw log is not a regular single-link file"
                )
            if resume:
                os.lseek(descriptor, 0, os.SEEK_END)
            else:
                os.ftruncate(descriptor, 0)
            stream = os.fdopen(descriptor, "ab" if resume else "wb")
            descriptor = None
            return stream
        finally:
            if descriptor is not None:
                os.close(descriptor)

    def _wait_for_prompt(self, transport: _Transport, raw) -> bool:
        deadline = self._monotonic() + self.config.boot_timeout_seconds
        tail = self._prompt_tail
        self._prompt_tail = b""
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
        self._prompt_tail = b""
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
        terminal = _terminal_event_offset(data)
        if terminal is None or terminal[0] != "suite_end":
            raise ControllerError("guest POSIX suite has no terminal event")
        _event_name, end_offset = terminal
        try:
            parsed = parse_serial_log(data.decode("utf-8", errors="replace"))
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
        starts = [
            event.values
            for event in parsed.events
            if event.event in {"suite_start", "test_start"}
        ]
        if len(starts) != 2 or not _start_pair_is_valid(
            starts[0], starts[1], test, self.identity
        ):
            raise ControllerError("guest start events do not match the command")
        post_terminal = data[end_offset:]
        prompt_after = PROMPT in post_terminal
        self._prompt_tail = (
            b"" if prompt_after else post_terminal[-(_STREAM_TOKEN_BYTES - 1) :]
        )
        return attempt, end_offset, prompt_after

    def _validate_guest_infrastructure_error(
        self, data: bytes, test: SuiteTest
    ) -> tuple[SerialAttempt | None, str, int, bool]:
        terminal = _terminal_event_offset(data)
        if terminal is None or terminal[0] != "infrastructure_error":
            raise ControllerError("guest POSIX suite has no infrastructure terminal")
        _event_name, end_offset = terminal
        try:
            parsed = parse_serial_log(data.decode("utf-8", errors="replace"))
        except ValueError as error:
            raise ControllerError(
                f"invalid guest POSIX event stream: {error}"
            ) from error
        if (
            parsed.complete
            or parsed.terminal_event is None
            or parsed.terminal_event.event != "infrastructure_error"
            or not parsed.infrastructure_error
            or len(parsed.attempts) > 1
        ):
            raise ControllerError("guest POSIX infrastructure terminal is invalid")
        if parsed.events[0].event == "suite_start":
            suite_start = parsed.events[0].values
            if not _suite_start_is_valid(suite_start, test, self.identity):
                raise ControllerError(
                    "guest POSIX start events do not match the command"
                )
            test_starts = [
                event.values
                for event in parsed.events
                if event.event == "test_start"
            ]
            if test_starts and (
                len(test_starts) != 1
                or not _start_pair_is_valid(
                    suite_start,
                    test_starts[0],
                    test,
                    self.identity,
                )
            ):
                raise ControllerError(
                    "guest POSIX start events do not match the command"
                )
        elif len(parsed.events) != 1 or parsed.attempts:
            raise ControllerError("guest POSIX preflight terminal is invalid")
        attempt = parsed.attempts[0] if parsed.attempts else None
        if attempt is not None and (
            attempt.test_id != test.test_id
            or attempt.group != test.group
            or attempt.api != test.api
        ):
            raise ControllerError("guest test identity does not match the command")
        post_terminal = data[end_offset:]
        prompt_after = PROMPT in post_terminal
        self._prompt_tail = (
            b"" if prompt_after else post_terminal[-(_STREAM_TOKEN_BYTES - 1) :]
        )
        return (
            attempt,
            parsed.infrastructure_error,
            end_offset,
            prompt_after,
        )

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
        field_limits = _persisted_attempt_field_limits(len(self.selected))
        stdout, stdout_bytes, stdout_truncated = _bounded_stream(
            guest.stdout, field_limits.stream_bytes
        )
        stderr, stderr_bytes, stderr_truncated = _bounded_stream(
            guest.stderr, field_limits.stream_bytes
        )
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
            stdout=stdout,
            stderr=stderr,
            source=SOURCE,
            launch_error=(
                _bounded_reason(guest.launch_error, field_limits.error_bytes)
                if guest.launch_error is not None
                else None
            ),
            infrastructure_error=(
                _bounded_reason(
                    guest.infrastructure_error,
                    field_limits.error_bytes,
                )
                if guest.infrastructure_error is not None
                else None
            ),
            stdout_bytes=stdout_bytes,
            stderr_bytes=stderr_bytes,
            stdout_truncated=stdout_truncated,
            stderr_truncated=stderr_truncated,
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
        field_limits = _persisted_attempt_field_limits(len(self.selected))
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
            infrastructure_error=_bounded_reason(
                reason,
                field_limits.error_bytes,
            ),
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
            remaining_capacity = self.config.max_test_serial_bytes - len(data)
            overflow = len(chunk) > max(0, remaining_capacity)
            if remaining_capacity > 0:
                data.extend(chunk[:remaining_capacity])
            matching_start = matching_start or _matching_start_seen(
                bytes(data), test, self.identity
            )
            if overflow:
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
            terminal = _terminal_event_offset(bytes(data))
            if terminal is not None:
                if terminal[0] == "infrastructure_error":
                    (
                        guest,
                        infrastructure_error,
                        _end_offset,
                        prompt_after,
                    ) = self._validate_guest_infrastructure_error(
                        bytes(data), test
                    )
                    field_limits = _persisted_attempt_field_limits(
                        len(self.selected)
                    )
                    self._infrastructure_error = _bounded_reason(
                        infrastructure_error,
                        field_limits.error_bytes,
                    )
                    attempt = (
                        self._guest_attempt(
                            guest,
                            test,
                            raw_log_start=raw_start,
                            raw_log_end=raw.tell(),
                        )
                        if guest is not None
                        else None
                    )
                    return attempt, prompt_after, False
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

    def _complete(self) -> bool:
        attempt_ids = [attempt.test_id for attempt in self._attempts]
        selected_ids = [test.test_id for test in self.selected]
        return (
            attempt_ids == selected_ids
            and self._infrastructure_error is None
            and not any(
                attempt.infrastructure_error
                and attempt.source != WATCHDOG_SOURCE
                for attempt in self._attempts
            )
        )

    def _terminal(self) -> dict[str, object]:
        terminal: dict[str, object] = {
            "boot_count": self._boot_count,
            "build_id": self.identity.build_id,
            "build_results_sha256": self.identity.metadata.build_results_sha256,
            "complete": self._complete(),
            "completed_count": len(self._attempts),
            "manifest_sha256": self.identity.metadata.manifest_sha256,
            "patch_sha256": self.identity.metadata.patch_sha256,
            "platform": PLATFORM,
            "qemu": _bounded_reason(" ".join(self.config.qemu_argv)),
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
        if self._infrastructure_error is not None:
            terminal["infrastructure_error"] = self._infrastructure_error
        return terminal

    def _result_bytes(self) -> bytes:
        rows = [_attempt_record(attempt) for attempt in self._attempts]
        rows.append(self._terminal())
        return _canonical_report(rows)

    def _publish(
        self,
        output_descriptor: int,
        marker_descriptor: int,
    ) -> None:
        data = self._result_bytes()
        temporary_name = (
            f".{self._result_path.name}.{secrets.token_hex(8)}.publish"
        )
        generated_descriptor: int | None = None
        try:
            generated_descriptor = os.open(
                temporary_name,
                os.O_WRONLY
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_CLOEXEC", 0)
                | getattr(os, "O_NOFOLLOW", 0),
                0o644,
                dir_fd=output_descriptor,
            )
            _write_all(generated_descriptor, data)
            os.fsync(generated_descriptor)
            _rename_exchange(
                output_descriptor,
                self._result_path.name,
                temporary_name,
            )
            os.fsync(output_descriptor)
            generated_is_public = _entry_matches(
                output_descriptor,
                self._result_path.name,
                generated_descriptor,
            )
            marker_was_displaced = _entry_matches(
                output_descriptor,
                temporary_name,
                marker_descriptor,
            )
            if not generated_is_public:
                if marker_was_displaced:
                    _remove_owned_result_entry(
                        output_descriptor,
                        temporary_name,
                        marker_descriptor,
                    )
                    temporary_name = ""
                raise ControllerError("QEMU results changed during publication")
            if not marker_was_displaced:
                displaced = os.stat(
                    temporary_name,
                    dir_fd=output_descriptor,
                    follow_symlinks=False,
                )
                displaced_identity = (displaced.st_dev, displaced.st_ino)
                try:
                    _rename_exchange(
                        output_descriptor,
                        self._result_path.name,
                        temporary_name,
                    )
                    os.fsync(output_descriptor)
                except BaseException as rollback_error:
                    raise ControllerError(
                        "QEMU results changed and rollback failed during publication"
                    ) from rollback_error
                restored = os.stat(
                    self._result_path.name,
                    dir_fd=output_descriptor,
                    follow_symlinks=False,
                )
                if (
                    (restored.st_dev, restored.st_ino) != displaced_identity
                    or not _entry_matches(
                        output_descriptor,
                        temporary_name,
                        generated_descriptor,
                    )
                ):
                    raise ControllerError(
                        "QEMU results publication rollback is inconsistent"
                    )
                _remove_owned_result_entry(
                    output_descriptor,
                    temporary_name,
                    generated_descriptor,
                )
                temporary_name = ""
                raise ControllerError("QEMU results changed during publication")
            _remove_owned_result_entry(
                output_descriptor,
                temporary_name,
                marker_descriptor,
            )
            temporary_name = ""
            if not _entry_matches(
                output_descriptor,
                self._result_path.name,
                generated_descriptor,
            ):
                raise ControllerError("QEMU results changed during publication")
        except ControllerError:
            raise
        except OSError as error:
            raise ControllerError(
                "QEMU results could not be published safely"
            ) from error
        finally:
            cleanup_error: ControllerError | None = None
            if temporary_name:
                try:
                    owned_descriptor: int | None = None
                    if generated_descriptor is not None and _entry_matches(
                        output_descriptor,
                        temporary_name,
                        generated_descriptor,
                    ):
                        owned_descriptor = generated_descriptor
                    elif _entry_matches(
                        output_descriptor,
                        temporary_name,
                        marker_descriptor,
                    ):
                        owned_descriptor = marker_descriptor
                    if owned_descriptor is not None:
                        _remove_owned_result_entry(
                            output_descriptor,
                            temporary_name,
                            owned_descriptor,
                        )
                except ControllerError as error:
                    cleanup_error = error
                except OSError:
                    pass
            if generated_descriptor is not None:
                os.close(generated_descriptor)
            if cleanup_error is not None:
                raise cleanup_error

    def run(self, resume: bool = False) -> ControllerResult:
        output_descriptor = _open_parent(self._output)
        campaign_lock_descriptor: int | None = None
        marker_descriptor: int | None = None
        checkpoint: _ResumeCheckpoint | None = None
        published = False
        operation_error: BaseException | None = None
        try:
            campaign_lock_descriptor = _open_campaign_lock(output_descriptor)
            self._recover_interrupted_progress_retirement(output_descriptor)
            if not resume:
                run_id = self._run_id_factory()
                if not is_valid_run_id(run_id):
                    raise ValueError("QEMU run ID is invalid")
                self._run_id = run_id
                self._attempts = []
                self._infrastructure_error = None
                self._restart_count = 0
                self._boot_count = 0
                marker_descriptor = self._invalidate_results(output_descriptor)
            with self._open_raw_log(output_descriptor, resume=resume) as raw:
                recovered_result = False
                if resume:
                    checkpoint = self._load_progress(
                        output_descriptor,
                        os.fstat(raw.fileno()),
                    )
                    recovered_result = self._resume_result_is_committed(
                        output_descriptor,
                        checkpoint=checkpoint,
                    )
                    self._validate_resume_checkpoint(
                        output_descriptor,
                        checkpoint,
                    )
                    if recovered_result:
                        self._retire_progress(
                            output_descriptor,
                            checkpoint.progress_descriptor,
                            checkpoint.progress_fingerprint,
                            expected_bytes=checkpoint.progress_bytes,
                            result_descriptor=checkpoint.result_descriptor,
                            result_fingerprint=checkpoint.result_fingerprint,
                        )
                        published = True
                    else:
                        self._resume_output_descriptor = output_descriptor
                        self._resume_progress_descriptor = (
                            checkpoint.take_progress_descriptor()
                        )
                        self._resume_progress_fingerprint = (
                            checkpoint.progress_fingerprint
                        )
                        self._persist_progress()
                        if (
                            checkpoint.state
                            is _ResumeCheckpointState.INCOMPLETE
                            and checkpoint.result_state
                            is _ResumeResultState.EXACT
                        ):
                            marker_descriptor = self._invalidate_results(
                                output_descriptor,
                                checkpoint.take_result_descriptor(),
                            )
                        elif (
                            checkpoint.result_state
                            is _ResumeResultState.MARKER
                        ):
                            marker_descriptor = self._bind_result_marker(
                                output_descriptor,
                                checkpoint.take_result_descriptor(),
                            )
                        else:
                            marker_descriptor = self._bind_result_marker(
                                output_descriptor,
                                expect_missing=True,
                            )
                if not recovered_result:
                    transport: _Transport | None = None
                    prompt_ready = False
                    completed_ids = {attempt.test_id for attempt in self._attempts}
                    self._persist_progress()
                    try:
                        for test in self.selected:
                            if self._infrastructure_error is not None:
                                break
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
                            if attempt is not None:
                                self._attempts.append(attempt)
                                completed_ids.add(test.test_id)
                            self._current_test = None
                            self._persist_progress()
                            if self._infrastructure_error is not None:
                                break
                            if restart:
                                self._stop(transport)
                                transport = None
                                prompt_ready = False
                        assert marker_descriptor is not None
                        self._publish(output_descriptor, marker_descriptor)
                        published = True
                        self._retire_progress(
                            output_descriptor,
                            self._resume_progress_descriptor,
                            self._resume_progress_fingerprint,
                        )
                    finally:
                        if transport is not None:
                            self._stop(transport)
        except BaseException as error:
            operation_error = error
        cleanup_errors: list[BaseException] = []
        if marker_descriptor is not None:
            if not published:
                try:
                    if not _entry_matches(
                        output_descriptor,
                        self._result_path.name,
                        marker_descriptor,
                    ):
                        cleanup_errors.append(ControllerError(
                            "QEMU results changed during campaign cleanup"
                        ))
                except OSError as error:
                    cleanup_error = ControllerError(
                        "QEMU results could not be validated during campaign cleanup"
                    )
                    cleanup_error.__cause__ = error
                    cleanup_errors.append(cleanup_error)
            try:
                os.close(marker_descriptor)
            except OSError as error:
                cleanup_error = ControllerError(
                    "QEMU result marker could not be closed safely"
                )
                cleanup_error.__cause__ = error
                cleanup_errors.append(cleanup_error)
        if checkpoint is not None:
            try:
                checkpoint.close()
            except BaseException as error:
                cleanup_errors.append(error)
        resume_progress_descriptor = self._resume_progress_descriptor
        self._resume_output_descriptor = None
        self._resume_progress_descriptor = None
        self._resume_progress_fingerprint = None
        if resume_progress_descriptor is not None:
            try:
                os.close(resume_progress_descriptor)
            except BaseException as error:
                cleanup_errors.append(error)
        try:
            os.close(output_descriptor)
        except OSError as error:
            cleanup_errors.append(error)
        if campaign_lock_descriptor is not None:
            try:
                os.close(campaign_lock_descriptor)
            except OSError as error:
                cleanup_errors.append(error)
        if cleanup_errors:
            cleanup_error: BaseException
            if len(cleanup_errors) == 1:
                cleanup_error = cleanup_errors[0]
            else:
                cleanup_error = BaseExceptionGroup(
                    "QEMU campaign cleanup failed",
                    cleanup_errors,
                )
            if operation_error is not None:
                raise cleanup_error from operation_error
            raise cleanup_error
        if operation_error is not None:
            raise operation_error
        return ControllerResult(
            attempts=tuple(self._attempts),
            complete=self._complete(),
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
