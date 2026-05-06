//! Minimal Fuchsia-style service directory and fixed-message IPC protocol.
//!
//! This is deliberately smaller than FIDL. Services are named entries under
//! `/svc`; connecting creates a Zircon channel pair, and requests use a fixed
//! little-endian struct layout.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::vec::Vec;

use crate::kernel_objects::channel;
use crate::kernel_objects::{HandleValue, ZxError, ZxResult};
use crate::user_level::{fxfs, user_logic};

const MAX_SERVICES: usize = 8;
const MAX_CONNECTIONS: usize = 16;
const IPC_MAGIC: u32 = 0x534d_4950;
const IPC_VERSION: u16 = 1;
const IPC_MESSAGE_SIZE: usize = 32;
const SVC_RIGHT_CONNECT: u32 = 1 << 0;
const SVC_RIGHT_ENUMERATE: u32 = 1 << 1;
const SVC_ALLOWED_RIGHTS: u32 = SVC_RIGHT_CONNECT | SVC_RIGHT_ENUMERATE;

pub const SERVICE_COMPONENT_MANAGER: &str = "fuchsia.component.Manager";
pub const SERVICE_ELF_RUNNER: &str = "fuchsia.component.runner.Elf";
pub const SERVICE_FXFS: &str = "fuchsia.fxfs.Service";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    ComponentManager,
    Runner,
    Filesystem,
}

