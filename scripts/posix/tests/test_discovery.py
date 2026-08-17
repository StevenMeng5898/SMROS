import contextlib
import io
import tempfile
import unittest
from collections import Counter
from pathlib import Path
from unittest import mock

from scripts.posix import cli
from scripts.posix.discovery import (
    STUB_DISPOSITIONS,
    SHELL_DISPOSITIONS,
    ReviewEntry,
    api_group,
    apply_reviews,
    audit_reviews,
    discover_shell_candidates,
    discover_shell_files,
    discover_stub_candidates,
    discover_tests,
    load_review,
    write_candidates,
)


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
SHELL_REVIEW_PATH = (
    REPOSITORY_ROOT / "third_party" / "posixtest" / "shell-review.tsv"
)


class DiscoveryFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.write_source(
            "conformance/interfaces/mmap/1-1.c",
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/definitions/aio_h/1-1.c",
            "int declaration;\n",
        )
        self.write_source(
            "conformance/definitions/sys/mman_h/2-1.c",
            "int declaration;\n",
        )
        for relative_path in (
            "conformance/behavior/WIFEXITED/1-1.c",
            "conformance/behavior/WIFEXITED/1-2.c",
            "conformance/behavior/WIFEXITED/1-3.c",
            "conformance/behavior/timers/1-1.c",
            "conformance/behavior/timers/2-1.c",
        ):
            self.write_source(relative_path, "int main(void) { return 0; }\n")
        self.write_source(
            "conformance/interfaces/mq_open/2-1.c",
            "int main(void) {\n"
            "    return PTS_UNTESTED;\n"
            "}\n",
        )
        self.write_source(
            "conformance/interfaces/pthread_create/3-1.c",
            "int main(int argc, char **argv) {\n"
            "    if (argc > 1)\n"
            "        return PTS_UNTESTED;\n"
            "    return 0;\n"
            "}\n",
        )
        self.write_source(
            "conformance/interfaces/sigaction/4-1-buildonly.c",
            "int declaration;\n",
        )
        self.write_source(
            "conformance/interfaces/time/5-1.c",
            "/* return PTS_UNTESTED; */\n"
            'const char *message = "PTS_UNTESTED";\n'
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/time/7-1.c",
            "#ifndef PTS_UNTESTED\n"
            "#define PTS_UNTESTED 5\n"
            "#endif\n"
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/time/8-1.c",
            "#if 0\n"
            "int disabled(void) { return PTS_UNTESTED; }\n"
            "#endif\n"
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/time/9-1.c",
            "#if 1\n"
            "#define ACTIVE_CONFIGURATION 1\n"
            "#elif CONFIG_OPTION\n"
            "int unreachable(void) { return PTS_UNTESTED; }\n"
            "#endif\n"
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/mmap/helper.c",
            "int helper(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/mmap/1.c",
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "outside/conformance/interfaces/mmap/9-9.c",
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/mmap/run.sh",
            "#!/bin/sh\n./1-1.run-test\n",
        )
        self.write_source(
            "conformance/interfaces/pthread_create/nested/cln.sh",
            "#!/bin/sh\nrm -f generated\n",
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_source(self, relative_path: str, contents: str) -> Path:
        path = self.root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8", newline="\n")
        return path

    def write_review(self, name: str, rows: list[tuple[str, str, str]]) -> Path:
        path = self.root / name
        text = "path\tdisposition\treason\n" + "".join(
            f"{review_path}\t{disposition}\t{reason}\n"
            for review_path, disposition, reason in rows
        )
        path.write_text(text, encoding="utf-8", newline="\n")
        return path


class TestDiscovery(DiscoveryFixture):
    def test_discovers_buildable_sources_with_stable_posix_ids(self) -> None:
        tests = discover_tests(self.root)

        self.assertEqual(
            [test.test_id for test in tests],
            sorted(
                [
                    "conformance/behavior/WIFEXITED/1-1.c",
                    "conformance/behavior/WIFEXITED/1-2.c",
                    "conformance/behavior/WIFEXITED/1-3.c",
                    "conformance/behavior/timers/1-1.c",
                    "conformance/behavior/timers/2-1.c",
                    "conformance/definitions/aio_h/1-1.c",
                    "conformance/definitions/sys/mman_h/2-1.c",
                    "conformance/interfaces/mmap/1-1.c",
                    "conformance/interfaces/mq_open/2-1.c",
                    "conformance/interfaces/pthread_create/3-1.c",
                    "conformance/interfaces/sigaction/4-1-buildonly.c",
                    "conformance/interfaces/time/5-1.c",
                    "conformance/interfaces/time/7-1.c",
                    "conformance/interfaces/time/8-1.c",
                    "conformance/interfaces/time/9-1.c",
                ]
            ),
        )
        self.assertTrue(all("\\" not in test.test_id for test in tests))
        self.assertEqual([test.source for test in tests], [test.test_id for test in tests])

    def test_classifies_interfaces_definitions_and_buildonly_sources(self) -> None:
        tests = {test.test_id: test for test in discover_tests(self.root)}

        mmap = tests["conformance/interfaces/mmap/1-1.c"]
        self.assertEqual((mmap.api, mmap.group, mmap.kind), ("mmap", "memory", "runnable"))
        aio = tests["conformance/definitions/aio_h/1-1.c"]
        self.assertEqual((aio.api, aio.group, aio.kind), ("aio_h", "aio", "definition"))
        nested = tests["conformance/definitions/sys/mman_h/2-1.c"]
        self.assertEqual((nested.api, nested.group), ("sys/mman_h", "base"))
        buildonly = tests["conformance/interfaces/sigaction/4-1-buildonly.c"]
        self.assertEqual((buildonly.api, buildonly.group, buildonly.kind), ("sigaction", "signals", "definition"))

    def test_behavior_sources_use_their_parent_directory_as_api(self) -> None:
        tests = {test.test_id: test for test in discover_tests(self.root)}
        expected = {
            "conformance/behavior/WIFEXITED/1-1.c": "WIFEXITED",
            "conformance/behavior/WIFEXITED/1-2.c": "WIFEXITED",
            "conformance/behavior/WIFEXITED/1-3.c": "WIFEXITED",
            "conformance/behavior/timers/1-1.c": "timers",
            "conformance/behavior/timers/2-1.c": "timers",
        }

        for test_id, api in expected.items():
            with self.subTest(test_id=test_id):
                test = tests[test_id]
                self.assertEqual((test.api, test.group, test.kind), (api, "base", "runnable"))

    def test_cpu_clock_syscall_volume_tests_have_reviewed_timeouts(self) -> None:
        self.write_source(
            "conformance/interfaces/clock/1-1.c",
            "int main(void) { return 0; }\n",
        )
        self.write_source(
            "conformance/interfaces/clock_gettime/4-1.c",
            "int main(void) { return 0; }\n",
        )

        tests = {test.test_id: test for test in discover_tests(self.root)}

        self.assertEqual(
            tests["conformance/interfaces/clock/1-1.c"].timeout_ms,
            180_000,
        )
        self.assertEqual(
            tests["conformance/interfaces/clock_gettime/4-1.c"].timeout_ms,
            60_000,
        )
        self.assertEqual(tests["conformance/interfaces/mmap/1-1.c"].timeout_ms, 30_000)

    def test_api_group_uses_the_approved_mapping(self) -> None:
        expected = {
            "pthread_create": "threads",
            "mq_open": "message-queues",
            "sem_wait": "semaphores",
            "aio_read": "aio",
            "lio_listio": "aio",
            "sched_yield": "scheduling",
            "sigaction": "signals",
            "kill": "signals",
            "clock_gettime": "time",
            "timer_create": "time",
            "nanosleep": "time",
            "mmap": "memory",
            "shm_unlink": "memory",
            "close": "base",
            "aio_h": "aio",
            "pthread_h": "threads",
            "mqueue_h": "base",
            "semaphore_h": "base",
            "sched_h": "scheduling",
            "signal_h": "signals",
            "time_h": "base",
            "sys/mman_h": "base",
            "sys/shm_h": "base",
            "unistd_h": "base",
        }

        for api, group in expected.items():
            with self.subTest(api=api):
                self.assertEqual(api_group(api), group)

    def test_stub_candidates_only_include_executable_c_references(self) -> None:
        tests = discover_tests(self.root)

        candidates = discover_stub_candidates(self.root, tests)

        self.assertEqual(
            [candidate.path for candidate in candidates],
            [
                "conformance/interfaces/mq_open/2-1.c",
                "conformance/interfaces/pthread_create/3-1.c",
            ],
        )
        self.assertTrue(all("PTS_UNTESTED" in candidate.evidence for candidate in candidates))

    def test_stub_discovery_accepts_non_utf8_upstream_source(self) -> None:
        path = self.root / "conformance/interfaces/mmap/6-1.c"
        path.write_bytes(
            b"/* ISO-8859 prose: \xa9 */\n"
            b"int main(void) { return PTS_UNTESTED; }\n"
        )

        candidates = discover_stub_candidates(self.root, discover_tests(self.root))

        self.assertIn(
            "conformance/interfaces/mmap/6-1.c",
            [candidate.path for candidate in candidates],
        )

    def test_rejects_unsafe_non_stub_source_paths(self) -> None:
        relative_paths = (
            "conformance/interfaces/mmap/6-1\\spoof.c",
            "conformance/interfaces/mmap/7-1\x01.c",
            "conformance/interfaces/mmap/8-1\u202e.c",
            "conformance/interfaces/bad\u202e/9-1.c",
        )

        for relative_path in relative_paths:
            with self.subTest(relative_path=relative_path):
                path = self.write_source(
                    relative_path, "int main(void) { return 0; }\n"
                )
                try:
                    with self.assertRaisesRegex(ValueError, "invalid.*path"):
                        discover_tests(self.root)
                finally:
                    path.unlink()

    def test_rejects_ordinary_non_ascii_source_paths(self) -> None:
        relative_path = "conformance/interfaces/mmap/10-\u00e9.c"
        self.write_source(relative_path, "int main(void) { return 0; }\n")

        with self.assertRaisesRegex(ValueError, "invalid.*path"):
            discover_tests(self.root)

    def test_inventories_every_shell_file_deterministically(self) -> None:
        expected = [
            "conformance/interfaces/mmap/run.sh",
            "conformance/interfaces/pthread_create/nested/cln.sh",
        ]

        self.assertEqual(list(discover_shell_files(self.root)), expected)
        self.assertEqual(
            [candidate.path for candidate in discover_shell_candidates(self.root)],
            expected,
        )


class TestReviewParsing(DiscoveryFixture):
    def test_loads_review_and_preserves_reason(self) -> None:
        path = self.write_review(
            "stub.tsv",
            [("conformance/interfaces/mq_open/2-1.c", "exclude-stub", "always returns PTS_UNTESTED")],
        )

        reviews = load_review(path, STUB_DISPOSITIONS)

        self.assertEqual(reviews["conformance/interfaces/mq_open/2-1.c"].reason, "always returns PTS_UNTESTED")

    def test_rejects_duplicate_review_paths(self) -> None:
        path = self.write_review(
            "stub.tsv",
            [
                ("conformance/interfaces/mq_open/2-1.c", "exclude-stub", "first"),
                ("conformance/interfaces/mq_open/2-1.c", "runtime-path", "second"),
            ],
        )

        with self.assertRaisesRegex(ValueError, "duplicate.*path"):
            load_review(path, STUB_DISPOSITIONS)

    def test_rejects_unknown_disposition_and_empty_reason(self) -> None:
        invalid_rows = (
            ("unknown", "reviewed", "evidence"),
            ("empty", "runtime-path", "   "),
        )

        for name, disposition, reason in invalid_rows:
            with self.subTest(name=name):
                path = self.write_review(
                    f"{name}.tsv",
                    [("conformance/interfaces/mq_open/2-1.c", disposition, reason)],
                )
                with self.assertRaises(ValueError):
                    load_review(path, STUB_DISPOSITIONS)

    def test_shell_review_classifies_assertion_drivers_as_tests(self) -> None:
        reviews = load_review(SHELL_REVIEW_PATH, SHELL_DISPOSITIONS)
        assertion_drivers = {
            "conformance/interfaces/sigaddset/1-1.sh",
            "conformance/interfaces/sigaddset/1-2.sh",
            "conformance/interfaces/sigaddset/4-1.sh",
            "conformance/interfaces/sigaddset/4-2.sh",
            "conformance/interfaces/sigaddset/4-3.sh",
            "conformance/interfaces/sigaddset/4-4.sh",
            "conformance/interfaces/sigdelset/1-1.sh",
            "conformance/interfaces/sigdelset/1-2.sh",
            "conformance/interfaces/sigdelset/4-1.sh",
            "conformance/interfaces/sigdelset/4-2.sh",
            "conformance/interfaces/sigdelset/4-3.sh",
            "conformance/interfaces/sigdelset/4-4.sh",
            "conformance/interfaces/sighold/3-1.sh",
            "conformance/interfaces/sighold/3-2.sh",
            "conformance/interfaces/sighold/3-3.sh",
            "conformance/interfaces/sighold/3-4.sh",
            "conformance/interfaces/sigignore/5-1.sh",
            "conformance/interfaces/sigignore/5-2.sh",
            "conformance/interfaces/sigignore/5-3.sh",
            "conformance/interfaces/sigignore/5-4.sh",
            "conformance/interfaces/sigismember/5-1.sh",
            "conformance/interfaces/sigismember/5-2.sh",
            "conformance/interfaces/sigismember/5-3.sh",
            "conformance/interfaces/sigismember/5-4.sh",
            "conformance/interfaces/sigprocmask/17-1.sh",
            "conformance/interfaces/sigprocmask/17-2.sh",
            "conformance/interfaces/sigprocmask/17-3.sh",
            "conformance/interfaces/sigprocmask/17-4.sh",
            "conformance/interfaces/sigrelse/3-1.sh",
            "conformance/interfaces/sigrelse/3-2.sh",
            "conformance/interfaces/sigrelse/3-3.sh",
            "conformance/interfaces/sigrelse/3-4.sh",
        }
        cleanup_helpers = {
            "conformance/interfaces/sem_close/cln.sh",
            "conformance/interfaces/sem_destroy/cln.sh",
            "conformance/interfaces/sem_getvalue/cln.sh",
            "conformance/interfaces/sem_open/cln.sh",
            "conformance/interfaces/sem_post/cln.sh",
            "conformance/interfaces/sem_unlink/cln.sh",
            "conformance/interfaces/sem_wait/cln.sh",
        }

        self.assertEqual(
            Counter(review.disposition for review in reviews.values()),
            Counter({"test": 169, "helper": 7}),
        )
        for path in assertion_drivers:
            with self.subTest(driver=path):
                self.assertEqual(reviews[path].disposition, "test")
                self.assertEqual(
                    reviews[path].reason,
                    "shell driver executes a build-only C assertion case and "
                    "forwards its result",
                )
        for path in cleanup_helpers:
            with self.subTest(helper=path):
                self.assertEqual(reviews[path].disposition, "helper")

    def test_rejects_invalid_paths_and_non_lf_or_control_data(self) -> None:
        invalid_data = (
            b"path\tdisposition\treason\r\n",
            b"path\tdisposition\treason\n../escape.c\truntime-path\treason\n",
            b"path\tdisposition\treason\nconformance\\bad.c\truntime-path\treason\n",
            b"path\tdisposition\treason\nconformance/bad\x01.c\truntime-path\treason\n",
            b"path\tdisposition\treason\nconformance/bad.c\truntime-path\trea\x00son\n",
            b"path\tdisposition\treason\xe2\x80\xa8conformance/bad.c\truntime-path\treason\n",
        )

        for index, data in enumerate(invalid_data):
            with self.subTest(index=index):
                path = self.root / f"invalid-{index}.tsv"
                path.write_bytes(data)
                with self.assertRaises(ValueError):
                    load_review(path, STUB_DISPOSITIONS)

    def test_rejects_unicode_format_controls_in_review_paths_and_reasons(self) -> None:
        invalid_rows = (
            "conformance/interfaces/mmap/1-1\u202e.c\truntime-path\treason",
            "conformance/interfaces/mmap/1-1.c\truntime-path\tspoof\u202e reason",
        )

        for index, row in enumerate(invalid_rows):
            with self.subTest(index=index):
                path = self.root / f"format-control-{index}.tsv"
                path.write_bytes(
                    ("path\tdisposition\treason\n" + row + "\n").encode("utf-8")
                )
                with self.assertRaises(ValueError):
                    load_review(path, STUB_DISPOSITIONS)

    def test_rejects_wrong_header_and_non_utf8(self) -> None:
        for name, data in (
            ("header", b"path\tresult\treason\n"),
            ("encoding", b"path\tdisposition\treason\n\xff"),
        ):
            with self.subTest(name=name):
                path = self.root / f"{name}.tsv"
                path.write_bytes(data)
                with self.assertRaises(ValueError):
                    load_review(path, STUB_DISPOSITIONS)


class TestReviewCompleteness(DiscoveryFixture):
    def stub_candidates(self):
        tests = discover_tests(self.root)
        return tests, discover_stub_candidates(self.root, tests)

    def test_missing_stub_review_is_rejected(self) -> None:
        tests, stub_candidates = self.stub_candidates()
        shells = discover_shell_candidates(self.root)

        with self.assertRaisesRegex(ValueError, "missing stub review"):
            apply_reviews(tests, stub_candidates, {}, shells, {})

    def test_missing_shell_review_is_rejected(self) -> None:
        tests, stub_candidates = self.stub_candidates()
        stub_reviews = {
            candidate.path: ReviewEntry(
                candidate.path, "runtime-path", "conditional"
            )
            for candidate in stub_candidates
        }
        shells = discover_shell_candidates(self.root)

        with self.assertRaisesRegex(ValueError, "missing shell review"):
            apply_reviews(tests, stub_candidates, stub_reviews, shells, {})

    def test_stale_stub_and_shell_rows_are_rejected(self) -> None:
        tests, stub_candidates = self.stub_candidates()
        shells = discover_shell_candidates(self.root)
        complete_stubs = {
            candidate.path: ReviewEntry(
                candidate.path, "runtime-path", "conditional"
            )
            for candidate in stub_candidates
        }
        complete_shells = {
            candidate.path: ReviewEntry(candidate.path, "test", "assertion")
            for candidate in shells
        }

        for label, stub_reviews, shell_reviews in (
            (
                "stub",
                {
                    **complete_stubs,
                    "conformance/interfaces/stale/1-1.c": ReviewEntry(
                        "conformance/interfaces/stale/1-1.c",
                        "runtime-path",
                        "stale",
                    ),
                },
                complete_shells,
            ),
            (
                "shell",
                complete_stubs,
                {
                    **complete_shells,
                    "conformance/interfaces/stale/run.sh": ReviewEntry(
                        "conformance/interfaces/stale/run.sh", "test", "stale"
                    ),
                },
            ),
        ):
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, f"stale {label} review"):
                    apply_reviews(tests, stub_candidates, stub_reviews, shells, shell_reviews)

    def test_applies_reviewed_dispositions_without_inference(self) -> None:
        tests, stub_candidates = self.stub_candidates()
        shells = discover_shell_candidates(self.root)
        stub_review_path = self.write_review(
            "stub.tsv",
            [
                (stub_candidates[0].path, "exclude-stub", "unconditional return"),
                (stub_candidates[1].path, "runtime-path", "conditional branch"),
            ],
        )
        shell_review_path = self.write_review(
            "shell.tsv",
            [
                (shells[0].path, "test", "runs assertion binary"),
                (shells[1].path, "helper", "cleanup only"),
            ],
        )

        result = apply_reviews(
            tests,
            stub_candidates,
            load_review(stub_review_path, STUB_DISPOSITIONS),
            shells,
            load_review(shell_review_path, SHELL_DISPOSITIONS),
        )

        dispositions = {test.test_id: test.disposition for test in result.tests}
        self.assertEqual(dispositions[stub_candidates[0].path], "excluded-upstream-stub")
        self.assertEqual(dispositions[stub_candidates[1].path], "complete")
        self.assertEqual(result.stub_reviews[stub_candidates[1].path].reason, "conditional branch")
        self.assertEqual(result.shell_reviews[shells[1].path].disposition, "helper")


