# AArch64 EL0 Memory-Fault Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert AArch64 EL0 instruction and data aborts into synchronous POSIX `SIGSEGV`/`SIGBUS` delivery so `mmap/6-2.c` and adjacent memory-protection tests terminate correctly instead of fault-looping.

**Architecture:** A pure architecture-local decoder turns `ESR_EL1` into a typed memory access and abort kind. Linux process-memory metadata then chooses `SEGV_MAPERR`, `SEGV_ACCERR`, or proven file-tail `BUS_ADRERR`; an immediate synchronous signal path reuses the existing AArch64 handler frame and process termination lifecycle. Dedicated vector entries keep current-EL faults kernel-fatal and ensure lower-EL aborts never reach an unchanged `eret`.

**Tech Stack:** Rust `no_std`, AArch64 exception-vector assembly, SMROS Linux process/task/memory runtimes, host Rust unit and integration tests, Open POSIX Test Suite C, QEMU, Python result tooling, Verus, Tarpaulin, and Coverity when installed.

---

## File Structure

- Create `src/kernel_lowlevel/aarch64_exception_logic_shared.rs`: pure AArch64 ESR exception-class, fault-status, and access decoder.
- Modify `src/kernel_lowlevel/mod.rs`: expose the AArch64 exception decoder only on AArch64 builds and re-export the selected architecture boot module for the fatal bridge.
- Modify `src/kernel_lowlevel/ARM64/boot.rs`: separate current-EL and lower-EL synchronous vectors, forward complete fault metadata, and print fatal kernel diagnostics.
- Modify `src/syscall/linux_process_memory_logic_shared.rs`: pure mapping permission, file-tail, and POSIX fault-code policy.
- Modify `src/syscall/linux_process_memory.rs`: retain backing-object length, preserve it across mapping transformations, enforce inaccessible pages wholly beyond the object, and classify the current fault address.
- Modify `src/syscall/linux_fork_logic_shared.rs`: allow fork page mapping to select effective protection per page without losing existing failure rollback.
- Modify `src/syscall/linux_task_logic_shared.rs`: build the exact 128-byte AArch64 Linux `siginfo_t` payload and 464-byte core of a 4560-byte AArch64 `ucontext_t` for synchronous memory faults.
- Modify `src/syscall/syscall.rs`: extract reusable signal-handler installation and add fail-closed immediate synchronous fault delivery.
- Modify `src/syscall/syscall_dispatch.rs`: bridge saved AArch64 fault state to the pure decoder and Linux signal runtime.
- Modify `tests/host/src/lib.rs`: unit tests for syndrome decoding, fault policy, backing-page protection, `siginfo_t`, and per-page fork mapping.
- Modify `tests/host/tests/integration_contracts.rs`: source contracts for vector origin separation, metadata forwarding, synchronous delivery, fatal current-EL behavior, and no fault-loop return.
- Create `docs/posix/2026-08-13-aarch64-el0-memory-fault-results.md`: exact runtime, resource, coverage, static-analysis, and provenance evidence.

The approved design is `docs/superpowers/specs/2026-08-13-aarch64-el0-memory-fault-design.md`.

### Task 1: Decode AArch64 EL0 Abort Syndromes

**Files:**
- Create: `src/kernel_lowlevel/aarch64_exception_logic_shared.rs`
- Modify: `src/kernel_lowlevel/mod.rs`
- Test: `tests/host/src/lib.rs`

- [ ] **Step 1: Add failing host tests for exception decoding**

Add this host-only module to `tests/host/src/lib.rs`:

```rust
mod aarch64_exception_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/aarch64_exception_logic_shared.rs"
    ));

    fn esr(ec: u64, iss: u64) -> u64 {
        (ec << 26) | iss
    }

    #[test]
    fn lower_el_abort_decoder_preserves_access_and_fault_kind() {
        assert_eq!(
            aarch64_lower_el_sync(esr(0x24, 0x07)),
            Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault {
                access: Aarch64El0MemoryAccess::Read,
                kind: Aarch64El0AbortKind::Translation,
            })
        );
        assert_eq!(
            aarch64_lower_el_sync(esr(0x24, (1 << 6) | 0x0f)),
            Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault {
                access: Aarch64El0MemoryAccess::Write,
                kind: Aarch64El0AbortKind::Permission,
            })
        );
        assert_eq!(
            aarch64_lower_el_sync(esr(0x20, 0x09)),
            Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault {
                access: Aarch64El0MemoryAccess::Execute,
                kind: Aarch64El0AbortKind::AccessFlag,
            })
        );
    }

    #[test]
    fn decoder_separates_svc_and_unsupported_lower_el_exceptions() {
        assert_eq!(aarch64_lower_el_sync(esr(0x15, 0)), Aarch64LowerElSync::Svc);
        assert_eq!(
            aarch64_lower_el_sync(esr(0x24, 0x21)),
            Aarch64LowerElSync::Unsupported
        );
        assert_eq!(
            aarch64_lower_el_sync(esr(0x3c, 0)),
            Aarch64LowerElSync::Unsupported
        );
    }
}
```

- [ ] **Step 2: Run the decoder tests and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml aarch64_exception_logic -- --nocapture
```

Expected: compilation fails because `aarch64_exception_logic_shared.rs` and its typed decoder do not exist.

- [ ] **Step 3: Implement the pure decoder**

Create `src/kernel_lowlevel/aarch64_exception_logic_shared.rs` with these exact public types and behavior:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64El0MemoryAccess {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64El0AbortKind {
    Translation,
    AccessFlag,
    Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Aarch64El0MemoryFault {
    pub access: Aarch64El0MemoryAccess,
    pub kind: Aarch64El0AbortKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64LowerElSync {
    Svc,
    MemoryFault(Aarch64El0MemoryFault),
    Unsupported,
}

pub(crate) fn aarch64_lower_el_sync(esr: u64) -> Aarch64LowerElSync {
    let ec = (esr >> 26) & 0x3f;
    if ec == 0x15 {
        return Aarch64LowerElSync::Svc;
    }
    let access = match ec {
        0x20 => Aarch64El0MemoryAccess::Execute,
        0x24 if esr & (1 << 6) != 0 => Aarch64El0MemoryAccess::Write,
        0x24 => Aarch64El0MemoryAccess::Read,
        _ => return Aarch64LowerElSync::Unsupported,
    };
    let kind = match esr & 0x3f {
        0x04..=0x07 => Aarch64El0AbortKind::Translation,
        0x08..=0x0b => Aarch64El0AbortKind::AccessFlag,
        0x0c..=0x0f => Aarch64El0AbortKind::Permission,
        _ => return Aarch64LowerElSync::Unsupported,
    };
    Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault { access, kind })
}
```

