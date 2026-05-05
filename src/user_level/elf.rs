//! Minimal ELF64/AArch64 image parser for boot userspace components.
//!
//! This loader intentionally handles only the first bring-up shape SMROS uses:
//! little-endian ELF64 images with a small program-header table and PT_LOAD
//! segments. It records load metadata for component processes; real copying of
//! segment bytes into per-process EL0 mappings is the next loader stage.

#![allow(dead_code)]

use alloc::vec::Vec;

use crate::user_level::user_logic;

pub const ELF_HEADER_SIZE: usize = 64;
pub const ELF_PHDR_SIZE: usize = 56;
pub const ELF_MAX_PHDRS: usize = 8;
pub const ELF_MAX_LOAD_SEGMENTS: usize = 4;
pub const ELF_MACHINE_AARCH64: u16 = 183;

const ELF_CLASS_64: u8 = 2;
const ELF_DATA_LSB: u8 = 1;
const ELF_VERSION_CURRENT: u8 = 1;
const ELF_TYPE_EXEC: u16 = 2;
const ELF_TYPE_DYN: u16 = 3;
const ELF_PT_LOAD: u32 = 1;
const ELF_PF_EXEC: u32 = 1;
const ELF_PF_READ: u32 = 4;
const ELF_BOOT_ALIGN: u64 = 4096;

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
    pub entry: u64,
    pub segments: Vec<ElfSegment>,
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

    let e_type = read_u16_le(image, 16).ok_or(ElfError::ShortHeader)?;
    if !user_logic::elf_type_valid(e_type) {
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
    for index in 0..phnum {
        let Some(header_offset) = phentsize
            .checked_mul(index)
            .and_then(|offset| phoff.checked_add(offset))
        else {
            return Err(ElfError::BadProgramHeaderTable);
        };

        let p_type = read_u32_le(image, header_offset).ok_or(ElfError::BadProgramHeaderTable)?;
        if p_type != ELF_PT_LOAD {
            continue;
        }
        if segments.len() >= ELF_MAX_LOAD_SEGMENTS {
            return Err(ElfError::TooManySegments);
        }

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

    Ok(ElfImage { entry, segments })
}

pub fn load_from_fxfs(path: &str) -> Result<ElfImage, ElfError> {
    let mut image = [0u8; user_logic::USER_FXFS_MAX_FILE_BYTES];
    let len =
        crate::user_level::fxfs::read_file(path, &mut image).map_err(|_| ElfError::Storage)?;
    parse(&image[..len])
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
