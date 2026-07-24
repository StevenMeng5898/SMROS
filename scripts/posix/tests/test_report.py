from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from dataclasses import asdict, replace
import csv
import hashlib
from html.parser import HTMLParser
import inspect
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
import xml.etree.ElementTree as ET

from scripts.posix import cli, report as report_module
from scripts.posix.build import (
    CHECKSUM_DEFINITION,
    EMPTY_SHA256,
    ManifestMetadata,
    _build_results_digest,
    _json_build_result,
    compile_command,
    link_command,
    nm_command,
    render_manifest,
)
from scripts.posix.model import (
    BuildResult,
    ResourceDeltas,
    RuntimeAttempt,
    SuiteTest,
)
from scripts.posix.report import OUTPUT_NAMES, generate_report


class _StrictHTMLParser(HTMLParser):
    pass


class ResourceModelTests(unittest.TestCase):
    def test_resource_deltas_are_immutable_canonical_and_bounded(self) -> None:
        deltas = ResourceDeltas.from_mapping(
            {
                "aio_requests": 3,
                "ipc_objects": 4,
                "linux_fds": 2,
                "scheduler_threads": -1,
                "timers": 5,
            }
        )

        self.assertEqual(
            deltas.to_dict(),
            {
                "aio_requests": 3,
                "ipc_objects": 4,
                "kernel_handles": 0,
                "linux_fds": 2,
                "linux_mappings": 0,
                "linux_shared_memory": 0,
                "processes": 0,
                "scheduler_threads": -1,
                "timers": 5,
            },
        )
        self.assertIsInstance(hash(deltas), int)
        for invalid in (
            {"unknown": 1},
            {"linux_fds": True},
            {"linux_fds": 2**63},
            {"linux_fds": -(2**63) - 1},
        ):
            with self.subTest(invalid=invalid):
                with self.assertRaises(ValueError):
                    ResourceDeltas.from_mapping(invalid)


