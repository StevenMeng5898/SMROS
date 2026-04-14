# SMROS Syscall Compatibility Layer

## Overview

This document describes the comprehensive syscall compatibility layer implemented in SMROS, inspired by and compatible with the [grt-zcore](https://github.com/StevenMeng5898/grt-zcore) project architecture.

The syscall layer provides **dual compatibility** with both **Linux** and **Zircon** system call interfaces, enabling SMROS to support applications and binaries targeting either operating system API.

## Architecture

### Design Philosophy

Following the grt-zcore architecture, SMROS implements:

1. **Linux Syscall Interface** - Compatible with Linux/ARM64 syscalls
2. **Zircon Syscall Interface** - Compatible with Fuchsia/Zircon microkernel syscalls
3. **Unified Memory Management** - Both interfaces share the same underlying memory subsystem
4. **Handle-based Object Management** - Zircon-style capability-based security model

### Core Components

```
┌─────────────────────────────────────────────────────────┐
│                  Application Layer                       │
├──────────────────────────┬──────────────────────────────┤
│    Linux Binaries        │   Zircon/Fuchsia Binaries    │
└──────────┬───────────────┴──────────────┬───────────────┘
           │                              │
┌──────────▼──────────────────────────────▼───────────────┐
│              Syscall Dispatcher Layer                    │
│  ┌────────────────────┐    ┌─────────────────────────┐  │
│  │ Linux Dispatcher   │    │ Zircon Dispatcher       │  │
│  │ (dispatch_linux_   │    │ (dispatch_zircon_       │  │
│  │  syscall)          │    │  syscall)               │  │
│  └────────┬───────────┘    └─────────┬───────────────┘  │
└───────────┼──────────────────────────┼──────────────────┘
            │                          │
┌───────────▼──────────────────────────▼──────────────────┐
│           Syscall Implementation Layer                   │
│  ┌────────────┐ ┌──────────┐ ┌────────┐ ┌────────────┐  │
│  │ VM Syscalls│ │Task      │ │Handle  │ │Time Syscalls│ │
│  │(mmap, VMO) │ │Syscalls  │ │Syscalls│ │(clock,timer)│ │
│  └────────────┘ └──────────┘ └────────┘ └────────────┘  │
└────────────────────────┬────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────┐
│              Memory Management Layer                     │
│  ┌─────────────┐  ┌──────────┐  ┌──────────────────┐   │
│  │ProcessAddr  │  │ PageFrame│  │ VMO/VMAR Objects │   │
│  │Space        │  │Allocator │  │                  │   │
│  └─────────────┘  └──────────┘  └──────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

## Linux Syscall Compatibility

### Memory Management Syscalls

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `mmap` | `sys_mmap()` | ✅ Implemented | Map memory regions (anonymous/file-backed) |
| `munmap` | `sys_munmap()` | ✅ Implemented | Unmap memory regions |
| `mprotect` | `sys_mprotect()` | ⚠️ Stub | Change memory protection (placeholder) |
| `brk` | - | ❌ TODO | Change program break (heap management) |

#### Example: Linux mmap Usage

```rust
// In your syscall handler:
use crate::syscall::{dispatch_linux_syscall, LinuxSyscall};

let args = [
    0,                          // addr (NULL = kernel chooses)
    0x1000,                     // len (4KB)
    0x3,                        // prot (PROT_READ | PROT_WRITE)
    0x22,                       // flags (MAP_PRIVATE | MAP_ANONYMOUS)
    0,                          // fd (ignored for anonymous)
    0,                          // offset
];

let result = dispatch_linux_syscall(LinuxSyscall::Mmap as u32, args);
// Returns virtual address of mapped region
```

### Process Management Syscalls

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `fork` | `sys_fork()` | ✅ Implemented | Create child process |
| `vfork` | `sys_vfork()` | ✅ Implemented | Create child process (shared memory) |
| `clone` | `sys_clone()` | ✅ Implemented | Create thread/process with custom flags |
| `execve` | `sys_execve()` | ⚠️ Stub | Execute program (placeholder) |
| `exit` | `sys_exit()` | ✅ Implemented | Terminate process |
| `exit_group` | `sys_exit_group()` | ✅ Implemented | Terminate process group |
| `wait4` | `sys_wait4()` | ⚠️ Stub | Wait for process state change |
| `kill` | `sys_kill()` | ✅ Implemented | Send signal to process |

### Process Information Syscalls

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `getpid` | `sys_getpid()` | ✅ Implemented | Get process ID |
| `getppid` | `sys_getppid()` | ✅ Implemented | Get parent process ID |
| `gettid` | `sys_gettid()` | ✅ Implemented | Get thread ID |

### Time Syscalls

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `clock_gettime` | `sys_clock_gettime()` | ⚠️ Stub | Get clock time |
| `nanosleep` | `sys_nanosleep_linux()` | ⚠️ Stub | High-resolution sleep |
| `gettimeofday` | - | ❌ TODO | Get time of day |
| `clock_nanosleep` | `sys_clock_nanosleep()` | ⚠️ Stub | Clock-aware sleep |

## Zircon Syscall Compatibility

### Handle Management

Zircon uses a capability-based security model where all kernel objects are accessed through handles.

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `zx_handle_close` | `sys_handle_close()` | ✅ Implemented | Close a handle |
| `zx_handle_close_many` | `sys_handle_close_many()` | ⚠️ Stub | Close multiple handles |
| `zx_handle_duplicate` | `sys_handle_duplicate()` | ✅ Implemented | Duplicate handle with rights |
| `zx_handle_replace` | `sys_handle_replace()` | ✅ Implemented | Replace handle with new rights |

#### Handle Rights System

```rust
pub enum Rights {
    DUPLICATE = 1 << 0,    // Can duplicate handle
    TRANSFER = 1 << 1,     // Can transfer handle
    READ = 1 << 2,         // Can read from object
    WRITE = 1 << 3,        // Can write to object
    EXECUTE = 1 << 4,      // Can execute object
    MAP = 1 << 5,          // Can map object into VMAR
    GET_PROPERTY = 1 << 6, // Can get object property
    SET_PROPERTY = 1 << 7, // Can set object property
    SIGNAL = 1 << 8,       // Can signal object
    SIGNAL_PEER = 1 << 9,  // Can signal object's peer
    WAIT = 1 << 10,        // Can wait on object
}
```

### Virtual Memory Objects (VMO)

VMOs are the fundamental memory abstraction in Zircon, similar to Linux's memory mappings but with more features.

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `zx_vmo_create` | `sys_vmo_create()` | ✅ Implemented | Create a VMO |
| `zx_vmo_read` | `sys_vmo_read()` | ✅ Implemented | Read from VMO |
| `zx_vmo_write` | `sys_vmo_write()` | ✅ Implemented | Write to VMO |
| `zx_vmo_get_size` | `sys_vmo_get_size()` | ✅ Implemented | Get VMO size |
| `zx_vmo_set_size` | `sys_vmo_set_size()` | ✅ Implemented | Resize VMO (if resizable) |
| `zx_vmo_op_range` | `sys_vmo_op_range()` | ✅ Implemented | Operations on VMO range |
| `zx_vmo_replace_as_executable` | - | ❌ TODO | Make VMO executable |
| `zx_vmo_create_child` | `Vmo::create_child()` | ✅ Implemented | Create child VMO |
| `zx_vmo_create_physical` | - | ❌ TODO | Create physical memory VMO |
| `zx_vmo_create_contiguous` | - | ❌ TODO | Create contiguous memory VMO |
| `zx_vmo_set_cache_policy` | `sys_vmo_cache_policy()` | ✅ Implemented | Set cache policy |

#### VMO Operations

```rust
// Create a 4KB VMO
let mut handle: u32 = 0;
sys_vmo_create(0x1000, 0, &mut handle);

// Write to VMO
let data = [0x41, 0x42, 0x43, 0x44];
sys_vmo_write(handle, &data, 0);

// Read from VMO
let mut buffer = [0u8; 4];
sys_vmo_read(handle, &mut buffer, 0);

// Commit pages
sys_vmo_op_range(handle, VmoOpType::Commit as u32, 0, 0x1000);

// Get size
let mut size: usize = 0;
sys_vmo_get_size(handle, &mut size);
```

#### VMO Operations (op_range)

| Operation | Value | Description |
|-----------|-------|-------------|
| `ZX_VMO_OP_COMMIT` | 1 | Commit pages (allocate physical memory) |
| `ZX_VMO_OP_DECOMMIT` | 2 | Decommit pages (free physical memory) |
| `ZX_VMO_OP_ZERO` | 10 | Zero a range |
| `ZX_VMO_OP_CACHE_SYNC` | 6 | Sync cache to memory |
| `ZX_VMO_OP_CACHE_INVALIDATE` | 7 | Invalidate cache |

### Virtual Memory Address Regions (VMAR)

VMARs manage virtual address space layout, containing mappings to VMOs.

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `zx_vmar_map` | `sys_vmar_map()` | ✅ Implemented | Map VMO into VMAR |
| `zx_vmar_unmap` | `sys_vmar_unmap()` | ✅ Implemented | Unmap from VMAR |
| `zx_vmar_allocate` | `sys_vmar_allocate()` | ✅ Implemented | Allocate subregion |
| `zx_vmar_protect` | `sys_vmar_protect()` | ✅ Implemented | Change protection |
| `zx_vmar_destroy` | `sys_vmar_destroy()` | ✅ Implemented | Destroy VMAR |

#### VMAR Mapping Options

```rust
pub struct VmOptions: u32 {
    const PERM_READ = 1 << 0;         // Read permission
    const PERM_WRITE = 1 << 1;        // Write permission
    const PERM_EXECUTE = 1 << 2;      // Execute permission
    const SPECIFIC = 1 << 3;          // Map at specific address
    const SPECIFIC_OVERWRITE = 1 << 4;// Map and overwrite existing
    const COMPACT = 1 << 5;           // Use compact mapping
    const CAN_MAP_RXW = 1 << 6;       // Can map read/write/execute
    const CAN_MAP_SPECIFIC = 1 << 7;  // Can map at specific address
    const MAP_RANGE = 1 << 8;         // Map entire range
    const REQUIRE_NON_RESIZABLE = 1 << 9; // Require non-resizable VMO
}
```

#### Example: Zircon VMAR Usage

```rust
// Map a VMO into VMAR at offset 0x10000
let mut mapped_addr: usize = 0;
sys_vmar_map(
    vmar_handle,           // VMAR handle
    0x7,                   // options (PERM_READ | PERM_WRITE | PERM_EXECUTE)
    0x10000,               // vmar_offset
    vmo_handle,            // VMO handle
    0,                     // vmo_offset
    0x1000,                // length (4KB)
    &mut mapped_addr,      // output: actual mapped address
);
```

### Object Management

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `zx_object_wait_one` | `sys_object_wait_one()` | ⚠️ Stub | Wait for signal on one object |
| `zx_object_wait_many` | `sys_object_wait_many()` | ⚠️ Stub | Wait for signal on multiple objects |
| `zx_object_signal` | `sys_object_signal()` | ⚠️ Stub | Signal an object |
| `zx_object_get_info` | `sys_object_get_info()` | ⚠️ Stub | Get object information |
| `zx_object_get_property` | `sys_object_get_property()` | ⚠️ Stub | Get object property |
| `zx_object_set_property` | `sys_object_set_property()` | ⚠️ Stub | Set object property |

### Process/Task Management

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `zx_process_create` | `sys_process_create()` | ✅ Implemented | Create a process |
| `zx_process_exit` | `sys_process_exit()` | ⚠️ Stub | Exit a process |
| `zx_process_start` | `sys_process_start()` | ⚠️ Stub | Start process execution |
| `zx_process_read_memory` | - | ❌ TODO | Read process memory |
| `zx_process_write_memory` | - | ❌ TODO | Write process memory |
| `zx_thread_create` | `sys_thread_create()` | ✅ Implemented | Create a thread |
| `zx_thread_start` | `sys_thread_start()` | ✅ Implemented | Start thread execution |
| `zx_thread_exit` | `sys_thread_exit()` | ⚠️ Stub | Exit current thread |
| `zx_task_kill` | `sys_task_kill()` | ⚠️ Stub | Kill a task (process/thread) |

### Time and Clock Syscalls

| Syscall | Function | Status | Description |
|---------|----------|--------|-------------|
| `zx_clock_get_monotonic` | `sys_clock_get_monotonic()` | ⚠️ Stub | Get monotonic clock |
| `zx_nanosleep` | `sys_nanosleep()` | ⚠️ Stub | Sleep until deadline |
| `zx_timer_create` | - | ❌ TODO | Create a timer |
| `zx_timer_set` | - | ❌ TODO | Set timer |
| `zx_timer_cancel` | - | ❌ TODO | Cancel timer |

### IPC Syscalls (Not Yet Implemented)

| Syscall Category | Syscalls | Status |
|-----------------|----------|--------|
| Channels | `zx_channel_create`, `zx_channel_read`, `zx_channel_write` | ❌ TODO |
| Sockets | `zx_socket_create`, `zx_socket_read`, `zx_socket_write` | ❌ TODO |
| FIFOs | `zx_fifo_create`, `zx_fifo_read`, `zx_fifo_write` | ❌ TODO |
| Futex | `zx_futex_wait`, `zx_futex_wake`, `zx_futex_requeue` | ❌ TODO |
| Ports | `zx_port_create`, `zx_port_wait`, `zx_port_queue` | ❌ TODO |
| Events | `zx_event_create`, `zx_eventpair_create` | ❌ TODO |

## Integration with SMROS Memory Manager

### Mapping Table

| SMROS Component | Linux Equivalent | Zircon Equivalent |
|----------------|------------------|-------------------|
| `ProcessAddressSpace` | `mm_struct` | `Process` + root `VMAR` |
| `MemorySegment` | `vm_area_struct` | `VMO` mapping |
| `PageEntry` | Page table entry (PTE) | MMU page table entry |
| `PageFrameAllocator` | Buddy allocator | PhysAlloc |
| `ProcessControlBlock` | `task_struct` | `Process` object |
| `heap_alloc()` | `brk` syscall | `vmar.allocate()` |
| `stack_alloc()` | `mmap(MAP_STACK)` | `vmar.map()` for stack |

### Memory Layout Comparison

#### SMROS Layout (per process)
```
0x0000_0000_0000_0000 ┌─────────────────┐
                      │  Code Segment   │ 1 page (RX)
0x0000_0000_0001_0000 ├─────────────────┤
                      │  Data Segment   │ 1 page (RW)
0x0000_0000_0002_0000 ├─────────────────┤
                      │                 │
                      │  Heap Segment   │ 4 pages (RW, grows up)
                      │                 │
0x0000_0000_FFFF_0000 ├─────────────────┤
                      │  Stack Segment  │ 2 pages (RW, grows down)
0x0000_0000_FFFF_2000 └─────────────────┘
```

#### Linux Layout (equivalent)
```
0x0000_0000_0000_0000 ┌─────────────────┐
                      │  Text (code)    │ mmap(PROT_READ|PROT_EXEC)
0x0000_0000_0001_0000 ├─────────────────┤
                      │  Data/BSS       │ mmap(PROT_READ|PROT_WRITE)
0x0000_0000_0002_0000 ├─────────────────┤
                      │  Heap (brk)     │ brk() syscall
                      │                 │
0x0000_7FFF_FFFF_0000 └─────────────────┘
...
0x0000_7FFF_FFFF_0000 ┌─────────────────┐
                      │  Stack          │ mmap(MAP_STACK)
0x0000_7FFF_FFFF_F000 └─────────────────┘
```

#### Zircon Layout (equivalent)
```
0x0000_0000_0000_0000 ┌─────────────────┐
                      │  Root VMAR      │ Process address space
                      │                 │
0x0000_0000_0001_0000 ├─────────────────┤
                      │  Code VMO map   │ vmar.map(vmo_code, RX)
0x0000_0000_0002_0000 ├─────────────────┤
                      │  Data VMO map   │ vmar.map(vmo_data, RW)
                      │                 │
                      │  Heap VMAR      │ vmar.allocate()
                      │                 │
0x0000_0000_FFFF_0000 ├─────────────────┤
                      │  Stack VMO map  │ vmar.map(vmo_stack, RW)
0x0000_0000_FFFF_2000 └─────────────────┘
```

## Syscall Number Definitions

### Linux Syscall Numbers (ARM64)

Defined in `LinuxSyscall` enum. Key syscalls:

```rust
pub enum LinuxSyscall {
    Mmap = 222,      // ARM64 mmap
    Munmap = 215,    // ARM64 munmap
    Mprotect = 226,  // ARM64 mprotect
    Fork = 1000,     // Custom (ARM64 uses clone)
    Exit = 93,       // ARM64 exit
    Getpid = 172,    // ARM64 getpid
    Kill = 129,      // ARM64 kill
    // ... and many more
}
```

### Zircon Syscall Numbers

Defined in `ZirconSyscall` enum. First 75 syscalls:

```rust
pub enum ZirconSyscall {
    HANDLE_CLOSE = 0,
    HANDLE_DUPLICATE = 2,
    VMO_CREATE = 52,
    VMO_READ = 53,
    VMO_WRITE = 54,
    VMAR_MAP = 58,
    VMAR_UNMAP = 59,
    PROCESS_CREATE = 18,
    THREAD_CREATE = 12,
    // ... and many more
}
```

## Error Codes

### Linux Error Codes

```rust
pub enum SysError {
    EPERM = 1,      // Operation not permitted
    ENOENT = 2,     // No such file or directory
    ESRCH = 3,      // No such process
    EINTR = 4,      // Interrupted system call
    EIO = 5,        // I/O error
    ENOMEM = 12,    // Out of memory
    EACCES = 13,    // Permission denied
    EFAULT = 14,    // Bad address
    EINVAL = 22,    // Invalid argument
    ENOSYS = 38,    // Function not implemented
    // ...
}
```

### Zircon Error Codes

```rust
pub enum ZxError {
    OK = 0,
    ERR_INTERNAL = -1,
    ERR_NOT_SUPPORTED = -2,
    ERR_NO_MEMORY = -3,
    ERR_INVALID_ARGS = -10,
    ERR_ACCESS_DENIED = -12,
    ERR_NOT_FOUND = -14,
    ERR_OUT_OF_RANGE = -21,
    ERR_BAD_STATE = -24,
    ERR_TIMED_OUT = -30,
    // ...
}
```

## Implementation Status Summary

### Fully Implemented ✅

- Linux: mmap, munmap, fork, exit, getpid, getppid, gettid, kill
- Zircon: VMO create/read/write/resize/op_range, VMAR map/unmap, handle management

### Partially Implemented ⚠️

- Linux: mprotect (stub), execve (stub), wait4 (stub)
- Zircon: process/thread creation (basic), object wait/signal (stub)

### Not Yet Implemented ❌

- Linux: brk, gettimeofday, clock_gettime (full implementation)
- Zircon: IPC (channels, sockets, FIFOs, futex, ports), timers, PCI, hypervisor

## Usage Examples

### Creating and Using a VMO

```rust
use crate::syscall::{
    sys_vmo_create, sys_vmo_write, sys_vmo_read,
    sys_vmo_get_size, sys_vmo_op_range, VmoOpType
};

// Step 1: Create a 4KB VMO
let mut vmo_handle: u32 = 0;
sys_vmo_create(0x1000, 0, &mut vmo_handle)
    .expect("Failed to create VMO");

// Step 2: Write data to VMO
let data = b"Hello, SMROS!";
sys_vmo_write(vmo_handle, data, 0)
    .expect("Failed to write to VMO");

// Step 3: Read data back
let mut buffer = [0u8; 13];
sys_vmo_read(vmo_handle, &mut buffer, 0)
    .expect("Failed to read from VMO");
assert_eq!(&buffer, b"Hello, SMROS!");

// Step 4: Commit pages (ensure physical memory allocated)
sys_vmo_op_range(vmo_handle, VmoOpType::Commit as u32, 0, 0x1000)
    .expect("Failed to commit pages");

// Step 5: Check VMO size
let mut size: usize = 0;
sys_vmo_get_size(vmo_handle, &mut size)
    .expect("Failed to get VMO size");
assert_eq!(size, 0x1000);
```

### Linux-Style Memory Mapping

```rust
use crate::syscall::{
    dispatch_linux_syscall, LinuxSyscall,
    MmapProt, MmapFlags
};

// Anonymous private mapping
let prot = MmapProt::READ.bits | MmapProt::WRITE.bits;
let flags = MmapFlags::PRIVATE.bits | MmapFlags::ANONYMOUS.bits;

let args = [
    0,              // addr (NULL)
    0x2000,         // len (8KB)
    prot,           // prot (RW)
    flags,          // flags (PRIVATE|ANONYMOUS)
    0,              // fd (ignored)
    0,              // offset (ignored)
];

let vaddr = dispatch_linux_syscall(LinuxSyscall::Mmap as u32, args)
    .expect("mmap failed");

println!("Mapped memory at: {:#x}", vaddr);
```

## Future Work

### Priority 1: Complete Memory Syscalls
- [ ] Implement Linux `brk` syscall for heap management
- [ ] Implement Linux `mremap` for resizing mappings
- [ ] Implement full Zircon VMO operations (physical, contiguous)
- [ ] Implement Zircon `vmar_unmap_handle_close_thread_exit`

### Priority 2: IPC Support
- [ ] Implement Zircon channels (core IPC mechanism)
- [ ] Implement Zircon sockets (stream/datagram)
- [ ] Implement Zircon futex (fast userspace mutex)
- [ ] Implement Linux futex (for Linux threading)
- [ ] Implement Zircon ports (async event delivery)

### Priority 3: Process/Thread Management
- [ ] Implement full `execve` with ELF loading
- [ ] Implement Zircon `process_start` with proper entry point
- [ ] Implement thread creation with TLS support
- [ ] Implement wait/exit semantics fully

### Priority 4: Time and Synchronization
- [ ] Implement real clock syscalls with hardware timer
- [ ] Implement Zircon timers (one-shot and periodic)
- [ ] Implement Linux `gettimeofday` and `clock_gettime`

## References

- [grt-zcore Repository](https://github.com/StevenMeng5898/grt-zcore)
- [Zircon Syscall Documentation](https://fuchsia.dev/fuchsia-src/reference/syscalls)
- [Linux ARM64 Syscall ABI](https://man7.org/linux/man-pages/man2/syscalls.2.html)
- [zCore Original Project](https://github.com/rcore-os/zCore)

## License

This syscall compatibility layer is part of the SMROS project and follows the same license as the main kernel.
