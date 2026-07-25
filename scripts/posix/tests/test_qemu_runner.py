from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from dataclasses import replace
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
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
        output.mkdir()
        (output / "qemu-serial.log").write_bytes(b"")
        controller._persist_progress()
        controller._publish()
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
            resumed._load_progress(
                output_descriptor,
                (output / "qemu-serial.log").stat(),
            )
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
        self.assertEqual(
            terminal["infrastructure_error"],
            "guest collection failed",
        )

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
