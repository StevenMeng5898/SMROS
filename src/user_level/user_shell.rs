//! EL0 Shell - User-mode shell implementation
//!
//! This shell runs in EL0 (user mode) and makes syscalls
//! to interact with the kernel. It uses the syscall interface
//! for I/O operations instead of direct hardware access.

use crate::kernel_lowlevel::memory::{
    process_manager, PageFrameAllocator, ProcessState, PAGE_SIZE,
};
use crate::kernel_lowlevel::serial::Serial;
use crate::kernel_objects::scheduler;
use crate::user_level::user_logic;
use crate::user_level::user_test::test_write;
use alloc::string::String;
use alloc::vec::Vec;

/// Shell command handlers
struct ShellCommand {
    name: &'static str,
    description: &'static str,
    handler: fn(&mut ShellContext, &[&str]),
}

/// Shell context shared across command handlers
struct ShellContext {
    serial: Serial,
    command_count: u32,
}

/// Available shell commands
const SHELL_COMMANDS: &[ShellCommand] = &[
    ShellCommand {
        name: "help",
        description: "Show this help message",
        handler: cmd_help,
    },
    ShellCommand {
        name: "version",
        description: "Show kernel version",
        handler: cmd_version,
    },
    ShellCommand {
        name: "ps",
        description: "List all processes",
        handler: cmd_ps,
    },
    ShellCommand {
        name: "top",
        description: "Show process status monitor",
        handler: cmd_top,
    },
    ShellCommand {
        name: "meminfo",
        description: "Show memory information",
        handler: cmd_meminfo,
    },
    ShellCommand {
        name: "uptime",
        description: "Show system uptime",
        handler: cmd_uptime,
    },
    ShellCommand {
        name: "kill",
        description: "Terminate a process by PID",
        handler: cmd_kill,
    },
    ShellCommand {
        name: "testsc",
        description: "Test Linux and Zircon memory syscalls",
        handler: cmd_test_syscall,
    },
    ShellCommand {
        name: "echo",
        description: "Echo arguments back",
        handler: cmd_echo,
    },
    ShellCommand {
        name: "clear",
        description: "Clear the screen",
        handler: cmd_clear,
    },
    ShellCommand {
        name: "exit",
        description: "Exit the shell",
        handler: cmd_exit,
    },
];

/// Shell structure
pub struct UserShell {
    context: ShellContext,
    input_buf: [u8; 256],
    input_len: usize,
}

impl UserShell {
    /// Create a new user shell
    pub fn new() -> Self {
        let mut serial = Serial::new();
        serial.init();

        Self {
            context: ShellContext {
                serial,
                command_count: 0,
            },
            input_buf: [0; 256],
            input_len: 0,
        }
    }

    /// Print a string
    fn print(&mut self, s: &str) {
        self.context.serial.write_str(s);
    }

    /// Print shell welcome message
    fn print_welcome(&mut self) {
        self.print("\n");
        self.print("╔═══════════════════════════════════════════════════════════╗\n");
        self.print("║                                                           ║\n");
        self.print("║     SMROS User-Mode Shell v0.5.0                         ║\n");
        self.print("║                                                           ║\n");
        self.print("╚═══════════════════════════════════════════════════════════╝\n");
        self.print("\n");
        self.print("Welcome to SMROS shell!\n");
        self.print("Type 'help' for available commands.\n\n");
    }

    /// Print shell prompt
    fn print_prompt(&mut self) {
        self.print("smros> ");
    }

    /// Read a line of input from serial (waits for timer interrupt to yield)
    fn read_line(&mut self) -> String {
        self.input_len = 0;

        loop {
            // Read from UART data register
            const UART_BASE: usize = 0x9000000;
            const UART_FR: usize = 0x18;
            const UART_DR: usize = 0x00;
            const FR_RXFE: u32 = 1 << 4;

            // Check if RX FIFO is empty
            let fr = unsafe { core::ptr::read_volatile((UART_BASE + UART_FR) as *const u32) };

            if fr & FR_RXFE != 0 {
                // No data available - wait for next timer tick
                // This gives the user time to type (10ms at 100Hz)
                // Then preemption will return to this thread
                cortex_a::asm::wfe();
                continue;
            }

            // Read character
            let c = unsafe { core::ptr::read_volatile((UART_BASE + UART_DR) as *const u8) };

            if c == b'\r' || c == b'\n' {
                // End of line
                self.print("\n");
                break;
            } else if c == b'\x08' || c == b'\x7f' {
                // Backspace
                if self.input_len > 0 {
                    self.input_len -= 1;
                    self.print("\x08 \x08");
                }
            } else if user_logic::ascii_shell_input(c) {
                // Printable character
                if self.input_len < 255 {
                    self.input_buf[self.input_len] = c;
                    self.input_len += 1;
                    self.print_byte(c);
                }
            }
        }

        // Return the line as a String to avoid borrowing issues
        String::from_utf8_lossy(&self.input_buf[..self.input_len]).into_owned()
    }

    /// Print a single byte
    fn print_byte(&mut self, c: u8) {
        self.context.serial.write_byte(c);
    }

    /// Execute a command
    fn execute_command(&mut self, cmd: &str, args: &[&str]) {
        self.context.command_count = self.context.command_count.saturating_add(1);

        // Find command in table
        for command in SHELL_COMMANDS {
            if cmd == command.name {
                (command.handler)(&mut self.context, args);
                return;
            }
        }

        // Command not found
        self.print("Unknown command: ");
        self.print(cmd);
        self.print("\nType 'help' for available commands.\n");
    }

    /// Run the shell main loop
    pub fn run(&mut self) -> ! {
        self.print_welcome();

        loop {
            self.print_prompt();

            // Read user input (yields to scheduler while waiting)
            let line = self.read_line();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Parse command - extract first word
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let cmd = parts[0];

            // Execute command
            self.execute_command(cmd, &parts[1..]);
        }
    }
}

// ============================================================================
// Command Handlers
// ============================================================================

/// Command: help - Show available commands
fn cmd_help(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial.write_str("\nAvailable commands:\n\n");

    // Find longest command for alignment
    let mut max_len = 0;
    for cmd in SHELL_COMMANDS {
        if cmd.name.len() > max_len {
            max_len = cmd.name.len();
        }
    }

    for cmd in SHELL_COMMANDS {
        ctx.serial.write_str("  ");
        ctx.serial.write_str(cmd.name);
        for _ in 0..(max_len - cmd.name.len() + 2) {
            ctx.serial.write_byte(b' ');
        }
        ctx.serial.write_str(cmd.description);
        ctx.serial.write_str("\n");
    }

    ctx.serial.write_str("\n");
}

/// Command: version - Show kernel version
fn cmd_version(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial
        .write_str("\nSMROS v0.5.0 - Simple Operating System\n");
    ctx.serial.write_str("Architecture: ARM64 (AArch64)\n");
    ctx.serial
        .write_str("Features: Multi-process, Syscalls, Preemptive Scheduler\n\n");
}