class ReportFixture:
    def setUp(self) -> None:
        super().setUp()
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.stage = self.root / "stage"
        self.output = self.root / "report"
        self.tests = (
            self._test("pass-one", "complete"),
            self._test("pass-two", "complete"),
            self._test("fail-case", "complete"),
            self._test("unbuilt-case", "compile-failed"),
            self._test(
                "definition-case",
                "definition-only",
                kind="definition",
                api="unistd_h",
            ),
            self._test(
                "stub-case",
                "excluded-upstream-stub",
                group="aio",
                api="aio_read",
            ),
        )
        self.metadata, self.build_results = self._write_manifest(self.tests)
        self.smros_results = self.root / "smros-results.ndjson"
        self._write_runtime(
            self.smros_results,
            (
                self._attempt(self.tests[0], "pass", stdout="normal output\n"),
                self._attempt(self.tests[1], "pass"),
                self._attempt(
                    self.tests[2],
                    "fail",
                    stdout="<script>alert('bad')</script> & failure\n",
                    exit_code=1,
                ),
            ),
            complete=True,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()
        super().tearDown()

    def _test(
        self,
        name: str,
        disposition: str,
        *,
        kind: str = "runnable",
        group: str = "base",
        api: str = "getpid",
    ) -> SuiteTest:
        runnable = disposition == "complete"
        return SuiteTest(
            test_id=f"conformance/interfaces/{api}/{name}.c",
            group=group,
            api=api,
            kind=kind,
            disposition=disposition,
            source=f"conformance/interfaces/{api}/{name}.c",
            binary=f"bin/{name}.test" if runnable else "-",
            sha256=hashlib.sha256(name.encode("ascii")).hexdigest()
            if runnable
            else EMPTY_SHA256,
            timeout_ms=30_000,
        )

    def _write_manifest(
        self,
        tests: tuple[SuiteTest, ...],
        *,
        stage: Path | None = None,
    ) -> tuple[ManifestMetadata, tuple[BuildResult, ...]]:
        stage = self.stage if stage is None else stage
        revision = "1" * 40
        checkout = Path("target/posix/src") / revision
        results: list[BuildResult] = []
        for test in sorted(tests, key=lambda item: item.test_id):
            object_path = Path("target/posix/aarch64/obj") / f"{test.test_id}.o"
            executable = Path("target/posix/aarch64/bin") / f"{test.test_id}.test"
            compile_passed = test.disposition != "compile-failed"
            results.append(
                BuildResult(
                    test_id=test.test_id,
                    stage="compile",
                    status="passed" if compile_passed else "failed",
                    argv=tuple(
                        compile_command(
                            "aarch64-linux-gnu-gcc",
                            checkout / test.source,
                            object_path,
                            checkout / "include",
                        )
                    ),
                    returncode=0 if compile_passed else 1,
                    stdout="",
                    stderr="" if compile_passed else "compile failed",
                    duration_ms=2,
                    artifact_sha256="b" * 64 if compile_passed else None,
                )
            )
            if not compile_passed or test.kind == "definition":
                continue
            results.append(
                BuildResult(
                    test_id=test.test_id,
                    stage="nm",
                    status="passed",
                    argv=tuple(nm_command("aarch64-linux-gnu-nm", object_path)),
                    returncode=0,
                    stdout="0000000000000000 T main\n",
                    stderr="",
                    duration_ms=1,
                    artifact_sha256=None,
                )
            )
            results.append(
                BuildResult(
                    test_id=test.test_id,
                    stage="link",
                    status="passed",
                    argv=tuple(
                        link_command(
                            "aarch64-linux-gnu-gcc", object_path, executable
                        )
                    ),
                    returncode=0,
                    stdout="",
                    stderr="",
                    duration_ms=2,
                    artifact_sha256=(
                        test.sha256 if test.disposition == "complete" else "c" * 64
                    ),
                )
            )
        metadata = ManifestMetadata(
            source="https://example.invalid/posixtest.git",
            revision=revision,
            architecture="aarch64",
            compiler="aarch64-linux-gnu-gcc test",
            libc="libc.so.6:" + "2" * 64,
            patch_sha256="3" * 64,
            smros_commit="4" * 40,
            build_results_sha256=_build_results_digest(results),
        )
        manifest_text, manifest_digest = render_manifest(metadata, tests)
        metadata = replace(metadata, manifest_sha256=manifest_digest)
        stage.mkdir()
        (stage / "manifest.tsv").write_text(manifest_text, encoding="utf-8")
        (stage / "build-results.ndjson").write_text(
            "".join(_json_build_result(result) + "\n" for result in results),
            encoding="utf-8",
        )
        host = {
            "checksum_definition": CHECKSUM_DEFINITION,
            "metadata": asdict(metadata),
            "runtime": [],
            "schema": 1,
            "tests": [asdict(test) for test in sorted(tests, key=lambda item: item.test_id)],
        }
        (stage / "manifest.json").write_text(
            json.dumps(host, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        return metadata, tuple(results)

    def _attempt(
        self,
        test: SuiteTest,
        status: str,
        *,
        stdout: str = "",
        stderr: str = "",
        exit_code: int | None = 0,
        run_id: str = "run-smros",
    ) -> RuntimeAttempt:
        return RuntimeAttempt(
            test_id=test.test_id,
            group=test.group,
            api=test.api,
            platform="smros-aarch64",
            build_status="passed",
            link_status="passed",
            launch_status="launched",
            pts_status=status,
            status=status,
            exit_code=exit_code,
            signal=None,
            timed_out=False,
            duration_ms=5,
            stdout=stdout,
            stderr=stderr,
            source="smros-qemu",
            stdout_bytes=len(stdout.encode("utf-8")),
            stderr_bytes=len(stderr.encode("utf-8")),
            manifest_sha256=self.metadata.manifest_sha256,
            build_results_sha256=self.metadata.build_results_sha256,
            build_id="5" * 64,
            revision=self.metadata.revision,
            patch_sha256=self.metadata.patch_sha256,
            smros_commit=self.metadata.smros_commit,
            binary_sha256=test.sha256 or EMPTY_SHA256,
            runtime_snapshot_sha256="6" * 64,
            run_id=run_id,
            resource_evidence="measured",
        )

    def _write_runtime(
        self,
        path: Path,
        attempts: tuple[RuntimeAttempt, ...],
        *,
        complete: bool,
        metadata: ManifestMetadata | None = None,
        infrastructure_error: str | None = None,
    ) -> None:
        metadata = self.metadata if metadata is None else metadata
        rows: list[dict[str, object]] = [
            {"record_type": "attempt", **attempt.to_dict()}
            for attempt in attempts
        ]
        run_id = attempts[0].run_id if attempts else "run-smros"
        platform = attempts[0].platform if attempts else "smros-aarch64"
        source = attempts[0].source if attempts else "smros-qemu"
        terminal: dict[str, object] = {
            "build_id": "5" * 64,
            "build_results_sha256": metadata.build_results_sha256,
            "complete": complete,
            "completed_count": len(attempts),
            "manifest_sha256": metadata.manifest_sha256,
            "patch_sha256": metadata.patch_sha256,
            "platform": platform,
            "record_type": "run",
            "revision": metadata.revision,
            "run_id": run_id,
            "selected_count": len(attempts),
            "smros_commit": metadata.smros_commit,
            "source": source,
            "status_counts": {
                status: sum(attempt.status == status for attempt in attempts)
                for status in sorted({attempt.status for attempt in attempts})
            },
        }
        if infrastructure_error is not None:
            terminal["infrastructure_error"] = infrastructure_error
        rows.append(terminal)
        path.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )


class AggregationTests(ReportFixture, unittest.TestCase):
    def _rewrite_runtime_identity(
        self, path: Path, *, platform: str, source: str
    ) -> None:
        rows = [
            json.loads(line)
            for line in path.read_text(encoding="utf-8").splitlines()
        ]
        for row in rows:
            row["platform"] = platform
            row["source"] = source
        path.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

    def test_coverage_denominators_and_exclusions_are_exact(self) -> None:
        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )

        build = summary["metrics"]["build_coverage"]
        execution = summary["metrics"]["execution_coverage"]
        passing = summary["metrics"]["pass_coverage"]
        completion = summary["metrics"]["program_completion"]
        self.assertEqual((build["numerator"], build["denominator"]), (3, 4))
        self.assertEqual((execution["numerator"], execution["denominator"]), (3, 3))
        self.assertEqual((passing["numerator"], passing["denominator"]), (2, 3))
        self.assertEqual((completion["numerator"], completion["denominator"]), (3, 5))
        self.assertNotIn(
            "conformance/interfaces/aio_read/stub-case.c",
            completion["test_ids"],
        )
        self.assertIn("aio", summary["groups"])
        self.assertEqual(summary["groups"]["aio"]["counts"]["excluded_upstream_stub"], 1)
        self.assertEqual(
            summary["apis"]["getpid"]["metrics"]["program_completion"]["denominator"],
            4,
        )
        tests = {test["test_id"]: test for test in summary["tests"]}
        self.assertEqual(tests["conformance/interfaces/getpid/unbuilt-case.c"]["status"], "build-fail")
        self.assertEqual(
            tests["conformance/interfaces/aio_read/stub-case.c"]["exclusion_evidence"]["disposition"],
            "excluded-upstream-stub",
        )

    def test_not_launched_untested_attempt_is_accepted_but_not_executed(self) -> None:
        attempt = replace(
            self._attempt(self.tests[0], "untested", exit_code=None),
            launch_status="not-launched",
            pts_status=None,
        )
        self.assertFalse(report_module._attempt_executed(attempt))
        runtime = self.root / "not-launched.ndjson"
        self._write_runtime(runtime, (attempt,), complete=True)

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(runtime,),
            output_directory=self.output,
        )

        test = next(
            item for item in summary["tests"] if item["test_id"] == attempt.test_id
        )
        self.assertEqual(test["status"], "untested")
        self.assertEqual(test["smros_attempts"][0]["launch_status"], "not-launched")
        self.assertNotIn(
            attempt.test_id,
            summary["metrics"]["execution_coverage"]["numerator_test_ids"],
        )

    def test_passing_definition_contributes_to_program_completion_in_every_scope(
        self,
    ) -> None:
        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        definition_id = self.tests[4].test_id

        for completion in (
            summary["metrics"]["program_completion"],
            summary["groups"]["base"]["metrics"]["program_completion"],
            summary["apis"]["unistd_h"]["metrics"]["program_completion"],
        ):
            self.assertIn(definition_id, completion["test_ids"])
            self.assertIn(definition_id, completion["numerator_test_ids"])

    def test_failed_definition_blocks_full_program_completion_in_every_scope(
        self,
    ) -> None:
        passing = self._test(
            "definition-pass",
            "definition-only",
            kind="definition",
            api="definitions_h",
        )
        failed = self._test(
            "definition-fail",
            "compile-failed",
            kind="definition",
            api="definitions_h",
        )
        stage = self.root / "definition-stage"
        metadata, _ = self._write_manifest((passing, failed), stage=stage)
        runtime = self.root / "definition-results.ndjson"
        self._write_runtime(runtime, (), complete=True, metadata=metadata)

        summary = generate_report(
            stage / "manifest.json",
            smros_results=(runtime,),
            output_directory=self.output,
        )

        for completion in (
            summary["metrics"]["program_completion"],
            summary["groups"]["base"]["metrics"]["program_completion"],
            summary["apis"]["definitions_h"]["metrics"]["program_completion"],
        ):
            self.assertEqual(
                (completion["numerator"], completion["denominator"]),
                (1, 2),
            )
            self.assertEqual(completion["numerator_test_ids"], [passing.test_id])

    def test_repeated_different_outcomes_are_flaky_and_retained(self) -> None:
        first = self.root / "first-run.ndjson"
        second = self.root / "second-run.ndjson"
        self._write_runtime(
            first,
            (self._attempt(self.tests[0], "pass", run_id="run-first"),),
            complete=True,
        )
        self._write_runtime(
            second,
            (
                self._attempt(
                    self.tests[0], "fail", exit_code=1, run_id="run-second"
                ),
            ),
            complete=True,
        )

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(first, second),
            output_directory=self.output,
        )

        result = next(
            item for item in summary["tests"] if item["test_id"] == self.tests[0].test_id
        )
        self.assertEqual(result["status"], "flaky")
        self.assertEqual([attempt["status"] for attempt in result["attempts"]], ["pass", "fail"])
        self.assertEqual(result["duration_ms"]["runtime"], 10)
        self.assertEqual(
            [run["run_id"] for run in summary["provenance"]["smros_runs"]],
            ["run-first", "run-second"],
        )
        self.assertNotIn(self.tests[0].test_id, summary["metrics"]["program_completion"]["numerator_test_ids"])

    def test_rejects_identical_duplicate_runtime_run_identity(self) -> None:
        with self.assertRaisesRegex(
            ValueError,
            "duplicate runtime run identity.*smros-aarch64.*run-smros",
        ):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(self.smros_results, self.smros_results),
                output_directory=self.output,
            )

    def test_rejects_conflicting_duplicate_runtime_run_identity(self) -> None:
        conflicting = self.root / "conflicting-copy.ndjson"
        self._write_runtime(
            conflicting,
            (self._attempt(self.tests[0], "fail", exit_code=1),),
            complete=True,
        )

        with self.assertRaisesRegex(
            ValueError,
            "duplicate runtime run identity.*smros-aarch64.*run-smros",
        ):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(self.smros_results, conflicting),
                output_directory=self.output,
            )

    def test_incomplete_runtime_input_is_never_reported_complete(self) -> None:
        incomplete = self.root / "incomplete.ndjson"
        self._write_runtime(
            incomplete,
            (self._attempt(self.tests[0], "pass"),),
            complete=False,
        )

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(incomplete,),
            output_directory=self.output,
        )

        self.assertFalse(summary["complete"])
        self.assertEqual(summary["run_status"], "incomplete")
        self.assertEqual(summary["metrics"]["program_completion"]["numerator"], 2)

    def test_complete_runtime_rejects_interrupted_attempt(self) -> None:
        interrupted = replace(
            self._attempt(self.tests[0], "pass"),
            launch_status="interrupted",
            pts_status=None,
            status="interrupted",
            exit_code=None,
            infrastructure_error="runtime capture interrupted",
        )
        contradictory = self.root / "complete-interrupted.ndjson"
        self._write_runtime(contradictory, (interrupted,), complete=True)

        with self.assertRaisesRegex(ValueError, "complete.*interrupted"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(contradictory,),
                output_directory=self.output,
            )

    def test_complete_runtime_rejects_explicit_infrastructure_error(self) -> None:
        contradictory = self.root / "complete-infrastructure-error.ndjson"
        self._write_runtime(
            contradictory,
            (self._attempt(self.tests[0], "pass"),),
            complete=True,
            infrastructure_error="report cleanup failed",
        )

        with self.assertRaisesRegex(ValueError, "complete.*infrastructure error"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(contradictory,),
                output_directory=self.output,
            )

    def test_incomplete_runtime_preserves_infrastructure_failure_evidence(
        self,
    ) -> None:
        interrupted = replace(
            self._attempt(self.tests[0], "pass"),
            launch_status="interrupted",
            pts_status=None,
            status="interrupted",
            exit_code=None,
            infrastructure_error="runtime capture interrupted",
        )
        incomplete = self.root / "honest-infrastructure-error.ndjson"
        self._write_runtime(
            incomplete,
            (interrupted,),
            complete=False,
            infrastructure_error="report cleanup failed",
        )

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(incomplete,),
            output_directory=self.output,
        )

        self.assertFalse(summary["complete"])
        self.assertEqual(
            summary["provenance"]["smros_runs"][0]["infrastructure_error"],
            "report cleanup failed",
        )
        attempt = next(
            test["smros_attempts"][0]
            for test in summary["tests"]
            if test["test_id"] == interrupted.test_id
        )
        self.assertEqual(attempt["status"], "interrupted")
        self.assertEqual(
            attempt["infrastructure_error"], "runtime capture interrupted"
        )

    def test_aggregation_does_not_trust_a_contradictory_complete_terminal(
        self,
    ) -> None:
        interrupted = replace(
            self._attempt(self.tests[0], "pass"),
            launch_status="interrupted",
            pts_status=None,
            status="interrupted",
            exit_code=None,
            infrastructure_error="runtime capture interrupted",
        )
        incomplete = self.root / "internal-incomplete.ndjson"
        self._write_runtime(incomplete, (interrupted,), complete=False)
        manifest = report_module._load_manifest(self.stage / "manifest.json")
        source = report_module._load_runtime_results(
            incomplete,
            manifest.tests,
            manifest.build_results,
            manifest.metadata,
            role="smros",
        )
        contradictory = replace(
            source,
            terminal={**source.terminal, "complete": True},
        )

        summary = report_module._aggregate(manifest, (), (contradictory,))

        self.assertFalse(summary["complete"])
        self.assertEqual(summary["run_status"], "incomplete")

    def test_terminal_build_identity_must_match_attempts(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[-1]["build_id"] = "7" * 64
        mismatched = self.root / "mismatched-build.ndjson"
        mismatched.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "build identity mismatch"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(mismatched,),
                output_directory=self.output,
            )

    def test_pass_attempt_requires_launched_pass_dimensions(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[0]["launch_status"] = "interrupted"
        contradictory = self.root / "contradictory-pass.ndjson"
        contradictory.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "pass dimensions"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(contradictory,),
                output_directory=self.output,
            )

    def test_complete_runtime_requires_every_selected_attempt(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[-1]["selected_count"] = 4
        incomplete = self.root / "false-complete.ndjson"
        incomplete.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "complete.*selected"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(incomplete,),
                output_directory=self.output,
            )
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            result = cli.main(
                [
                    "report",
                    "--manifest",
                    str(self.stage / "manifest.json"),
                    "--smros-results",
                    str(incomplete),
                    "--out",
                    str(self.output),
                ]
            )
        self.assertEqual(result, 1)
        self.assertFalse(self.output.exists())

    def test_complete_runtime_requires_unique_selected_attempts(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        for key in ("test_id", "group", "api", "binary_sha256"):
            rows[1][key] = rows[0][key]
        duplicated = self.root / "duplicate-completion.ndjson"
        duplicated.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "unique selected"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(duplicated,),
                output_directory=self.output,
            )

    def test_runtime_attempts_reject_aggregate_only_statuses(self) -> None:
        for status in ("flaky", "build-fail", "not-built"):
            with self.subTest(status=status):
                rows = [
                    json.loads(line)
                    for line in self.smros_results.read_text(
                        encoding="utf-8"
                    ).splitlines()
                ]
                rows[0]["status"] = status
                rows[-1]["status_counts"] = {
                    "pass": 1,
                    "fail": 1,
                    status: 1,
                }
                invalid = self.root / f"aggregate-{status}.ndjson"
                invalid.write_text(
                    "".join(
                        json.dumps(row, sort_keys=True, separators=(",", ":"))
                        + "\n"
                        for row in rows
                    ),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(ValueError, "raw runtime status"):
                    generate_report(
                        self.stage / "manifest.json",
                        smros_results=(invalid,),
                        output_directory=self.output,
                    )

    def test_runtime_inputs_must_match_their_platform_role(self) -> None:
        with self.assertRaisesRegex(ValueError, "Linux-reference platform"):
            generate_report(
                self.stage / "manifest.json",
                linux_results=(self.smros_results,),
                output_directory=self.output,
            )

        linux = self.root / "linux-results.ndjson"
        linux.write_bytes(self.smros_results.read_bytes())
        self._rewrite_runtime_identity(
            linux, platform="aarch64-linux-reference", source="qemu-user"
        )
        with self.assertRaisesRegex(ValueError, "SMROS platform"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(linux,),
                output_directory=self.output,
            )

    def test_linux_rows_without_resource_field_default_to_zero(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        for row in rows:
            row["platform"] = "aarch64-linux-reference"
            row["source"] = "qemu-user"
            if row["record_type"] == "attempt":
                row.pop("resource_deltas")
                row.pop("resource_evidence")
        linux = self.root / "legacy-linux.ndjson"
        linux.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        summary = generate_report(
            self.stage / "manifest.json",
            linux_results=(linux,),
            output_directory=self.output,
        )

        self.assertEqual(
            summary["resource_deltas"], ResourceDeltas().to_dict()
        )
        self.assertEqual(summary["counts"]["leaked_resources"], 0)
        pass_result = next(
            row for row in summary["tests"] if row["test_id"] == self.tests[0].test_id
        )
        self.assertEqual(pass_result["resource_evidence"], "unavailable")

    def test_smros_rows_require_complete_resource_evidence(self) -> None:
        for label in ("missing", "partial"):
            with self.subTest(label=label):
                rows = [
                    json.loads(line)
                    for line in self.smros_results.read_text(
                        encoding="utf-8"
                    ).splitlines()
                ]
                if label == "missing":
                    rows[0].pop("resource_deltas")
                else:
                    rows[0]["resource_deltas"].pop("timers")
                invalid = self.root / f"resource-{label}.ndjson"
                invalid.write_text(
                    "".join(
                        json.dumps(row, sort_keys=True, separators=(",", ":"))
                        + "\n"
                        for row in rows
                    ),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(
                    ValueError, "complete resource evidence"
                ):
                    generate_report(
                        self.stage / "manifest.json",
                        smros_results=(invalid,),
                        output_directory=self.output,
                    )

    def test_smros_rows_require_explicit_measured_resource_evidence(self) -> None:
        for label, evidence in (("missing", None), ("unavailable", "unavailable")):
            with self.subTest(label=label):
                rows = [
                    json.loads(line)
                    for line in self.smros_results.read_text(
                        encoding="utf-8"
                    ).splitlines()
                ]
                if evidence is None:
                    rows[0].pop("resource_evidence")
                else:
                    rows[0]["resource_evidence"] = evidence
                invalid = self.root / f"resource-evidence-{label}.ndjson"
                invalid.write_text(
                    "".join(
                        json.dumps(row, sort_keys=True, separators=(",", ":"))
                        + "\n"
                        for row in rows
                    ),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(ValueError, "resource evidence"):
                    generate_report(
                        self.stage / "manifest.json",
                        smros_results=(invalid,),
                        output_directory=self.output,
                    )

    def test_attempt_dimensions_are_bound_to_manifest_and_build_results(self) -> None:
        cases = {
            "binary checksum": ("binary_sha256", "9" * 64),
            "build status": ("build_status", "failed"),
            "link status": ("link_status", "not-linked"),
        }
        for label, (field, replacement) in cases.items():
            with self.subTest(label=label):
                rows = [
                    json.loads(line)
                    for line in self.smros_results.read_text(
                        encoding="utf-8"
                    ).splitlines()
                ]
                rows[0][field] = replacement
                invalid = self.root / f"attempt-{field}.ndjson"
                invalid.write_text(
                    "".join(
                        json.dumps(row, sort_keys=True, separators=(",", ":"))
                        + "\n"
                        for row in rows
                    ),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(ValueError, label):
                    generate_report(
                        self.stage / "manifest.json",
                        smros_results=(invalid,),
                        output_directory=self.output,
                    )


class RendererTests(ReportFixture, unittest.TestCase):
    def test_raw_serial_log_flows_through_cli_and_report(self) -> None:
        test = self.tests[0]

        def event(seq: int, name: str, **values: object) -> str:
            return "SMROS_POSIX_EVENT " + json.dumps(
                {
                    "architecture": "aarch64",
                    "event": name,
                    "manifest_sha256": self.metadata.manifest_sha256,
                    "run_id": "serial-run",
                    "schema": 1,
                    "seq": seq,
                    **values,
                },
                sort_keys=True,
                separators=(",", ":"),
            )

        serial = self.root / "serial.log"
        serial.write_text(
            "\n".join(
                (
                    "kernel: ordinary boot output",
                    event(
                        1,
                        "suite_start",
                        selected_count=1,
                        build_id="5" * 64,
                        build_results_sha256=self.metadata.build_results_sha256,
                        revision=self.metadata.revision,
                        patch_sha256=self.metadata.patch_sha256,
                        smros_commit=self.metadata.smros_commit,
                    ),
                    event(
                        2,
                        "test_start",
                        test_id=test.test_id,
                        group=test.group,
                        api=test.api,
                    ),
                    "serial program output",
                    event(
                        3,
                        "test_end",
                        test_id=test.test_id,
                        group=test.group,
                        api=test.api,
                        status="pass",
                        pts_status="pass",
                        launch_status="launched",
                        exit_code=0,
                        signal=None,
                        timed_out=False,
                        duration_ms=9,
                        resource_deltas=ResourceDeltas.from_mapping(
                            {"linux_fds": 2, "timers": 1}
                        ).to_dict(),
                    ),
                    event(
                        4,
                        "suite_end",
                        complete=True,
                        selected_count=1,
                        completed_count=1,
                        status_counts={"pass": 1},
                    ),
                )
            )
            + "\n",
            encoding="utf-8",
        )

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(serial,),
            output_directory=self.output,
        )

        result = next(
            row for row in summary["tests"] if row["test_id"] == test.test_id
        )
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["attempts"][0]["stdout"], "serial program output\n")
        self.assertEqual(result["resource_deltas"]["linux_fds"], 2)
        self.assertEqual(summary["resource_deltas"]["timers"], 1)
        events = [
            json.loads(line)
            for line in (self.output / "events.ndjson").read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual([row["event"] for row in events], [
            "suite_start", "test_start", "test_end", "suite_end"
        ])
        cli_output = self.root / "cli-serial-report"
        with redirect_stdout(io.StringIO()):
            cli_result = cli.main(
                [
                    "report",
                    "--manifest",
                    str(self.stage / "manifest.json"),
                    "--smros-results",
                    str(serial),
                    "--out",
                    str(cli_output),
                ]
            )
        self.assertEqual(cli_result, 0)
        self.assertTrue((cli_output / "summary.json").is_file())

    def test_junit_replaces_xml_forbidden_controls(self) -> None:
        output = "allowed\tline\nforbidden:\x01:end\r"
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[2]["stdout"] = output
        rows[2]["stdout_bytes"] = len(output.encode("utf-8"))
        controls = self.root / "control-results.ndjson"
        controls.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        generate_report(
            self.stage / "manifest.json",
            smros_results=(controls,),
            output_directory=self.output,
        )

        junit = ET.parse(self.output / "junit.xml")
        system_output = "".join(
            node.text or "" for node in junit.findall(".//system-out")
        )
        self.assertIn("allowed\tline\n", system_output)
        self.assertIn("forbidden:\ufffd:end", system_output)
        self.assertNotIn("\x01", system_output)

    def test_markdown_escapes_links_emphasis_code_and_tables(self) -> None:
        unsafe = "[click](javascript:alert(1)) *bold* `code` | table\n"
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[2]["stdout"] = unsafe
        rows[2]["stdout_bytes"] = len(unsafe.encode("utf-8"))
        markdown_input = self.root / "markdown-results.ndjson"
        markdown_input.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        generate_report(
            self.stage / "manifest.json",
            smros_results=(markdown_input,),
            output_directory=self.output,
        )

        markdown = (self.output / "report.md").read_text(encoding="utf-8")
        escaped = (
            r"\[click\]\(javascript:alert\(1\)\) "
            r"\*bold\* \`code\` \| table"
        )
        self.assertIn(escaped, markdown)
        self.assertNotIn(unsafe.strip(), markdown)

    def test_resource_deltas_survive_every_report_layer(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[0]["resource_deltas"] = {
            "aio_requests": 3,
            "ipc_objects": 4,
            "kernel_handles": 0,
            "linux_fds": 2,
            "linux_mappings": 0,
            "linux_shared_memory": 0,
            "processes": 0,
            "scheduler_threads": 1,
            "timers": 5,
        }
        resources = self.root / "resource-results.ndjson"
        resources.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(resources,),
            output_directory=self.output,
        )

        test = next(
            row for row in summary["tests"] if row["test_id"] == self.tests[0].test_id
        )
        self.assertEqual(test["resource_deltas"]["linux_fds"], 2)
        self.assertEqual(test["resource_evidence"], "measured")
        self.assertEqual(test["attempts"][0]["resource_deltas"]["scheduler_threads"], 1)
        self.assertEqual(summary["resource_deltas"]["linux_fds"], 2)
        self.assertEqual(summary["resource_deltas"]["timers"], 5)
        self.assertEqual(summary["resource_deltas"]["aio_requests"], 3)
        self.assertEqual(summary["resource_deltas"]["ipc_objects"], 4)
        self.assertEqual(summary["resource_leaks"]["timers"], 1)
        self.assertEqual(summary["resource_leaks"]["aio_requests"], 1)
        self.assertEqual(summary["resource_leaks"]["ipc_objects"], 1)
        self.assertEqual(summary["groups"]["base"]["resource_deltas"]["scheduler_threads"], 1)
        self.assertEqual(summary["groups"]["base"]["resource_leaks"]["timers"], 1)
        self.assertEqual(summary["apis"]["getpid"]["counts"]["leaked_resources"], 1)
        events = [
            json.loads(line)
            for line in (self.output / "events.ndjson").read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(events[0]["resource_deltas"]["linux_fds"], 2)
        with (self.output / "groups.csv").open(encoding="utf-8", newline="") as stream:
            group = next(row for row in csv.DictReader(stream) if row["group"] == "base")
        self.assertEqual(group["resource_delta_linux_fds"], "2")
        self.assertEqual(group["resource_delta_timers"], "5")
        self.assertEqual(group["resource_delta_aio_requests"], "3")
        self.assertEqual(group["resource_delta_ipc_objects"], "4")
        junit = ET.parse(self.output / "junit.xml")
        properties = {
            item.attrib["name"]: item.attrib["value"]
            for item in junit.findall(".//property")
        }
        self.assertEqual(properties["resource_delta.linux_fds"], "2")
        self.assertEqual(properties["resource_delta.timers"], "5")
        self.assertEqual(properties["resource_delta.aio_requests"], "3")
        self.assertEqual(properties["resource_delta.ipc_objects"], "4")
        markdown = (self.output / "report.md").read_text(encoding="utf-8")
        html_text = (self.output / "index.html").read_text(encoding="utf-8")
        self.assertIn(r"linux\_fds=2", markdown)
        self.assertIn("linux_fds=2", html_text)

    def test_negative_cleanup_delta_is_visible_but_not_a_leak(self) -> None:
        rows = [
            json.loads(line)
            for line in self.smros_results.read_text(encoding="utf-8").splitlines()
        ]
        rows[0]["resource_deltas"]["linux_fds"] = -2
        cleanup = self.root / "cleanup-results.ndjson"
        cleanup.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(cleanup,),
            output_directory=self.output,
        )

        self.assertEqual(summary["resource_deltas"]["linux_fds"], -2)
        self.assertEqual(summary["counts"]["leaked_resources"], 0)
        self.assertEqual(summary["resource_leaks"]["linux_fds"], 0)

    def test_positive_residual_is_not_canceled_by_later_cleanup(self) -> None:
        paths: list[Path] = []
        for name, run_id, delta in (
            ("leak", "run-leak", 2),
            ("cleanup", "run-cleanup", -2),
        ):
            path = self.root / f"{name}.ndjson"
            self._write_runtime(
                path,
                (self._attempt(self.tests[0], "pass", run_id=run_id),),
                complete=True,
            )
            rows = [
                json.loads(line)
                for line in path.read_text(encoding="utf-8").splitlines()
            ]
            rows[0]["resource_deltas"]["linux_fds"] = delta
            path.write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":"))
                    + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )
            paths.append(path)

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=tuple(paths),
            output_directory=self.output,
        )

        self.assertEqual(summary["resource_deltas"]["linux_fds"], 0)
        self.assertEqual(summary["resource_leaks"]["linux_fds"], 1)
        self.assertEqual(summary["counts"]["leaked_resources"], 1)

    def test_all_outputs_parse_and_escape_untrusted_output(self) -> None:
        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )

        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )
        persisted = json.loads((self.output / "summary.json").read_text(encoding="utf-8"))
        self.assertEqual(persisted, summary)
        events = [
            json.loads(line)
            for line in (self.output / "events.ndjson").read_text(encoding="utf-8").splitlines()
        ]
        self.assertTrue(events)
        ET.parse(self.output / "junit.xml")
        for name in ("groups.csv", "apis.csv"):
            with (self.output / name).open(encoding="utf-8", newline="") as stream:
                self.assertTrue(list(csv.DictReader(stream)))
        markdown = (self.output / "report.md").read_text(encoding="utf-8")
        html = (self.output / "index.html").read_text(encoding="utf-8")
        _StrictHTMLParser().feed(html)
        self.assertNotIn("<script>alert('bad')</script>", markdown)
        self.assertNotIn("<script>alert('bad')</script>", html)
        self.assertIn("&lt;script&gt;alert", markdown)
        self.assertIn("&lt;script&gt;alert", html)
        self.assertIn("status-filter", html)
        self.assertNotIn("http://", html)
        self.assertNotIn("https://", html)

    def test_publication_replaces_a_whole_generation_and_rejects_symlink(self) -> None:
        self.output.mkdir()
        (self.output / "stale.txt").write_text("stale", encoding="utf-8")
        generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )

        outside = self.root / "outside"
        outside.mkdir()
        linked = self.root / "linked-report"
        linked.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            generate_report(
                self.stage / "manifest.json",
                smros_results=(self.smros_results,),
                output_directory=linked,
            )
        self.assertEqual(list(outside.iterdir()), [])

    def test_initial_publication_rejects_a_concurrently_created_destination(
        self,
    ) -> None:
        stat_call = report_module.os.stat
        replacement_identity: tuple[int, int] | None = None

        def create_destination_after_missing(
            path: object,
            *args: object,
            **kwargs: object,
        ) -> os.stat_result:
            nonlocal replacement_identity
            try:
                return stat_call(path, *args, **kwargs)
            except FileNotFoundError:
                parent = kwargs.get("dir_fd")
                if (
                    path == self.output.name
                    and isinstance(parent, int)
                    and replacement_identity is None
                ):
                    os.mkdir(self.output.name, 0o700, dir_fd=parent)
                    descriptor = os.open(
                        self.output.name,
                        os.O_RDONLY | os.O_DIRECTORY,
                        dir_fd=parent,
                    )
                    try:
                        info = os.fstat(descriptor)
                        replacement_identity = (info.st_dev, info.st_ino)
                    finally:
                        os.close(descriptor)
                raise

        with mock.patch.object(
            report_module.os,
            "stat",
            side_effect=create_destination_after_missing,
        ):
            with self.assertRaisesRegex(ValueError, "concurrent destination"):
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=self.output,
                )

        output_info = self.output.stat()
        self.assertEqual(
            (output_info.st_dev, output_info.st_ino), replacement_identity
        )
        self.assertEqual(list(self.output.iterdir()), [])

        generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )

    def test_completed_initial_rename_is_not_cleared_when_wrapper_raises(
        self,
    ) -> None:
        rename_noreplace = report_module._rename_noreplace

        def rename_then_raise(
            source_parent: int,
            source_name: str,
            destination_parent: int,
            destination_name: str,
        ) -> None:
            rename_noreplace(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
            )
            raise OSError("failure after completed rename")

        with mock.patch.object(
            report_module,
            "_rename_noreplace",
            side_effect=rename_then_raise,
        ):
            with self.assertRaisesRegex(OSError, "after completed rename"):
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=self.output,
                )

        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )
        work_root = (
            self.root
            / report_module._REPORT_QUARANTINE_NAME
            / report_module._report_work_slot_name(self.output.name)
            / report_module._REPORT_WORK_ROOT_NAME
        )
        self.assertEqual(list(work_root.iterdir()), [])

        generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )

    def test_completed_exchange_finalizes_slot_and_preserves_interruption(
        self,
    ) -> None:
        generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        prior_info = self.output.stat()
        prior_identity = (prior_info.st_dev, prior_info.st_ino)
        interruption = KeyboardInterrupt("failure after completed exchange")
        rename_exchange = report_module._rename_exchange
        reset_work_root = report_module._reset_report_work_root
        reset_identities: list[tuple[int, int]] = []

        def exchange_then_interrupt(
            source_parent: int,
            source_name: str,
            destination_parent: int,
            destination_name: str,
        ) -> None:
            rename_exchange(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
            )
            raise interruption

        def record_reset(slot: int, work: int) -> None:
            info = os.fstat(work)
            reset_identities.append((info.st_dev, info.st_ino))
            reset_work_root(slot, work)

        with mock.patch.object(
            report_module,
            "_rename_exchange",
            side_effect=exchange_then_interrupt,
        ), mock.patch.object(
            report_module,
            "_reset_report_work_root",
            side_effect=record_reset,
        ):
            with self.assertRaises(KeyboardInterrupt) as raised:
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=self.output,
                )

        self.assertIs(raised.exception, interruption)
        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )
        json.loads((self.output / "summary.json").read_text(encoding="utf-8"))
        ET.parse(self.output / "junit.xml")
        work_root = (
            self.root
            / report_module._REPORT_QUARANTINE_NAME
            / report_module._report_work_slot_name(self.output.name)
            / report_module._REPORT_WORK_ROOT_NAME
        )
        work_info = work_root.stat()
        self.assertEqual((work_info.st_dev, work_info.st_ino), prior_identity)
        self.assertEqual(reset_identities, [prior_identity])
        self.assertEqual(list(work_root.iterdir()), [])

        generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        self.assertEqual(
            set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES)
        )

    def test_completed_exchange_recovers_post_commit_fsync_interruptions(
        self,
    ) -> None:
        for failure_point in ("slot", "parent"):
            with self.subTest(failure_point=failure_point):
                output = self.root / f"report-{failure_point}"
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=output,
                )
                prior_info = output.stat()
                prior_identity = (prior_info.st_dev, prior_info.st_ino)
                interruption = KeyboardInterrupt(
                    f"{failure_point} fsync after completed exchange"
                )
                rename_exchange = report_module._rename_exchange
                fsync = report_module.os.fsync
                reset_work_root = report_module._reset_report_work_root
                exchange_completed = False
                interrupted = False
                reset_identities: list[tuple[int, int]] = []
                slot_path = (
                    self.root
                    / report_module._REPORT_QUARANTINE_NAME
                    / report_module._report_work_slot_name(output.name)
                )

                def exchange_then_mark(
                    source_parent: int,
                    source_name: str,
                    destination_parent: int,
                    destination_name: str,
                ) -> None:
                    nonlocal exchange_completed
                    rename_exchange(
                        source_parent,
                        source_name,
                        destination_parent,
                        destination_name,
                    )
                    exchange_completed = True

                def interrupt_target_fsync(descriptor: int) -> None:
                    nonlocal interrupted
                    descriptor_path = Path(
                        os.readlink(f"/proc/self/fd/{descriptor}")
                    )
                    target = slot_path if failure_point == "slot" else self.root
                    if (
                        exchange_completed
                        and not interrupted
                        and descriptor_path == target
                    ):
                        interrupted = True
                        raise interruption
                    fsync(descriptor)

                def record_reset(slot: int, work: int) -> None:
                    info = os.fstat(work)
                    reset_identities.append((info.st_dev, info.st_ino))
                    reset_work_root(slot, work)

                with mock.patch.object(
                    report_module,
                    "_rename_exchange",
                    side_effect=exchange_then_mark,
                ), mock.patch.object(
                    report_module.os,
                    "fsync",
                    side_effect=interrupt_target_fsync,
                ), mock.patch.object(
                    report_module,
                    "_reset_report_work_root",
                    side_effect=record_reset,
                ):
                    with self.assertRaises(KeyboardInterrupt) as raised:
                        generate_report(
                            self.stage / "manifest.json",
                            smros_results=(self.smros_results,),
                            output_directory=output,
                        )

                self.assertIs(raised.exception, interruption)
                self.assertTrue(interrupted)
                self.assertEqual(
                    set(path.name for path in output.iterdir()), set(OUTPUT_NAMES)
                )
                json.loads((output / "summary.json").read_text(encoding="utf-8"))
                ET.parse(output / "junit.xml")
                work_root = slot_path / report_module._REPORT_WORK_ROOT_NAME
                work_info = work_root.stat()
                self.assertEqual(
                    (work_info.st_dev, work_info.st_ino), prior_identity
                )
                self.assertEqual(reset_identities, [prior_identity])
                self.assertEqual(list(work_root.iterdir()), [])

                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=output,
                )
                self.assertEqual(
                    set(path.name for path in output.iterdir()), set(OUTPUT_NAMES)
                )

    def test_post_commit_assignment_interruptions_use_inode_location(
        self,
    ) -> None:
        source, first_line = inspect.getsourcelines(
            report_module._publish_generation
        )
        target_lines = {
            "noreplace": next(
                first_line + index
                for index, line in enumerate(source)
                if line == "            generated_in_slot = False\n"
            ),
            "exchange": next(
                first_line + index
                for index, line in enumerate(source)
                if line == "                generated_in_slot = False\n"
            ),
        }

        for operation in ("noreplace", "exchange"):
            with self.subTest(operation=operation):
                output = self.root / f"assignment-{operation}"
                if operation == "exchange":
                    generate_report(
                        self.stage / "manifest.json",
                        smros_results=(self.smros_results,),
                        output_directory=output,
                    )
                rename = getattr(report_module, f"_rename_{operation}")
                interruption = KeyboardInterrupt(
                    f"{operation} normal-return assignment"
                )
                armed = False
                triggered = False

                def rename_then_arm(
                    source_parent: int,
                    source_name: str,
                    destination_parent: int,
                    destination_name: str,
                ) -> None:
                    nonlocal armed
                    rename(
                        source_parent,
                        source_name,
                        destination_parent,
                        destination_name,
                    )
                    armed = True

                def interrupt_assignment(
                    frame: object,
                    event: str,
                    argument: object,
                ) -> object:
                    del argument
                    nonlocal triggered
                    if (
                        event == "line"
                        and armed
                        and getattr(frame, "f_code", None)
                        is report_module._publish_generation.__code__
                        and getattr(frame, "f_lineno", None)
                        == target_lines[operation]
                    ):
                        triggered = True
                        sys.settrace(None)
                        raise interruption
                    return interrupt_assignment

                previous_trace = sys.gettrace()
                with mock.patch.object(
                    report_module,
                    f"_rename_{operation}",
                    side_effect=rename_then_arm,
                ):
                    try:
                        sys.settrace(interrupt_assignment)
                        with self.assertRaises(KeyboardInterrupt) as raised:
                            generate_report(
                                self.stage / "manifest.json",
                                smros_results=(self.smros_results,),
                                output_directory=output,
                            )
                    finally:
                        sys.settrace(previous_trace)

                self.assertTrue(triggered)
                self.assertIs(raised.exception, interruption)
                self.assertEqual(
                    set(path.name for path in output.iterdir()), set(OUTPUT_NAMES)
                )
                json.loads((output / "summary.json").read_text(encoding="utf-8"))
                ET.parse(output / "junit.xml")
                work_root = (
                    self.root
                    / report_module._REPORT_QUARANTINE_NAME
                    / report_module._report_work_slot_name(output.name)
                    / report_module._REPORT_WORK_ROOT_NAME
                )
                self.assertEqual(list(work_root.iterdir()), [])

                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=output,
                )
                self.assertEqual(
                    set(path.name for path in output.iterdir()), set(OUTPUT_NAMES)
                )

    def test_publication_does_not_unlink_a_post_validation_replacement(
        self,
    ) -> None:
        self.output.mkdir()
        (self.output / "prior.txt").write_text("prior", encoding="utf-8")
        entry_matches = report_module._directory_entry_matches
        published = False
        replacement_identity: tuple[int, int] | None = None
        replacement_path: Path | None = None

        def replace_after_empty_identity_check(
            parent: int,
            name: str,
            descriptor: int,
        ) -> bool:
            nonlocal published, replacement_identity, replacement_path
            result = entry_matches(parent, name, descriptor)
            if (
                result
                and name == self.output.name
                and set(os.listdir(descriptor)) == set(OUTPUT_NAMES)
            ):
                published = True
            elif (
                result
                and published
                and replacement_identity is None
                and not os.listdir(descriptor)
            ):
                parent_path = Path(os.readlink(f"/proc/self/fd/{parent}"))
                os.rename(
                    name,
                    "owned-report-moved",
                    src_dir_fd=parent,
                    dst_dir_fd=parent,
                )
                os.mkdir(name, 0o700, dir_fd=parent)
                replacement = os.open(
                    name,
                    os.O_RDONLY | os.O_DIRECTORY,
                    dir_fd=parent,
                )
                try:
                    info = os.fstat(replacement)
                    replacement_identity = (info.st_dev, info.st_ino)
                finally:
                    os.close(replacement)
                replacement_path = parent_path / name
            return result

        with mock.patch.object(
            report_module,
            "_directory_entry_matches",
            side_effect=replace_after_empty_identity_check,
        ):
            with self.assertRaisesRegex(ValueError, "work root changed during cleanup"):
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=self.output,
                )

        self.assertIsNotNone(replacement_path)
        assert replacement_path is not None
        replacement_info = replacement_path.stat()
        self.assertEqual(
            (replacement_info.st_dev, replacement_info.st_ino),
            replacement_identity,
        )
        self.assertEqual(list(replacement_path.iterdir()), [])
        self.assertEqual(set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES))

    def test_publication_rolls_back_raced_destination_without_deleting_it(self) -> None:
        self.output.mkdir()
        (self.output / "prior.txt").write_text("prior", encoding="utf-8")
        prior_name = "validated-prior-report"
        real_exchange = report_module._rename_exchange
        replacement_identity: tuple[int, int] | None = None
        exchange_count = 0

        def race_then_exchange(
            source_parent: int,
            first: str,
            destination_parent: int,
            second: str,
        ) -> None:
            nonlocal exchange_count, replacement_identity
            exchange_count += 1
            if exchange_count == 1:
                os.rename(
                    second,
                    prior_name,
                    src_dir_fd=destination_parent,
                    dst_dir_fd=destination_parent,
                )
                os.mkdir(second, 0o700, dir_fd=destination_parent)
                replacement = self.output / "nested"
                replacement.mkdir()
                (replacement / "keep.txt").write_text(
                    "replacement", encoding="utf-8"
                )
                info = self.output.stat()
                replacement_identity = (info.st_dev, info.st_ino)
            real_exchange(source_parent, first, destination_parent, second)

        with mock.patch.object(
            report_module, "_rename_exchange", side_effect=race_then_exchange
        ):
            with self.assertRaisesRegex(ValueError, "changed during publication"):
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=self.output,
                )

        output_info = self.output.stat()
        self.assertEqual(
            (output_info.st_dev, output_info.st_ino), replacement_identity
        )
        self.assertEqual(
            (self.output / "nested/keep.txt").read_text(encoding="utf-8"),
            "replacement",
        )
        self.assertEqual(
            (self.root / prior_name / "prior.txt").read_text(encoding="utf-8"),
            "prior",
        )
        self.assertEqual(exchange_count, 2)
        self.assertEqual(list(self.root.glob(".report.*.tmp")), [])

        generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )
        self.assertEqual(set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES))

    def test_publication_reports_rollback_failure_without_deleting_replacement(
        self,
    ) -> None:
        self.output.mkdir()
        (self.output / "prior.txt").write_text("prior", encoding="utf-8")
        prior_name = "validated-prior-report"
        real_exchange = report_module._rename_exchange
        replacement_identity: tuple[int, int] | None = None
        exchange_count = 0

        def race_then_fail_rollback(
            source_parent: int,
            first: str,
            destination_parent: int,
            second: str,
        ) -> None:
            nonlocal exchange_count, replacement_identity
            exchange_count += 1
            if exchange_count == 1:
                os.rename(
                    second,
                    prior_name,
                    src_dir_fd=destination_parent,
                    dst_dir_fd=destination_parent,
                )
                os.mkdir(second, 0o700, dir_fd=destination_parent)
                replacement = self.output / "nested"
                replacement.mkdir()
                (replacement / "keep.txt").write_text(
                    "replacement", encoding="utf-8"
                )
                info = self.output.stat()
                replacement_identity = (info.st_dev, info.st_ino)
                real_exchange(source_parent, first, destination_parent, second)
                return
            raise OSError("rollback refused")

        with mock.patch.object(
            report_module,
            "_rename_exchange",
            side_effect=race_then_fail_rollback,
        ):
            with self.assertRaises(ExceptionGroup) as raised:
                generate_report(
                    self.stage / "manifest.json",
                    smros_results=(self.smros_results,),
                    output_directory=self.output,
                )

        failures = raised.exception.exceptions
        self.assertEqual(len(failures), 2)
        self.assertRegex(str(failures[0]), "changed during publication")
        self.assertRegex(str(failures[1]), "rollback refused")
        self.assertEqual(exchange_count, 2)
        work_root = (
            self.root
            / report_module._REPORT_QUARANTINE_NAME
            / report_module._report_work_slot_name(self.output.name)
            / report_module._REPORT_WORK_ROOT_NAME
        )
        temporary_info = work_root.stat()
        self.assertEqual(
            (temporary_info.st_dev, temporary_info.st_ino), replacement_identity
        )
        self.assertEqual(
            (work_root / "nested/keep.txt").read_text(encoding="utf-8"),
            "replacement",
        )
        self.assertEqual(
            (self.root / prior_name / "prior.txt").read_text(encoding="utf-8"),
            "prior",
        )


