# Kernel Objects Directory

This document describes the current `src/kernel_objects/` layout and how those modules are used by the live kernel.

## Directory Layout

```text
src/kernel_objects/
├── mod.rs
├── channel.rs
├── compat.rs
├── handle.rs
├── scheduler.rs
├── thread.rs
├── types.rs
├── vmar.rs
└── vmo.rs
```

## File Responsibilities

| File | Purpose | Current Role |
|------|---------|--------------|
| `mod.rs` | module declarations, light re-exports, global `KernelObjectManager` | owns the global handle-table wrapper |
| `thread.rs` | `CpuContext`, `ThreadState`, `ThreadId`, `ThreadControlBlock`, stack helpers | live thread object model used by the scheduler |
| `scheduler.rs` | global scheduler, idle thread, context-switch entry points, tick accounting | live scheduling path used to start the shell |
| `types.rs` | shared handle/object enums, rights, VM flags, Zircon-style errors, page helpers | shared definitions for the syscall and object layers |
| `handle.rs` | fixed-size handle table and handle duplication/removal helpers | currently a simple in-kernel table, not a full per-process capability system |
| `compat.rs` | lightweight compatibility objects with handle lifetime, peer links, signals, properties, and byte queues | backs Zircon object interfaces that do not yet have full subsystems |
| `vmo.rs` | VMO constructors and operations | backing object model for memory syscalls |
| `vmar.rs` | VMAR bookkeeping and mapping/protection helpers | bookkeeping layer for Zircon-style VM regions |
| `channel.rs` | channel object, global channel table, channel syscall wrappers | live channel subsystem initialization plus syscall helpers |

## `mod.rs`

`mod.rs` currently:

- declares all object modules
- re-exports:
  - `types::*`
  - `handle::*`
  - `vmo::*`
  - `scheduler::*`
- owns `KernelObjectManager`
- exposes a single global `HandleTable`

Important detail: `vmar`, `channel`, and `thread` are public modules, but they are not fully re-exported from `mod.rs`. Code that needs them normally imports them via their module paths.

## `thread.rs`

`thread.rs` is the ABI-sensitive part of the directory.

It defines:

- `ThreadState`
- `ThreadId`
- `CpuContext`
- `ThreadControlBlock`
- stack utilities used by the scheduler

`CpuContext` and `ThreadControlBlock` must stay layout-compatible with `src/kernel_lowlevel/context_switch.S`. That file saves and restores fields based on fixed offsets.

## `scheduler.rs`

`scheduler.rs` implements the live thread scheduler.

Main responsibilities:

- create the idle thread
- create worker threads
- track the current thread and next runnable thread
- account for scheduler ticks
- perform `context_switch()` and `context_switch_start()`
- expose CPU-aware helpers such as `schedule_on_cpu()`

This is one of the most active modules in the current kernel: the shell reaches users through this scheduler path.

## `types.rs`

`types.rs` contains the shared object vocabulary:

- `HandleValue`
- `ObjectType`
- `Rights`
- `VmOptions`
- `MmuFlags`
- `VmoCloneFlags`
- `VmarFlags`
- `VmoType`
- `VmoOpType`
- `CachePolicy`
- `ZxError`
- `ZxResult`
- `pages()` and `roundup_pages()`

These definitions are used both by the object layer and by `src/syscall/syscall.rs`.

`ObjectType` now includes the sample Zircon object families (`EventPair`, `Fifo`, `Stream`, `DebugLog`, `Clock`, `Job`, `SuspendToken`, `Exception`, `Iommu`, `Bti`, `Pmt`, `PciDevice`, `Guest`, `Vcpu`, `Profile`, `Pager`, framebuffer/trace objects) and the Linux object families needed by the sample tree (`LinuxFile`, `LinuxDir`, `LinuxPipe`, TCP/UDP/raw/netlink sockets, `EventFd`, `SignalFd`, `TimerFd`, `Inotify`, `MemFd`, `PidFd`, `Futex`, Linux process/thread/signal/event/device categories, IPC/semaphore/shared-memory/message-queue categories).

`Rights` follows the Zircon handle-right bit layout, including policy, destroy, inspect, task-management, VMAR child-operation, VMO resize, and VMO-management bits. The syscall model keeps object identity separate from handle rights for memory/task objects. Duplication is a capability operation that requires `Duplicate` and a valid subset or `RIGHT_SAME_RIGHTS`; replacement consumes the source handle and only requires the requested rights to be valid and non-escalating.

## `handle.rs`

`handle.rs` currently provides a simple fixed-size handle table with:

- `add()`
- `add_existing()`
- `remove()`
- `get_rights()`
- `get_object_type()`
- `has_rights()`
- `contains()`
- `duplicate()`
- `replace()`

This is intentionally simple and currently closer to a global kernel utility than a complete per-process capability implementation.

## `compat.rs`

