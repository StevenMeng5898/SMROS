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


@dataclass(frozen=True)
class RunMetadata:
    run_id: str
    platform: str
    manifest_sha256: str
    build_id: str
    complete: bool
