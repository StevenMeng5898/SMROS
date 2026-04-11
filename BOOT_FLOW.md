# SMROS ARM64 Kernel - Detailed Booting Flow

## Overview

SMROS is a preemptive multitasking ARM64 OS kernel with SMP multi-core support and multi-process memory management, written in Rust and designed to run on QEMU's `virt` machine. The boot process follows the standard ARM64 bare-metal boot sequence with multi-core extensions: **Bootloader → Assembly Boot Code → Rust Kernel Entry → Hardware Initialization → SMP Boot → Process Creation → Interactive Shell**.

---

## Boot Flow Diagram (v0.3.0)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ 1. QEMU Boot (Primary Bootloader)                                      │
│    - QEMU loads kernel8.img at 0x40000000                              │
│    - CPU0 starts in EL2 (Hypervisor mode)                              │
│    - Jumps to _start entry point                                       │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 2. Assembly Boot Code (_start in main.rs global_asm!)                  │
│    - Check CPU ID (MPIDR_EL1) - Boot CPU or Secondary CPU?             │
│    - [Boot CPU] Mask interrupts (DAIF)                                 │
│    - [Boot CPU] Set stack pointer to __stack_top                       │
│    - [Boot CPU] Clear BSS section                                      │
│    - Set exception vector base (VBAR_EL1)                              │
│    - Branch to kernel_main()                                           │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 3. Rust Kernel Entry (kernel_main in main.rs) - CPU0 Only              │
│    - Initialize Serial (PL011 UART) with input support                 │
│    - Print kernel banner & version (v0.3.0)                            │
│    - Print system information                                          │
│    - Initialize GIC interrupt controller                               │
│    - Initialize ARM Generic Timer                                      │
│    - Initialize SMP support                                            │
│    - Initialize memory management (processes, pages, segments)         │
│    - Create sample processes (init, shell, editor, compiler)           │
│    - Initialize preemptive RR scheduler                                │
│    - Enable timer interrupts (100Hz)                                   │
│    - Unmask CPU interrupts                                             │
│    - Boot secondary CPUs via PSCI                                      │
│    - Print process & memory status                                     │
│    - Enter interactive shell                                           │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 4. Secondary CPU Boot (PSCI CPU_ON) - CPU1, CPU2, CPU3                 │
│    - PSCI wakes secondary CPUs                                         │
│    - secondary_entry sets up stack from context_id                     │
│    - secondary_cpu_entry() initializes CPU                             │
│    - Enables FP/SIMD, exception vectors, interrupts                    │
│    - Marks CPU as online                                               │
└──────────────────────────┬──────────────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ 5. Interactive Shell Mode (NEW in v0.3.0)                              │
│    - Shell prints welcome banner                                       │
│    - Waits for user input via serial console                           │
│    - Parses and executes commands:                                     │
│      * ps, top, tree, kill (process management)                        │
│      * meminfo, pages, heap (memory management)                        │
│      * help, version, uptime, echo, clear, etc.                        │
│    - Runs indefinitely until user exits QEMU                           │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Version History

### v0.3.0 (Current) - Multi-Process & Interactive Shell

- **Multi-Process Support**: Process isolation with separate address spaces
- **4K Page Management**: Bitmap-based page frame allocator
- **Segment Management**: Code, data, heap, stack segments per process
- **Interactive Shell**: 15+ commands (ps, top, meminfo, etc.)
- **Serial Input**: Line editing with backspace, Ctrl+C, Ctrl+U, Ctrl+L
- **Process Creation**: Sample processes created at boot (init, shell, editor, compiler)
- **No Worker Threads**: Replaced multi-thread demo with shell

### v0.2.0 - Preemptive Multithreading

- Preemptive Round-Robin Scheduler
- SMP Multi-Core Support (4 CPUs)
- Thread Management with CPU Affinity
- Context Switching
- GICv2 Interrupt Controller
- ARM Generic Timer (100Hz)
- Worker Thread Demo (4 threads)

### v0.1.0 - Basic Kernel

- Basic serial output
- Simple kernel entry
- Boot assembly code

---

## Detailed Boot Stages

### Stage 1: QEMU Boot (Primary Bootloader)

**Trigger:** `qemu-system-aarch64 -M virt -cpu cortex-a57 -m 512M -smp 4 -nographic -kernel kernel8.img`

| Step | Description |
|------|-------------|
| **1.1** | QEMU starts and emulates an ARM64 `virt` machine with Cortex-A57 CPU |
| **1.2** | QEMU's built-in bootloader loads `kernel8.img` (ELF format) into memory |
| **1.3** | Kernel is loaded at **`0x40000000`** (as defined in `linker/kernel.ld`) |
| **1.4** | **CPU0** starts at **Exception Level 2 (EL2)** - Hypervisor mode |
| **1.5** | QEMU jumps to the ELF entry point: `_start` (defined via `ENTRY(_start)` in linker script) |
| **1.6** | Secondary CPUs (CPU1-3) remain offline until booted via PSCI |

