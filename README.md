# SMROS - ARM64 OS Kernel

A minimal ARM64 OS kernel framework written in Rust, designed to run on QEMU.

## Features

- **Bare-metal Rust**: No standard library, pure `#![no_std]` kernel
- **ARM64 Architecture**: Targets AArch64 processors
- **QEMU Support**: Runs on QEMU virt machine
- **Serial Console**: PL011 UART driver for output
- **Boot Assembly**: Custom boot code for ARM64 initialization

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

# Or manually with QEMU
qemu-system-aarch64 -M virt -cpu cortex-a57 -m 512M -nographic -kernel kernel8.img
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
├── .cargo/
│   └── config.toml     # Cargo configuration for ARM64
├── linker/
│   └── kernel.ld       # Linker script for ARM64
├── src/
│   ├── boot.s          # Assembly boot code
│   ├── main.rs         # Kernel entry point
│   ├── serial.rs       # PL011 UART driver
│   └── drivers.rs      # Driver modules
└── scripts/
    ├── build.sh        # Build script
    ├── run.sh          # Run script (debug mode)
    └── run-simple.sh   # Run script (simple mode)
```

## Memory Layout

```
0x00000000 - 0x0007FFFF: Reserved
0x00080000 - ...........: Kernel code/data
...
Stack: 512KB allocated at kernel end
```

## Serial Output

The kernel uses the PL011 UART at address `0x9000000` for serial output, which is mapped to the QEMU serial console.

## Customization

### Adding New Drivers

1. Create a new module in `src/`
2. Add the module declaration in `drivers.rs`
3. Initialize the driver in `kernel_main()`

### Changing Memory Layout

Edit `linker/kernel.ld` to modify:
- Kernel base address
- Section alignment
- Stack size

### Adding Interrupt Handling

1. Define exception vectors in `boot.s`
2. Implement exception handlers in Rust
3. Set VBAR_EL1 to point to your vectors

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

## License

This project is open source. Feel free to use and modify as needed.

## References

- [Rust Embedded Book](https://docs.rust-embedded.org/book/)
- [ARM Architecture Reference Manual](https://developer.arm.com/documentation/)
- [QEMU ARM64 Virt Machine](https://www.qemu.org/docs/master/system/arm/virt.html)
