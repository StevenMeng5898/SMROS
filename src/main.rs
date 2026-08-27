#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::mem::{align_of, size_of};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

mod kernel_lowlevel;
mod kernel_objects;
mod main_logic;
mod syscall;
mod user_level;

use kernel_lowlevel::serial::Serial;
use kernel_lowlevel::smp::{boot_all_cpus, current_cpu_id, print_status as smp_print_status};
use kernel_objects::scheduler::schedule_on_cpu;

/// A Sync wrapper around UnsafeCell that is safe to use as a static.
struct SyncUnsafeCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncUnsafeCell<T> {}
impl<T> SyncUnsafeCell<T> {
    const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

// Global allocator for no_std environment
struct KernelAllocator;

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
    prev: *mut FreeBlock,
}

#[repr(C)]
struct AllocationHeader {
    block_start: usize,
    block_size: usize,
}

struct KernelAllocatorState {
    initialized: bool,
    free_head: *mut FreeBlock,
    largest_free: *mut FreeBlock,
}

struct AllocIrqGuard {
    state: kernel_lowlevel::cpu::IrqState,
}

// 64 MiB heap for kernel dynamic allocations.
static HEAP: SyncUnsafeCell<[u8; main_logic::KERNEL_HEAP_SIZE]> =
    SyncUnsafeCell::new([0; main_logic::KERNEL_HEAP_SIZE]);
static ALLOC_STATE: SyncUnsafeCell<KernelAllocatorState> =
    SyncUnsafeCell::new(KernelAllocatorState {
        initialized: false,
        free_head: core::ptr::null_mut(),
        largest_free: core::ptr::null_mut(),
    });
static ALLOC_LOCK: AtomicBool = AtomicBool::new(false);
static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);
static ALLOC_SCAN_STEPS: AtomicUsize = AtomicUsize::new(0);
const LARGE_ALLOCATION_THRESHOLD: usize = 64 * 1024;

// AArch64 timer interrupts can be observed more than once for the same
// physical counter tick while an IRQ/context switch is in flight.  The
// scheduler and POSIX expiration paths are global, so admit each tick once.
static LAST_ACCEPTED_TIMER_TICK: AtomicU64 = AtomicU64::new(0);

fn claim_timer_tick(now: u64) -> bool {
    if now == 0 {
        return false;
    }
    let mut last = LAST_ACCEPTED_TIMER_TICK.load(Ordering::Acquire);
    loop {
        if !kernel_lowlevel::lowlevel_logic::timer_tick_is_new(last, now) {
            return false;
        }
        match LAST_ACCEPTED_TIMER_TICK.compare_exchange_weak(
            last,
            now,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => last = observed,
        }
    }
}

fn allocator_align_up(value: usize, align: usize) -> Option<usize> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    let mask = align - 1;
    value.checked_add(mask).map(|next| next & !mask)
}

fn allocator_lock() -> AllocIrqGuard {
    let state = kernel_lowlevel::cpu::mask_interrupts();
    while ALLOC_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    AllocIrqGuard { state }
}

impl Drop for AllocIrqGuard {
    fn drop(&mut self) {
        ALLOC_LOCK.store(false, Ordering::Release);
        kernel_lowlevel::cpu::restore_interrupts(self.state);
    }
}

unsafe fn init_kernel_allocator(state: &mut KernelAllocatorState) {
    let heap_start = (*HEAP.get()).as_mut_ptr() as usize;
    let heap_end = heap_start + main_logic::KERNEL_HEAP_SIZE;
    let block_start = match allocator_align_up(heap_start, align_of::<FreeBlock>()) {
        Some(value) => value,
        None => {
            state.initialized = true;
            state.free_head = core::ptr::null_mut();
            state.largest_free = core::ptr::null_mut();
            return;
        }
    };
    if block_start + size_of::<FreeBlock>() > heap_end {
        state.initialized = true;
        state.free_head = core::ptr::null_mut();
        state.largest_free = core::ptr::null_mut();
        return;
    }
    let block = block_start as *mut FreeBlock;
    (*block).size = heap_end - block_start;
    (*block).next = core::ptr::null_mut();
    (*block).prev = core::ptr::null_mut();
    state.initialized = true;
    state.free_head = block;
    state.largest_free = block;
}

unsafe fn replace_free_block(
    state: &mut KernelAllocatorState,
    prev: *mut FreeBlock,
    old: *mut FreeBlock,
    replacement: *mut FreeBlock,
) {
    if prev.is_null() {
        state.free_head = replacement;
    } else {
        (*prev).next = replacement;
    }
    if !replacement.is_null() {
        (*replacement).prev = prev;
    }
    let _ = old;
}

