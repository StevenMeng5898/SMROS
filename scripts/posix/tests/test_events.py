from __future__ import annotations

import json
import unittest

from scripts.posix.events import EVENT_PREFIX, parse_serial_log
from scripts.posix.model import RESOURCE_DELTA_NAMES


RUN_ID = "run-123"
MANIFEST_SHA256 = "a" * 64
EMPTY_SHA256 = "0" * 64


def _resources(**values: int) -> dict[str, int]:
    result = {name: 0 for name in RESOURCE_DELTA_NAMES}
    result.update(values)
    return result


def _event(seq: int, event: str, **values: object) -> str:
    payload = {
        "architecture": "aarch64",
        "event": event,
        "manifest_sha256": MANIFEST_SHA256,
        "run_id": RUN_ID,
        "schema": 1,
        "seq": seq,
        **values,
    }
    return EVENT_PREFIX + json.dumps(
        payload, sort_keys=True, separators=(",", ":")
    )


class SerialEventTests(unittest.TestCase):
    def _one_attempt_log(
        self,
        *,
        include_resources: bool = True,
        terminal_complete: bool = True,
        **end_values: object,
    ) -> str:
        test_id = "conformance/interfaces/getpid/1-1.c"
        status = str(end_values.get("status", "pass"))
        end_payload: dict[str, object] = {
            "status": status,
            "pts_status": "pass",
            "launch_status": "launched",
            "exit_code": 0,
            "signal": None,
            "timed_out": False,
            "duration_ms": 1,
        }
        if include_resources:
            end_payload["resource_deltas"] = _resources()
        end_payload.update(end_values)
        return "\n".join(
            (
                _event(1, "suite_start", selected_count=1),
                _event(
                    2,
                    "test_start",
                    test_id=test_id,
                    group="base",
                    api="getpid",
                ),
                _event(
                    3,
                    "test_end",
                    test_id=test_id,
                    group="base",
                    api="getpid",
                    **end_payload,
                ),
                _event(
                    4,
                    "suite_end",
                    complete=terminal_complete,
                    selected_count=1,
                    completed_count=1,
                    status_counts={status: 1},
                ),
            )
        )

    def test_parses_interleaved_output_and_complete_run(self) -> None:
        test_id = "conformance/interfaces/getpid/1-1.c"
        log = "\n".join(
            (
                "kernel: booting normally",
                _event(1, "suite_start", selected_count=1),
                _event(
                    2,
                    "test_start",
                    test_id=test_id,
                    group="base",
                    api="getpid",
                ),
                "program says <unsafe> & keeps running",
                "kernel: diagnostic interleaved with output",
                _event(
                    3,
                    "test_end",
                    test_id=test_id,
                    group="base",
                    api="getpid",
                    status="pass",
                    pts_status="pass",
                    launch_status="launched",
                    exit_code=0,
                    signal=None,
                    timed_out=False,
                    duration_ms=7,
                    resource_deltas=_resources(),
                ),
                _event(
                    4,
                    "suite_end",
                    complete=True,
                    selected_count=1,
                    completed_count=1,
                    status_counts={"pass": 1},
                ),
                "kernel: prompt",
            )
        )

        parsed = parse_serial_log(log)

        self.assertTrue(parsed.complete)
        self.assertEqual(parsed.status, "complete")
        self.assertEqual([event.seq for event in parsed.events], [1, 2, 3, 4])
        self.assertEqual(len(parsed.attempts), 1)
        attempt = parsed.attempts[0]
        self.assertEqual(attempt.test_id, test_id)
        self.assertEqual(attempt.stdout, (
            "program says <unsafe> & keeps running\n"
            "kernel: diagnostic interleaved with output\n"
        ))
        self.assertEqual(attempt.resource_deltas.linux_fds, 0)
        self.assertEqual(attempt.resource_deltas.processes, 0)
        self.assertEqual(attempt.resource_evidence, "measured")

    def test_parses_standalone_preflight_infrastructure_error(self) -> None:
        for detail_field in ("message", "detail"):
            with self.subTest(detail_field=detail_field):
                log = _event(
                    1,
                    "infrastructure_error",
                    run_id="error-123",
                    manifest_sha256=EMPTY_SHA256,
                    **{detail_field: "manifest-read"},
                )

                parsed = parse_serial_log(log)

                self.assertFalse(parsed.complete)
                self.assertEqual(parsed.status, "incomplete")
                self.assertEqual(parsed.attempts, ())
                self.assertEqual(parsed.run_id, "error-123")
                self.assertEqual(parsed.manifest_sha256, EMPTY_SHA256)
                self.assertEqual(parsed.architecture, "aarch64")
                self.assertEqual(len(parsed.events), 1)
                self.assertIs(parsed.terminal_event, parsed.events[0])
                self.assertEqual(parsed.terminal_event.event, "infrastructure_error")
                self.assertEqual(parsed.infrastructure_error, "manifest-read")

    def test_rejects_noncanonical_standalone_preflight_error(self) -> None:
        valid = _event(
            1,
            "infrastructure_error",
            run_id="error-123",
            manifest_sha256=EMPTY_SHA256,
            message="manifest-read",
        )
        cases = {
            "nonzero digest": _event(
                1,
                "infrastructure_error",
                run_id="error-123",
                message="manifest-read",
            ),
            "wrong sequence": _event(
                2,
                "infrastructure_error",
                run_id="error-123",
                manifest_sha256=EMPTY_SHA256,
                message="manifest-read",
            ),
            "wrong run ID": _event(
                1,
                "infrastructure_error",
                run_id="run-123",
                manifest_sha256=EMPTY_SHA256,
                message="manifest-read",
            ),
            "wrong schema": valid.replace('"schema":1', '"schema":2'),
            "wrong architecture": valid.replace("aarch64", "x86_64"),
            "missing detail": _event(
                1,
                "infrastructure_error",
                run_id="error-123",
                manifest_sha256=EMPTY_SHA256,
            ),
            "preceding event": "\n".join(
                (
                    _event(0, "test_start", test_id="unexpected"),
                    valid,
                )
            ),
            "following event": "\n".join(
                (
                    valid,
                    _event(
                        2,
                        "suite_start",
                        run_id="error-123",
                        manifest_sha256=EMPTY_SHA256,
                        selected_count=0,
                    ),
                )
            ),
        }
        for label, log in cases.items():
            with self.subTest(label=label):
                with self.assertRaises(ValueError):
                    parse_serial_log(log)

    def test_rejects_duplicate_terminal_event(self) -> None:
        log = "\n".join(
            (
                _event(1, "suite_start", selected_count=0),
                _event(
                    2,
                    "suite_end",
                    complete=True,
                    selected_count=0,
                    completed_count=0,
                    status_counts={},
                ),
                _event(
                    3,
                    "suite_end",
                    complete=True,
                    selected_count=0,
                    completed_count=0,
                    status_counts={},
                ),
            )
        )

        with self.assertRaisesRegex(ValueError, "duplicate terminal"):
            parse_serial_log(log)

    def test_rejects_malformed_event_json(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid event JSON"):
            parse_serial_log(EVENT_PREFIX + "{not-json}")

    def test_rejects_schema_sequence_run_and_test_mismatches(self) -> None:
        test_id = "conformance/interfaces/getpid/1-1.c"
        valid_start = _event(1, "suite_start", selected_count=1)
        valid_test = _event(
            2,
            "test_start",
            test_id=test_id,
            group="base",
            api="getpid",
        )
        cases = {
            "schema": EVENT_PREFIX + json.dumps(
                {
                    "architecture": "aarch64",
                    "event": "suite_start",
                    "manifest_sha256": MANIFEST_SHA256,
                    "run_id": RUN_ID,
                    "schema": 2,
                    "seq": 1,
                    "selected_count": 0,
                }
            ),
            "sequence": "\n".join(
                (valid_start, _event(1, "suite_end", complete=False))
            ),
            "run ID": "\n".join(
                (
                    valid_start,
                    _event(2, "suite_end", complete=False).replace(
                        RUN_ID, "different-run"
                    ),
                )
            ),
            "test ID": "\n".join(
                (
                    valid_start,
                    valid_test,
                    _event(
                        3,
                        "test_end",
                        test_id="conformance/interfaces/getpid/2-1.c",
                        group="base",
                        api="getpid",
                        status="fail",
                        pts_status="fail",
                        launch_status="launched",
                        exit_code=1,
                        signal=None,
                        timed_out=False,
                        duration_ms=2,
                    ),
                )
            ),
        }
        for label, log in cases.items():
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, label):
                    parse_serial_log(log)

    def test_missing_suite_end_is_explicitly_incomplete(self) -> None:
        test_id = "conformance/interfaces/getpid/1-1.c"
        log = "\n".join(
            (
                _event(1, "suite_start", selected_count=1),
                _event(
                    2,
                    "test_start",
                    test_id=test_id,
                    group="base",
                    api="getpid",
                ),
                "partial output",
            )
        )

        parsed = parse_serial_log(log)

        self.assertFalse(parsed.complete)
        self.assertEqual(parsed.status, "incomplete")
        self.assertEqual(len(parsed.attempts), 1)
        self.assertEqual(parsed.attempts[0].status, "interrupted")
        self.assertEqual(parsed.attempts[0].stdout, "partial output\n")
        self.assertEqual(parsed.attempts[0].resource_evidence, "unavailable")

    def test_explicit_test_end_requires_complete_resource_evidence(self) -> None:
        cases = {
            "missing": {"include_resources": False},
            "partial": {"resource_deltas": {"linux_fds": 0}},
        }
        for label, values in cases.items():
            with self.subTest(label=label):
                with self.assertRaisesRegex(
                    ValueError, "complete resource evidence"
                ):
                    parse_serial_log(self._one_attempt_log(**values))

    def test_complete_suite_requires_every_selected_attempt(self) -> None:
        log = "\n".join(
            (
                _event(1, "suite_start", selected_count=1),
                _event(
                    2,
                    "suite_end",
                    complete=True,
                    selected_count=1,
                    completed_count=0,
                    status_counts={},
                ),
            )
        )

        with self.assertRaisesRegex(ValueError, "complete.*selected"):
            parse_serial_log(log)

    def test_complete_suite_requires_unique_selected_attempts(self) -> None:
        test_id = "conformance/interfaces/getpid/1-1.c"
        lines = [_event(1, "suite_start", selected_count=2)]
        for seq in (2, 4):
            lines.extend(
                (
                    _event(
                        seq,
                        "test_start",
                        test_id=test_id,
                        group="base",
                        api="getpid",
                    ),
                    _event(
                        seq + 1,
                        "test_end",
                        test_id=test_id,
                        group="base",
                        api="getpid",
                        status="pass",
                        pts_status="pass",
                        launch_status="launched",
                        exit_code=0,
                        signal=None,
                        timed_out=False,
                        duration_ms=1,
                        resource_deltas=_resources(),
                    ),
                )
            )
        lines.append(
            _event(
                6,
                "suite_end",
                complete=True,
                selected_count=2,
                completed_count=2,
                status_counts={"pass": 2},
            )
        )

        with self.assertRaisesRegex(ValueError, "unique selected"):
            parse_serial_log("\n".join(lines))

    def test_complete_suite_rejects_interrupted_attempt(self) -> None:
        with self.assertRaisesRegex(ValueError, "complete.*interrupted"):
            parse_serial_log(
                self._one_attempt_log(
                    status="interrupted",
                    pts_status=None,
                    launch_status="interrupted",
                    exit_code=None,
                    infrastructure_error="runtime capture interrupted",
                )
            )

    def test_complete_suite_rejects_explicit_infrastructure_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "complete.*infrastructure error"):
            parse_serial_log(
                self._one_attempt_log(
                    status="launch-error",
                    pts_status=None,
                    launch_status="launch-error",
                    exit_code=None,
                    launch_error="launcher failed",
                    infrastructure_error="launcher cleanup failed",
                )
            )

    def test_incomplete_suite_preserves_attempt_infrastructure_evidence(self) -> None:
        cases = {
            "interrupted": {
                "pts_status": None,
                "launch_status": "interrupted",
                "exit_code": None,
                "infrastructure_error": "runtime capture interrupted",
            },
            "launch-error": {
                "pts_status": None,
                "launch_status": "launch-error",
                "exit_code": None,
                "launch_error": "launcher failed",
                "infrastructure_error": "launcher cleanup failed",
            },
        }
        for status, values in cases.items():
            with self.subTest(status=status):
                parsed = parse_serial_log(
                    self._one_attempt_log(
                        status=status,
                        terminal_complete=False,
                        **values,
                    )
                )

                self.assertFalse(parsed.complete)
                self.assertEqual(parsed.status, "incomplete")
                self.assertEqual(parsed.attempts[0].status, status)
                self.assertEqual(
                    parsed.attempts[0].infrastructure_error,
                    values["infrastructure_error"],
                )

    def test_rejects_contradictory_pass_dimensions(self) -> None:
        with self.assertRaisesRegex(ValueError, "pass dimensions"):
            parse_serial_log(
                self._one_attempt_log(launch_status="interrupted")
            )

    def test_parses_coherent_not_launched_untested_attempt(self) -> None:
        parsed = parse_serial_log(
            self._one_attempt_log(
                status="untested",
                pts_status=None,
                launch_status="not-launched",
                exit_code=None,
                signal=None,
                timed_out=False,
            )
        )

        attempt = parsed.attempts[0]
        self.assertEqual(attempt.status, "untested")
        self.assertEqual(attempt.launch_status, "not-launched")
        self.assertIsNone(attempt.pts_status)
        self.assertIsNone(attempt.exit_code)
        self.assertIsNone(attempt.signal)
        self.assertFalse(attempt.timed_out)
        self.assertIsNone(attempt.launch_error)
        self.assertIsNone(attempt.infrastructure_error)

    def test_rejects_contradictory_not_launched_dimensions(self) -> None:
        cases = {
            "exit code": {"exit_code": 5},
            "PTS result": {"pts_status": "untested"},
            "signal": {"signal": 9},
            "timeout": {"timed_out": True},
            "launch error": {"launch_error": "not executed"},
            "infrastructure error": {"infrastructure_error": "not executed"},
        }
        for label, values in cases.items():
            with self.subTest(label=label):
                dimensions: dict[str, object] = {
                    "status": "untested",
                    "pts_status": None,
                    "launch_status": "not-launched",
                    "exit_code": None,
                    "signal": None,
                    "timed_out": False,
                }
                dimensions.update(values)
                with self.assertRaisesRegex(ValueError, "untested dimensions"):
                    parse_serial_log(self._one_attempt_log(**dimensions))

    def test_rejects_contradictory_raw_status_dimensions(self) -> None:
        cases = {
            "fail": {
                "status": "fail",
                "pts_status": "fail",
                "exit_code": -1,
            },
            "PTS": {
                "status": "unsupported",
                "pts_status": "fail",
                "exit_code": 4,
            },
            "timeout": {
                "status": "timeout",
                "pts_status": None,
                "exit_code": None,
                "timed_out": False,
            },
            "crash": {
                "status": "crash",
                "pts_status": None,
                "exit_code": None,
                "signal": None,
            },
            "launch": {
                "status": "launch-error",
                "pts_status": None,
                "exit_code": None,
                "launch_status": "launched",
                "launch_error": None,
            },
            "interrupted": {
                "status": "interrupted",
                "infrastructure_error": None,
            },
        }
        for label, values in cases.items():
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, "dimensions"):
                    parse_serial_log(self._one_attempt_log(**values))


if __name__ == "__main__":
    unittest.main()
