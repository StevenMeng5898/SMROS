//! Minimal ELF64/AArch64 image parser for boot userspace components.
//!
//! This loader intentionally handles only the first bring-up shape SMROS uses:
//! little-endian ELF64 images with a small program-header table, PT_LOAD
//! segments, and enough dynamic metadata to resolve PT_INTERP/DT_NEEDED from
//! the shell. It records load metadata for component processes; real copying
//! of segment bytes into per-process EL0 mappings is the next loader stage.

#![allow(dead_code)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::user_level::user_logic;

pub const ELF_HEADER_SIZE: usize = 64;
pub const ELF_PHDR_SIZE: usize = 56;
pub const ELF_MAX_PHDRS: usize = 16;
pub const ELF_MAX_LOAD_SEGMENTS: usize = 4;
pub const ELF_MACHINE_AARCH64: u16 = 183;

const ELF_CLASS_64: u8 = 2;
const ELF_DATA_LSB: u8 = 1;
const ELF_VERSION_CURRENT: u8 = 1;
pub const ELF_TYPE_EXEC: u16 = 2;
pub const ELF_TYPE_DYN: u16 = 3;
const ELF_PT_DYNAMIC: u32 = 2;
const ELF_PT_INTERP: u32 = 3;
const ELF_PT_LOAD: u32 = 1;
const ELF_PF_EXEC: u32 = 1;
const ELF_PF_READ: u32 = 4;
const ELF_BOOT_ALIGN: u64 = 4096;
const ELF_DYN_ENTRY_SIZE: usize = 16;
const ELF_DT_NULL: u64 = 0;
const ELF_DT_NEEDED: u64 = 1;
const ELF_DT_STRTAB: u64 = 5;
const ELF_DT_STRSZ: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    Storage,
    ShortHeader,
    BadMagic,
    UnsupportedClass,
    UnsupportedType,
    UnsupportedMachine,
    BadHeaderSize,
    BadProgramHeaderTable,
    BadSegment,
    TooManySegments,
    MissingLoadSegment,
    BadEntry,
}

impl ElfError {
    pub fn as_str(self) -> &'static str {
        match self {
            ElfError::Storage => "storage",
            ElfError::ShortHeader => "short-header",
            ElfError::BadMagic => "bad-magic",
            ElfError::UnsupportedClass => "unsupported-class",
            ElfError::UnsupportedType => "unsupported-type",
            ElfError::UnsupportedMachine => "unsupported-machine",
            ElfError::BadHeaderSize => "bad-header-size",
            ElfError::BadProgramHeaderTable => "bad-phdr-table",
            ElfError::BadSegment => "bad-segment",
            ElfError::TooManySegments => "too-many-segments",
            ElfError::MissingLoadSegment => "missing-load-segment",
            ElfError::BadEntry => "bad-entry",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ElfSegment {
    pub vaddr: u64,
    pub mem_size: u64,
    pub file_size: u64,
    pub file_offset: u64,
    pub flags: u32,
    pub align: u64,
}

#[derive(Clone, Debug)]
pub struct ElfImage {
    pub elf_type: u16,
    pub entry: u64,
    pub phoff: u64,
    pub phentsize: u16,
    pub phnum: u16,
    pub segments: Vec<ElfSegment>,
    pub interpreter: Option<String>,
    pub needed: Vec<String>,
    pub dynamic: bool,
}

fn read_u16_le(image: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    if end > image.len() {
        return None;
    }
    Some(u16::from_le_bytes([image[offset], image[offset + 1]]))
}

fn read_u32_le(image: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    if end > image.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        image[offset],
        image[offset + 1],
        image[offset + 2],
        image[offset + 3],
    ]))
}

fn read_u64_le(image: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    if end > image.len() {
        return None;
    }
    Some(u64::from_le_bytes([
        image[offset],
        image[offset + 1],
        image[offset + 2],
        image[offset + 3],
        image[offset + 4],
        image[offset + 5],
        image[offset + 6],
        image[offset + 7],
    ]))
}

