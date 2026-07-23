"""Run staged AArch64 POSIX tests under the Linux qemu-user reference."""

from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Mapping, Sequence
from dataclasses import asdict, dataclass
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
import tempfile
import time

from .build import (
    CHECKSUM_DEFINITION,
    MAX_HOST_MANIFEST_BYTES,
    MAX_MANIFEST_BYTES,
    ManifestMetadata,
    parse_manifest,
    sha256_file,
    verify_stage,
)
from .model import (
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
_DIGEST_LENGTH = 64


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
class _StageIdentity:
    metadata: ManifestMetadata
    tests: tuple[SuiteTest, ...]
    runtime: tuple[tuple[str, str], ...]
    manifest_data: bytes
    host_data: bytes
    build_id: str


class _ProcessLaunchError(Exception):
    def __init__(self, error: OSError) -> None:
        super().__init__(str(error))
        self.error = error


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


def _run_captured(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: float,
) -> tuple[int, bool, _Capture, _Capture]:
    try:
        process = subprocess.Popen(
            list(argv),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=cwd,
            env=dict(env),
            start_new_session=True,
        )
    except OSError as error:
        raise _ProcessLaunchError(error) from error
    selector: selectors.BaseSelector | None = None
    try:
        assert process.stdout is not None and process.stderr is not None
        selector = selectors.DefaultSelector()
        streams: dict[int, object] = {}
        buffers = {"stdout": bytearray(), "stderr": bytearray()}
        counts = {"stdout": 0, "stderr": 0}
        for name, stream in (("stdout", process.stdout), ("stderr", process.stderr)):
            descriptor = stream.fileno()
            os.set_blocking(descriptor, False)
            selector.register(descriptor, selectors.EVENT_READ, name)
            streams[descriptor] = stream
        deadline = time.monotonic() + timeout_seconds
        terminate_deadline: float | None = None
        kill_deadline: float | None = None
        drain_deadline: float | None = None
        timed_out = False

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
                _kill_group(process, signal.SIGTERM)
                terminate_deadline = now + _TERMINATE_GRACE_SECONDS
            if (
                timed_out
                and terminate_deadline is not None
                and now >= terminate_deadline
            ):
                _kill_group(process, signal.SIGKILL)
                terminate_deadline = None
                kill_deadline = now + _KILL_REAP_SECONDS
            if (
                returncode is None
                and kill_deadline is not None
                and now >= kill_deadline
            ):
                _reap(process)
                returncode = process.poll()
            if returncode is not None and drain_deadline is None:
                drain_deadline = now + _DRAIN_GRACE_SECONDS
                if timed_out:
                    _kill_group(process, signal.SIGKILL)
            if returncode is not None and not streams:
                break
            if drain_deadline is not None and now >= drain_deadline:
                _kill_group(process, signal.SIGKILL)
                for descriptor in tuple(streams):
                    close_stream(descriptor)
                break
            active_deadlines = [] if timed_out else [deadline]
            if terminate_deadline is not None:
                active_deadlines.append(terminate_deadline)
            if drain_deadline is not None:
                active_deadlines.append(drain_deadline)
            if kill_deadline is not None:
                active_deadlines.append(kill_deadline)
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
                counts[name] += len(chunk)
                remaining = MAX_CAPTURE_BYTES - len(buffers[name])
                if remaining > 0:
                    buffers[name].extend(chunk[:remaining])

        _reap(process)
        return (
            int(process.returncode),
            timed_out,
            _captured(buffers["stdout"], counts["stdout"]),
            _captured(buffers["stderr"], counts["stderr"]),
        )
    except BaseException:
        try:
            _kill_group(process, signal.SIGKILL)
        except BaseException:
            pass
        try:
            _reap(process)
        except BaseException:
            pass
        raise
    finally:
        if selector is not None:
            try:
                selector.close()
            except BaseException:
                pass
        for stream in (process.stdout, process.stderr):
            if stream is not None:
                try:
                    stream.close()
                except BaseException:
                    pass


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
    run_id: str = "",
) -> RuntimeAttempt:
    started = time.monotonic()
    launch_error: str | None = None
    returncode: int | None = None
    timed_out = False
    stdout = _Capture("", 0, False)
    stderr = _Capture("", 0, False)
    with tempfile.TemporaryDirectory(prefix="smros-posix-baseline-") as temporary:
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
            launch_error = f"{error.strerror or str(error)}: {qemu}"
    duration_ms = max(0, int((time.monotonic() - started) * 1000))
    signum = -returncode if returncode is not None and returncode < 0 else None
    exit_code = returncode if returncode is not None and returncode >= 0 else None
    return RuntimeAttempt(
        test_id=test.test_id,
        platform=PLATFORM,
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
) -> str:
    try:
        path_info = path.lstat()
    except OSError as error:
        raise ValueError(f"missing {label}: {path}") from error
    if stat.S_ISLNK(path_info.st_mode) or not stat.S_ISREG(path_info.st_mode):
        raise ValueError(f"{label} is not a regular file: {path}")
    mode = stat.S_IMODE(path_info.st_mode)
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
    resolved = shutil.which(value) if "/" not in value else value
    if resolved is None:
        raise ValueError("qemu-aarch64 is unavailable")
    path = Path(resolved).resolve()
    _require_executable(path, "qemu-aarch64")
    return path


def _validate_prerequisites(
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
    ):
        raise ValueError("stage provenance changed during verification")
    _validate_prerequisites(stage, sysroot, verified_identity)
    selected = filter_runnable_tests(
        verified_identity.tests, api=api, group=group, test_id=test_id
    )
    _validate_selected(stage, selected)
    run_id = secrets.token_hex(16)
    attempts: list[RuntimeAttempt] = []
    try:
        for test in selected:
            _validate_selected(stage, (test,))
            attempts.append(
                run_runtime_attempt(
                    test,
                    stage=stage,
                    sysroot=sysroot,
                    qemu=qemu_path,
                    metadata=verified_identity.metadata,
                    build_id=verified_identity.build_id,
                    run_id=run_id,
                )
            )
    except BaseException:
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
        except BaseException:
            pass
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
        all_passed=all(attempt.status == "pass" for attempt in frozen_attempts),
        result_path=result_path,
    )
