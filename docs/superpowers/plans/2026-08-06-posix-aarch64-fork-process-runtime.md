# AArch64 POSIX Fork Process Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace synthetic AArch64 fork and wait results with process-owned address spaces, eager private-page copying, shared-page visibility, real child execution, and exact POSIX child status.

**Architecture:** A 39-bit, three-level AArch64 translation-table implementation maps the low user window separately from supervisor-only RAM and MMIO identity mappings. A fixed-capacity process table binds existing Linux tasks to per-process memory and resources; fork reserves every child resource while suspended, eagerly copies private pages, retains shared backing references, then publishes atomically. Process exit becomes a zombie/wait lifecycle with `SIGCHLD` and launch-scoped cleanup.

**Tech Stack:** Rust `no_std`, AArch64 assembly, 4 KiB page tables, fixed-capacity kernel tables, Cargo host tests, source-level integration contracts, Verus shared logic, Open POSIX Test Suite, FxFS, AArch64 GNU tools, QEMU system emulation, Tarpaulin, and optional Coverity tools.

---

## Scope And Checkpoints

This plan implements the approved design in three dependent checkpoints:

1. **Address-space foundation:** enable a correct AArch64 MMU path, retain kernel/MMIO access, restore TTBR0 with each scheduler context, and run the existing system smoke unchanged.
2. **Process and fork runtime:** introduce process ownership, move user memory and descriptors behind the current process, launch a real eager-copy child, and prove private/shared memory behavior.
3. **Exit and evidence:** implement signal/normal termination, zombie selection and reaping, run focused POSIX groups, then publish the full campaign and quality evidence.

Do not start a later checkpoint while an earlier checkpoint has a failing host test, build, linker-layout check, or QEMU smoke.

## File Map

- Create `src/kernel_lowlevel/aarch64_vm_logic_shared.rs`: pure AArch64 VA geometry, descriptor, range, and frame-range rules.
- Create `src/kernel_lowlevel/ARM64/user_address_space.rs`: production three-level page tables, checked translation/copy, mapping, protection, unmapping, and table destruction.
- Modify `src/kernel_lowlevel/ARM64/drivers.rs`: retain the active FDT/static RAM range with the existing MMIO resources.
- Modify `src/kernel_lowlevel/memory.rs`: configure the page-frame allocator from aligned `__kernel_end` to the detected RAM end and return real physical PFNs.
- Modify `src/kernel_lowlevel/mmu.rs`: own the bootstrap AArch64 root, install MAIR/TCR/TTBR0/SCTLR, and expose bootstrap activation to secondary CPUs and new contexts.
- Modify `src/kernel_lowlevel/ARM64/context_shared.rs` and `context_switch.S`: include `TTBR0_EL1` in the scheduler context ABI.
- Modify `src/kernel_lowlevel/ARM64/thread.rs` and `smp.rs`: initialize kernel-thread roots and activate the bootstrap root on secondary CPUs.
- Create `src/syscall/linux_process_logic_shared.rs`: pure PID allocation, parent/process-group relations, lifecycle, wait selection, status encoding, and reaping rules.
- Create `src/syscall/linux_process.rs`: synchronized production process table, scheduler/task bindings, fork reservations, exit, wait blocking, reparenting, and launch cleanup.
- Create `src/syscall/linux_process_memory_logic_shared.rs`: pure mapping ownership, shared-reference, clone, and rollback rules.
- Create `src/syscall/linux_process_memory.rs`: per-process mappings, program break, stack, address-space owner, page backing references, and checked user copies.
- Modify `src/syscall/linux_task_logic_shared.rs` and `linux_task.rs`: bind each task to its TGID/process and reserve a process child separately from `CLONE_THREAD`.
- Modify `src/syscall/syscall.rs`: split system/process compatibility state and route memory, descriptor, fork, exit, signal, and wait syscalls through current process ownership.
- Modify `src/user_level/services/run_elf.rs`: build ELF segments and a fixed user stack through checked process-memory copies and enter EL0 with the real process root.
- Modify `tests/host/src/lib.rs`: host tests for all new shared logic.
- Modify `tests/host/tests/integration_contracts.rs`: architecture offsets and cross-module process ownership contracts.
- Modify `verification/kernel_lowlevel/src/lib.rs` and `verification/syscall/src/lib.rs`: verify new pure range and lifecycle logic.
- Create `docs/posix/2026-08-06-aarch64-fork-process-runtime-results.md`: exact build, runtime, resource, coverage, Verus, and Coverity evidence.

### Task 1: Lock AArch64 VM Geometry In Shared Logic