/// Command: testsc - Test syscall interface
fn cmd_test_syscall(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial.write_str("\n=== Memory Syscall Test ===\n\n");
    print_memory_syscall_snapshot(ctx, "before");

    // Test 1: Write syscall
    ctx.serial.write_str("[TEST] Testing write syscall... ");
    let msg = b"Write works!\n";
    let result = test_write(1, msg);
    if result > 0 {
        ctx.serial.write_str("[OK] Write syscall successful\n");
    } else {
        ctx.serial.write_str("[FAIL] Write syscall failed\n");
    }

    // Test 2: Getpid syscall
    ctx.serial.write_str("[TEST] Testing getpid syscall... ");
    let pid = crate::syscall::sys_getpid();
    match pid {
        Ok(p) => {
            ctx.serial.write_str("[OK] getpid returned ");
            print_number(&mut ctx.serial, p as u32);
            ctx.serial.write_str("\n");
        }
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
        }
    }

    // Test 2b: Linux process and time syscalls
    ctx.serial
        .write_str("[TEST] Testing Linux process/time syscalls... ");
    let exec_path = b"/bin/smros-test\0";
    let mut wait_status = 1i32;
    #[repr(C)]
    struct ShellTimespec {
        tv_sec: i64,
        tv_nsec: i64,
    }
    let mut now = ShellTimespec {
        tv_sec: -1,
        tv_nsec: -1,
    };
    let sleep_req = ShellTimespec {
        tv_sec: 0,
        tv_nsec: 1,
    };
    if crate::syscall::sys_getppid().is_err()
        || crate::syscall::sys_gettid().is_err()
        || crate::syscall::sys_execve(exec_path.as_ptr() as usize, 0, 0).is_err()
        || crate::syscall::sys_wait4(0, &mut wait_status as *mut i32 as usize, 0).is_err()
        || wait_status != 0
        || crate::syscall::sys_clock_gettime(1, &mut now as *mut ShellTimespec as usize).is_err()
        || now.tv_sec < 0
        || now.tv_nsec < 0
        || crate::syscall::sys_nanosleep_linux(&sleep_req as *const ShellTimespec as usize).is_err()
    {
        ctx.serial.write_str("[FAIL] process/time path failed\n");
        return;
    }
    ctx.serial.write_str("[OK] process/time calls returned\n");

    // Test 3: Linux brk syscall
    ctx.serial.write_str("[TEST] Testing brk syscall... ");
    let brk_base = match crate::syscall::sys_brk(0) {
        Ok(addr) => addr,
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
            return;
        }
    };
    let brk_target = match user_logic::page_offset_vaddr(brk_base, 2, PAGE_SIZE) {
        Some(addr) => addr,
        None => {
            ctx.serial.write_str("[FAIL] brk target overflow\n");
            return;
        }
    };
    match crate::syscall::sys_brk(brk_target) {
        Ok(addr) if addr == brk_target => {
            ctx.serial.write_str("[OK] brk moved to 0x");
            print_hex(&mut ctx.serial, addr as u64);
            ctx.serial.write_str("\n");
        }
        Ok(addr) => {
            ctx.serial.write_str("[FAIL] brk stopped at 0x");
            print_hex(&mut ctx.serial, addr as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
            return;
        }
    }

    // Test 4: Linux mmap/mprotect/mremap/munmap
    ctx.serial.write_str("[TEST] Testing Linux mmap path... ");
    const MAP_PRIVATE: usize = 1 << 1;
    const MAP_ANONYMOUS: usize = 1 << 5;
    const MREMAP_MAYMOVE: usize = 1 << 0;
    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let prot = 0x3; // PROT_READ | PROT_WRITE

    let mapped_addr = match crate::syscall::sys_mmap(0, PAGE_SIZE * 2, prot, flags, 0, 0) {
        Ok(addr) => {
            ctx.serial.write_str("[OK] mmap returned address 0x");
            print_hex(&mut ctx.serial, addr as u64);
            ctx.serial.write_str("\n");
            addr
        }
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
            return;
        }
    };

    ctx.serial.write_str("[TEST] Testing mremap syscall... ");
    let remapped_addr = match crate::syscall::sys_mremap(
        mapped_addr,
        PAGE_SIZE * 2,
        PAGE_SIZE * 3,
        MREMAP_MAYMOVE,
        0,
    ) {
        Ok(addr) => {
            ctx.serial.write_str("[OK] mremap returned 0x");
            print_hex(&mut ctx.serial, addr as u64);
            ctx.serial.write_str("\n");
            addr
        }
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
            return;
        }
    };

    ctx.serial.write_str("[TEST] Testing mprotect syscall... ");
    match crate::syscall::sys_mprotect(remapped_addr, PAGE_SIZE, 0x1) {
        Ok(_) => ctx
            .serial
            .write_str("[OK] mprotect updated mapping permissions\n"),
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
            return;
        }
    }

    ctx.serial.write_str("[TEST] Testing munmap syscall... ");
    match crate::syscall::sys_munmap(remapped_addr, PAGE_SIZE * 3) {
        Ok(_) => ctx.serial.write_str("[OK] munmap removed the mapping\n"),
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
            return;
        }
    }

    // Restore brk so repeated test runs show stable stats.
    let _ = crate::syscall::sys_brk(brk_base);

    // Test 5: Zircon VMO/VMAR syscalls
    ctx.serial
        .write_str("[TEST] Testing Zircon VMO create/read/write... ");
    let mut vmo_handle = 0u32;
    if let Err(e) = crate::syscall::sys_vmo_create((PAGE_SIZE * 2) as u64, 1, &mut vmo_handle) {
        ctx.serial.write_str("[FAIL] create error ");
        print_number(&mut ctx.serial, (-(e as i32)) as u32);
        ctx.serial.write_str("\n");
        return;
    }

    let payload = b"smros-memory";
    if crate::syscall::sys_vmo_write(vmo_handle, payload, 0).is_err() {
        ctx.serial.write_str("[FAIL] write failed\n");
        return;
    }
    let mut read_back = [0u8; 12];
    if crate::syscall::sys_vmo_read(vmo_handle, &mut read_back, 0).is_err() || read_back != *payload
    {
        ctx.serial.write_str("[FAIL] read verification failed\n");
        return;
    }
    ctx.serial.write_str("[OK] VMO handle 0x");
    print_hex(&mut ctx.serial, vmo_handle as u64);
    ctx.serial.write_str(" preserved data\n");

    ctx.serial
        .write_str("[TEST] Testing VMO size and op_range syscalls... ");
    let mut size = 0usize;
    if crate::syscall::sys_vmo_get_size(vmo_handle, &mut size).is_err() || size != PAGE_SIZE * 2 {
        ctx.serial.write_str("[FAIL] get_size mismatch\n");
        return;
    }
    if crate::syscall::sys_vmo_set_size(vmo_handle, PAGE_SIZE * 3).is_err() {
        ctx.serial.write_str("[FAIL] set_size failed\n");
        return;
    }
    if crate::syscall::sys_vmo_op_range(
        vmo_handle,
        crate::syscall::VmoOpType::Commit as u32,
        0,
        PAGE_SIZE,
    )
    .is_err()
        || crate::syscall::sys_vmo_op_range(
            vmo_handle,
            crate::syscall::VmoOpType::Zero as u32,
            0,
            payload.len(),
        )
        .is_err()
        || crate::syscall::sys_vmo_op_range(
            vmo_handle,
            crate::syscall::VmoOpType::Lock as u32,
            0,
            PAGE_SIZE,
        )
        .is_err()
        || crate::syscall::sys_vmo_op_range(
            vmo_handle,
            crate::syscall::VmoOpType::Unlock as u32,
            0,
            PAGE_SIZE,
        )
        .is_err()
        || crate::syscall::sys_vmo_op_range(
            vmo_handle,
            crate::syscall::VmoOpType::CacheSync as u32,
            0,
            PAGE_SIZE,
        )
        .is_err()
        || crate::syscall::sys_vmo_op_range(
            vmo_handle,
            crate::syscall::VmoOpType::Decommit as u32,
            PAGE_SIZE * 2,
            PAGE_SIZE,
        )
        .is_err()
    {
        ctx.serial.write_str("[FAIL] op_range failed\n");
        return;
    }
    ctx.serial
        .write_str("[OK] size, commit, zero, lock, unlock, cache, and decommit all succeeded\n");

    let root_vmar = crate::syscall::memory_root_vmar_handle();
    ctx.serial
        .write_str("[TEST] Testing VMAR map/protect/allocate/unmap/destroy... ");
    let mut mapped_vaddr = 0usize;
    if crate::syscall::sys_vmar_map(
        root_vmar,
        crate::syscall::VmOptions::PERM_RW.bits(),
        0,
        vmo_handle,
        0,
        PAGE_SIZE,
        &mut mapped_vaddr,
    )
    .is_err()
    {
        ctx.serial.write_str("[FAIL] vmar_map failed\n");
        return;
    }

    if crate::syscall::sys_vmar_protect(
        root_vmar,
        crate::syscall::VmOptions::PERM_READ.bits(),
        mapped_vaddr as u64,
        PAGE_SIZE as u64,
    )
    .is_err()
    {
        ctx.serial.write_str("[FAIL] vmar_protect failed\n");
        return;
    }

    let mut child_vmar = 0u32;
    let mut child_addr = 0usize;
    if crate::syscall::sys_vmar_allocate(
        root_vmar,
        0,
        0,
        (PAGE_SIZE * 2) as u64,
        &mut child_vmar,
        &mut child_addr,
    )
    .is_err()
    {
        ctx.serial.write_str("[FAIL] vmar_allocate failed\n");
        return;
    }

    let mut child_map = 0usize;
    if crate::syscall::sys_vmar_map(
        child_vmar,
        crate::syscall::VmOptions::PERM_READ.bits(),
        0,
        vmo_handle,
        0,
        PAGE_SIZE,
        &mut child_map,
    )
    .is_err()
        || crate::syscall::sys_vmar_unmap(child_vmar, child_map, PAGE_SIZE).is_err()
        || crate::syscall::sys_vmar_unmap_handle_close_thread_exit(
            root_vmar,
            mapped_vaddr,
            PAGE_SIZE,
        )
        .is_err()
        || crate::syscall::sys_vmar_destroy(child_vmar).is_err()
    {
        ctx.serial.write_str("[FAIL] VMAR lifecycle step failed\n");
        return;
    }

    ctx.serial.write_str("[OK] root=0x");
    print_hex(&mut ctx.serial, root_vmar as u64);
    ctx.serial.write_str(", child=0x");
    print_hex(&mut ctx.serial, child_vmar as u64);
    ctx.serial.write_str("\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon handle/object calls... ");
    let mut dup_handle = 0u32;
    let mut info_value = 0u64;
    let mut actual_size = 0usize;
    let mut property_value = 0x534d_524f_5359_5343u64;
    let mut property_readback = 0u64;
    if crate::syscall::sys_handle_duplicate(vmo_handle, 0, &mut dup_handle).is_err()
        || dup_handle != vmo_handle
        || crate::syscall::sys_object_get_info(
            vmo_handle,
            0,
            &mut info_value as *mut u64 as usize,
            core::mem::size_of::<u64>(),
            &mut actual_size,
        )
        .is_err()
        || actual_size != core::mem::size_of::<u64>()
        || crate::syscall::sys_object_set_property(
            vmo_handle,
            0,
            &mut property_value as *mut u64 as usize,
            core::mem::size_of::<u64>(),
        )
        .is_err()
        || crate::syscall::sys_object_get_property(
            vmo_handle,
            0,
            &mut property_readback as *mut u64 as usize,
            core::mem::size_of::<u64>(),
        )
        .is_err()
        || property_readback != property_value
    {
        ctx.serial
            .write_str("[FAIL] handle/object metadata failed\n");
        return;
    }
    ctx.serial
        .write_str("[OK] metadata and property calls returned\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon object signal calls...\n");
    const SIGNAL_USER0: u32 = 1 << 24;
    const SIGNAL_USER1: u32 = 1 << 25;
    let mut signal_pending = 0u32;

    match crate::syscall::sys_object_signal(vmo_handle, 0, SIGNAL_USER0) {
        Ok(_) => print_signal_ok(ctx, "set user signal"),
        Err(e) => {
            print_signal_error(ctx, "set user signal", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(vmo_handle, SIGNAL_USER0, 0, &mut signal_pending) {
        Ok(_) if signal_pending & SIGNAL_USER0 != 0 => {
            print_signal_ok(ctx, "wait user signal");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] wait user signal pending=0x");
            print_hex(&mut ctx.serial, signal_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_signal_error(ctx, "wait user signal", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal(vmo_handle, SIGNAL_USER0, 0) {
        Ok(_) => print_signal_ok(ctx, "clear user signal"),
        Err(e) => {
            print_signal_error(ctx, "clear user signal", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(vmo_handle, SIGNAL_USER0, 0, &mut signal_pending) {
        Err(crate::syscall::ZxError::ErrTimedOut) if signal_pending & SIGNAL_USER0 == 0 => {
            print_signal_ok(ctx, "cleared user signal no longer waits");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] cleared user signal still satisfied pending=0x");
            print_hex(&mut ctx.serial, signal_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_signal_error(ctx, "cleared user signal no longer waits", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal(
        vmo_handle,
        0,
        crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE,
    ) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_signal_ok(ctx, "reject kernel-owned signal bit");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject kernel-owned signal bit unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_signal_error(ctx, "reject kernel-owned signal bit", e);
            return;
        }
    }

    let mut eventpair0 = 0u32;
    let mut eventpair1 = 0u32;
    match crate::syscall::sys_eventpair_create(0, &mut eventpair0, &mut eventpair1) {
        Ok(_) => print_signal_ok(ctx, "eventpair create"),
        Err(e) => {
            print_signal_error(ctx, "eventpair create", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal_peer(eventpair0, 0, SIGNAL_USER1) {
        Ok(_) => print_signal_ok(ctx, "signal peer user bit"),
        Err(e) => {
            print_signal_error(ctx, "signal peer user bit", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(eventpair1, SIGNAL_USER1, 0, &mut signal_pending) {
        Ok(_) if signal_pending & SIGNAL_USER1 != 0 => {
            print_signal_ok(ctx, "wait peer user bit");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] wait peer user bit pending=0x");
            print_hex(&mut ctx.serial, signal_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_signal_error(ctx, "wait peer user bit", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal_peer(
        eventpair0,
        0,
        crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE,
    ) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_signal_ok(ctx, "reject peer kernel-owned signal bit");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject peer kernel-owned signal bit unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_signal_error(ctx, "reject peer kernel-owned signal bit", e);
            return;
        }
    }
    let _ = crate::syscall::sys_handle_close(eventpair0);
    let _ = crate::syscall::sys_handle_close(eventpair1);
    ctx.serial.write_str("[OK] object signal tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon port calls...\n");
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ShellPortPacket {
        key: u64,
        packet_type: u32,
        status: i32,
        data0: u64,
        data1: u64,
        data2: u64,
        data3: u64,
    }
    const PORT_PACKET_USER: u32 = crate::kernel_objects::port::PORT_PACKET_TYPE_USER;
    const PORT_PACKET_SIGNAL_ONE: u32 = crate::kernel_objects::port::PORT_PACKET_TYPE_SIGNAL_ONE;
    let mut port_handle = 0u32;
    let mut queued_packet = ShellPortPacket {
        key: 0x5052_5401,
        packet_type: PORT_PACKET_USER,
        status: 0,
        data0: 10,
        data1: 20,
        data2: 30,
        data3: 40,
    };
    let mut received_packet = ShellPortPacket::default();

    match crate::syscall::sys_port_create(0, &mut port_handle) {
        Ok(_) => print_port_ok(ctx, "create"),
        Err(e) => {
            print_port_error(ctx, "create", e);
            return;
        }
    }
    match crate::syscall::sys_port_wait(
        port_handle,
        0,
        &mut received_packet as *mut ShellPortPacket as usize,
    ) {
        Err(crate::syscall::ZxError::ErrTimedOut) => print_port_ok(ctx, "empty wait times out"),
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] empty wait unexpectedly returned packet\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "empty wait times out", e);
            return;
        }
    }
    match crate::syscall::sys_port_queue(
        port_handle,
        &mut queued_packet as *mut ShellPortPacket as usize,
    ) {
        Ok(_) => print_port_ok(ctx, "queue user packet"),
        Err(e) => {
            print_port_error(ctx, "queue user packet", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(
        port_handle,
        crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE,
        0,
        &mut signal_pending,
    ) {
        Ok(_) if signal_pending & crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE != 0 => {
            print_port_ok(ctx, "readable after queue");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] readable after queue pending=0x");
            print_hex(&mut ctx.serial, signal_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "readable after queue", e);
            return;
        }
    }
    match crate::syscall::sys_port_wait(
        port_handle,
        0,
        &mut received_packet as *mut ShellPortPacket as usize,
    ) {
        Ok(_)
            if received_packet.key == queued_packet.key
                && received_packet.packet_type == PORT_PACKET_USER
                && received_packet.data0 == 10
                && received_packet.data3 == 40 =>
        {
            print_port_ok(ctx, "wait user packet");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] wait user packet key=0x");
            print_hex(&mut ctx.serial, received_packet.key);
            ctx.serial.write_str(", type=");
            print_number(&mut ctx.serial, received_packet.packet_type);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "wait user packet", e);
            return;
        }
    }

    queued_packet.key = 0x5052_5402;
    match crate::syscall::sys_port_queue(
        port_handle,
        &mut queued_packet as *mut ShellPortPacket as usize,
    ) {
        Ok(_) => print_port_ok(ctx, "queue cancel packet"),
        Err(e) => {
            print_port_error(ctx, "queue cancel packet", e);
            return;
        }
    }
    match crate::syscall::sys_port_cancel(port_handle, vmo_handle, queued_packet.key) {
        Ok(removed) if removed == 0 => print_port_ok(ctx, "cancel ignores user packet"),
        Ok(removed) => {
            print_port_count_mismatch(ctx, "cancel ignores user packet", 0, removed);
            return;
        }
        Err(e) => {
            print_port_error(ctx, "cancel ignores user packet", e);
            return;
        }
    }
    match crate::syscall::sys_port_wait(
        port_handle,
        0,
        &mut received_packet as *mut ShellPortPacket as usize,
    ) {
        Ok(_)
            if received_packet.key == queued_packet.key
                && received_packet.packet_type == PORT_PACKET_USER =>
        {
            print_port_ok(ctx, "wait uncanceled user packet");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] wait uncanceled user packet key=0x");
            print_hex(&mut ctx.serial, received_packet.key);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "wait uncanceled user packet", e);
            return;
        }
    }

    match crate::syscall::sys_object_wait_async(
        vmo_handle,
        port_handle,
        0x5052_5404,
        SIGNAL_USER1,
        0,
    ) {
        Ok(_) => print_port_ok(ctx, "register async wait for cancel"),
        Err(e) => {
            print_port_error(ctx, "register async wait for cancel", e);
            return;
        }
    }
    match crate::syscall::sys_port_cancel(port_handle, vmo_handle, 0x5052_5404) {
        Ok(removed) if removed == 1 => print_port_ok(ctx, "cancel async wait"),
        Ok(removed) => {
            print_port_count_mismatch(ctx, "cancel async wait", 1, removed);
            return;
        }
        Err(e) => {
            print_port_error(ctx, "cancel async wait", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal(vmo_handle, 0, SIGNAL_USER1) {
        Ok(_) => print_port_ok(ctx, "signal canceled source"),
        Err(e) => {
            print_port_error(ctx, "signal canceled source", e);
            return;
        }
    }
    match crate::syscall::sys_port_wait(
        port_handle,
        0,
        &mut received_packet as *mut ShellPortPacket as usize,
    ) {
        Err(crate::syscall::ZxError::ErrTimedOut) => {
            print_port_ok(ctx, "canceled packet removed");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] canceled packet unexpectedly returned key=0x");
            print_hex(&mut ctx.serial, received_packet.key);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "canceled packet removed", e);
            return;
        }
    }

    match crate::syscall::sys_object_wait_async(
        vmo_handle,
        port_handle,
        0x5052_5403,
        SIGNAL_USER0,
        0,
    ) {
        Ok(_) => print_port_ok(ctx, "wait async queues signal packet"),
        Err(e) => {
            print_port_error(ctx, "wait async queues signal packet", e);
            return;
        }
    }
    match crate::syscall::sys_port_wait(
        port_handle,
        0,
        &mut received_packet as *mut ShellPortPacket as usize,
    ) {
        Err(crate::syscall::ZxError::ErrTimedOut) => {
            print_port_ok(ctx, "unsignaled wait stays queued")
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] unsignaled wait unexpectedly returned key=0x");
            print_hex(&mut ctx.serial, received_packet.key);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "unsignaled wait stays queued", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal(vmo_handle, 0, SIGNAL_USER0) {
        Ok(_) => print_port_ok(ctx, "signal source for async wait"),
        Err(e) => {
            print_port_error(ctx, "signal source for async wait", e);
            return;
        }
    }
    match crate::syscall::sys_port_wait(
        port_handle,
        0,
        &mut received_packet as *mut ShellPortPacket as usize,
    ) {
        Ok(_)
            if received_packet.key == 0x5052_5403
                && received_packet.packet_type == PORT_PACKET_SIGNAL_ONE
                && received_packet.data0 == SIGNAL_USER0 as u64
                && (received_packet.data1 & SIGNAL_USER0 as u64) != 0 =>
        {
            print_port_ok(ctx, "wait async signal packet");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] async packet key=0x");
            print_hex(&mut ctx.serial, received_packet.key);
            ctx.serial.write_str(", type=");
            print_number(&mut ctx.serial, received_packet.packet_type);
            ctx.serial.write_str(", data1=0x");
            print_hex(&mut ctx.serial, received_packet.data1);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "wait async signal packet", e);
            return;
        }
    }
    match crate::syscall::dispatch_zircon_syscall(
        crate::syscall::ZirconSyscall::PortCancel as u32,
        [
            port_handle as usize,
            vmo_handle as usize,
            0x5052_5403,
            0,
            0,
            0,
            0,
            0,
        ],
    ) {
        Ok(removed) if removed == 1 => print_port_ok(ctx, "dispatch port cancel async"),
        Ok(removed) => {
            ctx.serial
                .write_str("  [FAIL] dispatch port cancel async expected=1, actual=");
            print_number(&mut ctx.serial, removed as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_port_error(ctx, "dispatch port cancel async", e);
            return;
        }
    }
    let _ = crate::syscall::sys_handle_close(port_handle);
    ctx.serial.write_str("[OK] port tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon channel IPC calls... ");
    let mut channel0 = 0u32;
    let mut channel1 = 0u32;
    let channel_payload = b"ipc-ok";
    let mut channel_readback = [0u8; 6];
    let mut actual_bytes = 0usize;
    let mut actual_handles = 0usize;
    if crate::syscall::sys_channel_create(0, &mut channel0, &mut channel1).is_err()
        || crate::syscall::sys_channel_write(
            channel0,
            0,
            channel_payload.as_ptr() as usize,
            channel_payload.len(),
            0,
            0,
        )
        .is_err()
        || crate::syscall::sys_channel_read(
            channel1,
            0,
            channel_readback.as_mut_ptr() as usize,
            channel_readback.len(),
            0,
            0,
            &mut actual_bytes,
            &mut actual_handles,
        )
        .is_err()
        || actual_bytes != channel_payload.len()
        || actual_handles != 0
        || channel_readback != *channel_payload
        || crate::syscall::sys_handle_close(channel0).is_err()
    {
        ctx.serial.write_str("[FAIL] channel IPC failed\n");
        return;
    }
    ctx.serial.write_str("[OK] channel message round-tripped\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon socket IPC calls...\n");
    #[repr(C)]
    #[derive(Default)]
    struct ShellSocketInfo {
        options: u32,
        padding1: u32,
        rx_buf_max: u64,
        rx_buf_size: u64,
        rx_buf_available: u64,
        tx_buf_max: u64,
        tx_buf_size: u64,
    }
    const SOCKET_DATAGRAM: u32 = crate::kernel_objects::socket::SOCKET_DATAGRAM;
    const SOCKET_PEEK: u32 = crate::kernel_objects::socket::SOCKET_PEEK;
    const SOCKET_SHUTDOWN_READ: u32 = crate::kernel_objects::socket::SOCKET_SHUTDOWN_READ;
    const SOCKET_RX_THRESHOLD_PROPERTY: u32 =
        crate::kernel_objects::socket::SOCKET_PROPERTY_RX_THRESHOLD;
    const SOCKET_INFO_TOPIC: u32 = crate::kernel_objects::socket::OBJECT_INFO_TOPIC_SOCKET;

    let mut socket0 = 0u32;
    let mut socket1 = 0u32;
    let socket_payload = b"socket-ok";
    let mut socket_written = 0usize;
    let mut socket_peek = [0u8; 9];
    let mut socket_readback = [0u8; 9];
    let mut socket_info = ShellSocketInfo::default();
    let mut socket_info_actual = 0usize;
    let mut threshold = 1u64;
    let mut threshold_readback = 0u64;

    match crate::syscall::sys_socket_create(0, &mut socket0, &mut socket1) {
        Ok(_) => {
            ctx.serial.write_str("  [OK] stream create handles=0x");
            print_hex(&mut ctx.serial, socket0 as u64);
            ctx.serial.write_str(",0x");
            print_hex(&mut ctx.serial, socket1 as u64);
            ctx.serial.write_str("\n");
        }
        Err(e) => {
            print_socket_error(ctx, "stream create", e);
            return;
        }
    }

    match crate::syscall::sys_socket_write(
        socket0,
        0,
        socket_payload.as_ptr() as usize,
        socket_payload.len(),
        &mut socket_written,
    ) {
        Ok(_) if socket_written == socket_payload.len() => {
            print_socket_ok(ctx, "stream write");
        }
        Ok(_) => {
            print_socket_count_mismatch(
                ctx,
                "stream write count",
                socket_payload.len(),
                socket_written,
            );
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "stream write", e);
            return;
        }
    }

    match crate::syscall::sys_socket_read(
        socket1,
        SOCKET_PEEK,
        socket_peek.as_mut_ptr() as usize,
        socket_peek.len(),
        &mut socket_written,
    ) {
        Ok(_) if socket_written == socket_payload.len() && socket_peek == *socket_payload => {
            print_socket_ok(ctx, "stream peek");
        }
        Ok(_) => {
            print_socket_count_mismatch(
                ctx,
                "stream peek count",
                socket_payload.len(),
                socket_written,
            );
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "stream peek", e);
            return;
        }
    }

    match crate::syscall::sys_object_get_info(
        socket1,
        SOCKET_INFO_TOPIC,
        &mut socket_info as *mut ShellSocketInfo as usize,
        core::mem::size_of::<ShellSocketInfo>(),
        &mut socket_info_actual,
    ) {
        Ok(_)
            if socket_info_actual == core::mem::size_of::<ShellSocketInfo>()
                && socket_info.rx_buf_available == socket_payload.len() as u64
                && socket_info.rx_buf_size == socket_payload.len() as u64 =>
        {
            print_socket_ok(ctx, "socket info");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] socket info actual=");
            print_number(&mut ctx.serial, socket_info_actual as u32);
            ctx.serial.write_str(", rx_size=");
            print_number(&mut ctx.serial, socket_info.rx_buf_size as u32);
            ctx.serial.write_str(", rx_available=");
            print_number(&mut ctx.serial, socket_info.rx_buf_available as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "socket info", e);
            return;
        }
    }

    match crate::syscall::sys_object_set_property(
        socket1,
        SOCKET_RX_THRESHOLD_PROPERTY,
        &mut threshold as *mut u64 as usize,
        core::mem::size_of::<u64>(),
    ) {
        Ok(_) => print_socket_ok(ctx, "set rx threshold"),
        Err(e) => {
            print_socket_error(ctx, "set rx threshold", e);
            return;
        }
    }

    match crate::syscall::sys_object_get_property(
        socket1,
        SOCKET_RX_THRESHOLD_PROPERTY,
        &mut threshold_readback as *mut u64 as usize,
        core::mem::size_of::<u64>(),
    ) {
        Ok(_) if threshold_readback == threshold => {
            print_socket_ok(ctx, "get rx threshold");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] get rx threshold expected=");
            print_number(&mut ctx.serial, threshold as u32);
            ctx.serial.write_str(", actual=");
            print_number(&mut ctx.serial, threshold_readback as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "get rx threshold", e);
            return;
        }
    }

    match crate::syscall::sys_socket_read(
        socket1,
        0,
        socket_readback.as_mut_ptr() as usize,
        socket_readback.len(),
        &mut socket_written,
    ) {
        Ok(_) if socket_written == socket_payload.len() && socket_readback == *socket_payload => {
            print_socket_ok(ctx, "stream read");
        }
        Ok(_) => {
            print_socket_count_mismatch(
                ctx,
                "stream read count",
                socket_payload.len(),
                socket_written,
            );
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "stream read", e);
            return;
        }
    }

    let mut datagram0 = 0u32;
    let mut datagram1 = 0u32;
    let datagram_payload = b"datagram";
    let mut datagram_readback = [0u8; 4];
    let mut datagram_actual = 0usize;
    match crate::syscall::sys_socket_create(SOCKET_DATAGRAM, &mut datagram0, &mut datagram1) {
        Ok(_) => print_socket_ok(ctx, "datagram create"),
        Err(e) => {
            print_socket_error(ctx, "datagram create", e);
            return;
        }
    }
    match crate::syscall::sys_socket_write(
        datagram0,
        0,
        datagram_payload.as_ptr() as usize,
        datagram_payload.len(),
        &mut datagram_actual,
    ) {
        Ok(_) if datagram_actual == datagram_payload.len() => {
            print_socket_ok(ctx, "datagram write");
        }
        Ok(_) => {
            print_socket_count_mismatch(
                ctx,
                "datagram write count",
                datagram_payload.len(),
                datagram_actual,
            );
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "datagram write", e);
            return;
        }
    }
    match crate::syscall::sys_socket_read(
        datagram1,
        0,
        datagram_readback.as_mut_ptr() as usize,
        datagram_readback.len(),
        &mut datagram_actual,
    ) {
        Ok(_) if datagram_actual == datagram_readback.len() && datagram_readback == *b"data" => {
            print_socket_ok(ctx, "datagram truncate read");
        }
        Ok(_) => {
            print_socket_count_mismatch(
                ctx,
                "datagram truncate count",
                datagram_readback.len(),
                datagram_actual,
            );
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "datagram truncate read", e);
            return;
        }
    }
    match crate::syscall::sys_socket_read(
        datagram1,
        0,
        datagram_readback.as_mut_ptr() as usize,
        datagram_readback.len(),
        &mut datagram_actual,
    ) {
        Err(crate::syscall::ZxError::ErrShouldWait) => {
            print_socket_ok(ctx, "datagram tail discard");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] datagram tail discard unexpectedly read count=");
            print_number(&mut ctx.serial, datagram_actual as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "datagram tail discard", e);
            return;
        }
    }

    let mut shared0 = 0u32;
    let mut shared1 = 0u32;
    let mut accepted = 0u32;
    let shutdown_payload = b"x";
    match crate::syscall::sys_socket_create(0, &mut shared0, &mut shared1) {
        Ok(_) => print_socket_ok(ctx, "shared socket create"),
        Err(e) => {
            print_socket_error(ctx, "shared socket create", e);
            return;
        }
    }
    match crate::syscall::sys_socket_share(socket0, shared0) {
        Ok(_) => print_socket_ok(ctx, "socket share"),
        Err(e) => {
            print_socket_error(ctx, "socket share", e);
            return;
        }
    }
    match crate::syscall::sys_socket_accept(socket1, &mut accepted) {
        Ok(_) if accepted == shared0 => print_socket_ok(ctx, "socket accept"),
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] socket accept expected=0x");
            print_hex(&mut ctx.serial, shared0 as u64);
            ctx.serial.write_str(", actual=0x");
            print_hex(&mut ctx.serial, accepted as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "socket accept", e);
            return;
        }
    }
    match crate::syscall::sys_socket_shutdown(socket1, SOCKET_SHUTDOWN_READ) {
        Ok(_) => print_socket_ok(ctx, "socket shutdown read"),
        Err(e) => {
            print_socket_error(ctx, "socket shutdown read", e);
            return;
        }
    }
    match crate::syscall::sys_socket_write(
        socket0,
        0,
        shutdown_payload.as_ptr() as usize,
        shutdown_payload.len(),
        &mut socket_written,
    ) {
        Err(crate::syscall::ZxError::ErrBadState) => {
            print_socket_ok(ctx, "shutdown blocks peer write");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] shutdown blocks peer write unexpectedly wrote=");
            print_number(&mut ctx.serial, socket_written as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_socket_error(ctx, "shutdown blocks peer write", e);
            return;
        }
    }

    let _ = crate::syscall::sys_handle_close(socket0);
    let _ = crate::syscall::sys_handle_close(socket1);
    let _ = crate::syscall::sys_handle_close(datagram0);
    let _ = crate::syscall::sys_handle_close(datagram1);
    let _ = crate::syscall::sys_handle_close(shared0);
    let _ = crate::syscall::sys_handle_close(shared1);
    ctx.serial
        .write_str("[OK] socket kernel object tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon FIFO IPC calls...\n");
    const FIFO_ELEM_COUNT: usize = 4;
    const FIFO_ELEM_SIZE: usize = core::mem::size_of::<u32>();
    const FIFO_TEST_SIGNAL: u32 = 1 << 25;
    let mut fifo0 = 0u32;
    let mut fifo1 = 0u32;
    let mut fifo_actual = 0usize;
    let mut fifo_pending = 0u32;

    match crate::syscall::sys_fifo_create(0, FIFO_ELEM_SIZE, 0, &mut fifo0, &mut fifo1) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_fifo_ok(ctx, "reject zero elem count");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject zero elem count unexpectedly succeeded\n");
            let _ = crate::syscall::sys_handle_close(fifo0);
            let _ = crate::syscall::sys_handle_close(fifo1);
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "reject zero elem count", e);
            return;
        }
    }

    match crate::syscall::sys_fifo_create(
        crate::kernel_objects::fifo::FIFO_MAX_ELEMS + 1,
        FIFO_ELEM_SIZE,
        0,
        &mut fifo0,
        &mut fifo1,
    ) {
        Err(crate::syscall::ZxError::ErrOutOfRange) => {
            print_fifo_ok(ctx, "reject oversized elem count");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject oversized elem count unexpectedly succeeded\n");
            let _ = crate::syscall::sys_handle_close(fifo0);
            let _ = crate::syscall::sys_handle_close(fifo1);
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "reject oversized elem count", e);
            return;
        }
    }

    match crate::syscall::sys_fifo_create(
        FIFO_ELEM_COUNT,
        FIFO_ELEM_SIZE,
        0,
        &mut fifo0,
        &mut fifo1,
    ) {
        Ok(_) => {
            ctx.serial.write_str("  [OK] create handles=0x");
            print_hex(&mut ctx.serial, fifo0 as u64);
            ctx.serial.write_str(",0x");
            print_hex(&mut ctx.serial, fifo1 as u64);
            ctx.serial.write_str("\n");
        }
        Err(e) => {
            print_fifo_error(ctx, "create", e);
            return;
        }
    }

    match crate::syscall::sys_object_wait_one(
        fifo0,
        crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE,
        0,
        &mut fifo_pending,
    ) {
        Ok(_) if fifo_pending & crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE != 0 => {
            print_fifo_ok(ctx, "initial writable signal");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] initial writable signal pending=0x");
            print_hex(&mut ctx.serial, fifo_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "initial writable signal", e);
            return;
        }
    }

    let fifo_first_write = [11u32, 22, 33];
    match crate::syscall::sys_fifo_write(
        fifo0,
        FIFO_ELEM_SIZE,
        fifo_first_write.as_ptr() as usize,
        fifo_first_write.len(),
        &mut fifo_actual,
    ) {
        Ok(_) if fifo_actual == fifo_first_write.len() => {
            print_fifo_ok(ctx, "write three elements");
        }
        Ok(_) => {
            print_fifo_count_mismatch(
                ctx,
                "write three elements count",
                fifo_first_write.len(),
                fifo_actual,
            );
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "write three elements", e);
            return;
        }
    }

    match crate::syscall::sys_object_wait_one(
        fifo1,
        crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE,
        0,
        &mut fifo_pending,
    ) {
        Ok(_) if fifo_pending & crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE != 0 => {
            print_fifo_ok(ctx, "peer readable signal");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] peer readable pending=0x");
            print_hex(&mut ctx.serial, fifo_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "peer readable signal", e);
            return;
        }
    }

    let mut fifo_read_two = [0u32; 2];
    match crate::syscall::sys_fifo_read(
        fifo1,
        FIFO_ELEM_SIZE,
        fifo_read_two.as_mut_ptr() as usize,
        fifo_read_two.len(),
        &mut fifo_actual,
    ) {
        Ok(_) if fifo_actual == fifo_read_two.len() && fifo_read_two == [11u32, 22] => {
            print_fifo_ok(ctx, "read preserves order");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] read preserves order count=");
            print_number(&mut ctx.serial, fifo_actual as u32);
            ctx.serial.write_str(", values=");
            print_number(&mut ctx.serial, fifo_read_two[0]);
            ctx.serial.write_str(",");
            print_number(&mut ctx.serial, fifo_read_two[1]);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "read preserves order", e);
            return;
        }
    }

    let fifo_second_write = [44u32, 55, 66, 77];
    match crate::syscall::sys_fifo_write(
        fifo0,
        FIFO_ELEM_SIZE,
        fifo_second_write.as_ptr() as usize,
        fifo_second_write.len(),
        &mut fifo_actual,
    ) {
        Ok(_) if fifo_actual == 3 => {
            print_fifo_ok(ctx, "partial write when peer nearly full");
        }
        Ok(_) => {
            print_fifo_count_mismatch(ctx, "partial write count", 3, fifo_actual);
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "partial write when peer nearly full", e);
            return;
        }
    }

    let mut fifo_read_all = [0u32; 4];
    match crate::syscall::sys_fifo_read(
        fifo1,
        FIFO_ELEM_SIZE,
        fifo_read_all.as_mut_ptr() as usize,
        fifo_read_all.len(),
        &mut fifo_actual,
    ) {
        Ok(_) if fifo_actual == fifo_read_all.len() && fifo_read_all == [33u32, 44, 55, 66] => {
            print_fifo_ok(ctx, "read wrapped elements");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] read wrapped count=");
            print_number(&mut ctx.serial, fifo_actual as u32);
            ctx.serial.write_str(", values=");
            for value in fifo_read_all {
                print_number(&mut ctx.serial, value);
                ctx.serial.write_str(" ");
            }
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "read wrapped elements", e);
            return;
        }
    }

    match crate::syscall::sys_fifo_read(
        fifo1,
        FIFO_ELEM_SIZE,
        fifo_read_all.as_mut_ptr() as usize,
        0,
        &mut fifo_actual,
    ) {
        Ok(_) if fifo_actual == 0 => print_fifo_ok(ctx, "zero-count read"),
        Ok(_) => {
            print_fifo_count_mismatch(ctx, "zero-count read", 0, fifo_actual);
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "zero-count read", e);
            return;
        }
    }

    match crate::syscall::sys_fifo_write(fifo0, FIFO_ELEM_SIZE, 0, 0, &mut fifo_actual) {
        Ok(_) if fifo_actual == 0 => print_fifo_ok(ctx, "zero-count write"),
        Ok(_) => {
            print_fifo_count_mismatch(ctx, "zero-count write", 0, fifo_actual);
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "zero-count write", e);
            return;
        }
    }

    match crate::syscall::sys_fifo_read(
        fifo1,
        FIFO_ELEM_SIZE,
        fifo_read_all.as_mut_ptr() as usize,
        fifo_read_all.len(),
        &mut fifo_actual,
    ) {
        Err(crate::syscall::ZxError::ErrShouldWait) => {
            print_fifo_ok(ctx, "empty read should wait");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] empty read unexpectedly read=");
            print_number(&mut ctx.serial, fifo_actual as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "empty read should wait", e);
            return;
        }
    }

    match crate::syscall::sys_fifo_write(
        fifo0,
        FIFO_ELEM_SIZE * 2,
        fifo_second_write.as_ptr() as usize,
        1,
        &mut fifo_actual,
    ) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_fifo_ok(ctx, "reject mismatched elem size");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject mismatched elem size unexpectedly wrote=");
            print_number(&mut ctx.serial, fifo_actual as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "reject mismatched elem size", e);
            return;
        }
    }

    match crate::syscall::sys_object_signal_peer(fifo0, 0, FIFO_TEST_SIGNAL) {
        Ok(_) => print_fifo_ok(ctx, "signal peer"),
        Err(e) => {
            print_fifo_error(ctx, "signal peer", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(fifo1, FIFO_TEST_SIGNAL, 0, &mut fifo_pending) {
        Ok(_) if fifo_pending & FIFO_TEST_SIGNAL != 0 => {
            print_fifo_ok(ctx, "wait peer signal");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] wait peer signal pending=0x");
            print_hex(&mut ctx.serial, fifo_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "wait peer signal", e);
            return;
        }
    }

    match crate::syscall::sys_handle_close(fifo0) {
        Ok(_) => print_fifo_ok(ctx, "close writer endpoint"),
        Err(e) => {
            print_fifo_error(ctx, "close writer endpoint", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(
        fifo1,
        crate::kernel_objects::channel::CHANNEL_SIGNAL_PEER_CLOSED,
        0,
        &mut fifo_pending,
    ) {
        Ok(_) if fifo_pending & crate::kernel_objects::channel::CHANNEL_SIGNAL_PEER_CLOSED != 0 => {
            print_fifo_ok(ctx, "peer closed signal");
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] peer closed pending=0x");
            print_hex(&mut ctx.serial, fifo_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "peer closed signal", e);
            return;
        }
    }
    match crate::syscall::sys_fifo_read(
        fifo1,
        FIFO_ELEM_SIZE,
        fifo_read_all.as_mut_ptr() as usize,
        fifo_read_all.len(),
        &mut fifo_actual,
    ) {
        Err(crate::syscall::ZxError::ErrPeerClosed) => {
            print_fifo_ok(ctx, "empty read after peer close");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] empty read after peer close unexpectedly read=");
            print_number(&mut ctx.serial, fifo_actual as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_fifo_error(ctx, "empty read after peer close", e);
            return;
        }
    }
    let _ = crate::syscall::sys_handle_close(fifo1);
    ctx.serial
        .write_str("[OK] FIFO kernel object tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon futex calls...\n");
    let mut futex_a = 7i32;
    let mut futex_b = 11i32;
    let futex_a_ptr = &mut futex_a as *mut i32 as usize;
    let futex_b_ptr = &mut futex_b as *mut i32 as usize;

    match crate::syscall::sys_futex_wait(0, 0, 0, 0) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_futex_ok(ctx, "reject null wait pointer");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject null wait pointer unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "reject null wait pointer", e);
            return;
        }
    }
    match crate::syscall::sys_futex_wake(futex_a_ptr + 1, 1) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_futex_ok(ctx, "reject unaligned wake pointer");
        }
        Ok(count) => {
            ctx.serial
                .write_str("  [FAIL] reject unaligned wake pointer unexpectedly woke=");
            print_number(&mut ctx.serial, count);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "reject unaligned wake pointer", e);
            return;
        }
    }
    match crate::syscall::sys_futex_wait(futex_a_ptr, 99, 0, 0) {
        Err(crate::syscall::ZxError::ErrBadState) => {
            print_futex_ok(ctx, "reject mismatched value");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject mismatched value unexpectedly waited\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "reject mismatched value", e);
            return;
        }
    }
    match crate::syscall::sys_futex_wait(futex_a_ptr, futex_a, 0x44, 1) {
        Ok(_) => print_futex_ok(ctx, "record waiter with owner"),
        Err(e) => {
            print_futex_error(ctx, "record waiter with owner", e);
            return;
        }
    }
    match crate::syscall::sys_futex_get_owner(futex_a_ptr) {
        Ok(owner) if owner == 0x44 => print_futex_ok(ctx, "get owner"),
        Ok(owner) => {
            ctx.serial
                .write_str("  [FAIL] get owner expected=68, actual=");
            print_number(&mut ctx.serial, owner);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "get owner", e);
            return;
        }
    }
    match crate::syscall::sys_futex_wake(futex_a_ptr, 1) {
        Ok(count) if count == 1 => print_futex_ok(ctx, "wake waiter"),
        Ok(count) => {
            print_futex_count_mismatch(ctx, "wake waiter count", 1, count);
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "wake waiter", e);
            return;
        }
    }
    match crate::syscall::sys_futex_get_owner(futex_a_ptr) {
        Ok(owner) if owner == 0 => print_futex_ok(ctx, "owner cleared after wake"),
        Ok(owner) => {
            ctx.serial
                .write_str("  [FAIL] owner cleared after wake actual=");
            print_number(&mut ctx.serial, owner);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "owner cleared after wake", e);
            return;
        }
    }

    if crate::syscall::sys_futex_wait(futex_a_ptr, futex_a, 0x51, 1).is_err()
        || crate::syscall::sys_futex_wait(futex_a_ptr, futex_a, 0x52, 1).is_err()
        || crate::syscall::sys_futex_wait(futex_a_ptr, futex_a, 0x53, 1).is_err()
    {
        ctx.serial
            .write_str("  [FAIL] setup requeue waiters failed\n");
        return;
    }
    match crate::syscall::sys_futex_requeue(futex_a_ptr, 1, futex_a, futex_b_ptr, 2, 0x99) {
        Ok((woken, requeued)) if woken == 1 && requeued == 2 => {
            print_futex_ok(ctx, "wake and requeue waiters");
        }
        Ok((woken, requeued)) => {
            ctx.serial
                .write_str("  [FAIL] wake and requeue expected=1/2, actual=");
            print_number(&mut ctx.serial, woken);
            ctx.serial.write_str("/");
            print_number(&mut ctx.serial, requeued);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "wake and requeue waiters", e);
            return;
        }
    }
    match crate::syscall::sys_futex_get_owner(futex_b_ptr) {
        Ok(owner) if owner == 0x99 => print_futex_ok(ctx, "requeue owner"),
        Ok(owner) => {
            ctx.serial
                .write_str("  [FAIL] requeue owner expected=153, actual=");
            print_number(&mut ctx.serial, owner);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "requeue owner", e);
            return;
        }
    }
    match crate::syscall::sys_futex_wake_single_owner(futex_b_ptr) {
        Ok(count) if count == 1 => print_futex_ok(ctx, "wake single owner"),
        Ok(count) => {
            print_futex_count_mismatch(ctx, "wake single owner count", 1, count);
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "wake single owner", e);
            return;
        }
    }
    match crate::syscall::dispatch_zircon_syscall(
        crate::syscall::ZirconSyscall::FutexWake as u32,
        [futex_b_ptr, 8, 0, 0, 0, 0, 0, 0],
    ) {
        Ok(count) if count == 1 => print_futex_ok(ctx, "dispatch futex wake"),
        Ok(count) => {
            ctx.serial
                .write_str("  [FAIL] dispatch futex wake expected=1, actual=");
            print_number(&mut ctx.serial, count as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_futex_error(ctx, "dispatch futex wake", e);
            return;
        }
    }
    ctx.serial.write_str("[OK] futex tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon process/thread/wait calls... ");
    #[repr(C)]
    struct ShellWaitItem {
        handle: u32,
        waitfor: u32,
        pending: u32,
    }
    const TEST_SIGNAL: u32 = 1 << 24;
    let mut proc_handle = 0u32;
    let mut proc_vmar = 0u32;
    let mut thread_handle = 0u32;
    let mut pending = 0u32;
    let mut wait_item = ShellWaitItem {
        handle: 0,
        waitfor: TEST_SIGNAL,
        pending: 0,
    };
    if crate::syscall::sys_process_create(0, 0, 0, 0, &mut proc_handle, &mut proc_vmar).is_err()
        || crate::syscall::sys_thread_create(
            proc_handle,
            0,
            0,
            0x1000,
            PAGE_SIZE,
            &mut thread_handle,
        )
        .is_err()
        || crate::syscall::sys_thread_start(thread_handle, 0x1000, 0x8000, 1, 2).is_err()
        || crate::syscall::sys_object_signal(thread_handle, 0, TEST_SIGNAL).is_err()
        || crate::syscall::sys_object_wait_one(thread_handle, TEST_SIGNAL, 0, &mut pending).is_err()
        || pending & TEST_SIGNAL == 0
    {
        ctx.serial
            .write_str("[FAIL] process/thread/wait setup failed\n");
        return;
    }
    wait_item.handle = thread_handle;
    if crate::syscall::sys_object_wait_many(&mut wait_item as *mut ShellWaitItem as usize, 1, 0)
        .is_err()
        || wait_item.pending & TEST_SIGNAL == 0
        || crate::syscall::sys_nanosleep(0).is_err()
        || crate::syscall::sys_clock_get_monotonic().is_err()
        || crate::syscall::sys_task_kill(thread_handle).is_err()
        || crate::syscall::sys_process_exit(proc_handle, 0).is_err()
    {
        ctx.serial
            .write_str("[FAIL] process/thread/wait lifecycle failed\n");
        return;
    }
    let close_many = [thread_handle, proc_handle];
    if crate::syscall::sys_handle_close_many(close_many.as_ptr() as usize, close_many.len())
        .is_err()
    {
        ctx.serial.write_str("[FAIL] close_many failed\n");
        return;
    }
    ctx.serial
        .write_str("[OK] process/thread lifecycle completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon time/debug/system/exception calls...\n");
    let mut clock_handle = 0u32;
    let mut clock_value = 0u64;
    let mut timer_handle = 0u32;
    let mut timer_pending = 0u32;
    let mut debuglog_handle = 0u32;
    let debug_payload = b"debuglog-ok";
    let mut debug_readback = [0u8; 11];
    let mut debug_zero = [0xffu8; 4];
    let mut system_event = 0u32;
    let mut exception_proc = 0u32;
    let mut exception_vmar = 0u32;
    let mut exception_channel = 0u32;
    let mut exception_packet = [0u8; 8];
    let mut exception_handles = [0u32; 1];
    let mut exception_bytes_actual = 0usize;
    let mut exception_handles_actual = 0usize;
    let mut exception_thread = 0u32;
    let mut exception_process = 0u32;

    match crate::syscall::sys_clock_get(1, &mut clock_value as *mut u64 as usize) {
        Ok(_) => print_time_debug_ok(ctx, "clock get monotonic id"),
        Err(e) => {
            print_time_debug_error(ctx, "clock get monotonic id", e);
            return;
        }
    }
    match crate::syscall::sys_clock_get(2, &mut clock_value as *mut u64 as usize) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_time_debug_ok(ctx, "reject invalid clock id");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid clock id unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_time_debug_error(ctx, "reject invalid clock id", e);
            return;
        }
    }
    match crate::syscall::sys_clock_create(1, 0, &mut clock_handle) {
        Ok(_) => print_time_debug_ok(ctx, "clock create auto-start"),
        Err(e) => {
            print_time_debug_error(ctx, "clock create auto-start", e);
            return;
        }
    }
    if crate::syscall::sys_clock_read(clock_handle, &mut clock_value as *mut u64 as usize).is_err()
        || crate::syscall::sys_clock_update(clock_handle, 1, 0).is_err()
        || crate::syscall::dispatch_zircon_syscall(
            crate::syscall::ZirconSyscall::ClockRead as u32,
            [
                clock_handle as usize,
                &mut clock_value as *mut u64 as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        )
        .is_err()
    {
        ctx.serial.write_str("  [FAIL] clock read/update failed\n");
        return;
    }
    print_time_debug_ok(ctx, "clock read/update/dispatch");
    match crate::syscall::sys_clock_update(clock_handle, 4, 0) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_time_debug_ok(ctx, "reject invalid clock update option");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid clock update option unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_time_debug_error(ctx, "reject invalid clock update option", e);
            return;
        }
    }

    match crate::syscall::sys_timer_create(0, 1, &mut timer_handle) {
        Ok(_) => print_time_debug_ok(ctx, "timer create"),
        Err(e) => {
            print_time_debug_error(ctx, "timer create", e);
            return;
        }
    }
    if crate::syscall::sys_timer_set(timer_handle, 0, 0).is_err()
        || crate::syscall::sys_object_wait_one(timer_handle, 1 << 7, 0, &mut timer_pending).is_err()
        || timer_pending & (1 << 7) == 0
    {
        ctx.serial.write_str("  [FAIL] timer set/signaled failed\n");
        return;
    }
    print_time_debug_ok(ctx, "timer set signals expired deadline");
    if crate::syscall::sys_timer_cancel(timer_handle).is_err() {
        ctx.serial.write_str("  [FAIL] timer cancel failed\n");
        return;
    }
    match crate::syscall::sys_object_wait_one(timer_handle, 1 << 7, 0, &mut timer_pending) {
        Err(crate::syscall::ZxError::ErrTimedOut) if timer_pending & (1 << 7) == 0 => {
            print_time_debug_ok(ctx, "timer cancel clears signal");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] timer cancel left signal pending=0x");
            print_hex(&mut ctx.serial, timer_pending as u64);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_time_debug_error(ctx, "timer cancel clears signal", e);
            return;
        }
    }

    match crate::syscall::sys_debuglog_create(0, 0, &mut debuglog_handle) {
        Ok(_) => print_time_debug_ok(ctx, "debuglog create"),
        Err(e) => {
            print_time_debug_error(ctx, "debuglog create", e);
            return;
        }
    }
    if crate::syscall::sys_debuglog_write(
        debuglog_handle,
        0,
        debug_payload.as_ptr() as usize,
        debug_payload.len(),
    )
    .is_err()
        || crate::syscall::sys_debuglog_read(
            debuglog_handle,
            0,
            debug_readback.as_mut_ptr() as usize,
            debug_readback.len(),
        )
        .ok()
            != Some(debug_payload.len())
        || debug_readback != *debug_payload
    {
        ctx.serial
            .write_str("  [FAIL] debuglog write/read failed\n");
        return;
    }
    print_time_debug_ok(ctx, "debuglog write/read");
    if crate::syscall::sys_debug_read(debug_zero.as_mut_ptr() as usize, debug_zero.len()).ok()
        != Some(0)
        || debug_zero != [0u8; 4]
        || crate::syscall::sys_debug_write(debug_payload.as_ptr() as usize, debug_payload.len())
            .is_err()
        || crate::syscall::sys_debug_send_command(
            debug_payload.as_ptr() as usize,
            debug_payload.len(),
        )
        .is_err()
    {
        ctx.serial
            .write_str("  [FAIL] debug read/write/send-command failed\n");
        return;
    }
    print_time_debug_ok(ctx, "debug read/write/send-command");

    match crate::syscall::sys_system_get_event(0, 0, &mut system_event) {
        Ok(_) => print_time_debug_ok(ctx, "system get event"),
        Err(e) => {
            print_time_debug_error(ctx, "system get event", e);
            return;
        }
    }
    match crate::syscall::sys_system_get_event(0, 4, &mut system_event) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_time_debug_ok(ctx, "reject invalid system event kind");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid system event kind unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_time_debug_error(ctx, "reject invalid system event kind", e);
            return;
        }
    }

    if crate::syscall::sys_process_create(0, 0, 0, 0, &mut exception_proc, &mut exception_vmar)
        .is_err()
        || crate::syscall::sys_create_exception_channel(exception_proc, 0, &mut exception_channel)
            .is_err()
        || crate::syscall::sys_channel_read(
            exception_channel,
            0,
            exception_packet.as_mut_ptr() as usize,
            exception_packet.len(),
            exception_handles.as_mut_ptr() as usize,
            exception_handles.len(),
            &mut exception_bytes_actual,
            &mut exception_handles_actual,
        )
        .is_err()
        || exception_bytes_actual != exception_packet.len()
        || exception_handles_actual != 1
        || crate::syscall::sys_exception_get_thread(exception_handles[0], &mut exception_thread)
            .is_err()
        || crate::syscall::sys_exception_get_process(exception_handles[0], &mut exception_process)
            .is_err()
        || crate::syscall::sys_task_resume_from_exception(exception_proc, exception_handles[0], 0)
            .is_err()
    {
        ctx.serial
            .write_str("  [FAIL] exception channel lifecycle failed\n");
        return;
    }
    print_time_debug_ok(ctx, "exception channel lifecycle");
    match crate::syscall::sys_create_exception_channel(exception_proc, 2, &mut exception_channel) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_time_debug_ok(ctx, "reject invalid exception option");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid exception option unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_time_debug_error(ctx, "reject invalid exception option", e);
            return;
        }
    }

    let _ = crate::syscall::sys_handle_close(clock_handle);
    let _ = crate::syscall::sys_handle_close(timer_handle);
    let _ = crate::syscall::sys_handle_close(debuglog_handle);
    let _ = crate::syscall::sys_handle_close(system_event);
    let _ = crate::syscall::sys_handle_close(exception_channel);
    let _ = crate::syscall::sys_handle_close(exception_handles[0]);
    let _ = crate::syscall::sys_handle_close(exception_thread);
    let _ = crate::syscall::sys_handle_close(exception_process);
    let _ = crate::syscall::sys_handle_close(exception_proc);
    ctx.serial
        .write_str("[OK] time/debug/system/exception tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Zircon hypervisor calls...\n");
    let mut guest_handle = 0u32;
    let mut guest_vmar = 0u32;
    let mut guest_port = 0u32;
    let mut vcpu_handle = 0u32;
    let mut vcpu_packet = [0xffu8; 48];
    let mut vcpu_state = [0xffu8; 256];
    let mut vcpu_io = [0x5au8; 24];
    let smc_params = [0u8; 64];
    let mut smc_result = [0xffu8; 64];

    match crate::syscall::sys_guest_create(0, 0, &mut guest_handle, &mut guest_vmar) {
        Ok(_) => print_hypervisor_ok(ctx, "guest create"),
        Err(e) => {
            print_hypervisor_error(ctx, "guest create", e);
            return;
        }
    }
    match crate::syscall::sys_guest_create(0, 1, &mut guest_handle, &mut guest_vmar) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_hypervisor_ok(ctx, "reject invalid guest option");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid guest option unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "reject invalid guest option", e);
            return;
        }
    }
    if crate::syscall::sys_port_create(0, &mut guest_port).is_err()
        || crate::syscall::sys_guest_set_trap(
            guest_handle,
            1,
            0x4000,
            PAGE_SIZE as u64,
            guest_port,
            0x55,
        )
        .is_err()
    {
        ctx.serial.write_str("  [FAIL] guest trap setup failed\n");
        return;
    }
    print_hypervisor_ok(ctx, "guest memory trap");
    match crate::syscall::sys_guest_set_trap(
        guest_handle,
        3,
        0x4000,
        PAGE_SIZE as u64,
        guest_port,
        0,
    ) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_hypervisor_ok(ctx, "reject invalid trap kind");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid trap kind unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "reject invalid trap kind", e);
            return;
        }
    }
    match crate::syscall::sys_guest_set_trap(
        guest_handle,
        1,
        0x4001,
        PAGE_SIZE as u64,
        guest_port,
        0,
    ) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_hypervisor_ok(ctx, "reject unaligned memory trap");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject unaligned memory trap unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "reject unaligned memory trap", e);
            return;
        }
    }
    match crate::syscall::sys_guest_set_trap(guest_handle, 2, 3, 7, guest_port, 0) {
        Ok(_) => print_hypervisor_ok(ctx, "guest io trap"),
        Err(e) => {
            print_hypervisor_error(ctx, "guest io trap", e);
            return;
        }
    }

    match crate::syscall::sys_vcpu_create(guest_handle, 0, 0x8000, &mut vcpu_handle) {
        Ok(_) => print_hypervisor_ok(ctx, "vcpu create"),
        Err(e) => {
            print_hypervisor_error(ctx, "vcpu create", e);
            return;
        }
    }
    match crate::syscall::sys_vcpu_create(guest_handle, 0, 0x8001, &mut vcpu_handle) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_hypervisor_ok(ctx, "reject unaligned vcpu entry");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject unaligned vcpu entry unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "reject unaligned vcpu entry", e);
            return;
        }
    }
    match crate::syscall::sys_vcpu_resume(vcpu_handle, vcpu_packet.as_mut_ptr() as usize) {
        Err(crate::syscall::ZxError::ErrNotSupported) if vcpu_packet == [0u8; 48] => {
            print_hypervisor_ok(ctx, "vcpu resume packet modeled");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] vcpu resume unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "vcpu resume packet modeled", e);
            return;
        }
    }
    match crate::syscall::sys_vcpu_interrupt(vcpu_handle, 128) {
        Ok(_) => print_hypervisor_ok(ctx, "vcpu interrupt"),
        Err(e) => {
            print_hypervisor_error(ctx, "vcpu interrupt", e);
            return;
        }
    }
    match crate::syscall::sys_vcpu_interrupt(vcpu_handle, 1024) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_hypervisor_ok(ctx, "reject invalid interrupt vector");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid interrupt vector unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "reject invalid interrupt vector", e);
            return;
        }
    }
    if crate::syscall::sys_vcpu_read_state(
        vcpu_handle,
        0,
        vcpu_state.as_mut_ptr() as usize,
        vcpu_state.len(),
    )
    .is_err()
        || vcpu_state != [0u8; 256]
        || crate::syscall::sys_vcpu_write_state(
            vcpu_handle,
            0,
            vcpu_state.as_ptr() as usize,
            vcpu_state.len(),
        )
        .is_err()
        || crate::syscall::sys_vcpu_write_state(
            vcpu_handle,
            1,
            vcpu_io.as_ptr() as usize,
            vcpu_io.len(),
        )
        .is_err()
    {
        ctx.serial
            .write_str("  [FAIL] vcpu state read/write failed\n");
        return;
    }
    print_hypervisor_ok(ctx, "vcpu state read/write");
    match crate::syscall::sys_vcpu_read_state(
        vcpu_handle,
        1,
        vcpu_io.as_mut_ptr() as usize,
        vcpu_io.len(),
    ) {
        Err(crate::syscall::ZxError::ErrInvalidArgs) => {
            print_hypervisor_ok(ctx, "reject invalid read-state kind");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid read-state kind unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "reject invalid read-state kind", e);
            return;
        }
    }
    match crate::syscall::sys_smc_call(
        0,
        smc_params.as_ptr() as usize,
        smc_result.as_mut_ptr() as usize,
    ) {
        Err(crate::syscall::ZxError::ErrNotSupported) if smc_result == [0u8; 64] => {
            print_hypervisor_ok(ctx, "smc call unsupported with zero result");
        }
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] smc call unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_hypervisor_error(ctx, "smc call unsupported with zero result", e);
            return;
        }
    }

    let _ = crate::syscall::sys_handle_close(vcpu_handle);
    let _ = crate::syscall::sys_handle_close(guest_port);
    let _ = crate::syscall::sys_handle_close(guest_handle);
    let _ = crate::syscall::sys_handle_close(guest_vmar);
    ctx.serial.write_str("[OK] hypervisor tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing Linux signal calls...\n");
    let mut sigset = 0xffff_ffff_ffff_ffffu64;
    let mut siginfo = [0xffu8; 128];
    match crate::syscall::sys_rt_sigaction(
        0,
        0,
        &mut sigset as *mut u64 as usize,
        core::mem::size_of::<u64>(),
    ) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject signal zero action"),
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject signal zero action unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject signal zero action", e);
            return;
        }
    }
    if crate::syscall::sys_rt_sigaction(
        2,
        0,
        &mut sigset as *mut u64 as usize,
        core::mem::size_of::<u64>(),
    )
    .is_err()
        || sigset != 0
        || crate::syscall::sys_rt_sigprocmask(
            0,
            0,
            &mut sigset as *mut u64 as usize,
            core::mem::size_of::<u64>(),
        )
        .is_err()
        || crate::syscall::sys_rt_sigpending(
            &mut sigset as *mut u64 as usize,
            core::mem::size_of::<u64>(),
        )
        .is_err()
        || crate::syscall::sys_rt_sigtimedwait(
            &mut sigset as *mut u64 as usize,
            siginfo.as_mut_ptr() as usize,
            0,
            core::mem::size_of::<u64>(),
        )
        .is_err()
        || crate::syscall::sys_rt_sigqueueinfo(1, 10, siginfo.as_ptr() as usize).is_err()
        || crate::syscall::sys_kill(1, 0).is_err()
    {
        ctx.serial
            .write_str("  [FAIL] modeled signal path failed\n");
        return;
    }
    print_linux_ok(ctx, "signal masks and queue info");
    match crate::syscall::sys_signalfd4(
        usize::MAX,
        &sigset as *const u64 as usize,
        core::mem::size_of::<u64>(),
        0,
    ) {
        Ok(fd) => {
            print_linux_ok(ctx, "signalfd create");
            let _ = crate::syscall::sys_close(fd);
        }
        Err(e) => {
            print_linux_error(ctx, "signalfd create", e);
            return;
        }
    }
    match crate::syscall::sys_signalfd4(
        usize::MAX,
        &sigset as *const u64 as usize,
        core::mem::size_of::<u64>() + 1,
        0,
    ) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad sigset size"),
        Ok(fd) => {
            ctx.serial
                .write_str("  [FAIL] reject bad sigset size unexpectedly fd=");
            print_number(&mut ctx.serial, fd as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad sigset size", e);
            return;
        }
    }

    ctx.serial.write_str("[TEST] Testing Linux IPC calls...\n");
    let semaphore_id = match crate::syscall::sys_semget(0, 2, 0) {
        Ok(id) => {
            print_linux_ok(ctx, "semget");
            id
        }
        Err(e) => {
            print_linux_error(ctx, "semget", e);
            return;
        }
    };
    let semop = [0u16; 3];
    if crate::syscall::sys_semctl(semaphore_id, 0, 0, 0).is_err()
        || crate::syscall::sys_semop(semaphore_id, semop.as_ptr() as usize, 1).is_err()
    {
        ctx.serial
            .write_str("  [FAIL] semaphore object ops failed\n");
        return;
    }
    print_linux_ok(ctx, "semaphore object ops");
    match crate::syscall::sys_semget(0, 0, 0) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject zero sem count"),
        Ok(id) => {
            ctx.serial
                .write_str("  [FAIL] reject zero sem count unexpectedly id=");
            print_number(&mut ctx.serial, id as u32);
            ctx.serial.write_str("\n");
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject zero sem count", e);
            return;
        }
    }
    let msg_id = match crate::syscall::sys_msgget(0, 0) {
        Ok(id) => {
            print_linux_ok(ctx, "msgget");
            id
        }
        Err(e) => {
            print_linux_error(ctx, "msgget", e);
            return;
        }
    };
    let msg_payload = b"linux-ipc";
    let mut msg_readback = [0u8; 9];
    match crate::syscall::sys_msgsnd(msg_id, msg_payload.as_ptr() as usize, msg_payload.len(), 0) {
        Ok(0) => print_linux_ok(ctx, "msgsnd returns zero"),
        Ok(value) => {
            print_linux_count_mismatch(ctx, "msgsnd return", 0, value);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "msgsnd", e);
            return;
        }
    }
    match crate::syscall::sys_msgrcv(
        msg_id,
        msg_readback.as_mut_ptr() as usize,
        msg_readback.len(),
        0,
        0,
    ) {
        Ok(read) if read == msg_payload.len() && msg_readback == *msg_payload => {
            print_linux_ok(ctx, "msgrcv round trip");
        }
        Ok(read) => {
            print_linux_count_mismatch(ctx, "msgrcv count", msg_payload.len(), read);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "msgrcv", e);
            return;
        }
    }
    let shm_id = match crate::syscall::sys_shmget(0, PAGE_SIZE, 0) {
        Ok(id) => {
            print_linux_ok(ctx, "shmget");
            id
        }
        Err(e) => {
            print_linux_error(ctx, "shmget", e);
            return;
        }
    };
    let shm_addr = match crate::syscall::sys_shmat(shm_id, 0, 0) {
        Ok(addr) if addr != 0 => {
            print_linux_ok(ctx, "shmat");
            addr
        }
        Ok(_) => {
            ctx.serial.write_str("  [FAIL] shmat returned null\n");
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "shmat", e);
            return;
        }
    };
    if crate::syscall::sys_shmdt(shm_id, shm_addr, 0).is_err()
        || crate::syscall::sys_shmctl(shm_id, 0, siginfo.as_mut_ptr() as usize).is_err()
    {
        ctx.serial
            .write_str("  [FAIL] shared memory object ops failed\n");
        return;
    }
    print_linux_ok(ctx, "shared memory object ops");

    ctx.serial.write_str("[TEST] Testing Linux net calls...\n");
    const AF_UNIX: usize = 1;
    const AF_INET: usize = 2;
    const SOCK_STREAM: usize = 1;
    const SOCK_DGRAM: usize = 2;
    const IPPROTO_TCP: usize = 6;
    let mut socket_pair = [0i32; 2];
    match crate::syscall::sys_socketpair(AF_UNIX, SOCK_STREAM, 0, socket_pair.as_mut_ptr() as usize)
    {
        Ok(_) => print_linux_ok(ctx, "socketpair"),
        Err(e) => {
            print_linux_error(ctx, "socketpair", e);
            return;
        }
    }
    let net_payload = b"net-ok";
    let mut net_readback = [0u8; 6];
    let mut sock_addr = [0u8; 16];
    if crate::syscall::sys_sendto(
        socket_pair[0] as usize,
        net_payload.as_ptr() as usize,
        net_payload.len(),
        0,
        0,
        0,
    )
    .is_err()
    {
        ctx.serial.write_str("  [FAIL] sendto socketpair failed\n");
        let _ = crate::syscall::sys_close(socket_pair[0] as usize);
        let _ = crate::syscall::sys_close(socket_pair[1] as usize);
        return;
    }
    match crate::syscall::sys_recvfrom(
        socket_pair[1] as usize,
        net_readback.as_mut_ptr() as usize,
        net_readback.len(),
        0,
        0,
        0,
    ) {
        Ok(read) if read == net_payload.len() && net_readback == *net_payload => {
            print_linux_ok(ctx, "sendto recvfrom pair");
        }
        Ok(read) => {
            print_linux_count_mismatch(ctx, "recvfrom count", net_payload.len(), read);
            let _ = crate::syscall::sys_close(socket_pair[0] as usize);
            let _ = crate::syscall::sys_close(socket_pair[1] as usize);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "recvfrom pair", e);
            let _ = crate::syscall::sys_close(socket_pair[0] as usize);
            let _ = crate::syscall::sys_close(socket_pair[1] as usize);
            return;
        }
    }
    if crate::syscall::sys_sendto(
        socket_pair[0] as usize,
        net_payload.as_ptr() as usize,
        net_payload.len(),
        0,
        0,
        0,
    )
    .is_err()
    {
        ctx.serial
            .write_str("  [FAIL] second sendto socketpair failed\n");
        let _ = crate::syscall::sys_close(socket_pair[0] as usize);
        let _ = crate::syscall::sys_close(socket_pair[1] as usize);
        return;
    }
    let mut recv_addr_len = sock_addr.len() as u32;
    if crate::syscall::sys_recvfrom(
        socket_pair[1] as usize,
        net_readback.as_mut_ptr() as usize,
        net_readback.len(),
        0,
        sock_addr.as_mut_ptr() as usize,
        &mut recv_addr_len as *mut u32 as usize,
    )
    .is_err()
        || recv_addr_len != 0
    {
        ctx.serial
            .write_str("  [FAIL] recvfrom address path failed\n");
        let _ = crate::syscall::sys_close(socket_pair[0] as usize);
        let _ = crate::syscall::sys_close(socket_pair[1] as usize);
        return;
    }
    print_linux_ok(ctx, "recvfrom address path");
    let mut sock_addr_len = sock_addr.len() as u32;
    if crate::syscall::sys_getsockname(
        socket_pair[0] as usize,
        sock_addr.as_mut_ptr() as usize,
        &mut sock_addr_len as *mut u32 as usize,
    )
    .is_err()
        || sock_addr_len != 0
        || crate::syscall::sys_setsockopt(socket_pair[0] as usize, 1, 1, 0, 0).is_err()
        || crate::syscall::sys_shutdown(socket_pair[0] as usize, 2).is_err()
    {
        ctx.serial
            .write_str("  [FAIL] socket option/name path failed\n");
        let _ = crate::syscall::sys_close(socket_pair[0] as usize);
        let _ = crate::syscall::sys_close(socket_pair[1] as usize);
        return;
    }
    print_linux_ok(ctx, "socket option/name path");
    let tcp_fd = match crate::syscall::sys_socket(AF_INET, SOCK_STREAM, IPPROTO_TCP) {
        Ok(fd) => {
            print_linux_ok(ctx, "inet socket create");
            fd
        }
        Err(e) => {
            print_linux_error(ctx, "inet socket create", e);
            let _ = crate::syscall::sys_close(socket_pair[0] as usize);
            let _ = crate::syscall::sys_close(socket_pair[1] as usize);
            return;
        }
    };
    match crate::syscall::sys_socket(AF_UNIX, 3, 0) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject invalid socket combo"),
        Ok(fd) => {
            ctx.serial
                .write_str("  [FAIL] reject invalid socket combo unexpectedly fd=");
            print_number(&mut ctx.serial, fd as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(fd);
            let _ = crate::syscall::sys_close(tcp_fd);
            let _ = crate::syscall::sys_close(socket_pair[0] as usize);
            let _ = crate::syscall::sys_close(socket_pair[1] as usize);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject invalid socket combo", e);
            let _ = crate::syscall::sys_close(tcp_fd);
            let _ = crate::syscall::sys_close(socket_pair[0] as usize);
            let _ = crate::syscall::sys_close(socket_pair[1] as usize);
            return;
        }
    }
    let _ = crate::syscall::sys_close(tcp_fd);
    let _ = crate::syscall::sys_close(socket_pair[0] as usize);
    let _ = crate::syscall::sys_close(socket_pair[1] as usize);

    ctx.serial.write_str("[TEST] Testing Linux misc calls...\n");
    let memfd_name = b"smros-memfd\0";
    let memfd = match crate::syscall::sys_memfd_create(memfd_name.as_ptr() as usize, 0x1) {
        Ok(fd) => {
            print_linux_ok(ctx, "memfd create");
            fd
        }
        Err(e) => {
            print_linux_error(ctx, "memfd create", e);
            return;
        }
    };
    let mut random_bytes = [0u8; 8];
    if crate::syscall::sys_getrandom(random_bytes.as_mut_ptr() as usize, random_bytes.len(), 0x1)
        .is_err()
        || crate::syscall::sys_eventfd2(1, 0).is_err()
        || crate::syscall::sys_epoll_create1(0).is_err()
        || crate::syscall::sys_membarrier(0, 0, 0).is_err()
        || random_bytes == [0u8; 8]
    {
        ctx.serial.write_str("  [FAIL] misc positive path failed\n");
        let _ = crate::syscall::sys_close(memfd);
        return;
    }
    print_linux_ok(ctx, "random eventfd epoll membarrier");
    match crate::syscall::sys_getrandom(random_bytes.as_mut_ptr() as usize, random_bytes.len(), 4) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad getrandom flags"),
        Ok(value) => {
            ctx.serial
                .write_str("  [FAIL] reject bad getrandom flags unexpectedly read=");
            print_number(&mut ctx.serial, value as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(memfd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad getrandom flags", e);
            let _ = crate::syscall::sys_close(memfd);
            return;
        }
    }
    match crate::syscall::sys_memfd_create(memfd_name.as_ptr() as usize, 0x8000_0000) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad memfd flags"),
        Ok(fd) => {
            ctx.serial
                .write_str("  [FAIL] reject bad memfd flags unexpectedly fd=");
            print_number(&mut ctx.serial, fd as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(fd);
            let _ = crate::syscall::sys_close(memfd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad memfd flags", e);
            let _ = crate::syscall::sys_close(memfd);
            return;
        }
    }
    match crate::syscall::sys_close_range(memfd, memfd, 0) {
        Ok(_) => print_linux_ok(ctx, "close_range closes memfd"),
        Err(e) => {
            print_linux_error(ctx, "close_range closes memfd", e);
            let _ = crate::syscall::sys_close(memfd);
            return;
        }
    }
    match crate::syscall::sys_close_range(9, 8, 0) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad close_range"),
        Ok(_) => {
            ctx.serial
                .write_str("  [FAIL] reject bad close_range unexpectedly succeeded\n");
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad close_range", e);
            return;
        }
    }
    ctx.serial
        .write_str("[OK] Linux signal, IPC, misc, and net tests completed\n");

    ctx.serial.write_str("[TEST] Closing VMO handle... ");
    match crate::syscall::sys_handle_close(vmo_handle) {
        Ok(_) => ctx.serial.write_str("[OK] handle closed\n"),
        Err(_) => {
            ctx.serial.write_str("[FAIL] handle close failed\n");
            return;
        }
    }

    print_memory_syscall_snapshot(ctx, "after");
    ctx.serial.write_str("\n=== Test Complete ===\n\n");
}

