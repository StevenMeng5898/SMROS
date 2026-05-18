//! Docker/runc compatibility bring-up for SMROS userspace.
//!
//! This is the next step after the raw syscall smoke test: install a compact
//! OCI-style bundle, parse its `config.json` subset, create the runtime task
//! with boot-time capability rights, and apply the Linux container surfaces
//! that runc/containerd expect first.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel_objects::right;
use crate::kernel_objects::{Rights, ZxError};
use crate::syscall::{self, SysError};
use crate::user_level::{fxfs, net};

const AT_FDCWD: usize = usize::MAX - 99;
const CLONE_NEWNS: usize = 0x0002_0000;
const CLONE_NEWCGROUP: usize = 0x0200_0000;
const CLONE_NEWUTS: usize = 0x0400_0000;
const CLONE_NEWIPC: usize = 0x0800_0000;
const CLONE_NEWUSER: usize = 0x1000_0000;
const CLONE_NEWPID: usize = 0x2000_0000;
const CLONE_NEWNET: usize = 0x4000_0000;
const DOCKER_NS_FLAGS: usize = CLONE_NEWNS
    | CLONE_NEWCGROUP
    | CLONE_NEWUTS
    | CLONE_NEWIPC
    | CLONE_NEWUSER
    | CLONE_NEWPID
    | CLONE_NEWNET;
const MS_RDONLY: usize = 0x1;
const MS_NOSUID: usize = 0x2;
const MS_NODEV: usize = 0x4;
const MS_NOEXEC: usize = 0x8;
const MS_BIND: usize = 0x1000;
const MS_REC: usize = 0x4000;
const MS_PRIVATE: usize = 0x40000;
const PR_SET_NO_NEW_PRIVS: usize = 38;
const PR_GET_NO_NEW_PRIVS: usize = 39;
const SECCOMP_SET_MODE_FILTER: usize = 1;
const SECCOMP_FILTER_FLAG_TSYNC: usize = 1;
const CAP_VERSION_3: u32 = 0x2008_0522;
const O_WRONLY_CREATE_TRUNC: usize = 0o1 | 0o100 | 0o1000;
const O_DIRECTORY: usize = 0o200000;

const OCI_BUNDLE_DIR: &str = "/oci/docker-smoke";
const OCI_CONFIG_PATH: &str = "/oci/docker-smoke/config.json";
const OCI_ROOTFS_DIR: &str = "/oci/docker-smoke/rootfs";
const OCI_ROOTFS_SH: &str = "/oci/docker-smoke/rootfs/bin/sh";
const OCI_CONFIG_MAX_BYTES: usize = 4096;
const RUNC_PROCESS_NAME: &[u8] = b"runc";
const RUNC_THREAD_NAME: &[u8] = b"runc-main";
const RUNC_ENTRY_POINT: usize = 0x1000;
const RUNC_STACK_TOP: usize = 0x8000;
const DOCKER_IMAGE_ROOT: &str = "/docker/images";
const DOCKER_CONTAINER_ROOT: &str = "/docker/containers";
const DOCKER_IMAGE_META_FILE: &str = "image.meta";
const SAMPLE_IMAGE_NAME: &str = "smros/hello:latest";
const SAMPLE_IMAGE_ALIAS: &str = "hello-world:latest";
const SAMPLE_IMAGE_SHORT: &str = "smros/hello";
const SAMPLE_IMAGE_DIR: &str = "/docker/images/smros_hello_latest";
const SAMPLE_IMAGE_ROOTFS: &str = "/docker/images/smros_hello_latest/rootfs";
const SAMPLE_IMAGE_MANIFEST: &str = "/docker/images/smros_hello_latest/manifest.json";
const SAMPLE_IMAGE_CONFIG: &str = "/docker/images/smros_hello_latest/config.json";
const DOCKER_IMAGE_CONFIG_MAX_BYTES: usize = 4096;
const DOCKER_PULL_MAX_BYTES: usize = 64 * 1024 * 1024;
const DOCKER_HTTP_RESPONSE_MAX_BYTES: usize = 64 * 1024 * 1024;
const TAR_BLOCK_BYTES: usize = 512;
const DOCKER_CONTAINER_RECORD_MAX_BYTES: usize = 2048;
const DOCKER_CONTAINER_LOG_MAX_BYTES: usize = 1024;
const DOCKER_MAX_CONTAINER_NAME_BYTES: usize = 48;
const DOCKER_MAX_COMMAND_ITEMS: usize = 16;
const DOCKER_MAX_CONTAINERS: usize = 32;
const DOCKER_MAX_IMAGES: usize = 32;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const APPARMOR_CURRENT: &str = "/proc/self/attr/current";
const APPARMOR_EXEC: &str = "/proc/self/attr/exec";
const APPARMOR_EXEC_CSTR: &[u8] = b"/proc/self/attr/exec\0";
const ROOT_PATH: &[u8] = b"/\0";
const OLD_ROOT_PATH: &[u8] = b"/tmp\0";
const PROC_PATH: &[u8] = b"/proc\0";
const DEV_PATH: &[u8] = b"/dev\0";
const TMP_PATH: &[u8] = b"/tmp\0";
const PROC_FS: &[u8] = b"proc\0";
const TMPFS: &[u8] = b"tmpfs\0";
const DOMAIN: &[u8] = b"container.local";
const CGROUP_PAYLOAD: &[u8] = b"1\n";

const OCI_SAMPLE_CONFIG_JSON: &str = r#"{
  "ociVersion": "1.1.0",
  "root": { "path": "rootfs", "readonly": true },
  "process": {
    "terminal": false,
    "cwd": "/",
    "args": ["/bin/sh", "-c", "echo SMROS OCI runc bundle"],
    "env": ["PATH=/usr/sbin:/usr/bin:/sbin:/bin", "container=oci"],
    "noNewPrivileges": true,
    "apparmorProfile": "docker-default",
    "capabilities": {
      "bounding": ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER", "CAP_FSETID", "CAP_KILL", "CAP_SETGID", "CAP_SETUID", "CAP_SETPCAP", "CAP_NET_BIND_SERVICE", "CAP_NET_RAW", "CAP_SYS_CHROOT", "CAP_MKNOD", "CAP_AUDIT_WRITE", "CAP_SETFCAP"],
      "effective": ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER", "CAP_FSETID", "CAP_KILL", "CAP_SETGID", "CAP_SETUID", "CAP_SETPCAP", "CAP_NET_BIND_SERVICE", "CAP_NET_RAW", "CAP_SYS_CHROOT", "CAP_MKNOD", "CAP_AUDIT_WRITE", "CAP_SETFCAP"],
      "permitted": ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER", "CAP_FSETID", "CAP_KILL", "CAP_SETGID", "CAP_SETUID", "CAP_SETPCAP", "CAP_NET_BIND_SERVICE", "CAP_NET_RAW", "CAP_SYS_CHROOT", "CAP_MKNOD", "CAP_AUDIT_WRITE", "CAP_SETFCAP"],
      "inheritable": []
    }
  },
  "hostname": "smros-docker",
  "mounts": [
    { "destination": "/proc", "type": "proc", "source": "proc", "options": ["nosuid", "noexec", "nodev"] },
    { "destination": "/dev", "type": "tmpfs", "source": "tmpfs", "options": ["nosuid", "mode=755", "size=65536k"] },
    { "destination": "/tmp", "type": "bind", "source": "/", "options": ["rbind", "rw"] }
  ],
  "linux": {
    "cgroupsPath": "docker-smoke",
    "namespaces": [
      { "type": "mount" },
      { "type": "cgroup" },
      { "type": "uts" },
      { "type": "ipc" },
      { "type": "user" },
      { "type": "pid" },
      { "type": "network" }
    ],
    "maskedPaths": ["/proc/kcore", "/proc/keys", "/proc/latency_stats", "/proc/timer_list", "/proc/sched_debug", "/proc/scsi"],
    "readonlyPaths": ["/proc/asound", "/proc/bus", "/proc/fs", "/proc/irq", "/proc/sys", "/proc/sysrq-trigger"],
    "seccomp": { "defaultAction": "SCMP_ACT_ERRNO", "architectures": ["SCMP_ARCH_AARCH64"], "syscalls": [] }
  },
  "annotations": { "org.opencontainers.image.title": "smros-runc-smoke" }
}
"#;

const SAMPLE_IMAGE_MANIFEST_JSON: &str = r#"{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "config": {
    "mediaType": "application/vnd.oci.image.config.v1+json",
    "digest": "sha256:smros-hello-config",
    "size": 512
  },
  "layers": [
    {
      "mediaType": "application/vnd.oci.image.layer.v1.tar",
      "digest": "sha256:smros-hello-rootfs",
      "size": 128
    }
  ]
}
"#;

const SAMPLE_IMAGE_CONFIG_JSON: &str = r#"{
  "created": "2026-05-06T00:00:00Z",
  "architecture": "arm64",
  "os": "linux",
  "config": {
    "Env": ["PATH=/usr/sbin:/usr/bin:/sbin:/bin", "container=docker"],
    "Entrypoint": ["/bin/sh"],
    "Cmd": ["-c", "echo SMROS local Docker image"],
    "WorkingDir": "/",
    "Labels": { "org.opencontainers.image.title": "smros/hello" }
  },
  "rootfs": {
    "type": "layers",
    "diff_ids": ["sha256:smros-hello-rootfs"]
  },
  "history": [
    { "created_by": "SMROS built-in image seed" }
  ]
}
"#;

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