**Files:**
- Create: `src/kernel_lowlevel/aarch64_vm_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `verification/kernel_lowlevel/src/lib.rs`

- [ ] **Step 1: Write the failing host tests**

Add this host module:

~~~rust
mod aarch64_vm_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/aarch64_vm_logic_shared.rs"
    ));

    #[test]
    fn three_level_indices_distinguish_adjacent_user_pages() {
        assert_eq!(aarch64_table_indices(0x1000_0000), Some([0, 128, 0]));
        assert_eq!(aarch64_table_indices(0x1000_1000), Some([0, 128, 1]));
        assert_eq!(aarch64_table_indices(0x4000_0000), Some([1, 0, 0]));
        assert_eq!(aarch64_table_indices(1usize << 39), None);
    }

    #[test]
    fn physical_allocator_range_excludes_kernel_and_partial_pages() {
        assert_eq!(
            aarch64_frame_range(0x4fb0_7001, 0x4000_0000, 0x4000_0000),
            None
        );
        assert_eq!(
            aarch64_frame_range(0x4fb0_7001, 0x4000_0000, 0x6000_0000),
            Some((0x4fb0_8000, 0x6000_0000))
        );
    }

    #[test]
    fn user_window_is_below_qemu_ram_and_page_aligned() {
        assert!(aarch64_user_range_valid(0x1000_0000, 0x1000));
        assert!(aarch64_user_range_valid(0x1fff_e000, 0x2000));
        assert!(!aarch64_user_range_valid(0x0fff_f000, 0x2000));
        assert!(!aarch64_user_range_valid(0x1fff_f000, 0x2000));
    }
}
~~~

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib aarch64_vm_logic
~~~

Expected: compilation fails because `aarch64_vm_logic_shared.rs` does not exist.

- [ ] **Step 3: Implement the exact shared geometry**

Create:

~~~rust
pub(crate) const AARCH64_PAGE_SIZE: usize = 0x1000;
pub(crate) const AARCH64_VA_BITS: usize = 39;
pub(crate) const AARCH64_TABLE_ENTRIES: usize = 512;
pub(crate) const AARCH64_USER_BASE: usize = 0x1000_0000;
pub(crate) const AARCH64_USER_LIMIT: usize = 0x2000_0000;

pub(crate) fn aarch64_table_indices(vaddr: usize) -> Option<[usize; 3]> {
    if vaddr >= 1usize.checked_shl(AARCH64_VA_BITS as u32)? {
        return None;
    }
    Some([
        (vaddr >> 30) & (AARCH64_TABLE_ENTRIES - 1),
        (vaddr >> 21) & (AARCH64_TABLE_ENTRIES - 1),
        (vaddr >> 12) & (AARCH64_TABLE_ENTRIES - 1),
    ])
}

pub(crate) fn aarch64_user_range_valid(start: usize, len: usize) -> bool {
    start & (AARCH64_PAGE_SIZE - 1) == 0
        && len != 0
        && len & (AARCH64_PAGE_SIZE - 1) == 0
        && start >= AARCH64_USER_BASE
        && start
            .checked_add(len)
            .map(|end| end <= AARCH64_USER_LIMIT)
            .unwrap_or(false)
}

pub(crate) fn aarch64_frame_range(
    kernel_end: usize,
    ram_base: usize,
    ram_end: usize,
) -> Option<(usize, usize)> {
    let start = kernel_end
        .checked_add(AARCH64_PAGE_SIZE - 1)?
        & !(AARCH64_PAGE_SIZE - 1);
    let start = core::cmp::max(start, ram_base);
    let end = ram_end & !(AARCH64_PAGE_SIZE - 1);
    (start < end).then_some((start, end))
}
~~~

- [ ] **Step 4: Run GREEN and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib aarch64_vm_logic
git add src/kernel_lowlevel/aarch64_vm_logic_shared.rs tests/host/src/lib.rs verification/kernel_lowlevel/src/lib.rs
git commit -m "test: define AArch64 process VM geometry"
~~~

Expected: the focused tests pass and the proof harness includes the same shared functions.

### Task 2: Use The Detected RAM Range For Real Page Frames

**Files:**
- Modify: `src/kernel_lowlevel/ARM64/drivers.rs`
- Modify: `src/kernel_lowlevel/memory.rs`
- Modify: `src/kernel_lowlevel/lowlevel_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing RAM and allocator tests**

Test that the FDT RAM range overrides the 512 MiB static fallback, an invalid/overflowing range is rejected, the first allocated PFN equals the aligned physical start divided by 4096, freeing and reallocating returns that PFN, and no allocation lies below `__kernel_end` or at/above RAM end. Add an integration contract requiring `memory::init()` to pass the driver RAM range and linker end into `PageFrameAllocator::init_range`.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib kernel_lowlevel_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_page_allocator_uses_detected_ram_after_kernel_end -- --exact
~~~

Expected: the driver stats do not retain RAM size and the allocator still starts at PFN zero.

- [ ] **Step 3: Retain RAM resources**

Extend `PlatformResources`, cached atomics, `DriverStats`, FDT conversion, and static fallback with:

~~~rust
pub memory_base: usize,
pub memory_size: usize,
~~~

Expose:

~~~rust
pub fn memory_reg() -> Option<DeviceReg> {
    ensure_initialized();
    let base = MEMORY_BASE.load(Ordering::Acquire);
    let size = MEMORY_SIZE.load(Ordering::Acquire);
    (base != 0 && size != 0).then_some(DeviceReg { base, size })
}
~~~

- [ ] **Step 4: Configure the allocator with physical PFNs**

Increase the bitmap capacity to cover the maximum supported 2 GiB QEMU range. Add:

~~~rust
pub fn init_range(start: usize, end: usize) -> bool {
    if start & (PAGE_SIZE - 1) != 0
        || end & (PAGE_SIZE - 1) != 0
        || start >= end
    {
        return false;
    }
    let pages = (end - start) / PAGE_SIZE;
    let allocator = unsafe { &mut *ALLOCATOR.get() };
    if pages > allocator.bitmap.len() * 64 {
        return false;
    }
    allocator.bitmap.fill(0);
    allocator.base_pfn = (start / PAGE_SIZE) as u64;
    allocator.total_pages = pages;
    allocator.allocated_pages = 0;
    true
}

pub fn pfn_address(pfn: u64) -> Option<usize> {
    let allocator = unsafe { &*ALLOCATOR.get() };
    let index = pfn.checked_sub(allocator.base_pfn)?;
    (index < allocator.total_pages as u64)
        .then(|| (pfn as usize) * PAGE_SIZE)
}
~~~

Make `alloc()` return `base_pfn + page_idx` and make `free()` subtract `base_pfn` before indexing the bitmap. In `memory::init()`, obtain `__kernel_end`, call `aarch64_frame_range`, then require `init_range` success before the MMU initializes.

- [ ] **Step 5: Run GREEN, build, and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib kernel_lowlevel_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_page_allocator_uses_detected_ram_after_kernel_end -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/kernel_lowlevel/ARM64/drivers.rs src/kernel_lowlevel/memory.rs src/kernel_lowlevel/lowlevel_logic_shared.rs src/kernel_lowlevel/aarch64_vm_logic_shared.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: allocate real AArch64 physical frames"
~~~

### Task 3: Implement Three-Level AArch64 Address Spaces

