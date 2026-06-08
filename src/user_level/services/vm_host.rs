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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmHostError {
    NoHostConfig,
    InvalidConfig,
    RequestTooLarge,
    Connect(NetError),
    Write(NetError),
    Read(NetError),
    ResponseInvalid,
    LaunchDenied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmHostLaunch {
    pub qemu_pid: u32,
}

pub fn launch(vm: &VmRecord) -> Result<VmHostLaunch, VmHostError> {
    let host = vm.host.as_ref().ok_or(VmHostError::NoHostConfig)?;
    let request = build_launch_request(vm, host)?;
    let mut socket = net::tcp_connect(NetworkSocketAddr {
        ip: net::QEMU_USER_GATEWAY,
        port: host.launcher_port,
    })
    .map_err(VmHostError::Connect)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;

    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = socket.read(&mut response).map_err(VmHostError::Read)?;
    let _ = socket.close();
    parse_launch_response(&response[..bytes])
}

pub fn stop(vm: &VmRecord) -> Result<(), VmHostError> {
    let Some(host) = vm.host.as_ref() else {
        return Ok(());
    };
    let request = build_stop_request(vm, host)?;
    let mut socket = net::tcp_connect(NetworkSocketAddr {
        ip: net::QEMU_USER_GATEWAY,
        port: host.launcher_port,
    })
    .map_err(VmHostError::Connect)?;
    socket
        .write(request.as_bytes())
        .map_err(VmHostError::Write)?;

    let mut response = [0u8; MAX_RESPONSE_BYTES];
    let bytes = socket.read(&mut response).map_err(VmHostError::Read)?;
    let _ = socket.close();
    parse_stop_response(&response[..bytes])
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

fn build_stop_request(vm: &VmRecord, host: &VmHostConfig) -> Result<String, VmHostError> {
    let mut request = String::from("SMROS_VM_STOP 1\n");
    push_kv(&mut request, "name", vm.name.as_str())?;
    push_kv(&mut request, "pid", u32_to_string(vm.host_qemu_pid).as_str())?;
    push_kv(&mut request, "port", u32_to_string(host.launcher_port as u32).as_str())?;
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
    Ok(VmHostLaunch { qemu_pid: pid })
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
        value = value
            .checked_mul(10)?
            .checked_add((byte - b'0') as u32)?;
        saw_digit = true;
        index += 1;
    }
    if saw_digit {
        Some(value)
    } else {
        None
    }
}

fn u32_to_string(value: u32) -> String {
    value.to_string()
}