unsafe fn rebuild_largest_free(state: &mut KernelAllocatorState) {
    let mut largest: *mut FreeBlock = core::ptr::null_mut();
    let mut current = state.free_head;
    while !current.is_null() {
        ALLOC_SCAN_STEPS.fetch_add(1, Ordering::Relaxed);
        if largest.is_null() || (*current).size > (*largest).size {
            largest = current;
        }
        current = (*current).next;
    }
    state.largest_free = largest;
}

unsafe fn alloc_from_known_block(
    state: &mut KernelAllocatorState,
    current: *mut FreeBlock,
    layout: Layout,
) -> *mut u8 {
    let request_size = layout.size().max(1);
    let request_align = layout.align().max(align_of::<FreeBlock>());
    let min_free = size_of::<FreeBlock>();
    let header_size = size_of::<AllocationHeader>();
    let block_start = current as usize;
    let block_size = (*current).size;
    let block_end = match block_start.checked_add(block_size) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    let payload_addr = match allocator_align_up(block_start + header_size, request_align) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    let header_addr = payload_addr - header_size;
    let alloc_end = match payload_addr.checked_add(request_size) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    if alloc_end > block_end {
        return core::ptr::null_mut();
    }

    let prev = (*current).prev;
    let next = (*current).next;
    let prefix_size = header_addr - block_start;
    let has_prefix = prefix_size >= min_free;
    let alloc_start = if has_prefix { header_addr } else { block_start };
    let suffix_start = match allocator_align_up(alloc_end, align_of::<FreeBlock>()) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    let suffix_size = block_end - suffix_start;
    let has_suffix = suffix_size >= min_free;
    let alloc_size = if has_suffix {
        suffix_start - alloc_start
    } else {
        block_end - alloc_start
    };
    let was_largest = current == state.largest_free;

    if has_prefix {
        (*current).size = prefix_size;
        if has_suffix {
            let suffix = suffix_start as *mut FreeBlock;
            (*suffix).size = suffix_size;
            (*suffix).prev = current;
            (*suffix).next = next;
            (*current).next = suffix;
            if !next.is_null() {
                (*next).prev = suffix;
            }
            if was_largest {
                state.largest_free = if (*suffix).size > (*current).size {
                    suffix
                } else {
                    current
                };
            }
        } else {
            (*current).next = next;
            if !next.is_null() {
                (*next).prev = current;
            }
            if was_largest {
                rebuild_largest_free(state);
            }
        }
    } else if has_suffix {
        let suffix = suffix_start as *mut FreeBlock;
        (*suffix).size = suffix_size;
        (*suffix).next = next;
        (*suffix).prev = prev;
        replace_free_block(state, prev, current, suffix);
        if !next.is_null() {
            (*next).prev = suffix;
        }
        if was_largest {
            state.largest_free = suffix;
        }
    } else {
        replace_free_block(state, prev, current, next);
        if was_largest {
            rebuild_largest_free(state);
        }
    }

    let header = header_addr as *mut AllocationHeader;
    (*header).block_start = alloc_start;
    (*header).block_size = alloc_size;
    payload_addr as *mut u8
}

unsafe fn alloc_from_free_list(state: &mut KernelAllocatorState, layout: Layout) -> *mut u8 {
    ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    if !state.initialized {
        init_kernel_allocator(state);
    }

    if layout.size() >= LARGE_ALLOCATION_THRESHOLD && !state.largest_free.is_null() {
        let payload = alloc_from_known_block(state, state.largest_free, layout);
        if !payload.is_null() {
            return payload;
        }
    }

    let mut current = state.free_head;
    while !current.is_null() {
        ALLOC_SCAN_STEPS.fetch_add(1, Ordering::Relaxed);
        let payload = alloc_from_known_block(state, current, layout);
        if !payload.is_null() {
            return payload;
        }
        current = (*current).next;
    }
    core::ptr::null_mut()
}

