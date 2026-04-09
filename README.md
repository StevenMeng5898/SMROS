# SMROS - ARM64 OS Kernel

A preemptive multitasking ARM64 OS kernel framework written in Rust, designed to run on QEMU with SMP multi-core support.

## Features

- **Bare-metal Rust**: No standard library, pure `#![no_std]` kernel
- **ARM64 Architecture**: Targets AArch64 processors
- **QEMU Support**: Runs on QEMU virt machine with 4 CPU cores
- **Serial Console**: PL011 UART driver for output
- **Boot Assembly**: Custom boot code for ARM64 initialization
- **Preemptive Round-Robin Scheduler**: Time-slice based scheduling with voluntary and forced preemption
- **SMP Multi-Core Support**: Boots and manages multiple CPU cores using PSCI
- **Thread Management**: Full thread abstraction with CPU affinity binding
- **Context Switching**: Assembly-based context switch for ARM64
- **GICv2 Interrupt Controller**: Hardware interrupt handling
- **ARM Generic Timer**: System timer with configurable tick rate (100Hz default)
- **Memory Allocator**: Global kernel allocator with bump allocation
- **Exception Vectors**: Full exception vector table with IRQ handlers
- **Panic Handler**: Graceful kernel panic handling with serial output

## Prerequisites

### Rust Toolchain

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install ARM64 target
rustup target add aarch64-unknown-none

# Install rust-src component for build-std
rustup component add rust-src
```

### QEMU

```bash
# Ubuntu/Debian
sudo apt-get install qemu-system-arm

# macOS
brew install qemu

# Arch Linux
sudo pacman -S qemu
```

## Building

```bash
# Using Make
make build

# Or using cargo directly
cargo build --release

# The kernel image will be created as kernel8.img
```

## Running

```bash
# Using Make (builds and runs)
make run

# Or using the script
./scripts/run-simple.sh

# Or manually with QEMU (4 CPU cores for SMP)
qemu-system-aarch64 -M virt -cpu cortex-a57 -m 512M -smp 4 -nographic -kernel kernel8.img
```

### Exit QEMU

Press `Ctrl+A`, then `X` to exit QEMU.

## Debugging

### Debug Mode (with logging)

```bash
make debug
```

This runs QEMU with additional logging. Check `qemu.log` for details.

### GDB Debugging

```bash
# Start QEMU with GDB server
make gdb

# In another terminal, connect with GDB
gdb
(gdb) target remote :1234
(gdb) symbol-file target/aarch64-unknown-none/release/smros
```

## Project Structure

```
SMROS/
├── Cargo.toml          # Rust package configuration
├── Makefile            # Build automation
├── build.rs            # Build script for C compilation
├── .cargo/
│   └── config.toml     # Cargo configuration for ARM64
├── linker/
│   └── kernel.ld       # Linker script for ARM64
├── src/
│   ├── main.rs         # Kernel entry point, boot assembly, exception vectors
│   ├── serial.rs       # PL011 UART driver
│   ├── timer.rs        # ARM Generic Timer driver
│   ├── interrupt.rs    # GICv2 interrupt controller driver
│   ├── scheduler.rs    # Preemptive round-robin scheduler
│   ├── thread.rs       # Thread management (TCB, CPU context, stack)
│   ├── smp.rs          # SMP multi-core support (PSCI CPU_ON)
│   ├── drivers.rs      # Driver module re-exports
│   └── context_switch.S # Assembly context switch code
└── scripts/
    ├── build.sh        # Build script
    ├── run.sh          # Run script (debug mode)
    └── run-simple.sh   # Run script (simple mode)
