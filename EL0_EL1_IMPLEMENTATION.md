# EL0/EL1 Separation Implementation Guide

## Overview

This document describes the implementation of EL0/EL1 separation in SMROS, moving the shell and other processes to user mode (EL0), and implementing the necessary kernel objects for Zircon syscall compatibility.

## What Was Implemented

### 1. MMU and Page Table Support (`src/mmu.rs`)

**Purpose**: Provide memory management unit (MMU) support with page tables for memory isolation between EL0 and EL1.

**Key Components**:
- `PageTableEntry`: Represents a page table entry with proper ARM64 flags
- `PageTableManager`: Manages page tables for both user space (TTBR0) and kernel space (TTBR1)
- `Vma`: Virtual Memory Area descriptor for tracking memory regions
- Memory mapping functions:
  - `map_user_region()`: Maps memory accessible from EL0
  - `map_kernel_region()`: Maps kernel-only memory (EL1)

**Features**:
- Support for user-accessible pages (AP_EL0 flag)
- Read/write/execute permissions
- Proper cache attributes
- TLB invalidation on address space switch

### 2. Syscall Handler (`src/syscall_handler.rs`)

**Purpose**: Handle system calls from EL0 processes via SVC exception.

**Key Components**:
- `handle_svc_exception_from_el0()`: Main SVC handler called from assembly exception vector
- `get_syscall_result()`: Helper to retrieve syscall result
- Dispatch logic for both Linux and Zircon syscalls

**Flow**:
1. EL0 process executes `svc #0` instruction
2. CPU traps to EL1 exception handler (assembly in `main.rs`)
3. Exception handler saves all registers on stack
4. Calls `handle_svc_exception_from_el0()` with saved context
5. Handler extracts syscall number from x8 register
6. Dispatches to appropriate syscall implementation (Linux or Zircon)
7. Stores result in x0
8. Assembly restores registers and executes `eret` to return to EL0

### 3. EL0 Process Management (`src/el0_process.rs`)

**Purpose**: Manage processes that run at EL0 (user mode).

**Key Components**:
- `UserProcess`: Extended PCB with EL0-specific data
  - Page table manager instance
  - User stack virtual address
  - Entry point for user-mode code
  - Process and VMAR handles (Zircon compatibility)
- `create_user_process()`: Creates a new user-mode process
- `init_user_process()`: Sets up user-space memory layout
- `switch_to_el0()`: Assembly function to transition from EL1 to EL0

**Memory Layout for EL0 Processes**:
```
0x0000_0000 - Code segment (read-execute, user-accessible)
0x0000_1000 - Data segment (read-write, user-accessible)
0x0000_2000 - Heap (read-write, user-accessible, 4 pages)
0xFFFF_0000 - Stack (read-write, user-accessible, 2 pages, grows down)
```

### 4. Channel Kernel Object (`src/channel.rs`)

**Purpose**: Implement Zircon-style channels for inter-process communication (IPC).

**Key Components**:
- `Channel`: Bidirectional message passing between processes
- `ChannelMessage`: Message data with optional handle transfer
- `ChannelTable`: Global table managing all channels
- Syscall implementations:
  - `sys_channel_create()`: Create a new channel (returns two handles)
  - `sys_channel_read()`: Read message from channel
  - `sys_channel_write()`: Write message to channel
  - `sys_channel_call_noretry()`: Atomic write+read operation

**Features**:
- Two endpoint handles (handle0, handle1)
- Message queues for each endpoint
- Handle transfer in messages
- Signal state for wait operations
- Peer closed notification

### 5. EL0 Test Process (`src/el0_test.rs`)

**Purpose**: Test process that runs in EL0 and validates syscall functionality.

**Key Components**:
- `linux_syscall()`: Inline assembly function to make syscalls from EL0
- Test functions:
  - `test_getpid()`: Test getpid syscall
  - `test_mmap()`: Test anonymous memory mapping
  - `test_write()`: Test write to file descriptor
  - `test_exit()`: Test process exit
- Entry points:
  - `el0_test_process_entry()`: Main test process
  - `el0_shell_entry()`: User-mode shell entry point
  - `el0_busy_loop_entry()`: Simple busy loop

**Example Usage**:
```rust
// Make a syscall from EL0
let pid = unsafe { linux_syscall(172, [0; 6]) }; // getpid

// Map anonymous memory
let addr = unsafe { linux_syscall(222, [0, 4096, 3, 0x22, 0, 0]) }; // mmap

// Exit process
unsafe { linux_syscall(93, [0, 0, 0, 0, 0, 0]); } // exit
```

