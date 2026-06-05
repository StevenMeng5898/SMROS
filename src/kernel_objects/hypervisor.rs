//! Modeled hypervisor kernel object.
//!
//! This tracks VM lifecycle, resource isolation metadata, and monitoring state
//! for the shell-facing `vm` command. It uses the existing Zircon
//! guest/VCPU syscall compatibility hooks for handles, but does not implement
//! hardware virtualization execution.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use super::{HandleValue, ObjectType, ZxError, ZxResult};
use alloc::string::String;
use alloc::vec::Vec;

include!("hypervisor_logic_shared.rs");

const MAX_HYPERVISOR_VMS: usize = 16;
const DEFAULT_CPU_TIME_SLICE_US: u32 = 1_000;
const DEFAULT_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_RESTART_LIMIT: u32 = 3;
const DEFAULT_MONITOR_LATENCY_US: u32 = 99;
const VM_NAME_MAX: usize = 32;
const HYPERVISOR_HANDLE_VALUE: u32 = 0x7000_0001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmState {
    Running,
    Stopped,
    Crashed,
}

impl VmState {
    pub fn as_str(self) -> &'static str {
        match self {
            VmState::Running => "running",
            VmState::Stopped => "stopped",
            VmState::Crashed => "crashed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct VmConfig {
    pub name: String,
    pub cpu_time_slice_us: u32,
    pub realtime_priority: u8,
    pub memory_bytes: usize,
    pub restart_on_crash: bool,
    pub restart_limit: u32,
}

impl VmConfig {
    fn default_with_name(name: String) -> Self {
        Self {
            name,
            cpu_time_slice_us: DEFAULT_CPU_TIME_SLICE_US,
            realtime_priority: 0,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            restart_on_crash: false,
            restart_limit: DEFAULT_RESTART_LIMIT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VmRecord {
    pub id: u32,
    pub name: String,
    pub config_path: String,
    pub cpu_time_slice_us: u32,
    pub realtime_priority: u8,
    pub memory_bytes: usize,
    pub restart_on_crash: bool,
    pub restart_limit: u32,
    pub restart_count: u32,
    pub state: VmState,
    pub guest_handle: u32,
    pub vmar_handle: u32,
    pub vcpu_handle: u32,
    pub process_pid: usize,
    pub start_tick: u64,
    pub last_event_tick: u64,
    pub forced_kill: bool,
    pub monitor_latency_us: u32,
}

impl VmRecord {
    fn from_config(
        id: u32,
        config_path: String,
        config: VmConfig,
        guest_handle: u32,
        vmar_handle: u32,
        vcpu_handle: u32,
        process_pid: usize,
        tick: u64,
    ) -> Self {
        Self {
            id,
            name: config.name,
            config_path,
            cpu_time_slice_us: config.cpu_time_slice_us,
            realtime_priority: config.realtime_priority,
            memory_bytes: config.memory_bytes,
            restart_on_crash: config.restart_on_crash,
            restart_limit: config.restart_limit,
            restart_count: 0,
            state: VmState::Running,
            guest_handle,
            vmar_handle,
            vcpu_handle,
            process_pid,
            start_tick: tick,
            last_event_tick: tick,
            forced_kill: false,
            monitor_latency_us: DEFAULT_MONITOR_LATENCY_US,
        }
    }

    pub fn uptime_ticks(&self, now_tick: u64) -> u64 {
        smros_hypervisor_uptime_ticks_body!(
            self.state,
            VmState::Running,
            self.start_tick,
            self.last_event_tick,
            now_tick
        )
    }

    pub fn cpu_usage_percent(&self, now_tick: u64) -> u32 {
        let uptime = self.uptime_ticks(now_tick);
        smros_hypervisor_cpu_usage_percent_body!(
            uptime,
            self.cpu_time_slice_us,
            self.realtime_priority as u32
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HypervisorStats {
    pub vm_count: usize,
    pub running_vms: usize,
    pub stopped_vms: usize,
    pub crashed_vms: usize,
    pub total_memory_bytes: usize,
    pub total_cpu_time_slice_us: u32,
    pub monitor_latency_us: u32,
    pub fault_domains: usize,
    pub forced_kills: u32,
    pub auto_restarts: u32,
}

#[derive(Clone, Debug)]
pub struct HypervisorStatus {
    pub stats: HypervisorStats,
    pub vms: Vec<VmRecord>,
    pub tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HypervisorConfigError {
    Empty,
    InvalidName,
    InvalidCpu,
    InvalidMemory,
    InvalidPriority,
    InvalidRestart,
}

impl HypervisorConfigError {
    pub fn as_str(self) -> &'static str {
        match self {
            HypervisorConfigError::Empty => "empty config",
            HypervisorConfigError::InvalidName => "invalid name",
            HypervisorConfigError::InvalidCpu => "invalid cpu",
            HypervisorConfigError::InvalidMemory => "invalid memory",
            HypervisorConfigError::InvalidPriority => "invalid priority",
            HypervisorConfigError::InvalidRestart => "invalid restart policy",
        }
    }
}

pub struct HypervisorObject {
    handle: HandleValue,
    vms: Vec<VmRecord>,
    next_vm_id: u32,
    forced_kills: u32,
    auto_restarts: u32,
}

impl HypervisorObject {
    pub const fn new() -> Self {
        Self {
            handle: HandleValue(HYPERVISOR_HANDLE_VALUE),
            vms: Vec::new(),
            next_vm_id: 1,
            forced_kills: 0,
            auto_restarts: 0,
        }
    }

    pub fn handle(&self) -> HandleValue {
        self.handle
    }

    pub fn contains_handle(&self, handle: HandleValue) -> bool {
        self.handle == handle
    }

    pub fn start_vm(
        &mut self,
        config_path: &str,
        config_xml: &str,
        tick: u64,
    ) -> Result<VmRecord, HypervisorConfigError> {
        let config = parse_vm_config(config_xml)?;
        if let Some(existing_index) = self
            .vms
            .iter()
            .position(|vm| vm.name.as_str() == config.name.as_str())
        {
            let previous = self.vms[existing_index].clone();
            close_vm_resources(&previous);

            let process_pid = create_vm_process(config.name.as_str())
                .ok_or(HypervisorConfigError::InvalidRestart)?;
            let (guest_handle, vmar_handle, vcpu_handle) = match create_guest_vcpu_handles() {
                Ok(handles) => handles,
                Err(err) => {
                    let _ = crate::kernel_lowlevel::memory::process_manager()
                        .terminate_process(process_pid);
                    return Err(err);
                }
            };

            let existing = &mut self.vms[existing_index];
            existing.state = VmState::Running;
            existing.config_path = String::from(config_path);
            existing.cpu_time_slice_us = config.cpu_time_slice_us;
            existing.realtime_priority = config.realtime_priority;
            existing.memory_bytes = config.memory_bytes;
            existing.restart_on_crash = config.restart_on_crash;
            existing.restart_limit = config.restart_limit;
            existing.restart_count = 0;
            existing.guest_handle = guest_handle;
            existing.vmar_handle = vmar_handle;
            existing.vcpu_handle = vcpu_handle;
            existing.process_pid = process_pid;
            existing.start_tick = tick;
            existing.last_event_tick = tick;
            existing.forced_kill = false;
            return Ok(existing.clone());
        }

        if self.vms.len() >= MAX_HYPERVISOR_VMS {
            return Err(HypervisorConfigError::InvalidRestart);
        }

        let process_pid =
            create_vm_process(config.name.as_str()).ok_or(HypervisorConfigError::InvalidRestart)?;
        let (guest_handle, vmar_handle, vcpu_handle) = match create_guest_vcpu_handles() {
            Ok(handles) => handles,
            Err(err) => {
                let _ = crate::kernel_lowlevel::memory::process_manager()
                    .terminate_process(process_pid);
                return Err(err);
            }
        };

        let record = VmRecord::from_config(
            self.next_vm_id,
            String::from(config_path),
            config,
            guest_handle,
            vmar_handle,
            vcpu_handle,
            process_pid,
            tick,
        );
        self.next_vm_id = self.next_vm_id.saturating_add(1);
        self.vms.push(record.clone());
        Ok(record)
    }

    pub fn kill_vm(&mut self, name: &str, tick: u64) -> ZxResult<VmRecord> {
        let Some(record) = self.vm_by_name_mut(name) else {
            return Err(ZxError::ErrNotFound);
        };

        let (state, last_event_tick, forced_kill) =
            smros_hypervisor_kill_transition_body!(tick, VmState::Stopped);
        record.state = state;
        record.last_event_tick = last_event_tick;
        record.forced_kill = forced_kill;
        let out = record.clone();
        self.forced_kills = smros_hypervisor_saturating_inc_u32_body!(self.forced_kills);

        close_vm_resources(&out);
        Ok(out)
    }

    pub fn record_crash(&mut self, name: &str, tick: u64) -> ZxResult<VmRecord> {
        let Some(record) = self.vm_by_name_mut(name) else {
            return Err(ZxError::ErrNotFound);
        };

        let (state, restart_count, start_tick, restarted) =
            smros_hypervisor_crash_transition_body!(
                record.restart_on_crash,
                record.restart_count,
                record.restart_limit,
                record.start_tick,
                tick,
                VmState::Running,
                VmState::Crashed
            );
        record.state = state;
        record.restart_count = restart_count;
        record.start_tick = start_tick;
        record.last_event_tick = tick;
        let out = record.clone();
        if restarted {
            self.auto_restarts = smros_hypervisor_saturating_inc_u32_body!(self.auto_restarts);
        }
        Ok(out)
    }

    pub fn status(&self, tick: u64) -> HypervisorStatus {
        let mut running = 0usize;
        let mut stopped = 0usize;
        let mut crashed = 0usize;
        let mut total_memory_bytes = 0usize;
        let mut total_cpu_time_slice_us = 0u32;

        for vm in &self.vms {
            let (running_delta, stopped_delta, crashed_delta) =
                smros_hypervisor_state_count_delta_body!(
                    vm.state,
                    VmState::Running,
                    VmState::Stopped,
                    VmState::Crashed
                );
            running += running_delta;
            stopped += stopped_delta;
            crashed += crashed_delta;
            total_memory_bytes = total_memory_bytes.saturating_add(vm.memory_bytes);
            total_cpu_time_slice_us = total_cpu_time_slice_us.saturating_add(vm.cpu_time_slice_us);
        }

        HypervisorStatus {
            stats: HypervisorStats {
                vm_count: self.vms.len(),
                running_vms: running,
                stopped_vms: stopped,
                crashed_vms: crashed,
                total_memory_bytes,
                total_cpu_time_slice_us,
                monitor_latency_us: DEFAULT_MONITOR_LATENCY_US,
                fault_domains: self.vms.len().saturating_add(1),
                forced_kills: self.forced_kills,
                auto_restarts: self.auto_restarts,
            },
            vms: self.vms.clone(),
            tick,
        }
    }

    fn vm_by_name_mut(&mut self, name: &str) -> Option<&mut VmRecord> {
        self.vms.iter_mut().find(|vm| vm.name.as_str() == name)
    }
}

fn create_guest_vcpu_handles() -> Result<(u32, u32, u32), HypervisorConfigError> {
    let mut guest_handle = 0u32;
    let mut vmar_handle = 0u32;
    let mut vcpu_handle = 0u32;
    if crate::syscall::sys_guest_create(0, 0, &mut guest_handle, &mut vmar_handle).is_err() {
        return Err(HypervisorConfigError::InvalidRestart);
    }
    if crate::syscall::sys_vcpu_create(guest_handle, 0, 0, &mut vcpu_handle).is_err() {
        let _ = crate::syscall::sys_handle_close(guest_handle);
        let _ = crate::syscall::sys_handle_close(vmar_handle);
        return Err(HypervisorConfigError::InvalidRestart);
    }
    Ok((guest_handle, vmar_handle, vcpu_handle))
}

fn close_vm_handles(record: &VmRecord) {
    let _ = crate::syscall::sys_handle_close(record.vcpu_handle);
    let _ = crate::syscall::sys_handle_close(record.guest_handle);
    let _ = crate::syscall::sys_handle_close(record.vmar_handle);
}

fn create_vm_process(name: &str) -> Option<usize> {
    crate::kernel_lowlevel::memory::process_manager().create_vm_process(name)
}

fn close_vm_resources(record: &VmRecord) {
    close_vm_handles(record);
    if record.process_pid != 0 {
        let _ = crate::kernel_lowlevel::memory::process_manager()
            .terminate_process(record.process_pid);
    }
}

static mut HYPERVISOR_OBJECT: HypervisorObject = HypervisorObject::new();

pub fn hypervisor() -> &'static mut HypervisorObject {
    unsafe { &mut HYPERVISOR_OBJECT }
}

pub fn init() {
    let _ = ObjectType::Hypervisor;
    crate::kobj_info!("hypervisor", "modeled hypervisor object initialized");
}

pub fn parse_vm_config(config_xml: &str) -> Result<VmConfig, HypervisorConfigError> {
    if config_xml.trim().is_empty() {
        return Err(HypervisorConfigError::Empty);
    }

    let name = tag_value(config_xml, "name")
        .or_else(|| attribute_value(config_xml, "vm", "name"))
        .unwrap_or_else(|| String::from("vm"));
    if !vm_name_valid(name.as_str()) {
        return Err(HypervisorConfigError::InvalidName);
    }

    let mut config = VmConfig::default_with_name(name);

    if let Some(cpu) = tag_value_non_empty(config_xml, "cpu") {
        config.cpu_time_slice_us =
            parse_u32_with_units(cpu.as_str()).ok_or(HypervisorConfigError::InvalidCpu)?;
    }
    if let Some(cpu) = attribute_value(config_xml, "cpu", "time_slice_us") {
        config.cpu_time_slice_us =
            parse_u32_with_units(cpu.as_str()).ok_or(HypervisorConfigError::InvalidCpu)?;
    }
    if config.cpu_time_slice_us == 0 {
        return Err(HypervisorConfigError::InvalidCpu);
    }

    if let Some(memory) = tag_value_non_empty(config_xml, "memory") {
        config.memory_bytes =
            parse_bytes_with_units(memory.as_str()).ok_or(HypervisorConfigError::InvalidMemory)?;
    }
    if let Some(memory) = attribute_value(config_xml, "memory", "bytes") {
        config.memory_bytes =
            parse_bytes_with_units(memory.as_str()).ok_or(HypervisorConfigError::InvalidMemory)?;
    }
    if config.memory_bytes == 0
        || config.memory_bytes % crate::kernel_lowlevel::memory::PAGE_SIZE != 0
    {
        return Err(HypervisorConfigError::InvalidMemory);
    }

    if let Some(priority) = tag_value_non_empty(config_xml, "priority")
        .or_else(|| tag_value_non_empty(config_xml, "realtime_priority"))
        .or_else(|| attribute_value(config_xml, "cpu", "priority"))
    {
        let parsed = parse_u32_with_units(priority.as_str())
            .ok_or(HypervisorConfigError::InvalidPriority)?;
        if parsed > 99 {
            return Err(HypervisorConfigError::InvalidPriority);
        }
        config.realtime_priority = parsed as u8;
    }

    if let Some(restart) = tag_value_non_empty(config_xml, "restart")
        .or_else(|| attribute_value(config_xml, "restart", "policy"))
        .or_else(|| attribute_value(config_xml, "restart", "on_crash"))
    {
        config.restart_on_crash =
            parse_restart_policy(restart.as_str()).ok_or(HypervisorConfigError::InvalidRestart)?;
    }
    if let Some(limit) = attribute_value(config_xml, "restart", "limit") {
        config.restart_limit =
            parse_u32_with_units(limit.as_str()).ok_or(HypervisorConfigError::InvalidRestart)?;
    }

    Ok(config)
}

fn vm_name_valid(name: &str) -> bool {
    if !smros_hypervisor_name_len_valid_body!(name.len(), VM_NAME_MAX) {
        return false;
    }
    for byte in name.bytes() {
        if !smros_hypervisor_name_byte_valid_body!(byte) {
            return false;
        }
    }
    true
}

fn tag_value(input: &str, tag: &str) -> Option<String> {
    let mut start_tag = String::from("<");
    start_tag.push_str(tag);
    let start = input.find(start_tag.as_str())?;
    let after_start = &input[start..];
    let start_close = after_start.find('>')?;
    let value_start = start + start_close + 1;

    let mut end_tag = String::from("</");
    end_tag.push_str(tag);
    end_tag.push('>');
    let end = input[value_start..].find(end_tag.as_str())? + value_start;
    Some(String::from(input[value_start..end].trim()))
}

fn tag_value_non_empty(input: &str, tag: &str) -> Option<String> {
    let value = tag_value(input, tag)?;
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn attribute_value(input: &str, tag: &str, attr: &str) -> Option<String> {
    let mut start_tag = String::from("<");
    start_tag.push_str(tag);
    let start = input.find(start_tag.as_str())?;
    let after_start = &input[start..];
    let tag_end = after_start.find('>')?;
    let tag_text = &after_start[..tag_end];

    let mut pattern = String::from(attr);
    pattern.push('=');
    let attr_start = tag_text.find(pattern.as_str())? + pattern.len();
    let quote = tag_text.as_bytes().get(attr_start).copied()?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let value_start = attr_start + 1;
    let value_end = tag_text[value_start..].find(quote as char)? + value_start;
    Some(String::from(tag_text[value_start..value_end].trim()))
}

fn parse_u32_with_units(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    let digits_end = trimmed
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(trimmed.len());
    if digits_end == 0 {
        return None;
    }

    let mut parsed = 0u32;
    for byte in trimmed[..digits_end].bytes() {
        parsed = parsed.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
    }
    Some(parsed)
}

fn parse_bytes_with_units(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    let digits_end = trimmed
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(trimmed.len());
    if digits_end == 0 {
        return None;
    }

    let mut parsed = 0usize;
    for byte in trimmed[..digits_end].bytes() {
        parsed = parsed
            .checked_mul(10)?
            .checked_add((byte - b'0') as usize)?;
    }

    let suffix = trimmed[digits_end..].trim();
    let multiplier = if suffix.eq_ignore_ascii_case("g")
        || suffix.eq_ignore_ascii_case("gb")
        || suffix.eq_ignore_ascii_case("gib")
    {
        1024usize.checked_mul(1024)?.checked_mul(1024)?
    } else if suffix.eq_ignore_ascii_case("m")
        || suffix.eq_ignore_ascii_case("mb")
        || suffix.eq_ignore_ascii_case("mib")
    {
        1024usize.checked_mul(1024)?
    } else if suffix.eq_ignore_ascii_case("k")
        || suffix.eq_ignore_ascii_case("kb")
        || suffix.eq_ignore_ascii_case("kib")
    {
        1024usize
    } else if suffix.is_empty() || suffix.eq_ignore_ascii_case("b") {
        1usize
    } else {
        return None;
    };
    parsed.checked_mul(multiplier)
}

fn parse_restart_policy(value: &str) -> Option<bool> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("on-crash")
        || trimmed.eq_ignore_ascii_case("on_crash")
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed == "1"
        || trimmed.eq_ignore_ascii_case("yes")
    {
        Some(true)
    } else if trimmed.eq_ignore_ascii_case("never")
        || trimmed.eq_ignore_ascii_case("false")
        || trimmed == "0"
        || trimmed.eq_ignore_ascii_case("no")
    {
        Some(false)
    } else {
        None
    }
}
