use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

const HOST_SHARE_EMBED_VERSION: &str = "2";
const HOST_SHARE_ARCHIVE: &str = "libsmros_host_share.a";
const HOST_SHARE_HOST_ARCHIVE: &str = "libsmros_host_share_host.a";

const HOST_SHARE_DIR: &str = "host_shared";
const HOST_SHARE_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const FALLBACK_LOGICAL_CPUS: usize = 8;
const MAX_LOGICAL_CPUS: usize = 64;

#[derive(Debug)]
struct SnapshotFile {
    relative: String,
    absolute: PathBuf,
    len: u64,
}

#[derive(Debug)]
struct SkippedFile {
    relative: String,
    reason: &'static str,
    len: u64,
}

fn relative_string(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if text.is_empty() || text.starts_with('/') || text.contains("//") {
        return None;
    }
    for part in text.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
    }
    Some(text.replace('\\', "/"))
}

fn logical_cpu_count() -> usize {
    println!("cargo:rerun-if-env-changed=SMROS_LOGICAL_CPUS");
    let Ok(value) = env::var("SMROS_LOGICAL_CPUS") else {
        return FALLBACK_LOGICAL_CPUS;
    };
    let trimmed = value.trim();
    match trimmed.parse::<usize>() {
        Ok(parsed) if (1..=MAX_LOGICAL_CPUS).contains(&parsed) => parsed,
        _ => {
            println!(
                "cargo:warning=invalid SMROS_LOGICAL_CPUS={value:?}; using {FALLBACK_LOGICAL_CPUS}"
            );
            FALLBACK_LOGICAL_CPUS
        }
    }
}

fn collect_snapshot(
    root: &Path,
    dir: &Path,
    files: &mut Vec<SnapshotFile>,
    dirs: &mut Vec<String>,
    skipped: &mut Vec<SkippedFile>,
    excluded: Option<&Path>,
) {
    println!("cargo:rerun-if-changed={}", dir.display());

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if excluded.is_some_and(|excluded| excluded == path) {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(relative_path) = path.strip_prefix(root) else {
            continue;
        };
        let Some(relative) = relative_string(relative_path) else {
            continue;
        };

        if metadata.is_dir() {
            dirs.push(relative);
            collect_snapshot(root, &path, files, dirs, skipped, excluded);
        } else if metadata.is_file() {
            if metadata.len() > HOST_SHARE_MAX_FILE_BYTES {
                skipped.push(SkippedFile {
                    relative,
                    reason: "too-large",
                    len: metadata.len(),
                });
            } else {
                files.push(SnapshotFile {
                    relative,
                    absolute: path,
                    len: metadata.len(),
                });
            }
        }
    }
}

fn target_architecture(target: &str) -> Option<&'static str> {
    match target {
        "aarch64-unknown-none" => Some("aarch64"),
        "riscv64gc-unknown-none-elf" => Some("riscv64"),
        "x86_64-unknown-none" => Some("x86_64"),
        _ => None,
    }
}

fn selected_posix_stage(manifest_dir: &Path, target: &str) -> Result<Option<PathBuf>, String> {
    println!("cargo:rerun-if-env-changed=SMROS_POSIX_STAGE");
    println!("cargo:rerun-if-env-changed=TARGET");
    let Some(architecture) = target_architecture(target) else {
        return Ok(None);
    };
    let host_share_root = manifest_dir.join(HOST_SHARE_DIR);
    let override_path = env::var_os("SMROS_POSIX_STAGE");
    let stage = match &override_path {
        Some(value) => {
            if value.is_empty() {
                return Err("POSIX stage override SMROS_POSIX_STAGE must not be empty".into());
            }
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                manifest_dir.join(path)
            }
        }
        _ if architecture == "aarch64" => host_share_root.join("posixtest"),
        _ => manifest_dir
            .join("target")
            .join("posix")
            .join(architecture)
            .join("stage"),
    };
    // Publishing a stage must invalidate a previous stage-less build, but a
    // missing `rerun-if-changed` path is always stale and recompiles smros on
    // every invocation. Watch an existing ancestor instead.
    watch_without_missing_path(&stage, manifest_dir);
    if override_path.is_none() && !stage.exists() && !host_share_root.join("posixtest").exists() {
        return Ok(None);
    }
    if !stage.is_dir() {
        return Err(format!(
            "POSIX stage for {architecture} is unavailable at {}; run `python3 -m scripts.posix.cli build --arch {architecture}` or set SMROS_POSIX_STAGE to a valid stage",
            stage.display()
        ));
    }

    let manifest = stage.join("manifest.tsv");
    let contents = fs::read_to_string(&manifest).map_err(|error| {
        format!(
            "POSIX stage for {architecture} has no readable manifest.tsv at {}: {error}",
            manifest.display()
        )
    })?;
    if contents.lines().next() != Some("SMROS_POSIX_MANIFEST\t1") {
        return Err(format!(
            "POSIX stage has invalid manifest schema: {}",
            manifest.display()
        ));
    }
    let architectures: Vec<&str> = contents
        .lines()
        .filter_map(|line| line.strip_prefix("meta\tarchitecture\t"))
        .collect();
    if architectures.len() != 1 || architectures[0] != architecture {
        return Err(format!(
            "POSIX stage architecture mismatch for target {target}: expected {architecture}, found {:?} in {}",
            architectures,
            manifest.display()
        ));
    }
    Ok(Some(stage))
}

fn linker_script_for_target(target: &str) -> Option<&'static str> {
    match target {
        "aarch64-unknown-none" => Some("linker/kernel.ld"),
        "riscv64gc-unknown-none-elf" => Some("linker/kernel-riscv64.ld"),
        "x86_64-unknown-none" => Some("linker/kernel-x86_64.ld"),
        _ => None,
    }
}

