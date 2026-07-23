"""Pinned-source loading and checkout management for the POSIX suite."""

from __future__ import annotations

import ctypes
import errno
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse


COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
_DIGEST_RE = re.compile(r"^[0-9a-f]{64}$")
_LOCK_FIELDS = frozenset({"schema", "url", "revision", "license", "standard"})
_METADATA_FIELDS = frozenset({"schema", "patch_sha256", "tree_sha256"})
_METADATA_NAME = ".smros-source.json"
_REVISION_NAME = ".smros-revision"


def _reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    values = {}
    for key, value in pairs:
        if key in values:
            raise ValueError(f"duplicate JSON key: {key}")
        values[key] = value
    return values


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


@dataclass(frozen=True)
class _Patch:
    name: str
    data: bytes


@dataclass(frozen=True)
class _SourceMetadata:
    schema: int
    patch_sha256: str
    tree_sha256: str

    def __post_init__(self) -> None:
        if type(self.schema) is not int or self.schema != 1:
            raise ValueError("source metadata schema must be 1")
        if not isinstance(self.patch_sha256, str) or _DIGEST_RE.fullmatch(
            self.patch_sha256
        ) is None:
            raise ValueError("source metadata patch digest is invalid")
        if not isinstance(self.tree_sha256, str) or _DIGEST_RE.fullmatch(
            self.tree_sha256
        ) is None:
            raise ValueError("source metadata tree digest is invalid")


def load_source_lock(path: Path) -> SourceLock:
    """Load and strictly validate a source lock from JSON."""
    with path.open(encoding="utf-8") as lock_file:
        values = json.load(lock_file, object_pairs_hook=_reject_duplicate_keys)
    if not isinstance(values, dict):
        raise ValueError("source lock must be a JSON object")
    if set(values) != _LOCK_FIELDS:
        missing = sorted(_LOCK_FIELDS - set(values))
        extra = sorted(set(values) - _LOCK_FIELDS)
        raise ValueError(
            f"source lock fields do not match schema (missing={missing}, extra={extra})"
        )
    return SourceLock(**values)


def _update_hash(digest: object, value: bytes) -> None:
    digest.update(len(value).to_bytes(8, byteorder="big"))
    digest.update(value)


def _patch_sha256(patches: tuple[_Patch, ...]) -> str:
    digest = hashlib.sha256()
    for patch in patches:
        _update_hash(digest, patch.name.encode("utf-8"))
        _update_hash(digest, patch.data)
    return digest.hexdigest()


def _tree_sha256(root: Path) -> str:
    root_descriptor = _open_directory(root, "checkout root")
    try:
        return _tree_sha256_from_descriptor(root_descriptor)
    finally:
        os.close(root_descriptor)


def _tree_sha256_from_descriptor(root_descriptor: int) -> str:
    digest = hashlib.sha256()
    _hash_tree_directory(digest, root_descriptor, ())
    return digest.hexdigest()


def _hash_tree_directory(
    digest: object, directory_descriptor: int, relative_parts: tuple[str, ...]
) -> None:
    try:
        with os.scandir(directory_descriptor) as iterator:
            entries = sorted(iterator, key=lambda entry: entry.name)
    except OSError as error:
        relative = "/".join(relative_parts) or "."
        raise ValueError(f"checkout tree cannot be traversed: {relative}") from error

    for entry in entries:
        entry_parts = (*relative_parts, entry.name)
        relative = "/".join(entry_parts)
        if not relative_parts and entry.name in {
            ".git",
            _METADATA_NAME,
            _REVISION_NAME,
        }:
            continue
        try:
            entry_stat = os.stat(
                entry.name, dir_fd=directory_descriptor, follow_symlinks=False
            )
        except OSError as error:
            raise ValueError(f"checkout tree entry cannot be inspected: {relative}") from error
        if stat.S_ISLNK(entry_stat.st_mode):
            raise ValueError(f"checkout tree symlink is not allowed: {relative}")
        if stat.S_ISDIR(entry_stat.st_mode):
            child_descriptor = _open_directory_at(
                directory_descriptor,
                entry.name,
                f"checkout tree directory {relative}",
            )
            try:
                _hash_tree_directory(digest, child_descriptor, entry_parts)
            finally:
                os.close(child_descriptor)
            continue
        data, file_stat = _read_regular_file_at(
            directory_descriptor,
            entry.name,
            f"checkout tree entry {relative}",
        )
        _update_hash(digest, relative.encode("utf-8"))
        _update_hash(digest, stat.S_IMODE(file_stat.st_mode).to_bytes(4, "big"))
        _update_hash(digest, data)


