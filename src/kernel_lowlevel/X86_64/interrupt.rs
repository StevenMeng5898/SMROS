use core::mem::size_of;

const IDT_ENTRIES: usize = 256;
const TIMER_VECTOR: usize = 32;
const IDT_PRESENT_INTERRUPT: u16 = 0x8e00;
const KERNEL_CODE_SELECTOR: u16 = 0x08;
const PIC1_DATA: u16 = 0x21;
const PIC2_DATA: u16 = 0xa1;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }

    fn new(handler: unsafe extern "C" fn()) -> Self {
        let addr = handler as usize as u64;
        Self {
            offset_low: addr as u16,
            selector: KERNEL_CODE_SELECTOR,
            options: IDT_PRESENT_INTERRUPT,
            offset_mid: (addr >> 16) as u16,
            offset_high: (addr >> 32) as u32,
            zero: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtDescriptor {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::missing(); IDT_ENTRIES];

extern "C" {
    fn x86_64_default_interrupt_stub();
    fn x86_64_timer_interrupt_stub();
}

core::arch::global_asm!(
    r#"
.section .text
.globl x86_64_default_interrupt_stub
x86_64_default_interrupt_stub:
    iretq

.globl x86_64_timer_interrupt_stub
x86_64_timer_interrupt_stub:
    push rax
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    call timer_interrupt_handler
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rax
    iretq
"#,
);

pub fn controller_name() -> &'static str {
    "x86_64 IDT/local APIC interrupt controller"
}

pub fn init() {
    unsafe {
        let default = IdtEntry::new(x86_64_default_interrupt_stub);
        let timer = IdtEntry::new(x86_64_timer_interrupt_stub);
        let mut index = 0;
        while index < IDT_ENTRIES {
            IDT[index] = default;
            index += 1;
        }
        IDT[TIMER_VECTOR] = timer;
        let descriptor = IdtDescriptor {
            limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
            base: (&raw const IDT) as *const _ as u64,
        };
        core::arch::asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack));
        outb(PIC1_DATA, 0xff);
        outb(PIC2_DATA, 0xff);
    }
}

pub fn enable_timer_interrupt() {}

pub fn acknowledge_interrupt() -> u32 {
    crate::kernel_lowlevel::timer::interrupt_id()
}

pub fn end_of_interrupt(_interrupt_id: u32) {
    let lapic_base = crate::kernel_lowlevel::drivers::lapic_base();
    if lapic_base != 0 {
        unsafe {
            core::ptr::write_volatile((lapic_base + 0xb0) as *mut u32, 0);
        }
    }
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags),
    );
}
