//! Bounded guest-side parser for the host-generated POSIX test manifest.

#![allow(dead_code)]

use crate::alloc::collections::BTreeSet;
use crate::alloc::string::String;
use crate::alloc::vec::Vec;

#[cfg(not(test))]
use core as core_compat;
#[cfg(test)]
use std as core_compat;

use super::{fxfs, posix_test_logic_shared};

#[cfg(not(test))]
use super::run_elf::{RunOutcome, RunTermination};

pub const POSIX_MANIFEST_PATH: &str = "/shared/posixtest/manifest.tsv";
pub const POSIX_MANIFEST_SCHEMA: u32 = 1;
pub const POSIX_MANIFEST_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const POSIX_MANIFEST_MAX_TESTS: usize = 4_096;
pub const POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES: usize = 1_024;
pub const POSIX_MANIFEST_MAX_TEST_ID_BYTES: usize = 256;
pub const POSIX_MANIFEST_MAX_GROUP_BYTES: usize = 96;
pub const POSIX_MANIFEST_MAX_API_BYTES: usize = 96;
pub const POSIX_MANIFEST_MAX_STAGED_PATH_BYTES: usize = 512;
pub const POSIX_FILTER_MAX_BYTES: usize = 256;

const MAX_TIMEOUT_MS: u32 = i32::MAX as u32;
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MANIFEST_HEADER: &str = "SMROS_POSIX_MANIFEST\t1";
const METADATA_KEYS: [&str; 9] = [
    "source",
    "revision",
    "architecture",
    "compiler",
    "libc",
    "patch_sha256",
    "build_results_sha256",
    "manifest_sha256",
    "smros_commit",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixTestError {
    FxfsPrepare,
    FxfsRead,
    ManifestTooLarge,
    InvalidUtf8,
    InvalidLineEndings,
    InvalidHeader,
    UnknownRowType,
    InvalidMetadataRow,
    UnknownMetadata,
    MissingMetadata,
    DuplicateMetadata,
    MetadataOutOfOrder,
    InvalidTestRow,
    TooManyTests,
    DuplicateTestId,
    DuplicateTestPath,
    InvalidAtom,
    InvalidPath,
    UnknownKind,
    UnknownDisposition,
    InvalidKindDisposition,
    InvalidChecksum,
    InvalidTimeout,
    InvalidProvenance,
    NonCanonicalManifest,
    ManifestChecksumMismatch,
    InvalidFilter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PosixFilter {
    All,
    Group(String),
    Api(String),
    Test(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixTestKind {
    Runnable,
    Definition,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixDisposition {
    Complete,
    DefinitionOnly,
    ExcludedUpstreamStub,
    CompileFailed,
    LinkFailed,
    NotBuiltShellTest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixManifestMetadata {
    pub source: String,
    pub revision: String,
    pub architecture: String,
    pub compiler: String,
    pub libc: String,
    pub patch_sha256: String,
    pub build_results_sha256: String,
    pub manifest_sha256: String,
    pub smros_commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixManifestTest {
    pub test_id: String,
    pub group: String,
    pub api: String,
    pub kind: PosixTestKind,
    pub disposition: PosixDisposition,
    pub binary_path: Option<String>,
    pub timeout_ms: u32,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixManifest {
    pub metadata: PosixManifestMetadata,
    pub tests: Vec<PosixManifestTest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixRunnerStatus {
    pub running: bool,
    pub run_id: Option<String>,
    pub filter: Option<PosixFilter>,
    pub current_test: Option<String>,
    pub completed: usize,
    pub selected: usize,
}

pub fn parse_filter(args: &[&str]) -> Result<PosixFilter, PosixTestError> {
    match args {
        ["all"] => Ok(PosixFilter::All),
        ["group", value] => parse_filter_value(value).map(PosixFilter::Group),
        ["api", value] => parse_filter_value(value).map(PosixFilter::Api),
        ["test", value] => parse_filter_value(value).map(PosixFilter::Test),
        _ => Err(PosixTestError::InvalidFilter),
    }
}

fn parse_filter_value(value: &str) -> Result<String, PosixTestError> {
    if value.len() > POSIX_FILTER_MAX_BYTES || !posix_test_logic_shared::manifest_atom_valid(value)
    {
        return Err(PosixTestError::InvalidFilter);
    }
    Ok(String::from(value))
}

pub fn load_manifest() -> Result<PosixManifest, PosixTestError> {
    fxfs::ensure_host_share().map_err(|_| PosixTestError::FxfsPrepare)?;
    let mut bytes = Vec::new();
    bytes.resize(POSIX_MANIFEST_MAX_BYTES + 1, 0u8);
    let read =
        fxfs::read_file(POSIX_MANIFEST_PATH, &mut bytes).map_err(|_| PosixTestError::FxfsRead)?;
    if read > POSIX_MANIFEST_MAX_BYTES {
        return Err(PosixTestError::ManifestTooLarge);
    }
    bytes.truncate(read);
    parse_manifest(&bytes)
}

pub fn status_snapshot() -> PosixRunnerStatus {
    PosixRunnerStatus {
        running: false,
        run_id: None,
        filter: None,
        current_test: None,
        completed: 0,
        selected: 0,
    }
}

/// Task 9 replaces this fail-closed bridge with the POSIX runner state machine.
#[cfg(not(test))]
pub fn on_run_outcome(outcome: RunOutcome) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("POSIX infrastructure failure: unhandled ELF outcome");
    serial.write_str(" path=");
    serial.write_str(outcome.path.as_str());
    serial.write_str(" termination=");
    match outcome.termination {
        RunTermination::Exit(_) => serial.write_str("exit"),
        RunTermination::LaunchError(err) => serial.write_str(err.as_str()),
    }
    serial.write_str("\n");
}

fn parse_fixed_fields<const N: usize>(line: &str) -> Option<[&str; N]> {
    let mut fields = [""; N];
    let mut parts = line.split('\t');
    for field in &mut fields {
        *field = parts.next()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(fields)
}

fn parse_manifest(data: &[u8]) -> Result<PosixManifest, PosixTestError> {
    if data.len() > POSIX_MANIFEST_MAX_BYTES {
        return Err(PosixTestError::ManifestTooLarge);
    }
    let text = core_compat::str::from_utf8(data).map_err(|_| PosixTestError::InvalidUtf8)?;
    if text.as_bytes().contains(&b'\r') || !text.ends_with('\n') {
        return Err(PosixTestError::InvalidLineEndings);
    }

    let mut lines = text[..text.len() - 1].split('\n');
    let header = lines.next().ok_or(PosixTestError::InvalidHeader)?;
    if header != MANIFEST_HEADER || POSIX_MANIFEST_SCHEMA != 1 {
        return Err(PosixTestError::InvalidHeader);
    }

    let mut metadata: [Option<String>; METADATA_KEYS.len()] = core_compat::array::from_fn(|_| None);
    let mut metadata_count = 0usize;
    let mut checksum_offset = None;
    let mut tests = Vec::new();
    let mut test_paths: BTreeSet<String> = BTreeSet::new();
    let mut previous_test_id: Option<String> = None;
    let mut saw_test = false;
    let mut line_offset = header.len() + 1;

    for line in lines {
        let current_offset = line_offset;
        line_offset = line_offset.saturating_add(line.len()).saturating_add(1);
        if line == "meta" || line.starts_with("meta\t") {
            let fields = parse_fixed_fields::<3>(line).ok_or(PosixTestError::InvalidMetadataRow)?;
            if saw_test {
                return Err(PosixTestError::InvalidMetadataRow);
            }
            let key = fields[1];
            let value = fields[2];
            let Some(key_index) = METADATA_KEYS.iter().position(|expected| *expected == key) else {
                return Err(PosixTestError::UnknownMetadata);
            };
            if metadata[key_index].is_some() {
                return Err(PosixTestError::DuplicateMetadata);
            }
            if key_index != metadata_count {
                return Err(PosixTestError::MetadataOutOfOrder);
            }
            validate_metadata_atom(value)?;
            if key == "manifest_sha256" {
                checksum_offset = Some(current_offset + "meta\tmanifest_sha256\t".len());
            }
            metadata[key_index] = Some(String::from(value));
            metadata_count += 1;
            continue;
        }

        if line == "test" || line.starts_with("test\t") {
            saw_test = true;
            let fields = parse_fixed_fields::<9>(line).ok_or(PosixTestError::InvalidTestRow)?;
            if tests.len() >= POSIX_MANIFEST_MAX_TESTS {
                return Err(PosixTestError::TooManyTests);
            }
            let test = parse_test_row(&fields)?;
            let previous_order = previous_test_id
                .as_ref()
                .map(|previous| previous.as_str().cmp(test.test_id.as_str()));
            if previous_order == Some(core_compat::cmp::Ordering::Equal) {
                return Err(PosixTestError::DuplicateTestId);
            }
            if let Some(path) = test.binary_path.as_ref() {
                if !test_paths.insert(path.clone()) {
                    return Err(PosixTestError::DuplicateTestPath);
                }
            }
            if previous_order == Some(core_compat::cmp::Ordering::Greater) {
                return Err(PosixTestError::NonCanonicalManifest);
            }
            previous_test_id = Some(test.test_id.clone());
            tests.push(test);
            continue;
        }

        return Err(PosixTestError::UnknownRowType);
    }

    if metadata_count != METADATA_KEYS.len() {
        return Err(PosixTestError::MissingMetadata);
    }
    let metadata = build_metadata(metadata)?;
    validate_provenance(&metadata)?;
    let checksum_offset = checksum_offset.ok_or(PosixTestError::MissingMetadata)?;
    if !manifest_checksum_matches(data, checksum_offset, &metadata.manifest_sha256) {
        return Err(PosixTestError::ManifestChecksumMismatch);
    }

    Ok(PosixManifest { metadata, tests })
}

fn parse_test_row(fields: &[&str]) -> Result<PosixManifestTest, PosixTestError> {
    let test_id = fields[1];
    let group = fields[2];
    let api = fields[3];
    validate_manifest_atom(test_id, POSIX_MANIFEST_MAX_TEST_ID_BYTES)?;
    validate_manifest_atom(group, POSIX_MANIFEST_MAX_GROUP_BYTES)?;
    validate_manifest_atom(api, POSIX_MANIFEST_MAX_API_BYTES)?;

    let kind = match fields[4] {
        "runnable" => PosixTestKind::Runnable,
        "definition" => PosixTestKind::Definition,
        "shell" => PosixTestKind::Shell,
        _ => return Err(PosixTestError::UnknownKind),
    };
    let disposition = match fields[5] {
        "complete" => PosixDisposition::Complete,
        "definition-only" => PosixDisposition::DefinitionOnly,
        "excluded-upstream-stub" => PosixDisposition::ExcludedUpstreamStub,
        "compile-failed" => PosixDisposition::CompileFailed,
        "link-failed" => PosixDisposition::LinkFailed,
        "not-built-shell-test" => PosixDisposition::NotBuiltShellTest,
        _ => return Err(PosixTestError::UnknownDisposition),
    };
    let valid_kind_disposition = matches!(
        (kind, disposition),
        (
            PosixTestKind::Runnable,
            PosixDisposition::Complete
                | PosixDisposition::ExcludedUpstreamStub
                | PosixDisposition::CompileFailed
                | PosixDisposition::LinkFailed
        ) | (
            PosixTestKind::Definition,
            PosixDisposition::DefinitionOnly
                | PosixDisposition::ExcludedUpstreamStub
                | PosixDisposition::CompileFailed
        ) | (PosixTestKind::Shell, PosixDisposition::NotBuiltShellTest)
    );
    if !valid_kind_disposition {
        return Err(PosixTestError::InvalidKindDisposition);
    }
    let timeout_ms = parse_timeout(fields[7])?;
    if !lower_hex(fields[8], 64) {
        return Err(PosixTestError::InvalidChecksum);
    }

    let binary_path = if disposition == PosixDisposition::Complete {
        let relative = fields[6];
        if relative.len() > POSIX_MANIFEST_MAX_STAGED_PATH_BYTES
            || !posix_test_logic_shared::manifest_atom_valid(relative)
        {
            return Err(PosixTestError::InvalidPath);
        }
        let mut guest_path = String::from("/shared/posixtest/");
        guest_path.push_str(relative);
        if !posix_test_logic_shared::staged_binary_path_valid(&guest_path)
            || fields[8] == EMPTY_SHA256
        {
            return Err(PosixTestError::InvalidPath);
        }
        Some(guest_path)
    } else {
        if fields[6] != "-" || fields[8] != EMPTY_SHA256 {
            return Err(PosixTestError::InvalidPath);
        }
        None
    };

    Ok(PosixManifestTest {
        test_id: String::from(test_id),
        group: String::from(group),
        api: String::from(api),
        kind,
        disposition,
        binary_path,
        timeout_ms,
        sha256: String::from(fields[8]),
    })
}

fn validate_manifest_atom(value: &str, maximum: usize) -> Result<(), PosixTestError> {
    if value.len() > maximum || !posix_test_logic_shared::manifest_atom_valid(value) {
        Err(PosixTestError::InvalidAtom)
    } else {
        Ok(())
    }
}

fn validate_metadata_atom(value: &str) -> Result<(), PosixTestError> {
    if value.is_empty()
        || value.len() > POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES
        || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        Err(PosixTestError::InvalidAtom)
    } else {
        Ok(())
    }
}

fn parse_timeout(value: &str) -> Result<u32, PosixTestError> {
    if value.is_empty()
        || (value.len() > 1 && value.as_bytes()[0] == b'0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(PosixTestError::InvalidTimeout);
    }
    let mut timeout = 0u32;
    for byte in value.bytes() {
        timeout = timeout
            .checked_mul(10)
            .and_then(|current| current.checked_add((byte - b'0') as u32))
            .ok_or(PosixTestError::InvalidTimeout)?;
    }
    if timeout == 0 || timeout > MAX_TIMEOUT_MS {
        Err(PosixTestError::InvalidTimeout)
    } else {
        Ok(timeout)
    }
}

fn build_metadata(
    mut values: [Option<String>; METADATA_KEYS.len()],
) -> Result<PosixManifestMetadata, PosixTestError> {
    let mut take = |index: usize| values[index].take().ok_or(PosixTestError::MissingMetadata);
    Ok(PosixManifestMetadata {
        source: take(0)?,
        revision: take(1)?,
        architecture: take(2)?,
        compiler: take(3)?,
        libc: take(4)?,
        patch_sha256: take(5)?,
        build_results_sha256: take(6)?,
        manifest_sha256: take(7)?,
        smros_commit: take(8)?,
    })
}

fn validate_provenance(metadata: &PosixManifestMetadata) -> Result<(), PosixTestError> {
    if metadata.architecture != "aarch64"
        || !lower_hex(&metadata.revision, 40)
        || !lower_hex(&metadata.smros_commit, 40)
        || !lower_hex(&metadata.patch_sha256, 64)
        || !lower_hex(&metadata.build_results_sha256, 64)
        || !lower_hex(&metadata.manifest_sha256, 64)
    {
        Err(PosixTestError::InvalidProvenance)
    } else {
        Ok(())
    }
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn manifest_checksum_matches(data: &[u8], offset: usize, expected: &str) -> bool {
    // Task 3 defines this as SHA-256 with the value replaced by 64 ASCII zeroes.
    if offset.checked_add(64).is_none() || offset + 64 > data.len() {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(&data[..offset]);
    hasher.update(EMPTY_SHA256.as_bytes());
    hasher.update(&data[offset + 64..]);
    let digest = sha256(hasher);
    let hex = b"0123456789abcdef";
    digest.iter().enumerate().all(|(index, byte)| {
        expected.as_bytes()[index * 2] == hex[(byte >> 4) as usize]
            && expected.as_bytes()[index * 2 + 1] == hex[(byte & 0x0f) as usize]
    })
}

fn sha256(hasher: Sha256) -> [u8; 32] {
    hasher.finish()
}

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    bytes: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            block_len: 0,
            bytes: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.bytes = self.bytes.saturating_add(input.len() as u64);
        if self.block_len != 0 {
            let take = core_compat::cmp::min(64 - self.block_len, input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
        while input.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&input[..64]);
            self.compress(&block);
            input = &input[64..];
        }
        if !input.is_empty() {
            self.block[..input.len()].copy_from_slice(input);
            self.block_len = input.len();
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_len = self.bytes.saturating_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
        } else {
            self.block[self.block_len..56].fill(0);
        }
        self.block[56..].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut schedule = [0u32; 64];
        for (index, bytes) in block.chunks_exact(4).enumerate() {
            schedule[index] = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const KNOWN_TASK3_MANIFEST: &str = concat!(
        "SMROS_POSIX_MANIFEST\t1\n",
        "meta\tsource\thttps://example.test/suite.git\n",
        "meta\trevision\t1111111111111111111111111111111111111111\n",
        "meta\tarchitecture\taarch64\n",
        "meta\tcompiler\tcc\n",
        "meta\tlibc\tlibc\n",
        "meta\tpatch_sha256\t2222222222222222222222222222222222222222222222222222222222222222\n",
        "meta\tbuild_results_sha256\t3333333333333333333333333333333333333333333333333333333333333333\n",
        "meta\tmanifest_sha256\t0fa18bad8c314f3633f768f2115c63e1a6ed7b2fe6d4ebca89ab000f25c8758b\n",
        "meta\tsmros_commit\t4444444444444444444444444444444444444444\n",
        "test\tconformance/interfaces/getpid/1-1.c\tbase\tgetpid\trunnable\tcomplete\tbin/getpid.test\t30000\t5555555555555555555555555555555555555555555555555555555555555555\n",
    );

    fn hex_digest(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let mut output = String::new();
        let hex = b"0123456789abcdef";
        for byte in hasher.finish() {
            output.push(hex[(byte >> 4) as usize] as char);
            output.push(hex[(byte & 0x0f) as usize] as char);
        }
        output
    }

    fn manifest(rows: &[String]) -> Vec<u8> {
        manifest_with_source("https://example.test/suite.git", rows)
    }

    fn manifest_with_source(source: &str, rows: &[String]) -> Vec<u8> {
        let mut text = format!(
            concat!(
                "SMROS_POSIX_MANIFEST\t1\n",
                "meta\tsource\t{}\n",
                "meta\trevision\t1111111111111111111111111111111111111111\n",
                "meta\tarchitecture\taarch64\n",
                "meta\tcompiler\tcc\n",
                "meta\tlibc\tlibc\n",
                "meta\tpatch_sha256\t2222222222222222222222222222222222222222222222222222222222222222\n",
                "meta\tbuild_results_sha256\t3333333333333333333333333333333333333333333333333333333333333333\n",
                "meta\tmanifest_sha256\t{}\n",
                "meta\tsmros_commit\t4444444444444444444444444444444444444444\n",
            ),
            source,
            EMPTY_SHA256
        );
        for row in rows {
            text.push_str(row);
            text.push('\n');
        }
        let digest = hex_digest(text.as_bytes());
        text = text.replacen(EMPTY_SHA256, &digest, 1);
        text.into_bytes()
    }

    fn row(
        id: &str,
        group: &str,
        api: &str,
        kind: &str,
        disposition: &str,
        path: &str,
        timeout: &str,
        checksum: &str,
    ) -> String {
        format!("test\t{id}\t{group}\t{api}\t{kind}\t{disposition}\t{path}\t{timeout}\t{checksum}")
    }

    fn runnable_row(id: &str, path: &str) -> String {
        row(
            id,
            "base",
            "getpid",
            "runnable",
            "complete",
            path,
            "30000",
            &"5".repeat(64),
        )
    }

    #[test]
    fn accepts_task3_manifest_and_sha256_interoperates() {
        let parsed = parse_manifest(KNOWN_TASK3_MANIFEST.as_bytes()).expect("canonical manifest");
        assert_eq!(parsed.tests.len(), 1);
        assert_eq!(
            parsed.tests[0].binary_path.as_deref(),
            Some("/shared/posixtest/bin/getpid.test")
        );
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let tampered = KNOWN_TASK3_MANIFEST.replace("\tbase\t", "\ttime\t");
        assert_eq!(
            parse_manifest(tampered.as_bytes()),
            Err(PosixTestError::ManifestChecksumMismatch)
        );
    }

    #[test]
    fn rejects_invalid_kind_disposition_combinations_with_valid_checksums() {
        let kinds = ["runnable", "definition", "shell"];
        let dispositions = [
            "complete",
            "definition-only",
            "excluded-upstream-stub",
            "compile-failed",
            "link-failed",
            "not-built-shell-test",
        ];
        for kind in kinds {
            for disposition in dispositions {
                let allowed = matches!(
                    (kind, disposition),
                    (
                        "runnable",
                        "complete" | "excluded-upstream-stub" | "compile-failed" | "link-failed"
                    ) | (
                        "definition",
                        "definition-only" | "excluded-upstream-stub" | "compile-failed"
                    ) | ("shell", "not-built-shell-test")
                );
                let (path, checksum) = if disposition == "complete" {
                    ("bin/case.test", "6".repeat(64))
                } else {
                    ("-", String::from(EMPTY_SHA256))
                };
                let bytes = manifest(&[row(
                    "case",
                    "base",
                    "api",
                    kind,
                    disposition,
                    path,
                    "30000",
                    &checksum,
                )]);
                assert_eq!(
                    parse_manifest(&bytes).is_ok(),
                    allowed,
                    "kind/disposition decision differs for {kind}/{disposition}"
                );
            }
        }
    }

    #[test]
    fn rejects_malformed_rows_duplicates_and_unsafe_values() {
        let valid = runnable_row("case-a", "bin/a.test");
        let cases = [
            manifest(&[String::from("bogus\tvalue")]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "bogus",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "bogus",
                "-",
                "1",
                EMPTY_SHA256,
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "../a",
                "1",
                &"5".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "bin/a",
                "0",
                &"5".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"A".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "-",
                "1",
                EMPTY_SHA256,
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "compile-failed",
                "bin/a",
                "1",
                &"5".repeat(64),
            )]),
            manifest(&[valid.clone(), valid.clone()]),
            manifest(&[
                runnable_row("case-a", "bin/same.test"),
                runnable_row("case-b", "bin/same.test"),
            ]),
            manifest(&[
                runnable_row("case-b", "bin/b.test"),
                runnable_row("case-a", "bin/a.test"),
            ]),
        ];
        for bytes in cases {
            assert!(parse_manifest(&bytes).is_err());
        }

        assert_eq!(parse_manifest(&[0xff]), Err(PosixTestError::InvalidUtf8));
        assert!(parse_manifest(KNOWN_TASK3_MANIFEST.replace('\n', "\r\n").as_bytes()).is_err());
        assert!(parse_manifest(KNOWN_TASK3_MANIFEST.trim_end().as_bytes()).is_err());

        let unknown_metadata = KNOWN_TASK3_MANIFEST.replace("meta\tsource\t", "meta\tunknown\t");
        assert!(parse_manifest(unknown_metadata.as_bytes()).is_err());
        let duplicate_metadata = KNOWN_TASK3_MANIFEST.replace(
            "meta\trevision\t1111111111111111111111111111111111111111\n",
            "meta\tsource\tduplicate\n",
        );
        assert!(parse_manifest(duplicate_metadata.as_bytes()).is_err());
        let missing_metadata = KNOWN_TASK3_MANIFEST.replace("meta\tcompiler\tcc\n", "");
        assert!(parse_manifest(missing_metadata.as_bytes()).is_err());
        let metadata_out_of_order = KNOWN_TASK3_MANIFEST.replace(
            "meta\tcompiler\tcc\nmeta\tlibc\tlibc\n",
            "meta\tlibc\tlibc\nmeta\tcompiler\tcc\n",
        );
        assert!(parse_manifest(metadata_out_of_order.as_bytes()).is_err());
    }

    #[test]
    fn rejects_near_limit_tab_row_with_fixed_field_extraction() {
        let row_len = POSIX_MANIFEST_MAX_BYTES - MANIFEST_HEADER.len() - 2;
        let mut row = String::from("test");
        row.push_str(&"\t".repeat(row_len - row.len()));
        assert!(parse_fixed_fields::<9>(&row).is_none());

        let mut data = String::from(MANIFEST_HEADER);
        data.push('\n');
        data.push_str(&row);
        data.push('\n');
        assert_eq!(data.len(), POSIX_MANIFEST_MAX_BYTES);
        assert_eq!(
            parse_manifest(data.as_bytes()),
            Err(PosixTestError::InvalidTestRow)
        );
    }

    #[test]
    fn parses_maximum_long_prefix_rows_and_rejects_duplicate_path() {
        let id_prefix = "i".repeat(190);
        let path_prefix = "p".repeat(180);
        let mut rows = Vec::new();
        for index in 0..POSIX_MANIFEST_MAX_TESTS {
            rows.push(runnable_row(
                &format!("{id_prefix}{index:04}"),
                &format!("bin/{path_prefix}{index:04}.test"),
            ));
        }

        let bytes = manifest(&rows);
        assert!(bytes.len() <= POSIX_MANIFEST_MAX_BYTES);
        assert_eq!(
            parse_manifest(&bytes)
                .expect("maximum canonical manifest")
                .tests
                .len(),
            POSIX_MANIFEST_MAX_TESTS
        );

        let duplicate_index = POSIX_MANIFEST_MAX_TESTS - 1;
        rows[duplicate_index] = runnable_row(
            &format!("{id_prefix}{duplicate_index:04}"),
            &format!("bin/{path_prefix}{:04}.test", duplicate_index - 1),
        );
        assert_eq!(
            parse_manifest(&manifest(&rows)),
            Err(PosixTestError::DuplicateTestPath)
        );
    }

    #[test]
    fn manifest_and_filter_bounds_are_exact() {
        assert!(parse_manifest(&manifest_with_source(
            &"s".repeat(POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES),
            &[]
        ))
        .is_ok());
        assert!(parse_manifest(&manifest_with_source(
            &"s".repeat(POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES + 1),
            &[]
        ))
        .is_err());

        let exact_id = "i".repeat(POSIX_MANIFEST_MAX_TEST_ID_BYTES);
        let exact_group = "g".repeat(POSIX_MANIFEST_MAX_GROUP_BYTES);
        let exact_api = "a".repeat(POSIX_MANIFEST_MAX_API_BYTES);
        let exact_path = format!(
            "bin/{}",
            "p".repeat(POSIX_MANIFEST_MAX_STAGED_PATH_BYTES - 4)
        );
        let bytes = manifest(&[row(
            &exact_id,
            &exact_group,
            &exact_api,
            "runnable",
            "complete",
            &exact_path,
            "1",
            &"5".repeat(64),
        )]);
        assert!(parse_manifest(&bytes).is_ok());

        for invalid in [
            row(
                &"i".repeat(POSIX_MANIFEST_MAX_TEST_ID_BYTES + 1),
                "g",
                "a",
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            ),
            row(
                "i",
                &"g".repeat(POSIX_MANIFEST_MAX_GROUP_BYTES + 1),
                "a",
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            ),
            row(
                "i",
                "g",
                &"a".repeat(POSIX_MANIFEST_MAX_API_BYTES + 1),
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            ),
            row(
                "i",
                "g",
                "a",
                "runnable",
                "complete",
                &format!(
                    "bin/{}",
                    "p".repeat(POSIX_MANIFEST_MAX_STAGED_PATH_BYTES - 3)
                ),
                "1",
                &"5".repeat(64),
            ),
        ] {
            assert!(parse_manifest(&manifest(&[invalid])).is_err());
        }

        let mut maximum_rows = Vec::new();
        for index in 0..POSIX_MANIFEST_MAX_TESTS {
            maximum_rows.push(runnable_row(
                &format!("case-{index:04}"),
                &format!("bin/case-{index:04}.test"),
            ));
        }
        assert!(parse_manifest(&manifest(&maximum_rows)).is_ok());
        maximum_rows.push(runnable_row("case-4096", "bin/case-4096.test"));
        assert_eq!(
            parse_manifest(&manifest(&maximum_rows)),
            Err(PosixTestError::TooManyTests)
        );
        let mut oversized = Vec::new();
        oversized.resize(POSIX_MANIFEST_MAX_BYTES + 1, b'x');
        assert_eq!(
            parse_manifest(&oversized),
            Err(PosixTestError::ManifestTooLarge)
        );

        assert_eq!(parse_filter(&["all"]), Ok(PosixFilter::All));
        assert_eq!(
            parse_filter(&["api", "getpid"]),
            Ok(PosixFilter::Api(String::from("getpid")))
        );
        assert!(parse_filter(&[]).is_err());
        assert!(parse_filter(&["all", "extra"]).is_err());
        assert_eq!(
            parse_filter(&["api", "get"]),
            Ok(PosixFilter::Api(String::from("get")))
        );
        assert!(parse_filter(&["group", "../unsafe"]).is_err());
        let oversized = "x".repeat(POSIX_FILTER_MAX_BYTES + 1);
        assert!(parse_filter(&["test", &oversized]).is_err());
    }
}
