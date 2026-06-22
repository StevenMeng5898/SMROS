#!/usr/bin/env python3
"""Sync persisted SMROS /shared overlay files back to host_shared/."""

from __future__ import annotations

import argparse
import struct
import sys
from dataclasses import dataclass
from pathlib import Path


FXFS_ROOT_OBJECT_ID = 1
FXFS_MAX_OBJECTS = 8192
FXFS_MAX_DIRENTS = 8192
FXFS_MAX_JOURNAL_RECORDS = 1024
FXFS_BLOCK_MAGIC = 0x5346_5846
FXFS_BLOCK_VERSION = 1
FXFS_BLOCK_HEADER_LEN = 56
FXFS_BLOCK_SLOT_COUNT = 2
FXFS_MIN_SLOT_BYTES = 64 * 1024
FXFS_BLOCK_SIZE = 512
FXFS_NODE_DIR = 1
FXFS_NODE_FILE = 2
FXFS_JOURNAL_RECORD_BYTES = 33
LEGACY_SHARED_NAMES = {
    "trace.pftrace": ("trace.json",),
}
LEGACY_SHARED_FILES = {
    "trace.json",
}

HEADER = struct.Struct("<IHHIIQQQIIII")
OBJECT = struct.Struct("<QBIIIQQQQII")
DIRENT = struct.Struct("<QQH")


@dataclass
class FxfsObject:
    object_id: int
    kind: int
    data: bytes


@dataclass
class LoadedImage:
    sequence: int
    objects: dict[int, FxfsObject]
    children: dict[int, list[tuple[str, int]]]


def rotate_left_u32(value: int, bits: int) -> int:
    return ((value << bits) | (value >> (32 - bits))) & 0xFFFF_FFFF


def fxfs_checksum(data: bytes) -> int:
    checksum = 0
    for byte in data:
        checksum = (rotate_left_u32(checksum, 5) + byte) & 0xFFFF_FFFF
    return checksum


def align_down(value: int, align: int) -> int:
    return value - (value % align) if align else value


