//! Perfetto-compatible trace export for SMROS.
//!
//! This is a small SMROS-native bridge, not the upstream Perfetto daemon stack.
//! It emits native Perfetto protobuf trace files so the browser Perfetto UI can
//! open scheduler timelines directly.

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel_lowlevel::{smp, thread::ThreadId};
use crate::kernel_objects::{
    hypervisor::{self, VmRecord},
    scheduler::{self, SchedulePolicy, SchedulerTraceEntry, SCHED_TRACE_CAPACITY},
};
use crate::user_level::{fxfs, vm_host};

pub const PERFETTO_TRACE_PATH: &str = "/shared/trace.pftrace";
pub const PERFETTO_COMPAT_FORMAT: &str = "perfetto-protobuf-trace";
pub const PERFETTO_TICK_US: u64 = 10_000;
pub const PERFETTO_POLICY_COMPARE_DEFAULT_STEPS: usize = 16;
pub const PERFETTO_POLICY_COMPARE_MAX_STEPS: usize = 64;

const PERFETTO_LEGACY_SCHED_PFTRACE_PATH: &str = "/data/perfetto/sched-trace.pftrace";
const PERFETTO_LEGACY_SCHED_TRACE_PATH: &str = "/data/perfetto/sched-trace.json";
const PERFETTO_LEGACY_SHARED_TRACE_PATH: &str = "/shared/trace.json";
const PERFETTO_NS_PER_US: u64 = 1_000;
const SMROS_PROCESS_TRACK_UUID: u64 = 0x534d_524f_5300_0001;
const SMROS_CPU_TRACK_UUID_BASE: u64 = 0x534d_524f_5301_0000;
const SMROS_VM_TRACK_UUID_BASE: u64 = 0x534d_524f_5302_0000;
const SMROS_VM_ROOT_TRACK_UUID: u64 = 0x534d_524f_5302_ffff;
const SMROS_POLICY_TRACK_UUID_BASE: u64 = 0x534d_524f_5303_0000;
const SMROS_POLICY_TRACK_UUID_STRIDE: u64 = 0x100;
const PERFETTO_WIRE_VARINT: u8 = 0;
const PERFETTO_WIRE_LENGTH_DELIMITED: u8 = 2;
const PERFETTO_TRACE_PACKET_FIELD: u32 = 1;
const PERFETTO_PACKET_TIMESTAMP_FIELD: u32 = 8;
const PERFETTO_PACKET_TRUSTED_SEQUENCE_ID_FIELD: u32 = 10;
const PERFETTO_PACKET_TRACK_EVENT_FIELD: u32 = 11;
const PERFETTO_PACKET_SEQUENCE_FLAGS_FIELD: u32 = 13;
const PERFETTO_PACKET_TRACK_DESCRIPTOR_FIELD: u32 = 60;
const PERFETTO_PACKET_FIRST_ON_SEQUENCE_FIELD: u32 = 87;
const PERFETTO_TRACK_UUID_FIELD: u32 = 1;
const PERFETTO_TRACK_NAME_FIELD: u32 = 2;
const PERFETTO_TRACK_PROCESS_FIELD: u32 = 3;
const PERFETTO_TRACK_THREAD_FIELD: u32 = 4;
const PERFETTO_TRACK_PARENT_UUID_FIELD: u32 = 5;
const PERFETTO_PROCESS_PID_FIELD: u32 = 1;
const PERFETTO_PROCESS_NAME_FIELD: u32 = 6;
const PERFETTO_THREAD_PID_FIELD: u32 = 1;
const PERFETTO_THREAD_TID_FIELD: u32 = 2;
const PERFETTO_THREAD_NAME_FIELD: u32 = 5;
const PERFETTO_EVENT_DEBUG_ANNOTATIONS_FIELD: u32 = 4;
const PERFETTO_EVENT_TYPE_FIELD: u32 = 9;
const PERFETTO_EVENT_TRACK_UUID_FIELD: u32 = 11;
const PERFETTO_EVENT_CATEGORIES_FIELD: u32 = 22;
const PERFETTO_EVENT_NAME_FIELD: u32 = 23;
const PERFETTO_EVENT_CORRELATION_ID_FIELD: u32 = 52;
const PERFETTO_EVENT_TYPE_SLICE_BEGIN: u64 = 1;
const PERFETTO_EVENT_TYPE_SLICE_END: u64 = 2;
const PERFETTO_DEBUG_UINT_VALUE_FIELD: u32 = 3;
const PERFETTO_DEBUG_STRING_VALUE_FIELD: u32 = 6;
const PERFETTO_DEBUG_NAME_FIELD: u32 = 10;
const SMROS_PROCESS_PID: u64 = 1;
const SMROS_SCHED_TID_BASE: u64 = 10_000;
const SMROS_VM_TID_BASE: u64 = 20_000;
const SMROS_TRUSTED_PACKET_SEQUENCE_ID: u64 = 8_008;
const PERFETTO_SEQ_INCREMENTAL_STATE_CLEARED: u64 = 1;
const PERFETTO_POLICY_COUNT: usize = 4;
const PERFETTO_POLICY_TASK_COUNT: usize = 8;
const SMROS_TASK_CORRELATION_ID_BASE: u64 = 1_000;
const PERFETTO_TASK_COLORS: &[(&str, u32); 12] = &[
    ("#1f77b4", 0x1f77b4),
    ("#ff7f0e", 0xff7f0e),
    ("#2ca02c", 0x2ca02c),
    ("#d62728", 0xd62728),
    ("#9467bd", 0x9467bd),
    ("#8c564b", 0x8c564b),
    ("#e377c2", 0xe377c2),
    ("#7f7f7f", 0x7f7f7f),
    ("#bcbd22", 0xbcbd22),
    ("#17becf", 0x17becf),
    ("#aec7e8", 0xaec7e8),
    ("#ffbb78", 0xffbb78),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfettoError {
    FxfsInit,
    FxfsPrepare,
    NoSamples,
    Encode,
}

impl PerfettoError {
    pub const fn as_str(self) -> &'static str {
        match self {
            PerfettoError::FxfsInit => "fxfs init",
            PerfettoError::FxfsPrepare => "fxfs prepare",
            PerfettoError::NoSamples => "no samples",
            PerfettoError::Encode => "encode",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerfettoSchedulerTraceExport {
    pub path: &'static str,
    pub format: &'static str,
    pub policy: &'static str,
    pub bytes: usize,
    pub samples: usize,
    pub slices: usize,
    pub cpu_tracks: usize,
    pub thread_count: usize,
    pub vm_tracks: usize,
    pub host_synced: bool,
    pub start_tick: u64,
    pub end_tick: u64,
    pub tick_us: u64,
}

pub fn export_scheduler_trace(
    samples: usize,
) -> Result<PerfettoSchedulerTraceExport, PerfettoError> {
    prepare_storage()?;

    let scheduler = scheduler::scheduler();
    scheduler.record_sample_worker_snapshot();
    let trace_len = scheduler.trace_len();
    if trace_len == 0 {
        return Err(PerfettoError::NoSamples);
    }

    let requested_samples = samples.clamp(1, SCHED_TRACE_CAPACITY).min(trace_len);
    let start = trace_len.saturating_sub(requested_samples);
    let filter_sample_workers = trace_window_has_sample_worker(scheduler, start, requested_samples);
    let mut entries = [SchedulerTraceEntry::empty(); SCHED_TRACE_CAPACITY];
    let mut entry_count = 0usize;
    let mut cpu_rows = [usize::MAX; scheduler::MAX_CPUS];
    let mut cpu_count = 0usize;
    let mut threads = [usize::MAX; 32];
    let mut thread_count = 0usize;

    for index in 0..requested_samples {
        if let Some(entry) = scheduler.trace_entry(start + index) {
            if !trace_entry_should_export(scheduler, entry, filter_sample_workers) {
                continue;
            }
            entries[entry_count] = entry;
            if !contains_usize(&cpu_rows[..cpu_count], entry.cpu_id) && cpu_count < cpu_rows.len() {
                cpu_rows[cpu_count] = entry.cpu_id;
                cpu_count += 1;
            }
            if !contains_usize(&threads[..thread_count], entry.thread_id)
                && thread_count < threads.len()
            {
                threads[thread_count] = entry.thread_id;
                thread_count += 1;
            }
            entry_count += 1;
        }
    }
    if entry_count == 0 {
        return Err(PerfettoError::NoSamples);
    }
    let (start_tick, end_tick) = trace_tick_bounds(&entries[..entry_count]);

    sort_usize_prefix(&mut cpu_rows, cpu_count);
    sort_usize_prefix(&mut threads, thread_count);

    let policy = scheduler.policy().as_str();
    let tick = scheduler.get_tick_count();
    let vm_status = hypervisor::hypervisor().status(tick);
    let trace = encode_scheduler_trace_pftrace(
        &entries[..entry_count],
        &cpu_rows[..cpu_count],
        &threads[..thread_count],
        &vm_status.vms,
        policy,
        start_tick,
        end_tick,
    )?;
    write_trace_outputs(trace.as_slice())?;
    let host_synced = vm_host::sync_trace(PERFETTO_TRACE_PATH, trace.as_slice()).is_ok();

    Ok(PerfettoSchedulerTraceExport {
        path: PERFETTO_TRACE_PATH,
        format: PERFETTO_COMPAT_FORMAT,
        policy,
        bytes: trace.len(),
        samples: entry_count,
        slices: entry_count.saturating_add(vm_status.vms.len()),
        cpu_tracks: cpu_count,
        thread_count,
        vm_tracks: vm_status.vms.len(),
        host_synced,
        start_tick,
        end_tick,
        tick_us: PERFETTO_TICK_US,
    })
}

pub fn export_scheduler_policy_comparison(
    steps: usize,
) -> Result<PerfettoSchedulerTraceExport, PerfettoError> {
    prepare_storage()?;

    let steps = steps.clamp(1, PERFETTO_POLICY_COMPARE_MAX_STEPS);
    let cpu_count = logical_cpu_count();
    let trace = encode_policy_comparison_pftrace(cpu_count, steps)?;
    write_trace_outputs(trace.as_slice())?;
    let host_synced = vm_host::sync_trace(PERFETTO_TRACE_PATH, trace.as_slice()).is_ok();

    Ok(PerfettoSchedulerTraceExport {
        path: PERFETTO_TRACE_PATH,
        format: PERFETTO_COMPAT_FORMAT,
        policy: "policy-compare",
        bytes: trace.len(),
        samples: steps,
        slices: steps
            .saturating_mul(cpu_count)
            .saturating_mul(PERFETTO_POLICY_COUNT),
        cpu_tracks: cpu_count.saturating_mul(PERFETTO_POLICY_COUNT),
        thread_count: PERFETTO_POLICY_TASK_COUNT,
        vm_tracks: 0,
        host_synced,
        start_tick: 0,
        end_tick: steps as u64,
        tick_us: PERFETTO_TICK_US,
    })
}

fn prepare_storage() -> Result<(), PerfettoError> {
    if !fxfs::init() {
        return Err(PerfettoError::FxfsInit);
    }
    fxfs::ensure_host_share().map_err(|_| PerfettoError::FxfsPrepare)?;
    Ok(())
}

fn write_trace_outputs(trace: &[u8]) -> Result<(), PerfettoError> {
    let _guard = fxfs::suspend_persist();
    let _ = fxfs::delete_file(PERFETTO_LEGACY_SCHED_PFTRACE_PATH);
    let _ = fxfs::delete_file(PERFETTO_LEGACY_SCHED_TRACE_PATH);
    let _ = fxfs::delete_file(PERFETTO_LEGACY_SHARED_TRACE_PATH);
    fxfs::write_file(PERFETTO_TRACE_PATH, trace).map_err(|_| PerfettoError::FxfsPrepare)?;
    drop(_guard);
    fxfs::flush_persist();
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PerfettoNativeEvent {
    timestamp_ns: u64,
    track_uuid: u64,
    event_type: u64,
    cpu_id: usize,
    thread_id: usize,
    tick: u64,
    duration_us: u64,
    configured_slice_us: u64,
    sequence: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PerfettoThreadDescriptor {
    pid: u64,
    tid: u64,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PerfettoVmEvent {
    timestamp_ns: u64,
    track_uuid: u64,
    event_type: u64,
    name: String,
    state: &'static str,
    process_pid: usize,
    host_qemu_pid: u32,
    memory_bytes: usize,
    cpu_time_slice_us: u32,
    realtime_priority: u8,
    start_tick: u64,
    end_tick: u64,
    sequence: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PerfettoPolicyTask {
    id: usize,
    deadline_tick: u64,
    credit: i32,
    credit_cap: i32,
    total_ticks: u32,
    weight: u32,
    cpu_affinity: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PerfettoPolicyEvent {
    timestamp_ns: u64,
    track_uuid: u64,
    event_type: u64,
    policy: SchedulePolicy,
    cpu_id: usize,
    task_id: usize,
    step: usize,
    deadline_tick: u64,
    credit: i32,
    total_ticks: u32,
    weight: u32,
    sequence: usize,
}

fn encode_scheduler_trace_pftrace(
    entries: &[SchedulerTraceEntry],
    cpu_rows: &[usize],
    threads: &[usize],
    vms: &[VmRecord],
    policy: &'static str,
    start_tick: u64,
    end_tick: u64,
) -> Result<Vec<u8>, PerfettoError> {
    let capacity = 2048usize
        .saturating_add(entries.len().saturating_mul(320))
        .saturating_add(cpu_rows.len().saturating_mul(128))
        .saturating_add(threads.len().saturating_mul(32))
        .saturating_add(vms.len().saturating_mul(512));
    let mut trace = new_proto_vec(capacity)?;
    let base_tick = start_tick;

    push_track_descriptor_packet(
        &mut trace,
        SMROS_PROCESS_TRACK_UUID,
        "SMROS Scheduler",
        None,
        Some("SMROS Scheduler"),
        None,
        true,
    )?;
    for cpu_id in cpu_rows {
        let mut cpu_name = new_string(16)?;
        cpu_name.push_str("CPU");
        append_usize(&mut cpu_name, *cpu_id);
        let mut thread_name = new_string(32)?;
        thread_name.push_str("SMROS ");
        thread_name.push_str(cpu_name.as_str());
        push_track_descriptor_packet(
            &mut trace,
            cpu_track_uuid(*cpu_id),
            cpu_name.as_str(),
            Some(SMROS_PROCESS_TRACK_UUID),
            None,
            Some(PerfettoThreadDescriptor {
                pid: SMROS_PROCESS_PID,
                tid: SMROS_SCHED_TID_BASE.saturating_add(*cpu_id as u64),
                name: thread_name,
            }),
            false,
        )?;
    }
    if !vms.is_empty() {
        push_track_descriptor_packet(
            &mut trace,
            SMROS_VM_ROOT_TRACK_UUID,
            "SMROS VMs",
            Some(SMROS_PROCESS_TRACK_UUID),
            None,
            None,
            false,
        )?;
        for (index, vm) in vms.iter().enumerate() {
            let vm_name = vm_thread_name(vm.name.as_str())?;
            push_track_descriptor_packet(
                &mut trace,
                vm_track_uuid(index),
                vm.name.as_str(),
                Some(SMROS_VM_ROOT_TRACK_UUID),
                None,
                Some(PerfettoThreadDescriptor {
                    pid: SMROS_PROCESS_PID,
                    tid: SMROS_VM_TID_BASE.saturating_add(index as u64),
                    name: vm_name,
                }),
                false,
            )?;
        }
    }

    let mut events = collect_native_trace_events(entries, base_tick)?;
    sort_native_trace_events(events.as_mut_slice());
    for event in events {
        push_track_event_packet(&mut trace, event, policy)?;
    }
    let mut vm_events = collect_vm_trace_events(vms, start_tick, end_tick, base_tick)?;
    sort_vm_trace_events(vm_events.as_mut_slice());
    for event in vm_events {
        push_vm_event_packet(&mut trace, event)?;
    }

    Ok(trace)
}

fn encode_policy_comparison_pftrace(
    cpu_count: usize,
    steps: usize,
) -> Result<Vec<u8>, PerfettoError> {
    let capacity = 4096usize
        .saturating_add(
            cpu_count
                .saturating_mul(PERFETTO_POLICY_COUNT)
                .saturating_mul(160),
        )
        .saturating_add(
            cpu_count
                .saturating_mul(steps)
                .saturating_mul(PERFETTO_POLICY_COUNT)
                .saturating_mul(260),
        );
    let mut trace = new_proto_vec(capacity)?;

    push_track_descriptor_packet(
        &mut trace,
        SMROS_PROCESS_TRACK_UUID,
        "SMROS Scheduler Policy Compare",
        None,
        Some("SMROS Scheduler Policy Compare"),
        None,
        true,
    )?;

    for policy_index in 0..PERFETTO_POLICY_COUNT {
        let policy = compare_policy(policy_index);
        for cpu_id in 0..cpu_count {
            let mut cpu_name = new_string(32)?;
            cpu_name.push_str(policy.as_str());
            cpu_name.push_str(" CPU");
            append_usize(&mut cpu_name, cpu_id);
            let mut thread_name = new_string(40)?;
            thread_name.push_str("SMROS compare ");
            thread_name.push_str(cpu_name.as_str());
            push_track_descriptor_packet(
                &mut trace,
                policy_track_uuid(policy_index, cpu_id),
                cpu_name.as_str(),
                Some(SMROS_PROCESS_TRACK_UUID),
                None,
                Some(PerfettoThreadDescriptor {
                    pid: SMROS_PROCESS_PID,
                    tid: SMROS_SCHED_TID_BASE
                        .saturating_add(10_000)
                        .saturating_add((policy_index as u64).saturating_mul(1_000))
                        .saturating_add(cpu_id as u64),
                    name: thread_name,
                }),
                false,
            )?;
        }
    }

    let mut events = collect_policy_comparison_events(cpu_count, steps)?;
    sort_policy_trace_events(events.as_mut_slice());
    for event in events {
        push_policy_event_slice(&mut trace, event)?;
    }

    Ok(trace)
}

fn new_proto_vec(capacity: usize) -> Result<Vec<u8>, PerfettoError> {
    let mut out = Vec::new();
    out.try_reserve_exact(capacity)
        .map_err(|_| PerfettoError::Encode)?;
    Ok(out)
}

fn new_string(capacity: usize) -> Result<String, PerfettoError> {
    let mut out = String::new();
    out.try_reserve_exact(capacity)
        .map_err(|_| PerfettoError::Encode)?;
    Ok(out)
}

fn collect_native_trace_events(
    entries: &[SchedulerTraceEntry],
    base_tick: u64,
) -> Result<Vec<PerfettoNativeEvent>, PerfettoError> {
    let mut events = Vec::new();
    events
        .try_reserve_exact(entries.len().saturating_mul(2))
        .map_err(|_| PerfettoError::Encode)?;
    let mut index = 0usize;
    while index < entries.len() {
        let entry = entries[index];
        let ts_us = trace_ts_us(entry.tick, base_tick, index);
        let configured_slice_us = thread_configured_slice_us(entry.thread_id);
        let dur_us = trace_duration_us(
            entries,
            index,
            entry.cpu_id,
            entry.thread_id,
            base_tick,
            ts_us,
        );
        let track_uuid = cpu_track_uuid(entry.cpu_id);
        events.push(PerfettoNativeEvent {
            timestamp_ns: trace_ts_ns(ts_us),
            track_uuid,
            event_type: PERFETTO_EVENT_TYPE_SLICE_BEGIN,
            cpu_id: entry.cpu_id,
            thread_id: entry.thread_id,
            tick: entry.tick,
            duration_us: dur_us,
            configured_slice_us,
            sequence: index.saturating_mul(2),
        });
        events.push(PerfettoNativeEvent {
            timestamp_ns: trace_ts_ns(ts_us.saturating_add(dur_us)),
            track_uuid,
            event_type: PERFETTO_EVENT_TYPE_SLICE_END,
            cpu_id: entry.cpu_id,
            thread_id: entry.thread_id,
            tick: entry.tick,
            duration_us: dur_us,
            configured_slice_us,
            sequence: index.saturating_mul(2).saturating_add(1),
        });
        index += 1;
    }
    Ok(events)
}

fn collect_idle_trace_events(
    entries: &[SchedulerTraceEntry],
    cpu_rows: &[usize],
    base_tick: u64,
    end_tick: u64,
) -> Result<Vec<PerfettoNativeEvent>, PerfettoError> {
    let mut events = Vec::new();
    let idle_count = count_idle_cpu_rows(entries, cpu_rows);
    events
        .try_reserve_exact(idle_count.saturating_mul(2))
        .map_err(|_| PerfettoError::Encode)?;
    let duration_us = trace_ts_us(end_tick.saturating_add(1), base_tick, 0)
        .max(PERFETTO_TICK_US)
        .saturating_add(PERFETTO_TICK_US);
    let mut sequence = entries.len().saturating_mul(2);

    for cpu_id in cpu_rows {
        if trace_has_cpu(entries, *cpu_id) {
            continue;
        }
        events.push(PerfettoNativeEvent {
            timestamp_ns: 0,
            track_uuid: cpu_track_uuid(*cpu_id),
            event_type: PERFETTO_EVENT_TYPE_SLICE_BEGIN,
            cpu_id: *cpu_id,
            thread_id: ThreadId::IDLE.0,
            tick: base_tick,
            duration_us,
            configured_slice_us: duration_us,
            sequence,
        });
        sequence = sequence.saturating_add(1);
        events.push(PerfettoNativeEvent {
            timestamp_ns: trace_ts_ns(duration_us),
            track_uuid: cpu_track_uuid(*cpu_id),
            event_type: PERFETTO_EVENT_TYPE_SLICE_END,
            cpu_id: *cpu_id,
            thread_id: ThreadId::IDLE.0,
            tick: end_tick,
            duration_us,
            configured_slice_us: duration_us,
            sequence,
        });
        sequence = sequence.saturating_add(1);
    }
    Ok(events)
}

fn collect_policy_comparison_events(
    cpu_count: usize,
    steps: usize,
) -> Result<Vec<PerfettoPolicyEvent>, PerfettoError> {
    let mut events = Vec::new();
    events
        .try_reserve_exact(
            cpu_count
                .saturating_mul(steps)
                .saturating_mul(PERFETTO_POLICY_COUNT),
        )
        .map_err(|_| PerfettoError::Encode)?;

    for policy_index in 0..PERFETTO_POLICY_COUNT {
        let policy = compare_policy(policy_index);
        let mut tasks = compare_tasks();
        let mut next_task = 0usize;

        for step in 0..steps {
            for cpu_id in 0..cpu_count {
                let task_index = pick_compare_task(&tasks, policy, next_task, cpu_id, step);
                let task = tasks[task_index];
                let timestamp_ns = trace_ts_ns((step as u64).saturating_mul(PERFETTO_TICK_US));
                let duration_ns = trace_ts_ns(PERFETTO_TICK_US);
                let sequence = events.len().saturating_mul(2);
                events.push(PerfettoPolicyEvent {
                    timestamp_ns,
                    track_uuid: policy_track_uuid(policy_index, cpu_id),
                    event_type: PERFETTO_EVENT_TYPE_SLICE_BEGIN,
                    policy,
                    cpu_id,
                    task_id: task.id,
                    step,
                    deadline_tick: task.deadline_tick,
                    credit: task.credit,
                    total_ticks: task.total_ticks,
                    weight: task.weight,
                    sequence,
                });
                events.push(PerfettoPolicyEvent {
                    timestamp_ns: timestamp_ns.saturating_add(duration_ns),
                    track_uuid: policy_track_uuid(policy_index, cpu_id),
                    event_type: PERFETTO_EVENT_TYPE_SLICE_END,
                    policy,
                    cpu_id,
                    task_id: task.id,
                    step,
                    deadline_tick: task.deadline_tick,
                    credit: task.credit,
                    total_ticks: task.total_ticks,
                    weight: task.weight,
                    sequence: sequence.saturating_add(1),
                });
                update_compare_task(&mut tasks[task_index], policy, step);
                next_task = (task_index + 1) % tasks.len();
            }
        }
    }

    Ok(events)
}

fn collect_vm_trace_events(
    vms: &[VmRecord],
    trace_start_tick: u64,
    trace_end_tick: u64,
    base_tick: u64,
) -> Result<Vec<PerfettoVmEvent>, PerfettoError> {
    let mut events = Vec::new();
    events
        .try_reserve_exact(vms.len().saturating_mul(2))
        .map_err(|_| PerfettoError::Encode)?;

    for (index, vm) in vms.iter().enumerate() {
        let start_tick = core::cmp::max(vm.start_tick, trace_start_tick);
        let end_tick = trace_end_tick
            .saturating_add(1)
            .max(start_tick.saturating_add(1));
        let track_uuid = vm_track_uuid(index);
        events.push(PerfettoVmEvent {
            timestamp_ns: trace_ts_ns(trace_ts_us(start_tick, base_tick, index)),
            track_uuid,
            event_type: PERFETTO_EVENT_TYPE_SLICE_BEGIN,
            name: copy_vm_name(vm.name.as_str())?,
            state: vm.state.as_str().trim(),
            process_pid: vm.process_pid,
            host_qemu_pid: vm.host_qemu_pid,
            memory_bytes: vm.memory_bytes,
            cpu_time_slice_us: vm.cpu_time_slice_us,
            realtime_priority: vm.realtime_priority,
            start_tick,
            end_tick,
            sequence: index.saturating_mul(2),
        });
        events.push(PerfettoVmEvent {
            timestamp_ns: trace_ts_ns(trace_ts_us(end_tick, base_tick, index)),
            track_uuid,
            event_type: PERFETTO_EVENT_TYPE_SLICE_END,
            name: copy_vm_name(vm.name.as_str())?,
            state: vm.state.as_str().trim(),
            process_pid: vm.process_pid,
            host_qemu_pid: vm.host_qemu_pid,
            memory_bytes: vm.memory_bytes,
            cpu_time_slice_us: vm.cpu_time_slice_us,
            realtime_priority: vm.realtime_priority,
            start_tick,
            end_tick,
            sequence: index.saturating_mul(2).saturating_add(1),
        });
    }
    Ok(events)
}

fn sort_native_trace_events(events: &mut [PerfettoNativeEvent]) {
    let mut i = 1usize;
    while i < events.len() {
        let value = events[i];
        let mut j = i;
        while j > 0 && native_trace_event_after(events[j - 1], value) {
            events[j] = events[j - 1];
            j -= 1;
        }
        events[j] = value;
        i += 1;
    }
}

fn native_trace_event_after(lhs: PerfettoNativeEvent, rhs: PerfettoNativeEvent) -> bool {
    if lhs.timestamp_ns != rhs.timestamp_ns {
        return lhs.timestamp_ns > rhs.timestamp_ns;
    }
    if lhs.track_uuid != rhs.track_uuid {
        return lhs.track_uuid > rhs.track_uuid;
    }
    let lhs_order = if lhs.event_type == PERFETTO_EVENT_TYPE_SLICE_END {
        0usize
    } else {
        1usize
    };
    let rhs_order = if rhs.event_type == PERFETTO_EVENT_TYPE_SLICE_END {
        0usize
    } else {
        1usize
    };
    if lhs_order != rhs_order {
        return lhs_order > rhs_order;
    }
    lhs.sequence > rhs.sequence
}

fn sort_vm_trace_events(events: &mut [PerfettoVmEvent]) {
    let mut i = 1usize;
    while i < events.len() {
        let value = events[i].clone();
        let mut j = i;
        while j > 0 && vm_trace_event_after(&events[j - 1], &value) {
            events[j] = events[j - 1].clone();
            j -= 1;
        }
        events[j] = value;
        i += 1;
    }
}

fn vm_trace_event_after(lhs: &PerfettoVmEvent, rhs: &PerfettoVmEvent) -> bool {
    if lhs.timestamp_ns != rhs.timestamp_ns {
        return lhs.timestamp_ns > rhs.timestamp_ns;
    }
    if lhs.track_uuid != rhs.track_uuid {
        return lhs.track_uuid > rhs.track_uuid;
    }
    let lhs_order = if lhs.event_type == PERFETTO_EVENT_TYPE_SLICE_END {
        0usize
    } else {
        1usize
    };
    let rhs_order = if rhs.event_type == PERFETTO_EVENT_TYPE_SLICE_END {
        0usize
    } else {
        1usize
    };
    if lhs_order != rhs_order {
        return lhs_order > rhs_order;
    }
    lhs.sequence > rhs.sequence
}

fn sort_policy_trace_events(events: &mut [PerfettoPolicyEvent]) {
    let mut i = 1usize;
    while i < events.len() {
        let value = events[i];
        let mut j = i;
        while j > 0 && policy_trace_event_after(events[j - 1], value) {
            events[j] = events[j - 1];
            j -= 1;
        }
        events[j] = value;
        i += 1;
    }
}

fn policy_trace_event_after(lhs: PerfettoPolicyEvent, rhs: PerfettoPolicyEvent) -> bool {
    if lhs.timestamp_ns != rhs.timestamp_ns {
        return lhs.timestamp_ns > rhs.timestamp_ns;
    }
    if lhs.track_uuid != rhs.track_uuid {
        return lhs.track_uuid > rhs.track_uuid;
    }
    let lhs_order = if lhs.event_type == PERFETTO_EVENT_TYPE_SLICE_END {
        0usize
    } else {
        1usize
    };
    let rhs_order = if rhs.event_type == PERFETTO_EVENT_TYPE_SLICE_END {
        0usize
    } else {
        1usize
    };
    if lhs_order != rhs_order {
        return lhs_order > rhs_order;
    }
    lhs.sequence > rhs.sequence
}

fn compare_policy(index: usize) -> SchedulePolicy {
    match index {
        0 => SchedulePolicy::RoundRobin,
        1 => SchedulePolicy::Edf,
        2 => SchedulePolicy::Credit,
        _ => SchedulePolicy::Fair,
    }
}

fn compare_tasks() -> [PerfettoPolicyTask; PERFETTO_POLICY_TASK_COUNT] {
    [
        PerfettoPolicyTask {
            id: 1,
            deadline_tick: 55,
            credit: 80,
            credit_cap: 80,
            total_ticks: 12,
            weight: 1,
            cpu_affinity: None,
        },
        PerfettoPolicyTask {
            id: 2,
            deadline_tick: 30,
            credit: 45,
            credit_cap: 45,
            total_ticks: 16,
            weight: 4,
            cpu_affinity: Some(1),
        },
        PerfettoPolicyTask {
            id: 3,
            deadline_tick: 45,
            credit: 120,
            credit_cap: 120,
            total_ticks: 24,
            weight: 2,
            cpu_affinity: None,
        },
        PerfettoPolicyTask {
            id: 4,
            deadline_tick: 20,
            credit: 30,
            credit_cap: 30,
            total_ticks: 8,
            weight: 3,
            cpu_affinity: Some(7),
        },
        PerfettoPolicyTask {
            id: 5,
            deadline_tick: 75,
            credit: 160,
            credit_cap: 160,
            total_ticks: 35,
            weight: 5,
            cpu_affinity: None,
        },
        PerfettoPolicyTask {
            id: 6,
            deadline_tick: 40,
            credit: 65,
            credit_cap: 65,
            total_ticks: 14,
            weight: 2,
            cpu_affinity: Some(15),
        },
        PerfettoPolicyTask {
            id: 7,
            deadline_tick: 90,
            credit: 100,
            credit_cap: 100,
            total_ticks: 28,
            weight: 1,
            cpu_affinity: None,
        },
        PerfettoPolicyTask {
            id: 8,
            deadline_tick: 60,
            credit: 20,
            credit_cap: 20,
            total_ticks: 6,
            weight: 6,
            cpu_affinity: None,
        },
    ]
}

fn pick_compare_task(
    tasks: &[PerfettoPolicyTask],
    policy: SchedulePolicy,
    next_task: usize,
    cpu_id: usize,
    step: usize,
) -> usize {
    match policy {
        SchedulePolicy::RoundRobin => pick_compare_round_robin(tasks, next_task, cpu_id),
        SchedulePolicy::Edf => pick_compare_edf(tasks, cpu_id),
        SchedulePolicy::Credit => pick_compare_credit(tasks, cpu_id),
        SchedulePolicy::Fair => pick_compare_fair(tasks, cpu_id, step),
    }
}

fn pick_compare_round_robin(
    tasks: &[PerfettoPolicyTask],
    next_task: usize,
    cpu_id: usize,
) -> usize {
    if tasks.is_empty() {
        return 0;
    }
    let mut offset = 0usize;
    while offset < tasks.len() {
        let idx = (next_task + offset) % tasks.len();
        if compare_task_allowed(tasks[idx], cpu_id) {
            return idx;
        }
        offset += 1;
    }
    0
}

fn pick_compare_edf(tasks: &[PerfettoPolicyTask], cpu_id: usize) -> usize {
    let mut best = 0usize;
    let mut best_found = false;
    let mut best_deadline = u64::MAX;
    let mut idx = 0usize;
    while idx < tasks.len() {
        if compare_task_allowed(tasks[idx], cpu_id)
            && compare_edf_better(tasks[idx].deadline_tick, best_found, best_deadline)
        {
            best = idx;
            best_found = true;
            best_deadline = tasks[idx].deadline_tick;
        }
        idx += 1;
    }
    best
}

fn pick_compare_credit(tasks: &[PerfettoPolicyTask], cpu_id: usize) -> usize {
    let mut best = 0usize;
    let mut best_found = false;
    let mut best_credit = i32::MIN;
    let mut idx = 0usize;
    while idx < tasks.len() {
        if compare_task_allowed(tasks[idx], cpu_id)
            && compare_credit_better(tasks[idx].credit, best_found, best_credit)
        {
            best = idx;
            best_found = true;
            best_credit = tasks[idx].credit;
        }
        idx += 1;
    }
    best
}

fn pick_compare_fair(tasks: &[PerfettoPolicyTask], cpu_id: usize, step: usize) -> usize {
    let mut best = 0usize;
    let mut best_found = false;
    let mut best_ticks = 0u32;
    let mut best_weight = 1u32;
    let mut idx = 0usize;
    while idx < tasks.len() {
        if compare_task_allowed(tasks[idx], cpu_id)
            && compare_fair_better(
                tasks[idx].total_ticks.saturating_add((step % 3) as u32),
                tasks[idx].weight,
                best_found,
                best_ticks,
                best_weight,
            )
        {
            best = idx;
            best_found = true;
            best_ticks = tasks[idx].total_ticks;
            best_weight = tasks[idx].weight;
        }
        idx += 1;
    }
    best
}

fn compare_task_allowed(task: PerfettoPolicyTask, cpu_id: usize) -> bool {
    task.cpu_affinity.is_none() || task.cpu_affinity == Some(cpu_id)
}

fn compare_edf_better(deadline: u64, best_found: bool, best_deadline: u64) -> bool {
    !best_found || deadline < best_deadline
}

fn compare_credit_better(credit: i32, best_found: bool, best_credit: i32) -> bool {
    !best_found || credit > best_credit
}

fn compare_fair_better(
    ticks: u32,
    weight: u32,
    best_found: bool,
    best_ticks: u32,
    best_weight: u32,
) -> bool {
    let weight = if weight == 0 { 1u128 } else { weight as u128 };
    let best_weight = if best_weight == 0 {
        1u128
    } else {
        best_weight as u128
    };
    let candidate_score = match (ticks as u128).checked_mul(best_weight) {
        Some(score) => score,
        None => u128::MAX,
    };
    let best_score = match (best_ticks as u128).checked_mul(weight) {
        Some(score) => score,
        None => u128::MAX,
    };
    !best_found || candidate_score < best_score
}

fn update_compare_task(task: &mut PerfettoPolicyTask, policy: SchedulePolicy, step: usize) {
    task.total_ticks = task.total_ticks.saturating_add(1);
    match policy {
        SchedulePolicy::Edf => {
            if step as u64 >= task.deadline_tick {
                task.deadline_tick = task
                    .deadline_tick
                    .saturating_add(50)
                    .saturating_add(task.id as u64);
            }
        }
        SchedulePolicy::Credit => {
            if task.credit > 0 {
                task.credit -= 1;
            }
            if task.credit <= 0 {
                task.credit = task.credit_cap.max(1);
            }
        }
        SchedulePolicy::Fair | SchedulePolicy::RoundRobin => {}
    }
}

fn push_track_descriptor_packet(
    trace: &mut Vec<u8>,
    uuid: u64,
    name: &str,
    parent_uuid: Option<u64>,
    process_name: Option<&str>,
    thread: Option<PerfettoThreadDescriptor>,
    sequence_start: bool,
) -> Result<(), PerfettoError> {
    let mut descriptor = new_proto_vec(96usize.saturating_add(name.len()))?;
    push_u64_field(&mut descriptor, PERFETTO_TRACK_UUID_FIELD, uuid);
    push_string_field(&mut descriptor, PERFETTO_TRACK_NAME_FIELD, name);
    if let Some(parent_uuid) = parent_uuid {
        push_u64_field(
            &mut descriptor,
            PERFETTO_TRACK_PARENT_UUID_FIELD,
            parent_uuid,
        );
    }
    if let Some(process_name) = process_name {
        let mut process = new_proto_vec(32usize.saturating_add(process_name.len()))?;
        push_u64_field(&mut process, PERFETTO_PROCESS_PID_FIELD, SMROS_PROCESS_PID);
        push_string_field(&mut process, PERFETTO_PROCESS_NAME_FIELD, process_name);
        push_bytes_field(
            &mut descriptor,
            PERFETTO_TRACK_PROCESS_FIELD,
            process.as_slice(),
        );
    }
    if let Some(thread) = thread {
        let mut thread_descriptor = new_proto_vec(48usize.saturating_add(thread.name.len()))?;
        push_u64_field(
            &mut thread_descriptor,
            PERFETTO_THREAD_PID_FIELD,
            thread.pid,
        );
        push_u64_field(
            &mut thread_descriptor,
            PERFETTO_THREAD_TID_FIELD,
            thread.tid,
        );
        push_string_field(
            &mut thread_descriptor,
            PERFETTO_THREAD_NAME_FIELD,
            thread.name.as_str(),
        );
        push_bytes_field(
            &mut descriptor,
            PERFETTO_TRACK_THREAD_FIELD,
            thread_descriptor.as_slice(),
        );
    }

    let mut packet = new_proto_vec(16usize.saturating_add(descriptor.len()))?;
    push_u64_field(&mut packet, PERFETTO_PACKET_TIMESTAMP_FIELD, 0);
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TRUSTED_SEQUENCE_ID_FIELD,
        SMROS_TRUSTED_PACKET_SEQUENCE_ID,
    );
    if sequence_start {
        push_u64_field(
            &mut packet,
            PERFETTO_PACKET_SEQUENCE_FLAGS_FIELD,
            PERFETTO_SEQ_INCREMENTAL_STATE_CLEARED,
        );
        push_u64_field(&mut packet, PERFETTO_PACKET_FIRST_ON_SEQUENCE_FIELD, 1);
    }
    push_bytes_field(
        &mut packet,
        PERFETTO_PACKET_TRACK_DESCRIPTOR_FIELD,
        descriptor.as_slice(),
    );
    push_bytes_field(trace, PERFETTO_TRACE_PACKET_FIELD, packet.as_slice());
    Ok(())
}

fn push_vm_event_packet(trace: &mut Vec<u8>, event: PerfettoVmEvent) -> Result<(), PerfettoError> {
    let mut track_event = new_proto_vec(256usize.saturating_add(event.name.len()))?;
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_TYPE_FIELD,
        event.event_type,
    );
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_TRACK_UUID_FIELD,
        event.track_uuid,
    );

    if event.event_type == PERFETTO_EVENT_TYPE_SLICE_BEGIN {
        push_string_field(&mut track_event, PERFETTO_EVENT_CATEGORIES_FIELD, "vm");
        push_string_field(
            &mut track_event,
            PERFETTO_EVENT_NAME_FIELD,
            event.name.as_str(),
        );
        push_debug_string(&mut track_event, "vm", event.name.as_str())?;
        push_debug_string(&mut track_event, "state", event.state)?;
        push_debug_uint(&mut track_event, "process_pid", event.process_pid as u64)?;
        push_debug_uint(
            &mut track_event,
            "host_qemu_pid",
            event.host_qemu_pid as u64,
        )?;
        push_debug_uint(
            &mut track_event,
            "memory_kb",
            (event.memory_bytes / 1024) as u64,
        )?;
        push_debug_uint(
            &mut track_event,
            "cpu_slice_us",
            event.cpu_time_slice_us as u64,
        )?;
        push_debug_uint(
            &mut track_event,
            "realtime_priority",
            event.realtime_priority as u64,
        )?;
        push_debug_uint(&mut track_event, "start_tick", event.start_tick)?;
        push_debug_uint(&mut track_event, "end_tick", event.end_tick)?;
    }

    let mut packet = new_proto_vec(24usize.saturating_add(track_event.len()))?;
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TIMESTAMP_FIELD,
        event.timestamp_ns,
    );
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TRUSTED_SEQUENCE_ID_FIELD,
        SMROS_TRUSTED_PACKET_SEQUENCE_ID,
    );
    push_bytes_field(
        &mut packet,
        PERFETTO_PACKET_TRACK_EVENT_FIELD,
        track_event.as_slice(),
    );
    push_bytes_field(trace, PERFETTO_TRACE_PACKET_FIELD, packet.as_slice());
    Ok(())
}

fn push_policy_event_slice(
    trace: &mut Vec<u8>,
    event: PerfettoPolicyEvent,
) -> Result<(), PerfettoError> {
    push_policy_event_packet(trace, event)
}

fn push_policy_event_packet(
    trace: &mut Vec<u8>,
    event: PerfettoPolicyEvent,
) -> Result<(), PerfettoError> {
    let mut track_event = new_proto_vec(224)?;
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_TYPE_FIELD,
        event.event_type,
    );
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_TRACK_UUID_FIELD,
        event.track_uuid,
    );

    if event.event_type == PERFETTO_EVENT_TYPE_SLICE_BEGIN {
        let label = policy_task_label(event.policy, event.task_id, event.weight)?;
        push_string_field(
            &mut track_event,
            PERFETTO_EVENT_CATEGORIES_FIELD,
            "sched-compare",
        );
        push_string_field(&mut track_event, PERFETTO_EVENT_NAME_FIELD, label.as_str());
        push_debug_string(&mut track_event, "policy", event.policy.as_str())?;
        push_debug_uint(&mut track_event, "cpu", event.cpu_id as u64)?;
        push_debug_uint(&mut track_event, "task", event.task_id as u64)?;
        push_debug_uint(&mut track_event, "step", event.step as u64)?;
        push_debug_uint(&mut track_event, "deadline_tick", event.deadline_tick)?;
        push_debug_i64(&mut track_event, "credit", event.credit as i64)?;
        push_debug_uint(&mut track_event, "total_ticks", event.total_ticks as u64)?;
        push_debug_uint(&mut track_event, "weight", event.weight as u64)?;
    }

    let mut packet = new_proto_vec(24usize.saturating_add(track_event.len()))?;
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TIMESTAMP_FIELD,
        event.timestamp_ns,
    );
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TRUSTED_SEQUENCE_ID_FIELD,
        SMROS_TRUSTED_PACKET_SEQUENCE_ID,
    );
    push_bytes_field(
        &mut packet,
        PERFETTO_PACKET_TRACK_EVENT_FIELD,
        track_event.as_slice(),
    );
    push_bytes_field(trace, PERFETTO_TRACE_PACKET_FIELD, packet.as_slice());
    Ok(())
}

