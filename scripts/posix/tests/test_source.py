import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
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
from scripts.posix import build as build_module


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


def worktree_oid(root: Path) -> str:
    with tempfile.TemporaryDirectory() as temporary_directory:
        environment = os.environ.copy()
        environment["GIT_INDEX_FILE"] = str(
            Path(temporary_directory) / "actual.index"
        )
        for arguments in (
            ("read-tree", "HEAD"),
            (
                "add",
                "-A",
                "--",
                ".",
                ":(exclude).smros-revision",
                ":(exclude).smros-source.json",
            ),
        ):
            subprocess.run(
                ["git", "-C", str(root), *arguments],
                check=True,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        return subprocess.run(
            ["git", "-C", str(root), "write-tree"],
            check=True,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        ).stdout.strip()


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


class BoundedGitProcessTests(unittest.TestCase):
    def test_git_process_capture_is_bounded(self) -> None:
        completed = source_module._run_git_process(
            [
                sys.executable,
                "-c",
                "import sys; sys.stderr.write('x' * 100000)",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout_seconds=1.0,
        )

        self.assertEqual(completed.returncode, 0)
        self.assertLessEqual(
            len(completed.stderr), build_module.MAX_DIAGNOSTIC_BYTES
        )

    def test_git_process_timeout_is_bounded(self) -> None:
        started = time.monotonic()
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            source_module._run_git_process(
                [sys.executable, "-c", "import time; time.sleep(5)"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout_seconds=0.05,
            )

        self.assertEqual(raised.exception.returncode, 124)
        self.assertLess(time.monotonic() - started, 1.0)


class LocalCheckoutTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.temporary_root = Path(self.temporary_directory.name)
        self.origin = self.temporary_root / "origin"
        run_git("init", "--quiet", str(self.origin))
        (self.origin / ".gitignore").write_text(
            "ignored-local.txt\n", encoding="ascii"
        )
        (self.origin / "COPYING").write_text("GPL version 2\n", encoding="ascii")
        (self.origin / "value.txt").write_text("before\n", encoding="ascii")
        run_git(
            "-C", str(self.origin), "add", ".gitignore", "COPYING", "value.txt"
        )
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

    def write_add_delete_patch(self) -> None:
        patch = self.patches / "add-delete.patch"
        patch.write_text(
            "--- /dev/null\n"
            "+++ b/added.txt\n"
            "@@ -0,0 +1 @@\n"
            "+added\n"
            "--- a/value.txt\n"
            "+++ /dev/null\n"
            "@@ -1 +0,0 @@\n"
            "-before\n",
            encoding="ascii",
        )
        self.series.write_text(f"{patch.name}\n", encoding="utf-8")

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

    def test_gitignored_untracked_file_is_rejected_on_reuse(self) -> None:
        self.fetch()
        (self.checkout / "ignored-local.txt").write_text(
            "ignored\n", encoding="ascii"
        )

        with self.assertRaisesRegex(ValueError, "tree"):
            fetch_checkout(self.lock, self.checkout, self.series)

    def test_globally_ignored_untracked_file_is_rejected_on_reuse(self) -> None:
        excludes = self.temporary_root / "global-excludes"
        excludes.write_text("global-ignored.txt\n", encoding="ascii")
        config = self.temporary_root / "global.gitconfig"
        config.write_text(
            f"[core]\n\texcludesfile = {excludes}\n", encoding="utf-8"
        )

        with mock.patch.dict(
            os.environ,
            {
                "GIT_CONFIG_GLOBAL": str(config),
                "GIT_CONFIG_NOSYSTEM": "1",
            },
        ):
            self.fetch()
            (self.checkout / "global-ignored.txt").write_text(
                "ignored\n", encoding="ascii"
            )

            with self.assertRaisesRegex(ValueError, "tree"):
                fetch_checkout(self.lock, self.checkout, self.series)

    def test_modified_tree_with_forged_metadata_is_rejected(self) -> None:
        self.fetch()
        (self.checkout / "value.txt").write_text("forged\n", encoding="ascii")
        metadata_path = self.checkout / ".smros-source.json"
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        metadata["tree_oid"] = worktree_oid(self.checkout)
        metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

        with self.assertRaisesRegex(ValueError, "tree"):
            fetch_checkout(self.lock, self.checkout, self.series)

    def test_patch_created_and_deleted_files_are_bound_on_reuse(self) -> None:
        self.write_add_delete_patch()

        self.fetch()
        validate_checkout(self.checkout, self.revision)
        fetch_checkout(self.lock, self.checkout, self.series)

        self.assertEqual(
            (self.checkout / "added.txt").read_text(encoding="ascii"), "added\n"
        )
        self.assertFalse((self.checkout / "value.txt").exists())

    def test_direct_validation_rejects_modified_patched_tree(self) -> None:
        self.write_add_delete_patch()
        self.fetch()
        (self.checkout / "added.txt").write_text("modified\n", encoding="ascii")

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
            '"schema": 2, "schema": 2, '
            f'"patch_sha256": "{metadata["patch_sha256"]}", '
            f'"tree_oid": "{metadata["tree_oid"]}"'
            "}\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ValueError, "duplicate"):
            validate_checkout(self.checkout, self.revision)

    def test_source_metadata_rejects_non_string_digest(self) -> None:
        self.fetch()
        metadata_path = self.checkout / ".smros-source.json"
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        metadata["tree_oid"] = 7
        metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

        with self.assertRaisesRegex(ValueError, "tree"):
            validate_checkout(self.checkout, self.revision)

    def test_source_tree_symlink_is_rejected(self) -> None:
        self.fetch()
        source = self.checkout / "value.txt"
        outside = self.temporary_root / "outside-source"
        source.rename(outside)
        source.symlink_to(outside)

        with self.assertRaisesRegex(ValueError, "tree"):
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

        index_calls = [
            call
            for call in run.call_args_list
            if "GIT_INDEX_FILE" in (call.kwargs.get("env") or {})
        ]
        self.assertGreater(len(index_calls), 0)
        for call in index_calls:
            self.assertTrue(call.args[0][2].startswith("/proc/self/fd/"))
            self.assertGreater(len(call.kwargs["pass_fds"]), 0)

    def test_marker_write_stays_on_held_checkout_after_ancestor_swap(self) -> None:
        write_marker = source_module._write_revision_marker
        outside = self.temporary_root / "outside-marker-write"
        outside.mkdir()

        def redirect(target: object, revision: str) -> None:
            if isinstance(target, int):
                checkout = Path(os.readlink(f"/proc/self/fd/{target}"))
            else:
                checkout = Path(target)
            moved = checkout.with_name(f"{checkout.name}-moved")
            checkout.rename(moved)
            checkout.symlink_to(outside, target_is_directory=True)
            write_marker(target, revision)

        with mock.patch(
            "scripts.posix.source._write_revision_marker", side_effect=redirect
        ):
            with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                self.fetch()

        self.assertFalse((outside / ".smros-revision").exists())

    def test_metadata_write_stays_on_held_checkout_after_ancestor_swap(self) -> None:
        write_metadata = source_module._write_source_metadata
        outside = self.temporary_root / "outside-metadata-write"
        outside.mkdir()

        def redirect(
            target: object, patch_sha256: str, tree_oid: str
        ) -> None:
            if isinstance(target, int):
                checkout = Path(os.readlink(f"/proc/self/fd/{target}"))
            else:
                checkout = Path(target)
            moved = checkout.with_name(f"{checkout.name}-moved")
            checkout.rename(moved)
            checkout.symlink_to(outside, target_is_directory=True)
            write_metadata(target, patch_sha256, tree_oid)

        with mock.patch(
            "scripts.posix.source._write_source_metadata", side_effect=redirect
        ):
            with self.assertRaises(ValueError):
                self.fetch()

        self.assertFalse((outside / ".smros-source.json").exists())

    def test_git_validation_uses_held_checkout_after_ancestor_swap(self) -> None:
        observed_directory: str | None = None
        moved_checkout: Path | None = None

        def redirect(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess:
            nonlocal observed_directory, moved_checkout
            if argv[-2:] == ["rev-parse", "HEAD"] and observed_directory is None:
                observed_directory = argv[2]
                if observed_directory.startswith("/proc/self/fd/"):
                    descriptor = kwargs["pass_fds"][0]
                    checkout = Path(os.readlink(f"/proc/self/fd/{descriptor}"))
                else:
                    checkout = Path(observed_directory)
                moved_checkout = checkout.with_name(f"{checkout.name}-moved")
                checkout.rename(moved_checkout)
                checkout.symlink_to(self.origin, target_is_directory=True)
            return REAL_SUBPROCESS_RUN(argv, **kwargs)

        with mock.patch(
            "scripts.posix.source.subprocess.run", side_effect=redirect
        ):
            with self.assertRaises(ValueError):
                self.fetch()

        self.assertIsNotNone(moved_checkout)
        self.assertIsNotNone(observed_directory)
        self.assertTrue(observed_directory.startswith("/proc/self/fd/"))

    def test_publish_uses_held_parent_directories_after_ancestor_swap(self) -> None:
        rename_no_replace = source_module._rename_no_replace
        moved_parent: Path | None = None
        outside = self.temporary_root / "outside-publish"
        outside.mkdir()

        def redirect(*arguments: object) -> None:
            nonlocal moved_parent
            if len(arguments) == 2:
                destination_parent = Path(arguments[1]).parent
            else:
                destination_parent = Path(
                    os.readlink(f"/proc/self/fd/{arguments[3]}")
                )
            moved_parent = destination_parent.with_name(
                f"{destination_parent.name}-moved"
            )
            destination_parent.rename(moved_parent)
            destination_parent.symlink_to(outside, target_is_directory=True)
            rename_no_replace(*arguments)

        with mock.patch(
            "scripts.posix.source._rename_no_replace", side_effect=redirect
        ):
            self.fetch()

        self.assertIsNotNone(moved_parent)
        self.assertTrue((moved_parent / self.revision).is_dir())
        self.assertFalse((outside / self.revision).exists())

    def test_checkout_appearing_during_publish_is_not_overwritten(self) -> None:
        competitor = self.checkout / "owned-by-other"
        rename_no_replace = source_module._rename_no_replace

        def create_competitor(*arguments: object) -> None:
            if len(arguments) == 2:
                destination = Path(arguments[1])
                destination.mkdir(parents=True)
                competitor.write_text("keep\n", encoding="ascii")
            else:
                destination_parent = int(arguments[3])
                destination_name = str(arguments[4])
                os.mkdir(destination_name, dir_fd=destination_parent)
                checkout_descriptor = os.open(
                    destination_name,
                    os.O_RDONLY | os.O_DIRECTORY,
                    dir_fd=destination_parent,
                )
                try:
                    descriptor = os.open(
                        "owned-by-other",
                        os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                        0o644,
                        dir_fd=checkout_descriptor,
                    )
                    try:
                        os.write(descriptor, b"keep\n")
                    finally:
                        os.close(descriptor)
                finally:
                    os.close(checkout_descriptor)
            rename_no_replace(*arguments)

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

    @mock.patch("scripts.posix.source._remove_owned_temporary_directory")
    @mock.patch("scripts.posix.source.subprocess.run")
    def test_cleanup_failure_does_not_mask_clone_error(
        self, run: mock.Mock, remove_temporary: mock.Mock
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
            remove_temporary.side_effect = RuntimeError("cleanup failed")

            with self.assertRaises(subprocess.CalledProcessError) as raised:
                fetch_checkout(pinned_lock(), root, series)

            self.assertIs(raised.exception, original_error)


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


class PublicApiTests(unittest.TestCase):
    def test_validate_checkout_docstring_points_to_strong_fetch_validation(self) -> None:
        self.assertIn("fetch_checkout", validate_checkout.__doc__)
        self.assertIn("current patch series", validate_checkout.__doc__)

    def test_process_page_resources_are_complete_at_every_event_boundary(self) -> None:
        model_source = (REPOSITORY_ROOT / "scripts/posix/model.py").read_text(
            encoding="utf-8"
        )
        guest_source = (
            REPOSITORY_ROOT / "src/user_level/services/posix_test.rs"
        ).read_text(encoding="utf-8")
        syscall_source = (REPOSITORY_ROOT / "src/syscall/syscall.rs").read_text(
            encoding="utf-8"
        )
        for field in (
            "linux_processes",
            "linux_zombies",
            "private_pages",
            "shared_pages",
            "page_table_pages",
        ):
            with self.subTest(field=field):
                self.assertIn(f'"{field}"', model_source)
                self.assertIn(f'"{field}"', guest_source)
                self.assertIn(f"pub {field}: usize", syscall_source)


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
                    "group",
                    "api",
                    "platform",
                    "build_status",
                    "link_status",
                    "launch_status",
                    "pts_status",
                    "status",
                    "exit_code",
                    "signal",
                    "timed_out",
                    "duration_ms",
                    "stdout",
                    "stderr",
                    "source",
                    "launch_error",
                    "infrastructure_error",
                    "stdout_bytes",
                    "stderr_bytes",
                    "stdout_truncated",
                    "stderr_truncated",
                    "manifest_sha256",
                    "build_results_sha256",
                    "build_id",
                    "revision",
                    "patch_sha256",
                    "smros_commit",
                    "binary_sha256",
                    "runtime_snapshot_sha256",
                    "run_id",
                    "resource_deltas",
                    "resource_evidence",
                    "raw_log_start",
                    "raw_log_end",
                ),
                model.RuntimeAttempt(
                    "unistd/close/1-1",
                    "unistd",
                    "close",
                    "host",
                    "passed",
                    "passed",
                    "launched",
                    "pass",
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


class RepositoryPatchTests(unittest.TestCase):
    def test_sporadic_runtime_validates_default_priority_and_extensions(self) -> None:
        runtime_source = REPOSITORY_ROOT / "scripts/posix/runtime/smros_posix_compat.c"
        runtime_map = REPOSITORY_ROOT / "scripts/posix/runtime/smros_posix_compat.map"
        include_directory = REPOSITORY_ROOT / "scripts/posix/runtime/include"
        compiler = shutil.which("gcc")
        if compiler is None:
            self.skipTest("gcc is required for the scheduler runtime smoke test")

        mock_source = r'''
#include <sched.h>

static int mock_policy = SCHED_OTHER;
static int mock_priority;

int sched_getscheduler(pid_t pid) {
    (void)pid;
    return mock_policy;
}

int sched_getparam(pid_t pid, struct sched_param *param) {
    (void)pid;
    if (param == 0) {
        return -1;
    }
    param->sched_priority = mock_priority;
    return 0;
}

int sched_setscheduler(
    pid_t pid,
    int policy,
    const struct sched_param *param
) {
    (void)pid;
    if (param == 0) {
        return -1;
    }
    mock_policy = policy;
    mock_priority = param->sched_priority;
    return 0;
}

int sched_setparam(pid_t pid, const struct sched_param *param) {
    (void)pid;
    if (param == 0) {
        return -1;
    }
    mock_priority = param->sched_priority;
    return 0;
}
'''
        harness_source = r'''
#include <errno.h>
#include <sched.h>

static int expect_einval(struct sched_param *param) {
    errno = 0;
    return sched_setparam(0, param) == -1 && errno == EINVAL;
}

int main(void) {
    struct sched_param param;

    if (sched_getparam(0, &param) != 0) {
        return 1;
    }
    param.sched_ss_repl_period.tv_sec = 1;
    param.sched_ss_init_budget.tv_sec = 2;
    if (sched_setparam(1234, &param) != 0) {
        return 2;
    }

    if (sched_getparam(0, &param) != 0) {
        return 3;
    }
    param.sched_ss_repl_period.tv_sec = 1;
    param.sched_ss_init_budget.tv_sec = 2;
    if (!expect_einval(&param)) {
        return 4;
    }

    if (sched_getparam(0, &param) != 0) {
        return 5;
    }
    param.sched_ss_max_repl = 0;
    if (!expect_einval(&param)) {
        return 6;
    }

    if (sched_getparam(0, &param) != 0) {
        return 7;
    }
    if (sched_setscheduler(0, SCHED_SPORADIC, &param) != 0) {
        return 8;
    }
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary = Path(temporary_directory)
            mock_path = temporary / "mock.c"
            harness_path = temporary / "harness.c"
            mock_library = temporary / "libmock-scheduler.so"
            compat_library = temporary / "libsmros-posix-compat.so"
            executable = temporary / "scheduler-smoke"
            mock_path.write_text(mock_source, encoding="ascii")
            harness_path.write_text(harness_source, encoding="ascii")

            subprocess.run(
                [
                    compiler,
                    "-std=gnu99",
                    "-fPIC",
                    "-shared",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-I",
                    str(include_directory),
                    str(mock_path),
                    "-o",
                    str(mock_library),
                ],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                [
                    compiler,
                    "-std=gnu99",
                    "-fPIC",
                    "-shared",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-I",
                    str(include_directory),
                    str(runtime_source),
                    "-o",
                    str(compat_library),
                    "-Wl,--version-script," + str(runtime_map),
                    "-ldl",
                ],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                [
                    compiler,
                    "-std=gnu99",
                    "-I",
                    str(include_directory),
                    str(harness_path),
                    "-L",
                    str(temporary),
                    "-Wl,--no-as-needed",
                    "-Wl,-rpath," + str(temporary),
                    "-l:libsmros-posix-compat.so",
                    "-l:libmock-scheduler.so",
                    "-o",
                    str(executable),
                ],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            result = subprocess.run(
                [str(executable)],
                check=False,
                env={key: value for key, value in os.environ.items() if key != "LD_PRELOAD"},
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=f"scheduler smoke test failed: {result.stdout}{result.stderr}",
            )

    def test_fork_11_patch_ports_ltp_record_lock_assertion_without_weakening(
        self,
    ) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        name = "replace-defective-fork-11-record-lock-test.patch"
        self.assertEqual(entries.count(name), 1)
        self.assertLess(
            entries.index(name),
            entries.index("fix-pthread-join-6-3-shutdown-race.patch"),
        )

        patch = (patch_root / name).read_text(encoding="utf-8")
        self.assertIn("conformance/interfaces/fork/11-1.c", patch)
        removed_lines = [
            line[1:].strip()
            for line in patch.splitlines()
            if line.startswith("-") and not line.startswith("---")
        ]
        for removed in (
            "flockfile( stdout );",
            "ret = ftrylockfile( stdout );",
            '#include "testfrmw.c"',
        ):
            self.assertIn(removed, removed_lines)
        for retained in (
            "fcntl(fd, F_GETLK, &fl)",
            "fcntl(fd, F_SETLK, &fl)",
            "errno == EACCES || errno == EAGAIN",
            ".l_start = 0",
            ".l_len = 100",
            ".l_start = 1",
            ".l_len = 99",
            "child_pid = fork()",
            "waitpid(child_pid, &child_stat, 0)",
            "result = WEXITSTATUS(child_stat)",
        ):
            self.assertIn(retained, patch)
        self.assertNotIn("PTS_ATTRIBUTE_UNUSED", patch)
        self.assertNotIn("timeout_ms", patch)
        self.assertNotIn("SMROS", patch)
        unconditional_pass = [
            line
            for line in patch.splitlines()
            if line.startswith("+") and "return PTS_PASS;" in line
        ]
        self.assertEqual(len(unconditional_pass), 1)
        self.assertIn("errno == EACCES || errno == EAGAIN", patch)

        readme = (patch_root.parent / "README.md").read_text(encoding="utf-8")
        self.assertIn("0b69550e055b5385822f001e2a27fedfbef31816", readme)
        self.assertIn(
            "fcf9b794dd054586f65625ee6dd9a5daee61b98c1a43887de57e8c230a7d1626",
            readme,
        )

    def test_aio_patch_accepts_newer_option_versions_without_weakening_tests(
        self,
    ) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        series_entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        self.assertIn("accept-newer-aio-option-versions.patch", series_entries)

        patch = (patch_root / series_entries[0]).read_text(encoding="ascii")
        self.assertEqual(
            patch.count("-#if _POSIX_ASYNCHRONOUS_IO != 200112L"), 104
        )
        self.assertEqual(
            patch.count("+#if _POSIX_ASYNCHRONOUS_IO < 200112L"), 104
        )
        self.assertNotIn("PTS_PASS", patch)
        self.assertNotIn("PTS_UNSUPPORTED;\n+", patch)

    def test_sched_setparam_patch_allocates_complete_pid_arrays_without_weakening_tests(
        self,
    ) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        series_entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        patch_name = "fix-sched-setparam-pid-array-allocation.patch"
        self.assertIn(patch_name, series_entries)

        patch = (patch_root / patch_name).read_text(encoding="ascii")
        removed = [
            line[1:]
            for line in patch.splitlines()
            if line.startswith("-") and not line.startswith("---")
        ]
        added = [
            line[1:]
            for line in patch.splitlines()
            if line.startswith("+") and not line.startswith("+++")
        ]
        self.assertCountEqual(
            removed,
            ["\tchild_pid = malloc(nb_child);"] * 2
            + ["\tchild_pid = malloc(nb_cpu);"] * 2,
        )
        self.assertCountEqual(
            added,
            ["\tchild_pid = malloc(nb_child * sizeof(*child_pid));"] * 2
            + ["\tchild_pid = malloc(nb_cpu * sizeof(*child_pid));"] * 2,
        )

    def test_pthread_key_limit_patch_stops_after_validating_terminal_eagain(self) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        series_entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        patch_name = "fix-pthread-key-create-limit-check.patch"
        self.assertIn(patch_name, series_entries)

        patch = (patch_root / patch_name).read_text(encoding="ascii")
        self.assertIn(
            "if(i == NUM_OF_KEYS)",
            patch,
        )
        self.assertIn(
            "+\t\t\tcontinue;",
            patch,
        )
        self.assertNotIn("PTS_PASS", patch)

    def test_cond_broadcast_shutdown_patch_releases_signal_sender_waiters(self) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        series_entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        patch_name = "fix-cond-broadcast-4-2-shutdown-race.patch"
        self.assertIn(patch_name, series_entries)

        patch = (patch_root / patch_name).read_text(encoding="ascii")
        self.assertIn("conformance/interfaces/pthread_cond_broadcast/4-2.c", patch)
        self.assertIn("sem_post( &semsig1 )", patch)
        self.assertIn("sem_post( &semsig2 )", patch)
        self.assertNotIn("PTS_PASS", patch)

    def test_difftime_patch_accepts_scheduler_delay_without_weakening_minimum(self) -> None:
        patch_root = REPOSITORY_ROOT / "third_party" / "posixtest" / "patches"
        series_entries = [
            line.strip()
            for line in (patch_root / "series").read_text(encoding="ascii").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        patch_name = "fix-difftime-sleep-boundary.patch"
        self.assertEqual(series_entries.count(patch_name), 1)

        patch = (patch_root / patch_name).read_text(encoding="ascii")
        self.assertIn("conformance/interfaces/difftime/1-1.c", patch)
        self.assertIn("-\tif (time_diff != WAIT_DURATION) {", patch)
        self.assertIn("+\tif (time_diff < WAIT_DURATION) {", patch)
        self.assertNotIn("return PTS_PASS", patch)
        self.assertNotIn("SMROS", patch)


class DocumentationTests(unittest.TestCase):
    def test_readme_describes_default_and_overridden_work_directories(self) -> None:
        readme = (
            REPOSITORY_ROOT / "third_party" / "posixtest" / "README.md"
        ).read_text(encoding="utf-8")

        self.assertIn("By default", readme)
        self.assertIn("--work-dir", readme)
        self.assertIn("Git tree", readme)
        self.assertIn("symlink", readme)
        self.assertIn("executable", readme)
        self.assertIn("empty directories", readme)
        self.assertIn("directory modes", readme)


if __name__ == "__main__":
    unittest.main()
