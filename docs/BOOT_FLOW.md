# SMROS ARM64 Kernel - Detailed Boot Flow

## Overview

SMROS is a preemptive multitasking ARM64 OS kernel with SMP multi-core support, multi-process memory management, and a comprehensive syscall layer (Linux & Zircon compatible). Written in Rust, it runs on QEMU's `virt` machine.

Boot sequence: **QEMU → Assembly Boot Code → kernel_main() → Subsystem Init → SMP Boot → Process Creation → User Test → User Shell → Scheduler**

---

## Boot Flow Diagram (Current)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ 1. QEMU Boot                                                           │
│    - QEMU loads kernel8.img at 0x40000000                              │
│    - CPU0 starts, jumps to _start entry point                          │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 2. Assembly Boot Code (_start in main.rs)                              │
│    - Read MPIDR_EL1 to determine CPU ID                                │
│    - [CPU0] Mask interrupts (DAIF)                                     │
│    - [CPU0] Set SP to __stack_top                                      │
│    - [CPU0] Clear BSS section                                          │
│    - Set VBAR_EL1 ← exception_vectors                                  │
│    - Branch to kernel_main()                                           │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 3. kernel_main() - CPU0                                                │
│    - Init Serial (PL011 UART @ 0x9000000)                              │
│    - Print banner: "SMROS ARM64 Kernel with Preemptive RR Scheduler"   │
│    - Print version: v0.2.0                                             │
│    - Print system info (MPIDR_EL1, SCTLR_EL1)                          │
│    - Init GIC interrupt controller                                     │
│    - Init ARM Generic Timer (100Hz)                                    │
│    - Init SMP support                                                  │
│    - Init memory management                                            │
│    - Init syscall interface                                            │
│    - Init MMU                                                          │
│    - Init channel subsystem                                            │
│    - Init user-level process management                                │
│    - Init preemptive RR scheduler                                      │
│    - Enable timer interrupts                                           │
│    - Unmask CPU interrupts (clear DAIF.I)                              │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 4. SMP Boot & Process Creation                                         │
│    - Boot secondary CPUs via PSCI CPU_ON                               │
│    - Print SMP status                                                  │
│    - Create 3 sample processes: shell, editor, compiler                │
│    - Print process & memory status                                     │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 5. User Test & Shell Startup                                           │
│    - Run user test process (verifies syscalls: getpid, mmap)           │
│    - Start user shell as scheduled thread (user_shell::start_user_shell)│
│    - Shell thread created, ready to run                                │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 6. Start Scheduler                                                     │
│    - start_first_thread() jumps to first ready thread (shell)          │
│    - Never returns - CPU0 now running shell                            │
│    - Shell prints welcome banner (v0.5.0)                              │
│    - Interactive shell with 11 commands                                │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Version History

### Current Version - User Shell & Full Syscall Support

- **Kernel Version**: 0.2.0
- **Shell Version**: 0.5.0
- **User-Mode Shell**: Runs as scheduled thread (user_shell.rs)
- **Syscall Interface**: Linux & Zircon compatibility layer
- **11 Shell Commands**: help, version, ps, top, meminfo, uptime, kill, testsc, echo, clear, exit
- **Directory Structure**: kernel_lowlevel/, kernel_objects/, syscall/, user_level/
- **Zero Compiler Warnings**
- **SMP Multi-Core**: 4 CPUs via PSCI
- **Preemptive Scheduler**: Round-robin, 10 ticks/time slice, CPU-aware scheduling
- **Thread Management**: TCBs in kernel_objects/thread.rs

### v0.2.0 - Preemptive Multithreading

- Preemptive Round-Robin Scheduler
- SMP Multi-Core Support (4 CPUs)
- Thread Management with CPU Affinity
- Context Switching
- GICv2 Interrupt Controller
- ARM Generic Timer (100Hz)

### v0.1.0 - Basic Kernel

- Basic serial output
- Simple kernel entry
- Boot assembly code