**Files:**
- Create: `src/kernel_lowlevel/ARM64/user_address_space.rs`
- Modify: `src/kernel_lowlevel/ARM64/mod.rs`
- Modify: `src/kernel_lowlevel/mod.rs`
- Modify: `src/kernel_lowlevel/mmu.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing descriptor and ownership tests**

Add host tests for table, L2 block, and L3 page descriptors; user read-only/read-write/execute-never bits; adjacent 4 KiB translations; independent roots; remap rejection; exact unmap; and destruction returning every table page. Add a source contract rejecting the current duplicated `user_root_pfn` allocation and the one-slot `page_table_slot(vaddr)` walk.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib aarch64_vm_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_process_roots_walk_distinct_four_kib_pages -- --exact
~~~

Expected: descriptor helpers and `Aarch64AddressSpace` are absent; the integration contract finds the old one-table walk.

- [ ] **Step 3: Add descriptor constructors**

Add exact constructors to shared logic:

~~~rust
pub(crate) const AARCH64_DESC_VALID: u64 = 1;
pub(crate) const AARCH64_DESC_TABLE_OR_PAGE: u64 = 2;
pub(crate) const AARCH64_DESC_AF: u64 = 1 << 10;
pub(crate) const AARCH64_DESC_AP_USER: u64 = 1 << 6;
pub(crate) const AARCH64_DESC_AP_READ_ONLY: u64 = 1 << 7;
pub(crate) const AARCH64_DESC_INNER_SHAREABLE: u64 = 3 << 8;
pub(crate) const AARCH64_DESC_PXN: u64 = 1 << 53;
pub(crate) const AARCH64_DESC_UXN: u64 = 1 << 54;
pub(crate) const AARCH64_DESC_ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

pub(crate) fn aarch64_table_descriptor(paddr: usize) -> u64 {
    (paddr as u64 & AARCH64_DESC_ADDR_MASK)
        | AARCH64_DESC_VALID
        | AARCH64_DESC_TABLE_OR_PAGE
}

pub(crate) fn aarch64_user_page_descriptor(
    paddr: usize,
    readable: bool,
    writable: bool,
    executable: bool,
) -> u64 {
    let user_access = if readable || writable {
        AARCH64_DESC_AP_USER
    } else {
        0
    };
    let read_only = if writable {
        0
    } else {
        AARCH64_DESC_AP_READ_ONLY
    };
    let execute_never = if executable { 0 } else { AARCH64_DESC_UXN };
    (paddr as u64 & AARCH64_DESC_ADDR_MASK)
        | AARCH64_DESC_VALID
        | AARCH64_DESC_TABLE_OR_PAGE
        | AARCH64_DESC_AF
        | user_access
        | AARCH64_DESC_INNER_SHAREABLE
        | read_only
        | execute_never
        | AARCH64_DESC_PXN
}

pub(crate) fn aarch64_supervisor_block_descriptor(
    paddr: usize,
    device: bool,
    executable: bool,
) -> u64 {
    let attr_index = if device { 1u64 << 2 } else { 0 };
    let execute_never = if executable {
        0
    } else {
        AARCH64_DESC_PXN | AARCH64_DESC_UXN
    };
    (paddr as u64 & AARCH64_DESC_ADDR_MASK)
        | AARCH64_DESC_VALID
        | AARCH64_DESC_AF
        | AARCH64_DESC_INNER_SHAREABLE
        | attr_index
        | execute_never
}
~~~

- [ ] **Step 4: Implement the production owner**

Expose this exact API from `user_address_space.rs`:

~~~rust
pub struct Aarch64AddressSpace {
    root_pfn: u64,
    table_pfns: Vec<u64>,
}

impl Aarch64AddressSpace {
    pub fn new_with_kernel_map() -> Result<Self, AddressSpaceError>;
    pub fn root_paddr(&self) -> u64;
    pub fn map_user_page(
        &mut self,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError>;
    pub fn protect_user_page(
        &mut self,
        vaddr: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError>;
    pub fn unmap_user_page(&mut self, vaddr: usize) -> Result<u64, AddressSpaceError>;
    pub fn translate_user(&self, vaddr: usize, write: bool) -> Option<usize>;
    pub fn copy_to_user(&self, vaddr: usize, bytes: &[u8]) -> Result<(), AddressSpaceError>;
    pub fn copy_from_user(&self, vaddr: usize, out: &mut [u8]) -> Result<(), AddressSpaceError>;
}
~~~

Allocate and zero each table through `PageFrameAllocator` and its identity physical address. Walk L1/L2/L3 using `aarch64_table_indices`. Roll back newly allocated intermediate tables when a later allocation fails. `Drop` frees table PFNs only; process mapping ownership frees data PFNs.

- [ ] **Step 5: Run GREEN, build, and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib aarch64_vm_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_process_roots_walk_distinct_four_kib_pages -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/kernel_lowlevel/aarch64_vm_logic_shared.rs src/kernel_lowlevel/ARM64/user_address_space.rs src/kernel_lowlevel/ARM64/mod.rs src/kernel_lowlevel/mod.rs src/kernel_lowlevel/mmu.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: add AArch64 process address spaces"
~~~

### Task 4: Enable The Bootstrap MMU And Preserve The Existing Boot

**Files:**
- Modify: `src/kernel_lowlevel/mmu.rs`
- Modify: `src/kernel_lowlevel/ARM64/user_address_space.rs`
- Modify: `src/kernel_lowlevel/ARM64/smp.rs`
- Modify: `src/kernel_lowlevel/ARM64/cpu.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing activation contracts**

Require the bootstrap root to map the detected RAM as supervisor normal memory and the UART, GIC, and virtio windows as supervisor device memory. Require MAIR index 0 normal WB, index 1 device nGnRE, 39-bit `T0SZ=25`, 4 KiB granule, `EPD1=1`, and barrier ordering before setting `SCTLR_EL1.M/C/I`. Require secondary CPUs to activate the same root before unmasking IRQs.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_bootstrap_mmu_maps_ram_mmio_and_secondary_cpus -- --exact
~~~

