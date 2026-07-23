from __future__ import annotations

from contextlib import redirect_stderr
from dataclasses import asdict, replace
import errno
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest
from unittest import mock

from scripts.posix import baseline as baseline_module
from scripts.posix import cli
from scripts.posix.baseline import (
    MAX_CAPTURE_BYTES,
    PLATFORM,
    BaselineResult,
    classify_status,
    filter_runnable_tests,
    run_baseline,
    run_runtime_attempt,
)
from scripts.posix.build import (
    CHECKSUM_DEFINITION,
    ManifestMetadata,
    render_manifest,
    sha256_file,
)
from scripts.posix.model import SuiteTest


def _metadata() -> ManifestMetadata:
    return ManifestMetadata(
        source="https://example.invalid/posixtest.git",
        revision="1" * 40,
        architecture="aarch64",
        compiler="aarch64-linux-gnu-gcc test",
        libc="libc.so.6:" + "2" * 64,
        patch_sha256="3" * 64,
        smros_commit="4" * 40,
        build_results_sha256="5" * 64,
    )


def _write_executable(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    path.chmod(0o755)


def _fake_qemu(path: Path, observation: Path, child_pid: Path) -> None:
    script = f"""#!/usr/bin/python3
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

target = Path(sys.argv[3]).name
Path({str(observation)!r}).write_text(json.dumps({{
    'argv': sys.argv,
    'cwd': os.getcwd(),
    'env': dict(os.environ),
}}), encoding='utf-8')
if target.startswith('pass'):
    print('pass-out')
    print('pass-err', file=sys.stderr)
    raise SystemExit(0)
if target.startswith('fail'):
    raise SystemExit(1)
if target.startswith('unresolved'):
    raise SystemExit(2)
if target.startswith('unsupported'):
    raise SystemExit(4)
if target.startswith('untested'):
    raise SystemExit(5)
if target.startswith('large'):
    os.write(1, b'O' * (MAX_CAPTURE_BYTES + 257))
    os.write(2, b'E' * (MAX_CAPTURE_BYTES + 513))
    raise SystemExit(0)
if target.startswith('signal'):
    os.kill(os.getpid(), signal.SIGUSR1)
if target.startswith('timeout'):
    child = subprocess.Popen([
        '/usr/bin/python3', '-c',
        'import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)'
    ])
    Path({str(child_pid)!r}).write_text(str(child.pid), encoding='ascii')
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    time.sleep(60)
raise SystemExit(9)
"""
    script = script.replace("MAX_CAPTURE_BYTES", str(MAX_CAPTURE_BYTES))
    _write_executable(path, script)


class BaselineFixture:
    def setUp(self) -> None:
        super().setUp()
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.stage = self.root / "stage"
        self.sysroot = self.root / "sysroot"
        self.results = self.root / "results" / "results.ndjson"
        self.observation = self.root / "observation.json"
        self.child_pid = self.root / "child.pid"
        self.qemu = self.root / "bin" / "qemu-aarch64"
        _fake_qemu(self.qemu, self.observation, self.child_pid)
        _write_executable(
            self.stage / "lib" / "ld-linux-aarch64.so.1", "staged loader"
        )
        libc = Path("/lib/x86_64-linux-gnu/libc.so.6")
        if not libc.is_file():
            libc = Path("/usr/lib/x86_64-linux-gnu/libc.so.6")
        (self.stage / "lib").mkdir(parents=True, exist_ok=True)
        shutil.copy2(libc, self.stage / "lib" / "libc.so.6")
        (self.stage / "lib" / "libc.so.6").chmod(0o755)
        (self.sysroot / "lib").mkdir(parents=True, exist_ok=True)
        shutil.copy2(
            self.stage / "lib" / "ld-linux-aarch64.so.1",
            self.sysroot / "lib" / "ld-linux-aarch64.so.1",
        )
        shutil.copy2(
            self.stage / "lib" / "libc.so.6",
            self.sysroot / "lib" / "libc.so.6",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()
        super().tearDown()

    def make_test(
        self,
        name: str,
        *,
        api: str = "getpid",
        group: str = "unistd",
        timeout_ms: int = 2_000,
        disposition: str = "complete",
        kind: str = "runnable",
    ) -> SuiteTest:
        relative = f"bin/{name}.test"
        binary = self.stage / relative
        if disposition == "complete":
            _write_executable(binary, "synthetic target")
            digest = sha256_file(binary)
            staged_path = relative
        else:
            digest = "0" * 64
            staged_path = "-"
        return SuiteTest(
            test_id=f"conformance/interfaces/{api}/{name}.c",
            group=group,
            api=api,
            kind=kind,
            disposition=disposition,
            source=f"conformance/interfaces/{api}/{name}.c",
            binary=staged_path,
            sha256=digest,
            timeout_ms=timeout_ms,
        )

    def write_manifest(self, tests: tuple[SuiteTest, ...]) -> ManifestMetadata:
        manifest, _ = render_manifest(_metadata(), tests)
        self.stage.mkdir(parents=True, exist_ok=True)
        (self.stage / "manifest.tsv").write_text(manifest, encoding="utf-8")
        metadata = _metadata()
        _, manifest_digest = render_manifest(metadata, tests)
        metadata = ManifestMetadata(
            **{
                **asdict(metadata),
                "manifest_sha256": manifest_digest,
            }
        )
        runtime = []
        for name in ("ld-linux-aarch64.so.1", "libc.so.6"):
            runtime.append(
                {
                    "path": f"lib/{name}",
                    "sha256": sha256_file(self.stage / "lib" / name),
                }
            )
        host = {
            "schema": 1,
            "checksum_definition": CHECKSUM_DEFINITION,
            "metadata": asdict(metadata),
            "runtime": runtime,
            "tests": [asdict(test) for test in sorted(tests, key=lambda item: item.test_id)],
        }
        (self.stage / "manifest.json").write_text(
            json.dumps(host, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        return metadata


class StatusTests(unittest.TestCase):
    def test_exit_codes_have_exact_open_posix_classification(self) -> None:
        expected = {
            0: "pass",
            1: "fail",
            2: "unresolved",
            4: "unsupported",
            5: "untested",
            6: "crash",
            127: "crash",
        }
        self.assertEqual(
            {code: classify_status(code) for code in expected}, expected
        )

    def test_signal_timeout_and_launch_error_take_precedence(self) -> None:
        self.assertEqual(classify_status(-signal.SIGSEGV), "crash")
        self.assertEqual(classify_status(-signal.SIGKILL, timed_out=True), "timeout")
        self.assertEqual(
            classify_status(None, launch_error="not found"), "launch-error"
        )


class FilterTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tests = (
            SuiteTest(
                "a/one.c", "g1", "api1", "runnable", "complete",
                "a/one.c", "bin/a", "1" * 64, 1,
            ),
            SuiteTest(
                "a/two.c", "g1", "api2", "definition", "definition-only",
                "a/two.c", "-", "0" * 64, 1,
            ),
            SuiteTest(
                "b/three.c", "g2", "api1", "runnable", "link-failed",
                "b/three.c", "-", "0" * 64, 1,
            ),
            SuiteTest(
                "b/four.c", "g2", "api2", "runnable", "complete",
                "b/four.c", "bin/b", "2" * 64, 1,
            ),
        )

    def test_filters_are_exact_and_only_return_complete_runnable_tests(self) -> None:
        self.assertEqual(
            [test.test_id for test in filter_runnable_tests(self.tests, api="api1")],
            ["a/one.c"],
        )
        self.assertEqual(
            [test.test_id for test in filter_runnable_tests(self.tests, group="g2")],
            ["b/four.c"],
        )
        self.assertEqual(
            [test.test_id for test in filter_runnable_tests(self.tests, test_id="b/four.c")],
            ["b/four.c"],
        )
        self.assertEqual(len(filter_runnable_tests(self.tests)), 2)

    def test_zero_runnable_selection_is_an_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "no complete runnable tests"):
            filter_runnable_tests(self.tests, api="missing")
        with self.assertRaisesRegex(ValueError, "no complete runnable tests"):
            filter_runnable_tests(self.tests, test_id="a/two.c")


class RuntimeAttemptTests(BaselineFixture, unittest.TestCase):
    def attempt(self, test: SuiteTest):
        metadata = self.write_manifest((test,))
        return run_runtime_attempt(
            test,
            stage=self.stage,
            sysroot=self.sysroot,
            qemu=self.qemu,
            metadata=metadata,
            build_id="build-test",
        )

    def test_qemu_uses_exact_argv_private_cwd_and_minimal_environment(self) -> None:
        test = self.make_test("pass-case")
        attempt = self.attempt(test)

        observation = json.loads(self.observation.read_text(encoding="utf-8"))
        self.assertEqual(
            observation["argv"],
            [
                str(self.qemu),
                "-L",
                str(self.sysroot.resolve()),
                str(Path(observation["cwd"]) / Path(test.binary).name),
            ],
        )
        self.assertEqual(
            set(observation["env"]),
            {"PATH", "LANG", "LC_ALL", "TMPDIR", "LD_LIBRARY_PATH"},
        )
        self.assertEqual(observation["env"]["LANG"], "C")
        self.assertEqual(observation["env"]["LC_ALL"], "C")
        self.assertEqual(
            observation["env"]["LD_LIBRARY_PATH"], "/lib:/usr/lib"
        )
        self.assertEqual(observation["cwd"], observation["env"]["TMPDIR"])
        self.assertFalse(Path(observation["cwd"]).exists())
        self.assertEqual(attempt.status, "pass")
        self.assertEqual((attempt.stdout, attempt.stderr), ("pass-out\n", "pass-err\n"))

    def test_signal_and_launch_error_are_recorded(self) -> None:
        signaled = self.attempt(self.make_test("signal-case"))
        self.assertEqual(signaled.status, "crash")
        self.assertEqual(signaled.signal, signal.SIGUSR1)
        self.assertIsNone(signaled.exit_code)

        missing = self.root / "missing-qemu"
        launch = run_runtime_attempt(
            self.make_test("pass-launch"),
            stage=self.stage,
            sysroot=self.sysroot,
            qemu=missing,
            metadata=_metadata(),
            build_id="build-test",
        )
        self.assertEqual(launch.status, "launch-error")
        self.assertIsNone(launch.exit_code)
        self.assertIsNone(launch.signal)
        self.assertIn("missing-qemu", launch.launch_error or "")

    def test_output_is_bounded_during_capture_with_original_byte_counts(self) -> None:
        attempt = self.attempt(self.make_test("large-case"))
        self.assertEqual(attempt.status, "pass")
        self.assertEqual(attempt.stdout_bytes, MAX_CAPTURE_BYTES + 257)
        self.assertEqual(attempt.stderr_bytes, MAX_CAPTURE_BYTES + 513)
        self.assertTrue(attempt.stdout_truncated)
        self.assertTrue(attempt.stderr_truncated)
        self.assertLessEqual(len(attempt.stdout.encode("utf-8")), MAX_CAPTURE_BYTES)
        self.assertLessEqual(len(attempt.stderr.encode("utf-8")), MAX_CAPTURE_BYTES)
        self.assertNotIn("E", attempt.stdout)
        self.assertNotIn("O", attempt.stderr)

    def test_timeout_terminates_the_whole_process_group(self) -> None:
        attempt = self.attempt(self.make_test("timeout-case", timeout_ms=120))
        self.assertEqual(attempt.status, "timeout")
        self.assertTrue(attempt.timed_out)
        pid = int(self.child_pid.read_text(encoding="ascii"))
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            stat_path = Path(f"/proc/{pid}/stat")
            if stat_path.exists() and stat_path.read_text().split()[2] == "Z":
                break
            time.sleep(0.02)
        else:
            self.fail("timeout descendant survived process-group cleanup")

    def test_capture_io_error_is_infrastructure_failure_not_launch_error(self) -> None:
        original = OSError(errno.EIO, "capture failed")
        with mock.patch(
            "scripts.posix.baseline.selectors.DefaultSelector.select",
            side_effect=original,
        ):
            with self.assertRaises(OSError) as raised:
                self.attempt(self.make_test("timeout-capture", timeout_ms=2_000))
        self.assertIs(raised.exception, original)

    def test_interruption_reaps_process_and_preserves_original_exception(self) -> None:
        original = MemoryError("selector interrupted")
        real_popen = subprocess.Popen
        processes: list[subprocess.Popen[bytes]] = []

        def tracked_popen(*args: object, **kwargs: object) -> subprocess.Popen[bytes]:
            process = real_popen(*args, **kwargs)
            processes.append(process)
            return process

        with (
            mock.patch("scripts.posix.baseline.subprocess.Popen", tracked_popen),
            mock.patch(
                "scripts.posix.baseline.selectors.DefaultSelector.select",
                side_effect=original,
            ),
        ):
            with self.assertRaises(MemoryError) as raised:
                self.attempt(self.make_test("timeout-interrupt", timeout_ms=2_000))
        self.assertIs(raised.exception, original)
        self.assertEqual(len(processes), 1)
        self.assertIsNotNone(processes[0].poll())

    def test_post_sigkill_cleanup_has_a_wall_clock_bound(self) -> None:
        read_stdout, write_stdout = os.pipe()
        read_stderr, write_stderr = os.pipe()

        class NeverReaped:
            pid = 999_999
            returncode = None
            stdout = os.fdopen(read_stdout, "rb", buffering=0)
            stderr = os.fdopen(read_stderr, "rb", buffering=0)

            def poll(self) -> None:
                return None

            def wait(self, timeout: float) -> None:
                raise subprocess.TimeoutExpired("fake-qemu", timeout)

        process = NeverReaped()
        clock = 0.0

        def monotonic() -> float:
            nonlocal clock
            clock += 0.1
            return clock

        select_calls = 0
        selector_waits: list[float] = []

        def bounded_select(wait: float) -> list[object]:
            nonlocal select_calls
            select_calls += 1
            selector_waits.append(wait)
            if select_calls > 20:
                raise AssertionError("selector loop exceeded its cleanup bound")
            return []

        try:
            with (
                mock.patch("scripts.posix.baseline.subprocess.Popen", return_value=process),
                mock.patch("scripts.posix.baseline.time.monotonic", side_effect=monotonic),
                mock.patch(
                    "scripts.posix.baseline.selectors.DefaultSelector.select",
                    side_effect=bounded_select,
                ),
                mock.patch("scripts.posix.baseline._kill_group"),
            ):
                with self.assertRaisesRegex(ValueError, "could not be reaped"):
                    baseline_module._run_captured(
                        ["fake-qemu"],
                        cwd=self.root,
                        env={},
                        timeout_seconds=0.01,
                    )
        finally:
            os.close(write_stdout)
            os.close(write_stderr)
        self.assertTrue(selector_waits)
        self.assertTrue(all(wait > 0 for wait in selector_waits))


class CampaignTests(BaselineFixture, unittest.TestCase):
    def test_campaign_writes_attempts_and_terminal_record_atomically(self) -> None:
        tests = (
            self.make_test("pass-one", api="getpid"),
            self.make_test("fail-two", api="getpid"),
            self.make_test("unsupported-three", api="getpid"),
        )
        self.write_manifest(tests)
        self.results.parent.mkdir(parents=True)
        self.results.write_text("old-report\n", encoding="utf-8")
        verified: list[Path] = []

        result = run_baseline(
            self.stage,
            self.sysroot,
            self.results,
            api="getpid",
            qemu=self.qemu,
            verifier=lambda stage: verified.append(stage),
        )

        self.assertIsInstance(result, BaselineResult)
        self.assertFalse(result.all_passed)
        self.assertEqual(
            [attempt.status for attempt in result.attempts],
            ["fail", "pass", "unsupported"],
        )
        self.assertEqual(verified, [self.stage.resolve()])
        rows = [json.loads(line) for line in self.results.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(
            [row["record_type"] for row in rows],
            ["attempt", "attempt", "attempt", "run"],
        )
        self.assertTrue(rows[-1]["complete"])
        self.assertEqual(rows[-1]["selected_count"], 3)
        self.assertEqual(rows[-1]["status_counts"], {"fail": 1, "pass": 1, "unsupported": 1})
        self.assertEqual(rows[0]["platform"], PLATFORM)
        self.assertEqual(rows[0]["source"], "qemu-user")
        self.assertEqual(rows[0]["manifest_sha256"], rows[-1]["manifest_sha256"])
        self.assertFalse(
            any(
                path.name.startswith(".results.ndjson.")
                for path in self.results.parent.iterdir()
            )
        )
        for line in self.results.read_text(encoding="utf-8").splitlines(keepends=True):
            value = json.loads(line)
            self.assertEqual(line, json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")

    def test_failed_verification_runs_nothing_and_preserves_existing_report(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        self.results.parent.mkdir(parents=True)
        self.results.write_text("known-good\n", encoding="utf-8")

        with self.assertRaisesRegex(ValueError, "stale stage"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: (_ for _ in ()).throw(ValueError("stale stage")),
            )

        self.assertEqual(self.results.read_text(encoding="utf-8"), "known-good\n")
        self.assertFalse(self.observation.exists())

    def test_execution_infrastructure_failure_publishes_incomplete_terminal_record(self) -> None:
        tests = (
            self.make_test("pass-one"),
            self.make_test("pass-two"),
        )
        self.write_manifest(tests)
        original = OSError(errno.EIO, "runtime capture failed")
        real_run_attempt = baseline_module.run_runtime_attempt
        calls = 0

        def fail_second(*args: object, **kwargs: object):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise original
            return real_run_attempt(*args, **kwargs)

        with mock.patch(
            "scripts.posix.baseline.run_runtime_attempt", side_effect=fail_second
        ):
            with self.assertRaises(OSError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )

        self.assertIs(raised.exception, original)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual([row["record_type"] for row in rows], ["attempt", "run"])
        self.assertFalse(rows[-1]["complete"])
        self.assertEqual(rows[-1]["selected_count"], 2)
        self.assertEqual(rows[-1]["completed_count"], 1)

    def test_missing_runtime_prerequisite_fails_before_verification_or_output(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        (self.stage / "lib" / "libc.so.6").unlink()
        verified = False

        def verifier(_stage: Path) -> None:
            nonlocal verified
            verified = True

        with self.assertRaisesRegex(ValueError, "runtime file"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=verifier,
            )
        self.assertFalse(verified)
        self.assertFalse(self.results.exists())

    def test_sysroot_interpreter_must_match_the_staged_interpreter(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        _write_executable(
            self.sysroot / "lib" / "ld-linux-aarch64.so.1",
            "different loader",
        )

        with self.assertRaisesRegex(ValueError, "does not match staged interpreter"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )
        self.assertFalse(self.observation.exists())
        self.assertFalse(self.results.exists())

    def test_sysroot_shared_library_accepts_debian_0644_mode(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        (self.sysroot / "lib" / "libc.so.6").chmod(0o644)

        result = run_baseline(
            self.stage,
            self.sysroot,
            self.results,
            qemu=self.qemu,
            verifier=lambda _stage: None,
        )

        self.assertTrue(result.all_passed)
        self.assertTrue(self.observation.exists())

    def test_runtime_changed_by_verifier_is_rejected_before_qemu_launch(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))

        def mutating_verifier(_stage: Path) -> None:
            _write_executable(self.stage / "lib" / "libc.so.6", "replacement")

        with self.assertRaisesRegex(ValueError, "runtime file checksum mismatch"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=mutating_verifier,
            )
        self.assertFalse(self.observation.exists())
        self.assertFalse(self.results.exists())

    def test_post_check_staged_runtime_swap_cannot_affect_execution(self) -> None:
        tests = (
            self.make_test("pass-one"),
            self.make_test("pass-two"),
        )
        self.write_manifest(tests)
        real_run_attempt = baseline_module.run_runtime_attempt
        swapped = False

        def swap_before_launch(*args: object, **kwargs: object):
            nonlocal swapped
            if not swapped:
                swapped = True
                _write_executable(
                    self.stage / "lib" / "libc.so.6", "untrusted replacement"
                )
            return real_run_attempt(*args, **kwargs)

        with mock.patch(
            "scripts.posix.baseline.run_runtime_attempt",
            side_effect=swap_before_launch,
        ):
            result = run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        self.assertTrue(result.all_passed)
        observation = json.loads(self.observation.read_text(encoding="utf-8"))
        self.assertEqual(observation["env"]["LD_LIBRARY_PATH"], "/lib:/usr/lib")
        self.assertNotIn(str(self.stage), observation["argv"][3])
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertTrue(rows[-1]["complete"])

    def test_repeated_campaigns_have_unique_run_ids_bound_to_attempts(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))

        run_baseline(
            self.stage,
            self.sysroot,
            self.results,
            qemu=self.qemu,
            verifier=lambda _stage: None,
        )
        first = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        run_baseline(
            self.stage,
            self.sysroot,
            self.results,
            qemu=self.qemu,
            verifier=lambda _stage: None,
        )
        second = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]

        self.assertNotEqual(first[-1]["run_id"], second[-1]["run_id"])
        self.assertEqual(first[0]["run_id"], first[-1]["run_id"])
        self.assertEqual(second[0]["run_id"], second[-1]["run_id"])


class RuntimeAttemptModelTests(BaselineFixture, unittest.TestCase):
    def test_extended_metadata_participates_in_equality_hash_and_serialization(self) -> None:
        arguments = {
            "test_id": "a/test.c",
            "platform": PLATFORM,
            "status": "launch-error",
            "exit_code": None,
            "signal": None,
            "timed_out": False,
            "duration_ms": 1,
            "stdout": "",
            "stderr": "",
            "source": "qemu-user",
        }
        first = baseline_module.RuntimeAttempt(
            **arguments,
            launch_error="first",
            build_id="build-a",
            stdout_bytes=4,
        )
        second = baseline_module.RuntimeAttempt(
            **arguments,
            launch_error="second",
            build_id="build-b",
            stdout_bytes=5,
        )

        self.assertNotEqual(first, second)
        self.assertNotEqual(hash(first), hash(second))
        self.assertEqual(first.to_dict()["launch_error"], "first")
        self.assertEqual(first.to_dict()["build_id"], "build-a")
        self.assertEqual(first.to_dict()["stdout_bytes"], 4)
        self.assertEqual(asdict(first)["launch_error"], "first")
        self.assertEqual(asdict(first)["build_id"], "build-a")
        updated = replace(first, duration_ms=2)
        self.assertEqual(updated.launch_error, "first")
        self.assertEqual(updated.build_id, "build-a")
        self.assertEqual(updated.stdout_bytes, 4)

    def test_results_parent_symlink_is_rejected_without_writing_outside(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        outside = self.root / "outside"
        outside.mkdir()
        linked_parent = self.root / "linked-results"
        linked_parent.symlink_to(outside, target_is_directory=True)
        result_path = linked_parent / "results.ndjson"

        with self.assertRaises(OSError):
            run_baseline(
                self.stage,
                self.sysroot,
                result_path,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )
        self.assertFalse((outside / "results.ndjson").exists())


class CliTests(unittest.TestCase):
    def test_baseline_parser_registers_mutually_exclusive_exact_filters(self) -> None:
        parser = cli.create_parser()
        arguments = parser.parse_args(
            [
                "baseline", "--test", "conformance/interfaces/getpid/1-1.c",
                "--sysroot", "/sysroot",
            ]
        )
        self.assertEqual(arguments.command, "baseline")
        self.assertEqual(arguments.test, "conformance/interfaces/getpid/1-1.c")
        with redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                parser.parse_args(
                    [
                        "baseline", "--api", "getpid", "--group", "unistd",
                        "--sysroot", "/sysroot",
                    ]
                )

    def test_missing_qemu_prints_exact_prerequisite_without_writing_results(self) -> None:
        diagnostic = (
            "sudo apt-get install qemu-user gcc-aarch64-linux-gnu "
            "libc6-dev-arm64-cross"
        )
        with tempfile.TemporaryDirectory() as temporary:
            results = Path(temporary) / "results.ndjson"
            stderr = io.StringIO()
            with (
                mock.patch("scripts.posix.cli.shutil.which", return_value=None),
                mock.patch.object(cli, "BASELINE_RESULTS_PATH", results),
                redirect_stderr(stderr),
            ):
                returncode = cli.main(
                    ["baseline", "--api", "getpid", "--sysroot", "/usr/aarch64-linux-gnu"]
                )
        self.assertNotEqual(returncode, 0)
        self.assertIn(diagnostic + "\n", stderr.getvalue())
        self.assertFalse(results.exists())

    @mock.patch("scripts.posix.cli._current_build_inputs")
    @mock.patch("scripts.posix.cli.run_baseline")
    @mock.patch("scripts.posix.cli.shutil.which", return_value="/fake/qemu-aarch64")
    def test_cli_returns_nonzero_for_completed_nonpassing_campaign(
        self,
        _which: mock.Mock,
        run: mock.Mock,
        current_inputs: mock.Mock,
    ) -> None:
        current_inputs.return_value = (_metadata(), Path("checkout"), (), ())
        run.return_value = BaselineResult(
            attempts=(), all_passed=False, result_path=Path("results")
        )
        self.assertEqual(
            cli.main(["baseline", "--group", "unistd", "--sysroot", "/sysroot"]),
            1,
        )
        self.assertEqual(run.call_args.kwargs["group"], "unistd")


if __name__ == "__main__":
    unittest.main()