---

## Detailed Boot Stages

### Stage 1: QEMU Boot

**Command:** `qemu-system-aarch64 -M virt -cpu cortex-a57 -m 512M -smp 4 -nographic -kernel kernel8.img`

| Step | Description |
|------|-------------|
| **1.1** | QEMU emulates ARM64 `virt` machine with Cortex-A57 |
| **1.2** | Loads `kernel8.img` at **`0x40000000`** |
| **1.3** | **CPU0** starts, jumps to `_start` entry point |
| **1.4** | Secondary CPUs (CPU1-3) remain offline until PSCI CPU_ON |

**Memory Layout:**
```
0x00000000 - 0x0007FFFF: Reserved
0x00080000 - 0x3FFFFFFF: Available RAM
0x40000000 - ...:         Kernel code/data
__stack_top:              Kernel stack (above kernel end)
Heap:                     1MB static bump allocator
```

---

### Stage 2: Assembly Boot Code

**Location:** `src/main.rs` → `core::arch::global_asm!()`
**Section:** `.text.boot` (first in binary)

#### CPU Detection

```assembly
mrs     x19, mpidr_el1
and     x19, x19, #0xFF       // Extract CPU ID
cbz     x19, 1f               // If CPU0, continue
mov     sp, x3                 // Secondary: set stack
bl      secondary_cpu_entry    // Jump to secondary entry
```

#### Boot CPU (CPU0) Setup

1. **Mask Interrupts** → `DAIF |= 0x3C0`
2. **Set Stack** → `SP = __stack_top`
3. **Clear BSS** → Zero `__bss_start` to `__bss_end`
4. **Set Vectors** → `VBAR_EL1 = exception_vectors`
5. **Branch** → `bl kernel_main`

#### Exception Vector Table

16 entries (0x80 bytes each, 2KB aligned):
- Synchronous, IRQ, FIQ, SError for Current EL (SP0/SPx)
- Synchronous, IRQ, FIQ, SError for Lower EL (AArch64/AArch32)

IRQ handlers:
1. Save x0-x15
2. Call `timer_interrupt_handler`
3. Call `check_preemption`
4. Restore x0-x15
5. `eret`

#### Secondary CPU Entry

```assembly
secondary_entry:
    mov     sp, x2                 // Set stack from PSCI
    and     x1, x1, #~0xF          // Align to 16 bytes
    mov     sp, x1
    mrs     x1, cpacr_el1          // Enable FP/SIMD
    orr     x1, x1, #(0x3 << 20)
    msr     cpacr_el1, x1
    isb
    b       secondary_cpu_entry    // Jump to Rust
```

---

### Stage 3: Rust Kernel Entry (kernel_main)

**Location:** `src/main.rs`
**Signature:** `pub extern "C" fn kernel_main() -> !`

#### Init Sequence (in order):

| Step | Code | Description |
|------|------|-------------|
| **1** | `Serial::new().init()` | PL011 UART @ 0x9000000 |
| **2** | Print banner | "SMROS ARM64 Kernel with Preemptive RR Scheduler v0.2.0" |
| **3** | `print_system_info()` | MPIDR_EL1, SCTLR_EL1 |
| **4** | `kernel_lowlevel::interrupt::init()` | GICv2 controller |
| **5** | `kernel_lowlevel::timer::init()` | ARM Generic Timer, 100Hz |
| **6** | `kernel_lowlevel::smp::init()` | SMP support |
| **7** | `kernel_lowlevel::memory::init()` | Memory management |
| **8** | `crate::syscall::init()` | Syscall interface |
| **9** | `kernel_lowlevel::mmu::init()` | MMU |
| **10** | `crate::kernel_objects::channel::init()` | Channel IPC |
| **11** | `crate::user_level::user_process::init()` | User process mgmt |
| **12** | `scheduler().init()` | Preemptive RR scheduler |
| **13** | `interrupt::enable_timer_interrupt()` | Enable 100Hz timer IRQ |
| **14** | Clear DAIF.I bit | Unmask CPU interrupts |