Expected: `mmu::init()` only creates the old software manager and never enables translation.

- [ ] **Step 3: Implement bootstrap activation**

Add:

~~~rust
pub fn bootstrap_root() -> u64 {
    BOOTSTRAP_ROOT.load(Ordering::Acquire)
}

pub fn activate_bootstrap_on_current_cpu() -> bool {
    let root = bootstrap_root();
    if root == 0 {
        return false;
    }
    unsafe {
        crate::kernel_lowlevel::cpu::install_stage1_translation(root);
    }
    true
}
~~~

During `mmu::init()`, create one root, map RAM and required MMIO identity ranges with EL0 access disabled, publish its physical root, then activate it on CPU0. Keep the root owner alive for the boot. At the beginning of `secondary_cpu_entry()`, activate the same root before serial, GIC, scheduler, or IRQ access.

- [ ] **Step 4: Verify the unchanged system smoke**

~~~bash
make build-test ARCH=aarch64-unknown-none
SMOKE_QEMU_SMP=1 SMOKE_QEMU_MEMORY=512M make st ARCH=aarch64-unknown-none
SMOKE_QEMU_SMP=4 SMOKE_QEMU_MEMORY=512M make st ARCH=aarch64-unknown-none
~~~

Expected: both runs reach every existing required smoke milestone and the shell; no translation, permission, synchronous exception, GIC, UART, virtio, or secondary-CPU failure appears.

- [ ] **Step 5: Commit**

~~~bash
git add src/kernel_lowlevel/mmu.rs src/kernel_lowlevel/ARM64/user_address_space.rs src/kernel_lowlevel/ARM64/smp.rs src/kernel_lowlevel/ARM64/cpu.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: enable AArch64 stage-one translation"
~~~

### Task 5: Save And Restore TTBR0 In Scheduler Contexts

**Files:**
- Modify: `src/kernel_lowlevel/ARM64/context_shared.rs`
- Modify: `src/kernel_lowlevel/ARM64/context_switch.S`
- Modify: `src/kernel_lowlevel/ARM64/thread.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing ABI tests**

Assert `ttbr0_el1` follows `tpidr_el0`, `fpcr` moves to offset `0x138`, `fpsr` to `0x140`, SIMD to `0x150`, and both context-switch entry points save/restore TTBR0 with `dsb ish; tlbi vmalle1is; dsb ish; isb`. Require `CpuContext::new` to use `mmu::bootstrap_root()`.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_context_switch_preserves_process_ttbr0 -- --exact
~~~

Expected: the Rust field and assembly offsets are absent.

- [ ] **Step 3: Extend the context and assembly**

Insert:

~~~rust
pub ttbr0_el1: u64,
~~~

after `tpidr_el0`. Initialize it to `crate::kernel_lowlevel::mmu::bootstrap_root()` in `CpuContext::new`; keep zero only in the const empty context.

In both save/restore paths use offset `0x130` for TTBR0 and shift FP/SIMD offsets by eight/sixteen bytes as required by alignment. Restore TTBR0 and invalidate before restoring SIMD and returning. Update every Rust `offset_of!` assertion and assembly comment from the same layout.

- [ ] **Step 4: Run GREEN, smoke, and commit**

~~~bash
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_context_switch_preserves_process_ttbr0 -- --exact
make build-test ARCH=aarch64-unknown-none
SMOKE_QEMU_SMP=4 SMOKE_QEMU_MEMORY=512M make st ARCH=aarch64-unknown-none
git add src/kernel_lowlevel/ARM64/context_shared.rs src/kernel_lowlevel/ARM64/context_switch.S src/kernel_lowlevel/ARM64/thread.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: switch AArch64 process translation roots"
~~~

### Task 6: Add The Process Lifecycle Model

**Files:**
- Create: `src/syscall/linux_process_logic_shared.rs`
- Create: `src/syscall/linux_process.rs`
- Modify: `src/syscall/mod.rs`
- Modify: `src/syscall/linux_task_logic_shared.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `verification/syscall/src/lib.rs`

- [ ] **Step 1: Write failing process-table tests**

Test root registration, monotonic PID allocation, parent and process-group inheritance, task/TGID lookup, hidden reservations, atomic publication, rollback, running-to-zombie transition, exact/any/process-group wait selection, `WNOHANG`, one-time reaping, reparenting to the launch reaper, and exhaustion without PID reuse.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
~~~

Expected: the process model and module do not exist.

- [ ] **Step 3: Define the exact shared model**

