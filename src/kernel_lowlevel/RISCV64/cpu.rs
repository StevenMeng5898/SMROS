pub type IrqState = usize;

use core::sync::atomic::{AtomicBool, Ordering};

const SSTATUS_SIE: usize = 1 << 1;

static USER_ADDRESS_SPACE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub const RISCV_TRAP_STACK_SIZE: usize = 0x10_000;

#[repr(C, align(16))]
pub struct RiscvTrapStack(pub [u8; RISCV_TRAP_STACK_SIZE]);

#[no_mangle]
pub static mut RISCV_TRAP_STACKS: [RiscvTrapStack; crate::kernel_lowlevel::RISCV_MAX_THREADS] =
    [const { RiscvTrapStack([0; RISCV_TRAP_STACK_SIZE]) }; crate::kernel_lowlevel::RISCV_MAX_THREADS];

#[inline(always)]
pub fn trap_stack_top() -> usize {
    trap_stack_top_for_thread(0)
}

#[inline(always)]
pub fn trap_stack_top_for_thread(thread_id: usize) -> usize {
    let index = core::cmp::min(thread_id, crate::kernel_lowlevel::RISCV_MAX_THREADS - 1);
    core::ptr::addr_of!(RISCV_TRAP_STACKS) as usize
        + (index + 1) * RISCV_TRAP_STACK_SIZE
}

#[inline(always)]
pub fn set_trap_stack_for_thread(thread_id: usize) {
    let top = trap_stack_top_for_thread(thread_id);
    unsafe {
        core::arch::asm!("csrw sscratch, {top}", top = in(reg) top, options(nomem, nostack));
    }
}

#[inline(always)]
pub fn user_address_space_active() -> bool {
    USER_ADDRESS_SPACE_ACTIVE.load(Ordering::Acquire)
}

#[inline(always)]
fn set_user_address_space_active(active: bool) {
    USER_ADDRESS_SPACE_ACTIVE.store(active, Ordering::Release);
}

#[inline(always)]
pub fn activate_user_address_space() {
    set_user_address_space_active(true);
}

pub fn deactivate_user_address_space() {
    set_user_address_space_active(false);
    let satp = 0usize;
    unsafe {
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack),
        );
    }
}

#[inline(always)]
pub fn mask_interrupts() -> IrqState {
    let old: usize;
    unsafe {
        core::arch::asm!(
            "csrrc {old}, sstatus, {mask}",
            old = out(reg) old,
            mask = in(reg) SSTATUS_SIE,
            options(nomem, nostack),
        );
    }
    old
}

#[inline(always)]
pub fn restore_interrupts(state: IrqState) {
    unsafe {
        core::arch::asm!(
            "csrw sstatus, {state}",
            state = in(reg) state,
            options(nomem, nostack),
        );
    }
}

#[inline(always)]
pub fn unmask_timer_interrupts() {
    unsafe {
        core::arch::asm!(
            "csrs sstatus, {mask}",
            mask = in(reg) SSTATUS_SIE,
            options(nomem, nostack),
        );
    }
}

#[inline(always)]
pub fn wait_for_interrupt() {
    unsafe {
        core::arch::asm!("wfi", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn flush_tlb() {
    unsafe {
        core::arch::asm!("sfence.vma", options(nomem, nostack));
    }
}

/// Switch the active Sv39 root for the scheduler's next thread. A zero root
/// selects the kernel's bare-address mode used by non-Linux kernel threads.
pub fn switch_user_address_space(root: u64) {
    if root == 0 {
        deactivate_user_address_space();
        return;
    }
    set_user_address_space_active(true);
    let satp = (8usize << 60) | ((root as usize) >> 12);
    unsafe {
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack),
        );
    }
}

#[inline(always)]
pub fn wait_for_event() {
    wait_for_interrupt();
}

#[inline(always)]
pub fn read_cycle_counter() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!("csrr {value}, time", value = out(reg) value, options(nomem, nostack));
    }
    value
}

#[inline(always)]
pub fn sync_instruction_cache() {
    unsafe {
        core::arch::asm!("fence rw, rw", "fence.i", options(nostack));
    }
}