In `src/kernel_lowlevel/mod.rs`, add:

```rust
#[cfg(target_arch = "aarch64")]
pub(crate) mod aarch64_exception_logic_shared;
```

- [ ] **Step 4: Run focused and complete host unit tests**

```bash
cargo test --manifest-path tests/host/Cargo.toml aarch64_exception_logic -- --nocapture
make ut
```

Expected: the focused decoder tests and all host library tests pass.

- [ ] **Step 5: Commit the decoder**

```bash
git add src/kernel_lowlevel/aarch64_exception_logic_shared.rs src/kernel_lowlevel/mod.rs tests/host/src/lib.rs
git commit -m "feat: decode AArch64 EL0 memory aborts"
```

### Task 2: Define POSIX Memory-Fault Policy, `siginfo_t`, and `ucontext_t`

**Files:**
- Modify: `src/syscall/linux_process_memory_logic_shared.rs`
- Modify: `src/syscall/linux_task_logic_shared.rs`
- Test: `tests/host/src/lib.rs`

- [ ] **Step 1: Add failing tests for mapping classification and file-tail protection**

Inside the existing `linux_process_memory_logic` host module, add tests using these new shared types:

```rust
#[test]
fn memory_fault_policy_distinguishes_maperr_accerr_and_file_tail_bus() {
    let anonymous = LinuxMemoryFaultRegion {
        addr: 0x1200_0000,
        len: 0x2000,
        prot: LINUX_PROT_READ,
        file_offset: None,
        backing_len: None,
    };
    let file = LinuxMemoryFaultRegion {
        addr: 0x1300_0000,
        len: 0x3000,
        prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
        file_offset: Some(0),
        backing_len: Some(0x800),
    };

    assert_eq!(
        linux_memory_fault_signal(&[anonymous, file], 0x1100_0000, LinuxMemoryFaultAccess::Read, 0x1000),
        LinuxMemoryFaultSignal::SegvMaperr
    );
    assert_eq!(
        linux_memory_fault_signal(&[anonymous, file], 0x1200_0008, LinuxMemoryFaultAccess::Write, 0x1000),
        LinuxMemoryFaultSignal::SegvAccerr
    );
    assert_eq!(
        linux_memory_fault_signal(&[anonymous, file], 0x1300_1001, LinuxMemoryFaultAccess::Write, 0x1000),
        LinuxMemoryFaultSignal::BusAdrerr
    );
    assert_eq!(
        linux_memory_fault_signal(&[file], 0x1300_07ff, LinuxMemoryFaultAccess::Read, 0x1000),
        LinuxMemoryFaultSignal::SegvAccerr,
        "the partial final page is not a beyond-object page"
    );
}

#[test]
fn effective_file_page_protection_blocks_only_pages_wholly_beyond_object() {
    assert_eq!(
        linux_effective_mapping_page_prot(
            LINUX_PROT_READ | LINUX_PROT_WRITE,
            Some(0), Some(0x800), 0, 0x1000,
        ),
        LINUX_PROT_READ | LINUX_PROT_WRITE
    );
    assert_eq!(
        linux_effective_mapping_page_prot(
            LINUX_PROT_READ | LINUX_PROT_WRITE,
            Some(0), Some(0x800), 1, 0x1000,
        ),
        0
    );
    assert_eq!(
        linux_effective_mapping_page_prot(LINUX_PROT_EXEC, None, None, 99, 0x1000),
        LINUX_PROT_EXEC
    );
}
```

- [ ] **Step 2: Add a failing `siginfo_t` wire-layout test**

Inside the existing Linux task signal tests, add:

```rust
#[test]
fn synchronous_fault_record_uses_aarch64_linux_siginfo_layout() {
    let record = LinuxPendingSignal::synchronous_fault(11, 2, 0x1234_5678_9abc_def0);
    assert_eq!(record.signum, 11);
    assert!(record.has_info);
    assert_eq!(i32::from_ne_bytes(record.info[0..4].try_into().unwrap()), 11);
    assert_eq!(i32::from_ne_bytes(record.info[4..8].try_into().unwrap()), 0);
    assert_eq!(i32::from_ne_bytes(record.info[8..12].try_into().unwrap()), 2);
    assert_eq!(
        u64::from_ne_bytes(record.info[16..24].try_into().unwrap()),
        0x1234_5678_9abc_def0
    );
}
```

Add a second test for the AArch64 glibc/Linux `ucontext_t` core layout:

```rust
#[test]
fn synchronous_fault_ucontext_core_preserves_faulting_aarch64_state() {
    let regs = core::array::from_fn::<u64, 32, _>(|index| 0x1000 + index as u64);
    let core = linux_aarch64_ucontext_core(
        0xdead_beef,
        regs,
        0x1fff_f000,
        0x1234_5000,
        0x6000_0000,
        0x55aa,
        LinuxSignalStack::DISABLED,
    );
    assert_eq!(LINUX_AARCH64_UCONTEXT_BYTES, 4560);
    assert_eq!(core.len(), LINUX_AARCH64_UCONTEXT_CORE_BYTES);
    assert_eq!(u64::from_ne_bytes(core[40..48].try_into().unwrap()), 0x55aa);
    assert_eq!(u64::from_ne_bytes(core[176..184].try_into().unwrap()), 0xdead_beef);
    assert_eq!(u64::from_ne_bytes(core[184..192].try_into().unwrap()), regs[0]);
    assert_eq!(u64::from_ne_bytes(core[424..432].try_into().unwrap()), regs[30]);
    assert_eq!(u64::from_ne_bytes(core[432..440].try_into().unwrap()), 0x1fff_f000);
    assert_eq!(u64::from_ne_bytes(core[440..448].try_into().unwrap()), 0x1234_5000);
    assert_eq!(u64::from_ne_bytes(core[448..456].try_into().unwrap()), 0x6000_0000);
}

#[test]
fn synchronous_fault_user_frame_is_aligned_bounded_and_non_overlapping() {
    let (sp, info, context) = linux_aarch64_signal_user_frame(0x20_000).unwrap();
    assert_eq!(sp & 0xf, 0);
    assert_eq!(info, sp);
    assert_eq!(context, info + LINUX_SIGNAL_INFO_BYTES as u64);
    assert!(context + LINUX_AARCH64_UCONTEXT_BYTES as u64 <= 0x20_000);
    assert_eq!(linux_aarch64_signal_user_frame(1), None);
}
```

