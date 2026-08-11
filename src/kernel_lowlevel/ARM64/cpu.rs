pub type IrqState = usize;

#[inline(always)]
pub fn mask_interrupts() -> IrqState {
    let daif: usize;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
        let masked = daif | 0x3c0;
        core::arch::asm!(
            "msr daif, {masked}",
            masked = in(reg) masked,
            options(nomem, nostack, preserves_flags),
        );
    }
    daif
}

#[inline(always)]
pub fn restore_interrupts(state: IrqState) {
    unsafe {
        core::arch::asm!(
            "msr daif, {state}",
            state = in(reg) state,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[inline(always)]
pub fn unmask_timer_interrupts() {
    let daif: u64;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
        let daif = daif & !0x80;
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[inline(always)]
pub fn wait_for_interrupt() {
    cortex_a::asm::wfi();
}

#[inline(always)]
pub fn wait_for_event() {
    cortex_a::asm::wfe();
}

#[inline(always)]
pub fn read_cycle_counter() -> u64 {
    let val: u64;
    unsafe {
        core::arch::asm!(
            "mrs {val}, cntpct_el0",
            val = out(reg) val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

#[inline(always)]
pub fn sync_instruction_cache() {
    unsafe {
        core::arch::asm!("dsb ishst", "ic iallu", "dsb ish", "isb", options(nostack));
    }
}

#[inline(always)]
pub fn invalidate_user_page(vaddr: usize) {
    let page = (vaddr as u64) >> 12;
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "tlbi vae1is, {page}",
            "dsb ish",
            "isb",
            page = in(reg) page,
            options(nostack, preserves_flags),
        );
    }
}

#[inline(always)]
pub fn complete_user_page_update() {
    unsafe {
        core::arch::asm!("dsb ishst", "isb", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn install_stage1_translation(root: u64) {
    let mair = 0xffu64 | (0x04u64 << 8);
    let tcr = 25u64 | (1 << 8) | (1 << 10) | (3 << 12) | (1 << 23) | (2u64 << 32);
    core::arch::asm!(
        "msr mair_el1, {mair}",
        "msr tcr_el1, {tcr}",
        "msr ttbr0_el1, {root}",
        "dsb ish",
        "tlbi vmalle1is",
        "dsb ish",
        "isb",
        mair = in(reg) mair,
        tcr = in(reg) tcr,
        root = in(reg) root,
        options(nostack),
    );
    let mut sctlr: u64;
    core::arch::asm!(
        "mrs {sctlr}, sctlr_el1",
        sctlr = out(reg) sctlr,
        options(nomem, nostack, preserves_flags),
    );
    sctlr |= (1 << 0) | (1 << 2) | (1 << 12);
    core::arch::asm!(
        "msr sctlr_el1, {sctlr}",
        "isb",
        sctlr = in(reg) sctlr,
        options(nostack),
    );
}

#[inline(always)]
pub fn mmio_barrier() {
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn set_kernel_resume(resume: u64, state: u64) {
    core::arch::asm!(
        "msr elr_el1, {resume}",
        "msr spsr_el1, {state}",
        resume = in(reg) resume,
        state = in(reg) state,
        options(nostack),
    );
}

#[inline(always)]
pub unsafe fn set_kernel_resume_preserve_flags(resume: u64, state: u64) {
    core::arch::asm!(
        "msr elr_el1, {resume}",
        "msr spsr_el1, {state}",
        resume = in(reg) resume,
        state = in(reg) state,
        options(nostack, preserves_flags),
    );
}

#[inline(always)]
pub fn set_exception_return_pc(pc: u64) {
    unsafe {
        core::arch::asm!(
            "msr elr_el1, {pc}",
            pc = in(reg) pc,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[inline(always)]
pub fn read_exception_return_pc() -> u64 {
    let pc: u64;
    unsafe {
        core::arch::asm!(
            "mrs {pc}, elr_el1",
            pc = out(reg) pc,
            options(nomem, nostack, preserves_flags),
        );
    }
    pc
}

#[inline(always)]
pub fn read_exception_return_state() -> u64 {
    let state: u64;
    unsafe {
        core::arch::asm!(
            "mrs {state}, spsr_el1",
            state = out(reg) state,
            options(nomem, nostack, preserves_flags),
        );
    }
    state
}

#[inline(always)]
pub fn read_user_stack_pointer() -> u64 {
    let sp: u64;
    unsafe {
        core::arch::asm!(
            "mrs {sp}, sp_el0",
            sp = out(reg) sp,
            options(nomem, nostack, preserves_flags),
        );
    }
    sp
}

#[inline(always)]
pub fn set_user_stack_pointer(sp: u64) {
    unsafe {
        core::arch::asm!(
            "msr sp_el0, {sp}",
            sp = in(reg) sp,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[inline(always)]
pub fn read_user_tls() -> u64 {
    let tls: u64;
    unsafe {
        core::arch::asm!(
            "mrs {tls}, tpidr_el0",
            tls = out(reg) tls,
            options(nomem, nostack, preserves_flags),
        );
    }
    tls
}

#[inline(always)]
pub unsafe fn switch_to_user(entry_point: u64, user_stack: u64, ttbr0: u64, state: u64) -> ! {
    let _interrupt_state = mask_interrupts();
    core::arch::asm!(
        "msr ttbr0_el1, {ttbr0}",
        "tlbi vmalle1is",
        "dsb ish",
        "isb",
        ttbr0 = in(reg) ttbr0,
        options(nostack),
    );
    core::arch::asm!("msr sp_el0, {sp}", sp = in(reg) user_stack, options(nostack));
    core::arch::asm!("msr elr_el1, {entry}", entry = in(reg) entry_point, options(nostack));
    core::arch::asm!("msr spsr_el1, {state}", state = in(reg) state, options(nostack));
    core::arch::asm!("eret", options(noreturn));
}

#[inline(always)]
pub unsafe fn linux_syscall(syscall_num: u32, args: [u64; 6]) -> u64 {
    let mut ret = args[0];
    core::arch::asm!(
        "svc #0",
        in("x8") syscall_num,
        inlateout("x0") ret,
        in("x1") args[1],
        in("x2") args[2],
        in("x3") args[3],
        in("x4") args[4],
        in("x5") args[5],
        options(nostack),
    );
    ret
}

pub fn print_system_info(serial: &mut crate::kernel_lowlevel::serial::Serial) {
    use tock_registers::interfaces::Readable;

    let mpidr = cortex_a::registers::MPIDR_EL1.get();
    serial.write_str("[CPU] MPIDR_EL1: 0x");
    serial.write_hex(mpidr);
    serial.write_str("\n");

    let sctlr = cortex_a::registers::SCTLR_EL1.get();
    serial.write_str("[SYS] SCTLR_EL1: 0x");
    serial.write_hex(sctlr);
    serial.write_str("\n");
}
