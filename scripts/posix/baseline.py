"""Run staged AArch64 POSIX tests under the Linux qemu-user reference."""

from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Mapping, Sequence
from contextlib import contextmanager
import ctypes
from dataclasses import asdict, dataclass, replace
import errno
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import secrets
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

from .build import (
    CHECKSUM_DEFINITION,
    MAX_HOST_MANIFEST_BYTES,
    MAX_MANIFEST_BYTES,
    ManifestMetadata,
    _build_results_digest,
    _load_build_results,
    parse_manifest,
    sha256_file,
    verify_stage,
)
from .model import (
    BuildResult,
    PTS_FAIL,
    PTS_PASS,
    PTS_UNRESOLVED,
    PTS_UNSUPPORTED,
    PTS_UNTESTED,
    RuntimeAttempt,
    SuiteTest,
)


PLATFORM = "aarch64-linux-reference"
SOURCE = "qemu-user"
MAX_CAPTURE_BYTES = 16_384
_TRUNCATION_MARKER = b"\n...[truncated]"
_TERMINATE_GRACE_SECONDS = 0.2
_DRAIN_GRACE_SECONDS = 0.5
_KILL_REAP_SECONDS = 1.0
_SUPERVISOR_SHUTDOWN_SECONDS = 2.7
_DIGEST_LENGTH = 64
_SUPERVISOR_CONTROL_MAX_BYTES = 4096
_INFRASTRUCTURE_ERROR_MAX_BYTES = 4096
_PR_SET_CHILD_SUBREAPER = 36
_PR_GET_CHILD_SUBREAPER = 37


@dataclass(frozen=True)
class BaselineResult:
    attempts: tuple[RuntimeAttempt, ...]
    all_passed: bool
    result_path: Path


@dataclass(frozen=True)
class _Capture:
    text: str
    byte_count: int
    truncated: bool


@dataclass(frozen=True)
class _RuntimeObservation:
    returncode: int | None
    timed_out: bool
    stdout: _Capture
    stderr: _Capture
    launch_status: str
    launch_error: str | None = None
    infrastructure_error: str | None = None


@dataclass(frozen=True)
class _StageIdentity:
    metadata: ManifestMetadata
    tests: tuple[SuiteTest, ...]
    runtime: tuple[tuple[str, str], ...]
    build_results: tuple[BuildResult, ...]
    manifest_data: bytes
    host_data: bytes
    build_id: str
    runtime_snapshot_sha256: str = ""


class _ProcessLaunchError(Exception):
    def __init__(self, error: OSError) -> None:
        super().__init__(str(error))
        self.error = error


class BaselinePrerequisiteError(ValueError):
    """A missing or unusable host prerequisite for the reference runner."""


@dataclass(frozen=True)
class _ProcessIdentity:
    pid: int
    start_time: int


@dataclass(frozen=True)
class _ProcessEntry:
    identity: _ProcessIdentity
    parent_pid: int


def _process_table() -> dict[int, _ProcessEntry]:
    table: dict[int, _ProcessEntry] = {}
    try:
        entries = tuple(Path("/proc").iterdir())
    except OSError as error:
        raise ValueError(f"cannot inspect Linux process table: {error}") from error
    for path in entries:
        if not path.name.isdecimal():
            continue
        try:
            text = (path / "stat").read_text(encoding="ascii")
            closing = text.rfind(")")
            fields = text[closing + 2 :].split()
            pid = int(path.name)
            parent_pid = int(fields[1])
            start_time = int(fields[19])
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        except (IndexError, UnicodeError, ValueError) as error:
            raise ValueError(f"invalid Linux process identity: {path}") from error
        identity = _ProcessIdentity(pid=pid, start_time=start_time)
        table[pid] = _ProcessEntry(identity=identity, parent_pid=parent_pid)
    return table


def _prctl(option: int, value: int | ctypes.c_void_p) -> int:
    libc = ctypes.CDLL(None, use_errno=True)
    result = int(libc.prctl(option, value, 0, 0, 0))
    if result != 0:
        error_number = ctypes.get_errno()
        raise OSError(error_number, os.strerror(error_number))
    return result


def _child_subreaper_enabled() -> bool:
    current = ctypes.c_int()
    _prctl(_PR_GET_CHILD_SUBREAPER, ctypes.byref(current))
    return current.value != 0


@contextmanager
def _child_subreaper():
    changed = not _child_subreaper_enabled()
    if changed:
        _prctl(_PR_SET_CHILD_SUBREAPER, 1)
    try:
        yield
    finally:
        if changed:
            _prctl(_PR_SET_CHILD_SUBREAPER, 0)


class _LinuxProcessTree:
    def __init__(self, *, claim_adopted_children: bool = True) -> None:
        self._owner_pid = os.getpid()
        self._claim_adopted_children = claim_adopted_children
        initial = _process_table()
        self._baseline_children = {
            entry.identity
            for entry in initial.values()
            if entry.parent_pid == self._owner_pid
        }
        self._root: _ProcessIdentity | None = None
        self._tracked: dict[_ProcessIdentity, int] = {}

    def attach(self, pid: int) -> None:
        entry = _process_table().get(pid)
        if entry is None:
            return
        self._root = entry.identity
        self._track(entry.identity)

    def _track(self, identity: _ProcessIdentity) -> None:
        if identity in self._tracked:
            return
        if not hasattr(signal, "pidfd_send_signal"):
            raise ValueError("Linux pidfd signaling support is unavailable")
        try:
            descriptor = os.pidfd_open(identity.pid, 0)
        except AttributeError as error:
            raise ValueError("Linux pidfd support is unavailable") from error
        except OSError as error:
            if error.errno == errno.ESRCH:
                return
            if error.errno == errno.ENOSYS:
                raise ValueError("Linux pidfd support is unavailable") from error
            raise
        current = _process_table().get(identity.pid)
        if current is None or current.identity != identity:
            os.close(descriptor)
            return
        self._tracked[identity] = descriptor

    def _discover(self) -> dict[_ProcessIdentity, _ProcessEntry]:
        table = _process_table()
        discovered: dict[_ProcessIdentity, _ProcessEntry] = {}
        if self._root is not None:
            root_entry = table.get(self._root.pid)
            if root_entry is not None and root_entry.identity == self._root:
                discovered[self._root] = root_entry
        for identity in tuple(self._tracked):
            entry = table.get(identity.pid)
            if entry is not None and entry.identity == identity:
                discovered[identity] = entry
        for entry in table.values():
            if (
                self._claim_adopted_children
                and entry.parent_pid == self._owner_pid
                and entry.identity not in self._baseline_children
            ):
                discovered[entry.identity] = entry
        descendant_pids = {identity.pid for identity in discovered}
        changed = True
        while changed:
            changed = False
            for entry in table.values():
                if (
                    entry.parent_pid in descendant_pids
                    and entry.identity not in discovered
                ):
                    discovered[entry.identity] = entry
                    descendant_pids.add(entry.identity.pid)
                    changed = True
        for identity in discovered:
            self._track(identity)
        return discovered

    def signal(self, sig: int) -> None:
        discovered = self._discover()
        for identity in sorted(
            discovered, key=lambda item: item.pid, reverse=True
        ):
            descriptor = self._tracked.get(identity)
            try:
                if descriptor is not None:
                    signal.pidfd_send_signal(descriptor, sig)
            except ProcessLookupError:
                pass

    def refresh(self) -> None:
        self._discover()

    def _reap_adopted(self) -> None:
        discovered = self._discover()
        for identity, entry in discovered.items():
            if identity == self._root or entry.parent_pid != self._owner_pid:
                continue
            try:
                waited, _status = os.waitpid(identity.pid, os.WNOHANG)
            except ChildProcessError:
                continue
            if waited == identity.pid:
                descriptor = self._tracked.pop(identity, None)
                if descriptor is not None:
                    os.close(descriptor)

    def _remaining(self) -> tuple[_ProcessIdentity, ...]:
        self._reap_adopted()
        return tuple(self._discover())

    def cleanup(self) -> None:
        remaining = self._remaining()
        if not remaining:
            return
        self.signal(signal.SIGTERM)
        terminate_deadline = time.monotonic() + _TERMINATE_GRACE_SECONDS
        while time.monotonic() < terminate_deadline:
            if not self._remaining():
                return
            time.sleep(0.01)
        self.signal(signal.SIGKILL)
        kill_deadline = time.monotonic() + _KILL_REAP_SECONDS
        while time.monotonic() < kill_deadline:
            if not self._remaining():
                return
            time.sleep(0.01)
        remaining = self._remaining()
        if remaining:
            raise ValueError(
                "runtime process tree could not be reaped after SIGKILL: "
                + ",".join(str(identity.pid) for identity in remaining)
            )

    def close(self) -> None:
        for descriptor in self._tracked.values():
            try:
                os.close(descriptor)
            except OSError:
                pass
        self._tracked.clear()


def classify_status(
    returncode: int | None,
    *,
    timed_out: bool = False,
    launch_error: str | None = None,
) -> str:
    if launch_error is not None:
        return "launch-error"
    if timed_out:
        return "timeout"
    if returncode is not None and returncode < 0:
        return "crash"
    return {
        PTS_PASS: "pass",
        PTS_FAIL: "fail",
        PTS_UNRESOLVED: "unresolved",
        PTS_UNSUPPORTED: "unsupported",
        PTS_UNTESTED: "untested",
    }.get(returncode, "fail")