fn read_c_string(image: &[u8], offset: usize, max_len: usize) -> Result<String, ElfError> {
    let end = offset.checked_add(max_len).ok_or(ElfError::BadSegment)?;
    if end > image.len() {
        return Err(ElfError::BadSegment);
    }

    let mut string_end = offset;
    while string_end < end && image[string_end] != 0 {
        string_end += 1;
    }
    core::str::from_utf8(&image[offset..string_end])
        .map(|value| value.to_string())
        .map_err(|_| ElfError::BadSegment)
}

fn file_offset_for_vaddr(segments: &[ElfSegment], vaddr: u64) -> Option<usize> {
    for segment in segments {
        let segment_end = segment.vaddr.checked_add(segment.file_size)?;
        if vaddr >= segment.vaddr && vaddr < segment_end {
            let delta = vaddr.checked_sub(segment.vaddr)?;
            let file_offset = segment.file_offset.checked_add(delta)?;
            if file_offset <= usize::MAX as u64 {
                return Some(file_offset as usize);
            }
        }
    }
    None
}

fn parse_needed(
    image: &[u8],
    segments: &[ElfSegment],
    dynamic_offset: usize,
    dynamic_size: usize,
) -> Result<Vec<String>, ElfError> {
    let dynamic_end = dynamic_offset
        .checked_add(dynamic_size)
        .ok_or(ElfError::BadSegment)?;
    if dynamic_end > image.len() {
        return Err(ElfError::BadSegment);
    }

    let mut strtab_vaddr = None;
    let mut strtab_size = None;
    let mut needed_offsets = Vec::new();
    let mut cursor = dynamic_offset;

    while cursor
        .checked_add(ELF_DYN_ENTRY_SIZE)
        .map(|end| end <= dynamic_end)
        .unwrap_or(false)
    {
        let tag = read_u64_le(image, cursor).ok_or(ElfError::BadSegment)?;
        let value = read_u64_le(image, cursor + 8).ok_or(ElfError::BadSegment)?;
        if tag == ELF_DT_NULL {
            break;
        }
        if tag == ELF_DT_NEEDED {
            needed_offsets.push(value);
        } else if tag == ELF_DT_STRTAB {
            strtab_vaddr = Some(value);
        } else if tag == ELF_DT_STRSZ {
            strtab_size = Some(value);
        }
        cursor += ELF_DYN_ENTRY_SIZE;
    }

    if needed_offsets.is_empty() {
        return Ok(Vec::new());
    }

    let strtab_vaddr = strtab_vaddr.ok_or(ElfError::BadSegment)?;
    let strtab_size = strtab_size.ok_or(ElfError::BadSegment)?;
    if strtab_size > usize::MAX as u64 {
        return Err(ElfError::BadSegment);
    }
    let strtab_file_offset =
        file_offset_for_vaddr(segments, strtab_vaddr).ok_or(ElfError::BadSegment)?;
    let strtab_size = strtab_size as usize;
    let strtab_end = strtab_file_offset
        .checked_add(strtab_size)
        .ok_or(ElfError::BadSegment)?;
    if strtab_end > image.len() {
        return Err(ElfError::BadSegment);
    }

    let mut needed = Vec::new();
    for name_offset in needed_offsets {
        if name_offset > usize::MAX as u64 {
            return Err(ElfError::BadSegment);
        }
        let name_offset = name_offset as usize;
        if name_offset >= strtab_size {
            return Err(ElfError::BadSegment);
        }
        let file_offset = strtab_file_offset
            .checked_add(name_offset)
            .ok_or(ElfError::BadSegment)?;
        let max_len = strtab_size - name_offset;
        needed.push(read_c_string(image, file_offset, max_len)?);
    }

    Ok(needed)
}

fn write_u16_le(image: &mut [u8], offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    image[offset..offset + 2].copy_from_slice(&bytes);
}