fn push_track_event_packet(
    trace: &mut Vec<u8>,
    event: PerfettoNativeEvent,
    policy: &'static str,
) -> Result<(), PerfettoError> {
    let mut track_event = new_proto_vec(192)?;
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_TYPE_FIELD,
        event.event_type,
    );
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_TRACK_UUID_FIELD,
        event.track_uuid,
    );
    push_u64_field(
        &mut track_event,
        PERFETTO_EVENT_CORRELATION_ID_FIELD,
        task_correlation_id(event.thread_id),
    );

    if event.event_type == PERFETTO_EVENT_TYPE_SLICE_BEGIN {
        let label = thread_label(event.thread_id)?;
        let (color_name, color_rgb) = task_color(event.thread_id);
        push_string_field(&mut track_event, PERFETTO_EVENT_CATEGORIES_FIELD, "sched");
        push_string_field(&mut track_event, PERFETTO_EVENT_NAME_FIELD, label.as_str());
        push_debug_uint(&mut track_event, "cpu", event.cpu_id as u64)?;
        push_debug_uint(&mut track_event, "thread", event.thread_id as u64)?;
        push_debug_uint(&mut track_event, "task_color_rgb", color_rgb as u64)?;
        push_debug_string(&mut track_event, "task_color", color_name)?;
        if let Some(name) = thread_name(event.thread_id) {
            if !name.is_empty() {
                push_debug_string(&mut track_event, "thread_name", name)?;
            }
        }
        push_debug_string(&mut track_event, "policy", policy)?;
        push_debug_uint(&mut track_event, "tick", event.tick)?;
        push_debug_uint(&mut track_event, "duration_us", event.duration_us)?;
        push_debug_uint(
            &mut track_event,
            "configured_slice_us",
            event.configured_slice_us,
        )?;
        if event.thread_id == ThreadId::IDLE.0 {
            push_debug_string(&mut track_event, "coverage", "logical-cpu-idle")?;
        }
        let mut symbol = new_string(1)?;
        symbol.push(trace_symbol(event.thread_id) as char);
        push_debug_string(&mut track_event, "symbol", symbol.as_str())?;
    }

    let mut packet = new_proto_vec(24usize.saturating_add(track_event.len()))?;
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TIMESTAMP_FIELD,
        event.timestamp_ns,
    );
    push_u64_field(
        &mut packet,
        PERFETTO_PACKET_TRUSTED_SEQUENCE_ID_FIELD,
        SMROS_TRUSTED_PACKET_SEQUENCE_ID,
    );
    push_bytes_field(
        &mut packet,
        PERFETTO_PACKET_TRACK_EVENT_FIELD,
        track_event.as_slice(),
    );
    push_bytes_field(trace, PERFETTO_TRACE_PACKET_FIELD, packet.as_slice());
    Ok(())
}

