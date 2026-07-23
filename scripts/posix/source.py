"""Pinned-source loading and checkout management for the POSIX suite."""

from __future__ import annotations

import ctypes
import errno
import hashlib
import json
import os
import re
import secrets
import shutil
import stat
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse


COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
_DIGEST_RE = re.compile(r"^[0-9a-f]{64}$")
_TREE_OID_RE = re.compile(r"^[0-9a-f]{40}$")
_LOCK_FIELDS = frozenset({"schema", "url", "revision", "license", "standard"})
_METADATA_FIELDS = frozenset({"schema", "patch_sha256", "tree_oid"})
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
    tree_oid: str

    def __post_init__(self) -> None:
        if type(self.schema) is not int or self.schema != 2:
            raise ValueError("source metadata schema must be 2")
        if not isinstance(self.patch_sha256, str) or _DIGEST_RE.fullmatch(
            self.patch_sha256
        ) is None:
            raise ValueError("source metadata patch digest is invalid")
        if not isinstance(self.tree_oid, str) or _TREE_OID_RE.fullmatch(
            self.tree_oid
        ) is None:
            raise ValueError("source metadata tree OID is invalid")


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


def _run_index_git(
    root_descriptor: int,
    index_path: Path,
    arguments: list[str],
    input_data: bytes | None = None,
) -> bytes:
    environment = os.environ.copy()
    environment["GIT_INDEX_FILE"] = str(index_path)
    return _run_git_at(
        root_descriptor, arguments, environment=environment, input_data=input_data
    )


def _run_git_at(
    root_descriptor: int,
    arguments: list[str],
    environment: dict[str, str] | None = None,
    input_data: bytes | None = None,
) -> bytes:
    return subprocess.run(
        ["git", "-C", f"/proc/self/fd/{root_descriptor}", *arguments],
        check=True,
        env=environment,
        input=input_data,
        pass_fds=(root_descriptor,),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    ).stdout


def _write_index_tree(root_descriptor: int, index_path: Path) -> str:
    tree_oid = (
        _run_index_git(root_descriptor, index_path, ["write-tree"])
        .decode("ascii")
        .strip()
    )
    if _TREE_OID_RE.fullmatch(tree_oid) is None:
        raise ValueError(f"Git returned an invalid tree OID: {tree_oid!r}")
    return tree_oid


def _derive_tree_oids(
    root_descriptor: int, revision: str, patches: tuple[_Patch, ...]
) -> tuple[str, str]:
    temporary_root = Path(tempfile.mkdtemp(prefix="smros-posix-index."))
    operation_error: BaseException | None = None
    try:
        expected_index = temporary_root / "expected.index"
        _run_index_git(root_descriptor, expected_index, ["read-tree", revision])
        for patch in patches:
            _run_index_git(
                root_descriptor,
                expected_index,
                ["apply", "--cached", "--"],
                input_data=patch.data,
            )
        expected_tree = _write_index_tree(root_descriptor, expected_index)

        actual_index = temporary_root / "actual.index"
        _run_index_git(root_descriptor, actual_index, ["read-tree", revision])
        _run_index_git(
            root_descriptor,
            actual_index,
            [
                "add",
                "-A",
                "--",
                ".",
                f":(exclude){_REVISION_NAME}",
                f":(exclude){_METADATA_NAME}",
            ],
        )
        actual_tree = _write_index_tree(root_descriptor, actual_index)
        return expected_tree, actual_tree
    except BaseException as error:
        operation_error = error
        raise
    finally:
        try:
            shutil.rmtree(temporary_root)
        except BaseException:
            if operation_error is None:
                raise


def _derive_actual_tree_oid(root_descriptor: int, revision: str) -> str:
    temporary_root = Path(tempfile.mkdtemp(prefix="smros-posix-index."))
    operation_error: BaseException | None = None
    try:
        actual_index = temporary_root / "actual.index"
        _run_index_git(root_descriptor, actual_index, ["read-tree", revision])
        _run_index_git(
            root_descriptor,
            actual_index,
            [
                "add",
                "-A",
                "--",
                ".",
                f":(exclude){_REVISION_NAME}",
                f":(exclude){_METADATA_NAME}",
            ],
        )
        return _write_index_tree(root_descriptor, actual_index)
    except BaseException as error:
        operation_error = error
        raise
    finally:
        try:
            shutil.rmtree(temporary_root)
        except BaseException:
            if operation_error is None:
                raise


