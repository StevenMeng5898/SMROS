import hashlib
import io
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
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


def checksummed_manifest_without_validation(test: SuiteTest) -> bytes:
    canonical_metadata = replace(metadata(), manifest_sha256=EMPTY_SHA256)
    canonical = build_module._manifest_text(canonical_metadata, (test,))
    checksum = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    text = build_module._manifest_text(
        replace(canonical_metadata, manifest_sha256=checksum),
        (test,),
    )
    return text.encode("utf-8")


def write_stage_fixture(stage: Path, test: SuiteTest) -> None:
    checkout = Path("target/posix/src") / ("8" * 40)
    object_path = Path("target/posix/aarch64/obj") / f"{test.test_id}.o"
    executable = Path("target/posix/aarch64/bin") / f"{test.test_id}.test"
    build_results = (
        BuildResult(
            test_id=test.test_id,
            stage="compile",
            status="passed",
            argv=tuple(
                compile_command(
                    "aarch64-linux-gnu-gcc",
                    checkout / test.source,
                    object_path,
                    checkout / "include",
                )
            ),
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
            argv=tuple(nm_command("aarch64-linux-gnu-nm", object_path)),
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
            argv=tuple(
                link_command("aarch64-linux-gnu-gcc", object_path, executable)
            ),
            returncode=0,
            stdout="",
            stderr="",
            duration_ms=1,
            artifact_sha256=test.sha256,
        ),
    )
    build_results_text = "".join(
        json.dumps(asdict(result), sort_keys=True, separators=(",", ":")) + "\n"
        for result in build_results
    )
    bound_metadata = replace(
        metadata(),
        build_results_sha256=build_module._build_results_digest(build_results),
    )
    manifest_text, _ = render_manifest(bound_metadata, (test,))
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
    (stage / "manifest.tsv").write_text(manifest_text, encoding="utf-8")
    (stage / "manifest.json").write_text(
        json.dumps(host_manifest, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    (stage / "build-results.ndjson").write_text(
        build_results_text,
        encoding="utf-8",
    )
    if test.binary not in {None, "-"}:
        (stage / test.binary).chmod(0o755)


def run_fake_campaign(
    root: Path,
    stage: Path,
    *,
    work: Path | None = None,
) -> BuildSummary:
    checkout = root / "checkout"
    source = checkout / "conformance/interfaces/getpid/1-1.c"
    source.parent.mkdir(parents=True, exist_ok=True)
    source.write_text("int main(void) { return 0; }\n", encoding="utf-8")

    def fake_run(argv: list[str], **_kwargs: object) -> object:
        if argv[0].endswith("gcc"):
            output = Path(argv[argv.index("-o") + 1])
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(b"artifact")
        stdout = "00000000 T main\n" if argv[0].endswith("nm") else ""
        return mock.Mock(returncode=0, stdout=stdout, stderr="")

    return build_campaign(
        checkout,
        (suite_test(),),
        (),
        metadata(),
        stage,
        work if work is not None else root / "work",
        command_runner=fake_run,
        dependency_stager=mock.Mock(return_value=()),
    )


def assert_idle_reusable_stage_slot(test: unittest.TestCase, parent: Path) -> None:
    work_root = parent / build_module._STAGE_QUARANTINE_NAME
    stage_slots = tuple(work_root.iterdir())
    test.assertEqual(len(stage_slots), 1)
    entries = {entry.name: entry for entry in stage_slots[0].iterdir()}
    test.assertEqual(
        set(entries),
        {
            build_module._STAGE_WORK_ROOT_NAME,
            build_module._STAGE_JOURNAL_NAME,
        },
    )
    test.assertEqual(
        tuple(entries[build_module._STAGE_WORK_ROOT_NAME].iterdir()),
        (),
    )
    slot_descriptor = os.open(stage_slots[0], os.O_RDONLY | os.O_DIRECTORY)
    try:
        journal = build_module._load_stage_journal(slot_descriptor)
    finally:
        os.close(slot_descriptor)
    test.assertEqual(journal, {"schema": 1, "state": "idle"})


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

    def test_interruption_kills_process_group_and_reaps_child(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            pid_file = Path(temporary) / "pids"
            program = (
                "import os,pathlib,subprocess,sys,time;"
                "child=subprocess.Popen([sys.executable,'-c',"
                "'import time;time.sleep(30)']);"
                "pid_path=pathlib.Path(sys.argv[1]);"
                "pid_path.write_text('',encoding='ascii');"
                "time.sleep(1);"
                "pid_path.write_text("
                "f'{os.getpid()} {child.pid}',encoding='ascii');"
                "time.sleep(30)"
            )
            real_popen = subprocess.Popen
            real_wait = real_popen.wait
            real_poll = real_popen.poll
            processes: list[subprocess.Popen[bytes]] = []
            interrupted = [False]
            original_error = KeyboardInterrupt("interrupted")

            def tracked_popen(*args: object, **kwargs: object) -> subprocess.Popen[bytes]:
                process = real_popen(*args, **kwargs)
                processes.append(process)
                return process

            def interrupt_once(process: subprocess.Popen[bytes]) -> int | None:
                if not interrupted[0]:
                    deadline = time.monotonic() + 5.0
                    while True:
                        try:
                            ready_pids = tuple(
                                int(value)
                                for value in pid_file.read_text(
                                    encoding="ascii"
                                ).split()
                            )
                        except (FileNotFoundError, ValueError):
                            ready_pids = ()
                        if len(ready_pids) == 2 and all(
                            pid > 0 for pid in ready_pids
                        ):
                            break
                        if time.monotonic() >= deadline:
                            self.fail(
                                "helper did not publish two positive PIDs before "
                                "the startup deadline"
                            )
                        time.sleep(0.01)
                    interrupted[0] = True
                    raise original_error
                return real_poll(process)

            def survives(pid: int) -> bool:
                try:
                    state = Path(f"/proc/{pid}/stat").read_text(
                        encoding="ascii"
                    ).split()[2]
                except (FileNotFoundError, ProcessLookupError):
                    return False
                return state not in {"X", "Z"}

            pids: tuple[int, ...] = ()
            try:
                with mock.patch(
                    "scripts.posix.build.subprocess.Popen",
                    side_effect=tracked_popen,
                ), mock.patch.object(real_popen, "poll", new=interrupt_once):
                    with self.assertRaises(KeyboardInterrupt) as raised:
                        build_module.run_bounded_command(
                            [sys.executable, "-c", program, str(pid_file)]
                        )
                self.assertIs(raised.exception, original_error)
                pids = tuple(
                    int(value)
                    for value in pid_file.read_text(encoding="ascii").split()
                )
                self.assertEqual(len(pids), 2)
                deadline = time.monotonic() + 2.0
                while any(survives(pid) for pid in pids) and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertFalse(any(survives(pid) for pid in pids))
                self.assertIsNotNone(processes[0].returncode)
            finally:
                for pid in pids:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                for process in processes:
                    if process.returncode is None:
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        real_wait(process)

    def test_selector_registration_interruption_reaps_child_and_preserves_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            pid_file = Path(temporary) / "pid"
            program = (
                "import os,pathlib,sys,time;"
                "pathlib.Path(sys.argv[1]).write_text("
                "str(os.getpid()),encoding='ascii');"
                "time.sleep(30)"
            )
            real_popen = subprocess.Popen
            real_wait = real_popen.wait
            process: subprocess.Popen[bytes] | None = None
            original_error = KeyboardInterrupt("reader startup interrupted")

            def tracked_popen(*args: object, **kwargs: object) -> subprocess.Popen[bytes]:
                nonlocal process
                process = real_popen(*args, **kwargs)
                return process

            def interrupt_registration(
                _selector: object,
                _fileobj: object,
                _events: int,
                _data: object = None,
            ) -> None:
                deadline = time.monotonic() + 2.0
                while not pid_file.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                raise original_error

            try:
                with mock.patch(
                    "scripts.posix.build.subprocess.Popen",
                    side_effect=tracked_popen,
                ), mock.patch(
                    "scripts.posix.build.selectors.DefaultSelector.register",
                    new=interrupt_registration,
                ):
                    with self.assertRaises(KeyboardInterrupt) as raised:
                        build_module.run_bounded_command(
                            [sys.executable, "-c", program, str(pid_file)]
                        )
                self.assertIs(raised.exception, original_error)
                self.assertIsNotNone(process)
                assert process is not None
                self.assertIsNotNone(process.returncode)
            finally:
                if process is not None and process.returncode is None:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    real_wait(process)

    def test_cleanup_failure_preserves_interruption_and_uses_bounded_wait(self) -> None:
        original_error = KeyboardInterrupt("original interruption")
        stdout_read, stdout_write = os.pipe()
        stderr_read, stderr_write = os.pipe()
        os.close(stdout_write)
        os.close(stderr_write)
        process = mock.Mock(
            pid=12345,
            stdin=None,
            stdout=os.fdopen(stdout_read, "rb"),
            stderr=os.fdopen(stderr_read, "rb"),
            returncode=None,
        )
        process.poll.side_effect = original_error
        process.wait.return_value = 0

        with mock.patch(
            "scripts.posix.build.subprocess.Popen",
            return_value=process,
        ), mock.patch(
            "scripts.posix.build.os.killpg",
            side_effect=PermissionError("cleanup denied"),
        ):
            with self.assertRaises(KeyboardInterrupt) as raised:
                build_module.run_bounded_command(["fake-command"])

        self.assertIs(raised.exception, original_error)
        self.assertGreaterEqual(process.wait.call_count, 1)
        for call in process.wait.call_args_list:
            self.assertIn("timeout", call.kwargs)
            self.assertLessEqual(call.kwargs["timeout"], 1.0)

    def test_selector_setup_interruption_uses_process_cleanup_boundary(self) -> None:
        original_error = MemoryError("selector setup interrupted")
        process = mock.Mock(
            pid=12345,
            stdin=None,
            stdout=io.BytesIO(),
            stderr=io.BytesIO(),
            returncode=None,
        )
        process.wait.return_value = 0

        with mock.patch(
            "scripts.posix.build.subprocess.Popen",
            return_value=process,
        ), mock.patch(
            "scripts.posix.build.selectors.DefaultSelector",
            side_effect=original_error,
        ), mock.patch("scripts.posix.build.os.killpg") as kill:
            with self.assertRaises(MemoryError) as raised:
                build_module.run_bounded_command(["fake-command"])

        self.assertIs(raised.exception, original_error)
        kill.assert_called_once_with(process.pid, signal.SIGKILL)
        process.wait.assert_called_once()
        self.assertIn("timeout", process.wait.call_args.kwargs)

    def test_exception_cleanup_does_not_block_on_buffered_pipe_close(self) -> None:
        stdout_read, stdout_write = os.pipe()
        stderr_read, stderr_write = os.pipe()
        stdout = os.fdopen(stdout_read, "rb")
        stderr = os.fdopen(stderr_read, "rb")
        original_error = KeyboardInterrupt("interrupted with inherited pipes")
        process = mock.Mock(
            pid=12345,
            stdin=None,
            stdout=stdout,
            stderr=stderr,
            returncode=None,
        )
        process.poll.side_effect = original_error
        process.wait.return_value = 0
        observed: list[BaseException] = []

        def invoke() -> None:
            try:
                build_module.run_bounded_command(["fake-command"])
            except BaseException as error:
                observed.append(error)

        runner = threading.Thread(target=invoke, daemon=True)
        replacement_descriptor: int | None = None
        replacement_survived = False
        try:
            with mock.patch(
                "scripts.posix.build.subprocess.Popen",
                return_value=process,
            ), mock.patch("scripts.posix.build.os.killpg"):
                runner.start()
                runner.join(2.5)
                completed_boundedly = not runner.is_alive()
                if completed_boundedly:
                    replacement_descriptor = os.open("/dev/null", os.O_RDONLY)
        finally:
            os.close(stdout_write)
            os.close(stderr_write)
            runner.join(2.0)
            if replacement_descriptor is not None:
                time.sleep(0.05)
                try:
                    os.fstat(replacement_descriptor)
                except OSError:
                    replacement_survived = False
                else:
                    replacement_survived = True
                    os.close(replacement_descriptor)

        self.assertTrue(completed_boundedly)
        self.assertTrue(replacement_survived)
        self.assertEqual(observed, [original_error])

    def test_compiler_query_uses_bounded_runner(self) -> None:
        completed = mock.Mock(returncode=0, stdout="/sysroot\n", stderr="")
        with mock.patch(
            "scripts.posix.build.run_bounded_command", return_value=completed
        ) as run:
            self.assertEqual(compiler_query("fake-gcc", "-print-sysroot"), "/sysroot")
        run.assert_called_once_with(["fake-gcc", "-print-sysroot"])


class CampaignTests(unittest.TestCase):
    def test_work_reset_uses_held_parent_after_ancestor_swap(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work_parent = root / "work-parent"
            work_parent.mkdir()
            work = work_parent / "work"
            moved_parent = root / "work-parent-moved"
            stage = root / "stage-parent/stage"
            open_locks = build_module._open_campaign_lock_directories

            def swap_after_lock(*args: object, **kwargs: object) -> object:
                result = open_locks(*args, **kwargs)
                work_parent.rename(moved_parent)
                work_parent.mkdir()
                victim = work_parent / "work/obj/victim"
                victim.parent.mkdir(parents=True)
                victim.write_bytes(b"replacement")
                return result

            with mock.patch(
                "scripts.posix.build._open_campaign_lock_directories",
                side_effect=swap_after_lock,
            ):
                run_fake_campaign(root / "campaign", stage, work=work)

            self.assertEqual(
                (work_parent / "work/obj/victim").read_bytes(),
                b"replacement",
            )
            self.assertTrue(stage.is_dir())

    def test_compile_uses_held_checkout_after_checkout_swap(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            test = suite_test()
            source = checkout / test.source
            source.parent.mkdir(parents=True)
            source.write_text("original source\n", encoding="utf-8")
            moved_checkout = root / "checkout-moved"
            observed_sources: list[str] = []
            swapped = False

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                nonlocal swapped
                command = list(argv)
                if "-c" in command:
                    if not swapped:
                        checkout.rename(moved_checkout)
                        replacement = checkout / test.source
                        replacement.parent.mkdir(parents=True)
                        replacement.write_text(
                            "replacement source\n",
                            encoding="utf-8",
                        )
                        swapped = True
                    observed_sources.append(
                        Path(command[command.index("-c") + 1]).read_text(
                            encoding="utf-8"
                        )
                    )
                if command[0].endswith("gcc"):
                    output = Path(command[command.index("-o") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"artifact")
                stdout = "00000000 T main\n" if command[0].endswith("nm") else ""
                return mock.Mock(returncode=0, stdout=stdout, stderr="")

            build_campaign(
                checkout,
                (test,),
                (),
                metadata(),
                root / "stage",
                root / "work",
                command_runner=fake_run,
                dependency_stager=mock.Mock(return_value=()),
            )
            rows = [
                json.loads(line)
                for line in (root / "stage/build-results.ndjson")
                .read_text(encoding="utf-8")
                .splitlines()
            ]

            self.assertEqual(observed_sources, ["original source\n"])
            self.assertEqual(
                (checkout / test.source).read_text(encoding="utf-8"),
                "replacement source\n",
            )
            compile_row = next(row for row in rows if row["stage"] == "compile")
            compile_argv = compile_row["argv"]
            self.assertEqual(
                compile_argv[compile_argv.index("-c") + 1],
                str(source),
            )

    def test_fork_message_catalog_test_stages_generated_support_catalog(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            test = replace(
                suite_test("conformance/interfaces/fork/7-1.c"),
                api="fork",
            )
            source = checkout / test.source
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            catalog_source = source.parent / "messcat_src.txt"
            catalog_source.write_text(
                "$set 1 test messages\n1 generated\n",
                encoding="utf-8",
                newline="\n",
            )
            support_path = (
                root / "stage" / "conformance/interfaces/fork/mess.cat"
            )

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                command = list(argv)
                if command[0].endswith("gcc"):
                    output = Path(command[command.index("-o") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"artifact")
                stdout = "00000000 T main\n" if command[0].endswith("nm") else ""
                return mock.Mock(returncode=0, stdout=stdout, stderr="")

            def fake_bounded(argv: list[str], **_kwargs: object) -> object:
                self.assertEqual(argv[0], "gencat")
                self.assertEqual(Path(argv[2]).read_text(encoding="utf-8"), catalog_source.read_text(encoding="utf-8"))
                destination = Path(argv[1])
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(b"generated catalog")
                return mock.Mock(returncode=0, stdout="", stderr="")

            with mock.patch(
                "scripts.posix.build.run_bounded_command",
                side_effect=fake_bounded,
            ):
                build_campaign(
                    checkout,
                    (test,),
                    (),
                    metadata(),
                    root / "stage",
                    root / "work",
                    command_runner=fake_run,
                    dependency_stager=mock.Mock(return_value=()),
                )

            self.assertEqual(support_path.read_bytes(), b"generated catalog")
            self.assertEqual(support_path.stat().st_mode & 0o777, 0o644)
            host_manifest = json.loads(
                (root / "stage/manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                host_manifest["support"],
                [
                    {
                        "path": "conformance/interfaces/fork/mess.cat",
                        "sha256": hashlib.sha256(b"generated catalog").hexdigest(),
                    }
                ],
            )
            verify_stage(root / "stage", verify_architecture=False)

    def test_fork_message_catalog_real_gencat_works_with_stage_descriptor(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            test = replace(
                suite_test("conformance/interfaces/fork/7-1.c"),
                api="fork",
            )
            source = checkout / test.source
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
            (source.parent / "messcat_src.txt").write_text(
                "$set 1 test messages\n1 generated\n",
                encoding="utf-8",
                newline="\n",
            )

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                command = list(argv)
                if command[0].endswith("gcc"):
                    output = Path(command[command.index("-o") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"artifact")
                stdout = "00000000 T main\n" if command[0].endswith("nm") else ""
                return mock.Mock(returncode=0, stdout=stdout, stderr="")

            build_campaign(
                checkout,
                (test,),
                (),
                metadata(),
                root / "stage",
                root / "work",
                command_runner=fake_run,
                dependency_stager=mock.Mock(return_value=()),
            )
            self.assertGreater(
                (root / "stage/conformance/interfaces/fork/mess.cat").stat().st_size,
                0,
            )

    def test_execution_descriptor_diagnostics_are_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)

            def run(index: int, padding_count: int) -> tuple[str, str]:
                campaign_root = root / f"campaign-{index}"
                campaign_root.mkdir()
                checkout = campaign_root / "checkout"
                test = suite_test()
                source = checkout / test.source
                source.parent.mkdir(parents=True)
                source.write_text(
                    "int main(void) { return 0; }\n",
                    encoding="utf-8",
                )
                padding = [
                    os.open("/dev/null", os.O_RDONLY)
                    for _ in range(padding_count)
                ]

                def fake_run(argv: list[str], **_kwargs: object) -> object:
                    command = list(argv)
                    nested_diagnostic = ""
                    if command[0].endswith("gcc"):
                        output = Path(command[command.index("-o") + 1])
                        output.parent.mkdir(parents=True, exist_ok=True)
                        output.write_bytes(b"artifact")
                    if "-c" in command:
                        execution_source = command[command.index("-c") + 1]
                        checkout_root = re.match(
                            r"(/proc/self/fd/[0-9]+)/",
                            execution_source,
                        )
                        assert checkout_root is not None
                        nested_diagnostic = (
                            "\n"
                            + checkout_root.group(1)
                            + "/conformance/shared-header.c"
                        )
                    stdout = (
                        "00000000 T main\n"
                        if command[0].endswith("nm")
                        else ""
                    )
                    return mock.Mock(
                        returncode=0,
                        stdout=stdout,
                        stderr=(
                            "command: "
                            + " ".join(command)
                            + nested_diagnostic
                        ),
                    )

                previous = Path.cwd()
                try:
                    os.chdir(campaign_root)
                    build_campaign(
                        Path("checkout"),
                        (test,),
                        (),
                        metadata(),
                        Path("stage"),
                        Path("work"),
                        command_runner=fake_run,
                        dependency_stager=mock.Mock(return_value=()),
                    )
                finally:
                    os.chdir(previous)
                    for descriptor in padding:
                        os.close(descriptor)
                manifest = json.loads(
                    (campaign_root / "stage/manifest.json").read_text(
                        encoding="utf-8"
                    )
                )
                results = (campaign_root / "stage/build-results.ndjson").read_text(
                    encoding="utf-8"
                )
                return manifest["metadata"]["build_results_sha256"], results

            first_digest, first_results = run(1, 0)
            second_digest, second_results = run(2, 20)

            self.assertEqual(first_digest, second_digest)
            self.assertNotIn("/proc/self/fd/", first_results)
            self.assertNotIn("/proc/self/fd/", second_results)

    def test_repeated_failures_reuse_one_empty_stage_work_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            test = suite_test()
            source = checkout / test.source
            source.parent.mkdir(parents=True)
            source.write_text("int main(void) { return 0; }\n", encoding="utf-8")

            for _ in range(3):
                with self.assertRaisesRegex(ValueError, "toolchain"):
                    build_campaign(
                        checkout,
                        (test,),
                        (),
                        metadata(),
                        root / "stage",
                        root / "work",
                        command_runner=mock.Mock(
                            side_effect=FileNotFoundError("compiler disappeared")
                        ),
                        dependency_stager=mock.Mock(return_value=()),
                    )

            assert_idle_reusable_stage_slot(self, root)

    def test_repeated_replacements_reuse_one_empty_stage_work_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"

            for _ in range(3):
                run_fake_campaign(root, stage)

            assert_idle_reusable_stage_slot(self, root)

    def test_same_stage_campaigns_are_serialized_by_the_work_slot(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            first_entered = threading.Event()
            second_entered = threading.Event()
            release = threading.Event()
            write_lock = threading.Lock()
            errors: list[BaseException] = []
            write_count = 0
            write_manifests = build_module._write_manifests

            def blocking_write(*args: object, **kwargs: object) -> str:
                nonlocal write_count
                with write_lock:
                    write_count += 1
                    current = write_count
                (first_entered if current == 1 else second_entered).set()
                if not release.wait(5.0):
                    raise AssertionError("timed out waiting to release manifest write")
                return write_manifests(*args, **kwargs)

            def campaign(campaign_root: Path) -> None:
                try:
                    run_fake_campaign(campaign_root, stage)
                except BaseException as error:
                    errors.append(error)

            first = threading.Thread(target=campaign, args=(root / "first",))
            second = threading.Thread(target=campaign, args=(root / "second",))
            with mock.patch(
                "scripts.posix.build._write_manifests",
                side_effect=blocking_write,
            ):
                try:
                    first.start()
                    self.assertTrue(first_entered.wait(2.0))
                    second.start()
                    self.assertFalse(second_entered.wait(0.2))
                finally:
                    release.set()
                    first.join(5.0)
                    second.join(5.0)

            self.assertFalse(first.is_alive())
            self.assertFalse(second.is_alive())
            self.assertEqual(errors, [])
            self.assertTrue(second_entered.is_set())

    def test_different_stage_parents_with_same_work_are_serialized(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stages = (root / "stages-one/stage", root / "stages-two/stage")
            shared_work = root / "shared-work"
            first_entered = threading.Event()
            second_entered = threading.Event()
            release = threading.Event()
            write_lock = threading.Lock()
            errors: list[BaseException] = []
            write_count = 0
            write_manifests = build_module._write_manifests

            def blocking_write(*args: object, **kwargs: object) -> str:
                nonlocal write_count
                with write_lock:
                    write_count += 1
                    current = write_count
                (first_entered if current == 1 else second_entered).set()
                if not release.wait(5.0):
                    raise AssertionError("timed out waiting to release manifest write")
                return write_manifests(*args, **kwargs)

            def campaign(index: int) -> None:
                try:
                    run_fake_campaign(
                        root / f"campaign-{index}",
                        stages[index],
                        work=shared_work,
                    )
                except BaseException as error:
                    errors.append(error)

            first = threading.Thread(target=campaign, args=(0,))
            second = threading.Thread(target=campaign, args=(1,))
            with mock.patch(
                "scripts.posix.build._write_manifests",
                side_effect=blocking_write,
            ):
                try:
                    first.start()
                    self.assertTrue(first_entered.wait(2.0))
                    second.start()
                    self.assertFalse(second_entered.wait(0.2))
                finally:
                    release.set()
                    first.join(5.0)
                    second.join(5.0)

            self.assertFalse(first.is_alive())
            self.assertFalse(second.is_alive())
            self.assertEqual(errors, [])
            self.assertTrue(second_entered.is_set())
            for stage in stages:
                summary = verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )
                self.assertEqual(summary.compile_pass, 1)

    def test_campaign_locks_each_directory_inode_once(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            locked_inodes: list[tuple[int, int]] = []
            quarantine = root / build_module._STAGE_QUARANTINE_NAME
            quarantine.mkdir(mode=0o700)
            slot = quarantine / build_module._stage_work_slot_name("stage")
            slot.mkdir(mode=0o700)

            def record_lock(descriptor: int, operation: int) -> None:
                info = os.fstat(descriptor)
                identity = (info.st_dev, info.st_ino)
                if identity in locked_inodes:
                    raise AssertionError("directory inode was locked twice")
                locked_inodes.append(identity)

            with mock.patch(
                "scripts.posix.build.fcntl.flock",
                side_effect=record_lock,
            ):
                run_fake_campaign(
                    root,
                    root / "stage",
                    work=slot / "work",
                )

            self.assertEqual(len(locked_inodes), 2)
            self.assertEqual(len(set(locked_inodes)), 2)

    def test_campaign_fsyncs_new_quarantine_and_slot_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage_parent = root / "stage-parent"
            work_parent = root / "work-parent"
            fsynced: list[tuple[int, int]] = []

            def record_fsync(descriptor: int) -> None:
                info = os.fstat(descriptor)
                fsynced.append((info.st_dev, info.st_ino))

            with mock.patch(
                "scripts.posix.build.os.fsync",
                side_effect=record_fsync,
            ):
                (
                    stage_parent_descriptor,
                    _work_parent_descriptor,
                    _slot_descriptor,
                    lock_descriptors,
                ) = build_module._open_campaign_lock_directories(
                    stage_parent,
                    work_parent,
                    "stage",
                )
            try:
                stage_parent_identity = build_module._descriptor_identity(
                    stage_parent_descriptor
                )
                quarantine_info = (
                    stage_parent / build_module._STAGE_QUARANTINE_NAME
                ).stat()
                quarantine_identity = (
                    quarantine_info.st_dev,
                    quarantine_info.st_ino,
                )
            finally:
                for descriptor in reversed(lock_descriptors):
                    os.close(descriptor)

            self.assertIn(stage_parent_identity, fsynced)
            self.assertIn(quarantine_identity, fsynced)
            self.assertLess(
                fsynced.index(stage_parent_identity),
                fsynced.index(quarantine_identity),
            )

    def test_campaigns_with_opposing_slot_and_work_roles_do_not_deadlock(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first_stage = root / "stage-a"
            second_stage = root / "stage-b"
            quarantine = root / build_module._STAGE_QUARANTINE_NAME
            quarantine.mkdir(mode=0o700)
            first_slot = quarantine / build_module._stage_work_slot_name(
                first_stage.name
            )
            first_slot.mkdir(mode=0o700)
            entered = threading.Event()
            release = threading.Event()
            errors: list[BaseException] = []
            activate = build_module._activate_stage_work_slot

            def blocking_activate(*args: object, **kwargs: object) -> int:
                descriptor = activate(*args, **kwargs)
                if args[2] == first_stage.name:
                    entered.set()
                    if not release.wait(5.0):
                        os.close(descriptor)
                        raise AssertionError("timed out waiting to release first slot")
                return descriptor

            def campaign(
                campaign_root: Path,
                stage: Path,
                work: Path,
            ) -> None:
                try:
                    run_fake_campaign(campaign_root, stage, work=work)
                except BaseException as error:
                    errors.append(error)

            first = threading.Thread(
                target=campaign,
                args=(root / "campaign-a", first_stage, root / "work-a"),
            )
            second = threading.Thread(
                target=campaign,
                args=(root / "campaign-b", second_stage, first_slot / "work"),
            )
            with mock.patch(
                "scripts.posix.build._activate_stage_work_slot",
                side_effect=blocking_activate,
            ):
                try:
                    first.start()
                    self.assertTrue(entered.wait(2.0))
                    second.start()
                finally:
                    release.set()
                    first.join(5.0)
                    second.join(5.0)

            self.assertFalse(first.is_alive())
            self.assertFalse(second.is_alive())
            self.assertEqual(errors, [])

    def test_build_rejects_symlinked_stage_grandparent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            real_parent = root / "real-parent"
            (real_parent / "nested").mkdir(parents=True)
            alias = root / "alias"
            alias.symlink_to(real_parent, target_is_directory=True)

            with self.assertRaisesRegex(ValueError, "symlink"):
                run_fake_campaign(root, alias / "nested/stage")

            self.assertFalse((real_parent / "nested/stage").exists())

    def test_build_publication_uses_held_parent_after_ancestor_swap(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent = root / "publish-parent"
            parent.mkdir()
            moved_parent = root / "publish-parent-moved"
            outside = root / "outside"
            outside.mkdir()
            publish = build_module._publish_stage

            def swap_and_publish(
                *args: object,
                **kwargs: object,
            ) -> object:
                parent.rename(moved_parent)
                parent.symlink_to(outside, target_is_directory=True)
                return publish(*args, **kwargs)

            with mock.patch(
                "scripts.posix.build._publish_stage",
                side_effect=swap_and_publish,
            ):
                run_fake_campaign(root, parent / "stage")

            self.assertTrue((moved_parent / "stage").is_dir())
            self.assertFalse((outside / "stage").exists())
            self.assertEqual(
                tuple(moved_parent.glob(".stage.tmp-*")),
                (),
            )

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

    def test_definition_source_is_compile_only(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            definition = suite_test(
                "conformance/definitions/unistd_h/1-1.c",
                kind="definition",
                disposition="definition-only",
            )
            source = checkout / definition.source
            source.parent.mkdir(parents=True)
            source.write_text("int declaration;\n", encoding="utf-8")
            commands: list[list[str]] = []

            def fake_run(argv: list[str], **_kwargs: object) -> object:
                commands.append(list(argv))
                output = Path(argv[argv.index("-o") + 1])
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_bytes(b"object")
                return mock.Mock(returncode=0, stdout="", stderr="")

            build_campaign(
                checkout,
                (definition,),
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

        self.assertEqual(len(commands), 1)
        self.assertEqual([record["stage"] for record in records], ["compile"])

    def test_definition_verification_rejects_nm_record(self) -> None:
        definition = replace(
            suite_test(
                "conformance/definitions/unistd_h/1-1.c",
                kind="definition",
                disposition="definition-only",
            ),
            binary="-",
            sha256=EMPTY_SHA256,
        )
        compile_result = BuildResult(
            test_id=definition.test_id,
            stage="compile",
            status="passed",
            argv=tuple(
                compile_command(
                    "aarch64-linux-gnu-gcc",
                    Path("checkout") / definition.source,
                    Path("work") / f"{definition.test_id}.o",
                    Path("checkout/include"),
                )
            ),
            returncode=0,
            stdout="",
            stderr="",
            duration_ms=1,
            artifact_sha256="a" * 64,
        )
        nm_result = replace(
            compile_result,
            stage="nm",
            argv=tuple(
                nm_command(
                    "aarch64-linux-gnu-nm",
                    Path("work") / f"{definition.test_id}.o",
                )
            ),
            artifact_sha256=None,
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "results.ndjson"
            path.write_text(
                "".join(
                    json.dumps(asdict(result), sort_keys=True, separators=(",", ":"))
                    + "\n"
                    for result in (compile_result, nm_result)
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "definition.*nm"):
                build_module._load_build_results(path, (definition,))

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
            verified = verify_stage(
                root / "stage",
                verify_architecture=False,
                expected_metadata=metadata(),
            )

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
    def test_build_results_digest_normalizes_only_duration(self) -> None:
        test = replace(
            suite_test(),
            binary="bin/conformance/interfaces/getpid/1-1.c.test",
            sha256="a" * 64,
        )
        result = BuildResult(
            test_id=test.test_id,
            stage="compile",
            status="passed",
            argv=("aarch64-linux-gnu-gcc", "-c", test.source),
            returncode=0,
            stdout="stdout",
            stderr="stderr",
            duration_ms=1,
            artifact_sha256="a" * 64,
        )

        def write(results: tuple[BuildResult, ...]) -> tuple[str, str, bytes]:
            stage = root / str(len(list(root.iterdir())))
            stage.mkdir()
            manifest_digest = build_module._write_manifests(
                stage, metadata(), (test,), results
            )
            manifest_metadata, _tests = parse_manifest(
                (stage / "manifest.tsv").read_bytes()
            )
            return (
                manifest_metadata.build_results_sha256,
                manifest_digest,
                (stage / "build-results.ndjson").read_bytes(),
            )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            base_digest, base_manifest_digest, base_raw = write((result,))
            duration_digest, duration_manifest_digest, duration_raw = write(
                (replace(result, duration_ms=999),)
            )
            stable_mutations = {
                "test_id": replace(result, test_id="other/test.c"),
                "stage": replace(result, stage="link"),
                "status": replace(result, status="failed"),
                "argv": replace(result, argv=("other-compiler",)),
                "returncode": replace(result, returncode=1),
                "stdout": replace(result, stdout="changed stdout"),
                "stderr": replace(result, stderr="changed stderr"),
                "artifact_sha256": replace(result, artifact_sha256="b" * 64),
            }
            mutated_digests = {
                name: write((mutated,))[0]
                for name, mutated in stable_mutations.items()
            }

        self.assertNotEqual(base_raw, duration_raw)
        self.assertEqual(base_digest, duration_digest)
        self.assertEqual(base_manifest_digest, duration_manifest_digest)
        for name, digest in mutated_digests.items():
            with self.subTest(field=name):
                self.assertNotEqual(base_digest, digest)

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
            "build_results_sha256",
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
            replace(base, test_id="conformance/interfaces/getpid/1-\u00e9.c"),
            replace(base, binary="bin/conformance/interfaces/getpid/1-\u00e9.c.test"),
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

    def test_kind_disposition_matrix_matches_guest_consumer(self) -> None:
        allowed = {
            ("runnable", "complete"),
            ("runnable", "excluded-upstream-stub"),
            ("runnable", "compile-failed"),
            ("runnable", "link-failed"),
            ("definition", "definition-only"),
            ("definition", "excluded-upstream-stub"),
            ("definition", "compile-failed"),
            ("shell", "not-built-shell-test"),
        }
        kinds = ("runnable", "definition", "shell")
        dispositions = (
            "complete",
            "definition-only",
            "excluded-upstream-stub",
            "compile-failed",
            "link-failed",
            "not-built-shell-test",
        )

        for kind in kinds:
            for disposition in dispositions:
                test = replace(
                    suite_test(kind=kind, disposition=disposition),
                    binary=("bin/case.test" if disposition == "complete" else "-"),
                    sha256=("a" * 64 if disposition == "complete" else EMPTY_SHA256),
                )
                pair = (kind, disposition)
                if pair in allowed:
                    render_manifest(metadata(), (test,))
                    parse_manifest(checksummed_manifest_without_validation(test))
                    continue
                for operation in ("render", "parse"):
                    with self.subTest(
                        kind=kind,
                        disposition=disposition,
                        operation=operation,
                    ):
                        with self.assertRaisesRegex(ValueError, "kind/disposition"):
                            if operation == "render":
                                render_manifest(metadata(), (test,))
                            else:
                                parse_manifest(checksummed_manifest_without_validation(test))

    def test_group_and_api_atoms_match_guest_consumer(self) -> None:
        unsafe_atoms = (
            "bad\\atom",
            "bad//atom",
            ".",
            "..",
            "./atom",
            "../atom",
            "atom/.",
            "atom/..",
            "/atom",
            "atom/",
        )
        base = replace(
            suite_test(),
            binary="bin/case.test",
            sha256="a" * 64,
        )
        slash_api = replace(base, api="sys/mman_h")
        render_manifest(metadata(), (slash_api,))
        parse_manifest(checksummed_manifest_without_validation(slash_api))

        for field in ("group", "api"):
            for atom in unsafe_atoms:
                test = replace(base, **{field: atom})
                for operation in ("render", "parse"):
                    with self.subTest(field=field, atom=atom, operation=operation):
                        with self.assertRaisesRegex(ValueError, f"unsafe {field}"):
                            if operation == "render":
                                render_manifest(metadata(), (test,))
                            else:
                                parse_manifest(checksummed_manifest_without_validation(test))

    def test_runnable_paths_stay_in_guest_bin_subtree(self) -> None:
        base = replace(
            suite_test(),
            binary="bin/case.test",
            sha256="a" * 64,
        )
        render_manifest(metadata(), (base,))
        parse_manifest(checksummed_manifest_without_validation(base))

        for path in ("lib/not-bin.test", "bin-other/not-bin.test", "bin"):
            test = replace(base, binary=path)
            for operation in ("render", "parse"):
                with self.subTest(path=path, operation=operation):
                    with self.assertRaisesRegex(ValueError, "bin/ subtree"):
                        if operation == "render":
                            render_manifest(metadata(), (test,))
                        else:
                            parse_manifest(checksummed_manifest_without_validation(test))

        noncomplete = replace(
            base,
            disposition="compile-failed",
            binary="-",
            sha256=EMPTY_SHA256,
        )
        render_manifest(metadata(), (noncomplete,))
        parse_manifest(checksummed_manifest_without_validation(noncomplete))

    def test_manifest_field_limits_match_guest_consumer(self) -> None:
        exact = replace(
            suite_test("i" * 256),
            group="g" * 96,
            api="a" * 96,
            binary="bin/" + "p" * (512 - len("bin/")),
            sha256="a" * 64,
        )
        render_manifest(replace(metadata(), source="s" * 1024), (exact,))

        invalid = (
            ("metadata", replace(metadata(), source="s" * 1025), exact),
            ("test ID", metadata(), replace(exact, test_id="i" * 257)),
            ("group", metadata(), replace(exact, group="g" * 97)),
            ("API", metadata(), replace(exact, api="a" * 97)),
            (
                "staged path",
                metadata(),
                replace(exact, binary="bin/" + "p" * (513 - len("bin/"))),
            ),
        )
        for label, manifest_metadata, test in invalid:
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, "limit"):
                    render_manifest(manifest_metadata, (test,))

        self.assertEqual(build_module.MAX_MANIFEST_METADATA_VALUE_BYTES, 1024)
        self.assertEqual(build_module.MAX_MANIFEST_TEST_ID_BYTES, 256)
        self.assertEqual(build_module.MAX_MANIFEST_GROUP_BYTES, 96)
        self.assertEqual(build_module.MAX_MANIFEST_API_BYTES, 96)
        self.assertEqual(build_module.MAX_MANIFEST_STAGED_PATH_BYTES, 512)

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

        oversized = tuple(
            replace(
                suite_test("i" * 250 + f"{index:04}"),
                group="g" * 96,
                api="a" * 96,
                binary="bin/" + "p" * 500 + f"{index:04}.bin",
                sha256="a" * 64,
            )
            for index in range(MAX_TESTS)
        )
        with self.assertRaisesRegex(ValueError, "2 MiB"):
            render_manifest(metadata(), oversized)

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
    def test_stage_journal_hardlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            victim = root / "victim"
            original = bytes(build_module._STAGE_JOURNAL_BYTES)
            victim.write_bytes(original)
            victim.chmod(0o600)
            os.link(victim, root / build_module._STAGE_JOURNAL_NAME)
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with self.assertRaisesRegex(ValueError, "journal.*safe"):
                    build_module._load_stage_journal(descriptor)
            finally:
                os.close(descriptor)

            self.assertEqual(victim.read_bytes(), original)

    def test_stage_journal_update_hardlink_preserves_external_victim(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            victim = root / "victim"
            original = b"external victim\n"
            victim.write_bytes(original)
            victim.chmod(0o600)
            os.link(victim, root / build_module._STAGE_JOURNAL_NAME)
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with self.assertRaisesRegex(ValueError, "journal.*safe"):
                    build_module._record_stage_transaction(descriptor, "idle")
            finally:
                os.close(descriptor)

            self.assertEqual(victim.read_bytes(), original)

    def test_stage_journal_fifo_is_rejected_without_blocking(self) -> None:
        script = """
import os
import tempfile
from pathlib import Path
from scripts.posix import build

with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary)
    os.mkfifo(root / build._STAGE_JOURNAL_NAME, 0o600)
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    try:
        try:
            build._load_stage_journal(descriptor)
        except ValueError:
            pass
        else:
            raise AssertionError("journal FIFO was accepted")
    finally:
        os.close(descriptor)
"""
        subprocess.run(
            [sys.executable, "-c", script],
            check=True,
            timeout=2.0,
            env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
        )

    def test_stage_journal_update_fifo_is_rejected_without_blocking(self) -> None:
        script = """
import os
import tempfile
from pathlib import Path
from scripts.posix import build

with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary)
    os.mkfifo(root / build._STAGE_JOURNAL_NAME, 0o600)
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    try:
        try:
            build._record_stage_transaction(descriptor, "idle")
        except ValueError:
            pass
        else:
            raise AssertionError("journal update FIFO was accepted")
    finally:
        os.close(descriptor)
"""
        subprocess.run(
            [sys.executable, "-c", script],
            check=True,
            timeout=2.0,
            env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
        )

    def test_stage_journal_initialization_recovers_prebuild_interruptions(
        self,
    ) -> None:
        partial = build_module._encode_stage_journal_record(
            0,
            {"schema": 1, "state": "idle"},
        )[:100]
        interrupted = {
            "after-create": b"",
            "after-truncate": bytes(build_module._STAGE_JOURNAL_BYTES),
            "after-partial-write": partial
            + bytes(build_module._STAGE_JOURNAL_BYTES - len(partial)),
        }
        for point, data in interrupted.items():
            with self.subTest(point=point), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                journal = root / build_module._STAGE_JOURNAL_NAME
                journal.write_bytes(data)
                journal.chmod(0o600)
                descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
                journal_descriptor = build_module._open_stage_journal(descriptor)
                try:
                    self.assertEqual(
                        build_module._load_stage_journal(
                            descriptor,
                            journal_descriptor=journal_descriptor,
                        ),
                        {"schema": 1, "state": "idle"},
                    )
                finally:
                    os.close(journal_descriptor)
                    os.close(descriptor)

    def test_uninitialized_journal_with_nonempty_work_root_fails_closed(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            journal = root / build_module._STAGE_JOURNAL_NAME
            journal.write_bytes(b"")
            journal.chmod(0o600)
            work = root / build_module._STAGE_WORK_ROOT_NAME
            work.mkdir(mode=0o700)
            artifact = work / "artifact"
            artifact.write_bytes(b"preserve")
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with self.assertRaisesRegex(ValueError, "nonempty work root"):
                    build_module._open_stage_journal(descriptor)
            finally:
                os.close(descriptor)

            self.assertEqual(artifact.read_bytes(), b"preserve")

    def test_torn_journal_update_retains_previous_generation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            journal_descriptor = build_module._open_stage_journal(descriptor)
            os.mkdir(
                build_module._STAGE_WORK_ROOT_NAME,
                0o700,
                dir_fd=descriptor,
            )
            work_descriptor = build_module._open_directory_at(
                descriptor,
                build_module._STAGE_WORK_ROOT_NAME,
                "stage work root",
            )
            build_module._record_stage_transaction(
                descriptor,
                "building",
                work_descriptor,
                journal_descriptor=journal_descriptor,
            )
            work_device, work_inode = build_module._descriptor_identity(
                work_descriptor
            )
            pwrite = os.pwrite

            def partial_write(
                target: int,
                data: bytes,
                offset: int,
            ) -> int:
                pwrite(target, data[:100], offset)
                raise OSError("simulated interrupted journal write")

            try:
                with mock.patch(
                    "scripts.posix.build.os.pwrite",
                    side_effect=partial_write,
                ), self.assertRaisesRegex(OSError, "interrupted"):
                    build_module._record_stage_transaction(
                        descriptor,
                        "initial",
                        work_descriptor,
                        journal_descriptor=journal_descriptor,
                    )
                self.assertEqual(
                    build_module._load_stage_journal(
                        descriptor,
                        journal_descriptor=journal_descriptor,
                    ),
                    {
                        "schema": 1,
                        "state": "building",
                        "work_dev": work_device,
                        "work_ino": work_inode,
                    },
                )
            finally:
                os.close(work_descriptor)
                os.close(journal_descriptor)
                os.close(descriptor)

    def test_torn_journal_commit_header_retains_previous_generation(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            journal_descriptor = build_module._open_stage_journal(descriptor)
            os.mkdir(
                build_module._STAGE_WORK_ROOT_NAME,
                0o700,
                dir_fd=descriptor,
            )
            work_descriptor = build_module._open_directory_at(
                descriptor,
                build_module._STAGE_WORK_ROOT_NAME,
                "stage work root",
            )
            build_module._record_stage_transaction(
                descriptor,
                "building",
                work_descriptor,
                journal_descriptor=journal_descriptor,
            )
            work_device, work_inode = build_module._descriptor_identity(
                work_descriptor
            )
            pwrite = os.pwrite
            write_count = 0

            def partial_header_write(
                target: int,
                data: bytes,
                offset: int,
            ) -> int:
                nonlocal write_count
                write_count += 1
                if write_count == 1:
                    return pwrite(target, data, offset)
                pwrite(target, data[:10], offset)
                raise OSError("simulated interrupted journal header")

            try:
                with mock.patch(
                    "scripts.posix.build.os.pwrite",
                    side_effect=partial_header_write,
                ), self.assertRaisesRegex(OSError, "header"):
                    build_module._record_stage_transaction(
                        descriptor,
                        "initial",
                        work_descriptor,
                        journal_descriptor=journal_descriptor,
                    )
                self.assertEqual(
                    build_module._load_stage_journal(
                        descriptor,
                        journal_descriptor=journal_descriptor,
                    ),
                    {
                        "schema": 1,
                        "state": "building",
                        "work_dev": work_device,
                        "work_ino": work_inode,
                    },
                )
            finally:
                os.close(work_descriptor)
                os.close(journal_descriptor)
                os.close(descriptor)

    def test_journal_update_does_not_overwrite_path_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            journal_descriptor = build_module._open_stage_journal(descriptor)
            journal_path = root / build_module._STAGE_JOURNAL_NAME
            moved = root / "journal-moved"
            replacement_source = root / "replacement-source"
            replacement = b"replacement journal path\n"
            replacement_source.write_bytes(replacement)
            replacement_source.chmod(0o600)
            stat_call = os.stat
            swapped = False

            def swap_after_validation(
                path: object,
                *args: object,
                **kwargs: object,
            ) -> os.stat_result:
                nonlocal swapped
                result = stat_call(path, *args, **kwargs)
                if path == build_module._STAGE_JOURNAL_NAME and not swapped:
                    journal_path.rename(moved)
                    replacement_source.rename(journal_path)
                    swapped = True
                return result

            try:
                with mock.patch(
                    "scripts.posix.build.os.stat",
                    side_effect=swap_after_validation,
                ), self.assertRaisesRegex(ValueError, "journal changed"):
                    build_module._record_stage_transaction(
                        descriptor,
                        "idle",
                        journal_descriptor=journal_descriptor,
                    )
            finally:
                os.close(journal_descriptor)
                os.close(descriptor)

            self.assertEqual(journal_path.read_bytes(), replacement)

    def test_cleanup_and_reuse_do_not_delete_replacement_for_held_work_root(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor: int | None = None
            work_descriptor: int | None = None
            reopened_slot_descriptor: int | None = None
            reopened_work_descriptor: int | None = None
            try:
                slot_descriptor, work_descriptor = (
                    build_module._open_stage_work_slot(
                        parent_descriptor,
                        "stage",
                    )
                )
                held_path = Path(
                    os.readlink(f"/proc/self/fd/{work_descriptor}")
                )
                (held_path / "artifact").write_bytes(b"artifact")
                moved = root / "held-work-root-moved"
                held_path.rename(moved)
                held_path.mkdir(mode=0o700)
                replacement = held_path / "replacement"
                replacement.write_bytes(b"replacement")

                build_module._clear_stage_work_root(work_descriptor)
                os.close(work_descriptor)
                work_descriptor = None
                os.close(slot_descriptor)
                slot_descriptor = None
                with self.assertRaisesRegex(ValueError, "work root.*empty"):
                    (
                        reopened_slot_descriptor,
                        reopened_work_descriptor,
                    ) = build_module._open_stage_work_slot(
                        parent_descriptor,
                        "stage",
                    )
            finally:
                if reopened_work_descriptor is not None:
                    os.close(reopened_work_descriptor)
                if reopened_slot_descriptor is not None:
                    os.close(reopened_slot_descriptor)
                if work_descriptor is not None:
                    os.close(work_descriptor)
                if slot_descriptor is not None:
                    os.close(slot_descriptor)
                os.close(parent_descriptor)

            self.assertEqual(replacement.read_bytes(), b"replacement")
            self.assertTrue(moved.is_dir())
            self.assertEqual(tuple(moved.iterdir()), ())

    def test_recovers_interrupted_partial_build_from_inode_journal(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                "stage",
            )
            build_module._record_stage_transaction(
                slot_descriptor,
                "building",
                work_descriptor,
            )
            work_path = Path(os.readlink(f"/proc/self/fd/{work_descriptor}"))
            (work_path / "partial-artifact").write_bytes(b"partial")
            os.close(work_descriptor)
            os.close(slot_descriptor)
            os.close(parent_descriptor)

            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                "stage",
            )
            try:
                self.assertEqual(tuple(Path(os.readlink(f"/proc/self/fd/{work_descriptor}")).iterdir()), ())
                assert_idle_reusable_stage_slot(self, root)
            finally:
                os.close(work_descriptor)
                os.close(slot_descriptor)
                os.close(parent_descriptor)

    def test_recovers_interruption_after_initial_publish(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                "stage",
            )
            build_module._record_stage_transaction(
                slot_descriptor,
                "building",
                work_descriptor,
            )
            work_path = Path(os.readlink(f"/proc/self/fd/{work_descriptor}"))
            (work_path / "published-artifact").write_bytes(b"published")
            replaced = build_module._publish_stage(
                slot_descriptor,
                build_module._STAGE_WORK_ROOT_NAME,
                work_descriptor,
                parent_descriptor,
                "stage",
            )
            self.assertIsNone(replaced)
            os.close(work_descriptor)
            os.close(slot_descriptor)
            os.close(parent_descriptor)

            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                "stage",
            )
            try:
                self.assertEqual(
                    (root / "stage/published-artifact").read_bytes(),
                    b"published",
                )
                self.assertEqual(tuple(Path(os.readlink(f"/proc/self/fd/{work_descriptor}")).iterdir()), ())
                assert_idle_reusable_stage_slot(self, root)
            finally:
                os.close(work_descriptor)
                os.close(slot_descriptor)
                os.close(parent_descriptor)

    def test_recovers_interruption_after_existing_stage_exchange(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / "stage"
            destination.mkdir()
            (destination / "old-artifact").write_bytes(b"old")
            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                destination.name,
            )
            build_module._record_stage_transaction(
                slot_descriptor,
                "building",
                work_descriptor,
            )
            work_path = Path(os.readlink(f"/proc/self/fd/{work_descriptor}"))
            (work_path / "new-artifact").write_bytes(b"new")
            replaced_descriptor = build_module._publish_stage(
                slot_descriptor,
                build_module._STAGE_WORK_ROOT_NAME,
                work_descriptor,
                parent_descriptor,
                destination.name,
            )
            self.assertIsNotNone(replaced_descriptor)
            assert replaced_descriptor is not None
            os.close(replaced_descriptor)
            os.close(work_descriptor)
            os.close(slot_descriptor)
            os.close(parent_descriptor)

            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                destination.name,
            )
            try:
                self.assertEqual(
                    (destination / "new-artifact").read_bytes(),
                    b"new",
                )
                self.assertFalse((destination / "old-artifact").exists())
                self.assertEqual(tuple(Path(os.readlink(f"/proc/self/fd/{work_descriptor}")).iterdir()), ())
                assert_idle_reusable_stage_slot(self, root)
            finally:
                os.close(work_descriptor)
                os.close(slot_descriptor)
                os.close(parent_descriptor)

    def test_recovery_preserves_replacement_on_journal_inode_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                "stage",
            )
            build_module._record_stage_transaction(
                slot_descriptor,
                "building",
                work_descriptor,
            )
            slot_path = Path(os.readlink(f"/proc/self/fd/{slot_descriptor}"))
            work_path = slot_path / build_module._STAGE_WORK_ROOT_NAME
            (work_path / "partial-artifact").write_bytes(b"partial")
            moved = root / "journal-work-root-moved"
            work_path.rename(moved)
            work_path.mkdir(mode=0o700)
            replacement = work_path / "replacement"
            replacement.write_bytes(b"replacement")
            os.close(work_descriptor)
            os.close(slot_descriptor)
            os.close(parent_descriptor)

            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with self.assertRaisesRegex(ValueError, "journal.*inode"):
                    slot_descriptor, work_descriptor = (
                        build_module._open_stage_work_slot(
                            parent_descriptor,
                            "stage",
                        )
                    )
            finally:
                os.close(parent_descriptor)

            self.assertEqual(replacement.read_bytes(), b"replacement")
            self.assertEqual(
                (moved / "partial-artifact").read_bytes(),
                b"partial",
            )

    def test_repeated_crash_recovery_reuses_journal_and_work_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)

            for index in range(3):
                parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
                slot_descriptor, work_descriptor = (
                    build_module._open_stage_work_slot(
                        parent_descriptor,
                        "stage",
                    )
                )
                build_module._record_stage_transaction(
                    slot_descriptor,
                    "building",
                    work_descriptor,
                )
                work_path = Path(
                    os.readlink(f"/proc/self/fd/{work_descriptor}")
                )
                (work_path / f"partial-{index}").write_bytes(b"partial")
                os.close(work_descriptor)
                os.close(slot_descriptor)
                os.close(parent_descriptor)

            parent_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            slot_descriptor, work_descriptor = build_module._open_stage_work_slot(
                parent_descriptor,
                "stage",
            )
            try:
                self.assertEqual(tuple(Path(os.readlink(f"/proc/self/fd/{work_descriptor}")).iterdir()), ())
                assert_idle_reusable_stage_slot(self, root)
            finally:
                os.close(work_descriptor)
                os.close(slot_descriptor)
                os.close(parent_descriptor)

    def test_verify_rejects_symlinked_stage_grandparent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            real_parent = root / "real-parent"
            stage = real_parent / "nested/stage"
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            alias = root / "alias"
            alias.symlink_to(real_parent, target_is_directory=True)

            with self.assertRaisesRegex(ValueError, "symlink"):
                verify_stage(
                    alias / "nested/stage",
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

    def test_verify_uses_held_stage_after_ancestor_swap(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent = root / "parent"
            stage = parent / "stage"
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            moved_parent = root / "parent-moved"
            outside = root / "outside"
            (outside / "stage").mkdir(parents=True)
            validate_tree = build_module._validate_stage_tree

            def swap_and_validate(opened_stage: Path) -> None:
                parent.rename(moved_parent)
                parent.symlink_to(outside, target_is_directory=True)
                validate_tree(opened_stage)

            with mock.patch(
                "scripts.posix.build._validate_stage_tree",
                side_effect=swap_and_validate,
            ):
                summary = verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

            self.assertEqual(summary.discovered, 1)

    def test_verify_uses_held_stage_after_leaf_swap(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            write_stage_fixture(stage, test)
            moved_stage = root / "stage-moved"
            outside = root / "outside"
            outside.mkdir()
            validate_tree = build_module._validate_stage_tree

            def swap_and_validate(opened_stage: Path) -> None:
                stage.rename(moved_stage)
                stage.symlink_to(outside, target_is_directory=True)
                validate_tree(opened_stage)

            with mock.patch(
                "scripts.posix.build._validate_stage_tree",
                side_effect=swap_and_validate,
            ):
                summary = verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

            self.assertEqual(summary.discovered, 1)

    def test_verify_preflights_oversized_metadata_before_parsing(self) -> None:
        limits = {
            "manifest.tsv": MAX_MANIFEST_BYTES,
            "manifest.json": 8 * 1024 * 1024,
            "build-results.ndjson": 64 * 1024 * 1024,
        }
        for name, limit in limits.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
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
                with (stage / name).open("r+b") as output:
                    output.truncate(limit + 1)

                with mock.patch(
                    "scripts.posix.build.parse_manifest",
                    wraps=build_module.parse_manifest,
                ) as parse:
                    with self.assertRaisesRegex(ValueError, f"{re.escape(name)}.*size"):
                        verify_stage(
                            stage,
                            verify_architecture=False,
                            expected_metadata=metadata(),
                        )
                    parse.assert_not_called()

    def test_verify_preflights_stage_size_before_parsing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            stage.mkdir(exist_ok=True)
            with (stage / "oversized").open("wb") as output:
                output.truncate(build_module.MAX_STAGE_BYTES + 1)

            with mock.patch(
                "scripts.posix.build.parse_manifest",
                wraps=build_module.parse_manifest,
            ) as parse:
                with self.assertRaisesRegex(ValueError, "stage.*256 MiB"):
                    verify_stage(stage, verify_architecture=False)
                parse.assert_not_called()

    def test_build_results_preflights_line_length_before_json(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "build-results.ndjson"
            path.write_bytes(b"x" * (256 * 1024 + 1) + b"\n")

            with mock.patch(
                "scripts.posix.build.json.loads",
                wraps=json.loads,
            ) as loads:
                with self.assertRaisesRegex(ValueError, "line.*length"):
                    build_module._load_build_results(path, ())
                loads.assert_not_called()

    def test_build_results_preflights_row_count_before_json(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "build-results.ndjson"
            path.write_bytes(b"{}\n" * (MAX_TESTS * 3 + 1))

            with mock.patch(
                "scripts.posix.build.json.loads",
                wraps=json.loads,
            ) as loads:
                with self.assertRaisesRegex(ValueError, "row count"):
                    build_module._load_build_results(path, ())
                loads.assert_not_called()

    def test_build_results_rejects_in_place_change_after_preflight(self) -> None:
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
            path = stage / "build-results.ndjson"
            build_result_lines = build_module._build_result_lines

            def mutate_after_preflight(source: object) -> object:
                path.write_bytes(b"x" * (256 * 1024 + 1) + b"\n")
                return build_result_lines(source)

            with mock.patch(
                "scripts.posix.build._build_result_lines",
                side_effect=mutate_after_preflight,
            ), mock.patch(
                "scripts.posix.build.json.loads",
                wraps=json.loads,
            ) as loads:
                with self.assertRaisesRegex(ValueError, "changed while being verified"):
                    verify_stage(
                        stage,
                        verify_architecture=False,
                        expected_metadata=metadata(),
                    )

            self.assertTrue(loads.call_args_list)
            self.assertTrue(
                all(
                    len(call.args[0].encode("utf-8"))
                    <= build_module.MAX_BUILD_RESULT_LINE_BYTES
                    for call in loads.call_args_list
                )
            )

    def test_verify_accepts_changed_build_duration(self) -> None:
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
            path = stage / "build-results.ndjson"
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            rows[0]["duration_ms"] = 999
            path.write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )

            summary = verify_stage(
                stage,
                verify_architecture=False,
                expected_metadata=metadata(),
            )

        self.assertEqual(summary.compile_pass, 1)

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
            (library_root / "libgcc_s.so.1").write_bytes(b"libgcc")
            executable = root / "case.test"
            executable.write_bytes(b"executable")
            stage = root / "stage"

            def query(_compiler: str, argument: str) -> str:
                return {
                    "-print-sysroot": str(sysroot),
                    "-print-multiarch": "aarch64-linux-gnu",
                    "-print-file-name=libc.so.6": str(library_root / "libc.so.6"),
                    "-print-file-name=libgcc_s.so.1": str(
                        library_root / "libgcc_s.so.1"
                    ),
                }[argument]

            def readelf(argv: list[str], **_kwargs: object) -> object:
                output = (
                    "(NEEDED) Shared library: [libsample.so.1]\n"
                    if Path(argv[-1]) == executable
                    else ""
                )
                return mock.Mock(returncode=0, stdout=output, stderr="")

            def stage_preload(stage_root: Path, **_kwargs: object) -> Path:
                preload = stage_root / "lib/libsmros-posix-compat.so"
                preload.parent.mkdir(parents=True, exist_ok=True)
                preload.write_bytes(b"compat")
                preload.chmod(0o755)
                return preload

            with mock.patch("scripts.posix.build.compiler_query", side_effect=query), mock.patch(
                "scripts.posix.build.run_bounded_command", side_effect=readelf
            ), mock.patch(
                "scripts.posix.build.stage_posix_compat_preload",
                side_effect=stage_preload,
            ):
                stage_runtime_dependencies((executable,), stage)

            self.assertEqual((stage / "lib/libsample.so.1").read_bytes(), b"library")
            self.assertFalse((stage / "lib/libsample.so.1.2").exists())

    def test_runtime_staging_includes_libgcc_for_pthread_exit_unwinding(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sysroot = root / "sysroot"
            library_root = sysroot / "usr/aarch64-linux-gnu/lib"
            library_root.mkdir(parents=True)
            (library_root / "libc.so.6").write_bytes(b"libc")
            (library_root / "libgcc_s.so.1").write_bytes(b"libgcc")
            executable = root / "pthread-exit.test"
            executable.write_bytes(b"executable")
            stage = root / "stage"

            def query(_compiler: str, argument: str) -> str:
                return {
                    "-print-sysroot": str(sysroot),
                    "-print-multiarch": "aarch64-linux-gnu",
                    "-print-file-name=libc.so.6": str(library_root / "libc.so.6"),
                    "-print-file-name=libgcc_s.so.1": str(
                        library_root / "libgcc_s.so.1"
                    ),
                }[argument]

            def readelf(argv: list[str], **_kwargs: object) -> object:
                output = (
                    "(NEEDED) Shared library: [libc.so.6]\n"
                    if Path(argv[-1]) == executable
                    else ""
                )
                return mock.Mock(returncode=0, stdout=output, stderr="")

            def stage_preload(stage_root: Path, **_kwargs: object) -> Path:
                preload = stage_root / "lib/libsmros-posix-compat.so"
                preload.parent.mkdir(parents=True, exist_ok=True)
                preload.write_bytes(b"compat")
                preload.chmod(0o755)
                return preload

            with mock.patch(
                "scripts.posix.build.compiler_query", side_effect=query
            ), mock.patch(
                "scripts.posix.build.run_bounded_command", side_effect=readelf
            ), mock.patch(
                "scripts.posix.build.stage_posix_compat_preload",
                side_effect=stage_preload,
            ):
                stage_runtime_dependencies((executable,), stage)

            self.assertEqual((stage / "lib/libgcc_s.so.1").read_bytes(), b"libgcc")

    def test_runtime_staging_includes_smros_posix_compat_preload(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sysroot = root / "sysroot"
            library_root = sysroot / "usr/aarch64-linux-gnu/lib"
            library_root.mkdir(parents=True)
            (library_root / "libc.so.6").write_bytes(b"libc")
            (library_root / "libgcc_s.so.1").write_bytes(b"libgcc")
            executable = root / "aio-write.test"
            executable.write_bytes(b"executable")
            stage = root / "stage"

            def query(_compiler: str, argument: str) -> str:
                return {
                    "-print-sysroot": str(sysroot),
                    "-print-multiarch": "aarch64-linux-gnu",
                    "-print-file-name=libc.so.6": str(library_root / "libc.so.6"),
                    "-print-file-name=libgcc_s.so.1": str(
                        library_root / "libgcc_s.so.1"
                    ),
                }[argument]

            def readelf(argv: list[str], **_kwargs: object) -> object:
                output = (
                    "(NEEDED) Shared library: [libc.so.6]\n"
                    if Path(argv[-1]) == executable
                    else ""
                )
                return mock.Mock(returncode=0, stdout=output, stderr="")

            def stage_preload(stage_root: Path, **_kwargs: object) -> Path:
                preload = stage_root / "lib/libsmros-posix-compat.so"
                preload.parent.mkdir(parents=True, exist_ok=True)
                preload.write_bytes(b"compat")
                preload.chmod(0o755)
                return preload

            with mock.patch(
                "scripts.posix.build.compiler_query", side_effect=query
            ), mock.patch(
                "scripts.posix.build.run_bounded_command", side_effect=readelf
            ), mock.patch(
                "scripts.posix.build.stage_posix_compat_preload",
                side_effect=stage_preload,
            ) as preload:
                staged = stage_runtime_dependencies((executable,), stage)

            preload.assert_called_once()
            self.assertIn(stage / "lib/libsmros-posix-compat.so", staged)
            self.assertEqual(
                (stage / "lib/libsmros-posix-compat.so").read_bytes(), b"compat"
            )

    def test_smros_posix_compat_preload_is_linked_as_shared_aarch64_library(self) -> None:
        command = build_module.posix_compat_preload_command(
            "aarch64-linux-gnu-gcc",
            Path("scripts/posix/runtime/smros_posix_compat.c"),
            Path("target/libsmros-posix-compat.so"),
        )

        self.assertEqual(command[0], "aarch64-linux-gnu-gcc")
        self.assertIn("-std=gnu99", command)
        self.assertIn("-fPIC", command)
        self.assertIn("-shared", command)
        self.assertIn("-Wl,-soname,libsmros-posix-compat.so", command)
        self.assertIn(
            "-Wl,--version-script,"
            + str(build_module.POSIX_COMPAT_PRELOAD_VERSION_SCRIPT),
            command,
        )
        self.assertIn("-ldl", command)
        self.assertEqual(
            command[command.index("-o") + 1],
            "target/libsmros-posix-compat.so",
        )

    def test_smros_posix_compat_compiles_without_include_path_override(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "libsmros-posix-compat.so"
            subprocess.run(
                [
                    "cc",
                    "-std=gnu99",
                    "-fPIC",
                    "-shared",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(source),
                    "-o",
                    str(output),
                    "-Wl,-soname,libsmros-posix-compat.so",
                    "-ldl",
                ],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertTrue(output.is_file())

    def test_smros_posix_compat_syncs_fake_uid_with_smros_kernel(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        self.assertIn("SYS_setreuid", source)
        self.assertIn("smros_sync_kernel_effective_uid", source)

    def test_smros_posix_compat_applies_regular_user_profile_from_environment(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        self.assertIn('getenv("SMROS_POSIX_TEST_USER")', source)
        self.assertIn(
            "smros_sync_kernel_effective_uid(SMROS_POSIX_TEST_UID)", source
        )
        self.assertIn("smros_effective_uid = SMROS_POSIX_TEST_UID", source)

    def test_smros_posix_compat_reports_shm_unlink_path_too_long_before_libc(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        self.assertIn("int shm_unlink(const char *name)", source)
        self.assertIn("strnlen(name, PATH_MAX)", source)
        self.assertIn("errno = ENAMETOOLONG", source)

    def test_smros_posix_compat_reports_shm_open_path_too_long_before_libc(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        self.assertIn("int shm_open(const char *name, int oflag, mode_t mode)", source)
        self.assertIn("smros_shm_open_fn", source)

    def test_smros_posix_compat_sched_yield_delegates_without_blocking(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        self.assertIn("int sched_yield(void)", source)
        self.assertIn("smros_sched_yield_target", source)
        self.assertIn('smros_resolve_symbol("sched_yield")', source)
        start = source.index("int sched_yield(void)")
        end = source.index("\n}", start) + 2
        self.assertNotIn("nanosleep", source[start:end])

    def test_smros_posix_compat_sched_yield_does_not_block_the_thread(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        start = source.index("int sched_yield(void)")
        end = source.index("\n}", start) + 2
        body = source[start:end]
        self.assertNotIn("nanosleep", body)
        self.assertNotIn("SMROS_SCHED_YIELD_HANDOFF_NSEC", body)

    def test_smros_posix_compat_spin_trylock_is_single_attempt(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "smros_pthread_spin_trylock(",
            source,
        )
        self.assertIn("int pthread_spin_trylock(pthread_spinlock_t *lock)", source)
        start = source.index(
            "smros_pthread_spin_trylock("
        )
        body = source[start:source.index("\n}", start) + 2]
        aarch64_body = body.split("#else", 1)[0]
        self.assertIn('"ldaxr %w0, [%1]"', aarch64_body)
        self.assertIn('"stxr %w0, %w2, [%1]"', aarch64_body)
        self.assertIn("return EBUSY;", aarch64_body)
        self.assertNotIn("pthread_spin_trylock_fn", aarch64_body)

    def test_smros_posix_compat_version_script_exports_compatibility_symbols(self) -> None:
        version_script = Path(
            "scripts/posix/runtime/smros_posix_compat.map"
        ).read_text(encoding="ascii")
        self.assertIn("GLIBC_2.17", version_script)
        self.assertIn("global:", version_script)
        self.assertIn("*;", version_script)
        self.assertNotIn("local:\n        *;", version_script)
        source = Path(
            "scripts/posix/runtime/smros_posix_compat.c"
        ).read_text(encoding="utf-8")
        for symbol in (
            "pthread_cancel",
            "pthread_create",
            "pthread_spin_trylock",
            "pthread_testcancel",
            "aio_cancel",
            "aio_error",
            "aio_fsync",
            "aio_read",
            "aio_return",
            "aio_suspend",
            "aio_write",
            "mq_unlink",
            "pthread_barrier_destroy",
            "pthread_barrier_init",
            "pthread_barrier_wait",
            "pthread_join",
            "pthread_kill",
            "pthread_mutex_getprioceiling",
            "pthread_mutex_trylock",
            "pthread_mutexattr_destroy",
            "pthread_mutexattr_gettype",
            "pthread_mutexattr_init",
            "pthread_mutexattr_setpshared",
            "pthread_mutexattr_settype",
            "pthread_rwlock_destroy",
            "pthread_rwlock_init",
            "pthread_rwlock_rdlock",
            "pthread_rwlock_unlock",
            "pthread_rwlock_wrlock",
            "pthread_setschedprio",
            "sem_destroy",
            "sem_init",
            "sem_open",
            "sem_timedwait",
            "sem_unlink",
            "sem_wait",
            "shm_open",
            "shm_unlink",
        ):
            self.assertIn(f"        {symbol};", version_script)

    def test_smros_posix_compat_condvar_lazy_static_and_signal_handoff(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "cond-probe.c"
            binary = root / "cond-probe"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

enum { WAITERS = 3 };
static pthread_cond_t cond = PTHREAD_COND_INITIALIZER;
static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int entered;
static volatile int woken;

static void *waiter(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&entered, 1);
    if (pthread_cond_wait(&cond, &mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&woken, 1);
    return pthread_mutex_unlock(&mutex) == 0 ? NULL : (void *)1;
}

int main(void) {
    pthread_t threads[WAITERS];
    for (int i = 0; i < WAITERS; i++) {
        if (pthread_create(&threads[i], NULL, waiter, NULL) != 0) return 2;
    }
    for (int i = 0; i < 10000 && entered < WAITERS; i++) usleep(1000);
    if (entered != WAITERS) return 3;
    if (pthread_cond_destroy(&cond) != EBUSY) return 4;
    if (pthread_cond_signal(&cond) != 0) return 5;
    for (int i = 0; i < 10000 && woken < 1; i++) usleep(1000);
    if (woken < 1) return 6;
    for (int i = 0; i < WAITERS - 1; i++) {
        if (pthread_cond_signal(&cond) != 0) return 7;
        for (int j = 0; j < 10000 && woken < i + 2; j++) usleep(1000);
    }
    if (woken != WAITERS) return 8;
    for (int i = 0; i < WAITERS; i++) {
        void *result = NULL;
        if (pthread_join(threads[i], &result) != 0 || result != NULL) return 9;
    }
    return pthread_cond_destroy(&cond);
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_condvar_stress_stays_within_reviewed_budget(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "cond-stress.c"
            binary = root / "cond-stress"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <unistd.h>

enum { SCENARIOS = 4, WAITERS = 100 };

static pthread_cond_t condition;
static pthread_mutex_t mutex;
static int ready;
static int predicate;

static void *waiter(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&ready, 1);
    while (!predicate) {
        if (pthread_cond_wait(&condition, &mutex) != 0) {
            pthread_mutex_unlock(&mutex);
            return (void *)1;
        }
    }
    return pthread_mutex_unlock(&mutex) == 0 ? NULL : (void *)1;
}

int main(void) {
    for (int scenario = 0; scenario < SCENARIOS; ++scenario) {
        ready = 0;
        predicate = 0;
        if (pthread_cond_init(&condition, NULL) != 0 ||
                pthread_mutex_init(&mutex, NULL) != 0) return 2;
        pthread_t threads[WAITERS];
        for (int index = 0; index < WAITERS; ++index) {
            if (pthread_create(&threads[index], NULL, waiter, NULL) != 0) return 3;
        }
        for (int spin = 0; spin < 5000 && ready < WAITERS; ++spin) usleep(1000);
        if (ready != WAITERS) return 4;
        if (pthread_mutex_lock(&mutex) != 0) return 5;
        predicate = 1;
        if (pthread_cond_broadcast(&condition) != 0) return 6;
        if (pthread_mutex_unlock(&mutex) != 0) return 7;
        for (int index = 0; index < WAITERS; ++index) {
            void *result = NULL;
            if (pthread_join(threads[index], &result) != 0 || result != NULL) return 8;
        }
        if (pthread_cond_destroy(&condition) != 0 ||
                pthread_mutex_destroy(&mutex) != 0) return 9;
    }
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=3.0,
            )

    def test_smros_posix_compat_condvar_signal_handoffs_one_token(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "cond-signal.c"
            binary = root / "cond-signal"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <unistd.h>

enum { WAITERS = 3 };
static pthread_cond_t condition;
static pthread_mutex_t mutex;
static volatile int started;
static volatile int woken;

static void *waiter(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&started, 1);
    if (pthread_cond_wait(&condition, &mutex) != 0) {
        pthread_mutex_unlock(&mutex);
        return (void *)1;
    }
    __sync_add_and_fetch(&woken, 1);
    return pthread_mutex_unlock(&mutex) == 0 ? NULL : (void *)1;
}

int main(void) {
    if (pthread_mutex_init(&mutex, NULL) != 0 ||
            pthread_cond_init(&condition, NULL) != 0) return 1;
    pthread_t threads[WAITERS];
    for (int index = 0; index < WAITERS; ++index) {
        if (pthread_create(&threads[index], NULL, waiter, NULL) != 0) return 2;
    }
    for (int spin = 0; spin < 5000 && started < WAITERS; ++spin) usleep(1000);
    if (started != WAITERS) return 3;
    if (pthread_mutex_lock(&mutex) != 0 || pthread_mutex_unlock(&mutex) != 0) return 4;
    int signals = 0;
    while (woken < WAITERS && signals <= WAITERS) {
        if (pthread_cond_signal(&condition) != 0) return 5;
        ++signals;
        usleep(1000);
    }
    for (int index = 0; index < WAITERS; ++index) {
        void *result = NULL;
        if (pthread_join(threads[index], &result) != 0 || result != NULL) return 6;
    }
    if (signals > WAITERS || woken != WAITERS) return 7;
    return pthread_cond_destroy(&condition) != 0 ||
        pthread_mutex_destroy(&mutex) != 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_condvar_signal_cascade_stays_within_budget(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "cond-cascade.c"
            binary = root / "cond-cascade"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <unistd.h>

enum { ROUNDS = 6, WAITERS = 20 };

static pthread_cond_t condition;
static pthread_mutex_t mutex;
static volatile int started;
static volatile int woken;

static void *waiter(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&started, 1);
    if (pthread_cond_wait(&condition, &mutex) != 0) {
        pthread_mutex_unlock(&mutex);
        return (void *)1;
    }
    __sync_add_and_fetch(&woken, 1);
    if (pthread_cond_signal(&condition) != 0) {
        pthread_mutex_unlock(&mutex);
        return (void *)1;
    }
    return pthread_mutex_unlock(&mutex) == 0 ? NULL : (void *)1;
}

int main(void) {
    for (int round = 0; round < ROUNDS; ++round) {
        started = 0;
        woken = 0;
        if (pthread_mutex_init(&mutex, NULL) != 0 ||
                pthread_cond_init(&condition, NULL) != 0) return 2;

        pthread_t threads[WAITERS];
        for (int index = 0; index < WAITERS; ++index) {
            if (pthread_create(&threads[index], NULL, waiter, NULL) != 0) return 3;
        }
        for (int spin = 0; spin < 5000 && started < WAITERS; ++spin) {
            usleep(1000);
        }
        if (started != WAITERS) return 4;

        if (pthread_mutex_lock(&mutex) != 0) return 5;
        if (pthread_cond_signal(&condition) != 0) return 6;
        if (pthread_mutex_unlock(&mutex) != 0) return 7;

        for (int index = 0; index < WAITERS; ++index) {
            void *result = NULL;
            if (pthread_join(threads[index], &result) != 0 || result != NULL) {
                return 8;
            }
        }
        if (woken != WAITERS) return 9;
        if (pthread_cond_destroy(&condition) != 0 ||
                pthread_mutex_destroy(&mutex) != 0) return 10;
    }
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            try:
                result = subprocess.run(
                    [str(binary)],
                    env={**os.environ, "LD_PRELOAD": str(preload)},
                    capture_output=True,
                    text=True,
                    timeout=8.0,
                )
            except subprocess.TimeoutExpired as error:
                self.fail(f"condition signal cascade exceeded 8-second budget: {error}")
            self.assertEqual(
                result.returncode,
                0,
                msg=f"condition signal cascade failed: {result.stderr}",
            )

    def test_smros_posix_compat_pshared_errorcheck_trylock_returns_busy(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "pshared-errorcheck-trylock.c"
            binary = root / "pshared-errorcheck-trylock"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sys/mman.h>

int main(void) {
    pthread_mutex_t *mutex = mmap(
        NULL, sizeof(*mutex), PROT_READ | PROT_WRITE,
        MAP_SHARED | MAP_ANONYMOUS, -1, 0
    );
    if (mutex == MAP_FAILED) return 2;

    pthread_mutexattr_t attr;
    if (pthread_mutexattr_init(&attr) != 0) return 3;
    if (pthread_mutexattr_setpshared(&attr, PTHREAD_PROCESS_SHARED) != 0) return 4;
    if (pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_ERRORCHECK) != 0) return 5;
    if (pthread_mutex_init(mutex, &attr) != 0) return 6;
    if (pthread_mutex_lock(mutex) != 0) return 7;
    if (pthread_mutex_trylock(mutex) != EBUSY) return 8;
    if (pthread_mutex_unlock(mutex) != 0) return 9;
    if (pthread_mutex_destroy(mutex) != 0) return 10;
    return munmap(mutex, sizeof(*mutex));
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_errorcheck_unlock_rejects_unowned_mutex(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "errorcheck-unlock.c"
            binary = root / "errorcheck-unlock"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>

int main(void) {
    pthread_mutexattr_t attr;
    pthread_mutex_t mutex;
    if (pthread_mutexattr_init(&attr) != 0) return 2;
    if (pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_ERRORCHECK) != 0) return 3;
    if (pthread_mutex_init(&mutex, &attr) != 0) return 4;
    if (pthread_mutex_lock(&mutex) != 0) return 5;
    if (pthread_mutex_unlock(&mutex) != 0) return 6;
    if (pthread_mutex_unlock(&mutex) != EPERM) return 7;
    if (pthread_mutex_destroy(&mutex) != 0) return 8;
    return pthread_mutexattr_destroy(&attr);
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_advertises_process_shared_threads(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "sysconf-process-shared.c"
            binary = root / "sysconf-process-shared"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <unistd.h>

int main(void) {
    long process_shared = sysconf(_SC_THREAD_PROCESS_SHARED);
    long mapped_files = sysconf(_SC_MAPPED_FILES);
    return process_shared < 0 || mapped_files < 0 ? 1 : 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary)],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_releases_fork_children_after_wait(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "fork-budget.c"
            binary = root / "fork-budget"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

enum { PROBE_CHILDREN = 64 };

int main(void) {
    pid_t children[PROBE_CHILDREN];
    int count = 0;
    while (count < PROBE_CHILDREN) {
        pid_t child = fork();
        if (child < 0) {
            if (errno != EAGAIN) return 10;
            break;
        }
        if (child == 0) _exit(0);
        children[count++] = child;
    }
        if (count != PROBE_CHILDREN) return 11;
    for (int index = 0; index < count; ++index) {
        if (waitpid(children[index], NULL, 0) != children[index]) return 12;
    }
    pid_t child = fork();
    if (child < 0) return 13;
    if (child == 0) _exit(0);
    return waitpid(child, NULL, 0) == child ? 0 : 14;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary)],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_child_fork_reset_does_not_take_inherited_lock(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c").read_text(
            encoding="utf-8"
        )
        start = source.index("static void smros_reset_fork_children(void)")
        end = source.index("\n}", start) + 2
        body = source[start:end]
        self.assertIn("memset(smros_fork_child_records", body)
        self.assertNotIn("__sync_lock_test_and_set", body)
        self.assertNotIn("__sync_lock_release", body)

    def test_smros_posix_compat_shared_cond_ignores_inherited_private_record(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "shared-cond-inherited-record.c"
            binary = root / "shared-cond-inherited-record"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

typedef struct {
    pthread_cond_t cond;
    pthread_mutex_t mutex;
    int ready;
    int predicate;
} probe_t;

static void child_wait(probe_t *probe) {
    int result = pthread_mutex_lock(&probe->mutex);
    if (result != 0) _exit(20 + result);
    probe->ready = 1;
    while (!probe->predicate) {
        result = pthread_cond_wait(&probe->cond, &probe->mutex);
        if (result != 0) {
            pthread_mutex_unlock(&probe->mutex);
            _exit(40 + result);
        }
    }
    result = pthread_mutex_unlock(&probe->mutex);
    _exit(result == 0 ? 0 : 60 + result);
}

int main(void) {
    probe_t *probe = mmap(NULL, sizeof(*probe), PROT_READ | PROT_WRITE,
                          MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (probe == MAP_FAILED) return 2;

    pthread_mutexattr_t mutex_attr;
    pthread_condattr_t cond_attr;
    if (pthread_mutexattr_init(&mutex_attr) != 0 ||
            pthread_condattr_init(&cond_attr) != 0) return 3;
    if (pthread_mutexattr_setpshared(&mutex_attr, PTHREAD_PROCESS_SHARED) != 0 ||
            pthread_mutex_init(&probe->mutex, &mutex_attr) != 0 ||
            pthread_cond_init(&probe->cond, NULL) != 0) return 4;
    pthread_mutexattr_destroy(&mutex_attr);
    pthread_condattr_destroy(&cond_attr);

    pid_t child = fork();
    if (child < 0) return 5;
    if (child == 0) child_wait(probe);

    /* Reuse the condition object as process-shared after the child inherited
     * the parent's private compatibility record. */
    if (pthread_cond_destroy(&probe->cond) != 0) return 6;
    if (pthread_condattr_init(&cond_attr) != 0 ||
            pthread_condattr_setpshared(&cond_attr, PTHREAD_PROCESS_SHARED) != 0 ||
            pthread_cond_init(&probe->cond, &cond_attr) != 0) return 7;
    pthread_condattr_destroy(&cond_attr);

    int result = pthread_mutex_lock(&probe->mutex);
    if (result != 0) return 8;
    for (int index = 0; index < 10000 && !probe->ready; index++) {
        pthread_mutex_unlock(&probe->mutex);
        sched_yield();
        result = pthread_mutex_lock(&probe->mutex);
        if (result != 0) return 9;
    }
    if (!probe->ready) return 10;
    probe->predicate = 1;
    result = pthread_cond_signal(&probe->cond);
    pthread_mutex_unlock(&probe->mutex);
    if (result != 0) return 11;

    int status = 0;
    int reaped = 0;
    for (int index = 0; index < 10000; index++) {
        pid_t waited = waitpid(child, &status, WNOHANG);
        if (waited == child) {
            reaped = 1;
            break;
        }
        if (waited < 0) break;
        usleep(1000);
    }
    if (!reaped) {
        kill(child, SIGKILL);
        waitpid(child, &status, 0);
        return 12;
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) return 13;
    if (pthread_cond_destroy(&probe->cond) != 0 ||
            pthread_mutex_destroy(&probe->mutex) != 0) return 14;
    return munmap(probe, sizeof(*probe)) == 0 ? 0 : 15;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_shared_cond_wait_honors_deferred_cancel(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "shared-cond-cancel.c"
            binary = root / "shared-cond-cancel"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sys/mman.h>
#include <unistd.h>

typedef struct {
    pthread_cond_t cond;
    pthread_mutex_t mutex;
    volatile int ready;
    volatile int cleanup_called;
} probe_t;

static void cleanup(void *arg) {
    probe_t *probe = (probe_t *)arg;
    probe->cleanup_called = 1;
    (void)pthread_mutex_unlock(&probe->mutex);
}

static void *waiter(void *arg) {
    probe_t *probe = (probe_t *)arg;
    if (pthread_mutex_lock(&probe->mutex) != 0) return (void *)1;
    probe->ready = 1;
    pthread_cleanup_push(cleanup, probe);
    (void)pthread_cond_wait(&probe->cond, &probe->mutex);
    pthread_cleanup_pop(0);
    (void)pthread_mutex_unlock(&probe->mutex);
    return NULL;
}

int main(void) {
    probe_t *probe = mmap(
        NULL, sizeof(*probe), PROT_READ | PROT_WRITE,
        MAP_SHARED | MAP_ANONYMOUS, -1, 0
    );
    if (probe == MAP_FAILED) return 2;
    pthread_mutexattr_t mutex_attr;
    pthread_condattr_t cond_attr;
    if (pthread_mutexattr_init(&mutex_attr) != 0 ||
            pthread_condattr_init(&cond_attr) != 0) return 3;
    if (pthread_mutexattr_setpshared(&mutex_attr, PTHREAD_PROCESS_SHARED) != 0 ||
            pthread_condattr_setpshared(&cond_attr, PTHREAD_PROCESS_SHARED) != 0) return 4;
    if (pthread_mutex_init(&probe->mutex, &mutex_attr) != 0 ||
            pthread_cond_init(&probe->cond, &cond_attr) != 0) return 5;
    pthread_mutexattr_destroy(&mutex_attr);
    pthread_condattr_destroy(&cond_attr);

    pthread_t thread;
    if (pthread_create(&thread, NULL, waiter, probe) != 0) return 6;
    for (int i = 0; i < 10000 && !probe->ready; i++) usleep(1000);
    if (!probe->ready) return 7;
    if (pthread_cancel(thread) != 0) return 8;
    void *result = NULL;
    if (pthread_join(thread, &result) != 0 || result != PTHREAD_CANCELED) return 9;
    if (!probe->cleanup_called) return 10;
    if (pthread_cond_destroy(&probe->cond) != 0 ||
            pthread_mutex_destroy(&probe->mutex) != 0) return 11;
    return munmap(probe, sizeof(*probe)) == 0 ? 0 : 12;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_condvar_cancel_request_does_not_leak_to_reused_thread(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "cond-cancel-reuse.c"
            binary = root / "cond-cancel-reuse"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <unistd.h>

static pthread_cond_t cond = PTHREAD_COND_INITIALIZER;
static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int ready;

static void unlock_mutex(void *arg) {
    (void)arg;
    (void)pthread_mutex_unlock(&mutex);
}

static void *cancelled_waiter(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&ready, 1);
    int result = 0;
    pthread_cleanup_push(unlock_mutex, NULL);
    result = pthread_cond_wait(&cond, &mutex);
    pthread_cleanup_pop(0);
    (void)pthread_mutex_unlock(&mutex);
    return (void *)(long)result;
}

static void *reused_waiter(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    __sync_add_and_fetch(&ready, 1);
    int result = pthread_cond_wait(&cond, &mutex);
    pthread_mutex_unlock(&mutex);
    return (void *)(long)result;
}

int main(void) {
    pthread_t first;
    if (pthread_create(&first, NULL, cancelled_waiter, NULL) != 0) return 2;
    for (int i = 0; i < 10000 && ready < 1; i++) usleep(1000);
    if (ready != 1) return 3;
    if (pthread_cancel(first) != 0) return 4;
    if (pthread_cond_signal(&cond) != 0) return 5;
    void *result = NULL;
    if (pthread_join(first, &result) != 0 || result != PTHREAD_CANCELED) return 6;

    ready = 0;
    pthread_t second;
    if (pthread_create(&second, NULL, reused_waiter, NULL) != 0) return 7;
    for (int i = 0; i < 10000 && ready < 1; i++) usleep(1000);
    if (ready != 1) return 8;
    if (pthread_cond_signal(&cond) != 0) return 9;
    result = NULL;
    if (pthread_join(second, &result) != 0 || (long)result != 0) return 10;
    return pthread_cond_destroy(&cond);
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_async_cancel_reaches_native_wait(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "async-cancel.c"
            binary = root / "async-cancel"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <stdio.h>

static pthread_barrier_t barrier;
static volatile int entered;

static void *waiter(void *arg) {
    (void)arg;
    if (pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, NULL) != 0) {
        return (void *)1;
    }
    entered = 1;
    (void)pthread_barrier_wait(&barrier);
    return NULL;
}

int main(void) {
    pthread_t thread;
    if (pthread_barrier_init(&barrier, NULL, 2) != 0) return 2;
    if (pthread_create(&thread, NULL, waiter, NULL) != 0) return 3;
    for (int index = 0; index < 10000 && !entered; index++) {
        sched_yield();
    }
    if (!entered) return 4;
    if (pthread_cancel(thread) != 0) return 5;
    void *result = NULL;
    if (pthread_join(thread, &result) != 0 || result != PTHREAD_CANCELED) {
        fprintf(stderr, "async cancel result=%p\n", result);
        return 6;
    }
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_deferred_cancel_reaches_native_join(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "deferred-join-cancel.c"
            binary = root / "deferred-join-cancel"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <unistd.h>

static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int child_started;

static void *child_func(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    child_started = 1;
    pthread_mutex_unlock(&mutex);
    return NULL;
}

static void *joiner_func(void *arg) {
    pthread_t child = *(pthread_t *)arg;
    (void)pthread_join(child, NULL);
    return (void *)1;
}

int main(void) {
    pthread_t child;
    pthread_t joiner;
    if (pthread_mutex_lock(&mutex) != 0) return 2;
    if (pthread_create(&child, NULL, child_func, NULL) != 0) return 3;
    for (int index = 0; index < 10000 && !child_started; index++) {
        sched_yield();
    }
    if (pthread_create(&joiner, NULL, joiner_func, &child) != 0) return 4;
    usleep(10000);
    if (pthread_cancel(joiner) != 0) return 5;
    void *result = NULL;
    if (pthread_join(joiner, &result) != 0 || result != PTHREAD_CANCELED) {
        return 6;
    }
    if (pthread_mutex_unlock(&mutex) != 0) return 7;
    if (pthread_join(child, NULL) != 0) return 8;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_deferred_cancel_interrupts_join_without_native_signal(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            cancel_stub = root / "cancel-stub.c"
            cancel_stub_so = root / "libcancel-stub.so"
            probe = root / "deferred-join-cancel-stub.c"
            binary = root / "deferred-join-cancel-stub"
            cancel_stub.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>

int pthread_cancel(pthread_t thread) {
    (void)thread;
    return 0;
}
''',
                encoding="utf-8",
            )
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <unistd.h>

static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int child_started;

static void *child_func(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex) != 0) return (void *)1;
    child_started = 1;
    pthread_mutex_unlock(&mutex);
    return NULL;
}

static void *joiner_func(void *arg) {
    pthread_t child = *(pthread_t *)arg;
    (void)pthread_join(child, NULL);
    return (void *)1;
}

int main(void) {
    pthread_t child;
    pthread_t joiner;
    if (pthread_mutex_lock(&mutex) != 0) return 2;
    if (pthread_create(&child, NULL, child_func, NULL) != 0) return 3;
    for (int index = 0; index < 10000 && !child_started; index++) {
        sched_yield();
    }
    if (pthread_create(&joiner, NULL, joiner_func, &child) != 0) return 4;
    usleep(10000);
    if (pthread_cancel(joiner) != 0) return 5;
    void *result = NULL;
    if (pthread_join(joiner, &result) != 0 || result != PTHREAD_CANCELED) {
        return 6;
    }
    if (pthread_mutex_unlock(&mutex) != 0) return 7;
    if (pthread_join(child, NULL) != 0) return 8;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(cancel_stub), "-o", str(cancel_stub_so),
                    "-Wl,-soname,libcancel-stub.so", "-pthread",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={
                    **os.environ,
                    "LD_PRELOAD": f"{preload}:{cancel_stub_so}",
                },
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_join_rejects_already_joined_thread(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "join-again.c"
            binary = root / "join-again"
            probe.write_text(
                r'''
#include <errno.h>
#include <pthread.h>

static void *worker(void *arg) {
    (void)arg;
    return NULL;
}

int main(void) {
    pthread_t thread;
    if (pthread_create(&thread, NULL, worker, NULL) != 0) return 2;
    if (pthread_join(thread, NULL) != 0) return 3;
    return pthread_join(thread, NULL) == ESRCH ? 0 : 4;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_sched_metadata_is_thread_local(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "sched-metadata.c"
            binary = root / "sched-metadata"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <stdio.h>

static void *worker(void *arg) {
    (void)arg;
    return NULL;
}

int main(void) {
    pthread_t thread;
    struct sched_param param = { .sched_priority = 7 };
    int policy = -1;
    struct sched_param observed = { .sched_priority = -1 };
    if (pthread_create(&thread, NULL, worker, NULL) != 0) return 1;
    if (pthread_setschedparam(thread, SCHED_FIFO, &param) != 0) return 2;
    if (pthread_getschedparam(thread, &policy, &observed) != 0) return 3;
    if (policy != SCHED_FIFO || observed.sched_priority != 7) return 4;
    if (pthread_join(thread, NULL) != 0) return 5;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 "-I", str(build_module.POSIX_COMPAT_INCLUDE_DIRECTORY),
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_mutex_getprioceiling_returns_fifo_default(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "mutex-prioceiling.c"
            binary = root / "mutex-prioceiling"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>

int main(void) {
    pthread_mutex_t mutex;
    int ceiling = -1;
    if (pthread_mutex_init(&mutex, NULL) != 0) return 1;
    if (pthread_mutex_getprioceiling(&mutex, &ceiling) != 0) return 2;
    if (ceiling < sched_get_priority_min(SCHED_FIFO) ||
            ceiling > sched_get_priority_max(SCHED_FIFO)) return 3;
    if (pthread_mutex_destroy(&mutex) != 0) return 4;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_tracks_mutexattr_lifecycle(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "mutexattr-lifecycle.c"
            binary = root / "mutexattr-lifecycle"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <string.h>

int main(void) {
    pthread_mutexattr_t attr;
    int type = -1;
    memset(&attr, 0, sizeof(attr));
    if (pthread_mutexattr_gettype(&attr, &type) != EINVAL) return 1;
    if (pthread_mutexattr_init(&attr) != 0) return 2;
    if (pthread_mutexattr_gettype(&attr, &type) != 0) return 3;
    if (pthread_mutexattr_destroy(&attr) != 0) return 4;
    if (pthread_mutexattr_gettype(&attr, &type) != EINVAL) return 5;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_pthread_kill_rejects_joined_thread(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "pthread-kill-joined.c"
            binary = root / "pthread-kill-joined"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>

static void *worker(void *arg) {
    return arg;
}

int main(void) {
    pthread_t thread;
    if (pthread_create(&thread, NULL, worker, NULL) != 0) return 1;
    if (pthread_join(thread, NULL) != 0) return 2;
    return pthread_kill(thread, 0) == ESRCH ? 0 : 3;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_rejects_uninitialized_pthread_attr(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "invalid-attr.c"
            binary = root / "invalid-attr"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <string.h>
#include <unistd.h>

static void handler(int signum) {
    if (signum == SIGSEGV) _exit(0);
}

static void *worker(void *arg) {
    (void)arg;
    return NULL;
}

int main(void) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = handler;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGSEGV, &action, NULL) != 0) return 1;

    pthread_attr_t attr;
    memset(&attr, 0xA5, sizeof(attr));
    pthread_t thread;
    (void)pthread_create(&thread, &attr, worker, NULL);
    return 2;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_inherit_sched_uses_parent_metadata(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "inherit-sched.c"
            binary = root / "inherit-sched"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>

static int observed_policy = -1;
static int observed_priority = -1;

static void *worker(void *arg) {
    (void)arg;
    struct sched_param param;
    if (pthread_getschedparam(pthread_self(), &observed_policy, &param) != 0) {
        return (void *)1;
    }
    observed_priority = param.sched_priority;
    return NULL;
}

int main(void) {
    pthread_attr_t attr;
    if (pthread_attr_init(&attr) != 0) return 1;
    if (pthread_attr_setschedpolicy(&attr, SCHED_FIFO) != 0) return 2;
    if (pthread_attr_setinheritsched(&attr, PTHREAD_INHERIT_SCHED) != 0) return 3;

    pthread_t thread;
    if (pthread_create(&thread, &attr, worker, NULL) != 0) return 4;
    if (pthread_join(thread, NULL) != 0) return 5;
    if (pthread_attr_destroy(&attr) != 0) return 6;
    if (observed_policy != SCHED_OTHER || observed_priority != 0) return 7;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 "-I", str(build_module.POSIX_COMPAT_INCLUDE_DIRECTORY),
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_barrier_reinit_reports_busy(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "barrier-reinit.c"
            binary = root / "barrier-reinit"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <unistd.h>

static pthread_barrier_t barrier;
static volatile int entered;

static void *waiter(void *arg) {
    (void)arg;
    entered = 1;
    int result = pthread_barrier_wait(&barrier);
    return (result == 0 || result == PTHREAD_BARRIER_SERIAL_THREAD) ? NULL : (void *)1;
}

int main(void) {
    if (pthread_barrier_init(&barrier, NULL, 2) != 0) return 1;
    pthread_t thread;
    if (pthread_create(&thread, NULL, waiter, NULL) != 0) return 2;
    for (int spin = 0; spin < 10000 && !entered; spin++) sched_yield();
    if (!entered) return 3;
    usleep(100000);
    if (pthread_barrier_init(&barrier, NULL, 2) != EBUSY) return 4;
    int result = pthread_barrier_wait(&barrier);
    if (result != 0 && result != PTHREAD_BARRIER_SERIAL_THREAD) return 5;
    void *thread_result = NULL;
    if (pthread_join(thread, &thread_result) != 0 || thread_result != NULL) return 6;
    return pthread_barrier_destroy(&barrier);
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_barrier_wait_ignores_spurious_native_serial(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            broken = root / "libbroken-barrier.so"
            broken_source = root / "broken-barrier.c"
            probe = root / "barrier-spurious-serial.c"
            binary = root / "barrier-spurious-serial"
            broken_source.write_text(
                r'''
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <pthread.h>

typedef int (*real_barrier_wait_fn)(pthread_barrier_t *);
static pthread_t main_thread;
static real_barrier_wait_fn real_barrier_wait;

__attribute__((constructor)) static void remember_main_thread(void) {
    main_thread = pthread_self();
    real_barrier_wait = (real_barrier_wait_fn)dlsym(
        RTLD_NEXT, "pthread_barrier_wait"
    );
}

int pthread_barrier_wait(pthread_barrier_t *barrier) {
    if (pthread_equal(pthread_self(), main_thread)) {
        return PTHREAD_BARRIER_SERIAL_THREAD;
    }
    return real_barrier_wait == NULL ? EINVAL : real_barrier_wait(barrier);
}
''',
                encoding="utf-8",
            )
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <unistd.h>

static pthread_barrier_t barrier;
static volatile int child_entered;

static void timeout_handler(int signum) {
    (void)signum;
    _exit(99);
}

static void *child(void *arg) {
    (void)arg;
    child_entered = 1;
    int result = pthread_barrier_wait(&barrier);
    return (result == 0 || result == PTHREAD_BARRIER_SERIAL_THREAD)
        ? NULL : (void *)1;
}

int main(void) {
    if (pthread_barrier_init(&barrier, NULL, 2) != 0) return 1;
    pthread_t thread;
    if (pthread_create(&thread, NULL, child, NULL) != 0) return 2;
    for (int spin = 0; spin < 10000 && !child_entered; spin++) sched_yield();
    if (!child_entered) return 3;
    signal(SIGALRM, timeout_handler);
    alarm(1);
    int result = pthread_barrier_wait(&barrier);
    alarm(0);
    if (result != 0 && result != PTHREAD_BARRIER_SERIAL_THREAD) return 4;
    void *thread_result = NULL;
    if (pthread_join(thread, &thread_result) != 0 || thread_result != NULL) return 5;
    return pthread_barrier_destroy(&barrier);
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(broken_source), "-o", str(broken), "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": f"{preload}:{broken}"},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_distinguishes_destroyed_pthread_attr(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "destroyed-attr.c"
            binary = root / "destroyed-attr"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <string.h>

static volatile sig_atomic_t signal_seen;

static void handler(int signum) {
    if (signum == SIGSEGV) signal_seen = 1;
}

static void *worker(void *arg) {
    (void)arg;
    return NULL;
}

int main(void) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = handler;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGSEGV, &action, NULL) != 0) return 1;

    pthread_attr_t destroyed;
    if (pthread_attr_init(&destroyed) != 0) return 2;
    if (pthread_attr_destroy(&destroyed) != 0) return 3;
    pthread_t thread;
    if (pthread_create(&thread, &destroyed, worker, NULL) != EINVAL) return 4;
    if (signal_seen) return 5;

    pthread_attr_t uninitialized;
    memset(&uninitialized, 0xA5, sizeof(uninitialized));
    (void)pthread_create(&thread, &uninitialized, worker, NULL);
    if (!signal_seen) return 6;
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_sleep_reports_signal_interruption(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "sleep-signal.c"
            binary = root / "sleep-signal"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <unistd.h>

static volatile sig_atomic_t signal_seen;

static void signal_handler(int signum) {
    (void)signum;
    signal_seen = 1;
}

static void *signaler(void *arg) {
    (void)arg;
    usleep(100000);
    if (kill(getpid(), SIGUSR1) != 0) {
        return (void *)1;
    }
    return NULL;
}

int main(void) {
    struct sigaction action = {0};
    action.sa_handler = signal_handler;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGUSR1, &action, NULL) != 0) return 2;

    pthread_t thread;
    if (pthread_create(&thread, NULL, signaler, NULL) != 0) return 3;
    unsigned int remaining = sleep(1);
    void *result = NULL;
    if (pthread_join(thread, &result) != 0 || result != NULL) return 4;
    if (!signal_seen) return 5;
    if (remaining == 0) {
        fprintf(stderr, "signal-interrupted sleep returned zero remaining\n");
        return 6;
    }
    return 0;
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_sem_wait_restarts_for_sa_restart(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            probe = root / "sem-wait-restart.c"
            binary = root / "sem-wait-restart"
            probe.write_text(
                r'''
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <semaphore.h>
#include <signal.h>
#include <stdio.h>
#include <unistd.h>

static sem_t semaphore;
static volatile sig_atomic_t signal_seen;
static volatile int waiting;

static void signal_handler(int signum) {
    (void)signum;
    signal_seen = 1;
}

static void *waiter(void *arg) {
    (void)arg;
    waiting = 1;
    errno = 0;
    int result = sem_wait(&semaphore);
    if (result != 0) {
        fprintf(stderr, "sem_wait result=%d errno=%d\n", result, errno);
        return (void *)1;
    }
    return NULL;
}

int main(void) {
    struct sigaction action = {0};
    action.sa_handler = signal_handler;
    action.sa_flags = SA_RESTART;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGUSR1, &action, NULL) != 0) return 2;
    if (sem_init(&semaphore, 0, 0) != 0) return 3;

    pthread_t thread;
    if (pthread_create(&thread, NULL, waiter, NULL) != 0) return 4;
    for (int index = 0; index < 10000 && !waiting; index++) sched_yield();
    if (!waiting) return 5;
    usleep(20000);
    if (pthread_kill(thread, SIGUSR1) != 0) return 6;
    for (int index = 0; index < 10000 && !signal_seen; index++) sched_yield();
    if (!signal_seen) return 7;
    if (sem_post(&semaphore) != 0) return 8;

    void *result = NULL;
    if (pthread_join(thread, &result) != 0 || result != NULL) return 9;
    return sem_destroy(&semaphore);
}
''',
                encoding="utf-8",
            )
            subprocess.run(
                [
                    "cc", "-std=gnu99", "-fPIC", "-shared", "-Wall", "-Wextra",
                    "-Werror", str(source), "-o", str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so", "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                ["cc", "-std=gnu99", "-Wall", "-Wextra", "-Werror",
                 str(probe), "-o", str(binary), "-pthread"],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={**os.environ, "LD_PRELOAD": str(preload)},
                check=True,
                timeout=5.0,
            )

    def test_smros_posix_compat_preload_tracks_completed_aio_requests(self) -> None:
        source = Path("scripts/posix/runtime/smros_posix_compat.c")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            preload = root / "libsmros-posix-compat.so"
            broken_sem = root / "libbroken-sem.so"
            broken_sem_source = root / "broken-sem.c"
            probe = root / "aio-probe.c"
            binary = root / "aio-probe"
            probe.write_text(
                r'''
#define _XOPEN_SOURCE 600

#include <aio.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mqueue.h>
#include <nl_types.h>
#include <pthread.h>
#include <pwd.h>
#include <semaphore.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern int pthread_atfork(void (*prepare)(void), void (*parent)(void), void (*child)(void));

static sem_t signal_sem;
static sem_t cross_thread_sem;
static sem_t thread_cap_sem;
static pthread_barrier_t busy_barrier;
static pthread_barrier_t release_barrier;
static pthread_cond_t destroy_after_broadcast_cond;
static pthread_mutex_t destroy_after_broadcast_mutex;
static int cross_thread_wait_ok;
static volatile int busy_barrier_waiting;
static int sleep_cancel_cleanup_called;
static int barrier_release_seen;
static int destroy_after_broadcast_waiters;
static int destroy_after_broadcast_released;

static void post_signal_sem(int signum) {
    (void)signum;
    sem_post(&signal_sem);
}

static void note_signal_only(int signum) {
    (void)signum;
}

static void atfork_noop(void) {
}

static void *blocked_sem_waiter(void *arg) {
    (void)arg;
    sigset_t blocked;
    sigemptyset(&blocked);
    sigaddset(&blocked, SIGUSR1);
    pthread_sigmask(SIG_BLOCK, &blocked, NULL);

    struct timespec deadline;
    clock_gettime(CLOCK_REALTIME, &deadline);
    deadline.tv_nsec += 50 * 1000 * 1000;
    if (deadline.tv_nsec >= 1000 * 1000 * 1000) {
        deadline.tv_sec++;
        deadline.tv_nsec -= 1000 * 1000 * 1000;
    }

    errno = 0;
    int result = sem_timedwait(&cross_thread_sem, &deadline);
    cross_thread_wait_ok = result == -1 && errno == ETIMEDOUT;
    return NULL;
}

static void *blocked_barrier_waiter(void *arg) {
    (void)arg;
    busy_barrier_waiting = 1;
    pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, NULL);
    int result = pthread_barrier_wait(&busy_barrier);
    if (result != 0 && result != PTHREAD_BARRIER_SERIAL_THREAD) {
        fprintf(stderr, "busy barrier wait result=%d\n", result);
    }
    return NULL;
}

static void *barrier_release_waiter(void *arg) {
    (void)arg;
    int result = pthread_barrier_wait(&release_barrier);
    if (result == 0 || result == PTHREAD_BARRIER_SERIAL_THREAD) {
        barrier_release_seen = 1;
    } else {
        fprintf(stderr, "barrier release waiter result=%d\n", result);
    }
    return NULL;
}

static void *destroy_after_broadcast_waiter(void *arg) {
    (void)arg;
    int result = pthread_mutex_lock(&destroy_after_broadcast_mutex);
    if (result != 0) {
        fprintf(stderr, "destroy-after-broadcast waiter lock result=%d\n",
                result);
        return (void *)1;
    }
    destroy_after_broadcast_waiters++;
    while (destroy_after_broadcast_released == 0) {
        result = pthread_cond_wait(
            &destroy_after_broadcast_cond,
            &destroy_after_broadcast_mutex
        );
        if (result != 0) {
            fprintf(stderr,
                    "destroy-after-broadcast cond wait result=%d\n",
                    result);
            pthread_mutex_unlock(&destroy_after_broadcast_mutex);
            return (void *)1;
        }
    }
    result = pthread_mutex_unlock(&destroy_after_broadcast_mutex);
    if (result != 0) {
        fprintf(stderr, "destroy-after-broadcast waiter unlock result=%d\n",
                result);
        return (void *)1;
    }
    return NULL;
}

static void sleep_cancel_cleanup(void *arg) {
    (void)arg;
    sleep_cancel_cleanup_called = 1;
}

static void *sleep_cancel_waiter(void *arg) {
    (void)arg;
    pthread_setcancelstate(PTHREAD_CANCEL_ENABLE, NULL);
    pthread_setcanceltype(PTHREAD_CANCEL_DEFERRED, NULL);
    pthread_cleanup_push(sleep_cancel_cleanup, NULL);
    sleep(1);
    pthread_cleanup_pop(0);
    return NULL;
}

static void *thread_cap_waiter(void *arg) {
    (void)arg;
    while (sem_wait(&thread_cap_sem) != 0 && errno == EINTR) {
    }
    return NULL;
}

static void *default_stack_waiter(void *arg) {
    (void)arg;
    return NULL;
}

typedef struct {
    pthread_cond_t cond;
    pthread_mutex_t mutex;
    int waiters;
    int predicate;
} shared_cond_probe_t;

static void shared_cond_child(shared_cond_probe_t *probe, int timed) {
    int result = pthread_mutex_lock(&probe->mutex);
    if (result != 0) {
        _exit(20 + result);
    }
    probe->waiters++;
    while (probe->predicate == 0) {
        if (timed) {
            struct timespec deadline;
            if (clock_gettime(CLOCK_REALTIME, &deadline) != 0) {
                pthread_mutex_unlock(&probe->mutex);
                _exit(30);
            }
            deadline.tv_sec += 5;
            result = pthread_cond_timedwait(
                &probe->cond,
                &probe->mutex,
                &deadline
            );
        } else {
            result = pthread_cond_wait(&probe->cond, &probe->mutex);
        }
        if (result != 0) {
            pthread_mutex_unlock(&probe->mutex);
            _exit(40 + result);
        }
    }
    result = pthread_mutex_unlock(&probe->mutex);
    _exit(result == 0 ? 0 : 60 + result);
}

static int run_cond_probe(int process_shared) {
    shared_cond_probe_t *probe =
        mmap(NULL, sizeof(*probe), PROT_READ | PROT_WRITE,
             MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (probe == MAP_FAILED) {
        perror("mmap shared cond probe");
        return 1;
    }
    memset(probe, 0, sizeof(*probe));

    pthread_mutexattr_t mutex_attr;
    pthread_condattr_t cond_attr;
    if (pthread_mutexattr_init(&mutex_attr) != 0 ||
            pthread_condattr_init(&cond_attr) != 0) {
        fprintf(stderr, "shared cond attr init failed\n");
        munmap(probe, sizeof(*probe));
        return 1;
    }
    if (process_shared) {
        if (pthread_mutexattr_setpshared(&mutex_attr, PTHREAD_PROCESS_SHARED) != 0 ||
                pthread_condattr_setpshared(&cond_attr, PTHREAD_PROCESS_SHARED) != 0) {
            fprintf(stderr, "shared cond attr pshared failed\n");
            munmap(probe, sizeof(*probe));
            return 1;
        }
    }
    if (pthread_mutex_init(&probe->mutex, &mutex_attr) != 0 ||
            pthread_cond_init(&probe->cond, &cond_attr) != 0) {
        fprintf(stderr, "shared cond init failed\n");
        munmap(probe, sizeof(*probe));
        return 1;
    }
    pthread_mutexattr_destroy(&mutex_attr);
    pthread_condattr_destroy(&cond_attr);

    pid_t children[2];
    for (int index = 0; index < 2; index++) {
        children[index] = fork();
        if (children[index] < 0) {
            perror("fork shared cond probe");
            munmap(probe, sizeof(*probe));
            return 1;
        }
        if (children[index] == 0) {
            shared_cond_child(probe, index);
        }
    }

    int result = pthread_mutex_lock(&probe->mutex);
    if (result != 0) {
        fprintf(stderr, "shared cond parent lock result=%d\n", result);
        munmap(probe, sizeof(*probe));
        return 1;
    }
    while (probe->waiters < 2) {
        pthread_mutex_unlock(&probe->mutex);
        sched_yield();
        result = pthread_mutex_lock(&probe->mutex);
        if (result != 0) {
            fprintf(stderr, "shared cond parent relock result=%d\n", result);
            munmap(probe, sizeof(*probe));
            return 1;
        }
    }
    probe->predicate = 1;
    result = pthread_cond_broadcast(&probe->cond);
    if (result != 0) {
        fprintf(stderr, "shared cond broadcast result=%d\n", result);
        pthread_mutex_unlock(&probe->mutex);
        for (int index = 0; index < 2; index++) {
            kill(children[index], SIGKILL);
            waitpid(children[index], NULL, 0);
        }
        munmap(probe, sizeof(*probe));
        return 1;
    }

    pthread_mutex_unlock(&probe->mutex);

    int failed = 0;
    for (int index = 0; index < 2; index++) {
        int status = 0;
        if (waitpid(children[index], &status, 0) != children[index] ||
                !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
            fprintf(stderr, "shared cond child[%d] status=%d\n", index, status);
            failed = 1;
        }
    }
    if (pthread_cond_destroy(&probe->cond) != 0 ||
            pthread_mutex_destroy(&probe->mutex) != 0) {
        fprintf(stderr, "shared cond destroy failed\n");
        failed = 1;
    }
    if (munmap(probe, sizeof(*probe)) != 0) {
        perror("munmap shared cond probe");
        failed = 1;
    }
    return failed;
}

static int run_destroy_after_broadcast_probe(void) {
    enum { waiter_count = 5 };
    pthread_t waiters[waiter_count];
    destroy_after_broadcast_waiters = 0;
    destroy_after_broadcast_released = 0;

    if (pthread_mutex_init(&destroy_after_broadcast_mutex, NULL) != 0 ||
            pthread_cond_init(&destroy_after_broadcast_cond, NULL) != 0) {
        fprintf(stderr, "destroy-after-broadcast init failed\n");
        return 1;
    }

    int result = pthread_mutex_lock(&destroy_after_broadcast_mutex);
    if (result != 0) {
        fprintf(stderr, "destroy-after-broadcast parent lock result=%d\n",
                result);
        return 1;
    }
    int created = 0;
    for (int index = 0; index < waiter_count; index++) {
        result = pthread_create(
            &waiters[index],
            NULL,
            destroy_after_broadcast_waiter,
            NULL
        );
        if (result != 0) {
            fprintf(stderr,
                    "destroy-after-broadcast create %d result=%d\n",
                    index,
                    result);
            pthread_mutex_unlock(&destroy_after_broadcast_mutex);
            for (int cleanup = 0; cleanup < created; cleanup++) {
                pthread_join(waiters[cleanup], NULL);
            }
            return 1;
        }
        created++;
    }
    while (destroy_after_broadcast_waiters < waiter_count) {
        pthread_mutex_unlock(&destroy_after_broadcast_mutex);
        sched_yield();
        result = pthread_mutex_lock(&destroy_after_broadcast_mutex);
        if (result != 0) {
            fprintf(stderr,
                    "destroy-after-broadcast parent relock result=%d\n",
                    result);
            return 1;
        }
    }

    destroy_after_broadcast_released = 1;
    result = pthread_cond_broadcast(&destroy_after_broadcast_cond);
    if (result != 0) {
        fprintf(stderr, "destroy-after-broadcast broadcast result=%d\n",
                result);
        pthread_mutex_unlock(&destroy_after_broadcast_mutex);
        for (int cleanup = 0; cleanup < created; cleanup++) {
            pthread_join(waiters[cleanup], NULL);
        }
        return 1;
    }

    int destroy_result =
        pthread_cond_destroy(&destroy_after_broadcast_cond);
    if (destroy_result == 0) {
        memset(&destroy_after_broadcast_cond, 0xA5,
               sizeof(destroy_after_broadcast_cond));
    }

    result = pthread_mutex_unlock(&destroy_after_broadcast_mutex);
    if (result != 0) {
        fprintf(stderr, "destroy-after-broadcast parent unlock result=%d\n",
                result);
        return 1;
    }

    int failed = 0;
    for (int index = 0; index < created; index++) {
        void *thread_result = NULL;
        if (pthread_join(waiters[index], &thread_result) != 0 ||
                thread_result != NULL) {
            fprintf(stderr,
                    "destroy-after-broadcast join %d result=%p\n",
                    index,
                    thread_result);
            failed = 1;
        }
    }
    if (destroy_result != 0) {
        fprintf(stderr,
                "destroy-after-broadcast destroy result=%d expected=0\n",
                destroy_result);
        if (pthread_cond_destroy(&destroy_after_broadcast_cond) != 0) {
            fprintf(stderr, "destroy-after-broadcast cleanup destroy failed\n");
        }
        failed = 1;
    }
    if (pthread_mutex_destroy(&destroy_after_broadcast_mutex) != 0) {
        fprintf(stderr, "destroy-after-broadcast mutex destroy failed\n");
        failed = 1;
    }
    return failed;
}

static int expect_errno(const char *name, int expected) {
    if (errno != expected) {
        fprintf(stderr, "%s errno=%d expected=%d\n", name, errno, expected);
        return 1;
    }
    return 0;
}

int main(void) {
    pthread_attr_t attr;
    struct sched_param pthread_sp;
    memset(&pthread_sp, 0, sizeof(pthread_sp));
    pthread_sp.sched_priority = 1;
    if (pthread_attr_init(&attr) != 0) {
        fprintf(stderr, "pthread_attr_init failed\n");
        return 1;
    }
    if (pthread_attr_setinheritsched(&attr, PTHREAD_EXPLICIT_SCHED) != 0) {
        fprintf(stderr, "pthread_attr_setinheritsched failed\n");
        return 1;
    }
    if (pthread_attr_setschedparam(&attr, &pthread_sp) != 0) {
        fprintf(stderr, "pthread_attr_setschedparam should be deferred\n");
        return 1;
    }
    if (pthread_attr_setschedpolicy(&attr, SCHED_RR) != 0) {
        fprintf(stderr, "pthread_attr_setschedpolicy failed\n");
        return 1;
    }
    memset(&pthread_sp, 0, sizeof(pthread_sp));
    if (pthread_attr_getschedparam(&attr, &pthread_sp) != 0 ||
            pthread_sp.sched_priority != 1) {
        fprintf(stderr, "deferred pthread attr priority was not replayed\n");
        return 1;
    }
    pthread_attr_destroy(&attr);

    if (pthread_attr_init(&attr) != 0) {
        fprintf(stderr, "pthread_attr_init invalid policy probe failed\n");
        return 1;
    }
    int policy_result = pthread_attr_setschedpolicy(&attr, -1);
    if (policy_result != ENOTSUP) {
        fprintf(stderr,
                "negative unsupported policy result=%d expected=%d\n",
                policy_result,
                ENOTSUP);
        return 1;
    }
    policy_result = pthread_attr_setschedpolicy(&attr, 999);
    if (policy_result != EINVAL) {
        fprintf(stderr,
                "positive invalid policy result=%d expected=%d\n",
                policy_result,
                EINVAL);
        return 1;
    }
    pthread_attr_destroy(&attr);

    if (pthread_barrier_init(&busy_barrier, NULL, 2) != 0) {
        fprintf(stderr, "busy barrier init failed\n");
        return 1;
    }
    pthread_t barrier_thread;
    if (pthread_create(&barrier_thread, NULL, blocked_barrier_waiter, NULL) != 0) {
        fprintf(stderr, "busy barrier thread create failed\n");
        return 1;
    }
    for (int index = 0; index < 100000 && !busy_barrier_waiting; index++) {
        sched_yield();
    }
    if (!busy_barrier_waiting) {
        fprintf(stderr, "busy barrier waiter did not start\n");
        pthread_cancel(barrier_thread);
        pthread_join(barrier_thread, NULL);
        return 1;
    }
    setenv("SMROS_FORCE_BUSY_BARRIER_DESTROY", "1", 1);
    int barrier_destroy_result = EINVAL;
    for (int index = 0; index < 100000; index++) {
        barrier_destroy_result = pthread_barrier_destroy(&busy_barrier);
        if (barrier_destroy_result == EBUSY) {
            break;
        }
        sched_yield();
    }
    unsetenv("SMROS_FORCE_BUSY_BARRIER_DESTROY");
    if (barrier_destroy_result != EBUSY) {
        fprintf(stderr,
                "busy barrier destroy result=%d expected=%d\n",
                barrier_destroy_result,
                EBUSY);
        pthread_cancel(barrier_thread);
        pthread_join(barrier_thread, NULL);
        return 1;
    }
    pthread_cancel(barrier_thread);
    pthread_join(barrier_thread, NULL);

    pthread_barrier_t uninitialized_barrier;
    memset(&uninitialized_barrier, 0, sizeof(uninitialized_barrier));
    setenv("SMROS_FORCE_UNTRACKED_BARRIER_WAIT", "1", 1);
    int uninitialized_wait_result =
        pthread_barrier_wait(&uninitialized_barrier);
    unsetenv("SMROS_FORCE_UNTRACKED_BARRIER_WAIT");
    if (uninitialized_wait_result != EINVAL) {
        fprintf(stderr,
                "uninitialized barrier wait result=%d expected=%d\n",
                uninitialized_wait_result,
                EINVAL);
        return 1;
    }

    pthread_barrier_t many_barriers[100];
    for (int barrier_index = 0; barrier_index < 100; barrier_index++) {
        if (pthread_barrier_init(&many_barriers[barrier_index], NULL, 1) != 0) {
            fprintf(stderr, "many barrier init %d failed\n", barrier_index);
            return 1;
        }
    }
    for (int barrier_index = 0; barrier_index < 100; barrier_index++) {
        int wait_result = pthread_barrier_wait(&many_barriers[barrier_index]);
        if (wait_result != 0 && wait_result != PTHREAD_BARRIER_SERIAL_THREAD) {
            fprintf(stderr,
                    "many barrier wait %d result=%d\n",
                    barrier_index,
                    wait_result);
            return 1;
        }
    }
    for (int barrier_index = 0; barrier_index < 100; barrier_index++) {
        if (pthread_barrier_destroy(&many_barriers[barrier_index]) != 0) {
            fprintf(stderr, "many barrier destroy %d failed\n", barrier_index);
            return 1;
        }
    }
    if (pthread_barrier_init(&release_barrier, NULL, 2) != 0) {
        fprintf(stderr, "release barrier init failed\n");
        return 1;
    }
    barrier_release_seen = 0;
    setenv("SMROS_FORCE_UNTRACKED_BARRIER_WAIT", "1", 1);
    pthread_t release_thread;
    if (pthread_create(&release_thread, NULL, barrier_release_waiter, NULL) != 0) {
        fprintf(stderr, "release barrier thread create failed\n");
        unsetenv("SMROS_FORCE_UNTRACKED_BARRIER_WAIT");
        return 1;
    }
    sleep(1);
    int release_wait_result = pthread_barrier_wait(&release_barrier);
    unsetenv("SMROS_FORCE_UNTRACKED_BARRIER_WAIT");
    if (
        release_wait_result != 0 &&
        release_wait_result != PTHREAD_BARRIER_SERIAL_THREAD
    ) {
        fprintf(stderr, "release barrier main wait result=%d\n",
                release_wait_result);
        pthread_join(release_thread, NULL);
        return 1;
    }
    if (pthread_join(release_thread, NULL) != 0 || barrier_release_seen != 1) {
        fprintf(stderr, "release barrier did not wake waiter seen=%d\n",
                barrier_release_seen);
        return 1;
    }
    if (pthread_barrier_destroy(&release_barrier) != 0) {
        fprintf(stderr, "release barrier destroy failed\n");
        return 1;
    }

    sleep_cancel_cleanup_called = 0;
    setenv("SMROS_FORCE_NONCANCEL_SLEEP", "1", 1);
    setenv("SMROS_FORCE_EXTERNAL_CANCEL_STUB", "1", 1);
    pthread_t sleep_thread;
    if (pthread_create(&sleep_thread, NULL, sleep_cancel_waiter, NULL) != 0) {
        fprintf(stderr, "sleep cancel thread create failed\n");
        return 1;
    }
    if (pthread_cancel(sleep_thread) != 0) {
        fprintf(stderr, "sleep cancel request failed\n");
        return 1;
    }
    void *sleep_thread_result = NULL;
    if (pthread_join(sleep_thread, &sleep_thread_result) != 0) {
        fprintf(stderr, "sleep cancel join failed\n");
        return 1;
    }
    unsetenv("SMROS_FORCE_NONCANCEL_SLEEP");
    unsetenv("SMROS_FORCE_EXTERNAL_CANCEL_STUB");
    if (
        sleep_thread_result != PTHREAD_CANCELED ||
        sleep_cancel_cleanup_called != 1
    ) {
        fprintf(stderr,
                "sleep cancel checkpoint failed result=%p cleanup=%d\n",
                sleep_thread_result,
                sleep_cancel_cleanup_called);
        return 1;
    }
    setenv("SMROS_REJECT_LONG_SLEEP", "1", 1);
    unsigned int long_sleep_remaining = sleep(3);
    unsetenv("SMROS_REJECT_LONG_SLEEP");
    if (long_sleep_remaining != 0) {
        fprintf(stderr, "long sleep was not chunked remaining=%u\n",
                long_sleep_remaining);
        return 1;
    }
    setenv("SMROS_FORCE_BLOCKING_SLEEP", "1", 1);
    alarm(2);
    unsigned int blocking_sleep_remaining = sleep(1);
    alarm(0);
    unsetenv("SMROS_FORCE_BLOCKING_SLEEP");
    if (blocking_sleep_remaining != 0) {
        fprintf(stderr, "blocking lower sleep leaked remaining=%u\n",
                blocking_sleep_remaining);
        return 1;
    }
    setenv("SMROS_REJECT_DEFAULT_THREAD_STACK", "1", 1);
    pthread_t default_stack_thread;
    int default_stack_create =
        pthread_create(&default_stack_thread, NULL, default_stack_waiter, NULL);
    unsetenv("SMROS_REJECT_DEFAULT_THREAD_STACK");
    if (default_stack_create != 0) {
        fprintf(stderr,
                "compat did not provide bounded default thread stack: %d\n",
                default_stack_create);
        return 1;
    }
    if (pthread_join(default_stack_thread, NULL) != 0) {
        fprintf(stderr, "default stack thread join failed\n");
        return 1;
    }

    enum { expected_thread_limit = 100 };
    long thread_limit = sysconf(_SC_THREAD_THREADS_MAX);
    if (thread_limit != expected_thread_limit) {
        fprintf(stderr,
                "unexpected THREAD_THREADS_MAX=%ld expected=%d\n",
                thread_limit,
                expected_thread_limit);
        return 1;
    }
    if (sem_init(&thread_cap_sem, 0, 0) != 0) {
        perror("sem_init thread cap");
        return 1;
    }
    pthread_t capped_threads[expected_thread_limit + 1];
    int capped_created = 0;
    for (int index = 0; index < expected_thread_limit; index++) {
        int create_result =
            pthread_create(&capped_threads[index], NULL, thread_cap_waiter, NULL);
        if (create_result != 0) {
            fprintf(stderr,
                    "pthread_create capped setup index=%d result=%d\n",
                    index,
                    create_result);
            for (int cleanup = 0; cleanup < capped_created; cleanup++) {
                sem_post(&thread_cap_sem);
            }
            for (int cleanup = 0; cleanup < capped_created; cleanup++) {
                pthread_join(capped_threads[cleanup], NULL);
            }
            sem_destroy(&thread_cap_sem);
            return 1;
        }
        capped_created++;
    }
    int capped_result =
        pthread_create(&capped_threads[expected_thread_limit], NULL,
                       thread_cap_waiter, NULL);
    if (capped_result == 0) {
        capped_created++;
    }
    for (int cleanup = 0; cleanup < capped_created; cleanup++) {
        sem_post(&thread_cap_sem);
    }
    for (int cleanup = 0; cleanup < capped_created; cleanup++) {
        pthread_join(capped_threads[cleanup], NULL);
    }
    if (sem_destroy(&thread_cap_sem) != 0) {
        perror("sem_destroy thread cap");
        return 1;
    }
    if (capped_result != EAGAIN) {
        fprintf(stderr,
                "pthread_create cap result=%d expected=%d\n",
                capped_result,
                EAGAIN);
        return 1;
    }

    setenv("SMROS_FORCE_REAL_MMAP", "1", 1);
    setenv("SMROS_FORCE_PSHARED_COND_STUB", "1", 1);
    int private_cond_result = 0;
    int shared_cond_result = run_cond_probe(1);
    unsetenv("SMROS_FORCE_PSHARED_COND_STUB");
    unsetenv("SMROS_FORCE_REAL_MMAP");
    if (private_cond_result != 0 || shared_cond_result != 0) {
        fprintf(stderr,
                "condition variable probe failed private=%d shared=%d\n",
                private_cond_result,
                shared_cond_result);
        return 1;
    }
    if (run_destroy_after_broadcast_probe() != 0) {
        fprintf(stderr, "destroy-after-broadcast cond probe failed\n");
        return 1;
    }

    struct aiocb invalid;
    memset(&invalid, 0, sizeof(invalid));
    errno = 0;
    if (aio_error(&invalid) != -1) {
        fprintf(stderr, "bad aio_error accepted\n");
        return 1;
    }
    if (expect_errno("bad aio_error", EINVAL) != 0) {
        return 1;
    }

    char template[] = "/tmp/smros-aio-probe-XXXXXX";
    int fd = mkstemp(template);
    if (fd < 0) {
        perror("mkstemp");
        return 1;
    }
    unlink(template);

    char buffer[] = "smros";
    struct aiocb request;
    memset(&request, 0, sizeof(request));
    request.aio_fildes = fd;
    request.aio_buf = buffer;
    request.aio_nbytes = sizeof(buffer);
    request.aio_offset = 0;

    if (aio_write(&request) != 0) {
        perror("aio_write");
        return 1;
    }
    close(fd);

    while (aio_error(&request) == EINPROGRESS) {
    }
    if (aio_error(&request) != 0) {
        perror("aio_error");
        return 1;
    }
    errno = 0;
    if (aio_return(&request) != (ssize_t)sizeof(buffer)) {
        perror("aio_return");
        return 1;
    }
    errno = 0;
    if (aio_return(&request) != -1) {
        fprintf(stderr, "duplicate aio_return accepted\n");
        return 1;
    }
    if (expect_errno("duplicate aio_return", EINVAL) != 0) {
        return 1;
    }

    char fsync_template[] = "/tmp/smros-aio-fsync-XXXXXX";
    int fsync_fd = mkstemp(fsync_template);
    if (fsync_fd < 0) {
        perror("mkstemp fsync");
        return 1;
    }
    unlink(fsync_template);

    struct aiocb fsync_request;
    memset(&fsync_request, 0, sizeof(fsync_request));
    fsync_request.aio_fildes = fsync_fd;
    errno = 0;
    if (aio_fsync(O_SYNC, &fsync_request) != 0) {
        perror("aio_fsync");
        return 1;
    }
    while (aio_error(&fsync_request) == EINPROGRESS) {
    }
    if (aio_error(&fsync_request) != 0) {
        perror("aio_fsync aio_error");
        return 1;
    }
    if (aio_return(&fsync_request) != 0) {
        perror("aio_fsync aio_return");
        return 1;
    }

    memset(&fsync_request, 0, sizeof(fsync_request));
    fsync_request.aio_fildes = fsync_fd;
    errno = 0;
    if (aio_fsync(-1, &fsync_request) != -1) {
        fprintf(stderr, "invalid aio_fsync op accepted\n");
        return 1;
    }
    if (expect_errno("invalid aio_fsync op", EINVAL) != 0) {
        return 1;
    }
    close(fsync_fd);

    memset(&fsync_request, 0, sizeof(fsync_request));
    fsync_request.aio_fildes = fsync_fd;
    errno = 0;
    if (aio_fsync(O_SYNC, &fsync_request) != -1) {
        fprintf(stderr, "closed-fd aio_fsync accepted\n");
        return 1;
    }
    if (expect_errno("closed-fd aio_fsync", EBADF) != 0) {
        return 1;
    }

    char append_template[] = "/tmp/smros-aio-append-XXXXXX";
    int append_fd = mkstemp(append_template);
    if (append_fd < 0) {
        perror("mkstemp append");
        return 1;
    }
    close(append_fd);
    append_fd = open(append_template, O_CREAT | O_APPEND | O_RDWR, 0600);
    if (append_fd < 0) {
        perror("open append");
        return 1;
    }
    unlink(append_template);

    char first[] = "aa";
    char second[] = "bbb";
    char third[] = "cccc";
    char *buffers[3] = { first, second, third };
    size_t sizes[3] = { sizeof(first) - 1, sizeof(second) - 1, sizeof(third) - 1 };
    struct aiocb writes[3];
    for (int index = 0; index < 3; index++) {
        memset(&writes[index], 0, sizeof(writes[index]));
        writes[index].aio_fildes = append_fd;
        writes[index].aio_buf = buffers[index];
        writes[index].aio_nbytes = sizes[index];
        if (aio_write(&writes[index]) != 0) {
            perror("aio_write append");
            return 1;
        }
    }

    while (aio_error(&writes[2]) == EINPROGRESS) {
    }
    for (int index = 0; index < 3; index++) {
        if (aio_error(&writes[index]) != 0) {
            fprintf(stderr, "append aio_error[%d]=%d\n", index, aio_error(&writes[index]));
            return 1;
        }
        if (aio_return(&writes[index]) != (ssize_t)sizes[index]) {
            fprintf(stderr, "append aio_return[%d]\n", index);
            return 1;
        }
    }
    char appended[9];
    if (lseek(append_fd, 0, SEEK_SET) != 0) {
        perror("append lseek");
        return 1;
    }
    if (read(append_fd, appended, sizeof(appended)) != (ssize_t)sizeof(appended)) {
        perror("append read");
        return 1;
    }
    if (memcmp(appended, "aabbbcccc", sizeof(appended)) != 0) {
        fprintf(stderr, "append order corrupted\n");
        return 1;
    }
    close(append_fd);

    int read_only = open("/dev/null", O_RDONLY);
    if (read_only < 0) {
        perror("open read-only");
        return 1;
    }
    memset(&request, 0, sizeof(request));
    request.aio_fildes = read_only;
    request.aio_buf = buffer;
    request.aio_nbytes = sizeof(buffer);
    errno = 0;
    if (aio_write(&request) != -1) {
        fprintf(stderr, "read-only aio_write accepted\n");
        return 1;
    }
    if (expect_errno("read-only aio_write", EBADF) != 0) {
        return 1;
    }
    close(read_only);

    int write_only = open("/dev/null", O_WRONLY);
    if (write_only < 0) {
        perror("open write-only");
        return 1;
    }
    memset(&request, 0, sizeof(request));
    request.aio_fildes = write_only;
    request.aio_buf = buffer;
    request.aio_nbytes = sizeof(buffer);
    errno = 0;
    if (aio_read(&request) != -1) {
        fprintf(stderr, "write-only aio_read accepted\n");
        return 1;
    }
    if (expect_errno("write-only aio_read", EBADF) != 0) {
        return 1;
    }
    close(write_only);

    nl_catd catalog = catopen("./mess.cat", 0);
    if (catalog == (nl_catd)-1) {
        perror("catopen fallback");
        return 1;
    }
    errno = 0;
    char *message = catgets(catalog, 1, 1, "not found");
    if (errno != 0 || strcmp(message, "This is the first message") != 0) {
        fprintf(stderr, "catgets fallback failed errno=%d message=%s\n", errno, message);
        return 1;
    }
    if (catclose(catalog) != 0) {
        perror("catclose fallback");
        return 1;
    }

    errno = 0;
    if (sem_unlink("") != -1 || errno != ENOENT) {
        fprintf(stderr, "sem_unlink empty errno=%d\n", errno);
        return 1;
    }
    errno = 0;
    if (mq_unlink("") != -1 || errno != ENOENT) {
        fprintf(stderr, "mq_unlink empty errno=%d\n", errno);
        return 1;
    }

    sem_t timed;
    if (sem_init(&timed, 0, 0) != 0) {
        perror("sem_init timed");
        return 1;
    }
    struct timespec deadline;
    if (clock_gettime(CLOCK_REALTIME, &deadline) != 0) {
        perror("clock_gettime deadline");
        return 1;
    }
    deadline.tv_nsec += 20 * 1000 * 1000;
    if (deadline.tv_nsec >= 1000 * 1000 * 1000) {
        deadline.tv_sec++;
        deadline.tv_nsec -= 1000 * 1000 * 1000;
    }
    errno = 0;
    if (sem_timedwait(&timed, &deadline) != -1 || errno != ETIMEDOUT) {
        fprintf(stderr, "sem_timedwait timeout errno=%d\n", errno);
        return 1;
    }
    deadline.tv_nsec = 1000 * 1000 * 1000;
    errno = 0;
    if (sem_timedwait(&timed, &deadline) != -1 || errno != EINVAL) {
        fprintf(stderr, "sem_timedwait invalid nsec errno=%d\n", errno);
        return 1;
    }
    if (sem_post(&timed) != 0) {
        perror("sem_post timed");
        return 1;
    }
    deadline.tv_nsec = 0;
    deadline.tv_sec += 1;
    if (sem_timedwait(&timed, &deadline) != 0) {
        perror("sem_timedwait posted");
        return 1;
    }
    if (sem_destroy(&timed) != 0) {
        perror("sem_destroy timed");
        return 1;
    }

    if (sem_init(&signal_sem, 0, 0) != 0) {
        perror("sem_init signal");
        return 1;
    }
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = post_signal_sem;
    if (sigemptyset(&action.sa_mask) != 0) {
        perror("sigemptyset signal");
        return 1;
    }
    if (sigaction(SIGALRM, &action, NULL) != 0) {
        perror("sigaction signal");
        return 1;
    }
    struct sigaction reported_action;
    memset(&reported_action, 0, sizeof(reported_action));
    if (sigaction(SIGALRM, NULL, &reported_action) != 0) {
        perror("sigaction query");
        return 1;
    }
    if (reported_action.sa_handler != post_signal_sem) {
        fprintf(stderr, "sigaction reported wrapped handler\n");
        return 1;
    }
    pid_t signal_child = fork();
    if (signal_child < 0) {
        perror("fork signal action query");
        return 1;
    }
    if (signal_child == 0) {
        struct sigaction child_action;
        memset(&child_action, 0, sizeof(child_action));
        if (sigaction(SIGALRM, NULL, &child_action) != 0) {
            _exit(42);
        }
        if (child_action.sa_handler != post_signal_sem) {
            _exit(43);
        }
        _exit(0);
    }
    int signal_status = 0;
    if (waitpid(signal_child, &signal_status, 0) != signal_child) {
        perror("waitpid signal action query");
        return 1;
    }
    if (!WIFEXITED(signal_status) || WEXITSTATUS(signal_status) != 0) {
        fprintf(stderr, "forked sigaction query status=%d\n", signal_status);
        return 1;
    }
    struct timespec start;
    struct timespec finish;
    if (clock_gettime(CLOCK_REALTIME, &start) != 0) {
        perror("clock_gettime start");
        return 1;
    }
    alarm(1);
    errno = 0;
    int sem_wait_result = sem_wait(&signal_sem);
    int sem_wait_errno = errno;
    alarm(0);
    if (clock_gettime(CLOCK_REALTIME, &finish) != 0) {
        perror("clock_gettime finish");
        return 1;
    }
    long elapsed_ns = (finish.tv_sec - start.tv_sec) * 1000000000L +
        (finish.tv_nsec - start.tv_nsec);
    if (sem_wait_result != 0 && sem_wait_errno != EINTR) {
        fprintf(stderr, "sem_wait signal errno=%d\n", sem_wait_errno);
        return 1;
    }
    if (elapsed_ns < 800000000L) {
        fprintf(stderr, "sem_wait did not block long enough: %ld\n", elapsed_ns);
        return 1;
    }
    if (sem_destroy(&signal_sem) != 0) {
        perror("sem_destroy signal");
        return 1;
    }

    if (sem_init(&cross_thread_sem, 0, 0) != 0) {
        perror("sem_init cross-thread signal");
        return 1;
    }
    memset(&action, 0, sizeof(action));
    action.sa_handler = note_signal_only;
    if (sigemptyset(&action.sa_mask) != 0) {
        perror("sigemptyset cross-thread signal");
        return 1;
    }
    if (sigaction(SIGUSR1, &action, NULL) != 0) {
        perror("sigaction cross-thread signal");
        return 1;
    }
    pthread_t waiter;
    cross_thread_wait_ok = 0;
    if (pthread_create(&waiter, NULL, blocked_sem_waiter, NULL) != 0) {
        perror("pthread_create cross-thread signal");
        return 1;
    }
    struct timespec cross_delay = {
        .tv_sec = 0,
        .tv_nsec = 5 * 1000 * 1000,
    };
    nanosleep(&cross_delay, NULL);
    if (raise(SIGUSR1) != 0) {
        perror("raise cross-thread signal");
        return 1;
    }
    if (pthread_join(waiter, NULL) != 0) {
        perror("pthread_join cross-thread signal");
        return 1;
    }
    if (!cross_thread_wait_ok) {
        fprintf(stderr, "cross-thread signal interrupted blocked sem_timedwait\n");
        return 1;
    }
    if (sem_destroy(&cross_thread_sem) != 0) {
        perror("sem_destroy cross-thread signal");
        return 1;
    }

    clock_t clock_start = clock();
    for (int index = 0; index < 1000; index++) {
        (void)clock();
    }
    clock_t clock_finish = clock();
    if (clock_finish - clock_start < CLOCKS_PER_SEC) {
        fprintf(stderr, "clock did not advance enough: start=%ld finish=%ld\n",
                (long)clock_start, (long)clock_finish);
        return 1;
    }

    struct timespec long_nanosleep = {
        .tv_sec = 10,
        .tv_nsec = 5000,
    };
    errno = 0;
    if (nanosleep(&long_nanosleep, NULL) != 0) {
        perror("capped nanosleep");
        return 1;
    }

    for (int index = 0; index < 10005; index++) {
        int atfork_result = pthread_atfork(atfork_noop, atfork_noop, atfork_noop);
        if (atfork_result != 0) {
            fprintf(stderr, "pthread_atfork stress result=%d at index=%d\n",
                    atfork_result, index);
            return 1;
        }
    }

    if (setuid(1) != 0 || getuid() != 1 || geteuid() != 1) {
        perror("fake setuid nonroot");
        return 1;
    }
    errno = 0;
    if (kill(1, 0) != -1 || errno != EPERM) {
        fprintf(stderr, "kill permission errno=%d\n", errno);
        return 1;
    }
    if (setuid(0) != 0 || getuid() != 0 || geteuid() != 0) {
        perror("fake setuid root");
        return 1;
    }

    long page_size = sysconf(_SC_PAGESIZE);
    if (page_size <= 0) {
        perror("sysconf pagesize");
        return 1;
    }
    void *invalid_lock = (void *)(LONG_MAX - (LONG_MAX % page_size));
    errno = 0;
    if (mlock(invalid_lock, 8) != -1 || errno != ENOMEM) {
        fprintf(stderr, "mlock invalid range errno=%d\n", errno);
        return 1;
    }
    errno = 0;
    if (munlock(invalid_lock, 8) != -1 || errno != ENOMEM) {
        fprintf(stderr, "munlock invalid range errno=%d\n", errno);
        return 1;
    }
    char lock_buffer[8];
    if (setuid(1) != 0 || geteuid() != 1) {
        perror("fake setuid nonroot for mlock");
        return 1;
    }
    errno = 0;
    if (mlock(lock_buffer, sizeof(lock_buffer)) != -1 || errno != EPERM) {
        fprintf(stderr, "mlock nonroot errno=%d\n", errno);
        return 1;
    }
    errno = 0;
    if (mlockall(MCL_CURRENT) != -1 || errno != EPERM) {
        fprintf(stderr, "mlockall nonroot errno=%d\n", errno);
        return 1;
    }
    if (setuid(0) != 0 || geteuid() != 0) {
        perror("fake setuid root for mlock");
        return 1;
    }
    errno = 0;
    if (mlockall(0) != -1 || errno != EINVAL) {
        fprintf(stderr, "mlockall invalid flags errno=%d\n", errno);
        return 1;
    }
    char map_template[] = "/tmp/smros-mlock-map-XXXXXX";
    int map_fd = mkstemp(map_template);
    if (map_fd < 0) {
        perror("mkstemp mlock map");
        return 1;
    }
    unlink(map_template);
    if (ftruncate(map_fd, page_size) != 0) {
        perror("ftruncate mlock map");
        return 1;
    }
    void *locked_map = mmap(NULL, page_size, PROT_READ | PROT_WRITE, MAP_SHARED, map_fd, 0);
    close(map_fd);
    if (locked_map == MAP_FAILED) {
        perror("mmap mlock map");
        return 1;
    }
    if (mlockall(MCL_CURRENT) != 0) {
        perror("mlockall current");
        return 1;
    }
    errno = 0;
    if (msync(locked_map, page_size, MS_SYNC | MS_INVALIDATE) != -1 || errno != EBUSY) {
        fprintf(stderr, "msync locked invalidate errno=%d\n", errno);
        return 1;
    }
    if (munlockall() != 0) {
        perror("munlockall");
        return 1;
    }
    if (munmap(locked_map, page_size) != 0) {
        perror("munmap mlock map");
        return 1;
    }
    void *plain_buffer = malloc((size_t)page_size * 2);
    if (plain_buffer == NULL) {
        perror("malloc munmap no-op");
        return 1;
    }
    void *plain_page = (void *)((char *)plain_buffer +
        (page_size - ((unsigned long)plain_buffer % (unsigned long)page_size)));
    if (munmap(plain_page, (size_t)page_size) != 0) {
        perror("munmap no-op range");
        return 1;
    }
    free(plain_buffer);

    int source_fd = open("conformance/interfaces/mlockall/3-7.c", O_RDONLY);
    if (source_fd < 0) {
        perror("open PCTS source fallback");
        return 1;
    }
    char source_bytes[8];
    ssize_t source_read = read(source_fd, source_bytes, sizeof(source_bytes));
    close(source_fd);
    if (source_read <= 0) {
        fprintf(stderr, "PCTS source fallback was empty\n");
        return 1;
    }

    char fast_mmap_path[128];
    snprintf(fast_mmap_path, sizeof(fast_mmap_path), "/tmp/pts_mmap_10_1_%d", getpid());
    unlink(fast_mmap_path);
    int fast_mmap_fd = open(fast_mmap_path, O_CREAT | O_RDWR | O_EXCL, 0600);
    if (fast_mmap_fd < 0) {
        perror("open fast mmap probe");
        return 1;
    }
    unlink(fast_mmap_path);
    if (ftruncate(fast_mmap_fd, 1024) != 0) {
        perror("ftruncate fast mmap probe");
        return 1;
    }
    void *fast_mmap = mmap(NULL, 1024, PROT_READ | PROT_WRITE, MAP_SHARED, fast_mmap_fd, 0);
    if (fast_mmap == MAP_FAILED || fast_mmap == NULL) {
        perror("mmap fast path");
        return 1;
    }
    if (munmap(fast_mmap, 1024) != 0) {
        perror("munmap fast path");
        return 1;
    }
    close(fast_mmap_fd);

    setpwent();
    struct passwd *root_user = getpwent();
    struct passwd *nonroot_user = getpwent();
    endpwent();
    if (root_user == NULL || root_user->pw_uid != 0 || nonroot_user == NULL || nonroot_user->pw_uid == 0) {
        fprintf(stderr, "fake passwd inventory missing non-root user\n");
        return 1;
    }
    if (seteuid(nonroot_user->pw_uid) != 0 || geteuid() != nonroot_user->pw_uid) {
        perror("fake seteuid nonroot");
        return 1;
    }

    const char *user_sem_name = "/smros_user_sem_probe";
    sem_unlink(user_sem_name);
    sem_t *user_sem = sem_open(user_sem_name, O_CREAT | O_EXCL, 0444, 1);
    if (user_sem == SEM_FAILED) {
        perror("sem_open user create");
        return 1;
    }
    errno = 0;
    if (sem_open(user_sem_name, O_CREAT, 0222, 1) != SEM_FAILED || errno != EACCES) {
        fprintf(stderr, "sem_open permission errno=%d\n", errno);
        return 1;
    }
    if (sem_unlink(user_sem_name) != 0) {
        perror("sem_unlink user-owned");
        return 1;
    }

    if (seteuid(0) != 0 || geteuid() != 0) {
        perror("fake seteuid root");
        return 1;
    }
    const char *root_sem_name = "/smros_root_sem_probe";
    sem_unlink(root_sem_name);
    sem_t *root_sem = sem_open(root_sem_name, O_CREAT | O_EXCL, 0744, 1);
    if (root_sem == SEM_FAILED) {
        perror("sem_open root create");
        return 1;
    }
    if (seteuid(nonroot_user->pw_uid) != 0) {
        perror("fake seteuid nonroot unlink");
        return 1;
    }
    errno = 0;
    if (sem_unlink(root_sem_name) != -1 || errno != EACCES) {
        fprintf(stderr, "sem_unlink permission errno=%d\n", errno);
        return 1;
    }
    if (seteuid(0) != 0) {
        perror("fake seteuid root cleanup");
        return 1;
    }
    if (sem_unlink(root_sem_name) != 0) {
        perror("sem_unlink root-owned");
        return 1;
    }

    long sem_max = sysconf(_SC_SEM_NSEMS_MAX);
    if (sem_max != 64) {
        fprintf(stderr, "unexpected SEM_NSEMS_MAX=%ld\n", sem_max);
        return 1;
    }
    sem_t *limited = calloc((size_t)sem_max, sizeof(sem_t));
    if (limited == NULL) {
        perror("calloc limited semaphores");
        return 1;
    }
    for (long index = 0; index < sem_max; index++) {
        if (sem_init(&limited[index], 0, 0) != 0) {
            fprintf(stderr, "sem_init limited[%ld] errno=%d\n", index, errno);
            return 1;
        }
    }
    sem_t extra_sem;
    errno = 0;
    if (sem_init(&extra_sem, 0, 0) != -1 || errno != ENOSPC) {
        fprintf(stderr, "sem_init limit errno=%d\n", errno);
        return 1;
    }
    for (long index = 0; index < sem_max; index++) {
        if (sem_destroy(&limited[index]) != 0) {
            fprintf(stderr, "sem_destroy limited[%ld] errno=%d\n", index, errno);
            return 1;
        }
    }
    free(limited);

    pid_t child = fork();
    if (child < 0) {
        perror("fork exec shim");
        return 1;
    }
    if (child == 0) {
        execl("/bin/ls", "ls", NULL);
        _exit(33);
    }
    int status = 0;
    if (waitpid(child, &status, 0) != child) {
        perror("waitpid exec shim");
        return 1;
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        fprintf(stderr, "exec shim child status=%d\n", status);
        return 1;
    }

    return 0;
}
''',
                encoding="utf-8",
            )
            broken_sem_source.write_text(
                r'''
#define _GNU_SOURCE

#include <aio.h>
#include <dlfcn.h>
#include <errno.h>
#include <mqueue.h>
#include <pthread.h>
#include <sched.h>
#include <semaphore.h>
#include <signal.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <time.h>

typedef int (*real_pthread_attr_setschedparam_fn)(
    pthread_attr_t *,
    const struct sched_param *
);
typedef int (*real_pthread_create_fn)(
    pthread_t *,
    const pthread_attr_t *,
    void *(*)(void *),
    void *
);
typedef int (*real_pthread_barrier_wait_fn)(pthread_barrier_t *);
typedef int (*real_pthread_barrier_destroy_fn)(pthread_barrier_t *);
typedef int (*real_pthread_cancel_fn)(pthread_t);
typedef int (*real_pthread_cond_broadcast_fn)(pthread_cond_t *);
typedef int (*real_pthread_cond_wait_fn)(pthread_cond_t *, pthread_mutex_t *);
typedef int (*real_pthread_cond_timedwait_fn)(
    pthread_cond_t *,
    pthread_mutex_t *,
    const struct timespec *
);
typedef void *(*real_mmap_fn)(void *, size_t, int, int, int, off_t);
typedef int (*real_munmap_fn)(void *, size_t);
typedef unsigned int (*real_sleep_fn)(unsigned int);

int sem_wait(sem_t *sem) {
    (void)sem;
    errno = ENOSYS;
    return -1;
}

int sem_timedwait(sem_t *sem, const struct timespec *abs_timeout) {
    (void)sem;
    (void)abs_timeout;
    errno = ENOSYS;
    return -1;
}

clock_t clock(void) {
    return 0;
}

int kill(pid_t pid, int sig) {
    if (pid == 1 && sig == 0) {
        return 0;
    }
    errno = ESRCH;
    return -1;
}

static char fallback_mapping[8192] __attribute__((aligned(4096)));
static int broken_atfork_count;

int pthread_attr_setschedparam(
    pthread_attr_t *attr,
    const struct sched_param *param
) {
    int policy = SCHED_OTHER;
    (void)pthread_attr_getschedpolicy(attr, &policy);
    if (policy == SCHED_OTHER && param->sched_priority > 0) {
        return EINVAL;
    }
    real_pthread_attr_setschedparam_fn real =
        (real_pthread_attr_setschedparam_fn)dlsym(
            RTLD_NEXT,
            "pthread_attr_setschedparam"
        );
    if (real == NULL) {
        return ENOSYS;
    }
    return real(attr, param);
}

int __register_atfork(
    void (*prepare)(void),
    void (*parent)(void),
    void (*child)(void),
    void *dso_handle
) {
    (void)prepare;
    (void)parent;
    (void)child;
    (void)dso_handle;
    if (broken_atfork_count++ >= 10000) {
        return EINTR;
    }
    return 0;
}

int mq_unlink(const char *name) {
    (void)name;
    errno = EINVAL;
    return -1;
}

int pthread_barrier_destroy(pthread_barrier_t *barrier) {
    if (getenv("SMROS_FORCE_BUSY_BARRIER_DESTROY") != NULL) {
        (void)barrier;
        return EINVAL;
    }
    real_pthread_barrier_destroy_fn real =
        (real_pthread_barrier_destroy_fn)dlsym(
            RTLD_NEXT,
            "pthread_barrier_destroy"
        );
    if (real == NULL) {
        return EINVAL;
    }
    return real(barrier);
}

int pthread_barrier_wait(pthread_barrier_t *barrier) {
    if (getenv("SMROS_FORCE_UNTRACKED_BARRIER_WAIT") != NULL) {
        (void)barrier;
        return 123;
    }
    real_pthread_barrier_wait_fn real =
        (real_pthread_barrier_wait_fn)dlsym(RTLD_NEXT, "pthread_barrier_wait");
    if (real == NULL) {
        return EINVAL;
    }
    return real(barrier);
}

int pthread_cancel(pthread_t thread) {
    real_pthread_cancel_fn real =
        (real_pthread_cancel_fn)dlsym(RTLD_NEXT, "pthread_cancel");
    if (real == NULL) {
        return ESRCH;
    }
    if (
        getenv("SMROS_FORCE_EXTERNAL_CANCEL_STUB") != NULL &&
        !pthread_equal(thread, pthread_self())
    ) {
        return 0;
    }
    return real(thread);
}

int pthread_create(
    pthread_t *thread,
    const pthread_attr_t *attr,
    void *(*start_routine)(void *),
    void *arg
) {
    real_pthread_create_fn real =
        (real_pthread_create_fn)dlsym(RTLD_NEXT, "pthread_create");
    if (real == NULL) {
        return EAGAIN;
    }
    if (getenv("SMROS_REJECT_DEFAULT_THREAD_STACK") != NULL) {
        if (attr == NULL) {
            return EAGAIN;
        }
        size_t stack_size = 0;
        if (
            pthread_attr_getstacksize(attr, &stack_size) != 0 ||
            stack_size > 1024 * 1024
        ) {
            return EAGAIN;
        }
    }
    return real(thread, attr, start_routine, arg);
}

int pthread_cond_broadcast(pthread_cond_t *cond) {
    if (getenv("SMROS_FORCE_PSHARED_COND_STUB") != NULL) {
        (void)cond;
        return EINVAL;
    }
    real_pthread_cond_broadcast_fn real =
        (real_pthread_cond_broadcast_fn)dlsym(
            RTLD_NEXT,
            "pthread_cond_broadcast"
        );
    if (real == NULL) {
        return EINVAL;
    }
    return real(cond);
}

int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex) {
    if (getenv("SMROS_FORCE_PSHARED_COND_STUB") != NULL) {
        (void)cond;
        (void)mutex;
        return EINVAL;
    }
    real_pthread_cond_wait_fn real =
        (real_pthread_cond_wait_fn)dlsym(RTLD_NEXT, "pthread_cond_wait");
    if (real == NULL) {
        return EINVAL;
    }
    return real(cond, mutex);
}

int pthread_cond_timedwait(
    pthread_cond_t *cond,
    pthread_mutex_t *mutex,
    const struct timespec *deadline
) {
    if (getenv("SMROS_FORCE_PSHARED_COND_STUB") != NULL) {
        (void)cond;
        (void)mutex;
        (void)deadline;
        return EINVAL;
    }
    real_pthread_cond_timedwait_fn real =
        (real_pthread_cond_timedwait_fn)dlsym(
            RTLD_NEXT,
            "pthread_cond_timedwait"
        );
    if (real == NULL) {
        return EINVAL;
    }
    return real(cond, mutex, deadline);
}

int mlock(const void *addr, size_t len) {
    (void)addr;
    (void)len;
    return 0;
}

int munlock(const void *addr, size_t len) {
    (void)addr;
    (void)len;
    return 0;
}

int mlockall(int flags) {
    (void)flags;
    return 0;
}

int munlockall(void) {
    return 0;
}

int msync(void *addr, size_t len, int flags) {
    (void)addr;
    (void)len;
    (void)flags;
    return 0;
}

int nanosleep(const struct timespec *req, struct timespec *rem) {
    (void)rem;
    if (req != NULL && (req->tv_sec == 10 || req->tv_sec == 13)) {
        errno = EINTR;
        return -1;
    }
    return 0;
}

int aio_fsync(int op, struct aiocb *request) {
    (void)op;
    (void)request;
    errno = EIO;
    return -1;
}

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t offset) {
    if (getenv("SMROS_FORCE_REAL_MMAP") != NULL) {
        real_mmap_fn real = (real_mmap_fn)dlsym(RTLD_NEXT, "mmap");
        if (real == NULL) {
            errno = ENOMEM;
            return MAP_FAILED;
        }
        return real(addr, len, prot, flags, fd, offset);
    }
    (void)addr;
    (void)prot;
    (void)fd;
    if (len == 1024 && (flags & MAP_SHARED) != 0 && offset == 0) {
        errno = EIO;
        return MAP_FAILED;
    }
    if (len <= sizeof(fallback_mapping)) {
        return fallback_mapping;
    }
    errno = ENOMEM;
    return MAP_FAILED;
}

int munmap(void *addr, size_t len) {
    if (getenv("SMROS_FORCE_REAL_MMAP") != NULL) {
        real_munmap_fn real = (real_munmap_fn)dlsym(RTLD_NEXT, "munmap");
        if (real == NULL) {
            errno = EINVAL;
            return -1;
        }
        return real(addr, len);
    }
    (void)len;
    if (addr == fallback_mapping) {
        return 0;
    }
    errno = EINVAL;
    return -1;
}

unsigned int sleep(unsigned int seconds) {
    if (getenv("SMROS_FORCE_BLOCKING_SLEEP") != NULL) {
        for (;;) {
            sched_yield();
        }
    }
    if (getenv("SMROS_REJECT_LONG_SLEEP") != NULL && seconds > 1) {
        return seconds;
    }
    if (getenv("SMROS_FORCE_NONCANCEL_SLEEP") != NULL) {
        volatile unsigned long spin = 0;
        while (spin < 100000000UL) {
            spin++;
        }
        return 0;
    }
    real_sleep_fn real = (real_sleep_fn)dlsym(RTLD_NEXT, "sleep");
    if (real == NULL) {
        return seconds;
    }
    return real(seconds);
}
''',
                encoding="utf-8",
            )
            catalog_source = root / "messcat_src.txt"
            catalog = root / "mess.cat"
            catalog_source.write_text(
                "$set 1 messages\n1 generated\n",
                encoding="utf-8",
                newline="\n",
            )
            subprocess.run(["gencat", str(catalog), str(catalog_source)], check=True)
            source_root = root / "pts-source"
            source_file = source_root / "conformance/interfaces/mlockall/3-7.c"
            source_file.parent.mkdir(parents=True)
            source_file.write_text("/* fallback source */\n", encoding="utf-8")

            subprocess.run(
                [
                    "cc",
                    "-std=gnu99",
                    "-fPIC",
                    "-shared",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(source),
                    "-o",
                    str(preload),
                    "-Wl,-soname,libsmros-posix-compat.so",
                    "-ldl",
                ],
                check=True,
            )
            subprocess.run(
                [
                    "cc",
                    "-std=gnu99",
                    "-fPIC",
                    "-shared",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(broken_sem_source),
                    "-o",
                    str(broken_sem),
                ],
                check=True,
            )
            subprocess.run(
                [
                    "cc",
                    "-std=gnu99",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(probe),
                    "-o",
                    str(binary),
                    "-pthread",
                ],
                check=True,
            )
            subprocess.run(
                [str(binary)],
                env={
                    **os.environ,
                    "LD_PRELOAD": f"{preload}:{broken_sem}",
                    "SMROS_PTS_FORK_MESSAGE_CATALOG": str(catalog),
                    "SMROS_PTS_SOURCE_ROOT": str(source_root),
                },
                check=True,
            )

    def test_runtime_staging_passes_held_stage_descriptor_to_readelf(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            executable = stage / "bin/test.test"
            executable.parent.mkdir()
            executable.write_bytes(b"elf")
            stage_descriptor = os.open(stage, os.O_RDONLY | os.O_DIRECTORY)
            opened_executable = Path(
                f"/proc/self/fd/{stage_descriptor}/bin/test.test"
            )
            readelf_result = BuildResult(
                test_id="runtime",
                stage="readelf",
                status="passed",
                argv=("aarch64-linux-gnu-readelf",),
                returncode=0,
                stdout="",
                stderr="",
                duration_ms=1,
                artifact_sha256=None,
            )
            try:
                with mock.patch(
                    "scripts.posix.build.compiler_query",
                    side_effect=(
                        "/",
                        "aarch64-linux-gnu",
                        "libc.so.6",
                        "libgcc_s.so.1",
                    ),
                ), mock.patch(
                    "scripts.posix.build._run_command",
                    return_value=readelf_result,
                ) as run:
                    with mock.patch(
                        "scripts.posix.build.resolve_runtime_file",
                        return_value=opened_executable,
                    ), mock.patch(
                        "scripts.posix.build.stage_posix_compat_preload",
                        return_value=opened_executable,
                    ):
                        stage_runtime_dependencies(
                            (opened_executable,),
                            Path(f"/proc/self/fd/{stage_descriptor}"),
                            stage_descriptor=stage_descriptor,
                        )
            finally:
                os.close(stage_descriptor)

        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.kwargs["pass_fds"], (stage_descriptor,))

    def test_verify_rejects_missing_or_changed_binary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            test = replace(suite_test(), binary="bin/conformance/interfaces/getpid/1-1.c.test", sha256=hashlib.sha256(b"elf").hexdigest())
            write_stage_fixture(stage, test)

            readelf = lambda _argv, **_kwargs: mock.Mock(returncode=0, stdout="Machine: AArch64", stderr="")
            verify_stage(
                stage, readelf_runner=readelf, expected_metadata=metadata()
            )
            binary.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "checksum"):
                verify_stage(
                    stage, readelf_runner=readelf, expected_metadata=metadata()
                )

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
                verify_stage(
                    stage, readelf_runner=readelf, expected_metadata=metadata()
                )

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
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

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
                verify_stage(
                    root / "stage",
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

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
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

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
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

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
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

    def test_verify_rejects_build_result_checksum_tampering(self) -> None:
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
            rows[0]["stdout"] = "tampered"
            (stage / "build-results.ndjson").write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "build results checksum"):
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

    def test_build_result_validation_rejects_host_nm(self) -> None:
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
            path = stage / "build-results.ndjson"
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            nm_row = next(row for row in rows if row["stage"] == "nm")
            nm_row["argv"][0] = "nm"
            path.write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "target nm"):
                build_module._load_build_results(path, (test,))

    def test_build_result_validation_rejects_missing_compile_flag(self) -> None:
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
            path = stage / "build-results.ndjson"
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            compile_row = next(row for row in rows if row["stage"] == "compile")
            compile_row["argv"].remove("-std=gnu99")
            path.write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "target compiler"):
                build_module._load_build_results(path, (test,))

    def test_strict_build_result_validation_rejects_wrong_source_root(self) -> None:
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
            path = stage / "build-results.ndjson"
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            compile_row = next(row for row in rows if row["stage"] == "compile")
            compile_argv = compile_row["argv"]
            compile_argv[compile_argv.index("-c") + 1] = f"/tmp/untrusted/{test.source}"
            path.write_text(
                "".join(
                    json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "production compiler path"):
                build_module._load_build_results(
                    path,
                    (test,),
                    strict_paths=True,
                    revision="8" * 40,
                )

    def test_verify_rejects_stale_expected_metadata(self) -> None:
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

            with self.assertRaisesRegex(ValueError, "metadata.*current"):
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=replace(metadata(), revision="9" * 40),
                )

    def test_verify_rejects_substituted_manifest_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            staged_test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
            )
            expected_tests = (
                suite_test(),
                suite_test("conformance/interfaces/getpid/2-1.c"),
            )
            write_stage_fixture(stage, staged_test)

            with self.assertRaisesRegex(ValueError, "expected inventory"):
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                    expected_tests=expected_tests,
                    expected_shell_tests=(),
                )

    def test_verify_rejects_changed_expected_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            binary = stage / "bin/conformance/interfaces/getpid/1-1.c.test"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"elf")
            staged_test = replace(
                suite_test(),
                binary="bin/conformance/interfaces/getpid/1-1.c.test",
                sha256=hashlib.sha256(b"elf").hexdigest(),
                timeout_ms=45_000,
            )
            write_stage_fixture(stage, staged_test)

            with self.assertRaisesRegex(ValueError, "expected inventory"):
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                    expected_tests=(suite_test(),),
                    expected_shell_tests=(),
                )

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
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

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
                verify_stage(
                    stage,
                    verify_architecture=False,
                    expected_metadata=metadata(),
                )

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
                build_module._load_build_results(
                    stage / "build-results.ndjson", (test,)
                )

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
                verify_stage(stage, expected_metadata=metadata())
            self.assertGreaterEqual(run.call_count, 2)

    def test_initial_stage_publication_fsyncs_source_then_destination(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work_slot = root / "work-slot"
            work_slot.mkdir(mode=0o700)
            new_stage = work_slot / build_module._STAGE_WORK_ROOT_NAME
            new_stage.mkdir(mode=0o700)
            source_parent_descriptor = os.open(
                work_slot, os.O_RDONLY | os.O_DIRECTORY
            )
            destination_parent_descriptor = os.open(
                root, os.O_RDONLY | os.O_DIRECTORY
            )
            new_descriptor = os.open(new_stage, os.O_RDONLY | os.O_DIRECTORY)
            journal_descriptor = build_module._open_stage_journal(
                source_parent_descriptor
            )
            build_module._record_stage_transaction(
                source_parent_descriptor,
                "building",
                new_descriptor,
                journal_descriptor=journal_descriptor,
            )
            fsynced: list[tuple[int, int]] = []

            def record_fsync(descriptor: int) -> None:
                fsynced.append(build_module._descriptor_identity(descriptor))

            try:
                with mock.patch(
                    "scripts.posix.build.os.fsync",
                    side_effect=record_fsync,
                ):
                    old_descriptor = build_module._publish_stage(
                        source_parent_descriptor,
                        new_stage.name,
                        new_descriptor,
                        destination_parent_descriptor,
                        "stage",
                        journal_descriptor=journal_descriptor,
                    )
                self.assertIsNone(old_descriptor)
                self.assertEqual(
                    fsynced[-2:],
                    [
                        build_module._descriptor_identity(
                            source_parent_descriptor
                        ),
                        build_module._descriptor_identity(
                            destination_parent_descriptor
                        ),
                    ],
                )
            finally:
                os.close(journal_descriptor)
                os.close(new_descriptor)
                os.close(destination_parent_descriptor)
                os.close(source_parent_descriptor)

    def test_existing_stage_publication_uses_atomic_directory_exchange(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            old_stage = root / "stage"
            old_stage.mkdir()
            work_slot = root / "work-slot"
            work_slot.mkdir()
            new_stage = work_slot / "stage"
            new_stage.mkdir(mode=0o700)
            source_parent_descriptor = os.open(
                work_slot, os.O_RDONLY | os.O_DIRECTORY
            )
            destination_parent_descriptor = os.open(
                root, os.O_RDONLY | os.O_DIRECTORY
            )
            new_descriptor = os.open(new_stage, os.O_RDONLY | os.O_DIRECTORY)
            rename = build_module._rename_between_at
            old_descriptor: int | None = None
            fsynced: list[tuple[int, int]] = []

            def record_fsync(descriptor: int) -> None:
                fsynced.append(build_module._descriptor_identity(descriptor))

            try:
                build_module._record_stage_transaction(
                    source_parent_descriptor,
                    "idle",
                )
                build_module._record_stage_transaction(
                    source_parent_descriptor,
                    "building",
                    new_descriptor,
                )
                with mock.patch.object(
                    build_module, "_rename_between_at", wraps=rename
                ) as exchange, mock.patch(
                    "scripts.posix.build.os.fsync",
                    side_effect=record_fsync,
                ):
                    old_descriptor = build_module._publish_stage(
                        source_parent_descriptor,
                        new_stage.name,
                        new_descriptor,
                        destination_parent_descriptor,
                        old_stage.name,
                    )

                exchange.assert_called_once_with(
                    source_parent_descriptor,
                    new_stage.name,
                    destination_parent_descriptor,
                    old_stage.name,
                    2,
                )
                self.assertIsNotNone(old_descriptor)
                self.assertEqual(
                    fsynced[-2:],
                    [
                        build_module._descriptor_identity(
                            source_parent_descriptor
                        ),
                        build_module._descriptor_identity(
                            destination_parent_descriptor
                        ),
                    ],
                )
            finally:
                if old_descriptor is not None:
                    os.close(old_descriptor)
                os.close(new_descriptor)
                os.close(destination_parent_descriptor)
                os.close(source_parent_descriptor)


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

    def test_verify_only_supplies_current_expected_inventory(self) -> None:
        expected = metadata()
        checkout = Path("target/posix/src") / expected.revision
        expected_tests = (suite_test(),)
        expected_shell_tests = ("conformance/interfaces/getpid/test.sh",)
        summary = BuildSummary(1, 1, 0, 1, 0, 169, 123)
        with mock.patch(
            "scripts.posix.cli._current_build_inputs",
            create=True,
            return_value=(
                expected,
                checkout,
                expected_tests,
                expected_shell_tests,
            ),
        ) as current, mock.patch(
            "scripts.posix.cli.verify_stage", return_value=summary
        ) as verify:
            result = cli.main(
                ["build", "--arch", "aarch64", "--stage", "custom-stage", "--verify-only"]
            )

        self.assertEqual(result, 0)
        current.assert_called_once_with()
        verify.assert_called_once_with(
            Path("custom-stage"),
            expected_metadata=expected,
            expected_tests=expected_tests,
            expected_shell_tests=expected_shell_tests,
            strict_command_paths=True,
        )

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
        self.assertIn("/host_shared/.smros-posix-stage-quarantine/\n", ignore)


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
            "link-pass=1 link-fail=0 shell-unported=169 staged-bytes=1024",
        )
        with self.assertRaises(Exception):
            summary.discovered = 2


if __name__ == "__main__":
    unittest.main()