#[derive(Clone, Copy)]
struct OciRuntimeRequest<'a> {
    root_path: &'a str,
    arg0: &'a str,
    args: usize,
    env: usize,
    hostname: &'a str,
    cgroups_path: &'a str,
    apparmor_profile: &'a str,
    namespace_flags: usize,
    mount_count: usize,
    masked_paths: usize,
    readonly_paths: usize,
    cap_effective: u64,
    cap_permitted: u64,
    no_new_privs: bool,
    seccomp_filter: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockerCompatError {
    FxfsInit,
    FxfsPrepare,
    OciInstall,
    OciRead,
    OciParse,
    RightsConfig(ZxError),
    RuntimeJob(ZxError),
    RuntimeProcess(ZxError),
    RuntimeThread(ZxError),
    RuntimeStart(ZxError),
    Namespace(SysError),
    Mount(SysError),
    PivotRoot(SysError),
    Chroot(SysError),
    Uts(SysError),
    NoNewPrivs(SysError),
    Seccomp(SysError),
    CapGet(SysError),
    CapSet(SysError),
    CgroupOpen(SysError),
    CgroupWrite(SysError),
    CgroupClose(SysError),
    AppArmorOpen(SysError),
    AppArmorWrite(SysError),
    AppArmorClose(SysError),
    Network(net::NetError),
    ImageNotFound,
    ImageInvalid,
    ArchiveInvalid,
    ArchiveUnsupported,
    RegistryUnsupported,
    ContainerExists,
    ContainerNotFound,
    ContainerInvalid,
    ContainerState,
    StateMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockerCompatResult {
    pub namespace_flags: usize,
    pub mount_count: usize,
    pub seccomp_mode: usize,
    pub seccomp_filters: usize,
    pub cap_effective: u64,
    pub cgroup_bytes: usize,
    pub apparmor_bytes: usize,
    pub oci_config_bytes: usize,
    pub oci_mounts: usize,
    pub oci_args: usize,
    pub oci_env: usize,
    pub masked_paths: usize,
    pub readonly_paths: usize,
    pub job_handle: u32,
    pub process_handle: u32,
    pub thread_handle: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerImageInfo {
    pub name: String,
    pub rootfs: String,
    pub manifest_bytes: usize,
    pub config_bytes: usize,
    pub layers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockerImageSource {
    Builtin,
    Archive,
    HttpArchive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerImageLoadResult {
    pub image: DockerImageInfo,
    pub source: DockerImageSource,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DockerImageRecord {
    name: String,
    rootfs: String,
    manifest_path: String,
    config_path: String,
    layers: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerContainer {
    pub id: String,
    pub image: String,
    pub command: String,
    pub args: String,
    pub status: DockerContainerStatus,
    pub exit_code: i32,
    pub runtime: Option<DockerCompatResult>,
    pub log_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockerContainerStatus {
    Created,
    Running,
    Exited,
}

impl DockerContainerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DockerContainerStatus::Created => "created",
            DockerContainerStatus::Running => "running",
            DockerContainerStatus::Exited => "exited",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerRunResult {
    pub container: DockerContainer,
    pub runtime: DockerCompatResult,
}

pub fn run_docker_runtime_port() -> Result<DockerCompatResult, DockerCompatError> {
    install_sample_oci_bundle()?;

    let mut config_bytes = [0u8; OCI_CONFIG_MAX_BYTES];
    let config_len = fxfs::read_file(OCI_CONFIG_PATH, &mut config_bytes)
        .map_err(|_| DockerCompatError::OciRead)?;
    let config = core::str::from_utf8(&config_bytes[..config_len])
        .map_err(|_| DockerCompatError::OciParse)?;
    let request = parse_oci_runtime_config(config)?;
    run_oci_runtime_request(&request, config_len)
}

pub fn smoke_test() -> bool {
    run_docker_runtime_port().is_ok()
}

pub fn install_builtin_docker_images() -> Result<(), DockerCompatError> {
    if !fxfs::init() {
        return Err(DockerCompatError::FxfsInit);
    }
    let _ = fxfs::create_dir("/docker");
    let _ = fxfs::create_dir(DOCKER_IMAGE_ROOT);
    let _ = fxfs::create_dir(DOCKER_CONTAINER_ROOT);
    let _ = fxfs::create_dir(SAMPLE_IMAGE_DIR);
    let _ = fxfs::create_dir(SAMPLE_IMAGE_ROOTFS);
    let _ = fxfs::create_dir("/docker/images/smros_hello_latest/rootfs/bin");
    let _ = fxfs::create_dir("/docker/images/smros_hello_latest/rootfs/proc");
    let _ = fxfs::create_dir("/docker/images/smros_hello_latest/rootfs/dev");
    let _ = fxfs::create_dir("/docker/images/smros_hello_latest/rootfs/tmp");
    fxfs::write_file(
        "/docker/images/smros_hello_latest/rootfs/bin/sh",
        b"#!/bin/sh\necho SMROS local Docker image\n",
    )
    .map_err(|_| DockerCompatError::OciInstall)?;
    fxfs::write_file(SAMPLE_IMAGE_MANIFEST, SAMPLE_IMAGE_MANIFEST_JSON.as_bytes())
        .map_err(|_| DockerCompatError::OciInstall)?;
    fxfs::write_file(SAMPLE_IMAGE_CONFIG, SAMPLE_IMAGE_CONFIG_JSON.as_bytes())
        .map_err(|_| DockerCompatError::OciInstall)?;
    let record = DockerImageRecord {
        name: String::from(SAMPLE_IMAGE_NAME),
        rootfs: String::from(SAMPLE_IMAGE_ROOTFS),
        manifest_path: String::from(SAMPLE_IMAGE_MANIFEST),
        config_path: String::from(SAMPLE_IMAGE_CONFIG),
        layers: 1,
    };
    write_image_record(&record)?;
    prune_invalid_container_entries();
    Ok(())
}

pub fn builtin_image_info() -> Result<DockerImageInfo, DockerCompatError> {
    install_builtin_docker_images()?;
    image_info(SAMPLE_IMAGE_NAME)
}

pub fn list_docker_images() -> Result<Vec<DockerImageInfo>, DockerCompatError> {
    install_builtin_docker_images()?;
    let entries = fxfs::entries(DOCKER_IMAGE_ROOT).map_err(|_| DockerCompatError::FxfsPrepare)?;
    let mut images = Vec::new();
    for entry in entries {
        if images.len() >= DOCKER_MAX_IMAGES {
            break;
        }
        let dir = docker_image_dir_path(entry.name.as_str());
        let meta = docker_image_meta_path_from_dir(dir.as_str());
        if !fxfs::exists(meta.as_str()) {
            continue;
        }
        if let Ok(record) = load_image_record_from_dir(dir.as_str()) {
            if let Ok(info) = image_info_from_record(&record) {
                images.push(info);
            }
        }
    }
    Ok(images)
}

pub fn load_docker_image(path: &str) -> Result<DockerImageLoadResult, DockerCompatError> {
    install_builtin_docker_images()?;
    if !path.starts_with('/') || path.as_bytes().contains(&0) {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    let attrs = fxfs::attrs(path).map_err(|_| DockerCompatError::OciRead)?;
    if attrs.size == 0 || attrs.size > DOCKER_PULL_MAX_BYTES {
        return Err(DockerCompatError::ArchiveUnsupported);
    }
    let mut archive = Vec::new();
    archive.resize(attrs.size, 0);
    let len = fxfs::read_file(path, archive.as_mut_slice()).map_err(|_| DockerCompatError::OciRead)?;
    archive.truncate(len);
    load_docker_archive(&archive, DockerImageSource::Archive)
}

pub fn pull_docker_image(reference: &str) -> Result<DockerImageLoadResult, DockerCompatError> {
    install_builtin_docker_images()?;
    if let Some(name) = resolve_builtin_image_name(reference) {
        let image = image_info(name)?;
        return Ok(DockerImageLoadResult {
            image,
            source: DockerImageSource::Builtin,
            bytes: 0,
        });
    }

    if let Some((scheme, host, path)) = parse_pull_url(reference) {
        if scheme == "https" {
            return Err(DockerCompatError::Network(net::NetError::TlsUnsupported));
        }
        if scheme != "http" {
            return Err(DockerCompatError::RegistryUnsupported);
        }
        let archive = http_download_body(host, path)?;
        return load_docker_archive(archive.as_slice(), DockerImageSource::HttpArchive);
    }

    if normalize_image_reference(reference).is_ok() {
        if let Some(path) = staged_registry_archive_path(reference) {
            if fxfs::exists(path.as_str()) {
                return load_docker_image(path.as_str());
            }
        }
        return Err(DockerCompatError::Network(net::NetError::TlsUnsupported));
    }

    Err(DockerCompatError::ImageInvalid)
}

pub fn run_docker_image(
    image: &str,
    command: &[&str],
) -> Result<DockerRunResult, DockerCompatError> {
    let container = create_docker_container(image, command, None)?;
    let started = start_docker_container(container.id.as_str())?;
    finish_docker_container(started.container.id.as_str(), 0)?;
    let completed = inspect_docker_container(started.container.id.as_str())?;
    Ok(DockerRunResult {
        container: completed,
        runtime: started.runtime,
    })
}

pub fn create_docker_container(
    image: &str,
    command: &[&str],
    name: Option<&str>,
) -> Result<DockerContainer, DockerCompatError> {
    install_builtin_docker_images()?;
    let image_record = resolve_image_record(image)?;

    let mut config_bytes = [0u8; DOCKER_IMAGE_CONFIG_MAX_BYTES];
    let config_len = fxfs::read_file(image_record.config_path.as_str(), &mut config_bytes)
        .map_err(|_| DockerCompatError::OciRead)?;
    let config = core::str::from_utf8(&config_bytes[..config_len])
        .map_err(|_| DockerCompatError::OciParse)?;
    validate_docker_command(command)?;
    let id = match name {
        Some(value) => {
            if !docker_name_valid(value) || container_exists(value) {
                return Err(DockerCompatError::ContainerExists);
            }
            String::from(value)
        }
        None => next_container_id()?,
    };

    let dir = container_dir_path(id.as_str());
    let _ = fxfs::create_dir(dir.as_str());
    let mut container = DockerContainer {
        id,
        image: image_record.name,
        command: docker_command_display(config, command)?,
        args: docker_command_args(command)?,
        status: DockerContainerStatus::Created,
        exit_code: 0,
        runtime: None,
        log_bytes: 0,
    };
    write_container_log(container.id.as_str(), &[])?;
    write_container_record(&container)?;
    container.log_bytes = 0;
    Ok(container)
}

pub fn start_docker_container(reference: &str) -> Result<DockerRunResult, DockerCompatError> {
    install_builtin_docker_images()?;
    let id = resolve_container_id(reference)?;
    let mut container = load_container_record(id.as_str())?;
    if container.status == DockerContainerStatus::Running {
        return Err(DockerCompatError::ContainerState);
    }

    let mut config_bytes = [0u8; DOCKER_IMAGE_CONFIG_MAX_BYTES];
    let image_record = resolve_image_record(container.image.as_str())?;
    let config_len = fxfs::read_file(image_record.config_path.as_str(), &mut config_bytes)
        .map_err(|_| DockerCompatError::OciRead)?;
    let config = core::str::from_utf8(&config_bytes[..config_len])
        .map_err(|_| DockerCompatError::OciParse)?;
    let args = docker_record_args(container.args.as_str());
    let request = docker_image_config_to_oci_request(&image_record, config, &args)?;
    let runtime = run_oci_runtime_request(&request, config_len)?;

    let log = docker_container_log_payload(&container);
    write_container_log(container.id.as_str(), log.as_bytes())?;
    container.status = DockerContainerStatus::Running;
    container.exit_code = 0;
    container.runtime = Some(runtime);
    container.log_bytes = log.len();
    write_container_record(&container)?;

    Ok(DockerRunResult { container, runtime })
}

pub fn stop_docker_container(reference: &str) -> Result<DockerContainer, DockerCompatError> {
    let id = resolve_container_id(reference)?;
    let container = load_container_record(id.as_str())?;
    if container.status != DockerContainerStatus::Running {
        return Err(DockerCompatError::ContainerState);
    }
    finish_docker_container(id.as_str(), 0)?;
    inspect_docker_container(id.as_str())
}

pub fn remove_docker_container(reference: &str) -> Result<(), DockerCompatError> {
    let id = resolve_container_id(reference)?;
    let container = load_container_record(id.as_str())?;
    if container.status == DockerContainerStatus::Running {
        return Err(DockerCompatError::ContainerState);
    }
    let record = container_record_path(id.as_str());
    let log = container_log_path(id.as_str());
    let _ = fxfs::delete_file(log.as_str());
    fxfs::delete_file(record.as_str()).map_err(|_| DockerCompatError::ContainerNotFound)
}

pub fn inspect_docker_container(reference: &str) -> Result<DockerContainer, DockerCompatError> {
    let id = resolve_container_id(reference)?;
    load_container_record(id.as_str())
}

pub fn list_docker_containers(all: bool) -> Result<Vec<DockerContainer>, DockerCompatError> {
    install_builtin_docker_images()?;
    let entries =
        fxfs::entries(DOCKER_CONTAINER_ROOT).map_err(|_| DockerCompatError::FxfsPrepare)?;
    let mut containers = Vec::new();
    for entry in entries {
        if containers.len() >= DOCKER_MAX_CONTAINERS {
            break;
        }
        let record_path = container_record_path(entry.name.as_str());
        if !fxfs::exists(record_path.as_str()) {
            continue;
        }
        if let Ok(container) = load_container_record(entry.name.as_str()) {
            if all || container.status == DockerContainerStatus::Running {
                containers.push(container);
            }
        }
    }
    Ok(containers)
}

pub fn docker_container_logs(reference: &str, out: &mut [u8]) -> Result<usize, DockerCompatError> {
    let id = resolve_container_id(reference)?;
    let path = container_log_path(id.as_str());
    fxfs::read_file(path.as_str(), out).map_err(|_| DockerCompatError::ContainerNotFound)
}

fn finish_docker_container(reference: &str, exit_code: i32) -> Result<(), DockerCompatError> {
    let id = resolve_container_id(reference)?;
    let mut container = load_container_record(id.as_str())?;
    if container.status != DockerContainerStatus::Exited {
        if let Some(runtime) = container.runtime {
            let _ = syscall::sys_handle_close(runtime.thread_handle);
            let _ = syscall::sys_handle_close(runtime.process_handle);
            let _ = syscall::sys_handle_close(runtime.job_handle);
        }
    }
    container.status = DockerContainerStatus::Exited;
    container.exit_code = exit_code;
    write_container_record(&container)
}

fn validate_docker_command(command: &[&str]) -> Result<(), DockerCompatError> {
    if command.len() > DOCKER_MAX_COMMAND_ITEMS {
        return Err(DockerCompatError::OciParse);
    }
    for item in command {
        if item.is_empty()
            || item.len() > 128
            || item.as_bytes().contains(&0)
            || item.as_bytes().iter().any(|byte| *byte < 0x20)
            || item.as_bytes().contains(&b'|')
            || item.as_bytes().contains(&b'\n')
        {
            return Err(DockerCompatError::OciParse);
        }
    }
    Ok(())
}

fn docker_name_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= DOCKER_MAX_CONTAINER_NAME_BYTES
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.'))
}

fn container_exists(id: &str) -> bool {
    let path = container_record_path(id);
    fxfs::exists(path.as_str())
}

fn prune_invalid_container_entries() {
    let Ok(entries) = fxfs::entries(DOCKER_CONTAINER_ROOT) else {
        return;
    };
    for entry in entries {
        let record = container_record_path(entry.name.as_str());
        let log = container_log_path(entry.name.as_str());
        if !fxfs::exists(record.as_str()) {
            let _ = fxfs::delete_file(log.as_str());
        }
    }
}

fn next_container_id() -> Result<String, DockerCompatError> {
    let mut index = 1usize;
    while index <= 9999 {
        let mut id = String::from("smros");
        append_usize(&mut id, index, 4);
        if !container_exists(id.as_str()) {
            return Ok(id);
        }
        index += 1;
    }
    Err(DockerCompatError::ContainerExists)
}

fn resolve_container_id(reference: &str) -> Result<String, DockerCompatError> {
    if !docker_name_valid(reference) {
        return Err(DockerCompatError::ContainerInvalid);
    }
    if container_exists(reference) {
        return Ok(String::from(reference));
    }

    let entries =
        fxfs::entries(DOCKER_CONTAINER_ROOT).map_err(|_| DockerCompatError::ContainerNotFound)?;
    let mut found: Option<String> = None;
    for entry in entries {
        let record = container_record_path(entry.name.as_str());
        if fxfs::exists(record.as_str()) && entry.name.starts_with(reference) {
            if found.is_some() {
                return Err(DockerCompatError::ContainerInvalid);
            }
            found = Some(entry.name);
        }
    }
    found.ok_or(DockerCompatError::ContainerNotFound)
}

fn write_container_record(container: &DockerContainer) -> Result<(), DockerCompatError> {
    let path = container_record_path(container.id.as_str());
    let mut record = String::new();
    push_record_field(&mut record, "id", container.id.as_str());
    push_record_field(&mut record, "image", container.image.as_str());
    push_record_field(&mut record, "command", container.command.as_str());
    push_record_field(&mut record, "args", container.args.as_str());
    push_record_field(&mut record, "status", container.status.as_str());
    push_record_field(
        &mut record,
        "exit",
        signed_to_string(container.exit_code).as_str(),
    );
    push_record_field(
        &mut record,
        "log",
        usize_to_string(container.log_bytes).as_str(),
    );
    if let Some(runtime) = container.runtime {
        push_record_field(
            &mut record,
            "job",
            usize_to_string(runtime.job_handle as usize).as_str(),
        );
        push_record_field(
            &mut record,
            "process",
            usize_to_string(runtime.process_handle as usize).as_str(),
        );
        push_record_field(
            &mut record,
            "thread",
            usize_to_string(runtime.thread_handle as usize).as_str(),
        );
        push_record_field(
            &mut record,
            "ns",
            usize_to_string(runtime.namespace_flags).as_str(),
        );
        push_record_field(
            &mut record,
            "mounts",
            usize_to_string(runtime.mount_count).as_str(),
        );
        push_record_field(
            &mut record,
            "seccomp",
            usize_to_string(runtime.seccomp_mode).as_str(),
        );
        push_record_field(
            &mut record,
            "filters",
            usize_to_string(runtime.seccomp_filters).as_str(),
        );
        push_record_field(
            &mut record,
            "cap",
            usize_to_string(runtime.cap_effective as usize).as_str(),
        );
    }
    fxfs::write_file(path.as_str(), record.as_bytes())
        .map(|_| ())
        .map_err(|_| DockerCompatError::FxfsPrepare)
}

fn load_container_record(id: &str) -> Result<DockerContainer, DockerCompatError> {
    let path = container_record_path(id);
    let mut bytes = [0u8; DOCKER_CONTAINER_RECORD_MAX_BYTES];
    let len = fxfs::read_file(path.as_str(), &mut bytes)
        .map_err(|_| DockerCompatError::ContainerNotFound)?;
    let record =
        core::str::from_utf8(&bytes[..len]).map_err(|_| DockerCompatError::ContainerInvalid)?;
    let id_value = record_field(record, "id").ok_or(DockerCompatError::ContainerInvalid)?;
    let image = record_field(record, "image").ok_or(DockerCompatError::ContainerInvalid)?;
    let command = record_field(record, "command").ok_or(DockerCompatError::ContainerInvalid)?;
    let args = record_field(record, "args").unwrap_or("");
    let status = parse_container_status(
        record_field(record, "status").ok_or(DockerCompatError::ContainerInvalid)?,
    )?;
    let exit_code = parse_i32(record_field(record, "exit").unwrap_or("0"))
        .ok_or(DockerCompatError::ContainerInvalid)?;
    let log_bytes = parse_usize(record_field(record, "log").unwrap_or("0"))
        .ok_or(DockerCompatError::ContainerInvalid)?;
    let runtime = parse_container_runtime(record);

    Ok(DockerContainer {
        id: String::from(id_value),
        image: String::from(image),
        command: String::from(command),
        args: String::from(args),
        status,
        exit_code,
        runtime,
        log_bytes,
    })
}

fn parse_container_runtime(record: &str) -> Option<DockerCompatResult> {
    let job = parse_usize(record_field(record, "job")?)? as u32;
    let process = parse_usize(record_field(record, "process")?)? as u32;
    let thread = parse_usize(record_field(record, "thread")?)? as u32;
    let namespace_flags = parse_usize(record_field(record, "ns")?)?;
    let mount_count = parse_usize(record_field(record, "mounts")?)?;
    let seccomp_mode = parse_usize(record_field(record, "seccomp")?)?;
    let seccomp_filters = parse_usize(record_field(record, "filters")?)?;
    let cap_effective = parse_usize(record_field(record, "cap")?)? as u64;
    Some(DockerCompatResult {
        namespace_flags,
        mount_count,
        seccomp_mode,
        seccomp_filters,
        cap_effective,
        cgroup_bytes: CGROUP_PAYLOAD.len(),
        apparmor_bytes: apparmor_enforce_payload("docker-default").len(),
        oci_config_bytes: 0,
        oci_mounts: 3,
        oci_args: 0,
        oci_env: 0,
        masked_paths: 6,
        readonly_paths: 6,
        job_handle: job,
        process_handle: process,
        thread_handle: thread,
    })
}

fn parse_container_status(value: &str) -> Result<DockerContainerStatus, DockerCompatError> {
    match value {
        "created" => Ok(DockerContainerStatus::Created),
        "running" => Ok(DockerContainerStatus::Running),
        "exited" => Ok(DockerContainerStatus::Exited),
        _ => Err(DockerCompatError::ContainerInvalid),
    }
}

fn push_record_field(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    out.push_str(value);
    out.push('\n');
}

fn record_field<'a>(record: &'a str, key: &str) -> Option<&'a str> {
    let key_bytes = key.as_bytes();
    let bytes = record.as_bytes();
    let mut line_start = 0usize;
    while line_start < bytes.len() {
        let mut line_end = line_start;
        while line_end < bytes.len() && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        let line = &bytes[line_start..line_end];
        if line.len() > key_bytes.len()
            && line.starts_with(key_bytes)
            && line[key_bytes.len()] == b'='
        {
            return core::str::from_utf8(&line[key_bytes.len() + 1..]).ok();
        }
        line_start = line_end.saturating_add(1);
    }
    None
}

fn write_container_log(id: &str, data: &[u8]) -> Result<(), DockerCompatError> {
    let path = container_log_path(id);
    let payload = if data.len() > DOCKER_CONTAINER_LOG_MAX_BYTES {
        &data[..DOCKER_CONTAINER_LOG_MAX_BYTES]
    } else {
        data
    };
    fxfs::write_file(path.as_str(), payload)
        .map(|_| ())
        .map_err(|_| DockerCompatError::FxfsPrepare)
}

fn docker_container_log_payload(container: &DockerContainer) -> String {
    let mut log = String::from("SMROS Docker container ");
    log.push_str(container.id.as_str());
    log.push('\n');
    log.push_str("image=");
    log.push_str(container.image.as_str());
    log.push('\n');
    log.push_str("command=");
    log.push_str(container.command.as_str());
    log.push('\n');
    log.push_str("runtime=runc namespace/cgroup/seccomp compatibility path\n");
    log
}

fn docker_command_display(
    image_config: &str,
    command: &[&str],
) -> Result<String, DockerCompatError> {
    let mut out = String::new();
    if command.is_empty() {
        let config = json_object_after(image_config, "config")?;
        let entrypoint = json_array_after(config, "Entrypoint").unwrap_or("[]");
        let cmd = json_array_after(config, "Cmd").unwrap_or("[]");
        if json_string_array_count(entrypoint).saturating_add(json_string_array_count(cmd)) == 0 {
            return Err(DockerCompatError::OciParse);
        }
        push_command_display_items(&mut out, entrypoint)?;
        push_command_display_items(&mut out, cmd)?;
    } else {
        for item in command {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(item);
        }
    }
    Ok(out)
}

fn push_command_display_items(out: &mut String, input: &str) -> Result<(), DockerCompatError> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'"' {
            let start = pos + 1;
            pos = start;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'"' => {
                        if !out.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(&input[start..pos]);
                        break;
                    }
                    b'\\' => return Err(DockerCompatError::OciParse),
                    _ => pos += 1,
                }
            }
        }
        pos += 1;
    }
    Ok(())
}

fn docker_command_args(command: &[&str]) -> Result<String, DockerCompatError> {
    validate_docker_command(command)?;
    let mut out = String::new();
    let mut index = 0usize;
    while index < command.len() {
        if index > 0 {
            out.push('|');
        }
        out.push_str(command[index]);
        index += 1;
    }
    Ok(out)
}

fn docker_record_args(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    if input.is_empty() {
        return out;
    }
    let bytes = input.as_bytes();
    let mut start = 0usize;
    let mut pos = 0usize;
    while pos <= bytes.len() {
        if pos == bytes.len() || bytes[pos] == b'|' {
            if let Ok(value) = core::str::from_utf8(&bytes[start..pos]) {
                out.push(value);
            }
            start = pos.saturating_add(1);
        }
        pos += 1;
    }
    out
}

fn load_docker_archive(
    archive: &[u8],
    source: DockerImageSource,
) -> Result<DockerImageLoadResult, DockerCompatError> {
    let manifest = tar_file_bytes(archive, "manifest.json")
        .ok_or(DockerCompatError::ArchiveInvalid)?;
    let manifest_text = core::str::from_utf8(manifest).map_err(|_| DockerCompatError::ArchiveInvalid)?;
    let config_name = docker_save_config_name(manifest_text)?;
    let repo_tag = docker_save_repo_tag(manifest_text)?;
    let layers = docker_save_layers(manifest_text)?;
    if layers.is_empty() {
        return Err(DockerCompatError::ArchiveInvalid);
    }

    let config = tar_file_bytes(archive, config_name).ok_or(DockerCompatError::ArchiveInvalid)?;
    let config_text = core::str::from_utf8(config).map_err(|_| DockerCompatError::ArchiveInvalid)?;
    validate_image_config(config_text)?;

    let image_name = normalize_image_reference(repo_tag)?;
    let key = image_storage_key(image_name.as_str());
    let image_dir = docker_image_dir_path(key.as_str());
    let rootfs = docker_image_rootfs_path(key.as_str());
    let blobs = docker_image_blobs_path(key.as_str());
    let manifest_path = docker_image_manifest_path(key.as_str());
    let config_path = docker_image_config_path(key.as_str());

    let _persist_guard = fxfs::suspend_persist();
    prepare_image_dirs(image_dir.as_str(), rootfs.as_str(), blobs.as_str())?;
    fxfs::write_file(manifest_path.as_str(), manifest).map_err(|_| DockerCompatError::OciInstall)?;
    fxfs::write_file(config_path.as_str(), config).map_err(|_| DockerCompatError::OciInstall)?;

    let mut layer_count = 0usize;
    for layer in layers {
        let layer_bytes = tar_file_bytes(archive, layer.as_str())
            .ok_or(DockerCompatError::ArchiveInvalid)?;
        if is_gzip(layer_bytes) {
            return Err(DockerCompatError::ArchiveUnsupported);
        }
        extract_layer_tar(layer_bytes, rootfs.as_str())?;
        layer_count += 1;
    }

    let record = DockerImageRecord {
        name: image_name,
        rootfs,
        manifest_path,
        config_path,
        layers: layer_count,
    };
    write_image_record(&record)?;
    fxfs::flush_persist();
    Ok(DockerImageLoadResult {
        image: image_info_from_record(&record)?,
        source,
        bytes: archive.len(),
    })
}

fn parse_pull_url(input: &str) -> Option<(&str, &str, &str)> {
    let scheme_end = input.find("://")?;
    let scheme = &input[..scheme_end];
    let rest = &input[scheme_end + 3..];
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    let path = &rest[slash..];
    if scheme.is_empty() || host.is_empty() || path.is_empty() {
        return None;
    }
    Some((scheme, host, path))
}

pub fn staged_registry_archive_path(reference: &str) -> Option<String> {
    let normalized = normalize_image_reference(reference).ok()?;
    let without_tag = match normalized.rfind(':') {
        Some(split) => &normalized[..split],
        None => normalized.as_str(),
    };
    let image_part = without_tag.rsplit('/').next()?;
    if !simple_path_component(image_part) {
        return None;
    }
    let mut out = String::from("/shared/");
    out.push_str(image_part);
    out.push_str(".tar");
    Some(out)
}

fn http_download_body(host: &str, path: &str) -> Result<Vec<u8>, DockerCompatError> {
    let (dial_host, port) = parse_http_host_port(host)?;
    let remote_ip = parse_ipv4_addr(dial_host)
        .map(Ok)
        .unwrap_or_else(|| net::dns_lookup_a(dial_host))
        .map_err(DockerCompatError::Network)?;
    let mut socket = net::tcp_connect(net::NetworkSocketAddr {
        ip: remote_ip,
        port,
    })
    .map_err(DockerCompatError::Network)?;
    let request = build_http_archive_request(host, path)?;
    socket.write(request.as_bytes()).map_err(DockerCompatError::Network)?;

    let mut response = Vec::new();
    response.resize(DOCKER_HTTP_RESPONSE_MAX_BYTES, 0);
    let bytes_read = socket.read(response.as_mut_slice()).map_err(DockerCompatError::Network)?;
    let _ = socket.close();
    response.truncate(bytes_read);
    if parse_http_status(response.as_slice()) != Some(200) {
        return Err(DockerCompatError::Network(net::NetError::NoAddress));
    }
    let body_start = http_body_offset(response.as_slice()).ok_or(DockerCompatError::ArchiveInvalid)?;
    if body_start >= response.len() {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    let body = response[body_start..].to_vec();
    if body.is_empty() || body.len() > DOCKER_PULL_MAX_BYTES {
        return Err(DockerCompatError::ArchiveUnsupported);
    }
    Ok(body)
}

fn parse_http_host_port(host: &str) -> Result<(&str, u16), DockerCompatError> {
    if host.is_empty() {
        return Err(DockerCompatError::ImageInvalid);
    }
    if let Some(split) = host.rfind(':') {
        let name = &host[..split];
        let port = parse_usize(&host[split + 1..]).ok_or(DockerCompatError::ImageInvalid)?;
        if name.is_empty() || port == 0 || port > u16::MAX as usize {
            return Err(DockerCompatError::ImageInvalid);
        }
        Ok((name, port as u16))
    } else {
        Ok((host, 80))
    }
}

fn build_http_archive_request(host: &str, path: &str) -> Result<String, DockerCompatError> {
    if !path.starts_with('/') || path.as_bytes().contains(&0) || path.len() > 512 {
        return Err(DockerCompatError::ImageInvalid);
    }
    let mut request = String::from("GET ");
    request.push_str(path);
    request.push_str(" HTTP/1.0\r\nHost: ");
    request.push_str(host);
    request.push_str("\r\nUser-Agent: SMROS-Docker/0.1\r\nConnection: close\r\n\r\n");
    if request.len() > 1024 {
        return Err(DockerCompatError::ImageInvalid);
    }
    Ok(request)
}

fn parse_http_status(response: &[u8]) -> Option<u16> {
    if response.len() < 12 || &response[0..5] != b"HTTP/" {
        return None;
    }
    let mut offset = 5usize;
    while offset < response.len() && response[offset] != b' ' {
        offset += 1;
    }
    if offset + 4 > response.len() {
        return None;
    }
    let code = &response[offset + 1..offset + 4];
    if code.iter().all(|byte| byte.is_ascii_digit()) {
        Some(
            ((code[0] - b'0') as u16) * 100
                + ((code[1] - b'0') as u16) * 10
                + (code[2] - b'0') as u16,
        )
    } else {
        None
    }
}

fn http_body_offset(response: &[u8]) -> Option<usize> {
    if response.len() < 4 {
        return None;
    }
    let mut pos = 0usize;
    while pos + 3 < response.len() {
        if &response[pos..pos + 4] == b"\r\n\r\n" {
            return Some(pos + 4);
        }
        pos += 1;
    }
    None
}

fn validate_image_config(config: &str) -> Result<(), DockerCompatError> {
    let rootfs = json_object_after(config, "rootfs")?;
    let diff_ids = json_array_after(rootfs, "diff_ids")?;
    let cfg = json_object_after(config, "config")?;
    if json_string_array_count(diff_ids) == 0 {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    if let Ok(working_dir) = json_string_after(cfg, "WorkingDir") {
        if !working_dir.is_empty() && !working_dir.starts_with('/') {
            return Err(DockerCompatError::ArchiveInvalid);
        }
    }
    Ok(())
}

fn docker_save_config_name(manifest: &str) -> Result<&str, DockerCompatError> {
    let value = json_string_after(manifest, "Config")?;
    if !tar_member_name_safe(value) {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    Ok(value)
}

fn docker_save_repo_tag(manifest: &str) -> Result<&str, DockerCompatError> {
    let tags = json_array_after(manifest, "RepoTags")?;
    let value = first_string_in_array(tags)?;
    if !image_reference_valid(value) && !image_reference_valid_with_optional_tag(value) {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    Ok(value)
}

fn docker_save_layers(manifest: &str) -> Result<Vec<String>, DockerCompatError> {
    let array = json_array_after(manifest, "Layers")?;
    let mut layers = Vec::new();
    let bytes = array.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'"' {
            let start = pos + 1;
            pos = start;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'"' => {
                        let value = &array[start..pos];
                        if !tar_member_name_safe(value) {
                            return Err(DockerCompatError::ArchiveInvalid);
                        }
                        layers.push(String::from(value));
                        break;
                    }
                    b'\\' => return Err(DockerCompatError::ArchiveInvalid),
                    _ => pos += 1,
                }
            }
        }
        pos += 1;
    }
    Ok(layers)
}

fn tar_file_bytes<'a>(archive: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + TAR_BLOCK_BYTES <= archive.len() {
        let header = &archive[offset..offset + TAR_BLOCK_BYTES];
        if tar_header_empty(header) {
            return None;
        }
        let size = tar_octal(&header[124..136])?;
        let file_start = offset + TAR_BLOCK_BYTES;
        let file_end = file_start.checked_add(size)?;
        if file_end > archive.len() {
            return None;
        }
        if tar_name_matches(header, name) {
            let kind = header[156];
            if kind == 0 || kind == b'0' {
                return Some(&archive[file_start..file_end]);
            }
        }
        offset = file_start.checked_add(tar_padded_len(size)?)?;
    }
    None
}

