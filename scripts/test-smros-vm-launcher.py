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


if __name__ == "__main__":
    unittest.main()