/// Command: echo - Echo arguments
fn cmd_echo(ctx: &mut ShellContext, args: &[&str]) {
    ctx.serial.write_str("Echo: ");
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            ctx.serial.write_str(" ");
        }
        ctx.serial.write_str(arg);
    }
    ctx.serial.write_str("\n\n");
}

/// Command: clear - Clear screen
fn cmd_clear(_ctx: &mut ShellContext, _args: &[&str]) {
    // Send ANSI clear screen code
    // In real impl, would use: _ctx.serial.write_str("\x1b[2J\x1b[H");
}

/// Command: ps - List all processes
fn cmd_ps(ctx: &mut ShellContext, _args: &[&str]) {
    let pm = process_manager();

    ctx.serial
        .write_str("\n  PID  State      Name         Threads  Parent\n");
    ctx.serial
        .write_str("  ─────────────────────────────────────────────\n");

    let mut count = 0;
    for i in 0..crate::kernel_lowlevel::memory::MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                print_number(&mut ctx.serial, pcb.pid as u32);
                ctx.serial.write_str("    ");
                ctx.serial.write_str(pcb.state.as_str());
                ctx.serial.write_str("  ");
                ctx.serial.write_str(pcb.name);
                for _ in 0..(12usize.saturating_sub(pcb.name.len())) {
                    ctx.serial.write_byte(b' ');
                }
                print_number(&mut ctx.serial, pcb.thread_count as u32);
                ctx.serial.write_str("         ");
                print_number(&mut ctx.serial, pcb.parent_pid as u32);
                ctx.serial.write_str("\n");
                count += 1;
            }
        }
    }

    ctx.serial
        .write_str("  ─────────────────────────────────────────────\n");
    ctx.serial.write_str("  Total: ");
    print_number(&mut ctx.serial, count as u32);
    ctx.serial.write_str(" process(es)\n");
}