def _pts_status(
    returncode: int | None,
    *,
    timed_out: bool,
    launch_error: str | None,
) -> str | None:
    if (
        returncode is None
        or returncode < 0
        or timed_out
        or launch_error is not None
    ):
        return None
    return classify_status(returncode)


def filter_runnable_tests(
    tests: Sequence[SuiteTest],
    *,
    api: str | None = None,
    group: str | None = None,
    test_id: str | None = None,
) -> tuple[SuiteTest, ...]:
    selected = tuple(
        test
        for test in tests
        if test.kind == "runnable"
        and test.disposition == "complete"
        and (api is None or test.api == api)
        and (group is None or test.group == group)
        and (test_id is None or test.test_id == test_id)
    )
    if not selected:
        raise ValueError("filter selected no complete runnable tests")
    return selected


def _fit_utf8(data: bytes, maximum: int) -> str:
    text = data.decode("utf-8", errors="replace")
    while len(text.encode("utf-8")) > maximum:
        text = text[:-1]
    return text


def _bounded_detail(
    value: str, maximum: int = _INFRASTRUCTURE_ERROR_MAX_BYTES
) -> str:
    marker = b"\n...[truncated]"
    data = value.encode("utf-8", errors="replace")
    if len(data) <= maximum:
        return data.decode("utf-8")
    prefix_bytes = maximum - len(marker)
    return (
        _fit_utf8(data[:prefix_bytes], prefix_bytes)
        + marker.decode("ascii")
    )


def _captured(data: bytearray, byte_count: int) -> _Capture:
    truncated = byte_count > MAX_CAPTURE_BYTES
    if truncated:
        prefix_limit = MAX_CAPTURE_BYTES - len(_TRUNCATION_MARKER)
        text = _fit_utf8(bytes(data[:prefix_limit]), prefix_limit)
        text += _TRUNCATION_MARKER.decode("ascii")
    else:
        text = _fit_utf8(bytes(data), MAX_CAPTURE_BYTES)
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    return _Capture(text=text, byte_count=byte_count, truncated=truncated)


def _kill_group(process: subprocess.Popen[bytes], sig: int) -> None:
    try:
        os.killpg(process.pid, sig)
    except ProcessLookupError:
        pass


def _reap(process: subprocess.Popen[bytes]) -> None:
    if process.returncode is not None:
        return
    try:
        process.wait(timeout=1.0)
    except subprocess.TimeoutExpired:
        _kill_group(process, signal.SIGKILL)
        try:
            process.wait(timeout=1.0)
        except subprocess.TimeoutExpired as error:
            raise ValueError(
                "runtime process could not be reaped after SIGKILL"
            ) from error


def _close_process_streams(process: subprocess.Popen[bytes]) -> None:
    for stream in (
        getattr(process, "stdin", None),
        getattr(process, "stdout", None),
        getattr(process, "stderr", None),
    ):
        if stream is not None:
            try:
                stream.close()
            except BaseException:
                pass


def _force_process_tree_cleanup(
    process: subprocess.Popen[bytes], process_tree: _LinuxProcessTree
) -> None:
    errors: list[BaseException] = []
    try:
        process_tree.signal(signal.SIGKILL)
    except BaseException as error:
        errors.append(error)
    try:
        _reap(process)
    except BaseException as error:
        errors.append(error)
    try:
        process_tree.cleanup()
    except BaseException as error:
        errors.append(error)
    if errors:
        failure = ValueError(
            "runtime process cleanup failed: "
            + "; ".join(f"{type(error).__name__}: {error}" for error in errors)
        )
        raise failure from errors[0]


def _rescue_known_process(process: subprocess.Popen[bytes]) -> None:
    descriptor: int | None = None
    try:
        if process.poll() is not None:
            return
        try:
            descriptor = os.pidfd_open(process.pid, 0)
        except (AttributeError, OSError):
            descriptor = None

        def send(sig: int) -> None:
            if process.poll() is not None:
                return
            try:
                if descriptor is not None:
                    signal.pidfd_send_signal(descriptor, sig)
                else:
                    os.kill(process.pid, sig)
            except ProcessLookupError:
                pass

        send(signal.SIGCONT)
        send(signal.SIGTERM)
        try:
            process.wait(timeout=_SUPERVISOR_SHUTDOWN_SECONDS)
        except subprocess.TimeoutExpired:
            send(signal.SIGKILL)
            try:
                process.wait(timeout=_KILL_REAP_SECONDS)
            except subprocess.TimeoutExpired as error:
                raise ValueError(
                    "runtime broker could not be reaped after SIGKILL"
                ) from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _attach_process_tree(
    process: subprocess.Popen[bytes], process_tree: _LinuxProcessTree
) -> None:
    try:
        process_tree.attach(process.pid)
    except BaseException as original:
        cleanup_errors: list[BaseException] = []
        try:
            _rescue_known_process(process)
        except BaseException as error:
            cleanup_errors.append(error)
        try:
            process_tree.cleanup()
        except BaseException as error:
            cleanup_errors.append(error)
        _close_process_streams(process)
        process_tree.close()
        if cleanup_errors:
            cleanup_error = ValueError(
                "runtime process cleanup failed: "
                + "; ".join(
                    f"{type(error).__name__}: {error}"
                    for error in cleanup_errors
                )
            )
            raise cleanup_error from original
        raise


class _SupervisorInterrupted(BaseException):
    def __init__(self, signum: int) -> None:
        super().__init__(signum)
        self.signum = signum


def _require_pidfd_support() -> None:
    if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
        raise ValueError("Linux pidfd signaling support is unavailable")
    try:
        descriptor = os.pidfd_open(os.getpid(), 0)
    except OSError as error:
        if error.errno in {errno.ENOSYS, errno.EINVAL}:
            raise ValueError("Linux pidfd signaling support is unavailable") from error
        raise
    try:
        signal.pidfd_send_signal(descriptor, 0)
    except OSError as error:
        if error.errno in {errno.ENOSYS, errno.EINVAL}:
            raise ValueError("Linux pidfd signaling support is unavailable") from error
        raise
    finally:
        os.close(descriptor)


def _write_supervisor_control(
    descriptor: int, payload: Mapping[str, object]
) -> None:
    def encoded(value: Mapping[str, object]) -> bytes:
        return (
            json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n"
        ).encode("ascii")

    bounded = dict(payload)
    data = encoded(bounded)
    if len(data) > _SUPERVISOR_CONTROL_MAX_BYTES:
        marker = "\n...[truncated]"
        scalable: dict[str, str] = {}
        candidate = dict(bounded)
        for field in ("message", "strerror"):
            value = bounded.get(field)
            if not isinstance(value, str):
                continue
            serialized_length = len(json.dumps(value).encode("ascii"))
            if serialized_length <= 512:
                continue
            source = _bounded_detail(
                value, _SUPERVISOR_CONTROL_MAX_BYTES
            )
            if source.endswith(marker):
                source = source[: -len(marker)]
            scalable[field] = source
            candidate[field] = marker
        if scalable and len(encoded(candidate)) <= _SUPERVISOR_CONTROL_MAX_BYTES:
            low = 0
            high = 1_000_000
            while low < high:
                middle = (low + high + 1) // 2
                candidate = dict(bounded)
                for field, source in scalable.items():
                    length = len(source) * middle // 1_000_000
                    candidate[field] = source[:length] + marker
                if len(encoded(candidate)) <= _SUPERVISOR_CONTROL_MAX_BYTES:
                    low = middle
                else:
                    high = middle - 1
            candidate = dict(bounded)
            for field, source in scalable.items():
                length = len(source) * low // 1_000_000
                candidate[field] = source[:length] + marker
            bounded = candidate
            data = encoded(bounded)
    if len(data) > _SUPERVISOR_CONTROL_MAX_BYTES:
        data = encoded(
            {
                "kind": "infrastructure_error",
                "message": "runtime supervisor control data exceeded its size limit",
            }
        )
    try:
        view = memoryview(data)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                break
            view = view[written:]
    except OSError:
        pass
    finally:
        os.close(descriptor)


def _mirror_returncode(returncode: int) -> int:
    if returncode < 0:
        signum = -returncode
        if signum not in {signal.SIGKILL, signal.SIGSTOP}:
            signal.signal(signum, signal.SIG_DFL)
        os.kill(os.getpid(), signum)
        return 128 + signum
    return returncode


def launch_runtime(argv: Sequence[str], control_descriptor: int) -> int:
    """Launch QEMU beneath the stable subreaper broker."""

    returncode = 125
    try:
        process = subprocess.Popen(list(argv))
    except OSError as error:
        payload: dict[str, object] = {
            "errno": error.errno,
            "kind": "launch_error",
            "strerror": error.strerror or str(error),
        }
    else:
        returncode = process.wait()
        payload = {"kind": "result", "returncode": returncode}
    _write_supervisor_control(control_descriptor, payload)
    return _mirror_returncode(returncode)


