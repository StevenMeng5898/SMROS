from __future__ import annotations

import unittest

from scripts.posix.model import validate_raw_attempt_semantics


class RuntimeAttemptSemanticsTests(unittest.TestCase):
    def _validate_not_launched(self, **overrides: object) -> None:
        values: dict[str, object] = {
            "status": "untested",
            "pts_status": None,
            "launch_status": "not-launched",
            "exit_code": None,
            "signal": None,
            "timed_out": False,
            "launch_error": None,
            "infrastructure_error": None,
            "label": "test attempt",
        }
        values.update(overrides)
        validate_raw_attempt_semantics(**values)  # type: ignore[arg-type]

    def test_not_launched_untested_without_execution_evidence_is_coherent(self) -> None:
        self._validate_not_launched()

    def test_not_launched_rejects_execution_or_error_evidence(self) -> None:
        cases = {
            "exit code": {"exit_code": 5},
            "PTS result": {"pts_status": "untested"},
            "signal": {"signal": 9},
            "timeout": {"timed_out": True},
            "launch error": {"launch_error": "not executed"},
            "infrastructure error": {"infrastructure_error": "not executed"},
        }
        for label, overrides in cases.items():
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, "untested dimensions"):
                    self._validate_not_launched(**overrides)

    def test_not_launched_requires_untested_status(self) -> None:
        with self.assertRaisesRegex(ValueError, "interrupted dimensions"):
            self._validate_not_launched(
                status="interrupted",
                infrastructure_error="suite interrupted",
            )

    def test_interrupted_requires_only_infrastructure_evidence(self) -> None:
        values: dict[str, object] = {
            "status": "interrupted",
            "pts_status": None,
            "launch_status": "interrupted",
            "exit_code": None,
            "signal": None,
            "timed_out": False,
            "launch_error": None,
            "infrastructure_error": "runtime capture interrupted",
            "label": "test attempt",
        }
        validate_raw_attempt_semantics(**values)  # type: ignore[arg-type]
        cases = (
            {"launch_status": "launched"},
            {"pts_status": "pass"},
            {"exit_code": 0},
            {"signal": 9},
            {"timed_out": True},
            {"launch_error": "launcher failed"},
            {"infrastructure_error": None},
        )
        for overrides in cases:
            with self.subTest(overrides=overrides):
                contradictory = {**values, **overrides}
                with self.assertRaisesRegex(ValueError, "interrupted dimensions"):
                    validate_raw_attempt_semantics(  # type: ignore[arg-type]
                        **contradictory
                    )


if __name__ == "__main__":
    unittest.main()