fn linker_arg_is_placement_option(argument: &str) -> bool {
    [
        "-Ttext",
        "-Ttext-segment",
        "-Trodata-segment",
        "-Tldata-segment",
        "-Tdata",
        "-Tbss",
    ]
    .iter()
    .any(|option| argument == *option || argument.starts_with(&format!("{option}=")))
}

fn linker_arg_selects_script(argument: &str) -> bool {
    if argument == "-T" || argument == "--script" {
        return true;
    }
    if linker_arg_is_placement_option(argument) {
        return false;
    }
    if argument
        .strip_prefix("-T")
        .is_some_and(|path| !path.is_empty())
        || argument
            .strip_prefix("--script=")
            .is_some_and(|path| !path.is_empty())
    {
        return true;
    }

    argument
        .strip_prefix("-Wl,")
        .is_some_and(|arguments| arguments.split(',').any(linker_arg_selects_script))
}

fn rustflags_select_linker_script(encoded_rustflags: &str) -> bool {
    let mut rustflags = encoded_rustflags.split('\x1f');
    while let Some(flag) = rustflags.next() {
        let codegen_option = if flag == "-C" {
            rustflags.next()
        } else {
            flag.strip_prefix("-C")
        };
        let Some(codegen_option) = codegen_option else {
            continue;
        };

        if let Some(argument) = codegen_option.strip_prefix("link-arg=") {
            if linker_arg_selects_script(argument) {
                return true;
            }
        } else if let Some(arguments) = codegen_option.strip_prefix("link-args=") {
            if arguments.split_whitespace().any(linker_arg_selects_script) {
                return true;
            }
        }
    }
    false
}

fn configure_linker() {
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");

    let target = env::var("TARGET").unwrap();
    let Some(script) = linker_script_for_target(&target) else {
        return;
    };
    println!("cargo:rerun-if-changed={script}");

    let rustflags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if !rustflags_select_linker_script(&rustflags) {
        println!("cargo:rustc-link-arg=-T{script}");
    }
}

fn watch_without_missing_path(path: &Path, manifest_dir: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
        return;
    }
    let target_root = manifest_dir.join("target");
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        if dir == manifest_dir || dir == target_root {
            break;
        }
        if dir.exists() {
            println!("cargo:rerun-if-changed={}", dir.display());
            return;
        }
        cursor = dir.parent();
    }
    if path.starts_with(&target_root) {
        let sentinel = target_root.join("posix");
        let _ = fs::create_dir_all(&sentinel);
        if sentinel.is_dir() {
            println!("cargo:rerun-if-changed={}", sentinel.display());
        }
    }
}