fn extract_layer_tar(layer: &[u8], rootfs: &str) -> Result<(), DockerCompatError> {
    let mut offset = 0usize;
    while offset + TAR_BLOCK_BYTES <= layer.len() {
        let header = &layer[offset..offset + TAR_BLOCK_BYTES];
        if tar_header_empty(header) {
            return Ok(());
        }
        let size = tar_octal(&header[124..136]).ok_or(DockerCompatError::ArchiveInvalid)?;
        let file_start = offset + TAR_BLOCK_BYTES;
        let file_end = file_start.checked_add(size).ok_or(DockerCompatError::ArchiveInvalid)?;
        if file_end > layer.len() {
            return Err(DockerCompatError::ArchiveInvalid);
        }
        let name = tar_header_name(header).ok_or(DockerCompatError::ArchiveInvalid)?;
        if name.contains(".wh.") {
            offset = file_start
                .checked_add(tar_padded_len(size).ok_or(DockerCompatError::ArchiveInvalid)?)
                .ok_or(DockerCompatError::ArchiveInvalid)?;
            continue;
        }
        match header[156] {
            0 | b'0' => {
                let dest = rootfs_child_path(rootfs, name)?;
                ensure_parent_dirs(dest.as_str())?;
                fxfs::write_file(dest.as_str(), &layer[file_start..file_end])
                    .map_err(|_| DockerCompatError::OciInstall)?;
            }
            b'5' => {
                let dest = rootfs_child_path(rootfs, name)?;
                ensure_dir_tree(dest.as_str())?;
            }
            _ => {}
        }
        offset = file_start
            .checked_add(tar_padded_len(size).ok_or(DockerCompatError::ArchiveInvalid)?)
            .ok_or(DockerCompatError::ArchiveInvalid)?;
    }
    Ok(())
}