- [ ] **Step 3: Run the policy tests and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml memory_fault_policy -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml effective_file_page_protection -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml synchronous_fault_record -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml synchronous_fault_ucontext -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml synchronous_fault_user_frame -- --nocapture
```

Expected: compilation fails because the policy types, helpers, synchronous signal-record constructor, and AArch64 context builder are missing.

- [ ] **Step 4: Implement the pure memory policy**

In `src/syscall/linux_process_memory_logic_shared.rs`, add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxMemoryFaultAccess { Read, Write, Execute }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxMemoryFaultSignal { SegvMaperr, SegvAccerr, BusAdrerr }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMemoryFaultRegion {
    pub addr: usize,
    pub len: usize,
    pub prot: usize,
    pub file_offset: Option<usize>,
    pub backing_len: Option<usize>,
}

pub(crate) fn linux_effective_mapping_page_prot(
    prot: usize,
    file_offset: Option<usize>,
    backing_len: Option<usize>,
    page_index: usize,
    page_size: usize,
) -> usize {
    let Some((file_offset, backing_len)) = file_offset.zip(backing_len) else { return prot; };
    let Some(page_offset) = page_index.checked_mul(page_size).and_then(|v| file_offset.checked_add(v)) else { return 0; };
    if page_offset >= backing_len { 0 } else { prot }
}

pub(crate) fn linux_memory_fault_signal(
    regions: &[LinuxMemoryFaultRegion],
    address: usize,
    access: LinuxMemoryFaultAccess,
    page_size: usize,
) -> LinuxMemoryFaultSignal {
    let Some(region) = regions.iter().copied().find(|region| {
        address >= region.addr
            && region.addr.checked_add(region.len).is_some_and(|end| address < end)
    }) else { return LinuxMemoryFaultSignal::SegvMaperr; };
    let permission = match access {
        LinuxMemoryFaultAccess::Read => LINUX_PROT_READ | LINUX_PROT_WRITE,
        LinuxMemoryFaultAccess::Write => LINUX_PROT_WRITE,
        LinuxMemoryFaultAccess::Execute => LINUX_PROT_EXEC,
    };
    if region.prot & permission == 0 { return LinuxMemoryFaultSignal::SegvAccerr; }
    let page_index = (address - region.addr) / page_size;
    if linux_effective_mapping_page_prot(
        region.prot, region.file_offset, region.backing_len, page_index, page_size,
    ) == 0 && region.file_offset.is_some() {
        LinuxMemoryFaultSignal::BusAdrerr
    } else {
        LinuxMemoryFaultSignal::SegvAccerr
    }
}
```

Keep the current POSIX protection constants as the single source of truth; do not introduce duplicate numeric protection bits.

- [ ] **Step 5: Implement the synchronous `siginfo_t` constructor**

In `LinuxPendingSignal` in `src/syscall/linux_task_logic_shared.rs`, add a non-const constructor that zero-initializes all 128 bytes, writes `si_signo` at byte 0, zero `si_errno` at byte 4, `si_code` at byte 8, and the 64-bit fault address at byte 16:

```rust
pub(crate) fn synchronous_fault(signum: usize, code: i32, address: u64) -> Self {
    let mut record = Self::standard(signum);
    record.has_info = true;
    record.info[0..4].copy_from_slice(&(signum as i32).to_ne_bytes());
    record.info[4..8].copy_from_slice(&0i32.to_ne_bytes());
    record.info[8..12].copy_from_slice(&code.to_ne_bytes());
    record.info[16..24].copy_from_slice(&address.to_ne_bytes());
    record
}
```

In the same shared file, define the verified AArch64 glibc/Linux layout:

```rust
pub(crate) const LINUX_AARCH64_UCONTEXT_BYTES: usize = 4560;
pub(crate) const LINUX_AARCH64_UCONTEXT_CORE_BYTES: usize = 464;

pub(crate) fn linux_aarch64_ucontext_core(
    fault_address: u64,
    regs: [u64; 32],
    sp: u64,
    pc: u64,
    pstate: u64,
    signal_mask: u64,
    signal_stack: LinuxSignalStack,
) -> [u8; LINUX_AARCH64_UCONTEXT_CORE_BYTES] {
    let mut core = [0; LINUX_AARCH64_UCONTEXT_CORE_BYTES];
    core[16..24].copy_from_slice(&signal_stack.sp.to_ne_bytes());
    core[24..28].copy_from_slice(&signal_stack.flags.to_ne_bytes());
    core[32..40].copy_from_slice(&signal_stack.size.to_ne_bytes());
    core[40..48].copy_from_slice(&signal_mask.to_ne_bytes());
    core[176..184].copy_from_slice(&fault_address.to_ne_bytes());
    for (index, value) in regs[..31].iter().enumerate() {
        let offset = 184 + index * 8;
        core[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
    }
    core[432..440].copy_from_slice(&sp.to_ne_bytes());
    core[440..448].copy_from_slice(&pc.to_ne_bytes());
    core[448..456].copy_from_slice(&pstate.to_ne_bytes());
    core
}

pub(crate) fn linux_aarch64_signal_user_frame(
    stack_top: u64,
) -> Option<(u64, u64, u64)> {
    let frame_bytes = (LINUX_SIGNAL_INFO_BYTES + LINUX_AARCH64_UCONTEXT_BYTES) as u64;
    let frame_base = stack_top.checked_sub(frame_bytes)? & !0xf;
    Some((
        frame_base,
        frame_base,
        frame_base.checked_add(LINUX_SIGNAL_INFO_BYTES as u64)?,
    ))
}
```

