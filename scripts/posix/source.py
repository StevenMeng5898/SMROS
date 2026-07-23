"""Pinned-source loading and checkout management for the POSIX suite."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse


COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
_LOCK_FIELDS = frozenset({"schema", "url", "revision", "license", "standard"})


@dataclass(frozen=True)
class SourceLock:
    schema: int
    url: str
    revision: str
    license: str
    standard: str

    def __post_init__(self) -> None:
        if type(self.schema) is not int or self.schema != 1:
            raise ValueError("source lock schema must be 1")
        if not isinstance(self.url, str):
            raise ValueError("source lock URL must be an HTTPS URL")
        parsed_url = urlparse(self.url)
        if parsed_url.scheme != "https" or not parsed_url.netloc:
            raise ValueError("source lock URL must be an HTTPS URL")
        if not isinstance(self.revision, str) or COMMIT_RE.fullmatch(
            self.revision
        ) is None:
            raise ValueError("source lock revision must be a full lowercase commit")
        if self.license != "GPL-2.0-only":
            raise ValueError("source lock license must be GPL-2.0-only")
        if not isinstance(self.standard, str):
            raise ValueError("source lock standard must be a string")


def load_source_lock(path: Path) -> SourceLock:
    """Load and strictly validate a source lock from JSON."""
    with path.open(encoding="utf-8") as lock_file:
        values = json.load(lock_file)
    if not isinstance(values, dict):
        raise ValueError("source lock must be a JSON object")
    if set(values) != _LOCK_FIELDS:
        missing = sorted(_LOCK_FIELDS - set(values))
        extra = sorted(set(values) - _LOCK_FIELDS)
        raise ValueError(
            f"source lock fields do not match schema (missing={missing}, extra={extra})"
        )
    return SourceLock(**values)


def validate_checkout(root: Path, revision: str) -> None:
    """Require the suite license and exact revision marker in a checkout."""
    copying = root / "COPYING"
    if not copying.is_file():
        raise ValueError(f"checkout is missing COPYING: {copying}")

    marker = root / ".smros-revision"
    if not marker.is_file():
        raise ValueError(f"checkout is missing revision marker: {marker}")
    marker_revision = marker.read_text(encoding="ascii")
    if marker_revision != f"{revision}\n":
        raise ValueError(f"checkout revision marker does not match {revision}")


def _load_patch_paths(patch_series: Path) -> tuple[Path, ...]:
    patch_names = []
    for line in patch_series.read_text(encoding="utf-8").splitlines():
        name = line.strip()
        if not name or name.startswith("#"):
            continue
        raw_parts = name.split("/")
        candidate = PurePosixPath(name)
        if candidate.is_absolute() or any(
            part in {"", ".", ".."} for part in raw_parts
        ):
            raise ValueError(f"unsafe patch series entry: {name}")
        patch_names.append(patch_series.parent / candidate)
    return tuple(patch_names)


def _write_revision_marker(root: Path, revision: str) -> None:
    marker = root / ".smros-revision"
    marker.unlink(missing_ok=True)
    descriptor = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    with os.fdopen(descriptor, "w", encoding="ascii") as marker_file:
        marker_file.write(f"{revision}\n")


def fetch_checkout(lock: SourceLock, root: Path, patch_series: Path) -> None:
    """Fetch a detached pinned checkout and apply its reviewed patch series."""
    if root.exists():
        validate_checkout(root, lock.revision)
        return

    patch_paths = _load_patch_paths(patch_series)
    root.parent.mkdir(parents=True, exist_ok=True)
    clone_started = False
    try:
        clone_started = True
        subprocess.run(
            ["git", "clone", "--no-checkout", lock.url, str(root)], check=True
        )
        subprocess.run(
            [
                "git",
                "-C",
                str(root),
                "fetch",
                "--depth",
                "1",
                "origin",
                lock.revision,
            ],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(root), "checkout", "--detach", lock.revision],
            check=True,
        )
        for patch_path in patch_paths:
            subprocess.run(
                ["git", "-C", str(root), "apply", str(patch_path)], check=True
            )

        _write_revision_marker(root, lock.revision)
        validate_checkout(root, lock.revision)
    except BaseException:
        if clone_started and root.is_dir() and not root.is_symlink():
            shutil.rmtree(root)
        raise