**Memory Layout at Load Time:**
```
0x00000000 - 0x0007FFFF: Reserved
0x00080000 - 0x3FFFFFFF: Available RAM
0x40000000 - ...:         Kernel code/data (loaded here)
...
__stack_top:              Kernel stack (512KB above kernel end)
Heap:                     1MB static bump allocator
```

---

### Stage 2: Assembly Boot Code (`_start`)

**Location:** `src/main.rs` → `core::arch::global_asm!()` block
**Section:** `.text.boot` (placed first by linker script)

#### Step 2.0: CPU Detection (NEW in v0.2.0)

```assembly
mrs     x19, mpidr_el1
and     x19, x19, #0xFF       // Extract affinity level 0 (CPU ID)

// If CPU0, continue with normal boot
cbz     x19, 1f

// Secondary CPU: jump to secondary entry point
mov     sp, x3                 // Stack from PSCI CPU_ON call
bl      secondary_cpu_entry
```

**Purpose:** Differentiate between boot CPU (CPU0) and secondary CPUs. Secondary CPUs skip BSS clearing and jump directly to `secondary_cpu_entry`.

---

#### Step 2.1: Mask All Interrupts (Boot CPU Only)

```assembly
mrs     x1, daif
orr     x1, x1, #0x3C0
msr     daif, x1
```

| Register | Action |
|----------|--------|
| `DAIF` | Debug, Abort, IRQ, FIQ mask bits |
| `0x3C0` | Masks all exception types (SDE, AIF) |

**Purpose:** Prevent interrupts from firing before the kernel is ready to handle them.

---

#### Step 2.2: Set Stack Pointer

```assembly
ldr     x1, =__stack_top
mov     sp, x1
```

| Symbol | Source | Value |
|--------|--------|-------|
| `__stack_top` | Linker script (`linker/kernel.ld`) | End of 512KB stack region |

**Purpose:** Initialize the stack pointer so Rust code can use the stack safely.

**Stack Layout:**
```
__stack_bottom ────────────────┐
                               │
                               │ 512KB (0x80000 bytes)
                               │
__stack_top  ──────────────────┘ (SP points here, grows downward)
```

---

#### Step 2.3: Clear BSS Section

```assembly
ldr     x1, =__bss_start
ldr     x2, =__bss_end
mov     x3, #0
1:
    cmp     x1, x2
    b.eq    2f
    str     x3, [x1], #8
    b       1b
2:
```

| Symbol | Source | Purpose |
|--------|--------|---------|
| `__bss_start` | Linker script | Start of `.bss` section |
| `__bss_end` | Linker script | End of `.bss` section |

**Purpose:** Zero-initialize all global/static variables (C/Rust convention for uninitialized data).

**Algorithm:**
```
for addr from __bss_start to __bss_end step 8:
    *addr = 0
```

---

#### Step 2.4: Set Exception Vector Base (NEW in v0.2.0)

```assembly
ldr     x1, =exception_vectors
msr     vbar_el1, x1
```

**Purpose:** Set the Vector Base Address Register (VBAR_EL1) to point to the exception vector table. This enables proper handling of synchronous exceptions, IRQs, FIQs, and SError interrupts.

**Exception Vector Table:**
```
Offset 0x000: Synchronous (Current EL, SP0)
Offset 0x080: IRQ (Current EL, SP0) → irq_handler
Offset 0x100: FIQ (Current EL, SP0)
Offset 0x180: SError (Current EL, SP0)
Offset 0x200: Synchronous (Current EL, SPx)
Offset 0x280: IRQ (Current EL, SPx) → irq_handler_sp
Offset 0x400: Synchronous (Lower EL, AArch64)
Offset 0x480: IRQ (Lower EL, AArch64) → irq_handler_lower
... (16 total entries, each 0x80 bytes apart)
```

**IRQ Handler Flow:**
1. Save caller-saved registers (x0-x15) to stack
2. Call `timer_interrupt_handler` (Rust)
3. Call `check_preemption` (Rust)
4. Restore caller-saved registers
5. `eret` (return from exception)

---

#### Step 2.5: Branch to Rust Kernel

```assembly
bl      kernel_main
```

**Purpose:** Transfer control to the Rust `kernel_main()` function.

---

#### Step 2.6: Halt if Kernel Returns (Should Never Happen)

```assembly
5:
    wfi
    b       5b
```

**Purpose:** Infinite loop with `WFI` (Wait For Interrupt) in case `kernel_main()` returns. The kernel is designed to run forever (`-> !`), so this is a safety net.

---

#### Secondary CPU Entry Point (`secondary_entry`)

**Location:** `src/main.rs` → `global_asm!()` block

This entry point is called when a secondary CPU boots via PSCI `CPU_ON`:

```assembly
secondary_entry:
    mov     sp, x2          // Set stack from context_id
    and     x1, x1, #~0xF   // Align stack to 16 bytes
    mov     sp, x1

    // Enable FP/SIMD
    mrs     x1, cpacr_el1
    orr     x1, x1, #(0x3 << 20)
    msr     cpacr_el1, x1
    isb

    b       secondary_cpu_entry  // Jump to Rust code
```

**Purpose:** Initial setup for secondary CPUs before entering Rust code.

