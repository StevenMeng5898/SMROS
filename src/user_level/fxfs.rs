//! Minimal FxFS-shaped object store for SMROS userspace bring-up.
//!
//! This is not a direct import of Fuchsia FxFS. It preserves the shape that
//! SMROS needs first: stable object ids, explicit directory entries, object
//! attributes, file contents, private component storage roots, and a small
//! logical journal with an in-memory replay model. The object store now syncs
//! a compact image into the userspace QEMU virtual block driver when that
//! driver is available.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::user_level::{drivers, user_logic};

const FXFS_ROOT_OBJECT_ID: u64 = 1;
const FXFS_MAX_OBJECTS: usize = 128;
const FXFS_MAX_DIRENTS: usize = 192;
const FXFS_MAX_FILE_BYTES: usize = 4096;
const FXFS_MAX_JOURNAL_RECORDS: usize = 128;
const FXFS_BLOCK_MAGIC: u32 = 0x5346_5846;
const FXFS_BLOCK_VERSION: u16 = 1;
const FXFS_BLOCK_HEADER_LEN: u16 = 56;
const FXFS_BLOCK_SLOT_COUNT: usize = 2;
const FXFS_MIN_SLOT_BYTES: usize = 64 * 1024;
const FXFS_DEFAULT_UID: u32 = 0;
const FXFS_DEFAULT_GID: u32 = 0;
const FXFS_DIR_MODE: u32 = 0o040755;
const FXFS_FILE_MODE: u32 = 0o100644;
const FXFS_ROOT_LINK_COUNT: u32 = 2;
const FXFS_FILE_LINK_COUNT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxfsNodeKind {
    Directory,
    File,
}