The offsets come from the installed AArch64 sysroot: `uc_stack=16`, `uc_sigmask=40`, `uc_mcontext=176`, `fault_address=176`, `regs=184`, `sp=432`, `pc=440`, `pstate=448`, and the aligned reserved extension area begins at 464. Add host assertions that `linux_aarch64_signal_user_frame(0x20000)` produces a 16-byte-aligned frame below the original stack, non-overlapping info/context ranges, and `None` on underflow. Do not allocate the entire 4560-byte context on the kernel stack; later delivery code zeros user storage and copies only this bounded core header.

- [ ] **Step 6: Run tests and commit the pure POSIX policy**

```bash
cargo test --manifest-path tests/host/Cargo.toml memory_fault_policy -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml effective_file_page_protection -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml synchronous_fault_record -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml synchronous_fault_ucontext -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml synchronous_fault_user_frame -- --nocapture
make ut
git add src/syscall/linux_process_memory_logic_shared.rs src/syscall/linux_task_logic_shared.rs tests/host/src/lib.rs
git commit -m "feat: classify POSIX memory fault signals"
```

Expected: all focused and complete host unit tests pass.

### Task 3: Retain Backing Length and Enforce File-Tail Faults

**Files:**
- Modify: `src/syscall/linux_process_memory.rs`
- Modify: `src/syscall/linux_fork_logic_shared.rs`
- Modify: `src/syscall/syscall.rs`
- Test: `tests/host/src/lib.rs`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing contracts for backing-length propagation**

Add an integration test that reads `linux_process_memory.rs` and `syscall.rs`, extracts the relevant braced bodies, and requires all of these tokens:

```rust
for token in [
    "File { fd, offset, path, backing_len }",
    "backing_len: *backing_len",
    "linux_effective_mapping_page_prot(",
    "classify_current_memory_fault(",
] {
    assert!(memory.contains(token), "missing file-fault metadata token {token}");
}
assert!(syscall.contains("backing_len: attrs.size"));
assert!(syscall.contains("linux_read_mmap_contents(&source, len)"));
```

Also require fork mapping to call a per-page protection variant and require mapping split, fork clone, fixed replacement, `mprotect`, and `mremap` paths to retain the source rather than reconstruct file metadata.

- [ ] **Step 2: Run the new integration contract and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts linux_file_tail_fault_metadata -- --nocapture
```

Expected: FAIL on the missing `backing_len`, classifier, and per-page protection tokens.

- [ ] **Step 3: Add per-page fork mapping with rollback tests**

Extend existing host tests for `map_linux_fork_pages` with a call to:

```rust
map_linux_fork_pages_with_protection(
    &mut ops,
    0x1000,
    0x1000,
    &pages,
    |index| if index == 1 { 0 } else { LINUX_PROT_READ },
    |_| false,
)
```

Assert the three mapped protections are `[LINUX_PROT_READ, 0, LINUX_PROT_READ]`. Inject failure on the third map and assert both prior pages are unmapped in reverse order, preserving the existing transaction guarantee.

- [ ] **Step 4: Run the per-page fork test and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml map_linux_fork_pages_with_protection -- --nocapture
```

Expected: compilation fails because the per-page mapper is missing.

- [ ] **Step 5: Implement per-page fork protection**

In `linux_fork_logic_shared.rs`, add `map_linux_fork_pages_with_protection` with the same rollback and failure-injection order as `map_linux_fork_pages`, except it calls `protection(mapped)` before `ops.map_page`. Keep `map_linux_fork_pages` as a compatibility wrapper:

```rust
pub(crate) fn map_linux_fork_pages<O: LinuxForkPageOps>(
    ops: &mut O, address: usize, page_size: usize, pages: &[O::Page], prot: usize,
    should_fail: impl FnMut(LinuxForkFailurePoint) -> bool,
) -> Result<(), O::Error> {
    map_linux_fork_pages_with_protection(
        ops, address, page_size, pages, |_| prot, should_fail,
    )
}
```

- [ ] **Step 6: Retain immutable file backing length**

Change `LinuxMappingSource::File` to:

```rust
File {
    fd: usize,
    offset: u64,
    path: String,
    backing_len: usize,
}
```

`try_slice` increments only `offset`; it copies `fd` and `backing_len` and clones `path`. Fork cloning, metadata-plan cloning, mapping snapshots, fixed replacement, `mprotect` splitting, and `mremap` retain the same `backing_len`.

In `sys_mmap`, obtain `fxfs::attrs` once when constructing the file source and set `backing_len: attrs.size`. Change `linux_read_mmap_contents` to use this retained length for bounds while still reading through `fxfs::read_file_at`.

- [ ] **Step 7: Apply effective protection in every page-table transition**

Add a `LinuxProcessMemory::mapping_page_prot(source, requested_prot, page_index)` wrapper around `linux_effective_mapping_page_prot`. Replace uniform mapping/protection at these boundaries:

```text
initial mmap and MAP_FIXED replacement
fork child page installation
mprotect, including split mappings
mremap destination and in-place growth
rollback/restore metadata snapshots
```

Use a per-page transaction record containing address, old effective protection, and new effective protection. If any map or protect fails, restore all earlier pages in reverse order and leave metadata uncommitted. A file page is given effective protection zero only when `file_offset + page_index * PAGE_SIZE >= backing_len`; page zero of a half-page file keeps its requested protection.

- [ ] **Step 8: Expose current-process fault classification**

Add:

```rust
pub(crate) fn classify_current_memory_fault(
    address: usize,
    access: LinuxMemoryFaultAccess,
) -> LinuxMemoryFaultSignal
```

