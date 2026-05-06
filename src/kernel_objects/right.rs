//! Capability right profiles for kernel task objects.
//!
//! The bit definitions live in `types.rs`; this module is the policy layer
//! that chooses which rights a user-space process receives.

use core::cell::UnsafeCell;

use super::types::{
    default_rights_for_object, rights_are_valid, ObjectType, Rights, ZxError, ZxResult,
};

pub const MAX_PROCESS_NAME_BYTES: usize = 32;
pub const MAX_PROCESS_RIGHT_CONFIG_ENTRIES: usize = 16;
pub const BOOT_PROCESS_RIGHT_CONFIG_JSON: &str = include_str!("../../config/process_rights.json");

pub const TRUSTED_PROCESS_RIGHTS: u32 = Rights::DefaultProcess as u32;
pub const TRUSTED_ROOT_VMAR_RIGHTS: u32 = Rights::DefaultVmar as u32;
pub const TRUSTED_JOB_RIGHTS: u32 = Rights::DefaultJob as u32;
pub const TRUSTED_THREAD_RIGHTS: u32 = Rights::DefaultThread as u32;

pub const SANDBOX_PROCESS_DENIED_RIGHTS: u32 =
    Rights::ManageProcess as u32 | Rights::SetProperty as u32;
pub const SANDBOX_ROOT_VMAR_DENIED_RIGHTS: u32 = Rights::SetProperty as u32;
pub const SANDBOX_JOB_DENIED_RIGHTS: u32 = Rights::ManageJob as u32
    | Rights::ManageProcess as u32
    | Rights::ManageThread as u32
    | Rights::SetPolicy as u32
    | Rights::SetProperty as u32;
pub const SANDBOX_THREAD_DENIED_RIGHTS: u32 =
    Rights::ManageThread as u32 | Rights::SetProperty as u32;

pub const fn rights_without(rights: u32, denied: u32) -> u32 {
    rights & !denied
}

pub const SANDBOX_PROCESS_RIGHTS: u32 =
    rights_without(TRUSTED_PROCESS_RIGHTS, SANDBOX_PROCESS_DENIED_RIGHTS);
pub const SANDBOX_ROOT_VMAR_RIGHTS: u32 =
    rights_without(TRUSTED_ROOT_VMAR_RIGHTS, SANDBOX_ROOT_VMAR_DENIED_RIGHTS);
pub const SANDBOX_JOB_RIGHTS: u32 =
    rights_without(TRUSTED_JOB_RIGHTS, SANDBOX_JOB_DENIED_RIGHTS);