def _validate_checkout(
    root: Path, revision: str, patches: tuple[_Patch, ...] | None
) -> None:
    root_descriptor = _open_directory(root, "checkout root")
    try:
        _validate_checkout_descriptor(root_descriptor, root, revision, patches)
    finally:
        os.close(root_descriptor)


def _validate_checkout_descriptor(
    root_descriptor: int,
    root_display: Path,
    revision: str,
    patches: tuple[_Patch, ...] | None,
) -> None:
    _read_regular_file_at(root_descriptor, "COPYING", "checkout COPYING")

    marker = root_display / _REVISION_NAME
    marker_bytes, _ = _read_regular_file_at(
        root_descriptor, _REVISION_NAME, "checkout revision marker"
    )
    try:
        marker_revision = marker_bytes.decode("ascii")
    except UnicodeError as error:
        raise ValueError(f"checkout revision marker is not ASCII: {marker}") from error
    if marker_revision != f"{revision}\n":
        raise ValueError(f"checkout revision marker does not match {revision}")

    metadata = _load_source_metadata(root_display, root_descriptor)
    if patches is not None:
        patch_sha256 = _patch_sha256(patches)
        if metadata.patch_sha256 != patch_sha256:
            raise ValueError("checkout patch digest does not match current patch series")

    try:
        git_stat = os.stat(".git", dir_fd=root_descriptor, follow_symlinks=False)
    except OSError as error:
        raise ValueError(f"checkout is not a Git checkout: {root_display}") from error
    if stat.S_ISLNK(git_stat.st_mode) or not (
        stat.S_ISDIR(git_stat.st_mode) or stat.S_ISREG(git_stat.st_mode)
    ):
        raise ValueError(
            f"checkout has invalid Git metadata: {root_display / '.git'}"
        )

    try:
        head = _run_git_at(root_descriptor, ["rev-parse", "HEAD"])
        prefix = _run_git_at(root_descriptor, ["rev-parse", "--show-prefix"])
    except subprocess.CalledProcessError as error:
        raise ValueError(f"checkout is not a Git checkout: {root_display}") from error
    if head.decode("ascii").strip() != revision:
        raise ValueError(f"checkout Git HEAD does not match {revision}")
    if prefix.strip():
        raise ValueError(f"checkout Git top level does not match root: {root_display}")

    try:
        if patches is None:
            actual_tree = _derive_actual_tree_oid(root_descriptor, revision)
            if actual_tree != metadata.tree_oid:
                raise ValueError("checkout Git tree does not match recorded tree")
        else:
            expected_tree, actual_tree = _derive_tree_oids(
                root_descriptor, revision, patches
            )
            if metadata.tree_oid != expected_tree:
                raise ValueError("checkout metadata tree OID does not match expected tree")
            if actual_tree != expected_tree:
                raise ValueError("checkout Git tree does not match expected patched tree")
    except subprocess.CalledProcessError as error:
        raise ValueError("checkout Git tree could not be derived") from error


def validate_checkout(root: Path, revision: str) -> None:
    """Diagnose HEAD and recorded-tree integrity for a generated checkout.

    Callers needing provenance against the current patch series must use
    fetch_checkout, which independently derives the expected patched tree.
    """
    _validate_checkout(root, revision, patches=None)


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


def _write_regular_file_exclusively_at(
    parent_descriptor: int, name: str, data: bytes
) -> None:
    try:
        os.unlink(name, dir_fd=parent_descriptor)
    except FileNotFoundError:
        pass
    except OSError as error:
        raise ValueError(f"generated metadata path cannot be replaced: {name}") from error
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(name, flags, 0o644, dir_fd=parent_descriptor)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)


