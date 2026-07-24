from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from scripts.posix.build import ManifestMetadata
from scripts.posix import cli as cli_module
from scripts.posix.cli import create_parser
from scripts.posix.model import BuildResult, ResourceDeltas, SuiteTest
from scripts.posix.qemu_runner import (
    CampaignIdentity,
    ControllerConfig,
    ControllerError,
    QemuController,
    _PopenTransport,
    build_qemu_argv,
)


MANIFEST_SHA256 = "a" * 64
BUILD_RESULTS_SHA256 = "b" * 64
BUILD_ID = "c" * 64
PATCH_SHA256 = "d" * 64
REVISION = "e" * 40
SMROS_COMMIT = "f" * 40
PROMPT = b"smros:/> "


class FakeClock:
    def __init__(self) -> None:
        self.value = 0.0

    def monotonic(self) -> float:
        return self.value

    def advance(self, seconds: float) -> None:
        self.value += seconds


class FakeTransport:
    def __init__(self, clock: FakeClock, script: list[object]) -> None:
        self.clock = clock
        self.script = list(script)
        self.returncode: int | None = None
        self.writes: list[bytes] = []
        self.reads = 0
        self.reads_at_write: list[int] = []
        self.terminated = False
        self.killed = False

    def read(self, timeout: float) -> bytes:
        self.reads += 1
        if not self.script:
            self.clock.advance(timeout)
            return b""
        item = self.script.pop(0)
        if isinstance(item, bytes):
            return item
        if isinstance(item, tuple) and item[0] == "advance":
            self.clock.advance(float(item[1]))
            return b""
        if isinstance(item, tuple) and item[0] == "exit":
            self.returncode = int(item[1])
            return b""
        if isinstance(item, BaseException):
            raise item
        raise AssertionError(f"unknown fake transport item: {item!r}")

    def write(self, data: bytes) -> None:
        self.writes.append(data)
        self.reads_at_write.append(self.reads)

    def poll(self) -> int | None:
        return self.returncode

    def terminate(self) -> None:
        self.terminated = True
        if self.returncode is None:
            self.returncode = -15

    def wait(self, timeout: float) -> int:
        del timeout
        if self.returncode is None:
            raise subprocess.TimeoutExpired("fake-qemu", 0)
        return self.returncode

    def kill(self) -> None:
        self.killed = True
        self.returncode = -9


class TransportFactory:
    def __init__(self, transports: list[FakeTransport]) -> None:
        self.transports = list(transports)
        self.argv: list[tuple[str, ...]] = []

    def __call__(self, argv: tuple[str, ...]) -> FakeTransport:
        self.argv.append(argv)
        if not self.transports:
            raise AssertionError("unexpected QEMU restart")
        return self.transports.pop(0)


def _test(name: str, *, timeout_ms: int = 1_000) -> SuiteTest:
    return SuiteTest(
        test_id=f"conformance/interfaces/getpid/{name}.c",
        group="base",
        api="getpid",
        kind="runnable",
        disposition="complete",
        source=f"conformance/interfaces/getpid/{name}.c",
        binary=f"bin/conformance/interfaces/getpid/{name}.c.test",
        sha256=("1" if name == "one" else "2") * 64,
        timeout_ms=timeout_ms,
    )


def _build_result(test: SuiteTest, stage: str) -> BuildResult:
    return BuildResult(
        test_id=test.test_id,
        stage=stage,
        status="passed",
        argv=("tool", test.test_id),
        returncode=0,
        stdout="",
        stderr="",
        duration_ms=1,
        artifact_sha256=test.sha256,
    )


def _identity(
    tests: tuple[SuiteTest, ...], *, build_id: str = BUILD_ID
) -> CampaignIdentity:
    metadata = ManifestMetadata(
        source="https://example.invalid/posixtest.git",
        revision=REVISION,
        architecture="aarch64",
        compiler="aarch64-linux-gnu-gcc test",
        libc="libc.so.6:" + "3" * 64,
        patch_sha256=PATCH_SHA256,
        smros_commit=SMROS_COMMIT,
        build_results_sha256=BUILD_RESULTS_SHA256,
        manifest_sha256=MANIFEST_SHA256,
    )
    results = tuple(
        _build_result(test, stage)
        for test in tests
        for stage in ("compile", "link")
    )
    return CampaignIdentity(metadata=metadata, build_id=build_id, build_results=results)


