//! Experimental ELF launcher for the shell `run` command.
//!
//! This keeps using the current identity-mapped EL0 bring-up model. It maps
//! the executable and interpreter into the Linux mmap window, builds the Linux
//! initial stack, then enters the dynamic loader from a short-lived scheduler
//! thread.

use alloc::alloc::{alloc, Layout};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use crate::kernel_lowlevel::{memory::PAGE_SIZE, timer};
use crate::kernel_objects::scheduler;
use crate::syscall;
use crate::user_level::{elf, fxfs, user_logic, user_process};

const RUN_ELF_MAIN_BASE: usize = 0x5000_0000;
const RUN_ELF_INTERP_BASE: usize = 0x5100_0000;
const RUN_ELF_STACK_SIZE: usize = 0x20_000;
const RUN_ELF_MAP_PROT: usize = 0x1 | 0x2 | 0x4; // PROT_READ | PROT_WRITE | PROT_EXEC
const RUN_ELF_MAP_FIXED_ANON_PRIVATE: usize = (1 << 4) | (1 << 5) | (1 << 1);
const RUN_ELF_TIMER_HZ: u64 = 100;

const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_BASE: u64 = 7;
const AT_FLAGS: u64 = 8;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;
const AT_PLATFORM: u64 = 15;
const AT_HWCAP: u64 = 16;
const AT_CLKTCK: u64 = 17;
const AT_SECURE: u64 = 23;
const AT_RANDOM: u64 = 25;
const AT_HWCAP2: u64 = 26;
const AT_EXECFN: u64 = 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunElfError {
    Busy,
    Storage,
    BadElf,
    Unsupported,
    MissingInterpreter,
    Map,
    Stack,
    Thread,
}

impl RunElfError {
    pub fn as_str(self) -> &'static str {
        match self {
            RunElfError::Busy => "busy",
            RunElfError::Storage => "storage",
            RunElfError::BadElf => "bad-elf",
            RunElfError::Unsupported => "unsupported-elf",
            RunElfError::MissingInterpreter => "missing-interpreter",
            RunElfError::Map => "map",
            RunElfError::Stack => "stack",
            RunElfError::Thread => "thread",
        }
    }
}

struct RunRequest {
    path: String,
    argv: Vec<String>,
}

struct RunSlot<T>(UnsafeCell<T>);

unsafe impl<T> Sync for RunSlot<T> {}

impl<T> RunSlot<T> {
    const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    fn get(&self) -> *mut T {
        self.0.get()
    }
}

static RUN_ACTIVE: AtomicBool = AtomicBool::new(false);
static RUN_EXIT_CODE: AtomicI32 = AtomicI32::new(0);
static RUN_START_TICK: AtomicU64 = AtomicU64::new(0);
static PENDING_RUN: RunSlot<Option<RunRequest>> = RunSlot::new(None);
static ACTIVE_PATH: RunSlot<Option<String>> = RunSlot::new(None);

pub fn spawn(path: String, argv: Vec<String>) -> Result<(), RunElfError> {
    if RUN_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err(RunElfError::Busy);
    }
    syscall::reset_linux_signal_timer_state();

    unsafe {
        let pending = &mut *PENDING_RUN.get();
        if pending.is_some() {
            RUN_ACTIVE.store(false, Ordering::SeqCst);
            return Err(RunElfError::Busy);
        }
        *pending = Some(RunRequest { path, argv });
    }
    RUN_START_TICK.store(timer::get_tick_count(), Ordering::SeqCst);

    scheduler::scheduler()
        .create_thread(run_elf_launcher_entry, "run_elf")
        .map(|_| ())
        .ok_or_else(|| {
            unsafe {
                *PENDING_RUN.get() = None;
            }
            RUN_START_TICK.store(0, Ordering::SeqCst);
            RUN_ACTIVE.store(false, Ordering::SeqCst);
            syscall::reset_linux_signal_timer_state();
            RunElfError::Thread
        })
}

pub fn active_exec_path() -> Option<String> {
    unsafe { (&*ACTIVE_PATH.get()).clone() }
}