def _require_directory(path: Path, label: str) -> os.stat_result:
    try:
        path_stat = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"{label} is missing: {path}") from error
    if stat.S_ISLNK(path_stat.st_mode):
        raise ValueError(f"{label} must not be a symlink: {path}")
    if not stat.S_ISDIR(path_stat.st_mode):
        raise ValueError(f"{label} must be a directory: {path}")
    return path_stat


def _directory_open_flags() -> int:
    flags = os.O_RDONLY
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_DIRECTORY"):
        flags |= os.O_DIRECTORY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _open_directory(path: Path, label: str) -> int:
    path_stat = _require_directory(path, label)
    try:
        descriptor = os.open(path, _directory_open_flags())
    except OSError as error:
        raise ValueError(f"{label} could not be opened safely: {path}") from error
    opened_stat = os.fstat(descriptor)
    if not stat.S_ISDIR(opened_stat.st_mode) or (
        opened_stat.st_dev,
        opened_stat.st_ino,
    ) != (path_stat.st_dev, path_stat.st_ino):
        os.close(descriptor)
        raise ValueError(f"{label} changed while being opened: {path}")
    return descriptor


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


def _regular_file_open_flags() -> int:
    flags = os.O_RDONLY
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _read_regular_file_at(
    parent_descriptor: int, name: str, label: str
) -> tuple[bytes, os.stat_result]:
    try:
        path_stat = os.stat(
            name, dir_fd=parent_descriptor, follow_symlinks=False
        )
    except OSError as error:
        raise ValueError(f"{label} is missing: {name}") from error
    if stat.S_ISLNK(path_stat.st_mode):
        raise ValueError(f"{label} must not be a symlink: {name}")
    if not stat.S_ISREG(path_stat.st_mode):
        raise ValueError(f"{label} must be a regular file: {name}")

    try:
        descriptor = os.open(
            name, _regular_file_open_flags(), dir_fd=parent_descriptor
        )
    except OSError as error:
        raise ValueError(f"{label} could not be opened safely: {name}") from error
    try:
        opened_stat = os.fstat(descriptor)
        if not stat.S_ISREG(opened_stat.st_mode) or (
            opened_stat.st_dev,
            opened_stat.st_ino,
        ) != (path_stat.st_dev, path_stat.st_ino):
            raise ValueError(f"{label} changed while being opened: {name}")
        chunks = []
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            chunks.append(chunk)
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
            raise ValueError(f"{label} changed while being read: {name}")
        return b"".join(chunks), opened_stat
    finally:
        os.close(descriptor)


def _read_regular_file(path: Path, label: str) -> tuple[bytes, os.stat_result]:
    parent_descriptor = _open_directory(path.parent, f"parent of {label}")
    try:
        return _read_regular_file_at(parent_descriptor, path.name, label)
    finally:
        os.close(parent_descriptor)


def _load_source_metadata(
    root: Path, root_descriptor: int | None = None
) -> _SourceMetadata:
    metadata_path = root / _METADATA_NAME
    try:
        if root_descriptor is None:
            metadata_bytes, _ = _read_regular_file(
                metadata_path, "checkout source metadata"
            )
        else:
            metadata_bytes, _ = _read_regular_file_at(
                root_descriptor, _METADATA_NAME, "checkout source metadata"
            )
        values = json.loads(
            metadata_bytes.decode("utf-8"), object_pairs_hook=_reject_duplicate_keys
        )
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"checkout source metadata is invalid: {metadata_path}") from error
    if not isinstance(values, dict) or set(values) != _METADATA_FIELDS:
        raise ValueError("checkout source metadata fields do not match schema")
    return _SourceMetadata(**values)


