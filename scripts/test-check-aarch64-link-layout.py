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


def elf_fixture(*, entry: int = CHECKER.EXPECTED_ENTRY, stack_address: int = 0x40204000) -> bytes:
    names = bytearray(b"\0")
    name_offsets = {"": 0}
    for name in SECTION_NAMES[1:]:
        name_offsets[name] = len(names)
        names.extend(name.encode("ascii"))
        names.append(0)

    section_count = len(SECTION_NAMES)
    section_table_offset = ELF_HEADER.size
    names_offset = section_table_offset + section_count * SECTION_HEADER.size
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
        section_count,
        section_count - 1,
    )

    sections = (
        ("", 0, 0, 0, 0, 0),
        (".text", 1, 0x6, 0x40200000, text_offset, 0x100),
        (".rodata", 1, 0x2, 0x40201000, rodata_offset, 0x80),
        (".data", 1, 0x3, 0x40202000, data_offset, 0x40),
        (".bss", 8, 0x3, 0x40203000, file_size, 0x100),
        (".stack", 8, 0x3, stack_address, file_size, 0x80),
        (".shstrtab", 3, 0, 0, names_offset, len(names)),
    )
    for index, (name, section_type, flags, address, offset, size) in enumerate(sections):
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


if __name__ == "__main__":
    unittest.main()
