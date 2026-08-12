//! Experimental ELF launcher for the shell `run` command.
//!
//! This keeps using the current identity-mapped EL0 bring-up model. It maps
//! the executable and interpreter into the Linux mmap window, builds the Linux
//! initial stack, then enters the dynamic loader from a short-lived scheduler
//! thread.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::kernel_lowlevel::{memory::PAGE_SIZE, timer};
use crate::kernel_objects::scheduler;
use crate::syscall;
use crate::user_level::{elf, fxfs, user_logic, user_process};

use super::posix_test;

const RUN_ELF_MAIN_BASE: usize = syscall::linux_process_memory::LINUX_MAIN_BASE;
const RUN_ELF_INTERP_BASE: usize = syscall::linux_process_memory::LINUX_INTERPRETER_BASE;
const RUN_ELF_STACK_SIZE: usize = 0x20_000;
const RUN_ELF_STAGING_PROT: usize = 0x1 | 0x2; // PROT_READ | PROT_WRITE
const RUN_ELF_MAP_FIXED_ANON_PRIVATE: usize = (1 << 4) | (1 << 5) | (1 << 1);
const RUN_ELF_TIMER_HZ: u64 = 100;
const RUN_ELF_MAX_ENV_ENTRIES: usize = 64;
const RUN_ELF_MAX_ENV_ENTRY_BYTES: usize = 4 * 1024;
const RUN_ELF_MAX_ENV_TOTAL_BYTES: usize = 32 * 1024;
const LINUX_RUNTIME_CPU: usize = 0;
const RUN_ELF_LD_LIBRARY_PATH_KEY: &str = "LD_LIBRARY_PATH";
const RUN_ELF_DEFAULT_LD_LIBRARY_PATH: &str =
    "LD_LIBRARY_PATH=/shared/posixtest/lib:/shared/lib:/lib";

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
    InvalidEnvironment,
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
            RunElfError::InvalidEnvironment => "invalid-environment",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunObserver {
    Shell,
    PosixTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunInfrastructureError {
    MissingRequest,
}

impl RunInfrastructureError {
    pub fn as_str(self) -> &'static str {
        match self {
            RunInfrastructureError::MissingRequest => "missing-request",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunTermination {
    Exit(i32),
    LaunchError(RunElfError),
    InfrastructureError(RunInfrastructureError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    pub path: String,
    pub termination: RunTermination,
    pub elapsed_ticks: u64,
}

#[derive(Clone)]
struct RunLaunchInputs {
    path: String,
    argv: Vec<String>,
    env: Vec<String>,
    observer: RunObserver,
    start_tick: u64,
}

type ActiveRun = user_logic::RunElfActiveRequest<RunLaunchInputs, fxfs::FxfsPersistGuard>;
type RunState = user_logic::RunElfLifecycleState<ActiveRun>;

static RUN_STATE: user_logic::RunElfStateCell<RunState> =
    user_logic::RunElfStateCell::new(RunState::new());
static RUN_CPU_BINDINGS: user_logic::RunElfCpuBindings<{ scheduler::MAX_CPUS }> =
    user_logic::RunElfCpuBindings::new();

fn with_run_state<R>(operation: impl FnOnce(&mut RunState) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut state = RUN_STATE.lock();
    let result = operation(&mut state);
    drop(state);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub fn spawn(path: String, argv: Vec<String>) -> Result<(), RunElfError> {
    spawn_observed(path, argv, Vec::new(), RunObserver::Shell)
}

pub fn spawn_observed(
    path: String,
    argv: Vec<String>,
    env: Vec<String>,
    observer: RunObserver,
) -> Result<(), RunElfError> {
    if validate_environment(&env).is_err() {
        return Err(RunElfError::InvalidEnvironment);
    }

    let request = user_logic::RunElfActiveRequest::new(RunLaunchInputs {
        path,
        argv,
        env,
        observer,
        start_tick: timer::get_tick_count(),
    });
    let launch_id = match with_run_state(|state| {
        user_logic::run_elf_start_transition(state, request, || {
            syscall::reset_linux_process_state()
        })
    }) {
        user_logic::RunElfStart::Started(launch_id) => launch_id,
        user_logic::RunElfStart::Busy(_) | user_logic::RunElfStart::Exhausted(_) => {
            return Err(RunElfError::Busy);
        }
    };

    let persist_guard = fxfs::suspend_persist();
    if let Err(error) = with_run_state(|state| {
        user_logic::run_elf_attach_resource_transition(state, launch_id, persist_guard)
    }) {
        drop(error.into_resource());
        clear_launch_state_without_outcome(LINUX_RUNTIME_CPU, launch_id);
        return Err(RunElfError::Thread);
    }

    let cpu = LINUX_RUNTIME_CPU;
    if RUN_CPU_BINDINGS.bind(cpu, launch_id).is_err() {
        clear_launch_state_without_outcome(cpu, launch_id);
        return Err(RunElfError::Thread);
    }

    scheduler::scheduler()
        .create_thread_on_cpu(run_elf_launcher_entry, "run_elf", Some(cpu))
        .map(|_| ())
        .ok_or_else(|| {
            clear_launch_state_without_outcome(cpu, launch_id);
            RunElfError::Thread
        })
}

pub fn active_exec_path() -> Option<String> {
    with_run_state(|state| state.request().map(|request| request.launch().path.clone()))
}

pub fn prepare_run_elf_return(exit_code: i32) -> Option<usize> {
    let cpu = crate::kernel_lowlevel::smp::current_cpu_id() as usize;
    let launch_id = RUN_CPU_BINDINGS.get(cpu)?;
    let id_raw = launch_id.to_usize()?;
    if with_run_state(|state| {
        user_logic::run_elf_prepare_return_transition(state, launch_id, exit_code, || {
            syscall::reset_linux_process_state()
        })
    }) != user_logic::RunElfTransition::Matched
    {
        return None;
    }

    let spsr_el1 = user_logic::el1h_spsr_masked();
    unsafe {
        crate::kernel_lowlevel::cpu::set_kernel_resume(
            run_elf_launcher_resume as *const () as u64,
            spsr_el1,
        );
    }
    Some(id_raw)
}

extern "C" fn run_elf_launcher_entry() -> ! {
    let cpu = crate::kernel_lowlevel::smp::current_cpu_id() as usize;
    let Some(launch_id) = RUN_CPU_BINDINGS.get(cpu) else {
        print_infrastructure_diagnostic(RunInfrastructureError::MissingRequest);
        finish_launcher_thread();
    };
    let request = with_run_state(|state| {
        state
            .request_for(launch_id)
            .map(|request| request.launch().clone())
    });
    let Some(request) = request else {
        complete_active_run(cpu, launch_id, |_| {
            RunTermination::InfrastructureError(RunInfrastructureError::MissingRequest)
        });
        finish_launcher_thread();
    };
    let scheduler_thread = scheduler::scheduler().current();
    let pid = match syscall::linux_task::register_root(scheduler_thread) {
        Ok(pid) => pid,
        Err(_) => {
            complete_active_run(cpu, launch_id, |_| {
                RunTermination::LaunchError(RunElfError::Thread)
            });
            finish_launcher_thread();
        }
    };
    if syscall::linux_process_memory::register_root(pid).is_err() {
        complete_active_run(cpu, launch_id, |_| {
            RunTermination::LaunchError(RunElfError::Map)
        });
        finish_launcher_thread();
    }
    let preparation = prepare_dynamic_loader(&request);

    match preparation {
        Ok(prepared) => {
            let entry = prepared.entry;
            let stack_top = prepared.stack_top;
            let root_paddr = syscall::linux_process_memory::current_root_paddr()
                .unwrap_or_else(|_| finish_launcher_thread());
            unsafe {
                user_process::switch_to_el0(entry, stack_top, root_paddr);
            }
        }
        Err(err) => {
            complete_active_run(cpu, launch_id, |_| RunTermination::LaunchError(err));
            finish_launcher_thread();
        }
    }
}

#[no_mangle]
pub extern "C" fn run_elf_launcher_resume(id_raw: usize) -> ! {
    let Some(launch_id) = user_logic::RunElfLaunchId::from_usize(id_raw) else {
        print_infrastructure_diagnostic(RunInfrastructureError::MissingRequest);
        finish_launcher_thread();
    };
    let cpu = crate::kernel_lowlevel::smp::current_cpu_id() as usize;
    complete_active_run(cpu, launch_id, RunTermination::Exit);
    finish_launcher_thread();
}

fn validate_environment(env: &[String]) -> Result<(), RunElfError> {
    if !user_logic::run_elf_environment_valid(
        env,
        RUN_ELF_LD_LIBRARY_PATH_KEY,
        RUN_ELF_DEFAULT_LD_LIBRARY_PATH.len().saturating_add(1),
        RUN_ELF_MAX_ENV_ENTRIES,
        RUN_ELF_MAX_ENV_ENTRY_BYTES,
        RUN_ELF_MAX_ENV_TOTAL_BYTES,
    ) {
        return Err(RunElfError::InvalidEnvironment);
    }
    Ok(())
}

fn take_active_request(
    launch_id: user_logic::RunElfLaunchId,
) -> (user_logic::RunElfCompletion<ActiveRun>, i32) {
    let taken = with_run_state(|state| {
        user_logic::run_elf_take_completion_transition(state, launch_id, || {
            syscall::reset_linux_process_state()
        })
    });
    (taken.completion, taken.exit_code)
}

fn clear_launch_state_without_outcome(cpu: usize, launch_id: user_logic::RunElfLaunchId) {
    let _ = RUN_CPU_BINDINGS.clear(cpu, launch_id);
    let completion = with_run_state(|state| {
        user_logic::run_elf_clear_transition(state, launch_id, || {
            syscall::reset_linux_process_state()
        })
    });
    drop(completion);
}

fn complete_active_run(
    cpu: usize,
    launch_id: user_logic::RunElfLaunchId,
    termination: impl FnOnce(i32) -> RunTermination,
) {
    if !RUN_CPU_BINDINGS.clear(cpu, launch_id) {
        return;
    }
    let (completion, exit_code) = take_active_request(launch_id);
    let active_request = match completion {
        user_logic::RunElfCompletion::Requested(request) => request,
        user_logic::RunElfCompletion::Repeated | user_logic::RunElfCompletion::Stale => return,
        user_logic::RunElfCompletion::MissingRequest => {
            print_infrastructure_diagnostic(RunInfrastructureError::MissingRequest);
            return;
        }
    };
    let (request, resource) = active_request.into_parts();
    drop(resource);
    let end_tick = timer::get_tick_count();
    let outcome = RunOutcome {
        path: request.path,
        termination: termination(exit_code),
        elapsed_ticks: user_logic::run_elf_elapsed_ticks(request.start_tick, end_tick),
    };
    dispatch_outcome(request.observer, outcome);
}

fn dispatch_outcome(observer: RunObserver, outcome: RunOutcome) {
    match observer {
        RunObserver::Shell => print_shell_outcome(&outcome),
        RunObserver::PosixTest => posix_test::on_run_outcome(outcome),
    }
}

fn print_infrastructure_diagnostic(error: RunInfrastructureError) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("run: infrastructure-failure: ");
    serial.write_str(error.as_str());
    serial.write_str("\n");
}

fn print_shell_outcome(outcome: &RunOutcome) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    match outcome.termination {
        RunTermination::InfrastructureError(error) => {
            serial.write_str("run: infrastructure-failure: ");
            serial.write_str(error.as_str());
            serial.write_str(" path=");
            serial.write_str(outcome.path.as_str());
            serial.write_str("\n");
        }
        RunTermination::LaunchError(err) => {
            serial.write_str("run: ELF launch-failed: ");
            serial.write_str(err.as_str());
            serial.write_str("\n");
        }
        RunTermination::Exit(exit_code) => {
            serial.write_str("\nrun: program output ended\n");
            serial.write_str("run: process finished\n");
            serial.write_str("  path: ");
            serial.write_str(outcome.path.as_str());
            serial.write_str("\n  exit code: ");
            print_i32(&mut serial, exit_code);
            if user_logic::run_elf_exit_succeeded(exit_code) {
                serial.write_str(" (success)");
            } else {
                serial.write_str(" (failure)");
            }
            serial.write_str("\n  elapsed: ");
            print_elapsed_ticks(&mut serial, outcome.elapsed_ticks);
            serial.write_str(" (");
            print_u64(&mut serial, outcome.elapsed_ticks);
            serial.write_str(" timer ticks)\n");
        }
    }
}

fn finish_launcher_thread() -> ! {
    scheduler::scheduler().finish_current_without_stack_free();
    scheduler::schedule();
    loop {
        crate::kernel_lowlevel::cpu::wait_for_event();
    }
}

struct PreparedRun {
    entry: u64,
    stack_top: u64,
}

fn prepare_dynamic_loader(request: &RunLaunchInputs) -> Result<PreparedRun, RunElfError> {
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

    let stack_base = syscall::linux_process_memory::LINUX_STACK_TOP
        .checked_sub(RUN_ELF_STACK_SIZE)
        .ok_or(RunElfError::Stack)?;
    let mapped_stack = syscall::sys_mmap(
        stack_base,
        RUN_ELF_STACK_SIZE,
        0x1 | 0x2,
        RUN_ELF_MAP_FIXED_ANON_PRIVATE,
        0,
        0,
    )
    .map_err(|_| RunElfError::Stack)?;
    if mapped_stack != stack_base
        || !syscall::register_linux_initial_stack(stack_base, RUN_ELF_STACK_SIZE)
    {
        return Err(RunElfError::Stack);
    }
    let stack = build_initial_stack(request, &main, stack_base)?;
    syscall::linux_process_memory::copy_to_current(stack.base, &stack.bytes)
        .map_err(|_| RunElfError::Stack)?;
    Ok(PreparedRun {
        entry: (RUN_ELF_INTERP_BASE as u64).saturating_add(interp.entry),
        stack_top: stack.stack_top(),
    })
}

fn read_fxfs_file(path: &str) -> Result<Vec<u8>, RunElfError> {
    let attrs = match fxfs::attrs(path) {
        Ok(attrs) => attrs,
        Err(_) if path_under_shared(path) => {
            let _ = fxfs::ensure_host_share();
            fxfs::attrs(path).map_err(|_| RunElfError::Storage)?
        }
        Err(_) => return Err(RunElfError::Storage),
    };
    let mut out = Vec::new();
    out.resize(attrs.size, 0);
    let size = fxfs::read_file(path, &mut out).map_err(|_| RunElfError::Storage)?;
    out.truncate(size);
    Ok(out)
}

fn resolve_library_path(name_or_path: &str) -> Option<String> {
    if !user_logic::run_elf_library_name_valid(name_or_path) {
        return None;
    }

    let name = name_or_path.rsplit('/').next().unwrap_or(name_or_path);
    let _ = fxfs::ensure_host_share();
    let mut stage_index = 0usize;
    while let Some(stage) = user_logic::run_elf_library_search_stage(stage_index) {
        match stage {
            user_logic::RunElfLibrarySearchStage::Posix
            | user_logic::RunElfLibrarySearchStage::Shared
            | user_logic::RunElfLibrarySearchStage::System => {
                let prefix = match stage {
                    user_logic::RunElfLibrarySearchStage::Posix => "/shared/posixtest/lib/",
                    user_logic::RunElfLibrarySearchStage::Shared => "/shared/lib/",
                    user_logic::RunElfLibrarySearchStage::System => "/lib/",
                    user_logic::RunElfLibrarySearchStage::Direct => unreachable!(),
                };
                let mut candidate = String::from(prefix);
                candidate.push_str(name);
                if fxfs::attrs(candidate.as_str()).is_ok() {
                    return Some(candidate);
                }
            }
            user_logic::RunElfLibrarySearchStage::Direct => {
                if (name_or_path.starts_with('/') || path_under_shared(name_or_path))
                    && fxfs::attrs(name_or_path).is_ok()
                {
                    return Some(String::from(name_or_path));
                }
            }
        }
        stage_index += 1;
    }

    None
}

fn path_under_shared(path: &str) -> bool {
    path == "/shared"
        || path
            .strip_prefix("/shared")
            .map(|suffix| suffix.starts_with('/'))
            .unwrap_or(false)
}

fn map_elf_image(image: &elf::ElfImage, bytes: &[u8], base: usize) -> Result<(), RunElfError> {
    let page_runs = map_elf_page_runs(image, base)?;

    for segment in &image.segments {
        let dest = base
            .checked_add(segment.vaddr as usize)
            .ok_or(RunElfError::Map)?;
        let mem_size = usize::try_from(segment.mem_size).map_err(|_| RunElfError::Map)?;
        let file_size = usize::try_from(segment.file_size).map_err(|_| RunElfError::Map)?;
        let file_offset = usize::try_from(segment.file_offset).map_err(|_| RunElfError::Map)?;
        let file_end = file_offset
            .checked_add(file_size)
            .filter(|end| *end <= bytes.len())
            .ok_or(RunElfError::Map)?;
        syscall::linux_process_memory::zero_current(dest, mem_size)
            .map_err(|_| RunElfError::Map)?;
        syscall::linux_process_memory::copy_to_current(dest, &bytes[file_offset..file_end])
            .map_err(|_| RunElfError::Map)?;
    }

    for run in page_runs {
        let address = base.checked_add(run.start).ok_or(RunElfError::Map)?;
        syscall::sys_mprotect(address, run.len, run.prot).map_err(|_| RunElfError::Map)?;
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct ElfPageRun {
    start: usize,
    len: usize,
    prot: usize,
}

fn map_elf_page_runs(image: &elf::ElfImage, base: usize) -> Result<Vec<ElfPageRun>, RunElfError> {
    let mut segments = Vec::new();
    let mut min_addr = usize::MAX;
    let mut max_addr = 0usize;

    for segment in &image.segments {
        if segment.mem_size == 0 {
            continue;
        }
        let vaddr = usize::try_from(segment.vaddr).map_err(|_| RunElfError::Map)?;
        let mem_size = usize::try_from(segment.mem_size).map_err(|_| RunElfError::Map)?;
        let (start, end) = user_logic::elf_segment_mapping_range(vaddr, mem_size, PAGE_SIZE)
            .ok_or(RunElfError::Map)?;
        let mut prot = 0usize;
        if segment.flags & 4 != 0 {
            prot |= 0x1;
        }
        if segment.flags & 2 != 0 {
            prot |= 0x2;
        }
        if segment.flags & 1 != 0 {
            prot |= 0x4;
        }
        segments.push((vaddr, mem_size, prot));
        min_addr = core::cmp::min(min_addr, start);
        max_addr = core::cmp::max(max_addr, end);
    }

    if min_addr == usize::MAX || max_addr <= min_addr {
        return Err(RunElfError::Map);
    }

    let mut runs: Vec<ElfPageRun> = Vec::new();
    let mut page = min_addr;
    while page < max_addr {
        let prot = user_logic::run_elf_page_protection(page, PAGE_SIZE, &segments);
        if let Some(prot) = prot {
            if let Some(previous) = runs.last_mut() {
                if previous.prot == prot && previous.start.checked_add(previous.len) == Some(page) {
                    previous.len = previous
                        .len
                        .checked_add(PAGE_SIZE)
                        .ok_or(RunElfError::Map)?;
                    page = page.checked_add(PAGE_SIZE).ok_or(RunElfError::Map)?;
                    continue;
                }
            }
            runs.push(ElfPageRun {
                start: page,
                len: PAGE_SIZE,
                prot,
            });
        }
        page = page.checked_add(PAGE_SIZE).ok_or(RunElfError::Map)?;
    }

    for run in &runs {
        let address = base.checked_add(run.start).ok_or(RunElfError::Map)?;
        let mapped = syscall::sys_mmap(
            address,
            run.len,
            RUN_ELF_STAGING_PROT,
            RUN_ELF_MAP_FIXED_ANON_PRIVATE,
            0,
            0,
        )
        .map_err(|_| RunElfError::Map)?;
        if mapped != address {
            return Err(RunElfError::Map);
        }
    }
    Ok(runs)
}

fn sync_instruction_cache() {
    crate::kernel_lowlevel::cpu::sync_instruction_cache();
}

struct StackBuilder {
    bytes: Vec<u8>,
    base: usize,
    sp: usize,
}

impl StackBuilder {
    fn new(base: usize, size: usize) -> Result<Self, RunElfError> {
        Ok(Self {
            bytes: vec![0; size],
            base,
            sp: base.checked_add(size).ok_or(RunElfError::Stack)?,
        })
    }

    fn stack_top(&self) -> u64 {
        self.sp as u64
    }

    fn align_down(&mut self, align: usize) {
        self.sp &= !(align - 1);
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> Result<usize, RunElfError> {
        self.sp = self.sp.checked_sub(bytes.len()).ok_or(RunElfError::Stack)?;
        if self.sp < self.base {
            return Err(RunElfError::Stack);
        }
        let offset = self.sp.checked_sub(self.base).ok_or(RunElfError::Stack)?;
        let end = offset
            .checked_add(bytes.len())
            .filter(|end| *end <= self.bytes.len())
            .ok_or(RunElfError::Stack)?;
        self.bytes[offset..end].copy_from_slice(bytes);
        Ok(self.sp)
    }

    fn push_cstr(&mut self, value: &str) -> Result<usize, RunElfError> {
        self.sp = self.sp.checked_sub(1).ok_or(RunElfError::Stack)?;
        if self.sp < self.base {
            return Err(RunElfError::Stack);
        }
        self.push_bytes(value.as_bytes())
    }

    fn push_u64(&mut self, value: u64) -> Result<(), RunElfError> {
        self.sp = self.sp.checked_sub(8).ok_or(RunElfError::Stack)?;
        if self.sp < self.base {
            return Err(RunElfError::Stack);
        }
        let offset = self.sp.checked_sub(self.base).ok_or(RunElfError::Stack)?;
        let end = offset.checked_add(8).ok_or(RunElfError::Stack)?;
        self.bytes
            .get_mut(offset..end)
            .ok_or(RunElfError::Stack)?
            .copy_from_slice(&value.to_ne_bytes());
        Ok(())
    }
}

fn build_initial_stack(
    request: &RunLaunchInputs,
    main: &elf::ElfImage,
    stack_base: usize,
) -> Result<StackBuilder, RunElfError> {
    let mut stack = StackBuilder::new(stack_base, RUN_ELF_STACK_SIZE)?;

    let random_ptr = stack.push_bytes(&[
        0x41, 0x52, 0x4d, 0x36, 0x34, 0x2d, 0x53, 0x4d, 0x52, 0x4f, 0x53, 0x2d, 0x45, 0x4c, 0x46,
        0x21,
    ])?;
    let platform_ptr = stack.push_cstr(elf_platform_name())?;

    let mut argv_ptrs = Vec::new();
    for arg in request.argv.iter().rev() {
        argv_ptrs.push(stack.push_cstr(arg.as_str())? as u64);
    }
    argv_ptrs.reverse();
    if argv_ptrs.is_empty() {
        argv_ptrs.push(stack.push_cstr(request.path.as_str())? as u64);
    }

    let has_caller_library_path = request.env.iter().any(|entry| {
        user_logic::run_elf_environment_entry_has_key(entry.as_str(), RUN_ELF_LD_LIBRARY_PATH_KEY)
    });
    let effective = user_logic::run_elf_environment_effective_totals(
        request.env.len(),
        0,
        has_caller_library_path,
        0,
    )
    .ok_or(RunElfError::Stack)?;
    let mut env_ptrs = Vec::new();
    for output_index in (0..effective.entry_count).rev() {
        let value = match user_logic::run_elf_environment_source_at(
            output_index,
            request.env.len(),
            has_caller_library_path,
        ) {
            Some(user_logic::RunElfEnvironmentSource::Caller(index)) => request.env[index].as_str(),
            Some(user_logic::RunElfEnvironmentSource::Default) => RUN_ELF_DEFAULT_LD_LIBRARY_PATH,
            None => return Err(RunElfError::Stack),
        };
        env_ptrs.push(stack.push_cstr(value)? as u64);
    }
    env_ptrs.reverse();
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
    Ok(stack)
}

#[cfg(target_arch = "aarch64")]
fn elf_platform_name() -> &'static str {
    "aarch64"
}

#[cfg(target_arch = "riscv64")]
fn elf_platform_name() -> &'static str {
    "riscv64"
}

#[cfg(target_arch = "x86_64")]
fn elf_platform_name() -> &'static str {
    "x86_64"
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