/// Command: top - Show process status (interactive-like display)
fn cmd_top(ctx: &mut ShellContext, _args: &[&str]) {
    let pm = process_manager();

    ctx.serial
        .write_str("\n┌─────────────────────────────────────────────────────────────┐\n");
    ctx.serial
        .write_str("│              SMROS Process Monitor (top)                    │\n");
    ctx.serial
        .write_str("├─────────────────────────────────────────────────────────────┤\n");

    // Header
    ctx.serial
        .write_str("│  PID  │ State    │ Name       │ Threads  │ CPU Time │\n");
    ctx.serial
        .write_str("│───────┼──────────┼────────────┼──────────┼──────────│\n");

    for i in 0..crate::kernel_lowlevel::memory::MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                ctx.serial.write_str("│  ");
                print_padded_number(&mut ctx.serial, pcb.pid as u32, 3);
                ctx.serial.write_str(" │ ");
                let state_str = pcb.state.as_str().trim();
                ctx.serial.write_str(state_str);
                for _ in 0..(8usize.saturating_sub(state_str.len())) {
                    ctx.serial.write_byte(b' ');
                }
                ctx.serial.write_str("│ ");
                ctx.serial.write_str(pcb.name);
                for _ in 0..(10usize.saturating_sub(pcb.name.len())) {
                    ctx.serial.write_byte(b' ');
                }
                ctx.serial.write_str(" │    ");
                print_number(&mut ctx.serial, pcb.thread_count as u32);
                ctx.serial.write_str("   │    N/A   │\n");
            }
        }
    }

    ctx.serial
        .write_str("├─────────────────────────────────────────────────────────────┤\n");

    // Show scheduler info
    let s = scheduler::scheduler();
    ctx.serial.write_str("│ Scheduler: ");
    print_number(&mut ctx.serial, s.get_tick_count() as u32);
    ctx.serial.write_str(" ticks                        │\n");

    // Memory summary
    ctx.serial.write_str("│ Memory: ");
    print_number(
        &mut ctx.serial,
        PageFrameAllocator::allocated_pages() as u32,
    );
    ctx.serial.write_str(" used / ");
    print_number(&mut ctx.serial, PageFrameAllocator::total_pages() as u32);
    ctx.serial.write_str(" total pages           │\n");

    ctx.serial.write_str("│ Free: ");
    print_number(&mut ctx.serial, PageFrameAllocator::free_pages() as u32);
    ctx.serial.write_str(" pages (");
    print_number(
        &mut ctx.serial,
        user_logic::pages_to_kb(PageFrameAllocator::free_pages(), PAGE_SIZE) as u32,
    );
    ctx.serial.write_str(" KB)                        │\n");

    ctx.serial
        .write_str("└─────────────────────────────────────────────────────────────┘\n");
}

