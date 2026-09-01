"""Cross-build and deterministic staging for the Open POSIX Test Suite.

The guest manifest checksum is SHA-256 over its complete UTF-8 TSV bytes after
replacing the ``manifest_sha256`` metadata value with 64 ASCII zeroes.  This is
a canonical, non-self-referential definition and covers every other byte.

The ``build_results_sha256`` value is SHA-256 over canonical UTF-8 NDJSON in
file order, using sorted JSON keys, compact separators, ASCII escaping, and LF
terminators, after replacing every ``duration_ms`` value with integer zero.
Every other field is preserved. The raw NDJSON retains the measured durations.
"""

from __future__ import annotations

import ctypes
import errno
import fcntl
import hashlib
import json
import os
import re
import selectors
import signal
import shutil
import stat
import subprocess
import tempfile
import time
from dataclasses import asdict, dataclass, replace
from pathlib import Path, PurePosixPath
from typing import BinaryIO, Callable, Iterable, Mapping, Sequence

from .model import BuildResult, BuildSummary, SuiteTest


MAX_MANIFEST_BYTES = 2 * 1024 * 1024
MAX_MANIFEST_METADATA_VALUE_BYTES = 1024
MAX_MANIFEST_TEST_ID_BYTES = 256
MAX_MANIFEST_GROUP_BYTES = 96
MAX_MANIFEST_API_BYTES = 96
MAX_MANIFEST_STAGED_PATH_BYTES = 512
MAX_HOST_MANIFEST_BYTES = 8 * 1024 * 1024
MAX_BUILD_RESULTS_BYTES = 64 * 1024 * 1024
MAX_BUILD_RESULT_LINE_BYTES = 256 * 1024
MAX_TESTS = 4_096
MAX_BUILD_RESULTS_ROWS = MAX_TESTS * 3
MAX_STAGE_BYTES = 256 * 1024 * 1024
MAX_DIAGNOSTIC_BYTES = 16_384
MAX_TIMEOUT_MS = 2**31 - 1
COMMAND_TIMEOUT_SECONDS = 120.0
EMPTY_SHA256 = "0" * 64
CHECKSUM_DEFINITION = (
    "sha256(manifest.tsv with meta manifest_sha256 value replaced by "
    "64 ASCII zeroes)"
)
_STAGE_QUARANTINE_NAME = ".smros-posix-stage-quarantine"
_STAGE_WORK_ROOT_NAME = "stage"
_STAGE_JOURNAL_NAME = "journal.bin"
_STAGE_JOURNAL_RECORD_BYTES = 512
_STAGE_JOURNAL_RECORD_COUNT = 2
_STAGE_JOURNAL_BYTES = (
    _STAGE_JOURNAL_RECORD_BYTES * _STAGE_JOURNAL_RECORD_COUNT
)
_STAGE_JOURNAL_MAGIC = "SMROSJ1"
_STAGE_JOURNAL_HEADER_BYTES = 95
_DIGEST_RE = re.compile(r"[0-9a-f]{64}\Z")
_REVISION_RE = re.compile(r"[0-9a-f]{40}\Z")
_NEEDED_RE = re.compile(r"\(NEEDED\).*Shared library: \[([^\]]+)\]")
_INTERPRETER_RE = re.compile(r"Requesting program interpreter: ([^\]]+)\]")
_IMPLICIT_RUNTIME_NAMES = ("libgcc_s.so.1",)
_FORK_MESSAGE_CATALOG_TEST_ID = "conformance/interfaces/fork/7-1.c"
_FORK_MESSAGE_CATALOG_SOURCE = "conformance/interfaces/fork/messcat_src.txt"
_FORK_MESSAGE_CATALOG_SUPPORT = "conformance/interfaces/fork/mess.cat"
_ALLOWED_KINDS = frozenset({"runnable", "definition", "shell"})
_ALLOWED_DISPOSITIONS = frozenset(
    {
        "complete",
        "definition-only",
        "excluded-upstream-stub",
        "compile-failed",
        "link-failed",
        "not-built-shell-test",
    }
)
_ALLOWED_KIND_DISPOSITIONS = frozenset(
    {
        ("runnable", "complete"),
        ("runnable", "excluded-upstream-stub"),
        ("runnable", "compile-failed"),
        ("runnable", "link-failed"),
        ("definition", "definition-only"),
        ("definition", "excluded-upstream-stub"),
        ("definition", "compile-failed"),
        ("shell", "not-built-shell-test"),
    }
)
_REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
POSIX_COMPAT_PRELOAD_NAME = "libsmros-posix-compat.so"
POSIX_COMPAT_PRELOAD_SOURCE = (
    _REPOSITORY_ROOT / "scripts" / "posix" / "runtime" / "smros_posix_compat.c"
)
POSIX_COMPAT_INCLUDE_DIRECTORY = (
    _REPOSITORY_ROOT / "scripts" / "posix" / "runtime" / "include"
)
POSIX_COMPAT_PRELOAD_VERSION_SCRIPT = (
    _REPOSITORY_ROOT / "scripts" / "posix" / "runtime" / "smros_posix_compat.map"
)
_METADATA_KEYS = (
    "source",
    "revision",
    "architecture",
    "compiler",
    "libc",
    "patch_sha256",
    "build_results_sha256",
    "manifest_sha256",
    "smros_commit",
)


@dataclass(frozen=True)
class ManifestMetadata:
    source: str
    revision: str
    architecture: str
    compiler: str
    libc: str
    patch_sha256: str
    smros_commit: str
    build_results_sha256: str = EMPTY_SHA256
    manifest_sha256: str = EMPTY_SHA256


CommandRunner = Callable[..., object]
DependencyStager = Callable[[Sequence[Path], Path], Sequence[Path]]
_SUBPROCESS_RUN = subprocess.run


def nm_command(tool: str, object_path: Path) -> list[str]:
    return [tool, "-g", "--defined-only", str(object_path)]


def compile_command(
    compiler: str,
    source: Path,
    object_path: Path,
    include_directory: Path,
) -> list[str]:
    return [
        compiler,
        "-std=gnu99",
        "-D_POSIX_C_SOURCE=200112L",
        "-D_XOPEN_SOURCE=600",
        "-pthread",
        "-I",
        str(POSIX_COMPAT_INCLUDE_DIRECTORY),
        "-I",
        str(include_directory),
        "-c",
        str(source),
        "-o",
        str(object_path),
    ]


def link_command(compiler: str, object_path: Path, executable: Path) -> list[str]:
    return [
        compiler,
        "-pthread",
        str(object_path),
        "-o",
        str(executable),
        "-lrt",
        "-lm",
    ]


def posix_compat_preload_command(
    compiler: str,
    source: Path,
    output: Path,
) -> list[str]:
    return [
        compiler,
        "-std=gnu99",
        "-fPIC",
        "-shared",
        "-Wall",
        "-Wextra",
        "-Werror",
        str(source),
        "-o",
        str(output),
        f"-Wl,-soname,{POSIX_COMPAT_PRELOAD_NAME}",
        f"-Wl,--version-script,{POSIX_COMPAT_PRELOAD_VERSION_SCRIPT}",
        "-ldl",
    ]


def readelf_command(tool: str, executable: Path) -> list[str]:
    return [tool, "-l", "-d", str(executable)]


def _has_forbidden_character(value: str) -> bool:
    return any(not 0x20 <= ord(character) <= 0x7e for character in value)


def _validate_atom(value: str, label: str) -> None:
    if not value or _has_forbidden_character(value):
        raise ValueError(f"invalid {label}: {value!r}")


def _validate_manifest_atom(value: str, label: str) -> None:
    _validate_atom(value, label)
    if (
        "\\" in value
        or "//" in value
        or any(segment in {"", ".", ".."} for segment in value.split("/"))
    ):
        raise ValueError(f"unsafe {label}: {value!r}")


def _validate_field_limit(value: str, label: str, maximum: int) -> None:
    if len(value.encode("utf-8")) > maximum:
        raise ValueError(f"{label} exceeds the {maximum}-byte manifest field limit")


def _validate_relative_path(value: str, label: str) -> PurePosixPath:
    _validate_atom(value, label)
    path = PurePosixPath(value)
    raw_parts = value.split("/")
    if (
        "\\" in value
        or path.is_absolute()
        or path.as_posix() != value
        or any(part in {"", ".", ".."} for part in raw_parts)
    ):
        raise ValueError(f"unsafe {label}: {value!r}")
    return path


def safe_stage_path(root: Path, relative: str) -> Path:
    path = _validate_relative_path(relative, "staged path")
    return root.joinpath(*path.parts)


def _validate_metadata(metadata: ManifestMetadata) -> None:
    for key in _METADATA_KEYS:
        value = getattr(metadata, key)
        _validate_atom(value, f"metadata {key}")
        _validate_field_limit(
            value, f"metadata {key}", MAX_MANIFEST_METADATA_VALUE_BYTES
        )
    if metadata.architecture != "aarch64":
        raise ValueError(f"unsupported architecture: {metadata.architecture}")
    if _REVISION_RE.fullmatch(metadata.revision) is None:
        raise ValueError("manifest revision is not a lowercase 40-hex commit")
    if _REVISION_RE.fullmatch(metadata.smros_commit) is None:
        raise ValueError("SMROS commit is not a lowercase 40-hex commit")
    if _DIGEST_RE.fullmatch(metadata.patch_sha256) is None:
        raise ValueError("patch checksum is invalid")
    if _DIGEST_RE.fullmatch(metadata.build_results_sha256) is None:
        raise ValueError("build results checksum is invalid")
    if _DIGEST_RE.fullmatch(metadata.manifest_sha256) is None:
        raise ValueError("manifest checksum is invalid")


def _reject_duplicate_json_keys(
    pairs: list[tuple[str, object]],
) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def _validate_test(test: SuiteTest) -> None:
    for label, value in (
        ("test ID", test.test_id),
        ("group", test.group),
        ("API", test.api),
        ("kind", test.kind),
        ("disposition", test.disposition),
    ):
        _validate_atom(value, label)
    _validate_relative_path(test.test_id, "test ID")
    _validate_manifest_atom(test.group, "group")
    _validate_manifest_atom(test.api, "api")
    _validate_field_limit(test.test_id, "test ID", MAX_MANIFEST_TEST_ID_BYTES)
    _validate_field_limit(test.group, "group", MAX_MANIFEST_GROUP_BYTES)
    _validate_field_limit(test.api, "API", MAX_MANIFEST_API_BYTES)
    if test.kind not in _ALLOWED_KINDS:
        raise ValueError(f"unknown kind: {test.kind}")
    if test.disposition not in _ALLOWED_DISPOSITIONS:
        raise ValueError(f"unknown disposition: {test.disposition}")
    if (test.kind, test.disposition) not in _ALLOWED_KIND_DISPOSITIONS:
        raise ValueError(
            f"invalid kind/disposition: {test.kind}/{test.disposition}"
        )
    if type(test.timeout_ms) is not int or not 0 < test.timeout_ms <= MAX_TIMEOUT_MS:
        raise ValueError(f"invalid timeout for {test.test_id}: {test.timeout_ms!r}")
    if test.binary is None or test.sha256 is None:
        raise ValueError(f"missing staged path or checksum for {test.test_id}")
    _validate_relative_path(test.binary, "staged path")
    _validate_field_limit(
        test.binary, "staged path", MAX_MANIFEST_STAGED_PATH_BYTES
    )
    if _DIGEST_RE.fullmatch(test.sha256) is None:
        raise ValueError(f"invalid checksum for {test.test_id}")
    runnable = test.disposition == "complete"
    if runnable and not test.binary.startswith("bin/"):
        raise ValueError(
            f"runnable staged path is outside the bin/ subtree: {test.binary!r}"
        )
    if runnable and (test.binary == "-" or test.sha256 == EMPTY_SHA256):
        raise ValueError(f"runnable test has no artifact: {test.test_id}")
    if not runnable and (test.binary != "-" or test.sha256 != EMPTY_SHA256):
        raise ValueError(f"non-runnable test has an artifact: {test.test_id}")


def _manifest_text(metadata: ManifestMetadata, tests: Sequence[SuiteTest]) -> str:
    lines = ["SMROS_POSIX_MANIFEST\t1"]
    for key in _METADATA_KEYS:
        lines.append(f"meta\t{key}\t{getattr(metadata, key)}")
    for test in tests:
        lines.append(
            "\t".join(
                (
                    "test",
                    test.test_id,
                    test.group,
                    test.api,
                    test.kind,
                    test.disposition,
                    test.binary or "",
                    str(test.timeout_ms),
                    test.sha256 or "",
                )
            )
        )
    return "\n".join(lines) + "\n"


def render_manifest(
    metadata: ManifestMetadata, tests: Iterable[SuiteTest]
) -> tuple[str, str]:
    canonical_metadata = replace(metadata, manifest_sha256=EMPTY_SHA256)
    _validate_metadata(canonical_metadata)
    ordered = tuple(sorted(tests, key=lambda test: test.test_id))
    if len(ordered) > MAX_TESTS:
        raise ValueError("manifest exceeds the 4,096 test limit")
    ids: set[str] = set()
    paths: set[str] = set()
    for test in ordered:
        _validate_test(test)
        if test.test_id in ids:
            raise ValueError(f"duplicate test ID: {test.test_id}")
        ids.add(test.test_id)
        if test.binary != "-":
            if test.binary in paths:
                raise ValueError(f"duplicate staged path: {test.binary}")
            paths.add(test.binary)
    canonical = _manifest_text(canonical_metadata, ordered)
    digest = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    text = _manifest_text(replace(canonical_metadata, manifest_sha256=digest), ordered)
    if len(text.encode("utf-8")) > MAX_MANIFEST_BYTES:
        raise ValueError("manifest exceeds the 2 MiB limit")
    return text, digest


