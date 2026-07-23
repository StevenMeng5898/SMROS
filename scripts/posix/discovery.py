"""Deterministic POSIX suite discovery and human-review validation."""

from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass, replace
from pathlib import Path, PurePosixPath
from types import MappingProxyType
from typing import Iterable, Mapping

from .model import SuiteTest


STUB_DISPOSITIONS = frozenset({"exclude-stub", "runtime-path"})
SHELL_DISPOSITIONS = frozenset({"test", "generator", "helper"})
_ALL_DISPOSITIONS = STUB_DISPOSITIONS | SHELL_DISPOSITIONS
_REVIEW_HEADER = "path\tdisposition\treason"
_CANDIDATE_HEADER = "path\tevidence"
_PTS_UNTESTED_RE = re.compile(r"\bPTS_UNTESTED\b")
_MEMORY_APIS = frozenset(
    {
        "mmap",
        "munmap",
        "mlock",
        "mlockall",
        "munlock",
        "munlockall",
        "shm_open",
        "shm_unlink",
    }
)
_DEFINITION_GROUPS = {
    "aio_h": "aio",
    "pthread_h": "threads",
    "mqueue_h": "message-queues",
    "semaphore_h": "semaphores",
    "sched_h": "scheduling",
    "signal_h": "signals",
    "time_h": "time",
    "sys/mman_h": "memory",
    "sys/shm_h": "memory",
}


@dataclass(frozen=True)
class ReviewCandidate:
    path: str
    evidence: str


@dataclass(frozen=True)
class ReviewEntry:
    path: str
    disposition: str
    reason: str


@dataclass
class _ConditionalFrame:
    parent_active: bool
    known: bool
    branch_taken: bool
    active: bool


@dataclass(frozen=True)
class AppliedReviews:
    tests: tuple[SuiteTest, ...]
    stub_reviews: Mapping[str, ReviewEntry]
    shell_reviews: Mapping[str, ReviewEntry]


@dataclass(frozen=True)
class AuditResult:
    tests: tuple[SuiteTest, ...]
    shell_candidates: tuple[ReviewCandidate, ...]
    stub_reviews: Mapping[str, ReviewEntry]
    shell_reviews: Mapping[str, ReviewEntry]

    @property
    def c_sources(self) -> int:
        return len(self.tests)

    @property
    def shell_files(self) -> int:
        return len(self.shell_candidates)

    @property
    def stub_candidates(self) -> int:
        return len(self.stub_reviews)

    def count_stub(self, disposition: str) -> int:
        return sum(
            review.disposition == disposition for review in self.stub_reviews.values()
        )

    def count_shell(self, disposition: str) -> int:
        return sum(
            review.disposition == disposition for review in self.shell_reviews.values()
        )

    def format_counts(self) -> str:
        return (
            f"discovered-c={self.c_sources} shell={self.shell_files} "
            f"stub={self.stub_candidates} "
            f"excluded={self.count_stub('exclude-stub')} "
            f"runtime={self.count_stub('runtime-path')} "
            f"test={self.count_shell('test')} "
            f"generator={self.count_shell('generator')} "
            f"helper={self.count_shell('helper')}"
        )


def api_group(api: str) -> str:
    """Map a normalized API name to the approved feature group."""
    if api in _DEFINITION_GROUPS:
        return _DEFINITION_GROUPS[api]
    if api.startswith("pthread_"):
        return "threads"
    if api.startswith("mq_"):
        return "message-queues"
    if api.startswith("sem_"):
        return "semaphores"
    if api.startswith("aio_") or api == "lio_listio":
        return "aio"
    if api.startswith("sched_"):
        return "scheduling"
    if api.startswith("sig") or api in {"kill", "killpg", "raise", "signal"}:
        return "signals"
    if (
        api.startswith("clock")
        or api.startswith("timer_")
        or api in {"nanosleep", "time"}
    ):
        return "time"
    if api in _MEMORY_APIS:
        return "memory"
    return "base"


def _conformance_root(checkout: Path) -> Path:
    root = checkout / "conformance"
    if not root.is_dir():
        raise ValueError(f"conformance directory is missing: {root}")
    return root


def _is_buildable_c_source(path: Path) -> bool:
    name = path.name
    return (
        path.suffix == ".c"
        and bool(name)
        and "0" <= name[0] <= "9"
        and "-" in name
    )


def _source_identity(checkout: Path, path: Path) -> str:
    return path.relative_to(checkout).as_posix()


