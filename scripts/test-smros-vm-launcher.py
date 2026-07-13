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
    def test_launcher_protocol_version_covers_isolated_st(self) -> None:
        self.assertGreaterEqual(LAUNCHER.LAUNCHER_VERSION, 6)

    def test_only_fixed_test_jobs_are_accepted(self) -> None:
        self.assertEqual(LAUNCHER.parse_test_job({"job": "ut"}), ("make", "ut"))
        self.assertEqual(LAUNCHER.parse_test_job({"job": "it"}), ("make", "it"))
        self.assertEqual(LAUNCHER.parse_test_job({"job": "st"}), ("make", "st"))

        for values in (
            {"job": "verify"},
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
    def test_st_uses_an_isolated_disk_and_smoke_log(self, run: mock.Mock) -> None:
        run.return_value = subprocess.CompletedProcess(
            args=["make", "st"], returncode=0, stdout="smoke passed\n", stderr=""
        )

        response = LAUNCHER.run_test_job({"job": "st"})

        args, _ = run.call_args
        self.assertEqual(
            args[0],
            (
                "make",
                "st",
                "FXFS_DISK=target/hermes-tests/st-fxfs.img",
                "SMROS_ST_LOG=target/hermes-tests/st-smoke.log",
            ),
        )
        self.assertIn("OK job=st status=0", response)


if __name__ == "__main__":
    unittest.main()
