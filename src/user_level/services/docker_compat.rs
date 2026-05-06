//! Docker/runc compatibility bring-up for SMROS userspace.
//!
//! This is the next step after the raw syscall smoke test: install a compact
//! OCI-style bundle, parse its `config.json` subset, create the runtime task
//! with boot-time capability rights, and apply the Linux container surfaces
//! that runc/containerd expect first.

#![allow(dead_code)]

use alloc::string::String;

use crate::kernel_objects::right;
use crate::kernel_objects::{Rights, ZxError};
use crate::syscall::{self, SysError};
use crate::user_level::fxfs;

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
const SAMPLE_IMAGE_NAME: &str = "smros/hello:latest";
const SAMPLE_IMAGE_ALIAS: &str = "hello-world:latest";
const SAMPLE_IMAGE_SHORT: &str = "smros/hello";
const SAMPLE_IMAGE_DIR: &str = "/docker/images/smros_hello_latest";
const SAMPLE_IMAGE_ROOTFS: &str = "/docker/images/smros_hello_latest/rootfs";
const SAMPLE_IMAGE_MANIFEST: &str = "/docker/images/smros_hello_latest/manifest.json";
const SAMPLE_IMAGE_CONFIG: &str = "/docker/images/smros_hello_latest/config.json";
const DOCKER_IMAGE_CONFIG_MAX_BYTES: usize = 4096;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockerImageInfo {
    pub name: &'static str,
    pub rootfs: &'static str,
    pub manifest_bytes: usize,
    pub config_bytes: usize,
    pub layers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockerRunResult {
    pub image: &'static str,
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
    Ok(())
}

pub fn builtin_image_info() -> Result<DockerImageInfo, DockerCompatError> {
    install_builtin_docker_images()?;
    let manifest_bytes = fxfs::attrs(SAMPLE_IMAGE_MANIFEST)
        .map_err(|_| DockerCompatError::OciRead)?
        .size;
    let config_bytes = fxfs::attrs(SAMPLE_IMAGE_CONFIG)
        .map_err(|_| DockerCompatError::OciRead)?
        .size;
    Ok(DockerImageInfo {
        name: SAMPLE_IMAGE_NAME,
        rootfs: SAMPLE_IMAGE_ROOTFS,
        manifest_bytes,
        config_bytes,
        layers: 1,
    })
}

pub fn run_docker_image(
    image: &str,
    command: &[&str],
) -> Result<DockerRunResult, DockerCompatError> {
    install_builtin_docker_images()?;
    let image_name = resolve_builtin_image_name(image).ok_or(DockerCompatError::OciRead)?;

    let mut config_bytes = [0u8; DOCKER_IMAGE_CONFIG_MAX_BYTES];
    let config_len = fxfs::read_file(SAMPLE_IMAGE_CONFIG, &mut config_bytes)
        .map_err(|_| DockerCompatError::OciRead)?;
    let config = core::str::from_utf8(&config_bytes[..config_len])
        .map_err(|_| DockerCompatError::OciParse)?;
    let request = docker_image_config_to_oci_request(config, image_name, command)?;
    let runtime = run_oci_runtime_request(&request, config_len)?;
    Ok(DockerRunResult {
        image: image_name,
        runtime,
    })
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
    image_config: &'a str,
    image_name: &str,
    command: &'a [&'a str],
) -> Result<OciRuntimeRequest<'a>, DockerCompatError> {
    let config = json_object_after(image_config, "config")?;
    let env = json_array_after(config, "Env")?;
    let entrypoint = json_array_after(config, "Entrypoint")?;
    let cmd = json_array_after(config, "Cmd")?;
    let working_dir = json_string_after(config, "WorkingDir")?;
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

    install_runtime_bundle_for_image(image_name, image_config, command)?;

    Ok(OciRuntimeRequest {
        root_path: SAMPLE_IMAGE_ROOTFS,
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
    image_name: &str,
    image_config: &str,
    command: &[&str],
) -> Result<(), DockerCompatError> {
    let mut bundle = String::from("{\n");
    bundle.push_str("  \"ociVersion\": \"1.1.0\",\n");
    bundle.push_str("  \"root\": { \"path\": \"");
    bundle.push_str(SAMPLE_IMAGE_ROOTFS);
    bundle.push_str("\", \"readonly\": true },\n");
    bundle.push_str("  \"process\": {\n");
    bundle.push_str("    \"terminal\": false,\n");
    bundle.push_str("    \"cwd\": \"/\",\n");
    bundle.push_str("    \"args\": ");
    push_runtime_args(&mut bundle, image_config, command)?;
    bundle.push_str(",\n");
    bundle.push_str("    \"env\": ");
    bundle.push_str(json_array_after(
        json_object_after(image_config, "config")?,
        "Env",
    )?);
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
    bundle.push_str(image_name);
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
        let entrypoint = json_array_after(config, "Entrypoint")?;
        let cmd = json_array_after(config, "Cmd")?;
        push_string_array_items(out, entrypoint, false)?;
        push_string_array_items(out, cmd, json_string_array_count(entrypoint) != 0)?;
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
    mut needs_comma: bool,
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
                        if needs_comma {
                            out.push_str(", ");
                        }
                        push_json_string(out, &input[start..pos])?;
                        needs_comma = true;
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
