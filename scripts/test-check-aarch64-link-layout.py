#!/usr/bin/env python3

import importlib.util
import struct
import sys
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
MODULE_PATH = Path(__file__).with_name("check-aarch64-link-layout.py")
SPEC = importlib.util.spec_from_file_location("check_aarch64_link_layout", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)

ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
SECTION_HEADER = struct.Struct("<IIQQQQIIQQ")
SECTION_NAMES = ("", ".text", ".rodata", ".data", ".bss", ".stack", ".shstrtab")


def elf_fixture(
    *,
    entry: int = CHECKER.EXPECTED_ENTRY,
    stack_address: int = 0x40204000,
    stack_size: int = 0x80,
    section_order: tuple[str, ...] = SECTION_NAMES,
    section_count: int | None = None,
    string_table_index: int | None = None,
    string_table_type: int = 3,
) -> bytes:
    names = bytearray(b"\0")
    name_offsets = {"": 0}
    for name in SECTION_NAMES[1:]:
        name_offsets[name] = len(names)
        names.extend(name.encode("ascii"))
        names.append(0)

    actual_section_count = len(section_order)
    actual_string_table_index = section_order.index(".shstrtab")
    section_table_offset = ELF_HEADER.size
    names_offset = section_table_offset + actual_section_count * SECTION_HEADER.size
    text_offset = names_offset + len(names)
    rodata_offset = text_offset + 0x100
    data_offset = rodata_offset + 0x80
    file_size = data_offset + 0x40
    data = bytearray(file_size)

    ident = b"\x7fELF" + bytes((2, 1, 1, 0)) + bytes(8)
    ELF_HEADER.pack_into(
        data,
        0,
        ident,
        2,
        183,
        1,
        entry,
        0,
        section_table_offset,
        0,
        ELF_HEADER.size,
        0,
        0,
        SECTION_HEADER.size,
        actual_section_count if section_count is None else section_count,
        actual_string_table_index if string_table_index is None else string_table_index,
    )

    sections = {
        "": (0, 0, 0, 0, 0),
        ".text": (1, 0x6, 0x40200000, text_offset, 0x100),
        ".rodata": (1, 0x2, 0x40201000, rodata_offset, 0x80),
        ".data": (1, 0x3, 0x40202000, data_offset, 0x40),
        ".bss": (8, 0x3, 0x40203000, file_size, 0x100),
        ".stack": (8, 0x3, stack_address, file_size, stack_size),
        ".shstrtab": (string_table_type, 0, 0, names_offset, len(names)),
    }
    for index, name in enumerate(section_order):
        section_type, flags, address, offset, size = sections[name]
        SECTION_HEADER.pack_into(
            data,
            section_table_offset + index * SECTION_HEADER.size,
            name_offsets[name],
            section_type,
            flags,
            address,
            offset,
            size,
            0,
            0,
            1,
            0,
        )
    data[names_offset : names_offset + len(names)] = names
    return bytes(data)


class Aarch64LinkLayoutTests(unittest.TestCase):
    def test_accepts_expected_monotonic_layout(self) -> None:
        CHECKER.validate_elf(elf_fixture())

    def test_rejects_stack_overlapping_bss(self) -> None:
        with self.assertRaisesRegex(CHECKER.LayoutError, "overlap"):
            CHECKER.validate_elf(elf_fixture(stack_address=0x40203080))

    def test_rejects_changed_entry_address(self) -> None:
        with self.assertRaisesRegex(CHECKER.LayoutError, "entry"):
            CHECKER.validate_elf(elf_fixture(entry=0x40000000))

    def test_rejects_address_range_ending_at_two_to_the_64th(self) -> None:
        with self.assertRaisesRegex(CHECKER.LayoutError, "overflows u64"):
            CHECKER.validate_elf(elf_fixture(stack_address=(1 << 64) - 1, stack_size=1))

    def test_rejects_reserved_section_count_encodings(self) -> None:
        for value in (0xFF00, 0xFFFE, 0xFFFF):
            with self.subTest(value=value):
                with self.assertRaisesRegex(CHECKER.LayoutError, "reserved section count"):
                    CHECKER.validate_elf(elf_fixture(section_count=value))

    def test_rejects_reserved_string_table_index_encodings(self) -> None:
        for value in (0xFF00, 0xFFFE, 0xFFFF):
            with self.subTest(value=value):
                with self.assertRaisesRegex(CHECKER.LayoutError, "reserved string-table index"):
                    CHECKER.validate_elf(elf_fixture(string_table_index=value))

    def test_accepts_reordered_disjoint_allocatable_section_headers(self) -> None:
        CHECKER.validate_elf(
            elf_fixture(
                section_order=(
                    "",
                    ".rodata",
                    ".text",
                    ".data",
                    ".bss",
                    ".stack",
                    ".shstrtab",
                )
            )
        )

    def test_rejects_non_string_section_name_table(self) -> None:
        with self.assertRaisesRegex(CHECKER.LayoutError, "SHT_STRTAB"):
            CHECKER.validate_elf(elf_fixture(string_table_type=1))


if __name__ == "__main__":
    unittest.main()
