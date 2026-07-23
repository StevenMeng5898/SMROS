from __future__ import annotations

import json
import unittest

from scripts.posix.events import EVENT_PREFIX, parse_serial_log


RUN_ID = "run-123"
MANIFEST_SHA256 = "a" * 64


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
                    resource_deltas={"linux_fds": 0, "processes": 0},
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
        self.assertEqual(attempt.resource_deltas, {"linux_fds": 0, "processes": 0})

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


if __name__ == "__main__":
    unittest.main()