impl ServiceKind {
    fn id(self) -> u16 {
        match self {
            ServiceKind::ComponentManager => user_logic::USER_SVC_COMPONENT_MANAGER,
            ServiceKind::Runner => user_logic::USER_SVC_RUNNER,
            ServiceKind::Filesystem => user_logic::USER_SVC_FILESYSTEM,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ServiceKind::ComponentManager => "component-manager",
            ServiceKind::Runner => "runner",
            ServiceKind::Filesystem => "filesystem",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcOrdinal {
    ComponentStart = 1,
    RunnerLoadElf = 2,
    FilesystemDescribe = 3,
}

impl IpcOrdinal {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(IpcOrdinal::ComponentStart),
            2 => Some(IpcOrdinal::RunnerLoadElf),
            3 => Some(IpcOrdinal::FilesystemDescribe),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcMessage {
    pub magic: u32,
    pub version: u16,
    pub ordinal: IpcOrdinal,
    pub txid: u32,
    pub arg0: u64,
    pub arg1: u64,
    pub status: i32,
}

impl IpcMessage {
    pub fn request(ordinal: IpcOrdinal, txid: u32, arg0: u64, arg1: u64) -> Self {
        Self {
            magic: IPC_MAGIC,
            version: IPC_VERSION,
            ordinal,
            txid,
            arg0,
            arg1,
            status: 0,
        }
    }

    pub fn reply(request: Self, status: i32, arg0: u64, arg1: u64) -> Self {
        Self {
            magic: IPC_MAGIC,
            version: IPC_VERSION,
            ordinal: request.ordinal,
            txid: request.txid,
            arg0,
            arg1,
            status,
        }
    }

    pub fn encode(self) -> [u8; IPC_MESSAGE_SIZE] {
        let mut out = [0u8; IPC_MESSAGE_SIZE];
        out[0..4].copy_from_slice(&self.magic.to_le_bytes());
        out[4..6].copy_from_slice(&self.version.to_le_bytes());
        out[6..8].copy_from_slice(&(self.ordinal as u16).to_le_bytes());
        out[8..12].copy_from_slice(&self.txid.to_le_bytes());
        out[12..20].copy_from_slice(&self.arg0.to_le_bytes());
        out[20..28].copy_from_slice(&self.arg1.to_le_bytes());
        out[28..32].copy_from_slice(&self.status.to_le_bytes());
        out
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if !user_logic::svc_ipc_message_size_valid(data.len()) {
            return None;
        }
        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let version = u16::from_le_bytes([data[4], data[5]]);
        let ordinal_raw = u16::from_le_bytes([data[6], data[7]]);
        let ordinal = IpcOrdinal::from_u16(ordinal_raw)?;
        let txid = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let arg0 = u64::from_le_bytes([
            data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
        ]);
        let arg1 = u64::from_le_bytes([
            data[20], data[21], data[22], data[23], data[24], data[25], data[26], data[27],
        ]);
        let status = i32::from_le_bytes([data[28], data[29], data[30], data[31]]);
        if !user_logic::svc_ipc_header_valid(magic, version) {
            return None;
        }
        Some(Self {
            magic,
            version,
            ordinal,
            txid,
            arg0,
            arg1,
            status,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ServiceEntry {
    pub name: &'static str,
    pub kind: ServiceKind,
    pub rights: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ServiceConnection {
    pub id: usize,
    pub service: ServiceKind,
    pub client: u32,
    pub server: u32,
    pub requests: usize,
    pub replies: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ServiceStats {
    pub services: usize,
    pub connections: usize,
    pub requests: usize,
    pub replies: usize,
    pub last_status: i32,
}

pub struct ServiceDirectory {
    mounted: bool,
    next_connection_id: usize,
    services: Vec<ServiceEntry>,
    connections: Vec<ServiceConnection>,
    total_requests: usize,
    total_replies: usize,
    last_status: i32,
}

impl ServiceDirectory {
    fn new() -> Self {
        Self {
            mounted: false,
            next_connection_id: 1,
            services: Vec::new(),
            connections: Vec::new(),
            total_requests: 0,
            total_replies: 0,
            last_status: 0,
        }
    }

    fn init(&mut self) -> bool {
        if self.mounted {
            return true;
        }
        self.mounted = true;
        self.next_connection_id = 1;
        self.services.clear();
        self.connections.clear();
        self.total_requests = 0;
        self.total_replies = 0;
        self.last_status = 0;

        let _ = fxfs::create_dir("/svc");
        self.add_service(SERVICE_COMPONENT_MANAGER, ServiceKind::ComponentManager)
            && self.add_service(SERVICE_ELF_RUNNER, ServiceKind::Runner)
            && self.add_service(SERVICE_FXFS, ServiceKind::Filesystem)
    }

    fn add_service(&mut self, name: &'static str, kind: ServiceKind) -> bool {
        if self.services.len() >= MAX_SERVICES
            || !user_logic::svc_name_valid(name.len())
            || self.services.iter().any(|service| service.name == name)
        {
            return false;
        }
        let rights = SVC_RIGHT_CONNECT | SVC_RIGHT_ENUMERATE;
        if !user_logic::svc_rights_valid(rights) {
            return false;
        }
        self.services.push(ServiceEntry { name, kind, rights });
        let path = service_path(name);
        fxfs::write_file(path, kind.as_str().as_bytes()).is_ok()
    }

    fn service_by_name(&self, name: &str) -> Option<ServiceEntry> {
        self.services
            .iter()
            .find(|service| service.name == name)
            .copied()
    }

    fn connect(&mut self, name: &str) -> ZxResult<u32> {
        let service = self.service_by_name(name).ok_or(ZxError::ErrNotFound)?;
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(ZxError::ErrNoMemory);
        }
        let (client, server) = channel::channel_table()
            .create_channel()
            .ok_or(ZxError::ErrNoMemory)?;
        self.connections.push(ServiceConnection {
            id: self.next_connection_id,
            service: service.kind,
            client: client.0,
            server: server.0,
            requests: 0,
            replies: 0,
        });
        self.next_connection_id = self.next_connection_id.saturating_add(1);
        Ok(client.0)
    }

    fn disconnect(&mut self, client: u32) -> ZxResult {
        let index = self
            .connections
            .iter()
            .position(|connection| connection.client == client)
            .ok_or(ZxError::ErrNotFound)?;
        let connection = self.connections.swap_remove(index);
        let _ = channel::channel_table().remove_channel(HandleValue(connection.client));
        Ok(())
    }

    fn call(&mut self, client: u32, request: IpcMessage) -> ZxResult<IpcMessage> {
        let index = self
            .connections
            .iter()
            .position(|connection| connection.client == client)
            .ok_or(ZxError::ErrNotFound)?;
        let service = self.connections[index].service;
        if !user_logic::svc_protocol_allowed(service.id(), request.ordinal as u16) {
            self.last_status = ZxError::ErrNotSupported as i32;
            return Err(ZxError::ErrNotSupported);
        }

        let server = self.connections[index].server;
        let encoded = request.encode();
        channel::channel_table().write_message(HandleValue(client), &encoded)?;
        self.connections[index].requests = self.connections[index].requests.saturating_add(1);
        self.total_requests = self.total_requests.saturating_add(1);

        let mut bytes = Vec::new();
        channel::channel_table().read_message(HandleValue(server), &mut bytes)?;
        let decoded = IpcMessage::decode(&bytes).ok_or(ZxError::ErrInvalidArgs)?;
        let reply = dispatch_service(service, decoded);
        let reply_bytes = reply.encode();
        channel::channel_table().write_message(HandleValue(server), &reply_bytes)?;
        let mut reply_readback = Vec::new();
        channel::channel_table().read_message(HandleValue(client), &mut reply_readback)?;
        let decoded_reply = IpcMessage::decode(&reply_readback).ok_or(ZxError::ErrInvalidArgs)?;
        self.connections[index].replies = self.connections[index].replies.saturating_add(1);
        self.total_replies = self.total_replies.saturating_add(1);
        self.last_status = decoded_reply.status;
        Ok(decoded_reply)
    }

    fn stats(&self) -> ServiceStats {
        ServiceStats {
            services: self.services.len(),
            connections: self.connections.len(),
            requests: self.total_requests,
            replies: self.total_replies,
            last_status: self.last_status,
        }
    }
}

static mut SERVICE_DIRECTORY: Option<ServiceDirectory> = None;

fn directory() -> &'static mut ServiceDirectory {
    unsafe {
        if SERVICE_DIRECTORY.is_none() {
            SERVICE_DIRECTORY = Some(ServiceDirectory::new());
        }
        SERVICE_DIRECTORY.as_mut().unwrap()
    }
}

fn service_path(name: &str) -> &'static str {
    match name {
        SERVICE_COMPONENT_MANAGER => "/svc/fuchsia.component.Manager",
        SERVICE_ELF_RUNNER => "/svc/fuchsia.component.runner.Elf",
        SERVICE_FXFS => "/svc/fuchsia.fxfs.Service",
        _ => "/svc/unknown",
    }
}

fn dispatch_service(service: ServiceKind, request: IpcMessage) -> IpcMessage {
    match (service, request.ordinal) {
        (ServiceKind::ComponentManager, IpcOrdinal::ComponentStart) => {
            IpcMessage::reply(request, 0, request.arg0, SERVICE_ELF_RUNNER.len() as u64)
        }
        (ServiceKind::Runner, IpcOrdinal::RunnerLoadElf) => {
            IpcMessage::reply(request, 0, request.arg0, request.arg1)
        }
        (ServiceKind::Filesystem, IpcOrdinal::FilesystemDescribe) => {
            let stats = fxfs::stats();
            IpcMessage::reply(request, 0, stats.nodes as u64, stats.dir_entries as u64)
        }
        _ => IpcMessage::reply(request, ZxError::ErrNotSupported as i32, 0, 0),
    }
}

pub fn init() -> bool {
    directory().init()
}

pub fn connect(name: &str) -> ZxResult<u32> {
    directory().connect(name)
}

pub fn disconnect(client: u32) -> ZxResult {
    directory().disconnect(client)
}

pub fn call(client: u32, request: IpcMessage) -> ZxResult<IpcMessage> {
    directory().call(client, request)
}

pub fn stats() -> ServiceStats {
    directory().stats()
}

pub fn services() -> Vec<ServiceEntry> {
    directory().services.clone()
}

pub fn smoke_test() -> bool {
    if !init() {
        return false;
    }

    let component = match connect(SERVICE_COMPONENT_MANAGER) {
        Ok(handle) => handle,
        Err(_) => return false,
    };
    let runner = match connect(SERVICE_ELF_RUNNER) {
        Ok(handle) => handle,
        Err(_) => return false,
    };
    let filesystem = match connect(SERVICE_FXFS) {
        Ok(handle) => handle,
        Err(_) => return false,
    };

    let component_reply = match call(
        component,
        IpcMessage::request(IpcOrdinal::ComponentStart, 1, 2, 0),
    ) {
        Ok(reply) => reply,
        Err(_) => return false,
    };
    let runner_reply = match call(
        runner,
        IpcMessage::request(IpcOrdinal::RunnerLoadElf, 2, 2, 1),
    ) {
        Ok(reply) => reply,
        Err(_) => return false,
    };
    let fs_reply = match call(
        filesystem,
        IpcMessage::request(IpcOrdinal::FilesystemDescribe, 3, 0, 0),
    ) {
        Ok(reply) => reply,
        Err(_) => return false,
    };

    let ok = component_reply.status == 0
        && component_reply.arg0 == 2
        && runner_reply.status == 0
        && runner_reply.arg1 == 1
        && fs_reply.status == 0
        && fs_reply.arg0 >= 1
        && stats().requests >= 3
        && stats().replies >= 3;
    let _ = disconnect(component);
    let _ = disconnect(runner);
    let _ = disconnect(filesystem);
    ok
}