def _write_revision_marker(root_descriptor: int, revision: str) -> None:
    _write_regular_file_exclusively_at(
        root_descriptor, _REVISION_NAME, f"{revision}\n".encode("ascii")
    )


def _write_source_metadata(
    root_descriptor: int, patch_sha256: str, tree_oid: str
) -> None:
    values = {
        "schema": 2,
        "patch_sha256": patch_sha256,
        "tree_oid": tree_oid,
    }
    _write_regular_file_exclusively_at(
        root_descriptor,
        _METADATA_NAME,
        (json.dumps(values, indent=2, sort_keys=True) + "\n").encode("utf-8"),
    )


def _rename_no_replace(
    source_parent_descriptor: int,
    source_name: str,
    source_descriptor: int,
    destination_parent_descriptor: int,
    destination_name: str,
) -> None:
    try:
        source_stat = os.stat(
            source_name,
            dir_fd=source_parent_descriptor,
            follow_symlinks=False,
        )
    except OSError as error:
        raise ValueError("owned checkout source entry disappeared before publish") from error
    held_stat = os.fstat(source_descriptor)
    if not stat.S_ISDIR(source_stat.st_mode) or (
        source_stat.st_dev,
        source_stat.st_ino,
    ) != (held_stat.st_dev, held_stat.st_ino):
        raise ValueError("owned checkout source entry changed before publish")

    try:
        renameat2 = ctypes.CDLL(None, use_errno=True).renameat2
    except AttributeError as error:
        raise OSError(
            errno.ENOSYS,
            "atomic no-replace rename is unavailable",
            destination_name,
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
        source_parent_descriptor,
        os.fsencode(source_name),
        destination_parent_descriptor,
        os.fsencode(destination_name),
        1,
    )
    if result != 0:
        error_number = ctypes.get_errno()
        raise OSError(
            error_number,
            os.strerror(error_number),
            destination_name,
        )


def _entry_exists_at(parent_descriptor: int, name: str) -> bool:
    try:
        os.stat(name, dir_fd=parent_descriptor, follow_symlinks=False)
    except FileNotFoundError:
        return False
    return True


def _create_owned_temporary_directory(
    parent_descriptor: int, prefix: str
) -> tuple[str, int]:
    for _ in range(128):
        name = f".{prefix}.{secrets.token_hex(8)}"
        try:
            os.mkdir(name, 0o700, dir_fd=parent_descriptor)
        except FileExistsError:
            continue
        return name, _open_directory_at(
            parent_descriptor, name, "owned temporary directory"
        )
    raise FileExistsError("could not allocate a unique temporary checkout directory")


def _clear_directory(directory_descriptor: int) -> None:
    with os.scandir(directory_descriptor) as iterator:
        entries = list(iterator)
    for entry in entries:
        entry_stat = os.stat(
            entry.name, dir_fd=directory_descriptor, follow_symlinks=False
        )
        if stat.S_ISDIR(entry_stat.st_mode):
            child_descriptor = _open_directory_at(
                directory_descriptor, entry.name, "temporary checkout directory"
            )
            try:
                _clear_directory(child_descriptor)
            finally:
                os.close(child_descriptor)
            os.rmdir(entry.name, dir_fd=directory_descriptor)
        else:
            os.unlink(entry.name, dir_fd=directory_descriptor)


def _remove_owned_temporary_directory(
    parent_descriptor: int, name: str, directory_descriptor: int
) -> None:
    entry_stat = os.stat(name, dir_fd=parent_descriptor, follow_symlinks=False)
    held_stat = os.fstat(directory_descriptor)
    if not stat.S_ISDIR(entry_stat.st_mode) or (
        entry_stat.st_dev,
        entry_stat.st_ino,
    ) != (held_stat.st_dev, held_stat.st_ino):
        raise ValueError("owned temporary directory changed before cleanup")
    _clear_directory(directory_descriptor)
    os.rmdir(name, dir_fd=parent_descriptor)


