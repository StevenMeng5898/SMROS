//! Ported compatibility app smoke targets.
//!
//! These are intentionally small, classic app shapes rather than full ABI
//! claims: a Linux `cat`-style file reader and a Fuchsia `/svc` client.

#![allow(dead_code)]

use crate::syscall::{self, SysError};
use crate::user_level::{fxfs, svc};

const AT_FDCWD: usize = usize::MAX - 99;
const LINUX_CAT_PATH: &str = "/data/linux-cat-port.txt";
const LINUX_CAT_PATH_CSTR: &[u8] = b"/data/linux-cat-port.txt\0";
const LINUX_CAT_PAYLOAD: &[u8] = b"SMROS Linux cat compatibility port\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatAppError {
    FxfsInit,
    FxfsWrite,
    LinuxOpen(SysError),
    LinuxRead(SysError),
    LinuxWrite(SysError),
    LinuxClose(SysError),
    LinuxReadMismatch,
    SvcInit,
    SvcConnect,
    SvcCall,
    SvcReply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinuxCatPortResult {
    pub fd: usize,
    pub bytes_read: usize,
    pub bytes_written: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuchsiaSvcPortResult {
    pub component_connections: usize,
    pub requests: usize,
    pub replies: usize,
    pub filesystem_nodes: u64,
}

pub fn run_linux_cat_port() -> Result<LinuxCatPortResult, CompatAppError> {
    if !fxfs::init() {
        return Err(CompatAppError::FxfsInit);
    }
    let _ = fxfs::create_dir("/data");
    fxfs::write_file(LINUX_CAT_PATH, LINUX_CAT_PAYLOAD).map_err(|_| CompatAppError::FxfsWrite)?;

    let fd = syscall::sys_openat(AT_FDCWD, LINUX_CAT_PATH_CSTR.as_ptr() as usize, 0, 0)
        .map_err(CompatAppError::LinuxOpen)?;
    let mut out = [0u8; 64];
    let bytes_read = match syscall::sys_read(fd, out.as_mut_ptr() as usize, out.len()) {
        Ok(read) => read,
        Err(err) => {
            let _ = syscall::sys_close(fd);
            return Err(CompatAppError::LinuxRead(err));
        }
    };
    if bytes_read != LINUX_CAT_PAYLOAD.len() || &out[..bytes_read] != LINUX_CAT_PAYLOAD {
        let _ = syscall::sys_close(fd);
        return Err(CompatAppError::LinuxReadMismatch);
    }

    let bytes_written = match syscall::sys_write(1, out.as_ptr() as usize, bytes_read) {
        Ok(written) => written,
        Err(err) => {
            let _ = syscall::sys_close(fd);
            return Err(CompatAppError::LinuxWrite(err));
        }
    };
    syscall::sys_close(fd).map_err(CompatAppError::LinuxClose)?;

    Ok(LinuxCatPortResult {
        fd,
        bytes_read,
        bytes_written,
    })
}

pub fn run_fuchsia_svc_client_port() -> Result<FuchsiaSvcPortResult, CompatAppError> {
    if !svc::init() {
        return Err(CompatAppError::SvcInit);
    }

    let component =
        svc::connect(svc::SERVICE_COMPONENT_MANAGER).map_err(|_| CompatAppError::SvcConnect)?;
    let runner = match svc::connect(svc::SERVICE_ELF_RUNNER) {
        Ok(handle) => handle,
        Err(_) => {
            let _ = svc::disconnect(component);
            return Err(CompatAppError::SvcConnect);
        }
    };
    let filesystem = match svc::connect(svc::SERVICE_FXFS) {
        Ok(handle) => handle,
        Err(_) => {
            let _ = svc::disconnect(component);
            let _ = svc::disconnect(runner);
            return Err(CompatAppError::SvcConnect);
        }
    };

    let component_reply = match svc::call(
        component,
        svc::IpcMessage::request(svc::IpcOrdinal::ComponentStart, 100, 7, 0),
    ) {
        Ok(reply) => reply,
        Err(_) => {
            close_fuchsia_port_handles(component, runner, filesystem);
            return Err(CompatAppError::SvcCall);
        }
    };
    let runner_reply = match svc::call(
        runner,
        svc::IpcMessage::request(
            svc::IpcOrdinal::RunnerLoadElf,
            101,
            component_reply.arg0,
            LINUX_CAT_PAYLOAD.len() as u64,
        ),
    ) {
        Ok(reply) => reply,
        Err(_) => {
            close_fuchsia_port_handles(component, runner, filesystem);
            return Err(CompatAppError::SvcCall);
        }
    };
    let filesystem_reply = match svc::call(
        filesystem,
        svc::IpcMessage::request(svc::IpcOrdinal::FilesystemDescribe, 102, 0, 0),
    ) {
        Ok(reply) => reply,
        Err(_) => {
            close_fuchsia_port_handles(component, runner, filesystem);
            return Err(CompatAppError::SvcCall);
        }
    };

    if component_reply.status != 0
        || component_reply.txid != 100
        || component_reply.arg0 != 7
        || runner_reply.status != 0
        || runner_reply.txid != 101
        || runner_reply.arg1 != LINUX_CAT_PAYLOAD.len() as u64
        || filesystem_reply.status != 0
        || filesystem_reply.txid != 102
        || filesystem_reply.arg0 == 0
    {
        close_fuchsia_port_handles(component, runner, filesystem);
        return Err(CompatAppError::SvcReply);
    }

    let stats = svc::stats();
    close_fuchsia_port_handles(component, runner, filesystem);
    Ok(FuchsiaSvcPortResult {
        component_connections: stats.connections,
        requests: stats.requests,
        replies: stats.replies,
        filesystem_nodes: filesystem_reply.arg0,
    })
}

pub fn smoke_test() -> bool {
    run_linux_cat_port().is_ok() && run_fuchsia_svc_client_port().is_ok()
}

fn close_fuchsia_port_handles(component: u32, runner: u32, filesystem: u32) {
    let _ = svc::disconnect(component);
    let _ = svc::disconnect(runner);
    let _ = svc::disconnect(filesystem);
}