class CliTests(ReportFixture, unittest.TestCase):
    def test_report_parser_registers_all_inputs(self) -> None:
        arguments = cli.create_parser().parse_args(
            [
                "report",
                "--manifest",
                str(self.stage / "manifest.json"),
                "--linux-results",
                "linux.ndjson",
                "--smros-results",
                "smros.ndjson",
                "--out",
                "report",
            ]
        )
        self.assertEqual(arguments.command, "report")
        self.assertEqual(arguments.linux_results, [Path("linux.ndjson")])
        self.assertEqual(arguments.smros_results, [Path("smros.ndjson")])

    def test_report_requires_at_least_one_runtime_input(self) -> None:
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            result = cli.main(
                [
                    "report",
                    "--manifest",
                    str(self.stage / "manifest.json"),
                    "--out",
                    str(self.output),
                ]
            )
        self.assertEqual(result, 1)
        self.assertIn("at least one runtime-result input", stderr.getvalue())

    def test_report_cli_rejects_cross_wired_runtime_role(self) -> None:
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            result = cli.main(
                [
                    "report",
                    "--manifest",
                    str(self.stage / "manifest.json"),
                    "--linux-results",
                    str(self.smros_results),
                    "--out",
                    str(self.output),
                ]
            )
        self.assertEqual(result, 1)
        self.assertIn("Linux-reference platform", stderr.getvalue())
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
