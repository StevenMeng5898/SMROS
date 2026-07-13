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
    St,
}

impl HermesHostTestJob {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ut => "ut",
            Self::It => "it",
            Self::St => "st",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesHostTestResult {
    pub job: HermesHostTestJob,
    pub passed: bool,
    pub summary: String,
}

pub fn run_hermes_test(job: HermesHostTestJob) -> Result<HermesHostTestResult, VmHostError> {
    let request = build_hermes_test_request(job);
    let mut socket = net::tcp_connect(NetworkSocketAddr {
        ip: net::QEMU_USER_GATEWAY,
        port: DEFAULT_LAUNCHER_PORT,
    })
    .map_err(map_connect_error)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;
    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = read_response(&mut socket, &mut response, RESPONSE_READ_ATTEMPTS)?;
    let _ = socket.close();
    parse_hermes_test_response(job, &response[..bytes])
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

pub fn launch(vm: &VmRecord) -> Result<VmHostLaunch, VmHostError> {
    let host = vm.host.as_ref().ok_or(VmHostError::NoHostConfig)?;
    let request = build_launch_request(vm, host)?;
    let mut socket = net::tcp_connect(NetworkSocketAddr {
        ip: net::QEMU_USER_GATEWAY,
        port: host.launcher_port,
    })
    .map_err(map_connect_error)?;
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
    let mut socket = net::tcp_connect(NetworkSocketAddr {
        ip: net::QEMU_USER_GATEWAY,
        port: host.launcher_port,
    })
    .map_err(map_connect_error)?;
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
    let mut socket = net::tcp_connect(NetworkSocketAddr {
        ip: net::QEMU_USER_GATEWAY,
        port: DEFAULT_LAUNCHER_PORT,
    })
    .map_err(map_connect_error)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;

    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = read_response(&mut socket, &mut response, RESPONSE_READ_ATTEMPTS)?;
    let _ = socket.close();
    parse_stop_response(&response[..bytes])
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
