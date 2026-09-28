#!/usr/bin/env python3
"""Host launcher for SMROS `vm -c`.

Run this on the host before starting SMROS:

    scripts/smros-vm-launcher.py

The SMROS guest reaches the host through QEMU user networking at 10.0.2.2 and
asks this daemon to spawn a real QEMU process for Linux VM configs.
"""

from __future__ import annotations

import argparse
import os
import shlex
import signal
import socket
import socketserver
import subprocess
import sys
import threading
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PORT = 7070
MAX_REQUEST = 4096
LAUNCHER_VERSION = 9
DEFAULT_LAUNCH_STABLE_SECONDS = 2.0
DEFAULT_LINUX_BOOT_SECONDS = 45.0
DEFAULT_TERMINATE_TIMEOUT_SECONDS = 3.0
DEFAULT_TEST_TIMEOUT_SECONDS = 300.0
LINUX_BOOT_MARKERS = ("SMROS Linux VM initramfs",)
MAX_TEST_LOG_BYTES = 64 * 1024

LOCK = threading.Lock()
PROCS: dict[str, subprocess.Popen[bytes]] = {}


def parse_request(data: bytes) -> tuple[str, dict[str, str]]:
    text = data.decode("utf-8", errors="strict")
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if not lines:
        raise ValueError("empty request")
    header = lines[0]
    if header not in {
        "SMROS_VM_LAUNCH 1",
        "SMROS_VM_STOP 1",
        "SMROS_VM_PING 1",
        "SMROS_VM_WAIT_BOOT 1",
        "SMROS_TRACE_SYNC 1",
        "SMROS_TEST_RUN 1",
    }:
        raise ValueError("bad header")
    values: dict[str, str] = {}
    for line in lines[1:]:
        if line == "end":
            break
        if "=" not in line:
            raise ValueError(f"bad line: {line!r}")
        key, value = line.split("=", 1)
        if not key or "\x00" in value or "\n" in value or "\r" in value:
            raise ValueError("bad key/value")
        values[key] = value
    return header, values


def repo_path(raw: str | None, *, required: bool) -> Path | None:
    if not raw:
        if required:
            raise ValueError("missing required path")
        return None
    path = Path(raw)
    if not path.is_absolute():
        path = ROOT / path
    path = path.resolve()
    if required and not path.exists():
        raise FileNotFoundError(str(path))
    if path.exists() and ROOT not in path.parents and path != ROOT:
        raise ValueError(f"path escapes repo: {path}")
    return path


def launch_qemu(values: dict[str, str]) -> str:
    name = values.get("name", "vm")
    kernel = repo_path(values.get("kernel"), required=True)
    initrd = repo_path(values.get("initrd"), required=False)
    dtb = repo_path(values.get("dtb"), required=False)
    disk = repo_path(values.get("disk"), required=False)

    cmd = [
        "qemu-system-aarch64",
        "-M",
        values.get("machine", "virt"),
        "-cpu",
        values.get("cpu", "cortex-a57"),
        "-smp",
        values.get("smp", "1"),
        "-m",
        values.get("memory", "512M"),
        "-display",
        values.get("display", "gtk"),
        "-monitor",
        "none",
        *qemu_serial_args(name, values.get("serial", "vc")),
        "-kernel",
        str(kernel),
        "-append",
        values.get("append", "console=ttyAMA0"),
        "-name",
        f"SMROS-{name}",
    ]
    if initrd is not None:
        cmd.extend(["-initrd", str(initrd)])
    if dtb is not None:
        cmd.extend(["-dtb", str(dtb)])
    if disk is not None:
        cmd.extend(
            [
                "-drive",
                f"file={disk},if=none,format={values.get('disk_format', 'raw')},id=rootfs",
                "-device",
                "virtio-blk-device,drive=rootfs",
            ]
        )

    log_path = vm_log_path(name)
    print("smros-vm-launcher: qemu " + shlex.join(cmd[1:]), flush=True)
    print(f"smros-vm-launcher: vm log {log_path.relative_to(ROOT)}", flush=True)
    with LOCK:
        old = PROCS.get(name)
        if old is not None and old.poll() is None:
            print(f"smros-vm-launcher: replacing running VM {name} pid={old.pid}", flush=True)
            terminate_process(old)
        for pid in terminate_qemu_by_name(name):
            print(f"smros-vm-launcher: terminated stale VM {name} pid={pid}", flush=True)
        try:
            serial_log_path(name).write_bytes(b"")
        except OSError:
            pass
        log_file = log_path.open("ab", buffering=0)
        log_file.write(f"\n--- launch {time.strftime('%Y-%m-%d %H:%M:%S')} ---\n".encode())
        log_file.write(("qemu " + shlex.join(cmd[1:]) + "\n").encode())
        try:
            proc = subprocess.Popen(
                cmd,
                cwd=str(ROOT),
                env=qemu_environment(),
                stdout=log_file,
                stderr=subprocess.STDOUT,
                close_fds=True,
            )
        except Exception:
            log_file.close()
            raise
        log_file.close()
        PROCS[name] = proc
    wait_for_stable_launch(name, proc, log_path)
    print(f"smros-vm-launcher: launched {name} pid={proc.pid}", flush=True)
    return f"OK pid={proc.pid} log={log_path.relative_to(ROOT)}\n"