fn prepare_image_dirs(image_dir: &str, rootfs: &str, blobs: &str) -> Result<(), DockerCompatError> {
    let _ = fxfs::create_dir("/docker");
    let _ = fxfs::create_dir(DOCKER_IMAGE_ROOT);
    let _ = fxfs::create_dir(image_dir);
    let _ = fxfs::create_dir(rootfs);
    let _ = fxfs::create_dir(blobs);
    ensure_dir_tree(rootfs)?;
    ensure_dir_tree(blobs)?;
    Ok(())
}

fn ensure_parent_dirs(path: &str) -> Result<(), DockerCompatError> {
    if let Some(split) = path.rfind('/') {
        if split > 0 {
            ensure_dir_tree(&path[..split])?;
        }
    }
    Ok(())
}

fn ensure_dir_tree(path: &str) -> Result<(), DockerCompatError> {
    if !path.starts_with('/') || path.as_bytes().contains(&0) {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    let mut current = String::new();
    current.push('/');
    for part in path.trim_matches('/').split('/') {
        if part.is_empty() {
            continue;
        }
        if !simple_path_component(part) {
            return Err(DockerCompatError::ArchiveInvalid);
        }
        if current.len() > 1 {
            current.push('/');
        }
        current.push_str(part);
        let _ = fxfs::create_dir(current.as_str());
    }
    Ok(())
}

fn rootfs_child_path(rootfs: &str, member: &str) -> Result<String, DockerCompatError> {
    if !tar_member_name_safe(member) {
        return Err(DockerCompatError::ArchiveInvalid);
    }
    let mut out = String::from(rootfs);
    for part in member.trim_matches('/').split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if !rootfs_path_component(part) {
            return Err(DockerCompatError::ArchiveInvalid);
        }
        out.push('/');
        out.push_str(part);
    }
    Ok(out)
}