pub fn prepare_run_elf_return(exit_code: i32) -> bool {
    if !RUN_ACTIVE.swap(false, Ordering::SeqCst) {
        return false;
    }

    syscall::reset_linux_signal_timer_state();
    RUN_EXIT_CODE.store(exit_code, Ordering::SeqCst);

    let spsr_el1 = user_logic::el1h_spsr_masked();
    unsafe {
        core::arch::asm!(
            "msr elr_el1, {resume}",
            "msr spsr_el1, {spsr}",
            resume = in(reg) run_elf_launcher_resume as *const () as u64,
            spsr = in(reg) spsr_el1,
            options(nostack),
        );
    }
    true
}

extern "C" fn run_elf_launcher_entry() -> ! {
    let request = unsafe { (&mut *PENDING_RUN.get()).take() };
    let Some(request) = request else {
        RUN_ACTIVE.store(false, Ordering::SeqCst);
        finish_launcher_thread();
    };

    unsafe {
        *ACTIVE_PATH.get() = Some(request.path.clone());
    }

    match prepare_dynamic_loader(&request) {
        Ok((entry, stack_top)) => unsafe {
            user_process::switch_to_el0(entry, stack_top, 0);
        },
        Err(err) => {
            RUN_ACTIVE.store(false, Ordering::SeqCst);
            syscall::reset_linux_signal_timer_state();
            unsafe {
                *ACTIVE_PATH.get() = None;
            }
            let mut serial = crate::kernel_lowlevel::serial::Serial::new();
            serial.init();
            serial.write_str("run: ELF launch-failed: ");
            serial.write_str(err.as_str());
            serial.write_str("\n");
            finish_launcher_thread();
        }
    }
}

#[no_mangle]
pub extern "C" fn run_elf_launcher_resume() -> ! {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    let exit_code = RUN_EXIT_CODE.load(Ordering::SeqCst);
    let start_tick = RUN_START_TICK.load(Ordering::SeqCst);
    let elapsed_ticks = timer::get_tick_count().saturating_sub(start_tick);
    let active_path = unsafe { (&*ACTIVE_PATH.get()).clone() };

    serial.write_str("\nrun: program output ended\n");
    serial.write_str("run: process finished\n");
    serial.write_str("  path: ");
    match active_path.as_ref() {
        Some(path) => serial.write_str(path.as_str()),
        None => serial.write_str("<unknown>"),
    }
    serial.write_str("\n  exit code: ");
    print_i32(&mut serial, exit_code);
    if exit_code == 0 {
        serial.write_str(" (success)");
    } else {
        serial.write_str(" (failure)");
    }
    serial.write_str("\n  elapsed: ");
    print_elapsed_ticks(&mut serial, elapsed_ticks);
    serial.write_str(" (");
    print_u64(&mut serial, elapsed_ticks);
    serial.write_str(" timer ticks)");
    serial.write_str("\n");

    RUN_START_TICK.store(0, Ordering::SeqCst);
    unsafe {
        *ACTIVE_PATH.get() = None;
    }
    finish_launcher_thread();
}

fn finish_launcher_thread() -> ! {
    scheduler::scheduler().finish_current_without_stack_free();
    scheduler::schedule();
    loop {
        cortex_a::asm::wfe();
    }
}

fn prepare_dynamic_loader(request: &RunRequest) -> Result<(u64, u64), RunElfError> {
    let main_bytes = read_fxfs_file(request.path.as_str())?;
    let main = elf::parse(&main_bytes).map_err(|_| RunElfError::BadElf)?;
    if main.elf_type != elf::ELF_TYPE_DYN {
        return Err(RunElfError::Unsupported);
    }

    let interpreter = main
        .interpreter
        .as_ref()
        .ok_or(RunElfError::MissingInterpreter)?;
    let interp_path =
        resolve_library_path(interpreter.as_str()).ok_or(RunElfError::MissingInterpreter)?;
    let interp_bytes = read_fxfs_file(interp_path.as_str())?;
    let interp = elf::parse(&interp_bytes).map_err(|_| RunElfError::BadElf)?;
    if interp.elf_type != elf::ELF_TYPE_DYN {
        return Err(RunElfError::Unsupported);
    }

    map_elf_image(&main, &main_bytes, RUN_ELF_MAIN_BASE)?;
    map_elf_image(&interp, &interp_bytes, RUN_ELF_INTERP_BASE)?;
    sync_instruction_cache();

    let stack_top = build_initial_stack(request, &main)?;
    Ok((
        (RUN_ELF_INTERP_BASE as u64).saturating_add(interp.entry),
        stack_top,
    ))
}

