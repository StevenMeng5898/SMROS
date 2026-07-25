from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from dataclasses import replace
import errno
import io
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import tempfile
import threading
import unittest
from unittest import mock

from scripts.posix.build import (
    MAX_MANIFEST_API_BYTES,
    MAX_MANIFEST_GROUP_BYTES,
    MAX_MANIFEST_TEST_ID_BYTES,
    MAX_TESTS,
    ManifestMetadata,
)
from scripts.posix import cli as cli_module
from scripts.posix import qemu_runner as qemu_runner_module
from scripts.posix import report as report_module
from scripts.posix.cli import create_parser
from scripts.posix.model import (
    BuildResult,
    ResourceDeltas,
    SerialAttempt,
    SuiteTest,
)
from scripts.posix.qemu_runner import (
    CampaignIdentity,
    ControllerConfig,
    ControllerError,
    ControllerResult,
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


class PausingTransport(FakeTransport):
    def __init__(
        self,
        clock: FakeClock,
        script: list[object],
        entered: threading.Event,
        release: threading.Event,
        *,
        pause_before_read: int,
    ) -> None:
        super().__init__(clock, script)
        self.entered = entered
        self.release = release
        self.pause_before_read = pause_before_read

    def read(self, timeout: float) -> bytes:
        if self.reads == self.pause_before_read:
            self.entered.set()
            if not self.release.wait(5.0):
                raise AssertionError("timed out waiting to resume fake QEMU")
        return super().read(timeout)


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


def _maximum_identity_test(index: int) -> SuiteTest:
    suffix = f"{index:04d}.c"
    prefix = "conformance/"
    return replace(
        _test(f"budget-{index:04d}"),
        test_id=(
            prefix
            + "i" * (MAX_MANIFEST_TEST_ID_BYTES - len(prefix) - len(suffix))
            + suffix
        ),
        group="g" * MAX_MANIFEST_GROUP_BYTES,
        api="a" * MAX_MANIFEST_API_BYTES,
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


def _end_events(test: SuiteTest, **overrides: object) -> bytes:
    end_values: dict[str, object] = {
        "status": "pass",
        "pts_status": "pass",
        "launch_status": "launched",
        "exit_code": 0,
        "timed_out": False,
        "elapsed_ticks": 3,
        "resource_deltas": ResourceDeltas().to_dict(),
    }
    end_values.update(overrides)
    return _event(
        3,
        "test_end",
        test_id=test.test_id,
        group=test.group,
        api=test.api,
        **end_values,
    ) + _event(
        4,
        "suite_end",
        complete=True,
        selected_count=1,
        completed_count=1,
        status_counts={"pass": 1},
        elapsed_ticks=4,
    )


def _replace_event_values(
    data: bytes,
    event: str,
    *,
    remove: tuple[str, ...] = (),
    **updates: object,
) -> bytes:
    result = bytearray()
    prefix = b"SMROS_POSIX_EVENT "
    for line in data.splitlines(keepends=True):
        if not line.startswith(prefix):
            result.extend(line)
            continue
        value = json.loads(line[len(prefix) :])
        if value.get("event") == event:
            for key in remove:
                value.pop(key, None)
            value.update(updates)
        result.extend(prefix)
        result.extend(
            json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
        )
        result.extend(b"\n")
    return bytes(result)


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

    def _create_resumable_campaign(
        self, output: Path
    ) -> tuple[FakeClock, bytes, bytes]:
        clock = FakeClock()
        transport = FakeTransport(
            clock,
            [
                PROMPT,
                _start_events(self.one),
                _end_events(self.one),
                KeyboardInterrupt(),
            ],
        )
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-resume-fixture",
        )
        with self.assertRaises(KeyboardInterrupt):
            controller.run()
        return (
            clock,
            (output / "progress.json").read_bytes(),
            (output / "qemu-serial.log").read_bytes(),
        )

    def _create_postcommit_campaign(
        self,
        output: Path,
        selected: tuple[SuiteTest, ...] | None = None,
    ) -> tuple[FakeClock, bytes, bytes]:
        tests = selected or (self.one,)
        clock = FakeClock()
        script: list[object] = [PROMPT]
        for test in tests:
            script.extend((_start_events(test), _end_events(test), PROMPT))
        controller = QemuController(
            identity=_identity(tests),
            selected=tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([FakeTransport(clock, script)]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-postcommit-fixture",
        )
        real_publish = controller._publish

        def publish_then_interrupt(
            output_descriptor: int,
            marker_descriptor: int,
        ) -> None:
            real_publish(output_descriptor, marker_descriptor)
            raise KeyboardInterrupt()

        with mock.patch.object(
            controller,
            "_publish",
            side_effect=publish_then_interrupt,
        ):
            with self.assertRaises(ControllerError) as raised:
                controller.run()
        self.assertIsInstance(raised.exception.__cause__, KeyboardInterrupt)
        return (
            clock,
            (output / "results.ndjson").read_bytes(),
            (output / "progress.json").read_bytes(),
        )

    def _create_exact_partial_result(
        self, output: Path
    ) -> tuple[FakeClock, bytes, QemuController]:
        clock, _progress_bytes, _raw_bytes = self._create_resumable_campaign(
            output
        )
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            monotonic=clock.monotonic,
        )
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        raw_descriptor = os.open(
            "qemu-serial.log",
            os.O_RDONLY,
            dir_fd=output_descriptor,
        )
        try:
            with controller._load_progress(
                output_descriptor,
                os.fstat(raw_descriptor),
            ):
                pass
        finally:
            os.close(raw_descriptor)
            os.close(output_descriptor)
        partial_bytes = controller._result_bytes()
        (output / "results.ndjson").write_bytes(partial_bytes)
        return clock, partial_bytes, controller

    def _replace_and_hold(
        self,
        path: Path,
        replacement_name: str,
        replacement: bytes,
    ) -> int:
        replacement_path = path.parent / replacement_name
        replacement_path.write_bytes(replacement)
        descriptor = os.open(replacement_path, os.O_RDONLY)
        os.replace(replacement_path, path)
        return descriptor

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

    def test_malformed_event_prefix_never_proves_guest_execution(self) -> None:
        suite_start, test_start = _start_events(self.one).splitlines(keepends=True)
        terminal = _event(
            2,
            "infrastructure_error",
            message="guest infrastructure failed",
        )
        unknown = _event(2, "unknown_event")
        bool_count = suite_start.replace(
            b'"selected_count":1', b'"selected_count":true'
        )
        cases = (
            ("duplicate-suite", suite_start + suite_start + test_start),
            ("terminal-before-start", suite_start + terminal + test_start),
            ("unknown-before-start", suite_start + unknown + test_start),
            ("bool-selected-count", bool_count + test_start),
        )
        for label, prefix in cases:
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
                        output_directory=self.root / f"malformed-{label}",
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([first, second]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                if label == "terminal-before-start":
                    with self.assertRaisesRegex(ControllerError, "guest POSIX"):
                        controller.run()
                    continue

                result = controller.run()

                self.assertEqual(result.attempts[0].status, "crash")
                self.assertEqual(result.attempts[0].launch_status, "interrupted")
                self.assertEqual(result.attempts[1].status, "pass")

    def test_invalid_run_ids_never_prove_execution_on_watchdog_paths(self) -> None:
        cases = (
            ("panic-surrogate", "\ud800", "panic"),
            ("timeout-oversized", "x" * 257, "timeout"),
            ("exit-surrogate", "\ud800", "exit"),
            ("overflow-oversized", "x" * 257, "overflow"),
        )
        for label, run_id, failure in cases:
            with self.subTest(label=label):
                clock = FakeClock()
                malformed = _replace_event_values(
                    _replace_event_values(
                        _start_events(self.one),
                        "suite_start",
                        run_id=run_id,
                    ),
                    "test_start",
                    run_id=run_id,
                )
                failure_script: list[object]
                maximum = self.config.max_test_serial_bytes
                if failure == "panic":
                    failure_script = [malformed, b"[PANIC] kernel stopped\n"]
                elif failure == "timeout":
                    failure_script = [malformed, ("advance", 1.1)]
                elif failure == "exit":
                    failure_script = [malformed, ("exit", 17)]
                else:
                    successful = (
                        _start_events(self.two) + _end_events(self.two) + PROMPT
                    )
                    maximum = max(len(malformed) + 8, len(successful))
                    failure_script = [
                        malformed,
                        b"x" * (maximum - len(malformed) + 1),
                    ]
                first = FakeTransport(clock, [PROMPT, *failure_script])
                second = FakeTransport(
                    clock,
                    [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
                )
                controller = QemuController(
                    identity=_identity(self.tests),
                    selected=self.tests,
                    config=ControllerConfig(
                        output_directory=self.root / label,
                        qemu_argv=self.config.qemu_argv,
                        max_test_serial_bytes=maximum,
                    ),
                    transport_factory=TransportFactory([first, second]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda label=label: f"controller-{label}",
                )

                result = controller.run()

                failure_attempt = result.attempts[0]
                expected_status = "timeout" if failure == "timeout" else "crash"
                self.assertEqual(failure_attempt.status, expected_status)
                self.assertEqual(failure_attempt.source, "host-watchdog")
                self.assertEqual(failure_attempt.launch_status, "interrupted")
                self.assertIs(failure_attempt.timed_out, failure == "timeout")
                self.assertTrue(failure_attempt.infrastructure_error)
                self.assertEqual(result.restart_count, 1)
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

    def test_complete_guest_rejects_duplicate_or_trailing_terminal_events(self) -> None:
        trailing_start = _event(
            5,
            "test_start",
            test_id=self.one.test_id,
            group=self.one.group,
            api=self.one.api,
            binary_sha256=self.one.sha256,
            source="smros-serial",
            started_ticks=5,
        )
        duplicate_end = _event(
            5,
            "suite_end",
            complete=True,
            selected_count=1,
            completed_count=1,
            status_counts={"pass": 1},
            elapsed_ticks=5,
        )
        for label, trailing in (
            ("trailing-event", trailing_start),
            ("duplicate-terminal", duplicate_end),
        ):
            with self.subTest(label=label):
                clock = FakeClock()
                transport = FakeTransport(
                    clock,
                    [
                        PROMPT,
                        _start_events(self.one) + _end_events(self.one) + trailing,
                    ],
                )
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=self.root / label,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                with self.assertRaisesRegex(ControllerError, "terminal"):
                    controller.run()

    def test_complete_guest_requires_strict_start_event_fields(self) -> None:
        starts = _start_events(self.one)
        cases = {
            "boolean-schema": _replace_event_values(
                starts, "suite_start", schema=True
            ),
            "suite-source": _replace_event_values(
                starts, "suite_start", remove=("source",)
            ),
            "suite-timestamp": _replace_event_values(
                starts, "suite_start", remove=("started_ticks",)
            ),
            "test-source": _replace_event_values(
                starts, "test_start", remove=("source",)
            ),
            "test-timestamp": _replace_event_values(
                starts, "test_start", remove=("started_ticks",)
            ),
        }
        for label, malformed in cases.items():
            with self.subTest(label=label):
                clock = FakeClock()
                output = self.root / f"complete-start-{label}"
                transport = FakeTransport(
                    clock,
                    [PROMPT, malformed + _end_events(self.one), PROMPT],
                )
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                with self.assertRaisesRegex(ControllerError, "event|start"):
                    controller.run()

    def test_fatal_pattern_and_qemu_exit_record_crash_then_restart(self) -> None:
        for label, failure in (
            ("fatal", [_start_events(self.one), b"[PANIC] kernel stopped\n"]),
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

    def test_generic_fatal_words_are_benign_guest_output(self) -> None:
        output = b"diagnostic [FATAL], Kernel panic, and kernel panic are text\n"
        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), output, _end_events(self.one), PROMPT],
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-benign-output",
        )

        result = controller.run()

        self.assertEqual(result.attempts[0].status, "pass")
        self.assertIn("[FATAL]", result.attempts[0].stdout)

    def test_persisted_guest_output_remains_report_compatible(self) -> None:
        stdout = b"x" * (600 * 1024) + b"\n"
        stderr = "y" * (600 * 1024)
        transport = FakeTransport(
            self.clock,
            [
                PROMPT,
                _start_events(self.one),
                stdout,
                _end_events(self.one, stderr=stderr),
                PROMPT,
            ],
        )
        identity = _identity((self.one,))
        controller = QemuController(
            identity=identity,
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-large-output",
        )

        result = controller.run()

        manifest = report_module._ManifestInput(
            identity.metadata,
            (self.one,),
            identity.build_results,
            (),
        )
        with mock.patch.object(report_module, "_load_manifest", return_value=manifest):
            summary = report_module.generate_report(
                Path("manifest.json"),
                smros_results=(result.result_path,),
                output_directory=self.root / "large-output-report",
            )
        attempt = result.attempts[0]
        self.assertEqual(attempt.stdout_bytes, len(stdout))
        self.assertEqual(attempt.stderr_bytes, len(stderr.encode("utf-8")))
        self.assertTrue(attempt.stdout_truncated)
        self.assertTrue(attempt.stderr_truncated)
        self.assertTrue(attempt.stdout.endswith("\n...[truncated]"))
        self.assertTrue(attempt.stderr.endswith("\n...[truncated]"))
        raw = result.raw_log_path.read_bytes()
        self.assertIn(stdout, raw)
        self.assertIn(stderr.encode("utf-8"), raw)
        self.assertLess(
            len(result.result_path.read_bytes().splitlines()[0]), 512 * 1024
        )
        self.assertTrue(summary["complete"])

    def test_maximum_campaign_escaping_stays_within_runtime_caps(self) -> None:
        tests = tuple(_maximum_identity_test(index) for index in range(MAX_TESTS))
        baseline_identity = _identity((tests[0],))
        identity = CampaignIdentity(
            metadata=baseline_identity.metadata,
            build_id=baseline_identity.build_id,
            build_results=(),
        )
        output = self.root / "maximum-campaign-budget"
        controller = QemuController(
            identity=identity,
            selected=tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        controller._run_id = "\0" * 256
        guest = SerialAttempt(
            test_id=tests[0].test_id,
            group=tests[0].group,
            api=tests[0].api,
            status="launch-error",
            pts_status=None,
            launch_status="launch-error",
            exit_code=None,
            signal=None,
            timed_out=False,
            duration_ms=1,
            stdout="\0" * (600 * 1024),
            stderr="\0" * (600 * 1024),
            resource_deltas=ResourceDeltas(),
            resource_evidence="measured",
            run_id="guest-run",
            manifest_sha256=identity.metadata.manifest_sha256,
            architecture="aarch64",
            launch_error="\0" * (600 * 1024),
            infrastructure_error="\0" * (600 * 1024),
        )
        prototype = controller._guest_attempt(
            guest,
            tests[0],
            raw_log_start=0,
            raw_log_end=0,
        )
        prototype_line = qemu_runner_module._json_bytes(
            qemu_runner_module._attempt_record(prototype)
        )
        projected_attempt_bytes = len(prototype_line) * MAX_TESTS
        with self.subTest(check="projected attempt budget"):
            self.assertLessEqual(
                projected_attempt_bytes,
                qemu_runner_module._PERSISTED_ATTEMPTS_BUDGET,
            )

        controller._attempts = [
            replace(
                prototype,
                test_id=test.test_id,
                group=test.group,
                api=test.api,
                binary_sha256=test.sha256 or "0" * 64,
            )
            for test in tests
        ]
        controller._boot_count = 1
        output.mkdir()
        (output / "qemu-serial.log").write_bytes(b"")
        controller._persist_progress()
        publication_parent = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        marker = controller._bind_result_marker(publication_parent)
        try:
            controller._publish(publication_parent, marker)
        finally:
            os.close(marker)
            os.close(publication_parent)
        progress_size = (output / "progress.json").stat().st_size
        results_size = (output / "results.ndjson").stat().st_size
        self.assertLessEqual(
            progress_size,
            qemu_runner_module._MAX_PROGRESS_BYTES,
        )
        self.assertLessEqual(
            results_size,
            report_module._MAX_RUNTIME_RESULTS_BYTES,
        )

        resumed = QemuController(
            identity=identity,
            selected=tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            with resumed._load_progress(
                output_descriptor,
                (output / "qemu-serial.log").stat(),
            ):
                pass
        finally:
            os.close(output_descriptor)
        self.assertEqual(len(resumed._attempts), MAX_TESTS)

        runtime = report_module._load_runtime_results(
            output / "results.ndjson",
            tests,
            (),
            identity.metadata,
            role="smros",
        )
        self.assertEqual(len(runtime.attempts), MAX_TESTS)

    def test_resume_rejects_errors_outside_maximum_campaign_budget(self) -> None:
        tests = tuple(
            _test(f"resume-budget-{index:04d}") for index in range(MAX_TESTS)
        )
        baseline_identity = _identity((tests[0],))
        identity = CampaignIdentity(
            metadata=baseline_identity.metadata,
            build_id=baseline_identity.build_id,
            build_results=(),
        )
        output = self.root / "resume-maximum-budget"
        output.mkdir()
        raw = output / "qemu-serial.log"
        raw.write_bytes(b"")
        controller = QemuController(
            identity=identity,
            selected=tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        controller._run_id = "controller-resume-maximum-budget"
        controller._boot_count = 1
        guest = SerialAttempt(
            test_id=tests[0].test_id,
            group=tests[0].group,
            api=tests[0].api,
            status="launch-error",
            pts_status=None,
            launch_status="launch-error",
            exit_code=None,
            signal=None,
            timed_out=False,
            duration_ms=1,
            stdout="",
            stderr="",
            resource_deltas=ResourceDeltas(),
            resource_evidence="measured",
            run_id="guest-run",
            manifest_sha256=identity.metadata.manifest_sha256,
            architecture="aarch64",
            launch_error="failure",
        )
        attempt = controller._guest_attempt(
            guest,
            tests[0],
            raw_log_start=0,
            raw_log_end=0,
        )
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        resumed = QemuController(
            identity=identity,
            selected=tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        try:
            error_limit = qemu_runner_module._persisted_attempt_field_limits(
                MAX_TESTS
            ).error_bytes
            invalid_attempts = {
                "launch-error": replace(
                    attempt,
                    launch_error="x" * (error_limit + 1),
                ),
                "infrastructure-error": replace(
                    attempt,
                    infrastructure_error="y" * (error_limit + 1),
                ),
            }
            for label, invalid_attempt in invalid_attempts.items():
                with self.subTest(label=label):
                    controller._attempts = [invalid_attempt]
                    controller._persist_progress()
                    with self.assertRaisesRegex(
                        ValueError,
                        "resume completed attempt identity",
                    ):
                        resumed._load_progress(output_descriptor, raw.stat())
        finally:
            os.close(output_descriptor)

    def test_persisted_error_validation_rejects_lone_surrogates(self) -> None:
        error_limit = qemu_runner_module._persisted_attempt_field_limits(
            MAX_TESTS
        ).error_bytes
        for label, errors in {
            "launch_error": ("\ud800", None),
            "infrastructure_error": (None, "\ud800"),
        }.items():
            with self.subTest(field=label):
                self.assertFalse(
                    qemu_runner_module._persisted_errors_are_valid(
                        *errors,
                        error_limit,
                    )
                )

    def test_maximum_campaign_watchdog_uses_guest_error_budget(self) -> None:
        tests = tuple(
            _test(f"watchdog-budget-{index:04d}") for index in range(MAX_TESTS)
        )
        baseline_identity = _identity((tests[0],))
        identity = CampaignIdentity(
            metadata=baseline_identity.metadata,
            build_id=baseline_identity.build_id,
            build_results=(),
        )
        controller = QemuController(
            identity=identity,
            selected=tests,
            config=ControllerConfig(
                output_directory=self.root / "watchdog-maximum-budget",
                qemu_argv=self.config.qemu_argv,
            ),
        )
        controller._run_id = "controller-watchdog-maximum-budget"
        reason = "\0" * (600 * 1024)
        guest = SerialAttempt(
            test_id=tests[0].test_id,
            group=tests[0].group,
            api=tests[0].api,
            status="launch-error",
            pts_status=None,
            launch_status="launch-error",
            exit_code=None,
            signal=None,
            timed_out=False,
            duration_ms=1,
            stdout="",
            stderr="",
            resource_deltas=ResourceDeltas(),
            resource_evidence="measured",
            run_id="guest-run",
            manifest_sha256=identity.metadata.manifest_sha256,
            architecture="aarch64",
            launch_error=reason,
        )
        guest_attempt = controller._guest_attempt(
            guest,
            tests[0],
            raw_log_start=0,
            raw_log_end=0,
        )
        watchdog_attempt = controller._watchdog_attempt(
            tests[0],
            status="crash",
            started=True,
            timed_out=False,
            reason=reason,
            duration_ms=1,
            raw_log_start=0,
            raw_log_end=0,
        )

        self.assertEqual(
            len((watchdog_attempt.infrastructure_error or "").encode("utf-8")),
            len((guest_attempt.launch_error or "").encode("utf-8")),
        )

    def test_fresh_raw_log_links_do_not_clobber_outside_files(self) -> None:
        for label in ("symlink", "hardlink"):
            with self.subTest(label=label):
                output = self.root / f"fresh-raw-{label}"
                output.mkdir()
                sentinel = self.root / f"outside-{label}.log"
                sentinel.write_bytes(b"outside sentinel\n")
                raw = output / "qemu-serial.log"
                if label == "symlink":
                    raw.symlink_to(sentinel)
                else:
                    os.link(sentinel, raw)
                transport = FakeTransport(
                    self.clock,
                    [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                )
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=self.clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                error: ControllerError | None = None
                try:
                    controller.run()
                except ControllerError as observed:
                    error = observed
                self.assertEqual(sentinel.read_bytes(), b"outside sentinel\n")
                self.assertIsNotNone(error)
                self.assertRegex(str(error), "raw log")

    def test_raw_log_open_cannot_block_on_a_fifo(self) -> None:
        output = self.root / "raw-fifo"
        output.mkdir()
        os.mkfifo(output / "qemu-serial.log")
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        real_open = os.open
        observed_flags: list[int] = []

        def guarded_open(path, flags, *args, **kwargs):
            observed_flags.append(flags)
            if not flags & os.O_NONBLOCK:
                raise BlockingIOError("raw FIFO open would block")
            return real_open(path, flags, *args, **kwargs)

        try:
            with mock.patch(
                "scripts.posix.qemu_runner.os.open", side_effect=guarded_open
            ):
                with self.assertRaisesRegex(ControllerError, "raw log"):
                    controller._open_raw_log(output_descriptor, resume=False)
        finally:
            os.close(output_descriptor)
        self.assertTrue(observed_flags[0] & os.O_NONBLOCK)

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

    def test_serial_capture_never_retains_more_than_configured_cap(self) -> None:
        retained_sizes: list[int] = []

        def observe(data: bytes, *_args: object) -> bool:
            retained_sizes.append(len(data))
            return False

        cap = 32
        transport = FakeTransport(self.clock, [PROMPT, b"x" * 128])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=self.root / "exact-cap",
                qemu_argv=self.config.qemu_argv,
                max_test_serial_bytes=cap,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-exact-cap",
        )

        with mock.patch(
            "scripts.posix.qemu_runner._matching_start_seen",
            side_effect=observe,
        ):
            result = controller.run()

        self.assertEqual(result.attempts[0].status, "crash")
        self.assertEqual(max(retained_sizes), cap)

    def test_split_post_suite_prompt_allows_next_command(self) -> None:
        transport = FakeTransport(
            self.clock,
            [
                PROMPT,
                _start_events(self.one),
                _end_events(self.one) + b"smros:/",
                b"> ",
                _start_events(self.two),
                _end_events(self.two),
                PROMPT,
            ],
        )
        controller, _factory = self._controller([transport])

        result = controller.run()

        self.assertEqual(
            [attempt.status for attempt in result.attempts], ["pass", "pass"]
        )
        self.assertEqual(len(transport.writes), 2)

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

        coherent_interrupted = json.loads(json.dumps(progress))
        completed = coherent_interrupted["completed_attempts"][0]
        completed.update(
            {
                "status": "interrupted",
                "launch_status": "interrupted",
                "pts_status": None,
                "exit_code": None,
                "signal": None,
                "timed_out": False,
                "launch_error": None,
                "infrastructure_error": "runtime capture interrupted",
            }
        )
        (self.root / "progress.json").write_text(
            json.dumps(
                coherent_interrupted, sort_keys=True, separators=(",", ":")
            )
            + "\n",
            encoding="utf-8",
        )
        unexpected_resume = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        rejected, _factory = self._controller([unexpected_resume])
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

    def test_resume_rejects_inconsistent_boot_and_restart_counts(self) -> None:
        output = self.root / "resume-counter-consistency"
        _clock, progress_bytes, _raw_bytes = self._create_resumable_campaign(output)
        progress_path = output / "progress.json"
        raw_path = output / "qemu-serial.log"
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            for boot_count, restart_count in ((0, 1), (1, 1), (3, 1)):
                with self.subTest(
                    boot_count=boot_count,
                    restart_count=restart_count,
                ):
                    progress = json.loads(progress_bytes)
                    progress["boot_count"] = boot_count
                    progress["restart_count"] = restart_count
                    progress_path.write_text(
                        json.dumps(
                            progress,
                            sort_keys=True,
                            separators=(",", ":"),
                        )
                        + "\n",
                        encoding="utf-8",
                    )
                    controller = QemuController(
                        identity=_identity(self.tests),
                        selected=self.tests,
                        config=ControllerConfig(
                            output_directory=output,
                            qemu_argv=self.config.qemu_argv,
                        ),
                    )
                    with self.assertRaisesRegex(ValueError, "resume progress"):
                        controller._load_progress(
                            output_descriptor,
                            raw_path.stat(),
                        )
        finally:
            os.close(output_descriptor)

    def test_resume_zero_boot_allows_only_an_empty_checkpoint(self) -> None:
        output = self.root / "resume-zero-boot-consistency"
        _clock, progress_bytes, _raw_bytes = self._create_resumable_campaign(output)
        progress_path = output / "progress.json"
        raw_path = output / "qemu-serial.log"
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )

        def write_progress(progress: dict[str, object]) -> None:
            progress_path.write_text(
                json.dumps(progress, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )

        try:
            empty = json.loads(progress_bytes)
            empty.update(
                {
                    "boot_count": 0,
                    "restart_count": 0,
                    "completed_attempts": [],
                    "current_test": None,
                    "infrastructure_error": None,
                }
            )
            write_progress(empty)
            accepted = QemuController(
                identity=_identity(self.tests),
                selected=self.tests,
                config=ControllerConfig(
                    output_directory=output,
                    qemu_argv=self.config.qemu_argv,
                ),
            )
            with accepted._load_progress(output_descriptor, raw_path.stat()):
                pass
            self.assertEqual(accepted._boot_count, 0)
            self.assertEqual(accepted._restart_count, 0)
            self.assertEqual(accepted._attempts, [])
            self.assertIsNone(accepted._current_test)
            self.assertIsNone(accepted._infrastructure_error)

            completed = json.loads(progress_bytes)
            completed["boot_count"] = 0
            completed["restart_count"] = 0
            corruptions = {
                "completed-attempt": completed,
                "current-test": {
                    **empty,
                    "current_test": self.one.test_id,
                },
                "infrastructure-error": {
                    **empty,
                    "infrastructure_error": "zero-boot terminal evidence",
                },
            }
            for label, progress in corruptions.items():
                with self.subTest(case=label):
                    write_progress(progress)
                    rejected = QemuController(
                        identity=_identity(self.tests),
                        selected=self.tests,
                        config=ControllerConfig(
                            output_directory=output,
                            qemu_argv=self.config.qemu_argv,
                        ),
                    )
                    with self.assertRaisesRegex(ValueError, "resume progress"):
                        rejected._load_progress(
                            output_descriptor,
                            raw_path.stat(),
                        )
        finally:
            os.close(output_descriptor)

    def test_resume_current_test_matches_first_incomplete_selection(self) -> None:
        output = self.root / "resume-current-test-consistency"
        _clock, progress_bytes, _raw_bytes = self._create_resumable_campaign(output)
        progress_path = output / "progress.json"
        raw_path = output / "qemu-serial.log"
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            valid = json.loads(progress_bytes)
            valid["current_test"] = self.two.test_id
            progress_path.write_text(
                json.dumps(valid, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            accepted = QemuController(
                identity=_identity(self.tests),
                selected=self.tests,
                config=ControllerConfig(
                    output_directory=output,
                    qemu_argv=self.config.qemu_argv,
                ),
            )
            with accepted._load_progress(output_descriptor, raw_path.stat()):
                pass
            self.assertEqual(
                [attempt.test_id for attempt in accepted._attempts],
                [self.one.test_id],
            )
            self.assertIsNone(accepted._current_test)

            corruptions = {
                "already-completed": {
                    **json.loads(progress_bytes),
                    "current_test": self.one.test_id,
                },
                "skips-first-pending": {
                    **json.loads(progress_bytes),
                    "completed_attempts": [],
                    "current_test": self.two.test_id,
                },
            }
            for label, progress in corruptions.items():
                with self.subTest(case=label):
                    progress_path.write_text(
                        json.dumps(
                            progress,
                            sort_keys=True,
                            separators=(",", ":"),
                        )
                        + "\n",
                        encoding="utf-8",
                    )
                    rejected = QemuController(
                        identity=_identity(self.tests),
                        selected=self.tests,
                        config=ControllerConfig(
                            output_directory=output,
                            qemu_argv=self.config.qemu_argv,
                        ),
                    )
                    with self.assertRaisesRegex(ValueError, "resume progress"):
                        rejected._load_progress(
                            output_descriptor,
                            raw_path.stat(),
                        )
        finally:
            os.close(output_descriptor)

    def test_resume_raw_log_swap_cannot_append_outside(self) -> None:
        output = self.root / "resume-raw-swap"
        clock, _progress, raw_bytes = self._create_resumable_campaign(output)
        raw = output / "qemu-serial.log"
        regular_info = raw.stat()
        raw.unlink()
        sentinel = self.root / "outside-resume.log"
        sentinel.write_bytes(b"outside sentinel\n")
        raw.symlink_to(sentinel)
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-resume-run-id",
        )

        error: ControllerError | None = None
        with mock.patch.object(Path, "lstat", return_value=regular_info):
            try:
                controller.run(resume=True)
            except ControllerError as observed:
                error = observed

        self.assertEqual(sentinel.read_bytes(), b"outside sentinel\n")
        self.assertTrue(raw.is_symlink())
        self.assertTrue(raw_bytes)
        self.assertIsNotNone(error)
        self.assertRegex(str(error), "raw log")

    def test_resume_progress_is_bounded_and_canonical(self) -> None:
        output = self.root / "resume-progress-safety"
        clock, progress_bytes, raw_bytes = self._create_resumable_campaign(output)
        progress = output / "progress.json"
        raw = output / "qemu-serial.log"
        parsed = json.loads(progress_bytes)
        corruptions = {
            "duplicate": progress_bytes.replace(
                b'"boot_count":1', b'"boot_count":1,"boot_count":1', 1
            ),
            "noncanonical": (
                json.dumps(parsed, indent=2, sort_keys=True) + "\n"
            ).encode("utf-8"),
            "non-lf": progress_bytes.rstrip(b"\n"),
        }
        for label, data in corruptions.items():
            with self.subTest(label=label):
                raw.write_bytes(raw_bytes)
                progress.write_bytes(data)
                transport = FakeTransport(
                    clock,
                    [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
                )
                controller = QemuController(
                    identity=_identity(self.tests),
                    selected=self.tests,
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: "unused-corrupt-progress",
                )
                with self.assertRaisesRegex(ValueError, "progress"):
                    controller.run(resume=True)

        raw.write_bytes(raw_bytes)
        progress.write_bytes(progress_bytes)
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-oversized-progress",
        )
        with mock.patch(
            "scripts.posix.qemu_runner._MAX_PROGRESS_BYTES",
            len(progress_bytes) - 1,
            create=True,
        ):
            with self.assertRaisesRegex(ValueError, "size limit"):
                controller.run(resume=True)

    def test_resume_rejects_canonical_lone_surrogates_in_attempt_text(self) -> None:
        output = self.root / "resume-progress-surrogates"
        _clock, progress_bytes, _raw_bytes = self._create_resumable_campaign(output)
        progress_path = output / "progress.json"
        raw_path = output / "qemu-serial.log"
        launch_error_dimensions = {
            "status": "launch-error",
            "launch_status": "launch-error",
            "pts_status": None,
            "exit_code": None,
            "signal": None,
            "timed_out": False,
        }
        corruptions = {
            "stdout": {"stdout": "\ud800", "stdout_bytes": 1},
            "stderr": {"stderr": "\ud800", "stderr_bytes": 1},
            "launch_error": {
                **launch_error_dimensions,
                "launch_error": "\ud800",
                "infrastructure_error": None,
            },
            "infrastructure_error": {
                **launch_error_dimensions,
                "launch_error": "launch failed",
                "infrastructure_error": "\ud800",
            },
        }
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            for label, changes in corruptions.items():
                with self.subTest(field=label):
                    tampered = json.loads(progress_bytes)
                    tampered["completed_attempts"][0].update(changes)
                    progress_path.write_bytes(
                        (
                            json.dumps(
                                tampered,
                                ensure_ascii=True,
                                sort_keys=True,
                                separators=(",", ":"),
                            )
                            + "\n"
                        ).encode("ascii")
                    )
                    controller = QemuController(
                        identity=_identity(self.tests),
                        selected=self.tests,
                        config=ControllerConfig(
                            output_directory=output,
                            qemu_argv=self.config.qemu_argv,
                        ),
                    )
                    with self.assertRaisesRegex(
                        ValueError,
                        "resume completed attempt",
                    ) as raised:
                        controller._load_progress(
                            output_descriptor,
                            raw_path.stat(),
                        )
                    self.assertIs(type(raised.exception), ValueError)
        finally:
            os.close(output_descriptor)

    def test_resume_rejects_invalid_matching_run_ids(self) -> None:
        output = self.root / "resume-progress-run-ids"
        _clock, progress_bytes, _raw_bytes = self._create_resumable_campaign(output)
        progress_path = output / "progress.json"
        raw_path = output / "qemu-serial.log"
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            for label, run_id in (
                ("lone-surrogate", "\ud800"),
                ("over-limit", "r" * 257),
            ):
                with self.subTest(case=label):
                    tampered = json.loads(progress_bytes)
                    tampered["run_id"] = run_id
                    tampered["completed_attempts"][0]["run_id"] = run_id
                    progress_path.write_bytes(
                        (
                            json.dumps(
                                tampered,
                                ensure_ascii=True,
                                sort_keys=True,
                                separators=(",", ":"),
                            )
                            + "\n"
                        ).encode("ascii")
                    )
                    controller = QemuController(
                        identity=_identity(self.tests),
                        selected=self.tests,
                        config=ControllerConfig(
                            output_directory=output,
                            qemu_argv=self.config.qemu_argv,
                        ),
                    )
                    with self.assertRaisesRegex(ValueError, "resume progress") as raised:
                        controller._load_progress(
                            output_descriptor,
                            raw_path.stat(),
                        )
                    self.assertIs(type(raised.exception), ValueError)
        finally:
            os.close(output_descriptor)

    def test_resume_progress_fifo_is_rejected_promptly(self) -> None:
        output = self.root / "resume-progress-fifo"
        output.mkdir()
        os.mkfifo(output / "progress.json")
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        real_open = os.open
        observed_flags: list[int] = []

        def guarded_open(path, flags, *args, **kwargs):
            observed_flags.append(flags)
            if not flags & os.O_NONBLOCK:
                raise BlockingIOError("progress FIFO open would block")
            return real_open(path, flags, *args, **kwargs)

        try:
            with mock.patch(
                "scripts.posix.qemu_runner.os.open",
                side_effect=guarded_open,
            ):
                with self.assertRaisesRegex(ValueError, "progress"):
                    controller._read_progress(output_descriptor)
        finally:
            os.close(output_descriptor)
        self.assertTrue(observed_flags[0] & os.O_NONBLOCK)

    def test_resume_progress_rejects_device_and_socket_files(self) -> None:
        output = self.root / "resume-progress-special"
        output.mkdir()
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )

        device_descriptor = os.open("/dev/null", os.O_RDONLY)
        with mock.patch(
            "scripts.posix.qemu_runner.os.open",
            return_value=device_descriptor,
        ):
            with self.assertRaisesRegex(ValueError, "regular single-link"):
                controller._read_progress(output_descriptor)

        progress = output / "progress.json"
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            listener.bind(str(progress))
            with self.assertRaisesRegex(ValueError, "progress"):
                controller._read_progress(output_descriptor)
        finally:
            listener.close()
            os.close(output_descriptor)

    def test_resume_progress_normalizes_open_and_read_errors(self) -> None:
        output = self.root / "resume-progress-errors"
        output.mkdir()
        (output / "progress.json").write_bytes(b"{}\n")
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )

        try:
            for operation in ("open", "read"):
                with self.subTest(operation=operation):
                    target = f"scripts.posix.qemu_runner.os.{operation}"
                    with mock.patch(target, side_effect=OSError(f"{operation} failed")):
                        observed: BaseException | None = None
                        try:
                            controller._read_progress(output_descriptor)
                        except Exception as error:
                            observed = error
                    self.assertIs(type(observed), ValueError)
                    self.assertRegex(str(observed), "resume progress is unavailable")
        finally:
            os.close(output_descriptor)

    def test_resume_progress_symlink_is_not_followed(self) -> None:
        output = self.root / "resume-progress-symlink"
        clock, progress_bytes, raw_bytes = self._create_resumable_campaign(output)
        progress = output / "progress.json"
        raw = output / "qemu-serial.log"
        raw.write_bytes(raw_bytes)
        sentinel = self.root / "outside-progress.json"
        sentinel.write_bytes(progress_bytes)
        try:
            progress.unlink()
        except FileNotFoundError:
            pass
        progress.symlink_to(sentinel)
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-symlink-progress",
        )
        with mock.patch.object(controller, "_persist_progress"):
            with self.assertRaisesRegex(ValueError, "progress"):
                controller.run(resume=True)
        self.assertEqual(sentinel.read_bytes(), progress_bytes)

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

    def test_progress_unlink_is_durable_before_success_is_reported(self) -> None:
        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-durable-progress-unlink",
        )
        output_identity = (self.root.stat().st_dev, self.root.stat().st_ino)
        events: list[str] = []
        real_publish = controller._publish
        real_unlink = qemu_runner_module.os.unlink
        real_fsync = qemu_runner_module.os.fsync

        def traced_publish(output_descriptor: int, marker_descriptor: int) -> None:
            real_publish(output_descriptor, marker_descriptor)
            events.append("result-published")

        def traced_unlink(path, *args, **kwargs) -> None:
            real_unlink(path, *args, **kwargs)
            if os.fspath(path) == "progress.json":
                events.append("progress-unlinked")

        def traced_fsync(descriptor: int) -> None:
            real_fsync(descriptor)
            info = os.fstat(descriptor)
            if (info.st_dev, info.st_ino) == output_identity:
                events.append("output-fsynced")

        with (
            mock.patch.object(controller, "_publish", side_effect=traced_publish),
            mock.patch.object(
                qemu_runner_module.os,
                "unlink",
                side_effect=traced_unlink,
            ),
            mock.patch.object(
                qemu_runner_module.os,
                "fsync",
                side_effect=traced_fsync,
            ),
        ):
            result = controller.run()
            events.append("success-reported")

        self.assertEqual(
            events[-4:],
            [
                "result-published",
                "progress-unlinked",
                "output-fsynced",
                "success-reported",
            ],
        )
        self.assertTrue(result.result_path.is_file())
        self.assertFalse((self.root / "progress.json").exists())

    def test_progress_unlink_fsync_failure_preserves_truthful_state(self) -> None:
        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-progress-unlink-fsync-failure",
        )
        output_identity = (self.root.stat().st_dev, self.root.stat().st_ino)
        progress_unlinked = False
        injected = OSError(errno.EIO, "progress unlink fsync failed")
        real_unlink = qemu_runner_module.os.unlink
        real_fsync = qemu_runner_module.os.fsync

        def tracked_unlink(path, *args, **kwargs) -> None:
            nonlocal progress_unlinked
            real_unlink(path, *args, **kwargs)
            if os.fspath(path) == "progress.json":
                progress_unlinked = True

        def fail_progress_unlink_fsync(descriptor: int) -> None:
            info = os.fstat(descriptor)
            if progress_unlinked and (info.st_dev, info.st_ino) == output_identity:
                raise injected
            real_fsync(descriptor)

        with (
            mock.patch.object(
                qemu_runner_module.os,
                "unlink",
                side_effect=tracked_unlink,
            ),
            mock.patch.object(
                qemu_runner_module.os,
                "fsync",
                side_effect=fail_progress_unlink_fsync,
            ),
        ):
            with self.assertRaises(ControllerError) as raised:
                controller.run()

        self.assertIs(type(raised.exception), ControllerError)
        self.assertIs(raised.exception.__cause__, injected)
        self.assertTrue((self.root / "results.ndjson").is_file())
        self.assertFalse((self.root / "progress.json").exists())

    def test_resume_finalizes_exact_postcommit_result_without_launch(self) -> None:
        output = self.root / "resume-postcommit-result"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        self.assertTrue(result_bytes)
        self.assertTrue(progress_bytes)
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-postcommit-resume",
        )
        results: list[ControllerResult] = []
        errors: list[BaseException] = []

        try:
            results.append(controller.run(resume=True))
        except BaseException as error:
            errors.append(error)

        self.assertEqual(errors, [])
        self.assertEqual(len(results), 1)
        self.assertTrue(results[0].complete)
        self.assertEqual(len(results[0].attempts), 1)
        self.assertEqual(results[0].attempts[0].test_id, self.one.test_id)
        self.assertEqual((output / "results.ndjson").read_bytes(), result_bytes)
        self.assertFalse((output / "progress.json").exists())
        self.assertEqual(factory.argv, [])

    def test_partial_no_infrastructure_checkpoint_is_not_complete(self) -> None:
        output = self.root / "resume-partial-completion"
        _clock, _partial_bytes, controller = self._create_exact_partial_result(
            output
        )

        self.assertFalse(controller._complete())
        self.assertFalse(controller._terminal()["complete"])

    def test_exact_inactive_partial_result_resumes_remaining_test(self) -> None:
        output = self.root / "resume-partial-result"
        clock, partial_bytes, _controller = self._create_exact_partial_result(
            output
        )
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-partial-resume",
        )

        result = controller.run(resume=True)

        self.assertNotEqual(result.result_path.read_bytes(), partial_bytes)
        self.assertTrue(result.complete)
        self.assertEqual(
            [attempt.test_id for attempt in result.attempts],
            [self.one.test_id, self.two.test_id],
        )
        self.assertEqual(
            transport.writes,
            [f"posixtest test {self.two.test_id}\n".encode()],
        )
        self.assertEqual(len(factory.argv), 1)
        terminal = json.loads(
            result.result_path.read_text(encoding="utf-8").splitlines()[-1]
        )
        self.assertTrue(terminal["complete"])
        self.assertEqual(terminal["completed_count"], 2)
        self.assertEqual(terminal["selected_count"], 2)
        self.assertFalse((output / "progress.json").exists())

    def test_terminal_resume_rejects_result_replacement_after_validation(
        self,
    ) -> None:
        output = self.root / "resume-terminal-result-race"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        replacement = b"terminal result replacement\n"
        replacement_descriptor: int | None = None
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_validation = controller._resume_result_is_committed

        def replace_after_validation(*args, **kwargs):
            nonlocal replacement_descriptor
            committed = real_validation(*args, **kwargs)
            replacement_descriptor = self._replace_and_hold(
                output / "results.ndjson",
                "terminal-result-replacement",
                replacement,
            )
            return committed

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                controller,
                "_resume_result_is_committed",
                side_effect=replace_after_validation,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                os.pread(replacement_descriptor, len(replacement), 0),
                replacement,
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), replacement
            )
            self.assertTrue((output / "progress.json").is_file())
            self.assertEqual(
                (output / "progress.json").read_bytes(), progress_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
            self.assertNotEqual(replacement, result_bytes)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_terminal_resume_restores_progress_when_result_changes_during_retirement(
        self,
    ) -> None:
        output = self.root / "resume-terminal-retirement-result-race"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        replacement = b"retirement result replacement\n"
        replacement_descriptor: int | None = None
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_rename = qemu_runner_module._rename_noreplace_between

        def replace_after_progress_move(
            source_parent: int,
            source_name: str,
            destination_parent: int,
            destination_name: str,
        ) -> None:
            nonlocal replacement_descriptor
            real_rename(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
            )
            if source_name == "progress.json":
                replacement_descriptor = self._replace_and_hold(
                    output / "results.ndjson",
                    "retirement-result-replacement",
                    replacement,
                )

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                qemu_runner_module,
                "_rename_noreplace_between",
                side_effect=replace_after_progress_move,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                os.pread(replacement_descriptor, len(replacement), 0),
                replacement,
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), replacement
            )
            self.assertTrue((output / "progress.json").is_file())
            self.assertEqual(
                (output / "progress.json").read_bytes(), progress_bytes
            )
            self.assertFalse((output / ".progress.json.retiring").exists())
            self.assertEqual(factory.argv, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
            self.assertNotEqual(replacement, result_bytes)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_terminal_resume_retains_recoverable_progress_when_restore_is_blocked(
        self,
    ) -> None:
        output = self.root / "resume-terminal-retirement-restore-blocked"
        clock, _result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        result_replacement = b"blocked restore result replacement\n"
        progress_blocker = b"blocked restore progress replacement\n"
        result_descriptor: int | None = None
        progress_descriptor: int | None = None
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_rename = qemu_runner_module._rename_noreplace_between

        def replace_after_progress_move(
            source_parent: int,
            source_name: str,
            destination_parent: int,
            destination_name: str,
        ) -> None:
            nonlocal result_descriptor, progress_descriptor
            real_rename(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
            )
            if source_name == "progress.json":
                result_descriptor = self._replace_and_hold(
                    output / "results.ndjson",
                    "blocked-restore-result-replacement",
                    result_replacement,
                )
                progress_descriptor = self._replace_and_hold(
                    output / "progress.json",
                    "blocked-restore-progress-replacement",
                    progress_blocker,
                )

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                qemu_runner_module,
                "_rename_noreplace_between",
                side_effect=replace_after_progress_move,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(result_descriptor)
            self.assertIsNotNone(progress_descriptor)
            assert result_descriptor is not None
            assert progress_descriptor is not None
            self.assertEqual(os.fstat(result_descriptor).st_nlink, 1)
            self.assertEqual(os.fstat(progress_descriptor).st_nlink, 1)
            self.assertEqual(
                (output / "results.ndjson").read_bytes(),
                result_replacement,
            )
            self.assertEqual(
                (output / "progress.json").read_bytes(), progress_blocker
            )
            retained = output / ".progress.json.retiring"
            self.assertTrue(retained.is_file())
            self.assertEqual(retained.read_bytes(), progress_bytes)
            self.assertEqual(factory.argv, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            if progress_descriptor is not None:
                os.close(progress_descriptor)
            if result_descriptor is not None:
                os.close(result_descriptor)

    def test_terminal_resume_recovers_interrupted_progress_retirement(self) -> None:
        output = self.root / "resume-terminal-interrupted-retirement"
        clock, result_bytes, _progress_bytes = self._create_postcommit_campaign(
            output
        )
        os.replace(
            output / "progress.json",
            output / ".progress.json.retiring",
        )
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )

        errors: list[BaseException] = []
        result: ControllerResult | None = None
        try:
            result = controller.run(resume=True)
        except BaseException as error:
            errors.append(error)

        self.assertEqual(factory.argv, [])
        self.assertEqual(
            (output / "results.ndjson").read_bytes(), result_bytes
        )
        self.assertEqual(errors, [])
        self.assertIsNotNone(result)
        assert result is not None
        self.assertTrue(result.complete)
        self.assertFalse((output / "progress.json").exists())
        self.assertFalse((output / ".progress.json.retiring").exists())

    def test_terminal_resume_restores_progress_when_retirement_cleanup_fails(
        self,
    ) -> None:
        output = self.root / "resume-terminal-retirement-cleanup-failure"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        injected = ControllerError("retirement cleanup failed")
        real_remove = qemu_runner_module._remove_owned_result_entry

        def fail_retirement_cleanup(
            output_descriptor: int,
            name: str,
            expected_descriptor: int,
        ) -> None:
            if name == ".progress.json.retiring":
                raise injected
            real_remove(output_descriptor, name, expected_descriptor)

        errors: list[BaseException] = []
        with mock.patch.object(
            qemu_runner_module,
            "_remove_owned_result_entry",
            side_effect=fail_retirement_cleanup,
        ):
            try:
                controller.run(resume=True)
            except BaseException as error:
                errors.append(error)

        self.assertEqual(
            (output / "results.ndjson").read_bytes(), result_bytes
        )
        self.assertTrue((output / "progress.json").is_file())
        self.assertEqual(
            (output / "progress.json").read_bytes(), progress_bytes
        )
        self.assertFalse((output / ".progress.json.retiring").exists())
        self.assertEqual(factory.argv, [])
        self.assertEqual(errors, [injected])

    def test_terminal_resume_recreates_progress_when_result_changes_during_cleanup(
        self,
    ) -> None:
        output = self.root / "resume-terminal-cleanup-result-race"
        clock, _result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        replacement = b"cleanup result replacement\n"
        replacement_descriptor: int | None = None
        transport = FakeTransport(clock, [])
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_remove = qemu_runner_module._remove_owned_result_entry

        def replace_after_retirement_cleanup(
            output_descriptor: int,
            name: str,
            expected_descriptor: int,
        ) -> None:
            nonlocal replacement_descriptor
            real_remove(output_descriptor, name, expected_descriptor)
            if name == ".progress.json.retiring":
                replacement_descriptor = self._replace_and_hold(
                    output / "results.ndjson",
                    "cleanup-result-replacement",
                    replacement,
                )

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                qemu_runner_module,
                "_remove_owned_result_entry",
                side_effect=replace_after_retirement_cleanup,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                os.pread(replacement_descriptor, len(replacement), 0),
                replacement,
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), replacement
            )
            progress_path = output / "progress.json"
            self.assertTrue(progress_path.is_file())
            self.assertEqual(progress_path.read_bytes(), progress_bytes)
            self.assertEqual(progress_path.stat().st_nlink, 1)
            self.assertEqual(
                [
                    path.name
                    for path in output.iterdir()
                    if path.name.startswith(".progress.json")
                ],
                [],
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_terminal_resume_retains_recreated_progress_when_cleanup_restore_is_blocked(
        self,
    ) -> None:
        output = self.root / "resume-terminal-cleanup-restore-blocked"
        clock, _result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        result_replacement = b"cleanup blocked result replacement\n"
        progress_blocker = b"cleanup blocked progress replacement\n"
        result_descriptor: int | None = None
        progress_descriptor: int | None = None
        transport = FakeTransport(clock, [])
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_remove = qemu_runner_module._remove_owned_result_entry

        def replace_after_retirement_cleanup(
            output_descriptor: int,
            name: str,
            expected_descriptor: int,
        ) -> None:
            nonlocal result_descriptor, progress_descriptor
            real_remove(output_descriptor, name, expected_descriptor)
            if name == ".progress.json.retiring":
                result_descriptor = self._replace_and_hold(
                    output / "results.ndjson",
                    "cleanup-blocked-result-replacement",
                    result_replacement,
                )
                progress_descriptor = self._replace_and_hold(
                    output / "progress.json",
                    "cleanup-blocked-progress-replacement",
                    progress_blocker,
                )

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                qemu_runner_module,
                "_remove_owned_result_entry",
                side_effect=replace_after_retirement_cleanup,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(result_descriptor)
            self.assertIsNotNone(progress_descriptor)
            assert result_descriptor is not None
            assert progress_descriptor is not None
            self.assertEqual(os.fstat(result_descriptor).st_nlink, 1)
            self.assertEqual(os.fstat(progress_descriptor).st_nlink, 1)
            self.assertEqual(
                (output / "results.ndjson").read_bytes(),
                result_replacement,
            )
            self.assertEqual(
                (output / "progress.json").read_bytes(), progress_blocker
            )
            retained = output / ".progress.json.retiring"
            self.assertTrue(retained.is_file())
            self.assertEqual(retained.read_bytes(), progress_bytes)
            self.assertEqual(retained.stat().st_nlink, 1)
            self.assertEqual(
                sorted(
                    path.name
                    for path in output.iterdir()
                    if path.name.startswith(".progress.json")
                ),
                [".progress.json.retiring"],
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            if progress_descriptor is not None:
                os.close(progress_descriptor)
            if result_descriptor is not None:
                os.close(result_descriptor)

    def test_terminal_resume_retains_checkpoint_when_progress_appears_before_commit(
        self,
    ) -> None:
        output = self.root / "resume-terminal-progress-before-commit"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        replacement = b"progress replacement before commit\n"
        replacement_descriptor: int | None = None
        transport = FakeTransport(clock, [])
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_validate = controller._validate_retirement_result

        def replace_after_staged_validation(*args, **kwargs) -> None:
            nonlocal replacement_descriptor
            real_validate(*args, **kwargs)
            phase = args[3] if len(args) > 3 else kwargs["phase"]
            if phase == "after progress staging":
                replacement_descriptor = self._replace_and_hold(
                    output / "progress.json",
                    "progress-before-commit-replacement",
                    replacement,
                )

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                controller,
                "_validate_retirement_result",
                side_effect=replace_after_staged_validation,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                (output / "progress.json").read_bytes(), replacement
            )
            retained = output / ".progress.json.retiring"
            self.assertTrue(retained.is_file())
            self.assertEqual(retained.read_bytes(), progress_bytes)
            self.assertEqual(retained.stat().st_nlink, 1)
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), result_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_terminal_resume_recreates_checkpoint_when_progress_appears_during_cleanup(
        self,
    ) -> None:
        output = self.root / "resume-terminal-progress-during-cleanup"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        replacement = b"progress replacement during cleanup\n"
        replacement_descriptor: int | None = None
        transport = FakeTransport(clock, [])
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_remove = qemu_runner_module._remove_owned_result_entry

        def replace_after_retirement_cleanup(
            output_descriptor: int,
            name: str,
            expected_descriptor: int,
        ) -> None:
            nonlocal replacement_descriptor
            real_remove(output_descriptor, name, expected_descriptor)
            if name == ".progress.json.retiring":
                replacement_descriptor = self._replace_and_hold(
                    output / "progress.json",
                    "progress-during-cleanup-replacement",
                    replacement,
                )

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                qemu_runner_module,
                "_remove_owned_result_entry",
                side_effect=replace_after_retirement_cleanup,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                (output / "progress.json").read_bytes(), replacement
            )
            retained = output / ".progress.json.retiring"
            self.assertTrue(retained.is_file())
            self.assertEqual(retained.read_bytes(), progress_bytes)
            self.assertEqual(retained.stat().st_nlink, 1)
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), result_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_fresh_campaign_reconciles_staged_progress_before_superseding_it(
        self,
    ) -> None:
        output = self.root / "fresh-reconciles-staged-progress"
        clock, _result_bytes, _progress_bytes = self._create_postcommit_campaign(
            output
        )
        os.replace(
            output / "progress.json",
            output / ".progress.json.retiring",
        )
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
            run_id_factory=lambda: "fresh-reconciles-staged-progress",
        )

        result = controller.run()

        self.assertTrue(result.complete)
        self.assertEqual(len(factory.argv), 1)
        self.assertFalse((output / "progress.json").exists())
        self.assertFalse((output / ".progress.json.retiring").exists())

    def test_fresh_campaign_rejects_staged_and_public_progress_before_mutation(
        self,
    ) -> None:
        output = self.root / "fresh-rejects-staged-and-public-progress"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        os.replace(
            output / "progress.json",
            output / ".progress.json.retiring",
        )
        blocker = b"fresh public progress blocker\n"
        blocker_descriptor = self._replace_and_hold(
            output / "progress.json",
            "fresh-progress-blocker",
            blocker,
        )
        result_descriptor = os.open(output / "results.ndjson", os.O_RDONLY)
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
            run_id_factory=lambda: "fresh-must-not-start",
        )
        errors: list[BaseException] = []
        try:
            try:
                controller.run()
            except BaseException as error:
                errors.append(error)

            self.assertEqual(os.fstat(blocker_descriptor).st_nlink, 1)
            self.assertEqual(os.fstat(result_descriptor).st_nlink, 1)
            self.assertEqual(
                (output / "progress.json").read_bytes(), blocker
            )
            self.assertEqual(
                (output / ".progress.json.retiring").read_bytes(),
                progress_bytes,
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), result_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            os.close(result_descriptor)
            os.close(blocker_descriptor)

    def test_fresh_campaign_reconciliation_prevents_later_resume_wedge(self) -> None:
        output = self.root / "fresh-reconciliation-prevents-resume-wedge"
        clock, _result_bytes, _progress_bytes = self._create_postcommit_campaign(
            output
        )
        os.replace(
            output / "progress.json",
            output / ".progress.json.retiring",
        )
        interrupted_transport = FakeTransport(clock, [PROMPT, KeyboardInterrupt()])
        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([interrupted_transport]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "fresh-before-resume-wedge",
        )
        with self.assertRaises(KeyboardInterrupt):
            fresh.run()

        resumed_transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        resumed_factory = TransportFactory([resumed_transport])
        resumed = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=resumed_factory,
            monotonic=clock.monotonic,
        )
        errors: list[BaseException] = []
        result: ControllerResult | None = None
        try:
            result = resumed.run(resume=True)
        except BaseException as error:
            errors.append(error)

        self.assertEqual(errors, [])
        self.assertIsNotNone(result)
        assert result is not None
        self.assertTrue(result.complete)
        self.assertEqual(len(resumed_factory.argv), 1)
        self.assertFalse((output / "progress.json").exists())
        self.assertFalse((output / ".progress.json.retiring").exists())

    def test_partial_resume_rejects_result_replacement_after_validation(
        self,
    ) -> None:
        output = self.root / "resume-partial-result-race"
        clock, partial_bytes, _controller = self._create_exact_partial_result(
            output
        )
        progress_bytes = (output / "progress.json").read_bytes()
        replacement = b"partial result replacement\n"
        replacement_descriptor: int | None = None
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_validation = controller._resume_result_is_committed

        def replace_after_validation(*args, **kwargs):
            nonlocal replacement_descriptor
            committed = real_validation(*args, **kwargs)
            replacement_descriptor = self._replace_and_hold(
                output / "results.ndjson",
                "partial-result-replacement",
                replacement,
            )
            return committed

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                controller,
                "_resume_result_is_committed",
                side_effect=replace_after_validation,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                os.pread(replacement_descriptor, len(replacement), 0),
                replacement,
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), replacement
            )
            self.assertEqual(
                (output / "progress.json").read_bytes(), progress_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
            self.assertNotEqual(replacement, partial_bytes)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_terminal_resume_rejects_progress_replacement_after_validation(
        self,
    ) -> None:
        output = self.root / "resume-terminal-progress-race"
        clock, result_bytes, progress_bytes = self._create_postcommit_campaign(
            output
        )
        replacement = b"terminal progress replacement\n"
        replacement_descriptor: int | None = None
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_load = controller._load_progress

        def replace_after_validation(*args, **kwargs):
            nonlocal replacement_descriptor
            checkpoint = real_load(*args, **kwargs)
            replacement_descriptor = self._replace_and_hold(
                output / "progress.json",
                "terminal-progress-replacement",
                replacement,
            )
            return checkpoint

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                controller,
                "_load_progress",
                side_effect=replace_after_validation,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                os.pread(replacement_descriptor, len(replacement), 0),
                replacement,
            )
            self.assertEqual(
                (output / "progress.json").read_bytes(), replacement
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), result_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
            self.assertNotEqual(replacement, progress_bytes)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_incomplete_resume_rejects_progress_replacement_before_rewrite(
        self,
    ) -> None:
        output = self.root / "resume-partial-progress-race"
        clock, partial_bytes, _controller = self._create_exact_partial_result(
            output
        )
        replacement = b"partial progress replacement\n"
        replacement_descriptor: int | None = None
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
        )
        real_load = controller._load_progress

        def replace_after_validation(*args, **kwargs):
            nonlocal replacement_descriptor
            checkpoint = real_load(*args, **kwargs)
            replacement_descriptor = self._replace_and_hold(
                output / "progress.json",
                "partial-progress-replacement",
                replacement,
            )
            return checkpoint

        errors: list[BaseException] = []
        try:
            with mock.patch.object(
                controller,
                "_load_progress",
                side_effect=replace_after_validation,
            ):
                try:
                    controller.run(resume=True)
                except BaseException as error:
                    errors.append(error)

            self.assertIsNotNone(replacement_descriptor)
            assert replacement_descriptor is not None
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(
                os.pread(replacement_descriptor, len(replacement), 0),
                replacement,
            )
            self.assertEqual(
                (output / "progress.json").read_bytes(), replacement
            )
            self.assertEqual(
                (output / "results.ndjson").read_bytes(), partial_bytes
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
            self.assertEqual(len(errors), 1)
            self.assertIs(type(errors[0]), ControllerError)
        finally:
            if replacement_descriptor is not None:
                os.close(replacement_descriptor)

    def test_resume_progress_replacement_after_exchange_cleans_owned_temporary_entry(
        self,
    ) -> None:
        output = self.root / "resume-progress-post-exchange-race"
        output.mkdir()
        progress_path = output / "progress.json"
        original = b"validated progress\n"
        progress_path.write_bytes(original)
        replacement_path = output / "progress-replacement"
        replacement = b"raced progress replacement\n"
        replacement_path.write_bytes(replacement)
        output_descriptor = os.open(
            output,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        expected_descriptor = os.open(
            progress_path.name,
            os.O_RDONLY,
            dir_fd=output_descriptor,
        )
        replacement_descriptor = os.open(replacement_path, os.O_RDONLY)
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([]),
            monotonic=self.clock.monotonic,
        )
        real_exchange = qemu_runner_module._rename_exchange
        exchange_count = 0

        def exchange_then_replace(parent: int, first: str, second: str) -> None:
            nonlocal exchange_count
            exchange_count += 1
            real_exchange(parent, first, second)
            if exchange_count == 1:
                os.replace(
                    replacement_path.name,
                    first,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )

        try:
            with mock.patch.object(
                qemu_runner_module,
                "_rename_exchange",
                side_effect=exchange_then_replace,
            ):
                with self.assertRaisesRegex(ControllerError, "progress changed"):
                    controller._replace_progress(
                        output_descriptor,
                        expected_descriptor,
                        qemu_runner_module._stat_fingerprint(
                            os.fstat(expected_descriptor)
                        ),
                        b"new progress\n",
                    )

            self.assertEqual(progress_path.read_bytes(), replacement)
            self.assertEqual(os.fstat(replacement_descriptor).st_nlink, 1)
            self.assertEqual(os.fstat(expected_descriptor).st_nlink, 0)
            self.assertEqual(list(output.glob(".progress.json.*.resume")), [])
            self.assertEqual(exchange_count, 1)
        finally:
            os.close(replacement_descriptor)
            os.close(expected_descriptor)
            os.close(output_descriptor)

    def test_postcommit_progress_fsync_failure_preserves_truthful_state(
        self,
    ) -> None:
        output = self.root / "resume-postcommit-fsync-failure"
        clock, result_bytes, _progress_bytes = self._create_postcommit_campaign(
            output
        )
        output_info = output.stat()
        output_identity = (output_info.st_dev, output_info.st_ino)
        progress_unlinked = False
        injected = OSError(errno.EIO, "postcommit progress fsync failed")
        real_rename = qemu_runner_module._rename_noreplace_between
        real_fsync = qemu_runner_module.os.fsync

        def tracked_rename(
            source_parent: int,
            source_name: str,
            destination_parent: int,
            destination_name: str,
        ) -> None:
            nonlocal progress_unlinked
            real_rename(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
            )
            if source_name == "progress.json":
                progress_unlinked = True

        def fail_progress_fsync(descriptor: int) -> None:
            info = os.fstat(descriptor)
            if progress_unlinked and (info.st_dev, info.st_ino) == output_identity:
                raise injected
            real_fsync(descriptor)

        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-postcommit-fsync-failure",
        )
        with (
            mock.patch.object(
                qemu_runner_module,
                "_rename_noreplace_between",
                side_effect=tracked_rename,
            ),
            mock.patch.object(
                qemu_runner_module.os,
                "fsync",
                side_effect=fail_progress_fsync,
            ),
        ):
            with self.assertRaises(ControllerError) as raised:
                controller.run(resume=True)

        self.assertIs(type(raised.exception), ControllerError)
        self.assertIs(raised.exception.__cause__, injected)
        self.assertEqual((output / "results.ndjson").read_bytes(), result_bytes)
        self.assertFalse((output / "progress.json").exists())
        self.assertEqual(factory.argv, [])

    def test_resume_rejects_unsafe_or_inexact_postcommit_results(self) -> None:
        for label in (
            "mismatched",
            "noncanonical",
            "truncated",
            "symlink",
            "fifo",
            "multi-link",
        ):
            with self.subTest(case=label):
                output = self.root / f"resume-postcommit-{label}"
                clock, result_bytes, progress_bytes = (
                    self._create_postcommit_campaign(output)
                )
                result_path = output / "results.ndjson"
                sentinel = self.root / f"postcommit-sentinel-{label}"
                if label == "mismatched":
                    rows = [
                        json.loads(line)
                        for line in result_bytes.splitlines()
                    ]
                    rows[-1]["run_id"] = "mismatched-postcommit-value-x"
                    altered = (
                        "".join(
                            json.dumps(
                                row,
                                sort_keys=True,
                                separators=(",", ":"),
                            )
                            + "\n"
                            for row in rows
                        )
                    ).encode("ascii")
                    self.assertEqual(len(altered), len(result_bytes))
                    result_path.write_bytes(altered)
                elif label == "noncanonical":
                    rows = [
                        json.loads(line)
                        for line in result_bytes.splitlines()
                    ]
                    altered = (
                        "".join(
                            json.dumps(
                                {
                                    key: row[key]
                                    for key in reversed(tuple(row))
                                },
                                separators=(",", ":"),
                            )
                            + "\n"
                            for row in rows
                        )
                    ).encode("ascii")
                    self.assertEqual(len(altered), len(result_bytes))
                    result_path.write_bytes(altered)
                elif label == "truncated":
                    altered = result_bytes[:-1]
                    result_path.write_bytes(altered)
                elif label == "symlink":
                    sentinel.write_bytes(result_bytes)
                    result_path.unlink()
                    result_path.symlink_to(sentinel)
                    altered = result_bytes
                elif label == "fifo":
                    result_path.unlink()
                    os.mkfifo(result_path)
                    altered = b""
                else:
                    sentinel.hardlink_to(result_path)
                    altered = result_bytes

                factory = TransportFactory([])
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=factory,
                    monotonic=clock.monotonic,
                    run_id_factory=lambda label=label: (
                        f"unused-postcommit-{label}"
                    ),
                )
                with self.assertRaises(ControllerError) as raised:
                    controller.run(resume=True)

                self.assertIs(type(raised.exception), ControllerError)
                self.assertEqual(
                    (output / "progress.json").read_bytes(),
                    progress_bytes,
                )
                self.assertEqual(factory.argv, [])
                if label in {"mismatched", "noncanonical", "truncated"}:
                    self.assertEqual(result_path.read_bytes(), altered)
                elif label == "symlink":
                    self.assertTrue(result_path.is_symlink())
                    self.assertEqual(sentinel.read_bytes(), result_bytes)
                elif label == "fifo":
                    self.assertTrue(stat.S_ISFIFO(result_path.lstat().st_mode))
                else:
                    self.assertEqual(result_path.stat().st_nlink, 2)
                    self.assertEqual(sentinel.read_bytes(), result_bytes)

    def test_resume_rejects_postcommit_result_with_active_checkpoint(self) -> None:
        output = self.root / "resume-postcommit-active-test"
        clock, result_bytes, _progress_bytes = self._create_postcommit_campaign(
            output,
            self.tests,
        )
        progress_path = output / "progress.json"
        progress = json.loads(progress_path.read_bytes())
        progress["completed_attempts"] = progress["completed_attempts"][:1]
        progress["current_test"] = self.two.test_id
        progress_path.write_bytes(
            (
                json.dumps(progress, sort_keys=True, separators=(",", ":"))
                + "\n"
            ).encode("ascii")
        )
        progress_bytes = progress_path.read_bytes()
        factory = TransportFactory([])
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=clock.monotonic,
            run_id_factory=lambda: "unused-postcommit-active-test",
        )

        with self.assertRaisesRegex(ControllerError, "active test"):
            controller.run(resume=True)

        self.assertEqual((output / "results.ndjson").read_bytes(), result_bytes)
        self.assertEqual(progress_path.read_bytes(), progress_bytes)
        self.assertEqual(factory.argv, [])

    def test_first_run_fsyncs_each_new_output_parent_before_descent(self) -> None:
        output = self.root / "durable-qemu-output" / "nested"
        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-durable-output-chain",
        )
        target_parts = {"durable-qemu-output", "nested"}
        events: list[tuple[str, object, object]] = []
        real_mkdir = qemu_runner_module.os.mkdir
        real_open = qemu_runner_module.os.open
        real_fsync = qemu_runner_module.os.fsync

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
                qemu_runner_module.os,
                "mkdir",
                side_effect=traced_mkdir,
            ),
            mock.patch.object(
                qemu_runner_module.os,
                "open",
                side_effect=traced_open,
            ),
            mock.patch.object(
                qemu_runner_module.os,
                "fsync",
                side_effect=traced_fsync,
            ),
        ):
            result = controller.run()

        for part in ("durable-qemu-output", "nested"):
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
        self.assertTrue(result.result_path.is_file())

    def test_fresh_run_invalidates_stale_results_before_progress(self) -> None:
        prior_transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([prior_transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-result",
        ).run()
        prior_bytes = prior.result_path.read_bytes()

        interrupted_transport = FakeTransport(
            self.clock,
            [PROMPT, KeyboardInterrupt()],
        )
        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([interrupted_transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-fresh-progress",
        )

        with self.assertRaises(KeyboardInterrupt):
            fresh.run()

        if prior.result_path.exists():
            self.assertNotEqual(prior.result_path.read_bytes(), prior_bytes)
        with self.assertRaises(ValueError):
            report_module._load_runtime_results(
                prior.result_path,
                (self.one,),
                _identity((self.one,)).build_results,
                _identity((self.one,)).metadata,
                role="smros",
            )
        progress = json.loads(
            (self.root / "progress.json").read_text(encoding="utf-8")
        )
        self.assertEqual(progress["run_id"], "controller-fresh-progress")
        self.assertNotEqual(
            progress["run_id"],
            json.loads(prior_bytes.splitlines()[-1])["run_id"],
        )

    def test_concurrent_fresh_campaign_fails_before_mutating_active_outputs(
        self,
    ) -> None:
        output = self.root / "concurrent-fresh-campaigns"
        entered = threading.Event()
        release = threading.Event()
        first_clock = FakeClock()
        first_transport = PausingTransport(
            first_clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
            entered,
            release,
            pause_before_read=1,
        )
        first = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([first_transport]),
            monotonic=first_clock.monotonic,
            run_id_factory=lambda: "controller-concurrent-first",
        )
        first_results: list[ControllerResult] = []
        first_errors: list[BaseException] = []

        def run_first() -> None:
            try:
                first_results.append(first.run())
            except BaseException as error:
                first_errors.append(error)

        runner = threading.Thread(target=run_first, daemon=True)
        runner.start()
        self.assertTrue(entered.wait(2.0))
        paths = (
            output / "results.ndjson",
            output / "progress.json",
            output / "qemu-serial.log",
        )
        before = tuple(path.read_bytes() for path in paths)
        second_clock = FakeClock()
        second_transport = FakeTransport(
            second_clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        second_factory = TransportFactory([second_transport])
        second = QemuController(
            identity=_identity((self.two,)),
            selected=(self.two,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=second_factory,
            monotonic=second_clock.monotonic,
            run_id_factory=lambda: "controller-concurrent-second",
        )
        second_error: BaseException | None = None
        try:
            second.run()
        except BaseException as error:
            second_error = error
        after_second = tuple(
            path.read_bytes() if path.exists() else None for path in paths
        )
        release.set()
        runner.join(5.0)

        self.assertFalse(runner.is_alive())
        self.assertIs(type(second_error), ControllerError)
        self.assertRegex(str(second_error), "campaign.*active")
        self.assertEqual(second_factory.argv, [])
        self.assertEqual(second_transport.writes, [])
        self.assertEqual(after_second, before)
        self.assertEqual(first_errors, [])
        self.assertEqual(len(first_results), 1)
        terminal = json.loads(paths[0].read_bytes().splitlines()[-1])
        self.assertEqual(terminal["selected_count"], 1)
        self.assertEqual(first_results[0].attempts[0].test_id, self.one.test_id)
        raw = paths[2].read_bytes()
        self.assertIn(self.one.test_id.encode("ascii"), raw)
        self.assertNotIn(self.two.test_id.encode("ascii"), raw)

    def test_post_acquisition_lock_replacement_cannot_admit_second_campaign(
        self,
    ) -> None:
        output = self.root / "post-acquisition-lock-replacement"
        entered = threading.Event()
        release = threading.Event()
        first_clock = FakeClock()
        first_transport = PausingTransport(
            first_clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
            entered,
            release,
            pause_before_read=1,
        )
        first = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([first_transport]),
            monotonic=first_clock.monotonic,
            run_id_factory=lambda: "controller-replaced-active-lock-first",
        )
        first_results: list[ControllerResult] = []
        first_errors: list[BaseException] = []

        def run_first() -> None:
            try:
                first_results.append(first.run())
            except BaseException as error:
                first_errors.append(error)

        runner = threading.Thread(target=run_first, daemon=True)
        runner.start()
        self.assertTrue(entered.wait(2.0))
        paths = (
            output / "results.ndjson",
            output / "progress.json",
            output / "qemu-serial.log",
        )
        before = tuple(path.read_bytes() for path in paths)
        lock = output / qemu_runner_module._CAMPAIGN_LOCK_NAME
        campaign_descriptors: dict[int, Path] = {}
        for entry in Path("/proc/self/fd").iterdir():
            try:
                target = Path(os.readlink(entry))
            except (FileNotFoundError, OSError):
                continue
            if target in {output, lock}:
                campaign_descriptors[int(entry.name)] = target
        self.assertEqual(set(campaign_descriptors.values()), {output, lock})
        self.assertTrue(
            all(
                not os.get_inheritable(descriptor)
                for descriptor in campaign_descriptors
            )
        )
        replacement = self.root / "replacement-active-campaign-lock"
        replacement.write_bytes(b"")
        replacement.chmod(0o600)
        os.replace(replacement, lock)
        second_clock = FakeClock()
        second_transport = FakeTransport(
            second_clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        second_factory = TransportFactory([second_transport])
        second = QemuController(
            identity=_identity((self.two,)),
            selected=(self.two,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=second_factory,
            monotonic=second_clock.monotonic,
            run_id_factory=lambda: "controller-replaced-active-lock-second",
        )
        second_error: BaseException | None = None
        try:
            second.run()
        except BaseException as error:
            second_error = error
        after_second = tuple(
            path.read_bytes() if path.exists() else None for path in paths
        )
        release.set()
        runner.join(5.0)

        self.assertFalse(runner.is_alive())
        self.assertIs(type(second_error), ControllerError)
        self.assertRegex(str(second_error), "campaign.*active")
        self.assertEqual(second_factory.argv, [])
        self.assertEqual(second_transport.writes, [])
        self.assertEqual(after_second, before)
        self.assertEqual(first_errors, [])
        self.assertEqual(len(first_results), 1)
        terminal = json.loads(paths[0].read_bytes().splitlines()[-1])
        self.assertEqual(terminal["selected_count"], 1)
        self.assertEqual(first_results[0].attempts[0].test_id, self.one.test_id)
        raw = paths[2].read_bytes()
        self.assertIn(self.one.test_id.encode("ascii"), raw)
        self.assertNotIn(self.two.test_id.encode("ascii"), raw)

    def test_campaign_lock_is_reused_across_fresh_and_resume_without_fd_leaks(
        self,
    ) -> None:
        output = self.root / "reusable-campaign-lock"
        clock, _progress_bytes, initial_raw = self._create_resumable_campaign(
            output
        )
        lock = output / qemu_runner_module._CAMPAIGN_LOCK_NAME

        def open_campaign_descriptors() -> tuple[int, ...]:
            descriptors: list[int] = []
            for entry in Path("/proc/self/fd").iterdir():
                try:
                    target = Path(os.readlink(entry))
                except (FileNotFoundError, OSError):
                    continue
                if target in {output, lock}:
                    descriptors.append(int(entry.name))
            return tuple(sorted(descriptors))

        self.assertEqual(open_campaign_descriptors(), ())
        resumed_transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.two), _end_events(self.two), PROMPT],
        )
        resumed = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([resumed_transport]),
            monotonic=clock.monotonic,
        ).run(resume=True)

        self.assertTrue(resumed.complete)
        self.assertEqual(
            [attempt.test_id for attempt in resumed.attempts],
            [self.one.test_id, self.two.test_id],
        )
        self.assertTrue((output / "qemu-serial.log").read_bytes().startswith(initial_raw))
        self.assertEqual(open_campaign_descriptors(), ())
        for index in range(3):
            fresh_clock = FakeClock()
            fresh_transport = FakeTransport(
                fresh_clock,
                [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
            )
            fresh = QemuController(
                identity=_identity((self.one,)),
                selected=(self.one,),
                config=ControllerConfig(
                    output_directory=output,
                    qemu_argv=self.config.qemu_argv,
                ),
                transport_factory=TransportFactory([fresh_transport]),
                monotonic=fresh_clock.monotonic,
                run_id_factory=lambda index=index: (
                    f"controller-reused-lock-{index}"
                ),
            ).run()
            self.assertTrue(fresh.complete)
            self.assertEqual(open_campaign_descriptors(), ())
        info = lock.stat(follow_symlinks=False)
        self.assertTrue(stat.S_ISREG(info.st_mode))
        self.assertEqual(info.st_uid, os.geteuid())
        self.assertEqual(stat.S_IMODE(info.st_mode), 0o600)
        self.assertEqual(info.st_nlink, 1)

    def test_campaign_lock_rejects_unsafe_entries_before_output_mutation(
        self,
    ) -> None:
        for label in ("symlink", "hardlink", "fifo", "directory", "mode"):
            with self.subTest(case=label):
                output = self.root / f"unsafe-campaign-lock-{label}"
                output.mkdir()
                artifacts = {
                    "results.ndjson": b"prior result bytes\n",
                    "progress.json": b"prior progress bytes\n",
                    "qemu-serial.log": b"prior raw bytes\n",
                }
                for name, data in artifacts.items():
                    (output / name).write_bytes(data)
                lock = output / qemu_runner_module._CAMPAIGN_LOCK_NAME
                sentinel = self.root / f"campaign-lock-sentinel-{label}"
                sentinel_bytes = f"sentinel {label}\n".encode("ascii")
                sentinel.write_bytes(sentinel_bytes)
                if label == "symlink":
                    lock.symlink_to(sentinel)
                elif label == "hardlink":
                    os.link(sentinel, lock)
                elif label == "fifo":
                    os.mkfifo(lock)
                elif label == "directory":
                    lock.mkdir()
                else:
                    lock.write_bytes(b"")
                    lock.chmod(0o644)
                transport = FakeTransport(
                    self.clock,
                    [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                )
                factory = TransportFactory([transport])
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=factory,
                    monotonic=self.clock.monotonic,
                    run_id_factory=lambda label=label: (
                        f"controller-unsafe-lock-{label}"
                    ),
                )

                with self.assertRaisesRegex(ControllerError, "campaign lock"):
                    controller.run()

                self.assertEqual(
                    {
                        name: (output / name).read_bytes()
                        for name in artifacts
                    },
                    artifacts,
                )
                self.assertEqual(sentinel.read_bytes(), sentinel_bytes)
                self.assertEqual(factory.argv, [])
                self.assertEqual(transport.writes, [])

    def test_campaign_directory_lock_error_is_normalized_before_mutation(
        self,
    ) -> None:
        output = self.root / "campaign-directory-lock-error"
        output.mkdir()
        artifacts = {
            "results.ndjson": b"prior result bytes\n",
            "progress.json": b"prior progress bytes\n",
            "qemu-serial.log": b"prior raw bytes\n",
        }
        for name, data in artifacts.items():
            (output / name).write_bytes(data)
        real_flock = qemu_runner_module.fcntl.flock

        def fail_directory_lock(descriptor: int, operation: int) -> None:
            if stat.S_ISDIR(os.fstat(descriptor).st_mode):
                raise OSError(errno.EIO, "injected directory flock failure")
            real_flock(descriptor, operation)

        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-directory-lock-error",
        )

        with mock.patch.object(
            qemu_runner_module.fcntl,
            "flock",
            side_effect=fail_directory_lock,
        ):
            with self.assertRaisesRegex(
                ControllerError,
                "campaign directory lock could not be acquired safely",
            ):
                controller.run()

        self.assertEqual(
            {name: (output / name).read_bytes() for name in artifacts},
            artifacts,
        )
        self.assertEqual(factory.argv, [])
        self.assertEqual(transport.writes, [])

    def test_campaign_lock_replacement_during_acquisition_fails_closed(
        self,
    ) -> None:
        output = self.root / "replaced-campaign-lock"
        output.mkdir()
        artifacts = {
            "results.ndjson": b"prior result bytes\n",
            "progress.json": b"prior progress bytes\n",
            "qemu-serial.log": b"prior raw bytes\n",
        }
        for name, data in artifacts.items():
            (output / name).write_bytes(data)
        lock = output / qemu_runner_module._CAMPAIGN_LOCK_NAME
        lock.write_bytes(b"")
        lock.chmod(0o600)
        replacement = self.root / "campaign-lock-replacement"
        replacement.write_bytes(b"")
        replacement.chmod(0o600)
        replacement_descriptor = os.open(replacement, os.O_RDONLY)
        real_flock = qemu_runner_module.fcntl.flock
        replaced = False

        def replace_while_locking(descriptor: int, operation: int) -> None:
            nonlocal replaced
            if not replaced and stat.S_ISREG(os.fstat(descriptor).st_mode):
                os.replace(replacement, lock)
                replaced = True
            real_flock(descriptor, operation)

        transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        factory = TransportFactory([transport])
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-replaced-campaign-lock",
        )
        try:
            with mock.patch.object(
                qemu_runner_module.fcntl,
                "flock",
                side_effect=replace_while_locking,
            ):
                with self.assertRaisesRegex(ControllerError, "campaign lock"):
                    controller.run()

            self.assertTrue(replaced)
            self.assertEqual(
                {
                    name: (output / name).read_bytes()
                    for name in artifacts
                },
                artifacts,
            )
            lock_info = lock.stat(follow_symlinks=False)
            held_info = os.fstat(replacement_descriptor)
            self.assertEqual(
                (lock_info.st_dev, lock_info.st_ino),
                (held_info.st_dev, held_info.st_ino),
            )
            self.assertEqual(factory.argv, [])
            self.assertEqual(transport.writes, [])
        finally:
            os.close(replacement_descriptor)

    def test_no_prior_interruption_leaves_an_empty_result_marker(self) -> None:
        output = self.root / "no-prior-interrupted-marker"
        clock = FakeClock()
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory(
                [FakeTransport(clock, [PROMPT, KeyboardInterrupt()])]
            ),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-no-prior-interrupted",
        )

        with self.assertRaises(KeyboardInterrupt):
            controller.run()

        marker = output / "results.ndjson"
        self.assertTrue(marker.is_file())
        self.assertEqual(marker.read_bytes(), b"")
        self.assertEqual(marker.stat().st_nlink, 1)

    def test_orphaned_cleanup_slot_recovers_across_two_fresh_runs(self) -> None:
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [
                            PROMPT,
                            _start_events(self.one),
                            _end_events(self.one),
                            PROMPT,
                        ],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-orphan-prior",
        ).run()
        prior_bytes = prior.result_path.read_bytes()
        sentinel = self.root / "unrelated-public-file"
        sentinel_bytes = b"unrelated public bytes\n"
        sentinel.write_bytes(sentinel_bytes)
        real_unlink = qemu_runner_module.os.unlink
        interrupted = False

        def interrupt_cleanup_unlink(path, *args, **kwargs) -> None:
            nonlocal interrupted
            if os.fspath(path) == "cleanup" and not interrupted:
                interrupted = True
                raise KeyboardInterrupt()
            real_unlink(path, *args, **kwargs)

        interrupted_controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-orphan-interrupted",
        )
        with mock.patch.object(
            qemu_runner_module.os,
            "unlink",
            side_effect=interrupt_cleanup_unlink,
        ):
            with self.assertRaises(KeyboardInterrupt):
                interrupted_controller.run()

        quarantine = self.root / ".smros-posix-qemu-quarantine"
        slot = quarantine / "cleanup"
        slot_info = slot.stat(follow_symlinks=False)
        self.assertTrue(stat.S_ISREG(slot_info.st_mode))
        self.assertEqual(slot_info.st_nlink, 1)
        self.assertEqual(slot.read_bytes(), prior_bytes)
        self.assertEqual(sentinel.read_bytes(), sentinel_bytes)
        quarantine_info = quarantine.stat()
        quarantine_identity = (quarantine_info.st_dev, quarantine_info.st_ino)
        recovery_events: list[str] = []
        real_fsync = qemu_runner_module.os.fsync

        def trace_recovery_unlink(path, *args, **kwargs) -> None:
            real_unlink(path, *args, **kwargs)
            if os.fspath(path) == "cleanup":
                recovery_events.append("cleanup-unlinked")

        def trace_recovery_fsync(descriptor: int) -> None:
            real_fsync(descriptor)
            info = os.fstat(descriptor)
            if (info.st_dev, info.st_ino) == quarantine_identity:
                recovery_events.append("quarantine-fsynced")

        errors: list[BaseException] = []
        results: list[ControllerResult] = []
        with (
            mock.patch.object(
                qemu_runner_module.os,
                "unlink",
                side_effect=trace_recovery_unlink,
            ),
            mock.patch.object(
                qemu_runner_module.os,
                "fsync",
                side_effect=trace_recovery_fsync,
            ),
        ):
            for attempt in range(2):
                transport = FakeTransport(
                    self.clock,
                    [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                )
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=self.config,
                    transport_factory=TransportFactory([transport]),
                    monotonic=self.clock.monotonic,
                    run_id_factory=lambda attempt=attempt: (
                        f"controller-orphan-recovery-{attempt}"
                    ),
                )
                try:
                    results.append(controller.run())
                except BaseException as error:
                    errors.append(error)
                    break

        self.assertEqual(errors, [])
        self.assertEqual(len(results), 2)
        self.assertTrue(all(result.complete for result in results))
        self.assertEqual(
            recovery_events[:2],
            ["cleanup-unlinked", "quarantine-fsynced"],
        )
        self.assertFalse(slot.exists())
        self.assertEqual(tuple(quarantine.iterdir()), ())
        self.assertEqual(sentinel.read_bytes(), sentinel_bytes)

    def test_orphan_cleanup_rejects_unsafe_private_slot_states(self) -> None:
        for label in (
            "symlink",
            "hardlink",
            "fifo",
            "directory",
            "extra-entry",
        ):
            with self.subTest(case=label):
                output = self.root / f"unsafe-orphan-{label}"
                output.mkdir()
                (output / "results.ndjson").write_bytes(b"prior result\n")
                quarantine = output / ".smros-posix-qemu-quarantine"
                quarantine.mkdir(mode=0o700)
                quarantine.chmod(0o700)
                slot = quarantine / "cleanup"
                sentinel = self.root / f"unsafe-orphan-sentinel-{label}"
                sentinel_bytes = f"sentinel {label}\n".encode("ascii")
                sentinel.write_bytes(sentinel_bytes)
                if label == "symlink":
                    slot.symlink_to(sentinel)
                elif label == "hardlink":
                    os.link(sentinel, slot)
                elif label == "fifo":
                    os.mkfifo(slot)
                elif label == "directory":
                    slot.mkdir()
                else:
                    slot.write_bytes(b"owned cleanup candidate\n")
                    (quarantine / "unexpected").write_bytes(b"unexpected\n")

                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([]),
                    monotonic=self.clock.monotonic,
                    run_id_factory=lambda label=label: (
                        f"controller-unsafe-orphan-{label}"
                    ),
                )
                with self.assertRaises(ControllerError) as raised:
                    controller.run()

                self.assertIs(type(raised.exception), ControllerError)
                self.assertRegex(str(raised.exception), "quarantine")
                self.assertEqual(sentinel.read_bytes(), sentinel_bytes)
                self.assertTrue(os.path.lexists(slot))

    def test_result_invalidation_preserves_a_raced_replacement(self) -> None:
        prior_transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([prior_transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-race",
        ).run()
        moved = self.root / "validated-prior-results.ndjson"
        replacement = b"unrelated raced replacement\n"
        real_exchange = getattr(qemu_runner_module, "_rename_exchange", None)
        raced = False

        def race_then_exchange(parent: int, first: str, second: str) -> None:
            nonlocal raced
            self.assertIsNotNone(real_exchange)
            if raced:
                assert real_exchange is not None
                real_exchange(parent, first, second)
                return
            raced = True
            os.rename(
                first,
                moved.name,
                src_dir_fd=parent,
                dst_dir_fd=parent,
            )
            descriptor = os.open(
                first,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                0o644,
                dir_fd=parent,
            )
            try:
                os.write(descriptor, replacement)
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            assert real_exchange is not None
            real_exchange(parent, first, second)

        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [
                            PROMPT,
                            _start_events(self.one),
                            _end_events(self.one),
                            PROMPT,
                        ],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-raced-invalidation",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_rename_exchange",
            side_effect=race_then_exchange,
            create=True,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                fresh.run()

        self.assertEqual(prior.result_path.read_bytes(), replacement)
        self.assertTrue(moved.is_file())
        self.assertFalse((self.root / "progress.json").exists())

    def test_result_invalidation_detects_replacement_after_exchange(self) -> None:
        prior_transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([prior_transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-post-exchange",
        ).run()
        replacement_path = self.root / "post-exchange-replacement"
        replacement = b"post-exchange replacement\n"
        replacement_path.write_bytes(replacement)
        real_exchange = qemu_runner_module._rename_exchange
        exchange_count = 0

        def exchange_then_replace(parent: int, first: str, second: str) -> None:
            nonlocal exchange_count
            exchange_count += 1
            real_exchange(parent, first, second)
            if exchange_count == 1:
                os.replace(
                    replacement_path.name,
                    first,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )

        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-post-exchange-race",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_rename_exchange",
            side_effect=exchange_then_replace,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                fresh.run()

        self.assertEqual(prior.result_path.read_bytes(), replacement)
        self.assertEqual(exchange_count, 1)
        self.assertFalse((self.root / "progress.json").exists())

    def test_result_invalidation_rechecks_marker_after_old_result_cleanup(
        self,
    ) -> None:
        prior_transport = FakeTransport(
            self.clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([prior_transport]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-post-check",
        ).run()
        replacement_path = self.root / "post-check-replacement"
        replacement = b"post-check replacement\n"
        replacement_path.write_bytes(replacement)
        real_matches = qemu_runner_module._entry_matches
        public_checks = 0

        def replace_after_marker_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal public_checks
            matches = real_matches(parent, name, descriptor)
            if name == "results.ndjson":
                public_checks += 1
                if public_checks == 2 and matches:
                    os.replace(
                        replacement_path.name,
                        name,
                        src_dir_fd=parent,
                        dst_dir_fd=parent,
                    )
            return matches

        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-post-check-race",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_marker_check,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                fresh.run()

        self.assertEqual(prior.result_path.read_bytes(), replacement)
        self.assertGreaterEqual(public_checks, 2)
        self.assertFalse((self.root / "progress.json").exists())

    def test_result_invalidation_cleanup_preserves_a_checked_replacement(
        self,
    ) -> None:
        QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-cleanup-check",
        ).run()
        replacement_path = self.root / "invalidation-cleanup-replacement"
        replacement = b"invalidation cleanup replacement\n"
        replacement_path.write_bytes(replacement)
        real_matches = qemu_runner_module._entry_matches
        injected_path: Path | None = None

        def replace_after_hidden_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal injected_path
            matches = real_matches(parent, name, descriptor)
            if (
                injected_path is None
                and matches
                and name.startswith(".results.ndjson.")
                and name.endswith(".invalid")
            ):
                os.replace(
                    replacement_path.name,
                    name,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                injected_path = self.root / name
            return matches

        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-invalidation-cleanup-check",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_hidden_check,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                fresh.run()

        self.assertIsNotNone(injected_path)
        assert injected_path is not None
        self.assertTrue(injected_path.is_file())
        self.assertEqual(injected_path.read_bytes(), replacement)

    def test_result_invalidation_rollback_preserves_a_checked_replacement(
        self,
    ) -> None:
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-rollback-check",
        ).run()
        prior_bytes = prior.result_path.read_bytes()
        public_source = self.root / "invalidation-rollback-public"
        public_replacement = b"invalidation rollback public replacement\n"
        public_source.write_bytes(public_replacement)
        cleanup_source = self.root / "invalidation-rollback-cleanup"
        cleanup_replacement = b"invalidation rollback cleanup replacement\n"
        cleanup_source.write_bytes(cleanup_replacement)
        real_exchange = qemu_runner_module._rename_exchange
        real_matches = qemu_runner_module._entry_matches
        exchanged = False
        injected_path: Path | None = None

        def replace_public_then_exchange(parent: int, first: str, second: str) -> None:
            nonlocal exchanged
            if not exchanged:
                exchanged = True
                os.replace(
                    public_source.name,
                    first,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
            real_exchange(parent, first, second)

        def replace_after_rollback_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal injected_path
            matches = real_matches(parent, name, descriptor)
            if (
                exchanged
                and injected_path is None
                and matches
                and name.startswith(".results.ndjson.")
                and name.endswith(".invalid")
            ):
                os.replace(
                    cleanup_source.name,
                    name,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                injected_path = self.root / name
            return matches

        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-invalidation-rollback-check",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_rename_exchange",
            side_effect=replace_public_then_exchange,
        ), mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_rollback_check,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                fresh.run()

        self.assertNotEqual(prior_bytes, public_replacement)
        self.assertEqual(prior.result_path.read_bytes(), public_replacement)
        self.assertIsNotNone(injected_path)
        assert injected_path is not None
        self.assertTrue(injected_path.is_file())
        self.assertEqual(injected_path.read_bytes(), cleanup_replacement)

    def test_result_invalidation_finally_preserves_a_checked_replacement(
        self,
    ) -> None:
        prior = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        self.clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-prior-finally-check",
        ).run()
        prior_bytes = prior.result_path.read_bytes()
        replacement_path = self.root / "invalidation-finally-replacement"
        replacement = b"invalidation finally replacement\n"
        replacement_path.write_bytes(replacement)
        real_matches = qemu_runner_module._entry_matches
        injected_path: Path | None = None

        def replace_after_finally_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal injected_path
            matches = real_matches(parent, name, descriptor)
            if (
                injected_path is None
                and matches
                and name.startswith(".results.ndjson.")
                and name.endswith(".invalid")
            ):
                os.replace(
                    replacement_path.name,
                    name,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                injected_path = self.root / name
            return matches

        fresh = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=self.config,
            transport_factory=TransportFactory([]),
            monotonic=self.clock.monotonic,
            run_id_factory=lambda: "controller-invalidation-finally-check",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_rename_exchange",
            side_effect=OSError(errno.EIO, "injected exchange failure"),
        ), mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_finally_check,
        ):
            with self.assertRaises(ControllerError):
                fresh.run()

        self.assertEqual(prior.result_path.read_bytes(), prior_bytes)
        self.assertIsNotNone(injected_path)
        assert injected_path is not None
        self.assertTrue(injected_path.is_file())
        self.assertEqual(injected_path.read_bytes(), replacement)

    def test_result_publication_preserves_replacements_at_exchange_boundaries(
        self,
    ) -> None:
        for timing in ("before", "after"):
            with self.subTest(timing=timing):
                output = self.root / f"publish-race-{timing}"
                output.mkdir()
                clock = FakeClock()
                replacement_path = output / f"publish-{timing}-replacement"
                replacement = f"publish {timing} replacement\n".encode()
                replacement_path.write_bytes(replacement)
                real_exchange = qemu_runner_module._rename_exchange
                exchange_count = 0

                def race_at_exchange(parent: int, first: str, second: str) -> None:
                    nonlocal exchange_count
                    exchange_count += 1
                    if exchange_count == 1 and timing == "before":
                        os.replace(
                            replacement_path.name,
                            "results.ndjson",
                            src_dir_fd=parent,
                            dst_dir_fd=parent,
                        )
                    real_exchange(parent, first, second)
                    if exchange_count == 1 and timing == "after":
                        os.replace(
                            replacement_path.name,
                            "results.ndjson",
                            src_dir_fd=parent,
                            dst_dir_fd=parent,
                        )

                transport = FakeTransport(
                    clock,
                    [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                )
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-publish-{timing}",
                )
                with mock.patch.object(
                    qemu_runner_module,
                    "_rename_exchange",
                    side_effect=race_at_exchange,
                ):
                    with self.assertRaisesRegex(ControllerError, "results changed"):
                        controller.run()

                self.assertEqual(
                    (output / "results.ndjson").read_bytes(),
                    replacement,
                )
                self.assertEqual(exchange_count, 2 if timing == "before" else 1)

    def test_result_publication_cleanup_preserves_a_checked_replacement(
        self,
    ) -> None:
        output = self.root / "publication-cleanup-check"
        output.mkdir()
        replacement_path = output / "publication-cleanup-replacement"
        replacement = b"publication cleanup replacement\n"
        replacement_path.write_bytes(replacement)
        clock = FakeClock()
        real_matches = qemu_runner_module._entry_matches
        injected_path: Path | None = None

        def replace_after_marker_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal injected_path
            matches = real_matches(parent, name, descriptor)
            if (
                injected_path is None
                and matches
                and name.startswith(".results.ndjson.")
                and name.endswith(".publish")
            ):
                os.replace(
                    replacement_path.name,
                    name,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                injected_path = output / name
            return matches

        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-publication-cleanup-check",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_marker_check,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                controller.run()

        self.assertIsNotNone(injected_path)
        assert injected_path is not None
        self.assertTrue(injected_path.is_file())
        self.assertEqual(injected_path.read_bytes(), replacement)

    def test_result_publication_rollback_preserves_a_checked_replacement(
        self,
    ) -> None:
        output = self.root / "publication-rollback-check"
        output.mkdir()
        public_source = output / "publication-rollback-public"
        public_replacement = b"publication rollback public replacement\n"
        public_source.write_bytes(public_replacement)
        cleanup_source = output / "publication-rollback-cleanup"
        cleanup_replacement = b"publication rollback cleanup replacement\n"
        cleanup_source.write_bytes(cleanup_replacement)
        clock = FakeClock()
        real_exchange = qemu_runner_module._rename_exchange
        real_matches = qemu_runner_module._entry_matches
        exchange_count = 0
        injected_path: Path | None = None

        def replace_public_then_exchange(parent: int, first: str, second: str) -> None:
            nonlocal exchange_count
            exchange_count += 1
            if exchange_count == 1:
                os.replace(
                    public_source.name,
                    "results.ndjson",
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
            real_exchange(parent, first, second)

        def replace_after_generated_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal injected_path
            matches = real_matches(parent, name, descriptor)
            if (
                exchange_count == 2
                and injected_path is None
                and matches
                and name.startswith(".results.ndjson.")
                and name.endswith(".publish")
            ):
                os.replace(
                    cleanup_source.name,
                    name,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                injected_path = output / name
            return matches

        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-publication-rollback-check",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_rename_exchange",
            side_effect=replace_public_then_exchange,
        ), mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_generated_check,
        ):
            with self.assertRaisesRegex(ControllerError, "results changed"):
                controller.run()

        self.assertEqual(
            (output / "results.ndjson").read_bytes(),
            public_replacement,
        )
        self.assertIsNotNone(injected_path)
        assert injected_path is not None
        self.assertTrue(injected_path.is_file())
        self.assertEqual(injected_path.read_bytes(), cleanup_replacement)

    def test_result_publication_finally_preserves_a_checked_replacement(
        self,
    ) -> None:
        output = self.root / "publication-finally-check"
        output.mkdir()
        replacement_path = output / "publication-finally-replacement"
        replacement = b"publication finally replacement\n"
        replacement_path.write_bytes(replacement)
        clock = FakeClock()
        real_matches = qemu_runner_module._entry_matches
        injected_path: Path | None = None

        def replace_after_finally_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal injected_path
            matches = real_matches(parent, name, descriptor)
            if (
                injected_path is None
                and matches
                and name.startswith(".results.ndjson.")
                and name.endswith(".publish")
            ):
                os.replace(
                    replacement_path.name,
                    name,
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                injected_path = output / name
            return matches

        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory(
                [
                    FakeTransport(
                        clock,
                        [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                    )
                ]
            ),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-publication-finally-check",
        )
        with mock.patch.object(
            qemu_runner_module,
            "_rename_exchange",
            side_effect=OSError(errno.EIO, "injected exchange failure"),
        ), mock.patch.object(
            qemu_runner_module,
            "_entry_matches",
            side_effect=replace_after_finally_check,
        ):
            with self.assertRaises(ControllerError):
                controller.run()

        self.assertIsNotNone(injected_path)
        assert injected_path is not None
        self.assertTrue(injected_path.is_file())
        self.assertEqual(injected_path.read_bytes(), replacement)

    def test_interruption_cleanup_detects_and_preserves_result_replacement(
        self,
    ) -> None:
        output = self.root / "interrupted-result-replacement"
        clock = FakeClock()
        replacement_path = self.root / "interrupted-replacement"
        replacement = b"interrupted replacement\n"
        replacement_path.write_bytes(replacement)
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory(
                [FakeTransport(clock, [PROMPT, KeyboardInterrupt()])]
            ),
            monotonic=clock.monotonic,
            run_id_factory=lambda: "controller-interrupted-replacement",
        )
        real_persist = controller._persist_progress
        replaced = False

        def persist_then_replace() -> None:
            nonlocal replaced
            real_persist()
            if not replaced:
                replaced = True
                os.replace(replacement_path, output / "results.ndjson")

        with mock.patch.object(
            controller,
            "_persist_progress",
            side_effect=persist_then_replace,
        ):
            observed: BaseException | None = None
            try:
                controller.run()
            except BaseException as error:
                observed = error

        self.assertIs(type(observed), ControllerError)
        assert observed is not None
        self.assertRegex(str(observed), "results changed")
        self.assertIsInstance(observed.__cause__, KeyboardInterrupt)
        self.assertEqual((output / "results.ndjson").read_bytes(), replacement)

    def test_generated_run_ids_are_strict_utf8_nonempty_and_bounded(self) -> None:
        invalid_ids = (
            ("empty", ""),
            ("lone-surrogate", "\ud800"),
            ("over-limit", "r" * 257),
            ("not-text", None),
        )
        for label, run_id in invalid_ids:
            with self.subTest(case=label):
                clock = FakeClock()
                transport = FakeTransport(
                    clock,
                    [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
                )
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=self.root / f"invalid-run-id-{label}",
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda run_id=run_id: run_id,  # type: ignore[return-value]
                )
                with self.assertRaisesRegex(ValueError, "run ID"):
                    controller.run()

        unicode_run_id = "\u8fd0\u884c-\u03c0"
        clock = FakeClock()
        transport = FakeTransport(
            clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=self.root / "unicode-run-id",
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([transport]),
            monotonic=clock.monotonic,
            run_id_factory=lambda: unicode_run_id,
        )
        result = controller.run()
        terminal = json.loads(
            result.result_path.read_text(encoding="utf-8").splitlines()[-1]
        )
        self.assertEqual(result.attempts[0].run_id, unicode_run_id)
        self.assertEqual(terminal["run_id"], unicode_run_id)

        boundary_run_id = "\u96ea" * 85 + "a"
        boundary_clock = FakeClock()
        boundary_transport = FakeTransport(
            boundary_clock,
            [PROMPT, _start_events(self.one), _end_events(self.one), PROMPT],
        )
        boundary_controller = QemuController(
            identity=_identity((self.one,)),
            selected=(self.one,),
            config=ControllerConfig(
                output_directory=self.root / "boundary-run-id",
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=TransportFactory([boundary_transport]),
            monotonic=boundary_clock.monotonic,
            run_id_factory=lambda: boundary_run_id,
        )
        boundary_result = boundary_controller.run()
        self.assertEqual(len(boundary_run_id.encode("utf-8")), 256)
        self.assertEqual(boundary_result.attempts[0].run_id, boundary_run_id)

    def test_guest_infrastructure_terminals_stop_campaign_immediately(self) -> None:
        suite_start = _start_events(self.one).splitlines(keepends=True)[0]
        cases = {
            "active": (
                _start_events(self.one)
                + _event(
                    3,
                    "infrastructure_error",
                    detail="guest active collection failed",
                    test_id=self.one.test_id,
                    group=self.one.group,
                    api=self.one.api,
                ),
                "guest active collection failed",
                1,
            ),
            "post-suite-start": (
                suite_start
                + _event(
                    2,
                    "infrastructure_error",
                    message="guest setup failed",
                ),
                "guest setup failed",
                0,
            ),
            "standalone-preflight": (
                _event(
                    1,
                    "infrastructure_error",
                    run_id="error-17",
                    manifest_sha256="0" * 64,
                    message="guest manifest read failed",
                ),
                "guest manifest read failed",
                0,
            ),
        }
        for label, (terminal_bytes, detail, attempt_count) in cases.items():
            with self.subTest(case=label):
                clock = FakeClock()
                output = self.root / f"guest-terminal-{label}"
                transport = FakeTransport(clock, [PROMPT, terminal_bytes])
                controller = QemuController(
                    identity=_identity(self.tests),
                    selected=self.tests,
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: f"controller-{label}",
                )

                result = controller.run()

                self.assertFalse(result.complete)
                self.assertEqual(len(result.attempts), attempt_count)
                self.assertEqual(
                    transport.writes,
                    [f"posixtest test {self.one.test_id}\n".encode()],
                )
                self.assertEqual(result.restart_count, 0)
                self.assertIn(terminal_bytes, result.raw_log_path.read_bytes())
                rows = [
                    json.loads(line)
                    for line in result.result_path.read_text(
                        encoding="utf-8"
                    ).splitlines()
                ]
                terminal = rows[-1]
                self.assertFalse(terminal["complete"])
                self.assertEqual(terminal["infrastructure_error"], detail)
                self.assertEqual(terminal["completed_count"], attempt_count)
                if attempt_count:
                    attempt = result.attempts[0]
                    self.assertEqual(attempt.source, "smros-qemu")
                    self.assertEqual(attempt.status, "interrupted")
                    self.assertEqual(attempt.launch_status, "interrupted")
                    self.assertIsNone(attempt.pts_status)
                    self.assertIsNone(attempt.exit_code)
                    self.assertIsNone(attempt.signal)
                    self.assertFalse(attempt.timed_out)
                    self.assertEqual(attempt.resource_evidence, "unavailable")
                    self.assertFalse(attempt.resource_deltas.has_nonzero())
                    self.assertEqual(attempt.infrastructure_error, detail)
                    loaded = report_module._load_runtime_results(
                        result.result_path,
                        self.tests,
                        _identity(self.tests).build_results,
                        _identity(self.tests).metadata,
                        role="smros",
                    )
                    self.assertEqual(
                        loaded.attempts[0].resource_evidence,
                        "unavailable",
                    )

    def test_guest_infrastructure_terminal_resume_publishes_without_restart(
        self,
    ) -> None:
        output = self.root / "resume-guest-terminal"
        output.mkdir()
        (output / "qemu-serial.log").write_bytes(b"guest terminal evidence\n")
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        controller._run_id = "controller-resume-terminal"
        controller._boot_count = 1
        controller._infrastructure_error = "guest collection failed"
        guest = SerialAttempt(
            test_id=self.one.test_id,
            group=self.one.group,
            api=self.one.api,
            status="interrupted",
            pts_status=None,
            launch_status="interrupted",
            exit_code=None,
            signal=None,
            timed_out=False,
            duration_ms=0,
            stdout="",
            stderr="",
            resource_deltas=ResourceDeltas(),
            resource_evidence="unavailable",
            run_id="guest-run",
            manifest_sha256=MANIFEST_SHA256,
            architecture="aarch64",
            infrastructure_error="guest collection failed",
        )
        controller._attempts = [
            controller._guest_attempt(
                guest,
                self.one,
                raw_log_start=0,
                raw_log_end=0,
            )
        ]
        controller._persist_progress()
        factory = TransportFactory([])
        resumed = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
        )

        result = resumed.run(resume=True)

        self.assertFalse(result.complete)
        self.assertEqual(result.restart_count, 0)
        self.assertEqual(factory.argv, [])
        self.assertFalse((output / "progress.json").exists())
        terminal = json.loads(
            result.result_path.read_text(encoding="utf-8").splitlines()[-1]
        )
        self.assertEqual(terminal["boot_count"], 1)
        self.assertEqual(terminal["restart_count"], 0)
        self.assertEqual(
            terminal["infrastructure_error"],
            "guest collection failed",
        )

    def test_exact_guest_infrastructure_result_resumes_without_launch(
        self,
    ) -> None:
        output = self.root / "resume-exact-guest-terminal"
        output.mkdir()
        (output / "qemu-serial.log").write_bytes(b"guest terminal evidence\n")
        controller = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
        )
        controller._run_id = "controller-exact-resume-terminal"
        controller._boot_count = 1
        controller._infrastructure_error = "guest collection failed"
        controller._persist_progress()
        result_bytes = controller._result_bytes()
        (output / "results.ndjson").write_bytes(result_bytes)
        factory = TransportFactory([])
        resumed = QemuController(
            identity=_identity(self.tests),
            selected=self.tests,
            config=ControllerConfig(
                output_directory=output,
                qemu_argv=self.config.qemu_argv,
            ),
            transport_factory=factory,
        )

        result = resumed.run(resume=True)

        self.assertFalse(result.complete)
        self.assertEqual(result.attempts, ())
        self.assertEqual(factory.argv, [])
        self.assertEqual(result.result_path.read_bytes(), result_bytes)
        self.assertFalse((output / "progress.json").exists())

    def test_guest_infrastructure_terminal_rejects_invalid_context(self) -> None:
        valid_active = _start_events(self.one) + _event(
            3,
            "infrastructure_error",
            detail="guest collection failed",
            test_id=self.one.test_id,
            group=self.one.group,
            api=self.one.api,
        )
        suite_start = _start_events(self.one).splitlines(keepends=True)[0]
        cases = {
            "active-identity": _replace_event_values(
                valid_active,
                "infrastructure_error",
                test_id=self.two.test_id,
            ),
            "suite-provenance": _replace_event_values(
                suite_start
                + _event(2, "infrastructure_error", detail="guest collection failed"),
                "suite_start",
                build_id="0" * 64,
            ),
            "identity-without-active-test": suite_start
            + _event(
                2,
                "infrastructure_error",
                detail="guest collection failed",
                test_id=self.one.test_id,
            ),
            "escaped-surrogate-run-id": _replace_event_values(
                valid_active,
                "suite_start",
                run_id="\ud800",
            ),
        }
        for label, terminal_bytes in cases.items():
            with self.subTest(case=label):
                clock = FakeClock()
                transport = FakeTransport(clock, [PROMPT, terminal_bytes])
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=self.root / f"invalid-terminal-{label}",
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory([transport]),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda: "controller-invalid-terminal",
                )
                with self.assertRaisesRegex(ControllerError, "guest POSIX"):
                    controller.run()

    def test_guest_infrastructure_error_marks_only_guest_run_incomplete(
        self,
    ) -> None:
        for label, watchdog, expected_complete in (
            ("guest", False, False),
            ("host-watchdog", True, True),
        ):
            with self.subTest(source=label):
                clock = FakeClock()
                output = self.root / f"terminal-{label}"
                run_id = f"controller-{label}"
                controller = QemuController(
                    identity=_identity((self.one,)),
                    selected=(self.one,),
                    config=ControllerConfig(
                        output_directory=output,
                        qemu_argv=self.config.qemu_argv,
                    ),
                    transport_factory=TransportFactory(
                        [FakeTransport(clock, [PROMPT])]
                    ),
                    monotonic=clock.monotonic,
                    run_id_factory=lambda run_id=run_id: run_id,
                )
                controller._run_id = run_id
                if watchdog:
                    attempt = controller._watchdog_attempt(
                        self.one,
                        status="crash",
                        started=True,
                        timed_out=False,
                        reason="host watchdog observed a crash",
                        duration_ms=1,
                        raw_log_start=0,
                        raw_log_end=0,
                    )
                else:
                    guest = SerialAttempt(
                        test_id=self.one.test_id,
                        group=self.one.group,
                        api=self.one.api,
                        status="launch-error",
                        pts_status=None,
                        launch_status="launch-error",
                        exit_code=None,
                        signal=None,
                        timed_out=False,
                        duration_ms=1,
                        stdout="",
                        stderr="",
                        resource_deltas=ResourceDeltas(),
                        resource_evidence="measured",
                        run_id="guest-run",
                        manifest_sha256=MANIFEST_SHA256,
                        architecture="aarch64",
                        launch_error="launch failed",
                        infrastructure_error="cleanup failed",
                    )
                    attempt = controller._guest_attempt(
                        guest,
                        self.one,
                        raw_log_start=0,
                        raw_log_end=0,
                    )
                with mock.patch.object(
                    controller,
                    "_run_test",
                    return_value=(attempt, True, False),
                ):
                    result = controller.run()

                terminal = json.loads(
                    result.result_path.read_text(encoding="utf-8").splitlines()[-1]
                )
                self.assertIs(result.complete, expected_complete)
                self.assertIs(terminal["complete"], expected_complete)


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
            complete=True,
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

    @mock.patch("scripts.posix.cli.run_smros")
    def test_cli_run_smros_exit_tracks_collection_completeness(
        self,
        runner: mock.Mock,
    ) -> None:
        cases = (
            (
                "guest-infrastructure",
                mock.Mock(
                    status="launch-error",
                    source="smros-qemu",
                    infrastructure_error="guest cleanup failed",
                ),
                False,
                1,
            ),
            ("posix-fail", mock.Mock(status="fail"), True, 0),
            ("posix-timeout", mock.Mock(status="timeout"), True, 0),
            ("posix-crash", mock.Mock(status="crash"), True, 0),
        )
        for label, attempt, complete, expected in cases:
            with self.subTest(case=label):
                runner.return_value = ControllerResult(
                    attempts=(attempt,),
                    complete=complete,
                    restart_count=0,
                    result_path=Path("results.ndjson"),
                    raw_log_path=Path("qemu-serial.log"),
                )
                with redirect_stdout(io.StringIO()):
                    observed = cli_module.main(["run-smros"])
                self.assertEqual(observed, expected)


if __name__ == "__main__":
    unittest.main()