unsafe fn insert_free_block(
    state: &mut KernelAllocatorState,
    block_start: usize,
    block_size: usize,
) {
    if block_size < size_of::<FreeBlock>() {
        return;
    }

    let heap_start = (*HEAP.get()).as_mut_ptr() as usize;
    let heap_end = heap_start + main_logic::KERNEL_HEAP_SIZE;
    let block_end = match block_start.checked_add(block_size) {
        Some(value) => value,
        None => return,
    };
    if block_start < heap_start || block_end > heap_end {
        return;
    }

    let mut prev = core::ptr::null_mut();
    let mut current = state.free_head;
    while !current.is_null() && (current as usize) < block_start {
        prev = current;
        current = (*current).next;
    }

    if !prev.is_null() {
        let prev_end = (prev as usize).saturating_add((*prev).size);
        if prev_end > block_start {
            return;
        }
    }
    if !current.is_null() {
        if block_end > current as usize {
            return;
        }
    }

    let block = block_start as *mut FreeBlock;
    (*block).size = block_size;
    (*block).next = current;
    (*block).prev = prev;

    if prev.is_null() {
        state.free_head = block;
    } else {
        (*prev).next = block;
    }
    if !current.is_null() {
        (*current).prev = block;
    }

    if !current.is_null() && block_start + (*block).size == current as usize {
        (*block).size += (*current).size;
        (*block).next = (*current).next;
        if !(*block).next.is_null() {
            (*(*block).next).prev = block;
        }
    }

    if !prev.is_null() {
        let prev_end = prev as usize + (*prev).size;
        if prev_end == block_start {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
            if !(*prev).next.is_null() {
                (*(*prev).next).prev = prev;
            }
        }
    }

    let result = if !prev.is_null() && prev as usize + (*prev).size >= block_end {
        prev
    } else {
        block
    };
    if state.largest_free.is_null() || (*result).size > (*state.largest_free).size {
        state.largest_free = result;
    }

}

// SAFETY: The heap buffer is exclusively managed behind a global spin lock.
// Freed allocations are returned to a coalescing free list stored inside the
// heap itself, so large temporary Vec buffers do not permanently consume heap.
unsafe impl alloc::alloc::GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = allocator_lock();
        alloc_from_free_list(&mut *ALLOC_STATE.get(), layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let _guard = allocator_lock();
        let header = (ptr as usize - size_of::<AllocationHeader>()) as *const AllocationHeader;
        insert_free_block(
            &mut *ALLOC_STATE.get(),
            (*header).block_start,
            (*header).block_size,
        );
    }
}

/// Kernel version
const KERNEL_VERSION: &str = "1.2.0";

/// Kernel banner
const KERNEL_BANNER: &str = r#"
**************************************************

  SMROS-A Distributed AI-Native Operating System

**************************************************
  v"#;

/// Main kernel entry point
#[no_mangle]
pub extern "C" fn kernel_main(fdt_base: usize) -> ! {
    let _ = kernel_lowlevel::drivers::init_from_fdt(fdt_base);

    // Initialize serial console
    let mut serial = Serial::new();
    serial.init();

    // Print kernel banner
    serial.write_str(KERNEL_BANNER);
    serial.write_str(KERNEL_VERSION);
    serial.write_str("\n\n");

    serial.write_str("[OK] Kernel initialized successfully!\n");
    serial.write_str("[OK] Serial console initialized\n");
    serial.write_str("[OK] ");
    serial.write_str(kernel_lowlevel::drivers::architecture_name());
    serial.write_str(" architecture detected\n");

    kernel_lowlevel::drivers::describe(&mut serial);

    // Print system information
    print_system_info(&mut serial);

    // Initialize interrupt controller
    serial.write_str("[OK] Initializing ");
    serial.write_str(kernel_lowlevel::interrupt::controller_name());
    serial.write_str("... ");
    kernel_lowlevel::interrupt::init();
    serial.write_str("done\n");

    // Initialize timer
    serial.write_str("[OK] Initializing ");
    serial.write_str(kernel_lowlevel::timer::driver_name());
    serial.write_str("... ");
    kernel_lowlevel::timer::init();
    serial.write_str("done\n");

    serial.write_str("[INFO] Timer frequency: ");
    let freq = kernel_lowlevel::timer::get_frequency();
    print_number(&mut serial, (freq / 1000000) as u32);
    serial.write_str(" MHz\n");

    // Initialize SMP support
    kernel_lowlevel::smp::init();

    // Initialize memory management
    serial.write_str("[OK] Initializing memory management... ");
    kernel_lowlevel::memory::init();
    serial.write_str("done\n");

    // Install kernel-owned capability profiles before any user process exists.
    serial.write_str("[OK] Installing kernel object rights config... ");
    crate::kernel_objects::init();
    serial.write_str("done\n");

    // Initialize syscall interface
    serial.write_str("[OK] Initializing syscall interface... ");
    crate::syscall::init();
    serial.write_str("done\n");

    // Initialize MMU
    serial.write_str("[OK] Initializing MMU... ");
    kernel_lowlevel::mmu::init().expect("initialize MMU before continuing boot");
    serial.write_str("done\n");

    // Initialize syscall handler
    serial.write_str("[OK] Initializing syscall handler... ");
    crate::syscall::init();
    serial.write_str("done\n");

    // Initialize channel subsystem
    serial.write_str("[OK] Initializing channel subsystem... ");
    crate::kernel_objects::channel::init();
    serial.write_str("done\n");

    // Initialize user-level process management
    serial.write_str("[OK] Initializing user-level process management... ");
    crate::user_level::init();
    serial.write_str("done\n");

    // Initialize scheduler
    serial.write_str("[OK] Initializing preemptive RR scheduler... ");
    crate::kernel_objects::scheduler::scheduler().init();
    serial.write_str("done\n");

    serial.write_str("[OK] Deferring bootstrap component EL0 launchers until requested\n");

    // Enable timer interrupts
    serial.write_str("[OK] Enabling timer interrupts (100Hz tick)... ");
    kernel_lowlevel::interrupt::enable_timer_interrupt();
    serial.write_str("done\n");

    // Unmask interrupts
    serial.write_str("[OK] Unmasking CPU interrupts... ");
    kernel_lowlevel::cpu::unmask_timer_interrupts();
    serial.write_str("done\n");

    // Boot all secondary CPUs
    serial.write_str("\n--- SMP Multi-Core Initialization ---\n");
    boot_all_cpus();
    smp_print_status();

    serial.write_str(
        "\n[INFO] Fast boot complete. Starting shell; run testsc for syscall validation.\n",
    );
    crate::user_level::user_shell::start_user_shell();
    serial.write_str("[KERNEL] Starting scheduler - jumping to shell thread...\n\n");
    crate::kernel_objects::scheduler::start_first_thread();
}

