//! EL0 Shell - User-mode shell implementation
//!
//! This shell runs in EL0 (user mode) and makes syscalls
//! to interact with the kernel. It uses the syscall interface
//! for I/O operations instead of direct hardware access.

use alloc::string::String;
use alloc::vec::Vec;
use crate::kernel_objects::scheduler;
use crate::kernel_lowlevel::serial::Serial;
use crate::kernel_lowlevel::memory::{process_manager, ProcessState, PAGE_SIZE, PageFrameAllocator};
use crate::user_level::user_test::test_write;

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
        description: "Test syscall interface (getpid, write, mmap)",
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
            } else if c.is_ascii_graphic() || c == b' ' {
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
        self.context.command_count += 1;

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
    ctx.serial.write_str("\nSMROS v0.5.0 - Simple Operating System\n");
    ctx.serial.write_str("Architecture: ARM64 (AArch64)\n");
    ctx.serial.write_str("Features: Multi-process, Syscalls, Preemptive Scheduler\n\n");
}

/// Command: testsc - Test syscall interface
fn cmd_test_syscall(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial.write_str("\n=== Syscall Test ===\n\n");

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

    // Test 3: Mmap syscall
    ctx.serial.write_str("[TEST] Testing mmap syscall... ");
    const MAP_PRIVATE: usize = 1 << 1;
    const MAP_ANONYMOUS: usize = 1 << 5;
    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let prot = 0x3; // PROT_READ | PROT_WRITE

    let result = crate::syscall::sys_mmap(0, 4096, prot, flags, 0, 0);
    match result {
        Ok(addr) => {
            ctx.serial.write_str("[OK] mmap returned address 0x");
            print_hex(&mut ctx.serial, addr as u64);
            ctx.serial.write_str("\n");
        }
        Err(e) => {
            ctx.serial.write_str("[FAIL] Error ");
            print_number(&mut ctx.serial, e as u32);
            ctx.serial.write_str("\n");
        }
    }

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

    ctx.serial.write_str("\n  PID  State      Name         Threads  Parent\n");
    ctx.serial.write_str("  ─────────────────────────────────────────────\n");

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

    ctx.serial.write_str("  ─────────────────────────────────────────────\n");
    ctx.serial.write_str("  Total: ");
    print_number(&mut ctx.serial, count as u32);
    ctx.serial.write_str(" process(es)\n");
}

/// Command: top - Show process status (interactive-like display)
fn cmd_top(ctx: &mut ShellContext, _args: &[&str]) {
    let pm = process_manager();

    ctx.serial.write_str("\n┌─────────────────────────────────────────────────────────────┐\n");
    ctx.serial.write_str("│              SMROS Process Monitor (top)                    │\n");
    ctx.serial.write_str("├─────────────────────────────────────────────────────────────┤\n");

    // Header
    ctx.serial.write_str("│  PID  │ State    │ Name       │ Threads  │ CPU Time │\n");
    ctx.serial.write_str("│───────┼──────────┼────────────┼──────────┼──────────│\n");

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

    ctx.serial.write_str("├─────────────────────────────────────────────────────────────┤\n");

    // Show scheduler info
    let s = scheduler::scheduler();
    ctx.serial.write_str("│ Scheduler: ");
    print_number(&mut ctx.serial, s.get_tick_count() as u32);
    ctx.serial.write_str(" ticks                        │\n");

    // Memory summary
    ctx.serial.write_str("│ Memory: ");
    print_number(&mut ctx.serial, PageFrameAllocator::allocated_pages() as u32);
    ctx.serial.write_str(" used / ");
    print_number(&mut ctx.serial, PageFrameAllocator::total_pages() as u32);
    ctx.serial.write_str(" total pages           │\n");

    ctx.serial.write_str("│ Free: ");
    print_number(&mut ctx.serial, PageFrameAllocator::free_pages() as u32);
    ctx.serial.write_str(" pages (");
    print_number(&mut ctx.serial, (PageFrameAllocator::free_pages() * PAGE_SIZE / 1024) as u32);
    ctx.serial.write_str(" KB)                        │\n");

    ctx.serial.write_str("└─────────────────────────────────────────────────────────────┘\n");
}

/// Command: meminfo - Show memory information
fn cmd_meminfo(ctx: &mut ShellContext, _args: &[&str]) {
    let total_pages = PageFrameAllocator::total_pages();
    let used_pages = PageFrameAllocator::allocated_pages();
    let free_pages = PageFrameAllocator::free_pages();
    let total_kb = total_pages * PAGE_SIZE / 1024;
    let used_kb = used_pages * PAGE_SIZE / 1024;
    let free_kb = free_pages * PAGE_SIZE / 1024;
    let usage_pct = if total_pages > 0 { (used_pages * 100) / total_pages } else { 0 };

    ctx.serial.write_str("\n┌─────────────────────────────────────────┐\n");
    ctx.serial.write_str("│           Memory Information            │\n");
    ctx.serial.write_str("├─────────────────────────────────────────┤\n");
    ctx.serial.write_str("│  Total Memory:                          │\n");
    ctx.serial.write_str("│    Pages: ");
    print_number(&mut ctx.serial, total_pages as u32);
    ctx.serial.write_str("                            │\n");
    ctx.serial.write_str("│    Size:  ");
    print_number(&mut ctx.serial, total_kb as u32);
    ctx.serial.write_str(" KB (");
    print_number(&mut ctx.serial, (total_kb / 1024) as u32);
    ctx.serial.write_str(" MB)                   │\n");
    ctx.serial.write_str("│                                         │\n");
    ctx.serial.write_str("│  Used Memory:                           │\n");
    ctx.serial.write_str("│    Pages: ");
    print_number(&mut ctx.serial, used_pages as u32);
    ctx.serial.write_str("                            │\n");
    ctx.serial.write_str("│    Size:  ");
    print_number(&mut ctx.serial, used_kb as u32);
    ctx.serial.write_str(" KB                          │\n");
    ctx.serial.write_str("│    Usage: ");
    print_number(&mut ctx.serial, usage_pct as u32);
    ctx.serial.write_str("%                             │\n");
    ctx.serial.write_str("│                                         │\n");
    ctx.serial.write_str("│  Free Memory:                           │\n");
    ctx.serial.write_str("│    Pages: ");
    print_number(&mut ctx.serial, free_pages as u32);
    ctx.serial.write_str("                            │\n");
    ctx.serial.write_str("│    Size:  ");
    print_number(&mut ctx.serial, free_kb as u32);
    ctx.serial.write_str(" KB                          │\n");
    ctx.serial.write_str("│                                         │\n");
    ctx.serial.write_str("│  Page Size: 4 KB (4096 bytes)           │\n");
    ctx.serial.write_str("└─────────────────────────────────────────┘\n");
}

/// Command: uptime - Show system uptime
fn cmd_uptime(ctx: &mut ShellContext, _args: &[&str]) {
    let s = scheduler::scheduler();
    let ticks = s.get_tick_count();
    
    // Assuming 100Hz timer (10ms per tick)
    let seconds = ticks / 100;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;

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
    for c in s.chars() {
        if c >= '0' && c <= '9' {
            result = result * 10 + (c as usize - '0' as usize);
        } else {
            return None;
        }
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
    for _ in 0..(width.saturating_sub(num_len)) {
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
        serial.write_str("0x0");
        return;
    }

    serial.write_str("0x");
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
