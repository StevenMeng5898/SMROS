// Pure RISC-V trap-frame predicates. The unsafe frame load/store stays in
// `linux_riscv_syscall_context.rs` and is not part of this file.

macro_rules! smros_riscv_frame_aligned_body {
    ($saved_frame:expr) => {{
        $saved_frame != 0 && ($saved_frame & 7) == 0
    }};
}

macro_rules! smros_riscv_syscall_return_pc_body {
    ($sepc:expr) => {{
        match $sepc.checked_add(4) {
            Some(next) => next,
            None => u64::MAX,
        }
    }};
}

/// SSTATUS.SPP (bit 8) is set when the trap interrupted supervisor mode.
macro_rules! smros_riscv_sstatus_spp_set_body {
    ($pstate:expr) => {{
        ($pstate & (1u64 << 8)) != 0
    }};
}