def _classify_source(relative_path: PurePosixPath) -> tuple[str, str]:
    parts = relative_path.parts
    if len(parts) < 4 or parts[0] != "conformance":
        raise ValueError(f"source is outside a recognized conformance tree: {relative_path}")
    if parts[1] == "interfaces":
        api = parts[2]
        kind = "definition" if "-buildonly" in parts[-1] else "runnable"
        return api, kind
    if parts[1] == "definitions":
        api_parts = parts[2:-1]
        if not api_parts:
            raise ValueError(f"definition source has no API directory: {relative_path}")
        return PurePosixPath(*api_parts).as_posix(), "definition"
    api = parts[1]
    kind = "definition" if "-buildonly" in parts[-1] else "runnable"
    return api, kind


def discover_tests(checkout: Path) -> tuple[SuiteTest, ...]:
    """Discover all upstream-buildable C sources under ``conformance``."""
    conformance = _conformance_root(checkout)
    paths = sorted(
        (path for path in conformance.rglob("*.c") if _is_buildable_c_source(path)),
        key=lambda path: _source_identity(checkout, path),
    )
    tests = []
    for path in paths:
        source = _source_identity(checkout, path)
        api, kind = _classify_source(PurePosixPath(source))
        tests.append(
            SuiteTest(
                test_id=source,
                group=api_group(api),
                api=api,
                kind=kind,
                disposition="definition-only" if kind == "definition" else "complete",
                source=source,
                binary=None,
                sha256=None,
                timeout_ms=30_000,
            )
        )
    return tuple(tests)


def discover_shell_files(checkout: Path) -> tuple[str, ...]:
    """Inventory every shell source below ``conformance``."""
    conformance = _conformance_root(checkout)
    return tuple(
        sorted(
            _source_identity(checkout, path)
            for path in conformance.rglob("*.sh")
            if path.is_file()
        )
    )


def _strip_c_comments_and_literals(source: str) -> str:
    output: list[str] = []
    index = 0
    state = "code"
    while index < len(source):
        character = source[index]
        following = source[index + 1] if index + 1 < len(source) else ""
        if state == "code":
            if character == "/" and following == "/":
                output.extend((" ", " "))
                index += 2
                state = "line-comment"
                continue
            if character == "/" and following == "*":
                output.extend((" ", " "))
                index += 2
                state = "block-comment"
                continue
            if character == '"':
                output.append(" ")
                index += 1
                state = "string"
                continue
            if character == "'":
                output.append(" ")
                index += 1
                state = "character"
                continue
            output.append(character)
            index += 1
            continue
        if state == "line-comment":
            output.append("\n" if character == "\n" else " ")
            index += 1
            if character == "\n":
                state = "code"
            continue
        if state == "block-comment":
            if character == "*" and following == "/":
                output.extend((" ", " "))
                index += 2
                state = "code"
            else:
                output.append("\n" if character == "\n" else " ")
                index += 1
            continue
        if character == "\\" and following:
            output.append(" ")
            output.append("\n" if following == "\n" else " ")
            index += 2
            continue
        if (state == "string" and character == '"') or (
            state == "character" and character == "'"
        ):
            output.append(" ")
            index += 1
            state = "code"
            continue
        output.append("\n" if character == "\n" else " ")
        index += 1
    return "".join(output)


def _clean_evidence(value: str) -> str:
    return " ".join(value.split())


def _pts_untested_evidence(path: Path) -> str | None:
    source = path.read_bytes().decode("utf-8", errors="replace")
    code_lines = _strip_c_comments_and_literals(source).splitlines()
    original_lines = source.splitlines()
    matches = []
    conditional_stack: list[_ConditionalFrame] = []
    for line_number, code_line in enumerate(code_lines, start=1):
        stripped = code_line.lstrip()
        directive = re.match(
            r"#\s*(if|ifdef|ifndef|elif|else|endif)\b(.*)", stripped
        )
        if directive is not None:
            operation, argument = directive.groups()
            parent_active = (
                conditional_stack[-1].active if conditional_stack else True
            )
            if operation in {"if", "ifdef", "ifndef"}:
                expression = argument.strip()
                known = operation == "if" and expression in {"0", "1"}
                taken = known and expression == "1"
                conditional_stack.append(
                    _ConditionalFrame(
                        parent_active=parent_active,
                        known=known,
                        branch_taken=taken,
                        active=parent_active and (taken if known else True),
                    )
                )
            elif operation == "elif" and conditional_stack:
                frame = conditional_stack[-1]
                expression = argument.strip()
                if frame.known and frame.branch_taken:
                    frame.active = False
                elif frame.known and expression in {"0", "1"}:
                    selected = not frame.branch_taken and expression == "1"
                    frame.active = frame.parent_active and selected
                    frame.branch_taken = frame.branch_taken or selected
                else:
                    frame.known = False
                    frame.active = frame.parent_active
            elif operation == "else" and conditional_stack:
                frame = conditional_stack[-1]
                if frame.known:
                    frame.active = frame.parent_active and not frame.branch_taken
                    frame.branch_taken = True
                else:
                    frame.active = frame.parent_active
            elif operation == "endif" and conditional_stack:
                conditional_stack.pop()
            continue
        if conditional_stack and not conditional_stack[-1].active:
            continue
        preprocessor_use = stripped.startswith("#")
        executable_macro = re.match(
            r"#\s*define\s+(?!PTS_UNTESTED\b)\w+(?:\([^)]*\))?.*\bPTS_UNTESTED\b",
            stripped,
        )
        if _PTS_UNTESTED_RE.search(code_line) and (
            not preprocessor_use or executable_macro is not None
        ):
            evidence = _clean_evidence(original_lines[line_number - 1])
            matches.append(f"line {line_number}: {evidence}")
    return "; ".join(matches) if matches else None