---

### Stage 3: Linker Script Resolution

**File:** `linker/kernel.ld`

The linker script organizes the kernel binary layout:

| Section | Alignment | Contents |
|---------|-----------|----------|
| `.text.boot` | 4KB | Boot assembly code (`_start`) - **must be first** |
| `.text` | 4KB | All other code |
| `.rodata` | 4KB | Read-only data (strings, constants) |
| `.data` | 4KB | Initialized data |
| `.bss` | 4KB | Uninitialized data (zeroed at boot) |
| `.stack` | 16 bytes | 512KB stack (NOLOAD - not in binary) |

**Key Symbols Exported:**
- `__bss_start`, `__bss_end` - BSS section boundaries
- `__stack_bottom`, `__stack_top` - Stack boundaries
- `__kernel_end` - End of kernel image

---

### Stage 4: Rust Kernel Entry (`kernel_main`)

**Location:** `src/main.rs:kernel_main()`
**Signature:** `pub extern "C" fn kernel_main() -> !` (never returns)

#### Step 4.1: Initialize Serial Console

```rust
let mut serial = Serial::new();
serial.init();
```

**Serial Driver Details:** `src/serial.rs`

| Parameter | Value |
|-----------|-------|
| Hardware | ARM PrimeCell PL011 UART |
| Base Address | `0x9000000` |
| Baud Rate | 115200 |
| Data Format | 8-bit, FIFO enabled |

**Initialization Sequence:**
1. **Disable UART** → `UART_CR = 0`
2. **Set Baud Rate** → `UART_IBRD = 13`, `UART_FBRD = 2` (for 115200 @ 24MHz clock)
3. **Configure Line Control** → `UART_LCRH = 8-bit + FIFO`
4. **Clear Interrupts** → `UART_ICR = 0x7FF`
5. **Enable UART** → `UART_CR = TX + RX + UARTEN`

---

#### Step 4.2: Print Kernel Banner

```rust
serial.write_str(KERNEL_BANNER);
serial.write_str(KERNEL_VERSION);
serial.write_str("\n\n");
```

**Output:**
```
*********************************************

  SMROS ARM64 Kernel with Preemptive RR Scheduler

*********************************************
  v0.2.0
```

---

#### Step 4.3: Print Initialization Status

```rust
serial.write_str("[OK] Kernel initialized successfully!\n");
serial.write_str("[OK] Serial console initialized\n");
serial.write_str("[OK] ARM64 architecture detected\n");
```

---

#### Step 4.4: Print System Information

```rust
print_system_info(&mut serial);
```

**Registers Read:**

| Register | Description |
|----------|-------------|
| `MPIDR_EL1` | Multiprocessor Affinity Register (CPU ID) |
| `SCTLR_EL1` | System Control Register (EL1) |

**Output Example:**
```
--- System Information ---
[CPU] MPIDR_EL1: 0x80000000
[SYS] SCTLR_EL1: 0x30C50838
--------------------------
```

---

#### Step 4.5: Initialize GIC Interrupt Controller (NEW in v0.2.0)

```rust
interrupt::init();
```

**GICv2 Driver Details:** `src/interrupt.rs`

| Parameter | Value |
|-----------|-------|
| GIC Distributor Base | `0x8000000` |
| GIC CPU Interface Base | `0x8010000` |
| Timer IRQ (PPI 30) | Configured |
| Interrupt Priority | High (0x50) |

**Initialization Sequence:**
1. **Enable Distributor** → `GICD_CTLR = 1`
2. **Set Group 0** → All interrupts as secure (Group 0)
3. **Set Timer Priority** → `GICD_IPRIORITYR` for IRQ 30
4. **Set Target CPU** → `GICD_ITARGETSR` to CPU0
5. **Enable Timer IRQ** → `GICD_ISENABLER` bit 30
6. **Enable CPU Interface** → `GICC_CTLR = 1`
7. **Set Priority Mask** → `GICC_PMR = 0xFF` (allow all)

---

#### Step 4.6: Initialize ARM Generic Timer (NEW in v0.2.0)

```rust
timer::init();
```

**Timer Driver Details:** `src/timer.rs`

| Parameter | Value |
|-----------|-------|
| Timer Type | ARM Generic Timer (Physical) |
| Frequency | Read from `CNTFRQ_EL0` |
| Tick Rate | 100Hz (10ms interval) |
| Control Register | `CNTP_CTL_EL0` |
| Compare Value Register | `CNTP_CVAL_EL0` |

**Initialization Sequence:**
1. **Read Frequency** → `cntfrq_el0`
2. **Calculate Tick Period** → `freq / 100`
3. **Disable Timer** → `cntp_ctl_el0 = 0`
4. **Set Compare Value** → `cntp_cval_el0 = current_count + period`
5. **Enable Timer** → `cntp_ctl_el0 = ENABLE | IMASK`

---

#### Step 4.7: Initialize SMP Support (NEW in v0.2.0)

```rust
smp::init();
```

**SMP Module Details:** `src/smp.rs`