fn tar_header_empty(header: &[u8]) -> bool {
    header.iter().all(|byte| *byte == 0)
}

fn tar_name_matches(header: &[u8], expected: &str) -> bool {
    tar_header_name(header) == Some(expected)
}

fn tar_header_name(header: &[u8]) -> Option<&str> {
    if header.len() < TAR_BLOCK_BYTES {
        return None;
    }
    let name = tar_string(&header[0..100])?;
    if !tar_member_name_safe(name) {
        return None;
    }
    Some(name)
}

fn tar_string(input: &[u8]) -> Option<&str> {
    let mut end = 0usize;
    while end < input.len() && input[end] != 0 {
        end += 1;
    }
    core::str::from_utf8(&input[..end]).ok()
}

fn tar_octal(input: &[u8]) -> Option<usize> {
    let mut value = 0usize;
    let mut saw_digit = false;
    for byte in input {
        match *byte {
            0 | b' ' => {
                if saw_digit {
                    break;
                }
            }
            b'0'..=b'7' => {
                saw_digit = true;
                value = value.checked_mul(8)?.checked_add((byte - b'0') as usize)?;
            }
            _ => return None,
        }
    }
    Some(value)
}

fn tar_padded_len(size: usize) -> Option<usize> {
    size.checked_add(TAR_BLOCK_BYTES - 1)
        .map(|value| value / TAR_BLOCK_BYTES * TAR_BLOCK_BYTES)
}

fn is_gzip(input: &[u8]) -> bool {
    input.len() >= 2 && input[0] == 0x1f && input[1] == 0x8b
}

fn tar_member_name_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 180
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.as_bytes().contains(&0)
}

fn simple_path_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'-' | b'_'))
}

fn rootfs_path_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && value
            .as_bytes()
            .iter()
            .all(|byte| matches!(*byte, 0x21..=0x7e) && *byte != b'/')
}

fn docker_image_rootfs_path(key: &str) -> String {
    let mut out = docker_image_dir_path(key);
    out.push_str("/rootfs");
    out
}

fn docker_image_blobs_path(key: &str) -> String {
    let mut out = docker_image_dir_path(key);
    out.push_str("/blobs");
    out
}

fn docker_image_manifest_path(key: &str) -> String {
    let mut out = docker_image_dir_path(key);
    out.push_str("/manifest.json");
    out
}

fn docker_image_config_path(key: &str) -> String {
    let mut out = docker_image_dir_path(key);
    out.push_str("/config.json");
    out
}

fn docker_image_blob_path(key: &str, layer_index: usize) -> String {
    let mut out = docker_image_blobs_path(key);
    out.push_str("/layer");
    append_usize(&mut out, layer_index, 2);
    out.push_str(".tar");
    out
}

fn image_info(image: &str) -> Result<DockerImageInfo, DockerCompatError> {
    let record = resolve_image_record(image)?;
    image_info_from_record(&record)
}

fn image_info_from_record(record: &DockerImageRecord) -> Result<DockerImageInfo, DockerCompatError> {
    let manifest_bytes = fxfs::attrs(record.manifest_path.as_str())
        .map_err(|_| DockerCompatError::OciRead)?
        .size;
    let config_bytes = fxfs::attrs(record.config_path.as_str())
        .map_err(|_| DockerCompatError::OciRead)?
        .size;
    Ok(DockerImageInfo {
        name: record.name.clone(),
        rootfs: record.rootfs.clone(),
        manifest_bytes,
        config_bytes,
        layers: record.layers,
    })
}

fn resolve_image_record(image: &str) -> Result<DockerImageRecord, DockerCompatError> {
    let normalized = normalize_image_reference(image)?;
    let dir = docker_image_dir_path(image_storage_key(normalized.as_str()).as_str());
    if fxfs::exists(docker_image_meta_path_from_dir(dir.as_str()).as_str()) {
        return load_image_record_from_dir(dir.as_str());
    }

    if let Some(name) = resolve_builtin_image_name(image) {
        let dir = docker_image_dir_path(image_storage_key(name).as_str());
        return load_image_record_from_dir(dir.as_str());
    }

    Err(DockerCompatError::ImageNotFound)
}

fn write_image_record(record: &DockerImageRecord) -> Result<(), DockerCompatError> {
    let dir = docker_image_dir_path(image_storage_key(record.name.as_str()).as_str());
    let _ = fxfs::create_dir(dir.as_str());
    let mut data = String::new();
    push_record_field(&mut data, "name", record.name.as_str());
    push_record_field(&mut data, "rootfs", record.rootfs.as_str());
    push_record_field(&mut data, "manifest", record.manifest_path.as_str());
    push_record_field(&mut data, "config", record.config_path.as_str());
    push_record_field(&mut data, "layers", usize_to_string(record.layers).as_str());
    fxfs::write_file(docker_image_meta_path_from_dir(dir.as_str()).as_str(), data.as_bytes())
        .map(|_| ())
        .map_err(|_| DockerCompatError::FxfsPrepare)
}

fn load_image_record_from_dir(dir: &str) -> Result<DockerImageRecord, DockerCompatError> {
    let path = docker_image_meta_path_from_dir(dir);
    let mut bytes = [0u8; DOCKER_CONTAINER_RECORD_MAX_BYTES];
    let len = fxfs::read_file(path.as_str(), &mut bytes).map_err(|_| DockerCompatError::ImageNotFound)?;
    let record = core::str::from_utf8(&bytes[..len]).map_err(|_| DockerCompatError::ImageInvalid)?;
    let name = record_field(record, "name").ok_or(DockerCompatError::ImageInvalid)?;
    let rootfs = record_field(record, "rootfs").ok_or(DockerCompatError::ImageInvalid)?;
    let manifest_path = record_field(record, "manifest").ok_or(DockerCompatError::ImageInvalid)?;
    let config_path = record_field(record, "config").ok_or(DockerCompatError::ImageInvalid)?;
    let layers = parse_usize(record_field(record, "layers").unwrap_or("0"))
        .ok_or(DockerCompatError::ImageInvalid)?;
    if !image_reference_valid(name)
        || !rootfs.starts_with('/')
        || !manifest_path.starts_with('/')
        || !config_path.starts_with('/')
        || layers == 0
    {
        return Err(DockerCompatError::ImageInvalid);
    }
    Ok(DockerImageRecord {
        name: String::from(name),
        rootfs: String::from(rootfs),
        manifest_path: String::from(manifest_path),
        config_path: String::from(config_path),
        layers,
    })
}

fn docker_image_dir_path(key: &str) -> String {
    let mut out = String::from(DOCKER_IMAGE_ROOT);
    out.push('/');
    out.push_str(key);
    out
}

fn docker_image_meta_path_from_dir(dir: &str) -> String {
    let mut out = String::from(dir);
    out.push('/');
    out.push_str(DOCKER_IMAGE_META_FILE);
    out
}

fn container_dir_path(id: &str) -> String {
    let mut out = String::from(DOCKER_CONTAINER_ROOT);
    out.push('/');
    out.push_str(id);
    out
}

fn container_record_path(id: &str) -> String {
    let mut out = container_dir_path(id);
    out.push_str("/state");
    out
}

fn container_log_path(id: &str) -> String {
    let mut out = container_dir_path(id);
    out.push_str("/log");
    out
}