/// Command: meminfo - Show memory information
fn cmd_meminfo(ctx: &mut ShellContext, _args: &[&str]) {
    let total_pages = PageFrameAllocator::total_pages();
    let used_pages = PageFrameAllocator::allocated_pages();
    let free_pages = PageFrameAllocator::free_pages();
    let total_kb = user_logic::pages_to_kb(total_pages, PAGE_SIZE);
    let used_kb = user_logic::pages_to_kb(used_pages, PAGE_SIZE);
    let free_kb = user_logic::pages_to_kb(free_pages, PAGE_SIZE);
    let usage_pct = user_logic::usage_percent(used_pages, total_pages);

    ctx.serial
        .write_str("\n┌─────────────────────────────────────────┐\n");
    ctx.serial
        .write_str("│           Memory Information            │\n");
    ctx.serial
        .write_str("├─────────────────────────────────────────┤\n");
    ctx.serial
        .write_str("│  Total Memory:                          │\n");
    ctx.serial.write_str("│    Pages: ");
    print_number(&mut ctx.serial, total_pages as u32);
    ctx.serial.write_str("                            │\n");
    ctx.serial.write_str("│    Size:  ");
    print_number(&mut ctx.serial, total_kb as u32);
    ctx.serial.write_str(" KB (");
    print_number(&mut ctx.serial, (total_kb / 1024) as u32);
    ctx.serial.write_str(" MB)                   │\n");
    ctx.serial
        .write_str("│                                         │\n");
    ctx.serial
        .write_str("│  Used Memory:                           │\n");
    ctx.serial.write_str("│    Pages: ");
    print_number(&mut ctx.serial, used_pages as u32);
    ctx.serial.write_str("                            │\n");
    ctx.serial.write_str("│    Size:  ");
    print_number(&mut ctx.serial, used_kb as u32);
    ctx.serial.write_str(" KB                          │\n");
    ctx.serial.write_str("│    Usage: ");
    print_number(&mut ctx.serial, usage_pct as u32);
    ctx.serial.write_str("%                             │\n");
    ctx.serial
        .write_str("│                                         │\n");
    ctx.serial
        .write_str("│  Free Memory:                           │\n");
    ctx.serial.write_str("│    Pages: ");
    print_number(&mut ctx.serial, free_pages as u32);
    ctx.serial.write_str("                            │\n");
    ctx.serial.write_str("│    Size:  ");
    print_number(&mut ctx.serial, free_kb as u32);
    ctx.serial.write_str(" KB                          │\n");
    ctx.serial
        .write_str("│                                         │\n");
    ctx.serial
        .write_str("│  Page Size: 4 KB (4096 bytes)           │\n");
    ctx.serial
        .write_str("└─────────────────────────────────────────┘\n");

    let stats = crate::syscall::memory_syscall_stats();
    ctx.serial.write_str("  Linux VM: maps=");
    print_number(&mut ctx.serial, stats.linux_mapping_count as u32);
    ctx.serial.write_str(", bytes=");
    print_number(&mut ctx.serial, stats.linux_mapped_bytes as u32);
    ctx.serial.write_str(", pages=");
    print_number(&mut ctx.serial, stats.linux_committed_pages as u32);
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Linux brk: start=0x");
    print_hex(&mut ctx.serial, stats.brk_start as u64);
    ctx.serial.write_str(", current=0x");
    print_hex(&mut ctx.serial, stats.brk_current as u64);
    ctx.serial.write_str(", limit=0x");
    print_hex(&mut ctx.serial, stats.brk_limit as u64);
    ctx.serial.write_str(", pages=");
    print_number(&mut ctx.serial, stats.brk_committed_pages as u32);
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Zircon VM: vmos=");
    print_number(&mut ctx.serial, stats.zircon_vmo_count as u32);
    ctx.serial.write_str(", bytes=");
    print_number(&mut ctx.serial, stats.zircon_vmo_bytes as u32);
    ctx.serial.write_str(", pages=");
    print_number(&mut ctx.serial, stats.zircon_vmo_committed_pages as u32);
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Zircon VMAR: vmars=");
    print_number(&mut ctx.serial, stats.zircon_vmar_count as u32);
    ctx.serial.write_str(", mappings=");
    print_number(&mut ctx.serial, stats.zircon_mapping_count as u32);
    ctx.serial.write_str(", root=0x");
    print_hex(&mut ctx.serial, stats.zircon_root_vmar_handle as u64);
    ctx.serial.write_str("\n");
}

