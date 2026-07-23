"""Shared immutable records for POSIX suite host tooling."""

from __future__ import annotations

from dataclasses import dataclass


PTS_PASS = 0
PTS_FAIL = 1
PTS_UNRESOLVED = 2
PTS_UNSUPPORTED = 4
PTS_UNTESTED = 5


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


@dataclass(frozen=True)
class RuntimeAttempt:
    test_id: str
    platform: str
    status: str
    exit_code: int | None
    signal: int | None
    timed_out: bool
    duration_ms: int
    stdout: str
    stderr: str
    source: str
    launch_error: str | None = None
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

    def to_dict(self) -> dict[str, object]:
        """Return every core and finalized runtime field for persistence."""
        return {
            "binary_sha256": self.binary_sha256,
            "build_id": self.build_id,
            "build_results_sha256": self.build_results_sha256,
            "duration_ms": self.duration_ms,
            "exit_code": self.exit_code,
            "launch_error": self.launch_error,
            "manifest_sha256": self.manifest_sha256,
            "patch_sha256": self.patch_sha256,
            "platform": self.platform,
            "revision": self.revision,
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