fn run_oci_runtime_request(
    request: &OciRuntimeRequest<'_>,
    config_len: usize,
) -> Result<DockerCompatResult, DockerCompatError> {
    if request.root_path.is_empty()
        || request.arg0.is_empty()
        || request.namespace_flags & DOCKER_NS_FLAGS != DOCKER_NS_FLAGS
        || request.mount_count == 0
        || request.masked_paths == 0
        || request.readonly_paths == 0
        || request.cap_effective == 0
        || request.cap_permitted == 0
        || !request.no_new_privs
        || !request.seccomp_filter
    {
        return Err(DockerCompatError::StateMismatch);
    }

    prepare_docker_pseudo_files(request)?;
    let runtime = create_runc_runtime_task()?;

    syscall::reset_linux_container_state();
    syscall::sys_unshare(request.namespace_flags).map_err(DockerCompatError::Namespace)?;
    let ns_fd = syscall::sys_openat(AT_FDCWD, ROOT_PATH.as_ptr() as usize, O_DIRECTORY, 0)
        .map_err(DockerCompatError::Namespace)?;
    let setns_result =
        syscall::sys_setns(ns_fd, CLONE_NEWNET).map_err(DockerCompatError::Namespace);
    let _ = syscall::sys_close(ns_fd);
    setns_result?;

    apply_oci_mounts(request.mount_count)?;
    syscall::sys_pivot_root(ROOT_PATH.as_ptr() as usize, OLD_ROOT_PATH.as_ptr() as usize)
        .map_err(DockerCompatError::PivotRoot)?;
    syscall::sys_chroot(ROOT_PATH.as_ptr() as usize).map_err(DockerCompatError::Chroot)?;
    syscall::sys_sethostname(request.hostname.as_ptr() as usize, request.hostname.len())
        .map_err(DockerCompatError::Uts)?;
    syscall::sys_setdomainname(DOMAIN.as_ptr() as usize, DOMAIN.len())
        .map_err(DockerCompatError::Uts)?;

    syscall::sys_prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0).map_err(DockerCompatError::NoNewPrivs)?;
    if syscall::sys_prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0).ok() != Some(1) {
        return Err(DockerCompatError::StateMismatch);
    }
    syscall::sys_seccomp(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, 1)
        .map_err(DockerCompatError::Seccomp)?;
    apply_capabilities(request.cap_effective, request.cap_permitted)?;

    let cgroup_path = cgroup_procs_path(request.cgroups_path);
    let cgroup_cstr = c_string(&cgroup_path);
    let cgroup_bytes = write_linux_file(
        cgroup_cstr.as_bytes(),
        CGROUP_PAYLOAD,
        DockerCompatError::CgroupOpen,
        DockerCompatError::CgroupWrite,
        DockerCompatError::CgroupClose,
    )?;
    let apparmor_payload = apparmor_enforce_payload(request.apparmor_profile);
    let apparmor_bytes = write_linux_file(
        APPARMOR_EXEC_CSTR,
        apparmor_payload.as_bytes(),
        DockerCompatError::AppArmorOpen,
        DockerCompatError::AppArmorWrite,
        DockerCompatError::AppArmorClose,
    )?;

    let stats = syscall::linux_container_stats();
    let expected_mounts = request.mount_count.saturating_add(1);
    if stats.namespace_flags & request.namespace_flags != request.namespace_flags
        || stats.setns_count == 0
        || stats.mount_count != expected_mounts
        || stats.mount_flags & MS_PRIVATE == 0
        || stats.mount_flags & MS_BIND == 0
        || !stats.pivot_rooted
        || !stats.chrooted
        || !stats.no_new_privs
        || stats.seccomp_mode != 2
        || stats.seccomp_filters == 0
        || stats.cap_effective != request.cap_effective
        || stats.cap_permitted != request.cap_permitted
        || !stats.hostname_set
        || !stats.domainname_set
        || cgroup_bytes != CGROUP_PAYLOAD.len()
        || apparmor_bytes != apparmor_payload.len()
    {
        return Err(DockerCompatError::StateMismatch);
    }

    Ok(DockerCompatResult {
        namespace_flags: stats.namespace_flags,
        mount_count: stats.mount_count,
        seccomp_mode: stats.seccomp_mode,
        seccomp_filters: stats.seccomp_filters,
        cap_effective: stats.cap_effective,
        cgroup_bytes,
        apparmor_bytes,
        oci_config_bytes: config_len,
        oci_mounts: request.mount_count,
        oci_args: request.args,
        oci_env: request.env,
        masked_paths: request.masked_paths,
        readonly_paths: request.readonly_paths,
        job_handle: runtime.0,
        process_handle: runtime.1,
        thread_handle: runtime.2,
    })
}

fn install_sample_oci_bundle() -> Result<(), DockerCompatError> {
    if !fxfs::init() {
        return Err(DockerCompatError::FxfsInit);
    }
    let _ = fxfs::create_dir("/oci");
    let _ = fxfs::create_dir("/docker");
    let _ = fxfs::create_dir(DOCKER_CONTAINER_ROOT);
    let _ = fxfs::create_dir(OCI_BUNDLE_DIR);
    let _ = fxfs::create_dir(OCI_ROOTFS_DIR);
    let _ = fxfs::create_dir("/oci/docker-smoke/rootfs/bin");
    let _ = fxfs::create_dir("/oci/docker-smoke/rootfs/proc");
    let _ = fxfs::create_dir("/oci/docker-smoke/rootfs/dev");
    let _ = fxfs::create_dir("/oci/docker-smoke/rootfs/tmp");
    fxfs::write_file(OCI_ROOTFS_SH, b"#!/bin/sh\necho SMROS OCI runc bundle\n")
        .map_err(|_| DockerCompatError::OciInstall)?;
    fxfs::write_file(OCI_CONFIG_PATH, OCI_SAMPLE_CONFIG_JSON.as_bytes())
        .map_err(|_| DockerCompatError::OciInstall)?;
    Ok(())
}

fn docker_image_config_to_oci_request<'a>(
    image_record: &'a DockerImageRecord,
    image_config: &'a str,
    command: &'a [&'a str],
) -> Result<OciRuntimeRequest<'a>, DockerCompatError> {
    let config = json_object_after(image_config, "config")?;
    let env = json_array_after(config, "Env").unwrap_or("[]");
    let entrypoint = json_array_after(config, "Entrypoint").unwrap_or("[]");
    let cmd = json_array_after(config, "Cmd").unwrap_or("[]");
    let working_dir = json_string_after(config, "WorkingDir").unwrap_or("/");
    let rootfs = json_object_after(image_config, "rootfs")?;
    let diff_ids = json_array_after(rootfs, "diff_ids")?;

    let arg0 = if !command.is_empty() {
        command[0]
    } else {
        first_string_in_array(entrypoint)?
    };
    let image_cmd_args =
        json_string_array_count(entrypoint).saturating_add(json_string_array_count(cmd));
    let args = if command.is_empty() {
        image_cmd_args
    } else {
        command.len()
    };
    if arg0.is_empty()
        || arg0.as_bytes().contains(&0)
        || args == 0
        || working_dir.is_empty()
        || json_string_array_count(diff_ids) == 0
    {
        return Err(DockerCompatError::OciParse);
    }

    install_runtime_bundle_for_image(image_record, image_config, command)?;

    Ok(OciRuntimeRequest {
        root_path: image_record.rootfs.as_str(),
        arg0,
        args,
        env: json_string_array_count(env),
        hostname: "smros-docker",
        cgroups_path: "docker-smoke",
        apparmor_profile: "docker-default",
        namespace_flags: DOCKER_NS_FLAGS,
        mount_count: 3,
        masked_paths: 6,
        readonly_paths: 6,
        cap_effective: default_docker_capability_mask(),
        cap_permitted: default_docker_capability_mask(),
        no_new_privs: true,
        seccomp_filter: true,
    })
}

fn install_runtime_bundle_for_image(
    image_record: &DockerImageRecord,
    image_config: &str,
    command: &[&str],
) -> Result<(), DockerCompatError> {
    let mut bundle = String::from("{\n");
    bundle.push_str("  \"ociVersion\": \"1.1.0\",\n");
    bundle.push_str("  \"root\": { \"path\": \"");
    bundle.push_str(image_record.rootfs.as_str());
    bundle.push_str("\", \"readonly\": true },\n");
    bundle.push_str("  \"process\": {\n");
    bundle.push_str("    \"terminal\": false,\n");
    bundle.push_str("    \"cwd\": \"/\",\n");
    bundle.push_str("    \"args\": ");
    push_runtime_args(&mut bundle, image_config, command)?;
    bundle.push_str(",\n");
    bundle.push_str("    \"env\": ");
    bundle.push_str(json_array_after(json_object_after(image_config, "config")?, "Env").unwrap_or("[]"));
    bundle.push_str(",\n");
    bundle.push_str("    \"noNewPrivileges\": true,\n");
    bundle.push_str("    \"apparmorProfile\": \"docker-default\",\n");
    bundle.push_str("    \"capabilities\": {\n");
    bundle.push_str("      \"effective\": [\"CAP_CHOWN\", \"CAP_DAC_OVERRIDE\", \"CAP_FOWNER\", \"CAP_FSETID\", \"CAP_KILL\", \"CAP_SETGID\", \"CAP_SETUID\", \"CAP_SETPCAP\", \"CAP_NET_BIND_SERVICE\", \"CAP_NET_RAW\", \"CAP_SYS_CHROOT\", \"CAP_MKNOD\", \"CAP_AUDIT_WRITE\", \"CAP_SETFCAP\"],\n");
    bundle.push_str("      \"permitted\": [\"CAP_CHOWN\", \"CAP_DAC_OVERRIDE\", \"CAP_FOWNER\", \"CAP_FSETID\", \"CAP_KILL\", \"CAP_SETGID\", \"CAP_SETUID\", \"CAP_SETPCAP\", \"CAP_NET_BIND_SERVICE\", \"CAP_NET_RAW\", \"CAP_SYS_CHROOT\", \"CAP_MKNOD\", \"CAP_AUDIT_WRITE\", \"CAP_SETFCAP\"],\n");
    bundle.push_str("      \"inheritable\": []\n");
    bundle.push_str("    }\n");
    bundle.push_str("  },\n");
    bundle.push_str("  \"hostname\": \"smros-docker\",\n");
    bundle.push_str("  \"mounts\": [\n");
    bundle.push_str("    { \"destination\": \"/proc\", \"type\": \"proc\", \"source\": \"proc\", \"options\": [\"nosuid\", \"noexec\", \"nodev\"] },\n");
    bundle.push_str("    { \"destination\": \"/dev\", \"type\": \"tmpfs\", \"source\": \"tmpfs\", \"options\": [\"nosuid\", \"mode=755\", \"size=65536k\"] },\n");
    bundle.push_str("    { \"destination\": \"/tmp\", \"type\": \"bind\", \"source\": \"/\", \"options\": [\"rbind\", \"rw\"] }\n");
    bundle.push_str("  ],\n");
    bundle.push_str("  \"linux\": {\n");
    bundle.push_str("    \"cgroupsPath\": \"docker-smoke\",\n");
    bundle.push_str("    \"namespaces\": [{ \"type\": \"mount\" }, { \"type\": \"cgroup\" }, { \"type\": \"uts\" }, { \"type\": \"ipc\" }, { \"type\": \"user\" }, { \"type\": \"pid\" }, { \"type\": \"network\" }],\n");
    bundle.push_str("    \"maskedPaths\": [\"/proc/kcore\", \"/proc/keys\", \"/proc/latency_stats\", \"/proc/timer_list\", \"/proc/sched_debug\", \"/proc/scsi\"],\n");
    bundle.push_str("    \"readonlyPaths\": [\"/proc/asound\", \"/proc/bus\", \"/proc/fs\", \"/proc/irq\", \"/proc/sys\", \"/proc/sysrq-trigger\"],\n");
    bundle.push_str("    \"seccomp\": { \"defaultAction\": \"SCMP_ACT_ERRNO\", \"architectures\": [\"SCMP_ARCH_AARCH64\"], \"syscalls\": [] }\n");
    bundle.push_str("  },\n");
    bundle.push_str("  \"annotations\": { \"org.opencontainers.image.ref.name\": \"");
    bundle.push_str(image_record.name.as_str());
    bundle.push_str("\" }\n");
    bundle.push_str("}\n");

    let _ = fxfs::create_dir("/oci");
    let _ = fxfs::create_dir(OCI_BUNDLE_DIR);
    fxfs::write_file(OCI_CONFIG_PATH, bundle.as_bytes())
        .map_err(|_| DockerCompatError::OciInstall)?;
    Ok(())
}