pub const SANDBOX_THREAD_RIGHTS: u32 =
    rights_without(TRUSTED_THREAD_RIGHTS, SANDBOX_THREAD_DENIED_RIGHTS);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserProcessKind {
    ComponentManager,
    Runner,
    Filesystem,
    UserInit,
    Shell,
    Test,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessRightProfile {
    pub kind: UserProcessKind,
    pub process_rights: u32,
    pub root_vmar_rights: u32,
    pub job_rights: u32,
    pub thread_rights: u32,
}

impl ProcessRightProfile {
    pub const fn new(
        kind: UserProcessKind,
        process_rights: u32,
        root_vmar_rights: u32,
        job_rights: u32,
        thread_rights: u32,
    ) -> Self {
        Self {
            kind,
            process_rights,
            root_vmar_rights,
            job_rights,
            thread_rights,
        }
    }

    pub fn rights_valid(&self) -> bool {
        rights_are_valid(self.process_rights)
            && rights_are_valid(self.root_vmar_rights)
            && rights_are_valid(self.job_rights)
            && rights_are_valid(self.thread_rights)
    }

    pub fn rights_for_object(&self, obj_type: ObjectType) -> u32 {
        match obj_type {
            ObjectType::Process | ObjectType::LinuxProcess => self.process_rights,
            ObjectType::Vmar => self.root_vmar_rights,
            ObjectType::Job => self.job_rights,
            ObjectType::Thread | ObjectType::LinuxThread => self.thread_rights,
            _ => default_rights_for_object(obj_type),
        }
    }
}

pub const fn trusted_process_right_profile(kind: UserProcessKind) -> ProcessRightProfile {
    ProcessRightProfile::new(
        kind,
        TRUSTED_PROCESS_RIGHTS,
        TRUSTED_ROOT_VMAR_RIGHTS,
        TRUSTED_JOB_RIGHTS,
        TRUSTED_THREAD_RIGHTS,
    )
}

pub const fn sandbox_process_right_profile(kind: UserProcessKind) -> ProcessRightProfile {
    ProcessRightProfile::new(
        kind,
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
    )
}

pub const fn process_right_profile_for_kind(kind: UserProcessKind) -> ProcessRightProfile {
    match kind {
        UserProcessKind::ComponentManager | UserProcessKind::Runner | UserProcessKind::Test => {
            trusted_process_right_profile(kind)
        }
        UserProcessKind::Shell | UserProcessKind::UserInit => ProcessRightProfile::new(
            kind,
            TRUSTED_PROCESS_RIGHTS,
            TRUSTED_ROOT_VMAR_RIGHTS,
            SANDBOX_JOB_RIGHTS,
            TRUSTED_THREAD_RIGHTS,
        ),
        UserProcessKind::Filesystem | UserProcessKind::Guest => sandbox_process_right_profile(kind),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessRightConfigEntry {
    pub name: &'static str,
    pub profile: ProcessRightProfile,
}

impl ProcessRightConfigEntry {
    pub const fn new(name: &'static str, profile: ProcessRightProfile) -> Self {
        Self { name, profile }
    }

    pub const fn empty() -> Self {
        Self {
            name: "",
            profile: sandbox_process_right_profile(UserProcessKind::Guest),
        }
    }
}
pub struct ProcessRightConfig {
    entries: [Option<ProcessRightConfigEntry>; MAX_PROCESS_RIGHT_CONFIG_ENTRIES],
    len: usize,
    initialized: bool,
}

impl ProcessRightConfig {
    pub const fn empty() -> Self {
        Self {
            entries: [None; MAX_PROCESS_RIGHT_CONFIG_ENTRIES],
            len: 0,
            initialized: false,
        }
    }

    pub fn install_boot_entries(&mut self, entries: &[ProcessRightConfigEntry]) -> ZxResult {
        if self.initialized {
            return Err(ZxError::ErrBadState);
        }
        if entries.is_empty() || entries.len() > MAX_PROCESS_RIGHT_CONFIG_ENTRIES {
            return Err(ZxError::ErrInvalidArgs);
        }

        let mut i = 0;
        while i < entries.len() {
            if entries[i].name.is_empty() || !entries[i].profile.rights_valid() {
                return Err(ZxError::ErrInvalidArgs);
            }

            let mut j = 0;
            while j < i {
                if entries[j].name == entries[i].name {
                    return Err(ZxError::ErrAlreadyExists);
                }
                j += 1;
            }
            i += 1;
        }

        self.entries = [None; MAX_PROCESS_RIGHT_CONFIG_ENTRIES];
        self.len = entries.len();

        let mut index = 0;
        while index < entries.len() {
            self.entries[index] = Some(entries[index]);
            index += 1;
        }

        self.initialized = true;
        Ok(())
    }

    pub fn initialized(&self) -> bool {
        self.initialized
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn profile_for_name(&self, name: &str) -> Option<ProcessRightProfile> {
        let mut index = 0;
        while index < self.len {
            if let Some(entry) = self.entries[index] {
                if entry.name == name {
                    return Some(entry.profile);
                }
            }
            index += 1;
        }
        None
    }
}

pub struct ParsedProcessRightConfig {
    pub entries: [ProcessRightConfigEntry; MAX_PROCESS_RIGHT_CONFIG_ENTRIES],
    pub len: usize,
}

struct ProcessRightJsonParser {
    input: &'static str,
    pos: usize,
    entries: [ProcessRightConfigEntry; MAX_PROCESS_RIGHT_CONFIG_ENTRIES],
    len: usize,
}

impl ProcessRightJsonParser {
    fn new(input: &'static str) -> Self {
        Self {
            input,
            pos: 0,
            entries: [ProcessRightConfigEntry::empty(); MAX_PROCESS_RIGHT_CONFIG_ENTRIES],
            len: 0,
        }
    }

    fn parse(mut self) -> ZxResult<ParsedProcessRightConfig> {
        self.skip_ws();
        self.expect_byte(b'{')?;

        let mut saw_version = false;
        let mut saw_processes = false;
        loop {
            self.skip_ws();
            if self.consume_byte(b'}') {
                break;
            }

            let key = self.parse_string()?;
            self.expect_byte(b':')?;
            match key {
                "version" => {
                    if saw_version || self.parse_u32()? != 1 {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    saw_version = true;
                }
                "processes" => {
                    if saw_processes {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    self.parse_processes()?;
                    saw_processes = true;
                }
                _ => return Err(ZxError::ErrInvalidArgs),
            }

            self.skip_ws();
            if self.consume_byte(b',') {
                continue;
            }
            self.expect_byte(b'}')?;
            break;
        }

        self.skip_ws();
        if self.pos != self.input.len() || !saw_version || !saw_processes || self.len == 0 {
            return Err(ZxError::ErrInvalidArgs);
        }
        Ok(ParsedProcessRightConfig {
            entries: self.entries,
            len: self.len,
        })
    }

    fn parse_processes(&mut self) -> ZxResult {
        self.expect_byte(b'[')?;
        loop {
            self.skip_ws();
            if self.consume_byte(b']') {
                break;
            }
            if self.len >= MAX_PROCESS_RIGHT_CONFIG_ENTRIES {
                return Err(ZxError::ErrOutOfRange);
            }
            let entry = self.parse_process_entry()?;
            let mut i = 0;
            while i < self.len {
                if self.entries[i].name == entry.name {
                    return Err(ZxError::ErrAlreadyExists);
                }
                i += 1;
            }
            self.entries[self.len] = entry;
            self.len += 1;

            self.skip_ws();
            if self.consume_byte(b',') {
                continue;
            }
            self.expect_byte(b']')?;
            break;
        }
        Ok(())
    }

    fn parse_process_entry(&mut self) -> ZxResult<ProcessRightConfigEntry> {
        self.expect_byte(b'{')?;
        let mut name = "";
        let mut kind = None;
        let mut process_rights = None;
        let mut root_vmar_rights = None;
        let mut job_rights = None;
        let mut thread_rights = None;

        loop {
            self.skip_ws();
            if self.consume_byte(b'}') {
                break;
            }

            let key = self.parse_string()?;
            self.expect_byte(b':')?;
            match key {
                "name" => {
                    if !name.is_empty() {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    name = self.parse_string()?;
                    if name.is_empty() || name.len() > MAX_PROCESS_NAME_BYTES {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                }
                "kind" => {
                    if kind.is_some() {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    kind = Some(parse_user_process_kind(self.parse_string()?)?);
                }
                "process_rights" => {
                    if process_rights.is_some() {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    process_rights = Some(self.parse_right_array()?);
                }
                "root_vmar_rights" => {
                    if root_vmar_rights.is_some() {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    root_vmar_rights = Some(self.parse_right_array()?);
                }
                "job_rights" => {
                    if job_rights.is_some() {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    job_rights = Some(self.parse_right_array()?);
                }
                "thread_rights" => {
                    if thread_rights.is_some() {
                        return Err(ZxError::ErrInvalidArgs);
                    }
                    thread_rights = Some(self.parse_right_array()?);
                }
                _ => return Err(ZxError::ErrInvalidArgs),
            }

            self.skip_ws();
            if self.consume_byte(b',') {
                continue;
            }
            self.expect_byte(b'}')?;
            break;
        }

        let profile = ProcessRightProfile::new(
            kind.ok_or(ZxError::ErrInvalidArgs)?,
            process_rights.ok_or(ZxError::ErrInvalidArgs)?,
            root_vmar_rights.ok_or(ZxError::ErrInvalidArgs)?,
            job_rights.ok_or(ZxError::ErrInvalidArgs)?,
            thread_rights.ok_or(ZxError::ErrInvalidArgs)?,
        );
        if name.is_empty() || !profile.rights_valid() {
            return Err(ZxError::ErrInvalidArgs);
        }

        Ok(ProcessRightConfigEntry::new(name, profile))
    }

    fn parse_right_array(&mut self) -> ZxResult<u32> {
        self.expect_byte(b'[')?;
        let mut rights = Rights::None as u32;
        let mut saw_right = false;
        loop {
            self.skip_ws();
            if self.consume_byte(b']') {
                break;
            }

            let right = parse_right_name(self.parse_string()?)?;
            if (rights & right) != 0 {
                return Err(ZxError::ErrAlreadyExists);
            }
            rights |= right;
            saw_right = true;

            self.skip_ws();
            if self.consume_byte(b',') {
                continue;
            }
            self.expect_byte(b']')?;
            break;
        }

        if !saw_right || !rights_are_valid(rights) {
            return Err(ZxError::ErrInvalidArgs);
        }
        Ok(rights)
    }

    fn parse_u32(&mut self) -> ZxResult<u32> {
        self.skip_ws();
        let bytes = self.input.as_bytes();
        let mut value = 0u32;
        let mut saw_digit = false;
        while self.pos < bytes.len() {
            let byte = bytes[self.pos];
            if !byte.is_ascii_digit() {
                break;
            }
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add((byte - b'0') as u32))
                .ok_or(ZxError::ErrOutOfRange)?;
            saw_digit = true;
            self.pos += 1;
        }
        if saw_digit {
            Ok(value)
        } else {
            Err(ZxError::ErrInvalidArgs)
        }
    }

    fn parse_string(&mut self) -> ZxResult<&'static str> {
        self.skip_ws();
        self.expect_byte(b'"')?;
        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            match bytes[self.pos] {
                b'"' => {
                    let value = &self.input[start..self.pos];
                    self.pos += 1;
                    return Ok(value);
                }
                b'\\' => return Err(ZxError::ErrInvalidArgs),
                byte if byte < 0x20 => return Err(ZxError::ErrInvalidArgs),
                _ => self.pos += 1,
            }
        }
        Err(ZxError::ErrInvalidArgs)
    }

    fn expect_byte(&mut self, expected: u8) -> ZxResult {
        self.skip_ws();
        if self.consume_byte(expected) {
            Ok(())
        } else {
            Err(ZxError::ErrInvalidArgs)
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.input.as_bytes().get(self.pos).copied() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        while let Some(byte) = self.input.as_bytes().get(self.pos).copied() {
            if matches!(byte, b' ' | b'\n' | b'\r' | b'\t') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }
}

fn parse_user_process_kind(name: &str) -> ZxResult<UserProcessKind> {
    match name {
        "component_manager" => Ok(UserProcessKind::ComponentManager),
        "runner" => Ok(UserProcessKind::Runner),
        "filesystem" => Ok(UserProcessKind::Filesystem),
        "user_init" => Ok(UserProcessKind::UserInit),
        "shell" => Ok(UserProcessKind::Shell),
        "test" => Ok(UserProcessKind::Test),
        "guest" => Ok(UserProcessKind::Guest),
        _ => Err(ZxError::ErrInvalidArgs),
    }
}

fn parse_right_name(name: &str) -> ZxResult<u32> {
    match name {
        "duplicate" => Ok(Rights::Duplicate as u32),
        "transfer" => Ok(Rights::Transfer as u32),
        "read" => Ok(Rights::Read as u32),
        "write" => Ok(Rights::Write as u32),
        "execute" => Ok(Rights::Execute as u32),
        "map" => Ok(Rights::Map as u32),
        "get_property" => Ok(Rights::GetProperty as u32),
        "set_property" => Ok(Rights::SetProperty as u32),
        "enumerate" => Ok(Rights::Enumerate as u32),
        "destroy" => Ok(Rights::Destroy as u32),
        "set_policy" => Ok(Rights::SetPolicy as u32),
        "get_policy" => Ok(Rights::GetPolicy as u32),
        "signal" => Ok(Rights::Signal as u32),
        "signal_peer" => Ok(Rights::SignalPeer as u32),
        "wait" => Ok(Rights::Wait as u32),
        "inspect" => Ok(Rights::Inspect as u32),
        "manage_job" => Ok(Rights::ManageJob as u32),
        "manage_process" => Ok(Rights::ManageProcess as u32),
        "manage_thread" => Ok(Rights::ManageThread as u32),
        "apply_profile" => Ok(Rights::ApplyProfile as u32),
        "manage_socket" => Ok(Rights::ManageSocket as u32),
        "op_children" => Ok(Rights::OpChildren as u32),
        "resize" => Ok(Rights::Resize as u32),
        "attach_vmo" => Ok(Rights::AttachVmo as u32),
        "manage_vmo" => Ok(Rights::ManageVmo as u32),
        _ => Err(ZxError::ErrInvalidArgs),
    }
}

pub fn parse_process_right_config_json(
    json: &'static str,
) -> ZxResult<ParsedProcessRightConfig> {
    ProcessRightJsonParser::new(json).parse()
}

struct ProcessRightConfigCell(UnsafeCell<ProcessRightConfig>);

unsafe impl Sync for ProcessRightConfigCell {}

impl ProcessRightConfigCell {
    const fn new(config: ProcessRightConfig) -> Self {
        Self(UnsafeCell::new(config))
    }

    fn get(&self) -> *mut ProcessRightConfig {
        self.0.get()
    }
}

static PROCESS_RIGHT_CONFIG: ProcessRightConfigCell =
    ProcessRightConfigCell::new(ProcessRightConfig::empty());

pub fn process_right_config() -> &'static ProcessRightConfig {
    unsafe { &*PROCESS_RIGHT_CONFIG.get() }
}

fn process_right_config_mut() -> &'static mut ProcessRightConfig {
    unsafe { &mut *PROCESS_RIGHT_CONFIG.get() }
}

pub fn configure_process_rights_at_boot(entries: &[ProcessRightConfigEntry]) -> ZxResult {
    process_right_config_mut().install_boot_entries(entries)
}

pub fn init_boot_right_config() -> ZxResult {
    if process_right_config().initialized() {
        return Ok(());
    }
    let parsed = parse_process_right_config_json(BOOT_PROCESS_RIGHT_CONFIG_JSON)?;
    configure_process_rights_at_boot(&parsed.entries[..parsed.len])
}

pub fn process_right_config_initialized() -> bool {
    process_right_config().initialized()
}

pub fn user_process_kind_for_name(name: &str) -> UserProcessKind {
    match name {
        "component_mgr" | "component_manager" => UserProcessKind::ComponentManager,
        "elf_runner" | "runner" => UserProcessKind::Runner,
        "fxfs" | "filesystem" | "smros-fxfs" => UserProcessKind::Filesystem,
        "user_init" | "user-init" | "init" => UserProcessKind::UserInit,
        "shell" | "user_shell" => UserProcessKind::Shell,
        "test" | "smros-test" | "zircon_proc" => UserProcessKind::Test,
        _ => UserProcessKind::Guest,
    }
}

pub fn canonical_process_name(kind: UserProcessKind) -> &'static str {
    match kind {
        UserProcessKind::ComponentManager => "component_mgr",
        UserProcessKind::Runner => "elf_runner",
        UserProcessKind::Filesystem => "fxfs",
        UserProcessKind::UserInit => "user_init",
        UserProcessKind::Shell => "shell",
        UserProcessKind::Test => "zircon_proc",
        UserProcessKind::Guest => "guest",
    }
}

pub fn canonical_process_name_for_user_name(name: &str) -> &'static str {
    canonical_process_name(user_process_kind_for_name(name))
}

pub fn process_right_profile_for_name_checked(name: &str) -> ZxResult<ProcessRightProfile> {
    let config = process_right_config();
    if !config.initialized() {
        return Err(ZxError::ErrBadState);
    }
    if let Some(profile) = config.profile_for_name(name) {
        return Ok(profile);
    }
    config
        .profile_for_name("guest")
        .ok_or(ZxError::ErrBadState)
}

pub fn process_right_profile_for_name(name: &str) -> ProcessRightProfile {
    match process_right_profile_for_name_checked(name) {
        Ok(profile) => profile,
        Err(_) => process_right_profile_for_kind(user_process_kind_for_name(name)),
    }
}
