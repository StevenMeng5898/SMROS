#!/usr/bin/env python3

import importlib.util
import os
import subprocess
import tempfile
import threading
import time
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
        self.assertGreaterEqual(LAUNCHER.LAUNCHER_VERSION, 11)
        self.assertIn("async_test_jobs=1", LAUNCHER.launcher_status())

    def test_only_fixed_test_jobs_are_accepted(self) -> None:
        self.assertEqual(
            LAUNCHER.parse_test_job({"job": "ut"}),
            tuple(LAUNCHER.host_make_command("ut")),
        )
        self.assertEqual(
            LAUNCHER.parse_test_job({"job": "it"}),
            tuple(LAUNCHER.host_make_command("it")),
        )
        self.assertIn(str(LAUNCHER.ROOT), LAUNCHER.host_make_command("ut"))
        self.assertIn(str(LAUNCHER.ROOT / "Makefile"), LAUNCHER.host_make_command("it"))

        for values in (
            {"job": "verify"},
            {"job": "st"},
            {"job": "skt"},
            {"job": "posixtest"},
            {"job": "posixtest all"},
            {"job": "ut", "command": "make clean"},
            {"command": "make ut"},
            {"job": ""},
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
        self.assertEqual(list(args[0]), LAUNCHER.host_make_command("ut"))
        self.assertEqual(kwargs.get("cwd"), str(LAUNCHER.ROOT))
        self.assertNotIn("shell", kwargs)
        self.assertEqual(kwargs["env"].get("LC_ALL"), "C")
        self.assertNotIn("MAKEFLAGS", kwargs["env"])
        self.assertNotIn("MAKELEVEL", kwargs["env"])
        self.assertTrue(kwargs.get("start_new_session"))
        self.assertIn("OK job=ut status=0", response)

    @mock.patch.object(LAUNCHER.subprocess, "run")
    def test_it_job_does_not_boot_qemu_smoke(self, run: mock.Mock) -> None:
        run.return_value = subprocess.CompletedProcess(
            args=["make", "it"], returncode=0, stdout="199 passed\n", stderr=""
        )

        response = LAUNCHER.run_test_job({"job": "it"})

        args, _ = run.call_args
        self.assertEqual(list(args[0]), LAUNCHER.host_make_command("it"))
        self.assertNotIn("skt", args[0])
        self.assertNotIn("st", args[0])
        self.assertIn("OK job=it status=0", response)

    def test_failed_it_summary_prefers_the_test_panic(self) -> None:
        output = (
            "test process_create_rolls_back_failed_init_slots ... FAILED\n"
            "make[1]: Leaving directory '/home/steven/workspace/SMROS'\n"
            "error: test failed, to rerun pass `--test integration_contracts`\n"
            "thread 'process_create_rolls_back_failed_init_slots' panicked at tests/integration_contracts.rs:8605:5:\n"
        )
        summary = LAUNCHER.bounded_test_summary(output)
        self.assertIn("panicked_at", summary)
        self.assertNotIn("Leaving_directory", summary)

    def test_failed_it_summary_ignores_make_leaving_directory_when_stdout_only(self) -> None:
        stdout = (
            "test process_create_rolls_back_failed_init_slots ... FAILED\n"
            "make[1]: Leaving directory '/home/steven/workspace/SMROS'\n"
        )
        stderr = (
            "thread 'process_create_rolls_back_failed_init_slots' panicked at tests/integration_contracts.rs:8605:5:\n"
        )
        summary = LAUNCHER.bounded_test_summary(stdout)
        self.assertNotIn("Leaving_directory", summary)
        self.assertIn("FAILED", summary)
        only_make = LAUNCHER.bounded_test_summary(
            "make[1]: Leaving directory '/home/steven/workspace/SMROS'\n"
        )
        self.assertNotIn("Leaving_directory", only_make)
        combined = LAUNCHER.bounded_test_summary(stdout + stderr)
        self.assertIn("panicked_at", combined)
        last_line = LAUNCHER.bounded_test_summary(
            "thread 'cli_contract' panicked at tests/integration_contracts.rs:1:1:\n"
            "assertion failed: host job\n"
            "make[1]: Leaving directory '/repo'\n"
        )
        self.assertIn("assertion", last_line)
        self.assertNotIn("Leaving", last_line)
        only_footer = LAUNCHER.bounded_test_summary(
            "ordinary status line\nmake: Leaving directory '/repo'\n"
        )
        self.assertEqual(only_footer, "ordinary_status_line")
        self.assertNotIn("Leaving", only_footer)
        english = LAUNCHER.bounded_test_summary(
            "make[1]: *** No rule to make target 'ut'. Stop.\n"
        )
        self.assertIn("make[1]", english)
        self.assertIn("No_rule_to_make_target", english)
        self.assertIn("'ut'", english)
        self.assertNotIn(" ", english)
        src = MODULE_PATH.read_text()
        run_start = src.index("def run_test_job")
        run_end = src.index("def write_test_log")
        self.assertIn("bounded_test_summary(output)", src[run_start:run_end])
        self.assertNotIn("bounded_test_summary(result.stdout or output)", src[run_start:run_end])

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

    def test_handler_assembles_split_tcp_requests(self) -> None:
        src = MODULE_PATH.read_text()
        handle_start = src.index("class Handler")
        handle_end = src.index("class LauncherServer")
        body = src[handle_start:handle_end]
        self.assertIn('while b"\\nend\\n" not in data', body)
        self.assertIn("self.request.recv(MAX_REQUEST - len(data))", body)



    def test_host_job_targets_repo_makefile_from_a_hostile_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            dummy = Path(tmp)
            (dummy / "Makefile").write_text("all:\n\t@echo dummy\n", encoding="utf-8")
            hostile = os.environ.copy()
            hostile["MAKEFLAGS"] = f"-C {dummy}"
            hostile["GNUMAKEFLAGS"] = f"--directory={dummy}"
            hostile["MAKELEVEL"] = "1"
            hostile["CURDIR"] = str(dummy)
            hostile["LANG"] = "zh_CN.UTF-8"
            hostile["LC_ALL"] = "zh_CN.UTF-8"
            with mock.patch.dict(os.environ, hostile, clear=True):
                env = LAUNCHER.test_job_environment()
                command = list(LAUNCHER.parse_test_job({"job": "ut"}))
            self.assertEqual(env.get("LC_ALL"), "C")
            self.assertEqual(env.get("LANG"), "C")
            for leaked in ("MAKEFLAGS", "GNUMAKEFLAGS", "MAKELEVEL", "CURDIR", "MFLAGS"):
                self.assertNotIn(leaked, env)
            self.assertEqual(command, LAUNCHER.host_make_command("ut"))
            self.assertIn(str(LAUNCHER.ROOT / "Makefile"), command)
            dry = subprocess.run(
                [*command, "-n"],
                cwd=dummy,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(dry.returncode, 0, dry.stderr + dry.stdout)
            self.assertNotIn("No rule to make target", dry.stderr)
            bare = subprocess.run(
                ["make", "ut"],
                cwd=dummy,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(bare.returncode, 0)
            self.assertIn("No rule to make target", bare.stderr)

    def test_version_11_and_fingerprint_mismatch_do_not_count_as_current(self) -> None:
        old = (
            "OK version=11 monitor_none=1 stale_qemu_cleanup=1 "
            "trace_sync=1 stable_launch=1 vm_log=1 hermes_test_jobs=1 linux_boot=1 "
            "launch_retry=1 async_test_jobs=1\n"
        )
        self.assertFalse(LAUNCHER.accept_running_launcher(old))
        self.assertEqual(LAUNCHER.classify_launcher(old), "replace")
        current = LAUNCHER.launcher_status()
        self.assertTrue(LAUNCHER.accept_running_launcher(current))
        self.assertEqual(LAUNCHER.classify_launcher(current), "keep")
        self.assertIn(LAUNCHER.FINGERPRINT, current)
        self.assertIn(LAUNCHER.script_sha256(), current)
        stale = current.replace(LAUNCHER.FINGERPRINT, "stale-fingerprint")
        self.assertFalse(LAUNCHER.accept_running_launcher(stale))
        busy = stale.replace("host_jobs_inflight=0", "host_jobs_inflight=1")
        self.assertEqual(LAUNCHER.classify_launcher(busy), "drain")
        self.assertNotEqual(LAUNCHER.classify_launcher(busy), "replace")

    def test_chinese_make_error_is_ascii_and_names_the_launcher(self) -> None:
        chinese = "make[1]: *** 没有规则可制作目标 'ut'。 停止。\n"
        summary = LAUNCHER.bounded_test_summary(chinese)
        self.assertIn("make[1]", summary)
        self.assertNotIn("没有", summary)
        self.assertNotIn("make_1_", summary)
        failed = subprocess.CompletedProcess(
            args=["make", "ut"], returncode=2, stdout="", stderr=chinese
        )
        with mock.patch.object(LAUNCHER.subprocess, "run", return_value=failed):
            with mock.patch.object(LAUNCHER, "test_job_attempts", return_value=1):
                response = LAUNCHER.run_test_job({"job": "ut"})
        self.assertIn("status=2", response)
        self.assertIn(f"@fingerprint={LAUNCHER.FINGERPRINT}", response)
        self.assertIn("@argv=", response)
        self.assertIn(",ut", response)
        self.assertNotIn("没有", response)

    def test_resume_after_reboot_reuses_an_unacked_host_job(self) -> None:
        LAUNCHER.reset_jobs_for_tests()
        started = threading.Event()
        release = threading.Event()

        def blocking(*_args, **_kwargs):
            started.set()
            self.assertTrue(release.wait(timeout=2))
            return subprocess.CompletedProcess(
                args=["make", "ut"], returncode=0, stdout="ut ok\n", stderr=""
            )

        with mock.patch.object(LAUNCHER.subprocess, "run", side_effect=blocking) as run:
            first = LAUNCHER.start_test_job({"job": "ut"})
            self.assertTrue(started.wait(timeout=1))
            self.assertIn("host_jobs_inflight=1", LAUNCHER.launcher_status())
            self.assertEqual(LAUNCHER.classify_launcher(LAUNCHER.launcher_status()), "keep")
            resumed = LAUNCHER.start_test_job({"job": "ut"})
            self.assertEqual(first, resumed)
            self.assertEqual(run.call_count, 1)
            release.set()
            token = [
                field.split("=", 1)[1]
                for field in first.split()
                if field.startswith("token=")
            ][0]
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                with LAUNCHER.JOBS_LOCK:
                    state = LAUNCHER.JOBS[token].get("state")
                if state == "done":
                    break
                time.sleep(0.01)
            else:
                self.fail("host job did not finish")
            replay = LAUNCHER.start_test_job({"job": "ut"})
            self.assertEqual(replay, first)
            self.assertEqual(run.call_count, 1)
            status = LAUNCHER.test_job_status({"job": "ut", "token": token})
            self.assertIn("status=0", status)
            self.assertIn("ut_ok", status)
            LAUNCHER.JOBS.clear()
            LAUNCHER.load_jobs()
            reloaded = LAUNCHER.test_job_status({"job": "ut", "token": token})
            self.assertEqual(reloaded, status)
            again = LAUNCHER.start_test_job({"job": "ut"})
            self.assertNotEqual(again, first)
            next_token = [
                field.split("=", 1)[1]
                for field in again.split()
                if field.startswith("token=")
            ][0]
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                with LAUNCHER.JOBS_LOCK:
                    state = LAUNCHER.JOBS[next_token].get("state")
                if state == "done":
                    break
                time.sleep(0.01)
            else:
                self.fail("follow-up host job did not finish")
        LAUNCHER.reset_jobs_for_tests()

    def test_start_script_scrubs_make_env_and_restarts_only_from_the_host(self) -> None:
        starter = Path(__file__).with_name("start-smros-vm-launcher.sh").read_text()
        makefile = Path(__file__).resolve().parents[1].joinpath("Makefile").read_text()
        self.assertIn("REQUIRED_VERSION=12", starter)
        self.assertIn("-u MAKEFLAGS", starter)
        self.assertIn("-u MAKELEVEL", starter)
        self.assertIn("-u GNUMAKEFLAGS", starter)
        self.assertIn("LC_ALL=C", starter)
        self.assertIn("drain_host_jobs", starter)
        self.assertIn('launcher_action', starter)
        self.assertIn("vm-launcher-restart:", makefile)
        self.assertIn("SMROS_VM_LAUNCHER_RESTART=1", makefile)


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


    def test_start_test_job_accepts_immediately_and_status_polls(self) -> None:
        started = threading.Event()
        finished = threading.Event()

        def blocking(*args, **kwargs):
            started.set()
            finished.wait(timeout=2)
            return subprocess.CompletedProcess(
                args=["make", "ut"],
                returncode=0,
                stdout="test result: ok. 1 passed\n",
                stderr="",
            )

        with mock.patch.object(LAUNCHER.subprocess, "run", side_effect=blocking):
            response = LAUNCHER.start_test_job({"job": "ut"})
            self.assertIn("accepted=1", response)
            token = [
                field.split("=", 1)[1]
                for field in response.split()
                if field.startswith("token=")
            ][0]
            status = LAUNCHER.test_job_status({"job": "ut", "token": token})
            self.assertIn("running=1", status)
            self.assertTrue(started.wait(timeout=1))
            finished.set()
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                status = LAUNCHER.test_job_status({"job": "ut", "token": token})
                if "status=0" in status:
                    break
                time.sleep(0.01)
            else:
                self.fail(status)
            self.assertIn("OK job=ut status=0", status)

        with self.assertRaises(ValueError):
            LAUNCHER.test_job_status({"job": "it", "token": token})
        with self.assertRaises(ValueError):
            LAUNCHER.test_job_status({"job": "ut", "token": "999999"})
        with self.assertRaises(ValueError):
            LAUNCHER.test_job_status({"token": "not-a-token"})

        src = MODULE_PATH.read_text()
        self.assertIn("SMROS_TEST_STATUS 1", src)
        self.assertIn("response = start_test_job(values)", src)
        self.assertIn("response = test_job_status(values)", src)
        self.assertIn("terminate_process_group(exc.pid)", src)
        self.assertIn("def terminate_process_group(", src)

    def test_run_test_job_reports_timeout_without_passing(self) -> None:
        def expire(*_args, **kwargs):
            expired = subprocess.TimeoutExpired(
                cmd=["make", "it"], timeout=kwargs.get("timeout", 1)
            )
            expired.pid = 4321
            raise expired

        with mock.patch.object(LAUNCHER.subprocess, "run", side_effect=expire):
            with mock.patch.object(LAUNCHER, "terminate_process_group") as kill:
                with mock.patch.object(LAUNCHER.time, "sleep"):
                    with mock.patch.object(LAUNCHER, "test_job_attempts", return_value=1):
                        response = LAUNCHER.run_test_job({"job": "it"})
        self.assertIn("ERR job=it status=timeout", response)
        self.assertNotIn("status=0", response)
        kill.assert_called_once_with(4321)


if __name__ == "__main__":
    unittest.main()