def _validate_competing_checkout(
    parent_descriptor: int,
    name: str,
    root_display: Path,
    revision: str,
    patches: tuple[_Patch, ...],
) -> None:
    try:
        checkout_descriptor = _open_directory_at(
            parent_descriptor, name, "competing checkout"
        )
        try:
            _validate_checkout_descriptor(
                checkout_descriptor, root_display, revision, patches
            )
        finally:
            os.close(checkout_descriptor)
    except (OSError, ValueError) as error:
        raise ValueError(
            f"checkout destination appeared during fetch and is invalid: {root_display}"
        ) from error


def fetch_checkout(lock: SourceLock, root: Path, patch_series: Path) -> None:
    """Fetch a detached pinned checkout and apply its reviewed patch series."""
    patches = _load_patches(patch_series)
    patch_sha256 = _patch_sha256(patches)
    root = Path(os.path.abspath(root))
    root.parent.mkdir(parents=True, exist_ok=True)
    destination_parent_descriptor = _open_directory(
        root.parent, "checkout destination parent"
    )
    temporary_name: str | None = None
    temporary_descriptor: int | None = None
    checkout_descriptor: int | None = None
    operation_error: BaseException | None = None
    try:
        if _entry_exists_at(destination_parent_descriptor, root.name):
            checkout_descriptor = _open_directory_at(
                destination_parent_descriptor, root.name, "checkout root"
            )
            try:
                _validate_checkout_descriptor(
                    checkout_descriptor, root, lock.revision, patches
                )
            finally:
                os.close(checkout_descriptor)
                checkout_descriptor = None
            return

        temporary_name, temporary_descriptor = _create_owned_temporary_directory(
            destination_parent_descriptor, root.name
        )
        subprocess.run(
            [
                "git",
                "clone",
                "--no-checkout",
                "--",
                lock.url,
                f"/proc/self/fd/{temporary_descriptor}/checkout",
            ],
            check=True,
            pass_fds=(temporary_descriptor,),
        )
        checkout_descriptor = _open_directory_at(
            temporary_descriptor, "checkout", "owned checkout"
        )
        _run_git_at(
            checkout_descriptor,
            ["fetch", "--depth", "1", "origin", lock.revision],
        )
        _run_git_at(
            checkout_descriptor, ["checkout", "--detach", lock.revision]
        )
        for patch in patches:
            _run_git_at(
                checkout_descriptor, ["apply", "--"], input_data=patch.data
            )

        _write_revision_marker(checkout_descriptor, lock.revision)
        expected_tree, actual_tree = _derive_tree_oids(
            checkout_descriptor, lock.revision, patches
        )
        if actual_tree != expected_tree:
            raise ValueError("new checkout Git tree does not match expected patched tree")
        _write_source_metadata(checkout_descriptor, patch_sha256, expected_tree)
        _validate_checkout_descriptor(
            checkout_descriptor, root, lock.revision, patches
        )

        if _entry_exists_at(destination_parent_descriptor, root.name):
            _validate_competing_checkout(
                destination_parent_descriptor,
                root.name,
                root,
                lock.revision,
                patches,
            )
            return
        try:
            _rename_no_replace(
                temporary_descriptor,
                "checkout",
                checkout_descriptor,
                destination_parent_descriptor,
                root.name,
            )
        except FileExistsError:
            _validate_competing_checkout(
                destination_parent_descriptor,
                root.name,
                root,
                lock.revision,
                patches,
            )
    except BaseException as error:
        operation_error = error
        raise
    finally:
        cleanup_error: BaseException | None = None
        try:
            if checkout_descriptor is not None:
                os.close(checkout_descriptor)
        except BaseException as error:
            cleanup_error = error
        try:
            if temporary_name is not None and temporary_descriptor is not None:
                _remove_owned_temporary_directory(
                    destination_parent_descriptor,
                    temporary_name,
                    temporary_descriptor,
                )
        except BaseException as error:
            if cleanup_error is None:
                cleanup_error = error
        try:
            if temporary_descriptor is not None:
                os.close(temporary_descriptor)
        except BaseException as error:
            if cleanup_error is None:
                cleanup_error = error
        try:
            os.close(destination_parent_descriptor)
        except BaseException as error:
            if cleanup_error is None:
                cleanup_error = error
        if operation_error is None and cleanup_error is not None:
            raise cleanup_error
