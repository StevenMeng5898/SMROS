import json
import subprocess
import tempfile
import unittest
from dataclasses import FrozenInstanceError, fields
from pathlib import Path
from unittest import mock

from scripts.posix.source import (
    SourceLock,
    fetch_checkout,
    load_source_lock,
    validate_checkout,
)
from scripts.posix import cli, model, source as source_module


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
LOCK_PATH = REPOSITORY_ROOT / "third_party" / "posixtest" / "source.lock.json"
PINNED_REVISION = "85555325079ea362fa680bd2209c843cfe47e670"
REAL_SUBPROCESS_RUN = subprocess.run


def pinned_lock() -> SourceLock:
    return SourceLock(
        schema=1,
        url="https://github.com/emscripten-core/posixtestsuite.git",
        revision=PINNED_REVISION,
        license="GPL-2.0-only",
        standard="IEEE Std 1003.1-2001 System Interfaces",
    )


def run_git(*arguments: str, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=cwd,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return completed.stdout.strip()


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

    def test_rejects_duplicate_json_keys(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            path = Path(temporary_directory) / "source.lock.json"
            path.write_text(
                "{"
                '"schema": 1, "schema": 1, '
                '"url": "https://example.invalid/suite.git", '
                f'"revision": "{PINNED_REVISION}", '
                '"license": "GPL-2.0-only", '
                '"standard": "IEEE Std 1003.1-2001 System Interfaces"'
                "}",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "duplicate"):
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


class LocalCheckoutTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.temporary_root = Path(self.temporary_directory.name)
        self.origin = self.temporary_root / "origin"
        run_git("init", "--quiet", str(self.origin))
        (self.origin / "COPYING").write_text("GPL version 2\n", encoding="ascii")
        (self.origin / "value.txt").write_text("before\n", encoding="ascii")
        run_git("-C", str(self.origin), "add", "COPYING", "value.txt")
        run_git(
            "-C",
            str(self.origin),
            "-c",
            "user.name=SMROS tests",
            "-c",
            "user.email=smros@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "pinned",
        )
        self.revision = run_git("-C", str(self.origin), "rev-parse", "HEAD")
        (self.origin / "later.txt").write_text("later\n", encoding="ascii")
        run_git("-C", str(self.origin), "add", "later.txt")
        run_git(
            "-C",
            str(self.origin),
            "-c",
            "user.name=SMROS tests",
            "-c",
            "user.email=smros@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "later",
        )
        self.other_revision = run_git("-C", str(self.origin), "rev-parse", "HEAD")
        self.patches = self.temporary_root / "patches"
        self.patches.mkdir()
        self.series = self.patches / "series"
        self.series.write_text("", encoding="utf-8")
        self.checkout = self.temporary_root / "-work" / "src" / self.revision
        self.lock = pinned_lock()
        object.__setattr__(self.lock, "url", str(self.origin))
        object.__setattr__(self.lock, "revision", self.revision)

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def fetch(self) -> None:
        fetch_checkout(self.lock, self.checkout, self.series)

    def write_patch(self, name: str = "value.patch") -> Path:
        patch = self.patches / name
        patch.write_text(
            "--- a/value.txt\n"
            "+++ b/value.txt\n"
            "@@ -1 +1 @@\n"
            "-before\n"
            "+after\n",
            encoding="ascii",
        )
        self.series.write_text(f"{name}\n", encoding="utf-8")
        return patch

    def test_correct_git_head_and_generated_metadata_are_accepted(self) -> None:
        self.fetch()

        validate_checkout(self.checkout, self.revision)

        self.assertTrue((self.checkout / ".smros-source.json").is_file())

    def test_wrong_git_head_is_rejected(self) -> None:
        self.fetch()
        run_git(
            "-C", str(self.checkout), "checkout", "--quiet", "--detach", self.other_revision
        )

        with self.assertRaisesRegex(ValueError, "HEAD"):
            validate_checkout(self.checkout, self.revision)

    def test_modified_tracked_file_is_rejected(self) -> None:
        self.fetch()
        (self.checkout / "value.txt").write_text("modified\n", encoding="ascii")

        with self.assertRaisesRegex(ValueError, "tree"):
            validate_checkout(self.checkout, self.revision)

    def test_untracked_file_is_rejected(self) -> None:
        self.fetch()
        (self.checkout / "untracked.txt").write_text("untracked\n", encoding="ascii")

        with self.assertRaisesRegex(ValueError, "tree"):
            validate_checkout(self.checkout, self.revision)

    def test_file_mode_change_is_rejected(self) -> None:
        self.fetch()
        (self.checkout / "value.txt").chmod(0o755)

        with self.assertRaisesRegex(ValueError, "tree"):
            validate_checkout(self.checkout, self.revision)

    def test_changed_patch_bytes_are_rejected_on_reuse(self) -> None:
        patch = self.write_patch()
        self.fetch()
        patch.write_bytes(patch.read_bytes() + b"# review changed\n")

        with self.assertRaisesRegex(ValueError, "patch"):
            fetch_checkout(self.lock, self.checkout, self.series)

    def test_changed_patch_name_is_rejected_on_reuse(self) -> None:
        patch = self.write_patch()
        self.fetch()
        renamed = patch.with_name("renamed.patch")
        patch.rename(renamed)
        self.series.write_text(f"{renamed.name}\n", encoding="utf-8")

        with self.assertRaisesRegex(ValueError, "patch"):
            fetch_checkout(self.lock, self.checkout, self.series)

    def test_root_symlink_is_rejected(self) -> None:
        self.fetch()
        actual_checkout = self.checkout.with_name("actual-checkout")
        self.checkout.rename(actual_checkout)
        self.checkout.symlink_to(actual_checkout, target_is_directory=True)

        with self.assertRaisesRegex(ValueError, "symlink"):
            validate_checkout(self.checkout, self.revision)

    def test_copying_symlink_is_rejected(self) -> None:
        self.fetch()
        copying = self.checkout / "COPYING"
        outside = self.temporary_root / "outside-copying"
        copying.rename(outside)
        copying.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "COPYING.*symlink"):
            validate_checkout(self.checkout, self.revision)

    def test_revision_marker_symlink_is_rejected(self) -> None:
        self.fetch()
        marker = self.checkout / ".smros-revision"
        outside = self.temporary_root / "outside-marker"
        marker.rename(outside)
        marker.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "revision marker.*symlink"):
            validate_checkout(self.checkout, self.revision)

    def test_source_metadata_symlink_is_rejected(self) -> None:
        self.fetch()
        metadata = self.checkout / ".smros-source.json"
        outside = self.temporary_root / "outside-metadata"
        metadata.rename(outside)
        metadata.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "metadata.*symlink"):
            validate_checkout(self.checkout, self.revision)

    def test_source_metadata_rejects_duplicate_json_keys(self) -> None:
        self.fetch()
        metadata_path = self.checkout / ".smros-source.json"
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        metadata_path.write_text(
            "{"
            '"schema": 1, "schema": 1, '
            f'"patch_sha256": "{metadata["patch_sha256"]}", '
            f'"tree_sha256": "{metadata["tree_sha256"]}"'
            "}\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "duplicate"):
            validate_checkout(self.checkout, self.revision)

    def test_source_metadata_rejects_non_string_digest(self) -> None:
        self.fetch()
        metadata_path = self.checkout / ".smros-source.json"
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        metadata["tree_sha256"] = 7
        metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

        with self.assertRaisesRegex(ValueError, "tree digest"):
            validate_checkout(self.checkout, self.revision)

    def test_source_tree_symlink_is_rejected(self) -> None:
        self.fetch()
        source = self.checkout / "value.txt"
        outside = self.temporary_root / "outside-source"
        source.rename(outside)
        source.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "tree.*symlink"):
            validate_checkout(self.checkout, self.revision)

    def test_patch_symlink_is_rejected_before_clone(self) -> None:
        patch = self.write_patch()
        outside = self.temporary_root / "outside.patch"
        patch.rename(outside)
        patch.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "patch.*symlink"):
            self.fetch()

        self.assertFalse(self.checkout.exists())

    def test_patch_series_symlink_is_rejected_before_clone(self) -> None:
        outside = self.temporary_root / "outside-series"
        self.series.rename(outside)
        self.series.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "patch series.*symlink"):
            self.fetch()

        self.assertFalse(self.checkout.exists())

    def test_git_argv_uses_boundaries_and_captured_patch_stdin(self) -> None:
        patch = self.write_patch()
        expected_patch = patch.read_bytes()

        with mock.patch(
            "scripts.posix.source.subprocess.run", side_effect=REAL_SUBPROCESS_RUN
        ) as run:
            self.fetch()

        clone_call = next(
            call for call in run.call_args_list if call.args[0][1] == "clone"
        )
        clone_argv = clone_call.args[0]
        self.assertEqual(clone_argv[2:4], ["--no-checkout", "--"])
        self.assertTrue(Path(clone_argv[-1]).is_absolute())

        apply_call = next(
            call for call in run.call_args_list if call.args[0][3] == "apply"
        )
        self.assertEqual(apply_call.args[0][-2:], ["apply", "--"])
        self.assertEqual(apply_call.kwargs["input"], expected_patch)
        self.assertTrue(apply_call.kwargs["check"])
        self.assertNotIn("shell", apply_call.kwargs)

    def test_checkout_appearing_during_publish_is_not_overwritten(self) -> None:
        competitor = self.checkout / "owned-by-other"
        rename_no_replace = source_module._rename_no_replace

        def create_competitor(source: Path, destination: Path) -> None:
            destination.mkdir(parents=True)
            competitor.write_text("keep\n", encoding="ascii")
            rename_no_replace(source, destination)

        with mock.patch(
            "scripts.posix.source._rename_no_replace",
            side_effect=create_competitor,
        ):
            with self.assertRaisesRegex(ValueError, "destination"):
                self.fetch()

        self.assertEqual(competitor.read_text(encoding="ascii"), "keep\n")