def stop_qemu(values: dict[str, str]) -> str:
    name = values.get("name", "")
    pid_text = values.get("pid", "0")
    with LOCK:
        proc = PROCS.pop(name, None)
    if proc is not None and proc.poll() is None:
        terminate_process(proc)
        return "OK stopped=tracked\n"
    killed = terminate_qemu_by_name(name)
    if killed:
        return f"OK stopped=name count={len(killed)}\n"
    try:
        pid = int(pid_text)
    except ValueError:
        pid = 0
    if pid > 0:
        try:
            os.kill(pid, signal.SIGTERM)
            wait_pid_exit(pid, terminate_timeout_seconds())
            return "OK stopped=pid\n"
        except ProcessLookupError:
            return "OK stopped=already-exited\n"
    return "OK stopped=none\n"


def launcher_status() -> str:
    return (
        f"OK version={LAUNCHER_VERSION} monitor_none=1 stale_qemu_cleanup=1 "
        "trace_sync=1 stable_launch=1 vm_log=1 hermes_test_jobs=1 linux_boot=1\n"
    )


def parse_test_job(values: dict[str, str]) -> tuple[str, str]:
    if set(values) != {"job"}:
        raise ValueError("test request requires exactly one job field")
    job = values["job"]
    if job not in {"ut", "it"}:
        raise ValueError(f"unsupported test job: {job}")
    return ("make", job)