```

## Memory Layout

```
0x00000000 - 0x0007FFFF: Reserved
0x00080000 - 0x3FFFFFFF: Available RAM
0x40000000 - ...:         Kernel code/data
...
Stack: 512KB allocated at kernel end
Heap:  1MB static bump allocator
```

## Serial Output

The kernel uses the PL011 UART at address `0x9000000` for serial output, which is mapped to the QEMU serial console.

## Interrupt Handling

The kernel implements a GICv2 interrupt controller driver for handling hardware interrupts:

- **GIC Distributor**: Configures interrupt groups, priorities, and CPU targets
- **GIC CPU Interface**: Acknowledges and ends interrupts
- **Timer Interrupt (PPI 30)**: Used for scheduler ticks at 100Hz
- **Exception Vectors**: Full 16-entry vector table with handlers for synchronous exceptions and IRQs

## Timer Driver

The ARM Generic Timer provides system timing:

- **Counter-timer Frequency**: Read from `CNTFRQ_EL0`
- **Physical Count**: Read from `CNTPCT_EL0`
- **Compare Value**: Set via `CNTP_CVAL_EL0` for periodic ticks
- **Tick Rate**: 100Hz (10ms interval) by default

## Scheduler

The preemptive round-robin scheduler manages thread execution:

- **Time Slice**: 10 ticks (100ms at 100Hz) per thread
- **Preemption**: Forced context switch when time slice expires
- **Voluntary Yield**: Threads can yield via `yield_now()`
- **Thread States**: Empty, Ready, Running, Blocked, Terminated
- **Max Threads**: 16 concurrent threads
- **Idle Thread**: Always present (thread 0)

## Thread Management

Threads are managed via Thread Control Blocks (TCBs):

- **CPU Context**: Full ARM64 register state (x0-x28, FP, LR, SP, PC, PSTATE)
- **Stack Allocation**: 8KB per thread, dynamically allocated
- **CPU Affinity**: Threads can be bound to specific CPUs
- **Thread Entry**: `extern "C" fn() -> !` (never returns)

## SMP Multi-Core Support

Secondary CPUs are booted using PSCI (Power State Coordination Interface):

- **PSCI CPU_ON**: HVC call to boot secondary CPUs
- **CPU States**: Offline, Booting, Online
- **Per-CPU Data**: Cache-line aligned structures for each CPU
- **CPU-Aware Scheduling**: Threads bound to specific CPUs are scheduled on those CPUs

## Customization

### Adding New Drivers

1. Create a new module in `src/`
2. Add the module declaration in `main.rs` or `drivers.rs`
3. Initialize the driver in `kernel_main()`

### Adding New Threads

```rust
// Create a thread on any CPU
scheduler::scheduler().create_thread(my_thread_func, "my-thread");

// Create a thread bound to a specific CPU
scheduler::scheduler().create_thread_on_cpu(my_thread_func, "my-thread", Some(0));
```

### Changing Memory Layout

Edit `linker/kernel.ld` to modify:

- Kernel base address
- Section alignment
- Stack size

### Changing Timer Frequency

Modify the tick period calculation in `src/timer.rs`:

```rust
// For 100Hz (10ms tick)
let tick_period = freq / 100;

// For 1000Hz (1ms tick)
let tick_period = freq / 1000;
```

### Adding Interrupt Handling

1. Define handlers in exception vectors in `main.rs`
2. Implement handler functions in Rust
3. Configure interrupt priorities in `interrupt.rs`

## Troubleshooting

### Build Errors

```bash
# Ensure ARM64 target is installed
rustup target add aarch64-unknown-none

# Ensure rust-src is available
rustup component add rust-src
```

### QEMU Errors

```bash
# Verify QEMU installation
qemu-system-aarch64 --version

# Check machine type support
qemu-system-aarch64 -M help | grep virt
```

## Dependencies


| Crate            | Version | Usage                                                           |
| ---------------- | ------- | --------------------------------------------------------------- |
| `cortex-a`       | 8       | Register access (`MPIDR_EL1`, `SCTLR_EL1`), `wfi()` instruction |
| `tock-registers` | 0.8     | Register interface traits                                       |
| `volatile`       | 0.4     | Volatile memory access (for hardware registers)                 |
| `cc`             | 1.0     | Build dependency for C code compilation                         |

## License

This project is open source,  based on MiT Licnse

## References

- [Rust Embedded Book](https://docs.rust-embedded.org/book/)
- [ARM Architecture Reference Manual](https://developer.arm.com/documentation/)
- [QEMU ARM64 Virt Machine](https://www.qemu.org/docs/master/system/arm/virt.html)
- [ARM Generic Timer Documentation](https://developer.arm.com/documentation/100746/0100/aarch64-register-descriptions/cntfrq-el0)
- [GICv2 Architecture Specification](https://developer.arm.com/documentation/ihi0048/latest/)
- [PSCI Specification](https://developer.arm.com/documentation/den0022/latest/)
- [AArch64 Exception Levels](https://developer.arm.com/documentation/102411/0100/Exception-levels)