def _read_launcher_control(descriptor: int) -> dict[str, object] | None:
    data = bytearray()
    while True:
        chunk = os.read(descriptor, _SUPERVISOR_CONTROL_MAX_BYTES + 1)
        if not chunk:
            break
        data.extend(chunk)
        if len(data) > _SUPERVISOR_CONTROL_MAX_BYTES:
            raise ValueError("runtime launcher control message exceeds its size limit")
    if not data:
        return None
    return _parse_supervisor_control(bytes(data))


def supervise_runtime(argv: Sequence[str], control_descriptor: int) -> int:
    """Run one QEMU attempt beneath a trusted, exclusive subreaper broker."""

    previous_handlers: dict[int, object] = {}

    def interrupted(signum: int, _frame: object) -> None:
        raise _SupervisorInterrupted(signum)

    def infrastructure_payload(error: BaseException) -> dict[str, object]:
        result: dict[str, object] = {
            "kind": "infrastructure_error",
            "message": f"{type(error).__name__}: {error}",
        }
        if launcher_payload is not None:
            if launcher_payload["kind"] == "result":
                result["returncode"] = launcher_payload["returncode"]
            elif launcher_payload["kind"] == "launch_error":
                result["errno"] = launcher_payload["errno"]
                result["strerror"] = launcher_payload["strerror"]
        return result

    for signum in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
        previous_handlers[signum] = signal.signal(signum, interrupted)

    payload: dict[str, object]
    returncode = 125
    process_tree: _LinuxProcessTree | None = None
    process: subprocess.Popen[bytes] | None = None
    launcher_read: int | None = None
    launcher_write: int | None = None
    launcher_payload: dict[str, object] | None = None
    try:
        with _child_subreaper():
            _require_pidfd_support()
            process_tree = _LinuxProcessTree()
            launcher_read, launcher_write = os.pipe2(
                getattr(os, "O_CLOEXEC", 0)
            )
            try:
                process = subprocess.Popen(
                    _launcher_command(argv, launcher_write),
                    pass_fds=(launcher_write,),
                    start_new_session=True,
                )
            except OSError as error:
                payload = infrastructure_payload(error)
            else:
                os.close(launcher_write)
                launcher_write = None
                try:
                    _attach_process_tree(process, process_tree)
                    returncode = process.wait()
                    launcher_payload = _read_launcher_control(launcher_read)
                    os.close(launcher_read)
                    launcher_read = None
                    process_tree.cleanup()
                except _SupervisorInterrupted as error:
                    try:
                        _force_process_tree_cleanup(process, process_tree)
                    except BaseException as cleanup_error:
                        payload = infrastructure_payload(cleanup_error)
                        returncode = 125
                    else:
                        returncode = -error.signum
                        payload = {"kind": "result", "returncode": returncode}
                except BaseException as error:
                    try:
                        _force_process_tree_cleanup(process, process_tree)
                    except BaseException as cleanup_error:
                        cleanup_error.add_note(
                            f"while handling {type(error).__name__}: {error}"
                        )
                        error = cleanup_error
                    payload = infrastructure_payload(error)
                    returncode = 125
                else:
                    if launcher_payload is None:
                        payload = infrastructure_payload(
                            ValueError(
                                "runtime launcher returned invalid control data"
                            )
                        )
                        returncode = 125
                    elif launcher_payload["kind"] == "launch_error":
                        payload = launcher_payload
                        returncode = 125
                    elif launcher_payload["kind"] == "result":
                        value = launcher_payload.get("returncode")
                        if not isinstance(value, int):
                            raise ValueError(
                                "runtime launcher returned invalid control data"
                            )
                        returncode = value
                        payload = launcher_payload
                    else:
                        raise ValueError(
                            "runtime launcher returned invalid control data"
                        )
    except _SupervisorInterrupted as error:
        if process is not None and process_tree is not None:
            try:
                _force_process_tree_cleanup(process, process_tree)
            except BaseException as cleanup_error:
                payload = infrastructure_payload(cleanup_error)
                returncode = 125
            else:
                returncode = -error.signum
                payload = {"kind": "result", "returncode": returncode}
        else:
            returncode = -error.signum
            payload = {"kind": "result", "returncode": returncode}
    except BaseException as error:
        if process is not None and process_tree is not None:
            try:
                _force_process_tree_cleanup(process, process_tree)
            except BaseException as cleanup_error:
                cleanup_error.add_note(
                    f"while handling {type(error).__name__}: {error}"
                )
                error = cleanup_error
        payload = infrastructure_payload(error)
        returncode = 125
    finally:
        for descriptor in (launcher_read, launcher_write):
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
        if process_tree is not None:
            process_tree.close()
        for signum, previous in previous_handlers.items():
            signal.signal(signum, previous)

    _write_supervisor_control(control_descriptor, payload)
    return _mirror_returncode(returncode)


def _supervisor_command(
    argv: Sequence[str], control_descriptor: int
) -> list[str]:
    supervisor = Path(__file__).with_name("supervisor.py").resolve()
    return [
        sys.executable,
        "-B",
        str(supervisor),
        str(control_descriptor),
        *argv,
    ]


def _launcher_command(
    argv: Sequence[str], control_descriptor: int
) -> list[str]:
    supervisor = Path(__file__).with_name("supervisor.py").resolve()
    return [
        sys.executable,
        "-B",
        str(supervisor),
        "launch",
        str(control_descriptor),
        *argv,
    ]


def _request_supervisor_shutdown(process_tree: _LinuxProcessTree) -> None:
    process_tree.signal(signal.SIGCONT)
    process_tree.signal(signal.SIGTERM)


def _parse_supervisor_control(data: bytes) -> dict[str, object]:
    if not data:
        raise ValueError("runtime supervisor returned invalid control data")
    if len(data) > _SUPERVISOR_CONTROL_MAX_BYTES:
        raise ValueError("runtime supervisor control message exceeds its size limit")
    try:
        text = data.decode("ascii")
        payload = json.loads(text, object_pairs_hook=_reject_duplicate_keys)
    except (UnicodeError, json.JSONDecodeError, ValueError) as error:
        raise ValueError("runtime supervisor returned invalid control data") from error
    if not isinstance(payload, dict):
        raise ValueError("runtime supervisor returned invalid control data")
    kind = payload.get("kind")
    if kind == "result":
        returncode = payload.get("returncode")
        valid = (
            set(payload) == {"kind", "returncode"}
            and isinstance(returncode, int)
            and not isinstance(returncode, bool)
        )
    elif kind == "launch_error":
        error_number = payload.get("errno")
        valid = (
            set(payload) == {"errno", "kind", "strerror"}
            and isinstance(error_number, int)
            and not isinstance(error_number, bool)
            and isinstance(payload.get("strerror"), str)
        )
    elif kind == "infrastructure_error":
        keys = set(payload)
        valid = isinstance(payload.get("message"), str)
        if keys == {"kind", "message", "returncode"}:
            returncode = payload["returncode"]
            valid = (
                valid
                and isinstance(returncode, int)
                and not isinstance(returncode, bool)
            )
        elif keys == {"errno", "kind", "message", "strerror"}:
            error_number = payload["errno"]
            valid = (
                valid
                and isinstance(error_number, int)
                and not isinstance(error_number, bool)
                and isinstance(payload["strerror"], str)
            )
        elif keys != {"kind", "message"}:
            valid = False
    else:
        valid = False
    if not valid:
        raise ValueError("runtime supervisor returned invalid control data")
    return payload


def _runtime_observation(
    process: subprocess.Popen[bytes] | None,
    qemu: str,
    control_data: bytearray,
    buffers: Mapping[str, bytearray],
    counts: Mapping[str, int],
    *,
    timed_out: bool,
) -> _RuntimeObservation | None:
    if process is None or process.returncode is None:
        return None
    try:
        control = _parse_supervisor_control(bytes(control_data))
    except ValueError:
        return _RuntimeObservation(
            returncode=None,
            timed_out=timed_out,
            stdout=_captured(buffers["stdout"], counts["stdout"]),
            stderr=_captured(buffers["stderr"], counts["stderr"]),
            launch_status="interrupted",
        )
    if control["kind"] == "launch_error":
        error_number = control.get("errno")
        if not isinstance(error_number, int):
            error_number = errno.ENOENT
        message = control.get("strerror")
        if not isinstance(message, str):
            message = os.strerror(error_number)
        returncode = None
        launch_status = "launch-error"
        launch_error = _bounded_detail(f"{message}: {qemu}")
        infrastructure_error = None
    elif control["kind"] == "result":
        value = control.get("returncode")
        if not isinstance(value, int):
            return None
        returncode = value
        launch_status = "launched"
        launch_error = None
        infrastructure_error = None
    elif control["kind"] == "infrastructure_error":
        if "returncode" in control:
            value = control["returncode"]
            assert isinstance(value, int)
            returncode = value
            launch_status = "launched"
            launch_error = None
        elif "errno" in control:
            error_number = control["errno"]
            strerror = control["strerror"]
            assert isinstance(error_number, int)
            assert isinstance(strerror, str)
            returncode = None
            launch_status = "launch-error"
            launch_error = _bounded_detail(f"{strerror}: {qemu}")
        else:
            returncode = None
            launch_status = "interrupted"
            launch_error = None
        message = control["message"]
        assert isinstance(message, str)
        infrastructure_error = _bounded_detail(message)
    else:
        return None
    return _RuntimeObservation(
        returncode=returncode,
        timed_out=timed_out,
        stdout=_captured(buffers["stdout"], counts["stdout"]),
        stderr=_captured(buffers["stderr"], counts["stderr"]),
        launch_status=launch_status,
        launch_error=launch_error,
        infrastructure_error=infrastructure_error,
    )