fn push_debug_uint(out: &mut Vec<u8>, name: &str, value: u64) -> Result<(), PerfettoError> {
    let mut annotation = new_proto_vec(32usize.saturating_add(name.len()))?;
    push_string_field(&mut annotation, PERFETTO_DEBUG_NAME_FIELD, name);
    push_u64_field(&mut annotation, PERFETTO_DEBUG_UINT_VALUE_FIELD, value);
    push_bytes_field(
        out,
        PERFETTO_EVENT_DEBUG_ANNOTATIONS_FIELD,
        annotation.as_slice(),
    );
    Ok(())
}

fn push_debug_i64(out: &mut Vec<u8>, name: &str, value: i64) -> Result<(), PerfettoError> {
    if value < 0 {
        let mut text = new_string(21)?;
        text.push('-');
        append_u64(&mut text, value.saturating_abs() as u64);
        push_debug_string(out, name, text.as_str())
    } else {
        push_debug_uint(out, name, value as u64)
    }
}

fn push_debug_string(out: &mut Vec<u8>, name: &str, value: &str) -> Result<(), PerfettoError> {
    let mut annotation = new_proto_vec(
        32usize
            .saturating_add(name.len())
            .saturating_add(value.len()),
    )?;
    push_string_field(&mut annotation, PERFETTO_DEBUG_NAME_FIELD, name);
    push_string_field(&mut annotation, PERFETTO_DEBUG_STRING_VALUE_FIELD, value);
    push_bytes_field(
        out,
        PERFETTO_EVENT_DEBUG_ANNOTATIONS_FIELD,
        annotation.as_slice(),
    );
    Ok(())
}

