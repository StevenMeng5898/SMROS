"""Exercise the real build snapshot across target changes, not source text."""

from contextlib import redirect_stdout
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from scripts.posix import cli


ROOT = Path(__file__).resolve().parents[3]


class StageSelectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compiler_directory = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.compiler_directory.cleanup)
        cls.build_script = Path(cls.compiler_directory.name) / "build-script"
        subprocess.run(
            ["rustc", str(ROOT / "build.rs"), "--edition=2021", "-o", str(cls.build_script)],
            check=True, capture_output=True,
        )

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.legacy = self.root / "host_shared/posixtest"
        self.riscv = self.root / "target/posix/riscv64/stage"
        self.out = self.root / "out"
        self.out.mkdir()
        self.environment = dict(os.environ)
        self.environment.pop("SMROS_POSIX_STAGE", None)
        self.environment.update(
            CARGO_MANIFEST_DIR=str(self.root), OUT_DIR=str(self.out),
            CARGO_ENCODED_RUSTFLAGS="",
        )

    def stage(self, path, architecture):
        (path / "bin").mkdir(parents=True)
        (path / "manifest.tsv").write_text(
            f"SMROS_POSIX_MANIFEST\t1\nmeta\tarchitecture\t{architecture}\n"
        )
        (path / "bin/probe").write_text(architecture)

    def build(self, target, override=None):
        environment = dict(self.environment, TARGET=target)
        if override is not None:
            environment["SMROS_POSIX_STAGE"] = str(override)
        return subprocess.run(
            [str(self.build_script)], env=environment, cwd="/",
            text=True, capture_output=True,
        )

    def snapshot(self):
        # Compile include_bytes! entries too: a right-looking path with the
        # wrong contents must fail just as it would inside the kernel.
        source = self.out / "probe.rs"
        source.write_text(
            'include!("host_share.rs");\n'
            'fn main() { for f in HOST_SHARE_FILES { '
            'println!("{}={}", f.path, String::from_utf8_lossy(f.data)); } }\n'
        )
        binary = self.out / "probe"
        subprocess.run(
            ["rustc", str(source), "-o", str(binary)], check=True, capture_output=True,
        )
        return subprocess.check_output([str(binary)], text=True)

    def test_switching_targets_embeds_only_the_selected_stage(self):
        self.stage(self.legacy, "aarch64")
        (self.legacy / "bin/arm-only").write_text("must not leak")
        (self.root / "host_shared/notes.txt").write_text("preserved")
        self.stage(self.riscv, "riscv64")
        for target, architecture in (
            ("riscv64gc-unknown-none-elf", "riscv64"),
            ("aarch64-unknown-none", "aarch64"),
            ("riscv64gc-unknown-none-elf", "riscv64"),
        ):
            with self.subTest(target=target):
                result = self.build(target)
                self.assertEqual(result.returncode, 0, result.stderr)
                snapshot = self.snapshot()
                self.assertIn(f"posixtest/bin/probe={architecture}\n", snapshot)
                self.assertIn(f"meta\tarchitecture\t{architecture}\n", snapshot)
                self.assertIn("notes.txt=preserved\n", snapshot)
                self.assertEqual(snapshot.count("posixtest/manifest.tsv="), 1)
                if architecture == "riscv64":
                    self.assertNotIn("arm-only", snapshot)
                    self.assertIn(f"cargo:rerun-if-changed={self.riscv / 'manifest.tsv'}", result.stdout)
                    self.assertIn("cargo:rerun-if-env-changed=SMROS_POSIX_STAGE", result.stdout)

    def test_missing_target_stage_cannot_silently_embed_arm64(self):
        self.stage(self.legacy, "aarch64")
        result = self.build("riscv64gc-unknown-none-elf")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("riscv64", result.stderr)
        self.assertIn("SMROS_POSIX_STAGE", result.stderr)

    def test_stage_override_is_atomic_and_relative_to_repository(self):
        self.stage(self.legacy, "aarch64")
        self.stage(self.riscv, "riscv64")
        custom = self.root / "custom stage"
        self.stage(custom, "riscv64")
        (custom / "bin/probe").write_text("custom-riscv")
        for override in (custom, Path("custom stage")):
            result = self.build("riscv64gc-unknown-none-elf", override)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("posixtest/bin/probe=custom-riscv\n", self.snapshot())

    def test_invalid_selected_stage_fails_before_kernel_compilation(self):
        self.stage(self.riscv, "riscv64")
        manifest = self.riscv / "manifest.tsv"
        for contents in (
            "SMROS_POSIX_MANIFEST\t1\nmeta\tarchitecture\taarch64\n",
            "SMROS_POSIX_MANIFEST\t2\nmeta\tarchitecture\triscv64\n",
            "SMROS_POSIX_MANIFEST\t1\n",
            "SMROS_POSIX_MANIFEST\t1\nmeta\tarchitecture\triscv64\nmeta\tarchitecture\taarch64\n",
        ):
            with self.subTest(contents=contents):
                manifest.write_text(contents)
                result = self.build("riscv64gc-unknown-none-elf")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("POSIX stage", result.stderr)
        manifest.unlink()
        self.assertNotEqual(self.build("riscv64gc-unknown-none-elf").returncode, 0)
        self.assertNotEqual(self.build("riscv64gc-unknown-none-elf", "missing").returncode, 0)
        self.assertNotEqual(self.build("riscv64gc-unknown-none-elf", "").returncode, 0)

    def test_kernel_without_any_posix_stage_still_builds(self):
        result = self.build("riscv64gc-unknown-none-elf")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.snapshot(), "")

    def test_cli_runner_uses_the_same_target_stage_as_the_kernel(self):
        runner_result = mock.Mock(attempts=(), complete=True, restart_count=0, result_path="results")
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(
            cli, "REPOSITORY_ROOT", self.root
        ), mock.patch.object(cli, "run_smros", return_value=runner_result) as runner:
            for architecture, expected in (("aarch64", self.legacy), ("riscv64", self.riscv)):
                with self.subTest(architecture=architecture), redirect_stdout(io.StringIO()):
                    self.assertEqual(cli.main(["run-smros", "--arch", architecture]), 0)
                    self.assertEqual(runner.call_args.args[0], expected)
            with mock.patch.dict(os.environ, {"SMROS_POSIX_STAGE": "custom stage"}):
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(cli.main(["run-smros", "--arch", "riscv64"]), 0)
                self.assertEqual(runner.call_args.args[0], self.root / "custom stage")

    def test_make_posix_commands_follow_the_selected_kernel_architecture(self):
        for target, architecture in (
            ("aarch64-unknown-none", "aarch64"),
            ("riscv64gc-unknown-none-elf", "riscv64"),
        ):
            for command, prerequisites in (
                ("posix-build", ["posix-audit"]),
                ("posix-stage", ["posix-build"]),
                ("posix-run", ["posix-stage", "smros-fxfs.img"]),
                ("posix-baseline", ["posix-stage"]),
            ):
                with self.subTest(target=target, command=command):
                    args = ["make", "--no-print-directory", "-n", command, f"ARCH={target}"]
                    for prerequisite in prerequisites:
                        args.extend(["-o", prerequisite])
                    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=True)
                    self.assertIn(f"--arch {architecture}", result.stdout)
                    if architecture != "aarch64":
                        self.assertNotIn("--stage host_shared/posixtest", result.stdout)
                    if command == "posix-run":
                        self.assertIn(f"build ARCH={target}", result.stdout)


if __name__ == "__main__":
    unittest.main()
