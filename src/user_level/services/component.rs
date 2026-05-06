//! Minimal Fuchsia-style component and userspace process framework.
//!
//! Fuchsia's current userspace is organized around component_manager,
//! resolvers, runners, per-component namespaces, and sandboxed process
//! launch. This module provides the smallest SMROS-native version of that
//! shape: a boot topology, namespace entries backed by FxFS paths, and an
//! ELF-runner-shaped start path into the existing user process table.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use crate::kernel_lowlevel::memory::process_manager;
use crate::kernel_objects::scheduler::scheduler;
use crate::user_level::{elf, fxfs, user_logic, user_process, user_test};

const MAX_COMPONENTS: usize = 16;
const MAX_PENDING_COMPONENT_LAUNCHES: usize = 4;
const NS_RIGHT_READ: u32 = 1 << 0;
const NS_RIGHT_WRITE: u32 = 1 << 1;
const NS_RIGHT_EXECUTE: u32 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentState {
    Discovered,
    Resolved,
    Started,
    Stopped,
    Destroyed,
}

impl ComponentState {
    pub fn as_str(self) -> &'static str {
        match self {
            ComponentState::Discovered => "discovered",
            ComponentState::Resolved => "resolved",
            ComponentState::Started => "started",
            ComponentState::Stopped => "stopped",
            ComponentState::Destroyed => "destroyed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerKind {
    Elf,
}

impl RunnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunnerKind::Elf => "elf",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NamespaceEntry {
    pub prefix: &'static str,
    pub target: &'static str,
    pub rights: u32,
}

#[derive(Clone, Debug)]
pub struct ComponentInstance {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub moniker: &'static str,
    pub url: &'static str,
    pub runner: RunnerKind,
    pub binary: &'static str,
    pub process_name: &'static str,
    pub state: ComponentState,
    pub pid: Option<usize>,
    pub thread_id: Option<usize>,
    pub exited: bool,
    pub exit_code: i32,
    pub loaded_entry: Option<u64>,
    pub loaded_segments: usize,
    pub load_error: Option<elf::ElfError>,
    pub namespace: Vec<NamespaceEntry>,
}

#[derive(Clone, Copy, Debug)]
pub struct ComponentStats {
    pub components: usize,
    pub discovered: usize,
    pub resolved: usize,
    pub started: usize,
    pub stopped: usize,
    pub destroyed: usize,
    pub runnable_threads: usize,
    pub exited: usize,
    pub loaded_images: usize,
    pub load_errors: usize,
}

pub struct ComponentManager {
    components: Vec<ComponentInstance>,
    next_id: usize,
    launchers_started: bool,
}

impl ComponentManager {
    fn new() -> Self {
        Self {
            components: Vec::new(),
            next_id: 1,
            launchers_started: false,
        }
    }

    fn reset(&mut self) {
        self.components.clear();
        self.next_id = 1;
        self.launchers_started = false;
    }

    fn default_namespace() -> Vec<NamespaceEntry> {
        let mut namespace = Vec::new();
        let entries = [
            ("/pkg", "/pkg", NS_RIGHT_READ | NS_RIGHT_EXECUTE),
            ("/data", "/data", NS_RIGHT_READ | NS_RIGHT_WRITE),
            ("/tmp", "/tmp", NS_RIGHT_READ | NS_RIGHT_WRITE),
            ("/svc", "/svc", NS_RIGHT_READ | NS_RIGHT_WRITE),
        ];
        for (prefix, target, rights) in entries {
            if user_logic::namespace_rights_valid(rights) {
                namespace.push(NamespaceEntry {
                    prefix,
                    target,
                    rights,
                });
            }
        }
        namespace
    }