~~~rust
pub(crate) const LINUX_ROOT_PID: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxProcessState {
    Empty,
    Reserved,
    Running,
    Zombie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessCore {
    pub pid: usize,
    pub parent_pid: usize,
    pub process_group: usize,
    pub root_scheduler_thread: usize,
    pub state: LinuxProcessState,
    pub wait_status: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessReservation {
    pub slot: usize,
    pub pid: usize,
    pub parent_pid: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxWaitSelector {
    Pid(usize),
    Any,
    ProcessGroup(usize),
}

pub(crate) fn linux_wait_status_exit(code: i32) -> i32 {
    ((code as u32 & 0xff) << 8) as i32
}

pub(crate) fn linux_wait_status_signal(signum: usize, core_dumped: bool) -> Option<i32> {
    (1..=127).contains(&signum)
        .then_some(signum as i32 | if core_dumped { 0x80 } else { 0 })
}
~~~

Implement `LinuxProcessTable<const N: usize>` methods `register_root`, `reserve_child`, `publish`, `rollback`, `by_pid`, `by_scheduler`, `exit`, `select_waitable`, `has_matching_child`, `reap`, `reparent_children`, and `reset`. A reserved process is invisible to all public lookup.

- [ ] **Step 4: Bind tasks and production runtime**

Use `tgid` as the task's process PID. Root task registration also registers the root process. `CLONE_THREAD` children retain parent TGID. Add a production lock and APIs:

~~~rust
pub(crate) fn register_root(scheduler: ThreadId) -> Result<usize, SysError>;
pub(crate) fn current() -> Result<LinuxProcessCore, SysError>;
pub(crate) fn current_pid() -> Result<usize, SysError>;
pub(crate) fn current_parent_pid() -> Result<usize, SysError>;
pub(crate) fn reset_launch();
~~~

Route `sys_getpid()` to `current_pid()`, `sys_getppid()` to `current_parent_pid()`, and `sys_gettid()` to the current task TID. The root reports its launch reaper as parent only internally; the user-visible root PPID remains zero. Return `ESRCH` when a Linux syscall context has no live task/process binding.

- [ ] **Step 5: Run GREEN and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_process_logic_shared.rs src/syscall/linux_process.rs src/syscall/mod.rs src/syscall/linux_task_logic_shared.rs src/syscall/linux_task.rs src/syscall/syscall.rs tests/host/src/lib.rs verification/syscall/src/lib.rs
git commit -m "feat: model Linux process lifecycle"
~~~

### Task 7: Move User Memory Behind Process Ownership

**Files:**
- Create: `src/syscall/linux_process_memory_logic_shared.rs`
- Create: `src/syscall/linux_process_memory.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/syscall/address_logic_shared.rs`
- Modify: `src/user_level/services/run_elf.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `verification/syscall/src/lib.rs`

- [ ] **Step 1: Write failing memory-isolation tests**

Test that each PID has an independent mapping list, program break, stack range, next mmap address, and address-space root; private backing is owned once; shared backing increments/decrements references; checked copies cross page boundaries; permissions return `EFAULT`; and dropping one process cannot unmap another. Require no raw ELF/stack `write_bytes` or `copy_nonoverlapping` to user virtual addresses.

Add the standalone shared-logic module to `tests/host/src/lib.rs` so the pure ownership rules compile and run on the host:

~~~rust
mod linux_process_memory_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_process_memory_logic_shared.rs"
    ));
}
~~~

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_memory_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_memory_and_loader_are_process_owned -- --exact
~~~

Expected: mappings and stack remain in global `MemorySyscallState`, and the loader writes identity addresses.

- [ ] **Step 3: Introduce explicit backing and mapping records**

~~~rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPageBacking {
    Private { pfn: u64 },
    Shared { object_id: u32, page_index: usize, pfn: u64 },
}

pub(crate) struct LinuxProcessMapping {
    pub addr: usize,
    pub len: usize,
    pub prot: usize,
    pub flags: usize,
    pub pages: Vec<LinuxPageBacking>,
    pub source: LinuxMappingSource,
}

pub(crate) struct LinuxProcessMemory {
    pub address_space: Aarch64AddressSpace,
    pub mappings: Vec<LinuxProcessMapping>,
    pub initial_stack: Option<(usize, usize)>,
    pub next_addr: usize,
    pub brk: BrkState,
}
~~~

Use `AARCH64_USER_BASE=0x1000_0000`, `AARCH64_USER_LIMIT=0x2000_0000`, main base `0x1000_0000`, interpreter base `0x1100_0000`, and a fixed stack ending at `0x1fff_f000`. The mapping allocator starts after the ELF/interpreter reserved spans.

- [ ] **Step 4: Route memory syscalls and checked copies**

Replace global mapping/brk/stack lookups with `linux_process_memory::with_current`. Make `mmap` allocate page PFNs, install PTEs, then publish metadata; rollback PTEs/PFNs on failure. Make `munmap`, `mprotect`, `mremap`, and `brk` update page tables and metadata transactionally.

Expose:

~~~rust
pub(crate) fn copy_from_current(address: usize, out: &mut [u8]) -> Result<(), SysError>;
pub(crate) fn copy_to_current(address: usize, bytes: &[u8]) -> Result<(), SysError>;
pub(crate) fn zero_current(address: usize, len: usize) -> Result<(), SysError>;
pub(crate) fn current_root_paddr() -> Result<u64, SysError>;
~~~

Convert syscall C-string, struct, signal-frame, TID, and wait-status access touched by this increment to these checked copies.

- [ ] **Step 5: Build the root ELF and stack inside its address space**

Create the root process/memory before `prepare_dynamic_loader`. Map segment spans with actual ELF permissions, copy file bytes through `copy_to_current`, zero BSS through `zero_current`, build the argv/env/auxv image in a temporary kernel vector, copy it into the fixed user stack, and enter EL0 with `current_root_paddr()`.

- [ ] **Step 6: Run GREEN, current ELF canaries, and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_memory_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_memory_and_loader_are_process_owned -- --exact
make build-test ARCH=aarch64-unknown-none
SMOKE_QEMU_SMP=1 SMOKE_QEMU_MEMORY=1024M make st ARCH=aarch64-unknown-none
git add src/syscall/linux_process_memory_logic_shared.rs src/syscall/linux_process_memory.rs src/syscall/syscall.rs src/syscall/address_logic_shared.rs src/user_level/services/run_elf.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs verification/syscall/src/lib.rs
git commit -m "feat: run AArch64 ELF in process-owned memory"
~~~

### Task 8: Split Descriptor Ownership And Shared Page Backing

**Files:**
- Modify: `src/syscall/syscall.rs`
- Modify: `src/syscall/linux_process.rs`
- Modify: `src/syscall/linux_process_memory_logic_shared.rs`
- Modify: `src/syscall/linux_process_memory.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing resource-clone tests**

Test a resource-clone reservation directly, before it is connected to `sys_fork`: copied descriptor numbers and flags, shared open-description file offsets, independent descriptor close, pipe endpoint survival, message queue/shared-memory object references, private mapping divergence, shared mapping visibility, final-reference destruction, and rollback without closing a parent resource.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --lib linux_process_memory_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_resource_clone_inherits_open_descriptions_and_shared_pages -- --exact
~~~

Expected: descriptors and shared attachments still belong to one global compatibility state.

- [ ] **Step 3: Split descriptor entry from open description**

Use:

~~~rust
pub(crate) struct LinuxDescriptorEntry {
    pub fd: usize,
    pub description_id: u32,
    pub close_on_exec: bool,
}

pub(crate) struct LinuxOpenDescription {
    pub id: u32,
    pub handle: u32,
    pub object_type: ObjectType,
    pub status_flags: usize,
    pub offset: usize,
    pub references: usize,
}
~~~

Each process owns descriptor entries. System state owns open descriptions. Add an unpublished `LinuxResourceClone` reservation that copies entries and increments each description once; Task 9 attaches that reservation to the runnable fork transaction. Close removes one entry and releases the description; the underlying object closes only at zero references. Dropping an uncommitted resource clone releases only its acquired references.

- [ ] **Step 4: Make shared attachments process-specific**

System shared-memory records own object identity and pages. Process mapping records own attachment VA/length. `munmap` and process exit release only that process attachment. `IPC_RMID` removes the name/id immediately but retains pages until the final attachment/reference.

- [ ] **Step 5: Run GREEN and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --lib linux_process_memory_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_resource_clone_inherits_open_descriptions_and_shared_pages -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/syscall.rs src/syscall/linux_process.rs src/syscall/linux_process_memory_logic_shared.rs src/syscall/linux_process_memory.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: inherit process descriptors and shared pages"
~~~

### Task 9: Implement Transactional Eager Fork

**Files:**
- Modify: `src/syscall/linux_process_logic_shared.rs`
- Modify: `src/syscall/linux_process.rs`
- Modify: `src/syscall/linux_process_memory.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/kernel_lowlevel/ARM64/thread.rs`
- Modify: `src/kernel_lowlevel/ARM64/context_switch.S`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing reservation and rollback tests**

Inject failure after process slot, scheduler thread, child root, each private page, shared reference, descriptor reference, child task, and scheduler publication. Assert the parent snapshot is byte-for-byte unchanged, the PID is never visible, and every acquired resource returns to baseline. Test the child frame has `x0=0`, copied PC/SP/PSTATE/TLS/SIMD, and a distinct root; parent return is the child PID.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --lib linux_process_memory_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_fork_publishes_only_a_complete_child -- --exact
~~~

Expected: `sys_fork()` still increments `LINUX_NEXT_SYNTHETIC_PID`.

- [ ] **Step 3: Add the process start and reservation owners**

~~~rust
pub(crate) struct Aarch64ProcessStart {
    pub frame: Aarch64ExceptionFrame,
    pub return_pc: u64,
    pub pstate: u64,
    pub root_paddr: u64,
}

pub(crate) struct LinuxForkReservation {
    pub process: LinuxProcessReservation,
    pub task: LinuxTaskReservation,
    pub scheduler_thread: ThreadId,
    pub child_start: Aarch64ProcessStart,
    published: bool,
}
~~~

Its `Drop` unwinds unpublished child task, descriptor/shared references, private pages, table pages, scheduler thread, and process slot in reverse acquisition order. A successful `commit` publishes process, task, then scheduler and marks `published=true`.

- [ ] **Step 4: Clone memory and resources with exact ownership**

For each private page, allocate one child PFN, copy 4096 bytes through physical identity addresses, and map it at the same child VA. For each shared page, increment the system backing reference and map the same PFN. Consume Task 8's unpublished `LinuxResourceClone` reservation for descriptor entries, open-description references, and shared attachments. Copy mapping metadata, brk, stack range, signal dispositions, calling-thread mask, process group, credentials, directory/container view, and namespace view. Clear pending signals, active signal frames, AIO requests, POSIX timers, interval timers, and wait registrations.

- [ ] **Step 5: Replace synthetic fork dispatch**

`sys_fork()` requires `linux_syscall_context::current()`, creates a suspended scheduler thread on the Linux runtime CPU, calls `linux_process::reserve_fork`, copies the exception frame with child x0 zero, commits, and returns the PID. `sys_vfork()` calls the eager fork path. Non-thread `clone` rejects `CLONE_VM|CLONE_FILES|CLONE_SIGHAND` sharing with `ENOSYS` and applies accepted namespace flags to the child only.

- [ ] **Step 6: Run GREEN, build, and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --lib linux_process_memory_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_fork_publishes_only_a_complete_child -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_process_logic_shared.rs src/syscall/linux_process.rs src/syscall/linux_process_memory.rs src/syscall/linux_task.rs src/syscall/syscall.rs src/kernel_lowlevel/ARM64/thread.rs src/kernel_lowlevel/ARM64/context_switch.S tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: execute eager-copy AArch64 fork children"
~~~

### Task 10: Implement Exit, Wait Selection, And One-Time Reaping

**Files:**
- Modify: `src/syscall/linux_process_logic_shared.rs`
- Modify: `src/syscall/linux_process.rs`
- Modify: `src/syscall/linux_process_memory.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/user_level/services/posix_test.rs`
- Modify: `scripts/posix/model.py`
- Modify: `scripts/posix/events.py`
- Modify: `scripts/posix/qemu_runner.py`
- Modify: `scripts/posix/report.py`
- Modify: `scripts/posix/tests/test_events.py`
- Modify: `scripts/posix/tests/test_model.py`
- Modify: `scripts/posix/tests/test_qemu_runner.py`
- Modify: `scripts/posix/tests/test_report.py`
- Modify: `scripts/posix/tests/test_source.py`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing exit/wait tests**

Test normal status `(code & 0xff) << 8`, exact PID, `-1`, `0`, and negative process-group selection, `WNOHANG`, invalid option `EINVAL`, no child `ECHILD`, bad status pointer without reaping, blocked-parent wake, repeated wait, process-scoped `exit_group`, descendant exit not completing the root launcher, and cleanup of an orphaned descendant. Snapshot each lifecycle transition and assert exact live-process, zombie, private-page, shared-page, and page-table-page counts.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_wait_reaps_one_real_child_status -- --exact
~~~

Expected: `sys_wait4()` echoes a positive PID and writes zero.

- [ ] **Step 3: Implement the wait outcome API**

~~~rust
pub(crate) enum LinuxWaitOutcome {
    Ready { pid: usize, status: i32 },
    WouldBlock,
    NoChildren,
}

pub(crate) fn wait_current(
    selector: LinuxWaitSelector,
    nohang: bool,
) -> Result<LinuxWaitOutcome, SysError>;
~~~

Parse selectors without signed overflow. If ready, validate/copy status before reaping. If nohang and matching live children exist, return zero. If blocking, set `LinuxBlockReason::ChildWait`, schedule, and retry after wake. Reap exactly once.

- [ ] **Step 4: Make exit process-scoped**

`sys_exit` retires the calling task; the final task releases private execution resources and creates a zombie. `sys_exit_group` terminates only tasks with the current TGID. Descendants never call `prepare_run_elf_return`. Final root exit records the launch outcome, retires surviving descendants through the internal launch reaper, then resumes the launcher.

- [ ] **Step 5: Carry process memory counts through resource evidence**

Expose a single locked process-runtime snapshot with these counts:

~~~rust
pub(crate) struct LinuxProcessResourceCounts {
    pub linux_processes: usize,
    pub linux_zombies: usize,
    pub private_pages: usize,
    pub shared_pages: usize,
    pub page_table_pages: usize,
}
~~~

Add the same five signed delta fields to `PosixResourceSnapshot`, the guest `resource_deltas` object, Python `ResourceDeltas`, canonical parsing, resume validation, reports, and test fixtures. Keep the existing generic `processes` field for the kernel process manager; do not alias it to the Linux process table. Require a complete mapping at every schema-1 event boundary, preserve signed overflow checks, and test that a nonzero value in each new field survives serial parsing, QEMU persistence/resume, JSON/Markdown/HTML reports, and `has_nonzero`/`has_positive` checks.

~~~bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_resources_include_linux_process_page_lifecycle -- --exact
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  scripts.posix.tests.test_events \
  scripts.posix.tests.test_model \
  scripts.posix.tests.test_qemu_runner \
  scripts.posix.tests.test_report \
  scripts.posix.tests.test_source
~~~

- [ ] **Step 6: Run GREEN and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_wait_reaps_one_real_child_status -- --exact
./scripts/run-host-unit-tests.sh --test integration_contracts posix_resources_include_linux_process_page_lifecycle -- --exact
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  scripts.posix.tests.test_events \
  scripts.posix.tests.test_model \
  scripts.posix.tests.test_qemu_runner \
  scripts.posix.tests.test_report \
  scripts.posix.tests.test_source
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_process_logic_shared.rs src/syscall/linux_process.rs src/syscall/linux_process_memory.rs src/syscall/linux_task.rs src/syscall/syscall.rs src/user_level/services/posix_test.rs scripts/posix/model.py scripts/posix/events.py scripts/posix/qemu_runner.py scripts/posix/report.py scripts/posix/tests/test_events.py scripts/posix/tests/test_model.py scripts/posix/tests/test_qemu_runner.py scripts/posix/tests/test_report.py scripts/posix/tests/test_source.py tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: wait for and reap Linux child processes"
~~~

### Task 11: Add Signal Termination And SIGCHLD Rules

**Files:**
- Modify: `src/syscall/linux_process_logic_shared.rs`
- Modify: `src/syscall/linux_process.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing signal lifecycle tests**

Test default terminate status low seven bits, no core bit without a core, process-directed target selection, `SIGKILL` process termination, copied dispositions and calling-thread mask, cleared child pending queues, normal zombie plus `SIGCHLD`, immediate reap for ignored `SIGCHLD`, immediate reap plus notification for `SA_NOCLDWAIT`, and blocked wait wake in all cases.

- [ ] **Step 2: Run RED**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_signal_termination_reports_wait_status_and_sigchld -- --exact
~~~

Expected: signals route only through the root TGID model and do not create process wait status.

- [ ] **Step 3: Move process signal ownership**

Move signal dispositions and process-pending queues into the process record. Keep masks, task-pending queues, alternate stacks, and signal frames in task state. Fork copies dispositions and the caller mask, clears all pending/frame state, and creates only one child task.

- [ ] **Step 4: Route default actions through process exit**

Default terminate actions call `linux_process::terminate_by_signal`, retire every task in the target TGID, encode `signum | core_bit`, wake a matching parent, and apply the exact ignored/NOCLDWAIT zombie rules. `SIGKILL` and `SIGSTOP` remain unmaskable.

- [ ] **Step 5: Run GREEN and commit**

~~~bash
./scripts/run-host-unit-tests.sh --lib linux_process_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_signal_termination_reports_wait_status_and_sigchld -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_process_logic_shared.rs src/syscall/linux_process.rs src/syscall/linux_task.rs src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: terminate Linux processes through signals"
~~~

### Task 12: Run Focused AArch64 POSIX Canaries

**Files:**
- Generated: `target/posix/aarch64/smros-fxfs-fork-process-<commit>.img`
- Generated: `target/posix/aarch64/smros-run-fork-process-<commit>-*`

- [ ] **Step 1: Run all offline and build gates**

~~~bash
make host-fmt-check script-check launcher-test linker-layout-test ut it posix-tool-test
make build-test ARCH=aarch64-unknown-none
git diff --check
~~~

Expected: every command exits zero. Record actual unit/integration/POSIX-tool counts rather than copying earlier counts.

- [ ] **Step 2: Rebuild and verify the exact AArch64 stage**

~~~bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
~~~

Expected: the full reviewed inventory is preserved, the manifest binds the current commit and patch series, and verification is complete.

- [ ] **Step 3: Create a fresh private disk**

~~~bash
commit=$(git rev-parse --short=12 HEAD)
disk="target/posix/aarch64/smros-fxfs-fork-process-${commit}.img"
test ! -e "$disk"
qemu-img create -f raw "$disk" 128M
~~~

Do not access the repository-root `smros-fxfs.img`.

- [ ] **Step 4: Run source-selected canaries**

Read the pinned sources and select tests covering child return, private memory, shared mappings, descriptors, normal exit, signal exit, and wait. The mandatory first test is `conformance/behavior/WIFEXITED/1-3.c`. Run each test in a fresh QEMU process against the private disk:

~~~bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
import subprocess

from scripts.posix.qemu_runner import run_smros

commit = subprocess.run(
    ["git", "rev-parse", "--short=12", "HEAD"],
    check=True,
    capture_output=True,
    text=True,
).stdout.strip()
disk = Path(f"target/posix/aarch64/smros-fxfs-fork-process-{commit}.img")
output = Path(
    f"target/posix/aarch64/smros-run-fork-process-{commit}-wifexited"
)
assert disk.is_file()
assert not output.exists()
result = run_smros(
    Path("host_shared/posixtest"),
    output,
    kernel=Path("kernel8.img"),
    disk=disk,
    memory="1024M",
    test_id="conformance/behavior/WIFEXITED/1-3.c",
)
assert result.complete
assert len(result.attempts) == 1
print(result.attempts[0].status)
PY
~~~

Use the runner's explicit output and disk options when executing from the implementation worktree so every result directory contains one terminal attempt and its complete serial log.

- [ ] **Step 5: Validate canary evidence**

Parse `results.ndjson` with Python's `json` module. Require `launch_status=launched`, no timeout/restart, exact PTS status, measured non-positive resource deltas including `linux_processes`, `linux_zombies`, `private_pages`, `shared_pages`, and `page_table_pages`, and absence of `Kernel panic`, `Fatal glibc error`, heap-corruption, translation-fault, and stale-root markers. Do not convert an assertion failure into pass.

### Task 13: Run Groups, Full Campaign, And Quality Evidence

**Files:**
- Generated: focused/group/full run and report directories below `target/posix/aarch64/`
- Create: `docs/posix/2026-08-06-aarch64-fork-process-runtime-results.md`

- [ ] **Step 1: Run focused APIs and affected groups**

Use fresh private disks for the complete `fork` API, base group, and affected memory/shared-memory selection. Require every selected attempt to terminate and retain its genuine status. Compare counts against the `f39aaf6` baseline and the post-allocation scheduling evidence.

- [ ] **Step 2: Run the complete 1,598-test campaign**

Create another blank private disk and run `run_smros` with no API/group filter, `memory="1024M"`, and a commit-specific output directory. Require `selected_count=1598`, complete event framing, and explicit unavailable resource evidence only for real host watchdog timeouts.

- [ ] **Step 3: Run coverage and Verus**

~~~bash
make coverage-host
make verus
~~~

Record Tarpaulin's actual numerator, denominator, percent, uncovered lines, command exit, and artifact. A below-100% gate remains failed. Record every Verus harness result and the coverage-audit findings.

- [ ] **Step 4: Run Coverity when available**

~~~bash
if command -v cov-build >/dev/null 2>&1 \
  && command -v cov-analyze >/dev/null 2>&1 \
  && command -v cov-format-errors >/dev/null 2>&1; then
  commit=$(git rev-parse --short=12 HEAD)
  covdir="target/coverity-aarch64-fork-process-${commit}"
  test ! -e "$covdir"
  cov-build --dir "$covdir" make build-test ARCH=aarch64-unknown-none
  cov-analyze --dir "$covdir" --all
  cov-format-errors --dir "$covdir" \
    --json-output-v7 "target/coverity-aarch64-fork-process-${commit}.json"
else
  command -v cov-build || true
  command -v cov-analyze || true
  command -v cov-format-errors || true
fi
~~~

If any command is absent, record Coverity as unavailable with the missing command names. Do not report zero findings.

- [ ] **Step 5: Publish the detailed report**

Generate canonical quality evidence bound to the exact commit and architecture. Run the POSIX report with manifest, Linux reference results, SMROS full results, and quality evidence. Verify all seven report artifacts, hashes, API/group coverage, optional-group selection, resource evidence, and quality tables.

- [ ] **Step 6: Write and commit the evidence document**

The document must include:

- implementation commit and architecture;
- pinned suite revision and patch-series digest;
- canonical/file hashes for manifest, build results, results, serial log, quality evidence, and report summary;
- discovered/build/link/staged inventory;
- canary and group rows with exact status, duration, timeout/restart, and resource evidence;
- full status, API, group, optional-group, execution/pass/program coverage;
- fatal-marker scan;
- Tarpaulin numerator/denominator/uncovered lines and failed/pass gate;
- Verus harness and audit detail;
- Coverity findings/artifact or explicit missing-command evidence;
- baseline deltas without remapping statuses; and
- remaining work, including named IPC, optional scheduling, x86_64, and RISC-V64.

~~~bash
git add docs/posix/2026-08-06-aarch64-fork-process-runtime-results.md
git commit -m "docs: record AArch64 fork process results"
~~~

### Task 14: Final Review And Local Merge Gate

**Files:**
- No new files expected

- [ ] **Step 1: Run the complete verification gate**

~~~bash
make test
make verus
SMOKE_QEMU_SMP=4 SMOKE_QEMU_MEMORY=512M make st ARCH=aarch64-unknown-none
git diff --check
git status --short --branch
~~~

Expected: tests, Verus, system smoke, build, and layout pass; the worktree is clean. Coverage and Coverity retain the exact separately recorded status rather than being hidden by this gate.

- [ ] **Step 2: Review requirements line by line**

Compare the implementation and evidence against every goal, non-goal, invariant, test gate, and success criterion in `docs/superpowers/specs/2026-08-06-posix-aarch64-fork-process-runtime-design.md`. Any unmet required item keeps the branch unmerged.

- [ ] **Step 3: Request code review**

Use `superpowers:requesting-code-review`. Resolve findings with `superpowers:receiving-code-review`, rerun the affected tests, then rerun Step 1.

- [ ] **Step 4: Merge locally only after approval**

Use `superpowers:finishing-a-development-branch`. Fast-forward local `master` only when the implementation branch is an ancestor-compatible clean branch and every required gate above has fresh passing evidence. Preserve all existing worktrees and generated evidence.
