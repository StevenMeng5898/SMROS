import json
import subprocess
import tempfile
import unittest
from dataclasses import FrozenInstanceError
from pathlib import Path
from unittest import mock

from scripts.posix.source import (
    SourceLock,
    fetch_checkout,
    load_source_lock,
    validate_checkout,
)
from scripts.posix import cli, model


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
LOCK_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "source.lock.json"
PINNED_REVISION = "85555325079ea362fa680bd2209c843cfe47e670"


class SourceLockTests(unittest.TestCase):
    def test_loads_exact_pinned_revision(self) -> None:
        self.assertEqual(
            load_source_lock(LOCK_PATH),
            SourceLock(
                schema=1,
                url="https://github.com/emscripten-core/posixtestsuite.git",
                revision=PINNED_REVISION,
                license="GPL-2.0-only",
                standard="IEEE Std 1003.1-2001 System Interfaces",
            ),
        )

    def test_rejects_non_commit_revision(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            path = Path(temporary_directory) / "source.lock.json"
            self._write_lock(path, revision="main")

            with self.assertRaisesRegex(ValueError, "revision"):
                load_source_lock(path)

    def test_rejects_missing_and_extra_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            path = Path(temporary_directory) / "source.lock.json"
            valid = self._lock_values()
            invalid_values = (
                {key: value for key, value in valid.items() if key != "standard"},
                {**valid, "branch": "main"},
            )

            for values in invalid_values:
                with self.subTest(fields=sorted(values)):
                    path.write_text(json.dumps(values), encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "fields"):
                        load_source_lock(path)

    @staticmethod
    def _lock_values(**overrides: object) -> dict[str, object]:
        values: dict[str, object] = {
            "schema": 1,
            "url": "https://github.com/emscripten-core/posixtestsuite.git",
            "revision": PINNED_REVISION,
            "license": "GPL-2.0-only",
            "standard": "IEEE Std 1003.1-2001 System Interfaces",
        }
        values.update(overrides)
        return values

    def _write_lock(self, path: Path, **overrides: object) -> None:
        path.write_text(json.dumps(self._lock_values(**overrides)), encoding="utf-8")


class CheckoutTests(unittest.TestCase):
    def test_checkout_requires_copying_and_exact_revision_marker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)

            with self.assertRaisesRegex(ValueError, "COPYING"):
                validate_checkout(root, PINNED_REVISION)

            (root / "COPYING").write_text("GPL version 2\n", encoding="ascii")
            (root / ".smros-revision").write_text("different\n", encoding="ascii")
            with self.assertRaisesRegex(ValueError, "revision"):
                validate_checkout(root, PINNED_REVISION)

            (root / ".smros-revision").write_text(
                f"{PINNED_REVISION}\n", encoding="ascii"
            )
            validate_checkout(root, PINNED_REVISION)

    @mock.patch("scripts.posix.source.subprocess.run")
    def test_existing_checkout_is_validated_without_git(self, run: mock.Mock) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            root = temporary_root / "src" / PINNED_REVISION
            root.mkdir(parents=True)
            (root / "COPYING").write_text("GPL version 2\n", encoding="ascii")
            (root / ".smros-revision").write_text(
                f"{PINNED_REVISION}\n", encoding="ascii"
            )

            fetch_checkout(self._lock(), root, temporary_root / "series")

            run.assert_not_called()

    @mock.patch("scripts.posix.source.subprocess.run")
    def test_fetch_uses_argument_arrays_and_applies_series_in_order(
        self, run: mock.Mock
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            root = temporary_root / "work" / "src" / PINNED_REVISION
            patches = temporary_root / "patches"
            patches.mkdir()
            first = patches / "first.patch"
            second = patches / "second.patch"
            first.touch()
            second.touch()
            series = patches / "series"
            series.write_text(
                "# Applied in review order\nfirst.patch\n\nsecond.patch\n",
                encoding="utf-8",
            )

            def simulate_git(argv: list[str], **_: object) -> subprocess.CompletedProcess:
                if argv[:3] == ["git", "clone", "--no-checkout"]:
                    root.mkdir(parents=True)
                    (root / "COPYING").write_text("GPL version 2\n", encoding="ascii")
                return subprocess.CompletedProcess(argv, 0)

            run.side_effect = simulate_git

            fetch_checkout(self._lock(), root, series)

            self.assertEqual(
                [call.args[0] for call in run.call_args_list],
                [
                    [
                        "git",
                        "clone",
                        "--no-checkout",
                        self._lock().url,
                        str(root),
                    ],
                    [
                        "git",
                        "-C",
                        str(root),
                        "fetch",
                        "--depth",
                        "1",
                        "origin",
                        PINNED_REVISION,
                    ],
                    [
                        "git",
                        "-C",
                        str(root),
                        "checkout",
                        "--detach",
                        PINNED_REVISION,
                    ],
                    ["git", "-C", str(root), "apply", str(first)],
                    ["git", "-C", str(root), "apply", str(second)],
                ],
            )
            for call in run.call_args_list:
                self.assertTrue(call.kwargs["check"])
                self.assertNotIn("shell", call.kwargs)
            self.assertEqual(
                (root / ".smros-revision").read_text(encoding="ascii"),
                f"{PINNED_REVISION}\n",
            )

    @mock.patch("scripts.posix.source.subprocess.run")
    def test_unsafe_patch_entry_is_rejected_before_git(self, run: mock.Mock) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            series = temporary_root / "series"
            series.write_text("../../outside.patch\n", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "unsafe patch"):
                fetch_checkout(self._lock(), temporary_root / "checkout", series)

            run.assert_not_called()

    @mock.patch("scripts.posix.source.subprocess.run")
    def test_failed_fetch_removes_checkout_created_by_clone(
        self, run: mock.Mock
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            root = temporary_root / "work" / "src" / PINNED_REVISION
            series = temporary_root / "series"
            series.write_text("", encoding="utf-8")

            def fail_after_clone(
                argv: list[str], **_: object
            ) -> subprocess.CompletedProcess:
                if argv[:3] == ["git", "clone", "--no-checkout"]:
                    root.mkdir(parents=True)
                    return subprocess.CompletedProcess(argv, 0)
                raise subprocess.CalledProcessError(1, argv)

            run.side_effect = fail_after_clone

            with self.assertRaises(subprocess.CalledProcessError):
                fetch_checkout(self._lock(), root, series)

            self.assertFalse(root.exists())

    @mock.patch("scripts.posix.source.subprocess.run")
    def test_fetch_replaces_revision_symlink_without_following_it(
        self, run: mock.Mock
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            root = temporary_root / "work" / "src" / PINNED_REVISION
            outside = temporary_root / "outside-marker"
            outside.write_text("outside\n", encoding="ascii")
            series = temporary_root / "series"
            series.write_text("", encoding="utf-8")

            def simulate_git(
                argv: list[str], **_: object
            ) -> subprocess.CompletedProcess:
                if argv[:3] == ["git", "clone", "--no-checkout"]:
                    root.mkdir(parents=True)
                    (root / "COPYING").write_text(
                        "GPL version 2\n", encoding="ascii"
                    )
                    (root / ".smros-revision").symlink_to(outside)
                return subprocess.CompletedProcess(argv, 0)

            run.side_effect = simulate_git

            fetch_checkout(self._lock(), root, series)

            self.assertEqual(outside.read_text(encoding="ascii"), "outside\n")
            self.assertFalse((root / ".smros-revision").is_symlink())

    @staticmethod
    def _lock() -> SourceLock:
        return SourceLock(
            schema=1,
            url="https://github.com/emscripten-core/posixtestsuite.git",
            revision=PINNED_REVISION,
            license="GPL-2.0-only",
            standard="IEEE Std 1003.1-2001 System Interfaces",
        )


class CommandLineTests(unittest.TestCase):
    def test_registers_fetch_with_default_work_directory(self) -> None:
        arguments = cli.create_parser().parse_args(["fetch"])

        self.assertEqual(arguments.command, "fetch")
        self.assertEqual(arguments.work_dir, Path("target/posix"))

    @mock.patch("scripts.posix.cli.fetch_checkout")
    @mock.patch("scripts.posix.cli.load_source_lock")
    def test_fetch_dispatches_pinned_checkout_under_work_directory(
        self, load_source_lock: mock.Mock, fetch_checkout: mock.Mock
    ) -> None:
        lock = CheckoutTests._lock()
        load_source_lock.return_value = lock
        work_directory = Path("custom-work")

        result = cli.main(["fetch", "--work-dir", str(work_directory)])

        self.assertEqual(result, 0)
        load_source_lock.assert_called_once_with(LOCK_PATH)
        fetch_checkout.assert_called_once_with(
            lock,
            work_directory / "src" / PINNED_REVISION,
            REPOSITORY_ROOT / "third_party" / "posixtest" / "patches" / "series",
        )


class SharedModelTests(unittest.TestCase):
    def test_suite_status_constants_match_open_posix_test_suite(self) -> None:
        self.assertEqual(
            (
                model.PTS_PASS,
                model.PTS_FAIL,
                model.PTS_UNRESOLVED,
                model.PTS_UNSUPPORTED,
                model.PTS_UNTESTED,
            ),
            (0, 1, 2, 4, 5),
        )

    def test_shared_records_are_immutable(self) -> None:
        test = model.SuiteTest(
            test_id="unistd/close/1-1",
            group="unistd",
            api="close",
            kind="conformance",
            disposition="run",
            source="conformance/interfaces/close/1-1.c",
            binary=None,
            sha256=None,
            timeout_ms=1000,
        )

        with self.assertRaises(FrozenInstanceError):
            test.timeout_ms = 2000


if __name__ == "__main__":
    unittest.main()