def _event(sequence: int, event: str, **values: object) -> bytes:
    payload = {
        "architecture": "aarch64",
        "event": event,
        "manifest_sha256": MANIFEST_SHA256,
        "run_id": "guest-run",
        "schema": 1,
        "seq": sequence,
        **values,
    }
    return (
        "SMROS_POSIX_EVENT "
        + json.dumps(payload, sort_keys=True, separators=(",", ":"))
        + "\n"
    ).encode("utf-8")


def _start_events(test: SuiteTest) -> bytes:
    return _event(
        1,
        "suite_start",
        selected_count=1,
        build_id=BUILD_ID,
        build_results_sha256=BUILD_RESULTS_SHA256,
        smros_commit=SMROS_COMMIT,
        revision=REVISION,
        patch_sha256=PATCH_SHA256,
        filter=f"test={test.test_id}",
        started_ticks=1,
        source="smros-serial",
    ) + _event(
        2,
        "test_start",
        test_id=test.test_id,
        group=test.group,
        api=test.api,
        binary_sha256=test.sha256,
        source="smros-serial",
        started_ticks=2,
    )


def _end_events(test: SuiteTest) -> bytes:
    return _event(
        3,
        "test_end",
        test_id=test.test_id,
        group=test.group,
        api=test.api,
        status="pass",
        pts_status="pass",
        launch_status="launched",
        exit_code=0,
        timed_out=False,
        elapsed_ticks=3,
        resource_deltas=ResourceDeltas().to_dict(),
    ) + _event(
        4,
        "suite_end",
        complete=True,
        selected_count=1,
        completed_count=1,
        status_counts={"pass": 1},
        elapsed_ticks=4,
    )


class QemuControllerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.one = _test("one")
        self.two = _test("two")
        self.tests = (self.one, self.two)
        self.clock = FakeClock()
        self.config = ControllerConfig(
            output_directory=self.root,
            qemu_argv=("qemu-system-aarch64", "-nographic"),
            boot_timeout_seconds=2.0,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _controller(
        self,
        transports: list[FakeTransport],
        *,
        identity: CampaignIdentity | None = None,
    ) -> tuple[QemuController, TransportFactory]:
        factory = TransportFactory(transports)
        controller = QemuController(
            identity=identity or _identity(self.tests),
            selected=self.tests,
            config=self.config,
            transport_factory=factory,
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-run",
        )
        return controller, factory

    def test_exact_prompt_and_matching_events_serialize_commands(self) -> None:
        transport = FakeTransport(
            self.clock,
            [
                b"booted\nsmros:/>",
                b" ",
                _start_events(self.one),
                _end_events(self.one),
                PROMPT,
                _start_events(self.two),
                _end_events(self.two),
                PROMPT,
            ],
        )
        controller, _factory = self._controller([transport])

        result = controller.run()

        self.assertEqual(
            transport.writes,
            [
                f"posixtest test {self.one.test_id}\n".encode(),
                f"posixtest test {self.two.test_id}\n".encode(),
            ],
        )
        self.assertEqual(transport.reads_at_write, [2, 5])
        self.assertEqual(
            [attempt.status for attempt in result.attempts], ["pass", "pass"]
        )
        self.assertTrue(result.complete)

    def test_mismatched_test_event_is_rejected_before_next_command(self) -> None:
        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller, _factory = self._controller([transport])

        with self.assertRaisesRegex(ControllerError, "test identity"):
            controller.run()

        self.assertEqual(len(transport.writes), 1)

    def test_only_coherent_event_prefix_proves_guest_execution(self) -> None:
        naked_start = _event(
            2,
            "test_start",
            test_id=self.one.test_id,
            group=self.one.group,
            api=self.one.api,
            binary_sha256=self.one.sha256,
            source="smros-serial",
            started_ticks=2,
        )
        wrong_manifest = _start_events(self.one).replace(
            MANIFEST_SHA256.encode("ascii"), b"9" * 64
        )
        bool_schema = _start_events(self.one).replace(
            b'"schema":1', b'"schema":true'
        )
        for label, prefix in (
            ("naked", naked_start),
            ("wrong-manifest", wrong_manifest),
            ("bool-schema", bool_schema),
        ):
            with self.subTest(label=label):
                clock = FakeClock()
                first = FakeTransport(clock, [PROMPT, prefix, b"[PANIC]\n"])
                second = FakeTransport(
                    clock,
                    [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
                )
                controller = QemuController(
                    identity=_identity(self.tests),
                    selected=self.tests,
                    config=ControllerConfig(
                        output_directory=self.root / f"prefix-{label}",
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([first, second]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                result = controller.run()

                self.assertEqual(result.attempts[0].status, "crash")
                self.assertEqual(result.attempts[0].launch_status, "interrupted")
                self.assertEqual(result.attempts[1].status, "pass")

    def test_duplicate_event_keys_are_rejected(self) -> None:
        duplicate = _start_events(self.one).replace(
            b'"schema":1,', b'"schema":1,"schema":1,', 1
        )
        transport = FakeTransport(self.clock, [PROMPT, duplicate, b"[PANIC]\n"])
        controller, _factory = self._controller([transport])

        with self.assertRaisesRegex(ControllerError, "duplicate"):
            controller.run()

    def test_split_kernel_panic_is_detected_during_boot_and_test(self) -> None:
        boot = FakeTransport(
            self.clock, [b"booting\n!!! KERNEL ", b"PANIC !!!\n"]
        )
        controller, _factory = self._controller([boot])
        with self.assertRaisesRegex(ControllerError, "exact shell prompt"):
            controller.run()
        self.assertTrue(boot.terminated)

        clock = FakeClock()
        first = FakeTransport(
            clock,
            [
                PROMPT,
                _start_events(self.one),
                b"!!! KERNEL ",
                b"PANIC !!!\n",
            ],
        )
        second = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=self.root / "split-panic",
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([first, second]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-split-panic",
        )

        result = controller.run()

        self.assertEqual(result.attempts[0].status, "crash")
        self.assertEqual(result.attempts[0].launch_status, "launched")
        self.assertIn("KERNEL PANIC", result.attempts[0].infrastructure_error)

    def test_deadline_records_host_timeout_reboots_and_continues(self) -> None:
        first = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), ("advance", 1.1)],
        )
        second = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller, factory = self._controller([first, second])

        result = controller.run()

        timeout = result.attempts[0]
        self.assertEqual(timeout.status, "timeout")
        self.assertEqual(timeout.source, "host-watchdog")
        self.assertIsNone(timeout.pts_status)
        self.assertIsNone(timeout.exit_code)
        self.assertIsNone(timeout.signal)
        self.assertTrue(timeout.timed_out)
        self.assertEqual(timeout.launch_status, "launched")
        self.assertIn("deadline", timeout.infrastructure_error)
        self.assertEqual(timeout.resource_evidence, "unavailable")
        self.assertIsNotNone(timeout.raw_log_start)
        self.assertGreater(timeout.raw_log_end, timeout.raw_log_start)
        self.assertTrue(first.terminated)
        self.assertEqual(len(factory.argv), 2)
        self.assertEqual(result.restart_count, 1)
        self.assertEqual(result.attempts[1].test_id, self.two.test_id)

    def test_fatal_pattern_and_qemu_exit_record_crash_then_restart(self) -> None:
        for label, failure in (
            ("fatal", [_start_events(self.one), b"[FATAL] kernel stopped\n"]),
            ("exit", [("exit", 17)]),
        ):
            with self.subTest(label=label):
                clock = FakeClock()
                first = FakeTransport(clock, [PROMPT, *failure])
                second = FakeTransport(
                    clock,
                    [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
                )
                factory = TransportFactory([first, second])
                controller = QemuController(
                    identity=_identity(self.tests),
                    selected=self.tests,
                    config=ControllerConfig(
                        output_directory=self.root / label,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=factory,
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                result = controller.run()

                crash = result.attempts[0]
                self.assertEqual(crash.status, "crash")
                self.assertEqual(crash.source, "host-watchdog")
                self.assertIsNone(crash.pts_status)
                self.assertIsNone(crash.signal)
                self.assertFalse(crash.timed_out)
                self.assertTrue(crash.infrastructure_error)
                self.assertEqual(
                    crash.launch_status,
                    "launched" if label == "fatal" else "interrupted",
                )
                self.assertEqual(result.restart_count, 1)
                self.assertEqual(result.attempts[1].test_id, self.two.test_id)

    def test_command_write_race_records_pre_start_crash_and_continues(self) -> None:
        first = FakeTransport(self.clock, [PROMPT])
        first.write = mock.Mock(  # type: ignore[method-assign]
            side_effect=BrokenPipeError("closed pipe")
        )
        second = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller, _factory = self._controller([first, second])

        result = controller.run()

        crash = result.attempts[0]
        self.assertEqual(crash.status, "crash")
        self.assertEqual(crash.launch_status, "interrupted")
        self.assertIn("command write", crash.infrastructure_error)
        self.assertEqual(result.attempts[1].test_id, self.two.test_id)
        self.assertEqual(result.restart_count, 1)

    def test_serial_capture_limit_records_post_start_crash_and_keeps_raw_log(
        self,
    ) -> None:
        start_events = _start_events(self.one)
        spam = b"x" * (len(_end_events(self.one)) + 64)
        first = FakeTransport(self.clock, [PROMPT, start_events, spam])
        second = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        factory = TransportFactory([first, second])
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=self.root / "bounded",
                qemu_argv=self.config.qemu_argv,
                max_test_serial_bytes=(
                    len(start_events) + len(_end_events(self.one)) + 16
                ),
            ),
            transport_factory=factory,
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-bounded",
        )

        result = controller.run()

        crash = result.attempts[0]
        self.assertEqual(crash.status, "crash")
        self.assertEqual(crash.launch_status, "launched")
        self.assertIn("serial byte limit", crash.infrastructure_error)
        self.assertIn(start_events + spam, result.raw_log_path.read_bytes())
        self.assertEqual(result.attempts[1].status, "pass")

    def test_resume_skips_completed_ids_and_rejects_changed_provenance(self) -> None:
        interrupted = FakeTransport(
            self.clock,
            [
                PROMPT,
                _start_events(self.one),
                _end_events(self.one),
                KeyboardInterrupt(),
            ],
        )
        controller, _factory = self._controller([interrupted])
        with self.assertRaises(KeyboardInterrupt):
            controller.run()
        progress = json.loads((self.root / "progress.json").read_text(encoding="utf-8"))
        self.assertEqual(
            [item["test_id"] for item in progress["completed_attempts"]],
            [self.one.test_id],
        )
        raw_bytes = (self.root / "qemu-serial.log").read_bytes()

        tampered_build = json.loads(json.dumps(progress))
        tampered_build["completed_attempts"][0]["build_id"] = "8" * 64
        (self.root / "progress.json").write_text(
            json.dumps(tampered_build, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        rejected, _factory = self._controller([])
        with self.assertRaisesRegex(ValueError, "resume completed attempt"):
            rejected.run(resume=True)

        tampered_run = json.loads(json.dumps(progress))
        tampered_run["completed_attempts"][0]["run_id"] = "different-run"
        (self.root / "progress.json").write_text(
            json.dumps(tampered_run, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        rejected, _factory = self._controller([])
        with self.assertRaisesRegex(ValueError, "resume completed attempt"):
            rejected.run(resume=True)

        (self.root / "progress.json").write_text(
            json.dumps(progress, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        (self.root / "qemu-serial.log").write_bytes(raw_bytes[:-1])
        rejected, _factory = self._controller([])
        with self.assertRaisesRegex(ValueError, "resume raw log"):
            rejected.run(resume=True)
        (self.root / "qemu-serial.log").write_bytes(raw_bytes)

        resumed_transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        resumed, _factory = self._controller([resumed_transport])
        result = resumed.run(resume=True)
        self.assertEqual(
            resumed_transport.writes,
            [f"posixtest test {self.two.test_id}\n".encode()],
        )
        self.assertEqual(len(result.attempts), 2)
        self.assertEqual(result.restart_count, 1)
        self.assertFalse((self.root / "progress.json").exists())
        terminal = json.loads(
            result.result_path.read_text(encoding="utf-8").splitlines()[-1]
        )
        self.assertEqual(terminal["boot_count"], 2)
        self.assertEqual(terminal["restart_count"], 1)

        (self.root / "progress.json").write_text(
            json.dumps(progress, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        changed, _factory = self._controller(
            [], identity=_identity(self.tests, build_id="9" * 64)
        )
        with self.assertRaisesRegex(ValueError, "build identity"):
            changed.run(resume=True)

    def test_terminal_record_and_progress_are_crash_safe_and_bounded(self) -> None:
        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=factory,
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-terminal",
        )

        result = controller.run()

        rows = [
            json.loads(line)
            for line in result.result_path.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual([row["record_type"] for row in rows], ["attempt", "run"])
        terminal = rows[-1]
        self.assertTrue(terminal["complete"])
        self.assertEqual(terminal["restart_count"], 0)
        self.assertEqual(terminal["boot_count"], 1)
        self.assertEqual(terminal["raw_log"], str(result.raw_log_path))
        self.assertEqual(terminal["manifest_sha256"], MANIFEST_SHA256)
        self.assertEqual(terminal["build_id"], BUILD_ID)
        self.assertFalse((self.root / "progress.json").exists())
        self.assertTrue(result.raw_log_path.is_file())
        self.assertFalse(tuple(self.root.glob(".*.tmp")))


class QemuIntegrationSurfaceTests(unittest.TestCase):
    def test_qemu_argv_mirrors_smoke_options_and_clamps_memory(self) -> None:
        argv = build_qemu_argv(
            qemu="qemu-system-aarch64",
            kernel=Path("kernel8.img"),
            disk=Path("smros-fxfs.img"),
            memory="512M",
        )
        self.assertEqual(argv[0], "qemu-system-aarch64")
        self.assertIn("virt,gic-version=4,virtualization=on", argv)
        self.assertIn("cortex-a710", argv)
        self.assertIn("virtio-blk-device,drive=fxfs", argv)
        self.assertIn("virtio-net-device,netdev=smrosnet", argv)
        self.assertEqual(argv[argv.index("-m") + 1], "1024M")
        self.assertNotIn("-shell", argv)

    @mock.patch("scripts.posix.qemu_runner.subprocess.Popen")
    def test_popen_transport_uses_argument_array_and_combined_serial(
        self, popen: mock.Mock
    ) -> None:
        process = popen.return_value
        process.stdin = mock.Mock()
        process.stdout = mock.Mock()
        _PopenTransport.launch(("qemu-system-aarch64", "-nographic"))
        args, kwargs = popen.call_args
        self.assertEqual(args[0], ["qemu-system-aarch64", "-nographic"])
        self.assertIs(kwargs["stderr"], subprocess.STDOUT)
        self.assertFalse(kwargs.get("shell", False))

    def test_cli_registers_exclusive_run_smros_filters(self) -> None:
        parser = create_parser()
        arguments = parser.parse_args(
            ["run-smros", "--api", "getpid", "--qemu-memory", "1024M", "--resume"]
        )
        self.assertEqual(arguments.command, "run-smros")
        self.assertEqual(arguments.api, "getpid")
        self.assertEqual(arguments.qemu_memory, "1024M")
        self.assertTrue(arguments.resume)
        with self.assertRaises(SystemExit):
            parser.parse_args(["run-smros", "--api", "getpid", "--group", "base"])

    @mock.patch("scripts.posix.cli.run_smros")
    def test_cli_dispatches_run_smros_and_reports_controller_errors(
        self, runner: mock.Mock
    ) -> None:
        runner.return_value = mock.Mock(
            attempts=(mock.Mock(status="pass"),),
            restart_count=2,
            result_path=Path("results.ndjson"),
        )
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            result = cli_module.main(
                ["run-smros", "--test", "case.c", "--qemu-memory", "2G", "--resume"]
            )
        self.assertEqual(result, 0)
        self.assertIn("selected=1", stdout.getvalue())
        self.assertEqual(runner.call_args.kwargs["test_id"], "case.c")
        self.assertEqual(runner.call_args.kwargs["memory"], "2G")
        self.assertTrue(runner.call_args.kwargs["resume"])

        runner.side_effect = ControllerError("QEMU failed")
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            result = cli_module.main(["run-smros"])
        self.assertEqual(result, 1)
        self.assertIn("run-smros failed: QEMU failed", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