def discover_stub_candidates(
    checkout: Path, tests: Iterable[SuiteTest] | None = None
) -> tuple[ReviewCandidate, ...]:
    """Find C sources with executable references to ``PTS_UNTESTED``."""
    discovered = discover_tests(checkout) if tests is None else tuple(tests)
    candidates = []
    for test in sorted(discovered, key=lambda item: item.source):
        evidence = _pts_untested_evidence(checkout / PurePosixPath(test.source))
        if evidence is not None:
            candidates.append(ReviewCandidate(test.source, evidence))
    return tuple(candidates)


def _shell_evidence(path: Path) -> str:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except UnicodeError as error:
        raise ValueError(f"shell source is not UTF-8: {path}") from error
    meaningful = [
        _clean_evidence(line)
        for line in lines
        if line.strip() and not line.lstrip().startswith("#")
    ]
    summary = " | ".join(meaningful[:8])
    if len(summary) > 1000:
        summary = summary[:997] + "..."
    return summary or "empty/comment-only shell script"


def discover_shell_candidates(checkout: Path) -> tuple[ReviewCandidate, ...]:
    return tuple(
        ReviewCandidate(path, _shell_evidence(checkout / PurePosixPath(path)))
        for path in discover_shell_files(checkout)
    )


def _has_control(value: str) -> bool:
    return any(
        unicodedata.category(character) in {"Cc", "Zl", "Zp"}
        for character in value
    )


def _validate_review_path(path: str) -> None:
    candidate = PurePosixPath(path)
    raw_parts = path.split("/")
    if (
        not path
        or "\\" in path
        or candidate.is_absolute()
        or candidate.as_posix() != path
        or any(part in {"", ".", ".."} for part in raw_parts)
        or not candidate.parts
        or candidate.parts[0] != "conformance"
        or _has_control(path)
    ):
        raise ValueError(f"invalid review path: {path!r}")


def load_review(
    path: Path, allowed_dispositions: Iterable[str] | None = None
) -> Mapping[str, ReviewEntry]:
    """Load a strict, duplicate-free human review TSV."""
    try:
        data = path.read_bytes()
    except FileNotFoundError as error:
        raise ValueError(f"review file is missing: {path}") from error
    if b"\r" in data or (data and not data.endswith(b"\n")):
        raise ValueError(f"review file must use LF line endings: {path}")
    try:
        text = data.decode("utf-8")
    except UnicodeError as error:
        raise ValueError(f"review file is not UTF-8: {path}") from error
    lines = text.split("\n")
    if lines[-1] == "":
        lines.pop()
    if not lines or lines[0] != _REVIEW_HEADER:
        raise ValueError(f"review header must be {_REVIEW_HEADER!r}: {path}")
    allowed = frozenset(allowed_dispositions or _ALL_DISPOSITIONS)
    reviews: dict[str, ReviewEntry] = {}
    for line_number, line in enumerate(lines[1:], start=2):
        fields = line.split("\t")
        if len(fields) != 3:
            raise ValueError(f"review row {line_number} must have exactly three fields")
        review_path, disposition, reason = fields
        _validate_review_path(review_path)
        if review_path in reviews:
            raise ValueError(f"duplicate review path: {review_path}")
        if disposition not in allowed:
            raise ValueError(
                f"unknown review disposition {disposition!r} for {review_path}"
            )
        if not reason.strip():
            raise ValueError(f"empty review reason for {review_path}")
        if _has_control(disposition) or _has_control(reason):
            raise ValueError(f"control character in review row for {review_path}")
        reviews[review_path] = ReviewEntry(review_path, disposition, reason)
    return MappingProxyType(dict(sorted(reviews.items())))


