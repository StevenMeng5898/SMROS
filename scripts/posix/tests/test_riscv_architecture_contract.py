import pathlib
import re
import unittest


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[3]


class RiscvArchitectureContractTests(unittest.TestCase):
    def test_fork_deferred_updates_are_aarch64_only(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_process_memory.rs").read_text()
        self.assertRegex(
            source,
            r"#\[cfg\(target_arch = \"aarch64\"\)\]\n"
            r"\s*address_space\.begin_deferred_user_updates\(\);",
        )
        self.assertRegex(
            source,
            r"#\[cfg\(target_arch = \"aarch64\"\)\]\n"
            r"\s*child\.address_space\.end_deferred_user_updates\(\);",
        )

    def test_riscv_futex_wake_address_is_available(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_futex.rs").read_text()
        self.assertRegex(
            source,
            r"#\[cfg\(any\(target_arch = \"aarch64\", target_arch = \"riscv64\"\)\)\]\n"
            r"pub\(crate\) fn wake_address",
        )
        syscall_source = (REPOSITORY_ROOT / "src/syscall/syscall.rs").read_text()
        self.assertRegex(
            syscall_source,
            r"#\[cfg\(any\(target_arch = \"aarch64\", target_arch = \"riscv64\"\)\)\]\n"
            r"pub\(crate\) fn linux_user_range_readable",
        )

    def test_riscv_exception_return_state_has_an_implementation(self):
        source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/cpu.rs").read_text()
        self.assertRegex(source, r"pub fn read_exception_return_state\(\) -> u64")

    def test_guest_protocol_and_elf_parser_use_riscv_architecture(self):
        posix_source = (REPOSITORY_ROOT / "src/user_level/services/posix_test.rs").read_text()
        self.assertRegex(
            posix_source,
            r"#\[cfg\(all\(not\(test\), target_arch = \"riscv64\"\)\)\]\n"
            r"const POSIX_EVENT_ARCHITECTURE: &str = \"riscv64\";",
        )
        logic_source = (REPOSITORY_ROOT / "src/user_level/services/user_logic.rs").read_text()
        self.assertRegex(
            logic_source,
            r"USER_ELF_MACHINE_RISCV64: u16 = 243",
        )
        elf_source = (REPOSITORY_ROOT / "src/user_level/services/elf.rs").read_text()
        self.assertRegex(
            elf_source,
            r"write_u16_le\(&mut image, 18, ELF_MACHINE\);",
        )

    def test_riscv_baseline_validates_riscv_build_rows_and_platform(self):
        source = (REPOSITORY_ROOT / "scripts/posix/baseline.py").read_text()
        self.assertRegex(
            source,
            r"_load_build_results\(\s*stage / \"build-results\.ndjson\",\s*tests,\s*"
            r"architecture=metadata\.architecture",
        )
        self.assertIn('f"{architecture}-linux-reference"', source)

    def test_riscv_report_and_serial_events_are_architecture_aware(self):
        report_source = (REPOSITORY_ROOT / "scripts/posix/report.py").read_text()
        self.assertIn('def linux_reference_platform(architecture: str) -> str:', report_source)
        self.assertIn('def smros_platform(architecture: str) -> str:', report_source)
        self.assertRegex(
            report_source,
            r"_load_build_results\(\s*descriptor,\s*tests,\s*revision=metadata\.revision,\s*"
            r"architecture=metadata\.architecture",
        )
        events_source = (REPOSITORY_ROOT / "scripts/posix/events.py").read_text()
        self.assertIn("def parse_serial_log(", events_source)
        self.assertIn('expected_architecture: str = "aarch64"', events_source)

    def test_riscv_cli_uses_architecture_specific_baseline_results(self):
        source = (REPOSITORY_ROOT / "scripts/posix/cli.py").read_text()
        self.assertIn("def baseline_results_path(architecture: str) -> Path:", source)
        self.assertRegex(
            source,
            r"baseline_results_path\(arguments\.arch\)",
        )

    def test_smros_runner_refreshes_embedded_stage_before_tests(self):
        source = (REPOSITORY_ROOT / "scripts/posix/qemu_runner.py").read_text()
        self.assertIn("refresh_host_share: bool = False", source)
        self.assertRegex(source, r"transport\.write\(b\"mount share\\n\"\)")
        self.assertIn("refresh_host_share=True", source)

    def test_riscv_single_hart_runtime_uses_logical_cpu_zero(self):
        source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/smp.rs").read_text()
        current_start = source.index("pub fn current_cpu_id() -> u32")
        current_body = source[current_start : source.index("\n}\n", current_start) + 3]
        self.assertRegex(
            current_body,
            r"pub fn current_cpu_id\(\) -> u32 \{[\s\S]*?\b0\s*\}",
            "the active RISC-V runtime hart must use scheduler logical CPU 0",
        )
        self.assertIn("pub fn read_hartid() -> usize", source)
        self.assertIn("read_hartid() as u64", source)

    def test_riscv_user_entry_allows_supervisor_access_to_user_buffers(self):
        source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/cpu.rs").read_text()
        self.assertIn("const SSTATUS_SUM", source)
        switch_start = source.index("pub unsafe fn switch_to_user")
        switch_body = source[switch_start : source.index("\n}\n", switch_start) + 3]
        self.assertRegex(
            switch_body,
            r"sstatus\s*\|=[^\n]*SSTATUS_SUM",
            "RISC-V S-mode must be allowed to save trap frames and copy user buffers",
        )

    def test_riscv_linux_memory_has_real_sv39_mappings(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_process_memory.rs").read_text()
        start = source.index("#[cfg(target_arch = \"riscv64\")]\nconst RISCV_PAGE_TABLE_COUNT")
        end = source.index("#[cfg(target_arch = \"x86_64\")]\nimpl FallbackAddressSpace", start)
        riscv_source = source[start:end]
        self.assertIn("struct RiscvPageTable", riscv_source)
        self.assertIn("fn riscv_translate_user", riscv_source)
        self.assertIn("fn map_user_page", riscv_source)
        self.assertNotIn("pid.checked_mul(PAGE_SIZE)", riscv_source)

    def test_riscv_user_roots_alias_uart_outside_linux_user_window(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_process_memory.rs").read_text()
        start = source.index("#[cfg(target_arch = \"riscv64\")]\nimpl FallbackAddressSpace")
        end = source.index("#[cfg(target_arch = \"x86_64\")]\nimpl FallbackAddressSpace", start)
        riscv_source = source[start:end]
        self.assertIn("RISCV_USER_UART_ALIAS", riscv_source)
        self.assertRegex(
            riscv_source,
            r"riscv_map_page\([\s\S]*?RISCV_USER_UART_ALIAS",
            "each RISC-V user root must map the UART through a kernel-safe alias",
        )

    def test_riscv_user_roots_map_all_virtio_mmio_regions_for_kernel_access(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_process_memory.rs").read_text()
        start = source.index("#[cfg(target_arch = \"riscv64\")]\nimpl FallbackAddressSpace")
        end = source.index("#[cfg(target_arch = \"x86_64\")]\nimpl FallbackAddressSpace", start)
        riscv_source = source[start:end]
        self.assertIn("virtio_mmio_reg(virtio_mmio_index)", riscv_source)
        self.assertRegex(
            riscv_source,
            r"while let Some\(reg\) = crate::kernel_lowlevel::drivers::virtio_mmio_reg\(virtio_mmio_index\)",
            "each RISC-V process root must discover every virtio-MMIO transport",
        )
        self.assertRegex(riscv_source, r"reg\s*\.size")
        self.assertRegex(
            riscv_source,
            r"riscv_map_page\([\s\S]*?alias_page[\s\S]*?false,\s*\n\s*\)\?;",
            "virtio-MMIO ranges must be supervisor mappings so kernel drivers survive SATP switches",
        )

    def test_riscv_virtio_mmio_uses_a_kernel_alias_outside_linux_user_window(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_process_memory.rs").read_text()
        start = source.index("#[cfg(target_arch = \"riscv64\")]\nimpl FallbackAddressSpace")
        end = source.index("#[cfg(target_arch = \"x86_64\")]\nimpl FallbackAddressSpace", start)
        riscv_source = source[start:end]
        self.assertIn("virtio_mmio_user_alias(page_base)", riscv_source)
        self.assertNotRegex(
            riscv_source,
            r"riscv_map_page\(\s*root_paddr,\s*page,\s*page as u64",
            "virtio-MMIO mappings must not occupy the Linux executable user window",
        )
        block_source = (REPOSITORY_ROOT / "src/user_level/drivers/block.rs").read_text()
        net_source = (REPOSITORY_ROOT / "src/user_level/drivers/net.rs").read_text()
        for driver_source in (block_source, net_source):
            self.assertIn("virtio_mmio_user_alias(base)", driver_source)
            self.assertIn("user_address_space_active()", driver_source)

    def test_riscv_fork_has_a_native_child_start_path(self):
        syscall_source = (REPOSITORY_ROOT / "src/syscall/syscall.rs").read_text()
        fork_start = syscall_source.index("fn sys_fork_with_child_tid(")
        fork_end = syscall_source.index("\n}\n\n/// Linux sys_vfork", fork_start) + 3
        fork_source = syscall_source[fork_start:fork_end]
        self.assertIn('#[cfg(target_arch = "riscv64")]', fork_source)
        self.assertIn('#[cfg(target_arch = "x86_64")]', fork_source)
        self.assertRegex(
            fork_source,
            r"#\[cfg\(target_arch = \"x86_64\"\)\][\s\S]*?Err\(SysError::ENOSYS\)",
            "the synthetic x86_64 fallback remains isolated from RISC-V",
        )
        self.assertIn("current_riscv_syscall_context", syscall_source)
        process_source = (REPOSITORY_ROOT / "src/syscall/linux_process.rs").read_text()
        self.assertIn("struct RiscvProcessStart", process_source)
        self.assertRegex(
            process_source,
            r"#\[cfg\(target_arch = \"riscv64\"\)\]\n"
            r"pub\(crate\) extern \"C\" fn riscv_linux_fork_child_entry",
        )
        thread_source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/thread.rs").read_text()
        self.assertIn("start_linux_process_child", thread_source)

        context_source = (REPOSITORY_ROOT / "src/syscall/linux_riscv_syscall_context.rs").read_text()
        self.assertIn("pub(crate) fn current_riscv_syscall_context", context_source)

    def test_riscv_trap_frame_preserves_return_csrs_across_context_switch(self):
        boot_source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/boot.rs").read_text()
        self.assertIn("RISCV_TRAP_FRAME_SEPC_SLOT", boot_source)
        self.assertIn("RISCV_TRAP_FRAME_SSTATUS_SLOT", boot_source)
        self.assertRegex(
            boot_source,
            r"csrr\s+t0, sepc[\s\S]*?sd\s+t0, 240\(sp\)",
            "trap entry must save sepc in the private frame",
        )
        self.assertRegex(
            boot_source,
            r"ld\s+t0, 240\(t6\)[\s\S]*?csrw\s+sepc, t0",
            "trap restore must restore the frame's sepc",
        )
        self.assertIn("csrr    t0, sstatus", boot_source)
        self.assertIn("csrw    sstatus, t0", boot_source)

    def test_riscv_scheduler_switches_process_address_spaces(self):
        scheduler_source = (REPOSITORY_ROOT / "src/kernel_objects/scheduler.rs").read_text()
        self.assertIn("switch_riscv_process_address_space", scheduler_source)
        cpu_source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/cpu.rs").read_text()
        self.assertIn("RISCV_TRAP_STACKS", cpu_source)
        self.assertIn("set_trap_stack_for_thread", scheduler_source)
        self.assertIn("SSTATUS_SPP", cpu_source)
        self.assertIn("set_return_state", cpu_source)
        context_source = (REPOSITORY_ROOT / "src/syscall/linux_riscv_syscall_context.rs").read_text()
        self.assertIn("set_return_stack_to_trap_top", context_source)

    def test_riscv_syscall_return_delivers_pending_signals(self):
        boot_source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/boot.rs").read_text()
        trap_start = boot_source.index("trap_user_ecall:")
        trap_end = boot_source.index("trap_page_fault:", trap_start)
        trap_source = boot_source[trap_start:trap_end]
        self.assertIn(
            "call    complete_linux_signal_syscall_return",
            trap_source,
            "RISC-V syscall returns must deliver pending signals before sret",
        )
        dispatch_source = (REPOSITORY_ROOT / "src/syscall/syscall_dispatch.rs").read_text()
        self.assertIn(
            "clear_linux_riscv_syscall_context",
            dispatch_source,
            "the RISC-V syscall frame must remain installed until signal return handling",
        )

    def test_riscv_timer_trap_checks_preemption_after_accounting(self):
        boot_source = (REPOSITORY_ROOT / "src/kernel_lowlevel/RISCV64/boot.rs").read_text()
        trap_start = boot_source.index("trap_timer:")
        trap_end = boot_source.index("trap_user_ecall:", trap_start)
        trap_source = boot_source[trap_start:trap_end]
        self.assertRegex(
            trap_source,
            r"call\s+timer_interrupt_handler[\s\S]*call\s+check_preemption",
            "RISC-V timer traps must run scheduler preemption before restoring user mode",
        )

    def test_riscv_mqueue_timer_hook_expires_waiters(self):
        source = (REPOSITORY_ROOT / "src/syscall/linux_mqueue.rs").read_text()
        hook_start = source.index("pub(crate) fn on_timer_tick")
        hook_end = source.index("\n}\n", hook_start) + 2
        hook_source = source[hook_start:hook_end]
        self.assertRegex(
            hook_source,
            r"#\[cfg\(any\(target_arch = \"aarch64\", target_arch = \"riscv64\"\)\)\]",
            "RISC-V mqueue waits must be expired by the timer hook",
        )
        self.assertIn("state.expire_one(now)", hook_source)

    def test_riscv_sleep_does_not_yield_before_wait(self):
        source = (REPOSITORY_ROOT / "scripts/posix/runtime/smros_posix_compat.c").read_text()
        sleep_start = source.index("unsigned int sleep(unsigned int seconds)")
        sleep_end = source.index("\n}\n", sleep_start) + 2
        sleep_source = source[sleep_start:sleep_end]
        self.assertRegex(
            sleep_source,
            r"#if !defined\(__riscv\)\s*\(void\)sched_yield\(\);\s*#endif",
            "RISC-V sleep must enter its wait before yielding to an awakened mqueue peer",
        )


if __name__ == "__main__":
    unittest.main()