---

### Stage 4: SMP Boot & Process Creation

```rust
// Boot secondary CPUs
boot_all_cpus();
smp_print_status();

// Create sample processes
let pm = process_manager();
pm.create_process("shell");
pm.create_process("editor");
pm.create_process("compiler");

// Print status
pm.print_status(&mut serial);
```

**Output:**
```
--- SMP Multi-Core Initialization ---
--- Multi-Process Memory Management ---
Creating sample processes for demonstration...

[INFO] Created 3 sample processes:
  - shell (PID 2)
  - editor (PID 3)
  - compiler (PID 4)
```

---

### Stage 5: User Test & Shell Startup

```rust
// Run user test process
crate::user_level::user_test::run_user_test();

// Start user shell
crate::user_level::user_shell::start_user_shell();

// Start scheduler (never returns)
crate::kernel_objects::scheduler::start_first_thread();
```

**User Test** verifies syscalls work (getpid, mmap).

**start_user_shell()** creates a shell thread via scheduler:
```rust
let thread_id = scheduler().create_thread(shell_thread_wrapper, "user_shell");
```

---

### Stage 6: Scheduler Start

`start_first_thread()`:
1. Finds first `Ready` thread (shell)
2. Updates thread states: Idle → Ready, Shell → Running
3. Calls `context_switch_start()` → jumps to shell entry
4. **Never returns**

**Shell Entry:**
```rust
#[no_mangle]
pub extern "C" fn user_shell_entry() -> ! {
    let mut shell = UserShell::new();
    shell.run()
}
```

**Shell Welcome:**
```
╔═══════════════════════════════════════════════════════════╗
║     SMROS User-Mode Shell v0.5.0                         ║
╚═══════════════════════════════════════════════════════════╝

Welcome to SMROS shell!
Type 'help' for available commands.

smros>
```

---

## Interrupt Handling

### Timer IRQ (every 10ms, 100Hz)

```rust
#[no_mangle]
extern "C" fn timer_interrupt_handler() {
    kernel_lowlevel::timer::clear_interrupt();
    let interrupt_id = kernel_lowlevel::interrupt::acknowledge_interrupt();
    crate::kernel_objects::scheduler::scheduler().on_timer_tick();
    kernel_lowlevel::interrupt::end_of_interrupt(interrupt_id);
}
```

### Preemption Check

```rust
#[no_mangle]
extern "C" fn check_preemption() {
    let cpu_id = current_cpu_id();
    let s = crate::kernel_objects::scheduler::scheduler();
    
    if s.should_preempt() {  // time_slice == 0 && active_threads > 1
        if let Some(next_id) = s.schedule_next_for_cpu(cpu_id as usize) {
            s.reset_time_slice(next_id);
        }
        schedule_on_cpu(cpu_id as usize);  // Context switch
    }
}
```

---

## Thread & Scheduler Details

### Scheduler

| Feature | Value |
|---------|-------|
| Type | Preemptive Round-Robin |
| Location | `src/kernel_objects/scheduler.rs` |
| Max Threads | 16 |
| Time Slice | 10 ticks (100ms @ 100Hz) |
| Thread States | Empty, Ready, Running, Blocked, Terminated |
| Idle Thread | Thread 0 (always present) |
| CPU-Aware | Yes (schedule_next_for_cpu) |

### Thread Control Block (TCB)

**Location:** `src/kernel_objects/thread.rs`

| Field | Description |
|-------|-------------|
| `id` | ThreadId |
| `state` | ThreadState (Empty/Ready/Running/Blocked/Terminated) |
| `context` | CpuContext (x0-x28, FP, LR, SP, PC, PSTATE) |
| `stack` | Stack pointer |
| `stack_size` | 8KB (DEFAULT_STACK_SIZE) |
| `time_slice` | Remaining ticks |
| `total_ticks` | Total ticks run |
| `cpu_affinity` | Optional CPU binding |
| `current_cpu` | Currently executing CPU |