#[inline(always)]
pub fn mmio_barrier() {
    unsafe {
        core::arch::asm!("fence rw, rw", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn set_kernel_resume(resume: u64, _state: u64) {
    // The shared user-logic state value is an ARM SPSR encoding. RISC-V must
    // explicitly request an S-mode sret target and keep interrupts masked.
    const SSTATUS_SPP: u64 = 1 << 8;
    const SSTATUS_SUM: u64 = 1 << 18;
    let kernel_state = SSTATUS_SPP | SSTATUS_SUM;
    if crate::syscall::linux_riscv_syscall_context::set_return_pc(resume) {
        let _ = crate::syscall::linux_riscv_syscall_context::set_return_state(kernel_state);
        let _ = crate::syscall::linux_riscv_syscall_context::set_return_stack_to_trap_top();
    } else {
        core::arch::asm!(
            "csrw sepc, {resume}",
            resume = in(reg) resume,
            options(nostack),
        );
        core::arch::asm!(
            "csrw sstatus, {kernel_state}",
            kernel_state = in(reg) kernel_state,
            options(nostack),
        );
    }
}

#[inline(always)]
pub unsafe fn set_kernel_resume_preserve_flags(resume: u64, state: u64) {
    set_kernel_resume(resume, state);
}

#[inline(always)]
pub fn set_exception_return_pc(pc: u64) {
    if !crate::syscall::linux_riscv_syscall_context::set_return_pc(pc) {
        unsafe {
            core::arch::asm!("csrw sepc, {pc}", pc = in(reg) pc, options(nomem, nostack));
        }
    }
}

#[inline(always)]
pub fn read_exception_return_pc() -> u64 {
    let pc: u64;
    unsafe {
        core::arch::asm!("csrr {pc}, sepc", pc = out(reg) pc, options(nomem, nostack));
    }
    pc
}

#[inline(always)]
pub fn read_exception_return_state() -> u64 {
    if let Some(context) = crate::syscall::linux_riscv_syscall_context::current_riscv_syscall_context()
    {
        return context.pstate;
    }
    let state: usize;
    unsafe {
        core::arch::asm!(
            "csrr {state}, sstatus",
            state = out(reg) state,
            options(nomem, nostack),
        );
    }
    state as u64
}

#[inline(always)]
pub fn read_user_stack_pointer() -> u64 {
    crate::syscall::linux_riscv_syscall_context::current_riscv_syscall_context()
        .map(|context| context.user_sp)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn riscv64_record_unhandled_trap(scause: u64, sepc: u64, stval: u64) {
    crate::kobj_err!(
        "riscv-trap",
        "unhandled scause={:#x} sepc={:#x} stval={:#x}",
        scause,
        sepc,
        stval
    );
}

#[inline(always)]
pub fn set_user_stack_pointer(sp: u64) {
    let _ = crate::syscall::linux_riscv_syscall_context::set_return_stack(sp);
}

#[inline(always)]
pub unsafe fn switch_to_user(entry_point: u64, user_stack: u64, ttbr0: u64, _state: u64) -> ! {
    if ttbr0 != 0 {
        switch_user_address_space(ttbr0);
    }

    const SSTATUS_SPIE: usize = 1 << 5;
    const SSTATUS_SPP: usize = 1 << 8;
    const SSTATUS_SUM: usize = 1 << 18;
    let mut sstatus: usize;
    core::arch::asm!("csrr {sstatus}, sstatus", sstatus = out(reg) sstatus, options(nostack));
    sstatus &= !SSTATUS_SPP;
    // Trap entry and syscall handlers use the current address space for the
    // user stack and buffers. Permit those S-mode accesses while the process
    // is active; user mode itself cannot observe or modify SUM.
    sstatus |= SSTATUS_SPIE | SSTATUS_SUM;
    core::arch::asm!(
        "csrw sstatus, {sstatus}",
        "csrw sepc, {entry}",
        "mv sp, {sp}",
        "sret",
        sstatus = in(reg) sstatus,
        entry = in(reg) entry_point,
        sp = in(reg) user_stack,
        options(noreturn),
    );
}

#[inline(always)]
pub unsafe fn linux_syscall(syscall_num: u32, args: [u64; 6]) -> u64 {
    let mut ret = args[0];
    core::arch::asm!(
        "ecall",
        in("a7") syscall_num as u64,
        inlateout("a0") ret,
        in("a1") args[1],
        in("a2") args[2],
        in("a3") args[3],
        in("a4") args[4],
        in("a5") args[5],
        options(nostack),
    );
    ret
}

pub fn print_system_info(serial: &mut crate::kernel_lowlevel::serial::Serial) {
    let hartid = crate::kernel_lowlevel::smp::read_hartid();
    serial.write_str("[CPU] hartid: ");
    crate::kernel_lowlevel::smp::print_number(serial, hartid as u32);
    serial.write_str("\n");

    let sstatus: usize;
    let sie: usize;
    unsafe {
        core::arch::asm!(
            "csrr {sstatus}, sstatus",
            sstatus = out(reg) sstatus,
            options(nomem, nostack),
        );
        core::arch::asm!("csrr {sie}, sie", sie = out(reg) sie, options(nomem, nostack));
    }
    serial.write_str("[SYS] sstatus: 0x");
    serial.write_hex(sstatus as u64);
    serial.write_str("\n[SYS] sie: 0x");
    serial.write_hex(sie as u64);
    serial.write_str("\n");
}