fn create_runc_runtime_task() -> Result<(u32, u32, u32), DockerCompatError> {
    let runc_profile = right::process_right_profile_for_name_checked("runc")
        .map_err(DockerCompatError::RightsConfig)?;
    if runc_profile.process_rights & Rights::ManageProcess as u32 != 0
        || runc_profile.job_rights & Rights::ManageJob as u32 != 0
        || runc_profile.job_rights & Rights::SetPolicy as u32 != 0
    {
        return Err(DockerCompatError::StateMismatch);
    }

    let mut job_handle = 0u32;
    syscall::sys_job_create(0, 0, &mut job_handle).map_err(DockerCompatError::RuntimeJob)?;

    let mut process_handle = 0u32;
    let mut vmar_handle = 0u32;
    syscall::sys_process_create(
        job_handle,
        RUNC_PROCESS_NAME.as_ptr() as usize,
        RUNC_PROCESS_NAME.len(),
        0,
        &mut process_handle,
        &mut vmar_handle,
    )
    .map_err(DockerCompatError::RuntimeProcess)?;

    let mut thread_handle = 0u32;
    syscall::sys_thread_create(
        process_handle,
        RUNC_THREAD_NAME.as_ptr() as usize,
        RUNC_THREAD_NAME.len(),
        0,
        0,
        &mut thread_handle,
    )
    .map_err(DockerCompatError::RuntimeThread)?;
    syscall::sys_process_start(
        process_handle,
        thread_handle,
        RUNC_ENTRY_POINT,
        RUNC_STACK_TOP,
        0,
        0,
    )
    .map_err(DockerCompatError::RuntimeStart)?;

    Ok((job_handle, process_handle, thread_handle))
}

fn prepare_docker_pseudo_files(request: &OciRuntimeRequest<'_>) -> Result<(), DockerCompatError> {
    if !fxfs::init() {
        return Err(DockerCompatError::FxfsInit);
    }
    let cgroup_dir = cgroup_dir_path(request.cgroups_path);
    let cgroup_procs = cgroup_procs_path(request.cgroups_path);
    let apparmor_payload = apparmor_enforce_payload(request.apparmor_profile);

    let _ = fxfs::create_dir("/sys");
    let _ = fxfs::create_dir("/sys/fs");
    let _ = fxfs::create_dir(CGROUP_ROOT);
    let _ = fxfs::create_dir(&cgroup_dir);
    let _ = fxfs::create_dir("/proc");
    let _ = fxfs::create_dir("/proc/self");
    let _ = fxfs::create_dir("/proc/self/attr");
    let _ = fxfs::create_dir("/dev");
    let _ = fxfs::create_dir("/tmp");
    fxfs::write_file(&cgroup_procs, &[]).map_err(|_| DockerCompatError::FxfsPrepare)?;
    fxfs::write_file(APPARMOR_CURRENT, apparmor_payload.as_bytes())
        .map_err(|_| DockerCompatError::FxfsPrepare)?;
    fxfs::write_file(APPARMOR_EXEC, &[]).map_err(|_| DockerCompatError::FxfsPrepare)?;
    Ok(())
}

fn apply_oci_mounts(mount_count: usize) -> Result<(), DockerCompatError> {
    syscall::sys_mount(0, ROOT_PATH.as_ptr() as usize, 0, MS_REC | MS_PRIVATE, 0)
        .map_err(DockerCompatError::Mount)?;
    if mount_count >= 1 {
        syscall::sys_mount(
            PROC_FS.as_ptr() as usize,
            PROC_PATH.as_ptr() as usize,
            PROC_FS.as_ptr() as usize,
            MS_NOSUID | MS_NOEXEC | MS_NODEV,
            0,
        )
        .map_err(DockerCompatError::Mount)?;
    }
    if mount_count >= 2 {
        syscall::sys_mount(
            TMPFS.as_ptr() as usize,
            DEV_PATH.as_ptr() as usize,
            TMPFS.as_ptr() as usize,
            MS_NOSUID,
            0,
        )
        .map_err(DockerCompatError::Mount)?;
    }
    if mount_count >= 3 {
        syscall::sys_mount(
            ROOT_PATH.as_ptr() as usize,
            TMP_PATH.as_ptr() as usize,
            0,
            MS_BIND | MS_REC,
            0,
        )
        .map_err(DockerCompatError::Mount)?;
    }
    if mount_count > 3 {
        syscall::sys_mount(
            ROOT_PATH.as_ptr() as usize,
            OLD_ROOT_PATH.as_ptr() as usize,
            0,
            MS_BIND | MS_RDONLY,
            0,
        )
        .map_err(DockerCompatError::Mount)?;
    }
    Ok(())
}

fn apply_capabilities(effective: u64, permitted: u64) -> Result<(), DockerCompatError> {
    let mut cap_header = CapHeader {
        version: CAP_VERSION_3,
        pid: 0,
    };
    let mut caps = [CapData {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    syscall::sys_capget(
        &mut cap_header as *mut CapHeader as usize,
        caps.as_mut_ptr() as usize,
    )
    .map_err(DockerCompatError::CapGet)?;
    caps[0].effective = effective as u32;
    caps[0].permitted = permitted as u32;
    caps[0].inheritable = 0;
    caps[1].effective = (effective >> 32) as u32;
    caps[1].permitted = (permitted >> 32) as u32;
    caps[1].inheritable = 0;
    syscall::sys_capset(
        &mut cap_header as *mut CapHeader as usize,
        caps.as_ptr() as usize,
    )
    .map_err(DockerCompatError::CapSet)?;
    Ok(())
}

fn write_linux_file(
    path: &[u8],
    payload: &[u8],
    open_err: fn(SysError) -> DockerCompatError,
    write_err: fn(SysError) -> DockerCompatError,
    close_err: fn(SysError) -> DockerCompatError,
) -> Result<usize, DockerCompatError> {
    let fd = syscall::sys_openat(AT_FDCWD, path.as_ptr() as usize, O_WRONLY_CREATE_TRUNC, 0)
        .map_err(open_err)?;
    let written = match syscall::sys_write(fd, payload.as_ptr() as usize, payload.len()) {
        Ok(written) => written,
        Err(err) => {
            let _ = syscall::sys_close(fd);
            return Err(write_err(err));
        }
    };
    syscall::sys_close(fd).map_err(close_err)?;
    Ok(written)
}

fn parse_oci_runtime_config(input: &str) -> Result<OciRuntimeRequest<'_>, DockerCompatError> {
    let oci_version = json_string_after(input, "ociVersion")?;
    if !oci_version.starts_with("1.") {
        return Err(DockerCompatError::OciParse);
    }

    let root = json_object_after(input, "root")?;
    let root_path = json_string_after(root, "path")?;
    if root_path.is_empty() || root_path.as_bytes().contains(&0) {
        return Err(DockerCompatError::OciParse);
    }

    let process = json_object_after(input, "process")?;
    let args = json_array_after(process, "args")?;
    let env = json_array_after(process, "env")?;
    let arg0 = first_string_in_array(args)?;
    let no_new_privs = json_bool_after(process, "noNewPrivileges")?;
    let apparmor_profile = json_string_after(process, "apparmorProfile")?;
    let capabilities = json_object_after(process, "capabilities")?;
    let effective = capability_mask_from_array(json_array_after(capabilities, "effective")?);
    let permitted = capability_mask_from_array(json_array_after(capabilities, "permitted")?);

    let hostname = json_string_after(input, "hostname")?;
    let mounts = json_array_after(input, "mounts")?;
    let linux = json_object_after(input, "linux")?;
    let cgroups_path = json_string_after(linux, "cgroupsPath")?;
    let namespaces = json_array_after(linux, "namespaces")?;
    let masked_paths = json_array_after(linux, "maskedPaths")?;
    let readonly_paths = json_array_after(linux, "readonlyPaths")?;
    let seccomp = json_object_after(linux, "seccomp")?;
    let seccomp_default = json_string_after(seccomp, "defaultAction")?;

    if !simple_relative_name(cgroups_path)
        || apparmor_profile.is_empty()
        || hostname.is_empty()
        || hostname.len() > 64
        || seccomp_default.is_empty()
    {
        return Err(DockerCompatError::OciParse);
    }

    Ok(OciRuntimeRequest {
        root_path,
        arg0,
        args: json_string_array_count(args),
        env: json_string_array_count(env),
        hostname,
        cgroups_path,
        apparmor_profile,
        namespace_flags: namespace_flags_from_array(namespaces),
        mount_count: json_key_count(mounts, "destination"),
        masked_paths: json_string_array_count(masked_paths),
        readonly_paths: json_string_array_count(readonly_paths),
        cap_effective: effective,
        cap_permitted: permitted,
        no_new_privs,
        seccomp_filter: true,
    })
}

fn json_value_pos(input: &str, key: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let key_bytes = key.as_bytes();
    if key_bytes.is_empty() || bytes.len() < key_bytes.len().saturating_add(3) {
        return None;
    }

    let mut pos = 0usize;
    while pos + key_bytes.len() + 2 <= bytes.len() {
        if bytes[pos] == b'"'
            && bytes[pos + 1..].starts_with(key_bytes)
            && bytes.get(pos + 1 + key_bytes.len()) == Some(&b'"')
        {
            let mut cursor = pos + 2 + key_bytes.len();
            skip_ws(bytes, &mut cursor);
            if bytes.get(cursor) != Some(&b':') {
                pos += 1;
                continue;
            }
            cursor += 1;
            skip_ws(bytes, &mut cursor);
            return Some(cursor);
        }
        pos += 1;
    }
    None
}

fn json_string_after<'a>(input: &'a str, key: &str) -> Result<&'a str, DockerCompatError> {
    let bytes = input.as_bytes();
    let mut pos = json_value_pos(input, key).ok_or(DockerCompatError::OciParse)?;
    if bytes.get(pos) != Some(&b'"') {
        return Err(DockerCompatError::OciParse);
    }
    pos += 1;
    let start = pos;
    while pos < bytes.len() {
        match bytes[pos] {
            b'"' => return Ok(&input[start..pos]),
            b'\\' => return Err(DockerCompatError::OciParse),
            byte if byte < 0x20 => return Err(DockerCompatError::OciParse),
            _ => pos += 1,
        }
    }
    Err(DockerCompatError::OciParse)
}

fn json_bool_after(input: &str, key: &str) -> Result<bool, DockerCompatError> {
    let pos = json_value_pos(input, key).ok_or(DockerCompatError::OciParse)?;
    let tail = &input.as_bytes()[pos..];
    if tail.starts_with(b"true") {
        Ok(true)
    } else if tail.starts_with(b"false") {
        Ok(false)
    } else {
        Err(DockerCompatError::OciParse)
    }
}

fn json_object_after<'a>(input: &'a str, key: &str) -> Result<&'a str, DockerCompatError> {
    json_block_after(input, key, b'{', b'}')
}