fn push_key(out: &mut Vec<u8>, field_number: u32, wire_type: u8) {
    push_varint(out, ((field_number as u64) << 3) | u64::from(wire_type));
}

fn push_u64_field(out: &mut Vec<u8>, field_number: u32, value: u64) {
    push_key(out, field_number, PERFETTO_WIRE_VARINT);
    push_varint(out, value);
}

fn push_string_field(out: &mut Vec<u8>, field_number: u32, value: &str) {
    push_bytes_field(out, field_number, value.as_bytes());
}

fn push_bytes_field(out: &mut Vec<u8>, field_number: u32, bytes: &[u8]) {
    push_key(out, field_number, PERFETTO_WIRE_LENGTH_DELIMITED);
    push_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn trace_duration_us(
    entries: &[SchedulerTraceEntry],
    index: usize,
    cpu_id: usize,
    thread_id: usize,
    base_tick: u64,
    ts: u64,
) -> u64 {
    let max_duration = thread_configured_slice_us(thread_id).max(1);
    let mut next = index + 1;
    while next < entries.len() {
        if entries[next].cpu_id == cpu_id {
            let next_ts = trace_ts_us(entries[next].tick, base_tick, next);
            return next_ts.saturating_sub(ts).clamp(1, max_duration);
        }
        next += 1;
    }
    max_duration
}

fn thread_configured_slice_us(thread_id: usize) -> u64 {
    scheduler::scheduler()
        .thread_schedule_info(ThreadId(thread_id))
        .map(|info| u64::from(info.time_slice_ticks).saturating_mul(PERFETTO_TICK_US))
        .unwrap_or(PERFETTO_TICK_US)
        .max(1)
}

fn trace_ts_us(tick: u64, base_tick: u64, index: usize) -> u64 {
    tick.saturating_sub(base_tick)
        .saturating_mul(PERFETTO_TICK_US)
        .saturating_add(index as u64)
}

fn trace_ts_ns(ts_us: u64) -> u64 {
    ts_us.saturating_mul(PERFETTO_NS_PER_US)
}

fn trace_tick_bounds(entries: &[SchedulerTraceEntry]) -> (u64, u64) {
    let mut start_tick = u64::MAX;
    let mut end_tick = 0u64;
    for entry in entries {
        start_tick = core::cmp::min(start_tick, entry.tick);
        end_tick = core::cmp::max(end_tick, entry.tick);
    }
    if start_tick == u64::MAX {
        (0, 0)
    } else {
        (start_tick, end_tick)
    }
}

fn trace_window_has_sample_worker(
    scheduler: &scheduler::Scheduler,
    start: usize,
    samples: usize,
) -> bool {
    for index in 0..samples {
        if let Some(entry) = scheduler.trace_entry(start + index) {
            if scheduler.trace_entry_is_sample_worker(entry) {
                return true;
            }
        }
    }
    false
}

fn trace_entry_should_export(
    scheduler: &scheduler::Scheduler,
    entry: SchedulerTraceEntry,
    filter_sample_workers: bool,
) -> bool {
    if filter_sample_workers {
        scheduler.trace_entry_is_sample_worker(entry)
    } else {
        scheduler.trace_entry_is_task(entry)
    }
}

fn logical_cpu_count() -> usize {
    core::cmp::max(
        1,
        core::cmp::min(smp::online_cpu_count() as usize, scheduler::MAX_CPUS),
    )
}

fn fill_logical_cpu_rows(rows: &mut [usize]) -> usize {
    let count = core::cmp::min(logical_cpu_count(), rows.len());
    let mut index = 0usize;
    while index < count {
        rows[index] = index;
        index += 1;
    }
    count
}

fn count_idle_cpu_rows(entries: &[SchedulerTraceEntry], cpu_rows: &[usize]) -> usize {
    let mut count = 0usize;
    for cpu_id in cpu_rows {
        if !trace_has_cpu(entries, *cpu_id) {
            count += 1;
        }
    }
    count
}

fn trace_has_cpu(entries: &[SchedulerTraceEntry], cpu_id: usize) -> bool {
    for entry in entries {
        if entry.cpu_id == cpu_id {
            return true;
        }
    }
    false
}

fn cpu_track_uuid(cpu_id: usize) -> u64 {
    SMROS_CPU_TRACK_UUID_BASE.saturating_add(cpu_id as u64)
}

fn vm_track_uuid(index: usize) -> u64 {
    SMROS_VM_TRACK_UUID_BASE.saturating_add(index as u64)
}

fn policy_track_uuid(policy_index: usize, cpu_id: usize) -> u64 {
    SMROS_POLICY_TRACK_UUID_BASE
        .saturating_add((policy_index as u64).saturating_mul(SMROS_POLICY_TRACK_UUID_STRIDE))
        .saturating_add(cpu_id as u64)
}

fn copy_vm_name(name: &str) -> Result<String, PerfettoError> {
    let mut out = new_string(name.len())?;
    out.push_str(name);
    Ok(out)
}

fn vm_thread_name(name: &str) -> Result<String, PerfettoError> {
    let mut out = new_string(3usize.saturating_add(name.len()))?;
    out.push_str("VM ");
    out.push_str(name);
    Ok(out)
}

fn policy_task_label(
    policy: SchedulePolicy,
    task_id: usize,
    weight: u32,
) -> Result<String, PerfettoError> {
    let mut label = new_string(32)?;
    label.push_str(policy.as_str());
    label.push_str(" T");
    append_usize(&mut label, task_id);
    label.push_str(" w");
    append_u64(&mut label, weight as u64);
    Ok(label)
}

fn thread_label(thread_id: usize) -> Result<String, PerfettoError> {
    let mut label = new_string(32)?;
    label.push('T');
    append_usize(&mut label, thread_id);
    if let Some(name) = thread_name(thread_id) {
        if !name.is_empty() {
            label.push(' ');
            label.push_str(name);
        }
    }
    Ok(label)
}

fn thread_name(thread_id: usize) -> Option<&'static str> {
    scheduler::scheduler()
        .get_thread(ThreadId(thread_id))
        .map(|thread| thread.name)
}

fn trace_symbol(thread_id: usize) -> u8 {
    const SYMBOLS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if thread_id < SYMBOLS.len() {
        SYMBOLS[thread_id]
    } else {
        b'*'
    }
}

fn task_correlation_id(thread_id: usize) -> u64 {
    SMROS_TASK_CORRELATION_ID_BASE.saturating_add(thread_id as u64)
}

fn task_color(thread_id: usize) -> (&'static str, u32) {
    PERFETTO_TASK_COLORS[thread_id % PERFETTO_TASK_COLORS.len()]
}

fn append_usize(out: &mut String, value: usize) {
    append_u64(out, value as u64);
}

fn append_u64(out: &mut String, mut value: u64) {
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
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
}

fn contains_usize(values: &[usize], needle: usize) -> bool {
    for value in values {
        if *value == needle {
            return true;
        }
    }
    false
}

fn sort_usize_prefix(values: &mut [usize], len: usize) {
    let len = len.min(values.len());
    let mut i = 1usize;
    while i < len {
        let value = values[i];
        let mut j = i;
        while j > 0 && values[j - 1] > value {
            values[j] = values[j - 1];
            j -= 1;
        }
        values[j] = value;
        i += 1;
    }
}