### 6. Exception Handler Updates (`src/main.rs`)

**Purpose**: Update assembly exception vectors to handle SVC from EL0.

**Key Changes**:
- Enhanced `exception_handler` to detect SVC exceptions from EL0
- Extract exception syndrome register (ESR_EL1) to identify exception type
- Check for EC = 0x15 (SVC from AArch64)
- Call Rust syscall handler with saved register context
- Properly restore registers and return to EL0 via `eret`

**Assembly Flow**:
```assembly
exception_handler:
    // Save all registers to stack
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    ...
    
    // Read exception registers
    mrs x0, esr_el1
    mrs x1, elr_el1
    mrs x2, spsr_el1
    mrs x3, sp_el0
    
    // Check if SVC from EL0
    ubfx x4, x0, #26, #6
    cmp x4, #0x15
    b.ne 1f
    
    // Call syscall handler
    bl handle_svc_exception_from_el0
    
    // Restore registers
    ldp x0, x1, [sp, #0]
    ...
    
    // Return to EL0
    eret
```

## Architecture

### Execution Levels

```
┌─────────────────────────────────────────┐
│          EL1 (Kernel Mode)              │
│  - Full hardware access                 │
│  - Exception handlers                   │
│  - Syscall dispatch                     │
│  - Memory management                    │
│  - Scheduler                            │
│  - Device drivers                       │
└─────────────────┬───────────────────────┘
                  │ SVC #0 (syscall)
                  │ Exception return (eret)
┌─────────────────┴───────────────────────┐
│          EL0 (User Mode)                │
│  - Restricted access                    │
│  - Shell process                        │
│  - Test processes                       │
│  - User applications                    │
│  - Can only access user-mapped memory   │
└─────────────────────────────────────────┘
```

### Syscall Flow

```
EL0 Process
    │
    ├─ Executes: svc #0
    │
    ↓
CPU Exception (traps to EL1)
    │
    ├─ Saves PC to ELR_EL1
    ├─ Saves state to SPSR_EL1
    ├─ Jumps to exception vector
    │
    ↓
Assembly Exception Handler
    │
    ├─ Saves all registers to stack
    ├─ Reads ESR_EL1, ELR_EL1, SPSR_EL1, SP_EL0
    ├─ Checks if SVC exception (EC = 0x15)
    ├─ Calls: handle_svc_exception_from_el0()
    │
    ↓
Rust Syscall Handler
    │
    ├─ Extracts syscall number from x8
    ├─ Extracts arguments from x0-x7
    ├─ Dispatches to syscall implementation
    │   ├─ Linux syscalls (< 1000)
    │   └─ Zircon syscalls (≥ 1000)
    ├─ Stores result in x0
    └─ Advances ELR_EL1 past svc instruction
    │
    ↓
Assembly Exception Handler (return)
    │
    ├─ Restores all registers from stack
    ├─ x0 contains syscall result
    └─ Executes: eret
    │
    ↓
EL0 Process (resumes)
    │
    └─ x0 contains syscall result
```

## Kernel Objects Implemented

### 1. VMA (Virtual Memory Area)
- **Location**: `src/mmu.rs`
- **Purpose**: Describe virtual memory regions with permissions
- **Used by**: PageTableManager for tracking mapped regions

### 2. VMO (Virtual Memory Object)
- **Location**: `src/syscall.rs` (already existed)
- **Purpose**: Zircon-style virtual memory objects
- **Operations**: create, read, write, resize, commit, decommit

### 3. VMAR (Virtual Memory Address Region)
- **Location**: `src/syscall.rs` (already existed)
- **Purpose**: Manage virtual address space layout
- **Operations**: map, unmap, allocate, protect, destroy

### 4. Channel
- **Location**: `src/channel.rs`
- **Purpose**: IPC mechanism for message passing
- **Operations**: create, read, write, call
- **Features**: Two endpoints, message queues, handle transfer

### 5. Handle Table
- **Location**: `src/syscall.rs` (already existed)
- **Purpose**: Track kernel object handles per process
- **Operations**: add, remove, duplicate, get_rights

### 6. Process/Thread Handles
- **Location**: `src/el0_process.rs`, `src/syscall.rs`
- **Purpose**: Represent process and thread objects
- **Integration**: UserProcess extends PCB with EL0 data

