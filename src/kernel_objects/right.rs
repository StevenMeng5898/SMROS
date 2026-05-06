//! Capability right profiles for kernel task objects.
//!
//! The bit definitions live in `types.rs`; this module is the policy layer
//! that chooses which rights a user-space process receives.

use super::types::{default_rights_for_object, rights_are_valid, ObjectType, Rights};

pub const MAX_PROCESS_NAME_BYTES: usize = 32;

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
        UserProcessKind::Filesystem | UserProcessKind::Guest => {
            sandbox_process_right_profile(kind)
        }
    }
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

pub fn process_right_profile_for_name(name: &str) -> ProcessRightProfile {
    process_right_profile_for_kind(user_process_kind_for_name(name))
}

