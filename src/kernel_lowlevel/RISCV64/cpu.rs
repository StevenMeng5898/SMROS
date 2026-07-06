pub type IrqState = usize;

const SSTATUS_SIE: usize = 1 << 1;

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
    core::arch::asm!(
        "csrw sepc, {resume}",
        resume = in(reg) resume,
        options(nostack),
    );
}

#[inline(always)]
pub unsafe fn set_kernel_resume_preserve_flags(resume: u64, state: u64) {
    set_kernel_resume(resume, state);
}

#[inline(always)]
pub fn set_exception_return_pc(pc: u64) {
    unsafe {
        core::arch::asm!("csrw sepc, {pc}", pc = in(reg) pc, options(nomem, nostack));
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
pub unsafe fn switch_to_user(entry_point: u64, user_stack: u64, ttbr0: u64, _state: u64) -> ! {
    if ttbr0 != 0 {
        let satp = (8usize << 60) | ((ttbr0 as usize) >> 12);
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack),
        );
    }

    const SSTATUS_SPIE: usize = 1 << 5;
    const SSTATUS_SPP: usize = 1 << 8;
    let mut sstatus: usize;
    core::arch::asm!("csrr {sstatus}, sstatus", sstatus = out(reg) sstatus, options(nostack));
    sstatus &= !SSTATUS_SPP;
    sstatus |= SSTATUS_SPIE;
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
    let hartid = crate::kernel_lowlevel::smp::current_cpu_id();
    serial.write_str("[CPU] hartid: ");
    crate::kernel_lowlevel::smp::print_number(serial, hartid);
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