fn write_if_changed(path: &Path, contents: &[u8]) {
    if let Ok(existing) = fs::read(path) {
        if existing == contents {
            return;
        }
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, contents).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn mtime_parts(path: &Path) -> (u64, u32) {
    let meta =
        fs::metadata(path).unwrap_or_else(|error| panic!("metadata {}: {error}", path.display()));
    let modified = meta
        .modified()
        .unwrap_or_else(|error| panic!("mtime {}: {error}", path.display()));
    match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => (duration.as_secs(), duration.subsec_nanos()),
        Err(_) => (0, 0),
    }
}

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    filled: usize,
    bits: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            buffer: [0; 64],
            filled: 0,
            bits: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.bits = self.bits.wrapping_add((data.len() as u64).wrapping_mul(8));
        if self.filled > 0 {
            let take = (64 - self.filled).min(data.len());
            self.buffer[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                let block = self.buffer;
                self.transform(&block);
                self.filled = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.transform(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.filled = data.len();
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let mut block = [0u8; 64];
        block[..self.filled].copy_from_slice(&self.buffer[..self.filled]);
        block[self.filled] = 0x80;
        if self.filled >= 56 {
            self.transform(&block);
            block = [0u8; 64];
        }
        block[56..].copy_from_slice(&self.bits.to_be_bytes());
        self.transform(&block);
        let mut out = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            out[index * 4..(index + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn transform(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a_2f98,
            0x7137_4491,
            0xb5c0_fbcf,
            0xe9b5_dba5,
            0x3956_c25b,
            0x59f1_11f1,
            0x923f_82a4,
            0xab1c_5ed5,
            0xd807_aa98,
            0x1283_5b01,
            0x2431_85be,
            0x550c_7dc3,
            0x72be_5d74,
            0x80de_b1fe,
            0x9bdc_06a7,
            0xc19b_f174,
            0xe49b_69c1,
            0xefbe_4786,
            0x0fc1_9dc6,
            0x240c_a1cc,
            0x2de9_2c6f,
            0x4a74_84aa,
            0x5cb0_a9dc,
            0x76f9_88da,
            0x983e_5152,
            0xa831_c66d,
            0xb003_27c8,
            0xbf59_7fc7,
            0xc6e0_0bf3,
            0xd5a7_9147,
            0x06ca_6351,
            0x1429_2967,
            0x27b7_0a85,
            0x2e1b_2138,
            0x4d2c_6dfc,
            0x5338_0d13,
            0x650a_7354,
            0x766a_0abb,
            0x81c2_c92e,
            0x9272_2c85,
            0xa2bf_e8a1,
            0xa81a_664b,
            0xc24b_8b70,
            0xc76c_51a3,
            0xd192_e819,
            0xd699_0624,
            0xf40e_3585,
            0x106a_a070,
            0x19a4_c116,
            0x1e37_6c08,
            0x2748_774c,
            0x34b0_bcb5,
            0x391c_0cb3,
            0x4ed8_aa4a,
            0x5b9c_ca4f,
            0x682e_6ff3,
            0x748f_82ee,
            0x78a5_636f,
            0x84c8_7814,
            0x8cc7_0208,
            0x90be_fffa,
            0xa450_6ceb,
            0xbef9_a3f7,
            0xc671_78f2,
        ];
        let mut words = [0u32; 64];
        for index in 0..16 {
            let start = index * 4;
            words[index] = u32::from_be_bytes(block[start..start + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
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

fn sha256_self_check() {
    let digest = Sha256::hash_bytes(b"abc");
    const EXPECTED: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    if hex_encode(&digest) != EXPECTED {
        panic!("sha256 self-check failed");
    }
}

impl Sha256 {
    fn hash_bytes(data: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finish()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(buf: &mut Vec<u8>, value: u64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn elf_header(
    machine: u16,
    flags: u32,
    section_offset: u64,
    section_count: u16,
    name_index: u16,
) -> [u8; 64] {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    buf.extend_from_slice(&[0; 8]);
    push_u16(&mut buf, 1);
    push_u16(&mut buf, machine);
    push_u32(&mut buf, 1);
    push_u64(&mut buf, 0);
    push_u64(&mut buf, 0);
    push_u64(&mut buf, section_offset);
    push_u32(&mut buf, flags);
    push_u16(&mut buf, 64);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 64);
    push_u16(&mut buf, section_count);
    push_u16(&mut buf, name_index);
    buf.try_into().unwrap()
}

fn section_header(
    name: u32,
    kind: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entry_size: u64,
) -> [u8; 64] {
    let mut buf = Vec::with_capacity(64);
    push_u32(&mut buf, name);
    push_u32(&mut buf, kind);
    push_u64(&mut buf, flags);
    push_u64(&mut buf, 0);
    push_u64(&mut buf, offset);
    push_u64(&mut buf, size);
    push_u32(&mut buf, link);
    push_u32(&mut buf, info);
    push_u64(&mut buf, align);
    push_u64(&mut buf, entry_size);
    buf.try_into().unwrap()
}

fn elf_symbol(name: u32, info: u8, section: u16, value: u64, size: u64) -> [u8; 24] {
    let mut buf = Vec::with_capacity(24);
    push_u32(&mut buf, name);
    buf.push(info);
    buf.push(0);
    push_u16(&mut buf, section);
    push_u64(&mut buf, value);
    push_u64(&mut buf, size);
    buf.try_into().unwrap()
}

fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

fn write_zeros(file: &mut File, mut count: u64) {
    let zeros = [0u8; 4096];
    while count > 0 {
        let chunk = std::cmp::min(count, zeros.len() as u64) as usize;
        file.write_all(&zeros[..chunk])
            .unwrap_or_else(|error| panic!("write elf padding: {error}"));
        count -= chunk as u64;
    }
}

fn target_elf_machine(target: &str) -> Option<(u16, u32)> {
    match target {
        "aarch64-unknown-none" => Some((183, 0)),
        "riscv64gc-unknown-none-elf" => Some((243, 0x5)),
        "x86_64-unknown-none" => Some((62, 0)),
        _ => None,
    }
}

fn host_elf_machine() -> (u16, u32) {
    match env::consts::ARCH {
        "x86_64" => (62, 0),
        "aarch64" => (183, 0),
        "riscv64" => (243, 0x5),
        other => panic!("host-share probe object is unsupported on {other}"),
    }
}

fn hash_file(path: &Path) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let mut input =
        File::open(path).unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    hasher.finish()
}

struct RecordedFile {
    secs: u64,
    nanos: u32,
    len: u64,
    hash: String,
    relative: String,
}

fn parse_recorded_files(stamp: &str) -> Vec<RecordedFile> {
    let mut recorded = Vec::new();
    for line in stamp.lines() {
        let Some(rest) = line.strip_prefix("F ") else {
            continue;
        };
        let mut parts = rest.splitn(5, ' ');
        let Some(secs) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        let Some(nanos) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        let Some(len) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        let Some(hash) = parts.next() else { continue };
        let Some(relative) = parts.next() else {
            continue;
        };
        recorded.push(RecordedFile {
            secs,
            nanos,
            len,
            hash: hash.to_string(),
            relative: relative.to_string(),
        });
    }
    recorded
}

fn file_hashes(files: &[SnapshotFile], recorded: &[RecordedFile]) -> Vec<[u8; 32]> {
    files
        .iter()
        .map(|file| {
            let (secs, nanos) = mtime_parts(&file.absolute);
            if let Some(previous) = recorded.iter().find(|item| item.relative == file.relative) {
                if previous.len == file.len && previous.secs == secs && previous.nanos == nanos {
                    if let Ok(bytes) = hex_decode(&previous.hash) {
                        return bytes;
                    }
                }
            }
            hash_file(&file.absolute)
        })
        .collect()
}

fn hex_decode(text: &str) -> Result<[u8; 32], ()> {
    if text.len() != 64 {
        return Err(());
    }
    let mut out = [0u8; 32];
    let bytes = text.as_bytes();
    for index in 0..32 {
        let high = hex_nibble(bytes[index * 2])?;
        let low = hex_nibble(bytes[index * 2 + 1])?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(()),
    }
}

fn aggregate_hash(
    target: &str,
    dirs: &[String],
    files: &[SnapshotFile],
    hashes: &[[u8; 32]],
    skipped: &[SkippedFile],
    emit_host: bool,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(HOST_SHARE_EMBED_VERSION.as_bytes());
    hasher.update(&[0]);
    hasher.update(target.as_bytes());
    hasher.update(&[0]);
    hasher.update(if emit_host { b"host" } else { b"cross" });
    hasher.update(&[0]);
    for dir in dirs {
        hasher.update(b"d");
        hasher.update(&[0]);
        hasher.update(dir.as_bytes());
        hasher.update(&[0]);
    }
    for (file, hash) in files.iter().zip(hashes) {
        hasher.update(b"f");
        hasher.update(&[0]);
        hasher.update(file.relative.as_bytes());
        hasher.update(&[0]);
        hasher.update(&file.len.to_le_bytes());
        hasher.update(hash);
    }
    for file in skipped {
        hasher.update(b"s");
        hasher.update(&[0]);
        hasher.update(file.relative.as_bytes());
        hasher.update(&[0]);
        hasher.update(file.reason.as_bytes());
        hasher.update(&[0]);
        hasher.update(&file.len.to_le_bytes());
    }
    hasher.finish()
}

fn stamp_text(
    hash: &[u8; 32],
    target: &str,
    dirs: &[String],
    files: &[SnapshotFile],
    hashes: &[[u8; 32]],
    skipped: &[SkippedFile],
    emit_host: bool,
) -> String {
    let mut text = String::new();
    text.push_str("v");
    text.push_str(HOST_SHARE_EMBED_VERSION);
    text.push(' ');
    text.push_str(&hex_encode(hash));
    text.push('\n');
    text.push_str("T ");
    text.push_str(target);
    text.push('\n');
    text.push_str("H ");
    text.push_str(if emit_host { "1" } else { "0" });
    text.push('\n');
    for dir in dirs {
        text.push_str("D ");
        text.push_str(dir);
        text.push('\n');
    }
    for (file, hash) in files.iter().zip(hashes) {
        let (secs, nanos) = mtime_parts(&file.absolute);
        text.push_str("F ");
        text.push_str(&secs.to_string());
        text.push(' ');
        text.push_str(&nanos.to_string());
        text.push(' ');
        text.push_str(&file.len.to_string());
        text.push(' ');
        text.push_str(&hex_encode(hash));
        text.push(' ');
        text.push_str(&file.relative);
        text.push('\n');
    }
    for file in skipped {
        text.push_str("S ");
        text.push_str(&file.len.to_string());
        text.push(' ');
        text.push_str(file.reason);
        text.push(' ');
        text.push_str(&file.relative);
        text.push('\n');
    }
    text
}

fn stamp_header_hash(stamp: &str) -> Option<&str> {
    let header = stamp.lines().next()?;
    let prefix = format!("v{HOST_SHARE_EMBED_VERSION} ");
    header.strip_prefix(&prefix)
}

fn metadata_lines_match(
    stamp: &str,
    target: &str,
    dirs: &[String],
    skipped: &[SkippedFile],
    emit_host: bool,
) -> bool {
    let mut saw_target = false;
    let mut saw_host = false;
    let mut stamp_dirs = Vec::new();
    let mut stamp_skipped = Vec::new();
    for line in stamp.lines() {
        if let Some(rest) = line.strip_prefix("T ") {
            saw_target = rest == target;
        } else if let Some(rest) = line.strip_prefix("H ") {
            saw_host = rest == if emit_host { "1" } else { "0" };
        } else if let Some(rest) = line.strip_prefix("D ") {
            stamp_dirs.push(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("S ") {
            stamp_skipped.push(rest.to_string());
        }
    }
    let skipped_text: Vec<String> = skipped
        .iter()
        .map(|file| format!("{} {} {}", file.len, file.reason, file.relative))
        .collect();
    saw_target && saw_host && stamp_dirs == dirs && stamp_skipped == skipped_text
}

fn mtimes_match(recorded: &[RecordedFile], files: &[SnapshotFile]) -> bool {
    if recorded.len() != files.len() {
        return false;
    }
    recorded.iter().zip(files).all(|(previous, file)| {
        let (secs, nanos) = mtime_parts(&file.absolute);
        previous.relative == file.relative
            && previous.len == file.len
            && previous.secs == secs
            && previous.nanos == nanos
    })
}

fn outputs_ready(out_dir: &Path, emit_host: bool) -> bool {
    out_dir.join("host_share.rs").is_file()
        && out_dir.join(HOST_SHARE_ARCHIVE).is_file()
        && (!emit_host || out_dir.join(HOST_SHARE_HOST_ARCHIVE).is_file())
}

fn link_host_share_archive(out_dir: &Path) {
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=smros_host_share");
}

fn archive_object(object: &Path, archive: &Path) {
    let tmp = archive.with_extension("a.tmp");
    let _ = fs::remove_file(&tmp);
    let status = Command::new("ar")
        .arg("rcs")
        .arg(&tmp)
        .arg(object)
        .status()
        .unwrap_or_else(|error| panic!("spawn ar: {error}"));
    if !status.success() {
        panic!("ar failed while packing {}", object.display());
    }
    fs::rename(&tmp, archive).unwrap_or_else(|error| {
        panic!("rename {} to {}: {error}", tmp.display(), archive.display())
    });
    let _ = fs::remove_file(object);
}

fn intern_string(raw: &mut Vec<u8>, base: u32, text: &str) -> (u32, u32) {
    let offset = base
        .checked_add(u32::try_from(raw.len()).expect("host share string table exceeds 4 GiB"))
        .expect("host share string offset exceeds 4 GiB");
    let len = u32::try_from(text.len()).expect("host share path exceeds 4 GiB");
    raw.extend_from_slice(text.as_bytes());
    (offset, len)
}

fn write_blob_object(
    path: &Path,
    dirs: &[String],
    files: &[SnapshotFile],
    skipped: &[SkippedFile],
    machine: u16,
    flags: u32,
) {
    let dir_bytes = dirs.len() * 8;
    let file_bytes = files.len() * 16;
    let skip_bytes = skipped.len() * 24;
    let tables_end = 32 + dir_bytes + file_bytes + skip_bytes;
    let raw_base =
        u32::try_from(align_up(tables_end as u64, 8)).expect("host share header exceeds 4 GiB");
    let mut raw = Vec::new();
    let mut dir_entries = Vec::with_capacity(dirs.len());
    for dir in dirs {
        dir_entries.push(intern_string(&mut raw, raw_base, dir));
    }
    let mut file_paths = Vec::with_capacity(files.len());
    for file in files {
        file_paths.push(intern_string(&mut raw, raw_base, &file.relative));
    }
    let mut skipped_entries = Vec::with_capacity(skipped.len());
    for file in skipped {
        let path = intern_string(&mut raw, raw_base, &file.relative);
        let reason = intern_string(&mut raw, raw_base, file.reason);
        skipped_entries.push((path, reason, file.len));
    }
    let data_base = raw_base
        .checked_add(u32::try_from(raw.len()).expect("host share strings exceed 4 GiB"))
        .expect("host share data offset exceeds 4 GiB");
    let mut data_cursor = data_base;
    let mut file_data = Vec::with_capacity(files.len());
    for file in files {
        let len = u32::try_from(file.len).expect("embedded host share file exceeds 4 GiB");
        file_data.push((data_cursor, len));
        data_cursor = data_cursor
            .checked_add(len)
            .expect("host share blob exceeds 4 GiB");
    }
    let total = data_cursor as u64;

    let mut payload_prefix = Vec::with_capacity(tables_end);
    payload_prefix.extend_from_slice(b"SMHS");
    push_u32(&mut payload_prefix, 1);
    push_u64(&mut payload_prefix, total);
    push_u32(&mut payload_prefix, dirs.len() as u32);
    push_u32(&mut payload_prefix, files.len() as u32);
    push_u32(&mut payload_prefix, skipped.len() as u32);
    push_u32(&mut payload_prefix, 0);
    for (offset, len) in dir_entries {
        push_u32(&mut payload_prefix, offset);
        push_u32(&mut payload_prefix, len);
    }
    for ((path_off, path_len), (data_off, data_len)) in file_paths.iter().zip(file_data) {
        push_u32(&mut payload_prefix, *path_off);
        push_u32(&mut payload_prefix, *path_len);
        push_u32(&mut payload_prefix, data_off);
        push_u32(&mut payload_prefix, data_len);
    }
    for ((path_off, path_len), (reason_off, reason_len), size) in skipped_entries {
        push_u32(&mut payload_prefix, path_off);
        push_u32(&mut payload_prefix, path_len);
        push_u32(&mut payload_prefix, reason_off);
        push_u32(&mut payload_prefix, reason_len);
        push_u64(&mut payload_prefix, size);
    }
    assert_eq!(payload_prefix.len(), tables_end);

    let section_names = b"\0.rodata.smros_host_share\0.symtab\0.strtab\0.shstrtab\0";
    let mut symbol_names = Vec::new();
    symbol_names.push(0);
    let symbol_name_offset = symbol_names.len() as u32;
    symbol_names.extend_from_slice(b"SMROS_HS_BLOB\0");
    let ro_off = 64u64;
    let shstr_off = align_up(ro_off + total, 8);
    let str_off = align_up(shstr_off + section_names.len() as u64, 8);
    let sym_off = align_up(str_off + symbol_names.len() as u64, 8);
    let sym_size = 24u64 * 2;
    let section_offset = align_up(sym_off + sym_size, 8);
    let shstr_rodata = 1u32;
    let shstr_symtab = section_names
        .windows(8)
        .position(|window| window == b".symtab\0")
        .unwrap() as u32;
    let shstr_strtab = section_names
        .windows(8)
        .position(|window| window == b".strtab\0")
        .unwrap() as u32;
    let shstr_shstrtab = section_names
        .windows(10)
        .position(|window| window == b".shstrtab\0")
        .unwrap() as u32;

    let mut symbols = Vec::with_capacity(48);
    symbols.extend_from_slice(&elf_symbol(0, 0, 0, 0, 0));
    symbols.extend_from_slice(&elf_symbol(symbol_name_offset, (1 << 4) | 1, 1, 0, total));
    let mut sections = Vec::with_capacity(5 * 64);
    sections.extend_from_slice(&section_header(0, 0, 0, 0, 0, 0, 0, 0, 0));
    sections.extend_from_slice(&section_header(
        shstr_rodata,
        1,
        2,
        ro_off,
        total,
        0,
        0,
        1,
        0,
    ));
    sections.extend_from_slice(&section_header(
        shstr_symtab,
        2,
        0,
        sym_off,
        sym_size,
        3,
        1,
        8,
        24,
    ));
    sections.extend_from_slice(&section_header(
        shstr_strtab,
        3,
        0,
        str_off,
        symbol_names.len() as u64,
        0,
        0,
        1,
        0,
    ));
    sections.extend_from_slice(&section_header(
        shstr_shstrtab,
        3,
        0,
        shstr_off,
        section_names.len() as u64,
        0,
        0,
        1,
        0,
    ));

    let tmp = path.with_extension("o.tmp");
    let mut output =
        File::create(&tmp).unwrap_or_else(|error| panic!("create {}: {error}", tmp.display()));
    output
        .write_all(&elf_header(machine, flags, section_offset, 5, 4))
        .unwrap_or_else(|error| panic!("write elf header: {error}"));
    output
        .write_all(&payload_prefix)
        .unwrap_or_else(|error| panic!("write host share header: {error}"));
    write_zeros(
        &mut output,
        u64::from(raw_base) - payload_prefix.len() as u64,
    );
    output
        .write_all(&raw)
        .unwrap_or_else(|error| panic!("write host share strings: {error}"));
    let mut buffer = [0u8; 1024 * 1024];
    for file in files {
        if file.len == 0 {
            continue;
        }
        let mut input = File::open(&file.absolute)
            .unwrap_or_else(|error| panic!("open {}: {error}", file.absolute.display()));
        let mut remaining = file.len;
        while remaining > 0 {
            let want = std::cmp::min(remaining, buffer.len() as u64) as usize;
            let read = input
                .read(&mut buffer[..want])
                .unwrap_or_else(|error| panic!("read {}: {error}", file.absolute.display()));
            if read == 0 {
                panic!("short read of {}", file.absolute.display());
            }
            output
                .write_all(&buffer[..read])
                .unwrap_or_else(|error| panic!("write host share payload: {error}"));
            remaining -= read as u64;
        }
    }
    let mut cursor = ro_off + total;
    write_zeros(&mut output, shstr_off - cursor);
    output
        .write_all(section_names)
        .unwrap_or_else(|error| panic!("write elf section names: {error}"));
    cursor = shstr_off + section_names.len() as u64;
    write_zeros(&mut output, str_off - cursor);
    output
        .write_all(&symbol_names)
        .unwrap_or_else(|error| panic!("write elf strings: {error}"));
    cursor = str_off + symbol_names.len() as u64;
    write_zeros(&mut output, sym_off - cursor);
    output
        .write_all(&symbols)
        .unwrap_or_else(|error| panic!("write elf symbols: {error}"));
    cursor = sym_off + sym_size;
    write_zeros(&mut output, section_offset - cursor);
    output
        .write_all(&sections)
        .unwrap_or_else(|error| panic!("write elf sections: {error}"));
    output
        .flush()
        .unwrap_or_else(|error| panic!("flush {}: {error}", tmp.display()));
    drop(output);
    fs::rename(&tmp, path)
        .unwrap_or_else(|error| panic!("rename {} to {}: {error}", tmp.display(), path.display()));
}

const HOST_SHARE_RS: &str = r#"// Generated by build.rs. The file list lives in the linked host-share blob.
// This source does not change when host_shared/ or the POSIX stage changes.

use core::ptr::addr_of;
use core::sync::atomic::{AtomicPtr, AtomicU8, AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug)]
pub struct HostShareFile {
    pub path: &'static str,
    pub data: &'static [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct HostShareSkipped {
    pub path: &'static str,
    pub reason: &'static str,
    pub size: usize,
}

pub const HOST_SHARE_ROOT: &str = "host_shared";
pub const HOST_SHARE_MAX_FILE_BYTES: usize = 67108864;

#[derive(Clone, Copy)]
pub struct HostShareFileList;
#[derive(Clone, Copy)]
pub struct HostShareDirList;
#[derive(Clone, Copy)]
pub struct HostShareSkippedList;

extern "C" {
    static SMROS_HS_BLOB: u8;
}

static INIT: AtomicU8 = AtomicU8::new(0);
static FILE_PTR: AtomicPtr<HostShareFile> = AtomicPtr::new(core::ptr::null_mut());
static FILE_LEN: AtomicUsize = AtomicUsize::new(0);
static DIR_PTR: AtomicPtr<&'static str> = AtomicPtr::new(core::ptr::null_mut());
static DIR_LEN: AtomicUsize = AtomicUsize::new(0);
static SKIP_PTR: AtomicPtr<HostShareSkipped> = AtomicPtr::new(core::ptr::null_mut());
static SKIP_LEN: AtomicUsize = AtomicUsize::new(0);

fn load_blob() -> &'static [u8] {
    unsafe {
        let start = addr_of!(SMROS_HS_BLOB);
        let mut magic = [0u8; 4];
        core::ptr::copy_nonoverlapping(start, magic.as_mut_ptr(), 4);
        if &magic != b"SMHS" {
            panic!("host share blob is missing");
        }
        let total = core::ptr::read_unaligned(start.add(8).cast::<u64>()) as usize;
        if total < 32 {
            panic!("host share blob is truncated");
        }
        core::slice::from_raw_parts(start, total)
    }
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    let bytes: [u8; 4] = data[offset..offset + 4].try_into().expect("host share header");
    u32::from_le_bytes(bytes)
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let bytes: [u8; 8] = data[offset..offset + 8].try_into().expect("host share header");
    u64::from_le_bytes(bytes)
}

fn text_at(data: &'static [u8], offset: u32, len: u32) -> &'static str {
    let start = offset as usize;
    let end = start + len as usize;
    core::str::from_utf8(&data[start..end]).expect("host share path is not utf-8")
}

fn bytes_at(data: &'static [u8], offset: u32, len: u32) -> &'static [u8] {
    let start = offset as usize;
    let end = start + len as usize;
    &data[start..end]
}

fn ensure_tables() {
    match INIT.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            install_tables();
            INIT.store(2, Ordering::Release);
        }
        Err(2) => {}
        Err(_) => {
            while INIT.load(Ordering::Acquire) != 2 {
                core::hint::spin_loop();
            }
        }
    }
}

fn install_tables() {
    let data = load_blob();
    if read_u32(data, 4) != 1 {
        panic!("host share blob version is unsupported");
    }
    let dir_count = read_u32(data, 16) as usize;
    let file_count = read_u32(data, 20) as usize;
    let skip_count = read_u32(data, 24) as usize;
    let mut cursor = 32usize;
    let mut dirs = alloc::vec::Vec::with_capacity(dir_count);
    for _ in 0..dir_count {
        let offset = read_u32(data, cursor);
        let len = read_u32(data, cursor + 4);
        cursor += 8;
        dirs.push(text_at(data, offset, len));
    }
    let mut files = alloc::vec::Vec::with_capacity(file_count);
    for _ in 0..file_count {
        let path_off = read_u32(data, cursor);
        let path_len = read_u32(data, cursor + 4);
        let data_off = read_u32(data, cursor + 8);
        let data_len = read_u32(data, cursor + 12);
        cursor += 16;
        files.push(HostShareFile {
            path: text_at(data, path_off, path_len),
            data: bytes_at(data, data_off, data_len),
        });
    }
    let mut skipped = alloc::vec::Vec::with_capacity(skip_count);
    for _ in 0..skip_count {
        let path_off = read_u32(data, cursor);
        let path_len = read_u32(data, cursor + 4);
        let reason_off = read_u32(data, cursor + 8);
        let reason_len = read_u32(data, cursor + 12);
        let size = read_u64(data, cursor + 16);
        cursor += 24;
        skipped.push(HostShareSkipped {
            path: text_at(data, path_off, path_len),
            reason: text_at(data, reason_off, reason_len),
            size: usize::try_from(size).expect("skipped host share file exceeds usize"),
        });
    }
    let dirs = alloc::boxed::Box::leak(dirs.into_boxed_slice());
    let files = alloc::boxed::Box::leak(files.into_boxed_slice());
    let skipped = alloc::boxed::Box::leak(skipped.into_boxed_slice());
    DIR_PTR.store(dirs.as_mut_ptr(), Ordering::Release);
    DIR_LEN.store(dirs.len(), Ordering::Release);
    FILE_PTR.store(files.as_mut_ptr(), Ordering::Release);
    FILE_LEN.store(files.len(), Ordering::Release);
    SKIP_PTR.store(skipped.as_mut_ptr(), Ordering::Release);
    SKIP_LEN.store(skipped.len(), Ordering::Release);
}

fn file_slice() -> &'static [HostShareFile] {
    ensure_tables();
    unsafe { core::slice::from_raw_parts(FILE_PTR.load(Ordering::Acquire), FILE_LEN.load(Ordering::Acquire)) }
}

fn dir_slice() -> &'static [&'static str] {
    ensure_tables();
    unsafe { core::slice::from_raw_parts(DIR_PTR.load(Ordering::Acquire), DIR_LEN.load(Ordering::Acquire)) }
}

fn skipped_slice() -> &'static [HostShareSkipped] {
    ensure_tables();
    unsafe { core::slice::from_raw_parts(SKIP_PTR.load(Ordering::Acquire), SKIP_LEN.load(Ordering::Acquire)) }
}

impl core::ops::Deref for HostShareFileList {
    type Target = [HostShareFile];
    fn deref(&self) -> &Self::Target {
        file_slice()
    }
}

impl IntoIterator for HostShareFileList {
    type Item = &'static HostShareFile;
    type IntoIter = core::slice::Iter<'static, HostShareFile>;
    fn into_iter(self) -> Self::IntoIter {
        file_slice().iter()
    }
}

impl core::ops::Index<usize> for HostShareFileList {
    type Output = HostShareFile;
    fn index(&self, index: usize) -> &Self::Output {
        &file_slice()[index]
    }
}

impl core::ops::Deref for HostShareDirList {
    type Target = [&'static str];
    fn deref(&self) -> &Self::Target {
        dir_slice()
    }
}

impl IntoIterator for HostShareDirList {
    type Item = &'static &'static str;
    type IntoIter = core::slice::Iter<'static, &'static str>;
    fn into_iter(self) -> Self::IntoIter {
        dir_slice().iter()
    }
}

impl core::ops::Deref for HostShareSkippedList {
    type Target = [HostShareSkipped];
    fn deref(&self) -> &Self::Target {
        skipped_slice()
    }
}

impl IntoIterator for HostShareSkippedList {
    type Item = &'static HostShareSkipped;
    type IntoIter = core::slice::Iter<'static, HostShareSkipped>;
    fn into_iter(self) -> Self::IntoIter {
        skipped_slice().iter()
    }
}

pub static HOST_SHARE_DIRS: HostShareDirList = HostShareDirList;
pub static HOST_SHARE_FILES: HostShareFileList = HostShareFileList;
pub static HOST_SHARE_SKIPPED: HostShareSkippedList = HostShareSkippedList;
"#;

fn emit_host_share(
    out_dir: &Path,
    target: &str,
    dirs: &[String],
    files: &[SnapshotFile],
    skipped: &[SkippedFile],
) {
    sha256_self_check();
    println!("cargo:rerun-if-env-changed=SMROS_HOST_SHARE_EMIT_HOST");
    let emit_host = env::var("SMROS_HOST_SHARE_EMIT_HOST").ok().as_deref() == Some("1");
    let stamp_path = out_dir.join("host_share_embed.stamp");
    let existing_stamp = fs::read_to_string(&stamp_path).unwrap_or_default();
    let recorded = parse_recorded_files(&existing_stamp);
    let metadata_ok = metadata_lines_match(&existing_stamp, target, dirs, skipped, emit_host);
    if metadata_ok
        && mtimes_match(&recorded, files)
        && outputs_ready(out_dir, emit_host)
        && stamp_header_hash(&existing_stamp).is_some()
    {
        link_host_share_archive(out_dir);
        return;
    }

    let hashes = file_hashes(files, &recorded);
    let hash = aggregate_hash(target, dirs, files, &hashes, skipped, emit_host);
    let fresh_stamp = stamp_text(&hash, target, dirs, files, &hashes, skipped, emit_host);
    if stamp_header_hash(&existing_stamp) == Some(hex_encode(&hash).as_str())
        && outputs_ready(out_dir, emit_host)
    {
        write_if_changed(&stamp_path, fresh_stamp.as_bytes());
        link_host_share_archive(out_dir);
        return;
    }

    let Some((machine, elf_flags)) = target_elf_machine(target) else {
        panic!("host share embedding does not know ELF machine for target {target}");
    };
    let object = out_dir.join("host_share_blob.o");
    write_blob_object(&object, dirs, files, skipped, machine, elf_flags);
    archive_object(&object, &out_dir.join(HOST_SHARE_ARCHIVE));
    if emit_host {
        let (host_machine, host_flags) = host_elf_machine();
        let host_object = out_dir.join("host_share_blob_host.o");
        write_blob_object(&host_object, dirs, files, skipped, host_machine, host_flags);
        archive_object(&host_object, &out_dir.join(HOST_SHARE_HOST_ARCHIVE));
    }
    write_if_changed(&out_dir.join("host_share.rs"), HOST_SHARE_RS.as_bytes());
    write_if_changed(&stamp_path, fresh_stamp.as_bytes());
    link_host_share_archive(out_dir);
}

fn main() {
    configure_linker();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let host_share_root = manifest_dir.join(HOST_SHARE_DIR);
    watch_without_missing_path(&host_share_root, &manifest_dir);

    let selected_stage =
        selected_posix_stage(&manifest_dir, &target).unwrap_or_else(|error| panic!("{error}"));

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let legacy_stage = host_share_root.join("posixtest");
    if host_share_root.is_dir() {
        collect_snapshot(
            &host_share_root,
            &host_share_root,
            &mut files,
            &mut dirs,
            &mut skipped,
            selected_stage.as_ref().map(|_| legacy_stage.as_path()),
        );
    }

    if let Some(stage) = selected_stage {
        dirs.push("posixtest".into());
        let mut stage_dirs = Vec::new();
        let mut stage_files = Vec::new();
        let mut stage_skipped = Vec::new();
        collect_snapshot(
            &stage,
            &stage,
            &mut stage_files,
            &mut stage_dirs,
            &mut stage_skipped,
            None,
        );
        for dir in stage_dirs {
            dirs.push(format!("posixtest/{dir}"));
        }
        for mut file in stage_files {
            file.relative = format!("posixtest/{}", file.relative);
            files.push(file);
        }
        for mut file in stage_skipped {
            file.relative = format!("posixtest/{}", file.relative);
            skipped.push(file);
        }
    }

    dirs.sort();
    dirs.dedup();
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    skipped.sort_by(|a, b| a.relative.cmp(&b.relative));

    for file in &files {
        println!(
            "cargo:metadata=host_shared include {} ({} bytes)",
            file.relative, file.len
        );
    }
    for file in &skipped {
        println!(
            "cargo:metadata=host_shared skip {}: {} ({} bytes)",
            file.relative, file.reason, file.len
        );
    }
    emit_host_share(&out_dir, &target, &dirs, &files, &skipped);

    let logical_cpus = logical_cpu_count();
    println!("cargo:metadata=SMROS logical CPUs configured: {logical_cpus}");
    let build_config = format!("// Generated by build.rs. Do not edit.\n{logical_cpus}usize\n");
    write_if_changed(&out_dir.join("build_config.rs"), build_config.as_bytes());

    compile_linuxcompat(&manifest_dir, &target);
}

fn compiler_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("-dumpversion")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn path_newer_than(path: &Path, stamp: std::time::SystemTime) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return true;
    };
    if meta.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return true;
        };
        for entry in entries.flatten() {
            if path_newer_than(&entry.path(), stamp) {
                return true;
            }
        }
        return false;
    }
    meta.modified()
        .map(|modified| modified > stamp)
        .unwrap_or(true)
}

fn linuxcompat_archive_fresh(
    lib: &Path,
    src_dir: &Path,
    include_dir: &Path,
    sources: &[&str],
) -> bool {
    let Ok(meta) = fs::metadata(lib) else {
        return false;
    };
    let Ok(stamp) = meta.modified() else {
        return false;
    };
    for source in sources {
        if path_newer_than(&src_dir.join(source), stamp) {
            return false;
        }
    }
    !path_newer_than(&include_dir.join("linux"), stamp)
        && !path_newer_than(&include_dir.join("asm"), stamp)
}

fn compile_linuxcompat(manifest_dir: &Path, target: &str) {
    println!("cargo:rerun-if-changed=src/user_level/drivers/linuxcompat");
    println!("cargo:rerun-if-changed=include/linux");
    println!("cargo:rerun-if-changed=include/asm");
    println!("cargo:rustc-check-cfg=cfg(smros_linuxcompat)");

    let src_dir = manifest_dir.join("src/user_level/drivers/linuxcompat");
    let include_dir = manifest_dir.join("include");
    if !src_dir.join("edu.c").is_file() || !include_dir.join("linux/pci.h").is_file() {
        return;
    }

    let (compiler, extra_flags): (&str, &[&str]) = match target {
        "aarch64-unknown-none" => (
            "aarch64-linux-gnu-gcc",
            &[
                "-mgeneral-regs-only",
                "-mno-outline-atomics",
                "-march=armv8-a",
                "-mabi=lp64",
            ],
        ),
        "x86_64-unknown-none" => (
            "x86_64-linux-gnu-gcc",
            &["-mno-red-zone", "-mno-sse", "-mno-mmx", "-mno-avx"],
        ),
        _ => return,
    };

    if !compiler_exists(compiler) {
        println!("cargo:warning=linuxcompat skipped: {compiler} not found");
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let sources = ["compat.c", "pci.c", "edu.c"];
    let lib = out_dir.join("libsmros_linuxcompat.a");
    if linuxcompat_archive_fresh(&lib, &src_dir, &include_dir, &sources) {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=smros_linuxcompat");
        println!("cargo:rustc-cfg=smros_linuxcompat");
        return;
    }
    let mut objects = Vec::new();
    for source in sources {
        let src = src_dir.join(source);
        let obj = out_dir.join(format!("{source}.o"));
        let mut cmd = std::process::Command::new(compiler);
        cmd.arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .arg("-ffreestanding")
            .arg("-fno-builtin")
            .arg("-fno-stack-protector")
            .arg("-fno-asynchronous-unwind-tables")
            .arg("-fno-unwind-tables")
            .arg("-fno-pic")
            .arg("-fno-pie")
            .arg("-nostdinc")
            .arg("-std=gnu11")
            .arg("-O2")
            .arg("-I")
            .arg(&include_dir);
        for flag in extra_flags {
            cmd.arg(flag);
        }
        let status = cmd
            .status()
            .unwrap_or_else(|error| panic!("spawn {compiler}: {error}"));
        if !status.success() {
            panic!("linuxcompat compile failed for {source}");
        }
        objects.push(obj);
    }

    let archiver = compiler
        .strip_suffix("gcc")
        .map(|prefix| format!("{prefix}ar"))
        .unwrap_or_else(|| "ar".into());
    let mut ar = std::process::Command::new(&archiver);
    ar.arg("crs").arg(&lib);
    for obj in &objects {
        ar.arg(obj);
    }
    let status = ar
        .status()
        .unwrap_or_else(|error| panic!("spawn {archiver}: {error}"));
    if !status.success() {
        panic!("linuxcompat archive failed");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=smros_linuxcompat");
    println!("cargo:rustc-cfg=smros_linuxcompat");
}
