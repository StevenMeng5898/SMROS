# SMROS Shell Implementation

## Overview

The SMROS kernel includes a fully interactive user-mode shell (v0.5.0) with comprehensive process and memory management commands. The shell runs as a scheduled thread and provides system monitoring capabilities. After boot completion, the kernel starts the user shell instead of running the old multi-thread worker sample.

## Shell Features

### Interactive Input
- **Line editing** with backspace support
- **Ctrl+C** - Cancel current command
- **Ctrl+U** - Clear entire line
- **Ctrl+L** - Clear screen
- **Character echo** - Visual feedback while typing
- **Command parsing** - Automatic whitespace tokenization

### Available Commands

#### Process Management

1. **`ps`** - List all processes
   ```
   PID  State      Name         Threads  Parent
   ─────────────────────────────────────────────
   1    Ready       init         0         0
   2    Ready       shell        0         1
   3    Ready       editor       0         1
   4    Ready       compiler     0         1
   ─────────────────────────────────────────────
   Total: 4 process(es)
   ```

2. **`top`** - Process monitor (like Unix top command)
   ```
   ┌─────────────────────────────────────────────────────────────┐
   │              SMROS Process Monitor (top)                    │
   ├─────────────────────────────────────────────────────────────┤
   │  PID  │ State    │ Name       │ Segments │ Pages  │ Heap     │
   │───────┼──────────┼────────────┼──────────┼────────┼──────────│
   │    1  │ Ready    │ init       │    4     │   8    │  0KB     │
   │    2  │ Ready    │ shell      │    4     │   8    │  0KB     │
   │    3  │ Ready    │ editor     │    4     │   8    │  0KB     │
   │    4  │ Ready    │ compiler   │    4     │   8    │  0KB     │
   ├─────────────────────────────────────────────────────────────┤
   │ Memory: 32 used / 4096 total pages           │
   │ Free: 4064 pages (16256 KB)                   │
   └─────────────────────────────────────────────────────────────┘
   ```

3. **`tree`** - Process tree view
   ```
   Process Tree:
   ═══════════════════════════════

   └─ [1] init (Ready)
      ├─ [2] shell (Ready)
      ├─ [3] editor (Ready)
      └─ [4] compiler (Ready)
   ```

4. **`kill <pid>`** - Terminate a process
   ```
   smros$ kill 4
   Process 4 terminated.
   ```

5. **`info [pid]`** - Detailed process information
   - Shows address space layout
   - Lists all segments with addresses
   - Displays page table entries
   - Shows heap and stack usage

#### Memory Management

1. **`meminfo`** - Memory information
   ```
   ┌─────────────────────────────────────────┐
   │           Memory Information            │
   ├─────────────────────────────────────────┤
   │  Total Memory:                          │
   │    Pages: 4096                          │
   │    Size:  16384 KB (16 MB)              │
   │                                         │
   │  Used Memory:                           │
   │    Pages: 32                            │
   │    Size:  128 KB                        │
   │    Usage: 0%                            │
   │                                         │
   │  Free Memory:                           │
   │    Pages: 4064                          │
   │    Size:  16256 KB                      │
   │                                         │
   │  Page Size: 4 KB (4096 bytes)           │
   └─────────────────────────────────────────┘
   ```

2. **`pages`** - Page allocation details
   - Shows page table for each process
   - Lists physical frame numbers (PFN)
   - Shows permissions (RW/RO, X)
   - Summary of system-wide page usage

3. **`heap`** - Heap usage per process
   ```
   Process Heap Usage:
   ═══════════════════════════════════════════════════
   Name         Heap Used    Heap Max     Free
   ─────────────────────────────────────────────────
   init         0 KB         16 KB        16 KB
   shell        0 KB         16 KB        16 KB
   editor       0 KB         16 KB        16 KB
   compiler     0 KB         16 KB        16 KB
   ─────────────────────────────────────────────────
   ```

#### System Commands

1. **`version`** - Kernel version information
   ```
   SMROS ARM64 Kernel v0.3.0
   Features:
     - Preemptive Round-Robin Scheduler
     - SMP Multi-Core Support (4 CPUs)
     - Multi-Process Memory Management
     - 4K Page-based Memory Allocation
     - Segment-based Memory Management
     - Interactive Shell
   ```

2. **`uptime`** - System uptime
   - Shows scheduler tick count

3. **`whoami`** - Current user
   ```
   root
   ```

4. **`date`** - Date/time (stub)
   - Placeholder for RTC implementation

5. **`echo <text>`** - Print text
   ```
   smros$ echo Hello World
   Hello World
   ```

6. **`cat <file>`** - Display file (stub)
   - File system not yet implemented

7. **`clear`** - Clear screen
   - Uses ANSI escape codes

8. **`help`** - Show available commands
   - Complete command reference

## Boot Flow Changes

### Previous Behavior
1. Boot kernel
2. Initialize hardware
3. Create 4 worker threads
4. Run threads with round-robin scheduling
5. Enter idle loop

### New Behavior
1. Boot kernel
2. Initialize hardware
3. Create 3 sample processes (shell, editor, compiler)
4. Print memory management status
5. **Enter interactive shell** ← New!

## Implementation Details

### Serial Input Support
Added to `src/serial.rs`:
- `read_byte()` - Blocking read
- `has_byte()` - Non-blocking check
- `read_line()` - Line input with editing

### Shell Structure
Defined in `src/memory.rs`:
```rust
pub struct Shell {
    pub serial: crate::serial::Serial,
    pub input_buf: [u8; 256],
    pub command_history: [&'static str; 10],
    pub history_index: usize,
}
```

### Command Execution Flow
1. Print prompt (`smros$ `)
2. Read line with `serial.read_line()`
3. Parse command with `parse_command_static()`
4. Execute via `match` statement
5. Loop back to step 1

### Memory Management Architecture

Each process has:
- **ProcessControlBlock (PCB)**
  - PID, state, name, parent PID
  - Thread count
  - Address space reference

- **ProcessAddressSpace**
  - Page table (up to 64 pages)
  - Segment descriptors (up to 4 segments)
  - Heap management
  - Stack management

- **Memory Segments**
  - Code: 1 page @ 0x0000 (r-x)
  - Data: 1 page @ 0x1000 (rw-)
  - Heap: 4 pages @ 0x2000 (rw-, grows up)
  - Stack: 2 pages @ 0xF000 (rw-, grows down)

- **Page Frame Allocator**
  - Bitmap-based (64-bit entries)
  - Manages 4096 pages (16MB)
  - Allocates/frees 4K pages

## Safe Rust Implementation

All code follows safe Rust principles:
- Interior mutability with `UnsafeCell` + `Sync`
- No raw pointer dereferencing
- Proper borrow checker compliance
- Bitmap allocator avoids race conditions
- String/buffer operations use safe wrappers

## Future Enhancements

Potential additions:
- [ ] Command history (up/down arrow keys)
- [ ] Tab completion
- [ ] Process creation from shell (`exec` command)
- [ ] File system integration
- [ ] Environment variables
- [ ] Shell scripting support
- [ ] Pipes and redirection
- [ ] Background processes
- [ ] Signals
- [ ] Real-time `top` updates

## Testing

Run in QEMU:
```bash
make run
```

Once in the shell, try:
```
smros$ help
smros$ ps
smros$ top
smros$ meminfo
smros$ tree
smros$ version
smros$ echo Hello from SMROS!
```

Exit QEMU with `Ctrl+A` then `X`.
