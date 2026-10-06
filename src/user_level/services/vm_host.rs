//! Host-side QEMU launcher client for modeled VMs.
//!
//! SMROS cannot directly create host GUI windows from inside the guest. This
//! client asks a small host daemon on the QEMU user-network gateway to spawn a
//! real nested QEMU process for a configured Linux kernel.

#![allow(dead_code)]

use alloc::string::{String, ToString};

use crate::kernel_objects::hypervisor::{VmHostConfig, VmRecord};
use crate::user_level::net::{self, NetError, NetworkSocketAddr};

pub const DEFAULT_LAUNCHER_PORT: u16 = 7070;
const MAX_REQUEST_BYTES: usize = 2048;
const MAX_RESPONSE_BYTES: usize = 512;
const RESPONSE_READ_ATTEMPTS: usize = 8;
/// Host ST can take the full smoke timeout plus kernel build. Keep the TCP
/// session open until that completes instead of reporting UNAVAILABLE.
const HERMES_TEST_WAIT_NANOS: u64 = 6 * 60 * 1_000_000_000;
/// After guest reset, virtio-net and QEMU slirp need a bounded wait before the
/// launcher handshake succeeds.
const HERMES_HOST_READY_WAIT_NANOS: u64 = 30 * 1_000_000_000;
/// Resume after PSCI reset waits longer: the NIC, slirp, and host launcher can
/// take more than one connect attempt to accept TCP again.
const HERMES_HOST_RESUME_WAIT_NANOS: u64 = 60 * 1_000_000_000;
const HERMES_TEST_CONNECT_ATTEMPTS: usize = 3;
const HERMES_TEST_JOB_ATTEMPTS: usize = 3;
/// Nested Linux boot can take tens of seconds; keep the wait-boot TCP session
/// open until the launcher sees the initramfs banner or times out.
const HERMES_VM_BOOT_WAIT_NANOS: u64 = 60 * 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmHostError {
    NoHostConfig,
    HostUnavailable,
    InvalidConfig,
    RequestTooLarge,
    Connect(NetError),
    Write(NetError),
    Read(NetError),
    ResponseInvalid,
    LaunchDenied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmHostLaunch {
    pub qemu_pid: u32,
    pub log_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesHostTestJob {
    Ut,
    It,
}

impl HermesHostTestJob {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ut => "ut",
            Self::It => "it",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesHostTestResult {
    pub job: HermesHostTestJob,
    pub passed: bool,
    pub summary: String,
}

pub fn wait_for_host_transport() -> Result<(), VmHostError> {
    let deadline = crate::kernel_lowlevel::timer::get_nanoseconds()
        .saturating_add(HERMES_HOST_RESUME_WAIT_NANOS);
    let mut last_err = VmHostError::HostUnavailable;
    const PING: &[u8] = b"SMROS_VM_PING 1\nname=__smros_resume__\nend\n";
    loop {
        let now = crate::kernel_lowlevel::timer::get_nanoseconds();
        if now >= deadline {
            return Err(last_err);
        }
        match connect_launcher_wait(DEFAULT_LAUNCHER_PORT, deadline - now) {
            Ok(mut socket) => {
                if let Err(err) = socket.write(PING) {
                    let _ = socket.close();
                    last_err = VmHostError::Write(err);
                } else {
                    let mut response = [0u8; MAX_RESPONSE_BYTES];
                    match read_response(&mut socket, &mut response, RESPONSE_READ_ATTEMPTS) {
                        Ok(bytes) => {
                            let _ = socket.close();
                            match core::str::from_utf8(&response[..bytes]) {
                                Ok(text) if text.starts_with("OK ") => return Ok(()),
                                Ok(_) => last_err = VmHostError::HostUnavailable,
                                Err(_) => last_err = VmHostError::ResponseInvalid,
                            }
                        }
                        Err(err) => {
                            let _ = socket.close();
                            last_err = err;
                        }
                    }
                }
            }
            Err(err) => last_err = err,
        }
        crate::kernel_objects::scheduler::yield_now();
    }
}

pub fn run_hermes_test(job: HermesHostTestJob) -> Result<HermesHostTestResult, VmHostError> {
    let mut last_ok: Option<HermesHostTestResult> = None;
    let mut last_err = VmHostError::HostUnavailable;
    for attempt in 0..HERMES_TEST_JOB_ATTEMPTS {
        match run_hermes_test_once(job) {
            Ok(result) if result.passed => return Ok(result),
            Ok(result) => last_ok = Some(result),
            Err(err) if retryable_host_job(&err) => last_err = err,
            Err(err) => return Err(err),
        }
        if attempt + 1 < HERMES_TEST_JOB_ATTEMPTS {
            crate::kernel_objects::scheduler::yield_now();
        }
    }
    if let Some(result) = last_ok {
        Ok(result)
    } else {
        Err(last_err)
    }
}

fn run_hermes_test_once(job: HermesHostTestJob) -> Result<HermesHostTestResult, VmHostError> {
    let request = build_hermes_test_request(job);
    let mut last_err = VmHostError::HostUnavailable;
    for attempt in 0..HERMES_TEST_CONNECT_ATTEMPTS {
        match connect_launcher(DEFAULT_LAUNCHER_PORT) {
            Ok(mut socket) => {
                if let Err(err) = socket.write(request.as_bytes()) {
                    let _ = socket.close();
                    last_err = VmHostError::Write(err);
                    continue;
                }
                let mut response = [0u8; MAX_RESPONSE_BYTES];
                let deadline = crate::kernel_lowlevel::timer::get_nanoseconds()
                    .saturating_add(HERMES_TEST_WAIT_NANOS);
                match read_response_until(&mut socket, &mut response, deadline) {
                    Ok(bytes) => {
                        let _ = socket.close();
                        return parse_hermes_test_response(job, &response[..bytes]);
                    }
                    Err(err) => {
                        let _ = socket.close();
                        return Err(err);
                    }
                }
            }
            Err(err) if retryable_host_connect(&err) => {
                last_err = err;
                if attempt + 1 < HERMES_TEST_CONNECT_ATTEMPTS {
                    crate::kernel_objects::scheduler::yield_now();
                }
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_err)
}

fn retryable_host_connect(err: &VmHostError) -> bool {
    matches!(err, VmHostError::HostUnavailable | VmHostError::Connect(_))
}

fn retryable_host_job(err: &VmHostError) -> bool {
    matches!(
        err,
        VmHostError::HostUnavailable
            | VmHostError::Connect(_)
            | VmHostError::Write(_)
            | VmHostError::Read(_)
            | VmHostError::ResponseInvalid
    )
}

fn build_hermes_test_request(job: HermesHostTestJob) -> String {
    let mut request = String::from("SMROS_TEST_RUN 1\njob=");
    request.push_str(job.as_str());
    request.push_str("\nend\n");
    request
}

fn parse_hermes_test_response(
    job: HermesHostTestJob,
    response: &[u8],
) -> Result<HermesHostTestResult, VmHostError> {
    let text = core::str::from_utf8(response).map_err(|_| VmHostError::ResponseInvalid)?;
    let passed = text.starts_with("OK ") && text.contains("status=0");
    if !passed && !text.starts_with("ERR ") {
        return Err(VmHostError::ResponseInvalid);
    }
    let summary = text
        .split_whitespace()
        .find_map(|field| field.strip_prefix("summary="))
        .unwrap_or(if passed { "passed" } else { "failed" });
    Ok(HermesHostTestResult {
        job,
        passed,
        summary: summary.to_string(),
    })
}

pub fn wait_for_linux_boot(name: &str) -> Result<bool, VmHostError> {
    let request = build_wait_boot_request(name)?;
    let mut socket = connect_launcher(DEFAULT_LAUNCHER_PORT)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;
    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let deadline =
        crate::kernel_lowlevel::timer::get_nanoseconds().saturating_add(HERMES_VM_BOOT_WAIT_NANOS);
    let bytes = read_response_until(&mut socket, &mut response, deadline)?;
    let _ = socket.close();
    parse_wait_boot_response(&response[..bytes])
}

pub fn launch(vm: &VmRecord) -> Result<VmHostLaunch, VmHostError> {
    let host = vm.host.as_ref().ok_or(VmHostError::NoHostConfig)?;
    let request = build_launch_request(vm, host)?;
    let mut socket = connect_launcher(host.launcher_port)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;

    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = read_response(&mut socket, &mut response, RESPONSE_READ_ATTEMPTS)?;
    let _ = socket.close();
    parse_launch_response(&response[..bytes])
}

pub fn stop(vm: &VmRecord) -> Result<(), VmHostError> {
    let Some(host) = vm.host.as_ref() else {
        return Ok(());
    };
    if vm.host_qemu_pid == 0 {
        return Ok(());
    }
    let request = build_stop_request(vm, host)?;
    let mut socket = connect_launcher(host.launcher_port)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;

    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = read_response(&mut socket, &mut response, RESPONSE_READ_ATTEMPTS)?;
    let _ = socket.close();
    parse_stop_response(&response[..bytes])
}

pub fn sync_trace(path: &str, trace: &[u8]) -> Result<(), VmHostError> {
    let request = build_trace_sync_request(path, trace.len())?;
    let mut socket = connect_launcher(DEFAULT_LAUNCHER_PORT)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;

    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = read_response(&mut socket, &mut response, RESPONSE_READ_ATTEMPTS)?;
    let _ = socket.close();
    parse_stop_response(&response[..bytes])
}

fn connect_launcher(port: u16) -> Result<net::TcpSocket, VmHostError> {
    connect_launcher_wait(port, HERMES_HOST_READY_WAIT_NANOS)
}

fn connect_launcher_wait(port: u16, wait_nanos: u64) -> Result<net::TcpSocket, VmHostError> {
    let deadline = crate::kernel_lowlevel::timer::get_nanoseconds().saturating_add(wait_nanos);
    let _ = net::wait_until_ready(deadline);
    loop {
        match net::tcp_connect(NetworkSocketAddr {
            ip: net::QEMU_USER_GATEWAY,
            port,
        }) {
            Ok(socket) => return Ok(socket),
            Err(err) => {
                if crate::kernel_lowlevel::timer::get_nanoseconds() >= deadline {
                    return Err(map_connect_error(err));
                }
                crate::kernel_objects::scheduler::yield_now();
            }
        }
    }
}

fn map_connect_error(err: NetError) -> VmHostError {
    match err {
        NetError::NotReady => VmHostError::HostUnavailable,
        other => VmHostError::Connect(other),
    }
}

fn read_response(
    socket: &mut net::TcpSocket,
    response: &mut [u8],
    attempts: usize,
) -> Result<usize, VmHostError> {
    let mut last_timeout = false;
    for _ in 0..attempts {
        match socket.read(response) {
            Ok(0) => return Err(VmHostError::ResponseInvalid),
            Ok(bytes) => return Ok(bytes),
            Err(NetError::Timeout) => {
                last_timeout = true;
            }
            Err(err) => return Err(VmHostError::Read(err)),
        }
    }
    if last_timeout {
        Err(VmHostError::Read(NetError::Timeout))
    } else {
        Err(VmHostError::ResponseInvalid)
    }
}

fn read_response_until(
    socket: &mut net::TcpSocket,
    response: &mut [u8],
    deadline_ns: u64,
) -> Result<usize, VmHostError> {
    loop {
        match socket.read(response) {
            Ok(0) => return Err(VmHostError::ResponseInvalid),
            Ok(bytes) => return Ok(bytes),
            Err(NetError::Timeout) => {
                if crate::kernel_lowlevel::timer::get_nanoseconds() >= deadline_ns {
                    return Err(VmHostError::Read(NetError::Timeout));
                }
                let _ = socket.keepalive();
            }
            Err(err) => return Err(VmHostError::Read(err)),
        }
    }
}

fn build_launch_request(vm: &VmRecord, host: &VmHostConfig) -> Result<String, VmHostError> {
    let mut request = String::from("SMROS_VM_LAUNCH 1\n");
    push_kv(&mut request, "name", vm.name.as_str())?;
    push_kv(&mut request, "kernel", host.kernel_path.as_str())?;
    push_optional_kv(&mut request, "initrd", host.initrd_path.as_ref())?;
    push_optional_kv(&mut request, "dtb", host.dtb_path.as_ref())?;
    push_optional_kv(&mut request, "disk", host.disk_path.as_ref())?;
    push_kv(&mut request, "disk_format", host.disk_format.as_str())?;
    push_kv(&mut request, "append", host.append.as_str())?;
    push_kv(&mut request, "machine", host.qemu_machine.as_str())?;
    push_kv(&mut request, "cpu", host.qemu_cpu.as_str())?;
    push_kv(&mut request, "smp", u32_to_string(host.qemu_smp).as_str())?;
    push_kv(&mut request, "memory", host.qemu_memory.as_str())?;
    push_kv(&mut request, "display", host.qemu_display.as_str())?;
    push_kv(&mut request, "serial", host.qemu_serial.as_str())?;
    request.push_str("end\n");
    if request.len() > MAX_REQUEST_BYTES {
        return Err(VmHostError::RequestTooLarge);
    }
    Ok(request)
}

fn build_trace_sync_request(path: &str, trace_len: usize) -> Result<String, VmHostError> {
    let mut request = String::from("SMROS_TRACE_SYNC 1\n");
    push_kv(&mut request, "path", path)?;
    push_kv(&mut request, "bytes", usize_to_string(trace_len).as_str())?;
    request.push_str("end\n");
    if request.len() > MAX_REQUEST_BYTES {
        return Err(VmHostError::RequestTooLarge);
    }
    Ok(request)
}

fn build_wait_boot_request(name: &str) -> Result<String, VmHostError> {
    if !wire_value_valid(name) {
        return Err(VmHostError::InvalidConfig);
    }
    let mut request = String::from("SMROS_VM_WAIT_BOOT 1\n");
    push_kv(&mut request, "name", name)?;
    request.push_str("end\n");
    if request.len() > MAX_REQUEST_BYTES {
        return Err(VmHostError::RequestTooLarge);
    }
    Ok(request)
}

fn parse_wait_boot_response(response: &[u8]) -> Result<bool, VmHostError> {
    let text = core::str::from_utf8(response).map_err(|_| VmHostError::ResponseInvalid)?;
    if !text.starts_with("OK ") {
        return Err(VmHostError::LaunchDenied);
    }
    Ok(text.contains("boot=1"))
}

fn build_stop_request(vm: &VmRecord, host: &VmHostConfig) -> Result<String, VmHostError> {
    let mut request = String::from("SMROS_VM_STOP 1\n");
    push_kv(&mut request, "name", vm.name.as_str())?;
    push_kv(
        &mut request,
        "pid",
        u32_to_string(vm.host_qemu_pid).as_str(),
    )?;
    push_kv(
        &mut request,
        "port",
        u32_to_string(host.launcher_port as u32).as_str(),
    )?;
    request.push_str("end\n");
    if request.len() > MAX_REQUEST_BYTES {
        return Err(VmHostError::RequestTooLarge);
    }
    Ok(request)
}

fn push_optional_kv(
    request: &mut String,
    key: &str,
    value: Option<&String>,
) -> Result<(), VmHostError> {
    if let Some(value) = value {
        push_kv(request, key, value.as_str())?;
    }
    Ok(())
}

fn push_kv(request: &mut String, key: &str, value: &str) -> Result<(), VmHostError> {
    if !wire_value_valid(key) || !wire_value_valid(value) {
        return Err(VmHostError::InvalidConfig);
    }
    request.push_str(key);
    request.push('=');
    request.push_str(value);
    request.push('\n');
    Ok(())
}

fn wire_value_valid(value: &str) -> bool {
    if value.is_empty() || value.len() > 512 {
        return false;
    }
    for byte in value.bytes() {
        if byte == b'\n' || byte == b'\r' || byte == 0 {
            return false;
        }
    }
    true
}

fn parse_launch_response(response: &[u8]) -> Result<VmHostLaunch, VmHostError> {
    let text = core::str::from_utf8(response).map_err(|_| VmHostError::ResponseInvalid)?;
    if !text.starts_with("OK ") {
        return Err(VmHostError::LaunchDenied);
    }
    let pid = find_response_number(text, "pid=").ok_or(VmHostError::ResponseInvalid)?;
    if pid == 0 {
        return Err(VmHostError::ResponseInvalid);
    }
    let log_path = find_response_value(text, "log=")
        .map(String::from)
        .unwrap_or_else(String::new);
    Ok(VmHostLaunch {
        qemu_pid: pid,
        log_path,
    })
}

fn parse_stop_response(response: &[u8]) -> Result<(), VmHostError> {
    let text = core::str::from_utf8(response).map_err(|_| VmHostError::ResponseInvalid)?;
    if text.starts_with("OK") {
        Ok(())
    } else {
        Err(VmHostError::LaunchDenied)
    }
}

fn find_response_number(text: &str, key: &str) -> Option<u32> {
    let start = text.find(key)? + key.len();
    let bytes = text.as_bytes();
    let mut index = start;
    let mut value = 0u32;
    let mut saw_digit = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
        saw_digit = true;
        index += 1;
    }
    if saw_digit {
        Some(value)
    } else {
        None
    }
}

fn find_response_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let start = text.find(key)? + key.len();
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if end > start {
        Some(&text[start..end])
    } else {
        None
    }
}

fn u32_to_string(value: u32) -> String {
    value.to_string()
}

fn usize_to_string(value: usize) -> String {
    value.to_string()
}