class TestAudit(DiscoveryFixture):
    def test_candidate_files_are_deterministic_and_include_evidence(self) -> None:
        tests = discover_tests(self.root)
        stub_candidates = discover_stub_candidates(self.root, tests)
        shell_candidates = discover_shell_candidates(self.root)
        output = self.root / "review"

        write_candidates(output, stub_candidates, shell_candidates)
        first = (output / "stub-candidates.tsv").read_bytes(), (output / "shell-candidates.tsv").read_bytes()
        write_candidates(output, tuple(reversed(stub_candidates)), tuple(reversed(shell_candidates)))
        second = (output / "stub-candidates.tsv").read_bytes(), (output / "shell-candidates.tsv").read_bytes()

        self.assertEqual(first, second)
        self.assertTrue(first[0].startswith(b"path\tevidence\n"))
        self.assertIn(b"PTS_UNTESTED", first[0])
        self.assertTrue(first[1].startswith(b"path\tevidence\n"))

    def test_audit_rejects_incomplete_committed_reviews_after_writing_candidates(self) -> None:
        stub_review = self.write_review("stub.tsv", [])
        shell_review = self.write_review("shell.tsv", [])
        output = self.root / "review"

        with self.assertRaisesRegex(ValueError, "missing stub review"):
            audit_reviews(self.root, stub_review, shell_review, write_directory=output)

        self.assertTrue((output / "stub-candidates.tsv").is_file())
        self.assertTrue((output / "shell-candidates.tsv").is_file())