It snapshots each current mapping as `LinuxMemoryFaultRegion`, using `Some(offset)` and `Some(backing_len)` only for `LinuxMappingSource::File`. It also treats the active brk interval as anonymous read/write memory. If runtime lookup fails, return `SegvMaperr`; never retry the instruction.

- [ ] **Step 9: Run focused tests, integration contracts, and commit**

```bash
cargo test --manifest-path tests/host/Cargo.toml map_linux_fork_pages_with_protection -- --nocapture
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts linux_file_tail_fault_metadata -- --nocapture
make ut it
git add src/syscall/linux_process_memory.rs src/syscall/linux_fork_logic_shared.rs src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: preserve file mapping fault metadata"
```

Expected: focused tests and the full host unit/integration suites pass.

### Task 4: Deliver Synchronous Fault Signals Fail-Closed

**Files:**
- Modify: `src/syscall/syscall.rs`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add a failing synchronous-delivery integration contract**

Require `syscall.rs` to define `deliver_linux_synchronous_memory_fault`, map policy outcomes exactly as follows, and use `LinuxPendingSignal::synchronous_fault`:

```text
SegvMaperr -> (SIGSEGV=11, SEGV_MAPERR=1)
SegvAccerr -> (SIGSEGV=11, SEGV_ACCERR=2)
BusAdrerr  -> (SIGBUS=7, BUS_ADRERR=2)
```

Extract the function body and assert it:

```rust
assert!(delivery.contains("linux_task::current_task()"));
assert!(delivery.contains("linux_signal_action(signum)"));
assert!(delivery.contains("signal_state.mask & linux_signal_bit(signum)"));
assert!(delivery.contains("install_linux_signal_handler("));
assert!(delivery.contains("terminate_linux_process_by_signal(current.tgid, signum)"));
assert!(!delivery.contains("queue_process_linux_signal"));
assert!(!delivery.contains("requeue_linux_signal"));
```

Require the shared handler installer to reserve a 16-byte-aligned user signal frame below the selected normal or alternate stack top using `linux_aarch64_signal_user_frame`, zero all `LINUX_AARCH64_UCONTEXT_BYTES` context bytes in that frame, copy `linux_aarch64_ucontext_core`, and set saved `x2` to the context address for `SA_SIGINFO`. Assert it never constructs `[u8; LINUX_AARCH64_UCONTEXT_BYTES]` on the kernel stack and never expands the global trampoline mapping by task count or nesting depth.

Require ignored, default, blocked, and handler-install-failure branches to converge on termination. Require descendant termination to use `finish_current_without_el0_return`, and require the launch-root result to write the launch ID to saved `x0` after `prepare_run_elf_return` changes `ELR_EL1`/`SPSR_EL1`.

- [ ] **Step 2: Run the delivery contract and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts synchronous_memory_fault_delivery -- --nocapture
```

Expected: FAIL because the immediate delivery entry and shared handler installer do not exist.

- [ ] **Step 3: Extract handler-frame installation**

Extract the handler-only portion of `deliver_next_linux_signal` into:

```rust
fn install_linux_signal_handler(
    saved_regs: usize,
    return_pc: u64,
    pending: LinuxPendingSignal,
    action: LinuxKernelSigaction,
    restart: Option<LinuxRestartBlock>,
    context_fault_address: u64,
) -> Result<(), SysError>
```

The helper must preflight the trampoline address, handler mask, alternate-stack choice, user signal-frame subtraction, complete `siginfo_t` copy, and complete AArch64 `ucontext_t` copy before committing the kernel-side signal frame. Select `alt_stack.sp + alt_stack.size` only when `SA_ONSTACK` is requested and the alternate stack is enabled and inactive; otherwise select the interrupted `SP_EL0`. Reserve `LINUX_SIGNAL_INFO_BYTES + LINUX_AARCH64_UCONTEXT_BYTES` below that top, align the new SP down to 16 bytes, and validate the entire frame as writable. Copy the pending 128-byte info at the frame base. Zero the complete 4560-byte context through `linux_zero_user`, then copy only `linux_aarch64_ucontext_core(context_fault_address, regs, user_sp, return_pc, read_exception_return_state(), previous_mask, signal_state.alt_stack)`. Only after all fallible preflight succeeds may the helper push `LinuxSignalFrame`, change mask/alternate-stack state, install the new `SP_EL0`, set saved `x0`/`x1`/`x2`/`x16`, and set `ELR_EL1` to the trampoline. For `SA_SIGINFO`, `x1` is the frame-base `siginfo_t` pointer and `x2` is the following `ucontext_t` pointer; otherwise both remain zero. Preserve `SA_SIGINFO`, `SA_ONSTACK`, `SA_NODEFER`, and `SA_RESETHAND` behavior. Existing queued delivery passes context fault address zero and remains responsible for reservation commit/requeue and restart-block restoration. `sigreturn` continues to restore the trusted kernel-side `LinuxSignalFrame`, including the interrupted SP, so user edits to the informational context do not corrupt kernel bookkeeping.

- [ ] **Step 4: Implement immediate synchronous delivery**

Add constants for signals and codes beside existing signal constants, then add:

```rust
pub(crate) fn deliver_linux_synchronous_memory_fault(
    saved_regs: usize,
    return_pc: u64,
    fault_address: u64,
    access: LinuxMemoryFaultAccess,
) -> Result<(), SysError> {
    if saved_regs == 0 {
        return Err(SysError::EFAULT);
    }
    let current = linux_task::current_task()?;
    let (signum, code) = match linux_process_memory::classify_current_memory_fault(
        fault_address as usize,
        access,
    ) {
        LinuxMemoryFaultSignal::SegvMaperr => (LINUX_SIGSEGV, LINUX_SEGV_MAPERR),
        LinuxMemoryFaultSignal::SegvAccerr => (LINUX_SIGSEGV, LINUX_SEGV_ACCERR),
        LinuxMemoryFaultSignal::BusAdrerr => (LINUX_SIGBUS, LINUX_BUS_ADRERR),
    };
    let pending = LinuxPendingSignal::synchronous_fault(signum, code, fault_address);
    let action = linux_signal_action(signum);
    let blocked = linux_task::with_current_signal_state(|state| {
        state.mask & linux_signal_bit(signum) != 0
    })?;
    if linux_task::linux_signal_disposition(action.handler, signum)
        == LinuxSignalDisposition::Handled
        && !blocked
        && install_linux_signal_handler(
            saved_regs,
            return_pc,
            pending,
            action,
            None,
            fault_address,
        )
        .is_ok()
    {
        return Ok(());
    }

    let launch_id = terminate_linux_process_by_signal(current.tgid, signum)?;
    let regs = unsafe { &mut *(saved_regs as *mut [u64; 32]) };
    regs[0] = launch_id as u64;
    Ok(())
}
```

This function must not place the fault on an ordinary pending queue. A handled but currently blocked signal, an ignored disposition, default disposition, invalid current task, trampoline failure, info/context-copy failure, or frame-capacity failure is fatal. For a caught unblocked signal, call the shared installer with `restart=None` and `context_fault_address=fault_address` because no syscall restart is involved. For fatal delivery, call `terminate_linux_process_by_signal`; descendant termination does not return, while launch-root termination writes the returned launch ID into saved `x0` and returns `Ok(())` only so `eret` enters the already prepared kernel resume PC. If task lookup, signal termination, or launch-root preparation fails, return the exact `SysError` to the architecture bridge; never convert it to success.

- [ ] **Step 5: Run delivery contracts and host regressions**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts synchronous_memory_fault_delivery -- --nocapture
make ut it
```