def _run_captured(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: float,
) -> tuple[int, bool, _Capture, _Capture]:
    _require_pidfd_support()
    process_tree = _LinuxProcessTree(claim_adopted_children=False)
    pipe_flags = getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NONBLOCK", 0)
    control_read: int | None
    control_write: int | None
    control_read, control_write = os.pipe2(pipe_flags)
    process: subprocess.Popen[bytes] | None = None
    control_stream: object | None = None
    selector: selectors.BaseSelector | None = None
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    counts = {"stdout": 0, "stderr": 0}
    control_data = bytearray()
    timed_out = False
    try:
        process = subprocess.Popen(
            _supervisor_command(argv, control_write),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=cwd,
            env=dict(env),
            start_new_session=True,
            pass_fds=(control_write,),
        )
        _attach_process_tree(process, process_tree)
        os.close(control_write)
        control_write = None
        control_stream = os.fdopen(control_read, "rb", buffering=0)
        control_read = None
        assert process.stdout is not None and process.stderr is not None
        selector = selectors.DefaultSelector()
        streams: dict[int, object] = {}
        for name, stream in (("stdout", process.stdout), ("stderr", process.stderr)):
            descriptor = stream.fileno()
            os.set_blocking(descriptor, False)
            selector.register(descriptor, selectors.EVENT_READ, name)
            streams[descriptor] = stream
        control_descriptor = control_stream.fileno()  # type: ignore[attr-defined]
        selector.register(control_descriptor, selectors.EVENT_READ, "control")
        streams[control_descriptor] = control_stream
        deadline = time.monotonic() + timeout_seconds
        terminate_deadline: float | None = None
        drain_deadline: float | None = None

        def close_stream(descriptor: int) -> None:
            try:
                selector.unregister(descriptor)
            except (KeyError, ValueError):
                pass
            stream = streams.pop(descriptor, None)
            if stream is not None:
                stream.close()  # type: ignore[attr-defined]

        while True:
            returncode = process.poll()
            now = time.monotonic()
            if not timed_out and returncode is None and now >= deadline:
                timed_out = True
                process_tree.refresh()
                _request_supervisor_shutdown(process_tree)
                terminate_deadline = now + _SUPERVISOR_SHUTDOWN_SECONDS
            if (
                timed_out
                and terminate_deadline is not None
                and now >= terminate_deadline
            ):
                _rescue_known_process(process)
                process_tree.cleanup()
                terminate_deadline = None
                continue
            if returncode is not None and drain_deadline is None:
                drain_deadline = now + _DRAIN_GRACE_SECONDS
                if timed_out:
                    process_tree.signal(signal.SIGKILL)
            if returncode is not None and not streams:
                break
            if drain_deadline is not None and now >= drain_deadline:
                process_tree.signal(signal.SIGKILL)
                for descriptor in tuple(streams):
                    close_stream(descriptor)
                break
            active_deadlines = [] if timed_out else [deadline]
            if terminate_deadline is not None:
                active_deadlines.append(terminate_deadline)
            if drain_deadline is not None:
                active_deadlines.append(drain_deadline)
            assert active_deadlines
            wait = max(0.0, min(0.05, min(active_deadlines) - now))
            for key, _mask in selector.select(wait):
                descriptor = int(key.fd)
                try:
                    chunk = os.read(descriptor, 8192)
                except BlockingIOError:
                    continue
                if not chunk:
                    close_stream(descriptor)
                    continue
                name = str(key.data)
                if name == "control":
                    control_data.extend(chunk)
                    if len(control_data) > _SUPERVISOR_CONTROL_MAX_BYTES:
                        raise ValueError(
                            "runtime supervisor control message exceeds its size limit"
                        )
                    continue
                counts[name] += len(chunk)
                remaining = MAX_CAPTURE_BYTES - len(buffers[name])
                if remaining > 0:
                    buffers[name].extend(chunk[:remaining])

        _reap(process)
        process_tree.cleanup()
        control = _parse_supervisor_control(bytes(control_data))
        if control["kind"] == "launch_error":
            error_number = control.get("errno")
            if not isinstance(error_number, int):
                error_number = errno.ENOENT
            message = control.get("strerror")
            if not isinstance(message, str):
                message = os.strerror(error_number)
            raise _ProcessLaunchError(OSError(error_number, message))
        if control["kind"] == "infrastructure_error":
            message = control.get("message")
            raise ValueError(
                message if isinstance(message, str) else "runtime supervisor failed"
            )
        supervised_returncode = process.returncode
        if control["kind"] == "result":
            value = control.get("returncode")
            if not isinstance(value, int):
                raise ValueError("runtime supervisor returned invalid control data")
            supervised_returncode = value
        return (
            int(supervised_returncode),
            timed_out,
            _captured(buffers["stdout"], counts["stdout"]),
            _captured(buffers["stderr"], counts["stderr"]),
        )
    except BaseException as original:
        cleanup_error: BaseException | None = None
        if process is not None:
            try:
                process_tree.refresh()
            except BaseException as error:
                cleanup_error = error
            try:
                _request_supervisor_shutdown(process_tree)
            except BaseException as error:
                if cleanup_error is None:
                    cleanup_error = error
            try:
                process.wait(timeout=_SUPERVISOR_SHUTDOWN_SECONDS)
            except BaseException:
                try:
                    _force_process_tree_cleanup(process, process_tree)
                except BaseException as error:
                    cleanup_error = error
            else:
                try:
                    process_tree.cleanup()
                except BaseException as error:
                    cleanup_error = error
        failure = cleanup_error if cleanup_error is not None else original
        observation = _runtime_observation(
            process,
            str(argv[0]),
            control_data,
            buffers,
            counts,
            timed_out=timed_out,
        )
        if observation is not None and observation.launch_error is not None and (
            cleanup_error is not None
            or not isinstance(original, _ProcessLaunchError)
        ):
            cleanup_failure = (
                cleanup_error if cleanup_error is not None else original
            )
            observation = replace(
                observation,
                infrastructure_error=_bounded_infrastructure_error(
                    cleanup_failure
                ),
            )
            prerequisite = BaselinePrerequisiteError(
                observation.launch_error
            )
            setattr(
                prerequisite,
                "_smros_posix_runtime_observation",
                observation,
            )
            raise prerequisite from cleanup_failure
        if observation is not None:
            try:
                setattr(failure, "_smros_posix_runtime_observation", observation)
            except BaseException:
                pass
        if cleanup_error is not None:
            raise cleanup_error from original
        raise
    finally:
        if selector is not None:
            try:
                selector.close()
            except BaseException:
                pass
        if process is not None:
            _close_process_streams(process)
        if control_stream is not None:
            try:
                control_stream.close()  # type: ignore[attr-defined]
            except BaseException:
                pass
        for descriptor in (control_read, control_write):
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
        process_tree.close()