/// Command: uptime - Show system uptime
fn cmd_uptime(ctx: &mut ShellContext, _args: &[&str]) {
    let s = scheduler::scheduler();
    let ticks = s.get_tick_count();

    // Assuming 100Hz timer (10ms per tick)
    let (seconds, minutes, hours, days) = user_logic::uptime_parts(ticks);

    ctx.serial.write_str("\nSystem Uptime: ");
    if days > 0 {
        print_number(&mut ctx.serial, days as u32);
        ctx.serial.write_str(" day(s), ");
    }
    let remaining_hours = hours % 24;
    if hours > 0 {
        print_number(&mut ctx.serial, remaining_hours as u32);
        ctx.serial.write_str(" hour(s), ");
    }
    let remaining_minutes = minutes % 60;
    print_number(&mut ctx.serial, remaining_minutes as u32);
    ctx.serial.write_str(" minute(s), ");
    let remaining_seconds = seconds % 60;
    print_number(&mut ctx.serial, remaining_seconds as u32);
    ctx.serial.write_str(" second(s)\n\n");
}

/// Command: kill - Terminate a process by PID
fn cmd_kill(ctx: &mut ShellContext, args: &[&str]) {
    if args.is_empty() {
        ctx.serial.write_str("Usage: kill <pid>\n");
        return;
    }

    // Parse PID from argument
    let pid_str = args[0];
    let pid: usize = match parse_number(pid_str) {
        Some(n) => n,
        None => {
            ctx.serial.write_str("Invalid PID: ");
            ctx.serial.write_str(pid_str);
            ctx.serial.write_str("\n");
            return;
        }
    };

    let pm = process_manager();
    if pm.terminate_process(pid) {
        ctx.serial.write_str("Process ");
        print_number(&mut ctx.serial, pid as u32);
        ctx.serial.write_str(" terminated\n");
    } else {
        ctx.serial.write_str("Failed to terminate process ");
        print_number(&mut ctx.serial, pid as u32);
        ctx.serial.write_str("\n");
    }
}

