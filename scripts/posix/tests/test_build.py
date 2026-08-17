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
            self.assertEqual(compile_row["argv"][8], str(source))

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
        self.assertIn("-ldl", command)
        self.assertEqual(
            command[command.index("-o") + 1],
            "target/libsmros-posix-compat.so",
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
            compile_row["argv"][8] = f"/tmp/untrusted/{test.source}"
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