def _snapshot_binary(test: SuiteTest, stage: Path, cwd: Path) -> Path:
    if test.binary is None or test.sha256 is None:
        raise ValueError(f"missing staged binary for {test.test_id}")
    relative = _safe_relative(test.binary, "staged binary")
    source = stage.joinpath(*relative.parts)
    try:
        path_info = source.lstat()
    except OSError as error:
        raise ValueError(f"missing binary for {test.test_id}: {source}") from error
    if stat.S_ISLNK(path_info.st_mode) or not stat.S_ISREG(path_info.st_mode):
        raise ValueError(f"binary is not a regular file for {test.test_id}")
    if stat.S_IMODE(path_info.st_mode) != 0o755:
        raise ValueError(f"invalid executable mode for {test.test_id}")
    read_flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    source_descriptor = os.open(source, read_flags)
    destination = cwd / relative.name
    destination_descriptor: int | None = None
    try:
        opened_info = os.fstat(source_descriptor)
        if (
            not stat.S_ISREG(opened_info.st_mode)
            or (opened_info.st_dev, opened_info.st_ino)
            != (path_info.st_dev, path_info.st_ino)
            or stat.S_IMODE(opened_info.st_mode) != 0o755
        ):
            raise ValueError(f"binary changed while being opened: {test.test_id}")
        write_flags = (
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        destination_descriptor = os.open(destination, write_flags, 0o700)
        digest = hashlib.sha256()
        while chunk := os.read(source_descriptor, 1024 * 1024):
            digest.update(chunk)
            view = memoryview(chunk)
            while view:
                written = os.write(destination_descriptor, view)
                if written <= 0:
                    raise OSError("short write while snapshotting test binary")
                view = view[written:]
        after_info = os.fstat(source_descriptor)
        fingerprint = lambda info: (
            info.st_dev,
            info.st_ino,
            info.st_mode,
            info.st_size,
            info.st_mtime_ns,
            info.st_ctime_ns,
        )
        if fingerprint(opened_info) != fingerprint(after_info):
            raise ValueError(f"binary changed while being copied: {test.test_id}")
        if digest.hexdigest() != test.sha256:
            raise ValueError(f"binary checksum mismatch for {test.test_id}")
        os.fchmod(destination_descriptor, 0o755)
        os.fsync(destination_descriptor)
        os.close(destination_descriptor)
        destination_descriptor = None
        return destination
    except BaseException:
        if destination_descriptor is not None:
            try:
                os.close(destination_descriptor)
            except BaseException:
                pass
        try:
            destination.unlink()
        except FileNotFoundError:
            pass
        raise
    finally:
        os.close(source_descriptor)


def run_runtime_attempt(
    test: SuiteTest,
    *,
    stage: Path,
    sysroot: Path,
    qemu: Path,
    metadata: ManifestMetadata,
    build_id: str,
    build_status: str,
    link_status: str,
    run_id: str = "",
    runtime_snapshot_sha256: str = "",
) -> RuntimeAttempt:
    started = time.monotonic()
    launch_error: str | None = None
    returncode: int | None = None
    timed_out = False
    stdout = _Capture("", 0, False)
    stderr = _Capture("", 0, False)
    attempt_observed = False
    pending_error: BaseException | None = None
    try:
        with tempfile.TemporaryDirectory(
            prefix="smros-posix-baseline-"
        ) as temporary:
            cwd = Path(temporary)
            binary = _snapshot_binary(test, stage, cwd)
            environment = {
                "PATH": os.defpath,
                "LANG": "C",
                "LC_ALL": "C",
                "TMPDIR": str(cwd),
                "LD_LIBRARY_PATH": "/lib:/usr/lib",
            }
            try:
                returncode, timed_out, stdout, stderr = _run_captured(
                    [str(qemu), "-L", str(sysroot.resolve()), str(binary)],
                    cwd=cwd,
                    env=environment,
                    timeout_seconds=test.timeout_ms / 1000.0,
                )
            except _ProcessLaunchError as failure:
                error = failure.error
                launch_error = _bounded_detail(
                    f"{error.strerror or str(error)}: {qemu}"
                )
            except BaseException as error:
                pending_error = error
                raise
            attempt_observed = True
    except BaseException as cleanup_error:
        if pending_error is not None:
            if cleanup_error is pending_error:
                raise
            observation = getattr(
                pending_error,
                "_smros_posix_runtime_observation",
                None,
            )
            if isinstance(observation, _RuntimeObservation):
                inner_detail = (
                    observation.infrastructure_error
                    or _bounded_infrastructure_error(pending_error)
                )
                observation = replace(
                    observation,
                    infrastructure_error=_combined_infrastructure_error(
                        inner_detail, cleanup_error
                    ),
                )
                setattr(
                    pending_error,
                    "_smros_posix_runtime_observation",
                    observation,
                )
            else:
                detail = _combined_infrastructure_error(
                    _bounded_infrastructure_error(pending_error),
                    cleanup_error,
                )
                try:
                    setattr(
                        pending_error,
                        "_smros_posix_infrastructure_error",
                        detail,
                    )
                except BaseException:
                    pass
            raise pending_error from cleanup_error
        if not attempt_observed:
            raise
        launch_status = (
            "launch-error" if launch_error is not None else "launched"
        )
        observation = _RuntimeObservation(
            returncode=returncode,
            timed_out=timed_out,
            stdout=stdout,
            stderr=stderr,
            launch_status=launch_status,
            launch_error=launch_error,
            infrastructure_error=_bounded_infrastructure_error(
                cleanup_error
            ),
        )
        if launch_error is not None:
            prerequisite = BaselinePrerequisiteError(launch_error)
            setattr(
                prerequisite,
                "_smros_posix_runtime_observation",
                observation,
            )
            raise prerequisite from cleanup_error
        try:
            setattr(
                cleanup_error,
                "_smros_posix_runtime_observation",
                observation,
            )
        except BaseException:
            pass
        raise
    duration_ms = max(0, int((time.monotonic() - started) * 1000))
    signum = -returncode if returncode is not None and returncode < 0 else None
    exit_code = returncode if returncode is not None and returncode >= 0 else None
    return RuntimeAttempt(
        test_id=test.test_id,
        group=test.group,
        api=test.api,
        platform=PLATFORM,
        build_status=build_status,
        link_status=link_status,
        launch_status="launch-error" if launch_error is not None else "launched",
        pts_status=_pts_status(
            returncode,
            timed_out=timed_out,
            launch_error=launch_error,
        ),
        status=classify_status(
            returncode, timed_out=timed_out, launch_error=launch_error
        ),
        exit_code=exit_code,
        signal=signum,
        timed_out=timed_out,
        duration_ms=duration_ms,
        stdout=stdout.text,
        stderr=stderr.text,
        source=SOURCE,
        launch_error=launch_error,
        stdout_bytes=stdout.byte_count,
        stderr_bytes=stderr.byte_count,
        stdout_truncated=stdout.truncated,
        stderr_truncated=stderr.truncated,
        manifest_sha256=metadata.manifest_sha256,
        build_results_sha256=metadata.build_results_sha256,
        build_id=build_id,
        revision=metadata.revision,
        patch_sha256=metadata.patch_sha256,
        smros_commit=metadata.smros_commit,
        binary_sha256=test.sha256,
        runtime_snapshot_sha256=runtime_snapshot_sha256,
        run_id=run_id,
    )


def _bounded_infrastructure_error(error: BaseException) -> str:
    return _bounded_detail(f"{type(error).__name__}: {error}")


def _combined_infrastructure_error(
    current: str | None, error: BaseException
) -> str:
    detail = _bounded_infrastructure_error(error)
    if current is None:
        return detail
    content_bytes = _INFRASTRUCTURE_ERROR_MAX_BYTES - 1
    current_bytes = content_bytes // 2
    detail_bytes = content_bytes - current_bytes
    return (
        _bounded_detail(current, current_bytes)
        + "\n"
        + _bounded_detail(detail, detail_bytes)
    )


def _interrupted_attempt(
    test: SuiteTest,
    *,
    metadata: ManifestMetadata,
    build_id: str,
    build_status: str,
    link_status: str,
    run_id: str,
    runtime_snapshot_sha256: str,
    started: float,
    error: BaseException,
) -> RuntimeAttempt:
    observation = getattr(
        error, "_smros_posix_runtime_observation", None
    )
    if not isinstance(observation, _RuntimeObservation):
        observation = None
    returncode = observation.returncode if observation is not None else None
    launch_status = (
        observation.launch_status if observation is not None else "interrupted"
    )
    launch_error = (
        observation.launch_error if observation is not None else None
    )
    signum = -returncode if returncode is not None and returncode < 0 else None
    exit_code = (
        returncode if returncode is not None and returncode >= 0 else None
    )
    timed_out = observation.timed_out if observation is not None else False
    stdout = (
        observation.stdout
        if observation is not None
        else _Capture("", 0, False)
    )
    stderr = (
        observation.stderr
        if observation is not None
        else _Capture("", 0, False)
    )
    infrastructure_error = (
        observation.infrastructure_error
        if observation is not None
        else None
    )
    transferred_error = getattr(
        error, "_smros_posix_infrastructure_error", None
    )
    if not isinstance(transferred_error, str):
        transferred_error = None
    infrastructure_error = (
        infrastructure_error
        or transferred_error
        or _bounded_infrastructure_error(error)
    )
    return RuntimeAttempt(
        test_id=test.test_id,
        group=test.group,
        api=test.api,
        platform=PLATFORM,
        build_status=build_status,
        link_status=link_status,
        launch_status=launch_status,
        pts_status=_pts_status(
            returncode,
            timed_out=timed_out,
            launch_error=launch_error,
        ),
        status="launch-error" if launch_error is not None else "interrupted",
        exit_code=exit_code,
        signal=signum,
        timed_out=timed_out,
        duration_ms=max(0, int((time.monotonic() - started) * 1000)),
        stdout=stdout.text,
        stderr=stderr.text,
        source=SOURCE,
        launch_error=launch_error,
        infrastructure_error=infrastructure_error,
        stdout_bytes=stdout.byte_count,
        stderr_bytes=stderr.byte_count,
        stdout_truncated=stdout.truncated,
        stderr_truncated=stderr.truncated,
        manifest_sha256=metadata.manifest_sha256,
        build_results_sha256=metadata.build_results_sha256,
        build_id=build_id,
        revision=metadata.revision,
        patch_sha256=metadata.patch_sha256,
        smros_commit=metadata.smros_commit,
        binary_sha256=test.sha256,
        runtime_snapshot_sha256=runtime_snapshot_sha256,
        run_id=run_id,
    )


def _read_regular(path: Path, label: str, maximum: int) -> bytes:
    try:
        before = path.lstat()
    except OSError as error:
        raise ValueError(f"missing {label}: {path}") from error
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
        raise ValueError(f"{label} is not a regular file: {path}")
    if before.st_size > maximum:
        raise ValueError(f"{label} exceeds its size limit")
    flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    descriptor = os.open(path, flags)
    try:
        opened = os.fstat(descriptor)
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise ValueError(f"{label} changed while being opened")
        data = bytearray()
        while chunk := os.read(descriptor, min(65_536, maximum + 1 - len(data))):
            data.extend(chunk)
            if len(data) > maximum:
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
        return bytes(data)
    finally:
        os.close(descriptor)


def _reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _safe_relative(value: str, label: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if (
        not value
        or "\\" in value
        or path.is_absolute()
        or path.as_posix() != value
        or any(part in {"", ".", ".."} for part in value.split("/"))
    ):
        raise ValueError(f"unsafe {label}: {value!r}")
    return path


def _load_stage_identity(stage: Path) -> _StageIdentity:
    manifest_data = _read_regular(
        stage / "manifest.tsv", "manifest.tsv", MAX_MANIFEST_BYTES
    )
    metadata, tests = parse_manifest(manifest_data)
    host_data = _read_regular(
        stage / "manifest.json", "manifest.json", MAX_HOST_MANIFEST_BYTES
    )
    try:
        text = host_data.decode("utf-8")
        host = json.loads(text, object_pairs_hook=_reject_duplicate_keys)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("manifest.json is invalid JSON") from error
    if not isinstance(host, dict) or set(host) != {
        "schema",
        "checksum_definition",
        "metadata",
        "runtime",
        "tests",
    }:
        raise ValueError("manifest.json schema is invalid")
    canonical = (
        json.dumps(
            host, sort_keys=True, separators=(",", ":"), ensure_ascii=True
        )
        + "\n"
    )
    if text != canonical:
        raise ValueError("manifest.json is not canonical JSON")
    if host["schema"] != 1 or host["checksum_definition"] != CHECKSUM_DEFINITION:
        raise ValueError("manifest.json schema is invalid")
    if host["metadata"] != asdict(metadata) or host["tests"] != [
        asdict(test) for test in tests
    ]:
        raise ValueError("manifest.json differs from manifest.tsv")
    raw_runtime = host["runtime"]
    if not isinstance(raw_runtime, list):
        raise ValueError("manifest runtime is invalid")
    runtime: list[tuple[str, str]] = []
    seen: set[str] = set()
    for entry in raw_runtime:
        if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
            raise ValueError("manifest runtime entry is invalid")
        relative = entry["path"]
        digest = entry["sha256"]
        if not isinstance(relative, str) or not isinstance(digest, str):
            raise ValueError("manifest runtime entry is invalid")
        parsed = _safe_relative(relative, "runtime path")
        if len(parsed.parts) != 2 or parsed.parts[0] != "lib":
            raise ValueError(f"unsafe runtime path: {relative!r}")
        if relative in seen or len(digest) != _DIGEST_LENGTH or any(
            character not in "0123456789abcdef" for character in digest
        ):
            raise ValueError(f"invalid runtime entry: {relative}")
        seen.add(relative)
        runtime.append((relative, digest))
    build_results = _load_build_results(
        stage / "build-results.ndjson", tests
    )
    if _build_results_digest(build_results) != metadata.build_results_sha256:
        raise ValueError("build results checksum mismatch")
    provenance = {
        "build_results_sha256": metadata.build_results_sha256,
        "manifest_sha256": metadata.manifest_sha256,
        "patch_sha256": metadata.patch_sha256,
        "revision": metadata.revision,
        "smros_commit": metadata.smros_commit,
    }
    build_id = hashlib.sha256(
        json.dumps(provenance, sort_keys=True, separators=(",", ":")).encode("ascii")
    ).hexdigest()
    return _StageIdentity(
        metadata=metadata,
        tests=tests,
        runtime=tuple(runtime),
        build_results=build_results,
        manifest_data=manifest_data,
        host_data=host_data,
        build_id=build_id,
    )


def _require_executable(path: Path, label: str) -> None:
    try:
        info = path.lstat()
    except OSError as error:
        raise ValueError(f"missing {label}: {path}") from error
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise ValueError(f"{label} is not a regular file: {path}")
    if stat.S_IMODE(info.st_mode) != 0o755:
        raise ValueError(f"invalid executable mode for {label}: {path}")


def _sha256_open_regular(
    path: Path,
    label: str,
    *,
    executable: bool,
    expected_mode: int | None = None,
) -> str:
    try:
        path_info = path.lstat()
    except OSError as error:
        raise ValueError(f"missing {label}: {path}") from error
    if stat.S_ISLNK(path_info.st_mode) or not stat.S_ISREG(path_info.st_mode):
        raise ValueError(f"{label} is not a regular file: {path}")
    mode = stat.S_IMODE(path_info.st_mode)
    if expected_mode is not None and mode != expected_mode:
        raise ValueError(
            f"mode mismatch for {label}: expected {expected_mode:04o}, got {mode:04o}"
        )
    if mode & 0o444 == 0:
        raise ValueError(f"{label} is not readable: {path}")
    if executable and mode & 0o111 == 0:
        raise ValueError(f"{label} is not executable: {path}")
    flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise ValueError(f"{label} could not be opened safely: {path}") from error
    try:
        opened_info = os.fstat(descriptor)
        if (
            not stat.S_ISREG(opened_info.st_mode)
            or (opened_info.st_dev, opened_info.st_ino)
            != (path_info.st_dev, path_info.st_ino)
        ):
            raise ValueError(f"{label} changed while being opened: {path}")
        opened_mode = stat.S_IMODE(opened_info.st_mode)
        if expected_mode is not None and opened_mode != expected_mode:
            raise ValueError(
                f"mode mismatch for {label}: expected {expected_mode:04o}, "
                f"got {opened_mode:04o}"
            )
        if opened_mode & 0o444 == 0:
            raise ValueError(f"{label} is not readable: {path}")
        if executable and opened_mode & 0o111 == 0:
            raise ValueError(f"{label} is not executable: {path}")
        digest = hashlib.sha256()
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
        after_info = os.fstat(descriptor)
        fingerprint = lambda info: (
            info.st_dev,
            info.st_ino,
            info.st_mode,
            info.st_size,
            info.st_mtime_ns,
            info.st_ctime_ns,
        )
        if fingerprint(opened_info) != fingerprint(after_info):
            raise ValueError(f"{label} changed while being read: {path}")
        return digest.hexdigest()
    finally:
        os.close(descriptor)


def _resolve_qemu(qemu: str | Path) -> Path:
    value = os.fspath(qemu)
    try:
        resolved = shutil.which(value) if "/" not in value else value
    except OSError as error:
        raise BaselinePrerequisiteError(
            f"qemu-aarch64 is unavailable: {value}"
        ) from error
    if resolved is None:
        raise BaselinePrerequisiteError("qemu-aarch64 is unavailable")
    try:
        path = Path(resolved).resolve()
    except (OSError, RuntimeError) as error:
        raise BaselinePrerequisiteError(
            f"qemu-aarch64 is unavailable: {resolved}"
        ) from error
    try:
        info = path.lstat()
    except OSError as error:
        raise BaselinePrerequisiteError(
            f"qemu-aarch64 is unavailable: {path}"
        ) from error
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise BaselinePrerequisiteError(
            f"qemu-aarch64 is not a regular file: {path}"
        )
    if stat.S_IMODE(info.st_mode) & 0o111 == 0:
        raise BaselinePrerequisiteError(
            f"qemu-aarch64 is not executable: {path}"
        )
    return path


def _validate_prerequisites_impl(
    stage: Path,
    sysroot: Path,
    identity: _StageIdentity,
) -> None:
    try:
        sysroot_info = sysroot.lstat()
    except OSError as error:
        raise ValueError(f"configured AArch64 sysroot is missing: {sysroot}") from error
    if stat.S_ISLNK(sysroot_info.st_mode) or not stat.S_ISDIR(
        sysroot_info.st_mode
    ):
        raise ValueError(f"configured AArch64 sysroot is not a directory: {sysroot}")
    loaders: list[str] = []
    for relative, digest in identity.runtime:
        staged_path = stage / relative
        sysroot_path = sysroot / relative
        is_interpreter = PurePosixPath(relative).name.startswith("ld-linux-")
        _require_executable(staged_path, f"manifest runtime file {relative}")
        if _sha256_open_regular(
            staged_path,
            f"manifest runtime file {relative}",
            executable=True,
        ) != digest:
            raise ValueError(f"manifest runtime file checksum mismatch: {relative}")
        sysroot_digest = _sha256_open_regular(
            sysroot_path,
            f"configured sysroot runtime file {relative}",
            executable=is_interpreter,
        )
        if sysroot_digest != digest:
            if is_interpreter:
                raise ValueError(
                    "configured sysroot interpreter does not match "
                    "staged interpreter"
                )
            raise ValueError(
                f"configured sysroot runtime does not match staged runtime: {relative}"
            )
        if is_interpreter:
            loaders.append(relative)
    if len(loaders) != 1:
        raise ValueError("manifest must contain exactly one staged interpreter")


def _validate_prerequisites(
    stage: Path,
    sysroot: Path,
    identity: _StageIdentity,
) -> None:
    try:
        _validate_prerequisites_impl(stage, sysroot, identity)
    except BaselinePrerequisiteError:
        raise
    except (OSError, ValueError) as error:
        raise BaselinePrerequisiteError(str(error)) from error


def _copy_runtime_file(
    source: Path,
    destination: Path,
    *,
    label: str,
    expected_sha256: str,
    executable: bool,
) -> tuple[str, int]:
    try:
        path_info = source.lstat()
    except OSError as error:
        raise BaselinePrerequisiteError(f"missing {label}: {source}") from error
    if stat.S_ISLNK(path_info.st_mode) or not stat.S_ISREG(path_info.st_mode):
        raise BaselinePrerequisiteError(
            f"{label} is not a regular file: {source}"
        )
    mode = stat.S_IMODE(path_info.st_mode)
    if mode & 0o444 == 0 or (executable and mode & 0o111 == 0):
        raise BaselinePrerequisiteError(f"invalid mode for {label}: {source}")
    read_flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    try:
        source_descriptor = os.open(source, read_flags)
    except OSError as error:
        raise BaselinePrerequisiteError(
            f"{label} could not be opened safely: {source}"
        ) from error
    destination_descriptor: int | None = None
    try:
        try:
            opened_info = os.fstat(source_descriptor)
        except OSError as error:
            raise BaselinePrerequisiteError(
                f"{label} changed while being opened: {source}"
            ) from error
        if (
            not stat.S_ISREG(opened_info.st_mode)
            or (opened_info.st_dev, opened_info.st_ino)
            != (path_info.st_dev, path_info.st_ino)
        ):
            raise BaselinePrerequisiteError(
                f"{label} changed while being opened: {source}"
            )
        opened_mode = stat.S_IMODE(opened_info.st_mode)
        if opened_mode & 0o444 == 0 or (
            executable and opened_mode & 0o111 == 0
        ):
            raise BaselinePrerequisiteError(
                f"invalid mode for {label}: {source}"
            )
        destination.parent.mkdir(parents=True, exist_ok=True)
        output_mode = 0o755 if executable else 0o644
        write_flags = (
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        destination_descriptor = os.open(
            destination, write_flags, output_mode
        )
        digest = hashlib.sha256()
        while True:
            try:
                chunk = os.read(source_descriptor, 1024 * 1024)
            except OSError as error:
                raise BaselinePrerequisiteError(
                    f"{label} changed while being copied: {source}"
                ) from error
            if not chunk:
                break
            digest.update(chunk)
            view = memoryview(chunk)
            while view:
                written = os.write(destination_descriptor, view)
                if written <= 0:
                    raise OSError(f"short write while copying {label}")
                view = view[written:]
        try:
            after_info = os.fstat(source_descriptor)
        except OSError as error:
            raise BaselinePrerequisiteError(
                f"{label} changed while being copied: {source}"
            ) from error
        fingerprint = lambda info: (
            info.st_dev,
            info.st_ino,
            info.st_mode,
            info.st_size,
            info.st_mtime_ns,
            info.st_ctime_ns,
        )
        if fingerprint(opened_info) != fingerprint(after_info):
            raise BaselinePrerequisiteError(
                f"{label} changed while being copied: {source}"
            )
        actual_sha256 = digest.hexdigest()
        if actual_sha256 != expected_sha256:
            raise BaselinePrerequisiteError(
                f"checksum mismatch while copying {label}"
            )
        os.fchmod(destination_descriptor, output_mode)
        os.fsync(destination_descriptor)
        os.close(destination_descriptor)
        destination_descriptor = None
        return actual_sha256, output_mode
    except BaseException:
        if destination_descriptor is not None:
            try:
                os.close(destination_descriptor)
            except BaseException:
                pass
        try:
            destination.unlink()
        except FileNotFoundError:
            pass
        raise
    finally:
        os.close(source_descriptor)


def _snapshot_sysroot(
    configured_sysroot: Path,
    destination: Path,
    identity: _StageIdentity,
) -> str:
    canonical_files: list[dict[str, object]] = []
    directory_modes = {".": 0o700}
    for relative, expected_sha256 in identity.runtime:
        path = _safe_relative(relative, "runtime path")
        for length in range(1, len(path.parts)):
            directory_modes[PurePosixPath(*path.parts[:length]).as_posix()] = 0o755
        executable = path.name.startswith("ld-linux-")
        actual_sha256, mode = _copy_runtime_file(
            configured_sysroot.joinpath(*path.parts),
            destination.joinpath(*path.parts),
            label=f"configured sysroot runtime file {relative}",
            expected_sha256=expected_sha256,
            executable=executable,
        )
        canonical_files.append(
            {
                "kind": "file",
                "mode": mode,
                "path": relative,
                "sha256": actual_sha256,
            }
        )
    for relative, mode in sorted(directory_modes.items()):
        path = destination if relative == "." else destination / relative
        os.chmod(path, mode)
    canonical = [
        {"kind": "directory", "mode": mode, "path": relative}
        for relative, mode in sorted(directory_modes.items())
    ]
    canonical.extend(sorted(canonical_files, key=lambda item: str(item["path"])))
    encoded = (
        json.dumps(
            canonical, sort_keys=True, separators=(",", ":"), ensure_ascii=True
        )
        + "\n"
    ).encode("ascii")
    return hashlib.sha256(encoded).hexdigest()


def _verify_sysroot_snapshot(
    snapshot: Path,
    identity: _StageIdentity,
) -> None:
    expected_files = {
        relative: (
            digest,
            0o755 if PurePosixPath(relative).name.startswith("ld-linux-") else 0o644,
        )
        for relative, digest in identity.runtime
    }
    expected_directories = {".": 0o700}
    for relative in expected_files:
        path = PurePosixPath(relative)
        for length in range(1, len(path.parts)):
            expected_directories[
                PurePosixPath(*path.parts[:length]).as_posix()
            ] = 0o755
    actual_files: set[str] = set()
    actual_directories: set[str] = set()
    for directory, directory_names, file_names in os.walk(
        snapshot, followlinks=False
    ):
        directory_names.sort()
        file_names.sort()
        directory_path = Path(directory)
        relative_directory = directory_path.relative_to(snapshot).as_posix()
        if relative_directory == ".":
            relative_directory = "."
        actual_directories.add(relative_directory)
        info = directory_path.lstat()
        expected_mode = expected_directories.get(relative_directory)
        mode = stat.S_IMODE(info.st_mode)
        if expected_mode is not None and mode != expected_mode:
            raise ValueError(
                "runtime snapshot directory mode mismatch: "
                f"{relative_directory}: expected {expected_mode:04o}, got {mode:04o}"
            )
        for name in directory_names:
            path = Path(directory) / name
            if path.is_symlink():
                raise ValueError(f"runtime snapshot directory is a symlink: {path}")
        for name in file_names:
            path = Path(directory) / name
            actual_files.add(path.relative_to(snapshot).as_posix())
    if (
        actual_files != set(expected_files)
        or actual_directories != set(expected_directories)
    ):
        raise ValueError("runtime snapshot inventory mismatch")
    for relative, (expected_sha256, expected_mode) in expected_files.items():
        path = _safe_relative(relative, "runtime path")
        actual_sha256 = _sha256_open_regular(
            snapshot.joinpath(*path.parts),
            f"runtime snapshot file {relative}",
            executable=path.name.startswith("ld-linux-"),
            expected_mode=expected_mode,
        )
        if actual_sha256 != expected_sha256:
            raise ValueError(
                f"runtime snapshot checksum mismatch: {relative}"
            )


def _snapshot_prerequisites(
    configured_sysroot: Path,
    destination: Path,
    identity: _StageIdentity,
) -> str:
    return _snapshot_sysroot(configured_sysroot, destination, identity)


def _bind_runtime_snapshot(
    identity: _StageIdentity,
    runtime_snapshot_sha256: str,
) -> _StageIdentity:
    build_id = hashlib.sha256(
        json.dumps(
            {
                "base_build_id": identity.build_id,
                "runtime_snapshot_sha256": runtime_snapshot_sha256,
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode("ascii")
    ).hexdigest()
    return replace(
        identity,
        build_id=build_id,
        runtime_snapshot_sha256=runtime_snapshot_sha256,
    )


def _validate_selected(stage: Path, selected: Sequence[SuiteTest]) -> None:
    for test in selected:
        assert test.binary is not None and test.sha256 is not None
        relative = _safe_relative(test.binary, "staged binary")
        path = stage.joinpath(*relative.parts)
        _require_executable(path, f"binary for {test.test_id}")
        if sha256_file(path) != test.sha256:
            raise ValueError(f"binary checksum mismatch for {test.test_id}")


def _attempt_record(attempt: RuntimeAttempt) -> dict[str, object]:
    return {"record_type": "attempt", **attempt.to_dict()}


def _open_parent(path: Path) -> int:
    absolute = Path(os.path.abspath(path))
    flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_DIRECTORY", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    descriptor = os.open(absolute.anchor, flags)
    try:
        for part in absolute.parts[1:]:
            try:
                child = os.open(part, flags, dir_fd=descriptor)
            except FileNotFoundError:
                os.mkdir(part, 0o755, dir_fd=descriptor)
                os.fsync(descriptor)
                child = os.open(part, flags, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _atomic_write(path: Path, data: bytes) -> None:
    parent = _open_parent(path.parent)
    temporary_name = f".{path.name}.{secrets.token_hex(8)}.tmp"
    descriptor: int | None = None
    try:
        try:
            destination = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            destination = None
        if destination is not None and (
            stat.S_ISLNK(destination.st_mode) or not stat.S_ISREG(destination.st_mode)
        ):
            raise ValueError(f"results destination is not a regular file: {path}")
        flags = (
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        descriptor = os.open(temporary_name, flags, 0o644, dir_fd=parent)
        view = memoryview(data)
        while view:
            written = os.write(descriptor, view)
            view = view[written:]
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = None
        os.replace(
            temporary_name,
            path.name,
            src_dir_fd=parent,
            dst_dir_fd=parent,
        )
        os.fsync(parent)
    finally:
        if descriptor is not None:
            os.close(descriptor)
        try:
            os.unlink(temporary_name, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)


def _canonical_report(rows: Sequence[dict[str, object]]) -> bytes:
    text = "".join(
        json.dumps(
            row, sort_keys=True, separators=(",", ":"), ensure_ascii=True
        )
        + "\n"
        for row in rows
    )
    for line in text.splitlines():
        parsed = json.loads(line, object_pairs_hook=_reject_duplicate_keys)
        canonical = json.dumps(
            parsed, sort_keys=True, separators=(",", ":"), ensure_ascii=True
        )
        if canonical != line:
            raise ValueError("runtime result is not canonical JSON")
    return text.encode("utf-8")


def _terminal_record(
    identity: _StageIdentity,
    selected: Sequence[SuiteTest],
    attempts: Sequence[RuntimeAttempt],
    qemu: Path,
    sysroot: Path,
    run_id: str,
    *,
    complete: bool,
) -> dict[str, object]:
    status_counts = dict(
        sorted(Counter(attempt.status for attempt in attempts).items())
    )
    return {
        "build_id": identity.build_id,
        "build_results_sha256": identity.metadata.build_results_sha256,
        "complete": complete,
        "completed_count": len(attempts),
        "manifest_sha256": identity.metadata.manifest_sha256,
        "patch_sha256": identity.metadata.patch_sha256,
        "platform": PLATFORM,
        "qemu": str(qemu),
        "record_type": "run",
        "revision": identity.metadata.revision,
        "runtime_snapshot_sha256": identity.runtime_snapshot_sha256,
        "run_id": run_id,
        "selected_count": len(selected),
        "smros_commit": identity.metadata.smros_commit,
        "source": SOURCE,
        "status_counts": status_counts,
        "sysroot": str(sysroot),
    }


def _publish_report(
    result_path: Path,
    identity: _StageIdentity,
    selected: Sequence[SuiteTest],
    attempts: Sequence[RuntimeAttempt],
    qemu: Path,
    sysroot: Path,
    run_id: str,
    *,
    complete: bool,
) -> None:
    rows = [_attempt_record(attempt) for attempt in attempts]
    rows.append(
        _terminal_record(
            identity,
            selected,
            attempts,
            qemu,
            sysroot,
            run_id,
            complete=complete,
        )
    )
    _atomic_write(result_path, _canonical_report(rows))


def _note_incomplete_publication_failure(
    primary: BaseException, publication_error: BaseException
) -> None:
    primary.add_note(
        _bounded_detail(
            "incomplete report publication failed: "
            f"{type(publication_error).__name__}: {publication_error}"
        )
    )


def run_baseline(
    stage: Path,
    sysroot: Path,
    result_path: Path,
    *,
    api: str | None = None,
    group: str | None = None,
    test_id: str | None = None,
    qemu: str | Path = "qemu-aarch64",
    verifier: Callable[[Path], object] = verify_stage,
) -> BaselineResult:
    stage = Path(os.path.abspath(stage))
    sysroot = Path(os.path.abspath(sysroot))
    result_path = Path(os.path.abspath(result_path))
    qemu_path = _resolve_qemu(qemu)
    identity = _load_stage_identity(stage)
    _validate_prerequisites(stage, sysroot, identity)
    verifier(stage)
    verified_identity = _load_stage_identity(stage)
    if (
        verified_identity.manifest_data != identity.manifest_data
        or verified_identity.host_data != identity.host_data
        or verified_identity.build_results != identity.build_results
    ):
        raise ValueError("stage provenance changed during verification")
    _validate_prerequisites(stage, sysroot, verified_identity)
    selected = filter_runnable_tests(
        verified_identity.tests, api=api, group=group, test_id=test_id
    )
    _validate_selected(stage, selected)
    attempts: list[RuntimeAttempt] = []
    run_id = ""
    runtime_snapshot_sha256 = ""
    campaign_succeeded = False
    pending_error: BaseException | None = None
    try:
        with tempfile.TemporaryDirectory(
            prefix="smros-posix-sysroot-"
        ) as sysroot_temporary:
            execution_sysroot = Path(sysroot_temporary)
            runtime_snapshot_sha256 = _snapshot_prerequisites(
                sysroot, execution_sysroot, verified_identity
            )
            _verify_sysroot_snapshot(execution_sysroot, verified_identity)
            verified_identity = _bind_runtime_snapshot(
                verified_identity, runtime_snapshot_sha256
            )
            run_id = secrets.token_hex(16)
            build_statuses = {
                (result.test_id, result.stage): result.status
                for result in verified_identity.build_results
            }
            try:
                for test in selected:
                    started = time.monotonic()
                    build_status = build_statuses[(test.test_id, "compile")]
                    link_status = build_statuses[(test.test_id, "link")]
                    active_index: int | None = None
                    try:
                        _validate_selected(stage, (test,))
                        _verify_sysroot_snapshot(
                            execution_sysroot, verified_identity
                        )
                        attempt = run_runtime_attempt(
                            test,
                            stage=stage,
                            sysroot=execution_sysroot,
                            qemu=qemu_path,
                            metadata=verified_identity.metadata,
                            build_id=verified_identity.build_id,
                            build_status=build_status,
                            link_status=link_status,
                            run_id=run_id,
                            runtime_snapshot_sha256=(
                                runtime_snapshot_sha256
                            ),
                        )
                        attempts.append(attempt)
                        active_index = len(attempts) - 1
                        if attempt.launch_error is not None:
                            raise BaselinePrerequisiteError(
                                attempt.launch_error
                            )
                        _verify_sysroot_snapshot(
                            execution_sysroot, verified_identity
                        )
                    except BaseException as error:
                        if active_index is None:
                            attempts.append(
                                _interrupted_attempt(
                                    test,
                                    metadata=verified_identity.metadata,
                                    build_id=verified_identity.build_id,
                                    build_status=build_status,
                                    link_status=link_status,
                                    run_id=run_id,
                                    runtime_snapshot_sha256=(
                                        runtime_snapshot_sha256
                                    ),
                                    started=started,
                                    error=error,
                                )
                            )
                        elif attempts[active_index].launch_error is None:
                            attempts[active_index] = replace(
                                attempts[active_index],
                                status="interrupted",
                                infrastructure_error=(
                                    _bounded_infrastructure_error(error)
                                ),
                            )
                        raise
            except BaseException as error:
                pending_error = error
                try:
                    _publish_report(
                        result_path,
                        verified_identity,
                        selected,
                        attempts,
                        qemu_path,
                        sysroot,
                        run_id,
                        complete=False,
                    )
                except BaseException as publication_error:
                    _note_incomplete_publication_failure(
                        error, publication_error
                    )
                raise
            campaign_succeeded = True
    except BaseException as error:
        if campaign_succeeded:
            attempts[-1] = replace(
                attempts[-1],
                status="interrupted",
                infrastructure_error=_combined_infrastructure_error(
                    attempts[-1].infrastructure_error, error
                ),
            )
            try:
                _publish_report(
                    result_path,
                    verified_identity,
                    selected,
                    attempts,
                    qemu_path,
                    sysroot,
                    run_id,
                    complete=False,
                )
            except BaseException as publication_error:
                _note_incomplete_publication_failure(
                    error, publication_error
                )
        elif pending_error is not None and error is not pending_error:
            attempts[-1] = replace(
                attempts[-1],
                infrastructure_error=_combined_infrastructure_error(
                    attempts[-1].infrastructure_error, error
                ),
            )
            try:
                _publish_report(
                    result_path,
                    verified_identity,
                    selected,
                    attempts,
                    qemu_path,
                    sysroot,
                    run_id,
                    complete=False,
                )
            except BaseException as publication_error:
                _note_incomplete_publication_failure(
                    pending_error, publication_error
                )
            raise pending_error from error
        raise
    _publish_report(
        result_path,
        verified_identity,
        selected,
        attempts,
        qemu_path,
        sysroot,
        run_id,
        complete=True,
    )
    frozen_attempts = tuple(attempts)
    return BaselineResult(
        attempts=frozen_attempts,
        all_passed=all(
            attempt.status == "pass" for attempt in frozen_attempts
        ),
        result_path=result_path,
    )