/// Parse a decimal number from a string
fn parse_number(s: &str) -> Option<usize> {
    let mut result: usize = 0;
    for byte in s.bytes() {
        let digit = user_logic::decimal_digit_value(byte)?;
        result = user_logic::parse_digit_step(result, digit)?;
    }
    Some(result)
}

/// Print a number with padding (right-aligned)
fn print_padded_number(serial: &mut Serial, num: u32, width: usize) {
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut temp = num;

    if num == 0 {
        buf[i] = b'0';
        i = 1;
    } else {
        while temp > 0 && i < 10 {
            buf[i] = b'0' + (temp % 10) as u8;
            temp /= 10;
            i += 1;
        }
    }

    // Pad with spaces if needed
    let num_len = i;
    for _ in 0..user_logic::saturating_sub(width, num_len) {
        serial.write_byte(b' ');
    }

    // Print in reverse order
    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}

/// Command: exit - Exit shell
fn cmd_exit(_ctx: &mut ShellContext, _args: &[&str]) {
    // This should never return - would call exit syscall
    // For now, just hang
    loop {
        cortex_a::asm::wfi();
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn print_number(serial: &mut Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 10];
    let mut i = 0;

    while num > 0 && i < 10 {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_hex(serial: &mut Serial, num: u64) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = 0;
    let mut temp = num;

    while temp > 0 && i < 16 {
        buf[i] = hex_chars[(temp & 0xF) as usize];
        temp >>= 4;
        i += 1;
    }

    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}

fn print_zx_error(serial: &mut Serial, err: crate::syscall::ZxError) {
    let code = err as i32;
    if code < 0 {
        serial.write_str("-");
        print_number(serial, (-code) as u32);
    } else {
        print_number(serial, code as u32);
    }
}

fn print_sys_error(serial: &mut Serial, err: crate::syscall::SysError) {
    print_number(serial, err as u32);
}

fn print_linux_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_linux_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::SysError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" errno=");
    print_sys_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_linux_count_mismatch(ctx: &mut ShellContext, label: &str, expected: usize, actual: usize) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" expected=");
    print_number(&mut ctx.serial, expected as u32);
    ctx.serial.write_str(", actual=");
    print_number(&mut ctx.serial, actual as u32);
    ctx.serial.write_str("\n");
}