fn read_fxfs_file(path: &str) -> Result<Vec<u8>, RunElfError> {
    let attrs = fxfs::attrs(path).map_err(|_| RunElfError::Storage)?;
    let mut out = Vec::new();
    out.resize(attrs.size, 0);
    let size = fxfs::read_file(path, &mut out).map_err(|_| RunElfError::Storage)?;
    out.truncate(size);
    Ok(out)
}

fn resolve_library_path(name_or_path: &str) -> Option<String> {
    if name_or_path.starts_with('/') && fxfs::attrs(name_or_path).is_ok() {
        return Some(String::from(name_or_path));
    }

    let name = name_or_path.rsplit('/').next().unwrap_or(name_or_path);
    let mut shared = String::from("/shared/lib/");
    shared.push_str(name);
    if fxfs::attrs(shared.as_str()).is_ok() {
        return Some(shared);
    }

    let mut lib = String::from("/lib/");
    lib.push_str(name);
    if fxfs::attrs(lib.as_str()).is_ok() {
        return Some(lib);
    }

    None
}

fn map_elf_image(image: &elf::ElfImage, bytes: &[u8], base: usize) -> Result<(), RunElfError> {
    let (start, len) = elf_mapping_span(image).ok_or(RunElfError::Map)?;
    let map_addr = base.checked_add(start).ok_or(RunElfError::Map)?;
    let mapped = syscall::sys_mmap(
        map_addr,
        len,
        RUN_ELF_MAP_PROT,
        RUN_ELF_MAP_FIXED_ANON_PRIVATE,
        0,
        0,
    )
    .map_err(|_| RunElfError::Map)?;
    if mapped != map_addr {
        return Err(RunElfError::Map);
    }

    for segment in &image.segments {
        let dest = base
            .checked_add(segment.vaddr as usize)
            .ok_or(RunElfError::Map)?;
        unsafe {
            core::ptr::write_bytes(dest as *mut u8, 0, segment.mem_size as usize);
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr().add(segment.file_offset as usize),
                dest as *mut u8,
                segment.file_size as usize,
            );
        }
    }

    Ok(())
}

fn elf_mapping_span(image: &elf::ElfImage) -> Option<(usize, usize)> {
    let mut min_addr = usize::MAX;
    let mut max_addr = 0usize;

    for segment in &image.segments {
        if segment.mem_size == 0 || segment.vaddr > usize::MAX as u64 {
            continue;
        }
        let vaddr = segment.vaddr as usize;
        let mem_size = usize::try_from(segment.mem_size).ok()?;
        let (start, end) = user_logic::elf_segment_mapping_range(vaddr, mem_size, PAGE_SIZE)?;
        min_addr = core::cmp::min(min_addr, start);
        max_addr = core::cmp::max(max_addr, end);
    }

    if min_addr == usize::MAX || max_addr <= min_addr {
        return None;
    }
    Some((min_addr, max_addr - min_addr))
}

fn sync_instruction_cache() {
    unsafe {
        core::arch::asm!("dsb ishst", "ic iallu", "dsb ish", "isb", options(nostack),);
    }
}

struct StackBuilder {
    base: usize,
    sp: usize,
}

impl StackBuilder {
    fn new(size: usize) -> Result<Self, RunElfError> {
        let layout = Layout::from_size_align(size, 16).map_err(|_| RunElfError::Stack)?;
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err(RunElfError::Stack);
        }
        let base = ptr as usize;
        Ok(Self {
            base,
            sp: base.checked_add(size).ok_or(RunElfError::Stack)?,
        })
    }

    fn align_down(&mut self, align: usize) {
        self.sp &= !(align - 1);
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> Result<usize, RunElfError> {
        self.sp = self.sp.checked_sub(bytes.len()).ok_or(RunElfError::Stack)?;
        if self.sp < self.base {
            return Err(RunElfError::Stack);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.sp as *mut u8, bytes.len());
        }
        Ok(self.sp)
    }

    fn push_cstr(&mut self, value: &str) -> Result<usize, RunElfError> {
        self.sp = self.sp.checked_sub(1).ok_or(RunElfError::Stack)?;
        if self.sp < self.base {
            return Err(RunElfError::Stack);
        }
        unsafe {
            core::ptr::write(self.sp as *mut u8, 0);
        }
        self.push_bytes(value.as_bytes())
    }

    fn push_u64(&mut self, value: u64) -> Result<(), RunElfError> {
        self.sp = self.sp.checked_sub(8).ok_or(RunElfError::Stack)?;
        if self.sp < self.base {
            return Err(RunElfError::Stack);
        }
        unsafe {
            core::ptr::write_unaligned(self.sp as *mut u64, value);
        }
        Ok(())
    }
}

