# Memory Syscalls: Current Implementation Status

This document tracks the memory-related syscall behavior implemented in the current tree. It focuses on what the code actually does today, not the eventual Linux or Zircon compatibility target.

## Scope

Relevant code lives in:

- `src/syscall/syscall.rs`
- `src/kernel_objects/vmo.rs`
- `src/kernel_objects/vmar.rs`
- `src/kernel_lowlevel/memory.rs`
- `src/kernel_lowlevel/mmu.rs`

## Summary

SMROS currently has:

- a real bitmap-based physical page-frame allocator
- a simple per-process address-space model with fixed segments
- partially implemented Linux memory syscalls
- partially implemented Zircon VMO/VMAR syscalls

Most object constructors allocate real pages. Many syscall wrappers still return placeholder values or maintain software bookkeeping without changing live process mappings.

## Linux Memory Syscalls

| Syscall | Current Behavior | Status |
|---------|------------------|--------|
| `sys_mmap` | supports anonymous mappings only; allocates a paged VMO and returns a simple virtual address | partial |
| `sys_munmap` | validates page alignment and returns success without removing real mappings | placeholder |
| `sys_mprotect` | parses protection flags and returns success without changing mappings | placeholder |
| `sys_brk` | tracks a global heap window at `0x4000_0000..0x400F_FFFF` and allocates page frames on growth | partial |
| `sys_mremap` | returns the old address for shrink-in-place or allocates a new paged VMO and returns a synthetic new address | partial |

### `sys_mmap`

Current properties:

- only anonymous mappings are accepted
- `MAP_SHARED | MAP_ANONYMOUS` is rejected
- file-backed mappings return `ENOSYS`
- the returned address is synthetic rather than derived from a real process mapping database

This is enough for the current smoke tests, but it is not yet a full process mapping implementation.

### `sys_brk`

Current properties:

- uses one global break pointer for the whole kernel
- initializes on first use
- allocates physical pages through `PageFrameAllocator` when the break grows
- does not model a separate heap per process yet

### `sys_mremap`

Current properties:

- validates addresses and sizes for page alignment
- uses a simple relocate-or-return-old strategy
- does not perform real copy-on-remap or page-table edits

## Zircon VMO Syscalls

| Syscall | Current Behavior | Status |
|---------|------------------|--------|
| `sys_vmo_create` | creates paged, resizable, or contiguous VMOs; "physical" option currently falls back to the regular paged constructor in the syscall wrapper | partial |
| `sys_vmo_read` | returns the buffer length; does not read real data through a handle lookup | placeholder |
| `sys_vmo_write` | returns the buffer length; does not write through a handle lookup | placeholder |
| `sys_vmo_get_size` | reports `PAGE_SIZE` as a placeholder | placeholder |
| `sys_vmo_set_size` | returns success without resizing a handle-backed object | placeholder |
| `sys_vmo_op_range` | validates `Commit` and `Decommit`, accepts `Zero`, and leaves other ops unimplemented | partial |

## Zircon VMAR Syscalls

| Syscall | Current Behavior | Status |
|---------|------------------|--------|
| `sys_vmar_map` | validates options, rounds size, and returns an output address | bookkeeping |
| `sys_vmar_unmap` | returns success without editing real mappings | placeholder |
| `sys_vmar_protect` | validates options and returns success | placeholder |
| `sys_vmar_allocate` | validates options and size, then returns zeroed child outputs | placeholder |
| `sys_vmar_destroy` | returns success | placeholder |
| `sys_vmar_unmap_handle_close_thread_exit` | validates alignment and returns success without deferred cleanup | placeholder |

## What The Object Layer Can Do

The object layer in `src/kernel_objects/` is more capable than some of the syscall wrappers that sit above it.

### `Vmo`

`src/kernel_objects/vmo.rs` supports:

- paged VMOs
- resizable VMOs
- physical VMOs
- contiguous VMOs
- resize via `set_len()`
- child/slice creation
- cache-policy changes

Important distinction:

- the object constructors allocate page frames
- the syscall wrappers do not yet manage a complete handle-backed VMO registry

### `Vmar`

`src/kernel_objects/vmar.rs` supports:

- software tracking of mappings
- subregion allocation
- overwrite-aware mapping helper
- permission bookkeeping

But it still acts as a software model, not the definitive runtime mapping authority for the kernel.

## Underlying Memory Substrate

### Physical Memory

`PageFrameAllocator` in `src/kernel_lowlevel/memory.rs` tracks:

- 4096 physical pages
- 4 KiB page size
- 16 MiB total managed space

### Per-Process Address Space Model

Each `ProcessAddressSpace` currently uses four fixed segments:

| Segment | Base | Size |
|---------|------|------|
| Code | `0x0000` | 1 page |
| Data | `0x1000` | 1 page |
| Heap | `0x2000` | 4 pages |
| Stack | `0xF000` | 2 pages |

This is the address-space model used by the process manager in `src/kernel_lowlevel/memory.rs`.

### MMU Helpers

`src/kernel_lowlevel/mmu.rs` contains:

- page-table entry definitions
- TTBR0/TTBR1 management
- user and kernel region mapping helpers

Those helpers are real scaffolding for EL0 work, but they are not yet the engine behind the current shell/process demo path.

## What Is Actually Exercised Today

The current boot flow exercises only a small part of the memory syscall surface:

- `run_user_test()` directly calls `sys_getpid()` and `sys_mmap()` from kernel mode
- the shell's `testsc` command directly calls `sys_getpid()` and `sys_mmap()`
- the shell also contains a lightweight `test_write()` smoke path through `svc`

That means the live demo validates the shape of the interfaces more than full address-space correctness.

## Practical Interpretation

Today the memory syscall layer should be read as:

- real allocator underneath
- useful object-model scaffolding
- partial syscall surface
- incomplete runtime wiring

It is suitable for kernel bring-up and interface development, but it is not yet a complete Linux `mmap` implementation or a complete Zircon VMO/VMAR subsystem.

## Known Gaps

- no real file-backed `mmap`
- no real `munmap` teardown of live process mappings
- no per-process `brk`
- no handle-backed VMO registry in the syscall wrapper layer
- no VMAR-to-hardware page-table synchronization in the live boot path
- special thread-exit VMAR unmap semantics are not implemented yet