    fn add_component(
        &mut self,
        parent_id: Option<usize>,
        moniker: &'static str,
        url: &'static str,
        runner: RunnerKind,
        binary: &'static str,
        process_name: &'static str,
    ) -> Option<usize> {
        if self.components.len() >= MAX_COMPONENTS {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.components.push(ComponentInstance {
            id,
            parent_id,
            moniker,
            url,
            runner,
            binary,
            process_name,
            state: ComponentState::Discovered,
            pid: None,
            thread_id: None,
            exited: false,
            exit_code: 0,
            loaded_entry: None,
            loaded_segments: 0,
            load_error: None,
            namespace: Self::default_namespace(),
        });
        Some(id)
    }

    fn get_mut(&mut self, id: usize) -> Option<&mut ComponentInstance> {
        self.components
            .iter_mut()
            .find(|component| component.id == id)
    }

    fn get_by_moniker_mut(&mut self, moniker: &str) -> Option<&mut ComponentInstance> {
        self.components
            .iter_mut()
            .find(|component| component.moniker == moniker)
    }

    fn start_component(&mut self, id: usize) -> Option<usize> {
        let component = self.get_mut(id)?;
        Self::start_component_record(component)
    }

    fn start_component_by_moniker(&mut self, moniker: &str) -> Option<usize> {
        let component = self.get_by_moniker_mut(moniker)?;
        Self::start_component_record(component)
    }

    fn start_component_record(component: &mut ComponentInstance) -> Option<usize> {
        let already_started = component.state == ComponentState::Started;
        if already_started {
            return component.pid;
        }
        let destroyed = component.state == ComponentState::Destroyed;
        let binary_exists = fxfs::exists(component.binary);
        if !user_logic::component_start_allowed(binary_exists, destroyed, already_started) {
            return None;
        }

        component.state = ComponentState::Resolved;
        let image = match elf::load_from_fxfs(component.binary) {
            Ok(image) => image,
            Err(err) => {
                component.load_error = Some(err);
                return None;
            }
        };
        let pid = user_process::create_user_process(component.process_name, component_el0_entry)?;
        if !user_process::set_loaded_elf(pid, &image) {
            component.load_error = Some(elf::ElfError::BadSegment);
            return None;
        }
        component.pid = Some(pid);
        component.loaded_entry = Some(image.entry);
        component.loaded_segments = image.segments.len();
        component.load_error = None;
        component.state = ComponentState::Started;
        Some(pid)
    }

    fn stop_component_by_moniker(&mut self, moniker: &str) -> bool {
        let Some(component) = self.get_by_moniker_mut(moniker) else {
            return false;
        };
        if let Some(pid) = component.pid.take() {
            let _ = process_manager().terminate_process(pid);
        }
        component.state = ComponentState::Stopped;
        true
    }

    fn install_boot_topology(&mut self) -> bool {
        self.reset();
        let Some(root_id) = self.add_component(
            None,
            "/",
            "fuchsia-boot:///#meta/root.cm",
            RunnerKind::Elf,
            "/pkg/bin/component_manager",
            "component_mgr",
        ) else {
            return false;
        };
        if let Some(root) = self.get_mut(root_id) {
            root.state = ComponentState::Started;
            root.pid = Some(1);
            root.thread_id = None;
            root.loaded_entry = Some(component_el0_entry as *const () as u64);
            root.loaded_segments = 1;
        }

        let Some(fxfs_id) = self.add_component(
            Some(root_id),
            "/bootstrap/fxfs",
            "fuchsia-boot:///#meta/fxfs.cm",
            RunnerKind::Elf,
            "/pkg/bin/fxfs",
            "fxfs",
        ) else {
            return false;
        };
        let Some(init_id) = self.add_component(
            Some(root_id),
            "/bootstrap/user-init",
            "fuchsia-boot:///#meta/user-init.cm",
            RunnerKind::Elf,
            "/pkg/bin/user-init",
            "user_init",
        ) else {
            return false;
        };

        if !Self::install_boot_elfs() {
            return false;
        }

        self.start_component(fxfs_id).is_some() && self.start_component(init_id).is_some()
    }

    fn install_boot_elfs() -> bool {
        let image = elf::build_trampoline_elf(component_el0_entry as *const () as u64);
        fxfs::write_file("/pkg/bin/component_manager", &image).is_ok()
            && fxfs::write_file("/pkg/bin/fxfs", &image).is_ok()
            && fxfs::write_file("/pkg/bin/user-init", &image).is_ok()
    }

    fn stats(&self) -> ComponentStats {
        let mut stats = ComponentStats {
            components: self.components.len(),
            discovered: 0,
            resolved: 0,
            started: 0,
            stopped: 0,
            destroyed: 0,
            runnable_threads: 0,
            exited: 0,
            loaded_images: 0,
            load_errors: 0,
        };
        for component in &self.components {
            match component.state {
                ComponentState::Discovered => stats.discovered += 1,
                ComponentState::Resolved => stats.resolved += 1,
                ComponentState::Started => stats.started += 1,
                ComponentState::Stopped => stats.stopped += 1,
                ComponentState::Destroyed => stats.destroyed += 1,
            }
            if component.thread_id.is_some() {
                stats.runnable_threads += 1;
            }
            if component.exited {
                stats.exited += 1;
            }
            if component.loaded_entry.is_some() {
                stats.loaded_images += 1;
            }
            if component.load_error.is_some() {
                stats.load_errors += 1;
            }
        }
        stats
    }

    fn snapshot(&self) -> Vec<ComponentInstance> {
        self.components.clone()
    }

    fn mark_component_exited(&mut self, pid: usize, exit_code: i32) -> bool {
        let Some(component) = self
            .components
            .iter_mut()
            .find(|component| component.pid == Some(pid))
        else {
            return false;
        };
        component.exited = true;
        component.exit_code = exit_code;
        component.state = ComponentState::Stopped;
        true
    }

    fn start_component_launchers(&mut self) -> bool {
        if self.launchers_started {
            return true;
        }

        let mut all_started = true;
        for component in self.components.iter_mut() {
            if component.id == 1 || component.thread_id.is_some() || component.exited {
                continue;
            }
            let Some(pid) = component.pid else {
                continue;
            };
            let queued = enqueue_component_launch(pid);
            let thread_id = if queued {
                scheduler().create_thread(component_launcher_entry, component.process_name)
            } else {
                None
            };
            let launched =
                user_logic::component_thread_launch_valid(true, queued, thread_id.is_some());
            if let (true, Some(thread_id)) = (launched, thread_id) {
                component.thread_id = Some(thread_id.as_usize());
                let _ = user_process::bind_primary_thread(pid, thread_id);
            } else {
                all_started = false;
            }
        }

        self.launchers_started = all_started;
        all_started
    }
}

static mut COMPONENT_MANAGER: Option<ComponentManager> = None;
static mut PENDING_COMPONENT_LAUNCHES: [usize; MAX_PENDING_COMPONENT_LAUNCHES] =
    [0; MAX_PENDING_COMPONENT_LAUNCHES];
static mut PENDING_COMPONENT_COUNT: usize = 0;
static ACTIVE_COMPONENT_PID: AtomicUsize = AtomicUsize::new(0);
static COMPONENT_RETURN_ACTIVE: AtomicBool = AtomicBool::new(false);
static COMPONENT_RETURN_PID: AtomicUsize = AtomicUsize::new(0);
static COMPONENT_RETURN_EXIT_CODE: AtomicI32 = AtomicI32::new(0);

fn manager() -> &'static mut ComponentManager {
    unsafe {
        if COMPONENT_MANAGER.is_none() {
            COMPONENT_MANAGER = Some(ComponentManager::new());
        }
        COMPONENT_MANAGER.as_mut().unwrap()
    }
}

fn enqueue_component_launch(pid: usize) -> bool {
    unsafe {
        if PENDING_COMPONENT_COUNT >= MAX_PENDING_COMPONENT_LAUNCHES {
            return false;
        }
        PENDING_COMPONENT_LAUNCHES[PENDING_COMPONENT_COUNT] = pid;
        PENDING_COMPONENT_COUNT += 1;
        true
    }
}

fn dequeue_component_launch() -> Option<usize> {
    unsafe {
        if PENDING_COMPONENT_COUNT == 0 {
            return None;
        }
        let pid = PENDING_COMPONENT_LAUNCHES[0];
        let mut index = 1usize;
        while index < PENDING_COMPONENT_COUNT {
            PENDING_COMPONENT_LAUNCHES[index - 1] = PENDING_COMPONENT_LAUNCHES[index];
            index += 1;
        }
        PENDING_COMPONENT_COUNT -= 1;
        Some(pid)
    }
}

extern "C" fn component_launcher_entry() -> ! {
    let pid = dequeue_component_launch().unwrap_or(0);
    ACTIVE_COMPONENT_PID.store(pid, Ordering::SeqCst);

    unsafe {
        let user_stack = user_process::user_stack_top(pid).unwrap_or(
            user_logic::USER_STACK_VADDR as u64 + (user_logic::USER_STACK_PAGES * 4096) as u64,
        );
        let entry = user_process::loaded_entry_point(pid)
            .unwrap_or(component_el0_entry as *const () as u64);
        user_process::switch_to_el0(entry, user_stack, 0);
    }
}

pub extern "C" fn component_el0_entry() -> ! {
    let _ = unsafe { user_test::linux_syscall(172, [0, 0, 0, 0, 0, 0]) };
    unsafe {
        user_test::linux_syscall(93, [0, 0, 0, 0, 0, 0]);
    }

    loop {
        cortex_a::asm::wfe();
    }
}

pub fn init() -> bool {
    if !fxfs::init() {
        return false;
    }
    manager().install_boot_topology()
}

pub fn start_boot_component_threads() -> bool {
    manager().start_component_launchers()
}

pub fn stats() -> ComponentStats {
    manager().stats()
}

pub fn snapshot() -> Vec<ComponentInstance> {
    manager().snapshot()
}

pub fn start_component(moniker: &str) -> Option<usize> {
    manager().start_component_by_moniker(moniker)
}

pub fn stop_component(moniker: &str) -> bool {
    manager().stop_component_by_moniker(moniker)
}

pub fn smoke_test() -> bool {
    if !fxfs::smoke_test() {
        return false;
    }
    let stats = manager().stats();
    stats.components >= 3
        && stats.started + stats.stopped >= 3
        && manager().components.iter().any(|component| {
            component.moniker == "/bootstrap/fxfs"
                && component.pid.is_some()
                && component.thread_id.is_some()
                && component.loaded_entry.is_some()
                && component.loaded_segments > 0
        })
}

pub fn prepare_component_return(exit_code: i32) -> bool {
    let pid = ACTIVE_COMPONENT_PID.load(Ordering::SeqCst);
    if !user_logic::component_return_active(pid) {
        return false;
    }

    ACTIVE_COMPONENT_PID.store(0, Ordering::SeqCst);
    COMPONENT_RETURN_ACTIVE.store(true, Ordering::SeqCst);
    COMPONENT_RETURN_PID.store(pid, Ordering::SeqCst);
    COMPONENT_RETURN_EXIT_CODE.store(exit_code, Ordering::SeqCst);

    let spsr_el1: u64 = user_logic::el1h_spsr_masked();
    unsafe {
        core::arch::asm!(
            "msr elr_el1, {resume}",
            "msr spsr_el1, {spsr}",
            resume = in(reg) component_launcher_resume as *const () as u64,
            spsr = in(reg) spsr_el1,
            options(nostack),
        );
    }
    true
}

#[no_mangle]
pub extern "C" fn component_launcher_resume() -> ! {
    if COMPONENT_RETURN_ACTIVE.swap(false, Ordering::SeqCst) {
        let pid = COMPONENT_RETURN_PID.load(Ordering::SeqCst);
        let exit_code = COMPONENT_RETURN_EXIT_CODE.load(Ordering::SeqCst);
        let _ = manager().mark_component_exited(pid, exit_code);
    }

    crate::kernel_objects::scheduler::scheduler().finish_current_without_stack_free();
    crate::kernel_objects::scheduler::schedule();

    loop {
        cortex_a::asm::wfe();
    }
}
