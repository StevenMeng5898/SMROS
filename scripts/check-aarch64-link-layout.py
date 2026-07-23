#!/usr/bin/env python3

import argparse
import struct
import sys
from pathlib import Path
from typing import NamedTuple


EXPECTED_ENTRY = 0x40200000
EM_AARCH64 = 183
ET_EXEC = 2
SHF_ALLOC = 0x2
SHT_STRTAB = 3
SHT_NOBITS = 8
SHN_LORESERVE = 0xFF00
REQUIRED_SECTIONS = (".text", ".rodata", ".data", ".bss", ".stack")

ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
SECTION_HEADER = struct.Struct("<IIQQQQIIQQ")


class LayoutError(ValueError):
    pass


class Section(NamedTuple):
    index: int
    name: str
    section_type: int
    flags: int
    address: int
    offset: int
    size: int

    @property
    def end(self) -> int:
        return self.address + self.size


class ElfLayout(NamedTuple):
    entry: int
    sections: tuple[Section, ...]


def checked_slice(data: bytes, offset: int, size: int, description: str) -> bytes:
    if offset > len(data) or size > len(data) - offset:
        raise LayoutError(f"{description} is outside the ELF file")
    return data[offset : offset + size]


def section_name(string_table: bytes, offset: int, index: int) -> str:
    if offset >= len(string_table):
        raise LayoutError(f"section {index} name offset is outside .shstrtab")
    end = string_table.find(b"\0", offset)
    if end < 0:
        raise LayoutError(f"section {index} name is not NUL-terminated")
    try:
        return string_table[offset:end].decode("utf-8")
    except UnicodeDecodeError as error:
        raise LayoutError(f"section {index} name is not UTF-8") from error


def parse_elf(data: bytes) -> ElfLayout:
    if len(data) < ELF_HEADER.size:
        raise LayoutError("ELF header is truncated")

    (
        ident,
        elf_type,
        machine,
        version,
        entry,
        _program_header_offset,
        section_table_offset,
        _flags,
        header_size,
        _program_header_size,
        _program_header_count,
        section_header_size,
        section_count,
        string_table_index,
    ) = ELF_HEADER.unpack_from(data)

    if ident[:4] != b"\x7fELF":
        raise LayoutError("invalid ELF magic")
    if ident[4] != 2 or ident[5] != 1 or ident[6] != 1:
        raise LayoutError("expected ELF64 little-endian version 1")
    if elf_type != ET_EXEC:
        raise LayoutError(f"expected executable ELF type, found {elf_type}")
    if machine != EM_AARCH64:
        raise LayoutError(f"expected AArch64 machine {EM_AARCH64}, found {machine}")
    if version != 1:
        raise LayoutError(f"unsupported ELF version {version}")
    if header_size != ELF_HEADER.size:
        raise LayoutError(f"unexpected ELF header size {header_size}")
    if section_header_size != SECTION_HEADER.size:
        raise LayoutError(f"unexpected section header size {section_header_size}")
    if section_count == 0:
        raise LayoutError("extended section counts are not supported")
    if section_count >= SHN_LORESERVE:
        raise LayoutError(f"reserved section count 0x{section_count:x} is not supported")
    if string_table_index >= SHN_LORESERVE:
        raise LayoutError(
            f"reserved string-table index 0x{string_table_index:x} is not supported"
        )
    if string_table_index >= section_count:
        raise LayoutError("invalid section-name string-table index")

    table_size = section_count * section_header_size
    section_table = checked_slice(
        data, section_table_offset, table_size, "section header table"
    )
    raw_sections = [
        SECTION_HEADER.unpack_from(section_table, index * section_header_size)
        for index in range(section_count)
    ]

    string_header = raw_sections[string_table_index]
    if string_header[1] != SHT_STRTAB:
        raise LayoutError("section-name table must have type SHT_STRTAB")
    string_table = checked_slice(
        data, string_header[4], string_header[5], "section-name string table"
    )

    sections = []
    for index, raw in enumerate(raw_sections):
        name_offset, section_type, flags, address, offset, size = raw[:6]
        if section_type != SHT_NOBITS and size:
            checked_slice(data, offset, size, f"section {index} contents")
        if address + size >= (1 << 64):
            raise LayoutError(f"section {index} address range overflows u64")
        sections.append(
            Section(
                index=index,
                name=section_name(string_table, name_offset, index),
                section_type=section_type,
                flags=flags,
                address=address,
                offset=offset,
                size=size,
            )
        )
    return ElfLayout(entry=entry, sections=tuple(sections))


def validate_elf(data: bytes) -> ElfLayout:
    layout = parse_elf(data)
    if layout.entry != EXPECTED_ENTRY:
        raise LayoutError(
            f"entry is 0x{layout.entry:x}, expected 0x{EXPECTED_ENTRY:x}"
        )

    by_name: dict[str, list[Section]] = {}
    for section in layout.sections:
        by_name.setdefault(section.name, []).append(section)

    required = []
    for name in REQUIRED_SECTIONS:
        matches = by_name.get(name, [])
        if len(matches) != 1:
            raise LayoutError(f"expected exactly one {name} section, found {len(matches)}")
        section = matches[0]
        if not section.flags & SHF_ALLOC or section.size == 0:
            raise LayoutError(f"{name} must be a nonempty allocatable section")
        required.append(section)

    text = required[0]
    if not text.address <= layout.entry < text.end:
        raise LayoutError("entry is outside .text")

    allocatable = sorted(
        (
            section
            for section in layout.sections
            if section.flags & SHF_ALLOC and section.size > 0
        ),
        key=lambda section: (section.address, section.index),
    )
    for previous, current in zip(allocatable, allocatable[1:]):
        if current.address < previous.end:
            raise LayoutError(
                f"allocatable sections {previous.name} and {current.name} overlap: "
                f"[0x{previous.address:x}, 0x{previous.end:x}) and "
                f"[0x{current.address:x}, 0x{current.end:x})"
            )

    for previous, current in zip(required, required[1:]):
        if current.address < previous.end:
            raise LayoutError(f"{current.name} must follow {previous.name} without overlap")
    if by_name[".stack"][0].address < by_name[".bss"][0].end:
        raise LayoutError(".stack must be after .bss")

    return layout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate the linked AArch64 kernel layout")
    parser.add_argument("elf", type=Path, help="linked AArch64 kernel ELF")
    args = parser.parse_args(argv)

    try:
        layout = validate_elf(args.elf.read_bytes())
    except (OSError, LayoutError) as error:
        print(f"error: {args.elf}: {error}", file=sys.stderr)
        return 1

    sections = {section.name: section for section in layout.sections}
    ranges = " ".join(
        f"{name}=[0x{sections[name].address:x},0x{sections[name].end:x})"
        for name in REQUIRED_SECTIONS
    )
    print(f"AArch64 layout OK: entry=0x{layout.entry:x} {ranges}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