### Context Switch

**Assembly:** `src/main.rs` global_asm!

```assembly
context_switch:
    // Disable interrupts
    // Save callee-saved: x19-x28, FP(x29), LR(x30), SP, PC
    // Load next thread context
    // Enable interrupts
    // Branch to saved PC
```

**Registers:** x19-x28, x29(FP), x30(LR), SP, PC

---

## Project Structure

```
SMROS/
├── src/
│   ├── main.rs                     # Kernel entry, boot asm, exceptions
│   ├── context_switch.S            # Context switch assembly
│   ├── kernel_lowlevel/            # Hardware drivers
│   │   ├── memory.rs               # Process memory management
│   │   ├── mmu.rs                  # MMU & page tables
│   │   ├── serial.rs               # PL011 UART
│   │   ├── timer.rs                # ARM Generic Timer
│   │   ├── interrupt.rs            # GICv2
│   │   ├── smp.rs                  # PSCI multi-core
│   │   └── drivers.rs              # Re-exports
│   ├── kernel_objects/             # Kernel objects
│   │   ├── thread.rs               # TCB, thread management
│   │   ├── scheduler.rs            # Preemptive RR scheduler
│   │   ├── types.rs                # Shared types/constants
│   │   ├── handle.rs               # Handle table
│   │   ├── vmo.rs                  # Virtual Memory Object
│   │   ├── vmar.rs                 # Virtual Memory Address Region
│   │   └── channel.rs              # IPC channels
│   ├── syscall/                    # Syscall layer
│   │   ├── syscall.rs              # Linux & Zircon syscalls
│   │   ├── syscall_dispatch.rs     # Dispatch from exceptions
│   │   └── syscall_handler.rs      # SVC handler
│   └── user_level/                 # User processes
│       ├── user_process.rs         # Process management
│       ├── user_shell.rs           # User shell (11 commands)
│       └── user_test.rs            # Syscall tests
```

---

## Shell Commands

The user shell (v0.5.0) provides 11 commands:

| Command | Description |
|---------|-------------|
| `help` | Show available commands |
| `version` | Kernel version (v0.2.0) |
| `ps` | List processes (PID, state, name, threads, parent) |
| `top` | Process monitor with memory stats |
| `meminfo` | System memory info (total/used/free) |
| `uptime` | System uptime (days/hours/min/sec) |
| `kill <pid>` | Terminate process |
| `testsc` | Test syscalls (getpid, write, mmap) |
| `echo <text>` | Print text |
| `clear` | Clear screen |
| `exit` | Exit shell |

---

## Build & Run

### Build
```bash
make build
# or: cargo build
```

### Run
```bash
make run
# or: qemu-system-aarch64 -M virt -cpu cortex-a57 -m 512M -smp 4 -nographic -kernel kernel8.img
```

### Exit QEMU
Press `Ctrl+A`, then `X`

---

## Key Dependencies

| Crate | Version | Usage |
|-------|---------|-------|
| `cortex-a` | 8 | Register access (MPIDR_EL1, SCTLR_EL1), wfi() |
| `tock-registers` | 0.8 | Register interface traits |
| `volatile` | 0.4 | Volatile memory (hardware registers) |
| `bitflags` | 1.3 | Bitflag enums (Rights, VmOptions, etc.) |

---

## References

- [Rust Embedded Book](https://docs.rust-embedded.org/book/)
- [ARM Architecture Reference Manual](https://developer.arm.com/documentation/)
- [QEMU ARM64 Virt Machine](https://www.qemu.org/docs/master/system/arm/virt.html)
- [GICv2 Architecture](https://developer.arm.com/documentation/ihi0048/latest/)
- [PSCI Specification](https://developer.arm.com/documentation/den0022/latest/)