| Feature | Description |
|---------|-------------|
| Max CPUs | 4 (CPU0-CPU3) |
| Boot Method | PSCI `CPU_ON` via HVC |
| CPU States | Offline, Booting, Online |
| Per-CPU Data | Cache-line aligned (64 bytes) |

**Initialization:**
1. **Mark CPU0 Online** → `cpu_info[0].state = Online`
2. **Initialize Per-CPU Structures** → All 4 CPUs marked online for scheduling

---

#### Step 4.8: Initialize Scheduler (NEW in v0.2.0)

```rust
scheduler::scheduler().init();
```

**Scheduler Details:** `src/scheduler.rs`

| Feature | Value |
|---------|-------|
| Type | Preemptive Round-Robin |
| Max Threads | 16 |
| Time Slice | 10 ticks (100ms @ 100Hz) |
| Thread States | Empty, Ready, Running, Blocked, Terminated |
| Idle Thread | Thread 0 (always present) |

**Initialization:**
1. **Initialize TCBs** → All 16 thread slots cleared
2. **Create Idle Thread** → Thread 0 with `idle_thread_entry`
3. **Set Current Thread** → `ThreadId::IDLE`

---

#### Step 4.9: Enable Timer Interrupts (NEW in v0.2.0)

```rust
interrupt::enable_timer_interrupt();
```

**Result:** Timer fires IRQ every 10ms, triggering scheduler tick and potential preemption.

---

#### Step 4.10: Unmask CPU Interrupts (NEW in v0.2.0)

```rust
// Clear I (IRQ mask) bit in DAIF
let daif = daif & !0x80;
```

**Result:** CPU now accepts hardware interrupts (previously all masked).

---

#### Step 4.11: Boot Secondary CPUs (NEW in v0.2.0)

```rust
boot_all_cpus();
```

**PSCI CPU_ON Flow:**
1. **For each CPU (1-3):**
   - `psci_cpu_on(target_mpidr, entry_point, stack_ptr)`
   - HVC call to PSCI firmware
   - Secondary CPU starts at `secondary_entry`
2. **Secondary CPU Initialization:**
   - Set stack pointer
   - Enable FP/SIMD (`CPACR_EL1`)
   - Set exception vectors (`VBAR_EL1`)
   - Unmask interrupts
   - Mark CPU online
   - Start scheduler for this CPU

---

#### Step 4.12: Create Worker Threads (NEW in v0.2.0)

```rust
s.create_thread_on_cpu(thread_worker_1, "worker-1", Some(0));
s.create_thread_on_cpu(thread_worker_2, "worker-2", Some(1));
s.create_thread_on_cpu(thread_worker_3, "worker-3", Some(2));
s.create_thread_on_cpu(thread_worker_4, "worker-4", Some(3));
```

**Thread Creation Flow:**
1. **Allocate Stack** → 8KB via `ThreadStack::alloc()`
2. **Initialize TCB** → Entry point, name, CPU affinity, time slice
3. **Set CPU Context** → Registers (x0-x28, FP, LR, SP, PC, PSTATE)
4. **Mark as Ready** → Thread eligible for scheduling

---

#### Step 4.13: Start First Thread (NEW in v0.2.0)

```rust
start_first_thread();
```

**Context Switch Start:**
1. **Find First Ready Thread** → Scan TCBs for `Ready` state
2. **Update States** → Idle → Ready, Worker → Running
3. **`context_switch_start()`** → Load CPU context, jump to thread entry
4. **Never Returns** → CPU0 now executing worker thread

**Note:** After this call, CPU0 is running worker threads. The kernel main function continues only after all worker threads complete.

---

### Stage 5: Panic Handler (Error Path)

**Location:** `src/main.rs:panic()`

If the kernel panics (via `panic!()` macro or assertion failure):

```rust
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("\n!!! KERNEL PANIC !!!\n");
    // Print file and line number
    // ...
    serial.write_str("\n[ERROR] System halted\n");

    loop {
        cortex_a::asm::wfi();
    }
}
```

**Flow:**
1. Re-initialize serial console
2. Print panic message with file/line info
3. Enter infinite `WFI` loop

---

### Stage 6: Interrupt Handling (NEW in v0.2.0)

**Trigger:** Timer IRQ fires every 10ms (PPI 30)

#### Step 6.1: IRQ Entry (Assembly)

Defined in `global_asm!()` block in `main.rs`:

```assembly
irq_handler_sp:
    // Save caller-saved registers (x0-x15)
    sub     sp, sp, #128
    stp     x0, x1, [sp, #0]
    // ... (save x2-x15)

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Check if preemption is needed
    bl      check_preemption

    // Restore caller-saved registers
    ldp     x0, x1, [sp, #0]
    // ... (restore x2-x15)
    add     sp, sp, #128

    eret
```

---

#### Step 6.2: Timer Interrupt Handler (Rust)

```rust
#[no_mangle]
extern "C" fn timer_interrupt_handler() {
    timer::clear_interrupt();      // Re-arm timer for next tick
    let interrupt_id = interrupt::acknowledge_interrupt();
    scheduler::scheduler().on_timer_tick();  // Increment tick count
    interrupt::end_of_interrupt(interrupt_id);
}
```

