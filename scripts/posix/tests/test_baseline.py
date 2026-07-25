from __future__ import annotations

from contextlib import ExitStack, redirect_stderr
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
from typing import get_type_hints
import unittest
from unittest import mock

from scripts.posix import baseline as baseline_module
from scripts.posix import cli
from scripts.posix.baseline import (
    MAX_CAPTURE_BYTES,
    PLATFORM,
    BaselinePrerequisiteError,
    BaselineResult,
    classify_status,
    filter_runnable_tests,
    run_baseline,
    run_runtime_attempt,
)
from scripts.posix.build import (
    CHECKSUM_DEFINITION,
    ManifestMetadata,
    _build_results_digest,
    _json_build_result,
    compile_command,
    link_command,
    nm_command,
    render_manifest,
    sha256_file,
)
from scripts.posix.model import BuildResult, BuildSummary, SuiteTest


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
    escaped_payload = (
        "import os,signal,time;"
        f"open({str(child_pid)!r},'w').write(str(os.getpid()));"
        "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
        "time.sleep(60)"
    )
    escaped_code = (
        "import os;"
        "os.setpgrp();"
        "pid=os.fork();"
        "pid and os._exit(0);"
        "os.setsid();"
        "os.execve('/usr/bin/python3',"
        f"['/usr/bin/python3','-c',{escaped_payload!r}],{{}})"
    )
    script = f"""#!/usr/bin/python3
import hashlib
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
    'pid': os.getpid(),
    'sysroot_libc_sha256': hashlib.sha256(
        (Path(sys.argv[2]) / 'lib/libc.so.6').read_bytes()
    ).hexdigest(),
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
if target.startswith('sigkill'):
    os.kill(os.getpid(), signal.SIGKILL)
if target.startswith('mutate-snapshot'):
    (Path(sys.argv[2]) / 'lib/libc.so.6').write_text(
        'mutated private runtime', encoding='utf-8'
    )
    raise SystemExit(0)
if target.startswith('chmod-snapshot-file'):
    (Path(sys.argv[2]) / 'lib/libc.so.6').chmod(0o677)
    raise SystemExit(0)
if target.startswith('chmod-snapshot-directory'):
    (Path(sys.argv[2]) / 'lib').chmod(0o777)
    raise SystemExit(0)
if target.startswith('add-snapshot-directory'):
    (Path(sys.argv[2]) / 'empty').mkdir()
    raise SystemExit(0)
if target.startswith('escape-'):
    subprocess.Popen(['/usr/bin/python3', '-c', {escaped_code!r}])
    deadline = time.monotonic() + 2.0
    while not Path({str(child_pid)!r}).exists():
        if time.monotonic() >= deadline:
            raise SystemExit(98)
        time.sleep(0.005)
    if target.startswith('escape-normal'):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    time.sleep(60)
if target.startswith('kill-launcher'):
    subprocess.Popen(['/usr/bin/python3', '-c', {escaped_code!r}])
    deadline = time.monotonic() + 2.0
    while not Path({str(child_pid)!r}).exists():
        if time.monotonic() >= deadline:
            raise SystemExit(98)
        time.sleep(0.005)
    os.kill(os.getppid(), signal.SIGKILL)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    time.sleep(60)
if target.startswith('kill-process-group'):
    subprocess.Popen(['/usr/bin/python3', '-c', {escaped_code!r}])
    deadline = time.monotonic() + 2.0
    while not Path({str(child_pid)!r}).exists():
        if time.monotonic() >= deadline:
            raise SystemExit(98)
        time.sleep(0.005)
    os.killpg(os.getpgrp(), signal.SIGKILL)
if target.startswith('stop-supervisor'):
    subprocess.Popen(['/usr/bin/python3', '-c', {escaped_code!r}])
    deadline = time.monotonic() + 2.0
    while not Path({str(child_pid)!r}).exists():
        if time.monotonic() >= deadline:
            raise SystemExit(98)
        time.sleep(0.005)
    os.kill(os.getppid(), signal.SIGSTOP)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    time.sleep(60)
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
        build_results: list[BuildResult] = []
        checkout = Path("target/posix/src") / _metadata().revision
        for test in sorted(tests, key=lambda item: item.test_id):
            if test.kind == "shell":
                continue
            object_path = Path("target/posix/aarch64/obj") / f"{test.test_id}.o"
            executable = (
                Path("target/posix/aarch64/bin") / f"{test.test_id}.test"
            )
            build_results.append(
                BuildResult(
                    test_id=test.test_id,
                    stage="compile",
                    status="passed",
                    argv=tuple(
                        compile_command(
                            "aarch64-linux-gnu-gcc",
                            checkout / test.source,
                            object_path,
                            checkout / "include",
                        )
                    ),
                    returncode=0,
                    stdout="",
                    stderr="",
                    duration_ms=1,
                    artifact_sha256="a" * 64,
                )
            )
            if test.kind != "runnable":
                continue
            build_results.extend(
                (
                    BuildResult(
                        test_id=test.test_id,
                        stage="nm",
                        status="passed",
                        argv=tuple(
                            nm_command("aarch64-linux-gnu-nm", object_path)
                        ),
                        returncode=0,
                        stdout="0000000000000000 T main\n",
                        stderr="",
                        duration_ms=1,
                        artifact_sha256=None,
                    ),
                    BuildResult(
                        test_id=test.test_id,
                        stage="link",
                        status="passed",
                        argv=tuple(
                            link_command(
                                "aarch64-linux-gnu-gcc",
                                object_path,
                                executable,
                            )
                        ),
                        returncode=0,
                        stdout="",
                        stderr="",
                        duration_ms=1,
                        artifact_sha256=test.sha256,
                    ),
                )
            )
        metadata = replace(
            _metadata(),
            build_results_sha256=_build_results_digest(build_results),
        )
        manifest, _ = render_manifest(metadata, tests)
        self.stage.mkdir(parents=True, exist_ok=True)
        (self.stage / "manifest.tsv").write_text(manifest, encoding="utf-8")
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
        (self.stage / "build-results.ndjson").write_text(
            "".join(_json_build_result(result) + "\n" for result in build_results),
            encoding="utf-8",
        )
        return metadata

    def synchronized_popen(self):
        real_popen = subprocess.Popen

        def launch_and_wait(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            process = real_popen(*args, **kwargs)
            setup_deadline = time.monotonic() + 5.0
            try:
                while True:
                    try:
                        child_pid = int(
                            self.child_pid.read_text(encoding="ascii")
                        )
                    except (FileNotFoundError, ValueError):
                        child_pid = 0
                    if child_pid > 0:
                        return process
                    if process.poll() is not None:
                        raise AssertionError(
                            "fake qemu exited before creating its descendant"
                        )
                    if time.monotonic() >= setup_deadline:
                        raise AssertionError(
                            "fake qemu did not create its descendant before "
                            "the setup deadline"
                        )
                    time.sleep(0.005)
            except BaseException:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                if process.returncode is None:
                    try:
                        process.wait(timeout=1.0)
                    except subprocess.TimeoutExpired:
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        try:
                            process.wait(timeout=1.0)
                        except subprocess.TimeoutExpired:
                            pass
                raise

        return launch_and_wait

    def assert_processes_reaped(self, pids: tuple[int, ...]) -> None:
        deadline = time.monotonic() + 2.0
        survivors = set(pids)
        try:
            while time.monotonic() < deadline:
                survivors = {
                    pid for pid in survivors if Path(f"/proc/{pid}").exists()
                }
                if not survivors:
                    return
                time.sleep(0.02)
        finally:
            for pid in survivors:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                try:
                    os.waitpid(pid, 0)
                except ChildProcessError:
                    pass
        self.fail(
            "runtime processes survived or were not reaped: "
            + ",".join(str(pid) for pid in sorted(survivors))
        )

    def assert_descendant_reaped(self) -> None:
        pid = int(self.child_pid.read_text(encoding="ascii"))
        self.assert_processes_reaped((pid,))

    def failing_temporary_directory(
        self, prefix: str, failure: BaseException
    ) -> mock._patch:
        real_temporary_directory = tempfile.TemporaryDirectory

        class FailingExit:
            def __init__(self, directory: tempfile.TemporaryDirectory[str]) -> None:
                self.directory = directory

            def __enter__(self) -> str:
                return self.directory.__enter__()

            def __exit__(self, *arguments: object) -> None:
                self.directory.__exit__(*arguments)
                raise failure

        def create(*arguments: object, **keywords: object):
            directory = real_temporary_directory(*arguments, **keywords)
            if keywords.get("prefix") == prefix:
                return FailingExit(directory)
            return directory

        return mock.patch(
            "scripts.posix.baseline.tempfile.TemporaryDirectory",
            side_effect=create,
        )


class StatusTests(unittest.TestCase):
    def test_exit_codes_have_exact_open_posix_classification(self) -> None:
        expected = {
            0: "pass",
            1: "fail",
            2: "unresolved",
            4: "unsupported",
            5: "untested",
            6: "fail",
            127: "fail",
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
            build_status="passed",
            link_status="passed",
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

    def test_empty_supervisor_control_is_infrastructure_failure(self) -> None:
        with mock.patch(
            "scripts.posix.baseline._supervisor_command",
            return_value=["/usr/bin/python3", "-c", "pass"],
        ):
            with self.assertRaisesRegex(ValueError, "invalid control data"):
                baseline_module._run_captured(
                    ["fake-qemu"],
                    cwd=self.root,
                    env={},
                    timeout_seconds=1.0,
                )

    def test_unknown_supervisor_control_is_infrastructure_failure(self) -> None:
        def command(_argv: object, descriptor: int) -> list[str]:
            code = f"import os; os.write({descriptor}, b'{{\"kind\":\"unknown\"}}\\n')"
            return ["/usr/bin/python3", "-c", code]

        with mock.patch(
            "scripts.posix.baseline._supervisor_command", side_effect=command
        ):
            with self.assertRaisesRegex(ValueError, "invalid control data"):
                baseline_module._run_captured(
                    ["fake-qemu"],
                    cwd=self.root,
                    env={},
                    timeout_seconds=1.0,
                )

    def test_supervisor_control_rejects_invalid_records(self) -> None:
        invalid_records = (
            b"not-json\n",
            b'{"kind":"result","kind":"result","returncode":0}\n',
            b'{"kind":"result","returncode":0}\n'
            b'{"kind":"result","returncode":1}\n',
            b'{"kind":"result","returncode":true}\n',
            b'{"extra":0,"kind":"result","returncode":0}\n',
            b'{"errno":true,"kind":"launch_error","strerror":"failed"}\n',
            b'{"kind":"infrastructure_error","message":1}\n',
            b'{"kind":"infrastructure_error","message":"failed",'
            b'"returncode":false}\n',
            b'{"kind":"infrastructure_error","message":"failed",'
            b'"returncode":null}\n',
        )

        for record in invalid_records:
            with self.subTest(record=record):
                with self.assertRaisesRegex(ValueError, "invalid control data"):
                    baseline_module._parse_supervisor_control(record)

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
            build_status="passed",
            link_status="passed",
        )
        self.assertEqual(launch.status, "launch-error")
        self.assertIsNone(launch.exit_code)
        self.assertIsNone(launch.signal)
        self.assertIn("missing-qemu", launch.launch_error or "")

    def test_sigkill_has_exact_signal_and_no_supervisor_stderr(self) -> None:
        attempt = self.attempt(self.make_test("sigkill-case"))

        self.assertEqual(attempt.status, "crash")
        self.assertEqual(attempt.signal, signal.SIGKILL)
        self.assertIsNone(attempt.exit_code)
        self.assertEqual(attempt.stderr, "")

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
        with mock.patch(
            "scripts.posix.baseline.subprocess.Popen",
            side_effect=self.synchronized_popen(),
        ):
            attempt = self.attempt(
                self.make_test("timeout-case", timeout_ms=120)
            )
        self.assertEqual(attempt.status, "timeout")
        self.assertTrue(attempt.timed_out)
        self.assert_descendant_reaped()

    def test_timeout_reaps_double_forked_setsid_descendant(self) -> None:
        with mock.patch(
            "scripts.posix.baseline.subprocess.Popen",
            side_effect=self.synchronized_popen(),
        ):
            attempt = self.attempt(
                self.make_test("escape-timeout", timeout_ms=120)
            )
        self.assertEqual(attempt.status, "timeout")
        self.assert_descendant_reaped()

    def test_timeout_reaps_escaped_descendant_after_environment_replacement(
        self,
    ) -> None:
        with mock.patch(
            "scripts.posix.baseline.subprocess.Popen",
            side_effect=self.synchronized_popen(),
        ):
            attempt = self.attempt(
                self.make_test("escape-env-timeout", timeout_ms=120)
            )
        self.assertEqual(attempt.status, "timeout")
        self.assert_descendant_reaped()

    def test_timeout_resumes_stopped_supervisor_and_reaps_escaped_descendant(
        self,
    ) -> None:
        synchronized = self.synchronized_popen()
        real_popen = subprocess.Popen
        unrelated: subprocess.Popen[bytes] | None = None

        def launch_with_unrelated(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            nonlocal unrelated
            process = synchronized(*args, **kwargs)
            unrelated = real_popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            return process

        try:
            with mock.patch(
                "scripts.posix.baseline.subprocess.Popen",
                side_effect=launch_with_unrelated,
            ):
                attempt = self.attempt(
                    self.make_test("stop-supervisor", timeout_ms=120)
                )
            self.assertEqual(attempt.status, "timeout")
            self.assert_descendant_reaped()
            assert unrelated is not None
            self.assertIsNone(unrelated.poll())
        finally:
            if unrelated is not None and unrelated.poll() is None:
                os.killpg(unrelated.pid, signal.SIGKILL)
                unrelated.wait(timeout=1.0)

    def test_parent_rescue_only_kills_stopped_supervisor_ancestry(self) -> None:
        synchronized = self.synchronized_popen()
        real_popen = subprocess.Popen
        unrelated: subprocess.Popen[bytes] | None = None

        def launch_with_unrelated(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            nonlocal unrelated
            process = synchronized(*args, **kwargs)
            unrelated = real_popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            return process

        try:
            with (
                mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    side_effect=launch_with_unrelated,
                ),
                mock.patch("scripts.posix.baseline._request_supervisor_shutdown"),
            ):
                attempt = self.attempt(
                    self.make_test("stop-supervisor", timeout_ms=120)
                )
            self.assertEqual(attempt.status, "timeout")
            self.assert_descendant_reaped()
            assert unrelated is not None
            self.assertIsNone(unrelated.poll())
        finally:
            if unrelated is not None and unrelated.poll() is None:
                os.killpg(unrelated.pid, signal.SIGKILL)
                unrelated.wait(timeout=1.0)

    def test_broker_survives_qemu_killing_its_immediate_parent(self) -> None:
        synchronized = self.synchronized_popen()
        real_popen = subprocess.Popen
        unrelated: subprocess.Popen[bytes] | None = None

        def launch_with_unrelated(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            nonlocal unrelated
            process = synchronized(*args, **kwargs)
            unrelated = real_popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            return process

        try:
            with mock.patch(
                "scripts.posix.baseline.subprocess.Popen",
                side_effect=launch_with_unrelated,
            ):
                test = self.make_test("kill-launcher", timeout_ms=2_000)
                self.write_manifest((test,))
                with self.assertRaises(ValueError):
                    run_baseline(
                        self.stage,
                        self.sysroot,
                        self.results,
                        qemu=self.qemu,
                        verifier=lambda _stage: None,
                    )
            observation = json.loads(
                self.observation.read_text(encoding="utf-8")
            )
            descendant_pid = int(self.child_pid.read_text(encoding="ascii"))
            rows = [
                json.loads(line)
                for line in self.results.read_text(encoding="utf-8").splitlines()
            ]
            attempt = rows[0]
            self.assertEqual(attempt["status"], "interrupted")
            self.assertEqual(attempt["launch_status"], "interrupted")
            self.assertIsNone(attempt["pts_status"])
            self.assertIsNone(attempt["exit_code"])
            self.assertIsNone(attempt["signal"])
            self.assertIn("launcher", attempt["infrastructure_error"])
            self.assertFalse(rows[-1]["complete"])
            self.assert_processes_reaped(
                (int(observation["pid"]), descendant_pid)
            )
            assert unrelated is not None
            self.assertIsNone(unrelated.poll())
        finally:
            if unrelated is not None and unrelated.poll() is None:
                os.killpg(unrelated.pid, signal.SIGKILL)
                unrelated.wait(timeout=1.0)

    def test_broker_survives_qemu_killing_launcher_process_group(self) -> None:
        synchronized = self.synchronized_popen()
        real_popen = subprocess.Popen
        unrelated: subprocess.Popen[bytes] | None = None

        def launch_with_unrelated(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            nonlocal unrelated
            process = synchronized(*args, **kwargs)
            unrelated = real_popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            return process

        try:
            with mock.patch(
                "scripts.posix.baseline.subprocess.Popen",
                side_effect=launch_with_unrelated,
            ):
                test = self.make_test("kill-process-group", timeout_ms=2_000)
                self.write_manifest((test,))
                with self.assertRaises(ValueError):
                    run_baseline(
                        self.stage,
                        self.sysroot,
                        self.results,
                        qemu=self.qemu,
                        verifier=lambda _stage: None,
                    )
            observation = json.loads(
                self.observation.read_text(encoding="utf-8")
            )
            descendant_pid = int(self.child_pid.read_text(encoding="ascii"))
            rows = [
                json.loads(line)
                for line in self.results.read_text(encoding="utf-8").splitlines()
            ]
            attempt = rows[0]
            self.assertEqual(attempt["status"], "interrupted")
            self.assertEqual(attempt["launch_status"], "interrupted")
            self.assertIsNone(attempt["pts_status"])
            self.assertIsNone(attempt["exit_code"])
            self.assertIsNone(attempt["signal"])
            self.assertIn("launcher", attempt["infrastructure_error"])
            self.assertFalse(rows[-1]["complete"])
            self.assert_processes_reaped(
                (int(observation["pid"]), descendant_pid)
            )
            assert unrelated is not None
            self.assertIsNone(unrelated.poll())
        finally:
            if unrelated is not None and unrelated.poll() is None:
                os.killpg(unrelated.pid, signal.SIGKILL)
                unrelated.wait(timeout=1.0)

    def test_interruption_reaps_double_forked_setsid_descendant(self) -> None:
        original = MemoryError("selector interrupted after descendant setup")
        with (
            mock.patch(
                "scripts.posix.baseline.subprocess.Popen",
                side_effect=self.synchronized_popen(),
            ),
            mock.patch(
                "scripts.posix.baseline.selectors.DefaultSelector.select",
                side_effect=original,
            ),
        ):
            with self.assertRaises(MemoryError) as raised:
                self.attempt(
                    self.make_test("escape-interrupt", timeout_ms=2_000)
                )
        self.assertIs(raised.exception, original)
        self.assert_descendant_reaped()

    def test_normal_exit_reaps_escaped_descendant_before_next_test(self) -> None:
        with mock.patch(
            "scripts.posix.baseline.subprocess.Popen",
            side_effect=self.synchronized_popen(),
        ):
            attempt = self.attempt(
                self.make_test("escape-normal", timeout_ms=2_000)
            )
        self.assertEqual(attempt.status, "pass")
        self.assert_descendant_reaped()
        follow_up = self.attempt(self.make_test("pass-after-escape"))
        self.assertEqual(follow_up.status, "pass")

    def test_capture_io_error_is_infrastructure_failure_not_launch_error(self) -> None:
        original = OSError(errno.EIO, "capture failed")
        with mock.patch(
            "scripts.posix.baseline.selectors.DefaultSelector.select",
            side_effect=original,
        ):
            with self.assertRaises(OSError) as raised:
                self.attempt(self.make_test("timeout-capture", timeout_ms=2_000))
        self.assertIs(raised.exception, original)

    def test_launcher_spawn_failure_is_supervisor_infrastructure_error(self) -> None:
        control_read, control_write = os.pipe()
        original = OSError(errno.EAGAIN, "launcher spawn failed")
        try:
            with (
                mock.patch("scripts.posix.baseline._require_pidfd_support"),
                mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    side_effect=original,
                ),
            ):
                returncode = baseline_module.supervise_runtime(
                    ["/fake/qemu-aarch64"], control_write
                )
            control_write = -1
            payload = json.loads(os.read(control_read, 4096))
        finally:
            os.close(control_read)
            if control_write >= 0:
                os.close(control_write)

        self.assertEqual(returncode, 125)
        self.assertEqual(payload["kind"], "infrastructure_error")
        self.assertIn("launcher spawn failed", payload["message"])

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

    def test_post_launch_setup_failure_reaps_supervisor(self) -> None:
        original = MemoryError("control stream allocation failed")
        real_popen = subprocess.Popen
        processes: list[subprocess.Popen[bytes]] = []

        def tracked_popen(*args: object, **kwargs: object) -> subprocess.Popen[bytes]:
            process = real_popen(*args, **kwargs)
            processes.append(process)
            return process

        try:
            with (
                mock.patch("scripts.posix.baseline.subprocess.Popen", tracked_popen),
                mock.patch("scripts.posix.baseline.os.fdopen", side_effect=original),
            ):
                with self.assertRaises(MemoryError) as raised:
                    self.attempt(
                        self.make_test("timeout-control", timeout_ms=2_000)
                    )
            self.assertIs(raised.exception, original)
            self.assertEqual(len(processes), 1)
            reaped = processes[0].poll() is not None
        finally:
            for process in processes:
                if process.poll() is None:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait(timeout=1.0)
                baseline_module._close_process_streams(process)
        self.assertTrue(reaped)

    def test_supervisor_cleanup_failure_is_infrastructure_error(self) -> None:
        control_read, control_write = os.pipe()
        process = mock.Mock(
            pid=999_998,
            returncode=None,
            stdin=None,
            stdout=None,
            stderr=None,
        )
        process.wait.side_effect = baseline_module._SupervisorInterrupted(
            signal.SIGTERM
        )
        process_tree = mock.Mock()
        process_tree.cleanup.side_effect = ValueError("cleanup failed")
        try:
            with (
                mock.patch(
                    "scripts.posix.baseline._LinuxProcessTree",
                    return_value=process_tree,
                ),
                mock.patch("scripts.posix.baseline._require_pidfd_support"),
                mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    return_value=process,
                ),
                mock.patch("scripts.posix.baseline.os.kill"),
            ):
                returncode = baseline_module.supervise_runtime(
                    ["/fake/qemu-aarch64"], control_write
                )
            control_write = -1
            payload = json.loads(os.read(control_read, 4096))
        finally:
            os.close(control_read)
            if control_write >= 0:
                os.close(control_write)

        self.assertEqual(returncode, 125)
        self.assertEqual(payload["kind"], "infrastructure_error")
        self.assertIn("cleanup failed", payload["message"])

    def test_supervisor_cleanup_failure_retains_known_runtime_result(self) -> None:
        control_read, control_write = os.pipe()
        process = mock.Mock(
            pid=999_997,
            returncode=0,
            stdin=None,
            stdout=None,
            stderr=None,
        )
        process.wait.return_value = 0
        process_tree = mock.Mock()
        process_tree.cleanup.side_effect = ValueError("cleanup failed")
        try:
            with (
                mock.patch(
                    "scripts.posix.baseline._LinuxProcessTree",
                    return_value=process_tree,
                ),
                mock.patch("scripts.posix.baseline._require_pidfd_support"),
                mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    return_value=process,
                ),
                mock.patch(
                    "scripts.posix.baseline._read_launcher_control",
                    return_value={"kind": "result", "returncode": 0},
                ),
            ):
                returncode = baseline_module.supervise_runtime(
                    ["/fake/qemu-aarch64"], control_write
                )
            control_write = -1
            payload = json.loads(os.read(control_read, 4096))
        finally:
            os.close(control_read)
            if control_write >= 0:
                os.close(control_write)

        self.assertEqual(returncode, 125)
        self.assertEqual(payload["kind"], "infrastructure_error")
        self.assertEqual(payload.get("returncode"), 0)
        self.assertIn("cleanup failed", payload["message"])

    def test_supervisor_cleanup_failure_retains_known_launch_error(self) -> None:
        control_read, control_write = os.pipe()
        process = mock.Mock(
            pid=999_996,
            returncode=125,
            stdin=None,
            stdout=None,
            stderr=None,
        )
        process.wait.return_value = 125
        process_tree = mock.Mock()
        process_tree.cleanup.side_effect = ValueError("cleanup failed")
        try:
            with (
                mock.patch(
                    "scripts.posix.baseline._LinuxProcessTree",
                    return_value=process_tree,
                ),
                mock.patch("scripts.posix.baseline._require_pidfd_support"),
                mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    return_value=process,
                ),
                mock.patch(
                    "scripts.posix.baseline._read_launcher_control",
                    return_value={
                        "errno": errno.ENOEXEC,
                        "kind": "launch_error",
                        "strerror": "Exec format error",
                    },
                ),
            ):
                returncode = baseline_module.supervise_runtime(
                    ["/fake/qemu-aarch64"], control_write
                )
            control_write = -1
            payload = json.loads(os.read(control_read, 4096))
        finally:
            os.close(control_read)
            if control_write >= 0:
                os.close(control_write)

        self.assertEqual(returncode, 125)
        self.assertEqual(payload["kind"], "infrastructure_error")
        self.assertEqual(payload.get("errno"), errno.ENOEXEC)
        self.assertEqual(payload.get("strerror"), "Exec format error")
        self.assertIn("cleanup failed", payload["message"])

    def test_parent_cleanup_failure_is_chained_from_capture_error(self) -> None:
        original = MemoryError("capture interrupted")
        cleanup = ValueError("parent cleanup failed")

        with (
            mock.patch(
                "scripts.posix.baseline.selectors.DefaultSelector.select",
                side_effect=original,
            ),
            mock.patch(
                "scripts.posix.baseline._LinuxProcessTree.cleanup",
                side_effect=cleanup,
            ),
        ):
            with self.assertRaises(ValueError) as raised:
                self.attempt(
                    self.make_test("timeout-cleanup", timeout_ms=2_000)
                )

        self.assertIs(raised.exception, cleanup)
        self.assertIs(raised.exception.__cause__, original)

    def test_process_tree_attach_interruption_reaps_launched_process(self) -> None:
        original = MemoryError("pidfd attach interrupted")
        with baseline_module._child_subreaper():
            process_tree = baseline_module._LinuxProcessTree()
            process = subprocess.Popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
            with mock.patch.object(process_tree, "attach", side_effect=original):
                with self.assertRaises(MemoryError) as raised:
                    baseline_module._attach_process_tree(process, process_tree)
            self.assertIs(raised.exception, original)
            self.assertIsNotNone(process.poll())

    def test_process_tree_attach_interruption_reaps_escaped_descendant(self) -> None:
        original = MemoryError("pidfd attach interrupted after descendant setup")
        test = self.make_test("escape-attach", timeout_ms=2_000)
        with baseline_module._child_subreaper():
            process_tree = baseline_module._LinuxProcessTree()
            process = self.synchronized_popen()(
                [
                    str(self.qemu),
                    "-L",
                    str(self.sysroot),
                    str(self.stage / test.binary),
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
            with mock.patch.object(process_tree, "attach", side_effect=original):
                with self.assertRaises(MemoryError) as raised:
                    baseline_module._attach_process_tree(process, process_tree)
        self.assertIs(raised.exception, original)
        self.assert_descendant_reaped()

    def _assert_runner_attach_failure_reaps_broker_tree(
        self, *, pidfd_fallback: bool
    ) -> None:
        original = MemoryError("runner pidfd attach interrupted")
        synchronized = self.synchronized_popen()
        real_popen = subprocess.Popen
        broker: subprocess.Popen[bytes] | None = None
        unrelated: subprocess.Popen[bytes] | None = None

        def launch_with_unrelated(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            nonlocal broker, unrelated
            broker = synchronized(*args, **kwargs)
            unrelated = real_popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            return broker

        try:
            with ExitStack() as stack:
                stack.enter_context(mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    side_effect=launch_with_unrelated,
                ))
                stack.enter_context(mock.patch.object(
                    baseline_module._LinuxProcessTree,
                    "attach",
                    side_effect=original,
                ))
                if pidfd_fallback:
                    stack.enter_context(
                        mock.patch(
                            "scripts.posix.baseline._require_pidfd_support"
                        )
                    )
                    stack.enter_context(
                        mock.patch(
                            "scripts.posix.baseline.os.pidfd_open",
                            side_effect=OSError(
                                errno.ENOSYS, "pidfd unavailable"
                            ),
                        )
                    )
                with self.assertRaises(MemoryError) as raised:
                    self.attempt(
                        self.make_test("escape-runner-attach", timeout_ms=2_000)
                    )

            self.assertIs(raised.exception, original)
            observation = json.loads(
                self.observation.read_text(encoding="utf-8")
            )
            descendant_pid = int(self.child_pid.read_text(encoding="ascii"))
            self.assert_processes_reaped(
                (int(observation["pid"]), descendant_pid)
            )
            assert broker is not None and unrelated is not None
            self.assertIsNotNone(broker.poll())
            self.assertIsNone(unrelated.poll())
        finally:
            if unrelated is not None and unrelated.poll() is None:
                os.killpg(unrelated.pid, signal.SIGKILL)
                unrelated.wait(timeout=1.0)

    def test_runner_attach_failure_reaps_broker_tree_but_not_unrelated_child(
        self,
    ) -> None:
        self._assert_runner_attach_failure_reaps_broker_tree(
            pidfd_fallback=False
        )

    def test_runner_attach_failure_uses_direct_child_pid_fallback(self) -> None:
        self._assert_runner_attach_failure_reaps_broker_tree(
            pidfd_fallback=True
        )

    def test_process_tree_does_not_claim_unrelated_concurrent_child(self) -> None:
        real_popen = subprocess.Popen
        unrelated: subprocess.Popen[bytes] | None = None

        def launch_with_unrelated(
            *args: object, **kwargs: object
        ) -> subprocess.Popen[bytes]:
            nonlocal unrelated
            process = real_popen(*args, **kwargs)
            unrelated = real_popen(
                ["/usr/bin/python3", "-c", "import time; time.sleep(60)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            return process

        try:
            with mock.patch(
                "scripts.posix.baseline.subprocess.Popen",
                side_effect=launch_with_unrelated,
            ):
                attempt = self.attempt(self.make_test("pass-unrelated"))
            self.assertEqual(attempt.status, "pass")
            assert unrelated is not None
            self.assertIsNone(unrelated.poll())
        finally:
            if unrelated is not None and unrelated.poll() is None:
                os.killpg(unrelated.pid, signal.SIGKILL)
                unrelated.wait(timeout=1.0)

    def test_pidfd_is_closed_when_identity_changes_after_open(self) -> None:
        tree = baseline_module._LinuxProcessTree()
        pid = 432_100
        sampled = baseline_module._ProcessIdentity(pid=pid, start_time=10)
        replacement = baseline_module._ProcessIdentity(pid=pid, start_time=11)
        sampled_entry = baseline_module._ProcessEntry(sampled, os.getpid())
        replacement_entry = baseline_module._ProcessEntry(replacement, os.getpid())

        with (
            mock.patch(
                "scripts.posix.baseline._process_table",
                side_effect=[{pid: sampled_entry}, {pid: replacement_entry}],
            ),
            mock.patch("scripts.posix.baseline.os.pidfd_open", return_value=987),
            mock.patch("scripts.posix.baseline.os.close") as close,
        ):
            tree.attach(pid)

        close.assert_called_once_with(987)
        self.assertEqual(tree._tracked, {})

    def test_pidfd_unavailable_is_an_infrastructure_failure(self) -> None:
        tree = baseline_module._LinuxProcessTree()
        pid = 432_101
        identity = baseline_module._ProcessIdentity(pid=pid, start_time=10)
        entry = baseline_module._ProcessEntry(identity, os.getpid())

        with (
            mock.patch(
                "scripts.posix.baseline._process_table",
                return_value={pid: entry},
            ),
            mock.patch(
                "scripts.posix.baseline.os.pidfd_open",
                side_effect=OSError(errno.ENOSYS, "pidfd unavailable"),
            ),
        ):
            with self.assertRaisesRegex(ValueError, "pidfd"):
                tree.attach(pid)

        self.assertEqual(tree._tracked, {})

    def test_supervisor_checks_pidfd_support_before_launch(self) -> None:
        control_read, control_write = os.pipe()
        process = mock.Mock(
            pid=999_999,
            returncode=0,
            stdin=None,
            stdout=None,
            stderr=None,
        )
        process.wait.return_value = 0
        try:
            with (
                mock.patch(
                    "scripts.posix.baseline.os.pidfd_open",
                    side_effect=OSError(errno.ENOSYS, "pidfd unavailable"),
                ),
                mock.patch(
                    "scripts.posix.baseline.subprocess.Popen",
                    return_value=process,
                ) as launch,
            ):
                returncode = baseline_module.supervise_runtime(
                    ["/fake/qemu-aarch64"], control_write
                )
            control_write = -1
            payload = json.loads(os.read(control_read, 4096))
        finally:
            os.close(control_read)
            if control_write >= 0:
                os.close(control_write)

        self.assertEqual(returncode, 125)
        self.assertEqual(payload["kind"], "infrastructure_error")
        self.assertIn("pidfd", payload["message"])
        launch.assert_not_called()

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
        maximum_select_calls = int(
            (
                baseline_module._SUPERVISOR_SHUTDOWN_SECONDS
                + baseline_module._KILL_REAP_SECONDS
                + 1.0
            )
            / 0.05
        ) + 10

        def bounded_select(wait: float) -> list[object]:
            nonlocal select_calls
            select_calls += 1
            selector_waits.append(wait)
            if select_calls > maximum_select_calls:
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
        self.assertLessEqual(select_calls, maximum_select_calls)
        self.assertTrue(all(wait > 0 for wait in selector_waits))


class CampaignTests(BaselineFixture, unittest.TestCase):
    def _run_with_raw_supervisor_control(
        self, test: SuiteTest, control: bytes
    ) -> BaseException:
        self.write_manifest((test,))

        def command(_argv: object, descriptor: int) -> list[str]:
            code = (
                "import os;"
                "os.write(1,b'observed-out\\n');"
                "os.write(2,b'observed-err\\n');"
                f"os.write({descriptor},{control!r})"
            )
            return ["/usr/bin/python3", "-B", "-c", code]

        with mock.patch(
            "scripts.posix.baseline._supervisor_command", side_effect=command
        ):
            with self.assertRaises(BaseException) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )
        return raised.exception

    def _run_with_supervisor_control(
        self, test: SuiteTest, payload: dict[str, object]
    ) -> BaseException:
        self.write_manifest((test,))

        def command(_argv: object, descriptor: int) -> list[str]:
            repository = str(Path(baseline_module.__file__).resolve().parents[2])
            code = (
                "import os,sys;"
                f"sys.path.insert(0,{repository!r});"
                "from scripts.posix.baseline import _write_supervisor_control;"
                "os.write(1,b'observed-out\\n');"
                "os.write(2,b'observed-err\\n');"
                f"_write_supervisor_control({descriptor},{payload!r})"
            )
            return ["/usr/bin/python3", "-B", "-c", code]

        with mock.patch(
            "scripts.posix.baseline._supervisor_command", side_effect=command
        ):
            with self.assertRaises(BaseException) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )
        return raised.exception

    def test_supervisor_infrastructure_error_retains_capture_and_result(self) -> None:
        original = self._run_with_supervisor_control(
            self.make_test("pass-case"),
            {
                "kind": "infrastructure_error",
                "message": "broker cleanup failed",
                "returncode": 0,
            },
        )

        self.assertIsInstance(original, ValueError)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(attempt["launch_status"], "launched")
        self.assertEqual(attempt["pts_status"], "pass")
        self.assertEqual(attempt["exit_code"], 0)
        self.assertEqual(attempt["stdout"], "observed-out\n")
        self.assertEqual(attempt["stderr"], "observed-err\n")
        self.assertIn("broker cleanup failed", attempt["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_invalid_supervisor_control_retains_capture_without_dimensions(
        self,
    ) -> None:
        records = (
            b"",
            b'{"kind":"unknown"}\n',
            b'{"kind":"result","returncode":0}\n'
            b'{"kind":"result","returncode":1}\n',
        )

        for record in records:
            with self.subTest(record=record):
                original = self._run_with_raw_supervisor_control(
                    self.make_test("pass-case"), record
                )
                self.assertIsInstance(original, ValueError)
                rows = [
                    json.loads(line)
                    for line in self.results.read_text(
                        encoding="utf-8"
                    ).splitlines()
                ]
                attempt = rows[0]
                self.assertEqual(attempt["status"], "interrupted")
                self.assertEqual(attempt["launch_status"], "interrupted")
                self.assertIsNone(attempt["pts_status"])
                self.assertIsNone(attempt["exit_code"])
                self.assertIsNone(attempt["signal"])
                self.assertEqual(attempt["stdout"], "observed-out\n")
                self.assertEqual(attempt["stderr"], "observed-err\n")
                self.assertFalse(rows[-1]["complete"])

    def test_supervisor_infrastructure_error_does_not_invent_runtime_result(
        self,
    ) -> None:
        original = self._run_with_supervisor_control(
            self.make_test("pass-case"),
            {
                "kind": "infrastructure_error",
                "message": "broker setup failed",
            },
        )

        self.assertIsInstance(original, ValueError)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(attempt["launch_status"], "interrupted")
        self.assertIsNone(attempt["pts_status"])
        self.assertIsNone(attempt["exit_code"])
        self.assertIsNone(attempt["signal"])
        self.assertEqual(attempt["stdout"], "observed-out\n")
        self.assertEqual(attempt["stderr"], "observed-err\n")
        self.assertIn("broker setup failed", attempt["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_supervisor_infrastructure_error_retains_known_launch_failure(
        self,
    ) -> None:
        original = self._run_with_supervisor_control(
            self.make_test("pass-case"),
            {
                "errno": errno.ENOEXEC,
                "kind": "infrastructure_error",
                "message": "broker cleanup failed",
                "strerror": "Exec format error",
            },
        )

        self.assertIsInstance(original, BaselinePrerequisiteError)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "launch-error")
        self.assertEqual(attempt["launch_status"], "launch-error")
        self.assertIsNone(attempt["pts_status"])
        self.assertIsNone(attempt["exit_code"])
        self.assertIn("Exec format error", attempt["launch_error"])
        self.assertEqual(attempt["stdout"], "observed-out\n")
        self.assertEqual(attempt["stderr"], "observed-err\n")
        self.assertIn("broker cleanup failed", attempt["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_oversized_infrastructure_control_retains_capture_and_result(
        self,
    ) -> None:
        original = self._run_with_supervisor_control(
            self.make_test("pass-case"),
            {
                "kind": "infrastructure_error",
                "message": "broker cleanup failed " + "\x01" * 5_000,
                "returncode": 0,
            },
        )

        self.assertIsInstance(original, ValueError)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(attempt["launch_status"], "launched")
        self.assertEqual(attempt["pts_status"], "pass")
        self.assertEqual(attempt["exit_code"], 0)
        self.assertEqual(attempt["stdout"], "observed-out\n")
        self.assertEqual(attempt["stderr"], "observed-err\n")
        self.assertIn("broker cleanup failed", attempt["infrastructure_error"])
        self.assertTrue(
            attempt["infrastructure_error"].endswith("\n...[truncated]")
        )
        self.assertFalse(rows[-1]["complete"])

    def test_oversized_infrastructure_control_retains_launch_failure(
        self,
    ) -> None:
        original = self._run_with_supervisor_control(
            self.make_test("pass-case"),
            {
                "errno": errno.ENOEXEC,
                "kind": "infrastructure_error",
                "message": "broker cleanup failed " + "\x01" * 5_000,
                "strerror": "Exec format error " + "\x02" * 5_000,
            },
        )

        self.assertIsInstance(original, BaselinePrerequisiteError)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "launch-error")
        self.assertEqual(attempt["launch_status"], "launch-error")
        self.assertIsNone(attempt["pts_status"])
        self.assertIn("Exec format error", attempt["launch_error"])
        self.assertEqual(attempt["stdout"], "observed-out\n")
        self.assertEqual(attempt["stderr"], "observed-err\n")
        self.assertIn("broker cleanup failed", attempt["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_broker_spawn_failure_is_campaign_infrastructure_error(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        original = OSError(errno.EAGAIN, "broker spawn failed")

        with mock.patch(
            "scripts.posix.baseline.subprocess.Popen", side_effect=original
        ):
            with self.assertRaises(BaseException) as raised:
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
        attempt = rows[0]
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(attempt["launch_status"], "interrupted")
        self.assertIsNone(attempt["launch_error"])
        self.assertIn("broker spawn failed", attempt["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_sysroot_cleanup_failure_publishes_incomplete_attempt(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        original = OSError(errno.EIO, "sysroot cleanup failed")

        with self.failing_temporary_directory(
            "smros-posix-sysroot-", original
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
        self.assertEqual(
            [row["record_type"] for row in rows], ["attempt", "run"]
        )
        self.assertEqual(rows[0]["status"], "interrupted")
        self.assertEqual(rows[0]["launch_status"], "launched")
        self.assertEqual(rows[0]["pts_status"], "pass")
        self.assertIn("sysroot cleanup failed", rows[0]["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_attempt_temp_cleanup_retains_observed_result_and_capture(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        original = OSError(errno.EIO, "attempt cleanup failed")

        with self.failing_temporary_directory(
            "smros-posix-baseline-", original
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
        self.assertEqual(rows[0]["status"], "interrupted")
        self.assertEqual(rows[0]["launch_status"], "launched")
        self.assertEqual(rows[0]["pts_status"], "pass")
        self.assertEqual((rows[0]["stdout"], rows[0]["stderr"]), (
            "pass-out\n", "pass-err\n"
        ))
        self.assertIn("attempt cleanup failed", rows[0]["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_observed_runtime_error_survives_attempt_temp_cleanup(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        inner = OSError(errno.EIO, "inner-capture-failure")
        observation = baseline_module._RuntimeObservation(
            returncode=0,
            timed_out=False,
            stdout=baseline_module._Capture("observed-out\n", 13, False),
            stderr=baseline_module._Capture(
                "observed-err\n...[truncated]", MAX_CAPTURE_BYTES + 7, True
            ),
            launch_status="launched",
        )
        setattr(inner, "_smros_posix_runtime_observation", observation)
        cleanup = OSError(errno.ENOSPC, "attempt-temp-cleanup")

        with (
            self.failing_temporary_directory(
                "smros-posix-baseline-", cleanup
            ),
            mock.patch(
                "scripts.posix.baseline._run_captured",
                side_effect=inner,
            ),
        ):
            with self.assertRaises(OSError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )

        self.assertIs(raised.exception, inner)
        self.assertIs(raised.exception.__cause__, cleanup)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(attempt["launch_status"], "launched")
        self.assertEqual(attempt["pts_status"], "pass")
        self.assertEqual(attempt["exit_code"], 0)
        self.assertIsNone(attempt["signal"])
        self.assertEqual(attempt["stdout"], "observed-out\n")
        self.assertEqual(attempt["stderr"], "observed-err\n...[truncated]")
        self.assertEqual(attempt["stdout_bytes"], 13)
        self.assertEqual(attempt["stderr_bytes"], MAX_CAPTURE_BYTES + 7)
        self.assertFalse(attempt["stdout_truncated"])
        self.assertTrue(attempt["stderr_truncated"])
        detail = attempt["infrastructure_error"]
        self.assertLessEqual(len(detail.encode("utf-8")), 4_096)
        self.assertIn("inner-capture-failure", detail)
        self.assertIn("attempt-temp-cleanup", detail)
        self.assertFalse(rows[-1]["complete"])

    def test_unobserved_runtime_error_retains_attempt_cleanup_detail(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        inner = OSError(errno.EIO, "inner-before-observation")
        cleanup = OSError(errno.ENOSPC, "attempt-temp-cleanup")

        with (
            self.failing_temporary_directory(
                "smros-posix-baseline-", cleanup
            ),
            mock.patch(
                "scripts.posix.baseline._run_captured",
                side_effect=inner,
            ),
        ):
            with self.assertRaises(OSError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )

        self.assertIs(raised.exception, inner)
        self.assertIs(raised.exception.__cause__, cleanup)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        attempt = rows[0]
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(attempt["launch_status"], "interrupted")
        self.assertIsNone(attempt["pts_status"])
        self.assertIsNone(attempt["exit_code"])
        self.assertIsNone(attempt["signal"])
        self.assertEqual((attempt["stdout"], attempt["stderr"]), ("", ""))
        detail = attempt["infrastructure_error"]
        self.assertLessEqual(len(detail.encode("utf-8")), 4_096)
        self.assertIn("inner-before-observation", detail)
        self.assertIn("attempt-temp-cleanup", detail)
        self.assertFalse(rows[-1]["complete"])

    def test_launch_and_temp_cleanup_failure_retains_both_details(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        invalid_qemu = self.root / "invalid-qemu"
        invalid_qemu.write_bytes(b"not an executable image")
        invalid_qemu.chmod(0o755)
        cleanup = OSError(errno.EIO, "attempt cleanup failed")

        with self.failing_temporary_directory(
            "smros-posix-baseline-", cleanup
        ):
            with self.assertRaises(BaselinePrerequisiteError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=invalid_qemu,
                    verifier=lambda _stage: None,
                )

        self.assertIs(raised.exception.__cause__, cleanup)
        self.assertIn("Exec format error", str(raised.exception))
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(rows[0]["status"], "launch-error")
        self.assertEqual(rows[0]["launch_status"], "launch-error")
        self.assertIn("Exec format error", rows[0]["launch_error"])
        self.assertIn(str(invalid_qemu), rows[0]["launch_error"])
        self.assertIn("attempt cleanup failed", rows[0]["infrastructure_error"])
        self.assertIsNone(rows[0]["pts_status"])
        self.assertFalse(rows[-1]["complete"])

    def test_launch_and_sysroot_cleanup_preserves_typed_failure(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        invalid_qemu = self.root / "invalid-qemu"
        invalid_qemu.write_bytes(b"not an executable image")
        invalid_qemu.chmod(0o755)
        cleanup = OSError(errno.EIO, "sysroot cleanup failed")

        with self.failing_temporary_directory(
            "smros-posix-sysroot-", cleanup
        ):
            with self.assertRaises(BaselinePrerequisiteError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=invalid_qemu,
                    verifier=lambda _stage: None,
                )

        self.assertIs(raised.exception.__cause__, cleanup)
        self.assertIn("Exec format error", str(raised.exception))
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(rows[0]["status"], "launch-error")
        self.assertEqual(rows[0]["launch_status"], "launch-error")
        self.assertIn("Exec format error", rows[0]["launch_error"])
        self.assertIn("sysroot cleanup failed", rows[0]["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_combined_infrastructure_details_retain_both_failures(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        inner = OSError(
            errno.EIO, "inner-capture-failure " + "x" * 5_000
        )
        cleanup = OSError(errno.ENOSPC, "outer-sysroot-cleanup")

        with (
            self.failing_temporary_directory(
                "smros-posix-sysroot-", cleanup
            ),
            mock.patch(
                "scripts.posix.baseline.run_runtime_attempt",
                side_effect=inner,
            ),
        ):
            with self.assertRaises(OSError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )

        self.assertIs(raised.exception, inner)
        self.assertIs(raised.exception.__cause__, cleanup)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        detail = rows[0]["infrastructure_error"]
        self.assertLessEqual(len(detail.encode("utf-8")), 4_096)
        self.assertIn("inner-capture-failure", detail)
        self.assertIn("outer-sysroot-cleanup", detail)
        self.assertEqual(rows[0]["status"], "interrupted")
        self.assertFalse(rows[-1]["complete"])

    def test_sysroot_snapshot_failure_remains_a_typed_prerequisite(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))

        real_copy = baseline_module._copy_runtime_file

        def remove_source_before_copy(
            source: Path, destination: Path, **kwargs: object
        ) -> tuple[str, int]:
            source.unlink()
            return real_copy(source, destination, **kwargs)

        with mock.patch(
            "scripts.posix.baseline._copy_runtime_file",
            side_effect=remove_source_before_copy,
        ):
            with self.assertRaises(BaselinePrerequisiteError):
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )

    def test_snapshot_destination_failure_is_not_a_prerequisite(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))

        with mock.patch(
            "scripts.posix.baseline._copy_runtime_file",
            side_effect=OSError(errno.ENOSPC, "snapshot destination full"),
        ):
            with self.assertRaises(OSError) as raised:
                run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )

        self.assertEqual(raised.exception.errno, errno.ENOSPC)

    def test_authoritative_qemu_exec_failure_is_a_typed_prerequisite(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        invalid_qemu = self.root / "invalid-qemu"
        invalid_qemu.write_bytes(b"not an executable image")
        invalid_qemu.chmod(0o755)

        with self.assertRaises(BaselinePrerequisiteError):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=invalid_qemu,
                verifier=lambda _stage: None,
            )

        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(
            [row["record_type"] for row in rows], ["attempt", "run"]
        )
        self.assertEqual(rows[0]["status"], "launch-error")
        self.assertEqual(rows[0]["launch_status"], "launch-error")
        self.assertIsNone(rows[0]["pts_status"])
        self.assertIsNone(rows[0]["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

    def test_private_sysroot_mutation_marks_campaign_incomplete(self) -> None:
        test = self.make_test("mutate-snapshot")
        self.write_manifest((test,))

        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(
            [row["record_type"] for row in rows], ["attempt", "run"]
        )
        self.assertEqual(rows[0]["status"], "interrupted")
        self.assertEqual(rows[0]["launch_status"], "launched")
        self.assertEqual(rows[0]["pts_status"], "pass")
        self.assertIn("checksum mismatch", rows[0]["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])
        self.assertEqual(rows[-1]["status_counts"], {"interrupted": 1})

    def test_private_sysroot_file_mode_mutation_marks_campaign_incomplete(self) -> None:
        test = self.make_test("chmod-snapshot-file")
        self.write_manifest((test,))

        with self.assertRaisesRegex(ValueError, "mode mismatch"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertFalse(rows[-1]["complete"])

    def test_private_sysroot_directory_mode_mutation_marks_incomplete(self) -> None:
        test = self.make_test("chmod-snapshot-directory")
        self.write_manifest((test,))

        with self.assertRaisesRegex(ValueError, "mode mismatch"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertFalse(rows[-1]["complete"])

    def test_private_sysroot_extra_empty_directory_marks_incomplete(self) -> None:
        test = self.make_test("add-snapshot-directory")
        self.write_manifest((test,))

        with self.assertRaisesRegex(ValueError, "inventory mismatch"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertFalse(rows[-1]["complete"])

    def test_campaign_restores_existing_child_subreaper_state(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        before = baseline_module._child_subreaper_enabled()

        result = run_baseline(
            self.stage,
            self.sysroot,
            self.results,
            qemu=self.qemu,
            verifier=lambda _stage: None,
        )

        self.assertTrue(result.all_passed)
        self.assertEqual(baseline_module._child_subreaper_enabled(), before)

    def test_campaign_preserves_pre_enabled_child_subreaper_state(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))

        with baseline_module._child_subreaper():
            self.assertTrue(baseline_module._child_subreaper_enabled())
            result = run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )
            self.assertTrue(baseline_module._child_subreaper_enabled())

        self.assertTrue(result.all_passed)

    def test_qemu_accepts_any_regular_file_with_execute_bits(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        for mode in (0o555, 0o775):
            with self.subTest(mode=oct(mode)):
                self.qemu.chmod(mode)
                result = run_baseline(
                    self.stage,
                    self.sysroot,
                    self.results,
                    qemu=self.qemu,
                    verifier=lambda _stage: None,
                )
                self.assertTrue(result.all_passed)

        for mode in (0o050, 0o005):
            with self.subTest(validation_mode=oct(mode)):
                self.qemu.chmod(mode)
                self.assertEqual(
                    baseline_module._resolve_qemu(self.qemu),
                    self.qemu.resolve(),
                )

    def test_qemu_symlink_loop_is_a_typed_prerequisite_failure(self) -> None:
        first = self.root / "qemu-loop-a"
        second = self.root / "qemu-loop-b"
        first.symlink_to(second)
        second.symlink_to(first)

        with self.assertRaises(BaselinePrerequisiteError):
            baseline_module._resolve_qemu(first)

    def test_configured_sysroot_mutation_cannot_change_campaign_snapshot(self) -> None:
        tests = (
            self.make_test("pass-one"),
            self.make_test("pass-two"),
        )
        self.write_manifest(tests)
        base_build_id = baseline_module._load_stage_identity(
            self.stage
        ).build_id
        original_libc_sha256 = sha256_file(
            self.sysroot / "lib" / "libc.so.6"
        )
        real_run_attempt = baseline_module.run_runtime_attempt
        calls = 0

        def mutate_before_second(*args: object, **kwargs: object):
            nonlocal calls
            calls += 1
            if calls == 2:
                _write_executable(
                    self.sysroot / "lib" / "libc.so.6",
                    "configured sysroot replacement",
                )
            return real_run_attempt(*args, **kwargs)

        with mock.patch(
            "scripts.posix.baseline.run_runtime_attempt",
            side_effect=mutate_before_second,
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
        self.assertNotEqual(observation["argv"][2], str(self.sysroot))
        self.assertEqual(
            observation["sysroot_libc_sha256"], original_libc_sha256
        )
        snapshot_digests = {
            attempt.runtime_snapshot_sha256 for attempt in result.attempts
        }
        self.assertEqual(len(snapshot_digests), 1)
        runtime_snapshot_sha256 = snapshot_digests.pop()
        self.assertRegex(runtime_snapshot_sha256, r"^[0-9a-f]{64}$")
        self.assertEqual(len({attempt.build_id for attempt in result.attempts}), 1)
        self.assertNotEqual(result.attempts[0].build_id, base_build_id)
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(
            rows[-1]["runtime_snapshot_sha256"], runtime_snapshot_sha256
        )

    def test_nonstandard_exit_and_signal_have_distinct_terminal_counts(self) -> None:
        tests = (
            self.make_test("other-exit"),
            self.make_test("signal-exit"),
        )
        self.write_manifest(tests)

        result = run_baseline(
            self.stage,
            self.sysroot,
            self.results,
            qemu=self.qemu,
            verifier=lambda _stage: None,
        )

        self.assertEqual(
            [attempt.status for attempt in result.attempts], ["fail", "crash"]
        )
        rows = [
            json.loads(line)
            for line in self.results.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(rows[-1]["status_counts"], {"crash": 1, "fail": 1})

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
        self.assertEqual(rows[0]["group"], "unistd")
        self.assertEqual(rows[0]["api"], "getpid")
        self.assertEqual(rows[0]["build_status"], "passed")
        self.assertEqual(rows[0]["link_status"], "passed")
        self.assertEqual(rows[0]["launch_status"], "launched")
        self.assertEqual(rows[0]["pts_status"], "fail")
        self.assertIsNone(rows[0]["infrastructure_error"])
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

    def test_first_report_fsyncs_each_new_parent_before_descent(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        self.results = (
            self.root
            / "durable-baseline-results"
            / "nested"
            / "results.ndjson"
        )
        target_parts = {"durable-baseline-results", "nested"}
        events: list[tuple[str, object, object]] = []
        real_mkdir = baseline_module.os.mkdir
        real_open = baseline_module.os.open
        real_fsync = baseline_module.os.fsync

        def traced_mkdir(path, *args, **kwargs) -> None:
            parent = kwargs.get("dir_fd")
            if os.fspath(path) in target_parts:
                assert isinstance(parent, int)
                info = os.fstat(parent)
                parent_identity = (info.st_dev, info.st_ino)
            else:
                parent_identity = None
            real_mkdir(path, *args, **kwargs)
            if parent_identity is not None:
                events.append(("mkdir", os.fspath(path), parent_identity))

        def traced_open(path, flags, *args, **kwargs) -> int:
            descriptor = real_open(path, flags, *args, **kwargs)
            if os.fspath(path) in target_parts:
                events.append(("open", os.fspath(path), None))
            return descriptor

        def traced_fsync(descriptor: int) -> None:
            real_fsync(descriptor)
            info = os.fstat(descriptor)
            events.append(("fsync", None, (info.st_dev, info.st_ino)))

        with (
            mock.patch.object(
                baseline_module.os,
                "mkdir",
                side_effect=traced_mkdir,
            ),
            mock.patch.object(
                baseline_module.os,
                "open",
                side_effect=traced_open,
            ),
            mock.patch.object(
                baseline_module.os,
                "fsync",
                side_effect=traced_fsync,
            ),
        ):
            result = run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        for part in ("durable-baseline-results", "nested"):
            mkdir_index = next(
                index
                for index, event in enumerate(events)
                if event[0:2] == ("mkdir", part)
            )
            parent_identity = events[mkdir_index][2]
            open_index = next(
                index
                for index, event in enumerate(events)
                if index > mkdir_index and event[0:2] == ("open", part)
            )
            self.assertTrue(
                any(
                    event == ("fsync", None, parent_identity)
                    for event in events[mkdir_index + 1 : open_index]
                ),
                f"parent of {part} was not fsynced before descent",
            )
        self.assertTrue(result.all_passed)
        self.assertTrue(self.results.is_file())

    def test_build_statuses_are_checksum_bound_to_staged_results(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        build_results = self.stage / "build-results.ndjson"
        rows = [
            json.loads(line)
            for line in build_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[0]["stdout"] = "tampered build diagnostic"
        build_results.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "build results checksum mismatch"):
            run_baseline(
                self.stage,
                self.sysroot,
                self.results,
                qemu=self.qemu,
                verifier=lambda _stage: None,
            )

        self.assertFalse(self.observation.exists())
        self.assertFalse(self.results.exists())

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
        original = OSError(
            errno.EIO, "runtime capture failed " + "x" * 5_000
        )
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
        self.assertEqual(
            [row["record_type"] for row in rows],
            ["attempt", "attempt", "run"],
        )
        self.assertEqual(rows[1]["test_id"], tests[1].test_id)
        self.assertEqual(rows[1]["status"], "interrupted")
        self.assertEqual(rows[1]["launch_status"], "interrupted")
        self.assertIsNone(rows[1]["pts_status"])
        self.assertIn("runtime capture failed", rows[1]["infrastructure_error"])
        self.assertLessEqual(
            len(rows[1]["infrastructure_error"].encode("utf-8")), 4_096
        )
        self.assertTrue(
            rows[1]["infrastructure_error"].endswith("\n...[truncated]")
        )
        self.assertFalse(rows[-1]["complete"])
        self.assertEqual(rows[-1]["selected_count"], 2)
        self.assertEqual(rows[-1]["completed_count"], 2)

    def test_incomplete_publication_failure_is_not_hidden(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        self.results.parent.mkdir(parents=True)
        self.results.write_text("known-good\n", encoding="utf-8")
        original = OSError(errno.EIO, "runtime capture failed")
        publication = OSError(
            errno.ENOSPC, "report publication failed " + "x" * 5_000
        )

        with (
            mock.patch(
                "scripts.posix.baseline.run_runtime_attempt",
                side_effect=original,
            ),
            mock.patch(
                "scripts.posix.baseline._publish_report",
                side_effect=publication,
            ),
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
        self.assertEqual(self.results.read_text(encoding="utf-8"), "known-good\n")
        notes = getattr(raised.exception, "__notes__", ())
        self.assertTrue(all(len(note.encode("utf-8")) <= 4_096 for note in notes))
        self.assertTrue(
            any(
                "incomplete report publication failed" in note
                and "report publication failed" in note
                for note in notes
            )
        )
        self.assertTrue(notes[-1].endswith("\n...[truncated]"))

    def test_post_execution_cleanup_retains_raw_pts_status(self) -> None:
        test = self.make_test("pass-case")
        self.write_manifest((test,))
        original = ValueError("post-execution cleanup failed")

        with mock.patch.object(
            baseline_module._LinuxProcessTree,
            "cleanup",
            side_effect=original,
        ):
            with self.assertRaises(ValueError) as raised:
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
        self.assertEqual(
            [row["record_type"] for row in rows], ["attempt", "run"]
        )
        self.assertEqual(rows[0]["status"], "interrupted")
        self.assertEqual(rows[0]["launch_status"], "launched")
        self.assertEqual(rows[0]["pts_status"], "pass")
        self.assertIn("cleanup failed", rows[0]["infrastructure_error"])
        self.assertFalse(rows[-1]["complete"])

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
            "group": "base",
            "api": "test",
            "platform": PLATFORM,
            "build_status": "passed",
            "link_status": "passed",
            "launch_status": "launch-error",
            "pts_status": None,
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
            runtime_snapshot_sha256="1" * 64,
        )
        second = baseline_module.RuntimeAttempt(
            **arguments,
            launch_error="second",
            build_id="build-b",
            stdout_bytes=5,
            runtime_snapshot_sha256="2" * 64,
        )

        self.assertNotEqual(first, second)
        self.assertNotEqual(hash(first), hash(second))
        self.assertEqual(first.to_dict()["launch_error"], "first")
        self.assertEqual(first.to_dict()["build_id"], "build-a")
        self.assertEqual(first.to_dict()["stdout_bytes"], 4)
        self.assertEqual(first.to_dict()["runtime_snapshot_sha256"], "1" * 64)
        self.assertEqual(asdict(first)["launch_error"], "first")
        self.assertEqual(asdict(first)["build_id"], "build-a")
        updated = replace(first, duration_ms=2)
        self.assertEqual(updated.launch_error, "first")
        self.assertEqual(updated.build_id, "build-a")
        self.assertEqual(updated.stdout_bytes, 4)
        self.assertEqual(updated.runtime_snapshot_sha256, "1" * 64)

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
    def test_cli_prints_incomplete_publication_failure_note(
        self,
        _which: mock.Mock,
        run: mock.Mock,
        current_inputs: mock.Mock,
    ) -> None:
        current_inputs.return_value = (_metadata(), Path("checkout"), (), ())
        failure = OSError(errno.EIO, "runtime capture failed")
        failure.add_note(
            "incomplete report publication failed: "
            "OSError: report publication failed"
        )
        run.side_effect = failure
        stderr = io.StringIO()

        with redirect_stderr(stderr):
            returncode = cli.main(
                ["baseline", "--api", "getpid", "--sysroot", "/sysroot"]
            )

        self.assertEqual(returncode, 1)
        self.assertIn(
            "baseline failed: [Errno 5] runtime capture failed\n",
            stderr.getvalue(),
        )
        self.assertIn(
            "incomplete report publication failed: "
            "OSError: report publication failed\n",
            stderr.getvalue(),
        )
        self.assertNotIn(cli.BASELINE_PREREQUISITE, stderr.getvalue())

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
        verifier = run.call_args.kwargs["verifier"]
        self.assertIs(get_type_hints(verifier)["return"], BuildSummary)

    @mock.patch("scripts.posix.cli._current_build_inputs")
    @mock.patch("scripts.posix.cli.run_baseline")
    @mock.patch("scripts.posix.cli.shutil.which", return_value="/fake/qemu-aarch64")
    def test_typed_prerequisite_failure_always_prints_install_diagnostic(
        self,
        _which: mock.Mock,
        run: mock.Mock,
        current_inputs: mock.Mock,
    ) -> None:
        current_inputs.return_value = (_metadata(), Path("checkout"), (), ())
        prerequisite = getattr(
            baseline_module, "BaselinePrerequisiteError", ValueError
        )
        run.side_effect = prerequisite("opaque prerequisite failure")
        stderr = io.StringIO()

        with redirect_stderr(stderr):
            returncode = cli.main(
                ["baseline", "--api", "getpid", "--sysroot", "/sysroot"]
            )

        self.assertEqual(returncode, 1)
        self.assertIn(cli.BASELINE_PREREQUISITE + "\n", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