## Testing

### Building
```bash
cargo build --target aarch64-unknown-none
```

### Running in QEMU
```bash
make run
# or
qemu-system-aarch64 -machine virt -cpu cortex-a53 -nographic \
    -kernel kernel8.img -smp 4
```

### Expected Behavior

1. **Boot Sequence**:
   - Kernel boots at EL1
   - Initializes MMU, syscall handler, channels, EL0 process manager
   - Creates sample processes

2. **EL0 Process Creation**:
   - Test process created with user-mode entry point
   - Memory mapped with proper permissions
   - Page tables configured for user accessibility

3. **Syscall Test**:
   - EL0 test process makes syscalls via `svc #0`
   - Traps to EL1 exception handler
   - Syscall dispatched and executed
   - Result returned to EL0
   - Test process prints success messages

4. **Shell in EL0**:
   - Shell runs in user mode
   - Makes syscalls for I/O operations
   - Cannot access kernel memory directly

## What's Not Yet Implemented

The following kernel objects are defined but need full implementation:

1. **Socket**: Network/stream IPC (type defined, no implementation)
2. **Event**: Event signaling (type defined, no implementation)
3. **Port**: Event port mechanism (type defined, no implementation)
4. **Timer**: Timer objects (type defined, no implementation)
5. **Futex**: Fast userspace mutex (syscall stubs exist)
6. **Full MMU**: Complete 4-level page table walk (simplified version implemented)
7. **Address Space Isolation**: Per-process page tables (infrastructure ready)
8. **Copy-on-Write**: For fork/exec (VMO support exists)

## Key Files Modified/Created

### New Files
- `src/mmu.rs` - MMU and page table management
- `src/syscall_handler.rs` - SVC exception handler
- `src/el0_process.rs` - EL0 process management
- `src/channel.rs` - Channel IPC implementation
- `src/el0_test.rs` - EL0 test process

### Modified Files
- `src/main.rs` - Added new modules, updated exception handler, initialized new subsystems
- `Cargo.toml` - No changes needed (bitflags already present)

## Syscall Compatibility

### Linux Syscalls (Working)
- ✅ `sys_mmap` - Anonymous memory mapping
- ✅ `sys_munmap` - Unmap memory
- ✅ `sys_mprotect` - Change protection (stub)
- ✅ `sys_fork` - Create process
- ✅ `sys_exit` - Exit process
- ✅ `sys_getpid` - Get process ID (placeholder)
- ✅ `sys_getppid` - Get parent PID (placeholder)
- ✅ `sys_kill` - Kill process

### Zircon Syscalls (Working)
- ✅ `sys_vmo_create` - Create VMO
- ✅ `sys_vmo_read` / `sys_vmo_write` - Read/write VMO
- ✅ `sys_vmo_get_size` / `sys_vmo_set_size` - Resize VMO
- ✅ `sys_vmo_op_range` - VMO operations
- ✅ `sys_vmar_map` / `sys_vmar_unmap` - VMAR mapping
- ✅ `sys_vmar_allocate` / `sys_vmar_protect` / `sys_vmar_destroy` - VMAR operations
- ✅ `sys_handle_close` / `sys_handle_duplicate` - Handle operations
- ✅ `sys_process_create` - Create process
- ✅ `sys_channel_create` - Create channel
- ✅ `sys_channel_read` / `sys_channel_write` - Channel I/O

## Debugging Tips

1. **Syscall Not Working**: Check ESR_EL1 in exception handler to verify EC code
2. **Permission Faults**: Verify page table entries have AP_EL0 flag set
3. **Invalid Address**: Ensure user process only accesses user-mapped memory
4. **Exception Loop**: Check that ELR_EL1 is advanced past svc instruction

## Future Work

1. Implement full 4-level page table walk
2. Add proper address space isolation with ASIDs
3. Implement remaining kernel objects (socket, event, port, timer)
4. Add futex support for synchronization
5. Implement copy-on-write for fork
6. Add proper user-mode shell with command parsing
7. Implement filesystem support for execve
8. Add proper timer and clock syscalls

## References

- ARM Architecture Reference Manual (ARM DDI 0487)
- Zircon Kernel Documentation: https://fuchsia.dev/fuchsia-src/concepts/kernel
- ARM Exception Levels: https://developer.arm.com/documentation/102412/0100/Exception-levels
- grt-zcore project: https://github.com/StevenMeng5898/grt-zcore