**Actions:**
1. **Clear Timer Interrupt** → Re-arm compare value for next 10ms
2. **Acknowledge at GIC** → Read `GICC_IAR` to get IRQ ID
3. **Update Scheduler** → Increment `tick_count`, decrement current thread's `time_slice`
4. **End of Interrupt** → Write to `GICC_EOIR`

---

#### Step 6.3: Preemption Check (Rust)

```rust
#[no_mangle]
extern "C" fn check_preemption() {
    let cpu_id = current_cpu_id();
    let s = scheduler::scheduler();

    if s.should_preempt() {
        if let Some(next_id) = s.schedule_next_for_cpu(cpu_id as usize) {
            s.reset_time_slice(next_id);
        }
        schedule_on_cpu(cpu_id as usize);
    }
}
```

**Preemption Condition:** `time_slice == 0 && active_threads > 1`

**If Preempting:**
1. **Find Next Thread** → Round-robin scan for `Ready` thread on this CPU
2. **Reset Time Slice** → Set next thread's `time_slice` to 10 ticks
3. **Context Switch** → `context_switch(current_tcb, next_tcb)`

---

### Stage 7: Multi-Threaded Execution (NEW in v0.2.0)

**Worker Thread Lifecycle:**

```
Thread Entry (e.g., thread_worker_1)
    │
    ├─ Initialize serial console
    │
    ├─ Get CPU affinity from scheduler
    │
    ├─ Loop (3 iterations):
    │   │
    │   ├─ Print "Running iteration N/3"
    │   │
    │   ├─ Spin loop (100000 iterations)
    │   │
    │   ├─ Print "About to yield..."
    │   │
    │   ├─ yield_now() → Set time_slice=0, call schedule()
    │   │   │
    │   │   ├─ [Context Switch to Next Thread]
    │   │   │
    │   │   └─ [Return from Context Switch]
    │   │
    │   └─ Print "Returned from yield!"
    │
    ├─ Print "Completed all iterations, terminating..."
    │
    ├─ scheduler().terminate_current()
    │
    ├─ schedule() → Switch to next thread
    │
    └─ Idle loop (cortex_a::asm::wfi())
```

**Context Switch Details:**

```assembly
context_switch:
    // Disable interrupts
    mrs     x16, daif
    orr     x16, x16, #0x80
    msr     daif, x16

    // Save callee-saved registers (x19-x28, FP, LR)
    add     x16, x0, #0x10    // current TCB + context offset
    stp     x19, x20, [x16, #0x98]
    // ... (save x21-x28)
    stp     x29, x30, [x16, #0xE8]

    // Save current SP
    mov     x17, sp
    str     x17, [x16, #0xF8]

    // Save return address (PC)
    str     x30, [x16, #0x100]

    // Load next thread context
    add     x16, x1, #0x10    // next TCB + context offset

    // Restore SP
    ldr     x17, [x16, #0xF8]
    mov     sp, x17

    // Restore callee-saved registers
    ldp     x19, x20, [x16, #0x98]
    // ... (restore x21-x28)
    ldp     x29, x30, [x16, #0xE8]

    // Load saved PC
    ldr     x16, [x16, #0x100]

    // Enable interrupts
    mrs     x17, daif
    bic     x17, x17, #0x80
    msr     daif, x17

    // Branch to saved PC
    br      x16
```

**Registers Saved/Restored:**
- `x19-x28`: Callee-saved general purpose registers
- `x29 (FP)`: Frame pointer
- `x30 (LR)`: Link register (return address for thread)
- `SP`: Stack pointer
- `PC`: Program counter (entry point for new threads)

---

## Build & Execution Flow

### Build Process

```
┌─────────────────────────────────────────────────────────────┐
│ make build                                                 │
├─────────────────────────────────────────────────────────────┤
│ 1. cargo build --release                                   │
│    - Compiles Rust code for aarch64-unknown-none           │
│    - Uses build-std (core, alloc, compiler_builtins)       │
│    - Applies linker script: -Tlinker/kernel.ld             │
│    - Generates: target/aarch64-unknown-none/release/smros  │
│                                                            │
│ 2. cp target/.../smros kernel8.img                        │
│    - Copies ELF binary to kernel8.img                      │
│    - QEMU expects this format for -kernel flag             │
└─────────────────────────────────────────────────────────────┘
```

### QEMU Execution

```bash
qemu-system-aarch64 \
    -M virt \           # Virtual machine type
    -cpu cortex-a57 \   # CPU model
    -smp 4 \            # 4 CPU cores (only CPU0 boots initially)
    -m 512M \           # 512MB RAM
    -nographic \        # No GUI, serial console only
    -kernel kernel8.img # Load kernel image
```

---

## Multi-Process Memory Management (NEW in v0.3.0)

### Process Address Space Layout

Each process in SMROS gets its own virtual address space with isolated segments:

