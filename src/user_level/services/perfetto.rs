//! Perfetto-compatible trace export for SMROS.
//!
//! This is a small SMROS-native bridge, not the upstream Perfetto daemon stack.
//! It emits native Perfetto protobuf trace files so the browser Perfetto UI can
//! open scheduler timelines directly.

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel_lowlevel::thread::ThreadId;
use crate::kernel_objects::scheduler::{self, SchedulerTraceEntry, SCHED_TRACE_CAPACITY};
use crate::user_level::fxfs;

pub const PERFETTO_TRACE_PATH: &str = "/shared/trace.pftrace";
pub const PERFETTO_COMPAT_FORMAT: &str = "perfetto-protobuf-trace";
pub const PERFETTO_TICK_US: u64 = 10_000;

const PERFETTO_LEGACY_SCHED_PFTRACE_PATH: &str = "/data/perfetto/sched-trace.pftrace";
const PERFETTO_LEGACY_SCHED_TRACE_PATH: &str = "/data/perfetto/sched-trace.json";
const PERFETTO_LEGACY_SHARED_TRACE_PATH: &str = "/shared/trace.json";
const PERFETTO_NS_PER_US: u64 = 1_000;
const SMROS_PROCESS_TRACK_UUID: u64 = 0x534d_524f_5300_0001;
const SMROS_CPU_TRACK_UUID_BASE: u64 = 0x534d_524f_5301_0000;
const PERFETTO_WIRE_VARINT: u8 = 0;
const PERFETTO_WIRE_LENGTH_DELIMITED: u8 = 2;
const PERFETTO_TRACE_PACKET_FIELD: u32 = 1;
const PERFETTO_PACKET_TIMESTAMP_FIELD: u32 = 8;
const PERFETTO_PACKET_TRACK_EVENT_FIELD: u32 = 11;
const PERFETTO_PACKET_TRACK_DESCRIPTOR_FIELD: u32 = 60;
const PERFETTO_TRACK_UUID_FIELD: u32 = 1;
const PERFETTO_TRACK_NAME_FIELD: u32 = 2;
const PERFETTO_TRACK_PROCESS_FIELD: u32 = 3;
const PERFETTO_TRACK_PARENT_UUID_FIELD: u32 = 5;
const PERFETTO_PROCESS_PID_FIELD: u32 = 1;
const PERFETTO_PROCESS_NAME_FIELD: u32 = 6;
const PERFETTO_EVENT_DEBUG_ANNOTATIONS_FIELD: u32 = 4;
const PERFETTO_EVENT_TYPE_FIELD: u32 = 9;
const PERFETTO_EVENT_TRACK_UUID_FIELD: u32 = 11;
const PERFETTO_EVENT_CATEGORIES_FIELD: u32 = 22;
const PERFETTO_EVENT_NAME_FIELD: u32 = 23;
const PERFETTO_EVENT_TYPE_SLICE_BEGIN: u64 = 1;
const PERFETTO_EVENT_TYPE_SLICE_END: u64 = 2;
const PERFETTO_DEBUG_UINT_VALUE_FIELD: u32 = 3;
const PERFETTO_DEBUG_STRING_VALUE_FIELD: u32 = 6;
const PERFETTO_DEBUG_NAME_FIELD: u32 = 10;
const SMROS_PROCESS_PID: u64 = 1;

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
    pub start_tick: u64,
    pub end_tick: u64,
    pub tick_us: u64,
}

