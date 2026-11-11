//! Deadline for one Hermes campaign guest operation.
//!
//! The shell arms this around a single guest op. `yield_now` and the AArch64
//! WFI path consult it so a stuck virtio/timer wait resets the guest instead
//! of sitting forever after the iteration banner. Host `ut`/`it` polling is
//! outside the armed window and keeps its own job timeout.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Guest-op budget. Nested Linux boot is allowed 60s; this stays above that
/// and still fails a wedged iteration instead of hanging the campaign.
pub const OP_DEADLINE_NANOS: u64 = 120_000_000_000;

const OP_NAME_CAP: usize = 96;

static ARMED: AtomicBool = AtomicBool::new(false);
static DEADLINE_NS: AtomicU64 = AtomicU64::new(0);
static ITERATION: AtomicUsize = AtomicUsize::new(0);
static OP_NAME_LEN: AtomicUsize = AtomicUsize::new(0);
static mut OP_NAME: [u8; OP_NAME_CAP] = [0; OP_NAME_CAP];

pub fn arm(deadline_ns: u64, iteration: usize, op: &str) {
    let bytes = op.as_bytes();
    let len = bytes.len().min(OP_NAME_CAP);
    unsafe {
        OP_NAME[..len].copy_from_slice(&bytes[..len]);
    }
    OP_NAME_LEN.store(len, Ordering::Release);
    ITERATION.store(iteration, Ordering::Release);
    DEADLINE_NS.store(deadline_ns, Ordering::Release);
    ARMED.store(true, Ordering::Release);
}

pub fn clear() {
    ARMED.store(false, Ordering::Release);
    DEADLINE_NS.store(0, Ordering::Release);
}

pub fn exceeded(now_ns: u64) -> bool {
    if !ARMED.load(Ordering::Acquire) {
        return false;
    }
    let deadline = DEADLINE_NS.load(Ordering::Acquire);
    deadline != 0 && now_ns >= deadline
}

pub fn iteration() -> usize {
    ITERATION.load(Ordering::Acquire)
}

pub fn op_name() -> &'static str {
    let len = OP_NAME_LEN.load(Ordering::Acquire).min(OP_NAME_CAP);
    // SAFETY: `arm` writes `len` bytes of UTF-8 before publishing the length.
    let bytes = unsafe { &OP_NAME[..len] };
    core::str::from_utf8(bytes).unwrap_or("?")
}

/// Print the fail-closed line and reset. Resume sees `op_running` when the
/// pre-op checkpoint reached disk, then continues with the remaining cases.
pub fn expire_and_reset() -> ! {
    ARMED.store(false, Ordering::Release);
    DEADLINE_NS.store(0, Ordering::Release);
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("\nFAIL iteration=");
    write_usize(&mut serial, iteration());
    serial.write_str(" op=");
    serial.write_str(op_name());
    serial.write_str(" reason=timeout\n");
    let (now, compare) = crate::kernel_lowlevel::timer::debug_compare_snapshot();
    serial.write_str("diag timer now=");
    write_u64(&mut serial, now);
    serial.write_str(" compare=");
    write_u64(&mut serial, compare);
    serial.write_str("\n");
    crate::kernel_lowlevel::smp::system_reset()
}

fn write_usize(serial: &mut crate::kernel_lowlevel::serial::Serial, value: usize) {
    write_u64(serial, value as u64);
}

fn write_u64(serial: &mut crate::kernel_lowlevel::serial::Serial, mut value: u64) {
    if value == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut index = buf.len();
    while value > 0 {
        index -= 1;
        buf[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    for byte in &buf[index..] {
        serial.write_byte(*byte);
    }
}