```
Process Virtual Memory (per process):

0x0000_0000_0000_0000 ──────────────┐
                                     │ Code Segment (1 page, r-x)
0x0000_0000_0001_0000 ──────────────┤
                                     │ Data Segment (1 page, rw-)
0x0000_0000_0002_0000 ──────────────┤
                                     │ Heap Segment (4 pages, rw-)
                                     │ Grows upward ↑
0x0000_0000_0006_0000 ──────────────┤
                                     │ (gap)
0x0000_0000_000F_0000 ──────────────┤
                                     │ Stack Segment (2 pages, rw-)
                                     │ Grows downward ↓
0x0000_0000_0011_0000 ──────────────┘
```

### Page Frame Allocator

Physical memory management with bitmap allocator:

- **Total Physical Memory**: 16MB (4096 pages × 4KB)
- **Bitmap Size**: 64 × 64-bit integers (4096 bits)
- **Allocation Strategy**: First-fit bitmap scan
- **Max Pages per Process**: 64 pages (256KB virtual memory)

**Allocation Example:**
```rust
// Allocate a page
if let Some(pfn) = PageFrameAllocator::alloc() {
    // PFN = Physical Frame Number
    // Map to virtual address in process page table
    process.pages[idx] = PageEntry {
        pfn: pfn,
        valid: true,
        writable: true,
        executable: false,
        user_accessible: true,
    };
}
```

### Process Creation Flow

When `create_process("name")` is called:

1. **Find empty PCB slot** - Scan process table for `ProcessState::Empty`
2. **Assign PID** - Atomic increment of `next_pid`
3. **Initialize PCB**:
   - Set PID, parent PID (1 = init), name
   - Set state to `Ready`
4. **Create Address Space**:
   - Allocate code segment (1 page from physical allocator)
   - Allocate data segment (1 page)
   - Allocate heap segment (4 pages)
   - Allocate stack segment (2 pages)
   - Set up page table entries
5. **Update counters** - `active_processes += 1`

### Process Memory Isolation

Each process has:
- **Separate page table** - No shared pages between processes
- **Separate heap** - Independent heap allocation
- **Separate stack** - Isolated stack space
- **Permission flags** - Read/write/execute controls per page

---

## Interactive Shell (NEW in v0.3.0)

### Shell Entry Point

After all initialization, `kernel_main()` calls:

```rust
memory::start_shell();
```

This function:
1. Creates `Shell` struct with serial and input buffer
2. Initializes serial for input/output
3. Prints welcome banner
4. Enters infinite command loop

### Shell Main Loop

```rust
pub fn run(&mut self) -> ! {
    self.print_welcome();
    
    loop {
        self.print_prompt();           // "smros$ "
        let len = self.serial.read_line(&mut self.input_buf);
        
        if len == 0 { continue; }
        
        let args = Self::parse_command(command_str);
        if args.is_empty() { continue; }
        
        self.execute_command(&args);
    }
}
```

### Serial Input Processing

The `read_line()` function handles:

- **Blocking wait** - Waits indefinitely for input
- **Character echo** - Sends each character back to terminal
- **Backspace handling** - Deletes previous character with visual feedback
- **Control characters**:
  - `Ctrl+C` (0x03) - Cancel line
  - `Ctrl+U` (0x15) - Clear entire line
  - `Ctrl+L` (0x0C) - Clear screen
- **Line termination** - Enter (CR/LF) ends input

### Command Execution

Commands are dispatched via `match` statement:

```rust
match cmd {
    "help"    => cmd_help(serial, args),
    "ps"      => cmd_ps(serial, args),
    "top"     => cmd_top(serial, args),
    "meminfo" => cmd_meminfo(serial, args),
    "pages"   => cmd_pages(serial, args),
    "heap"    => cmd_heap(serial, args),
    "tree"    => cmd_tree(serial, args),
    "kill"    => cmd_kill(serial, args),
    "info"    => cmd_info(serial, args),
    "uptime"  => cmd_uptime(serial, args),
    "version" => cmd_version(serial, args),
    "whoami"  => cmd_whoami(serial, args),
    "echo"    => cmd_echo(serial, args),
    "clear"   => cmd_clear(serial, args),
    _         => print_error(serial, cmd),
}
```

### Shell Commands Overview

#### Process Management

- **`ps`** - Lists all processes with PID, state, name, threads, parent
- **`top`** - Formatted process monitor with memory usage
- **`tree`** - Process tree visualization
- **`kill <pid>`** - Terminates a process (protects init process)
- **`info [pid]`** - Detailed address space info

#### Memory Management

- **`meminfo`** - System memory stats (total/used/free)
- **`pages`** - Per-process page allocation details
- **`heap`** - Heap usage per process

#### System Commands

- **`help`** - Command reference
- **`version`** - Kernel version and features
- **`uptime`** - Scheduler tick count
- **`echo`** - Print text
- **`clear`** - Clear screen (ANSI codes)

---

## Complete Boot Timeline (v0.3.0)