impl FxfsNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FxfsNodeKind::Directory => "dir",
            FxfsNodeKind::File => "file",
        }
    }

    fn default_mode(self) -> u32 {
        match self {
            FxfsNodeKind::Directory => FXFS_DIR_MODE,
            FxfsNodeKind::File => FXFS_FILE_MODE,
        }
    }

    fn default_link_count(self) -> u32 {
        match self {
            FxfsNodeKind::Directory => FXFS_ROOT_LINK_COUNT,
            FxfsNodeKind::File => FXFS_FILE_LINK_COUNT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxfsError {
    NotMounted,
    InvalidPath,
    NotFound,
    AlreadyExists,
    NoSpace,
    NotDirectory,
    IsDirectory,
    NotFile,
    InvalidOffset,
    StorageUnavailable,
    StorageCorrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxfsJournalOp {
    Mount,
    CreateDir,
    CreateFile,
    DeleteFile,
    WriteFile,
    AppendFile,
    TruncateFile,
    ReadFile,
    Lookup,
    SetAttributes,
    Replay,
}

#[derive(Clone, Copy, Debug)]
pub struct FxfsJournalRecord {
    pub sequence: u64,
    pub op: FxfsJournalOp,
    pub object_id: u64,
    pub parent_id: u64,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxfsAttributes {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: usize,
    pub created_at: u64,
    pub modified_at: u64,
    pub accessed_at: u64,
    pub link_count: u32,
}

impl FxfsAttributes {
    fn new(kind: FxfsNodeKind, sequence: u64, size: usize) -> Self {
        Self {
            mode: kind.default_mode(),
            uid: FXFS_DEFAULT_UID,
            gid: FXFS_DEFAULT_GID,
            size,
            created_at: sequence,
            modified_at: sequence,
            accessed_at: sequence,
            link_count: kind.default_link_count(),
        }
    }
}

#[derive(Clone, Debug)]
struct FxfsObject {
    object_id: u64,
    kind: FxfsNodeKind,
    data: Vec<u8>,
    attrs: FxfsAttributes,
}

#[derive(Clone, Debug)]
struct FxfsDirectoryEntry {
    parent_id: u64,
    name: String,
    object_id: u64,
}

struct LoadedFxfsImage {
    active_slot: usize,
    next_object_id: u64,
    sequence: u64,
    replayed_records: usize,
    objects: Vec<FxfsObject>,
    dirents: Vec<FxfsDirectoryEntry>,
    journal: Vec<FxfsJournalRecord>,
}

#[derive(Clone, Debug)]
pub struct FxfsDirEntry {
    pub object_id: u64,
    pub kind: FxfsNodeKind,
    pub attrs: FxfsAttributes,
    pub size: usize,
    pub name: String,
}

#[derive(Clone, Copy, Debug)]
pub struct FxfsStats {
    pub mounted: bool,
    pub block_backed: bool,
    pub last_sync_ok: bool,
    pub last_storage_error: Option<FxfsError>,
    pub block_bytes: usize,
    pub storage_slots: usize,
    pub active_slot: usize,
    pub slot_bytes: usize,
    pub nodes: usize,
    pub directories: usize,
    pub files: usize,
    pub dir_entries: usize,
    pub bytes: usize,
    pub journal_records: usize,
    pub replayed_records: usize,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct FxfsCursor {
    object_id: u64,
    offset: usize,
}

impl FxfsCursor {
    pub fn object_id(self) -> u64 {
        self.object_id
    }

    pub fn offset(self) -> usize {
        self.offset
    }
}

fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_bytes<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], FxfsError> {
    let end = pos.checked_add(len).ok_or(FxfsError::StorageCorrupt)?;
    if end > data.len() {
        return Err(FxfsError::StorageCorrupt);
    }
    let bytes = &data[*pos..end];
    *pos = end;
    Ok(bytes)
}

fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, FxfsError> {
    Ok(read_bytes(data, pos, 1)?[0])
}

fn read_u16(data: &[u8], pos: &mut usize) -> Result<u16, FxfsError> {
    let bytes = read_bytes(data, pos, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, FxfsError> {
    let bytes = read_bytes(data, pos, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], pos: &mut usize) -> Result<u64, FxfsError> {
    let bytes = read_bytes(data, pos, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn fxfs_checksum(data: &[u8]) -> u32 {
    let mut checksum = 0u32;
    for byte in data {
        checksum = checksum.rotate_left(5).wrapping_add(*byte as u32);
    }
    checksum
}

fn fxfs_header_is_blank(header: &[u8]) -> bool {
    header.iter().all(|byte| *byte == 0)
}

fn fxfs_align_down(value: usize, align: usize) -> usize {
    if align == 0 {
        value
    } else {
        value - (value % align)
    }
}

fn fxfs_storage_layout() -> Result<(usize, usize), FxfsError> {
    let block_capacity = drivers::block_capacity();
    if block_capacity < FXFS_BLOCK_HEADER_LEN as usize {
        return Err(FxfsError::StorageUnavailable);
    }

    let block_size = drivers::block_size();
    let usable_capacity = if block_capacity > block_size {
        block_capacity - block_size
    } else {
        block_capacity
    };

    if usable_capacity >= FXFS_MIN_SLOT_BYTES * FXFS_BLOCK_SLOT_COUNT {
        let slot_size = fxfs_align_down(usable_capacity / FXFS_BLOCK_SLOT_COUNT, block_size);
        if slot_size >= FXFS_BLOCK_HEADER_LEN as usize {
            return Ok((FXFS_BLOCK_SLOT_COUNT, slot_size));
        }
    }

    Ok((1, usable_capacity))
}

fn fxfs_slot_offset(slot: usize, slot_size: usize) -> usize {
    slot.saturating_mul(slot_size)
}

fn kind_to_u8(kind: FxfsNodeKind) -> u8 {
    match kind {
        FxfsNodeKind::Directory => 1,
        FxfsNodeKind::File => 2,
    }
}

fn kind_from_u8(value: u8) -> Result<FxfsNodeKind, FxfsError> {
    match value {
        1 => Ok(FxfsNodeKind::Directory),
        2 => Ok(FxfsNodeKind::File),
        _ => Err(FxfsError::StorageCorrupt),
    }
}

fn journal_op_to_u8(op: FxfsJournalOp) -> u8 {
    match op {
        FxfsJournalOp::Mount => 1,
        FxfsJournalOp::CreateDir => 2,
        FxfsJournalOp::CreateFile => 3,
        FxfsJournalOp::DeleteFile => 4,
        FxfsJournalOp::WriteFile => 5,
        FxfsJournalOp::AppendFile => 6,
        FxfsJournalOp::TruncateFile => 7,
        FxfsJournalOp::ReadFile => 8,
        FxfsJournalOp::Lookup => 9,
        FxfsJournalOp::SetAttributes => 10,
        FxfsJournalOp::Replay => 11,
    }
}

fn journal_op_from_u8(value: u8) -> Result<FxfsJournalOp, FxfsError> {
    match value {
        1 => Ok(FxfsJournalOp::Mount),
        2 => Ok(FxfsJournalOp::CreateDir),
        3 => Ok(FxfsJournalOp::CreateFile),
        4 => Ok(FxfsJournalOp::DeleteFile),
        5 => Ok(FxfsJournalOp::WriteFile),
        6 => Ok(FxfsJournalOp::AppendFile),
        7 => Ok(FxfsJournalOp::TruncateFile),
        8 => Ok(FxfsJournalOp::ReadFile),
        9 => Ok(FxfsJournalOp::Lookup),
        10 => Ok(FxfsJournalOp::SetAttributes),
        11 => Ok(FxfsJournalOp::Replay),
        _ => Err(FxfsError::StorageCorrupt),
    }
}

pub struct FxfsState {
    mounted: bool,
    block_backed: bool,
    last_sync_ok: bool,
    last_storage_error: Option<FxfsError>,
    active_slot: usize,
    next_object_id: u64,
    sequence: u64,
    replayed_records: usize,
    objects: Vec<FxfsObject>,
    dirents: Vec<FxfsDirectoryEntry>,
    journal: Vec<FxfsJournalRecord>,
}

impl FxfsState {
    fn new() -> Self {
        Self {
            mounted: false,
            block_backed: false,
            last_sync_ok: false,
            last_storage_error: None,
            active_slot: 0,
            next_object_id: FXFS_ROOT_OBJECT_ID + 1,
            sequence: 0,
            replayed_records: 0,
            objects: Vec::new(),
            dirents: Vec::new(),
            journal: Vec::new(),
        }
    }

    fn ensure_mounted(&self) -> Result<(), FxfsError> {
        if self.mounted {
            Ok(())
        } else {
            Err(FxfsError::NotMounted)
        }
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }

    fn record(&mut self, op: FxfsJournalOp, object_id: u64, parent_id: u64, size: usize) {
        let sequence = self.next_sequence();
        if self.journal.len() >= FXFS_MAX_JOURNAL_RECORDS {
            let _ = self.journal.remove(0);
        }
        self.journal.push(FxfsJournalRecord {
            sequence,
            op,
            object_id,
            parent_id,
            size,
        });
    }

    fn persist(&mut self) {
        if !self.block_backed {
            self.last_sync_ok = false;
            return;
        }
        match self.sync_to_block() {
            Ok(()) => {
                self.last_sync_ok = true;
                self.last_storage_error = None;
            }
            Err(err) => {
                self.last_sync_ok = false;
                self.last_storage_error = Some(err);
            }
        }
    }

    fn make_object(&mut self, object_id: u64, kind: FxfsNodeKind, data: &[u8]) -> FxfsObject {
        let sequence = self.next_sequence();
        FxfsObject {
            object_id,
            kind,
            data: data.to_vec(),
            attrs: FxfsAttributes::new(kind, sequence, data.len()),
        }
    }

    fn root_object(&mut self) -> FxfsObject {
        self.make_object(FXFS_ROOT_OBJECT_ID, FxfsNodeKind::Directory, &[])
    }

    fn mount(&mut self) -> Result<(), FxfsError> {
        if self.mounted {
            return Ok(());
        }

        self.block_backed = drivers::init() && drivers::block_ready();
        self.last_sync_ok = false;
        self.last_storage_error = None;
        if self.block_backed {
            match self.load_from_block() {
                Ok(()) => {
                    self.last_sync_ok = true;
                    self.last_storage_error = None;
                    return Ok(());
                }
                Err(FxfsError::NotFound) => {}
                Err(err) => {
                    self.last_storage_error = Some(err);
                    return Err(err);
                }
            }
        }

        self.mounted = true;
        self.active_slot = 0;
        self.next_object_id = FXFS_ROOT_OBJECT_ID + 1;
        self.sequence = 0;
        self.replayed_records = 0;
        self.objects.clear();
        self.dirents.clear();
        self.journal.clear();
        let root = self.root_object();
        self.objects.push(root);
        self.record(
            FxfsJournalOp::Mount,
            FXFS_ROOT_OBJECT_ID,
            FXFS_ROOT_OBJECT_ID,
            0,
        );

        self.create_dir("/pkg")?;
        self.create_dir("/pkg/bin")?;
        self.create_dir("/data")?;
        self.create_dir("/tmp")?;
        self.create_dir("/svc")?;
        self.create_dir("/config")?;
        self.create_dir("/config/build-info")?;
        self.write_file("/pkg/bin/component_manager", b"smros component manager")?;
        self.write_file("/pkg/bin/fxfs", b"smros fxfs service")?;
        self.write_file("/pkg/bin/user-init", b"smros user init")?;
        self.write_file("/config/build-info/product", b"SMROS-Fuchsia-minimal")?;
        self.persist();
        Ok(())
    }

    fn serialize_image(&self) -> Result<Vec<u8>, FxfsError> {
        let mut body = Vec::new();
        for object in &self.objects {
            push_u64(&mut body, object.object_id);
            push_u8(&mut body, kind_to_u8(object.kind));
            push_u32(&mut body, object.attrs.mode);
            push_u32(&mut body, object.attrs.uid);
            push_u32(&mut body, object.attrs.gid);
            push_u64(&mut body, object.attrs.size as u64);
            push_u64(&mut body, object.attrs.created_at);
            push_u64(&mut body, object.attrs.modified_at);
            push_u64(&mut body, object.attrs.accessed_at);
            push_u32(&mut body, object.attrs.link_count);
            push_u32(&mut body, object.data.len() as u32);
            body.extend_from_slice(&object.data);
        }
        for dirent in &self.dirents {
            if dirent.name.len() > u16::MAX as usize {
                return Err(FxfsError::NoSpace);
            }
            push_u64(&mut body, dirent.parent_id);
            push_u64(&mut body, dirent.object_id);
            push_u16(&mut body, dirent.name.len() as u16);
            body.extend_from_slice(dirent.name.as_bytes());
        }
        for record in &self.journal {
            push_u64(&mut body, record.sequence);
            push_u8(&mut body, journal_op_to_u8(record.op));
            push_u64(&mut body, record.object_id);
            push_u64(&mut body, record.parent_id);
            push_u64(&mut body, record.size as u64);
        }

        let total_len = (FXFS_BLOCK_HEADER_LEN as usize)
            .checked_add(body.len())
            .ok_or(FxfsError::NoSpace)?;
        if total_len > drivers::block_capacity() {
            return Err(FxfsError::NoSpace);
        }

        let mut image = Vec::new();
        push_u32(&mut image, FXFS_BLOCK_MAGIC);
        push_u16(&mut image, FXFS_BLOCK_VERSION);
        push_u16(&mut image, FXFS_BLOCK_HEADER_LEN);
        push_u32(&mut image, total_len as u32);
        push_u32(&mut image, fxfs_checksum(&body));
        push_u64(&mut image, self.next_object_id);
        push_u64(&mut image, self.sequence);
        push_u64(&mut image, self.replayed_records as u64);
        push_u32(&mut image, self.objects.len() as u32);
        push_u32(&mut image, self.dirents.len() as u32);
        push_u32(&mut image, self.journal.len() as u32);
        push_u32(&mut image, 0);
        image.extend_from_slice(&body);
        Ok(image)
    }

    fn sync_to_block(&mut self) -> Result<(), FxfsError> {
        if !drivers::block_ready() {
            return Err(FxfsError::StorageUnavailable);
        }
        let image = self.serialize_image()?;
        let (slot_count, slot_size) = fxfs_storage_layout()?;
        if image.len() > slot_size {
            return Err(FxfsError::NoSpace);
        }

        let next_slot = if slot_count > 1 {
            (self.active_slot + 1) % slot_count
        } else {
            0
        };
        let offset = fxfs_slot_offset(next_slot, slot_size);
        drivers::block_write_at(offset, &image).map_err(|_| FxfsError::StorageUnavailable)?;
        drivers::block_flush().map_err(|_| FxfsError::StorageUnavailable)?;
        self.active_slot = next_slot;
        Ok(())
    }

    fn load_image_from_slot(
        &self,
        slot: usize,
        slot_size: usize,
    ) -> Result<Option<LoadedFxfsImage>, FxfsError> {
        let mut header = [0u8; FXFS_BLOCK_HEADER_LEN as usize];
        let slot_offset = fxfs_slot_offset(slot, slot_size);
        drivers::block_read_at(slot_offset, &mut header)
            .map_err(|_| FxfsError::StorageUnavailable)?;
        if fxfs_header_is_blank(&header) {
            return Ok(None);
        }

        let mut pos = 0usize;
        if read_u32(&header, &mut pos)? != FXFS_BLOCK_MAGIC {
            return Err(FxfsError::StorageCorrupt);
        }
        if read_u16(&header, &mut pos)? != FXFS_BLOCK_VERSION {
            return Err(FxfsError::StorageCorrupt);
        }
        let header_len = read_u16(&header, &mut pos)? as usize;
        if header_len != FXFS_BLOCK_HEADER_LEN as usize {
            return Err(FxfsError::StorageCorrupt);
        }
        let total_len = read_u32(&header, &mut pos)? as usize;
        if total_len < header_len || total_len > slot_size {
            return Err(FxfsError::StorageCorrupt);
        }
        let checksum = read_u32(&header, &mut pos)?;
        let next_object_id = read_u64(&header, &mut pos)?;
        let sequence = read_u64(&header, &mut pos)?;
        let replayed_records = read_u64(&header, &mut pos)? as usize;
        let object_count = read_u32(&header, &mut pos)? as usize;
        let dirent_count = read_u32(&header, &mut pos)? as usize;
        let journal_count = read_u32(&header, &mut pos)? as usize;
        let _reserved = read_u32(&header, &mut pos)?;

        if object_count == 0
            || object_count > FXFS_MAX_OBJECTS
            || dirent_count > FXFS_MAX_DIRENTS
            || journal_count > FXFS_MAX_JOURNAL_RECORDS
        {
            return Err(FxfsError::StorageCorrupt);
        }

        let body_len = total_len - header_len;
        let mut body = Vec::new();
        body.resize(body_len, 0);
        if body_len > 0 {
            drivers::block_read_at(slot_offset + header_len, &mut body)
                .map_err(|_| FxfsError::StorageUnavailable)?;
        }
        if fxfs_checksum(&body) != checksum {
            return Err(FxfsError::StorageCorrupt);
        }

        let mut body_pos = 0usize;
        let mut objects = Vec::new();
        for _ in 0..object_count {
            let object_id = read_u64(&body, &mut body_pos)?;
            let kind = kind_from_u8(read_u8(&body, &mut body_pos)?)?;
            let mode = read_u32(&body, &mut body_pos)?;
            let uid = read_u32(&body, &mut body_pos)?;
            let gid = read_u32(&body, &mut body_pos)?;
            let size = read_u64(&body, &mut body_pos)? as usize;
            let created_at = read_u64(&body, &mut body_pos)?;
            let modified_at = read_u64(&body, &mut body_pos)?;
            let accessed_at = read_u64(&body, &mut body_pos)?;
            let link_count = read_u32(&body, &mut body_pos)?;
            let data_len = read_u32(&body, &mut body_pos)? as usize;
            if !user_logic::fxfs_file_size_valid(data_len) || size != data_len {
                return Err(FxfsError::StorageCorrupt);
            }
            let data = read_bytes(&body, &mut body_pos, data_len)?.to_vec();
            objects.push(FxfsObject {
                object_id,
                kind,
                data,
                attrs: FxfsAttributes {
                    mode,
                    uid,
                    gid,
                    size,
                    created_at,
                    modified_at,
                    accessed_at,
                    link_count,
                },
            });
        }

        let mut dirents = Vec::new();
        for _ in 0..dirent_count {
            let parent_id = read_u64(&body, &mut body_pos)?;
            let object_id = read_u64(&body, &mut body_pos)?;
            let name_len = read_u16(&body, &mut body_pos)? as usize;
            let name_bytes = read_bytes(&body, &mut body_pos, name_len)?;
            let name =
                String::from_utf8(name_bytes.to_vec()).map_err(|_| FxfsError::StorageCorrupt)?;
            dirents.push(FxfsDirectoryEntry {
                parent_id,
                name,
                object_id,
            });
        }

        let mut journal = Vec::new();
        for _ in 0..journal_count {
            let sequence = read_u64(&body, &mut body_pos)?;
            let op = journal_op_from_u8(read_u8(&body, &mut body_pos)?)?;
            let object_id = read_u64(&body, &mut body_pos)?;
            let parent_id = read_u64(&body, &mut body_pos)?;
            let size = read_u64(&body, &mut body_pos)? as usize;
            journal.push(FxfsJournalRecord {
                sequence,
                op,
                object_id,
                parent_id,
                size,
            });
        }
        if body_pos != body.len() {
            return Err(FxfsError::StorageCorrupt);
        }
        if objects
            .iter()
            .position(|object| object.object_id == FXFS_ROOT_OBJECT_ID)
            .is_none()
        {
            return Err(FxfsError::StorageCorrupt);
        }

        Ok(Some(LoadedFxfsImage {
            active_slot: slot,
            next_object_id,
            sequence,
            replayed_records,
            objects,
            dirents,
            journal,
        }))
    }

    fn load_from_block(&mut self) -> Result<(), FxfsError> {
        if !drivers::block_ready() {
            return Err(FxfsError::StorageUnavailable);
        }

        let (slot_count, slot_size) = fxfs_storage_layout()?;
        let mut best: Option<LoadedFxfsImage> = None;
        let mut saw_corrupt = false;

        for slot in 0..slot_count {
            match self.load_image_from_slot(slot, slot_size) {
                Ok(Some(image)) => {
                    if best
                        .as_ref()
                        .map(|candidate| image.sequence > candidate.sequence)
                        .unwrap_or(true)
                    {
                        best = Some(image);
                    }
                }
                Ok(None) => {}
                Err(FxfsError::StorageCorrupt) => {
                    saw_corrupt = true;
                }
                Err(err) => return Err(err),
            }
        }

        let Some(image) = best else {
            if saw_corrupt {
                return Err(FxfsError::StorageCorrupt);
            }
            return Err(FxfsError::NotFound);
        };

        self.mounted = true;
        self.block_backed = true;
        self.last_sync_ok = true;
        self.active_slot = image.active_slot;
        self.next_object_id = image.next_object_id;
        self.sequence = image.sequence;
        self.replayed_records = image.replayed_records;
        self.objects = image.objects;
        self.dirents = image.dirents;
        self.journal = image.journal;
        Ok(())
    }

    fn find_object_index(&self, object_id: u64) -> Option<usize> {
        self.objects
            .iter()
            .position(|object| object.object_id == object_id)
    }

    fn find_dirent_index(&self, parent_id: u64, name: &str) -> Option<usize> {
        self.dirents
            .iter()
            .position(|entry| entry.parent_id == parent_id && entry.name == name)
    }

    fn child_object_id(&self, parent_id: u64, name: &str) -> Option<u64> {
        self.find_dirent_index(parent_id, name)
            .map(|index| self.dirents[index].object_id)
    }

    fn resolve_object_id(&self, path: &str) -> Result<u64, FxfsError> {
        self.ensure_mounted()?;
        if path == "/" {
            return Ok(FXFS_ROOT_OBJECT_ID);
        }
        if !path.starts_with('/') || path.ends_with("//") {
            return Err(FxfsError::InvalidPath);
        }

        let mut current = FXFS_ROOT_OBJECT_ID;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            let current_index = self.find_object_index(current).ok_or(FxfsError::NotFound)?;
            if self.objects[current_index].kind != FxfsNodeKind::Directory {
                return Err(FxfsError::NotDirectory);
            }
            current = self
                .child_object_id(current, part)
                .ok_or(FxfsError::NotFound)?;
        }
        if self.find_object_index(current).is_none() {
            return Err(FxfsError::NotFound);
        }
        Ok(current)
    }

    fn resolve_path(&self, path: &str) -> Result<usize, FxfsError> {
        let object_id = self.resolve_object_id(path)?;
        self.find_object_index(object_id).ok_or(FxfsError::NotFound)
    }

    fn parent_and_name<'a>(&self, path: &'a str) -> Result<(u64, &'a str), FxfsError> {
        self.ensure_mounted()?;
        if !path.starts_with('/') {
            return Err(FxfsError::InvalidPath);
        }

        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/" {
            return Err(FxfsError::InvalidPath);
        }

        let split = trimmed.rfind('/').ok_or(FxfsError::InvalidPath)?;
        let (parent_path, name_with_slash) = trimmed.split_at(split);
        let name = &name_with_slash[1..];
        if name.is_empty() || name == "." || name == ".." {
            return Err(FxfsError::InvalidPath);
        }

        let parent_path = if parent_path.is_empty() {
            "/"
        } else {
            parent_path
        };
        let parent_id = self.resolve_object_id(parent_path)?;
        let parent_index = self
            .find_object_index(parent_id)
            .ok_or(FxfsError::NotFound)?;
        if self.objects[parent_index].kind != FxfsNodeKind::Directory {
            return Err(FxfsError::NotDirectory);
        }
        Ok((parent_id, name))
    }

    fn create_object(
        &mut self,
        parent_id: u64,
        name: &str,
        kind: FxfsNodeKind,
        data: &[u8],
    ) -> Result<u64, FxfsError> {
        if !user_logic::fxfs_node_capacity_valid(self.objects.len()) {
            return Err(FxfsError::NoSpace);
        }
        if !user_logic::fxfs_dirent_capacity_valid(self.dirents.len()) {
            return Err(FxfsError::NoSpace);
        }
        if !user_logic::fxfs_file_size_valid(data.len()) {
            return Err(FxfsError::NoSpace);
        }
        if self.find_dirent_index(parent_id, name).is_some() {
            return Err(FxfsError::AlreadyExists);
        }

        let object_id = self.next_object_id;
        self.next_object_id = self.next_object_id.saturating_add(1);
        let object = self.make_object(object_id, kind, data);
        self.objects.push(object);
        self.dirents.push(FxfsDirectoryEntry {
            parent_id,
            name: name.to_string(),
            object_id,
        });
        self.touch_directory(parent_id);
        Ok(object_id)
    }

    fn touch_directory(&mut self, object_id: u64) {
        let sequence = self.next_sequence();
        if let Some(index) = self.find_object_index(object_id) {
            let attrs = &mut self.objects[index].attrs;
            attrs.modified_at = sequence;
            attrs.accessed_at = sequence;
        }
    }

    fn touch_file_write(&mut self, index: usize) {
        let sequence = self.next_sequence();
        let object = &mut self.objects[index];
        object.attrs.size = object.data.len();
        object.attrs.modified_at = sequence;
        object.attrs.accessed_at = sequence;
    }

    fn touch_file_read(&mut self, index: usize) {
        let sequence = self.next_sequence();
        self.objects[index].attrs.accessed_at = sequence;
    }

    fn create_dir(&mut self, path: &str) -> Result<u64, FxfsError> {
        let (parent_id, name) = self.parent_and_name(path)?;
        if let Some(object_id) = self.child_object_id(parent_id, name) {
            let index = self
                .find_object_index(object_id)
                .ok_or(FxfsError::NotFound)?;
            if self.objects[index].kind == FxfsNodeKind::Directory {
                return Ok(object_id);
            }
            return Err(FxfsError::NotDirectory);
        }
        let object_id = self.create_object(parent_id, name, FxfsNodeKind::Directory, &[])?;
        self.record(FxfsJournalOp::CreateDir, object_id, parent_id, 0);
        self.persist();
        Ok(object_id)
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<usize, FxfsError> {
        self.ensure_mounted()?;
        if !user_logic::fxfs_file_size_valid(data.len()) {
            return Err(FxfsError::NoSpace);
        }

        match self.resolve_path(path) {
            Ok(index) => {
                if self.objects[index].kind == FxfsNodeKind::Directory {
                    return Err(FxfsError::IsDirectory);
                }
                if self.objects[index].data.as_slice() == data {
                    return Ok(data.len());
                }
                self.objects[index].data.clear();
                self.objects[index].data.extend_from_slice(data);
                self.touch_file_write(index);
                let object_id = self.objects[index].object_id;
                self.record(FxfsJournalOp::WriteFile, object_id, 0, data.len());
                self.persist();
                Ok(data.len())
            }
            Err(FxfsError::NotFound) => {
                let (parent_id, name) = self.parent_and_name(path)?;
                let object_id = self.create_object(parent_id, name, FxfsNodeKind::File, data)?;
                self.record(FxfsJournalOp::CreateFile, object_id, parent_id, data.len());
                self.record(FxfsJournalOp::WriteFile, object_id, parent_id, data.len());
                self.persist();
                Ok(data.len())
            }
            Err(err) => Err(err),
        }
    }

    fn append_file(&mut self, path: &str, data: &[u8]) -> Result<usize, FxfsError> {
        let index = self.resolve_path(path)?;
        if self.objects[index].kind != FxfsNodeKind::File {
            return Err(FxfsError::NotFile);
        }
        let new_size =
            match user_logic::fxfs_append_size(self.objects[index].data.len(), data.len()) {
                Some(size) if user_logic::fxfs_file_size_valid(size) => size,
                _ => return Err(FxfsError::NoSpace),
            };
        let old_size = self.objects[index].data.len();
        self.objects[index].data.reserve(new_size - old_size);
        self.objects[index].data.extend_from_slice(data);
        self.touch_file_write(index);
        let object_id = self.objects[index].object_id;
        self.record(
            FxfsJournalOp::AppendFile,
            object_id,
            0,
            self.objects[index].data.len(),
        );
        self.persist();
        Ok(data.len())
    }

    fn truncate_file(&mut self, path: &str, size: usize) -> Result<usize, FxfsError> {
        if !user_logic::fxfs_file_size_valid(size) {
            return Err(FxfsError::NoSpace);
        }
        let index = self.resolve_path(path)?;
        if self.objects[index].kind != FxfsNodeKind::File {
            return Err(FxfsError::NotFile);
        }
        self.objects[index].data.resize(size, 0);
        self.touch_file_write(index);
        let object_id = self.objects[index].object_id;
        self.record(FxfsJournalOp::TruncateFile, object_id, 0, size);
        self.persist();
        Ok(size)
    }

    fn delete_file(&mut self, path: &str) -> Result<(), FxfsError> {
        let index = self.resolve_path(path)?;
        if self.objects[index].kind == FxfsNodeKind::Directory {
            return Err(FxfsError::IsDirectory);
        }
        let object_id = self.objects[index].object_id;
        let dirent_index = self
            .dirents
            .iter()
            .position(|entry| entry.object_id == object_id)
            .ok_or(FxfsError::NotFound)?;
        let parent_id = self.dirents[dirent_index].parent_id;
        self.dirents.remove(dirent_index);
        self.objects.remove(index);
        self.touch_directory(parent_id);
        self.record(FxfsJournalOp::DeleteFile, object_id, parent_id, 0);
        self.persist();
        Ok(())
    }

    fn open_cursor(&self, path: &str) -> Result<FxfsCursor, FxfsError> {
        let index = self.resolve_path(path)?;
        if self.objects[index].kind != FxfsNodeKind::File {
            return Err(FxfsError::NotFile);
        }
        Ok(FxfsCursor {
            object_id: self.objects[index].object_id,
            offset: 0,
        })
    }

    fn seek_cursor(&self, cursor: &mut FxfsCursor, offset: usize) -> Result<usize, FxfsError> {
        let index = self
            .find_object_index(cursor.object_id)
            .ok_or(FxfsError::NotFound)?;
        if self.objects[index].kind != FxfsNodeKind::File {
            return Err(FxfsError::NotFile);
        }
        if !user_logic::fxfs_seek_valid(offset, self.objects[index].attrs.size) {
            return Err(FxfsError::InvalidOffset);
        }
        cursor.offset = offset;
        Ok(cursor.offset)
    }

    fn cursor_read(&mut self, cursor: &mut FxfsCursor, out: &mut [u8]) -> Result<usize, FxfsError> {
        let index = self
            .find_object_index(cursor.object_id)
            .ok_or(FxfsError::NotFound)?;
        if self.objects[index].kind != FxfsNodeKind::File {
            return Err(FxfsError::NotFile);
        }
        let file_len = self.objects[index].data.len();
        if cursor.offset > file_len {
            return Err(FxfsError::InvalidOffset);
        }
        let available = file_len - cursor.offset;
        let len = core::cmp::min(out.len(), available);
        out[..len].copy_from_slice(&self.objects[index].data[cursor.offset..cursor.offset + len]);
        cursor.offset = cursor.offset.saturating_add(len);
        Ok(len)
    }

    fn cursor_write(&mut self, cursor: &mut FxfsCursor, data: &[u8]) -> Result<usize, FxfsError> {
        let index = self
            .find_object_index(cursor.object_id)
            .ok_or(FxfsError::NotFound)?;
        if self.objects[index].kind != FxfsNodeKind::File {
            return Err(FxfsError::NotFile);
        }
        let new_size = match user_logic::fxfs_write_end(cursor.offset, data.len()) {
            Some(end) if user_logic::fxfs_file_size_valid(end) => end,
            _ => return Err(FxfsError::NoSpace),
        };
        if new_size > self.objects[index].data.len() {
            self.objects[index].data.resize(new_size, 0);
        }
        let end = cursor.offset + data.len();
        self.objects[index].data[cursor.offset..end].copy_from_slice(data);
        cursor.offset = end;
        self.touch_file_write(index);
        let object_id = self.objects[index].object_id;
        self.record(
            FxfsJournalOp::WriteFile,
            object_id,
            0,
            self.objects[index].data.len(),
        );
        self.persist();
        Ok(data.len())
    }

    fn read_file_at(
        &mut self,
        path: &str,
        offset: usize,
        out: &mut [u8],
    ) -> Result<usize, FxfsError> {
        let mut cursor = self.open_cursor(path)?;
        self.seek_cursor(&mut cursor, offset)?;
        self.cursor_read(&mut cursor, out)
    }

    fn read_file(&mut self, path: &str, out: &mut [u8]) -> Result<usize, FxfsError> {
        self.read_file_at(path, 0, out)
    }

    fn attrs(&mut self, path: &str) -> Result<FxfsAttributes, FxfsError> {
        let index = self.resolve_path(path)?;
        Ok(self.objects[index].attrs)
    }

    fn set_attrs(
        &mut self,
        path: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<FxfsAttributes, FxfsError> {
        let index = self.resolve_path(path)?;
        let sequence = self.next_sequence();
        self.objects[index].attrs.mode = mode;
        self.objects[index].attrs.uid = uid;
        self.objects[index].attrs.gid = gid;
        self.objects[index].attrs.modified_at = sequence;
        self.objects[index].attrs.accessed_at = sequence;
        let object_id = self.objects[index].object_id;
        let size = self.objects[index].attrs.size;
        self.record(FxfsJournalOp::SetAttributes, object_id, 0, size);
        self.persist();
        Ok(self.objects[index].attrs)
    }

    fn exists(&mut self, path: &str) -> bool {
        self.resolve_path(path).is_ok()
    }

    fn entries(&self, path: &str) -> Result<Vec<FxfsDirEntry>, FxfsError> {
        let parent_id = self.resolve_object_id(path)?;
        let parent_index = self
            .find_object_index(parent_id)
            .ok_or(FxfsError::NotFound)?;
        if self.objects[parent_index].kind != FxfsNodeKind::Directory {
            return Err(FxfsError::NotDirectory);
        }

        let mut entries = Vec::new();
        for dirent in self
            .dirents
            .iter()
            .filter(|entry| entry.parent_id == parent_id)
        {
            if let Some(index) = self.find_object_index(dirent.object_id) {
                let object = &self.objects[index];
                entries.push(FxfsDirEntry {
                    object_id: object.object_id,
                    kind: object.kind,
                    attrs: object.attrs,
                    size: object.attrs.size,
                    name: dirent.name.clone(),
                });
            }
        }
        Ok(entries)
    }

    fn replay_journal(&mut self) -> Result<usize, FxfsError> {
        self.ensure_mounted()?;
        let journal = self.journal.clone();
        let mut replayed = 0usize;
        for record in &journal {
            if let Some(index) = self.find_object_index(record.object_id) {
                match record.op {
                    FxfsJournalOp::CreateDir | FxfsJournalOp::CreateFile => {
                        self.objects[index].attrs.created_at = record.sequence;
                        self.objects[index].attrs.modified_at = record.sequence;
                    }
                    FxfsJournalOp::DeleteFile => {}
                    FxfsJournalOp::WriteFile
                    | FxfsJournalOp::AppendFile
                    | FxfsJournalOp::TruncateFile => {
                        self.objects[index].attrs.size = self.objects[index].data.len();
                        self.objects[index].attrs.modified_at = record.sequence;
                    }
                    FxfsJournalOp::ReadFile | FxfsJournalOp::Lookup => {
                        self.objects[index].attrs.accessed_at = record.sequence;
                    }
                    FxfsJournalOp::SetAttributes => {
                        self.objects[index].attrs.modified_at = record.sequence;
                    }
                    FxfsJournalOp::Mount | FxfsJournalOp::Replay => {}
                }
                replayed = replayed.saturating_add(1);
            }
        }
        self.replayed_records = replayed;
        self.record(
            FxfsJournalOp::Replay,
            FXFS_ROOT_OBJECT_ID,
            FXFS_ROOT_OBJECT_ID,
            replayed,
        );
        self.persist();
        Ok(replayed)
    }

    fn stats(&self) -> FxfsStats {
        let mut directories = 0usize;
        let mut files = 0usize;
        let mut bytes = 0usize;
        for object in &self.objects {
            match object.kind {
                FxfsNodeKind::Directory => directories = directories.saturating_add(1),
                FxfsNodeKind::File => {
                    files = files.saturating_add(1);
                    bytes = bytes.saturating_add(object.attrs.size);
                }
            }
        }
        FxfsStats {
            mounted: self.mounted,
            block_backed: self.block_backed,
            last_sync_ok: self.last_sync_ok,
            last_storage_error: self.last_storage_error,
            block_bytes: drivers::block_capacity(),
            storage_slots: fxfs_storage_layout()
                .map(|(slot_count, _)| slot_count)
                .unwrap_or(0),
            active_slot: self.active_slot,
            slot_bytes: fxfs_storage_layout()
                .map(|(_, slot_size)| slot_size)
                .unwrap_or(0),
            nodes: self.objects.len(),
            directories,
            files,
            dir_entries: self.dirents.len(),
            bytes,
            journal_records: self.journal.len(),
            replayed_records: self.replayed_records,
            sequence: self.sequence,
        }
    }
}

static mut FXFS_STATE: Option<FxfsState> = None;

fn state() -> &'static mut FxfsState {
    unsafe {
        if FXFS_STATE.is_none() {
            FXFS_STATE = Some(FxfsState::new());
        }
        FXFS_STATE.as_mut().unwrap()
    }
}

pub fn init() -> bool {
    state().mount().is_ok()
}

pub fn stats() -> FxfsStats {
    state().stats()
}

pub fn exists(path: &str) -> bool {
    state().exists(path)
}

pub fn create_dir(path: &str) -> Result<u64, FxfsError> {
    state().create_dir(path)
}

pub fn write_file(path: &str, data: &[u8]) -> Result<usize, FxfsError> {
    state().write_file(path, data)
}

pub fn append_file(path: &str, data: &[u8]) -> Result<usize, FxfsError> {
    state().append_file(path, data)
}

pub fn truncate_file(path: &str, size: usize) -> Result<usize, FxfsError> {
    state().truncate_file(path, size)
}

pub fn delete_file(path: &str) -> Result<(), FxfsError> {
    state().delete_file(path)
}

pub fn read_file(path: &str, out: &mut [u8]) -> Result<usize, FxfsError> {
    state().read_file(path, out)
}

pub fn read_file_at(path: &str, offset: usize, out: &mut [u8]) -> Result<usize, FxfsError> {
    state().read_file_at(path, offset, out)
}

pub fn open_cursor(path: &str) -> Result<FxfsCursor, FxfsError> {
    state().open_cursor(path)
}

pub fn seek_cursor(cursor: &mut FxfsCursor, offset: usize) -> Result<usize, FxfsError> {
    state().seek_cursor(cursor, offset)
}

pub fn cursor_read(cursor: &mut FxfsCursor, out: &mut [u8]) -> Result<usize, FxfsError> {
    state().cursor_read(cursor, out)
}

pub fn cursor_write(cursor: &mut FxfsCursor, data: &[u8]) -> Result<usize, FxfsError> {
    state().cursor_write(cursor, data)
}

pub fn attrs(path: &str) -> Result<FxfsAttributes, FxfsError> {
    state().attrs(path)
}

pub fn set_attrs(path: &str, mode: u32, uid: u32, gid: u32) -> Result<FxfsAttributes, FxfsError> {
    state().set_attrs(path, mode, uid, gid)
}

pub fn entries(path: &str) -> Result<Vec<FxfsDirEntry>, FxfsError> {
    state().entries(path)
}

pub fn replay_journal() -> Result<usize, FxfsError> {
    state().replay_journal()
}

pub fn smoke_test() -> bool {
    if !init() {
        return false;
    }
    if !exists("/pkg/bin/component_manager") || !exists("/pkg/bin/fxfs") {
        return false;
    }
    if write_file("/data/smoke.txt", b"fxfs-smoke").is_err() {
        return false;
    }
    if append_file("/data/smoke.txt", b"-append").is_err() {
        return false;
    }
    if truncate_file("/data/smoke.txt", 10).is_err() {
        return false;
    }
    let mut out = [0u8; 10];
    if read_file("/data/smoke.txt", &mut out).ok() != Some(10) || out != *b"fxfs-smoke" {
        return false;
    }
    let attrs = match attrs("/data/smoke.txt") {
        Ok(attrs) => attrs,
        Err(_) => return false,
    };
    if attrs.size != 10 || attrs.link_count != FXFS_FILE_LINK_COUNT {
        return false;
    }
    let mut cursor = match open_cursor("/data/smoke.txt") {
        Ok(cursor) => cursor,
        Err(_) => return false,
    };
    if seek_cursor(&mut cursor, 5).ok() != Some(5) {
        return false;
    }
    let mut tail = [0u8; 5];
    if cursor_read(&mut cursor, &mut tail).ok() != Some(5) || tail != *b"smoke" {
        return false;
    }
    replay_journal().is_ok()
}