def fxfs_storage_layout(disk_size: int) -> list[tuple[int, int]]:
    if disk_size < FXFS_BLOCK_HEADER_LEN:
        return []
    usable = disk_size - FXFS_BLOCK_SIZE if disk_size > FXFS_BLOCK_SIZE else disk_size
    if usable >= FXFS_MIN_SLOT_BYTES * FXFS_BLOCK_SLOT_COUNT:
        slot_size = align_down(usable // FXFS_BLOCK_SLOT_COUNT, FXFS_BLOCK_SIZE)
        if slot_size >= FXFS_BLOCK_HEADER_LEN:
            return [(slot * slot_size, slot_size) for slot in range(FXFS_BLOCK_SLOT_COUNT)]
    return [(0, usable)]


def read_slot(disk: bytes, offset: int, slot_size: int) -> LoadedImage | None:
    if offset + FXFS_BLOCK_HEADER_LEN > len(disk):
        return None
    header = disk[offset : offset + FXFS_BLOCK_HEADER_LEN]
    if all(byte == 0 for byte in header):
        return None

    (
        magic,
        version,
        header_len,
        total_len,
        checksum,
        _next_object_id,
        sequence,
        _replayed_records,
        object_count,
        dirent_count,
        journal_count,
        _reserved,
    ) = HEADER.unpack(header)

    if magic != FXFS_BLOCK_MAGIC or version != FXFS_BLOCK_VERSION:
        raise ValueError("bad FxFS magic/version")
    if header_len != FXFS_BLOCK_HEADER_LEN:
        raise ValueError("bad FxFS header length")
    if total_len < header_len or total_len > slot_size or offset + total_len > len(disk):
        raise ValueError("bad FxFS image length")
    if (
        object_count == 0
        or object_count > FXFS_MAX_OBJECTS
        or dirent_count > FXFS_MAX_DIRENTS
        or journal_count > FXFS_MAX_JOURNAL_RECORDS
    ):
        raise ValueError("bad FxFS object counts")

    body = disk[offset + header_len : offset + total_len]
    if fxfs_checksum(body) != checksum:
        raise ValueError("bad FxFS checksum")

    pos = 0
    objects: dict[int, FxfsObject] = {}
    for _ in range(object_count):
        if pos + OBJECT.size > len(body):
            raise ValueError("truncated FxFS object table")
        (
            object_id,
            kind,
            _mode,
            _uid,
            _gid,
            size,
            _created_at,
            _modified_at,
            _accessed_at,
            _link_count,
            data_len,
        ) = OBJECT.unpack_from(body, pos)
        pos += OBJECT.size
        if kind not in {FXFS_NODE_DIR, FXFS_NODE_FILE}:
            raise ValueError("bad FxFS node kind")
        if size != data_len or pos + data_len > len(body):
            raise ValueError("bad FxFS file length")
        data = body[pos : pos + data_len]
        pos += data_len
        objects[object_id] = FxfsObject(object_id, kind, data)

    if FXFS_ROOT_OBJECT_ID not in objects:
        raise ValueError("FxFS root object missing")

    children: dict[int, list[tuple[str, int]]] = {}
    for _ in range(dirent_count):
        if pos + DIRENT.size > len(body):
            raise ValueError("truncated FxFS directory table")
        parent_id, object_id, name_len = DIRENT.unpack_from(body, pos)
        pos += DIRENT.size
        if pos + name_len > len(body):
            raise ValueError("bad FxFS directory name length")
        name = body[pos : pos + name_len].decode("utf-8")
        pos += name_len
        children.setdefault(parent_id, []).append((name, object_id))

    journal_len = journal_count * FXFS_JOURNAL_RECORD_BYTES
    if pos + journal_len != len(body):
        raise ValueError("bad FxFS journal length")

    return LoadedImage(sequence=sequence, objects=objects, children=children)


def load_latest_image(disk_path: Path) -> LoadedImage:
    disk = disk_path.read_bytes()
    candidates: list[LoadedImage] = []
    errors: list[str] = []
    for offset, slot_size in fxfs_storage_layout(len(disk)):
        try:
            image = read_slot(disk, offset, slot_size)
        except ValueError as exc:
            errors.append(f"slot@{offset}: {exc}")
            continue
        if image is not None:
            candidates.append(image)

    if not candidates:
        detail = "; ".join(errors) if errors else "no initialized FxFS slots"
        raise ValueError(detail)
    return max(candidates, key=lambda image: image.sequence)


def safe_child(root: Path, relative: str) -> Path:
    for part in relative.split("/"):
        if not part or part in {".", ".."} or "/" in part or "\0" in part:
            raise ValueError(f"unsafe /shared path: {relative!r}")
    target = (root / relative).resolve()
    root_resolved = root.resolve()
    if target != root_resolved and root_resolved not in target.parents:
        raise ValueError(f"/shared path escapes host root: {relative!r}")
    return target


def shared_object_id(image: LoadedImage) -> int | None:
    for name, object_id in image.children.get(FXFS_ROOT_OBJECT_ID, []):
        if name == "shared":
            return object_id
    return None


def sync_shared_tree(image: LoadedImage, host_root: Path, quiet: bool) -> int:
    shared_id = shared_object_id(image)
    if shared_id is None:
        return 0

    host_root.mkdir(parents=True, exist_ok=True)
    synced = 0
    visited: set[int] = set()

    def walk(object_id: int, relative: str) -> None:
        nonlocal synced
        if object_id in visited:
            raise ValueError("cycle in FxFS directory tree")
        visited.add(object_id)

        obj = image.objects.get(object_id)
        if obj is None:
            raise ValueError("FxFS directory entry points to missing object")

        if obj.kind == FXFS_NODE_FILE:
            if relative in LEGACY_SHARED_FILES:
                remove_host_file(host_root, relative, quiet)
                if not quiet:
                    print(f"skipped stale /shared/{relative}")
                return
            target = safe_child(host_root, relative)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(obj.data)
            remove_legacy_shared_files(host_root, relative, quiet)
            synced += 1
            if not quiet:
                print(f"synced /shared/{relative} -> {target}")
            return

        if obj.kind != FXFS_NODE_DIR:
            raise ValueError("bad FxFS object kind")

        if relative:
            safe_child(host_root, relative).mkdir(parents=True, exist_ok=True)
        for name, child_id in image.children.get(object_id, []):
            if not name or name in {".", ".."} or "/" in name or "\0" in name:
                raise ValueError(f"unsafe FxFS directory name: {name!r}")
            child_relative = f"{relative}/{name}" if relative else name
            walk(child_id, child_relative)

        visited.remove(object_id)

    walk(shared_id, "")
    return synced


def remove_legacy_shared_files(host_root: Path, relative: str, quiet: bool) -> None:
    dirname = str(Path(relative).parent)
    basename = Path(relative).name
    if dirname == ".":
        dirname = ""
    for legacy_name in LEGACY_SHARED_NAMES.get(basename, ()):
        legacy_relative = f"{dirname}/{legacy_name}" if dirname else legacy_name
        legacy_path = safe_child(host_root, legacy_relative)
        if remove_host_path(legacy_path) and not quiet:
            print(f"removed stale /shared/{legacy_relative} -> {legacy_path}")


def remove_host_file(host_root: Path, relative: str, quiet: bool) -> None:
    target = safe_child(host_root, relative)
    if remove_host_path(target) and not quiet:
        print(f"removed stale /shared/{relative} -> {target}")


def remove_host_path(path: Path) -> bool:
    try:
        path.unlink()
        return True
    except FileNotFoundError:
        return False


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Sync persisted SMROS /shared overlay files from smros-fxfs.img to host_shared/."
    )
    parser.add_argument("disk", nargs="?", default="smros-fxfs.img")
    parser.add_argument("host_shared", nargs="?", default="host_shared")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    disk_path = Path(args.disk)
    host_root = Path(args.host_shared)
    if not disk_path.exists():
        if not args.quiet:
            print(f"host_shared sync skipped: disk image not found: {disk_path}")
        return 0

    try:
        image = load_latest_image(disk_path)
        synced = sync_shared_tree(image, host_root, args.quiet)
    except Exception as exc:
        print(f"host_shared sync failed: {exc}", file=sys.stderr)
        return 1

    if not args.quiet:
        noun = "file" if synced == 1 else "files"
        print(f"host_shared sync complete: {synced} persisted /shared {noun}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