`compat.rs` provides a small global compatibility object table for interface coverage. It supports:

- single objects and peer pairs
- handle close and existence checks
- signal update/read state
- one `u64` property value per object
- one `u64` state value per object for modeled timer, clock, guest, VCPU, and similar state
- option bits recorded at object creation
- bounded byte queues for modeled sockets, FIFOs, streams, Linux files, Linux pipes, debug logs, message queues, and Linux socket categories

This module is deliberately not a substitute for full subsystems such as PCI, interrupts, networking, or a real per-process capability model. It gives common Linux/Zircon syscall entrypoints deterministic behavior while unsupported platform-heavy calls still return explicit unsupported errors.

Linux file and directory fds use this table through `src/syscall/syscall.rs`:

- `openat()` creates either a `LinuxFile` or `LinuxDir` compatibility object.
- the Linux fd table stores the compat handle plus readable/writable bits.
- `read()` and `write()` move bytes through the object's bounded queue.
- `dup`, `dup3`, and `fcntl` duplicate or validate fd records while preserving the underlying handle.
- `close()` removes one fd record and closes the compat handle only when no fd still references it.
- `getdents64()` requires `LinuxDir` and currently returns an empty zeroed directory buffer.
- stat and statfs helpers validate pointers and zero output buffers instead of reading real inode metadata.

## `vmo.rs`

`vmo.rs` supports:

- paged VMOs
- resizable VMOs
- physical VMOs
- contiguous VMOs
- size queries and resize
- child and slice creation
- placeholder read/write/zero/commit behavior

The constructors do perform real page-frame allocation through `PageFrameAllocator`, but many higher-level operations remain lightweight or placeholder-only.

## `vmar.rs`

`vmar.rs` manages a software model of virtual regions:

- base address and size
- region allocation
- mappings
- protection bookkeeping
- subregion allocation
- destroy/unmap helpers

Today this is primarily a bookkeeping layer. It is not yet a full, live mirror of the actual page-table state used by the running shell/process demo.

## `channel.rs`

`channel.rs` includes:

- `Channel`
- `ChannelMessage`
- `ChannelTable`
- global `CHANNEL_TABLE`
- syscall helpers:
  - `sys_channel_create`
  - `sys_channel_read`
  - `sys_channel_write`
  - `sys_channel_call_noretry`

`kernel_main()` calls `channel::init()`, so the subsystem is part of the live boot path. The syscall wrappers are routed through `dispatch_zircon_syscall()` and can also be reached from the active SVC bridge with syscall numbers `1000 + zircon_number`.

## How The Directory Is Used Today

### Live Boot Path

The current boot path directly uses:

- `scheduler.rs` to create and run the shell thread
- `thread.rs` for TCB/context layout
- `channel.rs` for subsystem initialization

### Syscall Layer

`src/syscall/syscall.rs` depends on:

- `types.rs`
- `handle.rs`
- `vmo.rs`
- `vmar.rs`
- `channel.rs`
- `compat.rs`

### User-Level Scaffolding

`src/user_level/` depends on:

- `scheduler.rs` for thread creation
- `thread.rs` for user-thread context scaffolding
- `src/user_level/services/component.rs` for a minimal Fuchsia-style component topology
- `src/user_level/services/elf.rs` for minimal ELF64/AArch64 boot-component parsing
- `src/user_level/services/fxfs.rs` for an in-memory FxFS-shaped object store used by the component namespace scaffold
- `src/user_level/services/svc.rs` for a minimal `/svc` directory using Zircon channels and fixed request/reply structs

## Current Design Reality

The refactor into a dedicated `kernel_objects/` directory is complete at the source layout level. The runtime wiring is more selective:

- threads and the scheduler are live
- channels are initialized and usable as kernel objects
- compatibility objects back events, eventpairs, sockets, FIFOs, ports, timers, clocks, debug logs, resources, streams, suspend tokens, jobs, exceptions, profiles, pagers, IOMMU/BTI/PMT, interrupts, PCI devices, guests, VCPUs, and modeled Linux file/directory/pipe/socket/IPC descriptor objects at an interface level
- VMO/VMAR objects exist and back syscall helpers
- user-level component, ELF loader, `/svc`, and FxFS scaffolding exists under `src/user_level/`, not `src/kernel_objects/`
- handle management remains simplified
- object-to-process ownership is not yet fully modeled

## Current Limitations

- The global handle table is not yet a full per-process handle namespace.
- Channel syscall helpers are exposed through the current Zircon dispatch table.
- VMAR state is not yet a full source of truth for hardware mappings.
- Linux file and directory compatibility objects are not a real VFS; they provide fd/object behavior for syscall bring-up.
- FxFS is currently an in-memory userspace scaffold with object attributes, explicit directory entries, and journal replay metadata, not a block-backed kernel object subsystem.
- Several object operations are placeholders intended to keep the interface shape stable while the kernel matures.