Expected: the focused contract and complete host suites pass.

- [ ] **Step 6: Commit synchronous delivery**

```bash
git add src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: deliver synchronous POSIX memory faults"
```

### Task 5: Route AArch64 Exception Vectors Without Fault Loops

**Files:**
- Modify: `src/kernel_lowlevel/ARM64/boot.rs`
- Modify: `src/kernel_lowlevel/mod.rs`
- Modify: `src/syscall/syscall_dispatch.rs`
- Test: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Add failing vector-routing contracts**

Add contracts requiring:

```text
current_sync_sp0 -> selects SP_EL1 -> fatal_aarch64_sync_exception
current_sync_spx -> fatal_aarch64_sync_exception
lower_sync_a64  -> complete 0x310-byte saved frame -> handle_aarch64_lower_el_sync
lower_sync_a32  -> fatal_aarch64_sync_exception
```

Within the lower-AArch64 saved-frame handler, require reads of `esr_el1`, `far_el1`, and `elr_el1`, followed by `bl handle_aarch64_lower_el_sync`. Require the SVC branch to retain `handle_syscall_simple`, `complete_linux_signal_syscall_return`, and `syscall_should_advance_elr`. Assert the non-SVC path branches directly to register restore and does not execute `mov x0, #-38`, store an errno, call `syscall_should_advance_elr`, or advance `ELR_EL1`.

Require `fatal_aarch64_sync_exception` to print origin-independent ESR/FAR/ELR hex diagnostics and loop in `wait_for_interrupt`; it returns `!`. Require `handle_aarch64_lower_el_sync` to use the pure decoder, translate the exact shared access enum, call synchronous delivery, and route unsupported syndromes or delivery errors to the fatal function with the original metadata.

- [ ] **Step 2: Run vector contracts and verify RED**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts aarch64_synchronous_exception_vectors -- --nocapture
```

Expected: FAIL because every synchronous vector still targets the shared SVC-only handler and its `-ENOSYS` fallthrough.

- [ ] **Step 3: Split vector origins and lower-EL returns**

In the vector table:

```asm
// Current EL with SP0
b       current_sync_sp0
...
// Current EL with SPx
b       current_sync_spx
...
// Lower EL using AArch64
b       lower_sync_a64
...
// Lower EL using AArch32
b       lower_sync_a32
```

`current_sync_sp0` first executes `msr spsel, #1` so fatal Rust code uses the EL1 kernel stack. The current-SPx and lower-AArch32 stubs read `ESR_EL1`, `FAR_EL1`, and `ELR_EL1` into `x0..x2`, call `fatal_aarch64_sync_exception`, and never return.

In `src/kernel_lowlevel/mod.rs`, add `boot` to the selected-architecture re-export:

```rust
pub use arch::{boot, cpu, drivers, interrupt, serial, smp, thread, timer};
```

This makes the Task 5 bridge path `crate::kernel_lowlevel::boot::fatal_aarch64_sync_exception` valid without exposing the private `arch` module.

- [ ] **Step 4: Add the fail-closed architecture bridge**

In `syscall_dispatch.rs`, add the C ABI entry:

```rust
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub extern "C" fn handle_aarch64_lower_el_sync(
    saved_frame: usize,
    esr: u64,
    far: u64,
    return_pc: u64,
) {
    use crate::kernel_lowlevel::aarch64_exception_logic_shared::{
        aarch64_lower_el_sync, Aarch64El0MemoryAccess, Aarch64LowerElSync,
    };
    use crate::syscall::linux_process_memory::LinuxMemoryFaultAccess;

    let access = match aarch64_lower_el_sync(esr) {
        Aarch64LowerElSync::MemoryFault(fault) => match fault.access {
            Aarch64El0MemoryAccess::Read => LinuxMemoryFaultAccess::Read,
            Aarch64El0MemoryAccess::Write => LinuxMemoryFaultAccess::Write,
            Aarch64El0MemoryAccess::Execute => LinuxMemoryFaultAccess::Execute,
        },
        _ => crate::kernel_lowlevel::boot::fatal_aarch64_sync_exception(esr, far, return_pc),
    };
    if crate::syscall::deliver_linux_synchronous_memory_fault(
        saved_frame, return_pc, far, access,
    )
    .is_err()
    {
        crate::kernel_lowlevel::boot::fatal_aarch64_sync_exception(esr, far, return_pc);
    }
}
```

Import the exact shared policy type rather than duplicating a second access enum. This bridge is the final fail-closed boundary: a recognized abort either installs a handler, completes the existing process-exit transition, or halts with the original ESR/FAR/ELR diagnostics.

