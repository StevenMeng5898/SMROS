#!/usr/bin/env python3
"""Boot AArch64 SMROS and collect sectest CASE lines.

Exit status follows `SECTEST SUMMARY`: 0 when `fail=0`, 1 when any hard FAIL
is present or the boot times out before a summary. XFAIL and XPASS do not
change the exit status. A timeout is never treated as PASS.

SEC-4 cases, including `sectest undef`, are real PASS/FAIL results. A fatal
synchronous exception or a missing summary is FAIL.
"""

from __future__ import annotations

import argparse
import os
import pty
import re
import subprocess
import sys
import time
from pathlib import Path

SUMMARY_RE = re.compile(
    r"SECTEST SUMMARY pass=(\d+) fail=(\d+) xfail=(\d+) xpass=(\d+)"
)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=int, default=30)
    parser.add_argument("--kernel", type=Path, default=None)
    parser.add_argument("--undef", action="store_true", help="boot only the undefined-instruction case")
    args = parser.parse_args()

    repo = Path(__file__).resolve().parents[3]
    kernel = args.kernel or (repo / "kernel8.img")
    if not kernel.is_file():
        print(f"CASE runner RESULT FAIL missing-kernel {kernel}", flush=True)
        print("SECTEST SUMMARY pass=0 fail=1 xfail=0 xpass=0", flush=True)
        return 1
    qemu = os.environ.get("QEMU_SYSTEM", "qemu-system-aarch64")
    if subprocess.call(["bash", "-lc", f"command -v {qemu}"], stdout=subprocess.DEVNULL) != 0:
        print("CASE runner RESULT SKIP qemu-absent", flush=True)
        print("SECTEST SUMMARY pass=0 fail=1 xfail=0 xpass=0", flush=True)
        return 1

    log_dir = repo / "target" / "security"
    log_dir.mkdir(parents=True, exist_ok=True)
    suffix = "undef" if args.undef else "all"
    log_path = log_dir / f"sec0-{suffix}.log"
    disk = log_dir / "smros-fxfs.img"
    if not disk.exists():
        subprocess.check_call(
            ["qemu-img", "create", "-f", "raw", str(disk), "128M"],
            stdout=subprocess.DEVNULL,
        )

    command = "sectest undef\n" if args.undef else "sectest all\n"
    return boot(qemu, kernel, disk, log_path, command, args.timeout)


def boot(qemu: str, kernel: Path, disk: Path, log_path: Path, command: str, timeout: int) -> int:
    master, slave = pty.openpty()
    proc = subprocess.Popen(
        [
            qemu,
            "-M",
            "virt,gic-version=4,virtualization=on",
            "-cpu",
            "cortex-a710",
            "-smp",
            "1",
            "-m",
            "1G",
            "-nographic",
            "-kernel",
            str(kernel),
            "-drive",
            f"file={disk},if=none,format=raw,id=fxfs,cache=writethrough",
            "-device",
            "virtio-blk-device,drive=fxfs",
            "-netdev",
            "user,id=smrosnet",
            "-device",
            "virtio-net-device,netdev=smrosnet",
        ],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
    )
    os.close(slave)
    os.set_blocking(master, False)
    sent = False
    deadline = time.time() + timeout
    suite = None
    text = ""
    try:
        with open(log_path, "w", buffering=1) as log:
            while time.time() < deadline:
                try:
                    chunk = os.read(master, 4096)
                except BlockingIOError:
                    chunk = b""
                if chunk:
                    piece = chunk.decode(errors="replace")
                    log.write(piece)
                    log.flush()
                    text += piece
                if not sent and "smros:/>" in text:
                    os.write(master, command.encode())
                    sent = True
                if SUMMARY_RE.search(text) and "SEC-HEARTBEAT" in text:
                    suite = "SUMMARY"
                    break
                if proc.poll() is not None:
                    break
                time.sleep(0.05)
    finally:
        proc.kill()
        proc.wait(timeout=5)
        os.close(master)

    text = log_path.read_text(errors="replace")
    for line in text.splitlines():
        if (
            line.startswith("CASE ")
            or line.startswith("SEC-HEARTBEAT")
            or line.startswith("SECTEST SUMMARY")
        ):
            print(line)
    summary = SUMMARY_RE.search(text)
    if summary is None and "fatal synchronous exception" in text:
        print("CASE runner RESULT FAIL kernel-halt")
        print("SECTEST SUMMARY pass=0 fail=1 xfail=0 xpass=0")
        print(f"LOG {log_path}")
        return 1
    if summary is None:
        print(f"CASE runner RESULT FAIL timeout-or-halt sent={sent} bytes={len(text)}")
        print("SECTEST SUMMARY pass=0 fail=1 xfail=0 xpass=0")
        print(f"LOG {log_path}")
        return 1
    print(f"LOG {log_path}")
    return 0 if int(summary.group(2)) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