pub fn export_scheduler_trace(
    samples: usize,
) -> Result<PerfettoSchedulerTraceExport, PerfettoError> {
    prepare_storage()?;

    let scheduler = scheduler::scheduler();
    let trace_len = scheduler.trace_len();
    if trace_len == 0 {
        return Err(PerfettoError::NoSamples);
    }

    let samples = samples.clamp(1, SCHED_TRACE_CAPACITY).min(trace_len);
    let start = trace_len.saturating_sub(samples);
    let mut entries = [SchedulerTraceEntry::empty(); SCHED_TRACE_CAPACITY];
    let mut entry_count = 0usize;
    let mut cpu_rows = [usize::MAX; 16];
    let mut cpu_count = 0usize;
    let mut threads = [usize::MAX; 32];
    let mut thread_count = 0usize;

    while entry_count < samples {
        if let Some(entry) = scheduler.trace_entry(start + entry_count) {
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
        }
        entry_count += 1;
    }

    sort_usize_prefix(&mut cpu_rows, cpu_count);
    sort_usize_prefix(&mut threads, thread_count);

    let policy = scheduler.policy().as_str();
    let trace = encode_scheduler_trace_pftrace(
        &entries[..entry_count],
        &cpu_rows[..cpu_count],
        &threads[..thread_count],
        policy,
    )?;
    write_trace_outputs(trace.as_slice())?;

    Ok(PerfettoSchedulerTraceExport {
        path: PERFETTO_TRACE_PATH,
        format: PERFETTO_COMPAT_FORMAT,
        policy,
        bytes: trace.len(),
        samples: entry_count,
        slices: entry_count,
        cpu_tracks: cpu_count,
        thread_count,
        start_tick: entries[0].tick,
        end_tick: entries[entry_count - 1].tick,
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
    sequence: usize,
}

fn encode_scheduler_trace_pftrace(
    entries: &[SchedulerTraceEntry],
    cpu_rows: &[usize],
    threads: &[usize],
    policy: &'static str,
) -> Result<Vec<u8>, PerfettoError> {
    let capacity = 2048usize
        .saturating_add(entries.len().saturating_mul(320))
        .saturating_add(cpu_rows.len().saturating_mul(128))
        .saturating_add(threads.len().saturating_mul(32));
    let mut trace = new_proto_vec(capacity)?;

    push_track_descriptor_packet(
        &mut trace,
        SMROS_PROCESS_TRACK_UUID,
        "SMROS Scheduler",
        None,
        Some("SMROS Scheduler"),
    )?;
    for cpu_id in cpu_rows {
        let mut cpu_name = new_string(16)?;
        cpu_name.push_str("CPU");
        append_usize(&mut cpu_name, *cpu_id);
        push_track_descriptor_packet(
            &mut trace,
            cpu_track_uuid(*cpu_id),
            cpu_name.as_str(),
            Some(SMROS_PROCESS_TRACK_UUID),
            None,
        )?;
    }

    let mut events = collect_native_trace_events(entries)?;
    sort_native_trace_events(events.as_mut_slice());
    for event in events {
        push_track_event_packet(&mut trace, event, policy)?;
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
) -> Result<Vec<PerfettoNativeEvent>, PerfettoError> {
    let mut events = Vec::new();
    events
        .try_reserve_exact(entries.len().saturating_mul(2))
        .map_err(|_| PerfettoError::Encode)?;
    let mut index = 0usize;
    while index < entries.len() {
        let entry = entries[index];
        let ts_us = trace_ts_us(entry.tick, index);
        let dur_us = trace_duration_us(entries, index, entry.cpu_id, ts_us);
        let track_uuid = cpu_track_uuid(entry.cpu_id);
        events.push(PerfettoNativeEvent {
            timestamp_ns: trace_ts_ns(ts_us),
            track_uuid,
            event_type: PERFETTO_EVENT_TYPE_SLICE_BEGIN,
            cpu_id: entry.cpu_id,
            thread_id: entry.thread_id,
            tick: entry.tick,
            sequence: index.saturating_mul(2),
        });
        events.push(PerfettoNativeEvent {
            timestamp_ns: trace_ts_ns(ts_us.saturating_add(dur_us)),
            track_uuid,
            event_type: PERFETTO_EVENT_TYPE_SLICE_END,
            cpu_id: entry.cpu_id,
            thread_id: entry.thread_id,
            tick: entry.tick,
            sequence: index.saturating_mul(2).saturating_add(1),
        });
        index += 1;
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

fn push_track_descriptor_packet(
    trace: &mut Vec<u8>,
    uuid: u64,
    name: &str,
    parent_uuid: Option<u64>,
    process_name: Option<&str>,
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

    let mut packet = new_proto_vec(16usize.saturating_add(descriptor.len()))?;
    push_bytes_field(
        &mut packet,
        PERFETTO_PACKET_TRACK_DESCRIPTOR_FIELD,
        descriptor.as_slice(),
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

    if event.event_type == PERFETTO_EVENT_TYPE_SLICE_BEGIN {
        let label = thread_label(event.thread_id)?;
        push_string_field(&mut track_event, PERFETTO_EVENT_CATEGORIES_FIELD, "sched");
        push_string_field(&mut track_event, PERFETTO_EVENT_NAME_FIELD, label.as_str());
        push_debug_uint(&mut track_event, "cpu", event.cpu_id as u64)?;
        push_debug_uint(&mut track_event, "thread", event.thread_id as u64)?;
        if let Some(name) = thread_name(event.thread_id) {
            if !name.is_empty() {
                push_debug_string(&mut track_event, "thread_name", name)?;
            }
        }
        push_debug_string(&mut track_event, "policy", policy)?;
        push_debug_uint(&mut track_event, "tick", event.tick)?;
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

fn trace_duration_us(entries: &[SchedulerTraceEntry], index: usize, cpu_id: usize, ts: u64) -> u64 {
    let mut next = index + 1;
    while next < entries.len() {
        if entries[next].cpu_id == cpu_id {
            let next_ts = trace_ts_us(entries[next].tick, next);
            return next_ts.saturating_sub(ts).max(1);
        }
        next += 1;
    }
    PERFETTO_TICK_US
}

fn trace_ts_us(tick: u64, index: usize) -> u64 {
    tick.saturating_mul(PERFETTO_TICK_US)
        .saturating_add(index as u64)
}

fn trace_ts_ns(ts_us: u64) -> u64 {
    ts_us.saturating_mul(PERFETTO_NS_PER_US)
}

fn cpu_track_uuid(cpu_id: usize) -> u64 {
    SMROS_CPU_TRACK_UUID_BASE.saturating_add(cpu_id as u64)
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