Rename the existing complete-frame handler to `lower_sync_a64`. Retain the SVC dispatch path. Replace label `99` with metadata forwarding:

```asm
mov     x0, sp
mrs     x1, esr_el1
mrs     x2, far_el1
mrs     x3, elr_el1
bl      handle_aarch64_lower_el_sync
b       restore_lower_el_frame
```

Only the SVC branch may call `syscall_should_advance_elr`; both handler delivery and launch-root kernel re-entry restore the frame and use the `ELR_EL1` chosen by Rust.

- [ ] **Step 5: Implement fatal synchronous diagnostics**

Add this Rust function below the assembly in `boot.rs`:

```rust
#[no_mangle]
pub extern "C" fn fatal_aarch64_sync_exception(esr: u64, far: u64, elr: u64) -> ! {
    let mut serial = super::serial::Serial::new();
    serial.init();
    serial.write_str("\n[AARCH64] fatal synchronous exception ESR=");
    serial.write_hex(esr);
    serial.write_str(" FAR=");
    serial.write_hex(far);
    serial.write_str(" ELR=");
    serial.write_hex(elr);
    serial.write_str("\n[ERROR] System halted\n");
    loop { super::cpu::wait_for_interrupt(); }
}
```

Do not call this function for a recognized lower-EL memory abort.

- [ ] **Step 6: Run contracts and warning-denied AArch64 build**

```bash
cargo test --manifest-path tests/host/Cargo.toml --test integration_contracts aarch64_synchronous_exception_vectors -- --nocapture
make ut it
make aarch64-warning-check
```

Expected: all host tests pass; the optimized AArch64 build, link, and layout checks exit zero with `-D warnings` and no warning output.

- [ ] **Step 7: Commit vector routing**

```bash
git add src/kernel_lowlevel/ARM64/boot.rs src/kernel_lowlevel/mod.rs src/syscall/syscall_dispatch.rs tests/host/tests/integration_contracts.rs
git commit -m "fix: route AArch64 EL0 memory faults to signals"
```

### Task 6: Run Repository and Formal Verification Gates

**Files:**
- Generated: `target/coverage/`
- Potentially modified by coverage audit: `docs/VERUS_COVERAGE.md`

- [ ] **Step 1: Format and inspect the complete change**

```bash
cargo fmt --manifest-path tests/host/Cargo.toml
cargo fmt --all -- --check
git diff --check
git status --short
```

Expected: format and whitespace checks pass; only intended source/test files or generated ignored artifacts appear.

- [ ] **Step 2: Run all host and tooling gates**

```bash
make host-fmt-check script-check launcher-test linker-layout-test
make ut it posix-tool-test
```

Expected: every command exits zero.

- [ ] **Step 3: Run warning-denied production build and Verus**

```bash
make aarch64-warning-check
make verus-kernel-lowlevel verus-syscall
make verus-coverage
```

Expected: AArch64 builds with no warnings; the affected low-level/syscall proof harnesses and coverage classification pass. If the coverage audit truthfully updates `docs/VERUS_COVERAGE.md`, review and commit only that tracked classification:

```bash
git add docs/VERUS_COVERAGE.md
git commit -m "docs: classify AArch64 fault verification"
```

Do not add `kernel8.img`, `host_shared/posixtest`, disk images, or `target` evidence.

### Task 7: Verify Focused AArch64 POSIX Behavior on Fresh Disks

**Files:**
- Generated: `target/posix/aarch64/el0-memory-fault-${commit}-disk-*.img`
- Generated: `target/posix/aarch64/el0-memory-fault-${commit}-run-*/`

- [ ] **Step 1: Build and verify the staged POSIX inventory**

```bash
make posix-stage
make aarch64-warning-check
pgrep -af qemu-system-aarch64 || true
```

Expected: the stage verifies against current pinned inputs and the kernel builds. Record any user-owned QEMU PIDs and disks. Never signal them and never read, mount, or modify repository-root `smros-fxfs.img`.

- [ ] **Step 2: Run `mmap/6-2.c` three times with one private disk per run**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
import subprocess
from scripts.posix.qemu_runner import run_smros

commit = subprocess.check_output(["git", "rev-parse", "--short=12", "HEAD"], text=True).strip()
for run in range(1, 4):
    disk = Path(f"target/posix/aarch64/el0-memory-fault-{commit}-disk-6-2-{run}.img")
    output = Path(f"target/posix/aarch64/el0-memory-fault-{commit}-run-6-2-{run}")
    assert not disk.exists() and not output.exists()
    subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M",
        test_id="conformance/interfaces/mmap/6-2.c",
    )
    assert result.complete and result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.launch_status == "launched"
    assert attempt.status == "pass", attempt
    assert attempt.exit_code == 0 and not attempt.timed_out
    assert not attempt.resource_deltas.has_positive(), attempt.resource_deltas
    print(run, attempt.status, attempt.duration_ms, result.result_path, result.raw_log_path)
PY
```

Expected: three genuine guest `test_end` events report `pts_status=pass`, no timeout/restart, and no positive terminal resource delta.

- [ ] **Step 3: Run adjacent protection and handler canaries**

Run each test in a separate fresh process and disk:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
import subprocess
from scripts.posix.qemu_runner import run_smros

tests = (
    "conformance/interfaces/mmap/6-1.c",
    "conformance/interfaces/mmap/6-3.c",
    "conformance/interfaces/mmap/11-2.c",
    "conformance/interfaces/mmap/11-3.c",
    "conformance/behavior/WIFEXITED/1-1.c",
    "conformance/behavior/WIFEXITED/1-2.c",
    "conformance/interfaces/fork/1-1.c",
)
commit = subprocess.check_output(["git", "rev-parse", "--short=12", "HEAD"], text=True).strip()
for index, test_id in enumerate(tests, 1):
    disk = Path(f"target/posix/aarch64/el0-memory-fault-{commit}-disk-canary-{index}.img")
    output = Path(f"target/posix/aarch64/el0-memory-fault-{commit}-run-canary-{index}")
    assert not disk.exists() and not output.exists()
    subprocess.run(["qemu-img", "create", "-f", "raw", str(disk), "128M"], check=True)
    result = run_smros(
        Path("host_shared/posixtest"), output,
        kernel=Path("kernel8.img"), disk=disk, memory="1024M", test_id=test_id,
    )
    assert result.complete and result.restart_count == 0 and len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.launch_status == "launched" and not attempt.timed_out
    assert not attempt.resource_deltas.has_positive(), attempt.resource_deltas
    print(test_id, attempt.status, attempt.exit_code, attempt.duration_ms, result.raw_log_path)
PY
```