class TestAuditCli(unittest.TestCase):
    def test_parser_requires_exactly_one_audit_action(self) -> None:
        parser = cli.create_parser()

        for argv in (["audit"], ["audit", "--check", "--write-candidates", "out"]):
            with self.subTest(argv=argv), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    parser.parse_args(argv)

        arguments = parser.parse_args(["audit", "--check", "--work-dir", "custom"])
        self.assertEqual(arguments.command, "audit")
        self.assertTrue(arguments.check)
        self.assertEqual(arguments.work_dir, Path("custom"))

    @mock.patch("scripts.posix.cli.audit_reviews")
    @mock.patch("scripts.posix.cli.fetch_checkout")
    @mock.patch("scripts.posix.cli.load_source_lock")
    def test_audit_dispatches_to_the_pinned_checkout(
        self,
        load_source_lock: mock.Mock,
        fetch_checkout: mock.Mock,
        audit: mock.Mock,
    ) -> None:
        lock = mock.Mock(revision="a" * 40)
        load_source_lock.return_value = lock
        audit.return_value.format_counts.return_value = "C=2 shell=1"
        output = Path("candidate-output")

        with contextlib.redirect_stdout(io.StringIO()):
            result = cli.main(
                ["audit", "--write-candidates", str(output), "--work-dir", "custom"]
            )

        self.assertEqual(result, 0)
        checkout = Path("custom") / "src" / lock.revision
        fetch_checkout.assert_called_once_with(lock, checkout, cli.PATCH_SERIES_PATH)
        audit.assert_called_once_with(
            checkout,
            cli.STUB_REVIEW_PATH,
            cli.SHELL_REVIEW_PATH,
            write_directory=output,
        )

    @mock.patch("scripts.posix.cli.audit_reviews", side_effect=ValueError("missing shell review"))
    @mock.patch("scripts.posix.cli.fetch_checkout")
    @mock.patch("scripts.posix.cli.load_source_lock")
    def test_audit_returns_nonzero_for_incomplete_reviews(
        self,
        load_source_lock: mock.Mock,
        _fetch_checkout: mock.Mock,
        _audit: mock.Mock,
    ) -> None:
        load_source_lock.return_value = mock.Mock(revision="b" * 40)

        with contextlib.redirect_stderr(io.StringIO()):
            result = cli.main(["audit", "--check"])

        self.assertEqual(result, 1)


if __name__ == "__main__":
    unittest.main()