def run_test_job(values: dict[str, str]) -> str:
    cmd = parse_test_job(values)
    job = values["job"]
    test_dir = ROOT / "target" / "hermes-tests"
    test_dir.mkdir(parents=True, exist_ok=True)
    timeout = float(os.environ.get("SMROS_HERMES_TEST_TIMEOUT", DEFAULT_TEST_TIMEOUT_SECONDS))
    if timeout <= 0 or timeout > 1800:
        timeout = DEFAULT_TEST_TIMEOUT_SECONDS
    with LOCK:
        try:
            result = subprocess.run(
                cmd,
                cwd=str(ROOT),
                capture_output=True,
                text=True,
                check=False,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired as exc:
            write_test_log(job, (exc.stdout or "") + (exc.stderr or ""))
            return f"ERR job={job} status=timeout\n"

    output = (result.stdout or "") + (result.stderr or "")
    write_test_log(job, output)
    summary = bounded_test_summary(output)
    prefix = "OK" if result.returncode == 0 else "ERR"
    response = f"{prefix} job={job} status={result.returncode} summary={summary}\n"
    return response


def write_test_log(job: str, output: str) -> None:
    log_dir = ROOT / "target" / "hermes-tests"
    log_dir.mkdir(parents=True, exist_ok=True)
    encoded = output.encode("utf-8", errors="replace")[-MAX_TEST_LOG_BYTES:]
    (log_dir / f"{job}.log").write_bytes(encoded)


def bounded_test_summary(output: str) -> str:
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    summary = lines[-1] if lines else "no-output"
    safe = "".join(char if char.isalnum() or char in ".:_-" else "_" for char in summary)
    return safe[:160] or "no-output"


def sync_trace(values: dict[str, str]) -> str:
    path = values.get("path", "")
    if path != "/shared/trace.pftrace":
        raise ValueError(f"unsupported trace path: {path}")
    disk = Path(os.environ.get("FXFS_DISK", "smros-fxfs.img"))
    if not disk.is_absolute():
        disk = ROOT / disk
    if not disk.exists():
        raise FileNotFoundError(str(disk))
    cmd = [sys.executable, "scripts/sync-host-shared.py", str(disk), "host_shared"]
    result = subprocess.run(cmd, cwd=str(ROOT), capture_output=True, text=True, check=False)
    if result.stdout.strip():
        print("smros-vm-launcher: " + result.stdout.strip(), flush=True)
    if result.stderr.strip():
        print("smros-vm-launcher: " + result.stderr.strip(), flush=True)
    if result.returncode != 0:
        raise RuntimeError(f"sync-host-shared exited {result.returncode}")
    target = ROOT / "host_shared" / "trace.pftrace"
    if not target.exists():
        raise FileNotFoundError(str(target))
    return f"OK synced=1 path=host_shared/trace.pftrace bytes={target.stat().st_size}\n"


def qemu_environment() -> dict[str, str]:
    keep = {
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "TERM",
    }
    drop_prefixes = ("LD_", "SNAP", "GTK_", "GDK_", "QT_")
    env: dict[str, str] = {
        "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    }
    for key in keep:
        value = os.environ.get(key)
        if value:
            env[key] = value
    for key in list(env):
        if key.startswith(drop_prefixes):
            env.pop(key, None)
    return env


def vm_log_path(name: str) -> Path:
    log_dir = ROOT / "target" / "vm-launcher"
    log_dir.mkdir(parents=True, exist_ok=True)
    safe = "".join(ch if ch.isalnum() or ch in ("-", "_", ".") else "_" for ch in name)
    if not safe:
        safe = "vm"
    return log_dir / f"{safe}.log"


def launch_stable_seconds() -> float:
    return env_float("SMROS_VM_LAUNCH_STABLE_SECONDS", DEFAULT_LAUNCH_STABLE_SECONDS)


def linux_boot_seconds() -> float:
    return env_float(
        "SMROS_VM_LINUX_BOOT_SECONDS",
        DEFAULT_LINUX_BOOT_SECONDS,
        maximum=120.0,
    )


def terminate_timeout_seconds() -> float:
    return env_float("SMROS_VM_TERMINATE_TIMEOUT_SECONDS", DEFAULT_TERMINATE_TIMEOUT_SECONDS)


def env_float(name: str, default: float, maximum: float = 30.0) -> float:
    raw = os.environ.get(name)
    if raw is None:
        return default
    try:
        value = float(raw)
    except ValueError:
        return default
    if value < 0.1:
        return 0.1
    if value > maximum:
        return maximum
    return value


def serial_log_path(name: str) -> Path:
    return vm_log_path(name).with_suffix(".serial")


def qemu_serial_args(name: str, serial: str) -> list[str]:
    if serial == "vc" or serial.startswith("vc:"):
        path = serial_log_path(name)
        chardev = f"vc,id=smros-serial,logfile={path},logappend=off"
        if serial.startswith("vc:") and "x" in serial:
            geom = serial.split(":", 1)[1]
            width, _, height = geom.partition("x")
            if width.isdigit() and height.isdigit():
                chardev = (
                    f"vc,id=smros-serial,width={width},height={height},"
                    f"logfile={path},logappend=off"
                )
        return ["-chardev", chardev, "-serial", "chardev:smros-serial"]
    return ["-serial", serial]


def linux_boot_seen_text(text: str) -> bool:
    return any(marker in text for marker in LINUX_BOOT_MARKERS)


def linux_boot_seen(path: Path) -> bool:
    try:
        data = path.read_bytes()
    except OSError:
        return False
    return linux_boot_seen_text(data.decode("utf-8", errors="replace"))


def wait_for_linux_boot(values: dict[str, str]) -> str:
    name = values.get("name", "")
    if not name:
        raise ValueError("missing name")
    serial_path = serial_log_path(name)
    with LOCK:
        proc = PROCS.get(name)
    deadline = time.monotonic() + linux_boot_seconds()
    while time.monotonic() < deadline:
        if proc is not None and proc.poll() is not None:
            raise RuntimeError(
                f"qemu exited before linux boot status={proc.returncode} "
                f"serial={serial_path.name} tail={log_tail(serial_path)}"
            )
        if linux_boot_seen(serial_path):
            print(f"smros-vm-launcher: linux boot OK {name}", flush=True)
            return "OK boot=1 marker=initramfs\n"
        time.sleep(0.1)
    if linux_boot_seen(serial_path):
        print(f"smros-vm-launcher: linux boot OK {name}", flush=True)
        return "OK boot=1 marker=initramfs\n"
    print(f"smros-vm-launcher: linux boot timed out {name}", flush=True)
    return "OK boot=0 timed_out=1\n"


def wait_for_stable_launch(name: str, proc: subprocess.Popen[bytes], log_path: Path) -> None:
    deadline = time.monotonic() + launch_stable_seconds()
    while time.monotonic() < deadline:
        return_code = proc.poll()
        if return_code is not None:
            with LOCK:
                if PROCS.get(name) is proc:
                    PROCS.pop(name, None)
            raise RuntimeError(
                f"qemu exited during startup status={return_code} "
                f"log={log_path.relative_to(ROOT)} tail={log_tail(log_path)}"
            )
        time.sleep(0.1)


def log_tail(path: Path, limit: int = 600) -> str:
    try:
        data = path.read_bytes()[-limit:]
    except OSError:
        return "<unavailable>"
    text = data.decode("utf-8", errors="replace").replace("\n", " | ")
    return text.strip() or "<empty>"


def terminate_process(proc: subprocess.Popen[bytes]) -> None:
    proc.terminate()
    deadline = time.monotonic() + terminate_timeout_seconds()
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            return
        time.sleep(0.05)
    proc.kill()
    try:
        proc.wait(timeout=1.0)
    except subprocess.TimeoutExpired:
        pass


def terminate_qemu_by_name(name: str) -> list[int]:
    if not name:
        return []
    expected = f"SMROS-{name}"
    killed: list[int] = []
    proc_root = Path("/proc")
    for entry in proc_root.iterdir():
        if not entry.name.isdecimal():
            continue
        pid = int(entry.name)
        try:
            raw = (entry / "cmdline").read_bytes()
        except OSError:
            continue
        if not raw:
            continue
        args = [part.decode("utf-8", errors="replace") for part in raw.split(b"\0") if part]
        if not args or Path(args[0]).name != "qemu-system-aarch64":
            continue
        if not qemu_args_match_name(args, expected):
            continue
        try:
            os.kill(pid, signal.SIGTERM)
            wait_pid_exit(pid, terminate_timeout_seconds())
            killed.append(pid)
        except ProcessLookupError:
            continue
    return killed


def wait_pid_exit(pid: int, timeout_seconds: float) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.05)
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return


def qemu_args_match_name(args: list[str], expected: str) -> bool:
    for index, arg in enumerate(args):
        if arg == "-name" and index + 1 < len(args):
            name_arg = args[index + 1]
            return name_arg == expected or name_arg.startswith(expected + ",")
    return False


class Handler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        try:
            self.request.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
        except OSError:
            pass
        data = self.request.recv(MAX_REQUEST)
        try:
            header, values = parse_request(data)
            if header == "SMROS_VM_LAUNCH 1":
                action = "launch"
            elif header == "SMROS_VM_STOP 1":
                action = "stop"
            elif header == "SMROS_VM_WAIT_BOOT 1":
                action = "wait-boot"
            elif header == "SMROS_TRACE_SYNC 1":
                action = "trace-sync"
            elif header == "SMROS_TEST_RUN 1":
                action = "test-run"
            else:
                action = "ping"
            print(
                f"smros-vm-launcher: {action} request from {self.client_address[0]}:{self.client_address[1]} name={values.get('name', '')}",
                flush=True,
            )
            if header == "SMROS_VM_LAUNCH 1":
                response = launch_qemu(values)
            elif header == "SMROS_VM_STOP 1":
                response = stop_qemu(values)
            elif header == "SMROS_VM_WAIT_BOOT 1":
                response = wait_for_linux_boot(values)
            elif header == "SMROS_TRACE_SYNC 1":
                response = sync_trace(values)
            elif header == "SMROS_TEST_RUN 1":
                response = run_test_job(values)
            else:
                response = launcher_status()
        except Exception as exc:  # Keep daemon alive; report concise cause.
            response = f"ERR {type(exc).__name__}: {exc}\n"
            print(f"smros-vm-launcher: {response.strip()}", flush=True)
        self.request.sendall(response.encode("utf-8"))


class LauncherServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True

    def server_bind(self) -> None:
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
        super().server_bind()


def main() -> int:
    parser = argparse.ArgumentParser(description="SMROS host VM launcher")
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    args = parser.parse_args()

    with LauncherServer((args.host, args.port), Handler) as server:
        print(f"smros-vm-launcher: listening on {args.host}:{args.port}", flush=True)
        print("smros-vm-launcher: paths are resolved relative to repo root", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            print("\nsmros-vm-launcher: stopped", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
