#!/usr/bin/env python3

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("smros-vm-launcher.py")
SPEC = importlib.util.spec_from_file_location("smros_vm_launcher", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
LAUNCHER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LAUNCHER)


class HermesHostTestProtocolTests(unittest.TestCase):
    def test_launcher_protocol_version_covers_host_jobs(self) -> None:
        self.assertGreaterEqual(LAUNCHER.LAUNCHER_VERSION, 8)

    def test_only_fixed_test_jobs_are_accepted(self) -> None:
        self.assertEqual(LAUNCHER.parse_test_job({"job": "ut"}), ("make", "ut"))
        self.assertEqual(LAUNCHER.parse_test_job({"job": "it"}), ("make", "it"))

        for values in (
            {"job": "verify"},
            {"job": "st"},
            {"job": "skt"},
            {"job": "ut", "command": "make clean"},
            {"command": "make ut"},
            {},
        ):
            with self.assertRaises(ValueError):
                LAUNCHER.parse_test_job(values)

    @mock.patch.object(LAUNCHER.subprocess, "run")
    def test_runner_uses_argv_without_a_shell(self, run: mock.Mock) -> None:
        run.return_value = subprocess.CompletedProcess(
            args=["make", "ut"], returncode=0, stdout="41 passed\n", stderr=""
        )

        response = LAUNCHER.run_test_job({"job": "ut"})

        run.assert_called_once()
        args, kwargs = run.call_args
        self.assertEqual(args[0], ("make", "ut"))
        self.assertNotIn("shell", kwargs)
        self.assertIn("OK job=ut status=0", response)

    @mock.patch.object(LAUNCHER.subprocess, "run")
    def test_it_job_does_not_boot_qemu_smoke(self, run: mock.Mock) -> None:
        run.return_value = subprocess.CompletedProcess(
            args=["make", "it"], returncode=0, stdout="199 passed\n", stderr=""
        )

        response = LAUNCHER.run_test_job({"job": "it"})

        args, _ = run.call_args
        self.assertEqual(args[0], ("make", "it"))
        self.assertNotIn("skt", args[0])
        self.assertNotIn("st", args[0])
        self.assertIn("OK job=it status=0", response)

    def test_linux_boot_marker_matches_initramfs_banner(self) -> None:
        self.assertGreaterEqual(LAUNCHER.LAUNCHER_VERSION, 9)
        self.assertIn("linux_boot=1", LAUNCHER.launcher_status())
        self.assertTrue(
            LAUNCHER.linux_boot_seen_text("\nSMROS Linux VM initramfs\nKernel: Linux")
        )
        self.assertFalse(LAUNCHER.linux_boot_seen_text("qemu started"))

    def test_vc_serial_tees_console_to_logfile(self) -> None:
        args = LAUNCHER.qemu_serial_args("linux-demo", "vc")
        joined = " ".join(args)
        self.assertIn("logfile=", joined)
        self.assertIn("chardev:smros-serial", joined)
        self.assertNotEqual(args, ["-serial", "vc"])

    def test_wait_boot_request_is_accepted(self) -> None:
        header, values = LAUNCHER.parse_request(
            b"SMROS_VM_WAIT_BOOT 1\nname=linux-demo\nend\n"
        )
        self.assertEqual(header, "SMROS_VM_WAIT_BOOT 1")
        self.assertEqual(values["name"], "linux-demo")


if __name__ == "__main__":
    unittest.main()
