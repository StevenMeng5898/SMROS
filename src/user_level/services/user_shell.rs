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

const SHELL_INPUT_CAPACITY: usize = 255;
const SHELL_HISTORY_CAPACITY: usize = 16;
const SCHED_TICK_MS: u32 = (crate::user_level::perfetto::PERFETTO_TICK_US / 1_000) as u32;

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
    cwd: String,
}

impl ShellContext {
    fn read_line(&mut self) -> String {
        let mut input_buf = [0u8; 256];
        let mut input_len = 0usize;

        loop {
            const UART_BASE: usize = 0x9000000;
            const UART_FR: usize = 0x18;
            const UART_DR: usize = 0x00;
            const FR_RXFE: u32 = 1 << 4;

            let fr = unsafe { core::ptr::read_volatile((UART_BASE + UART_FR) as *const u32) };
            if fr & FR_RXFE != 0 {
                cortex_a::asm::wfe();
                continue;
            }

            let c = unsafe { core::ptr::read_volatile((UART_BASE + UART_DR) as *const u8) };
            if c == b'\r' || c == b'\n' {
                self.serial.write_str("\n");
                break;
            } else if c == b'\x08' || c == b'\x7f' {
                if input_len > 0 {
                    input_len -= 1;
                    self.serial.write_str("\x08 \x08");
                }
            } else if user_logic::ascii_shell_input(c) || c == b'\t' {
                if input_len < 255 {
                    input_buf[input_len] = if c == b'\t' { b' ' } else { c };
                    input_len += 1;
                    self.serial.write_byte(if c == b'\t' { b' ' } else { c });
                }
            }
        }

        String::from_utf8_lossy(&input_buf[..input_len]).into_owned()
    }
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
        description: "List processes; ps -a shows memory maps",
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
        name: "components",
        description: "Show minimal component framework state",
        handler: cmd_components,
    },
    ShellCommand {
        name: "fxfs",
        description: "Show FxFS object-store and block-driver state",
        handler: cmd_fxfs,
    },
    ShellCommand {
        name: "drivers",
        description: "Show user-space device tree and driver bindings",
        handler: cmd_drivers,
    },
    ShellCommand {
        name: "ifconfig",
        description: "Show network interface state",
        handler: cmd_ifconfig,
    },
    ShellCommand {
        name: "dns",
        description: "Resolve a host through QEMU user networking",
        handler: cmd_dns,
    },
    ShellCommand {
        name: "dhcp",
        description: "Configure eth0 with DHCP",
        handler: cmd_dhcp,
    },
    ShellCommand {
        name: "ping",
        description: "Check network reachability",
        handler: cmd_ping,
    },
    ShellCommand {
        name: "curl",
        description: "Fetch an HTTP URL",
        handler: cmd_curl,
    },
    ShellCommand {
        name: "ftp",
        description: "Read an FTP server banner",
        handler: cmd_ftp,
    },
    ShellCommand {
        name: "tls",
        description: "Test TLS support",
        handler: cmd_tls,
    },
    ShellCommand {
        name: "pwd",
        description: "Show current FxFS directory",
        handler: cmd_pwd,
    },
    ShellCommand {
        name: "ls",
        description: "List FxFS directory entries",
        handler: cmd_ls,
    },
    ShellCommand {
        name: "cd",
        description: "Change current FxFS directory",
        handler: cmd_cd,
    },
    ShellCommand {
        name: "cd..",
        description: "Change to parent FxFS directory",
        handler: cmd_cd_up,
    },
    ShellCommand {
        name: "mkdir",
        description: "Create an FxFS directory",
        handler: cmd_mkdir,
    },
    ShellCommand {
        name: "write",
        description: "Write text to an FxFS file",
        handler: cmd_write,
    },
    ShellCommand {
        name: "cat",
        description: "Read an FxFS file",
        handler: cmd_cat,
    },
    ShellCommand {
        name: "cp",
        description: "Copy an FxFS file",
        handler: cmd_cp,
    },
    ShellCommand {
        name: "mv",
        description: "Move or rename an FxFS file",
        handler: cmd_mv,
    },
    ShellCommand {
        name: "rm",
        description: "Remove an FxFS file",
        handler: cmd_rm,
    },
    ShellCommand {
        name: "run",
        description: "Load an ELF and resolve /shared/lib dependencies",
        handler: cmd_run,
    },
    ShellCommand {
        name: "vi",
        description: "Edit an FxFS file",
        handler: cmd_vi,
    },
    ShellCommand {
        name: "mount",
        description: "Show mounts or refresh the embedded /shared seed",
        handler: cmd_mount,
    },
    ShellCommand {
        name: "share",
        description: "List the live /shared FxFS view",
        handler: cmd_share,
    },
    ShellCommand {
        name: "svc",
        description: "Show /svc service directory and IPC state",
        handler: cmd_svc,
    },
    ShellCommand {
        name: "docker",
        description: "Run local Docker/OCI images",
        handler: cmd_docker,
    },
    ShellCommand {
        name: "hermes",
        description: "Run Hermes on the native Gemma provider",
        handler: cmd_hermes,
    },
    ShellCommand {
        name: "lvgl",
        description: "Render the SMROS LVGL UI port",
        handler: cmd_lvgl,
    },
    ShellCommand {
        name: "uptime",
        description: "Show system uptime",
        handler: cmd_uptime,
    },
    ShellCommand {
        name: "dhrystone",
        description: "Run Dhrystone logical multi-core benchmark",
        handler: cmd_dhrystone,
    },
    ShellCommand {
        name: "sched",
        description: "Show, set, test, and trace scheduler policy",
        handler: cmd_sched,
    },
    ShellCommand {
        name: "vm",
        description: "Create, stop, and monitor modeled VMs",
        handler: cmd_vm,
    },
    ShellCommand {
        name: "loglevel",
        description: "Show or set kernel object log level",
        handler: cmd_loglevel,
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
        name: "fuzzsc",
        description: "Fuzz Linux and Zircon syscall dispatchers",
        handler: cmd_fuzz_syscall,
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
        name: "reboot",
        description: "Reboot the machine through PSCI",
        handler: cmd_reboot,
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
    input_cursor: usize,
    history: Vec<String>,
    history_index: usize,
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
                cwd: String::from("/"),
            },
            input_buf: [0; 256],
            input_len: 0,
            input_cursor: 0,
            history: Vec::new(),
            history_index: 0,
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
        self.print("smros:");
        let cwd = self.context.cwd.clone();
        self.print(cwd.as_str());
        self.print("> ");
    }

    fn input_line(&self) -> String {
        String::from_utf8_lossy(&self.input_buf[..self.input_len]).into_owned()
    }

    fn set_input_line(&mut self, line: &str) {
        let bytes = line.as_bytes();
        self.input_len = core::cmp::min(bytes.len(), SHELL_INPUT_CAPACITY);
        self.input_cursor = self.input_len;
        for (index, byte) in bytes.iter().take(self.input_len).enumerate() {
            self.input_buf[index] = *byte;
        }
        self.repaint_input_line();
    }

    fn repaint_input_line(&mut self) {
        self.print("\r");
        self.print_prompt();
        let line = self.input_line();
        self.print(line.as_str());
        self.print("\x1b[K");
        for _ in 0..self.input_len.saturating_sub(self.input_cursor) {
            self.print("\x1b[D");
        }
    }

    fn move_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            self.print("\x1b[D");
        }
    }

    fn move_cursor_right(&mut self) {
        if self.input_cursor < self.input_len {
            self.input_cursor += 1;
            self.print("\x1b[C");
        }
    }

    fn insert_input_byte(&mut self, c: u8) {
        if self.input_len >= SHELL_INPUT_CAPACITY {
            return;
        }

        let inserted_at_end = self.input_cursor == self.input_len;
        for index in (self.input_cursor..self.input_len).rev() {
            self.input_buf[index + 1] = self.input_buf[index];
        }
        self.input_buf[self.input_cursor] = c;
        self.input_len += 1;
        self.input_cursor += 1;
        self.history_index = self.history.len();

        if inserted_at_end {
            self.print_byte(c);
        } else {
            self.repaint_input_line();
        }
    }

    fn delete_input_byte_before_cursor(&mut self) {
        if self.input_cursor == 0 {
            return;
        }

        for index in self.input_cursor..self.input_len {
            self.input_buf[index - 1] = self.input_buf[index];
        }
        self.input_cursor -= 1;
        self.input_len -= 1;
        self.history_index = self.history.len();
        self.repaint_input_line();
    }

    fn remember_history(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if self
            .history
            .last()
            .map(|previous| previous.as_str() == line)
            .unwrap_or(false)
        {
            return;
        }
        if self.history.len() == SHELL_HISTORY_CAPACITY {
            self.history.remove(0);
        }
        self.history.push(String::from(line));
    }

    fn show_previous_history(&mut self) {
        if self.history.is_empty() || self.history_index == 0 {
            return;
        }
        self.history_index -= 1;
        let line = self.history[self.history_index].clone();
        self.set_input_line(line.as_str());
    }

    fn show_next_history(&mut self) {
        if self.history_index >= self.history.len() {
            return;
        }
        self.history_index += 1;
        if self.history_index == self.history.len() {
            self.set_input_line("");
        } else {
            let line = self.history[self.history_index].clone();
            self.set_input_line(line.as_str());
        }
    }

    fn try_read_uart_byte() -> Option<u8> {
        const UART_BASE: usize = 0x9000000;
        const UART_FR: usize = 0x18;
        const UART_DR: usize = 0x00;
        const FR_RXFE: u32 = 1 << 4;

        let fr = unsafe { core::ptr::read_volatile((UART_BASE + UART_FR) as *const u32) };
        if fr & FR_RXFE != 0 {
            None
        } else {
            Some(unsafe { core::ptr::read_volatile((UART_BASE + UART_DR) as *const u8) })
        }
    }

    fn read_uart_byte() -> u8 {
        loop {
            if let Some(c) = Self::try_read_uart_byte() {
                return c;
            }
            cortex_a::asm::wfe();
        }
    }

    fn read_uart_byte_with_retry(retries: usize) -> Option<u8> {
        for _ in 0..retries {
            if let Some(c) = Self::try_read_uart_byte() {
                return Some(c);
            }
            cortex_a::asm::wfe();
        }
        None
    }

    fn handle_escape_sequence(&mut self) {
        let Some(sequence_type) = Self::read_uart_byte_with_retry(16) else {
            return;
        };
        let Some(command) = Self::read_uart_byte_with_retry(16) else {
            return;
        };
        match (sequence_type, command) {
            (b'[', b'A') | (b'O', b'A') => self.show_previous_history(),
            (b'[', b'B') | (b'O', b'B') => self.show_next_history(),
            (b'[', b'C') | (b'O', b'C') => self.move_cursor_right(),
            (b'[', b'D') | (b'O', b'D') => self.move_cursor_left(),
            _ => {}
        }
    }

    fn complete_input(&mut self) {
        let current = String::from_utf8_lossy(&self.input_buf[..self.input_cursor]).into_owned();
        if current.is_empty() {
            self.print("\n");
            self.print_completion_commands("");
            self.repaint_input_line();
            return;
        }

        let token_start = current
            .as_bytes()
            .iter()
            .rposition(|byte| *byte == b' ')
            .map(|index| index + 1)
            .unwrap_or(0);
        let token = &current[token_start..];
        let completed = if token_start == 0 {
            self.complete_command(token)
        } else {
            self.complete_fxfs_path(token)
        };

        if let Some(completed) = completed {
            if completed.len() > token.len() {
                let suffix = &completed.as_bytes()[token.len()..];
                for byte in suffix {
                    self.insert_input_byte(*byte);
                }
            }
        } else if token_start == 0 {
            self.print("\n");
            self.print_completion_commands(token);
            self.repaint_input_line();
        } else {
            self.print("\n");
            self.print_completion_paths(token);
            self.repaint_input_line();
        }
    }

    fn complete_command(&self, prefix: &str) -> Option<String> {
        let mut match_name: Option<&str> = None;
        for command in SHELL_COMMANDS {
            if command.name.starts_with(prefix) {
                if match_name.is_some() {
                    return None;
                }
                match_name = Some(command.name);
            }
        }
        match_name.map(|name| {
            let mut completed = String::from(name);
            completed.push(' ');
            completed
        })
    }

    fn print_completion_commands(&mut self, prefix: &str) {
        for command in SHELL_COMMANDS {
            if command.name.starts_with(prefix) {
                self.print("  ");
                self.print(command.name);
                self.print("\n");
            }
        }
    }

    fn complete_fxfs_path(&self, token: &str) -> Option<String> {
        let (dir_token, name_prefix) = split_completion_path(token);
        let dir_path = normalize_fxfs_path(self.context.cwd.as_str(), dir_token.as_str())?;
        let entries = crate::user_level::fxfs::entries(dir_path.as_str()).ok()?;
        let mut matched: Option<String> = None;
        for entry in entries {
            if entry.name.starts_with(name_prefix.as_str()) {
                let candidate = join_completion_path(dir_token.as_str(), entry.name.as_str());
                if matched.is_some() {
                    return None;
                }
                matched = Some(candidate);
            }
        }
        matched
    }

    fn print_completion_paths(&mut self, token: &str) {
        let (dir_token, name_prefix) = split_completion_path(token);
        let Some(dir_path) = normalize_fxfs_path(self.context.cwd.as_str(), dir_token.as_str())
        else {
            return;
        };
        if let Ok(entries) = crate::user_level::fxfs::entries(dir_path.as_str()) {
            for entry in entries {
                if entry.name.starts_with(name_prefix.as_str()) {
                    self.print("  ");
                    self.print(
                        join_completion_path(dir_token.as_str(), entry.name.as_str()).as_str(),
                    );
                    self.print("\n");
                }
            }
        }
    }

    /// Read a line of input from serial (waits for timer interrupt to yield)
    fn read_line(&mut self) -> String {
        self.input_len = 0;
        self.input_cursor = 0;
        self.history_index = self.history.len();

        loop {
            let c = Self::read_uart_byte();

            if c == b'\r' || c == b'\n' {
                // End of line
                self.print("\n");
                break;
            } else if c == b'\t' {
                self.complete_input();
            } else if c == b'\x1b' {
                self.handle_escape_sequence();
            } else if c == b'\x08' || c == b'\x7f' {
                // Backspace
                self.delete_input_byte_before_cursor();
            } else if user_logic::ascii_shell_input(c) {
                // Printable character
                self.insert_input_byte(c);
            }
        }

        // Return the line as a String to avoid borrowing issues
        self.input_line()
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

            self.remember_history(line.as_str());

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

/// Command: loglevel - Show or configure kernel object log verbosity
fn cmd_loglevel(ctx: &mut ShellContext, args: &[&str]) {
    let current = crate::kernel_objects::log::level();
    if args.is_empty() {
        ctx.serial.write_str("kernel object log level: ");
        ctx.serial.write_str(current.as_str());
        ctx.serial.write_str("\n");
        return;
    }

    let Some(level) = crate::kernel_objects::log::level_from_str(args[0]) else {
        ctx.serial
            .write_str("usage: loglevel [debug|info|warning|err|fatal]\n");
        return;
    };

    crate::kernel_objects::log::set_level(level);
    ctx.serial.write_str("kernel object log level set to ");
    ctx.serial.write_str(level.as_str());
    ctx.serial.write_str("\n");
    crate::kobj_info!("log", "runtime log level changed to {}", level.as_str());
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
    if crate::syscall::sys_handle_duplicate(
        vmo_handle,
        crate::syscall::RIGHT_SAME_RIGHTS,
        &mut dup_handle,
    )
    .is_err()
        || dup_handle == 0
        || dup_handle == vmo_handle
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
    let mut signal_event = 0u32;
    let mut signal_pending = 0u32;

    match crate::syscall::sys_event_create(0, &mut signal_event) {
        Ok(_) => print_signal_ok(ctx, "event create"),
        Err(e) => {
            print_signal_error(ctx, "event create", e);
            return;
        }
    }
    match crate::syscall::sys_object_signal(signal_event, 0, SIGNAL_USER0) {
        Ok(_) => print_signal_ok(ctx, "set user signal"),
        Err(e) => {
            print_signal_error(ctx, "set user signal", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(signal_event, SIGNAL_USER0, 0, &mut signal_pending) {
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
    match crate::syscall::sys_object_signal(signal_event, SIGNAL_USER0, 0) {
        Ok(_) => print_signal_ok(ctx, "clear user signal"),
        Err(e) => {
            print_signal_error(ctx, "clear user signal", e);
            return;
        }
    }
    match crate::syscall::sys_object_wait_one(signal_event, SIGNAL_USER0, 0, &mut signal_pending) {
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
        signal_event,
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
        signal_event,
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
    match crate::syscall::sys_port_cancel(port_handle, signal_event, 0x5052_5404) {
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
    match crate::syscall::sys_object_signal(signal_event, 0, SIGNAL_USER1) {
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
        signal_event,
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
    match crate::syscall::sys_object_signal(signal_event, 0, SIGNAL_USER0) {
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
            signal_event as usize,
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
    let _ = crate::syscall::sys_handle_close(signal_event);
    let _ = crate::syscall::sys_handle_close(dup_handle);
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
    let mut job_handle = 0u32;
    let mut proc_handle = 0u32;
    let mut proc_vmar = 0u32;
    let mut thread_handle = 0u32;
    let mut pending = 0u32;
    let mut wait_item = ShellWaitItem {
        handle: 0,
        waitfor: TEST_SIGNAL,
        pending: 0,
    };
    if crate::syscall::sys_job_create(0, 0, &mut job_handle).is_err()
        || crate::syscall::sys_process_create(job_handle, 0, 0, 0, &mut proc_handle, &mut proc_vmar)
            .is_err()
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
    let close_many = [thread_handle, proc_handle, job_handle];
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
    let mut exception_job = 0u32;
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

    if crate::syscall::sys_job_create(0, 0, &mut exception_job).is_err()
        || crate::syscall::sys_process_create(
            exception_job,
            0,
            0,
            0,
            &mut exception_proc,
            &mut exception_vmar,
        )
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
    let _ = crate::syscall::sys_handle_close(exception_job);
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

    ctx.serial
        .write_str("[TEST] Testing Linux file, dir, fd, poll, and stat calls...\n");
    #[repr(C)]
    struct ShellIovec {
        base: usize,
        len: usize,
    }
    #[repr(C)]
    struct ShellPollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    let file_path = b"/tmp/smros-file\0";
    let copy_path = b"/tmp/smros-copy-dst\0";
    let dir_path = b"/tmp\0";
    if crate::user_level::fxfs::write_file("/tmp/smros-file", b"").is_err() {
        ctx.serial
            .write_str("  [FAIL] prepare stat fixture failed\n");
        return;
    }
    let file_fd =
        match crate::syscall::sys_openat(usize::MAX - 99, file_path.as_ptr() as usize, 2, 0) {
            Ok(fd) => {
                print_linux_ok(ctx, "open file");
                fd
            }
            Err(e) => {
                print_linux_error(ctx, "open file", e);
                return;
            }
        };
    let dir_fd = match crate::syscall::sys_openat(
        usize::MAX - 99,
        dir_path.as_ptr() as usize,
        0o200000,
        0,
    ) {
        Ok(fd) => {
            print_linux_ok(ctx, "open directory");
            fd
        }
        Err(e) => {
            print_linux_error(ctx, "open directory", e);
            let _ = crate::syscall::sys_close(file_fd);
            return;
        }
    };
    match crate::syscall::sys_openat(usize::MAX - 99, file_path.as_ptr() as usize, 0x8000_0000, 0) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad open flags"),
        Ok(fd) => {
            ctx.serial
                .write_str("  [FAIL] reject bad open flags unexpectedly fd=");
            print_number(&mut ctx.serial, fd as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad open flags", e);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    }

    let file_payload = b"file-fd";
    let mut file_readback = [0u8; 7];
    if crate::syscall::sys_write(file_fd, file_payload.as_ptr() as usize, file_payload.len())
        .is_err()
        || crate::syscall::sys_lseek(file_fd, 0, 0).is_err()
        || crate::syscall::sys_read(
            file_fd,
            file_readback.as_mut_ptr() as usize,
            file_readback.len(),
        )
        .is_err()
        || file_readback != *file_payload
    {
        ctx.serial.write_str("  [FAIL] file read/write failed\n");
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "file read/write");

    let _ = crate::user_level::fxfs::delete_file("/tmp/smros-copy-dst");
    let copy_fd = match crate::syscall::sys_openat(
        usize::MAX - 99,
        copy_path.as_ptr() as usize,
        0o1 | 0o100 | 0o1000,
        0,
    ) {
        Ok(fd) => fd,
        Err(e) => {
            print_linux_error(ctx, "open creat/trunc copy destination", e);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    };
    let copy_payload = b"linux-copy";
    if crate::syscall::sys_write(copy_fd, copy_payload.as_ptr() as usize, copy_payload.len()).ok()
        != Some(copy_payload.len())
    {
        ctx.serial
            .write_str("  [FAIL] write creat/trunc copy destination failed\n");
        let _ = crate::syscall::sys_close(copy_fd);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    if let Err(e) = crate::syscall::sys_close(copy_fd) {
        print_linux_error(ctx, "close creat/trunc copy destination", e);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    let mut copy_readback = [0u8; 10];
    if crate::user_level::fxfs::read_file("/tmp/smros-copy-dst", &mut copy_readback).ok()
        != Some(copy_payload.len())
        || copy_readback != *copy_payload
    {
        ctx.serial
            .write_str("  [FAIL] creat/trunc copy destination did not persist in FxFS\n");
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "creat/trunc copy persists");

    let dup_fd = match crate::syscall::sys_dup(file_fd) {
        Ok(fd) => {
            print_linux_ok(ctx, "dup fd");
            fd
        }
        Err(e) => {
            print_linux_error(ctx, "dup fd", e);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    };
    let dup3_fd = match crate::syscall::sys_dup3(file_fd, dup_fd + 10, 0o2000000) {
        Ok(fd) => {
            print_linux_ok(ctx, "dup3 fd");
            fd
        }
        Err(e) => {
            print_linux_error(ctx, "dup3 fd", e);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    };
    if crate::syscall::sys_fcntl(file_fd, 1, 0).is_err()
        || crate::syscall::sys_fcntl(file_fd, 2, 0o2000000).is_err()
        || crate::syscall::sys_fcntl(file_fd, 4, 0o4000).is_err()
    {
        ctx.serial.write_str("  [FAIL] fcntl fd ops failed\n");
        let _ = crate::syscall::sys_close(dup3_fd);
        let _ = crate::syscall::sys_close(dup_fd);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "fcntl fd ops");
    match crate::syscall::sys_dup3(file_fd, file_fd, 0) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject dup3 same fd"),
        Ok(fd) => {
            ctx.serial
                .write_str("  [FAIL] reject dup3 same fd unexpectedly fd=");
            print_number(&mut ctx.serial, fd as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject dup3 same fd", e);
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    }

    let mut dirents = [0xffu8; 64];
    if crate::syscall::sys_getdents64(dir_fd, dirents.as_mut_ptr() as usize, dirents.len()).is_err()
        || dirents != [0u8; 64]
        || crate::syscall::sys_fchdir(dir_fd).is_err()
    {
        ctx.serial.write_str("  [FAIL] directory fd ops failed\n");
        let _ = crate::syscall::sys_close(dup3_fd);
        let _ = crate::syscall::sys_close(dup_fd);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "directory fd ops");
    match crate::syscall::sys_getdents64(file_fd, dirents.as_mut_ptr() as usize, dirents.len()) {
        Err(crate::syscall::SysError::ENODEV) => print_linux_ok(ctx, "reject getdents on file"),
        Ok(value) => {
            ctx.serial
                .write_str("  [FAIL] reject getdents on file unexpectedly returned=");
            print_number(&mut ctx.serial, value as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject getdents on file", e);
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    }

    let mut stat_buf = [0xffu8; 256];
    let mut statfs_buf = [0u8; 160];
    if crate::syscall::sys_fstat(file_fd, stat_buf.as_mut_ptr() as usize).is_err()
        || crate::syscall::sys_fstatat(
            usize::MAX - 99,
            file_path.as_ptr() as usize,
            stat_buf.as_mut_ptr() as usize,
            0,
        )
        .is_err()
        || crate::syscall::sys_statfs(
            file_path.as_ptr() as usize,
            statfs_buf.as_mut_ptr() as usize,
        )
        .is_err()
        || crate::syscall::sys_fstatfs(file_fd, statfs_buf.as_mut_ptr() as usize).is_err()
        || crate::syscall::sys_statx(
            usize::MAX - 99,
            file_path.as_ptr() as usize,
            0,
            0x7ff,
            stat_buf.as_mut_ptr() as usize,
        )
        .is_err()
    {
        ctx.serial.write_str("  [FAIL] stat path failed\n");
        let _ = crate::syscall::sys_close(dup3_fd);
        let _ = crate::syscall::sys_close(dup_fd);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "stat and statfs ops");
    match crate::syscall::sys_statx(
        usize::MAX - 99,
        file_path.as_ptr() as usize,
        0x8000_0000,
        0x7ff,
        stat_buf.as_mut_ptr() as usize,
    ) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad statx flags"),
        Ok(value) => {
            ctx.serial
                .write_str("  [FAIL] reject bad statx flags unexpectedly returned=");
            print_number(&mut ctx.serial, value as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad statx flags", e);
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    }

    let iov_a = b"iv";
    let iov_b = b"ec";
    let iovs = [
        ShellIovec {
            base: iov_a.as_ptr() as usize,
            len: iov_a.len(),
        },
        ShellIovec {
            base: iov_b.as_ptr() as usize,
            len: iov_b.len(),
        },
    ];
    if crate::syscall::sys_writev(file_fd, iovs.as_ptr() as usize, iovs.len()).is_err() {
        ctx.serial.write_str("  [FAIL] writev failed\n");
        let _ = crate::syscall::sys_close(dup3_fd);
        let _ = crate::syscall::sys_close(dup_fd);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "writev");
    let mut polls = [
        ShellPollFd {
            fd: file_fd as i32,
            events: 0x0001 | 0x0004,
            revents: 0,
        },
        ShellPollFd {
            fd: -1,
            events: 0x0001,
            revents: 7,
        },
    ];
    match crate::syscall::sys_poll(polls.as_mut_ptr() as usize, polls.len(), 0) {
        Ok(ready) if ready == 1 && polls[0].revents != 0 && polls[1].revents == 0 => {
            print_linux_ok(ctx, "poll fd readiness");
        }
        Ok(ready) => {
            ctx.serial.write_str("  [FAIL] poll ready=");
            print_number(&mut ctx.serial, ready as u32);
            ctx.serial.write_str(", revents=");
            print_number(&mut ctx.serial, polls[0].revents as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "poll fd readiness", e);
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    }
    polls[0].events = 0x4000;
    match crate::syscall::sys_poll(polls.as_mut_ptr() as usize, 1, 0) {
        Err(crate::syscall::SysError::EINVAL) => print_linux_ok(ctx, "reject bad poll events"),
        Ok(value) => {
            ctx.serial
                .write_str("  [FAIL] reject bad poll events unexpectedly returned=");
            print_number(&mut ctx.serial, value as u32);
            ctx.serial.write_str("\n");
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
        Err(e) => {
            print_linux_error(ctx, "reject bad poll events", e);
            let _ = crate::syscall::sys_close(dup3_fd);
            let _ = crate::syscall::sys_close(dup_fd);
            let _ = crate::syscall::sys_close(file_fd);
            let _ = crate::syscall::sys_close(dir_fd);
            return;
        }
    }
    if crate::syscall::sys_lseek(file_fd, 0, 0).is_err()
        || crate::syscall::sys_ftruncate(file_fd, 0).is_err()
        || crate::syscall::sys_fsync(file_fd).is_err()
        || crate::syscall::sys_sync_file_range(file_fd, 0, 0, 0).is_err()
    {
        ctx.serial.write_str("  [FAIL] fd maintenance ops failed\n");
        let _ = crate::syscall::sys_close(dup3_fd);
        let _ = crate::syscall::sys_close(dup_fd);
        let _ = crate::syscall::sys_close(file_fd);
        let _ = crate::syscall::sys_close(dir_fd);
        return;
    }
    print_linux_ok(ctx, "fd maintenance ops");
    let _ = crate::syscall::sys_close(dup3_fd);
    let _ = crate::syscall::sys_close(dup_fd);
    let _ = crate::syscall::sys_close(file_fd);
    let _ = crate::syscall::sys_close(dir_fd);
    ctx.serial
        .write_str("[OK] Linux file, dir, fd, poll, and stat tests completed\n");

    ctx.serial
        .write_str("[TEST] Testing minimal component framework and FxFS... ");
    if crate::user_level::component::smoke_test()
        && crate::user_level::fxfs::smoke_test()
        && crate::user_level::svc::smoke_test()
    {
        ctx.serial
            .write_str("[OK] component framework, FxFS, and /svc IPC returned\n");
    } else {
        ctx.serial
            .write_str("[FAIL] component framework, FxFS, or /svc IPC failed\n");
        return;
    }

    if !run_ported_app_tests(ctx) {
        return;
    }

    if !run_docker_compat_tests(ctx) {
        return;
    }

    if !run_gemma_tests(ctx) {
        return;
    }

    if !run_hermes_agent_tests(ctx) {
        return;
    }

    if !run_lvgl_tests(ctx) {
        return;
    }

    if !run_qml_cluster_tests(ctx) {
        return;
    }

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

/// Command: fuzzsc - Fuzz syscall dispatchers
fn cmd_fuzz_syscall(ctx: &mut ShellContext, args: &[&str]) {
    let options = match parse_fuzz_command_options(ctx, args) {
        Some(options) => options,
        None => return,
    };

    ctx.serial.write_str("\n=== Syscall Fuzzer ===\n");
    ctx.serial.write_str("[FUZZ] seed=0x");
    print_hex(&mut ctx.serial, options.seed);
    ctx.serial.write_str(" iterations=");
    print_usize(&mut ctx.serial, options.iterations);
    if options.time_limit_ticks != 0 {
        ctx.serial.write_str(" time_ticks=");
        print_usize(&mut ctx.serial, options.time_limit_ticks as usize);
    }
    ctx.serial.write_str("\n");

    let report: crate::syscall::SyscallFuzzReport = if options.time_limit_ticks == 0 {
        crate::syscall::fuzz_syscalls(options.seed, options.iterations)
    } else {
        crate::syscall::fuzz_syscalls_with_config(crate::syscall::SyscallFuzzConfig {
            seed: options.seed,
            iterations: options.iterations,
            time_limit_ticks: options.time_limit_ticks,
        })
    };

    ctx.serial.write_str("[OK] syscall fuzz completed\n");
    ctx.serial.write_str("  seed=0x");
    print_hex(&mut ctx.serial, report.seed);
    ctx.serial.write_str(" iterations=");
    print_usize(&mut ctx.serial, report.completed_iterations);
    ctx.serial.write_str("/");
    print_usize(&mut ctx.serial, report.iterations);
    if report.time_limit_ticks != 0 {
        ctx.serial.write_str(" time_ticks=");
        print_usize(&mut ctx.serial, report.elapsed_ticks as usize);
        ctx.serial.write_str("/");
        print_usize(&mut ctx.serial, report.time_limit_ticks as usize);
        ctx.serial.write_str(" timed_out=");
        ctx.serial
            .write_str(if report.timed_out { "yes" } else { "no" });
    }
    ctx.serial.write_str(" skipped=");
    print_usize(&mut ctx.serial, report.skipped);
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Linux interface: syscalls=");
    print_usize(&mut ctx.serial, report.linux_interface_syscalls);
    ctx.serial.write_str(" success_syscalls=");
    print_usize(&mut ctx.serial, report.linux_success_syscalls);
    ctx.serial.write_str(" cases/iter=");
    print_usize(&mut ctx.serial, report.linux_success_call_cases);
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Zircon interface: syscalls=");
    print_usize(&mut ctx.serial, report.zircon_interface_syscalls);
    ctx.serial.write_str(" success_syscalls=");
    print_usize(&mut ctx.serial, report.zircon_success_syscalls);
    ctx.serial.write_str(" cases/iter=");
    print_usize(&mut ctx.serial, report.zircon_success_call_cases);
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Linux: calls=");
    print_usize(&mut ctx.serial, report.linux_calls);
    ctx.serial.write_str(" ok=");
    print_usize(&mut ctx.serial, report.linux_ok);
    ctx.serial.write_str(" err=");
    print_usize(&mut ctx.serial, report.linux_err);
    ctx.serial.write_str(" enosys=");
    print_usize(&mut ctx.serial, report.linux_enosys);
    if report.linux_err != 0 {
        ctx.serial.write_str(" err_syscall=");
        print_usize(&mut ctx.serial, report.linux_first_err_syscall as usize);
        ctx.serial.write_str("..");
        print_usize(&mut ctx.serial, report.linux_last_err_syscall as usize);
        ctx.serial.write_str(" err_list=");
        print_fuzz_error_buckets(
            ctx,
            &report.linux_err_syscalls,
            &report.linux_err_syscall_counts,
            report.linux_err_syscall_count,
        );
    }
    if report.linux_enosys != 0 {
        ctx.serial.write_str(" enosys_syscall=");
        print_usize(&mut ctx.serial, report.linux_first_enosys_syscall as usize);
        ctx.serial.write_str("..");
        print_usize(&mut ctx.serial, report.linux_last_enosys_syscall as usize);
    }
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Zircon: calls=");
    print_usize(&mut ctx.serial, report.zircon_calls);
    ctx.serial.write_str(" ok=");
    print_usize(&mut ctx.serial, report.zircon_ok);
    ctx.serial.write_str(" err=");
    print_usize(&mut ctx.serial, report.zircon_err);
    ctx.serial.write_str(" unsupported=");
    print_usize(&mut ctx.serial, report.zircon_unsupported);
    if report.zircon_err != 0 {
        ctx.serial.write_str(" err_syscall=");
        print_usize(&mut ctx.serial, report.zircon_first_err_syscall as usize);
        ctx.serial.write_str("..");
        print_usize(&mut ctx.serial, report.zircon_last_err_syscall as usize);
        ctx.serial.write_str(" err_list=");
        print_fuzz_error_buckets(
            ctx,
            &report.zircon_err_syscalls,
            &report.zircon_err_syscall_counts,
            report.zircon_err_syscall_count,
        );
    }
    if report.zircon_unsupported != 0 {
        ctx.serial.write_str(" unsupported_syscall=");
        print_usize(
            &mut ctx.serial,
            report.zircon_first_unsupported_syscall as usize,
        );
        ctx.serial.write_str("..");
        print_usize(
            &mut ctx.serial,
            report.zircon_last_unsupported_syscall as usize,
        );
    }
    ctx.serial.write_str("\n");

    ctx.serial.write_str("  Objects: handles=");
    print_usize(&mut ctx.serial, report.created_handles);
    ctx.serial.write_str(" fds=");
    print_usize(&mut ctx.serial, report.created_fds);
    ctx.serial.write_str("\n\n");
}

const FUZZ_SHELL_DEFAULT_SEED: u64 = 1_511_431_206;
const FUZZ_SHELL_DEFAULT_ITERATIONS: usize = 2;
const FUZZ_SHELL_MAX_UNTIMED_ITERATIONS: usize = 1_000_000;
const FUZZ_TICKS_PER_SECOND: u64 = 100;
const FUZZ_MILLIS_PER_TICK: u64 = 10;

struct FuzzCommandOptions {
    seed: u64,
    iterations: usize,
    time_limit_ticks: u64,
}

#[derive(Clone, Copy)]
enum FuzzArgKind {
    Seed,
    Iterations,
    TimeSeconds,
    TimeMillis,
}

fn parse_fuzz_command_options(ctx: &mut ShellContext, args: &[&str]) -> Option<FuzzCommandOptions> {
    let mut seed = FUZZ_SHELL_DEFAULT_SEED;
    let mut iterations = FUZZ_SHELL_DEFAULT_ITERATIONS;
    let mut time_limit_ticks = 0u64;
    let mut seed_set = false;
    let mut iterations_set = false;
    let mut time_set = false;
    let mut index = 0usize;

    while index < args.len() {
        let arg = args[index];
        if matches!(arg, "help" | "--help" | "-h") {
            print_fuzz_usage(ctx);
            return None;
        }

        if let Some((key, value)) = split_fuzz_assignment(arg) {
            let Some(kind) = fuzz_arg_kind(key) else {
                ctx.serial.write_str("Unknown fuzz parameter: ");
                ctx.serial.write_str(key);
                ctx.serial.write_str("\n");
                print_fuzz_usage(ctx);
                return None;
            };
            if !apply_fuzz_option(
                ctx,
                kind,
                value,
                &mut seed,
                &mut iterations,
                &mut time_limit_ticks,
                &mut seed_set,
                &mut iterations_set,
                &mut time_set,
            ) {
                return None;
            }
        } else if let Some(kind) = fuzz_arg_kind(arg) {
            index += 1;
            let Some(value) = args.get(index) else {
                ctx.serial.write_str("Missing value for fuzz parameter: ");
                ctx.serial.write_str(arg);
                ctx.serial.write_str("\n");
                print_fuzz_usage(ctx);
                return None;
            };
            if !apply_fuzz_option(
                ctx,
                kind,
                value,
                &mut seed,
                &mut iterations,
                &mut time_limit_ticks,
                &mut seed_set,
                &mut iterations_set,
                &mut time_set,
            ) {
                return None;
            }
        } else if !seed_set {
            if !apply_fuzz_option(
                ctx,
                FuzzArgKind::Seed,
                arg,
                &mut seed,
                &mut iterations,
                &mut time_limit_ticks,
                &mut seed_set,
                &mut iterations_set,
                &mut time_set,
            ) {
                return None;
            }
        } else if !iterations_set {
            if !apply_fuzz_option(
                ctx,
                FuzzArgKind::Iterations,
                arg,
                &mut seed,
                &mut iterations,
                &mut time_limit_ticks,
                &mut seed_set,
                &mut iterations_set,
                &mut time_set,
            ) {
                return None;
            }
        } else {
            ctx.serial.write_str("Unexpected fuzz argument: ");
            ctx.serial.write_str(arg);
            ctx.serial.write_str("\n");
            print_fuzz_usage(ctx);
            return None;
        }

        index += 1;
    }

    if time_set && time_limit_ticks != 0 && !iterations_set {
        iterations = usize::MAX;
    }
    if time_limit_ticks == 0 && iterations > FUZZ_SHELL_MAX_UNTIMED_ITERATIONS {
        ctx.serial
            .write_str("Iteration count too large without a time limit; use time=<seconds> or ms=<milliseconds>\n");
        return None;
    }

    Some(FuzzCommandOptions {
        seed,
        iterations,
        time_limit_ticks,
    })
}

fn apply_fuzz_option(
    ctx: &mut ShellContext,
    kind: FuzzArgKind,
    value: &str,
    seed: &mut u64,
    iterations: &mut usize,
    time_limit_ticks: &mut u64,
    seed_set: &mut bool,
    iterations_set: &mut bool,
    time_set: &mut bool,
) -> bool {
    if value.is_empty() {
        ctx.serial.write_str("Missing fuzz parameter value\n");
        print_fuzz_usage(ctx);
        return false;
    }

    let Some(parsed) = parse_number(value) else {
        match kind {
            FuzzArgKind::Seed => ctx.serial.write_str("Invalid seed: "),
            FuzzArgKind::Iterations => ctx.serial.write_str("Invalid iteration count: "),
            FuzzArgKind::TimeSeconds | FuzzArgKind::TimeMillis => {
                ctx.serial.write_str("Invalid time limit: ")
            }
        }
        ctx.serial.write_str(value);
        ctx.serial.write_str("\n");
        return false;
    };

    match kind {
        FuzzArgKind::Seed => {
            *seed = parsed as u64;
            *seed_set = true;
        }
        FuzzArgKind::Iterations => {
            *iterations = parsed;
            *iterations_set = true;
        }
        FuzzArgKind::TimeSeconds => {
            *time_limit_ticks = (parsed as u64).saturating_mul(FUZZ_TICKS_PER_SECOND);
            *time_set = true;
        }
        FuzzArgKind::TimeMillis => {
            *time_limit_ticks = millis_to_fuzz_ticks(parsed);
            *time_set = true;
        }
    }

    true
}

fn split_fuzz_assignment(arg: &str) -> Option<(&str, &str)> {
    let mut index = 0usize;
    for byte in arg.bytes() {
        if byte == b'=' {
            return Some((&arg[..index], &arg[index + 1..]));
        }
        index += 1;
    }
    None
}

fn fuzz_arg_kind(key: &str) -> Option<FuzzArgKind> {
    let key = strip_fuzz_key_prefix(key);
    match key {
        "seed" => Some(FuzzArgKind::Seed),
        "iter" | "iters" | "iteration" | "iterations" => Some(FuzzArgKind::Iterations),
        "time" | "timeout" | "sec" | "secs" | "second" | "seconds" => {
            Some(FuzzArgKind::TimeSeconds)
        }
        "ms" | "msec" | "millis" | "millisecond" | "milliseconds" => Some(FuzzArgKind::TimeMillis),
        _ => None,
    }
}

fn strip_fuzz_key_prefix(key: &str) -> &str {
    if let Some(stripped) = key.strip_prefix("--") {
        stripped
    } else if let Some(stripped) = key.strip_prefix('-') {
        stripped
    } else {
        key
    }
}

fn millis_to_fuzz_ticks(milliseconds: usize) -> u64 {
    if milliseconds == 0 {
        0
    } else {
        (milliseconds as u64).saturating_add(FUZZ_MILLIS_PER_TICK - 1) / FUZZ_MILLIS_PER_TICK
    }
}

fn print_fuzz_usage(ctx: &mut ShellContext) {
    ctx.serial.write_str("Usage: fuzzsc [seed] [iterations]\n");
    ctx.serial
        .write_str("       fuzzsc seed=<n> iterations=<n> time=<seconds>\n");
    ctx.serial
        .write_str("       fuzzsc iter <n> time <seconds> | ms=<milliseconds>\n");
    ctx.serial
        .write_str("       more than 1000000 iterations requires a time limit\n");
}

fn print_fuzz_error_buckets(
    ctx: &mut ShellContext,
    syscalls: &[u32],
    counts: &[usize],
    count: usize,
) {
    let mut index = 0;
    while index < count && index < syscalls.len() && index < counts.len() {
        if index != 0 {
            ctx.serial.write_str(",");
        }
        print_usize(&mut ctx.serial, syscalls[index] as usize);
        ctx.serial.write_str("x");
        print_usize(&mut ctx.serial, counts[index]);
        index += 1;
    }
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

fn join_args(args: &[&str], start: usize) -> String {
    let mut out = String::new();
    for (index, arg) in args.iter().enumerate().skip(start) {
        if index > start {
            out.push(' ');
        }
        out.push_str(arg);
    }
    out
}

fn print_compat_app_error(
    serial: &mut Serial,
    err: crate::user_level::compat_apps::CompatAppError,
) {
    let text = match err {
        crate::user_level::compat_apps::CompatAppError::FxfsInit => "fxfs init",
        crate::user_level::compat_apps::CompatAppError::FxfsWrite => "fxfs write",
        crate::user_level::compat_apps::CompatAppError::LinuxOpen(_) => "linux openat",
        crate::user_level::compat_apps::CompatAppError::LinuxRead(_) => "linux read",
        crate::user_level::compat_apps::CompatAppError::LinuxWrite(_) => "linux write",
        crate::user_level::compat_apps::CompatAppError::LinuxClose(_) => "linux close",
        crate::user_level::compat_apps::CompatAppError::LinuxReadMismatch => {
            "linux cat read mismatch"
        }
        crate::user_level::compat_apps::CompatAppError::SvcInit => "svc init",
        crate::user_level::compat_apps::CompatAppError::SvcConnect => "svc connect",
        crate::user_level::compat_apps::CompatAppError::SvcCall => "svc call",
        crate::user_level::compat_apps::CompatAppError::SvcReply => "svc reply",
    };
    serial.write_str(text);
    match err {
        crate::user_level::compat_apps::CompatAppError::LinuxOpen(code)
        | crate::user_level::compat_apps::CompatAppError::LinuxRead(code)
        | crate::user_level::compat_apps::CompatAppError::LinuxWrite(code)
        | crate::user_level::compat_apps::CompatAppError::LinuxClose(code) => {
            serial.write_str(" error=");
            print_number(serial, code as u32);
        }
        _ => {}
    }
}

fn run_ported_app_tests(ctx: &mut ShellContext) -> bool {
    ctx.serial
        .write_str("[TEST] Testing ported Linux cat app... ");
    match crate::user_level::compat_apps::run_linux_cat_port() {
        Ok(result) => {
            ctx.serial.write_str("[OK] read ");
            print_number(&mut ctx.serial, result.bytes_read as u32);
            ctx.serial
                .write_str(" bytes through Linux openat/read/write\n");
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_compat_app_error(&mut ctx.serial, err);
            ctx.serial.write_str("\n");
            return false;
        }
    }

    ctx.serial
        .write_str("[TEST] Testing ported Fuchsia /svc app... ");
    match crate::user_level::compat_apps::run_fuchsia_svc_client_port() {
        Ok(result) => {
            ctx.serial.write_str("[OK] requests=");
            print_number(&mut ctx.serial, result.requests as u32);
            ctx.serial.write_str(" replies=");
            print_number(&mut ctx.serial, result.replies as u32);
            ctx.serial.write_str(" fs_nodes=");
            print_number(&mut ctx.serial, result.filesystem_nodes as u32);
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_compat_app_error(&mut ctx.serial, err);
            ctx.serial.write_str("\n");
            return false;
        }
    }

    true
}

fn print_docker_compat_error(
    serial: &mut Serial,
    err: crate::user_level::docker_compat::DockerCompatError,
) {
    let text = match err {
        crate::user_level::docker_compat::DockerCompatError::FxfsInit => "fxfs init",
        crate::user_level::docker_compat::DockerCompatError::FxfsPrepare => "fxfs prepare",
        crate::user_level::docker_compat::DockerCompatError::OciInstall => "oci bundle install",
        crate::user_level::docker_compat::DockerCompatError::OciRead => "oci config read",
        crate::user_level::docker_compat::DockerCompatError::OciParse => "oci config parse",
        crate::user_level::docker_compat::DockerCompatError::RightsConfig(_) => "rights config",
        crate::user_level::docker_compat::DockerCompatError::RuntimeJob(_) => "runtime job",
        crate::user_level::docker_compat::DockerCompatError::RuntimeProcess(_) => "runtime process",
        crate::user_level::docker_compat::DockerCompatError::RuntimeThread(_) => "runtime thread",
        crate::user_level::docker_compat::DockerCompatError::RuntimeStart(_) => "runtime start",
        crate::user_level::docker_compat::DockerCompatError::Namespace(_) => "namespace",
        crate::user_level::docker_compat::DockerCompatError::Mount(_) => "mount",
        crate::user_level::docker_compat::DockerCompatError::PivotRoot(_) => "pivot_root",
        crate::user_level::docker_compat::DockerCompatError::Chroot(_) => "chroot",
        crate::user_level::docker_compat::DockerCompatError::Uts(_) => "uts",
        crate::user_level::docker_compat::DockerCompatError::NoNewPrivs(_) => "no_new_privs",
        crate::user_level::docker_compat::DockerCompatError::Seccomp(_) => "seccomp",
        crate::user_level::docker_compat::DockerCompatError::CapGet(_) => "capget",
        crate::user_level::docker_compat::DockerCompatError::CapSet(_) => "capset",
        crate::user_level::docker_compat::DockerCompatError::CgroupOpen(_) => "cgroup open",
        crate::user_level::docker_compat::DockerCompatError::CgroupWrite(_) => "cgroup write",
        crate::user_level::docker_compat::DockerCompatError::CgroupClose(_) => "cgroup close",
        crate::user_level::docker_compat::DockerCompatError::AppArmorOpen(_) => "apparmor open",
        crate::user_level::docker_compat::DockerCompatError::AppArmorWrite(_) => "apparmor write",
        crate::user_level::docker_compat::DockerCompatError::AppArmorClose(_) => "apparmor close",
        crate::user_level::docker_compat::DockerCompatError::Network(_) => "network",
        crate::user_level::docker_compat::DockerCompatError::ImageNotFound => "image not found",
        crate::user_level::docker_compat::DockerCompatError::ImageInvalid => "invalid image",
        crate::user_level::docker_compat::DockerCompatError::ArchiveInvalid => "invalid archive",
        crate::user_level::docker_compat::DockerCompatError::ArchiveUnsupported => {
            "unsupported archive"
        }
        crate::user_level::docker_compat::DockerCompatError::RegistryUnsupported => {
            "unsupported registry"
        }
        crate::user_level::docker_compat::DockerCompatError::ContainerExists => "container exists",
        crate::user_level::docker_compat::DockerCompatError::ContainerNotFound => {
            "container not found"
        }
        crate::user_level::docker_compat::DockerCompatError::ContainerInvalid => {
            "invalid container"
        }
        crate::user_level::docker_compat::DockerCompatError::ContainerState => {
            "invalid container state"
        }
        crate::user_level::docker_compat::DockerCompatError::StateMismatch => "state mismatch",
    };
    serial.write_str(text);
    match err {
        crate::user_level::docker_compat::DockerCompatError::Namespace(code)
        | crate::user_level::docker_compat::DockerCompatError::Mount(code)
        | crate::user_level::docker_compat::DockerCompatError::PivotRoot(code)
        | crate::user_level::docker_compat::DockerCompatError::Chroot(code)
        | crate::user_level::docker_compat::DockerCompatError::Uts(code)
        | crate::user_level::docker_compat::DockerCompatError::NoNewPrivs(code)
        | crate::user_level::docker_compat::DockerCompatError::Seccomp(code)
        | crate::user_level::docker_compat::DockerCompatError::CapGet(code)
        | crate::user_level::docker_compat::DockerCompatError::CapSet(code)
        | crate::user_level::docker_compat::DockerCompatError::CgroupOpen(code)
        | crate::user_level::docker_compat::DockerCompatError::CgroupWrite(code)
        | crate::user_level::docker_compat::DockerCompatError::CgroupClose(code)
        | crate::user_level::docker_compat::DockerCompatError::AppArmorOpen(code)
        | crate::user_level::docker_compat::DockerCompatError::AppArmorWrite(code)
        | crate::user_level::docker_compat::DockerCompatError::AppArmorClose(code) => {
            serial.write_str(" error=");
            print_number(serial, code as u32);
        }
        crate::user_level::docker_compat::DockerCompatError::RightsConfig(code)
        | crate::user_level::docker_compat::DockerCompatError::RuntimeJob(code)
        | crate::user_level::docker_compat::DockerCompatError::RuntimeProcess(code)
        | crate::user_level::docker_compat::DockerCompatError::RuntimeThread(code)
        | crate::user_level::docker_compat::DockerCompatError::RuntimeStart(code) => {
            serial.write_str(" zx=");
            print_number(serial, (-(code as i32)) as u32);
        }
        crate::user_level::docker_compat::DockerCompatError::Network(code) => {
            serial.write_str(": ");
            let label = match code {
                crate::user_level::net::NetError::Driver(_) => "driver",
                crate::user_level::net::NetError::NotReady => "not ready",
                crate::user_level::net::NetError::InvalidHost => "invalid host",
                crate::user_level::net::NetError::InvalidUrl => "invalid url",
                crate::user_level::net::NetError::BufferTooSmall => "buffer too small",
                crate::user_level::net::NetError::MalformedPacket => "malformed packet",
                crate::user_level::net::NetError::Timeout => "timeout",
                crate::user_level::net::NetError::NoAddress => "no address",
                crate::user_level::net::NetError::Unsupported => "unsupported",
                crate::user_level::net::NetError::ConnectionReset => "connection reset",
                crate::user_level::net::NetError::TlsUnsupported => "tls unsupported",
            };
            serial.write_str(label);
        }
        _ => {}
    }
}

fn run_docker_compat_tests(ctx: &mut ShellContext) -> bool {
    ctx.serial
        .write_str("[TEST] Testing Docker/runc compatibility surfaces... ");
    match crate::user_level::docker_compat::run_docker_runtime_port() {
        Ok(result) => {
            ctx.serial.write_str("[OK] ns=0x");
            print_hex(&mut ctx.serial, result.namespace_flags as u64);
            ctx.serial.write_str(" mounts=");
            print_number(&mut ctx.serial, result.mount_count as u32);
            ctx.serial.write_str(" seccomp=");
            print_number(&mut ctx.serial, result.seccomp_mode as u32);
            ctx.serial.write_str(" filters=");
            print_number(&mut ctx.serial, result.seccomp_filters as u32);
            ctx.serial.write_str(" oci=");
            print_number(&mut ctx.serial, result.oci_config_bytes as u32);
            ctx.serial.write_str("B mounts=");
            print_number(&mut ctx.serial, result.oci_mounts as u32);
            ctx.serial.write_str(" args=");
            print_number(&mut ctx.serial, result.oci_args as u32);
            ctx.serial.write_str(" env=");
            print_number(&mut ctx.serial, result.oci_env as u32);
            ctx.serial.write_str(" masked=");
            print_number(&mut ctx.serial, result.masked_paths as u32);
            ctx.serial.write_str(" ro=");
            print_number(&mut ctx.serial, result.readonly_paths as u32);
            ctx.serial.write_str(" proc=0x");
            print_hex(&mut ctx.serial, result.process_handle as u64);
            ctx.serial.write_str(" thread=0x");
            print_hex(&mut ctx.serial, result.thread_handle as u64);
            ctx.serial.write_str("\n");
            true
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_docker_compat_error(&mut ctx.serial, err);
            ctx.serial.write_str("\n");
            false
        }
    }
}

/// Command: docker - Run local Docker/OCI images
fn cmd_docker(ctx: &mut ShellContext, args: &[&str]) {
    if args.is_empty() {
        print_docker_usage(ctx);
        return;
    }

    match args[0] {
        "images" => match crate::user_level::docker_compat::list_docker_images() {
            Ok(images) => {
                ctx.serial
                    .write_str("REPOSITORY          TAG       LAYERS  CONFIG  ROOTFS\n");
                for image in images {
                    print_docker_image_row(ctx, &image);
                }
                ctx.serial.write_str("\n\n");
            }
            Err(err) => {
                ctx.serial.write_str("docker images failed: ");
                print_docker_compat_error(&mut ctx.serial, err);
                ctx.serial.write_str("\n\n");
            }
        },
        "pull" => {
            if args.len() < 2 {
                ctx.serial
                    .write_str("usage: docker pull <image-or-http-url>\n\n");
                return;
            }
            ctx.serial.write_str("[DOCKER] pull ");
            ctx.serial.write_str(args[1]);
            ctx.serial.write_str("\n");
            match crate::user_level::docker_compat::pull_docker_image(args[1]) {
                Ok(result) => {
                    ctx.serial.write_str("[OK] ");
                    print_docker_load_result(ctx, &result);
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("[FAIL] ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    if matches!(
                        err,
                        crate::user_level::docker_compat::DockerCompatError::Network(
                            crate::user_level::net::NetError::TlsUnsupported
                        )
                    ) {
                        if let Some(path) =
                            crate::user_level::docker_compat::staged_registry_archive_path(args[1])
                        {
                            ctx.serial.write_str(
                                "\nstage with host script, then retry; expected archive: ",
                            );
                            ctx.serial.write_str(path.as_str());
                        }
                    }
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "load" => {
            let Some(input) = docker_load_input_arg(args) else {
                ctx.serial
                    .write_str("usage: docker load [-i|--input] <archive.tar>\n\n");
                return;
            };
            let Some(path) = resolve_docker_load_path(ctx.cwd.as_str(), input) else {
                ctx.serial.write_str("docker load failed: invalid path\n\n");
                return;
            };
            match crate::user_level::docker_compat::load_docker_image(path.as_str()) {
                Ok(result) => {
                    ctx.serial.write_str("[OK] ");
                    print_docker_load_result(ctx, &result);
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker load failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "ps" => {
            let all = args.len() > 1 && args[1] == "-a";
            match crate::user_level::docker_compat::list_docker_containers(all) {
                Ok(containers) => {
                    ctx.serial
                        .write_str("CONTAINER ID  IMAGE               STATUS   COMMAND\n");
                    for container in containers {
                        print_docker_container_row(ctx, &container);
                    }
                    ctx.serial.write_str("\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker ps failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "create" => {
            let Some(parsed) = docker_container_args(args, false) else {
                ctx.serial.write_str(
                    "usage: docker create [--name <name>] [-i] [-t] <image> [command...]\n\n",
                );
                return;
            };
            match crate::user_level::docker_compat::create_docker_container(
                parsed.image,
                parsed.command,
                parsed.name,
            ) {
                Ok(container) => {
                    ctx.serial.write_str(container.id.as_str());
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker create failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "start" => {
            if args.len() < 2 {
                ctx.serial.write_str("usage: docker start <container>\n\n");
                return;
            }
            match crate::user_level::docker_compat::start_docker_container(args[1]) {
                Ok(result) => {
                    ctx.serial.write_str(result.container.id.as_str());
                    ctx.serial.write_str(" proc=0x");
                    print_hex(&mut ctx.serial, result.runtime.process_handle as u64);
                    ctx.serial.write_str(" thread=0x");
                    print_hex(&mut ctx.serial, result.runtime.thread_handle as u64);
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker start failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "run" => {
            let Some(parsed) = docker_container_args(args, true) else {
                ctx.serial.write_str(
                    "usage: docker run [-i] [-t] [--rm] [--name <name>] <image> [command...]\n\n",
                );
                return;
            };
            ctx.serial.write_str("[DOCKER] run ");
            ctx.serial.write_str(parsed.image);
            ctx.serial.write_str("\n");
            if parsed.interactive && parsed.tty {
                match run_interactive_docker_container(ctx, &parsed) {
                    Ok(()) => {}
                    Err(err) => {
                        ctx.serial.write_str("[FAIL] ");
                        print_docker_compat_error(&mut ctx.serial, err);
                        ctx.serial.write_str("\n\n");
                    }
                }
                return;
            }
            match crate::user_level::docker_compat::run_docker_image_named(
                parsed.image,
                parsed.command,
                parsed.name,
            ) {
                Ok(result) => {
                    ctx.serial.write_str("[OK] id=");
                    ctx.serial.write_str(result.container.id.as_str());
                    ctx.serial.write_str(" image=");
                    ctx.serial.write_str(result.container.image.as_str());
                    ctx.serial.write_str(" proc=0x");
                    print_hex(&mut ctx.serial, result.runtime.process_handle as u64);
                    ctx.serial.write_str(" thread=0x");
                    print_hex(&mut ctx.serial, result.runtime.thread_handle as u64);
                    ctx.serial.write_str(" ns=0x");
                    print_hex(&mut ctx.serial, result.runtime.namespace_flags as u64);
                    ctx.serial.write_str(" mounts=");
                    print_number(&mut ctx.serial, result.runtime.mount_count as u32);
                    ctx.serial.write_str(" seccomp=");
                    print_number(&mut ctx.serial, result.runtime.seccomp_mode as u32);
                    ctx.serial.write_str(" status=");
                    ctx.serial.write_str(result.container.status.as_str());
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("[FAIL] ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "inspect" => {
            if args.len() < 2 {
                ctx.serial
                    .write_str("usage: docker inspect <container>\n\n");
                return;
            }
            match crate::user_level::docker_compat::inspect_docker_container(args[1]) {
                Ok(container) => {
                    print_docker_container_detail(ctx, &container);
                    ctx.serial.write_str("\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker inspect failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "logs" => {
            if args.len() < 2 {
                ctx.serial.write_str("usage: docker logs <container>\n\n");
                return;
            }
            let mut log = [0u8; 1024];
            match crate::user_level::docker_compat::docker_container_logs(args[1], &mut log) {
                Ok(len) => {
                    if len == 0 {
                        ctx.serial.write_str("\n");
                    } else if let Ok(text) = core::str::from_utf8(&log[..len]) {
                        ctx.serial.write_str(text);
                        if !text.ends_with('\n') {
                            ctx.serial.write_str("\n");
                        }
                    } else {
                        ctx.serial.write_str("docker logs: non-utf8 log\n");
                    }
                    ctx.serial.write_str("\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker logs failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "stop" => {
            if args.len() < 2 {
                ctx.serial.write_str("usage: docker stop <container>\n\n");
                return;
            }
            match crate::user_level::docker_compat::stop_docker_container(args[1]) {
                Ok(container) => {
                    ctx.serial.write_str(container.id.as_str());
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker stop failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        "rm" => {
            if args.len() < 2 {
                ctx.serial.write_str("usage: docker rm <container>\n\n");
                return;
            }
            match crate::user_level::docker_compat::remove_docker_container(args[1]) {
                Ok(()) => {
                    ctx.serial.write_str(args[1]);
                    ctx.serial.write_str("\n\n");
                }
                Err(err) => {
                    ctx.serial.write_str("docker rm failed: ");
                    print_docker_compat_error(&mut ctx.serial, err);
                    ctx.serial.write_str("\n\n");
                }
            }
        }
        _ => {
            print_docker_usage(ctx);
        }
    }
}

/// Command: hermes - Run the native Hermes agent compatibility port
fn cmd_hermes(ctx: &mut ShellContext, args: &[&str]) {
    if args.is_empty() {
        print_hermes_usage(ctx);
        return;
    }

    match args[0] {
        "info" | "status" => match crate::user_level::hermes_agent::info() {
            Ok(info) => print_hermes_info(ctx, &info),
            Err(err) => {
                ctx.serial.write_str("hermes: ");
                ctx.serial.write_str(err.as_str());
                ctx.serial.write_str("\n");
            }
        },
        "test" | "smoke" => {
            ctx.serial.write_str("\n=== Hermes Agent Port Test ===\n\n");
            let _ = run_hermes_agent_tests(ctx);
            ctx.serial.write_str("\n");
        }
        "skills" => match crate::user_level::hermes_agent::list_skills() {
            Ok(skills) => {
                ctx.serial.write_str("Hermes skills\n");
                for skill in skills {
                    ctx.serial.write_str("  ");
                    ctx.serial.write_str(skill.slug);
                    ctx.serial.write_str(" - ");
                    ctx.serial.write_str(skill.description);
                    ctx.serial.write_str("\n    ");
                    ctx.serial.write_str(skill.path);
                    ctx.serial.write_str("\n");
                }
            }
            Err(err) => {
                ctx.serial.write_str("hermes: ");
                ctx.serial.write_str(err.as_str());
                ctx.serial.write_str("\n");
            }
        },
        "web" | "ui" => {
            if args[0] == "ui" && args.len() == 1 {
                run_hermes_ui_entry(ctx);
            } else if args.get(1).copied() == Some("source") || args.get(1).copied() == Some("html")
            {
                match crate::user_level::hermes_agent::render_web_ui() {
                    Ok(web) => {
                        ctx.serial.write_str(web.html.as_str());
                        ctx.serial.write_str("\n");
                    }
                    Err(err) => {
                        ctx.serial.write_str("hermes: ");
                        ctx.serial.write_str(err.as_str());
                        ctx.serial.write_str("\n");
                    }
                }
            } else if args.get(1).copied() == Some("text") {
                match crate::user_level::hermes_agent::render_native_ui(78) {
                    Ok(view) => {
                        ctx.serial.write_str(view.rendered.as_str());
                        ctx.serial.write_str("source=");
                        ctx.serial.write_str(view.source_path);
                        ctx.serial.write_str(" widgets=");
                        print_usize(&mut ctx.serial, view.widgets);
                        ctx.serial.write_str(" width=");
                        print_usize(&mut ctx.serial, view.width);
                        ctx.serial.write_str("\n");
                    }
                    Err(err) => {
                        ctx.serial.write_str("hermes: ");
                        ctx.serial.write_str(err.as_str());
                        ctx.serial.write_str("\n");
                    }
                }
            } else {
                match crate::user_level::hermes_agent::render_cpu_ui() {
                    Ok(view) => {
                        ctx.serial.write_str(view.preview.as_str());
                        ctx.serial.write_str("\nimage=");
                        ctx.serial.write_str(view.image_path);
                        ctx.serial.write_str(" source=");
                        ctx.serial.write_str(view.source_path);
                        ctx.serial.write_str(" size=");
                        print_usize(&mut ctx.serial, view.width);
                        ctx.serial.write_str("x");
                        print_usize(&mut ctx.serial, view.height);
                        ctx.serial.write_str(" bytes=");
                        print_usize(&mut ctx.serial, view.image_bytes);
                        ctx.serial.write_str(" widgets=");
                        print_usize(&mut ctx.serial, view.widgets);
                        ctx.serial.write_str("\n");
                    }
                    Err(err) => {
                        ctx.serial.write_str("hermes: ");
                        ctx.serial.write_str(err.as_str());
                        ctx.serial.write_str("\n");
                    }
                }
            }
        }
        "ask" | "run" => {
            if args.len() < 2 {
                ctx.serial.write_str("usage: hermes ask <prompt>\n");
                return;
            }
            let prompt = join_args(args, 1);
            match crate::user_level::hermes_agent::run_prompt(prompt.as_str()) {
                Ok(turn) => {
                    ctx.serial.write_str("Hermes: ");
                    ctx.serial.write_str(turn.answer.as_str());
                    ctx.serial.write_str("\n  tools=");
                    print_usize(&mut ctx.serial, turn.tool_calls);
                    ctx.serial.write_str(" skills=");
                    print_usize(&mut ctx.serial, turn.skill_hits);
                    ctx.serial.write_str(" [");
                    ctx.serial.write_str(turn.skill_summary.as_str());
                    ctx.serial.write_str("]");
                    ctx.serial.write_str(" delegates=");
                    print_usize(&mut ctx.serial, turn.delegated_agents);
                    ctx.serial.write_str(" memory=");
                    print_usize(&mut ctx.serial, turn.memory_writes);
                    ctx.serial.write_str(" transcript=");
                    print_usize(&mut ctx.serial, turn.transcript_bytes);
                    ctx.serial.write_str("B\n");
                }
                Err(err) => {
                    ctx.serial.write_str("hermes: ");
                    ctx.serial.write_str(err.as_str());
                    ctx.serial.write_str("\n");
                }
            }
        }
        _ => print_hermes_usage(ctx),
    }
}

/// Command: lvgl - Render the SMROS LVGL compatibility UI
fn cmd_lvgl(ctx: &mut ShellContext, args: &[&str]) {
    if args.is_empty() {
        match crate::user_level::lvgl::render_demo() {
            Ok(render) => print_lvgl_render(ctx, &render),
            Err(err) => {
                ctx.serial.write_str("lvgl: ");
                ctx.serial.write_str(err.as_str());
                ctx.serial.write_str("\n");
            }
        }
        return;
    }

    match args[0] {
        "info" | "status" => print_lvgl_info(ctx, &crate::user_level::lvgl::info()),
        "render" | "show" | "demo" => match crate::user_level::lvgl::render_demo() {
            Ok(render) => print_lvgl_render(ctx, &render),
            Err(err) => {
                ctx.serial.write_str("lvgl: ");
                ctx.serial.write_str(err.as_str());
                ctx.serial.write_str("\n");
            }
        },
        "sched" | "schedule" | "trace" => {
            match crate::user_level::lvgl::render_scheduler_trace(96) {
                Ok(render) => print_lvgl_sched_trace(ctx, &render),
                Err(err) => {
                    ctx.serial.write_str("lvgl sched: ");
                    ctx.serial.write_str(err.as_str());
                    ctx.serial.write_str("\n");
                }
            }
        }
        "test" | "smoke" => {
            ctx.serial.write_str("\n=== SMROS LVGL Port Test ===\n\n");
            let _ = run_lvgl_tests(ctx);
            ctx.serial.write_str("\n");
        }
        _ => print_lvgl_usage(ctx),
    }
}

fn print_lvgl_usage(ctx: &mut ShellContext) {
    ctx.serial
        .write_str("usage: lvgl [info|render|sched|test]\n");
}

fn print_lvgl_info(ctx: &mut ShellContext, info: &crate::user_level::lvgl::LvglPortInfo) {
    ctx.serial.write_str("SMROS LVGL port\n");
    ctx.serial.write_str("  port: ");
    ctx.serial.write_str(info.name);
    ctx.serial.write_str(" ");
    ctx.serial.write_str(info.compat_version);
    ctx.serial.write_str("\n  display: ");
    ctx.serial.write_str(info.display_backend);
    ctx.serial.write_str(" input=");
    ctx.serial.write_str(info.input_backend);
    ctx.serial.write_str(" tick=");
    ctx.serial.write_str(info.tick_backend);
    ctx.serial.write_str("\n  draw_buffer=");
    print_usize(&mut ctx.serial, info.draw_buffer_bytes);
    ctx.serial.write_str("B widgets=");
    print_usize(&mut ctx.serial, info.widgets);
    ctx.serial.write_str(" demo=");
    ctx.serial
        .write_str(crate::user_level::lvgl::LVGL_DEMO_PPM_PATH);
    ctx.serial.write_str("\n");
}

fn print_lvgl_render(ctx: &mut ShellContext, render: &crate::user_level::lvgl::LvglDemoRender) {
    ctx.serial.write_str(render.preview.as_str());
    ctx.serial.write_str("\nimage=");
    ctx.serial.write_str(render.image_path);
    ctx.serial.write_str(" size=");
    print_usize(&mut ctx.serial, render.width);
    ctx.serial.write_str("x");
    print_usize(&mut ctx.serial, render.height);
    ctx.serial.write_str(" bytes=");
    print_usize(&mut ctx.serial, render.image_bytes);
    ctx.serial.write_str(" widgets=");
    print_usize(&mut ctx.serial, render.widgets);
    ctx.serial.write_str("\n");
}

fn print_lvgl_sched_trace(
    ctx: &mut ShellContext,
    render: &crate::user_level::lvgl::LvglSchedulerTraceRender,
) {
    ctx.serial.write_str(render.preview.as_str());
    ctx.serial.write_str("\nimage=");
    ctx.serial.write_str(render.image_path);
    ctx.serial.write_str(" size=");
    print_usize(&mut ctx.serial, render.width);
    ctx.serial.write_str("x");
    print_usize(&mut ctx.serial, render.height);
    ctx.serial.write_str(" bytes=");
    print_usize(&mut ctx.serial, render.image_bytes);
    ctx.serial.write_str(" samples=");
    print_usize(&mut ctx.serial, render.samples);
    ctx.serial.write_str(" cpus=");
    print_usize(&mut ctx.serial, render.cpu_rows);
    ctx.serial.write_str(" threads=");
    print_usize(&mut ctx.serial, render.thread_count);
    ctx.serial.write_str("\n");
}

fn print_perfetto_sched_trace(
    ctx: &mut ShellContext,
    export: &crate::user_level::perfetto::PerfettoSchedulerTraceExport,
) {
    ctx.serial
        .write_str("scheduler trace exported for Perfetto\n");
    ctx.serial.write_str("  file: ");
    ctx.serial.write_str(export.path);
    ctx.serial.write_str("\n  format: ");
    ctx.serial.write_str(export.format);
    ctx.serial.write_str("\n  policy: ");
    ctx.serial.write_str(export.policy);
    ctx.serial.write_str("  samples=");
    print_usize(&mut ctx.serial, export.samples);
    ctx.serial.write_str("  slices=");
    print_usize(&mut ctx.serial, export.slices);
    ctx.serial.write_str("\n  cpu tracks=");
    print_usize(&mut ctx.serial, export.cpu_tracks);
    ctx.serial.write_str("  threads=");
    print_usize(&mut ctx.serial, export.thread_count);
    ctx.serial.write_str("  vm tracks=");
    print_usize(&mut ctx.serial, export.vm_tracks);
    ctx.serial.write_str("  tick_us=");
    print_usize(&mut ctx.serial, export.tick_us as usize);
    ctx.serial.write_str("\n  tick range: ");
    print_usize(&mut ctx.serial, export.start_tick as usize);
    ctx.serial.write_str("..");
    print_usize(&mut ctx.serial, export.end_tick as usize);
    ctx.serial.write_str("  bytes=");
    print_usize(&mut ctx.serial, export.bytes);
    ctx.serial.write_str("\n  host_shared sync: ");
    ctx.serial
        .write_str(if export.host_synced { "ok" } else { "pending" });
    if export.host_synced {
        ctx.serial
            .write_str("\nopen host_shared/trace.pftrace in https://ui.perfetto.dev\n");
    } else {
        ctx.serial.write_str(
            "\ntrace is written inside /shared; run scripts/sync-host-shared.py smros-fxfs.img host_shared after QEMU exits before opening host_shared/trace.pftrace\n",
        );
    }
}

fn run_gemma_tests(ctx: &mut ShellContext) -> bool {
    ctx.serial
        .write_str("[TEST] Testing Gemma model service... ");
    match crate::user_level::gemma::run_full_test() {
        Ok(report) if report.passed() => {
            ctx.serial.write_str("[OK] manifest=");
            ctx.serial
                .write_str(if report.manifest_ok { "yes" } else { "no" });
            ctx.serial.write_str(" prompt=");
            ctx.serial
                .write_str(if report.prompt_ok { "yes" } else { "no" });
            ctx.serial.write_str(" generation=");
            ctx.serial
                .write_str(if report.generation_ok { "yes" } else { "no" });
            ctx.serial.write_str(" backend=");
            ctx.serial.write_str(report.generation.backend);
            ctx.serial.write_str(" tokens=");
            print_usize(&mut ctx.serial, report.generation.generated_tokens);
            ctx.serial.write_str("\n");
            true
        }
        Ok(report) => {
            ctx.serial.write_str("[FAIL] incomplete report tokens=");
            print_usize(&mut ctx.serial, report.generation.generated_tokens);
            ctx.serial.write_str("\n");
            false
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            ctx.serial.write_str(err.as_str());
            ctx.serial.write_str("\n");
            false
        }
    }
}

fn print_hermes_usage(ctx: &mut ShellContext) {
    ctx.serial
        .write_str("usage: hermes <info|test|skills|web|ui|ask>\n");
    ctx.serial.write_str("       hermes ask <prompt>\n");
    ctx.serial
        .write_str("       hermes ui  # LVGL-styled keyboard/mouse UI\n");
    ctx.serial.write_str("       hermes web [text|source]\n");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HermesUiFocus {
    Prompt,
    Send,
    Clear,
    Load,
    Test,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HermesUiKey {
    Char(u8),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    BackTab,
    Esc,
    CtrlC,
    CtrlL,
    CtrlN,
    CtrlU,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HermesMouseButton {
    Left,
    WheelUp,
    WheelDown,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HermesMouseEvent {
    x: usize,
    y: usize,
    button: HermesMouseButton,
    pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HermesUiEvent {
    Key(HermesUiKey),
    Mouse(HermesMouseEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HermesUiCommand {
    Continue,
    Submit,
    RunTest,
    Exit,
}

struct HermesUiState {
    prompt: String,
    prompt_cursor: usize,
    prompt_scroll: usize,
    last_answer: String,
    metrics: String,
    runtime: String,
    activity: String,
    status: String,
    focus: HermesUiFocus,
    response_scroll: usize,
    active_preset: usize,
    dirty: HermesUiDirty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HermesUiDirty {
    layout: bool,
    header: bool,
    prompt: bool,
    actions: bool,
    response: bool,
    runtime: bool,
    activity: bool,
    presets: bool,
    status: bool,
}

const HERMES_UI_WIDTH: usize = 80;
const HERMES_UI_HEIGHT: usize = 30;
const HERMES_UI_PROMPT_MAX: usize = 320;
const HERMES_UI_LEFT_COL: usize = 2;
const HERMES_UI_LEFT_WIDTH: usize = 53;
const HERMES_UI_RIGHT_COL: usize = 56;
const HERMES_UI_RIGHT_WIDTH: usize = 23;
const HERMES_UI_PROMPT_BOX_ROW: usize = 4;
const HERMES_UI_PROMPT_BOX_HEIGHT: usize = 7;
const HERMES_UI_PROMPT_COL: usize = 4;
const HERMES_UI_PROMPT_ROW: usize = 7;
const HERMES_UI_PROMPT_WIDTH: usize = 49;
const HERMES_UI_RUNTIME_ROW: usize = 4;
const HERMES_UI_RUNTIME_HEIGHT: usize = 8;
const HERMES_UI_RESPONSE_BOX_ROW: usize = 13;
const HERMES_UI_RESPONSE_BOX_HEIGHT: usize = 12;
const HERMES_UI_RESPONSE_COL: usize = 4;
const HERMES_UI_RESPONSE_ROW: usize = 15;
const HERMES_UI_RESPONSE_WIDTH: usize = 49;
const HERMES_UI_RESPONSE_ROWS: usize = 9;
const HERMES_UI_ACTIVITY_ROW: usize = 13;
const HERMES_UI_ACTIVITY_HEIGHT: usize = 12;
const HERMES_UI_PRESET_ROW: usize = 26;
const HERMES_UI_PRESET_HEIGHT: usize = 3;
const HERMES_UI_SEND_COL: usize = 4;
const HERMES_UI_CLEAR_COL: usize = 14;
const HERMES_UI_LOAD_COL: usize = 24;
const HERMES_UI_TEST_COL: usize = 34;
const HERMES_UI_EXIT_COL: usize = 44;
const HERMES_UI_BUTTON_ROW: usize = 11;
const HERMES_UI_BUTTON_WIDTH: usize = 9;
const HERMES_UI_STATUS_ROW: usize = 30;
const HERMES_UI_PRESETS: &[&str] = &[
    "test hermes web ui on SMROS with memory and skills",
    "summarize FxFS, /svc, and Gemma state",
    "plan a Hermes smoke test for network and Docker",
    "review recent memory and transcript context",
];

impl HermesUiDirty {
    fn all() -> Self {
        Self {
            layout: true,
            header: true,
            prompt: true,
            actions: true,
            response: true,
            runtime: true,
            activity: true,
            presets: true,
            status: true,
        }
    }

    fn clear(&mut self) {
        *self = Self {
            layout: false,
            header: false,
            prompt: false,
            actions: false,
            response: false,
            runtime: false,
            activity: false,
            presets: false,
            status: false,
        };
    }
}

impl HermesUiState {
    fn new() -> Self {
        let prompt = String::from(HERMES_UI_PRESETS[0]);
        let prompt_cursor = prompt.len();
        let mut state = Self {
            prompt,
            prompt_cursor,
            prompt_scroll: 0,
            last_answer: String::from(
                "Ready. Compose a prompt, send it, and Hermes will update this response panel.",
            ),
            metrics: String::new(),
            runtime: String::new(),
            activity: String::from("No turn submitted yet."),
            status: String::from("Ready"),
            focus: HermesUiFocus::Prompt,
            response_scroll: 0,
            active_preset: 0,
            dirty: HermesUiDirty::all(),
        };
        state.refresh_metrics();
        state.sync_prompt_scroll();
        state
    }

    fn refresh_metrics(&mut self) {
        self.metrics.clear();
        self.runtime.clear();
        match crate::user_level::hermes_agent::info() {
            Ok(info) => {
                self.metrics.push_str(info.provider);
                self.metrics.push_str(" | ");
                self.metrics.push_str(info.model);
                self.metrics.push_str(" | tools ");
                append_usize_shell(&mut self.metrics, info.tools);
                self.metrics.push_str(" | skills ");
                append_usize_shell(&mut self.metrics, info.skills);
                self.metrics.push_str(" | memory ");
                append_usize_shell(&mut self.metrics, info.memory_items);
                self.metrics.push_str(" | transcripts ");
                append_usize_shell(&mut self.metrics, info.transcripts);

                self.runtime.push_str("provider: ");
                self.runtime.push_str(info.provider);
                self.runtime.push_str("\nmodel: ");
                self.runtime.push_str(info.model);
                self.runtime.push_str("\nbackend: ");
                self.runtime.push_str(info.generation_backend);
                self.runtime.push_str("\ntools: ");
                append_usize_shell(&mut self.runtime, info.tools);
                self.runtime.push_str("  skills: ");
                append_usize_shell(&mut self.runtime, info.skills);
                self.runtime.push_str("\nmemory: ");
                append_usize_shell(&mut self.runtime, info.memory_items);
                self.runtime.push_str("  sessions: ");
                append_usize_shell(&mut self.runtime, info.transcripts);
                self.runtime.push_str("\nui: ");
                self.runtime.push_str(info.web_ui_path);
                let lvgl = crate::user_level::lvgl::info();
                self.runtime.push_str("\nlvgl: ");
                self.runtime.push_str(lvgl.name);
                self.runtime.push_str("\ndisplay: ");
                self.runtime.push_str(lvgl.display_backend);
                self.runtime.push_str("\ninput: ");
                self.runtime.push_str(lvgl.input_backend);
            }
            Err(err) => {
                self.metrics.push_str("Hermes status unavailable: ");
                self.metrics.push_str(err.as_str());
                self.runtime.push_str("status unavailable\n");
                self.runtime.push_str(err.as_str());
            }
        }
        self.dirty.header = true;
        self.dirty.runtime = true;
    }

    fn sync_prompt_scroll(&mut self) {
        if self.prompt_cursor < self.prompt_scroll {
            self.prompt_scroll = self.prompt_cursor;
        }
        if self.prompt_cursor >= self.prompt_scroll.saturating_add(HERMES_UI_PROMPT_WIDTH) {
            self.prompt_scroll = self
                .prompt_cursor
                .saturating_sub(HERMES_UI_PROMPT_WIDTH.saturating_sub(1));
        }
    }

    fn insert_byte(&mut self, byte: u8) {
        if self.prompt.len() >= HERMES_UI_PROMPT_MAX {
            self.status = String::from("Prompt is full");
            self.dirty.status = true;
            return;
        }
        self.prompt.insert(self.prompt_cursor, byte as char);
        self.prompt_cursor += 1;
        self.status = String::from("Editing prompt");
        self.sync_prompt_scroll();
        self.dirty.prompt = true;
        self.dirty.status = true;
    }

    fn delete_before_cursor(&mut self) {
        if self.prompt_cursor == 0 {
            return;
        }
        self.prompt_cursor -= 1;
        self.prompt.remove(self.prompt_cursor);
        self.status = String::from("Editing prompt");
        self.sync_prompt_scroll();
        self.dirty.prompt = true;
        self.dirty.status = true;
    }

    fn delete_at_cursor(&mut self) {
        if self.prompt_cursor >= self.prompt.len() {
            return;
        }
        self.prompt.remove(self.prompt_cursor);
        self.status = String::from("Editing prompt");
        self.sync_prompt_scroll();
        self.dirty.prompt = true;
        self.dirty.status = true;
    }

    fn move_prompt_left(&mut self) {
        if self.prompt_cursor > 0 {
            self.prompt_cursor -= 1;
            self.sync_prompt_scroll();
            self.dirty.prompt = true;
        }
    }

    fn move_prompt_right(&mut self) {
        if self.prompt_cursor < self.prompt.len() {
            self.prompt_cursor += 1;
            self.sync_prompt_scroll();
            self.dirty.prompt = true;
        }
    }

    fn set_prompt_cursor_from_screen(&mut self, x: usize) {
        let offset = x.saturating_sub(HERMES_UI_PROMPT_COL);
        self.prompt_cursor = self
            .prompt_scroll
            .saturating_add(offset)
            .min(self.prompt.len());
        self.sync_prompt_scroll();
        self.dirty.prompt = true;
    }

    fn clear_prompt(&mut self) {
        self.prompt.clear();
        self.prompt_cursor = 0;
        self.prompt_scroll = 0;
        self.response_scroll = 0;
        self.last_answer = String::from("Prompt cleared.");
        self.activity = String::from("Composer cleared.");
        self.status = String::from("Cleared");
        self.focus = HermesUiFocus::Prompt;
        self.dirty.prompt = true;
        self.dirty.actions = true;
        self.dirty.response = true;
        self.dirty.activity = true;
        self.dirty.status = true;
    }

    fn focus_next(&mut self) {
        self.focus = match self.focus {
            HermesUiFocus::Prompt => HermesUiFocus::Send,
            HermesUiFocus::Send => HermesUiFocus::Clear,
            HermesUiFocus::Clear => HermesUiFocus::Load,
            HermesUiFocus::Load => HermesUiFocus::Test,
            HermesUiFocus::Test => HermesUiFocus::Exit,
            HermesUiFocus::Exit => HermesUiFocus::Prompt,
        };
        self.dirty.prompt = true;
        self.dirty.actions = true;
        self.dirty.status = true;
    }

    fn focus_previous(&mut self) {
        self.focus = match self.focus {
            HermesUiFocus::Prompt => HermesUiFocus::Exit,
            HermesUiFocus::Send => HermesUiFocus::Prompt,
            HermesUiFocus::Clear => HermesUiFocus::Send,
            HermesUiFocus::Load => HermesUiFocus::Clear,
            HermesUiFocus::Test => HermesUiFocus::Load,
            HermesUiFocus::Exit => HermesUiFocus::Test,
        };
        self.dirty.prompt = true;
        self.dirty.actions = true;
        self.dirty.status = true;
    }

    fn load_active_preset(&mut self) {
        self.prompt.clear();
        self.prompt.push_str(HERMES_UI_PRESETS[self.active_preset]);
        self.prompt_cursor = self.prompt.len();
        self.prompt_scroll = 0;
        self.focus = HermesUiFocus::Prompt;
        self.status = String::from("Preset loaded");
        self.sync_prompt_scroll();
        self.dirty.prompt = true;
        self.dirty.actions = true;
        self.dirty.presets = true;
        self.dirty.status = true;
    }

    fn next_preset(&mut self) {
        self.active_preset = (self.active_preset + 1) % HERMES_UI_PRESETS.len();
        self.status = String::from("Preset selected");
        self.dirty.presets = true;
        self.dirty.status = true;
    }

    fn previous_preset(&mut self) {
        if self.active_preset == 0 {
            self.active_preset = HERMES_UI_PRESETS.len().saturating_sub(1);
        } else {
            self.active_preset -= 1;
        }
        self.status = String::from("Preset selected");
        self.dirty.presets = true;
        self.dirty.status = true;
    }

    fn scroll_response_up(&mut self, amount: usize) {
        self.response_scroll = self.response_scroll.saturating_sub(amount);
        self.dirty.response = true;
        self.dirty.status = true;
    }

    fn scroll_response_down(&mut self, amount: usize) {
        self.response_scroll = self.response_scroll.saturating_add(amount);
        self.dirty.response = true;
        self.dirty.status = true;
    }

    fn force_redraw(&mut self) {
        self.status = String::from("Redrawn");
        self.dirty = HermesUiDirty::all();
    }
}

fn run_hermes_ui_entry(ctx: &mut ShellContext) {
    let mut state = HermesUiState::new();
    hermes_ui_enter(ctx);

    loop {
        hermes_ui_render(ctx, &mut state);
        let event = hermes_ui_read_event();
        match hermes_ui_handle_event(&mut state, event) {
            HermesUiCommand::Continue => {}
            HermesUiCommand::Submit => hermes_ui_submit(ctx, &mut state),
            HermesUiCommand::RunTest => hermes_ui_run_test(ctx, &mut state),
            HermesUiCommand::Exit => break,
        }
    }

    hermes_ui_leave(ctx);
    ctx.serial.write_str("leaving Hermes UI\n");
}

fn hermes_ui_enter(ctx: &mut ShellContext) {
    ctx.serial
        .write_str("\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[2J\x1b[H");
}

fn hermes_ui_leave(ctx: &mut ShellContext) {
    ctx.serial
        .write_str("\x1b[0m\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l");
}

fn hermes_ui_submit(ctx: &mut ShellContext, state: &mut HermesUiState) {
    let prompt = String::from(trim_ascii_shell(state.prompt.as_str()));
    if prompt.is_empty() {
        state.last_answer = String::from("Prompt is empty.");
        state.status = String::from("Nothing to send");
        state.dirty.response = true;
        state.dirty.status = true;
        return;
    }

    state.status = String::from("Submitting...");
    state.activity =
        String::from("Running tools, skills, delegation, Gemma, and transcript write.");
    state.dirty.response = true;
    state.dirty.activity = true;
    state.dirty.status = true;
    hermes_ui_render(ctx, state);
    match crate::user_level::hermes_agent::run_prompt(prompt.as_str()) {
        Ok(turn) => {
            state.last_answer.clear();
            state.last_answer.push_str(turn.answer.as_str());
            state.last_answer.push_str("\nmetrics: tools=");
            append_usize_shell(&mut state.last_answer, turn.tool_calls);
            state.last_answer.push_str(" skills=");
            append_usize_shell(&mut state.last_answer, turn.skill_hits);
            state.last_answer.push_str(" delegates=");
            append_usize_shell(&mut state.last_answer, turn.delegated_agents);
            state.last_answer.push_str(" transcript=");
            append_usize_shell(&mut state.last_answer, turn.transcript_bytes);
            state.last_answer.push('B');
            state.activity.clear();
            state.activity.push_str("prompt bytes=");
            append_usize_shell(&mut state.activity, turn.prompt.len());
            state.activity.push_str("\ntools=");
            append_usize_shell(&mut state.activity, turn.tool_calls);
            state.activity.push_str(" skills=");
            append_usize_shell(&mut state.activity, turn.skill_hits);
            state.activity.push_str("\ndelegates=");
            append_usize_shell(&mut state.activity, turn.delegated_agents);
            state.activity.push_str(" tokens=");
            append_usize_shell(&mut state.activity, turn.model_tokens);
            state.activity.push_str("\ntranscript=");
            append_usize_shell(&mut state.activity, turn.transcript_bytes);
            state.activity.push_str("B\nskills: ");
            state.activity.push_str(turn.skill_summary.as_str());
            state.status = String::from("Response updated");
            state.response_scroll = 0;
            state.refresh_metrics();
            state.dirty.response = true;
            state.dirty.activity = true;
            state.dirty.status = true;
        }
        Err(err) => {
            state.last_answer.clear();
            state.last_answer.push_str("Hermes error: ");
            state.last_answer.push_str(err.as_str());
            state.activity.clear();
            state.activity.push_str("Last run failed: ");
            state.activity.push_str(err.as_str());
            state.status = String::from("Hermes error");
            state.response_scroll = 0;
            state.dirty.response = true;
            state.dirty.activity = true;
            state.dirty.status = true;
        }
    }
}

fn hermes_ui_run_test(ctx: &mut ShellContext, state: &mut HermesUiState) {
    state.status = String::from("Running smoke test...");
    state.activity = String::from("Executing hermes test path.");
    state.dirty.activity = true;
    state.dirty.status = true;
    hermes_ui_render(ctx, state);

    match crate::user_level::hermes_agent::run_full_test() {
        Ok(report) if report.passed() => {
            state.last_answer = String::from(
                "Hermes smoke test passed. Config, model route, skills, memory, tools, delegation, Gemma, cron, transcript, /svc, and web UI are healthy.",
            );
            state.activity.clear();
            state.activity.push_str("test: pass\ntools=");
            append_usize_shell(&mut state.activity, report.turn.tool_calls);
            state.activity.push_str(" delegates=");
            append_usize_shell(&mut state.activity, report.turn.delegated_agents);
            state.activity.push_str("\ntokens=");
            append_usize_shell(&mut state.activity, report.turn.model_tokens);
            state.activity.push_str(" transcript=");
            append_usize_shell(&mut state.activity, report.turn.transcript_bytes);
            state.activity.push('B');
            state.status = String::from("Smoke test passed");
            state.response_scroll = 0;
            state.refresh_metrics();
        }
        Ok(report) => {
            state.last_answer = String::from("Hermes smoke test returned an incomplete report.");
            state.activity.clear();
            state.activity.push_str("test: incomplete\ntools=");
            append_usize_shell(&mut state.activity, report.turn.tool_calls);
            state.activity.push_str(" tokens=");
            append_usize_shell(&mut state.activity, report.turn.model_tokens);
            state.status = String::from("Smoke test incomplete");
            state.response_scroll = 0;
        }
        Err(err) => {
            state.last_answer.clear();
            state.last_answer.push_str("Hermes smoke test failed: ");
            state.last_answer.push_str(err.as_str());
            state.activity.clear();
            state.activity.push_str("test: failed\n");
            state.activity.push_str(err.as_str());
            state.status = String::from("Smoke test failed");
            state.response_scroll = 0;
        }
    }
    state.dirty.response = true;
    state.dirty.activity = true;
    state.dirty.status = true;
}

fn hermes_ui_handle_event(state: &mut HermesUiState, event: HermesUiEvent) -> HermesUiCommand {
    match event {
        HermesUiEvent::Key(key) => hermes_ui_handle_key(state, key),
        HermesUiEvent::Mouse(mouse) => hermes_ui_handle_mouse(state, mouse),
    }
}

fn hermes_ui_handle_key(state: &mut HermesUiState, key: HermesUiKey) -> HermesUiCommand {
    match key {
        HermesUiKey::Esc | HermesUiKey::CtrlC => return HermesUiCommand::Exit,
        HermesUiKey::Enter => match state.focus {
            HermesUiFocus::Prompt | HermesUiFocus::Send => return HermesUiCommand::Submit,
            HermesUiFocus::Clear => state.clear_prompt(),
            HermesUiFocus::Load => state.load_active_preset(),
            HermesUiFocus::Test => return HermesUiCommand::RunTest,
            HermesUiFocus::Exit => return HermesUiCommand::Exit,
        },
        HermesUiKey::Tab => state.focus_next(),
        HermesUiKey::BackTab => state.focus_previous(),
        HermesUiKey::Left => {
            if state.focus == HermesUiFocus::Prompt {
                state.move_prompt_left();
            } else {
                state.focus_previous();
            }
        }
        HermesUiKey::Right => {
            if state.focus == HermesUiFocus::Prompt {
                state.move_prompt_right();
            } else {
                state.focus_next();
            }
        }
        HermesUiKey::Home => {
            state.prompt_cursor = 0;
            state.sync_prompt_scroll();
            state.dirty.prompt = true;
        }
        HermesUiKey::End => {
            state.prompt_cursor = state.prompt.len();
            state.sync_prompt_scroll();
            state.dirty.prompt = true;
        }
        HermesUiKey::Backspace => state.delete_before_cursor(),
        HermesUiKey::Delete => state.delete_at_cursor(),
        HermesUiKey::CtrlL => state.force_redraw(),
        HermesUiKey::CtrlN => state.next_preset(),
        HermesUiKey::CtrlU => state.clear_prompt(),
        HermesUiKey::Up => state.scroll_response_up(1),
        HermesUiKey::Down => state.scroll_response_down(1),
        HermesUiKey::PageUp => state.scroll_response_up(HERMES_UI_RESPONSE_ROWS),
        HermesUiKey::PageDown => state.scroll_response_down(HERMES_UI_RESPONSE_ROWS),
        HermesUiKey::Char(byte) => {
            if state.focus == HermesUiFocus::Prompt {
                state.insert_byte(byte);
            } else {
                match byte.to_ascii_lowercase() {
                    b's' => return HermesUiCommand::Submit,
                    b'c' => state.clear_prompt(),
                    b'l' => state.load_active_preset(),
                    b'n' => state.next_preset(),
                    b'p' => state.previous_preset(),
                    b't' => return HermesUiCommand::RunTest,
                    b'q' => return HermesUiCommand::Exit,
                    _ => {}
                }
            }
        }
        HermesUiKey::Unknown => {}
    }
    HermesUiCommand::Continue
}

fn hermes_ui_handle_mouse(state: &mut HermesUiState, mouse: HermesMouseEvent) -> HermesUiCommand {
    if mouse.button == HermesMouseButton::WheelUp {
        state.scroll_response_up(1);
        return HermesUiCommand::Continue;
    }
    if mouse.button == HermesMouseButton::WheelDown {
        state.scroll_response_down(1);
        return HermesUiCommand::Continue;
    }
    if mouse.button != HermesMouseButton::Left || !mouse.pressed {
        return HermesUiCommand::Continue;
    }

    if mouse.y == HERMES_UI_PROMPT_ROW && mouse.x >= HERMES_UI_PROMPT_COL {
        state.focus = HermesUiFocus::Prompt;
        state.set_prompt_cursor_from_screen(mouse.x);
        state.status = String::from("Editing prompt");
        state.dirty.actions = true;
        state.dirty.status = true;
        return HermesUiCommand::Continue;
    }
    if hermes_ui_point_in_button(mouse.x, mouse.y, HERMES_UI_SEND_COL) {
        state.focus = HermesUiFocus::Send;
        state.dirty.actions = true;
        return HermesUiCommand::Submit;
    }
    if hermes_ui_point_in_button(mouse.x, mouse.y, HERMES_UI_CLEAR_COL) {
        state.focus = HermesUiFocus::Clear;
        state.clear_prompt();
        return HermesUiCommand::Continue;
    }
    if hermes_ui_point_in_button(mouse.x, mouse.y, HERMES_UI_LOAD_COL) {
        state.focus = HermesUiFocus::Load;
        state.load_active_preset();
        return HermesUiCommand::Continue;
    }
    if hermes_ui_point_in_button(mouse.x, mouse.y, HERMES_UI_TEST_COL) {
        state.focus = HermesUiFocus::Test;
        state.dirty.actions = true;
        return HermesUiCommand::RunTest;
    }
    if hermes_ui_point_in_button(mouse.x, mouse.y, HERMES_UI_EXIT_COL) {
        state.focus = HermesUiFocus::Exit;
        state.dirty.actions = true;
        return HermesUiCommand::Exit;
    }
    if mouse.y >= HERMES_UI_RESPONSE_ROW
        && mouse.y < HERMES_UI_RESPONSE_ROW.saturating_add(HERMES_UI_RESPONSE_ROWS)
    {
        state.focus = HermesUiFocus::Prompt;
        state.status = String::from("Response selected");
        state.dirty.actions = true;
        state.dirty.status = true;
    }
    if mouse.y >= HERMES_UI_PRESET_ROW
        && mouse.y < HERMES_UI_PRESET_ROW.saturating_add(HERMES_UI_PRESET_HEIGHT)
    {
        state.next_preset();
        state.load_active_preset();
    }
    HermesUiCommand::Continue
}

fn hermes_ui_point_in_button(x: usize, y: usize, col: usize) -> bool {
    y == HERMES_UI_BUTTON_ROW && x >= col && x < col.saturating_add(HERMES_UI_BUTTON_WIDTH)
}

fn hermes_ui_read_event() -> HermesUiEvent {
    let byte = UserShell::read_uart_byte();
    match byte {
        b'\r' | b'\n' => HermesUiEvent::Key(HermesUiKey::Enter),
        b'\t' => HermesUiEvent::Key(HermesUiKey::Tab),
        b'\x03' => HermesUiEvent::Key(HermesUiKey::CtrlC),
        b'\x0c' => HermesUiEvent::Key(HermesUiKey::CtrlL),
        b'\x0e' => HermesUiEvent::Key(HermesUiKey::CtrlN),
        b'\x15' => HermesUiEvent::Key(HermesUiKey::CtrlU),
        b'\x08' | b'\x7f' => HermesUiEvent::Key(HermesUiKey::Backspace),
        b'\x1b' => hermes_ui_read_escape_event(),
        byte if user_logic::ascii_shell_input(byte) => HermesUiEvent::Key(HermesUiKey::Char(byte)),
        _ => HermesUiEvent::Key(HermesUiKey::Unknown),
    }
}

fn hermes_ui_read_escape_event() -> HermesUiEvent {
    let Some(first) = hermes_ui_read_sequence_byte() else {
        return HermesUiEvent::Key(HermesUiKey::Esc);
    };
    if first != b'[' && first != b'O' {
        return HermesUiEvent::Key(HermesUiKey::Esc);
    }

    let Some(second) = hermes_ui_read_sequence_byte() else {
        return HermesUiEvent::Key(HermesUiKey::Esc);
    };
    if first == b'O' {
        return HermesUiEvent::Key(match second {
            b'A' => HermesUiKey::Up,
            b'B' => HermesUiKey::Down,
            b'C' => HermesUiKey::Right,
            b'D' => HermesUiKey::Left,
            b'H' => HermesUiKey::Home,
            b'F' => HermesUiKey::End,
            _ => HermesUiKey::Unknown,
        });
    }

    match second {
        b'A' => HermesUiEvent::Key(HermesUiKey::Up),
        b'B' => HermesUiEvent::Key(HermesUiKey::Down),
        b'C' => HermesUiEvent::Key(HermesUiKey::Right),
        b'D' => HermesUiEvent::Key(HermesUiKey::Left),
        b'H' => HermesUiEvent::Key(HermesUiKey::Home),
        b'F' => HermesUiEvent::Key(HermesUiKey::End),
        b'Z' => HermesUiEvent::Key(HermesUiKey::BackTab),
        b'<' => hermes_ui_read_sgr_mouse_event(),
        b'M' => hermes_ui_read_legacy_mouse_event(),
        byte if byte.is_ascii_digit() => hermes_ui_read_csi_numbered_event(byte),
        _ => HermesUiEvent::Key(HermesUiKey::Unknown),
    }
}

fn hermes_ui_read_csi_numbered_event(first: u8) -> HermesUiEvent {
    let mut bytes = [0u8; 8];
    let mut len = 0usize;
    bytes[len] = first;
    len += 1;
    while len < bytes.len() {
        let Some(byte) = hermes_ui_read_sequence_byte() else {
            break;
        };
        bytes[len] = byte;
        len += 1;
        if byte == b'~' || byte.is_ascii_alphabetic() {
            break;
        }
    }

    let key = if len >= 2 && bytes[len - 1] == b'~' {
        match bytes[0] {
            b'1' | b'7' => HermesUiKey::Home,
            b'3' => HermesUiKey::Delete,
            b'5' => HermesUiKey::PageUp,
            b'6' => HermesUiKey::PageDown,
            b'4' | b'8' => HermesUiKey::End,
            _ => HermesUiKey::Unknown,
        }
    } else {
        HermesUiKey::Unknown
    };
    HermesUiEvent::Key(key)
}

fn hermes_ui_read_sgr_mouse_event() -> HermesUiEvent {
    let mut values = [0usize; 3];
    let mut value_index = 0usize;
    let mut current = 0usize;
    let mut have_digit = false;

    for _ in 0..24 {
        let Some(byte) = hermes_ui_read_sequence_byte() else {
            return HermesUiEvent::Key(HermesUiKey::Unknown);
        };
        if byte.is_ascii_digit() {
            current = current
                .saturating_mul(10)
                .saturating_add((byte - b'0') as usize);
            have_digit = true;
        } else if byte == b';' {
            if value_index >= values.len() || !have_digit {
                return HermesUiEvent::Key(HermesUiKey::Unknown);
            }
            values[value_index] = current;
            value_index += 1;
            current = 0;
            have_digit = false;
        } else if byte == b'M' || byte == b'm' {
            if value_index >= values.len() || !have_digit {
                return HermesUiEvent::Key(HermesUiKey::Unknown);
            }
            values[value_index] = current;
            return HermesUiEvent::Mouse(HermesMouseEvent {
                x: values[1],
                y: values[2],
                button: hermes_mouse_button(values[0]),
                pressed: byte == b'M',
            });
        } else {
            return HermesUiEvent::Key(HermesUiKey::Unknown);
        }
    }

    HermesUiEvent::Key(HermesUiKey::Unknown)
}

fn hermes_ui_read_legacy_mouse_event() -> HermesUiEvent {
    let Some(button_byte) = hermes_ui_read_sequence_byte() else {
        return HermesUiEvent::Key(HermesUiKey::Unknown);
    };
    let Some(x_byte) = hermes_ui_read_sequence_byte() else {
        return HermesUiEvent::Key(HermesUiKey::Unknown);
    };
    let Some(y_byte) = hermes_ui_read_sequence_byte() else {
        return HermesUiEvent::Key(HermesUiKey::Unknown);
    };
    if button_byte < 32 || x_byte < 32 || y_byte < 32 {
        return HermesUiEvent::Key(HermesUiKey::Unknown);
    }
    let code = (button_byte - 32) as usize;
    HermesUiEvent::Mouse(HermesMouseEvent {
        x: (x_byte - 32) as usize,
        y: (y_byte - 32) as usize,
        button: hermes_mouse_button(code),
        pressed: code & 0x3 != 0x3,
    })
}

fn hermes_ui_read_sequence_byte() -> Option<u8> {
    for _ in 0..4096 {
        if let Some(byte) = UserShell::try_read_uart_byte() {
            return Some(byte);
        }
        core::hint::spin_loop();
    }
    None
}

fn hermes_mouse_button(code: usize) -> HermesMouseButton {
    if code & 64 != 0 {
        if code & 1 == 0 {
            HermesMouseButton::WheelUp
        } else {
            HermesMouseButton::WheelDown
        }
    } else {
        match code & 0x3 {
            0 => HermesMouseButton::Left,
            _ => HermesMouseButton::Other,
        }
    }
}

fn hermes_ui_render(ctx: &mut ShellContext, state: &mut HermesUiState) {
    state.sync_prompt_scroll();
    let total_response_lines =
        wrapped_line_count(state.last_answer.as_str(), HERMES_UI_RESPONSE_WIDTH).max(1);
    let max_scroll = total_response_lines.saturating_sub(HERMES_UI_RESPONSE_ROWS);
    if state.response_scroll > max_scroll {
        state.response_scroll = max_scroll;
        state.dirty.response = true;
    }

    if state.dirty.layout {
        ctx.serial.write_str("\x1b[0m\x1b[2J\x1b[H");
        hermes_ui_draw_box(
            ctx,
            HERMES_UI_PROMPT_BOX_ROW,
            HERMES_UI_LEFT_COL,
            HERMES_UI_LEFT_WIDTH,
            HERMES_UI_PROMPT_BOX_HEIGHT,
            "Prompt Composer",
            false,
        );
        hermes_ui_draw_box(
            ctx,
            HERMES_UI_RUNTIME_ROW,
            HERMES_UI_RIGHT_COL,
            HERMES_UI_RIGHT_WIDTH,
            HERMES_UI_RUNTIME_HEIGHT,
            "Runtime",
            false,
        );
        hermes_ui_draw_box(
            ctx,
            HERMES_UI_RESPONSE_BOX_ROW,
            HERMES_UI_LEFT_COL,
            HERMES_UI_LEFT_WIDTH,
            HERMES_UI_RESPONSE_BOX_HEIGHT,
            "Response",
            false,
        );
        hermes_ui_draw_box(
            ctx,
            HERMES_UI_ACTIVITY_ROW,
            HERMES_UI_RIGHT_COL,
            HERMES_UI_RIGHT_WIDTH,
            HERMES_UI_ACTIVITY_HEIGHT,
            "Activity",
            false,
        );
        hermes_ui_draw_box(
            ctx,
            HERMES_UI_PRESET_ROW,
            HERMES_UI_LEFT_COL,
            HERMES_UI_WIDTH - 2,
            HERMES_UI_PRESET_HEIGHT,
            "Presets",
            false,
        );
        state.dirty.header = true;
        state.dirty.prompt = true;
        state.dirty.actions = true;
        state.dirty.response = true;
        state.dirty.runtime = true;
        state.dirty.activity = true;
        state.dirty.presets = true;
        state.dirty.status = true;
    }

    if state.dirty.header {
        hermes_ui_move(ctx, 1, 1);
        ctx.serial
            .write_str("\x1b[38;2;238;244;248m\x1b[48;2;37;138;255m");
        hermes_ui_push_fixed(
            &mut ctx.serial,
            " SMROS LVGL Hermes Workbench",
            HERMES_UI_WIDTH,
        );

        hermes_ui_move(ctx, 2, 1);
        ctx.serial
            .write_str("\x1b[38;2;238;244;248m\x1b[48;2;34;40;46m");
        let mut status = String::from(" ");
        status.push_str(state.metrics.as_str());
        status.push_str(" | LVGL ");
        status.push_str(crate::user_level::lvgl::LVGL_COMPAT_VERSION);
        hermes_ui_push_fixed(&mut ctx.serial, status.as_str(), HERMES_UI_WIDTH);

        hermes_ui_move(ctx, 3, 1);
        ctx.serial
            .write_str("\x1b[38;2;154;166;176m\x1b[48;2;20;24;28m");
        hermes_ui_push_fixed(
            &mut ctx.serial,
            "  Prompt composer     Response stream        Runtime          Activity",
            HERMES_UI_WIDTH,
        );
    }

    if state.dirty.prompt {
        hermes_ui_draw_prompt_panel(ctx, state);
    }

    if state.dirty.actions {
        hermes_ui_draw_actions(ctx, state);
    }

    if state.dirty.response {
        hermes_ui_draw_response_panel(ctx, state, total_response_lines);
    }

    if state.dirty.runtime {
        hermes_ui_draw_side_panel(
            ctx,
            HERMES_UI_RUNTIME_ROW + 1,
            HERMES_UI_RIGHT_COL + 2,
            HERMES_UI_RIGHT_WIDTH - 4,
            HERMES_UI_RUNTIME_HEIGHT - 2,
            state.runtime.as_str(),
        );
    }

    if state.dirty.activity {
        hermes_ui_draw_side_panel(
            ctx,
            HERMES_UI_ACTIVITY_ROW + 1,
            HERMES_UI_RIGHT_COL + 2,
            HERMES_UI_RIGHT_WIDTH - 4,
            HERMES_UI_ACTIVITY_HEIGHT - 2,
            state.activity.as_str(),
        );
    }

    if state.dirty.presets {
        hermes_ui_draw_presets(ctx, state);
    }

    if state.dirty.status {
        hermes_ui_draw_status(ctx, state);
    }

    ctx.serial.write_str("\x1b[0m");
    state.dirty.clear();
}

fn hermes_ui_draw_prompt_panel(ctx: &mut ShellContext, state: &HermesUiState) {
    hermes_ui_clear_area(
        ctx,
        HERMES_UI_PROMPT_BOX_ROW + 1,
        HERMES_UI_LEFT_COL + 1,
        HERMES_UI_LEFT_WIDTH - 2,
        HERMES_UI_PROMPT_BOX_HEIGHT - 2,
        "\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m",
    );
    hermes_ui_move(ctx, HERMES_UI_PROMPT_BOX_ROW + 1, HERMES_UI_LEFT_COL + 2);
    ctx.serial
        .write_str("\x1b[38;2;154;166;176m\x1b[48;2;34;40;46m");
    hermes_ui_push_fixed(
        &mut ctx.serial,
        " LVGL textarea  Enter sends  Ctrl-N preset  Ctrl-U clear",
        HERMES_UI_LEFT_WIDTH - 4,
    );
    hermes_ui_draw_prompt(ctx, state);
    hermes_ui_move(ctx, HERMES_UI_PROMPT_BOX_ROW + 4, HERMES_UI_LEFT_COL + 2);
    ctx.serial
        .write_str("\x1b[38;2;154;166;176m\x1b[48;2;34;40;46m");
    let mut meta = String::from("chars ");
    append_usize_shell(&mut meta, state.prompt.len());
    meta.push('/');
    append_usize_shell(&mut meta, HERMES_UI_PROMPT_MAX);
    meta.push_str("  scroll ");
    append_usize_shell(&mut meta, state.prompt_scroll);
    hermes_ui_push_fixed(&mut ctx.serial, meta.as_str(), HERMES_UI_LEFT_WIDTH - 4);
    hermes_ui_move(ctx, HERMES_UI_PROMPT_BOX_ROW + 5, HERMES_UI_LEFT_COL + 2);
    ctx.serial
        .write_str("\x1b[38;2;72;190;123m\x1b[48;2;34;40;46m");
    let fill = state.prompt.len().min(HERMES_UI_PROMPT_MAX);
    let mut meter = String::from("buffer ");
    hermes_ui_push_meter(&mut meter, fill, HERMES_UI_PROMPT_MAX, 22);
    hermes_ui_push_fixed(&mut ctx.serial, meter.as_str(), HERMES_UI_LEFT_WIDTH - 4);
}

fn hermes_ui_draw_prompt(ctx: &mut ShellContext, state: &HermesUiState) {
    hermes_ui_move(ctx, HERMES_UI_PROMPT_ROW, HERMES_UI_PROMPT_COL);
    ctx.serial
        .write_str("\x1b[38;2;238;244;248m\x1b[48;2;45;52;60m");
    let bytes = state.prompt.as_bytes();
    for index in 0..HERMES_UI_PROMPT_WIDTH {
        let prompt_index = state.prompt_scroll.saturating_add(index);
        let byte = if prompt_index < bytes.len() {
            bytes[prompt_index]
        } else {
            b' '
        };
        if prompt_index == state.prompt_cursor && state.focus == HermesUiFocus::Prompt {
            ctx.serial
                .write_str("\x1b[38;2;255;255;255m\x1b[48;2;37;138;255m");
            ctx.serial
                .write_byte(if byte == b' ' { b' ' } else { byte });
            ctx.serial
                .write_str("\x1b[38;2;238;244;248m\x1b[48;2;45;52;60m");
        } else {
            ctx.serial.write_byte(sanitize_terminal_byte(byte));
        }
    }
}

fn hermes_ui_draw_actions(ctx: &mut ShellContext, state: &HermesUiState) {
    hermes_ui_draw_button(
        ctx,
        "Send",
        HERMES_UI_BUTTON_ROW,
        HERMES_UI_SEND_COL,
        state.focus == HermesUiFocus::Send,
    );
    hermes_ui_draw_button(
        ctx,
        "Clear",
        HERMES_UI_BUTTON_ROW,
        HERMES_UI_CLEAR_COL,
        state.focus == HermesUiFocus::Clear,
    );
    hermes_ui_draw_button(
        ctx,
        "Load",
        HERMES_UI_BUTTON_ROW,
        HERMES_UI_LOAD_COL,
        state.focus == HermesUiFocus::Load,
    );
    hermes_ui_draw_button(
        ctx,
        "Test",
        HERMES_UI_BUTTON_ROW,
        HERMES_UI_TEST_COL,
        state.focus == HermesUiFocus::Test,
    );
    hermes_ui_draw_button(
        ctx,
        "Exit",
        HERMES_UI_BUTTON_ROW,
        HERMES_UI_EXIT_COL,
        state.focus == HermesUiFocus::Exit,
    );
}

fn hermes_ui_draw_response_panel(
    ctx: &mut ShellContext,
    state: &HermesUiState,
    total_response_lines: usize,
) {
    hermes_ui_clear_area(
        ctx,
        HERMES_UI_RESPONSE_BOX_ROW + 1,
        HERMES_UI_LEFT_COL + 1,
        HERMES_UI_LEFT_WIDTH - 2,
        HERMES_UI_RESPONSE_BOX_HEIGHT - 2,
        "\x1b[38;2;238;244;248m\x1b[48;2;34;40;46m",
    );
    hermes_ui_move(ctx, HERMES_UI_RESPONSE_BOX_ROW + 1, HERMES_UI_LEFT_COL + 2);
    ctx.serial
        .write_str("\x1b[38;2;154;166;176m\x1b[48;2;34;40;46m");
    let mut meta = String::from("line ");
    append_usize_shell(&mut meta, state.response_scroll.saturating_add(1));
    meta.push('/');
    append_usize_shell(&mut meta, total_response_lines);
    meta.push_str("  Up/Down scroll  PgUp/PgDn page");
    hermes_ui_push_fixed(&mut ctx.serial, meta.as_str(), HERMES_UI_LEFT_WIDTH - 4);
    hermes_ui_draw_wrapped(
        ctx,
        state.last_answer.as_str(),
        HERMES_UI_RESPONSE_ROW,
        HERMES_UI_RESPONSE_COL,
        HERMES_UI_RESPONSE_WIDTH,
        HERMES_UI_RESPONSE_ROWS,
        state.response_scroll,
    );
}

fn hermes_ui_draw_side_panel(
    ctx: &mut ShellContext,
    row: usize,
    col: usize,
    width: usize,
    rows: usize,
    text: &str,
) {
    hermes_ui_clear_area(
        ctx,
        row,
        col,
        width,
        rows,
        "\x1b[38;2;238;244;248m\x1b[48;2;34;40;46m",
    );
    hermes_ui_draw_lines(ctx, text, row, col, width, rows);
}

fn hermes_ui_draw_presets(ctx: &mut ShellContext, state: &HermesUiState) {
    let row = HERMES_UI_PRESET_ROW + 1;
    let col = HERMES_UI_LEFT_COL + 2;
    let width = HERMES_UI_WIDTH - 6;
    hermes_ui_clear_area(
        ctx,
        row,
        col,
        width,
        1,
        "\x1b[38;2;238;244;248m\x1b[48;2;34;40;46m",
    );
    hermes_ui_move(ctx, row, col);
    ctx.serial
        .write_str("\x1b[38;2;238;244;248m\x1b[48;2;34;40;46m");
    let mut line = String::from("Preset ");
    append_usize_shell(&mut line, state.active_preset + 1);
    line.push('/');
    append_usize_shell(&mut line, HERMES_UI_PRESETS.len());
    line.push_str(": ");
    line.push_str(HERMES_UI_PRESETS[state.active_preset]);
    hermes_ui_push_fixed(&mut ctx.serial, line.as_str(), width);
}

fn hermes_ui_draw_status(ctx: &mut ShellContext, state: &HermesUiState) {
    hermes_ui_move(ctx, HERMES_UI_STATUS_ROW, 1);
    ctx.serial
        .write_str("\x1b[38;2;238;244;248m\x1b[48;2;45;52;60m");
    let mut status = String::from("Status: ");
    status.push_str(state.status.as_str());
    status.push_str(" | Tab focus | Enter action | s/c/l/t/q | mouse enabled");
    hermes_ui_push_fixed(&mut ctx.serial, status.as_str(), HERMES_UI_WIDTH);
}

fn hermes_ui_draw_button(
    ctx: &mut ShellContext,
    label: &str,
    row: usize,
    col: usize,
    focused: bool,
) {
    hermes_ui_move(ctx, row, col);
    if focused {
        ctx.serial
            .write_str("\x1b[38;2;255;255;255m\x1b[48;2;37;138;255m");
    } else {
        ctx.serial
            .write_str("\x1b[38;2;238;244;248m\x1b[48;2;45;52;60m");
    }
    let mut text = String::from(" ");
    text.push_str(label);
    text.push_str(" ");
    hermes_ui_push_fixed(&mut ctx.serial, text.as_str(), HERMES_UI_BUTTON_WIDTH);
}

fn hermes_ui_draw_box(
    ctx: &mut ShellContext,
    row: usize,
    col: usize,
    width: usize,
    height: usize,
    title: &str,
    focused: bool,
) {
    let border_color = if focused {
        "\x1b[38;2;37;138;255m\x1b[48;2;20;24;28m"
    } else {
        "\x1b[38;2;76;88;101m\x1b[48;2;20;24;28m"
    };
    let fill_color = "\x1b[38;2;238;244;248m\x1b[48;2;34;40;46m";

    hermes_ui_move(ctx, row, col);
    ctx.serial.write_str(border_color);
    ctx.serial.write_byte(b'/');
    for _ in 0..width.saturating_sub(2) {
        ctx.serial.write_byte(b'=');
    }
    ctx.serial.write_byte(b'\\');

    if width > 4 && !title.is_empty() {
        hermes_ui_move(ctx, row, col + 2);
        ctx.serial
            .write_str("\x1b[38;2;238;244;248m\x1b[48;2;20;24;28m");
        ctx.serial.write_byte(b' ');
        ctx.serial.write_str(title);
        ctx.serial.write_byte(b' ');
    }

    for y in row.saturating_add(1)..row.saturating_add(height.saturating_sub(1)) {
        hermes_ui_move(ctx, y, col);
        ctx.serial.write_str(border_color);
        ctx.serial.write_byte(b'|');
        ctx.serial.write_str(fill_color);
        for _ in 0..width.saturating_sub(2) {
            ctx.serial.write_byte(b' ');
        }
        ctx.serial.write_str(border_color);
        ctx.serial.write_byte(b'|');
    }

    hermes_ui_move(ctx, row.saturating_add(height.saturating_sub(1)), col);
    ctx.serial.write_str(border_color);
    ctx.serial.write_byte(b'\\');
    for _ in 0..width.saturating_sub(2) {
        ctx.serial.write_byte(b'=');
    }
    ctx.serial.write_byte(b'/');
}

fn hermes_ui_clear_area(
    ctx: &mut ShellContext,
    row: usize,
    col: usize,
    width: usize,
    rows: usize,
    color: &str,
) {
    for offset in 0..rows {
        hermes_ui_move(ctx, row.saturating_add(offset), col);
        ctx.serial.write_str(color);
        for _ in 0..width {
            ctx.serial.write_byte(b' ');
        }
    }
}

fn hermes_ui_draw_lines(
    ctx: &mut ShellContext,
    text: &str,
    row: usize,
    col: usize,
    width: usize,
    max_rows: usize,
) {
    let mut drawn = 0usize;
    for line in text.lines() {
        if drawn >= max_rows {
            break;
        }
        hermes_ui_move(ctx, row.saturating_add(drawn), col);
        ctx.serial
            .write_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m");
        hermes_ui_push_fixed(&mut ctx.serial, line, width);
        drawn += 1;
    }
}

fn hermes_ui_draw_wrapped(
    ctx: &mut ShellContext,
    text: &str,
    row: usize,
    col: usize,
    width: usize,
    max_rows: usize,
    skip_rows: usize,
) {
    let mut current = String::new();
    let mut produced = 0usize;
    let mut drawn = 0usize;
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len().saturating_add(1).saturating_add(word.len()) > width
        {
            hermes_ui_maybe_draw_wrapped_line(
                ctx,
                current.as_str(),
                row,
                col,
                width,
                max_rows,
                skip_rows,
                &mut produced,
                &mut drawn,
            );
            current.clear();
            if drawn >= max_rows {
                return;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        hermes_ui_maybe_draw_wrapped_line(
            ctx,
            current.as_str(),
            row,
            col,
            width,
            max_rows,
            skip_rows,
            &mut produced,
            &mut drawn,
        );
    }
}

fn hermes_ui_maybe_draw_wrapped_line(
    ctx: &mut ShellContext,
    line: &str,
    row: usize,
    col: usize,
    width: usize,
    max_rows: usize,
    skip_rows: usize,
    produced: &mut usize,
    drawn: &mut usize,
) {
    if *produced >= skip_rows && *drawn < max_rows {
        hermes_ui_move(ctx, row.saturating_add(*drawn), col);
        ctx.serial
            .write_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m");
        hermes_ui_push_fixed(&mut ctx.serial, line, width);
        *drawn = drawn.saturating_add(1);
    }
    *produced = produced.saturating_add(1);
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    let mut current_len = 0usize;
    let mut count = 0usize;
    for word in text.split_whitespace() {
        let word_len = word.len();
        if current_len > 0 && current_len.saturating_add(1).saturating_add(word_len) > width {
            count = count.saturating_add(1);
            current_len = 0;
        }
        if current_len > 0 {
            current_len = current_len.saturating_add(1);
        }
        current_len = current_len.saturating_add(word_len);
    }
    if current_len > 0 {
        count = count.saturating_add(1);
    }
    count
}

fn hermes_ui_move(ctx: &mut ShellContext, row: usize, col: usize) {
    ctx.serial.write_str("\x1b[");
    print_usize(&mut ctx.serial, row);
    ctx.serial.write_str(";");
    print_usize(&mut ctx.serial, col);
    ctx.serial.write_str("H");
}

fn hermes_ui_push_fixed(serial: &mut Serial, text: &str, width: usize) {
    let mut written = 0usize;
    for byte in text.bytes() {
        if written >= width {
            break;
        }
        serial.write_byte(sanitize_terminal_byte(byte));
        written += 1;
    }
    for _ in written..width {
        serial.write_byte(b' ');
    }
}

fn hermes_ui_push_meter(out: &mut String, value: usize, max: usize, width: usize) {
    out.push('[');
    let filled = if max == 0 {
        0
    } else {
        value.min(max).saturating_mul(width) / max
    };
    let mut index = 0usize;
    while index < width {
        out.push(if index < filled { '#' } else { '-' });
        index += 1;
    }
    out.push(']');
}

fn sanitize_terminal_byte(byte: u8) -> u8 {
    if byte == b'\n' || user_logic::ascii_shell_input(byte) {
        byte
    } else {
        b'?'
    }
}

fn append_usize_shell(out: &mut String, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        out.push('0');
        return;
    }
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn trim_ascii_shell(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &value[start..end]
}

fn print_hermes_info(
    ctx: &mut ShellContext,
    info: &crate::user_level::hermes_agent::HermesAgentInfo,
) {
    ctx.serial.write_str("Hermes Agent port\n");
    ctx.serial.write_str("  upstream: ");
    ctx.serial.write_str(info.upstream);
    ctx.serial.write_str(" v");
    ctx.serial.write_str(info.upstream_version);
    ctx.serial.write_str("\n  provider: ");
    ctx.serial.write_str(info.provider);
    ctx.serial.write_str("\n  model: ");
    ctx.serial.write_str(info.model);
    ctx.serial.write_str("\n  personality: ");
    ctx.serial.write_str(info.personality);
    ctx.serial.write_str("\n  tools=");
    print_usize(&mut ctx.serial, info.tools);
    ctx.serial.write_str(" skills=");
    print_usize(&mut ctx.serial, info.skills);
    ctx.serial.write_str(" memory_items=");
    print_usize(&mut ctx.serial, info.memory_items);
    ctx.serial.write_str(" cron=");
    print_usize(&mut ctx.serial, info.cron_jobs);
    ctx.serial.write_str(" transcripts=");
    print_usize(&mut ctx.serial, info.transcripts);
    ctx.serial.write_str(" web=");
    ctx.serial.write_str(info.web_ui_path);
    ctx.serial.write_str(" bytes=");
    print_usize(&mut ctx.serial, info.web_ui_bytes);
    ctx.serial.write_str(" cpu_ui=");
    ctx.serial.write_str(info.cpu_ui_path);
    ctx.serial.write_str(" cpu_bytes=");
    print_usize(&mut ctx.serial, info.cpu_ui_bytes);
    ctx.serial.write_str(" backend=");
    ctx.serial.write_str(info.generation_backend);
    ctx.serial.write_str("\n");
}

fn run_hermes_agent_tests(ctx: &mut ShellContext) -> bool {
    ctx.serial
        .write_str("[TEST] Testing Hermes agent SMROS port... ");
    match crate::user_level::hermes_agent::run_full_test() {
        Ok(report) if report.passed() => {
            ctx.serial.write_str("[OK] config=");
            ctx.serial
                .write_str(if report.config_ok { "yes" } else { "no" });
            ctx.serial.write_str(" model=");
            ctx.serial
                .write_str(if report.model_route_ok { "yes" } else { "no" });
            ctx.serial.write_str(" skills=");
            ctx.serial
                .write_str(if report.skill_ok { "yes" } else { "no" });
            ctx.serial.write_str(" memory=");
            ctx.serial
                .write_str(if report.memory_ok { "yes" } else { "no" });
            ctx.serial.write_str(" tools=");
            print_usize(&mut ctx.serial, report.turn.tool_calls);
            ctx.serial.write_str(" delegates=");
            print_usize(&mut ctx.serial, report.turn.delegated_agents);
            ctx.serial.write_str(" transcript=");
            print_usize(&mut ctx.serial, report.turn.transcript_bytes);
            ctx.serial.write_str("B gemma_tokens=");
            print_usize(&mut ctx.serial, report.turn.model_tokens);
            ctx.serial.write_str(" svc=");
            ctx.serial
                .write_str(if report.svc_ok { "yes" } else { "no" });
            ctx.serial.write_str(" web=");
            ctx.serial
                .write_str(if report.web_ui_ok { "yes" } else { "no" });
            ctx.serial.write_str("\n");
            true
        }
        Ok(report) => {
            ctx.serial.write_str("[FAIL] incomplete report tools=");
            print_usize(&mut ctx.serial, report.turn.tool_calls);
            ctx.serial.write_str(" delegates=");
            print_usize(&mut ctx.serial, report.turn.delegated_agents);
            ctx.serial.write_str("\n");
            false
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            ctx.serial.write_str(err.as_str());
            ctx.serial.write_str("\n");
            false
        }
    }
}

fn run_lvgl_tests(ctx: &mut ShellContext) -> bool {
    ctx.serial
        .write_str("[TEST] Testing SMROS LVGL UI port... ");
    match crate::user_level::lvgl::run_full_test() {
        Ok(report) if report.passed() => {
            ctx.serial.write_str("[OK] port=");
            ctx.serial
                .write_str(if report.port_ok { "yes" } else { "no" });
            ctx.serial.write_str(" display=");
            ctx.serial
                .write_str(if report.display_flush_ok { "yes" } else { "no" });
            ctx.serial.write_str(" input=");
            ctx.serial
                .write_str(if report.input_ok { "yes" } else { "no" });
            ctx.serial.write_str(" widgets=");
            ctx.serial
                .write_str(if report.widgets_ok { "yes" } else { "no" });
            ctx.serial.write_str(" fxfs=");
            ctx.serial
                .write_str(if report.fxfs_ok { "yes" } else { "no" });
            ctx.serial.write_str(" image=");
            ctx.serial.write_str(report.render.image_path);
            ctx.serial.write_str(" bytes=");
            print_usize(&mut ctx.serial, report.render.image_bytes);
            ctx.serial.write_str("\n");
            true
        }
        Ok(report) => {
            ctx.serial.write_str("[FAIL] incomplete report widgets=");
            print_usize(&mut ctx.serial, report.render.widgets);
            ctx.serial.write_str("\n");
            false
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            ctx.serial.write_str(err.as_str());
            ctx.serial.write_str("\n");
            false
        }
    }
}

fn run_qml_cluster_tests(ctx: &mut ShellContext) -> bool {
    ctx.serial
        .write_str("[TEST] Testing Qt/QML LVGL vehicle cluster port... ");
    match crate::user_level::qml_cluster::run_full_test() {
        Ok(report) if report.passed() => {
            ctx.serial.write_str("[OK] qml=");
            ctx.serial
                .write_str(if report.qml_ok { "yes" } else { "no" });
            ctx.serial.write_str(" parse=");
            ctx.serial
                .write_str(if report.parse_ok { "yes" } else { "no" });
            ctx.serial.write_str(" render=");
            ctx.serial
                .write_str(if report.render_ok { "yes" } else { "no" });
            ctx.serial.write_str(" fxfs=");
            ctx.serial
                .write_str(if report.fxfs_ok { "yes" } else { "no" });
            ctx.serial.write_str(" lvgl=");
            ctx.serial
                .write_str(if report.lvgl_ok { "yes" } else { "no" });
            ctx.serial.write_str(" speed=");
            print_usize(&mut ctx.serial, report.state.speed_kph);
            ctx.serial.write_str("kph rpm=");
            print_usize(&mut ctx.serial, report.state.rpm);
            ctx.serial.write_str(" image=");
            ctx.serial.write_str(report.render.image_path);
            ctx.serial.write_str(" bytes=");
            print_usize(&mut ctx.serial, report.render.image_bytes);
            ctx.serial.write_str("\n");
            true
        }
        Ok(report) => {
            ctx.serial.write_str("[FAIL] incomplete report speed=");
            print_usize(&mut ctx.serial, report.state.speed_kph);
            ctx.serial.write_str(" image_bytes=");
            print_usize(&mut ctx.serial, report.render.image_bytes);
            ctx.serial.write_str("\n");
            false
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            ctx.serial.write_str(err.as_str());
            ctx.serial.write_str("\n");
            false
        }
    }
}

fn print_docker_usage(ctx: &mut ShellContext) {
    ctx.serial.write_str(
        "usage: docker images | docker pull <image-or-http-url> | docker load [-i|--input] <archive.tar> | docker ps [-a] | docker create <image> [command...] | docker start <container> | docker run <image> [command...] | docker inspect <container> | docker logs <container> | docker stop <container> | docker rm <container>\n\n",
    );
}

struct DockerContainerArgs<'a> {
    image: &'a str,
    command: &'a [&'a str],
    name: Option<&'a str>,
    interactive: bool,
    tty: bool,
    rm: bool,
}

fn docker_container_args<'a>(
    args: &'a [&'a str],
    allow_rm: bool,
) -> Option<DockerContainerArgs<'a>> {
    if args.len() < 2 {
        return None;
    }

    let mut index = 1usize;
    let mut name = None;
    let mut interactive = false;
    let mut tty = false;
    let mut rm = false;
    while index < args.len() {
        match args[index] {
            "-i" | "--interactive" => {
                interactive = true;
                index += 1;
            }
            "-t" | "--tty" => {
                tty = true;
                index += 1;
            }
            "-it" | "-ti" => {
                interactive = true;
                tty = true;
                index += 1;
            }
            "--rm" if allow_rm => {
                rm = true;
                index += 1;
            }
            "--name" => {
                if index + 1 >= args.len() {
                    return None;
                }
                name = Some(args[index + 1]);
                index += 2;
            }
            value if value.starts_with("--name=") => {
                let value = &value["--name=".len()..];
                if value.is_empty() {
                    return None;
                }
                name = Some(value);
                index += 1;
            }
            value if value.starts_with('-') => return None,
            _ => break,
        }
    }

    if index >= args.len() {
        return None;
    }
    Some(DockerContainerArgs {
        image: args[index],
        command: &args[index + 1..],
        name,
        interactive,
        tty,
        rm,
    })
}

fn docker_load_input_arg<'a>(args: &'a [&str]) -> Option<&'a str> {
    if args.len() == 2 {
        if let Some(value) = args[1].strip_prefix("--input=") {
            return (!value.is_empty()).then_some(value);
        }
        return Some(args[1]);
    }
    if args.len() == 3 && (args[1] == "-i" || args[1] == "--input") {
        return Some(args[2]);
    }
    None
}

fn run_interactive_docker_container(
    ctx: &mut ShellContext,
    parsed: &DockerContainerArgs<'_>,
) -> Result<(), crate::user_level::docker_compat::DockerCompatError> {
    let image = crate::user_level::docker_compat::docker_image_info(parsed.image)?;
    let container = crate::user_level::docker_compat::create_docker_container(
        parsed.image,
        parsed.command,
        parsed.name,
    )?;
    let started = crate::user_level::docker_compat::start_docker_container(container.id.as_str())?;
    ctx.serial.write_str("[OK] attached id=");
    ctx.serial.write_str(started.container.id.as_str());
    ctx.serial.write_str(" image=");
    ctx.serial.write_str(started.container.image.as_str());
    ctx.serial.write_str("\n");
    run_docker_shell(ctx, started.container.id.as_str(), image.rootfs.as_str());
    let stopped =
        crate::user_level::docker_compat::stop_docker_container(started.container.id.as_str())?;
    if parsed.rm {
        let _ = crate::user_level::docker_compat::remove_docker_container(stopped.id.as_str());
    }
    ctx.serial.write_str("[DOCKER] detached ");
    ctx.serial.write_str(stopped.id.as_str());
    ctx.serial.write_str(" status=");
    ctx.serial.write_str(stopped.status.as_str());
    ctx.serial.write_str("\n\n");
    Ok(())
}

fn run_docker_shell(ctx: &mut ShellContext, container_id: &str, rootfs: &str) {
    let mut cwd = String::from("/");
    loop {
        ctx.serial.write_str(container_id);
        ctx.serial.write_str(":");
        ctx.serial.write_str(cwd.as_str());
        ctx.serial.write_str("# ");
        let line = ctx.read_line();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "exit" | "logout" => break,
            "pwd" => {
                ctx.serial.write_str(cwd.as_str());
                ctx.serial.write_str("\n");
            }
            "cd" => docker_shell_cd(ctx, rootfs, &mut cwd, parts.get(1).copied().unwrap_or("/")),
            "ls" => docker_shell_ls(
                ctx,
                rootfs,
                cwd.as_str(),
                parts.get(1).copied().unwrap_or("."),
            ),
            "cat" => {
                if parts.len() < 2 {
                    ctx.serial.write_str("cat: missing path\n");
                } else {
                    docker_shell_cat(ctx, rootfs, cwd.as_str(), parts[1]);
                }
            }
            "echo" => docker_shell_echo(ctx, &parts[1..]),
            "help" => ctx.serial.write_str("builtins: pwd ls cd cat echo exit\n"),
            _ => {
                ctx.serial.write_str(parts[0]);
                ctx.serial
                    .write_str(": command not available in SMROS modeled container shell\n");
            }
        }
    }
}

fn docker_shell_cd(ctx: &mut ShellContext, rootfs: &str, cwd: &mut String, target: &str) {
    let Some(path) = docker_rooted_path(rootfs, cwd.as_str(), target) else {
        ctx.serial.write_str("cd: invalid path\n");
        return;
    };
    match crate::user_level::fxfs::entries(path.as_str()) {
        Ok(_) => {
            if let Some(display) = docker_display_path(rootfs, path.as_str()) {
                *cwd = display;
            }
        }
        Err(err) => {
            ctx.serial.write_str("cd: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn docker_shell_ls(ctx: &mut ShellContext, rootfs: &str, cwd: &str, target: &str) {
    let Some(path) = docker_rooted_path(rootfs, cwd, target) else {
        ctx.serial.write_str("ls: invalid path\n");
        return;
    };
    match crate::user_level::fxfs::entries(path.as_str()) {
        Ok(entries) => {
            for entry in entries {
                ctx.serial.write_str(entry.name.as_str());
                if matches!(entry.kind, crate::user_level::fxfs::FxfsNodeKind::Directory) {
                    ctx.serial.write_str("/");
                }
                ctx.serial.write_str("\n");
            }
        }
        Err(crate::user_level::fxfs::FxfsError::NotDirectory) => {
            if let Some(display) = docker_display_path(rootfs, path.as_str()) {
                ctx.serial.write_str(display.as_str());
                ctx.serial.write_str("\n");
            }
        }
        Err(err) => {
            ctx.serial.write_str("ls: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn docker_shell_cat(ctx: &mut ShellContext, rootfs: &str, cwd: &str, target: &str) {
    let Some(path) = docker_rooted_path(rootfs, cwd, target) else {
        ctx.serial.write_str("cat: invalid path\n");
        return;
    };
    match read_fxfs_file_to_vec(path.as_str()) {
        Ok(bytes) => {
            print_bytes_as_text(ctx, &bytes);
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("cat: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn docker_shell_echo(ctx: &mut ShellContext, args: &[&str]) {
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            ctx.serial.write_str(" ");
        }
        ctx.serial.write_str(arg);
    }
    ctx.serial.write_str("\n");
}

fn docker_rooted_path(rootfs: &str, cwd: &str, target: &str) -> Option<String> {
    let display = if target.starts_with('/') {
        normalize_fxfs_path("/", target)?
    } else {
        normalize_fxfs_path(cwd, target)?
    };
    let mut out = String::from(rootfs.trim_end_matches('/'));
    if display != "/" {
        out.push_str(display.as_str());
    }
    Some(out)
}

fn docker_display_path(rootfs: &str, path: &str) -> Option<String> {
    let rootfs = rootfs.trim_end_matches('/');
    if path == rootfs {
        return Some(String::from("/"));
    }
    path.strip_prefix(rootfs)
        .filter(|suffix| suffix.starts_with('/'))
        .map(String::from)
}

fn resolve_docker_load_path(cwd: &str, target: &str) -> Option<String> {
    let direct = normalize_fxfs_path(cwd, target)?;
    if fxfs_path_exists(direct.as_str()) {
        return Some(direct);
    }
    if path_under_shared(direct.as_str()) {
        let _ = crate::user_level::fxfs::ensure_host_share();
        if fxfs_path_exists(direct.as_str()) {
            return Some(direct);
        }
    }
    if target.starts_with('/') || target.contains('/') {
        return Some(direct);
    }

    let _ = crate::user_level::fxfs::ensure_host_share();
    let shared = normalize_fxfs_path("/shared", target)?;
    if fxfs_path_exists(shared.as_str()) {
        return Some(shared);
    }
    Some(direct)
}

fn print_docker_image_row(
    ctx: &mut ShellContext,
    image: &crate::user_level::docker_compat::DockerImageInfo,
) {
    let (repo, tag) = docker_repo_tag(image.name.as_str());
    ctx.serial.write_str(repo);
    ctx.serial.write_str("  ");
    ctx.serial.write_str(tag);
    ctx.serial.write_str("    ");
    print_number(&mut ctx.serial, image.layers as u32);
    ctx.serial.write_str("       ");
    print_number(&mut ctx.serial, image.config_bytes as u32);
    ctx.serial.write_str("B    ");
    ctx.serial.write_str(image.rootfs.as_str());
    ctx.serial.write_str("\n");
}

fn print_docker_load_result(
    ctx: &mut ShellContext,
    result: &crate::user_level::docker_compat::DockerImageLoadResult,
) {
    ctx.serial.write_str("image=");
    ctx.serial.write_str(result.image.name.as_str());
    ctx.serial.write_str(" source=");
    ctx.serial.write_str(match result.source {
        crate::user_level::docker_compat::DockerImageSource::Builtin => "builtin",
        crate::user_level::docker_compat::DockerImageSource::Archive => "archive",
        crate::user_level::docker_compat::DockerImageSource::HttpArchive => "http",
    });
    ctx.serial.write_str(" layers=");
    print_number(&mut ctx.serial, result.image.layers as u32);
    ctx.serial.write_str(" bytes=");
    print_usize(&mut ctx.serial, result.bytes);
    ctx.serial.write_str(" rootfs=");
    ctx.serial.write_str(result.image.rootfs.as_str());
}

fn docker_repo_tag(name: &str) -> (&str, &str) {
    if let Some(split) = name.rfind(':') {
        (&name[..split], &name[split + 1..])
    } else {
        (name, "latest")
    }
}

fn print_docker_container_row(
    ctx: &mut ShellContext,
    container: &crate::user_level::docker_compat::DockerContainer,
) {
    ctx.serial.write_str(container.id.as_str());
    ctx.serial.write_str("  ");
    ctx.serial.write_str(container.image.as_str());
    ctx.serial.write_str("  ");
    ctx.serial.write_str(container.status.as_str());
    ctx.serial.write_str("  ");
    ctx.serial.write_str(container.command.as_str());
    ctx.serial.write_str("\n");
}

fn print_docker_container_detail(
    ctx: &mut ShellContext,
    container: &crate::user_level::docker_compat::DockerContainer,
) {
    ctx.serial.write_str("Id: ");
    ctx.serial.write_str(container.id.as_str());
    ctx.serial.write_str("\nImage: ");
    ctx.serial.write_str(container.image.as_str());
    ctx.serial.write_str("\nCommand: ");
    ctx.serial.write_str(container.command.as_str());
    ctx.serial.write_str("\nState: ");
    ctx.serial.write_str(container.status.as_str());
    ctx.serial.write_str("\nExitCode: ");
    if container.exit_code < 0 {
        ctx.serial.write_str("-");
        print_number(&mut ctx.serial, container.exit_code.unsigned_abs());
    } else {
        print_number(&mut ctx.serial, container.exit_code as u32);
    }
    ctx.serial.write_str("\nLogBytes: ");
    print_number(&mut ctx.serial, container.log_bytes as u32);
    if let Some(runtime) = container.runtime {
        ctx.serial.write_str("\nRuntime:\n  job=0x");
        print_hex(&mut ctx.serial, runtime.job_handle as u64);
        ctx.serial.write_str("\n  process=0x");
        print_hex(&mut ctx.serial, runtime.process_handle as u64);
        ctx.serial.write_str("\n  thread=0x");
        print_hex(&mut ctx.serial, runtime.thread_handle as u64);
        ctx.serial.write_str("\n  namespaces=0x");
        print_hex(&mut ctx.serial, runtime.namespace_flags as u64);
        ctx.serial.write_str("\n  mounts=");
        print_number(&mut ctx.serial, runtime.mount_count as u32);
        ctx.serial.write_str("\n  seccomp=");
        print_number(&mut ctx.serial, runtime.seccomp_mode as u32);
        ctx.serial.write_str("\n  filters=");
        print_number(&mut ctx.serial, runtime.seccomp_filters as u32);
    }
    ctx.serial.write_str("\n");
}

/// Command: components - Show minimal component framework state
fn cmd_components(ctx: &mut ShellContext, _args: &[&str]) {
    let stats = crate::user_level::component::stats();
    ctx.serial.write_str("\nComponents: ");
    print_number(&mut ctx.serial, stats.components as u32);
    ctx.serial.write_str(" total, ");
    print_number(&mut ctx.serial, stats.started as u32);
    ctx.serial.write_str(" started\n");
    ctx.serial.write_str("Component threads: ");
    print_number(&mut ctx.serial, stats.runnable_threads as u32);
    ctx.serial.write_str("  Exited: ");
    print_number(&mut ctx.serial, stats.exited as u32);
    ctx.serial.write_str("  ELF loaded: ");
    print_number(&mut ctx.serial, stats.loaded_images as u32);
    ctx.serial.write_str("  Load errors: ");
    print_number(&mut ctx.serial, stats.load_errors as u32);
    ctx.serial.write_str("\n");

    ctx.serial.write_str(
        "  ID  State       PID   TID   Exit  Segs  Entry              Runner  Moniker\n",
    );
    ctx.serial.write_str(
        "  --------------------------------------------------------------------------\n",
    );
    for component in crate::user_level::component::snapshot() {
        ctx.serial.write_str("  ");
        print_number(&mut ctx.serial, component.id as u32);
        ctx.serial.write_str("   ");
        ctx.serial.write_str(component.state.as_str());
        for _ in 0..(11usize.saturating_sub(component.state.as_str().len())) {
            ctx.serial.write_byte(b' ');
        }
        match component.pid {
            Some(pid) => print_number(&mut ctx.serial, pid as u32),
            None => ctx.serial.write_str("-"),
        }
        ctx.serial.write_str("     ");
        match component.thread_id {
            Some(tid) => print_number(&mut ctx.serial, tid as u32),
            None => ctx.serial.write_str("-"),
        }
        ctx.serial.write_str("     ");
        if component.exited {
            print_number(&mut ctx.serial, component.exit_code as u32);
        } else {
            ctx.serial.write_str("-");
        }
        ctx.serial.write_str("     ");
        if component.loaded_segments > 0 {
            print_number(&mut ctx.serial, component.loaded_segments as u32);
        } else {
            ctx.serial.write_str("-");
        }
        ctx.serial.write_str("     ");
        match component.loaded_entry {
            Some(entry) => {
                ctx.serial.write_str("0x");
                print_hex(&mut ctx.serial, entry);
            }
            None => {
                ctx.serial.write_str("-");
                if let Some(err) = component.load_error {
                    ctx.serial.write_str("(");
                    ctx.serial.write_str(err.as_str());
                    ctx.serial.write_str(")");
                }
            }
        }
        ctx.serial.write_str("     ");
        ctx.serial.write_str(component.runner.as_str());
        ctx.serial.write_str("     ");
        ctx.serial.write_str(component.moniker);
        ctx.serial.write_str("\n");
    }
    ctx.serial.write_str("\n");
}

fn normalize_fxfs_path(cwd: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if !path.starts_with('/') {
        for part in cwd.split('/') {
            if !part.is_empty() {
                parts.push(part);
            }
        }
    }

    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            let _ = parts.pop();
            continue;
        }
        parts.push(part);
    }

    let mut normalized = String::from("/");
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            return None;
        }
        if index > 0 {
            normalized.push('/');
        }
        normalized.push_str(part);
    }
    Some(normalized)
}

fn split_completion_path(token: &str) -> (String, String) {
    match token.rfind('/') {
        Some(index) => {
            let (dir, name) = token.split_at(index + 1);
            let dir = if dir.is_empty() { "." } else { dir };
            (String::from(dir), String::from(name))
        }
        None => (String::from("."), String::from(token)),
    }
}

fn join_completion_path(dir_token: &str, name: &str) -> String {
    if dir_token == "." {
        return String::from(name);
    }
    let mut out = String::from(dir_token);
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(name);
    out
}

fn parse_ipv4(input: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut count = 0usize;
    for part in input.split('.') {
        if count >= 4 || part.is_empty() {
            return None;
        }
        let mut value = 0u32;
        for byte in part.as_bytes() {
            if *byte < b'0' || *byte > b'9' {
                return None;
            }
            value = user_logic::ipv4_octet_step(value, (*byte - b'0') as u32)?;
        }
        out[count] = value as u8;
        count += 1;
    }
    if count == 4 {
        Some(out)
    } else {
        None
    }
}

enum PingTarget<'a> {
    Address([u8; 4]),
    Host(&'a str),
    Invalid,
}

fn ping_target(input: Option<&str>) -> PingTarget<'_> {
    let Some(raw) = input else {
        return PingTarget::Address(crate::user_level::net::QEMU_USER_GATEWAY);
    };
    if let Some(ip) = parse_ipv4(raw) {
        return PingTarget::Address(ip);
    }
    if let Some((_scheme, host, _path)) = parse_url(raw) {
        return if host_valid(host) {
            PingTarget::Host(host)
        } else {
            PingTarget::Invalid
        };
    }
    if host_valid(raw) {
        PingTarget::Host(raw)
    } else {
        PingTarget::Invalid
    }
}

fn parse_url(url: &str) -> Option<(&str, &str, &str)> {
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    let rest = &url[scheme_end + 3..];
    if rest.is_empty() {
        return None;
    }
    match rest.find('/') {
        Some(path_start) if path_start > 0 => {
            Some((scheme, &rest[..path_start], &rest[path_start..]))
        }
        Some(_) => None,
        None => Some((scheme, rest, "/")),
    }
}

fn host_valid(host: &str) -> bool {
    if !user_logic::dns_host_len_valid(host.len()) || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    for label in host.split('.') {
        if !user_logic::dns_label_len_valid(label.len()) {
            return false;
        }
        for byte in label.as_bytes() {
            if !user_logic::dns_label_byte_valid(*byte) {
                return false;
            }
        }
    }
    true
}

fn path_under_shared(path: &str) -> bool {
    path == "/shared"
        || path
            .strip_prefix("/shared")
            .map(|suffix| suffix.starts_with('/'))
            .unwrap_or(false)
}

fn read_fxfs_file_to_vec(path: &str) -> Result<Vec<u8>, crate::user_level::fxfs::FxfsError> {
    let attrs = match crate::user_level::fxfs::attrs(path) {
        Ok(attrs) => attrs,
        Err(crate::user_level::fxfs::FxfsError::NotFound) if path_under_shared(path) => {
            let _ = crate::user_level::fxfs::ensure_host_share();
            crate::user_level::fxfs::attrs(path)?
        }
        Err(err) => return Err(err),
    };
    let mut out = Vec::new();
    out.resize(attrs.size, 0);
    let size = crate::user_level::fxfs::read_file(path, &mut out)?;
    out.truncate(size);
    Ok(out)
}

fn fxfs_path_exists(path: &str) -> bool {
    crate::user_level::fxfs::attrs(path).is_ok()
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn fxfs_child_path(parent: &str, name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return None;
    }
    let mut out = String::from(parent.trim_end_matches('/'));
    if out.is_empty() {
        out.push('/');
    }
    if out != "/" {
        out.push('/');
    }
    out.push_str(name);
    Some(out)
}

fn resolve_copy_destination(
    src: &str,
    dst: &str,
) -> Result<String, crate::user_level::fxfs::FxfsError> {
    match crate::user_level::fxfs::entries(dst) {
        Ok(_) => {
            let name = basename(src);
            fxfs_child_path(dst, name).ok_or(crate::user_level::fxfs::FxfsError::InvalidPath)
        }
        Err(crate::user_level::fxfs::FxfsError::NotDirectory)
        | Err(crate::user_level::fxfs::FxfsError::NotFound) => Ok(String::from(dst)),
        Err(err) => Err(err),
    }
}

fn resolve_run_path(cwd: &str, target: &str) -> Option<String> {
    let direct = normalize_fxfs_path(cwd, target)?;
    if fxfs_path_exists(direct.as_str()) {
        return Some(direct);
    }

    if path_under_shared(direct.as_str()) {
        let _ = crate::user_level::fxfs::ensure_host_share();
        if fxfs_path_exists(direct.as_str()) {
            return Some(direct);
        }
    } else if !target.contains('/') {
        let _ = crate::user_level::fxfs::ensure_host_share();
        let shared = normalize_fxfs_path("/shared", target)?;
        if fxfs_path_exists(shared.as_str()) {
            return Some(shared);
        }
    }

    Some(direct)
}

fn resolve_run_library_path(name_or_path: &str) -> Option<String> {
    if name_or_path.starts_with('/') && fxfs_path_exists(name_or_path) {
        return Some(String::from(name_or_path));
    }

    let name = basename(name_or_path);
    let _ = crate::user_level::fxfs::ensure_host_share();

    let mut shared = String::from("/shared/lib/");
    shared.push_str(name);
    if fxfs_path_exists(shared.as_str()) {
        return Some(shared);
    }

    let mut lib = String::from("/lib/");
    lib.push_str(name);
    if fxfs_path_exists(lib.as_str()) {
        return Some(lib);
    }

    None
}

fn print_bytes_as_text(ctx: &mut ShellContext, bytes: &[u8]) {
    for byte in bytes {
        if user_logic::ascii_shell_input(*byte) || *byte == b'\n' || *byte == b'\t' {
            ctx.serial.write_byte(*byte);
        } else {
            ctx.serial.write_byte(b'.');
        }
    }
}

fn print_fxfs_error(ctx: &mut ShellContext, err: crate::user_level::fxfs::FxfsError) {
    let label = match err {
        crate::user_level::fxfs::FxfsError::NotMounted => "not mounted",
        crate::user_level::fxfs::FxfsError::InvalidPath => "invalid path",
        crate::user_level::fxfs::FxfsError::NotFound => "not found",
        crate::user_level::fxfs::FxfsError::AlreadyExists => "already exists",
        crate::user_level::fxfs::FxfsError::NoSpace => "no space",
        crate::user_level::fxfs::FxfsError::NotDirectory => "not a directory",
        crate::user_level::fxfs::FxfsError::IsDirectory => "is a directory",
        crate::user_level::fxfs::FxfsError::NotFile => "not a file",
        crate::user_level::fxfs::FxfsError::InvalidOffset => "invalid offset",
        crate::user_level::fxfs::FxfsError::StorageUnavailable => "storage unavailable",
        crate::user_level::fxfs::FxfsError::StorageCorrupt => "storage corrupt",
    };
    ctx.serial.write_str(label);
}

fn print_elf_error(ctx: &mut ShellContext, err: crate::user_level::elf::ElfError) {
    ctx.serial.write_str(err.as_str());
}

fn print_driver_error(ctx: &mut ShellContext, err: crate::user_level::drivers::UserDriverError) {
    let label = match err {
        crate::user_level::drivers::UserDriverError::NotInitialized => "not initialized",
        crate::user_level::drivers::UserDriverError::NotFound => "not found",
        crate::user_level::drivers::UserDriverError::NotReady => "not ready",
        crate::user_level::drivers::UserDriverError::OutOfRange => "out of range",
        crate::user_level::drivers::UserDriverError::InvalidBlock => "invalid block",
        crate::user_level::drivers::UserDriverError::Unsupported => "unsupported",
        crate::user_level::drivers::UserDriverError::Io => "io error",
        crate::user_level::drivers::UserDriverError::Timeout => "timeout",
    };
    ctx.serial.write_str(label);
}

fn print_net_error(ctx: &mut ShellContext, err: crate::user_level::net::NetError) {
    let label = match err {
        crate::user_level::net::NetError::Driver(driver_err) => {
            print_driver_error(ctx, driver_err);
            return;
        }
        crate::user_level::net::NetError::NotReady => "not ready",
        crate::user_level::net::NetError::InvalidHost => "invalid host",
        crate::user_level::net::NetError::InvalidUrl => "invalid url",
        crate::user_level::net::NetError::BufferTooSmall => "buffer too small",
        crate::user_level::net::NetError::MalformedPacket => "malformed packet",
        crate::user_level::net::NetError::Timeout => "timeout",
        crate::user_level::net::NetError::NoAddress => "no address",
        crate::user_level::net::NetError::Unsupported => "unsupported",
        crate::user_level::net::NetError::ConnectionReset => "connection reset",
        crate::user_level::net::NetError::TlsUnsupported => "tls unsupported",
    };
    ctx.serial.write_str(label);
}

fn print_vm_host_error(ctx: &mut ShellContext, err: crate::user_level::vm_host::VmHostError) {
    match err {
        crate::user_level::vm_host::VmHostError::Connect(net_err) => {
            ctx.serial.write_str("connect ");
            print_net_error(ctx, net_err);
        }
        crate::user_level::vm_host::VmHostError::Write(net_err) => {
            ctx.serial.write_str("write ");
            print_net_error(ctx, net_err);
        }
        crate::user_level::vm_host::VmHostError::Read(net_err) => {
            ctx.serial.write_str("read ");
            print_net_error(ctx, net_err);
        }
        crate::user_level::vm_host::VmHostError::NoHostConfig => {
            ctx.serial.write_str("no host config")
        }
        crate::user_level::vm_host::VmHostError::InvalidConfig => {
            ctx.serial.write_str("invalid launch config")
        }
        crate::user_level::vm_host::VmHostError::RequestTooLarge => {
            ctx.serial.write_str("request too large")
        }
        crate::user_level::vm_host::VmHostError::ResponseInvalid => {
            ctx.serial.write_str("invalid launcher response")
        }
        crate::user_level::vm_host::VmHostError::LaunchDenied => {
            ctx.serial.write_str("launcher denied request")
        }
    }
}

fn print_vm_host_hint(ctx: &mut ShellContext, err: crate::user_level::vm_host::VmHostError) {
    match err {
        crate::user_level::vm_host::VmHostError::Connect(_) => {
            ctx.serial
                .write_str("\n  host launcher unreachable; run: scripts/smros-vm-launcher.py\n");
        }
        crate::user_level::vm_host::VmHostError::Write(_)
        | crate::user_level::vm_host::VmHostError::Read(_) => {
            ctx.serial.write_str(
                "\n  host launcher connection failed mid-request; restart scripts/smros-vm-launcher.py\n",
            );
        }
        crate::user_level::vm_host::VmHostError::LaunchDenied => {
            ctx.serial.write_str(
                "\n  host launcher replied; check smros-vm-launcher.log or its terminal for missing kernel/initrd/disk paths\n",
            );
        }
        crate::user_level::vm_host::VmHostError::ResponseInvalid => {
            ctx.serial.write_str(
                "\n  host launcher response was malformed; check smros-vm-launcher.log\n",
            );
        }
        _ => ctx.serial.write_str("\n"),
    }
}

fn print_fxfs_entry(ctx: &mut ShellContext, entry: &crate::user_level::fxfs::FxfsDirEntry) {
    ctx.serial.write_str("  ");
    ctx.serial.write_str(entry.kind.as_str());
    ctx.serial.write_str("  ");
    print_number(&mut ctx.serial, entry.object_id as u32);
    ctx.serial.write_str("      ");
    print_number(&mut ctx.serial, entry.size as u32);
    ctx.serial.write_str("    0");
    print_octal(&mut ctx.serial, entry.attrs.mode);
    ctx.serial.write_str("  ");
    print_number(&mut ctx.serial, entry.attrs.link_count);
    ctx.serial.write_str("      ");
    print_number(&mut ctx.serial, entry.attrs.uid);
    ctx.serial.write_str(":");
    print_number(&mut ctx.serial, entry.attrs.gid);
    ctx.serial.write_str("   ");
    ctx.serial.write_str(entry.name.as_str());
    ctx.serial.write_str("\n");
}

fn cmd_pwd(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial.write_str(ctx.cwd.as_str());
    ctx.serial.write_str("\n");
}

fn cmd_ls(ctx: &mut ShellContext, args: &[&str]) {
    let target = args.first().copied().unwrap_or(".");
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("ls: invalid path\n");
        return;
    };

    match crate::user_level::fxfs::entries(path.as_str()) {
        Ok(entries) => {
            ctx.serial
                .write_str("  Kind  Object  Size  Mode    Links  Owner  Name\n");
            for entry in &entries {
                print_fxfs_entry(ctx, entry);
            }
        }
        Err(crate::user_level::fxfs::FxfsError::NotDirectory) => {
            match crate::user_level::fxfs::attrs(path.as_str()) {
                Ok(attrs) => {
                    ctx.serial.write_str("  file  ");
                    print_number(&mut ctx.serial, attrs.size as u32);
                    ctx.serial.write_str(" bytes  ");
                    ctx.serial.write_str(path.as_str());
                    ctx.serial.write_str("\n");
                }
                Err(err) => {
                    ctx.serial.write_str("ls: ");
                    print_fxfs_error(ctx, err);
                    ctx.serial.write_str("\n");
                }
            }
        }
        Err(err) => {
            ctx.serial.write_str("ls: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_cd(ctx: &mut ShellContext, args: &[&str]) {
    let target = args.first().copied().unwrap_or("/");
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("cd: invalid path\n");
        return;
    };

    match crate::user_level::fxfs::entries(path.as_str()) {
        Ok(_) => {
            ctx.cwd = path;
        }
        Err(err) => {
            ctx.serial.write_str("cd: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_cd_up(ctx: &mut ShellContext, _args: &[&str]) {
    cmd_cd(ctx, &[".."]);
}

fn cmd_mkdir(ctx: &mut ShellContext, args: &[&str]) {
    let Some(target) = args.first() else {
        ctx.serial.write_str("mkdir: missing path\n");
        return;
    };
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("mkdir: invalid path\n");
        return;
    };
    match crate::user_level::fxfs::create_dir(path.as_str()) {
        Ok(_) => {
            ctx.serial.write_str("created ");
            ctx.serial.write_str(path.as_str());
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("mkdir: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_write(ctx: &mut ShellContext, args: &[&str]) {
    let Some(target) = args.first() else {
        ctx.serial.write_str("write: missing path\n");
        return;
    };
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("write: invalid path\n");
        return;
    };

    let mut data = String::new();
    for (index, arg) in args.iter().skip(1).enumerate() {
        if index > 0 {
            data.push(' ');
        }
        data.push_str(arg);
    }

    match crate::user_level::fxfs::write_file(path.as_str(), data.as_bytes()) {
        Ok(size) => {
            ctx.serial.write_str("wrote ");
            print_number(&mut ctx.serial, size as u32);
            ctx.serial.write_str(" bytes to ");
            ctx.serial.write_str(path.as_str());
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("write: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_cat(ctx: &mut ShellContext, args: &[&str]) {
    let Some(target) = args.first() else {
        ctx.serial.write_str("cat: missing path\n");
        return;
    };
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("cat: invalid path\n");
        return;
    };
    match read_fxfs_file_to_vec(path.as_str()) {
        Ok(out) => {
            print_bytes_as_text(ctx, &out);
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("cat: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_cp(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() < 2 {
        ctx.serial
            .write_str("cp: expected source and destination\n");
        return;
    }
    let Some(src) = normalize_fxfs_path(ctx.cwd.as_str(), args[0]) else {
        ctx.serial.write_str("cp: invalid source path\n");
        return;
    };
    let Some(dst) = normalize_fxfs_path(ctx.cwd.as_str(), args[1]) else {
        ctx.serial.write_str("cp: invalid destination path\n");
        return;
    };

    let data = match read_fxfs_file_to_vec(src.as_str()) {
        Ok(data) => data,
        Err(err) => {
            ctx.serial.write_str("cp: source ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
            return;
        }
    };
    let dst = match resolve_copy_destination(src.as_str(), dst.as_str()) {
        Ok(dst) => dst,
        Err(err) => {
            ctx.serial.write_str("cp: destination ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
            return;
        }
    };
    match crate::user_level::fxfs::write_file(dst.as_str(), &data) {
        Ok(size) => {
            ctx.serial.write_str("copied ");
            print_number(&mut ctx.serial, size as u32);
            ctx.serial.write_str(" bytes to ");
            ctx.serial.write_str(dst.as_str());
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("cp: destination ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_mv(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() < 2 {
        ctx.serial
            .write_str("mv: expected source and destination\n");
        return;
    }
    let Some(src) = normalize_fxfs_path(ctx.cwd.as_str(), args[0]) else {
        ctx.serial.write_str("mv: invalid source path\n");
        return;
    };
    let Some(dst) = normalize_fxfs_path(ctx.cwd.as_str(), args[1]) else {
        ctx.serial.write_str("mv: invalid destination path\n");
        return;
    };
    if src == dst {
        ctx.serial
            .write_str("mv: source and destination are the same\n");
        return;
    }

    let data = match read_fxfs_file_to_vec(src.as_str()) {
        Ok(data) => data,
        Err(err) => {
            ctx.serial.write_str("mv: source ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
            return;
        }
    };
    if let Err(err) = crate::user_level::fxfs::write_file(dst.as_str(), &data) {
        ctx.serial.write_str("mv: destination ");
        print_fxfs_error(ctx, err);
        ctx.serial.write_str("\n");
        return;
    }
    match crate::user_level::fxfs::delete_file(src.as_str()) {
        Ok(()) => {
            ctx.serial.write_str("moved ");
            ctx.serial.write_str(src.as_str());
            ctx.serial.write_str(" to ");
            ctx.serial.write_str(dst.as_str());
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial
                .write_str("mv: copied but could not remove source: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_rm(ctx: &mut ShellContext, args: &[&str]) {
    let Some(target) = args.first() else {
        ctx.serial.write_str("rm: missing path\n");
        return;
    };
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("rm: invalid path\n");
        return;
    };

    match crate::user_level::fxfs::delete_file(path.as_str()) {
        Ok(()) => {
            ctx.serial.write_str("removed ");
            ctx.serial.write_str(path.as_str());
            ctx.serial.write_str("\n");
        }
        Err(err) => {
            ctx.serial.write_str("rm: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_run(ctx: &mut ShellContext, args: &[&str]) {
    let Some(target) = args.first() else {
        ctx.serial.write_str("run: missing elf path\n");
        ctx.serial.write_str("usage: run <elf-path> [args...]\n");
        return;
    };
    let Some(path) = resolve_run_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("run: invalid path\n");
        return;
    };

    let image = match crate::user_level::elf::load_from_fxfs(path.as_str()) {
        Ok(image) => image,
        Err(err) => {
            ctx.serial.write_str("run: ELF ");
            print_elf_error(ctx, err);
            ctx.serial.write_str("\n");
            return;
        }
    };

    ctx.serial.write_str("run: ");
    ctx.serial.write_str(path.as_str());
    ctx.serial.write_str("\n  type: ");
    ctx.serial
        .write_str(if image.dynamic { "dynamic" } else { "static" });
    if image.elf_type == crate::user_level::elf::ELF_TYPE_DYN {
        ctx.serial.write_str(" PIE");
    }
    ctx.serial.write_str("\n  entry: 0x");
    print_hex(&mut ctx.serial, image.entry);
    ctx.serial.write_str("\n  load segments: ");
    print_usize(&mut ctx.serial, image.segments.len());
    ctx.serial.write_str("\n");

    if let Some(interpreter) = image.interpreter.as_ref() {
        let Some(resolved) = resolve_run_library_path(interpreter.as_str()) else {
            ctx.serial
                .write_str("run: ELF dynamic-interpreter-missing: ");
            ctx.serial.write_str(interpreter.as_str());
            ctx.serial
                .write_str(" (copy the loader into /shared/lib or /lib)\n");
            return;
        };
        ctx.serial.write_str("  interpreter: ");
        ctx.serial.write_str(interpreter.as_str());
        if resolved.as_str() != interpreter.as_str() {
            ctx.serial.write_str(" -> ");
            ctx.serial.write_str(resolved.as_str());
        }
        ctx.serial.write_str("\n");
    }

    if !image.needed.is_empty() {
        ctx.serial.write_str("  needed:\n");
        for needed in &image.needed {
            let Some(resolved) = resolve_run_library_path(needed.as_str()) else {
                ctx.serial
                    .write_str("run: ELF dynamic-dependency-missing: ");
                ctx.serial.write_str(needed.as_str());
                ctx.serial
                    .write_str(" (copy DT_NEEDED libraries into /shared/lib or /lib)\n");
                return;
            };
            ctx.serial.write_str("    ");
            ctx.serial.write_str(needed.as_str());
            ctx.serial.write_str(" -> ");
            ctx.serial.write_str(resolved.as_str());
            ctx.serial.write_str("\n");
        }
    }

    let mut argv = Vec::new();
    argv.push(path.clone());
    for arg in args.iter().skip(1) {
        argv.push(String::from(*arg));
    }

    match crate::user_level::run_elf::spawn(path.clone(), argv) {
        Ok(()) => {
            ctx.serial.write_str("run: started dynamic loader thread\n");
            ctx.serial
                .write_str("run: program output begins below; exit code 0 means success\n");
        }
        Err(err) => {
            ctx.serial.write_str("run: ELF launch-failed: ");
            ctx.serial.write_str(err.as_str());
            ctx.serial.write_str("\n");
            return;
        }
    }
    scheduler::yield_now();
}

fn cmd_vi(ctx: &mut ShellContext, args: &[&str]) {
    let Some(target) = args.first() else {
        ctx.serial.write_str("vi: missing path\n");
        return;
    };
    let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), target) else {
        ctx.serial.write_str("vi: invalid path\n");
        return;
    };

    ctx.serial.write_str("\n--- ");
    ctx.serial.write_str(path.as_str());
    ctx.serial.write_str(" ---\n");
    let mut buffer = match read_fxfs_file_to_vec(path.as_str()) {
        Ok(existing) if !existing.is_empty() => {
            print_bytes_as_text(ctx, &existing);
            if existing.last().copied() != Some(b'\n') {
                ctx.serial.write_str("\n");
            }
            existing
        }
        Ok(existing) => existing,
        Err(crate::user_level::fxfs::FxfsError::NotFound) => Vec::new(),
        Err(err) => {
            ctx.serial.write_str("vi: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
            return;
        }
    };

    ctx.serial
        .write_str("--- enter text, :wq saves, :q cancels, :p prints buffer ---\n");
    loop {
        ctx.serial.write_str("vi> ");
        let line = ctx.read_line();
        if line == ":q" {
            ctx.serial.write_str("vi: canceled\n");
            return;
        }
        if line == ":p" {
            print_bytes_as_text(ctx, &buffer);
            if buffer.last().copied() != Some(b'\n') {
                ctx.serial.write_str("\n");
            }
            continue;
        }
        if line == ":wq" {
            match crate::user_level::fxfs::write_file(path.as_str(), &buffer) {
                Ok(size) => {
                    ctx.serial.write_str("vi: wrote ");
                    print_number(&mut ctx.serial, size as u32);
                    ctx.serial.write_str(" bytes\n");
                }
                Err(err) => {
                    ctx.serial.write_str("vi: ");
                    print_fxfs_error(ctx, err);
                    ctx.serial.write_str("\n");
                }
            }
            return;
        }
        if buffer.len().saturating_add(line.len()).saturating_add(1)
            > user_logic::USER_FXFS_MAX_FILE_BYTES
        {
            ctx.serial.write_str("vi: buffer full\n");
            continue;
        }
        buffer.extend_from_slice(line.as_bytes());
        buffer.push(b'\n');
    }
}

#[derive(Clone, Copy, Default)]
struct FxfsTreeCounts {
    files: usize,
    dirs: usize,
    entries: usize,
}

fn count_fxfs_tree(path: &str, counts: &mut FxfsTreeCounts) {
    match crate::user_level::fxfs::entries(path) {
        Ok(entries) => {
            for entry in entries {
                counts.entries = counts.entries.saturating_add(1);
                match entry.kind {
                    crate::user_level::fxfs::FxfsNodeKind::Directory => {
                        counts.dirs = counts.dirs.saturating_add(1);
                        if let Some(child) = fxfs_child_path(path, entry.name.as_str()) {
                            count_fxfs_tree(child.as_str(), counts);
                        }
                    }
                    crate::user_level::fxfs::FxfsNodeKind::File => {
                        counts.files = counts.files.saturating_add(1);
                    }
                }
            }
        }
        Err(crate::user_level::fxfs::FxfsError::NotDirectory) => {
            counts.files = counts.files.saturating_add(1);
            counts.entries = counts.entries.saturating_add(1);
        }
        Err(_) => {}
    }
}

fn shared_live_tree_counts() -> FxfsTreeCounts {
    let mut counts = FxfsTreeCounts::default();
    count_fxfs_tree("/shared", &mut counts);
    counts
}

fn print_host_share_summary(ctx: &mut ShellContext) {
    let live = shared_live_tree_counts();
    ctx.serial.write_str("live files=");
    print_usize(&mut ctx.serial, live.files);
    ctx.serial.write_str(" dirs=");
    print_usize(&mut ctx.serial, live.dirs);
    ctx.serial.write_str(" entries=");
    print_usize(&mut ctx.serial, live.entries);
    ctx.serial.write_str("; embedded files=");
    print_usize(
        &mut ctx.serial,
        crate::user_level::host_share::HOST_SHARE_FILES.len(),
    );
    ctx.serial.write_str(" dirs=");
    print_usize(
        &mut ctx.serial,
        crate::user_level::host_share::HOST_SHARE_DIRS.len(),
    );
    ctx.serial.write_str(" skipped=");
    print_usize(
        &mut ctx.serial,
        crate::user_level::host_share::HOST_SHARE_SKIPPED.len(),
    );
}

fn print_host_share_skipped(ctx: &mut ShellContext) {
    if crate::user_level::host_share::HOST_SHARE_SKIPPED.is_empty() {
        return;
    }

    ctx.serial.write_str("\nSkipped host_shared files:\n");
    for skipped in crate::user_level::host_share::HOST_SHARE_SKIPPED {
        ctx.serial.write_str("  ");
        ctx.serial.write_str(skipped.path);
        ctx.serial.write_str("  ");
        ctx.serial.write_str(skipped.reason);
        ctx.serial.write_str("  ");
        print_usize(&mut ctx.serial, skipped.size);
        ctx.serial.write_str(" bytes\n");
    }
}

/// Command: mount - Show mounts or refresh the embedded host_shared seed
fn cmd_mount(ctx: &mut ShellContext, args: &[&str]) {
    if args.is_empty() {
        let stats = crate::user_level::fxfs::stats();
        ctx.serial.write_str("\nMounted filesystems:\n");
        ctx.serial.write_str("  fxfs on / type fxfs");
        if stats.block_backed {
            ctx.serial.write_str(" (block-backed");
            if stats.last_sync_ok {
                ctx.serial.write_str(", synced");
            } else {
                ctx.serial.write_str(", not synced");
            }
            ctx.serial.write_str(")");
        } else {
            ctx.serial.write_str(" (memory)");
        }
        ctx.serial
            .write_str("\n  host_shared on /shared type fxfs.snapshot+overlay (FxFS-local)\n\n");
        ctx.serial
            .write_str("The embedded host_shared seed is installed during FxFS initialization.\n");
        ctx.serial
            .write_str("Local copies and edits under /shared are stored in the FxFS overlay.\n");
        ctx.serial.write_str(
            "Use: mount share    refresh missing embedded files while preserving the overlay\n",
        );
        ctx.serial
            .write_str("Live host directory sharing needs a 9p or virtio-fs guest driver.\n\n");
        return;
    }

    match args[0] {
        "share" | "shared" | "/shared" | "host_shared" => {
            match crate::user_level::fxfs::mount_host_share() {
                Ok(()) => {
                    ctx.serial.write_str("refreshed /shared embedded seed (");
                    print_host_share_summary(ctx);
                    ctx.serial.write_str(")\n");
                    print_host_share_skipped(ctx);
                }
                Err(err) => {
                    ctx.serial.write_str("mount: ");
                    print_fxfs_error(ctx, err);
                    ctx.serial.write_str("\n");
                }
            }
        }
        _ => {
            ctx.serial
                .write_str("usage: mount [share|shared|/shared|host_shared]\n");
        }
    }
}

/// Command: share - List the host_shared seed plus FxFS overlay under /shared
fn cmd_share(ctx: &mut ShellContext, args: &[&str]) {
    let mut target_index = 0usize;
    if args.first().copied() == Some("refresh") {
        match crate::user_level::fxfs::mount_host_share() {
            Ok(()) => {
                ctx.serial
                    .write_str("refreshed /shared embedded seed, preserving local overlay\n");
            }
            Err(err) => {
                ctx.serial.write_str("share: refresh ");
                print_fxfs_error(ctx, err);
                ctx.serial.write_str("\n");
                return;
            }
        }
        target_index = 1;
    } else if let Err(err) = crate::user_level::fxfs::ensure_host_share() {
        ctx.serial.write_str("share: ");
        print_fxfs_error(ctx, err);
        ctx.serial.write_str("\n");
        return;
    }

    let target = args.get(target_index).copied().unwrap_or("/shared");
    let base = if target.starts_with('/') {
        ctx.cwd.as_str()
    } else {
        "/shared"
    };
    let Some(path) = normalize_fxfs_path(base, target) else {
        ctx.serial.write_str("share: invalid path\n");
        return;
    };

    ctx.serial.write_str("\n/shared FxFS view (");
    print_host_share_summary(ctx);
    ctx.serial.write_str(")\n");

    match crate::user_level::fxfs::entries(path.as_str()) {
        Ok(entries) => {
            ctx.serial
                .write_str("  Kind  Object  Size  Mode    Links  Owner  Name\n");
            for entry in &entries {
                print_fxfs_entry(ctx, entry);
            }
            print_host_share_skipped(ctx);
        }
        Err(crate::user_level::fxfs::FxfsError::NotDirectory) => {
            match crate::user_level::fxfs::attrs(path.as_str()) {
                Ok(attrs) => {
                    ctx.serial.write_str("  file  ");
                    print_usize(&mut ctx.serial, attrs.size);
                    ctx.serial.write_str(" bytes  ");
                    ctx.serial.write_str(path.as_str());
                    ctx.serial.write_str("\n");
                }
                Err(err) => {
                    ctx.serial.write_str("share: ");
                    print_fxfs_error(ctx, err);
                    ctx.serial.write_str("\n");
                }
            }
        }
        Err(err) => {
            ctx.serial.write_str("share: ");
            print_fxfs_error(ctx, err);
            ctx.serial.write_str("\n");
        }
    }
}

/// Command: fxfs - Show minimal FxFS object-store state
fn cmd_fxfs(ctx: &mut ShellContext, _args: &[&str]) {
    let stats = crate::user_level::fxfs::stats();
    ctx.serial.write_str("\nFxFS mounted: ");
    ctx.serial
        .write_str(if stats.mounted { "yes" } else { "no" });
    ctx.serial.write_str("  Block backed: ");
    ctx.serial
        .write_str(if stats.block_backed { "yes" } else { "no" });
    ctx.serial.write_str("  Last sync: ");
    ctx.serial.write_str(if stats.last_sync_ok {
        "ok"
    } else {
        "not synced"
    });
    if let Some(err) = stats.last_storage_error {
        ctx.serial.write_str(" (");
        print_fxfs_error(ctx, err);
        ctx.serial.write_str(")");
    }
    ctx.serial.write_str("\nBlock bytes: ");
    print_number(&mut ctx.serial, stats.block_bytes as u32);
    ctx.serial.write_str("  Slots: ");
    print_number(&mut ctx.serial, stats.storage_slots as u32);
    ctx.serial.write_str("  Active: ");
    print_number(&mut ctx.serial, stats.active_slot as u32);
    ctx.serial.write_str("  Slot bytes: ");
    print_number(&mut ctx.serial, stats.slot_bytes as u32);
    ctx.serial.write_str("\nNodes: ");
    print_number(&mut ctx.serial, stats.nodes as u32);
    ctx.serial.write_str("  Dirs: ");
    print_number(&mut ctx.serial, stats.directories as u32);
    ctx.serial.write_str("  Files: ");
    print_number(&mut ctx.serial, stats.files as u32);
    ctx.serial.write_str("  Dirents: ");
    print_number(&mut ctx.serial, stats.dir_entries as u32);
    ctx.serial.write_str("  Bytes: ");
    print_number(&mut ctx.serial, stats.bytes as u32);
    ctx.serial.write_str("\nJournal records: ");
    print_number(&mut ctx.serial, stats.journal_records as u32);
    ctx.serial.write_str("  Replayed: ");
    print_number(&mut ctx.serial, stats.replayed_records as u32);
    ctx.serial.write_str("  Sequence: ");
    print_number(&mut ctx.serial, stats.sequence as u32);
    ctx.serial.write_str("\n\nCurrent directory: ");
    ctx.serial.write_str(ctx.cwd.as_str());
    ctx.serial
        .write_str("\n\n/pkg/bin:\n  Kind  Object  Size  Mode    Links  Owner  Name\n");

    match crate::user_level::fxfs::entries("/pkg/bin") {
        Ok(entries) => {
            for entry in entries {
                print_fxfs_entry(ctx, &entry);
            }
        }
        Err(_) => ctx.serial.write_str("  <unavailable>\n"),
    }
    ctx.serial.write_str("\n");
}

/// Command: drivers - Show user-space device tree and driver state
fn cmd_drivers(ctx: &mut ShellContext, _args: &[&str]) {
    let stats = crate::user_level::drivers::stats();
    ctx.serial.write_str("\nUser driver framework: ");
    ctx.serial.write_str(if stats.initialized {
        "ready"
    } else {
        "not ready"
    });
    ctx.serial.write_str("  Machine: ");
    ctx.serial.write_str(stats.machine);
    ctx.serial.write_str("\nDevice-tree nodes: ");
    print_number(&mut ctx.serial, stats.nodes as u32);
    ctx.serial.write_str("  Bindings: ");
    print_number(&mut ctx.serial, stats.bindings as u32);
    ctx.serial.write_str("\nBlock vblk0: ");
    ctx.serial.write_str(if stats.block_ready {
        "ready"
    } else {
        "not ready"
    });
    ctx.serial.write_str("  blocks=");
    print_number(&mut ctx.serial, stats.block_count as u32);
    ctx.serial.write_str("  block_size=");
    print_number(&mut ctx.serial, stats.block_size as u32);
    ctx.serial.write_str("  bytes=");
    print_number(&mut ctx.serial, stats.bytes as u32);
    ctx.serial.write_str("  mmio=0x");
    print_hex(&mut ctx.serial, stats.mmio_base as u64);
    ctx.serial.write_str("  status=0x");
    print_hex(&mut ctx.serial, stats.device_status as u64);
    if let Some(err) = stats.last_error {
        ctx.serial.write_str("  last_error=");
        print_driver_error(ctx, err);
    }
    ctx.serial.write_str("\nI/O: reads=");
    print_number(&mut ctx.serial, stats.reads as u32);
    ctx.serial.write_str(" writes=");
    print_number(&mut ctx.serial, stats.writes as u32);
    ctx.serial.write_str(" flushes=");
    print_number(&mut ctx.serial, stats.flushes as u32);
    ctx.serial.write_str(" bytes_read=");
    print_number(&mut ctx.serial, stats.bytes_read as u32);
    ctx.serial.write_str(" bytes_written=");
    print_number(&mut ctx.serial, stats.bytes_written as u32);
    ctx.serial.write_str("\nNetwork eth0: ");
    ctx.serial.write_str(if stats.net_ready {
        "ready"
    } else {
        "not ready"
    });
    ctx.serial.write_str("  link=");
    ctx.serial
        .write_str(if stats.net_link_up { "up" } else { "down" });
    ctx.serial.write_str("  mtu=");
    print_number(&mut ctx.serial, stats.net_mtu as u32);
    ctx.serial.write_str("  mac=");
    print_mac(&mut ctx.serial, stats.net_mac);
    ctx.serial.write_str("  mmio=0x");
    print_hex(&mut ctx.serial, stats.net_mmio_base as u64);
    ctx.serial.write_str("  status=0x");
    print_hex(&mut ctx.serial, stats.net_device_status as u64);
    if let Some(err) = stats.net_last_error {
        ctx.serial.write_str("  last_error=");
        print_driver_error(ctx, err);
    }
    ctx.serial.write_str("\nNet I/O: rx_packets=");
    print_number(&mut ctx.serial, stats.net_rx_packets as u32);
    ctx.serial.write_str(" tx_packets=");
    print_number(&mut ctx.serial, stats.net_tx_packets as u32);
    ctx.serial.write_str(" rx_bytes=");
    print_number(&mut ctx.serial, stats.net_rx_bytes as u32);
    ctx.serial.write_str(" tx_bytes=");
    print_number(&mut ctx.serial, stats.net_tx_bytes as u32);
    ctx.serial.write_str(" dropped=");
    print_number(&mut ctx.serial, stats.net_dropped_packets as u32);
    ctx.serial.write_str("\n\nNodes:\n");

    for node in crate::user_level::drivers::device_nodes() {
        ctx.serial.write_str("  ");
        ctx.serial.write_str(node.kind.as_str());
        ctx.serial.write_str("  ");
        ctx.serial.write_str(node.compatible);
        ctx.serial.write_str("  ");
        ctx.serial.write_str(node.path);
        if let Some(reg) = node.reg {
            ctx.serial.write_str("  reg=0x");
            print_hex(&mut ctx.serial, reg.base);
            ctx.serial.write_str("+0x");
            print_hex(&mut ctx.serial, reg.size);
        }
        if let Some(irq) = node.irq {
            ctx.serial.write_str("  irq=");
            print_number(&mut ctx.serial, irq);
        }
        ctx.serial.write_str("\n");
    }

    ctx.serial.write_str("\nBindings:\n");
    for binding in crate::user_level::drivers::bindings() {
        ctx.serial.write_str("  ");
        ctx.serial.write_str(binding.device_name);
        ctx.serial.write_str("  ");
        ctx.serial.write_str(binding.driver);
        ctx.serial.write_str("  <- ");
        ctx.serial.write_str(binding.node_path);
        if binding.kind == crate::user_level::drivers::UserDeviceKind::Network {
            ctx.serial.write_str("  mtu=");
            print_number(&mut ctx.serial, binding.mtu as u32);
            ctx.serial.write_str("  mac=");
            print_mac(&mut ctx.serial, binding.mac);
        }
        ctx.serial.write_str("\n");
    }
    ctx.serial.write_str("\n");
}

/// Command: ifconfig - Show network interface state
fn cmd_ifconfig(ctx: &mut ShellContext, _args: &[&str]) {
    let cfg = crate::user_level::net::config();
    ctx.serial.write_str("\neth0: ");
    ctx.serial
        .write_str(if crate::user_level::drivers::net_ready() {
            "ready"
        } else {
            "not ready"
        });
    ctx.serial.write_str("  link=");
    ctx.serial
        .write_str(if cfg.link_up { "up" } else { "down" });
    ctx.serial.write_str("  mtu=");
    print_number(&mut ctx.serial, cfg.mtu as u32);
    ctx.serial.write_str("\n  mac ");
    print_mac(&mut ctx.serial, cfg.mac);
    ctx.serial.write_str("  inet ");
    print_ipv4(&mut ctx.serial, cfg.ip);
    ctx.serial.write_str("  gateway ");
    print_ipv4(&mut ctx.serial, cfg.gateway);
    ctx.serial.write_str("  dns ");
    print_ipv4(&mut ctx.serial, cfg.dns);
    ctx.serial.write_str("  dhcp=");
    ctx.serial
        .write_str(if cfg.dhcp_configured { "yes" } else { "no" });
    if cfg.lease_seconds > 0 {
        ctx.serial.write_str(" lease=");
        print_number(&mut ctx.serial, cfg.lease_seconds);
        ctx.serial.write_str("s");
    }
    ctx.serial.write_str("\n\n");
}

/// Command: dhcp - Configure eth0 with DHCP
fn cmd_dhcp(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial.write_str("\nDHCP eth0 ... ");
    match crate::user_level::net::dhcp_configure() {
        Ok(cfg) => {
            ctx.serial.write_str("[OK] ip=");
            print_ipv4(&mut ctx.serial, cfg.ip);
            ctx.serial.write_str(" gateway=");
            print_ipv4(&mut ctx.serial, cfg.gateway);
            ctx.serial.write_str(" dns=");
            print_ipv4(&mut ctx.serial, cfg.dns);
            ctx.serial.write_str("\n\n");
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_net_error(ctx, err);
            ctx.serial.write_str("\n\n");
        }
    }
}

/// Command: dns - Resolve a host through QEMU user networking
fn cmd_dns(ctx: &mut ShellContext, args: &[&str]) {
    let host = if args.is_empty() {
        crate::user_level::net::DEFAULT_DNS_HOST
    } else {
        args[0]
    };

    ctx.serial.write_str("\nDNS query: ");
    ctx.serial.write_str(host);
    ctx.serial.write_str(" ... ");
    match crate::user_level::net::dns_lookup_a(host) {
        Ok(ip) => {
            ctx.serial.write_str("[OK] ");
            print_ipv4(&mut ctx.serial, ip);
            ctx.serial.write_str("\n\n");
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_net_error(ctx, err);
            ctx.serial.write_str("\n\n");
        }
    }
}

/// Command: ping - Check network reachability
fn cmd_ping(ctx: &mut ShellContext, args: &[&str]) {
    let (target, target_label) = match ping_target(args.first().copied()) {
        PingTarget::Address(ip) => (ip, None),
        PingTarget::Host(host) => match crate::user_level::net::dns_lookup_a(host) {
            Ok(ip) => (ip, Some(host)),
            Err(err) => {
                ctx.serial.write_str("ping: resolve ");
                ctx.serial.write_str(host);
                ctx.serial.write_str(" failed: ");
                print_net_error(ctx, err);
                ctx.serial.write_str("\n");
                return;
            }
        },
        PingTarget::Invalid => {
            ctx.serial
                .write_str("ping: expected IPv4 address, host, or URL\n");
            return;
        }
    };

    ctx.serial.write_str("\nPING ");
    if let Some(host) = target_label {
        ctx.serial.write_str(host);
        ctx.serial.write_str(" (");
    }
    print_ipv4(&mut ctx.serial, target);
    if target_label.is_some() {
        ctx.serial.write_str(")");
    }
    ctx.serial.write_str(" ... ");
    match crate::user_level::net::ping(target) {
        Ok(reply) => {
            ctx.serial.write_str("[OK] from ");
            print_ipv4(&mut ctx.serial, reply.from);
            ctx.serial.write_str(" bytes=");
            print_number(&mut ctx.serial, reply.bytes as u32);
            ctx.serial.write_str(" ttl=");
            print_number(&mut ctx.serial, reply.ttl as u32);
            ctx.serial.write_str("\n\n");
        }
        Err(err) => {
            if err == crate::user_level::net::NetError::Timeout && target_label.is_some() {
                match crate::user_level::net::tcp_probe(
                    target,
                    &crate::user_level::net::PING_TCP_FALLBACK_PORTS,
                ) {
                    Ok(reply) => {
                        ctx.serial.write_str("[OK] ");
                        ctx.serial.write_str("reachable via tcp/");
                        print_number(&mut ctx.serial, reply.port as u32);
                        ctx.serial.write_str(" from ");
                        print_ipv4(&mut ctx.serial, reply.remote_ip);
                        ctx.serial
                            .write_str(" (icmp blocked by QEMU user networking)");
                    }
                    Err(tcp_err) => {
                        ctx.serial.write_str("[FAIL] ");
                        ctx.serial.write_str("icmp timeout; tcp probe failed: ");
                        print_net_error(ctx, tcp_err);
                    }
                }
            } else {
                ctx.serial.write_str("[FAIL] ");
                print_net_error(ctx, err);
            }
            ctx.serial.write_str("\n\n");
        }
    }
}

/// Command: curl - Fetch an HTTP URL
fn cmd_curl(ctx: &mut ShellContext, args: &[&str]) {
    let url = args.first().copied().unwrap_or("http://example.com/");
    let Some((scheme, host, path)) = parse_url(url) else {
        ctx.serial.write_str("curl: invalid URL\n");
        return;
    };
    if scheme == "https" {
        ctx.serial
            .write_str("curl: https:// requires TLS, which is not implemented yet\n");
        ctx.serial
            .write_str("curl: network is up to TCP/HTTP; try `dns ");
        ctx.serial.write_str(host);
        ctx.serial.write_str("` or `ping ");
        ctx.serial.write_str(host);
        ctx.serial.write_str("`\n");
        return;
    }
    if scheme != "http" {
        ctx.serial
            .write_str("curl: only http:// is supported by this stack\n");
        return;
    }

    let mut out = [0u8; 1536];
    ctx.serial.write_str("\nHTTP GET ");
    ctx.serial.write_str(host);
    ctx.serial.write_str(path);
    ctx.serial.write_str(" ... ");
    match crate::user_level::net::http_get(host, path, &mut out) {
        Ok(response) => {
            ctx.serial.write_str("[OK] status=");
            print_number(&mut ctx.serial, response.status_code as u32);
            ctx.serial.write_str(" from ");
            print_ipv4(&mut ctx.serial, response.remote_ip);
            ctx.serial.write_str(" bytes=");
            print_number(&mut ctx.serial, response.bytes_read as u32);
            ctx.serial.write_str("\n");
            print_bytes_as_text(ctx, &out[..core::cmp::min(response.bytes_read, 512)]);
            ctx.serial.write_str("\n\n");
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_net_error(ctx, err);
            ctx.serial.write_str("\n\n");
        }
    }
}

/// Command: ftp - Read an FTP server banner
fn cmd_ftp(ctx: &mut ShellContext, args: &[&str]) {
    let host = args.first().copied().unwrap_or("speedtest.tele2.net");
    let mut out = [0u8; 512];
    ctx.serial.write_str("\nFTP banner ");
    ctx.serial.write_str(host);
    ctx.serial.write_str(" ... ");
    match crate::user_level::net::ftp_banner(host, &mut out) {
        Ok(response) => {
            ctx.serial.write_str("[OK] status=");
            print_number(&mut ctx.serial, response.status_code as u32);
            ctx.serial.write_str(" from ");
            print_ipv4(&mut ctx.serial, response.remote_ip);
            ctx.serial.write_str("\n");
            print_bytes_as_text(ctx, &out[..core::cmp::min(response.bytes_read, 256)]);
            ctx.serial.write_str("\n\n");
        }
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_net_error(ctx, err);
            ctx.serial.write_str("\n\n");
        }
    }
}

/// Command: tls - Report TLS support state
fn cmd_tls(ctx: &mut ShellContext, _args: &[&str]) {
    let mut out = [0u8; 64];
    ctx.serial.write_str("\nTLS test ... ");
    match crate::user_level::net::tls_get("example.com", "/", &mut out) {
        Err(err) => {
            ctx.serial.write_str("[FAIL] ");
            print_net_error(ctx, err);
            ctx.serial.write_str("\n\n");
        }
        Ok(_) => ctx.serial.write_str("[OK]\n\n"),
    }
}

/// Command: svc - Show minimal service directory and fixed-message IPC state
fn cmd_svc(ctx: &mut ShellContext, _args: &[&str]) {
    let stats = crate::user_level::svc::stats();
    ctx.serial.write_str("\n/svc services: ");
    print_number(&mut ctx.serial, stats.services as u32);
    ctx.serial.write_str("  Connections: ");
    print_number(&mut ctx.serial, stats.connections as u32);
    ctx.serial.write_str("  Requests: ");
    print_number(&mut ctx.serial, stats.requests as u32);
    ctx.serial.write_str("  Replies: ");
    print_number(&mut ctx.serial, stats.replies as u32);
    ctx.serial.write_str("  Last status: ");
    print_zx_status_i32(&mut ctx.serial, stats.last_status);
    ctx.serial.write_str("\n\nServices:\n");

    for service in crate::user_level::svc::services() {
        ctx.serial.write_str("  ");
        ctx.serial.write_str(service.kind.as_str());
        ctx.serial.write_str("  rights=0x");
        print_hex(&mut ctx.serial, service.rights as u64);
        ctx.serial.write_str("  /svc/");
        ctx.serial.write_str(service.name);
        ctx.serial.write_str("\n");
    }
    ctx.serial.write_str("\n");
}

/// Command: clear - Clear screen
fn cmd_clear(_ctx: &mut ShellContext, _args: &[&str]) {
    // Send ANSI clear screen code
    // In real impl, would use: _ctx.serial.write_str("\x1b[2J\x1b[H");
}

/// Command: reboot - Reset machine through PSCI
fn cmd_reboot(ctx: &mut ShellContext, _args: &[&str]) {
    ctx.serial.write_str("Rebooting...\n");
    crate::kernel_lowlevel::smp::system_reset();
}

/// Command: ps - List processes; ps -a also shows memory maps
fn cmd_ps(ctx: &mut ShellContext, args: &[&str]) {
    let show_memory_maps = if args.is_empty() {
        false
    } else if args.len() == 1 && (args[0] == "-a" || args[0] == "--all") {
        true
    } else {
        ctx.serial.write_str("usage: ps [-a]\n");
        return;
    };

    let pm = process_manager();
    let sched = scheduler::scheduler();
    let tick = sched.get_tick_count();
    let vm_status = crate::kernel_objects::hypervisor::hypervisor().status(tick);

    if show_memory_maps {
        ctx.serial
            .write_str("\n  PID  State      Name                  Threads  Parent  VCPU\n");
        ctx.serial
            .write_str("  ───────────────────────────────────────────────────────────────\n");
    } else {
        ctx.serial
            .write_str("\n  PID  State      Name                  Threads  Parent\n");
        ctx.serial
            .write_str("  ─────────────────────────────────────────────────────\n");
    }

    let mut count = 0;
    for i in 0..crate::kernel_lowlevel::memory::MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                print_number(&mut ctx.serial, pcb.pid as u32);
                ctx.serial.write_str("    ");
                ctx.serial.write_str(pcb.state.as_str());
                ctx.serial.write_str("  ");
                ctx.serial.write_str(pcb.name);
                for _ in 0..(22usize.saturating_sub(pcb.name.len())) {
                    ctx.serial.write_byte(b' ');
                }
                let live_threads = sched.live_thread_count_for_process(pcb.pid);
                let modeled_threads = ps_vm_thread_count(&vm_status, pcb.pid);
                let visible_threads = live_threads.saturating_add(modeled_threads);
                let visible_threads = if pcb.state == ProcessState::Terminated {
                    visible_threads
                } else {
                    core::cmp::max(visible_threads, pcb.thread_count)
                };
                print_number(&mut ctx.serial, visible_threads as u32);
                ctx.serial.write_str("         ");
                print_number(&mut ctx.serial, pcb.parent_pid as u32);
                if show_memory_maps {
                    ctx.serial.write_str("       ");
                    print_ps_process_vcpu(&mut ctx.serial, &vm_status, pcb.pid);
                }
                ctx.serial.write_str("\n");
                count += 1;
            }
        }
    }

    if show_memory_maps {
        ctx.serial
            .write_str("  ───────────────────────────────────────────────────────────────\n");
    } else {
        ctx.serial
            .write_str("  ─────────────────────────────────────────────────────\n");
    }
    ctx.serial.write_str("  Total: ");
    print_number(&mut ctx.serial, count as u32);
    ctx.serial.write_str(" process(es)\n");

    ctx.serial.write_str(
        "\n  TID  PID  State       Name                CPU  Bind  Left(ms)  Slice(ms)  Prio  Ticks  Weight  Credit\n",
    );
    ctx.serial.write_str(
        "  ─────────────────────────────────────────────────────────────────────────────────────────────────\n",
    );
    let mut thread_count = 0usize;
    for tid in 0..crate::kernel_lowlevel::thread::MAX_THREADS {
        let Some(thread) = sched.get_thread(crate::kernel_lowlevel::thread::ThreadId(tid)) else {
            continue;
        };
        if thread.state == crate::kernel_lowlevel::thread::ThreadState::Empty {
            continue;
        }
        let info = sched
            .thread_schedule_info(crate::kernel_lowlevel::thread::ThreadId(tid))
            .unwrap_or(crate::kernel_objects::scheduler::ThreadScheduleInfo::empty());

        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, tid as u32, 3);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, info.process_id as u32, 3);
        ctx.serial.write_str("  ");
        let state = thread.state.as_str().trim();
        ctx.serial.write_str(state);
        for _ in 0..(10usize.saturating_sub(state.len())) {
            ctx.serial.write_byte(b' ');
        }
        ctx.serial.write_str("  ");
        ctx.serial.write_str(thread.name);
        for _ in 0..(18usize.saturating_sub(thread.name.len())) {
            ctx.serial.write_byte(b' ');
        }
        match thread.current_cpu {
            Some(cpu) => print_padded_number(&mut ctx.serial, cpu as u32, 3),
            None => ctx.serial.write_str("  *"),
        }
        ctx.serial.write_str("  ");
        match thread.cpu_affinity {
            Some(cpu) => print_padded_number(&mut ctx.serial, cpu as u32, 4),
            None => ctx.serial.write_str(" any"),
        }
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, scheduler_ticks_to_ms(thread.time_slice), 8);
        ctx.serial.write_str("  ");
        print_padded_number(
            &mut ctx.serial,
            scheduler_ticks_to_ms(info.time_slice_ticks),
            9,
        );
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, info.priority as u32, 4);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, thread.total_ticks, 5);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, info.weight, 6);
        ctx.serial.write_str("  ");
        print_i32_shell(&mut ctx.serial, info.credit);
        ctx.serial.write_str("/");
        print_i32_shell(&mut ctx.serial, info.credit_cap);
        ctx.serial.write_str("\n");
        thread_count += 1;
    }
    thread_count += print_ps_vm_threads(ctx, &vm_status, show_memory_maps);
    ctx.serial.write_str(
        "  ─────────────────────────────────────────────────────────────────────────────────────────────────\n",
    );
    ctx.serial.write_str("  Total: ");
    print_usize(&mut ctx.serial, thread_count);
    ctx.serial.write_str(" thread(s)\n");

    if show_memory_maps {
        print_ps_memory_maps(ctx, pm, &vm_status);
    }
}

fn print_ps_memory_maps(
    ctx: &mut ShellContext,
    pm: &crate::kernel_lowlevel::memory::ProcessManager,
    vm_status: &crate::kernel_objects::hypervisor::HypervisorStatus,
) {
    let snapshot = crate::syscall::memory_map_snapshot();

    ctx.serial
        .write_str("\n  Process Memory Map / segments, pages, page tables (-a)\n");
    ctx.serial.write_str(
        "  Notes: PCB segments/pages are per process; Linux mmap and Zircon VMAR tables are compat-global models.\n",
    );
    ctx.serial.write_str(
        "         PCB page rows model normal process VA->PA; VM rows model guest stage-2 IPA->PA metadata only.\n",
    );
    ctx.serial.write_str(
        "         ARM64 4KB granule, TTBR0/TTBR1 roots; PageTableManager currently stores indexed entries, not a live walked 4-level tree.\n",
    );
    ctx.serial.write_str(
        "         Page rows show PGD/PUD/PMD indexes when used; PTE# is the PCB PageEntry slot, and PFN is its content.\n",
    );

    print_ps_process_memory_table(ctx, pm);
    print_ps_segment_table(ctx, pm);
    print_ps_page_table(ctx, pm);
    print_ps_vm_stage2_table(ctx, vm_status);
    print_ps_linux_mappings(ctx, &snapshot);
    print_ps_shared_memory(ctx, &snapshot);
    print_ps_zircon_mappings(ctx, &snapshot);
}

fn print_ps_process_memory_table(
    ctx: &mut ShellContext,
    pm: &crate::kernel_lowlevel::memory::ProcessManager,
) {
    ctx.serial.write_str("\n  Process address spaces\n");
    ctx.serial.write_str(
        "  PID  State       Parent  Name                  Heap(cur/max)       Stack(cur/top)      Pages\n",
    );
    ctx.serial.write_str(
        "  ---  ----------  ------  --------------------  ------------------  ------------------  -----\n",
    );

    for i in 0..crate::kernel_lowlevel::memory::MAX_PROCESSES {
        let Some(pcb) = pm.get_process(i) else {
            continue;
        };
        if pcb.state == ProcessState::Empty {
            continue;
        }

        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, pcb.pid, 3);
        ctx.serial.write_str("  ");
        print_padded_str(&mut ctx.serial, pcb.state.as_str().trim(), 10);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, pcb.parent_pid, 6);
        ctx.serial.write_str("  ");
        print_padded_str(&mut ctx.serial, pcb.name, 20);
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, pcb.address_space.heap_current as u64);
        ctx.serial.write_str("/0x");
        print_hex(&mut ctx.serial, pcb.address_space.heap_max as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_pair_width(
                pcb.address_space.heap_current as u64,
                pcb.address_space.heap_max as u64,
            ),
            18,
        );
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, pcb.address_space.stack_current as u64);
        ctx.serial.write_str("/0x");
        print_hex(&mut ctx.serial, pcb.address_space.stack_top as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_pair_width(
                pcb.address_space.stack_current as u64,
                pcb.address_space.stack_top as u64,
            ),
            18,
        );
        ctx.serial.write_str("  ");
        print_usize(&mut ctx.serial, pcb.address_space.valid_page_count);
        ctx.serial.write_str("/");
        print_usize(
            &mut ctx.serial,
            crate::kernel_lowlevel::memory::MAX_PAGES_PER_PROCESS,
        );
        ctx.serial.write_str("\n");
    }
}

fn print_ps_segment_table(
    ctx: &mut ShellContext,
    pm: &crate::kernel_lowlevel::memory::ProcessManager,
) {
    ctx.serial.write_str("\n  Segments\n");
    ctx.serial
        .write_str("  PID  Segment   Range                  Perm  Pages  Shared  Dirty\n");
    ctx.serial
        .write_str("  ---  --------  ---------------------  ----  -----  ------  -----------\n");

    for i in 0..crate::kernel_lowlevel::memory::MAX_PROCESSES {
        let Some(pcb) = pm.get_process(i) else {
            continue;
        };
        if pcb.state == ProcessState::Empty {
            continue;
        }

        let address_space = &pcb.address_space;
        for seg_idx in 0..address_space.valid_segment_count {
            let segment = &address_space.segments[seg_idx];
            if !segment.valid {
                continue;
            }

            ctx.serial.write_str("  ");
            print_padded_usize(&mut ctx.serial, pcb.pid, 3);
            ctx.serial.write_str("  ");
            print_padded_str(&mut ctx.serial, segment.seg_type.as_str(), 8);
            ctx.serial.write_str("  0x");
            print_hex(&mut ctx.serial, segment.base_vaddr as u64);
            ctx.serial.write_str("-0x");
            print_hex(&mut ctx.serial, segment.end_vaddr() as u64);
            pad_to_width(
                &mut ctx.serial,
                hex_range_width(segment.base_vaddr as u64, segment.end_vaddr() as u64),
                21,
            );
            ctx.serial.write_str("  ");
            print_padded_str(&mut ctx.serial, segment.permissions.as_str(), 4);
            ctx.serial.write_str("  ");
            print_padded_usize(&mut ctx.serial, segment.page_count, 5);
            ctx.serial.write_str("  no      not-tracked\n");
        }
    }
}

fn print_ps_page_table(
    ctx: &mut ShellContext,
    pm: &crate::kernel_lowlevel::memory::ProcessManager,
) {
    ctx.serial.write_str("\n  Pages / process VA->PA\n");
    ctx.serial.write_str(
        "  PID  Segment   VA          PGD  PUD  PMD  PTE#  PA          PFN     Flags  Dirty\n",
    );
    ctx.serial.write_str(
        "  ---  --------  ----------  ---  ---  ---  ----  ----------  ------  -----  -----------\n",
    );

    for i in 0..crate::kernel_lowlevel::memory::MAX_PROCESSES {
        let Some(pcb) = pm.get_process(i) else {
            continue;
        };
        if pcb.state == ProcessState::Empty {
            continue;
        }

        let mut page_index = 0usize;
        for seg_idx in 0..pcb.address_space.valid_segment_count {
            let segment = &pcb.address_space.segments[seg_idx];
            if !segment.valid {
                continue;
            }

            for page_offset in 0..segment.page_count {
                let page_slot = page_index.saturating_add(page_offset);
                if page_slot >= pcb.address_space.valid_page_count {
                    break;
                }
                let page = pcb.address_space.pages[page_slot];
                if !page.valid {
                    continue;
                }
                let vaddr = segment
                    .base_vaddr
                    .saturating_add(page_offset.saturating_mul(PAGE_SIZE));
                let pgd = ps_arm64_pgd_index(vaddr);
                let pmd = ps_smros_indexed_pmd(vaddr);
                let paddr = page.pfn.saturating_mul(PAGE_SIZE as u64);
                ctx.serial.write_str("  ");
                print_padded_usize(&mut ctx.serial, pcb.pid, 3);
                ctx.serial.write_str("  ");
                print_padded_str(&mut ctx.serial, segment.seg_type.as_str(), 8);
                ctx.serial.write_str("  ");
                ctx.serial.write_str("0x");
                print_hex(&mut ctx.serial, vaddr as u64);
                pad_to_width(&mut ctx.serial, hex_value_width(vaddr as u64), 10);
                ctx.serial.write_str("  ");
                print_padded_usize(&mut ctx.serial, pgd, 3);
                ctx.serial.write_str("  ");
                print_padded_str(&mut ctx.serial, "--", 3);
                ctx.serial.write_str("  ");
                print_padded_usize(&mut ctx.serial, pmd, 3);
                ctx.serial.write_str("  ");
                print_padded_usize(&mut ctx.serial, page_slot, 4);
                ctx.serial.write_str("  0x");
                print_hex(&mut ctx.serial, paddr);
                pad_to_width(&mut ctx.serial, hex_value_width(paddr), 10);
                ctx.serial.write_str("  ");
                print_padded_u64(&mut ctx.serial, page.pfn, 6);
                ctx.serial.write_str("  ");
                print_page_entry_flags(ctx, page);
                ctx.serial.write_str("   not-tracked\n");
            }
            page_index = page_index.saturating_add(segment.page_count);
        }
    }
}

fn print_ps_vm_stage2_table(
    ctx: &mut ShellContext,
    status: &crate::kernel_objects::hypervisor::HypervisorStatus,
) {
    ctx.serial
        .write_str("\n  Stage-2 IPA->PA / hypervisor metadata\n");
    ctx.serial.write_str(
        "  PID  VM                    State     IPA Range             PA          Guest       VMAR        VMO         VCPU        Memory(bytes)  Stage2\n",
    );
    ctx.serial.write_str(
        "  ---  --------------------  --------  ---------------------  ----------  ----------  ----------  ----------  ----------  -------------  ----------\n",
    );

    let mut count = 0usize;
    for vm in &status.vms {
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, vm.process_pid, 3);
        ctx.serial.write_str("  ");
        print_padded_str(&mut ctx.serial, vm.name.as_str(), 20);
        ctx.serial.write_str("  ");
        print_padded_str(&mut ctx.serial, vm.state.as_str(), 8);
        ctx.serial.write_str("  0x0-0x");
        print_hex(&mut ctx.serial, vm.memory_bytes as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_range_width(0, vm.memory_bytes as u64),
            21,
        );
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, vm.memory_base as u64);
        pad_to_width(&mut ctx.serial, hex_value_width(vm.memory_base as u64), 10);
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, vm.guest_handle as u64);
        pad_to_width(&mut ctx.serial, hex_value_width(vm.guest_handle as u64), 10);
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, vm.vmar_handle as u64);
        pad_to_width(&mut ctx.serial, hex_value_width(vm.vmar_handle as u64), 10);
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, vm.memory_vmo_handle as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_value_width(vm.memory_vmo_handle as u64),
            10,
        );
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, vm.vcpu_handle as u64);
        pad_to_width(&mut ctx.serial, hex_value_width(vm.vcpu_handle as u64), 10);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, vm.memory_bytes, 13);
        ctx.serial.write_str("  IPA->PA\n");
        count += 1;
    }

    if count == 0 {
        ctx.serial.write_str("  (no stage-2 VM metadata)\n");
    }
}

fn ps_vm_thread_count(
    status: &crate::kernel_objects::hypervisor::HypervisorStatus,
    process_pid: usize,
) -> usize {
    let mut count = 0usize;
    for vm in &status.vms {
        if vm.process_pid == process_pid
            && vm.state == crate::kernel_objects::hypervisor::VmState::Running
        {
            count += 1;
        }
    }
    count
}

fn print_ps_process_vcpu(
    serial: &mut Serial,
    status: &crate::kernel_objects::hypervisor::HypervisorStatus,
    process_pid: usize,
) {
    let mut printed = false;
    for vm in &status.vms {
        if vm.process_pid != process_pid {
            continue;
        }

        if printed {
            serial.write_str(",");
        }
        serial.write_str("0x");
        print_hex(serial, vm.vcpu_handle as u64);
        printed = true;
    }

    if !printed {
        serial.write_str("-");
    }
}

fn print_ps_vm_threads(
    ctx: &mut ShellContext,
    status: &crate::kernel_objects::hypervisor::HypervisorStatus,
    show_vcpu_handles: bool,
) -> usize {
    let mut count = 0usize;
    for vm in &status.vms {
        if vm.state != crate::kernel_objects::hypervisor::VmState::Running {
            continue;
        }

        let tid = 1000u32.saturating_add(vm.id);
        let slice_ms = vm.cpu_time_slice_us.saturating_add(999) / 1000;

        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, tid, 3);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, vm.process_pid as u32, 3);
        ctx.serial.write_str("  ");
        ctx.serial.write_str("Running");
        for _ in 0..(10usize.saturating_sub("Running".len())) {
            ctx.serial.write_byte(b' ');
        }
        ctx.serial.write_str("  ");
        ctx.serial.write_str("vcpu:");
        let vm_thread_name_len = if show_vcpu_handles {
            ctx.serial.write_str("0x");
            print_hex(&mut ctx.serial, vm.vcpu_handle as u64);
            7usize.saturating_add(hex_digit_count(vm.vcpu_handle as u64))
        } else {
            ctx.serial.write_str(vm.name.as_str());
            5usize.saturating_add(vm.name.len())
        };
        for _ in 0..(18usize.saturating_sub(vm_thread_name_len)) {
            ctx.serial.write_byte(b' ');
        }
        ctx.serial.write_str("  *");
        ctx.serial.write_str("   any");
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, slice_ms, 8);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, slice_ms, 9);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, vm.realtime_priority as u32, 4);
        ctx.serial.write_str("  ");
        print_padded_number(
            &mut ctx.serial,
            saturating_u64_to_u32(vm.uptime_ticks(status.tick)),
            5,
        );
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, 0, 6);
        ctx.serial.write_str("  0/0\n");

        count += 1;
    }
    count
}

fn print_page_entry_flags(ctx: &mut ShellContext, page: crate::kernel_lowlevel::memory::PageEntry) {
    ctx.serial
        .write_byte(if page.user_accessible { b'u' } else { b'k' });
    ctx.serial.write_byte(b'r');
    ctx.serial
        .write_byte(if page.writable { b'w' } else { b'-' });
    ctx.serial
        .write_byte(if page.executable { b'x' } else { b'-' });
}

fn print_ps_linux_mappings(ctx: &mut ShellContext, snapshot: &crate::syscall::MemoryMapSnapshot) {
    ctx.serial.write_str("\n  Linux compat mmap registry\n");
    ctx.serial
        .write_str("  Maps  Bytes       Pages  BrkStart    BrkCurrent  BrkLimit    BrkPages\n");
    ctx.serial
        .write_str("  ----  ----------  -----  ----------  ----------  ----------  --------\n");
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.linux_mapping_count, 4);
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.linux_mapped_bytes, 10);
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.linux_committed_pages, 5);
    ctx.serial.write_str("  0x");
    print_hex(&mut ctx.serial, snapshot.stats.brk_start as u64);
    pad_to_width(
        &mut ctx.serial,
        hex_value_width(snapshot.stats.brk_start as u64),
        10,
    );
    ctx.serial.write_str("  0x");
    print_hex(&mut ctx.serial, snapshot.stats.brk_current as u64);
    pad_to_width(
        &mut ctx.serial,
        hex_value_width(snapshot.stats.brk_current as u64),
        10,
    );
    ctx.serial.write_str("  0x");
    print_hex(&mut ctx.serial, snapshot.stats.brk_limit as u64);
    pad_to_width(
        &mut ctx.serial,
        hex_value_width(snapshot.stats.brk_limit as u64),
        10,
    );
    ctx.serial.write_str("  ");
    print_usize(&mut ctx.serial, snapshot.stats.brk_committed_pages);
    ctx.serial.write_str("\n");

    ctx.serial.write_str(
        "  Range                  Prot  Share    Pages  PFNs             Dirty        Source\n",
    );
    ctx.serial.write_str(
        "  ---------------------  ----  -------  -----  ---------------  -----------  ------\n",
    );

    if snapshot.linux_mappings.is_empty() {
        ctx.serial.write_str("  (no linux mmap records)\n");
        return;
    }

    for mapping in &snapshot.linux_mappings {
        let end = mapping.addr.saturating_add(mapping.len);
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, mapping.addr as u64);
        ctx.serial.write_str("-0x");
        print_hex(&mut ctx.serial, end as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_range_width(mapping.addr as u64, end as u64),
            21,
        );
        ctx.serial.write_str("  ");
        print_mmap_prot(ctx, mapping.prot);
        ctx.serial.write_str("   ");
        print_mmap_share_padded(ctx, mapping, 7);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, mapping.committed_pages, 5);
        ctx.serial.write_str("  ");
        print_linux_pfn_range(ctx, mapping);
        ctx.serial.write_str("  ");
        if mapping.dirty_tracked {
            print_padded_usize(&mut ctx.serial, mapping.dirty_pages, 11);
        } else {
            print_padded_str(&mut ctx.serial, "not-tracked", 11);
        }
        ctx.serial.write_str("  ");
        print_linux_mapping_source(ctx, &mapping.source);
        ctx.serial.write_str("\n");
    }
}

fn print_ps_shared_memory(ctx: &mut ShellContext, snapshot: &crate::syscall::MemoryMapSnapshot) {
    ctx.serial.write_str("\n  Shared memory\n");
    ctx.serial
        .write_str("  ID          Size        Attach  Mapped      Dirty\n");
    ctx.serial
        .write_str("  ----------  ----------  ------  ----------  -----------\n");
    if snapshot.shared_memory.is_empty() {
        ctx.serial.write_str("  (no SysV shared-memory objects)\n");
        return;
    }

    let mut attachment_rows = 0usize;
    for shm in &snapshot.shared_memory {
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, shm.id as u64);
        pad_to_width(&mut ctx.serial, hex_value_width(shm.id as u64), 10);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, shm.size, 10);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, shm.attach_count, 6);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, shm.mapped_bytes, 10);
        ctx.serial.write_str("  not-tracked\n");
        attachment_rows = attachment_rows.saturating_add(shm.attachments.len());
    }

    if attachment_rows == 0 {
        return;
    }

    ctx.serial.write_str("\n  Shared memory attachments\n");
    ctx.serial
        .write_str("  ID          Range                  Bytes\n");
    ctx.serial
        .write_str("  ----------  ---------------------  ----------\n");
    for shm in &snapshot.shared_memory {
        for attachment in &shm.attachments {
            let end = attachment.addr.saturating_add(attachment.len);
            ctx.serial.write_str("  0x");
            print_hex(&mut ctx.serial, shm.id as u64);
            pad_to_width(&mut ctx.serial, hex_value_width(shm.id as u64), 10);
            ctx.serial.write_str("  0x");
            print_hex(&mut ctx.serial, attachment.addr as u64);
            ctx.serial.write_str("-0x");
            print_hex(&mut ctx.serial, end as u64);
            pad_to_width(
                &mut ctx.serial,
                hex_range_width(attachment.addr as u64, end as u64),
                21,
            );
            ctx.serial.write_str("  ");
            print_usize(&mut ctx.serial, attachment.len);
            ctx.serial.write_str("\n");
        }
    }
}

fn print_ps_zircon_mappings(ctx: &mut ShellContext, snapshot: &crate::syscall::MemoryMapSnapshot) {
    ctx.serial.write_str("\n  Zircon VMO/VMAR bookkeeping\n");
    ctx.serial
        .write_str("  VMOs  VMOBytes    VMOPages  VMARs  Maps  RootVMAR\n");
    ctx.serial
        .write_str("  ----  ----------  --------  -----  ----  ----------\n");
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.zircon_vmo_count, 4);
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.zircon_vmo_bytes, 10);
    ctx.serial.write_str("  ");
    print_padded_usize(
        &mut ctx.serial,
        snapshot.stats.zircon_vmo_committed_pages,
        8,
    );
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.zircon_vmar_count, 5);
    ctx.serial.write_str("  ");
    print_padded_usize(&mut ctx.serial, snapshot.stats.zircon_mapping_count, 4);
    ctx.serial.write_str("  0x");
    print_hex(
        &mut ctx.serial,
        snapshot.stats.zircon_root_vmar_handle as u64,
    );
    ctx.serial.write_str("\n");

    ctx.serial.write_str("\n  Zircon VMOs\n");
    ctx.serial
        .write_str("  Handle      Type        Size        Pages     Resizable  Dirty\n");
    ctx.serial
        .write_str("  ----------  ----------  ----------  --------  ---------  -----------\n");
    if snapshot.vmos.is_empty() {
        ctx.serial.write_str("  (no VMO records)\n");
    } else {
        for vmo in &snapshot.vmos {
            ctx.serial.write_str("  0x");
            print_hex(&mut ctx.serial, vmo.handle as u64);
            pad_to_width(&mut ctx.serial, hex_value_width(vmo.handle as u64), 10);
            ctx.serial.write_str("  ");
            print_vmo_type_padded(ctx, vmo.vmo_type, 10);
            ctx.serial.write_str("  ");
            print_padded_usize(&mut ctx.serial, vmo.size, 10);
            ctx.serial.write_str("  ");
            print_pages_pair(&mut ctx.serial, vmo.committed_pages, vmo.page_count, 8);
            ctx.serial.write_str("  ");
            print_padded_str(&mut ctx.serial, if vmo.resizable { "yes" } else { "no" }, 9);
            ctx.serial.write_str("  not-tracked\n");
        }
    }

    ctx.serial.write_str("\n  Zircon VMARs\n");
    ctx.serial
        .write_str("  Handle      Range                  Maps   Child  Parent\n");
    ctx.serial
        .write_str("  ----------  ---------------------  -----  -----  ----------\n");
    if snapshot.vmars.is_empty() {
        ctx.serial.write_str("  (no VMAR records)\n");
    } else {
        for vmar in &snapshot.vmars {
            let end = vmar.base_addr.saturating_add(vmar.size);
            ctx.serial.write_str("  0x");
            print_hex(&mut ctx.serial, vmar.handle as u64);
            pad_to_width(&mut ctx.serial, hex_value_width(vmar.handle as u64), 10);
            ctx.serial.write_str("  0x");
            print_hex(&mut ctx.serial, vmar.base_addr as u64);
            ctx.serial.write_str("-0x");
            print_hex(&mut ctx.serial, end as u64);
            pad_to_width(
                &mut ctx.serial,
                hex_range_width(vmar.base_addr as u64, end as u64),
                21,
            );
            ctx.serial.write_str("  ");
            print_padded_usize(&mut ctx.serial, vmar.mapping_count, 5);
            ctx.serial.write_str("  ");
            print_padded_usize(&mut ctx.serial, vmar.child_count, 5);
            ctx.serial.write_str("  ");
            match vmar.parent_idx {
                Some(parent) => print_padded_usize(&mut ctx.serial, parent, 10),
                None => print_padded_str(&mut ctx.serial, "none", 10),
            }
            ctx.serial.write_str("\n");
        }
    }

    ctx.serial.write_str("\n  Zircon VMAR mappings\n");
    ctx.serial.write_str(
        "  VMAR        Range                  VMO         Offset      Flags  VMOPages  Valid\n",
    );
    ctx.serial.write_str(
        "  ----------  ---------------------  ----------  ----------  -----  --------  -----\n",
    );

    if snapshot.vmar_mappings.is_empty() {
        ctx.serial.write_str("  (no VMAR mapping records)\n");
        return;
    }

    for mapping in &snapshot.vmar_mappings {
        let end = mapping.vaddr.saturating_add(mapping.size);
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, mapping.vmar_handle as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_value_width(mapping.vmar_handle as u64),
            10,
        );
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, mapping.vaddr as u64);
        ctx.serial.write_str("-0x");
        print_hex(&mut ctx.serial, end as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_range_width(mapping.vaddr as u64, end as u64),
            21,
        );
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, mapping.vmo_handle as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_value_width(mapping.vmo_handle as u64),
            10,
        );
        ctx.serial.write_str("  0x");
        print_hex(&mut ctx.serial, mapping.vmo_offset as u64);
        pad_to_width(
            &mut ctx.serial,
            hex_value_width(mapping.vmo_offset as u64),
            10,
        );
        ctx.serial.write_str("  ");
        print_mmu_flags(ctx, mapping.mmu_flags);
        ctx.serial.write_str("  ");
        print_padded_usize(&mut ctx.serial, mapping.vmo_committed_pages, 8);
        ctx.serial.write_str("  ");
        ctx.serial
            .write_str(if mapping.valid { "yes" } else { "no" });
        ctx.serial.write_str("\n");
    }
}

fn print_mmap_prot(ctx: &mut ShellContext, prot: usize) {
    ctx.serial
        .write_byte(if prot & 0x1 != 0 { b'r' } else { b'-' });
    ctx.serial
        .write_byte(if prot & 0x2 != 0 { b'w' } else { b'-' });
    ctx.serial
        .write_byte(if prot & 0x4 != 0 { b'x' } else { b'-' });
}

fn print_mmap_share_padded(
    ctx: &mut ShellContext,
    mapping: &crate::syscall::LinuxMappingSnapshot,
    width: usize,
) {
    if mapping.shared {
        print_padded_str(&mut ctx.serial, "shared", width);
    } else if mapping.private {
        print_padded_str(&mut ctx.serial, "private", width);
    } else {
        print_padded_str(&mut ctx.serial, "unknown", width);
    }
}

fn print_linux_pfn_range(ctx: &mut ShellContext, mapping: &crate::syscall::LinuxMappingSnapshot) {
    match (mapping.first_pfn, mapping.last_pfn) {
        (Some(first), Some(last)) => {
            let mut buf = [0u8; 20];
            let first_len = decimal_u64_digits(first, &mut buf);
            print_u64(&mut ctx.serial, first);
            ctx.serial.write_str("..");
            print_u64(&mut ctx.serial, last);
            let used = first_len
                .saturating_add(2)
                .saturating_add(decimal_u64_len(last));
            pad_to_width(&mut ctx.serial, used, 15);
        }
        _ => print_padded_str(&mut ctx.serial, "none", 15),
    }
}

fn print_linux_mapping_source(
    ctx: &mut ShellContext,
    source: &crate::syscall::LinuxMappingSourceSnapshot,
) {
    match source {
        crate::syscall::LinuxMappingSourceSnapshot::Anonymous => ctx.serial.write_str("anonymous"),
        crate::syscall::LinuxMappingSourceSnapshot::File { fd, offset, path } => {
            ctx.serial.write_str("file(fd=");
            print_usize(&mut ctx.serial, *fd);
            ctx.serial.write_str(",off=0x");
            print_hex(&mut ctx.serial, *offset);
            ctx.serial.write_str(",path=");
            ctx.serial.write_str(path.as_str());
            ctx.serial.write_str(")");
        }
        crate::syscall::LinuxMappingSourceSnapshot::SharedMemory { id } => {
            ctx.serial.write_str("shm(0x");
            print_hex(&mut ctx.serial, *id as u64);
            ctx.serial.write_str(")");
        }
    }
}

fn print_vmo_type_padded(
    ctx: &mut ShellContext,
    vmo_type: crate::kernel_objects::VmoType,
    width: usize,
) {
    match vmo_type {
        crate::kernel_objects::VmoType::Paged => print_padded_str(&mut ctx.serial, "paged", width),
        crate::kernel_objects::VmoType::Physical => {
            print_padded_str(&mut ctx.serial, "physical", width)
        }
        crate::kernel_objects::VmoType::Contiguous => {
            print_padded_str(&mut ctx.serial, "contiguous", width)
        }
        crate::kernel_objects::VmoType::Resizable => {
            print_padded_str(&mut ctx.serial, "resizable", width)
        }
    }
}

fn print_pages_pair(serial: &mut Serial, committed: usize, total: usize, width: usize) {
    let mut buf = [0u8; 20];
    let used = decimal_usize_digits(committed, &mut buf)
        .saturating_add(1)
        .saturating_add(decimal_usize_len(total));
    print_usize(serial, committed);
    serial.write_str("/");
    print_usize(serial, total);
    pad_to_width(serial, used, width);
}

fn decimal_usize_len(value: usize) -> usize {
    let mut buf = [0u8; 20];
    decimal_usize_digits(value, &mut buf)
}

fn decimal_u64_len(value: u64) -> usize {
    let mut buf = [0u8; 20];
    decimal_u64_digits(value, &mut buf)
}

fn print_mmu_flags(ctx: &mut ShellContext, flags: u32) {
    ctx.serial.write_byte(
        if flags & crate::kernel_objects::MmuFlags::USER.bits() != 0 {
            b'u'
        } else {
            b'k'
        },
    );
    ctx.serial.write_byte(
        if flags & crate::kernel_objects::MmuFlags::READ.bits() != 0 {
            b'r'
        } else {
            b'-'
        },
    );
    ctx.serial.write_byte(
        if flags & crate::kernel_objects::MmuFlags::WRITE.bits() != 0 {
            b'w'
        } else {
            b'-'
        },
    );
    ctx.serial.write_byte(
        if flags & crate::kernel_objects::MmuFlags::EXECUTE.bits() != 0 {
            b'x'
        } else {
            b'-'
        },
    );
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

const DHRYSTONE_DEFAULT_RUNS_PER_CORE: u64 = 50_000;
const DHRYSTONE_MAX_RUNS_PER_CORE: u64 = 5_000_000;
const DHRYSTONE_DMIPS_DIVISOR: u128 = 1757;
const DHRYSTONE_SOME_STRING: &[u8] = b"DHRYSTONE PROGRAM, SOME STRING";
const DHRYSTONE_1ST_STRING: &[u8] = b"DHRYSTONE PROGRAM, 1'ST STRING";
const DHRYSTONE_2ND_STRING: &[u8] = b"DHRYSTONE PROGRAM, 2'ND STRING";
const DHRYSTONE_3RD_STRING: &[u8] = b"DHRYSTONE PROGRAM, 3'RD STRING";

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum DhryIdent {
    Ident1 = 0,
    Ident2 = 1,
    Ident3 = 2,
    Ident4 = 3,
    Ident5 = 4,
}

#[derive(Clone, Copy)]
struct DhryRecord {
    ptr_comp: usize,
    discr: DhryIdent,
    enum_comp: DhryIdent,
    int_comp: i32,
    str_comp: [u8; 31],
}

impl DhryRecord {
    const fn empty() -> Self {
        Self {
            ptr_comp: 0,
            discr: DhryIdent::Ident1,
            enum_comp: DhryIdent::Ident1,
            int_comp: 0,
            str_comp: [0; 31],
        }
    }
}

struct DhryState {
    ptr_glob: usize,
    next_ptr_glob: usize,
    records: [DhryRecord; 2],
    int_glob: i32,
    bool_glob: bool,
    ch_1_glob: u8,
    ch_2_glob: u8,
    arr_1_glob: [i32; 50],
    arr_2_glob: [[i32; 50]; 50],
    last_int_1_loc: i32,
    last_int_2_loc: i32,
    last_int_3_loc: i32,
    last_enum_loc: DhryIdent,
    last_str_1_loc: [u8; 31],
    last_str_2_loc: [u8; 31],
}

impl DhryState {
    fn new() -> Self {
        Self {
            ptr_glob: 0,
            next_ptr_glob: 1,
            records: [DhryRecord::empty(); 2],
            int_glob: 0,
            bool_glob: false,
            ch_1_glob: 0,
            ch_2_glob: 0,
            arr_1_glob: [0; 50],
            arr_2_glob: [[0; 50]; 50],
            last_int_1_loc: 0,
            last_int_2_loc: 0,
            last_int_3_loc: 0,
            last_enum_loc: DhryIdent::Ident1,
            last_str_1_loc: [0; 31],
            last_str_2_loc: [0; 31],
        }
    }

    fn initialize(&mut self) {
        self.records = [DhryRecord::empty(); 2];
        self.arr_1_glob = [0; 50];
        self.arr_2_glob = [[0; 50]; 50];
        self.ptr_glob = 0;
        self.next_ptr_glob = 1;

        self.records[self.ptr_glob].ptr_comp = self.next_ptr_glob;
        self.records[self.ptr_glob].discr = DhryIdent::Ident1;
        self.records[self.ptr_glob].enum_comp = DhryIdent::Ident3;
        self.records[self.ptr_glob].int_comp = 40;
        dhry_strcpy(
            &mut self.records[self.ptr_glob].str_comp,
            DHRYSTONE_SOME_STRING,
        );

        self.arr_2_glob[8][7] = 10;
    }

    #[inline(never)]
    fn run(&mut self, number_of_runs: u64) -> u64 {
        let mut int_1_loc = 0i32;
        let mut int_2_loc = 0i32;
        let mut int_3_loc = 0i32;
        let mut enum_loc = DhryIdent::Ident1;
        let mut str_1_loc = [0u8; 31];
        let mut str_2_loc = [0u8; 31];

        self.initialize();
        dhry_strcpy(&mut str_1_loc, DHRYSTONE_1ST_STRING);

        for run_index in 1..=number_of_runs {
            let run_index = core::hint::black_box(run_index);
            self.proc_5();
            self.proc_4();

            int_1_loc = 2;
            int_2_loc = 3;
            dhry_strcpy(&mut str_2_loc, DHRYSTONE_2ND_STRING);
            enum_loc = DhryIdent::Ident2;
            self.bool_glob = !self.func_2(&str_1_loc, &str_2_loc);

            while int_1_loc < int_2_loc {
                int_3_loc = 5 * int_1_loc - int_2_loc;
                core::hint::black_box(int_3_loc);
                int_3_loc = dhry_proc_7(int_1_loc, int_2_loc);
                int_1_loc += 1;
            }

            self.proc_8(int_1_loc, int_3_loc);
            self.proc_1(self.ptr_glob);

            let mut ch_index = b'A';
            while ch_index <= self.ch_2_glob {
                if enum_loc == self.func_1(core::hint::black_box(ch_index), b'C') {
                    enum_loc = self.proc_6(DhryIdent::Ident1);
                    dhry_strcpy(&mut str_2_loc, DHRYSTONE_3RD_STRING);
                    int_2_loc = run_index as i32;
                    self.int_glob = run_index as i32;
                }
                ch_index += 1;
            }

            int_2_loc *= int_1_loc;
            int_1_loc = int_2_loc / int_3_loc;
            int_2_loc = 7 * (int_2_loc - int_3_loc) - int_1_loc;
            self.proc_2(&mut int_1_loc);
        }

        self.last_int_1_loc = int_1_loc;
        self.last_int_2_loc = int_2_loc;
        self.last_int_3_loc = int_3_loc;
        self.last_enum_loc = enum_loc;
        self.last_str_1_loc = str_1_loc;
        self.last_str_2_loc = str_2_loc;

        number_of_runs
    }

    fn verify(&self, number_of_runs: u64) -> i32 {
        let mut failures = 0;

        failures += (self.int_glob != 5) as i32;
        failures += (self.bool_glob != true) as i32;
        failures += (self.ch_1_glob != b'A') as i32;
        failures += (self.ch_2_glob != b'B') as i32;
        failures += (self.arr_1_glob[8] != 7) as i32;
        failures += (self.arr_2_glob[8][7] != (number_of_runs as i32 + 10)) as i32;
        failures += (self.records[self.ptr_glob].ptr_comp != self.next_ptr_glob) as i32;
        failures += (self.records[self.ptr_glob].discr != DhryIdent::Ident1) as i32;
        failures += (self.records[self.ptr_glob].enum_comp != DhryIdent::Ident3) as i32;
        failures += (self.records[self.ptr_glob].int_comp != 17) as i32;
        failures +=
            (!dhry_streq(&self.records[self.ptr_glob].str_comp, DHRYSTONE_SOME_STRING)) as i32;
        failures += (self.records[self.next_ptr_glob].ptr_comp != self.next_ptr_glob) as i32;
        failures += (self.records[self.next_ptr_glob].discr != DhryIdent::Ident1) as i32;
        failures += (self.records[self.next_ptr_glob].enum_comp != DhryIdent::Ident2) as i32;
        failures += (self.records[self.next_ptr_glob].int_comp != 18) as i32;
        failures += (!dhry_streq(
            &self.records[self.next_ptr_glob].str_comp,
            DHRYSTONE_SOME_STRING,
        )) as i32;
        failures += (self.last_int_1_loc != 5) as i32;
        failures += (self.last_int_2_loc != 13) as i32;
        failures += (self.last_int_3_loc != 7) as i32;
        failures += (self.last_enum_loc != DhryIdent::Ident2) as i32;
        failures += (!dhry_streq(&self.last_str_1_loc, DHRYSTONE_1ST_STRING)) as i32;
        failures += (!dhry_streq(&self.last_str_2_loc, DHRYSTONE_2ND_STRING)) as i32;

        failures
    }

    #[inline(never)]
    fn proc_1(&mut self, ptr_val_par: usize) {
        let next_record = self.records[ptr_val_par].ptr_comp;

        self.records[next_record] = self.records[self.ptr_glob];
        self.records[ptr_val_par].int_comp = 5;
        self.records[next_record].int_comp = self.records[ptr_val_par].int_comp;
        self.records[next_record].ptr_comp = self.records[ptr_val_par].ptr_comp;
        self.records[next_record].ptr_comp = self.proc_3();

        if self.records[next_record].discr == DhryIdent::Ident1 {
            self.records[next_record].int_comp = 6;
            self.records[next_record].enum_comp = self.proc_6(self.records[ptr_val_par].enum_comp);
            self.records[next_record].ptr_comp = self.records[self.ptr_glob].ptr_comp;
            self.records[next_record].int_comp =
                dhry_proc_7(self.records[next_record].int_comp, 10);
        } else {
            self.records[ptr_val_par] = self.records[self.records[ptr_val_par].ptr_comp];
        }
    }

    #[inline(never)]
    fn proc_2(&mut self, int_par_ref: &mut i32) {
        let mut int_loc = *int_par_ref + 10;
        let mut enum_loc;

        loop {
            enum_loc = DhryIdent::Ident1;
            if self.ch_1_glob == b'A' {
                int_loc -= 1;
                *int_par_ref = int_loc - self.int_glob;
                enum_loc = DhryIdent::Ident1;
            }
            if enum_loc == DhryIdent::Ident1 {
                break;
            }
        }
    }

    #[inline(never)]
    fn proc_3(&mut self) -> usize {
        let ptr_ref_par = self.records[self.ptr_glob].ptr_comp;
        self.records[self.ptr_glob].int_comp = dhry_proc_7(10, self.int_glob);
        ptr_ref_par
    }

    #[inline(never)]
    fn proc_4(&mut self) {
        let bool_loc = self.ch_1_glob == b'A';
        self.bool_glob = bool_loc | self.bool_glob;
        self.ch_2_glob = b'B';
    }

    #[inline(never)]
    fn proc_5(&mut self) {
        self.ch_1_glob = b'A';
        self.bool_glob = false;
    }

    #[inline(never)]
    fn proc_6(&self, enum_val_par: DhryIdent) -> DhryIdent {
        let mut enum_ref_par = enum_val_par;
        if !dhry_func_3(enum_val_par) {
            enum_ref_par = DhryIdent::Ident4;
        }

        match enum_val_par {
            DhryIdent::Ident1 => DhryIdent::Ident1,
            DhryIdent::Ident2 => {
                if self.int_glob > 100 {
                    DhryIdent::Ident1
                } else {
                    DhryIdent::Ident4
                }
            }
            DhryIdent::Ident3 => DhryIdent::Ident2,
            DhryIdent::Ident4 => enum_ref_par,
            DhryIdent::Ident5 => DhryIdent::Ident3,
        }
    }

    #[inline(never)]
    fn proc_8(&mut self, int_1_par_val: i32, int_2_par_val: i32) {
        let int_loc = (int_1_par_val + 5) as usize;
        self.arr_1_glob[int_loc] = int_2_par_val;
        self.arr_1_glob[int_loc + 1] = self.arr_1_glob[int_loc];
        self.arr_1_glob[int_loc + 30] = int_loc as i32;
        for int_index in int_loc..=int_loc + 1 {
            self.arr_2_glob[int_loc][int_index] = int_loc as i32;
        }
        self.arr_2_glob[int_loc][int_loc - 1] += 1;
        self.arr_2_glob[int_loc + 20][int_loc] = self.arr_1_glob[int_loc];
        self.int_glob = 5;
    }

    #[inline(never)]
    fn func_1(&mut self, ch_1_par_val: u8, ch_2_par_val: u8) -> DhryIdent {
        let ch_1_loc = ch_1_par_val;
        let ch_2_loc = ch_1_loc;
        if ch_2_loc != ch_2_par_val {
            DhryIdent::Ident1
        } else {
            self.ch_1_glob = ch_1_loc;
            DhryIdent::Ident2
        }
    }

    #[inline(never)]
    fn func_2(&mut self, str_1_par_ref: &[u8; 31], str_2_par_ref: &[u8; 31]) -> bool {
        let mut ch_loc = b'A';
        let mut int_loc = 2usize;

        while int_loc <= 2 {
            if self.func_1(str_1_par_ref[int_loc], str_2_par_ref[int_loc + 1]) == DhryIdent::Ident1
            {
                ch_loc = b'A';
                int_loc += 1;
            }
        }

        if ch_loc >= b'W' && ch_loc < b'Z' {
            int_loc = 7;
        }

        if ch_loc == b'R' {
            true
        } else if dhry_strcmp(str_1_par_ref, str_2_par_ref) > 0 {
            int_loc += 7;
            self.int_glob = int_loc as i32;
            true
        } else {
            false
        }
    }
}

#[inline(never)]
fn dhry_proc_7(int_1_par_val: i32, int_2_par_val: i32) -> i32 {
    let int_loc = int_1_par_val + 2;
    int_2_par_val + int_loc
}

fn dhry_func_3(enum_par_val: DhryIdent) -> bool {
    let enum_loc = enum_par_val;
    enum_loc == DhryIdent::Ident3
}

fn dhry_strcpy(dst: &mut [u8; 31], src: &[u8]) {
    dst.fill(0);
    let len = core::cmp::min(src.len(), dst.len().saturating_sub(1));
    dst[..len].copy_from_slice(&src[..len]);
}

fn dhry_streq(lhs: &[u8; 31], rhs: &[u8]) -> bool {
    let len = core::cmp::min(rhs.len(), lhs.len());
    lhs[..len] == rhs[..len] && len < lhs.len() && lhs[len] == 0
}

fn dhry_strcmp(lhs: &[u8; 31], rhs: &[u8; 31]) -> i32 {
    let mut index = 0usize;
    while index < lhs.len() {
        let left = lhs[index];
        let right = rhs[index];
        if left != right || left == 0 || right == 0 {
            return left as i32 - right as i32;
        }
        index += 1;
    }
    0
}

fn read_shell_counter() -> u64 {
    let val: u64;
    unsafe {
        core::arch::asm!(
            "mrs {val}, cntpct_el0",
            val = out(reg) val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

fn counter_delta_ns(delta: u64, frequency: u64) -> u64 {
    if frequency == 0 {
        0
    } else {
        ((delta as u128).saturating_mul(1_000_000_000u128) / frequency as u128) as u64
    }
}

/// Command: dhrystone - Run Dhrystone logical multi-core benchmark
fn cmd_dhrystone(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() > 1 {
        ctx.serial.write_str("usage: dhrystone [runs-per-core]\n");
        return;
    }

    let runs_per_core = if args.is_empty() {
        DHRYSTONE_DEFAULT_RUNS_PER_CORE
    } else {
        let Some(parsed) = parse_number(args[0]) else {
            ctx.serial.write_str("usage: dhrystone [runs-per-core]\n");
            return;
        };
        parsed as u64
    };

    if runs_per_core == 0 || runs_per_core > DHRYSTONE_MAX_RUNS_PER_CORE {
        ctx.serial
            .write_str("usage: dhrystone [runs-per-core <= 5000000]\n");
        return;
    }

    let timer_frequency = crate::kernel_lowlevel::timer::get_frequency();
    if timer_frequency == 0 {
        ctx.serial
            .write_str("dhrystone: timer frequency is not initialized\n");
        return;
    }

    let logical_cpus = core::cmp::max(crate::kernel_lowlevel::smp::online_cpu_count() as u64, 1);
    let configured_cpus = crate::kernel_lowlevel::smp::MAX_CPUS as u64;

    ctx.serial
        .write_str("\nDhrystone 2.1 SMROS shell benchmark\n");
    ctx.serial
        .write_str("source: Rust port of BYTE UnixBench dhry_1.c/dhry_2.c/dhry.h\n");
    ctx.serial
        .write_str("mode: logical multi-core projection from one measured worker\n");
    ctx.serial.write_str("configured_cores=");
    print_u64(&mut ctx.serial, configured_cpus);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("online_cores=");
    print_u64(&mut ctx.serial, logical_cpus);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("runs_per_core=");
    print_u64(&mut ctx.serial, runs_per_core);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("total_logical_runs=");
    print_u128(
        &mut ctx.serial,
        (runs_per_core as u128).saturating_mul(logical_cpus as u128),
    );
    ctx.serial.write_str("\n");

    let mut state = DhryState::new();
    let start = read_shell_counter();
    let count = state.run(core::hint::black_box(runs_per_core));
    let elapsed_counts = read_shell_counter().saturating_sub(start);
    let elapsed_ns = counter_delta_ns(elapsed_counts, timer_frequency);
    let failures = state.verify(count);
    core::hint::black_box(&state);

    ctx.serial.write_str("elapsed_ns_per_core=");
    print_u64(&mut ctx.serial, elapsed_ns);
    ctx.serial.write_str("\n");

    if failures != 0 {
        ctx.serial.write_str("verify: FAIL ");
        print_number(&mut ctx.serial, failures as u32);
        ctx.serial.write_str("\n");
        ctx.serial.write_str("dhrystone: FAIL\n");
        return;
    }

    ctx.serial.write_str("verify: PASS\n");
    if elapsed_ns == 0 {
        ctx.serial
            .write_str("dhrystone: elapsed_ns=0, increase runs-per-core\n");
        return;
    }

    let single_core_dps = (count as u128).saturating_mul(1_000_000_000u128) / elapsed_ns as u128;
    let aggregate_dps = single_core_dps.saturating_mul(logical_cpus as u128);
    let dmips_per_core_x100 = single_core_dps.saturating_mul(100) / DHRYSTONE_DMIPS_DIVISOR;
    let dmips_total_x100 = aggregate_dps.saturating_mul(100) / DHRYSTONE_DMIPS_DIVISOR;

    ctx.serial.write_str("dhrystones_per_second_per_core=");
    print_u128(&mut ctx.serial, single_core_dps);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("dhrystones_per_second_");
    print_u64(&mut ctx.serial, logical_cpus);
    ctx.serial.write_str("_core=");
    print_u128(&mut ctx.serial, aggregate_dps);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("dmips_per_core=");
    print_fixed_x100(&mut ctx.serial, dmips_per_core_x100);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("dmips_");
    print_u64(&mut ctx.serial, logical_cpus);
    ctx.serial.write_str("_core=");
    print_fixed_x100(&mut ctx.serial, dmips_total_x100);
    ctx.serial.write_str("\n");
    ctx.serial.write_str("dhrystone: PASS\n");
}

/// Command: sched - Show or configure scheduler policy
fn cmd_sched(ctx: &mut ShellContext, args: &[&str]) {
    let s = scheduler::scheduler();
    if args.is_empty() || args[0] == "status" {
        ctx.serial.write_str("scheduler policy: ");
        ctx.serial.write_str(s.policy().as_str());
        ctx.serial.write_str("\nactive threads: ");
        print_number(&mut ctx.serial, s.active_threads() as u32);
        ctx.serial.write_str("\ntick count: ");
        print_number(&mut ctx.serial, s.get_tick_count() as u32);
        ctx.serial.write_str("\ntime slice: ");
        print_number(&mut ctx.serial, scheduler_ticks_to_ms(s.time_slice_ticks()));
        ctx.serial.write_str(" ms\n");
        ctx.serial.write_str("trace samples: ");
        print_usize(&mut ctx.serial, s.trace_len());
        ctx.serial.write_str("/");
        print_usize(
            &mut ctx.serial,
            crate::kernel_objects::scheduler::SCHED_TRACE_CAPACITY,
        );
        ctx.serial.write_str("\n");
        return;
    }

    if args[0] == "set" {
        if args.len() < 2 {
            ctx.serial
                .write_str("usage: sched set <rr|edf|credit|fair>\n");
            return;
        }
        let Some(policy) = scheduler::SchedulePolicy::from_str(args[1]) else {
            ctx.serial
                .write_str("usage: sched set <rr|edf|credit|fair>\n");
            return;
        };
        s.set_policy(policy);
        ctx.serial.write_str("scheduler policy set to ");
        ctx.serial.write_str(policy.as_str());
        ctx.serial.write_str("\n");
        return;
    }

    if args[0] == "slice" {
        cmd_sched_slice(ctx, &args[1..]);
        return;
    }

    if args[0] == "credit" {
        cmd_sched_credit(ctx, &args[1..]);
        return;
    }

    if args[0] == "cpu" {
        cmd_sched_cpu(ctx, &args[1..]);
        return;
    }

    if args[0] == "priority" || args[0] == "prio" {
        cmd_sched_priority(ctx, &args[1..]);
        return;
    }

    if args[0] == "test" {
        let result = s.run_policy_self_test();
        ctx.serial.write_str("scheduler policy self-test:\n");
        ctx.serial.write_str("  round-robin selected T");
        print_number(&mut ctx.serial, result.round_robin as u32);
        ctx.serial.write_str(" (expected T2)\n");
        ctx.serial.write_str("  edf selected T");
        print_number(&mut ctx.serial, result.edf as u32);
        ctx.serial.write_str(" (expected T2)\n");
        ctx.serial.write_str("  credit selected T");
        print_number(&mut ctx.serial, result.credit as u32);
        ctx.serial.write_str(" (expected T3)\n");
        ctx.serial.write_str("  fair selected T");
        print_number(&mut ctx.serial, result.fair as u32);
        ctx.serial.write_str(" (expected T2)\n");
        ctx.serial.write_str("  edf cpu0 selected T");
        print_number(&mut ctx.serial, result.cpu_filtered as u32);
        ctx.serial.write_str(" (expected T3)\n");
        if result.round_robin == 2
            && result.edf == 2
            && result.credit == 3
            && result.fair == 2
            && result.cpu_filtered == 3
        {
            ctx.serial.write_str("[OK] scheduler policy test passed\n");
        } else {
            ctx.serial
                .write_str("[FAIL] scheduler policy test failed\n");
        }
        return;
    }

    if args[0] == "perfetto" {
        cmd_sched_perfetto(ctx, &args[1..]);
        return;
    }

    if args[0] == "sample" {
        cmd_sched_sample(ctx, &args[1..]);
        return;
    }

    ctx.serial.write_str(
        "usage: sched [status|set <rr|edf|credit|fair>|slice <thread_id> <ms>|credit <thread_id> <credit>|cpu <thread_id> <cpu|any>|priority <thread_id> <1..255 higher=wins>|test|sample [workers]|perfetto [samples]]\n",
    );
}

fn cmd_sched_slice(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() != 2 {
        ctx.serial
            .write_str("usage: sched slice <thread_id> <ms>\n");
        return;
    }
    let Some(thread_id) = parse_number(args[0]) else {
        ctx.serial
            .write_str("usage: sched slice <thread_id> <ms>\n");
        return;
    };
    let Some(slice_ms) = parse_number(args[1]) else {
        ctx.serial
            .write_str("usage: sched slice <thread_id> <ms>\n");
        return;
    };
    if slice_ms == 0 || slice_ms > u32::MAX as usize {
        ctx.serial
            .write_str("usage: sched slice <thread_id> <ms > 0>\n");
        return;
    }
    let Some(ticks) = scheduler_ms_to_ticks(slice_ms as u32) else {
        ctx.serial.write_str("sched slice: invalid duration\n");
        return;
    };

    let id = crate::kernel_lowlevel::thread::ThreadId(thread_id);
    if !scheduler::scheduler().set_thread_time_slice(id, ticks) {
        ctx.serial.write_str("sched slice: invalid thread id\n");
        return;
    }

    ctx.serial.write_str("scheduler thread T");
    print_usize(&mut ctx.serial, thread_id);
    ctx.serial.write_str(" time slice set to ");
    print_usize(&mut ctx.serial, slice_ms);
    ctx.serial.write_str(" ms");
    let actual_ms = scheduler_ticks_to_ms(ticks);
    if actual_ms != slice_ms as u32 {
        ctx.serial.write_str(" (rounded to ");
        print_number(&mut ctx.serial, actual_ms);
        ctx.serial.write_str(" ms)");
    }
    ctx.serial.write_str("\n");
}

fn cmd_sched_credit(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() != 2 {
        ctx.serial
            .write_str("usage: sched credit <thread_id> <credit>\n");
        return;
    }
    let Some(thread_id) = parse_number(args[0]) else {
        ctx.serial
            .write_str("usage: sched credit <thread_id> <credit>\n");
        return;
    };
    let Some(credit) = parse_number(args[1]) else {
        ctx.serial
            .write_str("usage: sched credit <thread_id> <credit>\n");
        return;
    };
    if credit > i32::MAX as usize {
        ctx.serial
            .write_str("usage: sched credit <thread_id> <0..2147483647>\n");
        return;
    }

    let id = crate::kernel_lowlevel::thread::ThreadId(thread_id);
    if !scheduler::scheduler().set_thread_credit_value(id, credit as i32) {
        ctx.serial.write_str("sched credit: invalid thread id\n");
        return;
    }

    ctx.serial.write_str("scheduler thread T");
    print_usize(&mut ctx.serial, thread_id);
    ctx.serial.write_str(" credit set to ");
    print_usize(&mut ctx.serial, credit);
    ctx.serial.write_str("\n");
}

fn cmd_sched_cpu(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() != 2 {
        ctx.serial
            .write_str("usage: sched cpu <thread_id> <cpu|any>\n");
        return;
    }
    let Some(thread_id) = parse_number(args[0]) else {
        ctx.serial
            .write_str("usage: sched cpu <thread_id> <cpu|any>\n");
        return;
    };

    let cpu_affinity = if args[1].eq_ignore_ascii_case("any")
        || args[1].eq_ignore_ascii_case("none")
        || args[1] == "*"
    {
        None
    } else {
        let Some(cpu) = parse_number(args[1]) else {
            ctx.serial
                .write_str("usage: sched cpu <thread_id> <cpu|any>\n");
            return;
        };
        Some(cpu)
    };

    let id = crate::kernel_lowlevel::thread::ThreadId(thread_id);
    if !scheduler::scheduler().set_thread_cpu_affinity(id, cpu_affinity) {
        ctx.serial
            .write_str("sched cpu: invalid thread id or cpu\n");
        return;
    }

    ctx.serial.write_str("scheduler thread T");
    print_usize(&mut ctx.serial, thread_id);
    ctx.serial.write_str(" cpu affinity set to ");
    if let Some(cpu) = cpu_affinity {
        print_usize(&mut ctx.serial, cpu);
    } else {
        ctx.serial.write_str("any");
    }
    ctx.serial.write_str("\n");
}

fn cmd_sched_priority(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() != 2 {
        ctx.serial
            .write_str("usage: sched priority <thread_id> <priority>\n");
        return;
    }
    let Some(thread_id) = parse_number(args[0]) else {
        ctx.serial
            .write_str("usage: sched priority <thread_id> <priority>\n");
        return;
    };
    let Some(priority) = parse_number(args[1]) else {
        ctx.serial
            .write_str("usage: sched priority <thread_id> <priority>\n");
        return;
    };
    if priority == 0 || priority > u8::MAX as usize {
        ctx.serial
            .write_str("usage: sched priority <thread_id> <1..255>\n");
        return;
    }

    let id = crate::kernel_lowlevel::thread::ThreadId(thread_id);
    if !scheduler::scheduler().set_thread_priority(id, priority as u8) {
        ctx.serial.write_str("sched priority: invalid thread id\n");
        return;
    }

    ctx.serial.write_str("scheduler thread T");
    print_usize(&mut ctx.serial, thread_id);
    ctx.serial.write_str(" priority set to ");
    print_usize(&mut ctx.serial, priority);
    ctx.serial.write_str(" (higher value preempts lower)");
    ctx.serial.write_str("\n");
}

fn cmd_sched_perfetto(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() > 1 {
        ctx.serial.write_str("usage: sched perfetto [samples]\n");
        return;
    }

    let requested = if let Some(value) = args.first() {
        let Some(parsed) = parse_number(value) else {
            ctx.serial.write_str("usage: sched perfetto [samples]\n");
            return;
        };
        parsed
    } else {
        96usize
    };

    if requested == 0 {
        ctx.serial
            .write_str("usage: sched perfetto [samples > 0]\n");
        return;
    }

    match crate::user_level::perfetto::export_scheduler_trace(requested) {
        Ok(export) => print_perfetto_sched_trace(ctx, &export),
        Err(err) => {
            ctx.serial.write_str("sched perfetto: ");
            ctx.serial.write_str(err.as_str());
            ctx.serial.write_str("\n");
        }
    }
}

fn cmd_sched_sample(ctx: &mut ShellContext, args: &[&str]) {
    if args.len() > 1 {
        ctx.serial.write_str("usage: sched sample [workers]\n");
        return;
    }
    let workers = if let Some(value) = args.first() {
        let Some(parsed) = parse_number(value) else {
            ctx.serial.write_str("usage: sched sample [workers]\n");
            return;
        };
        parsed
    } else {
        crate::kernel_objects::scheduler::SCHED_SAMPLE_MAX_WORKERS
    };

    if workers == 0 {
        ctx.serial.write_str("usage: sched sample [workers > 0]\n");
        return;
    }

    let result = scheduler::scheduler().start_sample_workers(workers);
    ctx.serial.write_str("scheduler sample workers: created ");
    print_usize(&mut ctx.serial, result.created);
    ctx.serial.write_str("/");
    print_usize(&mut ctx.serial, result.requested);
    ctx.serial.write_str(" across ");
    print_usize(&mut ctx.serial, result.online_cpus);
    ctx.serial.write_str(" logical CPUs");
    if result.failed != 0 {
        ctx.serial.write_str(" failed=");
        print_usize(&mut ctx.serial, result.failed);
    }
    ctx.serial.write_str("\n");
    ctx.serial
        .write_str("run `sched perfetto` to export /shared/trace.pftrace\n");
    for slot in 0..result.created {
        let cpu = slot % core::cmp::max(result.online_cpus, 1);
        scheduler::yield_now_on_cpu(cpu);
    }
}

/// Command: vm - Create, start, or force-stop modeled VMs
fn cmd_vm(ctx: &mut ShellContext, args: &[&str]) {
    if args.is_empty() {
        print_vm_usage(ctx);
        return;
    }

    match args[0] {
        "-c" => {
            if args.len() != 2 {
                print_vm_usage(ctx);
                return;
            }
            let Some(path) = normalize_fxfs_path(ctx.cwd.as_str(), args[1]) else {
                ctx.serial.write_str("vm: invalid config path\n");
                return;
            };
            let data = match read_fxfs_file_to_vec(path.as_str()) {
                Ok(data) => data,
                Err(err) => {
                    ctx.serial.write_str("vm: config ");
                    print_fxfs_error(ctx, err);
                    ctx.serial.write_str("\n");
                    return;
                }
            };
            let Ok(config_xml) = core::str::from_utf8(data.as_slice()) else {
                ctx.serial.write_str("vm: config is not UTF-8 XML\n");
                return;
            };
            let tick = scheduler::scheduler().get_tick_count();
            match crate::kernel_objects::hypervisor::hypervisor().start_vm(
                path.as_str(),
                config_xml,
                tick,
            ) {
                Ok(mut vm) => {
                    let host_launch_configured = vm.host.is_some();
                    if host_launch_configured {
                        match crate::user_level::vm_host::launch(&vm) {
                            Ok(launch) => {
                                vm.host_qemu_pid = launch.qemu_pid;
                                let _ = crate::kernel_objects::hypervisor::hypervisor()
                                    .set_host_qemu_pid(vm.name.as_str(), launch.qemu_pid);
                            }
                            Err(err) => {
                                let _ = crate::kernel_objects::hypervisor::hypervisor()
                                    .kill_vm(vm.name.as_str(), tick);
                                ctx.serial.write_str("vm: host launch failed for ");
                                ctx.serial.write_str(vm.name.as_str());
                                ctx.serial.write_str(": ");
                                print_vm_host_error(ctx, err);
                                print_vm_host_hint(ctx, err);
                                return;
                            }
                        }
                    }

                    ctx.serial.write_str("vm: started ");
                    ctx.serial.write_str(vm.name.as_str());
                    ctx.serial.write_str(" pid=");
                    print_usize(&mut ctx.serial, vm.process_pid);
                    ctx.serial.write_str(" guest=0x");
                    print_hex(&mut ctx.serial, vm.guest_handle as u64);
                    ctx.serial.write_str(" vcpu=0x");
                    print_hex(&mut ctx.serial, vm.vcpu_handle as u64);
                    ctx.serial.write_str("\n  cpu_slice_us=");
                    print_number(&mut ctx.serial, vm.cpu_time_slice_us);
                    ctx.serial.write_str(" rt_priority=");
                    print_number(&mut ctx.serial, vm.realtime_priority as u32);
                    ctx.serial.write_str(" memory_kb=");
                    print_usize(&mut ctx.serial, vm.memory_bytes / 1024);
                    ctx.serial.write_str(" swap=disabled restart=");
                    ctx.serial.write_str(if vm.restart_on_crash {
                        "on-crash"
                    } else {
                        "never"
                    });
                    ctx.serial.write_str("\n");
                    if host_launch_configured {
                        ctx.serial.write_str("  host_qemu_pid=");
                        print_number(&mut ctx.serial, vm.host_qemu_pid);
                        ctx.serial.write_str(" window=requested\n");
                    } else {
                        ctx.serial
                            .write_str("  host launch=not configured (no <linux kernel=...>)\n");
                    }
                }
                Err(err) => {
                    ctx.serial.write_str("vm: config ");
                    ctx.serial.write_str(err.as_str());
                    ctx.serial.write_str("\n");
                }
            }
        }
        "-k" => {
            if args.len() != 2 {
                print_vm_usage(ctx);
                return;
            }
            let name = args[1];
            let tick = scheduler::scheduler().get_tick_count();
            match crate::kernel_objects::hypervisor::hypervisor().kill_vm(name, tick) {
                Ok(vm) => {
                    let host_stop = crate::user_level::vm_host::stop(&vm);
                    ctx.serial.write_str("vm: force-stopped ");
                    ctx.serial.write_str(vm.name.as_str());
                    ctx.serial
                        .write_str(" without rescheduling critical realtime tasks\n");
                    if vm.host.is_some() {
                        match host_stop {
                            Ok(()) => ctx.serial.write_str("  host qemu stop requested\n"),
                            Err(err) => {
                                ctx.serial.write_str("  host qemu stop failed: ");
                                print_vm_host_error(ctx, err);
                                ctx.serial.write_str("\n");
                            }
                        }
                    }
                }
                Err(_) => {
                    ctx.serial.write_str("vm: not found: ");
                    ctx.serial.write_str(name);
                    ctx.serial.write_str("\n");
                }
            }
        }
        "-s" => {
            if args.len() != 1 {
                print_vm_usage(ctx);
                return;
            }
            print_vm_status(ctx);
        }
        _ => print_vm_usage(ctx),
    }
}

fn print_vm_usage(ctx: &mut ShellContext) {
    ctx.serial
        .write_str("usage: vm -c <config.xml> | vm -k <VM-name> | vm -s\n");
}

fn print_vm_status(ctx: &mut ShellContext) {
    let tick = scheduler::scheduler().get_tick_count();
    let status = crate::kernel_objects::hypervisor::hypervisor().status(tick);

    ctx.serial.write_str("\nHypervisor daemon status\n");
    ctx.serial.write_str("  VMs: ");
    print_usize(&mut ctx.serial, status.stats.vm_count);
    ctx.serial.write_str(" running=");
    print_usize(&mut ctx.serial, status.stats.running_vms);
    ctx.serial.write_str(" stopped=");
    print_usize(&mut ctx.serial, status.stats.stopped_vms);
    ctx.serial.write_str(" crashed=");
    print_usize(&mut ctx.serial, status.stats.crashed_vms);
    ctx.serial.write_str("\n  total_memory_kb=");
    print_usize(&mut ctx.serial, status.stats.total_memory_bytes / 1024);
    ctx.serial.write_str(" cpu_slice_us=");
    print_number(&mut ctx.serial, status.stats.total_cpu_time_slice_us);
    ctx.serial.write_str(" monitor_latency_us<");
    print_number(&mut ctx.serial, status.stats.monitor_latency_us + 1);
    ctx.serial.write_str("\n  fault_domains=");
    print_usize(&mut ctx.serial, status.stats.fault_domains);
    ctx.serial.write_str(" forced_kills=");
    print_number(&mut ctx.serial, status.stats.forced_kills);
    ctx.serial.write_str(" auto_restarts=");
    print_number(&mut ctx.serial, status.stats.auto_restarts);
    ctx.serial.write_str("\n");

    if status.vms.is_empty() {
        ctx.serial.write_str("  no VMs\n");
        return;
    }

    ctx.serial.write_str(
        "  Name              PID  HostPID  State    CPU%  Slice(us)  RT  Mem(KB)  Restart  Uptime(ticks)\n",
    );
    for vm in &status.vms {
        ctx.serial.write_str("  ");
        ctx.serial.write_str(vm.name.as_str());
        for _ in 0..(18usize.saturating_sub(vm.name.len())) {
            ctx.serial.write_byte(b' ');
        }
        print_padded_number(&mut ctx.serial, vm.process_pid as u32, 3);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, vm.host_qemu_pid, 7);
        ctx.serial.write_str("  ");
        ctx.serial.write_str(vm.state.as_str());
        for _ in 0..(9usize.saturating_sub(vm.state.as_str().len())) {
            ctx.serial.write_byte(b' ');
        }
        print_padded_number(&mut ctx.serial, vm.cpu_usage_percent(status.tick), 3);
        ctx.serial.write_str("   ");
        print_padded_number(&mut ctx.serial, vm.cpu_time_slice_us, 9);
        ctx.serial.write_str("  ");
        print_padded_number(&mut ctx.serial, vm.realtime_priority as u32, 2);
        ctx.serial.write_str("  ");
        print_usize(&mut ctx.serial, vm.memory_bytes / 1024);
        ctx.serial.write_str("     ");
        ctx.serial.write_str(if vm.restart_on_crash {
            "on-crash"
        } else {
            "never"
        });
        ctx.serial.write_str("  ");
        print_u64(&mut ctx.serial, vm.uptime_ticks(status.tick));
        ctx.serial.write_str("\n");
    }
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

/// Parse a decimal or 0x-prefixed hexadecimal number from a string.
fn parse_number(s: &str) -> Option<usize> {
    let mut result: usize = 0;
    let (digits, radix) = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (hex, 16usize)
    } else {
        (s, 10usize)
    };
    if digits.is_empty() {
        return None;
    }
    for byte in digits.bytes() {
        let digit = match byte {
            b'0'..=b'9' => (byte - b'0') as usize,
            b'a'..=b'f' if radix == 16 => 10 + (byte - b'a') as usize,
            b'A'..=b'F' if radix == 16 => 10 + (byte - b'A') as usize,
            _ => return None,
        };
        if digit >= radix {
            return None;
        }
        result = result.checked_mul(radix)?.checked_add(digit)?;
    }
    Some(result)
}

fn scheduler_ticks_to_ms(ticks: u32) -> u32 {
    ticks.saturating_mul(SCHED_TICK_MS)
}

fn scheduler_ms_to_ticks(ms: u32) -> Option<u32> {
    if ms == 0 || SCHED_TICK_MS == 0 {
        return None;
    }
    Some(ms.saturating_add(SCHED_TICK_MS - 1) / SCHED_TICK_MS)
}

fn saturating_u64_to_u32(value: u64) -> u32 {
    if value > u32::MAX as u64 {
        u32::MAX
    } else {
        value as u32
    }
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

fn print_padded_usize(serial: &mut Serial, num: usize, width: usize) {
    let mut buf = [0u8; 20];
    let len = decimal_usize_digits(num, &mut buf);
    for _ in 0..width.saturating_sub(len) {
        serial.write_byte(b' ');
    }
    for j in (0..len).rev() {
        serial.write_byte(buf[j]);
    }
}

fn print_padded_u64(serial: &mut Serial, num: u64, width: usize) {
    let mut buf = [0u8; 20];
    let len = decimal_u64_digits(num, &mut buf);
    for _ in 0..width.saturating_sub(len) {
        serial.write_byte(b' ');
    }
    for j in (0..len).rev() {
        serial.write_byte(buf[j]);
    }
}

fn print_padded_str(serial: &mut Serial, value: &str, width: usize) {
    serial.write_str(value);
    for _ in 0..width.saturating_sub(value.len()) {
        serial.write_byte(b' ');
    }
}

fn pad_to_width(serial: &mut Serial, used: usize, width: usize) {
    for _ in 0..width.saturating_sub(used) {
        serial.write_byte(b' ');
    }
}

fn hex_digit_count(mut value: u64) -> usize {
    if value == 0 {
        return 1;
    }

    let mut count = 0usize;
    while value > 0 {
        count += 1;
        value >>= 4;
    }
    count
}

fn hex_value_width(value: u64) -> usize {
    2usize.saturating_add(hex_digit_count(value))
}

fn hex_range_width(start: u64, end: u64) -> usize {
    hex_value_width(start)
        .saturating_add(1)
        .saturating_add(hex_value_width(end))
}

fn hex_pair_width(first: u64, second: u64) -> usize {
    hex_value_width(first)
        .saturating_add(1)
        .saturating_add(hex_value_width(second))
}

fn ps_arm64_pgd_index(vaddr: usize) -> usize {
    (vaddr >> 39) & 0x1ff
}

fn ps_smros_indexed_pmd(vaddr: usize) -> usize {
    (vaddr >> 21) & 0x1ff
}

fn decimal_usize_digits(mut num: usize, buf: &mut [u8]) -> usize {
    if num == 0 {
        buf[0] = b'0';
        return 1;
    }

    let mut len = 0usize;
    while num > 0 && len < buf.len() {
        buf[len] = b'0' + (num % 10) as u8;
        num /= 10;
        len += 1;
    }
    len
}

fn decimal_u64_digits(mut num: u64, buf: &mut [u8]) -> usize {
    if num == 0 {
        buf[0] = b'0';
        return 1;
    }

    let mut len = 0usize;
    while num > 0 && len < buf.len() {
        buf[len] = b'0' + (num % 10) as u8;
        num /= 10;
        len += 1;
    }
    len
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

fn print_usize(serial: &mut Serial, mut num: usize) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 20];
    let mut i = 0;

    while num > 0 && i < buf.len() {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_u64(serial: &mut Serial, mut num: u64) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 20];
    let mut i = 0;

    while num > 0 && i < buf.len() {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_u128(serial: &mut Serial, mut num: u128) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 39];
    let mut i = 0;

    while num > 0 && i < buf.len() {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_fixed_x100(serial: &mut Serial, value_x100: u128) {
    print_u128(serial, value_x100 / 100);
    serial.write_byte(b'.');
    let frac = (value_x100 % 100) as u8;
    serial.write_byte(b'0' + (frac / 10));
    serial.write_byte(b'0' + (frac % 10));
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

fn print_mac(serial: &mut Serial, mac: [u8; 6]) {
    for (index, byte) in mac.iter().enumerate() {
        if index > 0 {
            serial.write_byte(b':');
        }
        print_fixed_hex_byte(serial, *byte);
    }
}

fn print_ipv4(serial: &mut Serial, ip: [u8; 4]) {
    for (index, octet) in ip.iter().enumerate() {
        if index > 0 {
            serial.write_byte(b'.');
        }
        print_number(serial, *octet as u32);
    }
}

fn print_fixed_hex_byte(serial: &mut Serial, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    serial.write_byte(HEX[(byte >> 4) as usize]);
    serial.write_byte(HEX[(byte & 0x0f) as usize]);
}

fn print_octal(serial: &mut Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 12];
    let mut i = 0;
    while num > 0 && i < buf.len() {
        buf[i] = b'0' + (num & 0x7) as u8;
        num >>= 3;
        i += 1;
    }

    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}

fn print_zx_error(serial: &mut Serial, err: crate::syscall::ZxError) {
    let code = err as i32;
    print_zx_status_i32(serial, code);
}

fn print_zx_status_i32(serial: &mut Serial, code: i32) {
    if code < 0 {
        serial.write_str("-");
        print_number(serial, (-code) as u32);
    } else {
        print_number(serial, code as u32);
    }
}

fn print_i32_shell(serial: &mut Serial, value: i32) {
    if value < 0 {
        serial.write_str("-");
        print_number(serial, value.saturating_abs() as u32);
    } else {
        print_number(serial, value as u32);
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
            let _ = scheduler().bind_thread_process(id, 1);
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