fn print_socket_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_socket_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_socket_count_mismatch(
    ctx: &mut ShellContext,
    label: &str,
    expected: usize,
    actual: usize,
) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" expected=");
    print_number(&mut ctx.serial, expected as u32);
    ctx.serial.write_str(", actual=");
    print_number(&mut ctx.serial, actual as u32);
    ctx.serial.write_str("\n");
}

fn print_fifo_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_fifo_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_fifo_count_mismatch(ctx: &mut ShellContext, label: &str, expected: usize, actual: usize) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" expected=");
    print_number(&mut ctx.serial, expected as u32);
    ctx.serial.write_str(", actual=");
    print_number(&mut ctx.serial, actual as u32);
    ctx.serial.write_str("\n");
}

fn print_futex_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_futex_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_futex_count_mismatch(ctx: &mut ShellContext, label: &str, expected: u32, actual: u32) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" expected=");
    print_number(&mut ctx.serial, expected);
    ctx.serial.write_str(", actual=");
    print_number(&mut ctx.serial, actual);
    ctx.serial.write_str("\n");
}

fn print_port_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_port_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_port_count_mismatch(ctx: &mut ShellContext, label: &str, expected: u32, actual: u32) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" expected=");
    print_number(&mut ctx.serial, expected);
    ctx.serial.write_str(", actual=");
    print_number(&mut ctx.serial, actual);
    ctx.serial.write_str("\n");
}

fn print_signal_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_signal_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_time_debug_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_time_debug_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_hypervisor_ok(ctx: &mut ShellContext, label: &str) {
    ctx.serial.write_str("  [OK] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str("\n");
}

fn print_hypervisor_error(ctx: &mut ShellContext, label: &str, err: crate::syscall::ZxError) {
    ctx.serial.write_str("  [FAIL] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(" error=");
    print_zx_error(&mut ctx.serial, err);
    ctx.serial.write_str("\n");
}

fn print_memory_syscall_snapshot(ctx: &mut ShellContext, label: &str) {
    let stats = crate::syscall::memory_syscall_stats();
    ctx.serial.write_str("[MEM] ");
    ctx.serial.write_str(label);
    ctx.serial.write_str(": linux_maps=");
    print_number(&mut ctx.serial, stats.linux_mapping_count as u32);
    ctx.serial.write_str(", linux_pages=");
    print_number(&mut ctx.serial, stats.linux_committed_pages as u32);
    ctx.serial.write_str(", brk_pages=");
    print_number(&mut ctx.serial, stats.brk_committed_pages as u32);
    ctx.serial.write_str(", vmos=");
    print_number(&mut ctx.serial, stats.zircon_vmo_count as u32);
    ctx.serial.write_str(", vmars=");
    print_number(&mut ctx.serial, stats.zircon_vmar_count as u32);
    ctx.serial.write_str(", vmar_maps=");
    print_number(&mut ctx.serial, stats.zircon_mapping_count as u32);
    ctx.serial.write_str(", root=0x");
    print_hex(&mut ctx.serial, stats.zircon_root_vmar_handle as u64);
    ctx.serial.write_str("\n");
}

// ============================================================================
// Shell Entry Point and Startup
// ============================================================================

/// EL0 shell entry point - this runs in user mode
#[no_mangle]
pub extern "C" fn user_shell_entry() -> ! {
    // Create and run shell
    let mut shell = UserShell::new();
    shell.run()
}

/// Start user shell as a scheduled thread
///
/// This function creates a kernel thread that runs the shell.
/// The thread will execute at EL1 (kernel mode) for now,
/// but is structured to support EL0 execution in the future.
pub fn start_user_shell() {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("[SHELL] Starting shell as scheduled thread...\n");

    // Create shell as a kernel thread for now
    // In the future, this would create a user process and switch to EL0
    use crate::kernel_objects::scheduler::scheduler;

    // Create thread bound to current CPU
    let thread_id = scheduler().create_thread(shell_thread_wrapper, "user_shell");

    match thread_id {
        Some(id) => {
            serial.write_str("[SHELL] Shell thread created (ID: ");
            print_number(&mut serial, id.0 as u32);
            serial.write_str(")\n");
            serial.write_str("[SHELL] Shell will start on next scheduler tick\n");
        }
        None => {
            serial.write_str("[SHELL] ERROR: Failed to create shell thread!\n");
        }
    }
}

/// Shell thread wrapper - runs the shell
extern "C" fn shell_thread_wrapper() -> ! {
    let mut shell = UserShell::new();
    shell.run()
}