fn json_array_after<'a>(input: &'a str, key: &str) -> Result<&'a str, DockerCompatError> {
    json_block_after(input, key, b'[', b']')
}

fn json_block_after<'a>(
    input: &'a str,
    key: &str,
    open: u8,
    close: u8,
) -> Result<&'a str, DockerCompatError> {
    let bytes = input.as_bytes();
    let start = json_value_pos(input, key).ok_or(DockerCompatError::OciParse)?;
    if bytes.get(start) != Some(&open) {
        return Err(DockerCompatError::OciParse);
    }
    let mut pos = start;
    let mut depth = 0usize;
    let mut in_string = false;
    while pos < bytes.len() {
        let byte = bytes[pos];
        if in_string {
            match byte {
                b'"' => in_string = false,
                b'\\' => return Err(DockerCompatError::OciParse),
                _ => {}
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b if b == open => depth = depth.saturating_add(1),
                b if b == close => {
                    depth = depth.checked_sub(1).ok_or(DockerCompatError::OciParse)?;
                    if depth == 0 {
                        return Ok(&input[start..=pos]);
                    }
                }
                _ => {}
            }
        }
        pos += 1;
    }
    Err(DockerCompatError::OciParse)
}

fn first_string_in_array(input: &str) -> Result<&str, DockerCompatError> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'"' {
            let start = pos + 1;
            pos = start;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'"' => return Ok(&input[start..pos]),
                    b'\\' => return Err(DockerCompatError::OciParse),
                    _ => pos += 1,
                }
            }
        }
        pos += 1;
    }
    Err(DockerCompatError::OciParse)
}

fn json_string_array_count(input: &str) -> usize {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'"' {
            count = count.saturating_add(1);
            pos += 1;
            while pos < bytes.len() && bytes[pos] != b'"' {
                pos += 1;
            }
        }
        pos += 1;
    }
    count
}

fn json_key_count(input: &str, key: &str) -> usize {
    let bytes = input.as_bytes();
    let key_bytes = key.as_bytes();
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos + key_bytes.len() + 2 <= bytes.len() {
        if bytes[pos] == b'"'
            && bytes[pos + 1..].starts_with(key_bytes)
            && bytes.get(pos + 1 + key_bytes.len()) == Some(&b'"')
        {
            count = count.saturating_add(1);
            pos += key_bytes.len() + 2;
        } else {
            pos += 1;
        }
    }
    count
}

fn array_contains_string(input: &str, value: &str) -> bool {
    let bytes = input.as_bytes();
    let value_bytes = value.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'"' {
            let start = pos + 1;
            pos = start;
            while pos < bytes.len() && bytes[pos] != b'"' {
                pos += 1;
            }
            if pos <= bytes.len() && &bytes[start..pos] == value_bytes {
                return true;
            }
        }
        pos += 1;
    }
    false
}

fn namespace_flags_from_array(input: &str) -> usize {
    let mut flags = 0usize;
    if array_contains_string(input, "mount") {
        flags |= CLONE_NEWNS;
    }
    if array_contains_string(input, "cgroup") {
        flags |= CLONE_NEWCGROUP;
    }
    if array_contains_string(input, "uts") {
        flags |= CLONE_NEWUTS;
    }
    if array_contains_string(input, "ipc") {
        flags |= CLONE_NEWIPC;
    }
    if array_contains_string(input, "user") {
        flags |= CLONE_NEWUSER;
    }
    if array_contains_string(input, "pid") {
        flags |= CLONE_NEWPID;
    }
    if array_contains_string(input, "network") {
        flags |= CLONE_NEWNET;
    }
    flags
}

fn capability_mask_from_array(input: &str) -> u64 {
    let caps = [
        ("CAP_CHOWN", 0u32),
        ("CAP_DAC_OVERRIDE", 1),
        ("CAP_DAC_READ_SEARCH", 2),
        ("CAP_FOWNER", 3),
        ("CAP_FSETID", 4),
        ("CAP_KILL", 5),
        ("CAP_SETGID", 6),
        ("CAP_SETUID", 7),
        ("CAP_SETPCAP", 8),
        ("CAP_NET_BIND_SERVICE", 10),
        ("CAP_NET_RAW", 13),
        ("CAP_SYS_CHROOT", 18),
        ("CAP_MKNOD", 27),
        ("CAP_AUDIT_WRITE", 29),
        ("CAP_SETFCAP", 31),
    ];
    let mut mask = 0u64;
    for (name, bit) in caps {
        if array_contains_string(input, name) {
            mask |= 1u64 << bit;
        }
    }
    mask
}

fn default_docker_capability_mask() -> u64 {
    let caps = [0u32, 1, 3, 4, 5, 6, 7, 8, 10, 13, 18, 27, 29, 31];
    let mut mask = 0u64;
    for bit in caps {
        mask |= 1u64 << bit;
    }
    mask
}

fn normalize_image_reference(image: &str) -> Result<String, DockerCompatError> {
    let trimmed = image.trim();
    if trimmed.is_empty() || trimmed.len() > 160 || trimmed.as_bytes().contains(&0) {
        return Err(DockerCompatError::ImageInvalid);
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(DockerCompatError::ImageInvalid);
    }
    let mut out = String::from(trimmed);
    if !image_reference_has_tag(out.as_str()) {
        out.push_str(":latest");
    }
    if !image_reference_valid(out.as_str()) {
        return Err(DockerCompatError::ImageInvalid);
    }
    Ok(out)
}

fn image_reference_has_tag(value: &str) -> bool {
    let last_slash = value.rfind('/').unwrap_or(0);
    let tag_search = if last_slash == 0 && !value.starts_with('/') {
        value
    } else {
        &value[last_slash + 1..]
    };
    tag_search.contains(':')
}

fn image_reference_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'.' | b'-' | b'_' | b':'))
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && value.contains(':')
}

fn image_reference_valid_with_optional_tag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'.' | b'-' | b'_' | b':'))
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
}

fn image_storage_key(image: &str) -> String {
    let mut out = String::new();
    for byte in image.bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push(byte as char);
        } else {
            out.push('_');
        }
    }
    out
}

fn parse_ipv4_addr(input: &str) -> Option<[u8; 4]> {
    let bytes = input.as_bytes();
    let mut octets = [0u8; 4];
    let mut octet_index = 0usize;
    let mut value = 0u16;
    let mut digits = 0usize;
    for byte in bytes {
        match *byte {
            b'0'..=b'9' => {
                value = value.checked_mul(10)?.checked_add((byte - b'0') as u16)?;
                if value > 255 {
                    return None;
                }
                digits += 1;
                if digits > 3 {
                    return None;
                }
            }
            b'.' => {
                if digits == 0 || octet_index >= 3 {
                    return None;
                }
                octets[octet_index] = value as u8;
                octet_index += 1;
                value = 0;
                digits = 0;
            }
            _ => return None,
        }
    }
    if digits == 0 || octet_index != 3 {
        return None;
    }
    octets[octet_index] = value as u8;
    Some(octets)
}

fn resolve_builtin_image_name(image: &str) -> Option<&'static str> {
    match image {
        SAMPLE_IMAGE_NAME | SAMPLE_IMAGE_ALIAS | SAMPLE_IMAGE_SHORT | "hello-world" | "hello" => {
            Some(SAMPLE_IMAGE_NAME)
        }
        _ => None,
    }
}

fn push_runtime_args(
    out: &mut String,
    image_config: &str,
    command: &[&str],
) -> Result<(), DockerCompatError> {
    out.push('[');
    if command.is_empty() {
        let config = json_object_after(image_config, "config")?;
        let entrypoint = json_array_after(config, "Entrypoint").unwrap_or("[]");
        let cmd = json_array_after(config, "Cmd").unwrap_or("[]");
        let mut needs_comma = false;
        push_string_array_items(out, entrypoint, &mut needs_comma)?;
        push_string_array_items(out, cmd, &mut needs_comma)?;
    } else {
        let mut index = 0usize;
        while index < command.len() {
            if index > 0 {
                out.push_str(", ");
            }
            push_json_string(out, command[index])?;
            index += 1;
        }
    }
    out.push(']');
    Ok(())
}

fn push_string_array_items(
    out: &mut String,
    input: &str,
    needs_comma: &mut bool,
) -> Result<(), DockerCompatError> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'"' {
            let start = pos + 1;
            pos = start;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'"' => {
                        if *needs_comma {
                            out.push_str(", ");
                        }
                        push_json_string(out, &input[start..pos])?;
                        *needs_comma = true;
                        break;
                    }
                    b'\\' => return Err(DockerCompatError::OciParse),
                    _ => pos += 1,
                }
            }
        }
        pos += 1;
    }
    Ok(())
}

fn push_json_string(out: &mut String, value: &str) -> Result<(), DockerCompatError> {
    if value.as_bytes().contains(&0) {
        return Err(DockerCompatError::OciParse);
    }
    out.push('"');
    for byte in value.bytes() {
        match byte {
            b'"' | b'\\' | 0x00..=0x1f => return Err(DockerCompatError::OciParse),
            _ => out.push(byte as char),
        }
    }
    out.push('"');
    Ok(())
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while let Some(byte) = bytes.get(*pos).copied() {
        if matches!(byte, b' ' | b'\n' | b'\r' | b'\t') {
            *pos += 1;
        } else {
            break;
        }
    }
}

fn simple_relative_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.'))
}

fn cgroup_dir_path(cgroups_path: &str) -> String {
    let mut out = String::from(CGROUP_ROOT);
    out.push('/');
    out.push_str(cgroups_path);
    out
}

fn cgroup_procs_path(cgroups_path: &str) -> String {
    let mut out = cgroup_dir_path(cgroups_path);
    out.push_str("/cgroup.procs");
    out
}

fn apparmor_enforce_payload(profile: &str) -> String {
    let mut out = String::from(profile);
    out.push_str(" (enforce)\n");
    out
}

fn c_string(path: &str) -> String {
    let mut out = String::from(path);
    out.push('\0');
    out
}

fn usize_to_string(mut value: usize) -> String {
    if value == 0 {
        return String::from("0");
    }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while value != 0 && len < buf.len() {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    let mut out = String::new();
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
    out
}

fn signed_to_string(value: i32) -> String {
    if value < 0 {
        let mut out = String::from("-");
        out.push_str(usize_to_string(value.unsigned_abs() as usize).as_str());
        out
    } else {
        usize_to_string(value as usize)
    }
}

fn append_usize(out: &mut String, mut value: usize, min_width: usize) {
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        buf[len] = b'0';
        len += 1;
    }
    while value != 0 && len < buf.len() {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len < min_width && len < buf.len() {
        buf[len] = b'0';
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
}

fn parse_usize(value: &str) -> Option<usize> {
    if value.is_empty() {
        return None;
    }
    let mut out = 0usize;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((byte - b'0') as usize)?;
    }
    Some(out)
}

fn parse_i32(value: &str) -> Option<i32> {
    if let Some(rest) = value.strip_prefix('-') {
        let abs = parse_usize(rest)?;
        if abs > i32::MAX as usize + 1 {
            return None;
        }
        if abs == i32::MAX as usize + 1 {
            Some(i32::MIN)
        } else {
            Some(-(abs as i32))
        }
    } else {
        let parsed = parse_usize(value)?;
        if parsed > i32::MAX as usize {
            return None;
        }
        Some(parsed as i32)
    }
}