class FetchInputTests(unittest.TestCase):
    @mock.patch("scripts.posix.source.subprocess.run")
    def test_unsafe_patch_entry_is_rejected_before_git(self, run: mock.Mock) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            series = temporary_root / "series"
            series.write_text("../../outside.patch\n", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "unsafe patch"):
                fetch_checkout(pinned_lock(), temporary_root / "checkout", series)

            run.assert_not_called()

    @mock.patch("scripts.posix.source.subprocess.run")
    def test_failed_clone_does_not_delete_competing_root(self, run: mock.Mock) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            root = temporary_root / "checkout"
            series = temporary_root / "series"
            series.write_text("", encoding="utf-8")
            original_error = subprocess.CalledProcessError(1, ["git", "clone"])

            def fail_clone(argv: list[str], **_: object) -> None:
                destination = Path(argv[-1])
                destination.mkdir(parents=True, exist_ok=True)
                (destination / "partial").write_text("partial\n", encoding="ascii")
                root.mkdir(parents=True, exist_ok=True)
                (root / "owned-by-other").write_text("keep\n", encoding="ascii")
                raise original_error

            run.side_effect = fail_clone

            with self.assertRaises(subprocess.CalledProcessError) as raised:
                fetch_checkout(pinned_lock(), root, series)

            self.assertIs(raised.exception, original_error)
            self.assertEqual(
                (root / "owned-by-other").read_text(encoding="ascii"), "keep\n"
            )

    @mock.patch("scripts.posix.source.shutil.rmtree")
    @mock.patch("scripts.posix.source.subprocess.run")
    def test_cleanup_failure_does_not_mask_clone_error(
        self, run: mock.Mock, rmtree: mock.Mock
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            root = temporary_root / "checkout"
            series = temporary_root / "series"
            series.write_text("", encoding="utf-8")
            original_error = subprocess.CalledProcessError(1, ["git", "clone"])

            def fail_clone(argv: list[str], **_: object) -> None:
                Path(argv[-1]).mkdir(parents=True, exist_ok=True)
                raise original_error

            run.side_effect = fail_clone
            rmtree.side_effect = RuntimeError("cleanup failed")

            with self.assertRaises(subprocess.CalledProcessError) as raised:
                fetch_checkout(pinned_lock(), root, series)

            self.assertIs(raised.exception, original_error)


class TreeDigestTests(unittest.TestCase):
    def test_git_control_file_is_excluded_from_source_tree_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            (root / "source.c").write_text("int main(void) {}\n", encoding="ascii")
            git_control = root / ".git"
            git_control.write_text("gitdir: /first\n", encoding="ascii")
            first_digest = source_module._tree_sha256(root)

            git_control.write_text("gitdir: /second\n", encoding="ascii")
            second_digest = source_module._tree_sha256(root)

            self.assertEqual(first_digest, second_digest)

    def test_tree_hash_uses_held_directory_when_ancestor_is_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory) / "root"
            nested = root / "nested"
            nested.mkdir(parents=True)
            source = nested / "source.c"
            source.write_text("safe\n", encoding="ascii")
            expected_digest = source_module._tree_sha256(root)

            moved_nested = root / "moved-nested"
            outside = Path(temporary_directory) / "outside"
            outside.mkdir()
            (outside / "source.c").write_text("outside\n", encoding="ascii")
            real_lstat = Path.lstat
            real_open = source_module.os.open
            replaced = False

            def replace_ancestor() -> None:
                nonlocal replaced
                if not replaced:
                    nested.rename(moved_nested)
                    nested.symlink_to(outside, target_is_directory=True)
                    replaced = True

            def racing_lstat(path: Path) -> object:
                if path == source:
                    replace_ancestor()
                return real_lstat(path)

            def racing_open(
                path: object,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                if dir_fd is not None and path == "source.c":
                    replace_ancestor()
                return real_open(path, flags, mode, dir_fd=dir_fd)

            with mock.patch.object(Path, "lstat", new=racing_lstat), mock.patch(
                "scripts.posix.source.os.open", side_effect=racing_open
            ):
                actual_digest = source_module._tree_sha256(root)

            self.assertEqual(actual_digest, expected_digest)


class PatchContainmentTests(unittest.TestCase):
    def test_patch_read_uses_held_directory_when_ancestor_is_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            patches = Path(temporary_directory) / "patches"
            nested = patches / "nested"
            nested.mkdir(parents=True)
            patch = nested / "change.patch"
            patch.write_bytes(b"safe patch\n")
            series = patches / "series"
            series.write_text("nested/change.patch\n", encoding="utf-8")

            moved_nested = patches / "moved-nested"
            outside = Path(temporary_directory) / "outside"
            outside.mkdir()
            (outside / "change.patch").write_bytes(b"outside patch\n")
            real_lstat = Path.lstat
            real_open = source_module.os.open
            replaced = False

            def replace_ancestor() -> None:
                nonlocal replaced
                if not replaced:
                    nested.rename(moved_nested)
                    nested.symlink_to(outside, target_is_directory=True)
                    replaced = True

            def racing_lstat(path: Path) -> object:
                if path == patch:
                    replace_ancestor()
                return real_lstat(path)

            def racing_open(
                path: object,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                if dir_fd is not None and path == "change.patch":
                    replace_ancestor()
                return real_open(path, flags, mode, dir_fd=dir_fd)

            with mock.patch.object(Path, "lstat", new=racing_lstat), mock.patch(
                "scripts.posix.source.os.open", side_effect=racing_open
            ):
                loaded = source_module._load_patches(series)

            self.assertEqual(loaded[0].data, b"safe patch\n")


class CommandLineTests(unittest.TestCase):
    def test_registers_fetch_with_default_work_directory(self) -> None:
        arguments = cli.create_parser().parse_args(["fetch"])

        self.assertEqual(arguments.command, "fetch")
        self.assertEqual(arguments.work_dir, Path("target/posix"))

    def test_accepts_dash_prefixed_work_directory_as_option_value(self) -> None:
        arguments = cli.create_parser().parse_args(
            ["fetch", "--work-dir=-dash-work"]
        )

        self.assertEqual(arguments.work_dir, Path("-dash-work"))

    @mock.patch("scripts.posix.cli.fetch_checkout")
    @mock.patch("scripts.posix.cli.load_source_lock")
    def test_fetch_dispatches_pinned_checkout_under_work_directory(
        self, load_source_lock: mock.Mock, fetch_checkout: mock.Mock
    ) -> None:
        lock = pinned_lock()
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

    def test_shared_record_fields_and_immutability(self) -> None:
        records = (
            (
                model.SuiteTest,
                (
                    "test_id",
                    "group",
                    "api",
                    "kind",
                    "disposition",
                    "source",
                    "binary",
                    "sha256",
                    "timeout_ms",
                ),
                model.SuiteTest(
                    "unistd/close/1-1",
                    "unistd",
                    "close",
                    "conformance",
                    "run",
                    "conformance/interfaces/close/1-1.c",
                    None,
                    None,
                    1000,
                ),
            ),
            (
                model.BuildResult,
                (
                    "test_id",
                    "stage",
                    "status",
                    "argv",
                    "returncode",
                    "stdout",
                    "stderr",
                    "duration_ms",
                    "artifact_sha256",
                ),
                model.BuildResult(
                    "unistd/close/1-1",
                    "compile",
                    "passed",
                    ("cc", "test.c"),
                    0,
                    "",
                    "",
                    12,
                    None,
                ),
            ),
            (
                model.RuntimeAttempt,
                (
                    "test_id",
                    "platform",
                    "status",
                    "exit_code",
                    "signal",
                    "timed_out",
                    "duration_ms",
                    "stdout",
                    "stderr",
                    "source",
                ),
                model.RuntimeAttempt(
                    "unistd/close/1-1",
                    "host",
                    "passed",
                    0,
                    None,
                    False,
                    4,
                    "",
                    "",
                    "runner",
                ),
            ),
            (
                model.RunMetadata,
                ("run_id", "platform", "manifest_sha256", "build_id", "complete"),
                model.RunMetadata("run-1", "host", "a" * 64, "build-1", True),
            ),
        )

        for record_type, expected_fields, instance in records:
            with self.subTest(record=record_type.__name__):
                self.assertEqual(
                    tuple(field.name for field in fields(record_type)), expected_fields
                )
                with self.assertRaises(FrozenInstanceError):
                    setattr(instance, expected_fields[0], "changed")


class DocumentationTests(unittest.TestCase):
    def test_readme_describes_default_and_overridden_work_directories(self) -> None:
        readme = (
            REPOSITORY_ROOT / "third_party" / "posixtest" / "README.md"
        ).read_text(encoding="utf-8")

        self.assertIn("By default", readme)
        self.assertIn("--work-dir", readme)


if __name__ == "__main__":
    unittest.main()