```
Time →
─────────────────────────────────────────────────────────────────────────────────
QEMU Start (4 CPUs configured)
    │
    ├─ Load kernel8.img @ 0x40000000
    │
    ▼
_start (Assembly) - CPU0 Only
    │
    ├─ Read MPIDR_EL1 → CPU ID = 0 (Boot CPU)
    │
    ├─ Mask interrupts (DAIF ← 0x3C0)
    │
    ├─ Set SP ← __stack_top (512KB stack)
    │
    ├─ Zero BSS section
    │
    ├─ Set VBAR_EL1 ← exception_vectors
    │
    ▼
kernel_main() (Rust) - CPU0
    │
    ├─ Serial::new() → UART @ 0x9000000
    │
    ├─ serial.init() → 115200 8N1, FIFO
    │
    ├─ Print banner "SMROS ARM64 Kernel v0.3.0"
    │
    ├─ Print "[OK] Kernel initialized..."
    │
    ├─ print_system_info()
    │   ├─ Read MPIDR_EL1
    │   └─ Read SCTLR_EL1
    │
    ├─ interrupt::init() → GICv2 setup
    │
    ├─ timer::init() → ARM Generic Timer 100Hz
    │
    ├─ smp::init() → Per-CPU data structures
    │
    ├─ memory::init() → Process & memory management
    │   ├─ Initialize process manager
    │   ├─ Create init process (PID 1)
    │   ├─ Allocate init's address space:
    │   │   ├─ Code segment (1 page @ 0x0000, r-x)
    │   │   ├─ Data segment (1 page @ 0x1000, rw-)
    │   │   ├─ Heap segment (4 pages @ 0x2000, rw-)
    │   │   └─ Stack segment (2 pages @ 0xF000, rw-)
    │   └─ Initialize page frame allocator
    │
    ├─ scheduler::init() → Create idle thread
    │
    ├─ enable_timer_interrupt() → PPI 30 enabled
    │
    ├─ Unmask DAIF I bit → Interrupts enabled
    │
    ├─ boot_all_cpus() → PSCI CPU_ON for CPU1-3
    │   │
    │   ├─ [CPU1] secondary_entry → secondary_cpu_entry()
    │   │   ├─ Enable FP/SIMD
    │   │   ├─ Set VBAR_EL1
    │   │   ├─ Unmask interrupts
    │   │   └─ Mark CPU online
    │   │
    │   ├─ [CPU2] secondary_entry → secondary_cpu_entry()
    │   │   └─ (Same as CPU1)
    │   │
    │   └─ [CPU3] secondary_entry → secondary_cpu_entry()
    │       └─ (Same as CPU1)
    │
    ├─ Create sample processes for demo
    │   ├─ shell (PID 2)
    │   ├─ editor (PID 3)
    │   └─ compiler (PID 4)
    │
    ├─ Print process & memory status
    │   ├─ Process table (PID, state, name, segments, pages)
    │   ├─ Physical memory usage (total/used/free pages)
    │   └─ Per-process address space details
    │
    ├─ Print "[INFO] Boot complete! Entering shell..."
    │
    ▼
memory::start_shell() - Interactive Shell Mode
    │
    ├─ Create Shell struct
    │   ├─ serial: Serial instance
    │   └─ input_buf: [u8; 256]
    │
    ├─ Print welcome banner
    │   ╔═══════════════════════════════════════════╗
    │   ║  SMROS Shell v0.3.0 - Process Management  ║
    │   ╚═══════════════════════════════════════════╝
    │
    └─ Enter shell main loop:
        │
        ├─ [Loop]
        │   ├─ Print prompt: "smros$ "
        │   │
        │   ├─ Wait for user input (serial.read_line)
        │   │   ├─ Read characters one-by-one
        │   │   ├─ Echo each character
        │   │   ├─ Handle backspace/Ctrl+C/etc.
        │   │   └─ Terminate on Enter
        │   │
        │   ├─ Parse command (split by whitespace)
        │   │
        │   ├─ Execute command:
        │   │   ├─ "help" → Show command reference
        │   │   ├─ "ps" → List all processes
        │   │   ├─ "top" → Process monitor
        │   │   ├─ "meminfo" → Memory statistics
        │   │   ├─ "pages" → Page allocation details
        │   │   ├─ "heap" → Heap usage
        │   │   ├─ "tree" → Process tree
        │   │   ├─ "kill" → Terminate process
        │   │   ├─ "info" → Process details
        │   │   ├─ "version" → Kernel version
        │   │   ├─ "uptime" → System uptime
        │   │   ├─ "echo" → Print text
        │   │   ├─ "clear" → Clear screen
        │   │   └─ unknown → Error message
        │   │
        │   └─ Print output to serial console
        │
        └─ [Continue loop indefinitely]
        
    │
    ▼
User Exits QEMU (Ctrl+A, X)
    │
    └─ QEMU terminates

─────────────────────────────────────────────────────────────────────────────────
```

## Key Changes from v0.2.0 to v0.3.0

### Removed
- ❌ Worker thread creation (thread_worker_1 through thread_worker_4)
- ❌ `start_first_thread()` call
- ❌ Multi-threaded execution demo
- ❌ Idle loop at end of `kernel_main()`