def _parse_timeout(value: str, test_id: str) -> int:
    if not value.isascii() or not value.isdecimal() or str(int(value)) != value:
        raise ValueError(f"invalid nondecimal timeout for {test_id}: {value!r}")
    timeout = int(value)
    if not 0 < timeout <= MAX_TIMEOUT_MS:
        raise ValueError(f"invalid timeout for {test_id}: {value!r}")
    return timeout


def parse_manifest(data: bytes) -> tuple[ManifestMetadata, tuple[SuiteTest, ...]]:
    if len(data) > MAX_MANIFEST_BYTES:
        raise ValueError("manifest exceeds the 2 MiB limit")
    try:
        text = data.decode("utf-8")
    except UnicodeError as error:
        raise ValueError("manifest is not UTF-8") from error
    if "\r" in text or not text.endswith("\n"):
        raise ValueError("manifest must use LF line endings")
    lines = text[:-1].split("\n")
    if not lines or lines[0] != "SMROS_POSIX_MANIFEST\t1":
        raise ValueError("invalid manifest header")
    values: dict[str, str] = {}
    tests: list[SuiteTest] = []
    saw_test = False
    for line_number, line in enumerate(lines[1:], start=2):
        fields = line.split("\t")
        if fields[0] == "meta":
            if saw_test or len(fields) != 3 or fields[1] not in _METADATA_KEYS:
                raise ValueError(f"invalid metadata row {line_number}")
            if fields[1] in values:
                raise ValueError(f"duplicate metadata key: {fields[1]}")
            values[fields[1]] = fields[2]
        elif fields[0] == "test":
            saw_test = True
            if len(fields) != 9:
                raise ValueError(f"test row {line_number} must have exactly 9 fields")
            _, test_id, group, api, kind, disposition, path, timeout, digest = fields
            tests.append(
                SuiteTest(
                    test_id=test_id,
                    group=group,
                    api=api,
                    kind=kind,
                    disposition=disposition,
                    source=test_id,
                    binary=path,
                    sha256=digest,
                    timeout_ms=_parse_timeout(timeout, test_id),
                )
            )
        else:
            raise ValueError(f"unknown manifest row type at line {line_number}")
    if tuple(values) != _METADATA_KEYS:
        raise ValueError("manifest metadata fields or order are invalid")
    metadata = ManifestMetadata(
        source=values["source"],
        revision=values["revision"],
        architecture=values["architecture"],
        compiler=values["compiler"],
        libc=values["libc"],
        patch_sha256=values["patch_sha256"],
        smros_commit=values["smros_commit"],
        build_results_sha256=values["build_results_sha256"],
        manifest_sha256=values["manifest_sha256"],
    )
    _validate_metadata(metadata)
    canonical = text.replace(
        f"meta\tmanifest_sha256\t{metadata.manifest_sha256}\n",
        f"meta\tmanifest_sha256\t{EMPTY_SHA256}\n",
        1,
    )
    if hashlib.sha256(canonical.encode("utf-8")).hexdigest() != metadata.manifest_sha256:
        raise ValueError("manifest checksum mismatch")
    rendered, digest = render_manifest(metadata, tests)
    if rendered != text or digest != metadata.manifest_sha256:
        raise ValueError("manifest is not canonical")
    return metadata, tuple(tests)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as input_file:
        while chunk := input_file.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _bounded_text(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        data = value
    else:
        data = str(value).encode("utf-8", errors="replace")
    suffix = b"\n...[truncated]"
    truncated = len(data) > MAX_DIAGNOSTIC_BYTES
    limit = MAX_DIAGNOSTIC_BYTES - (len(suffix) if truncated else 0)
    text = data[:limit].decode("utf-8", errors="replace")
    text = _fit_utf8(text, limit)
    return text + (suffix.decode("ascii") if truncated else "")


def _fit_utf8(value: str, maximum_bytes: int) -> str:
    if len(value.encode("utf-8")) <= maximum_bytes:
        return value
    low = 0
    high = len(value)
    while low < high:
        middle = (low + high + 1) // 2
        if len(value[:middle].encode("utf-8")) <= maximum_bytes:
            low = middle
        else:
            high = middle - 1
    return value[:low]


def _append_captured(
    output: bytearray,
    truncated: list[bool],
    chunk: bytes,
) -> None:
    remaining = MAX_DIAGNOSTIC_BYTES - len(output)
    if remaining > 0:
        output.extend(chunk[:remaining])
    if len(chunk) > remaining:
        truncated[0] = True


def _captured_output(data: bytearray, truncated: bool) -> str:
    marker = b"\n...[truncated]"
    if truncated:
        prefix = bytes(data[: MAX_DIAGNOSTIC_BYTES - len(marker)]).decode(
            "utf-8", errors="replace"
        )
        return _fit_utf8(
            prefix, MAX_DIAGNOSTIC_BYTES - len(marker)
        ) + marker.decode("ascii")
    return _fit_utf8(
        bytes(data).decode("utf-8", errors="replace"), MAX_DIAGNOSTIC_BYTES
    )


def run_bounded_command(
    argv: Sequence[str],
    *,
    timeout_seconds: float = COMMAND_TIMEOUT_SECONDS,
    cwd: Path | None = None,
    env: Mapping[str, str] | None = None,
    input_data: bytes | None = None,
    pass_fds: Sequence[int] = (),
    text: bool = True,
    check: bool = False,
) -> subprocess.CompletedProcess[str] | subprocess.CompletedProcess[bytes]:
    """Run a host tool with bounded capture and a process-group deadline."""
    process = subprocess.Popen(
        list(argv),
        stdin=subprocess.PIPE if input_data is not None else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        cwd=cwd,
        env=env,
        pass_fds=tuple(pass_fds),
    )
    selector: selectors.BaseSelector | None = None
    try:
        assert process.stdout is not None and process.stderr is not None
        stdout = bytearray()
        stderr = bytearray()
        stdout_truncated = [False]
        stderr_truncated = [False]
        selector = selectors.DefaultSelector()
        stdout_descriptor = process.stdout.fileno()
        stderr_descriptor = process.stderr.fileno()
        os.set_blocking(stdout_descriptor, False)
        os.set_blocking(stderr_descriptor, False)
        selector.register(
            stdout_descriptor,
            selectors.EVENT_READ,
            (process.stdout, stdout, stdout_truncated),
        )
        selector.register(
            stderr_descriptor,
            selectors.EVENT_READ,
            (process.stderr, stderr, stderr_truncated),
        )
        open_readers = {stdout_descriptor, stderr_descriptor}
        stdin_descriptor: int | None = None
        input_offset = 0
        if input_data is not None:
            assert process.stdin is not None
            stdin_descriptor = process.stdin.fileno()
            os.set_blocking(stdin_descriptor, False)
            selector.register(
                stdin_descriptor,
                selectors.EVENT_WRITE,
                process.stdin,
            )
        timed_out = False
        deadline = time.monotonic() + timeout_seconds
        drain_deadline: float | None = None

        def kill_process_group() -> None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

        def reap_after_kill() -> None:
            for _ in range(2):
                if process.returncode is not None:
                    return
                try:
                    process.wait(timeout=1.0)
                    return
                except subprocess.TimeoutExpired:
                    kill_process_group()
            raise ValueError("command could not be reaped after termination")

        def close_stream(descriptor: int, stream: object) -> None:
            nonlocal stdin_descriptor
            try:
                selector.unregister(descriptor)
            except (KeyError, ValueError):
                pass
            stream.close()
            open_readers.discard(descriptor)
            if descriptor == stdin_descriptor:
                stdin_descriptor = None

        while True:
            process.poll()
            now = time.monotonic()
            if process.returncode is not None and stdin_descriptor is not None:
                assert process.stdin is not None
                close_stream(stdin_descriptor, process.stdin)
            if process.returncode is not None and not open_readers:
                break
            if not timed_out and now >= deadline:
                timed_out = True
                kill_process_group()
                reap_after_kill()
                drain_deadline = time.monotonic() + 1.0
            if timed_out and drain_deadline is not None and now >= drain_deadline:
                break
            active_deadline = drain_deadline if timed_out else deadline
            assert active_deadline is not None
            events = selector.select(
                max(0.0, min(0.05, active_deadline - time.monotonic()))
            )
            for key, mask in events:
                descriptor = int(key.fd)
                if mask & selectors.EVENT_READ:
                    stream, output, truncated = key.data
                    try:
                        chunk = os.read(descriptor, 8192)
                    except BlockingIOError:
                        continue
                    if chunk:
                        _append_captured(output, truncated, chunk)
                    else:
                        close_stream(descriptor, stream)
                elif mask & selectors.EVENT_WRITE:
                    stream = key.data
                    assert input_data is not None
                    try:
                        written = os.write(
                            descriptor,
                            input_data[input_offset : input_offset + 65_536],
                        )
                    except BlockingIOError:
                        written = 0
                    except BrokenPipeError:
                        written = 0
                        close_stream(descriptor, stream)
                    input_offset += written
                    if stdin_descriptor is not None and input_offset >= len(input_data):
                        close_stream(descriptor, stream)

        for key in tuple(selector.get_map().values()):
            close_stream(int(key.fd), key.data[0] if isinstance(key.data, tuple) else key.data)
        if process.returncode is None:
            kill_process_group()
            reap_after_kill()
        stdout_text = _captured_output(stdout, stdout_truncated[0])
        stderr_text = _captured_output(stderr, stderr_truncated[0])
        if timed_out:
            timeout_marker = f"\ncommand timed out after {timeout_seconds:g} seconds"
            stderr_text = _bounded_text(stderr_text + timeout_marker)
        stdout_value: str | bytes = stdout_text if text else stdout_text.encode("utf-8")
        stderr_value: str | bytes = stderr_text if text else stderr_text.encode("utf-8")
        completed = subprocess.CompletedProcess(
            list(argv),
            124 if timed_out else int(process.returncode),
            stdout_value,
            stderr_value,
        )
        if check and completed.returncode != 0:
            raise subprocess.CalledProcessError(
                completed.returncode,
                list(argv),
                output=completed.stdout,
                stderr=completed.stderr,
            )
        return completed
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except BaseException:
            pass
        if process.returncode is None:
            try:
                process.wait(timeout=1.0)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except BaseException:
                    pass
                try:
                    process.wait(timeout=1.0)
                except BaseException:
                    pass
            except BaseException:
                pass
        if selector is not None:
            try:
                selector.close()
            except BaseException:
                pass
        for pipe in (process.stdin, process.stdout, process.stderr):
            if pipe is not None:
                try:
                    pipe.close()
                except BaseException:
                    pass
        raise
    finally:
        if selector is not None:
            try:
                selector.close()
            except BaseException:
                pass


def _logical_command_diagnostic(
    value: object,
    logical_argv: Sequence[str],
    execution_argv: Sequence[str],
    path_replacements: Sequence[tuple[str, str]],
) -> str:
    if value is None:
        text = ""
    elif isinstance(value, bytes):
        text = value.decode("utf-8", errors="replace")
    else:
        text = str(value)
    replacements = dict(path_replacements)
    replacements.update({
        execution: logical
        for logical, execution in zip(logical_argv, execution_argv)
        if execution != logical
    })
    for execution, logical in sorted(
        replacements.items(),
        key=lambda item: len(item[0]),
        reverse=True,
    ):
        text = text.replace(execution, logical)
    return _bounded_text(text)


def _run_command(
    test_id: str,
    stage: str,
    argv: list[str],
    runner: CommandRunner,
    artifact: Path | None,
    *,
    execution_argv: Sequence[str] | None = None,
    diagnostic_path_replacements: Sequence[tuple[str, str]] = (),
    pass_fds: Sequence[int] = (),
) -> BuildResult:
    command = list(execution_argv) if execution_argv is not None else argv
    started = time.monotonic_ns()
    try:
        if runner is _SUBPROCESS_RUN:
            completed = run_bounded_command(command, pass_fds=pass_fds)
        else:
            completed = runner(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
        returncode = int(getattr(completed, "returncode"))
        stdout = _logical_command_diagnostic(
            getattr(completed, "stdout", ""),
            argv,
            command,
            diagnostic_path_replacements,
        )
        stderr = _logical_command_diagnostic(
            getattr(completed, "stderr", ""),
            argv,
            command,
            diagnostic_path_replacements,
        )
    except OSError as error:
        returncode = None
        stdout = ""
        stderr = _logical_command_diagnostic(
            error,
            argv,
            command,
            diagnostic_path_replacements,
        )
    duration_ms = max(0, (time.monotonic_ns() - started) // 1_000_000)
    artifact_sha256 = (
        sha256_file(artifact)
        if returncode == 0 and artifact is not None and artifact.is_file()
        else None
    )
    status = "passed" if returncode == 0 and (artifact is None or artifact_sha256) else "failed"
    return BuildResult(
        test_id=test_id,
        stage=stage,
        status=status,
        argv=tuple(argv),
        returncode=returncode,
        stdout=stdout,
        stderr=stderr,
        duration_ms=duration_ms,
        artifact_sha256=artifact_sha256,
    )


def _has_main(output: str) -> bool:
    return any(line.split()[-1:] == ["main"] for line in output.splitlines())


def parse_elf_dependencies(output: str) -> tuple[str | None, tuple[str, ...]]:
    interpreter: str | None = None
    needed: set[str] = set()
    for line in output.splitlines():
        interpreter_match = _INTERPRETER_RE.search(line)
        if interpreter_match is not None:
            interpreter = interpreter_match.group(1)
        needed_match = _NEEDED_RE.search(line)
        if needed_match is not None:
            needed.add(needed_match.group(1))
    return interpreter, tuple(sorted(needed))


def _runtime_search_roots(
    sysroot: Path, multiarch: str, extra_roots: Sequence[Path]
) -> tuple[Path, ...]:
    candidates = (
        *extra_roots,
        sysroot / "lib",
        sysroot / "usr/lib",
        sysroot / "lib" / multiarch,
        sysroot / "usr/lib" / multiarch,
        sysroot / "usr" / multiarch / "lib",
        sysroot / multiarch / "lib",
    )
    unique: list[Path] = []
    for candidate in candidates:
        if candidate not in unique:
            unique.append(candidate)
    return tuple(unique)


def resolve_runtime_file(
    name: str, sysroot: Path, multiarch: str, extra_roots: Sequence[Path]
) -> Path:
    if "/" in name:
        relative = name.lstrip("/")
        _validate_relative_path(relative, "ELF interpreter")
        direct = sysroot / relative
        if direct.is_file():
            return direct
        name = PurePosixPath(name).name
    _validate_atom(name, "runtime library name")
    if PurePosixPath(name).name != name:
        raise ValueError(f"invalid runtime library name: {name!r}")
    matches: list[Path] = []
    for root in _runtime_search_roots(sysroot, multiarch, extra_roots):
        candidate = root / name
        if candidate.is_file():
            resolved = candidate.resolve(strict=True)
            if resolved not in matches:
                matches.append(resolved)
    if not matches:
        raise ValueError(f"unresolved AArch64 runtime file: {name}")
    fingerprints = {(path.stat().st_dev, path.stat().st_ino) for path in matches}
    if len(fingerprints) > 1:
        raise ValueError(f"basename-colliding runtime libraries: {name}")
    return matches[0]


def compiler_query(compiler: str, argument: str) -> str:
    try:
        completed = run_bounded_command([compiler, argument])
    except OSError as error:
        raise ValueError(f"required tool is unavailable: {compiler}: {error}") from error
    if completed.returncode != 0 or not completed.stdout.strip():
        raise ValueError(f"compiler query failed: {compiler} {argument}: {_bounded_text(completed.stderr)}")
    return completed.stdout.strip().splitlines()[0]


def validate_build_checkout(
    checkout: Path, revision: str, patch_series: Path
) -> str:
    """Validate pinned provenance against current patches without fetching."""
    from .source import _load_patches, _patch_sha256, _validate_checkout

    patches = _load_patches(patch_series)
    _validate_checkout(checkout, revision, patches)
    return _patch_sha256(patches)


def stage_runtime_dependencies(
    executables: Sequence[Path],
    stage_root: Path,
    *,
    compiler: str = "aarch64-linux-gnu-gcc",
    readelf: str = "aarch64-linux-gnu-readelf",
    stage_descriptor: int | None = None,
) -> tuple[Path, ...]:
    compat_preload = stage_posix_compat_preload(
        stage_root,
        compiler=compiler,
        stage_descriptor=stage_descriptor,
    )
    sysroot_text = compiler_query(compiler, "-print-sysroot")
    multiarch = compiler_query(compiler, "-print-multiarch")
    sysroot = Path(sysroot_text)
    extra_roots: list[Path] = []
    for runtime_name in ("libc.so.6", *_IMPLICIT_RUNTIME_NAMES):
        runtime_path = compiler_query(
            compiler, f"-print-file-name={runtime_name}"
        )
        if runtime_path != runtime_name:
            extra_roots.append(Path(runtime_path).resolve().parent)
    pending = list(
        sorted((*executables, compat_preload), key=lambda path: path.as_posix())
    )
    inspected: set[Path] = set()
    runtime_by_name: dict[str, Path] = {}
    for name in _IMPLICIT_RUNTIME_NAMES:
        source = resolve_runtime_file(name, sysroot, multiarch, extra_roots)
        runtime_by_name[name] = source
        pending.append(source)
    while pending:
        elf = pending.pop(0)
        resolved_elf = elf.resolve(strict=True)
        if resolved_elf in inspected:
            continue
        inspected.add(resolved_elf)
        result = _run_command(
            "runtime",
            "readelf",
            readelf_command(readelf, elf),
            _SUBPROCESS_RUN,
            None,
            pass_fds=(stage_descriptor,) if stage_descriptor is not None else (),
        )
        if result.returncode != 0:
            raise ValueError(f"AArch64 readelf failed for {elf}: {result.stderr}")
        interpreter, needed = parse_elf_dependencies(result.stdout)
        names = list(needed)
        if interpreter is not None:
            names.append(interpreter)
        for name in names:
            source = resolve_runtime_file(name, sysroot, multiarch, extra_roots)
            basename = PurePosixPath(name).name
            previous = runtime_by_name.get(basename)
            if previous is not None and previous != source:
                raise ValueError(f"basename-colliding runtime libraries: {basename}")
            if previous is None:
                runtime_by_name[basename] = source
                pending.append(source)
    staged: list[Path] = [compat_preload]
    for basename, source in sorted(runtime_by_name.items()):
        destination = safe_stage_path(stage_root, f"lib/{basename}")
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination, follow_symlinks=True)
        os.chmod(destination, 0o755)
        staged.append(destination)
    return tuple(staged)


def stage_posix_compat_preload(
    stage_root: Path,
    *,
    compiler: str = "aarch64-linux-gnu-gcc",
    stage_descriptor: int | None = None,
) -> Path:
    destination = safe_stage_path(stage_root, f"lib/{POSIX_COMPAT_PRELOAD_NAME}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    result = _run_command(
        "runtime",
        "compat-preload-link",
        posix_compat_preload_command(
            compiler,
            POSIX_COMPAT_PRELOAD_SOURCE,
            destination,
        ),
        _SUBPROCESS_RUN,
        destination,
        pass_fds=(stage_descriptor,) if stage_descriptor is not None else (),
    )
    if result.returncode is None or result.status != "passed":
        raise ValueError(
            "AArch64 POSIX compatibility preload link failed: "
            f"{result.stderr}"
        )
    os.chmod(destination, 0o755)
    return destination


def _support_catalog_required(tests: Sequence[SuiteTest]) -> bool:
    return any(
        test.test_id == _FORK_MESSAGE_CATALOG_TEST_ID
        and test.disposition != "excluded-upstream-stub"
        for test in tests
    )


def stage_support_files(
    checkout: Path,
    stage_root: Path,
    tests: Sequence[SuiteTest],
) -> tuple[Path, ...]:
    staged: list[Path] = []
    if not _support_catalog_required(tests):
        return ()

    source = safe_stage_path(checkout, _FORK_MESSAGE_CATALOG_SOURCE)
    if not source.is_file():
        raise ValueError(
            "missing POSIX support source: "
            f"{_FORK_MESSAGE_CATALOG_SOURCE}"
    )
    destination = safe_stage_path(stage_root, _FORK_MESSAGE_CATALOG_SUPPORT)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination_text = str(destination)
    if destination_text.startswith("/proc/self/fd/"):
        destination_text = "/proc/{}/fd/{}".format(
            os.getpid(), destination_text.removeprefix("/proc/self/fd/")
        )
    result = run_bounded_command(["gencat", destination_text, str(source)])
    if result.returncode != 0 or not destination.is_file():
        diagnostic = _bounded_text(result.stderr or result.stdout)
        raise ValueError(
            "POSIX support catalog generation failed: "
            f"{_FORK_MESSAGE_CATALOG_SUPPORT}: {diagnostic}"
        )
    os.chmod(destination, 0o644)
    staged.append(destination)
    return tuple(staged)


def _support_manifest(
    stage: Path,
    support_files: Sequence[Path],
) -> list[dict[str, str]]:
    manifest: list[dict[str, str]] = []
    for path in sorted(support_files, key=lambda item: item.as_posix()):
        relative = path.relative_to(stage).as_posix()
        manifest.append({"path": relative, "sha256": sha256_file(path)})
    return manifest


def _json_build_result(result: BuildResult) -> str:
    value = asdict(result)
    value["argv"] = list(result.argv)
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def _build_results_digest(results: Sequence[BuildResult]) -> str:
    canonical = "".join(
        _json_build_result(replace(result, duration_ms=0)) + "\n"
        for result in results
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _shell_test(path: str) -> SuiteTest:
    from .discovery import api_group

    parts = PurePosixPath(path).parts
    api = parts[-2] if len(parts) > 1 else "shell"
    return SuiteTest(
        test_id=path,
        group=api_group(api),
        api=api,
        kind="shell",
        disposition="not-built-shell-test",
        source=path,
        binary="-",
        sha256=EMPTY_SHA256,
        timeout_ms=30_000,
    )


def _stage_size(root: Path) -> int:
    total = 0
    for directory, directory_names, file_names in os.walk(root, followlinks=False):
        directory_names.sort()
        file_names.sort()
        for name in file_names:
            path = Path(directory) / name
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f"staged artifact is not a regular file: {path}")
            total += info.st_size
            if total > MAX_STAGE_BYTES:
                raise ValueError("POSIX stage exceeds the 256 MiB limit")
    return total


def _directory_open_flags() -> int:
    flags = os.O_RDONLY
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_DIRECTORY"):
        flags |= os.O_DIRECTORY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _open_directory_at(parent_descriptor: int, name: str, label: str) -> int:
    try:
        path_stat = os.stat(
            name, dir_fd=parent_descriptor, follow_symlinks=False
        )
    except OSError as error:
        raise ValueError(f"{label} is missing: {name}") from error
    if stat.S_ISLNK(path_stat.st_mode):
        raise ValueError(f"{label} must not be a symlink: {name}")
    if not stat.S_ISDIR(path_stat.st_mode):
        raise ValueError(f"{label} must be a directory: {name}")
    try:
        descriptor = os.open(
            name, _directory_open_flags(), dir_fd=parent_descriptor
        )
    except OSError as error:
        raise ValueError(f"{label} could not be opened safely: {name}") from error
    opened_stat = os.fstat(descriptor)
    if not stat.S_ISDIR(opened_stat.st_mode) or (
        opened_stat.st_dev,
        opened_stat.st_ino,
    ) != (path_stat.st_dev, path_stat.st_ino):
        os.close(descriptor)
        raise ValueError(f"{label} changed while being opened: {name}")
    return descriptor


def _open_directory_chain(path: Path, label: str, *, create: bool) -> int:
    absolute = Path(os.path.abspath(path))
    descriptor = os.open(absolute.anchor, _directory_open_flags())
    current = Path(absolute.anchor)
    try:
        for part in absolute.parts[1:]:
            current /= part
            try:
                child_descriptor = _open_directory_at(
                    descriptor, part, f"{label} component {current}"
                )
            except ValueError as error:
                try:
                    os.stat(part, dir_fd=descriptor, follow_symlinks=False)
                except FileNotFoundError:
                    if not create:
                        raise error
                    try:
                        os.mkdir(part, 0o755, dir_fd=descriptor)
                    except FileExistsError:
                        pass
                    child_descriptor = _open_directory_at(
                        descriptor, part, f"{label} component {current}"
                    )
                else:
                    raise
            os.close(descriptor)
            descriptor = child_descriptor
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _open_campaign_lock_directories(
    stage_parent: Path,
    work_parent: Path,
    destination_name: str,
) -> tuple[int, int, int, tuple[int, ...]]:
    roles = (
        (stage_parent, "stage parent"),
        (work_parent, "work parent"),
    )
    unique: dict[tuple[int, int], tuple[str, int]] = {}
    role_descriptors: list[int] = []
    quarantine_descriptor: int | None = None

    def add_role(path: Path, descriptor: int) -> int:
        info = os.fstat(descriptor)
        identity = (info.st_dev, info.st_ino)
        existing = unique.get(identity)
        if existing is not None:
            os.close(descriptor)
            return existing[1]
        unique[identity] = (os.path.abspath(path), descriptor)
        return descriptor

    try:
        for path, label in roles:
            descriptor = _open_directory_chain(path, label, create=True)
            role_descriptors.append(add_role(path, descriptor))
        quarantine_descriptor = _open_or_create_directory_unchecked(
            role_descriptors[0],
            _STAGE_QUARANTINE_NAME,
            "stage quarantine",
        )
        slot_name = _stage_work_slot_name(destination_name)
        slot_candidate = _open_or_create_directory_unchecked(
            quarantine_descriptor,
            slot_name,
            "stage work slot",
        )
        slot_path = stage_parent / _STAGE_QUARANTINE_NAME / slot_name
        slot_descriptor = add_role(slot_path, slot_candidate)
        ordered = tuple(
            descriptor
            for (_device, _inode), (_path, descriptor) in sorted(
                unique.items(),
                key=lambda item: (item[0][0], item[0][1], item[1][0]),
            )
        )
        for descriptor in ordered:
            fcntl.flock(descriptor, fcntl.LOCK_EX)
        _require_private_directory_descriptor(
            quarantine_descriptor,
            "stage quarantine",
        )
        _require_private_directory_descriptor(
            slot_descriptor,
            "stage work slot",
        )
        _validate_held_entry(
            role_descriptors[0],
            _STAGE_QUARANTINE_NAME,
            quarantine_descriptor,
            "stage quarantine",
        )
        _validate_held_entry(
            quarantine_descriptor,
            slot_name,
            slot_descriptor,
            "stage work slot",
        )
        os.fsync(role_descriptors[0])
        os.fsync(quarantine_descriptor)
        os.close(quarantine_descriptor)
        quarantine_descriptor = None
        return (
            role_descriptors[0],
            role_descriptors[1],
            slot_descriptor,
            ordered,
        )
    except BaseException:
        if quarantine_descriptor is not None:
            try:
                os.close(quarantine_descriptor)
            except BaseException:
                pass
        for _path, descriptor in unique.values():
            try:
                os.close(descriptor)
            except BaseException:
                pass
        raise


def _entry_exists_at(parent_descriptor: int, name: str) -> bool:
    try:
        os.stat(name, dir_fd=parent_descriptor, follow_symlinks=False)
    except FileNotFoundError:
        return False
    return True


def _open_or_create_private_directory(
    parent_descriptor: int,
    name: str,
    label: str,
) -> int:
    descriptor = _open_or_create_directory_unchecked(
        parent_descriptor,
        name,
        label,
    )
    try:
        _require_private_directory_descriptor(descriptor, label)
    except BaseException:
        os.close(descriptor)
        raise
    return descriptor


def _open_or_create_directory_unchecked(
    parent_descriptor: int,
    name: str,
    label: str,
) -> int:
    try:
        os.mkdir(name, 0o700, dir_fd=parent_descriptor)
    except FileExistsError:
        pass
    return _open_directory_at(parent_descriptor, name, label)


def _require_private_directory_descriptor(
    descriptor: int,
    label: str,
) -> None:
    info = os.fstat(descriptor)
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError(f"{label} ownership or mode is unsafe")


def _stage_work_slot_name(destination_name: str) -> str:
    digest = hashlib.sha256(os.fsencode(destination_name)).hexdigest()
    return f"stage-{digest}"


def _clear_directory(directory_descriptor: int) -> None:
    with os.scandir(directory_descriptor) as iterator:
        entries = list(iterator)
    for entry in entries:
        entry_stat = os.stat(
            entry.name, dir_fd=directory_descriptor, follow_symlinks=False
        )
        if stat.S_ISDIR(entry_stat.st_mode):
            child_descriptor = _open_directory_at(
                directory_descriptor, entry.name, "temporary stage directory"
            )
            try:
                _clear_directory(child_descriptor)
            finally:
                os.close(child_descriptor)
            os.rmdir(entry.name, dir_fd=directory_descriptor)
        else:
            os.unlink(entry.name, dir_fd=directory_descriptor)


def _open_and_reset_generated_directory(
    parent_descriptor: int,
    name: str,
    label: str,
) -> int:
    descriptor = _open_or_create_directory_unchecked(
        parent_descriptor,
        name,
        label,
    )
    try:
        _validate_held_entry(parent_descriptor, name, descriptor, label)
        _clear_directory(descriptor)
    except BaseException:
        os.close(descriptor)
        raise
    return descriptor


def _clear_stage_work_root(directory_descriptor: int) -> None:
    _clear_directory(directory_descriptor)
    os.fchmod(directory_descriptor, 0o700)
    os.fsync(directory_descriptor)


def _require_empty_stage_work_root(directory_descriptor: int) -> None:
    info = os.fstat(directory_descriptor)
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError("stage work root ownership or mode is unsafe")
    with os.scandir(directory_descriptor) as entries:
        if next(entries, None) is not None:
            raise ValueError("stage work root must be empty before reuse")


def _descriptor_identity(descriptor: int) -> tuple[int, int]:
    info = os.fstat(descriptor)
    return info.st_dev, info.st_ino


def _validate_stage_journal_value(
    value: Mapping[str, int | str],
) -> None:
    state = value.get("state")
    expected_fields = {
        "idle": {"schema", "state"},
        "building": {"schema", "state", "work_dev", "work_ino"},
        "initial": {"schema", "state", "work_dev", "work_ino"},
        "exchange": {
            "schema",
            "state",
            "work_dev",
            "work_ino",
            "destination_dev",
            "destination_ino",
        },
    }
    if (
        state not in expected_fields
        or value.get("schema") != 1
        or set(value) != expected_fields[state]
    ):
        raise ValueError("stage journal schema is invalid")
    for key in set(value) - {"schema", "state"}:
        if type(value[key]) is not int or int(value[key]) < 0:
            raise ValueError("stage journal inode identity is invalid")


def _stage_journal_payload(value: Mapping[str, int | str]) -> bytes:
    _validate_stage_journal_value(value)
    return (
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        + "\n"
    ).encode("ascii")


def _encode_stage_journal_record(
    generation: int,
    value: Mapping[str, int | str],
) -> bytes:
    if generation < 0 or generation > 0xFFFFFFFFFFFFFFFF:
        raise ValueError("stage journal generation is invalid")
    payload = _stage_journal_payload(value)
    payload_limit = (
        _STAGE_JOURNAL_RECORD_BYTES - _STAGE_JOURNAL_HEADER_BYTES
    )
    if len(payload) > payload_limit:
        raise ValueError("stage journal state exceeds its fixed record")
    checksum = hashlib.sha256(
        generation.to_bytes(8, "big") + payload
    ).hexdigest()
    header = (
        f"{_STAGE_JOURNAL_MAGIC} {generation:016x} "
        f"{len(payload):04x} {checksum}\n"
    ).encode("ascii")
    if len(header) != _STAGE_JOURNAL_HEADER_BYTES:
        raise AssertionError("stage journal header size changed")
    return header + payload + bytes(payload_limit - len(payload))


def _decode_stage_journal_record(
    record: bytes,
) -> tuple[int, dict[str, int | str]] | None:
    if len(record) != _STAGE_JOURNAL_RECORD_BYTES:
        return None
    try:
        header = record[:_STAGE_JOURNAL_HEADER_BYTES].decode("ascii")
        magic, generation_text, length_text, checksum = header[:-1].split(" ")
        if (
            not header.endswith("\n")
            or magic != _STAGE_JOURNAL_MAGIC
            or len(generation_text) != 16
            or len(length_text) != 4
            or _DIGEST_RE.fullmatch(checksum) is None
        ):
            return None
        generation = int(generation_text, 16)
        payload_length = int(length_text, 16)
        if generation_text != f"{generation:016x}":
            return None
        payload_limit = (
            _STAGE_JOURNAL_RECORD_BYTES - _STAGE_JOURNAL_HEADER_BYTES
        )
        if payload_length > payload_limit:
            return None
        payload_region = record[_STAGE_JOURNAL_HEADER_BYTES:]
        payload = payload_region[:payload_length]
        if any(payload_region[payload_length:]):
            return None
        if hashlib.sha256(
            generation.to_bytes(8, "big") + payload
        ).hexdigest() != checksum:
            return None
        text = payload.decode("ascii")
        value = json.loads(text, object_pairs_hook=_reject_duplicate_json_keys)
        if not isinstance(value, dict):
            return None
        _validate_stage_journal_value(value)
        if payload != _stage_journal_payload(value):
            return None
    except (UnicodeError, ValueError, json.JSONDecodeError):
        return None
    return generation, value


def _require_safe_stage_journal_descriptor(
    descriptor: int,
    *,
    expected_size: int | tuple[int, ...],
) -> os.stat_result:
    info = os.fstat(descriptor)
    sizes = (expected_size,) if isinstance(expected_size, int) else expected_size
    if (
        not stat.S_ISREG(info.st_mode)
        or info.st_uid != os.geteuid()
        or stat.S_IMODE(info.st_mode) != 0o600
        or info.st_nlink != 1
        or info.st_size not in sizes
    ):
        raise ValueError("stage journal is not a safe fixed-size regular file")
    return info


def _validate_held_stage_journal(
    slot_descriptor: int,
    journal_descriptor: int,
) -> None:
    try:
        entry_info = os.stat(
            _STAGE_JOURNAL_NAME,
            dir_fd=slot_descriptor,
            follow_symlinks=False,
        )
    except OSError as error:
        raise ValueError("held stage journal is missing") from error
    held_info = os.fstat(journal_descriptor)
    if (
        not stat.S_ISREG(entry_info.st_mode)
        or (entry_info.st_dev, entry_info.st_ino)
        != (held_info.st_dev, held_info.st_ino)
    ):
        raise ValueError("held stage journal changed")


def _pwrite_all(descriptor: int, data: bytes, offset: int) -> None:
    written = 0
    while written < len(data):
        count = os.pwrite(descriptor, data[written:], offset + written)
        if count <= 0:
            raise OSError("stage journal write made no progress")
        written += count


def _commit_stage_journal_record(
    journal_descriptor: int,
    record: bytes,
    offset: int,
) -> None:
    body = record[_STAGE_JOURNAL_HEADER_BYTES:]
    _pwrite_all(
        journal_descriptor,
        body,
        offset + _STAGE_JOURNAL_HEADER_BYTES,
    )
    os.fsync(journal_descriptor)
    _pwrite_all(
        journal_descriptor,
        record[:_STAGE_JOURNAL_HEADER_BYTES],
        offset,
    )
    os.fsync(journal_descriptor)


def _stage_work_root_is_empty_or_missing(slot_descriptor: int) -> bool:
    work_descriptor = _open_optional_directory_at(
        slot_descriptor,
        _STAGE_WORK_ROOT_NAME,
        "stage work root",
    )
    if work_descriptor is None:
        return True
    try:
        _require_empty_stage_work_root(work_descriptor)
        return True
    except ValueError:
        return False
    finally:
        os.close(work_descriptor)


def _initialize_stage_journal(
    slot_descriptor: int,
    journal_descriptor: int,
) -> None:
    if not _stage_work_root_is_empty_or_missing(slot_descriptor):
        raise ValueError(
            "uninitialized stage journal has a nonempty work root"
        )
    _validate_held_stage_journal(slot_descriptor, journal_descriptor)
    _require_safe_stage_journal_descriptor(
        journal_descriptor,
        expected_size=(0, _STAGE_JOURNAL_BYTES),
    )
    os.ftruncate(journal_descriptor, 0)
    os.ftruncate(journal_descriptor, _STAGE_JOURNAL_BYTES)
    _commit_stage_journal_record(
        journal_descriptor,
        _encode_stage_journal_record(
            0,
            {"schema": 1, "state": "idle"},
        ),
        0,
    )
    _require_safe_stage_journal_descriptor(
        journal_descriptor,
        expected_size=_STAGE_JOURNAL_BYTES,
    )
    _validate_held_stage_journal(slot_descriptor, journal_descriptor)
    os.fsync(slot_descriptor)


def _open_stage_journal(slot_descriptor: int) -> int:
    flags = os.O_RDWR
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    if hasattr(os, "O_NONBLOCK"):
        flags |= os.O_NONBLOCK
    created = False
    try:
        descriptor = os.open(
            _STAGE_JOURNAL_NAME,
            flags | os.O_CREAT | os.O_EXCL,
            0o600,
            dir_fd=slot_descriptor,
        )
        created = True
    except FileExistsError:
        try:
            descriptor = os.open(
                _STAGE_JOURNAL_NAME,
                flags,
                dir_fd=slot_descriptor,
            )
        except OSError as error:
            raise ValueError("stage journal could not be opened safely") from error
    except OSError as error:
        raise ValueError("stage journal could not be created safely") from error
    try:
        _require_safe_stage_journal_descriptor(
            descriptor,
            expected_size=(0, _STAGE_JOURNAL_BYTES),
        )
        _validate_held_stage_journal(slot_descriptor, descriptor)
        needs_initialization = created or os.fstat(descriptor).st_size == 0
        if not needs_initialization:
            try:
                _read_stage_journal(descriptor)
            except ValueError as error:
                if str(error) != "stage journal has no valid record":
                    raise
                needs_initialization = True
        if needs_initialization:
            _initialize_stage_journal(slot_descriptor, descriptor)
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _read_stage_journal(
    journal_descriptor: int,
) -> tuple[int, int, dict[str, int | str]]:
    _require_safe_stage_journal_descriptor(
        journal_descriptor,
        expected_size=_STAGE_JOURNAL_BYTES,
    )
    chunks: list[bytes] = []
    offset = 0
    while offset < _STAGE_JOURNAL_BYTES:
        chunk = os.pread(
            journal_descriptor,
            _STAGE_JOURNAL_BYTES - offset,
            offset,
        )
        if not chunk:
            break
        chunks.append(chunk)
        offset += len(chunk)
    data = b"".join(chunks)
    if len(data) != _STAGE_JOURNAL_BYTES:
        raise ValueError("stage journal could not be read completely")
    valid: list[tuple[int, int, dict[str, int | str]]] = []
    for index in range(_STAGE_JOURNAL_RECORD_COUNT):
        start = index * _STAGE_JOURNAL_RECORD_BYTES
        decoded = _decode_stage_journal_record(
            data[start : start + _STAGE_JOURNAL_RECORD_BYTES]
        )
        if decoded is not None:
            generation, value = decoded
            valid.append((generation, index, value))
    if not valid:
        raise ValueError("stage journal has no valid record")
    valid.sort(key=lambda item: item[0], reverse=True)
    if len(valid) > 1 and valid[0][0] == valid[1][0]:
        raise ValueError("stage journal generation is ambiguous")
    return valid[0]


def _load_stage_journal(
    slot_descriptor: int,
    *,
    journal_descriptor: int | None = None,
) -> dict[str, int | str]:
    owned_descriptor = journal_descriptor is None
    if journal_descriptor is None:
        journal_descriptor = _open_stage_journal(slot_descriptor)
    try:
        _validate_held_stage_journal(slot_descriptor, journal_descriptor)
        _generation, _index, value = _read_stage_journal(journal_descriptor)
        return value
    finally:
        if owned_descriptor:
            os.close(journal_descriptor)


def _write_stage_journal(
    slot_descriptor: int,
    value: Mapping[str, int | str],
    *,
    journal_descriptor: int,
) -> None:
    generation, active_index, _current = _read_stage_journal(
        journal_descriptor
    )
    if generation == 0xFFFFFFFFFFFFFFFF:
        raise ValueError("stage journal generation is exhausted")
    _require_safe_stage_journal_descriptor(
        journal_descriptor,
        expected_size=_STAGE_JOURNAL_BYTES,
    )
    _validate_held_stage_journal(slot_descriptor, journal_descriptor)
    next_index = (active_index + 1) % _STAGE_JOURNAL_RECORD_COUNT
    _commit_stage_journal_record(
        journal_descriptor,
        _encode_stage_journal_record(generation + 1, value),
        next_index * _STAGE_JOURNAL_RECORD_BYTES,
    )
    _require_safe_stage_journal_descriptor(
        journal_descriptor,
        expected_size=_STAGE_JOURNAL_BYTES,
    )
    _validate_held_stage_journal(slot_descriptor, journal_descriptor)


def _record_stage_transaction(
    slot_descriptor: int,
    state: str,
    work_descriptor: int | None = None,
    destination_descriptor: int | None = None,
    *,
    journal_descriptor: int | None = None,
) -> None:
    owned_descriptor = journal_descriptor is None
    if journal_descriptor is None:
        journal_descriptor = _open_stage_journal(slot_descriptor)
    try:
        if state == "idle":
            _write_stage_journal(
                slot_descriptor,
                {"schema": 1, "state": "idle"},
                journal_descriptor=journal_descriptor,
            )
            return
        if work_descriptor is None:
            raise ValueError("active stage journal requires a work root")
        current = _load_stage_journal(
            slot_descriptor,
            journal_descriptor=journal_descriptor,
        )
        work_device, work_inode = _descriptor_identity(work_descriptor)
        if state == "building":
            if current.get("state") != "idle":
                raise ValueError("stage journal is not idle before build")
            _require_empty_stage_work_root(work_descriptor)
        elif state in {"initial", "exchange"}:
            if (
                current.get("state") != "building"
                or (current.get("work_dev"), current.get("work_ino"))
                != (work_device, work_inode)
            ):
                raise ValueError(
                    "stage journal does not match the active build inode"
                )
        else:
            raise ValueError(f"unknown stage journal state: {state}")
        value: dict[str, int | str] = {
            "schema": 1,
            "state": state,
            "work_dev": work_device,
            "work_ino": work_inode,
        }
        if state == "exchange":
            if destination_descriptor is None:
                raise ValueError(
                    "exchange journal requires a destination inode"
                )
            destination_device, destination_inode = _descriptor_identity(
                destination_descriptor
            )
            value.update(
                destination_dev=destination_device,
                destination_ino=destination_inode,
            )
        _write_stage_journal(
            slot_descriptor,
            value,
            journal_descriptor=journal_descriptor,
        )
    finally:
        if owned_descriptor:
            os.close(journal_descriptor)


def _journal_matches_descriptor(
    journal: Mapping[str, int | str],
    prefix: str,
    descriptor: int | None,
) -> bool:
    if descriptor is None:
        return False
    return _descriptor_identity(descriptor) == (
        journal[f"{prefix}_dev"],
        journal[f"{prefix}_ino"],
    )


def _open_optional_directory_at(
    parent_descriptor: int,
    name: str,
    label: str,
) -> int | None:
    try:
        os.stat(name, dir_fd=parent_descriptor, follow_symlinks=False)
    except FileNotFoundError:
        return None
    return _open_directory_at(parent_descriptor, name, label)


def _recover_stage_transaction(
    slot_descriptor: int,
    destination_parent_descriptor: int,
    destination_name: str,
    *,
    journal_descriptor: int | None = None,
) -> None:
    owned_journal_descriptor = journal_descriptor is None
    if journal_descriptor is None:
        journal_descriptor = _open_stage_journal(slot_descriptor)
    work_descriptor: int | None = None
    destination_descriptor: int | None = None
    work_root_created = False
    try:
        journal = _load_stage_journal(
            slot_descriptor,
            journal_descriptor=journal_descriptor,
        )
        work_descriptor = _open_optional_directory_at(
            slot_descriptor,
            _STAGE_WORK_ROOT_NAME,
            "stage work root",
        )
        state = journal["state"]
        if state == "idle":
            if work_descriptor is None:
                os.mkdir(
                    _STAGE_WORK_ROOT_NAME,
                    0o700,
                    dir_fd=slot_descriptor,
                )
                work_descriptor = _open_directory_at(
                    slot_descriptor,
                    _STAGE_WORK_ROOT_NAME,
                    "stage work root",
                )
                work_root_created = True
            _require_empty_stage_work_root(work_descriptor)
            _validate_held_entry(
                slot_descriptor,
                _STAGE_WORK_ROOT_NAME,
                work_descriptor,
                "stage work root",
            )
            if work_root_created:
                os.fsync(slot_descriptor)
            return
        destination_descriptor = _open_optional_directory_at(
            destination_parent_descriptor,
            destination_name,
            "journal stage destination",
        )
        if work_descriptor is None and state == "initial" and (
            _journal_matches_descriptor(journal, "work", destination_descriptor)
        ):
            os.mkdir(
                _STAGE_WORK_ROOT_NAME,
                0o700,
                dir_fd=slot_descriptor,
            )
            work_descriptor = _open_directory_at(
                slot_descriptor,
                _STAGE_WORK_ROOT_NAME,
                "stage work root",
            )
            work_root_created = True
        if work_descriptor is None:
            raise ValueError("stage journal work inode is missing")
        work_is_new = _journal_matches_descriptor(
            journal, "work", work_descriptor
        )
        destination_is_new = _journal_matches_descriptor(
            journal, "work", destination_descriptor
        )
        if state == "building":
            if not work_is_new:
                raise ValueError("stage journal work inode mismatch")
            _clear_stage_work_root(work_descriptor)
        elif state == "initial":
            if work_is_new:
                _clear_stage_work_root(work_descriptor)
            elif destination_is_new:
                _require_empty_stage_work_root(work_descriptor)
            else:
                raise ValueError("stage journal publication inode mismatch")
        elif state == "exchange":
            work_is_old_destination = _journal_matches_descriptor(
                journal, "destination", work_descriptor
            )
            destination_is_old = _journal_matches_descriptor(
                journal, "destination", destination_descriptor
            )
            if work_is_new and destination_is_old:
                _clear_stage_work_root(work_descriptor)
            elif work_is_old_destination and destination_is_new:
                _clear_stage_work_root(work_descriptor)
            else:
                raise ValueError("stage journal exchange inode mismatch")
        else:
            raise ValueError("stage journal state is invalid")
        _validate_held_entry(
            slot_descriptor,
            _STAGE_WORK_ROOT_NAME,
            work_descriptor,
            "journal stage work root",
        )
        if destination_descriptor is not None and state == "exchange":
            _validate_held_entry(
                destination_parent_descriptor,
                destination_name,
                destination_descriptor,
                "journal stage destination",
            )
        elif destination_descriptor is not None and destination_is_new:
            _validate_held_entry(
                destination_parent_descriptor,
                destination_name,
                destination_descriptor,
                "journal stage destination",
            )
        if work_root_created:
            os.fsync(slot_descriptor)
        _record_stage_transaction(
            slot_descriptor,
            "idle",
            journal_descriptor=journal_descriptor,
        )
    finally:
        if destination_descriptor is not None:
            os.close(destination_descriptor)
        if work_descriptor is not None:
            os.close(work_descriptor)
        if owned_journal_descriptor:
            os.close(journal_descriptor)


def _open_stage_work_slot(
    parent_descriptor: int,
    destination_name: str,
) -> tuple[int, int]:
    slot_name = _stage_work_slot_name(destination_name)
    quarantine_descriptor = _open_or_create_private_directory(
        parent_descriptor,
        _STAGE_QUARANTINE_NAME,
        "stage quarantine",
    )
    try:
        slot_descriptor = _open_or_create_private_directory(
            quarantine_descriptor,
            slot_name,
            "stage work slot",
        )
        _validate_held_entry(
            parent_descriptor,
            _STAGE_QUARANTINE_NAME,
            quarantine_descriptor,
            "stage quarantine",
        )
        _validate_held_entry(
            quarantine_descriptor,
            slot_name,
            slot_descriptor,
            "stage work slot",
        )
        os.fsync(parent_descriptor)
        os.fsync(quarantine_descriptor)
    finally:
        os.close(quarantine_descriptor)
    try:
        fcntl.flock(slot_descriptor, fcntl.LOCK_EX)
        work_descriptor = _activate_stage_work_slot(
            slot_descriptor,
            parent_descriptor,
            destination_name,
        )
        return slot_descriptor, work_descriptor
    except BaseException:
        os.close(slot_descriptor)
        raise


def _activate_stage_work_slot(
    slot_descriptor: int,
    destination_parent_descriptor: int,
    destination_name: str,
    *,
    journal_descriptor: int | None = None,
) -> int:
    _recover_stage_transaction(
        slot_descriptor,
        destination_parent_descriptor,
        destination_name,
        journal_descriptor=journal_descriptor,
    )
    work_descriptor = _open_directory_at(
        slot_descriptor,
        _STAGE_WORK_ROOT_NAME,
        "stage work root",
    )
    try:
        _require_empty_stage_work_root(work_descriptor)
    except BaseException:
        os.close(work_descriptor)
        raise
    return work_descriptor


def _validate_held_entry(
    parent_descriptor: int,
    name: str,
    directory_descriptor: int,
    label: str,
) -> None:
    entry_stat = os.stat(name, dir_fd=parent_descriptor, follow_symlinks=False)
    held_stat = os.fstat(directory_descriptor)
    if not stat.S_ISDIR(entry_stat.st_mode) or (
        entry_stat.st_dev,
        entry_stat.st_ino,
    ) != (held_stat.st_dev, held_stat.st_ino):
        raise ValueError(f"{label} changed before publication")


def _rename_between_at(
    source_parent_descriptor: int,
    source_name: str,
    destination_parent_descriptor: int,
    destination_name: str,
    flags: int,
) -> None:
    renameat2 = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
    if renameat2 is None:
        raise ValueError("atomic stage rename is not supported by this host")
    renameat2.argtypes = (
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    )
    renameat2.restype = ctypes.c_int
    if renameat2(
        source_parent_descriptor,
        os.fsencode(source_name),
        destination_parent_descriptor,
        os.fsencode(destination_name),
        flags,
    ) != 0:
        error_number = ctypes.get_errno()
        if error_number in {errno.ENOSYS, errno.EINVAL, errno.ENOTSUP}:
            raise ValueError(
                "atomic stage rename is not supported by this filesystem"
            )
        raise OSError(
            error_number,
            os.strerror(error_number),
            destination_name,
        )


def _fsync_directory_descriptors(*descriptors: int) -> None:
    seen: set[tuple[int, int]] = set()
    for descriptor in descriptors:
        identity = _descriptor_identity(descriptor)
        if identity in seen:
            continue
        seen.add(identity)
        os.fsync(descriptor)


def _publish_stage(
    source_parent_descriptor: int,
    temporary_name: str,
    temporary_descriptor: int,
    destination_parent_descriptor: int,
    destination_name: str,
    *,
    journal_descriptor: int | None = None,
) -> int | None:
    _validate_held_entry(
        source_parent_descriptor,
        temporary_name,
        temporary_descriptor,
        "owned temporary stage",
    )
    if not _entry_exists_at(destination_parent_descriptor, destination_name):
        _record_stage_transaction(
            source_parent_descriptor,
            "initial",
            temporary_descriptor,
            journal_descriptor=journal_descriptor,
        )
        _rename_between_at(
            source_parent_descriptor,
            temporary_name,
            destination_parent_descriptor,
            destination_name,
            1,
        )
        _fsync_directory_descriptors(
            source_parent_descriptor,
            destination_parent_descriptor,
        )
        _validate_held_entry(
            destination_parent_descriptor,
            destination_name,
            temporary_descriptor,
            "published stage",
        )
        return None

    destination_descriptor = _open_directory_at(
        destination_parent_descriptor,
        destination_name,
        "stage destination",
    )
    try:
        _record_stage_transaction(
            source_parent_descriptor,
            "exchange",
            temporary_descriptor,
            destination_descriptor,
            journal_descriptor=journal_descriptor,
        )
        _rename_between_at(
            source_parent_descriptor,
            temporary_name,
            destination_parent_descriptor,
            destination_name,
            2,
        )
        _fsync_directory_descriptors(
            source_parent_descriptor,
            destination_parent_descriptor,
        )
        _validate_held_entry(
            destination_parent_descriptor,
            destination_name,
            temporary_descriptor,
            "published stage",
        )
        _validate_held_entry(
            source_parent_descriptor,
            temporary_name,
            destination_descriptor,
            "replaced stage",
        )
        return destination_descriptor
    except BaseException:
        os.close(destination_descriptor)
        raise


def _write_manifests(
    stage: Path,
    metadata: ManifestMetadata,
    tests: Sequence[SuiteTest],
    results: Sequence[BuildResult],
    support_files: Sequence[Path] = (),
) -> str:
    build_results_text = "".join(
        _json_build_result(result) + "\n" for result in results
    )
    build_results_digest = _build_results_digest(results)
    bound_metadata = replace(
        metadata, build_results_sha256=build_results_digest
    )
    manifest_text, manifest_digest = render_manifest(bound_metadata, tests)
    final_metadata = replace(bound_metadata, manifest_sha256=manifest_digest)
    runtime = [
        {
            "path": f"lib/{path.name}",
            "sha256": sha256_file(path),
        }
        for path in sorted(
            (stage / "lib").iterdir() if (stage / "lib").is_dir() else (),
            key=lambda item: item.name,
        )
    ]
    manifest_json = {
        "schema": 1,
        "checksum_definition": CHECKSUM_DEFINITION,
        "metadata": asdict(final_metadata),
        "runtime": runtime,
        "support": _support_manifest(stage, support_files),
        "tests": [asdict(test) for test in sorted(tests, key=lambda item: item.test_id)],
    }
    (stage / "manifest.tsv").write_text(manifest_text, encoding="utf-8", newline="\n")
    (stage / "manifest.json").write_text(
        json.dumps(manifest_json, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    (stage / "build-results.ndjson").write_text(
        build_results_text,
        encoding="utf-8",
        newline="\n",
    )
    return manifest_digest


def build_campaign(
    checkout: Path,
    tests: Iterable[SuiteTest],
    shell_tests: Iterable[str],
    metadata: ManifestMetadata,
    stage: Path,
    work: Path,
    *,
    command_runner: CommandRunner = subprocess.run,
    dependency_stager: DependencyStager | None = None,
) -> BuildSummary:
    ordered_tests = tuple(sorted(tests, key=lambda test: test.test_id))
    ordered_shells = tuple(sorted(shell_tests))
    object_root = work / "obj"
    artifact_root = work / "bin"
    stage = Path(os.path.abspath(stage))
    absolute_work = Path(os.path.abspath(work))
    if not stage.name:
        raise ValueError("stage destination must have a directory name")
    if not absolute_work.name:
        raise ValueError("work root must have a directory name")
    checkout_descriptor = _open_directory_chain(
        checkout,
        "checkout root",
        create=False,
    )
    try:
        (
            stage_parent_descriptor,
            work_parent_descriptor,
            work_slot_descriptor,
            campaign_lock_descriptors,
        ) = _open_campaign_lock_directories(
            stage.parent,
            absolute_work.parent,
            stage.name,
        )
    except BaseException:
        os.close(checkout_descriptor)
        raise
    work_descriptor: int | None = None
    object_descriptor: int | None = None
    artifact_descriptor: int | None = None
    journal_descriptor: int | None = None
    temporary_name: str | None = None
    temporary_descriptor: int | None = None
    replaced_stage_descriptor: int | None = None
    operation_error: BaseException | None = None
    results: list[BuildResult] = []
    manifested: list[SuiteTest] = []
    staged_executables: list[Path] = []
    compile_pass = compile_fail = link_pass = link_fail = 0
    try:
        journal_descriptor = _open_stage_journal(work_slot_descriptor)
        work_descriptor = _open_or_create_directory_unchecked(
            work_parent_descriptor,
            absolute_work.name,
            "work root",
        )
        object_descriptor = _open_and_reset_generated_directory(
            work_descriptor,
            "obj",
            "object root",
        )
        artifact_descriptor = _open_and_reset_generated_directory(
            work_descriptor,
            "bin",
            "artifact root",
        )
        temporary_descriptor = _activate_stage_work_slot(
            work_slot_descriptor,
            stage_parent_descriptor,
            stage.name,
            journal_descriptor=journal_descriptor,
        )
        temporary_name = _STAGE_WORK_ROOT_NAME
        _record_stage_transaction(
            work_slot_descriptor,
            "building",
            temporary_descriptor,
            journal_descriptor=journal_descriptor,
        )
        temporary_stage = Path(f"/proc/self/fd/{temporary_descriptor}")
        checkout_execution_root = Path(f"/proc/self/fd/{checkout_descriptor}")
        object_execution_root = Path(f"/proc/self/fd/{object_descriptor}")
        artifact_execution_root = Path(f"/proc/self/fd/{artifact_descriptor}")
        include_directory = checkout / "include"
        execution_include_directory = checkout_execution_root / "include"
        for test in ordered_tests:
            source = safe_stage_path(checkout, test.source)
            execution_source = safe_stage_path(
                checkout_execution_root,
                test.source,
            )
            object_path = safe_stage_path(object_root, f"{test.test_id}.o")
            execution_object_path = safe_stage_path(
                object_execution_root,
                f"{test.test_id}.o",
            )
            execution_object_path.parent.mkdir(parents=True, exist_ok=True)
            compile_result = _run_command(
                test.test_id,
                "compile",
                compile_command(
                    "aarch64-linux-gnu-gcc", source, object_path, include_directory
                ),
                command_runner,
                execution_object_path,
                execution_argv=compile_command(
                    "aarch64-linux-gnu-gcc",
                    execution_source,
                    execution_object_path,
                    execution_include_directory,
                ),
                diagnostic_path_replacements=(
                    (str(checkout_execution_root), str(checkout)),
                    (str(object_execution_root), str(object_root)),
                ),
                pass_fds=(checkout_descriptor, object_descriptor),
            )
            results.append(compile_result)
            if compile_result.returncode is None:
                raise ValueError(
                    "AArch64 compiler toolchain failed during compilation: "
                    f"{compile_result.stderr}"
                )
            if compile_result.status != "passed":
                compile_fail += 1
                disposition = (
                    test.disposition
                    if test.disposition == "excluded-upstream-stub"
                    else "compile-failed"
                )
                manifested.append(
                    replace(
                        test,
                        disposition=disposition,
                        binary="-",
                        sha256=EMPTY_SHA256,
                    )
                )
                continue
            compile_pass += 1
            if test.kind != "runnable":
                manifested.append(replace(test, binary="-", sha256=EMPTY_SHA256))
                continue
            nm_result = _run_command(
                test.test_id,
                "nm",
                nm_command("aarch64-linux-gnu-nm", object_path),
                command_runner,
                None,
                execution_argv=nm_command(
                    "aarch64-linux-gnu-nm",
                    execution_object_path,
                ),
                diagnostic_path_replacements=(
                    (str(object_execution_root), str(object_root)),
                ),
                pass_fds=(object_descriptor,),
            )
            results.append(nm_result)
            if nm_result.returncode != 0:
                raise ValueError(f"target nm failed for {test.test_id}: {nm_result.stderr}")
            if not _has_main(nm_result.stdout):
                link_fail += 1
                results.append(
                    BuildResult(
                        test_id=test.test_id,
                        stage="link",
                        status="failed",
                        argv=(),
                        returncode=None,
                        stdout="",
                        stderr="target object does not define main",
                        duration_ms=0,
                        artifact_sha256=None,
                    )
                )
                disposition = (
                    test.disposition
                    if test.disposition == "excluded-upstream-stub"
                    else "link-failed"
                )
                manifested.append(replace(test, disposition=disposition, binary="-", sha256=EMPTY_SHA256))
                continue
            executable = safe_stage_path(artifact_root, f"{test.test_id}.test")
            execution_executable = safe_stage_path(
                artifact_execution_root,
                f"{test.test_id}.test",
            )
            execution_executable.parent.mkdir(parents=True, exist_ok=True)
            link_result = _run_command(
                test.test_id,
                "link",
                link_command("aarch64-linux-gnu-gcc", object_path, executable),
                command_runner,
                execution_executable,
                execution_argv=link_command(
                    "aarch64-linux-gnu-gcc",
                    execution_object_path,
                    execution_executable,
                ),
                diagnostic_path_replacements=(
                    (str(object_execution_root), str(object_root)),
                    (str(artifact_execution_root), str(artifact_root)),
                ),
                pass_fds=(object_descriptor, artifact_descriptor),
            )
            results.append(link_result)
            if link_result.returncode is None:
                raise ValueError(
                    "AArch64 compiler toolchain failed during linking: "
                    f"{link_result.stderr}"
                )
            if link_result.status != "passed":
                link_fail += 1
                disposition = (
                    test.disposition
                    if test.disposition == "excluded-upstream-stub"
                    else "link-failed"
                )
                manifested.append(replace(test, disposition=disposition, binary="-", sha256=EMPTY_SHA256))
                continue
            link_pass += 1
            staged_relative = f"bin/{test.test_id}.test"
            staged_executable = safe_stage_path(temporary_stage, staged_relative)
            staged_executable.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(execution_executable, staged_executable)
            os.chmod(staged_executable, 0o755)
            staged_executables.append(staged_executable)
            if test.disposition == "excluded-upstream-stub":
                staged_executable.unlink()
                staged_executables.pop()
                manifested.append(replace(test, binary="-", sha256=EMPTY_SHA256))
            else:
                manifested.append(
                    replace(
                        test,
                        binary=staged_relative,
                        sha256=sha256_file(staged_executable),
                    )
                )
        manifested.extend(_shell_test(path) for path in ordered_shells)
        if dependency_stager is None:
            stage_runtime_dependencies(
                tuple(staged_executables),
                temporary_stage,
                stage_descriptor=temporary_descriptor,
            )
        else:
            dependency_stager(tuple(staged_executables), temporary_stage)
        support_files = stage_support_files(checkout, temporary_stage, ordered_tests)
        _write_manifests(
            temporary_stage,
            metadata,
            manifested,
            results,
            support_files,
        )
        staged_bytes = _stage_size(temporary_stage)
        _verify_open_stage(
            temporary_stage,
            temporary_descriptor,
            verify_architecture=dependency_stager is None,
            expected_metadata=metadata,
            expected_tests=ordered_tests,
            expected_shell_tests=ordered_shells,
        )
        replaced_stage_descriptor = _publish_stage(
            work_slot_descriptor,
            temporary_name,
            temporary_descriptor,
            stage_parent_descriptor,
            stage.name,
            journal_descriptor=journal_descriptor,
        )
        return BuildSummary(
            discovered=len(ordered_tests),
            compile_pass=compile_pass,
            compile_fail=compile_fail,
            link_pass=link_pass,
            link_fail=link_fail,
            shell_unported=len(ordered_shells),
            staged_bytes=staged_bytes,
        )
    except BaseException as error:
        operation_error = error
        raise
    finally:
        cleanup_error: BaseException | None = None
        try:
            if journal_descriptor is not None:
                _recover_stage_transaction(
                    work_slot_descriptor,
                    stage_parent_descriptor,
                    stage.name,
                    journal_descriptor=journal_descriptor,
                )
        except BaseException as error:
            cleanup_error = error
        for descriptor in (
            replaced_stage_descriptor,
            temporary_descriptor,
            artifact_descriptor,
            object_descriptor,
            work_descriptor,
            journal_descriptor,
            checkout_descriptor,
        ):
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except BaseException as error:
                    if cleanup_error is None:
                        cleanup_error = error
        for descriptor in reversed(campaign_lock_descriptors):
            try:
                os.close(descriptor)
            except BaseException as error:
                if cleanup_error is None:
                    cleanup_error = error
        if operation_error is None and cleanup_error is not None:
            raise cleanup_error


def _require_regular_file(path: Path, label: str) -> os.stat_result:
    try:
        info = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"missing {label}: {path}") from error
    if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
        raise ValueError(f"{label} is not a regular file: {path}")
    return info


def _regular_file_open_flags() -> int:
    flags = os.O_RDONLY
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _require_regular_file_at(
    parent_descriptor: int,
    name: str,
    label: str,
) -> os.stat_result:
    try:
        info = os.stat(name, dir_fd=parent_descriptor, follow_symlinks=False)
    except OSError as error:
        raise ValueError(f"missing {label}: {name}") from error
    if stat.S_ISLNK(info.st_mode):
        raise ValueError(f"{label} must not be a symlink: {name}")
    if not stat.S_ISREG(info.st_mode):
        raise ValueError(f"{label} is not a regular file: {name}")
    return info


def _open_regular_file_at(
    parent_descriptor: int,
    name: str,
    label: str,
    maximum_bytes: int,
) -> int:
    path_stat = _require_regular_file_at(parent_descriptor, name, label)
    if path_stat.st_size > maximum_bytes:
        raise ValueError(f"{label} size exceeds its limit")
    try:
        descriptor = os.open(
            name,
            _regular_file_open_flags(),
            dir_fd=parent_descriptor,
        )
    except OSError as error:
        raise ValueError(f"{label} could not be opened safely: {name}") from error
    opened_stat = os.fstat(descriptor)
    if not stat.S_ISREG(opened_stat.st_mode) or (
        opened_stat.st_dev,
        opened_stat.st_ino,
    ) != (path_stat.st_dev, path_stat.st_ino):
        os.close(descriptor)
        raise ValueError(f"{label} changed while being opened: {name}")
    if opened_stat.st_size > maximum_bytes:
        os.close(descriptor)
        raise ValueError(f"{label} size exceeds its limit")
    return descriptor


def _read_open_regular_file(
    descriptor: int,
    label: str,
    maximum_bytes: int,
) -> bytes:
    opened_stat = os.fstat(descriptor)
    if not stat.S_ISREG(opened_stat.st_mode) or opened_stat.st_size > maximum_bytes:
        raise ValueError(f"{label} size exceeds its limit")
    os.lseek(descriptor, 0, os.SEEK_SET)
    chunks: list[bytes] = []
    total = 0
    while True:
        chunk = os.read(descriptor, min(1024 * 1024, maximum_bytes + 1 - total))
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
        if total > maximum_bytes:
            raise ValueError(f"{label} size exceeds its limit")
    final_stat = os.fstat(descriptor)
    if (
        final_stat.st_dev,
        final_stat.st_ino,
        final_stat.st_mode,
        final_stat.st_size,
        final_stat.st_mtime_ns,
    ) != (
        opened_stat.st_dev,
        opened_stat.st_ino,
        opened_stat.st_mode,
        opened_stat.st_size,
        opened_stat.st_mtime_ns,
    ):
        raise ValueError(f"{label} changed while being read")
    return b"".join(chunks)


def _validate_stage_tree(stage: Path) -> None:
    for directory, directory_names, file_names in os.walk(stage, followlinks=False):
        directory_names.sort()
        file_names.sort()
        for name in directory_names:
            path = Path(directory) / name
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                raise ValueError(f"staged directory must not be a symlink: {path}")
            if not stat.S_ISDIR(info.st_mode):
                raise ValueError(f"invalid staged directory: {path}")
        for name in file_names:
            path = Path(directory) / name
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                raise ValueError(f"staged file must not be a symlink: {path}")
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f"invalid staged file: {path}")


def _run_readelf(
    readelf_runner: CommandRunner,
    argv: list[str],
    label: str,
    pass_fds: Sequence[int] = (),
) -> str:
    try:
        if readelf_runner is _SUBPROCESS_RUN:
            result = run_bounded_command(argv, pass_fds=pass_fds)
        else:
            result = readelf_runner(
                argv,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
                pass_fds=tuple(pass_fds),
            )
    except OSError as error:
        raise ValueError(f"AArch64 readelf unavailable while checking {label}: {error}") from error
    if int(getattr(result, "returncode")) != 0:
        raise ValueError(
            f"AArch64 readelf failed for {label}: "
            f"{_bounded_text(getattr(result, 'stderr', ''))}"
        )
    return str(getattr(result, "stdout", ""))


def _path_ends_with(value: str, suffix: str) -> bool:
    normalized = Path(value).as_posix()
    return normalized == suffix or normalized.endswith(f"/{suffix}")


def _validate_build_argv(
    test: SuiteTest,
    stage_name: str,
    status_name: str,
    argv: list[str],
    returncode: int | None,
    stderr: str,
    *,
    strict_paths: bool,
    revision: str | None,
) -> None:
    if any(_has_forbidden_character(argument) for argument in argv):
        raise ValueError(f"invalid control character in build argv for {test.test_id}")
    object_suffix = f"{test.test_id}.o"
    executable_suffix = f"{test.test_id}.test"
    if stage_name == "compile":
        prefix = [
            "aarch64-linux-gnu-gcc",
            "-std=gnu99",
            "-D_POSIX_C_SOURCE=200112L",
            "-D_XOPEN_SOURCE=600",
            "-pthread",
        ]
        legacy = (
            len(argv) == 11
            and argv[:5] == prefix
            and argv[5] == "-I"
            and Path(argv[6]).name == "include"
            and argv[7] == "-c"
            and _path_ends_with(argv[8], test.source)
            and argv[9] == "-o"
            and _path_ends_with(argv[10], object_suffix)
        )
        compatibility = (
            len(argv) == 13
            and argv[:5] == prefix
            and argv[5] == "-I"
            and Path(argv[6]).name == "include"
            and argv[7] == "-I"
            and Path(argv[8]).name == "include"
            and argv[9] == "-c"
            and _path_ends_with(argv[10], test.source)
            and argv[11] == "-o"
            and _path_ends_with(argv[12], object_suffix)
        )
        valid = legacy or compatibility
        if not valid:
            raise ValueError(f"invalid target compiler argv for {test.test_id}")
        if strict_paths:
            expected_source = f"target/posix/src/{revision}/{test.source}"
            expected_object = f"target/posix/aarch64/obj/{object_suffix}"
            expected_include = f"target/posix/src/{revision}/include"
            if legacy:
                actual_paths = [argv[6], argv[8], argv[10]]
                expected_paths = [expected_include, expected_source, expected_object]
            else:
                actual_paths = [argv[6], argv[8], argv[10], argv[12]]
                expected_paths = [
                    str(POSIX_COMPAT_INCLUDE_DIRECTORY),
                    expected_include,
                    expected_source,
                    expected_object,
                ]
            if actual_paths != expected_paths:
                raise ValueError(f"invalid production compiler path for {test.test_id}")
        return
    if stage_name == "nm":
        if not (
            len(argv) == 4
            and argv[:3]
            == ["aarch64-linux-gnu-nm", "-g", "--defined-only"]
            and _path_ends_with(argv[3], object_suffix)
        ):
            raise ValueError(f"invalid target nm argv for {test.test_id}")
        if strict_paths and argv[3] != f"target/posix/aarch64/obj/{object_suffix}":
            raise ValueError(f"invalid production nm path for {test.test_id}")
        return
    if not argv:
        if not (
            status_name == "failed"
            and returncode is None
            and stderr == "target object does not define main"
        ):
            raise ValueError(f"invalid synthetic link result for {test.test_id}")
        return
    valid = (
        len(argv) == 7
        and argv[0] == "aarch64-linux-gnu-gcc"
        and argv[1] == "-pthread"
        and _path_ends_with(argv[2], object_suffix)
        and argv[3] == "-o"
        and _path_ends_with(argv[4], executable_suffix)
        and argv[5:] == ["-lrt", "-lm"]
    )
    if not valid:
        raise ValueError(f"invalid target linker argv for {test.test_id}")
    if strict_paths and [argv[2], argv[4]] != [
        f"target/posix/aarch64/obj/{object_suffix}",
        f"target/posix/aarch64/bin/{executable_suffix}",
    ]:
        raise ValueError(f"invalid production linker path for {test.test_id}")


def _open_build_results_source(source: Path | int) -> BinaryIO:
    if isinstance(source, int):
        descriptor = os.dup(source)
        info = os.fstat(descriptor)
    else:
        path_stat = _require_regular_file(source, "build-results.ndjson")
        descriptor = os.open(source, _regular_file_open_flags())
        info = os.fstat(descriptor)
        if (info.st_dev, info.st_ino) != (path_stat.st_dev, path_stat.st_ino):
            os.close(descriptor)
            raise ValueError("build-results.ndjson changed while being opened")
    if not stat.S_ISREG(info.st_mode):
        os.close(descriptor)
        raise ValueError("build-results.ndjson is not a regular file")
    if info.st_size > MAX_BUILD_RESULTS_BYTES:
        os.close(descriptor)
        raise ValueError("build-results.ndjson size exceeds the 64 MiB limit")
    os.lseek(descriptor, 0, os.SEEK_SET)
    return os.fdopen(descriptor, "rb")


def _build_results_fingerprint(info: os.stat_result) -> tuple[int, ...]:
    return (
        info.st_dev,
        info.st_ino,
        info.st_mode,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )


def _copy_build_results_snapshot(
    source: BinaryIO,
    snapshot: BinaryIO,
) -> None:
    row_count = 0
    byte_count = 0
    while True:
        line = source.readline(MAX_BUILD_RESULT_LINE_BYTES + 1)
        if not line:
            break
        byte_count += len(line)
        if byte_count > MAX_BUILD_RESULTS_BYTES:
            raise ValueError("build-results.ndjson size exceeds the 64 MiB limit")
        if len(line) > MAX_BUILD_RESULT_LINE_BYTES:
            raise ValueError("build result line length exceeds the 256 KiB limit")
        if not line.endswith(b"\n") or b"\r" in line:
            raise ValueError("build results must use LF line endings")
        row_count += 1
        if row_count > MAX_BUILD_RESULTS_ROWS:
            raise ValueError("build result row count exceeds the 12,288 limit")
        snapshot.write(line)
    snapshot.flush()
    snapshot.seek(0)


def _build_result_lines(source: Path | int) -> Iterable[str]:
    with _open_build_results_source(source) as input_file:
        for line_number, line in enumerate(input_file, start=1):
            try:
                yield line[:-1].decode("utf-8")
            except UnicodeError as error:
                raise ValueError(
                    f"build result is not UTF-8 at line {line_number}"
                ) from error


def _parse_build_results(
    source: Path | int,
    tests: Sequence[SuiteTest],
    *,
    strict_paths: bool = False,
    revision: str | None = None,
) -> tuple[BuildResult, ...]:
    expected_fields = {
        "test_id",
        "stage",
        "status",
        "argv",
        "returncode",
        "stdout",
        "stderr",
        "duration_ms",
        "artifact_sha256",
    }
    tests_by_id = {test.test_id: test for test in tests}
    buildable_ids = {
        test.test_id for test in tests if test.kind != "shell"
    }
    results: list[BuildResult] = []
    seen: set[tuple[str, str]] = set()
    for line_number, line in enumerate(_build_result_lines(source), start=1):
        try:
            value = json.loads(
                line, object_pairs_hook=_reject_duplicate_json_keys
            )
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid build result JSON at line {line_number}") from error
        if not isinstance(value, dict) or set(value) != expected_fields:
            raise ValueError(f"invalid build result schema at line {line_number}")
        if line != json.dumps(
            value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
        ):
            raise ValueError(f"noncanonical build result JSON at line {line_number}")
        test_id = value["test_id"]
        stage_name = value["stage"]
        status_name = value["status"]
        argv = value["argv"]
        returncode = value["returncode"]
        stdout = value["stdout"]
        stderr = value["stderr"]
        duration_ms = value["duration_ms"]
        artifact_sha256 = value["artifact_sha256"]
        if not isinstance(test_id, str) or test_id not in buildable_ids:
            raise ValueError(f"unknown build result test ID at line {line_number}")
        if stage_name not in {"compile", "nm", "link"}:
            raise ValueError(f"invalid build stage at line {line_number}")
        if status_name not in {"passed", "failed"}:
            raise ValueError(f"invalid build status at line {line_number}")
        if not isinstance(argv, list) or not all(isinstance(item, str) for item in argv):
            raise ValueError(f"invalid build argv at line {line_number}")
        if returncode is not None and type(returncode) is not int:
            raise ValueError(f"invalid build return code at line {line_number}")
        if not isinstance(stdout, str) or not isinstance(stderr, str):
            raise ValueError(f"invalid build diagnostics at line {line_number}")
        if (
            len(stdout.encode("utf-8")) > MAX_DIAGNOSTIC_BYTES
            or len(stderr.encode("utf-8")) > MAX_DIAGNOSTIC_BYTES
        ):
            raise ValueError(f"oversized build diagnostics at line {line_number}")
        if type(duration_ms) is not int or duration_ms < 0:
            raise ValueError(f"invalid build duration at line {line_number}")
        if artifact_sha256 is not None and (
            not isinstance(artifact_sha256, str)
            or _DIGEST_RE.fullmatch(artifact_sha256) is None
        ):
            raise ValueError(f"invalid build artifact checksum at line {line_number}")
        if status_name == "passed" and returncode != 0:
            raise ValueError(f"passed build result has nonzero status at line {line_number}")
        if stage_name in {"compile", "link"}:
            if status_name == "passed" and artifact_sha256 is None:
                raise ValueError(f"passed build result lacks checksum at line {line_number}")
            if status_name == "failed" and artifact_sha256 is not None:
                raise ValueError(f"failed build result has checksum at line {line_number}")
        elif artifact_sha256 is not None:
            raise ValueError(f"nm build result has artifact checksum at line {line_number}")
        _validate_build_argv(
            tests_by_id[test_id],
            stage_name,
            status_name,
            argv,
            returncode,
            stderr,
            strict_paths=strict_paths,
            revision=revision,
        )
        identity = (test_id, stage_name)
        if identity in seen:
            raise ValueError(
                f"duplicate build result for {test_id} stage {stage_name}"
            )
        seen.add(identity)
        results.append(
            BuildResult(
                test_id=test_id,
                stage=stage_name,
                status=status_name,
                argv=tuple(argv),
                returncode=returncode,
                stdout=stdout,
                stderr=stderr,
                duration_ms=duration_ms,
                artifact_sha256=artifact_sha256,
            )
        )
    results_by_identity = {
        (result.test_id, result.stage): result for result in results
    }
    for test_id in sorted(buildable_ids):
        test = tests_by_id[test_id]
        compile_result = results_by_identity.get((test_id, "compile"))
        if compile_result is None:
            raise ValueError(f"missing build result for {test_id} stage compile")
        nm_result = results_by_identity.get((test_id, "nm"))
        link_result = results_by_identity.get((test_id, "link"))
        if compile_result.status == "failed":
            if test.disposition not in {
                "compile-failed",
                "excluded-upstream-stub",
            }:
                raise ValueError(
                    f"build result contradicts manifest for {test_id}: compile"
                )
            if nm_result is not None or link_result is not None:
                raise ValueError(f"unexpected post-compile build result for {test_id}")
            continue
        if test.kind == "definition":
            if nm_result is not None or link_result is not None:
                raise ValueError(
                    f"definition test has unexpected nm or link result: {test_id}"
                )
            if test.disposition not in {
                "definition-only",
                "excluded-upstream-stub",
            }:
                raise ValueError(
                    f"build result contradicts manifest for {test_id}: definition"
                )
            continue
        if nm_result is None or nm_result.status != "passed":
            raise ValueError(f"missing passed build result for {test_id} stage nm")
        if test.kind == "runnable":
            if link_result is None:
                raise ValueError(f"missing build result for {test_id} stage link")
            if test.disposition != "excluded-upstream-stub":
                if link_result.status == "passed":
                    if (
                        test.disposition != "complete"
                        or link_result.returncode != 0
                        or link_result.artifact_sha256 != test.sha256
                    ):
                        raise ValueError(
                            f"build result contradicts manifest for {test_id}: link"
                        )
                elif test.disposition != "link-failed":
                    raise ValueError(
                        f"build result contradicts manifest for {test_id}: link"
                    )
    return tuple(results)


def _load_build_results(
    source: Path | int,
    tests: Sequence[SuiteTest],
    *,
    strict_paths: bool = False,
    revision: str | None = None,
) -> tuple[BuildResult, ...]:
    with _open_build_results_source(source) as source_file, tempfile.TemporaryFile(
        "w+b"
    ) as snapshot:
        fingerprint = _build_results_fingerprint(os.fstat(source_file.fileno()))
        _copy_build_results_snapshot(source_file, snapshot)
        if _build_results_fingerprint(os.fstat(source_file.fileno())) != fingerprint:
            raise ValueError("build-results.ndjson changed while being verified")
        results = _parse_build_results(
            snapshot.fileno(),
            tests,
            strict_paths=strict_paths,
            revision=revision,
        )
        if _build_results_fingerprint(os.fstat(source_file.fileno())) != fingerprint:
            raise ValueError("build-results.ndjson changed while being verified")
        return results


def _manifest_inventory(
    tests: Sequence[SuiteTest],
) -> frozenset[tuple[str, str, str, str, str, int]]:
    return frozenset(
        (
            test.test_id,
            test.group,
            test.api,
            test.kind,
            test.disposition,
            test.timeout_ms,
        )
        for test in tests
    )


def _expected_manifest_inventory(
    tests: Sequence[SuiteTest],
    shell_tests: Sequence[str],
    build_results: Sequence[BuildResult],
) -> frozenset[tuple[str, str, str, str, str, int]]:
    results = {
        (result.test_id, result.stage): result for result in build_results
    }
    expected: list[SuiteTest] = []
    for test in tests:
        disposition = test.disposition
        compile_result = results.get((test.test_id, "compile"))
        if disposition != "excluded-upstream-stub":
            if compile_result is not None and compile_result.status == "failed":
                disposition = "compile-failed"
            elif test.kind == "definition":
                disposition = "definition-only"
            elif test.kind == "runnable":
                link_result = results.get((test.test_id, "link"))
                disposition = (
                    "link-failed"
                    if link_result is not None and link_result.status == "failed"
                    else "complete"
                )
        expected.append(replace(test, disposition=disposition))
    expected.extend(_shell_test(path) for path in shell_tests)
    inventory = _manifest_inventory(expected)
    if len(inventory) != len(expected):
        raise ValueError("duplicate test in expected inventory")
    return inventory


def _validate_support_inventory(raw_support: object) -> dict[str, str]:
    if raw_support is None:
        return {}
    if not isinstance(raw_support, list):
        raise ValueError("host support manifest is invalid")
    support: dict[str, str] = {}
    for entry in raw_support:
        if not isinstance(entry, Mapping) or set(entry) != {"path", "sha256"}:
            raise ValueError("host support manifest entry is invalid")
        relative = entry.get("path")
        digest = entry.get("sha256")
        if not isinstance(relative, str) or not isinstance(digest, str):
            raise ValueError("host support manifest entry is invalid")
        path = _validate_relative_path(relative, "support path")
        if relative != _FORK_MESSAGE_CATALOG_SUPPORT:
            raise ValueError(f"unsafe support path: {relative!r}")
        if relative in support:
            raise ValueError(f"duplicate support path: {relative}")
        if _DIGEST_RE.fullmatch(digest) is None:
            raise ValueError(f"invalid support checksum: {relative}")
        support[relative] = digest
    return support


def _verify_open_stage(
    stage: Path,
    stage_descriptor: int,
    *,
    readelf_runner: CommandRunner = subprocess.run,
    verify_architecture: bool = True,
    expected_metadata: ManifestMetadata | None = None,
    expected_tests: Sequence[SuiteTest] | None = None,
    expected_shell_tests: Sequence[str] | None = None,
    strict_command_paths: bool = False,
) -> BuildSummary:
    _validate_stage_tree(stage)
    _stage_size(stage)
    metadata_limits = (
        ("manifest.tsv", MAX_MANIFEST_BYTES, "2 MiB"),
        ("manifest.json", MAX_HOST_MANIFEST_BYTES, "8 MiB"),
        ("build-results.ndjson", MAX_BUILD_RESULTS_BYTES, "64 MiB"),
    )
    for name, maximum, display_limit in metadata_limits:
        info = _require_regular_file_at(stage_descriptor, name, name)
        if info.st_size > maximum:
            raise ValueError(f"{name} size exceeds the {display_limit} limit")
    manifest_descriptor = _open_regular_file_at(
        stage_descriptor,
        "manifest.tsv",
        "manifest.tsv",
        MAX_MANIFEST_BYTES,
    )
    try:
        manifest_data = _read_open_regular_file(
            manifest_descriptor,
            "manifest.tsv",
            MAX_MANIFEST_BYTES,
        )
    finally:
        os.close(manifest_descriptor)
    metadata, tests = parse_manifest(manifest_data)
    if expected_metadata is not None:
        expected_provenance = replace(
            expected_metadata,
            build_results_sha256=EMPTY_SHA256,
            manifest_sha256=EMPTY_SHA256,
        )
        actual_provenance = replace(
            metadata,
            build_results_sha256=EMPTY_SHA256,
            manifest_sha256=EMPTY_SHA256,
        )
        _validate_metadata(expected_provenance)
        if actual_provenance != expected_provenance:
            raise ValueError("manifest metadata does not match current build inputs")
    build_results_descriptor = _open_regular_file_at(
        stage_descriptor,
        "build-results.ndjson",
        "build-results.ndjson",
        MAX_BUILD_RESULTS_BYTES,
    )
    try:
        build_results = _load_build_results(
            build_results_descriptor,
            tests,
            strict_paths=strict_command_paths,
            revision=metadata.revision,
        )
    finally:
        os.close(build_results_descriptor)
    if _build_results_digest(build_results) != metadata.build_results_sha256:
        raise ValueError("build results checksum mismatch")
    if (expected_tests is None) != (expected_shell_tests is None):
        raise ValueError("complete expected inventory is required")
    if expected_tests is not None and expected_shell_tests is not None:
        expected_inventory = _expected_manifest_inventory(
            expected_tests,
            expected_shell_tests,
            build_results,
        )
        actual_inventory = _manifest_inventory(tests)
        if len(actual_inventory) != len(tests) or actual_inventory != expected_inventory:
            raise ValueError("manifest does not match current expected inventory")
    host_descriptor = _open_regular_file_at(
        stage_descriptor,
        "manifest.json",
        "manifest.json",
        MAX_HOST_MANIFEST_BYTES,
    )
    try:
        host_data = _read_open_regular_file(
            host_descriptor,
            "manifest.json",
            MAX_HOST_MANIFEST_BYTES,
        )
    finally:
        os.close(host_descriptor)
    try:
        host_text = host_data.decode("utf-8")
        host_manifest = json.loads(
            host_text, object_pairs_hook=_reject_duplicate_json_keys
        )
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("host manifest is invalid JSON") from error
    host_keys = set(host_manifest) if isinstance(host_manifest, Mapping) else set()
    required_host_keys = {"schema", "checksum_definition", "metadata", "runtime", "tests"}
    optional_host_keys = {"support"}
    if (
        not isinstance(host_manifest, Mapping)
        or not required_host_keys.issubset(host_keys)
        or not host_keys.issubset(required_host_keys | optional_host_keys)
        or host_manifest.get("schema") != 1
    ):
        raise ValueError("host manifest schema is invalid")
    canonical_host = (
        json.dumps(
            host_manifest,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
        )
        + "\n"
    )
    if host_text != canonical_host:
        raise ValueError("host manifest is not canonical JSON")
    if host_manifest.get("checksum_definition") != CHECKSUM_DEFINITION:
        raise ValueError("host manifest checksum definition is invalid")
    host_metadata = host_manifest.get("metadata")
    if host_metadata != asdict(metadata):
        raise ValueError("host manifest metadata differs from guest manifest")
    if host_manifest.get("tests") != [asdict(test) for test in tests]:
        raise ValueError("host manifest tests differ from guest manifest")
    support_manifest = _validate_support_inventory(host_manifest.get("support"))
    runtime_manifest = host_manifest.get("runtime")
    if not isinstance(runtime_manifest, list):
        raise ValueError("host runtime manifest is invalid")
    expected_runtime: dict[str, str] = {}
    for entry in runtime_manifest:
        if not isinstance(entry, Mapping) or set(entry) != {"path", "sha256"}:
            raise ValueError("host runtime manifest entry is invalid")
        relative = entry.get("path")
        digest = entry.get("sha256")
        if not isinstance(relative, str) or not isinstance(digest, str):
            raise ValueError("host runtime manifest entry is invalid")
        path = _validate_relative_path(relative, "runtime path")
        if len(path.parts) != 2 or path.parts[0] != "lib":
            raise ValueError(f"unsafe runtime path: {relative!r}")
        if relative in expected_runtime:
            raise ValueError(f"duplicate runtime path: {relative}")
        if _DIGEST_RE.fullmatch(digest) is None:
            raise ValueError(f"invalid runtime checksum: {relative}")
        expected_runtime[relative] = digest
    executable_count = 0
    elf_files: list[Path] = []
    expected_binaries: set[str] = set()
    for test in tests:
        if test.disposition != "complete":
            continue
        executable_count += 1
        assert test.binary is not None and test.sha256 is not None
        executable = safe_stage_path(stage, test.binary)
        expected_binaries.add(test.binary)
        executable_info = _require_regular_file(
            executable, f"binary for {test.test_id}"
        )
        if stat.S_IMODE(executable_info.st_mode) != 0o755:
            raise ValueError(f"invalid executable mode for {test.test_id}")
        if sha256_file(executable) != test.sha256:
            raise ValueError(f"binary checksum mismatch for {test.test_id}")
        if verify_architecture:
            header = _run_readelf(
                readelf_runner,
                ["aarch64-linux-gnu-readelf", "-h", str(executable)],
                test.binary,
                (stage_descriptor,),
            )
            if "AArch64" not in header:
                raise ValueError(f"staged binary is not AArch64 ELF: {test.binary}")
            elf_files.append(executable)
    actual_binaries = {
        path.relative_to(stage).as_posix()
        for path in (stage / "bin").rglob("*")
        if path.is_file()
    } if (stage / "bin").is_dir() else set()
    if actual_binaries != expected_binaries:
        missing = sorted(expected_binaries - actual_binaries)
        extra = sorted(actual_binaries - expected_binaries)
        raise ValueError(
            f"binary inventory mismatch (missing={missing}, extra={extra})"
        )
    runtime_files: dict[str, Path] = {}
    for library in sorted((stage / "lib").glob("*") if (stage / "lib").is_dir() else ()):
        library_info = _require_regular_file(library, "runtime library")
        if stat.S_IMODE(library_info.st_mode) != 0o755:
            raise ValueError(f"invalid runtime mode: {library.name}")
        runtime_files[library.name] = library
        if verify_architecture:
            header = _run_readelf(
                readelf_runner,
                ["aarch64-linux-gnu-readelf", "-h", str(library)],
                library.name,
                (stage_descriptor,),
            )
            if "AArch64" not in header:
                raise ValueError(f"staged runtime is not AArch64 ELF: {library.name}")
            elf_files.append(library)
    actual_runtime = {f"lib/{name}" for name in runtime_files}
    if actual_runtime != set(expected_runtime):
        missing = sorted(set(expected_runtime) - actual_runtime)
        extra = sorted(actual_runtime - set(expected_runtime))
        raise ValueError(
            f"runtime inventory mismatch (missing={missing}, extra={extra})"
        )
    for relative, digest in sorted(expected_runtime.items()):
        if sha256_file(safe_stage_path(stage, relative)) != digest:
            raise ValueError(f"runtime checksum mismatch: {relative}")
    for relative, digest in sorted(support_manifest.items()):
        support_file = safe_stage_path(stage, relative)
        support_info = _require_regular_file(support_file, f"support file {relative}")
        if stat.S_IMODE(support_info.st_mode) != 0o644:
            raise ValueError(f"invalid support mode: {relative}")
        if sha256_file(support_file) != digest:
            raise ValueError(f"support checksum mismatch: {relative}")
    expected_files = {
        "manifest.tsv",
        "manifest.json",
        "build-results.ndjson",
        *expected_binaries,
        *expected_runtime,
        *support_manifest,
    }
    actual_files = {
        path.relative_to(stage).as_posix()
        for path in stage.rglob("*")
        if path.is_file()
    }
    if actual_files != expected_files:
        missing = sorted(expected_files - actual_files)
        extra = sorted(actual_files - expected_files)
        raise ValueError(
            f"stage file inventory mismatch (missing={missing}, extra={extra})"
        )
    if verify_architecture:
        for elf_file in elf_files:
            dependencies = _run_readelf(
                readelf_runner,
                readelf_command("aarch64-linux-gnu-readelf", elf_file),
                elf_file.name,
                (stage_descriptor,),
            )
            interpreter, needed = parse_elf_dependencies(dependencies)
            required = set(needed)
            if interpreter is not None:
                required.add(PurePosixPath(interpreter).name)
            missing = sorted(required - set(runtime_files))
            if missing:
                raise ValueError(
                    f"missing runtime dependency for {elf_file.name}: {missing[0]}"
                )
    staged_bytes = _stage_size(stage)
    compile_results = tuple(
        result for result in build_results if result.stage == "compile"
    )
    link_results = tuple(result for result in build_results if result.stage == "link")
    return BuildSummary(
        discovered=sum(test.kind != "shell" for test in tests),
        compile_pass=(
            sum(result.status == "passed" for result in compile_results)
            if compile_results
            else sum(
                test.kind != "shell" and test.disposition != "compile-failed"
                for test in tests
            )
        ),
        compile_fail=(
            sum(result.status == "failed" for result in compile_results)
            if compile_results
            else sum(test.disposition == "compile-failed" for test in tests)
        ),
        link_pass=(
            sum(result.status == "passed" for result in link_results)
            if link_results
            else executable_count
        ),
        link_fail=(
            sum(result.status == "failed" for result in link_results)
            if link_results
            else sum(test.disposition == "link-failed" for test in tests)
        ),
        shell_unported=sum(test.disposition == "not-built-shell-test" for test in tests),
        staged_bytes=staged_bytes,
    )


def verify_stage(
    stage: Path,
    *,
    readelf_runner: CommandRunner = subprocess.run,
    verify_architecture: bool = True,
    expected_metadata: ManifestMetadata | None = None,
    expected_tests: Sequence[SuiteTest] | None = None,
    expected_shell_tests: Sequence[str] | None = None,
    strict_command_paths: bool = False,
) -> BuildSummary:
    stage = Path(os.path.abspath(stage))
    if not stage.name:
        raise ValueError("stage must have a directory name")
    parent_descriptor = _open_directory_chain(
        stage.parent, "stage parent", create=False
    )
    stage_descriptor: int | None = None
    try:
        stage_descriptor = _open_directory_at(
            parent_descriptor, stage.name, "stage"
        )
        return _verify_open_stage(
            Path(f"/proc/self/fd/{stage_descriptor}"),
            stage_descriptor,
            readelf_runner=readelf_runner,
            verify_architecture=verify_architecture,
            expected_metadata=expected_metadata,
            expected_tests=expected_tests,
            expected_shell_tests=expected_shell_tests,
            strict_command_paths=strict_command_paths,
        )
    finally:
        if stage_descriptor is not None:
            os.close(stage_descriptor)
        os.close(parent_descriptor)
