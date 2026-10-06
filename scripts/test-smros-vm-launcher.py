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
        self.assertTrue(kwargs.get("start_new_session"))
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




class FakeQemuProc:
    def __init__(self, pid: int, returncode: int | None) -> None:
        self.pid = pid
        self._returncode = returncode

    def poll(self) -> int | None:
        return self._returncode


class LaunchRetryTests(unittest.TestCase):
    def test_launcher_retries_early_qemu_exit(self) -> None:
        self.assertGreaterEqual(LAUNCHER.LAUNCHER_VERSION, 10)
        self.assertIn("launch_retry=1", LAUNCHER.launcher_status())
        self.assertGreaterEqual(LAUNCHER.launch_attempts(), 3)

        src = MODULE_PATH.read_text()
        launch_start = src.index("def launch_qemu")
        launch_end = src.index("\ndef stop_qemu")
        body = src[launch_start:launch_end]
        self.assertIn("retrying_launch(", body)
        self.assertIn("launch_attempts()", body)
        self.assertIn("stdin=subprocess.DEVNULL", src)
        self.assertIn("start_new_session=True", src)

        calls = {"n": 0}

        def operation() -> str:
            calls["n"] += 1
            if calls["n"] < 3:
                raise RuntimeError("qemu exited during startup status=0")
            return "OK pid=9\n"

        sleeps: list[float] = []
        result = LAUNCHER.retrying_launch(
            operation, attempts=3, delay_seconds=0.4, sleep=sleeps.append
        )
        self.assertEqual(result, "OK pid=9\n")
        self.assertEqual(calls["n"], 3)
        self.assertEqual(sleeps, [0.4, 0.4])

    def test_wait_for_stable_launch_adopts_still_running_named_qemu(self) -> None:
        log_path = LAUNCHER.ROOT / "target" / "vm-launcher" / "linux-demo.log"
        proc = FakeQemuProc(pid=1, returncode=0)
        with mock.patch.object(LAUNCHER, "qemu_pids_by_name", return_value=[4242]):
            LAUNCHER.wait_for_stable_launch("linux-demo", proc, log_path)

    def test_wait_for_stable_launch_still_fails_when_named_qemu_is_gone(self) -> None:
        log_path = LAUNCHER.ROOT / "target" / "vm-launcher" / "linux-demo.log"
        proc = FakeQemuProc(pid=1, returncode=0)
        with mock.patch.object(LAUNCHER, "qemu_pids_by_name", return_value=[]):
            with self.assertRaises(RuntimeError) as ctx:
                LAUNCHER.wait_for_stable_launch("linux-demo", proc, log_path)
        self.assertIn("status=0", str(ctx.exception))


    def test_run_test_job_retries_nonzero_status(self) -> None:
        fail = subprocess.CompletedProcess(
            args=["make", "ut"], returncode=101, stdout="FAILED\n", stderr=""
        )
        ok = subprocess.CompletedProcess(
            args=["make", "ut"],
            returncode=0,
            stdout="test result: ok. 344 passed\n",
            stderr="",
        )
        with mock.patch.object(LAUNCHER.subprocess, "run", side_effect=[fail, ok]) as run:
            with mock.patch.object(LAUNCHER.time, "sleep"):
                response = LAUNCHER.run_test_job({"job": "ut"})
        self.assertIn("OK job=ut status=0", response)
        self.assertEqual(run.call_count, 2)
        src = MODULE_PATH.read_text()
        self.assertIn("TEST_JOB_LOCK", src)
        self.assertIn("job=", src)
        body_start = src.index("def run_test_job")
        body_end = src.index("\ndef write_test_log")
        body = src[body_start:body_end]
        self.assertNotIn("with LOCK:", body)
        self.assertIn("test_job_attempts()", body)
        self.assertIn("start_new_session=True", body)
        args, kwargs = run.call_args
        self.assertTrue(kwargs.get("start_new_session"))


if __name__ == "__main__":
    unittest.main()