Expected: `mmap/6-1.c`, `6-3.c`, `11-2.c`, and `11-3.c` pass. The installed `SIGBUS` handlers prove handler entry and return-to-`exit`; host wire-layout tests prove the passed `siginfo_t` and `ucontext_t` contain the exact fault address and saved AArch64 state. Wait/fork canaries terminate without timeout or restart; preserve and investigate any truthful non-pass status rather than changing expected results.

- [ ] **Step 4: Audit every focused result as structured JSON**

Parse every generated `results.ndjson` using `json.loads`. Require exact manifest/build/patch/implementation provenance, selected count equal terminal attempt count, zero guest restarts, no host watchdog, and absence of `fatal synchronous exception`, `KERNEL PANIC`, repeated abort output, allocator corruption, or stale-root diagnostics. Require non-positive deltas for Linux fds, mappings, processes, zombies, private/shared pages, page-table pages, scheduler threads, handles, IPC objects, AIO requests, and timers.

If any assertion fails, retain the evidence and return to the smallest failing RED test. Do not raise watchdog timeouts or relabel a result.

- [ ] **Step 5: Run the complete `mmap` API selection**

Create one new private disk and output directory, then call `run_smros(..., api="mmap")`. Derive the expected selected count from the verified manifest at execution time. Require campaign completion, zero restart, no timeout, no positive resource delta, and exact terminal accounting. Print every non-pass test ID, PTS status, exit code, and diagnostic; this campaign validates the affected API, not full POSIX compliance.

### Task 8: Capture Coverage, Coverity, and Final Evidence

**Files:**
- Generate: `target/posix/aarch64/el0-memory-fault-quality/`
- Create: `docs/posix/2026-08-13-aarch64-el0-memory-fault-results.md`

- [ ] **Step 1: Re-run final deterministic gates at the evidence commit**

```bash
make host-fmt-check script-check launcher-test linker-layout-test
make ut it posix-tool-test aarch64-warning-check
make verus-kernel-lowlevel verus-syscall verus-coverage
git diff --check
```

Expected: every command exits zero with no AArch64 warnings.

- [ ] **Step 2: Capture host coverage honestly**

```bash
quality="target/posix/aarch64/el0-memory-fault-quality"
test ! -e "$quality"
mkdir -p "$quality"
if command -v cargo-tarpaulin >/dev/null 2>&1; then
  set +e
  make coverage-host >"$quality/coverage-host.log" 2>&1
  printf '%s\n' "$?" >"$quality/coverage-host.exit"
  set -e
else
  printf '%s\n' 'cargo-tarpaulin is not installed' >"$quality/coverage-host.log"
  printf '%s\n' 'unavailable' >"$quality/coverage-host.exit"
fi
```

An absent tool is `unavailable`, not pass. An installed tool returning nonzero is `failed`. Extract the actual percentage and report path if successful; never invent 100%.

- [ ] **Step 3: Capture Coverity honestly**

```bash
quality="target/posix/aarch64/el0-memory-fault-quality"
missing=()
for tool in cov-build cov-analyze cov-format-errors; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -eq 0 ]; then
  covdir="$quality/coverity-capture"
  set +e
  cov-build --dir "$covdir" make aarch64-warning-check >"$quality/coverity.log" 2>&1
  capture=$?
  if [ "$capture" -eq 0 ]; then
    cov-analyze --dir "$covdir" --all >>"$quality/coverity.log" 2>&1
    analyze=$?
    if [ "$analyze" -eq 0 ]; then
      cov-format-errors --dir "$covdir" --json-output-v7 "$quality/coverity-results.json" >>"$quality/coverity.log" 2>&1
      format=$?
    else
      format=not-run
    fi
  else
    analyze=not-run
    format=not-run
  fi
  printf 'capture=%s\nanalyze=%s\nformat=%s\n' "$capture" "$analyze" "$format" >"$quality/coverity.status"
else
  printf 'missing Coverity commands: %s\n' "${missing[*]}" >"$quality/coverity.log"
  printf '%s\n' unavailable >"$quality/coverity.status"
fi
```

Count actual outstanding defects by checker and impact when JSON exists. Never report an unavailable run as zero defects.

- [ ] **Step 4: Write the results document from generated evidence**

Create `docs/posix/2026-08-13-aarch64-el0-memory-fault-results.md` with:

```text
implementation commit and dirty-state status
design and plan commits
toolchain and QEMU versions
manifest/build/patch/source revision hashes
three mmap/6-2 attempts with duration and resource deltas
all adjacent canary statuses and diagnostics
complete mmap API selected/terminal/status counts
host, Python, warning-denied, and Verus command outcomes
Tarpaulin status and measured percentage or unavailable reason
Coverity status and actual defect counts or unavailable reason
explicit statement that this is affected-surface evidence, not full POSIX certification
```

Every number must be derived from logs or structured results. Link artifact paths under `target` without adding them to Git.

- [ ] **Step 5: Review, commit, and run the final completion audit**

```bash
git diff --check
git add docs/posix/2026-08-13-aarch64-el0-memory-fault-results.md
git commit -m "docs: record AArch64 EL0 fault results"
git status --short --branch
git log -8 --oneline --decorate
```

Compare the implementation and evidence line by line with all seven acceptance criteria in the approved design. Any timeout, missing handler behavior, wrong signal/code/address, positive resource delta, warning, failing host/formal gate, unverifiable provenance, or missing truthful quality status keeps the work incomplete.