/// Timer interrupt handler
#[no_mangle]
extern "C" fn timer_interrupt_handler() {
    // Acknowledge the interrupt first so the CPU interface has an active IRQ
    // to complete after the timer is serviced.
    let interrupt_id = kernel_lowlevel::interrupt::acknowledge_interrupt();

    // Clear the timer interrupt
    kernel_lowlevel::timer::clear_interrupt();

    let now = kernel_lowlevel::timer::get_tick_count();
    if current_cpu_id() == 0 {
        crate::syscall::linux_task::on_precision_timer(
            kernel_lowlevel::timer::get_nanoseconds(),
        );
        if claim_timer_tick(now) {
            crate::kernel_objects::scheduler::scheduler().on_timer_tick();
            crate::syscall::expire_linux_real_timers_from_irq();
            crate::syscall::linux_task::on_timer_tick(now);
            crate::syscall::deliver_linux_posix_timer_signals_from_irq();
            crate::syscall::linux_futex::on_timer_tick(now, now);
            crate::syscall::linux_mqueue::on_timer_tick(now);
            crate::kernel_objects::scheduler::scheduler().record_trace_sample(0);
        }
    }

    // End of interrupt
    kernel_lowlevel::interrupt::end_of_interrupt(interrupt_id);
}

/// Check if preemption is needed
#[no_mangle]
extern "C" fn check_preemption() {
    let cpu_id = current_cpu_id();
    if cpu_id != 0 {
        return;
    }
    let s = crate::kernel_objects::scheduler::scheduler();

    if s.should_preempt() {
        // Perform context switch on this CPU
        schedule_on_cpu(cpu_id as usize);
    }
}

/// Print a number to serial
fn print_number(serial: &mut Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 10];
    let mut i = 0;

    while num > 0 && i < 10 {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    // Print in reverse order
    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

/// Print system information
fn print_system_info(serial: &mut Serial) {
    serial.write_str("\n--- System Information ---\n");
    kernel_lowlevel::cpu::print_system_info(serial);
    serial.write_str("--------------------------\n");
}

/// Panic handler
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("\n!!! KERNEL PANIC !!!\n");

    if let Some(location) = info.location() {
        serial.write_str("[PANIC] In file ");
        serial.write_str(location.file());
        serial.write_str(" at line ");
        // Convert line number to string manually
        let mut num = location.line();
        let mut buf = [0u8; 16];
        let mut i = 0;
        if num == 0 {
            buf[i] = b'0';
            i += 1;
        } else {
            let mut temp = [0u8; 16];
            let mut j = 0;
            while num > 0 {
                temp[j] = b'0' + (num % 10) as u8;
                num /= 10;
                j += 1;
            }
            while j > 0 {
                j -= 1;
                buf[i] = temp[j];
                i += 1;
            }
        }
        serial.write_buf(&buf[..i]);
        serial.write_str("\n");
    }

    serial.write_str("\n[ERROR] System halted\n");

    loop {
        kernel_lowlevel::cpu::wait_for_interrupt();
    }
}