def _validate_checkout(
    root: Path, revision: str, expected_patch_sha256: str | None
) -> None:
    root_descriptor = _open_directory(root, "checkout root")
    try:
        _read_regular_file_at(root_descriptor, "COPYING", "checkout COPYING")

        marker = root / _REVISION_NAME
        marker_bytes, _ = _read_regular_file_at(
            root_descriptor, _REVISION_NAME, "checkout revision marker"
        )
        try:
            marker_revision = marker_bytes.decode("ascii")
        except UnicodeError as error:
            raise ValueError(
                f"checkout revision marker is not ASCII: {marker}"
            ) from error
        if marker_revision != f"{revision}\n":
            raise ValueError(f"checkout revision marker does not match {revision}")

        metadata = _load_source_metadata(root, root_descriptor)
        if (
            expected_patch_sha256 is not None
            and metadata.patch_sha256 != expected_patch_sha256
        ):
            raise ValueError("checkout patch digest does not match current patch series")

        try:
            git_stat = os.stat(
                ".git", dir_fd=root_descriptor, follow_symlinks=False
            )
        except OSError as error:
            raise ValueError(f"checkout is not a Git checkout: {root}") from error
        if stat.S_ISLNK(git_stat.st_mode) or not (
            stat.S_ISDIR(git_stat.st_mode) or stat.S_ISREG(git_stat.st_mode)
        ):
            raise ValueError(f"checkout has invalid Git metadata: {root / '.git'}")

        try:
            completed = subprocess.run(
                ["git", "-C", str(root), "rev-parse", "HEAD"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
        except subprocess.CalledProcessError as error:
            raise ValueError(f"checkout is not a Git checkout: {root}") from error
        if completed.stdout.strip() != revision:
            raise ValueError(f"checkout Git HEAD does not match {revision}")

        try:
            top_level = subprocess.run(
                ["git", "-C", str(root), "rev-parse", "--show-toplevel"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            ).stdout.strip()
        except subprocess.CalledProcessError as error:
            raise ValueError(f"checkout is not a Git checkout: {root}") from error
        if Path(top_level).resolve() != root.resolve():
            raise ValueError(f"checkout Git top level does not match root: {root}")

        if _tree_sha256_from_descriptor(root_descriptor) != metadata.tree_sha256:
            raise ValueError("checkout tree digest does not match source metadata")
    finally:
        os.close(root_descriptor)


def validate_checkout(root: Path, revision: str) -> None:
    """Validate the pinned Git revision and generated source-tree metadata."""
    _validate_checkout(root, revision, expected_patch_sha256=None)


def _load_patches(patch_series: Path) -> tuple[_Patch, ...]:
    patch_directory_descriptor = _open_directory(
        patch_series.parent, "patch directory"
    )
    try:
        series_bytes, _ = _read_regular_file_at(
            patch_directory_descriptor, patch_series.name, "patch series"
        )
        try:
            series_text = series_bytes.decode("utf-8")
        except UnicodeError as error:
            raise ValueError(f"patch series is not UTF-8: {patch_series}") from error
        patches = []
        for line in series_text.splitlines():
            name = line.strip()
            if not name or name.startswith("#"):
                continue
            raw_parts = name.split("/")
            candidate = PurePosixPath(name)
            if candidate.is_absolute() or any(
                part in {"", ".", ".."} for part in raw_parts
            ):
                raise ValueError(f"unsafe patch series entry: {name}")

            parent_descriptor = os.dup(patch_directory_descriptor)
            try:
                for part in candidate.parts[:-1]:
                    child_descriptor = _open_directory_at(
                        parent_descriptor, part, f"patch parent for {name}"
                    )
                    os.close(parent_descriptor)
                    parent_descriptor = child_descriptor
                data, _ = _read_regular_file_at(
                    parent_descriptor, candidate.parts[-1], f"patch {name}"
                )
            finally:
                os.close(parent_descriptor)
            patches.append(_Patch(name=name, data=data))
        return tuple(patches)
    finally:
        os.close(patch_directory_descriptor)


def _write_regular_file_exclusively(path: Path, data: bytes) -> None:
    try:
        path.unlink(missing_ok=True)
    except OSError as error:
        raise ValueError(f"generated metadata path cannot be replaced: {path}") from error
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o644)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)


def _write_revision_marker(root: Path, revision: str) -> None:
    _write_regular_file_exclusively(
        root / _REVISION_NAME, f"{revision}\n".encode("ascii")
    )


def _write_source_metadata(
    root: Path, patch_sha256: str, tree_sha256: str
) -> None:
    metadata_path = root / _METADATA_NAME
    values = {
        "schema": 1,
        "patch_sha256": patch_sha256,
        "tree_sha256": tree_sha256,
    }
    _write_regular_file_exclusively(
        metadata_path,
        (json.dumps(values, indent=2, sort_keys=True) + "\n").encode("utf-8"),
    )


def _path_exists(path: Path) -> bool:
    return os.path.lexists(path)


def _rename_no_replace(source: Path, destination: Path) -> None:
    try:
        renameat2 = ctypes.CDLL(None, use_errno=True).renameat2
    except AttributeError as error:
        raise OSError(
            errno.ENOSYS, "atomic no-replace rename is unavailable", destination
        ) from error
    renameat2.argtypes = (
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    )
    renameat2.restype = ctypes.c_int
    result = renameat2(
        -100,
        os.fsencode(source),
        -100,
        os.fsencode(destination),
        1,
    )
    if result != 0:
        error_number = ctypes.get_errno()
        raise OSError(
            error_number,
            os.strerror(error_number),
            str(destination),
        )


def _validate_competing_checkout(
    root: Path, revision: str, patch_sha256: str
) -> None:
    try:
        _validate_checkout(root, revision, patch_sha256)
    except (OSError, ValueError) as error:
        raise ValueError(
            f"checkout destination appeared during fetch and is invalid: {root}"
        ) from error


def fetch_checkout(lock: SourceLock, root: Path, patch_series: Path) -> None:
    """Fetch a detached pinned checkout and apply its reviewed patch series."""
    patches = _load_patches(patch_series)
    patch_sha256 = _patch_sha256(patches)
    root = Path(os.path.abspath(root))
    if _path_exists(root):
        _validate_checkout(root, lock.revision, patch_sha256)
        return

    root.parent.mkdir(parents=True, exist_ok=True)
    temporary_root = Path(
        tempfile.mkdtemp(prefix=f".{root.name}.", dir=root.parent)
    )
    owned_checkout = temporary_root / "checkout"
    operation_error: BaseException | None = None
    try:
        subprocess.run(
            [
                "git",
                "clone",
                "--no-checkout",
                "--",
                lock.url,
                str(owned_checkout),
            ],
            check=True,
        )
        subprocess.run(
            [
                "git",
                "-C",
                str(owned_checkout),
                "fetch",
                "--depth",
                "1",
                "origin",
                lock.revision,
            ],
            check=True,
        )
        subprocess.run(
            [
                "git",
                "-C",
                str(owned_checkout),
                "checkout",
                "--detach",
                lock.revision,
            ],
            check=True,
        )
        for patch in patches:
            subprocess.run(
                ["git", "-C", str(owned_checkout), "apply", "--"],
                check=True,
                input=patch.data,
            )

        _write_revision_marker(owned_checkout, lock.revision)
        _write_source_metadata(
            owned_checkout, patch_sha256, _tree_sha256(owned_checkout)
        )
        _validate_checkout(owned_checkout, lock.revision, patch_sha256)

        if _path_exists(root):
            _validate_competing_checkout(root, lock.revision, patch_sha256)
            return
        try:
            _rename_no_replace(owned_checkout, root)
        except FileExistsError:
            _validate_competing_checkout(root, lock.revision, patch_sha256)
    except BaseException as error:
        operation_error = error
        raise
    finally:
        try:
            shutil.rmtree(temporary_root)
        except BaseException:
            if operation_error is None:
                raise
