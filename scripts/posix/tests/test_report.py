from __future__ import annotations

from contextlib import redirect_stderr
from dataclasses import asdict, replace
import csv
import hashlib
from html.parser import HTMLParser
import io
import json
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

from scripts.posix import cli
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
from scripts.posix.model import BuildResult, RuntimeAttempt, SuiteTest
from scripts.posix.report import OUTPUT_NAMES, generate_report


class _StrictHTMLParser(HTMLParser):
    pass


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
        self, tests: tuple[SuiteTest, ...]
    ) -> tuple[ManifestMetadata, tuple[BuildResult, ...]]:
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
        self.stage.mkdir()
        (self.stage / "manifest.tsv").write_text(manifest_text, encoding="utf-8")
        (self.stage / "build-results.ndjson").write_text(
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
        (self.stage / "manifest.json").write_text(
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
            run_id="run-smros",
        )

    def _write_runtime(
        self,
        path: Path,
        attempts: tuple[RuntimeAttempt, ...],
        *,
        complete: bool,
    ) -> None:
        rows: list[dict[str, object]] = [
            {"record_type": "attempt", **attempt.to_dict()}
            for attempt in attempts
        ]
        rows.append(
            {
                "build_id": "5" * 64,
                "build_results_sha256": self.metadata.build_results_sha256,
                "complete": complete,
                "completed_count": len(attempts),
                "manifest_sha256": self.metadata.manifest_sha256,
                "patch_sha256": self.metadata.patch_sha256,
                "platform": "smros-aarch64",
                "record_type": "run",
                "revision": self.metadata.revision,
                "run_id": "run-smros",
                "selected_count": len(attempts),
                "smros_commit": self.metadata.smros_commit,
                "source": "smros-qemu",
                "status_counts": {
                    status: sum(attempt.status == status for attempt in attempts)
                    for status in sorted({attempt.status for attempt in attempts})
                },
            }
        )
        path.write_text(
            "".join(
                json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                for row in rows
            ),
            encoding="utf-8",
        )


class AggregationTests(ReportFixture, unittest.TestCase):
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
        self.assertEqual((completion["numerator"], completion["denominator"]), (2, 4))
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

    def test_repeated_different_outcomes_are_flaky_and_retained(self) -> None:
        attempts = (
            self._attempt(self.tests[0], "pass"),
            self._attempt(self.tests[0], "fail", exit_code=1),
        )
        repeated = self.root / "repeated.ndjson"
        self._write_runtime(repeated, attempts, complete=True)

        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(repeated,),
            output_directory=self.output,
        )

        result = next(
            item for item in summary["tests"] if item["test_id"] == self.tests[0].test_id
        )
        self.assertEqual(result["status"], "flaky")
        self.assertEqual([attempt["status"] for attempt in result["attempts"]], ["pass", "fail"])
        self.assertNotIn(self.tests[0].test_id, summary["metrics"]["program_completion"]["numerator_test_ids"])

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
        self.assertEqual(summary["metrics"]["program_completion"]["numerator"], 1)

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


class RendererTests(ReportFixture, unittest.TestCase):
    def test_all_outputs_parse_and_escape_untrusted_output(self) -> None:
        summary = generate_report(
            self.stage / "manifest.json",
            smros_results=(self.smros_results,),
            output_directory=self.output,
        )

        self.assertEqual(set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES))
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
        self.assertEqual(set(path.name for path in self.output.iterdir()), set(OUTPUT_NAMES))

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


if __name__ == "__main__":
    unittest.main()