fn write_u32_le(image: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    image[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_u64_le(image: &mut [u8], offset: usize, value: u64) {
    let bytes = value.to_le_bytes();
    image[offset..offset + 8].copy_from_slice(&bytes);
}

pub fn parse(image: &[u8]) -> Result<ElfImage, ElfError> {
    if !user_logic::elf_header_bounds_valid(image.len()) {
        return Err(ElfError::ShortHeader);
    }
    if !user_logic::elf_magic_valid(image[0], image[1], image[2], image[3]) {
        return Err(ElfError::BadMagic);
    }
    if !user_logic::elf_class_data_valid(image[4], image[5], image[6]) {
        return Err(ElfError::UnsupportedClass);
    }

    let elf_type = read_u16_le(image, 16).ok_or(ElfError::ShortHeader)?;
    if !user_logic::elf_type_valid(elf_type) {
        return Err(ElfError::UnsupportedType);
    }

    let machine = read_u16_le(image, 18).ok_or(ElfError::ShortHeader)?;
    if !user_logic::elf_machine_valid(machine) {
        return Err(ElfError::UnsupportedMachine);
    }

    let entry = read_u64_le(image, 24).ok_or(ElfError::ShortHeader)?;
    if !user_logic::elf_entry_valid(entry) {
        return Err(ElfError::BadEntry);
    }

    let phoff_u64 = read_u64_le(image, 32).ok_or(ElfError::ShortHeader)?;
    if phoff_u64 > usize::MAX as u64 {
        return Err(ElfError::BadProgramHeaderTable);
    }
    let phoff = phoff_u64 as usize;

    let ehsize = read_u16_le(image, 52).ok_or(ElfError::ShortHeader)? as usize;
    if ehsize != ELF_HEADER_SIZE {
        return Err(ElfError::BadHeaderSize);
    }

    let phentsize = read_u16_le(image, 54).ok_or(ElfError::ShortHeader)? as usize;
    let phnum = read_u16_le(image, 56).ok_or(ElfError::ShortHeader)? as usize;
    if !user_logic::elf_phdr_table_valid(phoff, phentsize, phnum, image.len()) {
        return Err(ElfError::BadProgramHeaderTable);
    }

    let mut segments = Vec::new();
    let mut interpreter = None;
    let mut dynamic_table = None;
    for index in 0..phnum {
        let Some(header_offset) = phentsize
            .checked_mul(index)
            .and_then(|offset| phoff.checked_add(offset))
        else {
            return Err(ElfError::BadProgramHeaderTable);
        };

        let p_type = read_u32_le(image, header_offset).ok_or(ElfError::BadProgramHeaderTable)?;
        let flags = read_u32_le(image, header_offset + 4).ok_or(ElfError::BadProgramHeaderTable)?;
        let file_offset =
            read_u64_le(image, header_offset + 8).ok_or(ElfError::BadProgramHeaderTable)?;
        let vaddr =
            read_u64_le(image, header_offset + 16).ok_or(ElfError::BadProgramHeaderTable)?;
        let file_size =
            read_u64_le(image, header_offset + 32).ok_or(ElfError::BadProgramHeaderTable)?;
        let mem_size =
            read_u64_le(image, header_offset + 40).ok_or(ElfError::BadProgramHeaderTable)?;
        let align =
            read_u64_le(image, header_offset + 48).ok_or(ElfError::BadProgramHeaderTable)?;

        if p_type == ELF_PT_INTERP {
            if file_offset > usize::MAX as u64
                || file_size > usize::MAX as u64
                || !user_logic::elf_segment_bounds_valid(
                    file_offset as usize,
                    file_size as usize,
                    file_size as usize,
                    image.len(),
                )
            {
                return Err(ElfError::BadSegment);
            }
            interpreter = Some(read_c_string(
                image,
                file_offset as usize,
                file_size as usize,
            )?);
            continue;
        }

        if p_type == ELF_PT_DYNAMIC {
            if file_offset > usize::MAX as u64
                || file_size > usize::MAX as u64
                || !user_logic::elf_segment_bounds_valid(
                    file_offset as usize,
                    file_size as usize,
                    file_size as usize,
                    image.len(),
                )
            {
                return Err(ElfError::BadSegment);
            }
            dynamic_table = Some((file_offset as usize, file_size as usize));
            continue;
        }

        if p_type != ELF_PT_LOAD {
            continue;
        }
        if segments.len() >= ELF_MAX_LOAD_SEGMENTS {
            return Err(ElfError::TooManySegments);
        }

        if file_offset > usize::MAX as u64
            || file_size > usize::MAX as u64
            || mem_size > usize::MAX as u64
            || !user_logic::elf_segment_bounds_valid(
                file_offset as usize,
                file_size as usize,
                mem_size as usize,
                image.len(),
            )
            || !user_logic::elf_vaddr_range_valid(vaddr, mem_size)
        {
            return Err(ElfError::BadSegment);
        }

        segments.push(ElfSegment {
            vaddr,
            mem_size,
            file_size,
            file_offset,
            flags,
            align,
        });
    }

    if segments.is_empty() {
        return Err(ElfError::MissingLoadSegment);
    }

    let needed = match dynamic_table {
        Some((offset, size)) => parse_needed(image, &segments, offset, size)?,
        None => Vec::new(),
    };
    let dynamic = interpreter.is_some() || !needed.is_empty();

    Ok(ElfImage {
        elf_type,
        entry,
        phoff: phoff_u64,
        phentsize: phentsize as u16,
        phnum: phnum as u16,
        segments,
        interpreter,
        needed,
        dynamic,
    })
}

pub fn load_from_fxfs(path: &str) -> Result<ElfImage, ElfError> {
    let attrs = crate::user_level::fxfs::attrs(path).map_err(|_| ElfError::Storage)?;
    if !user_logic::fxfs_file_size_valid(attrs.size) {
        return Err(ElfError::Storage);
    }
    let mut image = Vec::new();
    image.resize(attrs.size, 0);
    let len =
        crate::user_level::fxfs::read_file(path, &mut image).map_err(|_| ElfError::Storage)?;
    image.truncate(len);
    parse(&image)
}

/// Build a tiny valid ELF image whose entry is the current SMROS EL0 trampoline.
///
/// The PT_LOAD range describes the image bytes for loader metadata. It is not
/// a real userspace text mapping yet; component launch remains on the trusted
/// trampoline until segment copying and TTBR0-backed execution are implemented.
pub fn build_trampoline_elf(entry: u64) -> Vec<u8> {
    let size = ELF_HEADER_SIZE + ELF_PHDR_SIZE;
    let mut image = Vec::new();
    image.resize(size, 0);

    image[0] = 0x7f;
    image[1] = 0x45;
    image[2] = 0x4c;
    image[3] = 0x46;
    image[4] = ELF_CLASS_64;
    image[5] = ELF_DATA_LSB;
    image[6] = ELF_VERSION_CURRENT;
    image[7] = 0;
    image[16] = 2;

    write_u16_le(&mut image, 16, ELF_TYPE_EXEC);
    write_u16_le(&mut image, 18, ELF_MACHINE_AARCH64);
    write_u32_le(&mut image, 20, ELF_VERSION_CURRENT as u32);
    write_u64_le(&mut image, 24, entry);
    write_u64_le(&mut image, 32, ELF_HEADER_SIZE as u64);
    write_u64_le(&mut image, 40, 0);
    write_u32_le(&mut image, 48, 0);
    write_u16_le(&mut image, 52, ELF_HEADER_SIZE as u16);
    write_u16_le(&mut image, 54, ELF_PHDR_SIZE as u16);
    write_u16_le(&mut image, 56, 1);
    write_u16_le(&mut image, 58, 0);
    write_u16_le(&mut image, 60, 0);
    write_u16_le(&mut image, 62, 0);

    let base = entry & !(ELF_BOOT_ALIGN - 1);
    let ph = ELF_HEADER_SIZE;
    write_u32_le(&mut image, ph, ELF_PT_LOAD);
    write_u32_le(&mut image, ph + 4, ELF_PF_READ | ELF_PF_EXEC);
    write_u64_le(&mut image, ph + 8, 0);
    write_u64_le(&mut image, ph + 16, base);
    write_u64_le(&mut image, ph + 24, base);
    write_u64_le(&mut image, ph + 32, size as u64);
    write_u64_le(&mut image, ph + 40, ELF_BOOT_ALIGN);
    write_u64_le(&mut image, ph + 48, ELF_BOOT_ALIGN);

    image
}
