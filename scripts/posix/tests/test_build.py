import hashlib
import json
import sys
import tempfile
import unittest
from dataclasses import asdict, replace
from pathlib import Path
from unittest import mock

from scripts.posix import build as build_module
from scripts.posix import cli
from scripts.posix.build import (
    EMPTY_SHA256,
    MAX_MANIFEST_BYTES,
    MAX_TESTS,
    ManifestMetadata,
    build_campaign,
    compiler_query,
    compile_command,
    link_command,
    nm_command,
    parse_elf_dependencies,
    parse_manifest,
    render_manifest,
    resolve_runtime_file,
    safe_stage_path,
    stage_runtime_dependencies,
    verify_stage,
)
from scripts.posix.model import BuildResult, BuildSummary, SuiteTest


def suite_test(
    test_id: str = "conformance/interfaces/getpid/1-1.c",
    *,
    kind: str = "runnable",
    disposition: str = "complete",
) -> SuiteTest:
    return SuiteTest(
        test_id=test_id,
        group="base",
        api="getpid",
        kind=kind,
        disposition=disposition,
        source=test_id,
        binary=None,
        sha256=None,
        timeout_ms=30_000,
    )


def metadata() -> ManifestMetadata:
    return ManifestMetadata(
        source="https://github.com/emscripten-core/posixtestsuite.git",
        revision="8" * 40,
        architecture="aarch64",
        compiler="aarch64-linux-gnu-gcc 13.2.0",
        libc="glibc",
        patch_sha256="1" * 64,
        smros_commit="2" * 40,
    )


