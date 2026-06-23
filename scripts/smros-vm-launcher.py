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
import socketserver
import subprocess
import sys
import threading
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PORT = 7070
MAX_REQUEST = 4096
LAUNCHER_VERSION = 3

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
        "SMROS_TRACE_SYNC 1",
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
        "-serial",
        values.get("serial", "vc:1024x768"),
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

    print("smros-vm-launcher: qemu " + shlex.join(cmd[1:]), flush=True)
    with LOCK:
        old = PROCS.get(name)
        if old is not None and old.poll() is None:
            print(f"smros-vm-launcher: replacing running VM {name} pid={old.pid}", flush=True)
            old.terminate()
        for pid in terminate_qemu_by_name(name):
            print(f"smros-vm-launcher: terminated stale VM {name} pid={pid}", flush=True)
        proc = subprocess.Popen(cmd, cwd=str(ROOT), env=qemu_environment())
        PROCS[name] = proc
    time.sleep(0.2)
    return_code = proc.poll()
    if return_code is not None:
        with LOCK:
            if PROCS.get(name) is proc:
                PROCS.pop(name, None)
        raise RuntimeError(f"qemu exited immediately status={return_code}")
    print(f"smros-vm-launcher: launched {name} pid={proc.pid}", flush=True)
    return f"OK pid={proc.pid}\n"


def stop_qemu(values: dict[str, str]) -> str:
    name = values.get("name", "")
    pid_text = values.get("pid", "0")
    with LOCK:
        proc = PROCS.pop(name, None)
    if proc is not None and proc.poll() is None:
        proc.terminate()
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
            return "OK stopped=pid\n"
        except ProcessLookupError:
            return "OK stopped=already-exited\n"
    return "OK stopped=none\n"


def launcher_status() -> str:
    return f"OK version={LAUNCHER_VERSION} monitor_none=1 stale_qemu_cleanup=1 trace_sync=1\n"


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
    env: dict[str, str] = {
        "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    }
    for key in keep:
        value = os.environ.get(key)
        if value:
            env[key] = value
    return env


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
            killed.append(pid)
        except ProcessLookupError:
            continue
    return killed


def qemu_args_match_name(args: list[str], expected: str) -> bool:
    for index, arg in enumerate(args):
        if arg == "-name" and index + 1 < len(args):
            name_arg = args[index + 1]
            return name_arg == expected or name_arg.startswith(expected + ",")
    return False


class Handler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        data = self.request.recv(MAX_REQUEST)
        try:
            header, values = parse_request(data)
            if header == "SMROS_VM_LAUNCH 1":
                action = "launch"
            elif header == "SMROS_VM_STOP 1":
                action = "stop"
            elif header == "SMROS_TRACE_SYNC 1":
                action = "trace-sync"
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
            elif header == "SMROS_TRACE_SYNC 1":
                response = sync_trace(values)
            else:
                response = launcher_status()
        except Exception as exc:  # Keep daemon alive; report concise cause.
            response = f"ERR {type(exc).__name__}: {exc}\n"
            print(f"smros-vm-launcher: {response.strip()}", flush=True)
        self.request.sendall(response.encode("utf-8"))


class LauncherServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True


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