def _candidate_paths(
    candidates: Iterable[ReviewCandidate], label: str
) -> tuple[str, ...]:
    paths = tuple(candidate.path for candidate in candidates)
    if len(set(paths)) != len(paths):
        raise ValueError(f"duplicate {label} candidate path")
    for path in paths:
        _validate_review_path(path)
    return tuple(sorted(paths))


def _validate_complete_review(
    candidates: Iterable[ReviewCandidate],
    reviews: Mapping[str, ReviewEntry],
    label: str,
    allowed_dispositions: frozenset[str],
) -> None:
    candidate_paths = set(_candidate_paths(candidates, label))
    review_paths = set(reviews)
    missing = sorted(candidate_paths - review_paths)
    if missing:
        raise ValueError(f"missing {label} review: {missing[0]}")
    stale = sorted(review_paths - candidate_paths)
    if stale:
        raise ValueError(f"stale {label} review: {stale[0]}")
    for path in sorted(review_paths):
        review = reviews[path]
        if review.path != path:
            raise ValueError(f"review mapping key does not match row path: {path}")
        if review.disposition not in allowed_dispositions:
            raise ValueError(
                f"unknown {label} disposition {review.disposition!r} for {path}"
            )
        if not review.reason.strip() or _has_control(review.reason):
            raise ValueError(f"empty or invalid {label} review reason for {path}")


def apply_reviews(
    tests: Iterable[SuiteTest],
    stub_candidates: Iterable[ReviewCandidate],
    stub_reviews: Mapping[str, ReviewEntry],
    shell_candidates: Iterable[ReviewCandidate],
    shell_reviews: Mapping[str, ReviewEntry],
) -> AppliedReviews:
    """Validate review completeness and associate reviewed dispositions."""
    ordered_tests = tuple(sorted(tests, key=lambda test: test.test_id))
    if len({test.test_id for test in ordered_tests}) != len(ordered_tests):
        raise ValueError("duplicate discovered test ID")
    _validate_complete_review(
        stub_candidates, stub_reviews, "stub", STUB_DISPOSITIONS
    )
    _validate_complete_review(
        shell_candidates, shell_reviews, "shell", SHELL_DISPOSITIONS
    )
    reviewed_tests = []
    for test in ordered_tests:
        review = stub_reviews.get(test.source)
        if review is not None and review.disposition == "exclude-stub":
            test = replace(test, disposition="excluded-upstream-stub")
        reviewed_tests.append(test)
    return AppliedReviews(
        tests=tuple(reviewed_tests),
        stub_reviews=MappingProxyType(dict(sorted(stub_reviews.items()))),
        shell_reviews=MappingProxyType(dict(sorted(shell_reviews.items()))),
    )


def _write_candidate_file(path: Path, candidates: Iterable[ReviewCandidate]) -> None:
    ordered = sorted(candidates, key=lambda candidate: candidate.path)
    if len({candidate.path for candidate in ordered}) != len(ordered):
        raise ValueError(f"duplicate candidate path for {path.name}")
    lines = [_CANDIDATE_HEADER]
    for candidate in ordered:
        _validate_review_path(candidate.path)
        if not candidate.evidence or "\t" in candidate.evidence or _has_control(
            candidate.evidence
        ):
            raise ValueError(f"invalid candidate evidence for {candidate.path}")
        lines.append(f"{candidate.path}\t{candidate.evidence}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")


def write_candidates(
    directory: Path,
    stub_candidates: Iterable[ReviewCandidate],
    shell_candidates: Iterable[ReviewCandidate],
) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    _write_candidate_file(directory / "stub-candidates.tsv", stub_candidates)
    _write_candidate_file(directory / "shell-candidates.tsv", shell_candidates)


def audit_reviews(
    checkout: Path,
    stub_review_path: Path,
    shell_review_path: Path,
    *,
    write_directory: Path | None = None,
) -> AuditResult:
    """Discover the suite and require exact committed human-review coverage."""
    tests = discover_tests(checkout)
    stub_candidates = discover_stub_candidates(checkout, tests)
    shell_candidates = discover_shell_candidates(checkout)
    if write_directory is not None:
        write_candidates(write_directory, stub_candidates, shell_candidates)
    stub_reviews = load_review(stub_review_path, STUB_DISPOSITIONS)
    shell_reviews = load_review(shell_review_path, SHELL_DISPOSITIONS)
    applied = apply_reviews(
        tests,
        stub_candidates,
        stub_reviews,
        shell_candidates,
        shell_reviews,
    )
    return AuditResult(
        tests=applied.tests,
        shell_candidates=shell_candidates,
        stub_reviews=applied.stub_reviews,
        shell_reviews=applied.shell_reviews,
    )
