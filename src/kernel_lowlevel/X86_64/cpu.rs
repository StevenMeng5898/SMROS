pub type IrqState = usize;

#[inline(always)]
pub fn mask_interrupts() -> IrqState {
    let flags = read_rflags();
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
    flags
}

#[inline(always)]
pub fn restore_interrupts(state: IrqState) {
    if state & (1 << 9) != 0 {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
        }
    } else {
        unsafe {
            core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
        }
    }
}

#[inline(always)]
pub fn unmask_timer_interrupts() {
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn wait_for_interrupt() {
    core::hint::spin_loop();
}

#[inline(always)]
pub fn wait_for_event() {
    core::hint::spin_loop();
}

#[inline(always)]
pub fn read_cycle_counter() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[inline(always)]
pub fn sync_instruction_cache() {
    unsafe {
        core::arch::asm!("mfence", "lfence", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn mmio_barrier() {
    unsafe {
        core::arch::asm!("mfence", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn set_kernel_resume(_resume: u64, _state: u64) {}

#[inline(always)]
pub unsafe fn set_kernel_resume_preserve_flags(resume: u64, state: u64) {
    set_kernel_resume(resume, state);
}

#[inline(always)]
pub fn set_exception_return_pc(_pc: u64) {}

#[inline(always)]
pub fn read_exception_return_pc() -> u64 {
    0
}

#[inline(always)]
pub fn read_user_stack_pointer() -> u64 {
    0
}

#[inline(always)]
pub fn set_user_stack_pointer(_sp: u64) {}

#[inline(always)]
pub unsafe fn switch_to_user(_entry_point: u64, _user_stack: u64, _ttbr0: u64, _state: u64) -> ! {
    loop {
        wait_for_event();
    }
}

#[inline(always)]
pub unsafe fn linux_syscall(syscall_num: u32, args: [u64; 6]) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") syscall_num as u64,
        in("rdi") args[0],
        in("rsi") args[1],
        in("rdx") args[2],
        in("r10") args[3],
        in("r8") args[4],
        in("r9") args[5],
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack),
    );
    ret
}

pub fn print_system_info(serial: &mut crate::kernel_lowlevel::serial::Serial) {
    serial.write_str("[CPU] vendor: ");
    serial.write_str(crate::kernel_lowlevel::drivers::cpu_vendor());
    serial.write_str("\n[CPU] model: ");
    serial.write_str(crate::kernel_lowlevel::drivers::cpu_name());
    serial.write_str("\n[SYS] rflags: 0x");
    serial.write_hex(read_rflags() as u64);
    serial.write_str("\n");
}

fn read_rflags() -> usize {
    let flags: usize;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {flags}",
            flags = out(reg) flags,
            options(nomem, preserves_flags),
        );
    }
    flags
}