fn build_initial_stack(request: &RunRequest, main: &elf::ElfImage) -> Result<u64, RunElfError> {
    let mut stack = StackBuilder::new(RUN_ELF_STACK_SIZE)?;

    let random_ptr = stack.push_bytes(&[
        0x41, 0x52, 0x4d, 0x36, 0x34, 0x2d, 0x53, 0x4d, 0x52, 0x4f, 0x53, 0x2d, 0x45, 0x4c, 0x46,
        0x21,
    ])?;
    let platform_ptr = stack.push_cstr("aarch64")?;
    let env_ld_path = stack.push_cstr("LD_LIBRARY_PATH=/shared/lib:/lib")?;

    let mut argv_ptrs = Vec::new();
    for arg in request.argv.iter().rev() {
        argv_ptrs.push(stack.push_cstr(arg.as_str())? as u64);
    }
    argv_ptrs.reverse();
    if argv_ptrs.is_empty() {
        argv_ptrs.push(stack.push_cstr(request.path.as_str())? as u64);
    }

    let env_ptrs = [env_ld_path as u64];
    let auxv = [
        (
            AT_PHDR,
            (RUN_ELF_MAIN_BASE as u64).saturating_add(main.phoff),
        ),
        (AT_PHENT, main.phentsize as u64),
        (AT_PHNUM, main.phnum as u64),
        (AT_PAGESZ, PAGE_SIZE as u64),
        (AT_BASE, RUN_ELF_INTERP_BASE as u64),
        (AT_FLAGS, 0),
        (
            AT_ENTRY,
            (RUN_ELF_MAIN_BASE as u64).saturating_add(main.entry),
        ),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_PLATFORM, platform_ptr as u64),
        (AT_HWCAP, 0),
        (AT_CLKTCK, 100),
        (AT_SECURE, 0),
        (AT_RANDOM, random_ptr as u64),
        (AT_HWCAP2, 0),
        (AT_EXECFN, argv_ptrs[0]),
        (AT_NULL, 0),
    ];

    stack.align_down(16);
    let table_words = 1 + argv_ptrs.len() + 1 + env_ptrs.len() + 1 + auxv.len() * 2;
    if (stack.sp - table_words * 8) & 0xf != 0 {
        stack.push_u64(0)?;
    }

    for (key, value) in auxv.iter().rev() {
        stack.push_u64(*value)?;
        stack.push_u64(*key)?;
    }

    stack.push_u64(0)?;
    for ptr in env_ptrs.iter().rev() {
        stack.push_u64(*ptr)?;
    }

    stack.push_u64(0)?;
    for ptr in argv_ptrs.iter().rev() {
        stack.push_u64(*ptr)?;
    }
    stack.push_u64(argv_ptrs.len() as u64)?;

    if stack.sp & 0xf != 0 {
        return Err(RunElfError::Stack);
    }
    Ok(stack.sp as u64)
}

fn print_i32(serial: &mut crate::kernel_lowlevel::serial::Serial, value: i32) {
    if value < 0 {
        serial.write_byte(b'-');
        print_u64(serial, value.wrapping_neg() as u32 as u64);
    } else {
        print_u64(serial, value as u64);
    }
}

fn print_elapsed_ticks(serial: &mut crate::kernel_lowlevel::serial::Serial, ticks: u64) {
    let seconds = ticks / RUN_ELF_TIMER_HZ;
    let centiseconds = ticks % RUN_ELF_TIMER_HZ;
    print_u64(serial, seconds);
    serial.write_byte(b'.');
    if centiseconds < 10 {
        serial.write_byte(b'0');
    }
    print_u64(serial, centiseconds);
    serial.write_byte(b's');
}

fn print_u64(serial: &mut crate::kernel_lowlevel::serial::Serial, mut value: u64) {
    if value == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while value > 0 && len < buf.len() {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        serial.write_byte(buf[len]);
    }
}