### Added
- ✅ Memory management initialization (`memory::init()`)
- ✅ Process creation (init, shell, editor, compiler)
- ✅ Process & memory status printing
- ✅ Interactive shell entry (`memory::start_shell()`)
- ✅ Serial input support (read_line, backspace, Ctrl+C, etc.)
- ✅ 15+ shell commands (ps, top, meminfo, pages, heap, tree, etc.)

### Changed Flow
- **v0.2.0**: Boot → Initialize → Create threads → Run threads → Idle loop
- **v0.3.0**: Boot → Initialize → Create processes → Enter shell → Wait for commands

---

## Interrupt Handling (Reference from v0.2.0)

**Trigger:** Timer IRQ fires every 10ms (PPI 30)

### IRQ Entry (Assembly)

Defined in `global_asm!()` block in `main.rs`:

```assembly
irq_handler_sp:
    // Save caller-saved registers (x0-x15)
    sub     sp, sp, #128
    stp     x0, x1, [sp, #0]
    // ... (save x2-x15)

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Check if preemption is needed
    bl      check_preemption

    // Restore caller-saved registers
    ldp     x0, x1, [sp, #0]
    // ... (restore x2-x15)
    add     sp, sp, #128

    eret
```

---

## Key Files & Their Roles

| File | Role in Boot |
|------|--------------|
| `linker/kernel.ld` | Defines memory layout, sections, and symbols (`_start`, `__stack_top`, etc.) |
| `.cargo/config.toml` | Sets target (`aarch64-unknown-none`), linker flags, and `build-std` |
| `src/main.rs` | Contains `_start` assembly, exception vectors, context switch, and `kernel_main()` |
| `src/serial.rs` | PL011 UART driver for serial I/O with line input |
| `src/timer.rs` | ARM Generic Timer driver for system timing and scheduler ticks |
| `src/interrupt.rs` | GICv2 interrupt controller driver |
| `src/scheduler.rs` | Preemptive round-robin scheduler with CPU affinity |
| `src/thread.rs` | Thread control block, CPU context, and stack management |
| `src/memory.rs` | Multi-process memory management & interactive shell |
| `src/smp.rs` | SMP multi-core support (PSCI CPU_ON, per-CPU data) |
| `src/drivers.rs` | Driver module re-exports |
| `Cargo.toml` | Dependencies (`cortex-a`, `tock-registers`, `volatile`, `cc`), panic strategy |
| `Makefile` | Build automation (`make build`, `make run`) |

---

## Dependencies Used During Boot

| Crate | Version | Usage |
|-------|---------|-------|
| `cortex-a` | 8 | Register access (`MPIDR_EL1`, `SCTLR_EL1`), `wfi()` instruction |
| `tock-registers` | 0.8 | Register interface traits |
| `volatile` | 0.4 | Volatile memory access (for hardware registers) |
| `cc` | 1.0 | Build dependency for C/assembly compilation |

---

## Implemented Features (v0.3.0)

- [x] Exception vector table (16 entries with IRQ handlers)
- [x] Interrupt handling (GICv2 with timer IRQ)
- [x] ARM Generic Timer driver (100Hz tick)
- [x] Preemptive round-robin scheduler
- [x] Thread management (TCB, CPU context, stack allocation)
- [x] Context switching (assembly implementation)
- [x] SMP multi-core boot (PSCI CPU_ON for 4 CPUs)
- [x] CPU affinity for threads
- [x] Heap/allocator initialization (1MB bump allocator)
- [x] **Multi-process memory management** (NEW)
- [x] **4K page-based allocation** (NEW)
- [x] **Process isolation with segments** (NEW)
- [x] **Interactive shell with 15+ commands** (NEW)
- [x] **Serial input with line editing** (NEW)
- [x] Voluntary thread yield (`yield_now()`)
- [x] Thread termination and cleanup

## Future Enhancements (Not Yet Implemented)

- [ ] MMU setup (currently running with identity mapping)
- [ ] True virtual memory with page tables
- [ ] Multi-core scheduling (true parallel execution)
- [ ] Device tree parsing
- [ ] Memory protection (EL1/EL0 separation)
- [ ] User-space processes
- [ ] IPC mechanisms
- [ ] File system support
- [ ] Network driver
- [ ] Command history in shell
- [ ] Tab completion
- [ ] Process creation from shell

---

## References

- [ARM Architecture Reference Manual (ARMv8-A)](https://developer.arm.com/documentation/)
- [QEMU ARM64 virt Machine Documentation](https://www.qemu.org/docs/master/system/arm/virt.html)
- [Rust Embedded Book](https://docs.rust-embedded.org/book/)
- [AArch64 Boot Protocol](https://github.com/torvalds/linux/blob/master/Documentation/arm64/booting.rst)
- [ARM Generic Timer Documentation](https://developer.arm.com/documentation/100746/0100/aarch64-register-descriptions/cntfrq-el0)
- [GICv2 Architecture Specification](https://developer.arm.com/documentation/ihi0048/latest/)
- [PSCI Specification](https://developer.arm.com/documentation/den0022/latest/)
- [AArch64 Exception Levels](https://developer.arm.com/documentation/102411/0100/Exception-levels)
- [ARM Context Switching Guide](https://developer.arm.com/documentation/100934/0100/Context-switching)

