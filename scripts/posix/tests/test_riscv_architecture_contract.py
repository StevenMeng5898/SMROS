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


if __name__ == "__main__":
    unittest.main()