def write_stage_fixture(stage: Path, test: SuiteTest) -> None:
    manifest_text, _ = render_manifest(metadata(), (test,))
    parsed_metadata, _ = parse_manifest(manifest_text.encode())
    host_manifest = {
        "schema": 1,
        "checksum_definition": (
            "sha256(manifest.tsv with meta manifest_sha256 value replaced by "
            "64 ASCII zeroes)"
        ),
        "metadata": asdict(parsed_metadata),
        "runtime": [],
        "tests": [asdict(test)],
    }
    build_results = (
        BuildResult(
            test_id=test.test_id,
            stage="compile",
            status="passed",
            argv=("aarch64-linux-gnu-gcc", "-c", test.source),
            returncode=0,
            stdout="",
            stderr="",
            duration_ms=1,
            artifact_sha256="a" * 64,
        ),
        BuildResult(
            test_id=test.test_id,
            stage="nm",
            status="passed",
            argv=("aarch64-linux-gnu-nm", test.source),
            returncode=0,
            stdout="00000000 T main\n",
            stderr="",
            duration_ms=1,
            artifact_sha256=None,
        ),
        BuildResult(
            test_id=test.test_id,
            stage="link",
            status="passed",
            argv=("aarch64-linux-gnu-gcc", "-o", test.binary or ""),
            returncode=0,
            stdout="",
            stderr="",
            duration_ms=1,
            artifact_sha256=test.sha256,
        ),
    )
    (stage / "manifest.tsv").write_text(manifest_text, encoding="utf-8")
    (stage / "manifest.json").write_text(
        json.dumps(host_manifest, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    (stage / "build-results.ndjson").write_text(
        "".join(
            json.dumps(asdict(result), sort_keys=True, separators=(",", ":")) + "\n"
            for result in build_results
        ),
        encoding="utf-8",
    )
    if test.binary not in {None, "-"}:
        (stage / test.binary).chmod(0o755)


class CommandTests(unittest.TestCase):
    def test_nm_uses_target_tool_and_definition_only_flags(self) -> None:
        self.assertEqual(
            nm_command("aarch64-linux-gnu-nm", Path("case.o")),
            ["aarch64-linux-gnu-nm", "-g", "--defined-only", "case.o"],
        )

    def test_compile_uses_aarch64_gnu99_posix_and_thread_flags(self) -> None:
        command = compile_command(
            "aarch64-linux-gnu-gcc",
            Path("suite/case.c"),
            Path("obj/case.o"),
            Path("suite/include"),
        )

        self.assertEqual(command[0], "aarch64-linux-gnu-gcc")
        for flag in (
            "-std=gnu99",
            "-D_POSIX_C_SOURCE=200112L",
            "-D_XOPEN_SOURCE=600",
            "-pthread",
        ):
            self.assertIn(flag, command)
        self.assertIn("-c", command)
        self.assertNotIn("-lrt", command)
        self.assertNotIn("-lm", command)

    def test_link_uses_required_posix_libraries(self) -> None:
        command = link_command(
            "aarch64-linux-gnu-gcc", Path("obj/case.o"), Path("bin/case.test")
        )

        self.assertEqual(command[0], "aarch64-linux-gnu-gcc")
        for flag in ("-pthread", "-lrt", "-lm"):
            self.assertIn(flag, command)
        self.assertLess(command.index("obj/case.o"), command.index("-lrt"))

    def test_build_never_executes_target_object_or_binary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            source = checkout / "conformance/interfaces/getpid/1-1.c"
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            stage = root / "stage"
            work = root / "work"
            commands: list[list[str]] = []

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                command = list(argv)
                commands.append(command)
                if command[0].endswith("gcc"):
                    output = Path(command[command.index("-o") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"artifact")
                stdout = "00000000 T main\n" if command[0].endswith("nm") else ""
                return mock.Mock(returncode=0, stdout=stdout, stderr="")

            build_campaign(
                checkout,
                (suite_test(),),
                (),
                metadata(),
                stage,
                work,
                command_runner=fake_run,
                dependency_stager=mock.Mock(return_value=()),
            )

        executed_programs = {Path(command[0]).name for command in commands}
        self.assertEqual(
            executed_programs,
            {"aarch64-linux-gnu-gcc", "aarch64-linux-gnu-nm"},
        )
        self.assertFalse(
            any(command[0].endswith((".o", ".test")) for command in commands)
        )

    def test_default_process_capture_is_bounded_and_timed(self) -> None:
        noisy = build_module.run_bounded_command(
            [sys.executable, "-c", "import sys; sys.stdout.write('x' * 100000)"]
        )
        self.assertEqual(noisy.returncode, 0)
        self.assertLessEqual(
            len(noisy.stdout.encode("utf-8")), build_module.MAX_DIAGNOSTIC_BYTES
        )
        non_utf8 = build_module.run_bounded_command(
            [
                sys.executable,
                "-c",
                "import sys; sys.stdout.buffer.write(bytes([255]) * 100000)",
            ]
        )
        self.assertLessEqual(
            len(non_utf8.stdout.encode("utf-8")),
            build_module.MAX_DIAGNOSTIC_BYTES,
        )

        timed = build_module.run_bounded_command(
            [sys.executable, "-c", "import time; time.sleep(5)"],
            timeout_seconds=0.05,
        )
        self.assertEqual(timed.returncode, 124)
        self.assertIn("timed out", timed.stderr)

        inherited_pipe = build_module.run_bounded_command(
            [
                sys.executable,
                "-c",
                (
                    "import os,time; child=os.fork(); "
                    "time.sleep(5) if child == 0 else None"
                ),
            ],
            timeout_seconds=0.05,
        )
        self.assertEqual(inherited_pipe.returncode, 124)

    def test_compiler_query_uses_bounded_runner(self) -> None:
        completed = mock.Mock(returncode=0, stdout="/sysroot\n", stderr="")
        with mock.patch(
            "scripts.posix.build.run_bounded_command", return_value=completed
        ) as run:
            self.assertEqual(compiler_query("fake-gcc", "-print-sysroot"), "/sysroot")
        run.assert_called_once_with(["fake-gcc", "-print-sysroot"])


class CampaignTests(unittest.TestCase):
    def test_compile_failure_is_recorded_and_remaining_sources_continue(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            tests = (
                suite_test("conformance/interfaces/getpid/1-1.c"),
                suite_test("conformance/interfaces/getpid/2-1.c"),
            )
            for test in tests:
                source = checkout / test.source
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            compile_count = 0

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                nonlocal compile_count
                command = list(argv)
                if command[0].endswith("gcc") and "-c" in command:
                    compile_count += 1
                    if compile_count == 1:
                        return mock.Mock(
                            returncode=1, stdout="x" * 100_000, stderr="compile failed"
                        )
                if command[0].endswith("gcc"):
                    output = Path(command[command.index("-o") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"artifact")
                stdout = "00000000 T main\n" if command[0].endswith("nm") else ""
                return mock.Mock(returncode=0, stdout=stdout, stderr="")

            summary = build_campaign(
                checkout,
                tests,
                (),
                metadata(),
                root / "stage",
                root / "work",
                command_runner=fake_run,
                dependency_stager=mock.Mock(return_value=()),
            )

            records = [
                json.loads(line)
                for line in (root / "stage/build-results.ndjson")
                .read_text(encoding="utf-8")
                .splitlines()
            ]

        self.assertEqual(compile_count, 2)
        self.assertEqual((summary.compile_pass, summary.compile_fail), (1, 1))
        self.assertEqual(records[0]["status"], "failed")
        self.assertLessEqual(len(records[0]["stdout"].encode()), 16_384)
        self.assertTrue(any(record["stage"] == "link" for record in records))

    def test_definition_stub_and_shell_dispositions_are_visible(self) -> None:
        definition = replace(
            suite_test(
                "conformance/definitions/unistd_h/1-1.c",
                kind="definition",
                disposition="definition-only",
            ),
            binary="-",
            sha256=EMPTY_SHA256,
        )
        stub = replace(
            suite_test(disposition="excluded-upstream-stub"),
            binary="-",
            sha256=EMPTY_SHA256,
        )
        shell = replace(
            suite_test(
                "conformance/interfaces/getpid/1-1.sh",
                kind="shell",
                disposition="not-built-shell-test",
            ),
            binary="-",
            sha256=EMPTY_SHA256,
        )

        text, _checksum = render_manifest(metadata(), (shell, stub, definition))
        parsed_metadata, parsed_tests = parse_manifest(text.encode())

        self.assertEqual(parsed_metadata.architecture, "aarch64")
        self.assertEqual(
            [test.disposition for test in parsed_tests],
            ["definition-only", "excluded-upstream-stub", "not-built-shell-test"],
        )
        parsed_shell = next(test for test in parsed_tests if test.kind == "shell")
        self.assertEqual((parsed_shell.api, parsed_shell.group), ("getpid", "base"))

    def test_excluded_stub_keeps_disposition_when_compilation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            stub = suite_test(disposition="excluded-upstream-stub")
            source = checkout / stub.source
            source.parent.mkdir(parents=True)
            source.write_text("invalid C\n", encoding="utf-8")

            summary = build_campaign(
                checkout,
                (stub,),
                (),
                metadata(),
                root / "stage",
                root / "work",
                command_runner=lambda _argv, **_kwargs: mock.Mock(
                    returncode=1, stdout="", stderr="compile failed"
                ),
                dependency_stager=mock.Mock(return_value=()),
            )
            _metadata, tests = parse_manifest(
                (root / "stage/manifest.tsv").read_bytes()
            )
            verified = verify_stage(root / "stage", verify_architecture=False)

        self.assertEqual(summary.compile_fail, 1)
        self.assertEqual(tests[0].disposition, "excluded-upstream-stub")
        self.assertEqual((verified.compile_pass, verified.compile_fail), (0, 1))

    def test_toolchain_disappearance_is_campaign_fatal_and_preserves_stage(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            source = checkout / "conformance/interfaces/getpid/1-1.c"
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            stage = root / "stage"
            stage.mkdir()
            marker = stage / "previous"
            marker.write_text("valid\n", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "toolchain"):
                build_campaign(
                    checkout,
                    (suite_test(),),
                    (),
                    metadata(),
                    stage,
                    root / "work",
                    command_runner=mock.Mock(
                        side_effect=FileNotFoundError("compiler disappeared")
                    ),
                    dependency_stager=mock.Mock(return_value=()),
                )

            self.assertEqual(marker.read_text(encoding="utf-8"), "valid\n")
            self.assertEqual(list(root.glob(".stage.tmp-*")), [])

    def test_symlinked_work_root_is_rejected_before_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            test = suite_test()
            source = checkout / test.source
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            outside = root / "outside"
            stale = outside / "obj/keep.o"
            stale.parent.mkdir(parents=True)
            stale.write_bytes(b"keep")
            work = root / "work"
            work.symlink_to(outside, target_is_directory=True)

            with self.assertRaisesRegex(ValueError, "work.*symlink"):
                build_campaign(
                    checkout,
                    (test,),
                    (),
                    metadata(),
                    root / "stage",
                    work,
                    command_runner=mock.Mock(),
                    dependency_stager=mock.Mock(return_value=()),
                )

            self.assertEqual(stale.read_bytes(), b"keep")

    def test_stale_work_artifacts_are_not_accepted_as_fresh_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            test = suite_test()
            source = checkout / test.source
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            work = root / "work"
            stale_object = work / "obj" / f"{test.test_id}.o"
            stale_binary = work / "bin" / f"{test.test_id}.test"
            stale_object.parent.mkdir(parents=True)
            stale_binary.parent.mkdir(parents=True)
            stale_object.write_bytes(b"stale object")
            stale_binary.write_bytes(b"stale executable")

            summary = build_campaign(
                checkout,
                (test,),
                (),
                metadata(),
                root / "stage",
                work,
                command_runner=mock.Mock(
                    return_value=mock.Mock(returncode=0, stdout="", stderr="")
                ),
                dependency_stager=mock.Mock(return_value=()),
            )

            _metadata, staged_tests = parse_manifest(
                (root / "stage/manifest.tsv").read_bytes()
            )
            self.assertEqual((summary.compile_pass, summary.compile_fail), (0, 1))
            self.assertEqual(staged_tests[0].disposition, "compile-failed")
            self.assertFalse((root / "stage/bin").exists())


class ManifestTests(unittest.TestCase):
    def test_manifest_is_sorted_and_checksum_is_deterministic(self) -> None:
        first = replace(suite_test("conformance/interfaces/write/2-1.c"), binary="bin/conformance/interfaces/write/2-1.c.test", sha256="a" * 64)
        second = replace(suite_test("conformance/interfaces/write/1-1.c"), binary="bin/conformance/interfaces/write/1-1.c.test", sha256="b" * 64)

        text_a, digest_a = render_manifest(metadata(), (first, second))
        text_b, digest_b = render_manifest(metadata(), (second, first))

        self.assertEqual(text_a, text_b)
        self.assertEqual(digest_a, digest_b)
        self.assertEqual(hashlib.sha256(text_a.encode()).hexdigest() == digest_a, False)
        lines = text_a.splitlines()
        self.assertEqual(lines[0], "SMROS_POSIX_MANIFEST\t1")
        self.assertLess(lines.index(next(line for line in lines if "write/1-1.c" in line)), lines.index(next(line for line in lines if "write/2-1.c" in line)))

    def test_manifest_checksum_uses_zeroed_checksum_metadata_canonical_form(self) -> None:
        test = replace(suite_test(), binary="bin/conformance/interfaces/getpid/1-1.c.test", sha256="a" * 64)
        text, checksum = render_manifest(metadata(), (test,))
        canonical = text.replace(
            f"meta\tmanifest_sha256\t{checksum}",
            f"meta\tmanifest_sha256\t{EMPTY_SHA256}",
        )

        self.assertEqual(hashlib.sha256(canonical.encode()).hexdigest(), checksum)
        parse_manifest(text.encode())

    def test_manifest_row_has_exact_fields_and_required_metadata(self) -> None:
        test = replace(suite_test(), binary="bin/conformance/interfaces/getpid/1-1.c.test", sha256="a" * 64)
        text, _checksum = render_manifest(metadata(), (test,))
        lines = text.splitlines()
        test_line = next(line for line in lines if line.startswith("test\t"))

        self.assertEqual(len(test_line.split("\t")), 9)
        for key in (
            "source",
            "revision",
            "architecture",
            "compiler",
            "libc",
            "patch_sha256",
            "manifest_sha256",
            "smros_commit",
        ):
            self.assertTrue(any(line.startswith(f"meta\t{key}\t") for line in lines))

    def test_rejects_invalid_manifest_atoms_paths_timeouts_checksums_and_enums(self) -> None:
        base = replace(suite_test(), binary="bin/conformance/interfaces/getpid/1-1.c.test", sha256="a" * 64)
        invalid = (
            replace(base, test_id="bad\tid"),
            replace(base, test_id="bad\x01id"),
            replace(base, test_id="bad\u202eid"),
            replace(base, binary="bin/../escape.test"),
            replace(base, timeout_ms=0),
            replace(base, timeout_ms=2**32),
            replace(base, sha256="A" * 64),
            replace(base, sha256="a" * 63),
            replace(base, kind="unknown"),
            replace(base, disposition="unknown"),
        )

        for test in invalid:
            with self.subTest(test=test):
                with self.assertRaises(ValueError):
                    render_manifest(metadata(), (test,))

    def test_rejects_duplicate_ids_and_duplicate_staged_paths(self) -> None:
        base = replace(suite_test(), binary="bin/case.test", sha256="a" * 64)
        duplicate_id = replace(base, binary="bin/other.test")
        duplicate_path = replace(
            base,
            test_id="conformance/interfaces/getpid/2-1.c",
        )

        with self.assertRaisesRegex(ValueError, "duplicate.*ID"):
            render_manifest(metadata(), (base, duplicate_id))
        with self.assertRaisesRegex(ValueError, "duplicate.*path"):
            render_manifest(metadata(), (base, duplicate_path))

    def test_rejects_test_count_and_byte_size_limits(self) -> None:
        base = replace(suite_test(), binary="-", sha256=EMPTY_SHA256, disposition="compile-failed")
        too_many = tuple(
            replace(base, test_id=f"conformance/interfaces/getpid/{index}-1.c")
            for index in range(MAX_TESTS + 1)
        )
        with self.assertRaisesRegex(ValueError, "4,096"):
            render_manifest(metadata(), too_many)

        oversized = replace(base, test_id="conformance/" + "x" * MAX_MANIFEST_BYTES)
        with self.assertRaisesRegex(ValueError, "2 MiB"):
            render_manifest(metadata(), (oversized,))

    def test_parse_rejects_nondecimal_timeout_and_tampering(self) -> None:
        test = replace(suite_test(), binary="bin/conformance/interfaces/getpid/1-1.c.test", sha256="a" * 64)
        text, _checksum = render_manifest(metadata(), (test,))

        for invalid_timeout in ("+1", "01", "-1", "1_000"):
            with self.subTest(timeout=invalid_timeout):
                tampered = text.replace("\t30000\t", f"\t{invalid_timeout}\t")
                with self.assertRaises(ValueError):
                    parse_manifest(tampered.encode())
        with self.assertRaisesRegex(ValueError, "checksum"):
            parse_manifest(text.replace("\tbase\t", "\ttime\t").encode())


class StagingTests(unittest.TestCase):
    def test_safe_stage_path_rejects_escape_and_dot_components(self) -> None:
        for relative in ("../libc.so.6", "lib/../libc.so.6", "./libc.so.6", "/libc.so.6"):
            with self.subTest(relative=relative):
                with self.assertRaises(ValueError):
                    safe_stage_path(Path("stage"), relative)

    def test_safe_stage_path_accepts_nested_relative_path(self) -> None:
        self.assertEqual(
            safe_stage_path(Path("stage"), "bin/api/case.test"),
            Path("stage/bin/api/case.test"),
        )

    def test_parse_readelf_collects_interpreter_and_needed_entries(self) -> None:
        output = """
      [Requesting program interpreter: /lib/ld-linux-aarch64.so.1]
 0x0000000000000001 (NEEDED) Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED) Shared library: [libm.so.6]
"""
        interpreter, needed = parse_elf_dependencies(output)
        self.assertEqual(interpreter, "/lib/ld-linux-aarch64.so.1")
        self.assertEqual(needed, ("libc.so.6", "libm.so.6"))

    def test_runtime_resolution_handles_debian_cross_sysroot_layout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            libc = root / "usr/aarch64-linux-gnu/lib/libc.so.6"
            libc.parent.mkdir(parents=True)
            libc.write_bytes(b"libc")

            resolved = resolve_runtime_file(
                "libc.so.6", root, "aarch64-linux-gnu", ()
            )

        self.assertEqual(resolved, libc)

    def test_runtime_staging_preserves_requested_soname_basename(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sysroot = root / "sysroot"
            library_root = sysroot / "usr/aarch64-linux-gnu/lib"
            library_root.mkdir(parents=True)
            versioned = library_root / "libsample.so.1.2"
            versioned.write_bytes(b"library")
            (library_root / "libsample.so.1").symlink_to(versioned.name)
            executable = root / "case.test"
            executable.write_bytes(b"executable")
            stage = root / "stage"

            def query(_compiler: str, argument: str) -> str:
                return {
                    "-print-sysroot": str(sysroot),
                    "-print-multiarch": "aarch64-linux-gnu",
                    "-print-file-name=libc.so.6": str(library_root / "libc.so.6"),
                }[argument]

            def readelf(argv: list[str], **_kwargs: object) -> object:
                output = (
                    "(NEEDED) Shared library: [libsample.so.1]\n"
                    if Path(argv[-1]) == executable
                    else ""
                )
                return mock.Mock(returncode=0, stdout=output, stderr="")

            with mock.patch("scripts.posix.build.compiler_query", side_effect=query), mock.patch(
                "scripts.posix.build.run_bounded_command", side_effect=readelf
            ):
                stage_runtime_dependencies((executable,), stage)

            self.assertEqual((stage / "lib/libsample.so.1").read_bytes(), b"library")
            self.assertFalse((stage / "lib/libsample.so.1.2").exists())

    def test_verify_rejects_missing_or_changed_binary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(suite_test(), binary="bin/conformance/interfaces/getpid/1-1.c.test", sha256=hashlib.sha256(b"elf").hexdigest())
            write_stage_fixture(stage, test)

            readelf = lambda _argv, **_kwargs: mock.Mock(returncode=0, stdout="Machine: AArch64", stderr="")
            verify_stage(stage, readelf_runner=readelf)
            binary.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "checksum"):
                verify_stage(stage, readelf_runner=readelf)

    def test_verify_rejects_missing_runtime_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)

            def readelf(argv: list[str], **_kwargs: object) -> object:
                if "-h" in argv:
                    return mock.Mock(returncode=0, stdout="Machine: AArch64", stderr="")
                return mock.Mock(
                    returncode=0,
                    stdout=(
                        "[Requesting program interpreter: /lib/ld-linux-aarch64.so.1]\n"
                        "(NEEDED) Shared library: [libc.so.6]\n"
                    ),
                    stderr="",
                )

            with self.assertRaisesRegex(ValueError, "missing runtime"):
                verify_stage(stage, readelf_runner=readelf)

    def test_verify_rejects_symlinked_binary_parent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            stage.mkdir()
            outside = root / "outside"
            binary = outside / "conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            (stage / "bin").symlink_to(outside, target_is_directory=True)
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)

            with self.assertRaisesRegex(ValueError, "symlink"):
                verify_stage(stage, verify_architecture=False)

    def test_verify_rejects_changed_runtime_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            source = checkout / "conformance/interfaces/getpid/1-1.c"
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                if argv[0].endswith("gcc"):
                    output = Path(argv[argv.index("-o") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"artifact")
                stdout = "00000000 T main\n" if argv[0].endswith("nm") else ""
                return mock.Mock(returncode=0, stdout=stdout, stderr="")

            def stage_runtime(_executables: object, stage: Path) -> tuple[Path, ...]:
                runtime = stage / "lib/libc.so.6"
                runtime.parent.mkdir(parents=True)
                runtime.write_bytes(b"runtime")
                runtime.chmod(0o755)
                return (runtime,)

            build_campaign(
                checkout,
                (suite_test(),),
                (),
                metadata(),
                root / "stage",
                root / "work",
                command_runner=fake_run,
                dependency_stager=stage_runtime,
            )
            (root / "stage/lib/libc.so.6").write_bytes(b"changed")

            with self.assertRaisesRegex(ValueError, "runtime checksum"):
                verify_stage(root / "stage", verify_architecture=False)

    def test_verify_rejects_tampered_host_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            host = json.loads((stage / "manifest.json").read_text(encoding="utf-8"))
            host["tests"][0]["api"] = "tampered"
            (stage / "manifest.json").write_text(
                json.dumps(host, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "host manifest"):
                verify_stage(stage, verify_architecture=False)

    def test_verify_rejects_nonexecutable_runnable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            binary.chmod(0o644)

            with self.assertRaisesRegex(ValueError, "mode"):
                verify_stage(stage, verify_architecture=False)

    def test_verify_rejects_incomplete_build_results(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            (stage / "build-results.ndjson").write_text("", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "build result"):
                verify_stage(stage, verify_architecture=False)

    def test_verify_rejects_unmanifested_stage_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            (stage / "bin/extra.test").write_bytes(b"extra")

            with self.assertRaisesRegex(ValueError, "binary inventory"):
                verify_stage(stage, verify_architecture=False)

    def test_verify_rejects_unmanifested_root_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            (stage / "extra.txt").write_text("extra\n", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "stage file inventory"):
                verify_stage(stage, verify_architecture=False)

    def test_verify_rejects_link_result_that_contradicts_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            rows = [
                json.loads(line)
                for line in (stage / "build-results.ndjson")
                .read_text(encoding="utf-8")
                .splitlines()
            ]
            rows[-1].update(status="failed", returncode=1, artifact_sha256=None)
            (stage / "build-results.ndjson").write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "contradict"):
                verify_stage(stage, verify_architecture=False)

    def test_default_stage_readelf_uses_bounded_runner(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)

            def readelf(argv: list[str], **_kwargs: object) -> object:
                output = "Machine: AArch64" if "-h" in argv else ""
                return mock.Mock(returncode=0, stdout=output, stderr="")

            with mock.patch(
                "scripts.posix.build.run_bounded_command", side_effect=readelf
            ) as run:
                verify_stage(stage)
            self.assertGreaterEqual(run.call_count, 2)

    def test_existing_stage_publication_uses_atomic_directory_exchange(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            old_stage = root / "stage"
            old_stage.mkdir()
            new_stage = root / ".stage.tmp"
            new_stage.mkdir()

            with mock.patch.object(
                build_module, "_rename_exchange", create=True
            ) as exchange:
                build_module._publish_stage(new_stage, old_stage)

            exchange.assert_called_once_with(new_stage, old_stage)


class CliTests(unittest.TestCase):
    def test_registers_build_arguments(self) -> None:
        arguments = cli.create_parser().parse_args(
            ["build", "--arch", "aarch64", "--stage", "custom-stage", "--verify-only"]
        )
        self.assertEqual(arguments.command, "build")
        self.assertEqual(arguments.arch, "aarch64")
        self.assertEqual(arguments.stage, Path("custom-stage"))
        self.assertTrue(arguments.verify_only)

    def test_rejects_unsupported_architecture(self) -> None:
        stderr = mock.Mock()
        with mock.patch("sys.stderr", stderr):
            result = cli.main(["build", "--arch", "x86_64", "--stage", "stage"])
        self.assertEqual(result, 1)

    def test_build_inventory_requires_all_pinned_reviewed_cases(self) -> None:
        cli._validate_build_inventory(1_979, 176, 94, 169)
        for counts in (
            (1_978, 176, 94, 169),
            (1_979, 175, 94, 169),
            (1_979, 176, 93, 169),
            (1_979, 176, 94, 168),
        ):
            with self.subTest(counts=counts), self.assertRaisesRegex(
                ValueError, "inventory"
            ):
                cli._validate_build_inventory(*counts)

    def test_review_ledger_digest_rejects_count_preserving_swap(self) -> None:
        cli._validate_review_ledgers(cli.STUB_REVIEW_PATH, cli.SHELL_REVIEW_PATH)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stub = root / "stub-review.tsv"
            shell = root / "shell-review.tsv"
            stub_text = cli.STUB_REVIEW_PATH.read_text(encoding="utf-8")
            stub_text = stub_text.replace("\texclude-stub\t", "\tSWAP\t", 1)
            stub_text = stub_text.replace("\truntime-path\t", "\texclude-stub\t", 1)
            stub_text = stub_text.replace("\tSWAP\t", "\truntime-path\t", 1)
            stub.write_text(stub_text, encoding="utf-8")
            shell.write_bytes(cli.SHELL_REVIEW_PATH.read_bytes())

            with self.assertRaisesRegex(ValueError, "review ledger"):
                cli._validate_review_ledgers(stub, shell)

    def test_compiler_and_git_identity_use_bounded_runner(self) -> None:
        compiler = mock.Mock(returncode=0, stdout="fake gcc 1.0\n", stderr="")
        commit = mock.Mock(returncode=0, stdout="a" * 40 + "\n", stderr="")
        clean = mock.Mock(returncode=0, stdout="", stderr="")
        with mock.patch(
            "scripts.posix.cli.run_bounded_command",
            create=True,
            side_effect=(compiler, commit, clean),
        ) as run:
            self.assertEqual(cli._compiler_identity("fake-gcc"), "fake gcc 1.0")
            self.assertEqual(cli._smros_commit(), "a" * 40)
        self.assertEqual(run.call_count, 3)

    def test_smros_commit_rejects_dirty_relevant_tree(self) -> None:
        commit = mock.Mock(returncode=0, stdout="a" * 40 + "\n", stderr="")
        dirty = mock.Mock(
            returncode=0,
            stdout=" M scripts/posix/build.py\n",
            stderr="",
        )
        with mock.patch(
            "scripts.posix.cli.run_bounded_command",
            side_effect=(commit, dirty),
        ):
            with self.assertRaisesRegex(ValueError, "dirty"):
                cli._smros_commit()

    def test_gitignore_covers_owned_temporary_stage_directories(self) -> None:
        ignore = (cli.REPOSITORY_ROOT / ".gitignore").read_text(encoding="utf-8")
        self.assertIn("/host_shared/.posixtest.tmp-*/\n", ignore)
        self.assertIn("/host_shared/.posixtest.old-*/\n", ignore)


class ModelTests(unittest.TestCase):
    def test_build_summary_has_stable_counts(self) -> None:
        summary = BuildSummary(
            discovered=1,
            compile_pass=1,
            compile_fail=0,
            link_pass=1,
            link_fail=0,
            shell_unported=169,
            staged_bytes=1024,
        )
        self.assertEqual(
            summary.format_counts(),
            "discovered=1 build-pass=1 build-fail=0 "
            "link-pass=1 link-fail=0 shell-unported=169",
        )
        with self.assertRaises(Exception):
            summary.discovered = 2


if __name__ == "__main__":
    unittest.main()
