#!/bin/bash
# Start the SMROS host VM launcher if it is not already reachable.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PORT="${SMROS_VM_LAUNCHER_PORT:-7070}"
HOST="${SMROS_VM_LAUNCHER_HOST:-0.0.0.0}"
PROBE_HOST="${SMROS_VM_LAUNCHER_PROBE_HOST:-127.0.0.1}"
LOG_FILE="${SMROS_VM_LAUNCHER_LOG:-$ROOT_DIR/smros-vm-launcher.log}"
PID_FILE="${SMROS_VM_LAUNCHER_PID:-$ROOT_DIR/smros-vm-launcher.pid}"
REQUIRED_VERSION=12

cd "$ROOT_DIR"
COMMIT="$(git -C "$ROOT_DIR" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)"

probe_launcher() {
python3 - "$PROBE_HOST" "$PORT" "$REQUIRED_VERSION" "$SCRIPT_DIR/smros-vm-launcher.py" <<'PY'
import importlib.util
import socket
import sys

host = sys.argv[1]
port = int(sys.argv[2])
required_version = int(sys.argv[3])
module_path = sys.argv[4]
request = b"SMROS_VM_PING 1\nname=__smros_probe__\nend\n"
try:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.2)
        sock.connect((host, port))
        sock.sendall(request)
        response = sock.recv(2048)
except OSError:
    raise SystemExit(1)
text = response.decode("utf-8", errors="replace")
if not text.startswith("OK "):
    raise SystemExit(1)
fields = dict(
    part.split("=", 1)
    for part in text.strip().split()[1:]
    if "=" in part
)
if int(fields.get("version", "0")) < required_version:
    raise SystemExit(2)
spec = importlib.util.spec_from_file_location("smros_vm_launcher_probe", module_path)
if spec is None or spec.loader is None:
    raise SystemExit(2)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
if not module.accept_running_launcher(text):
    raise SystemExit(2)
if fields.get("monitor_none") != "1":
    raise SystemExit(1)
if fields.get("trace_sync") != "1":
    raise SystemExit(1)
if fields.get("stable_launch") != "1":
    raise SystemExit(1)
if fields.get("vm_log") != "1":
    raise SystemExit(1)
if fields.get("hermes_test_jobs") != "1":
    raise SystemExit(1)
if fields.get("linux_boot") != "1":
    raise SystemExit(1)
if fields.get("launch_retry") != "1":
    raise SystemExit(1)
if fields.get("async_test_jobs") != "1":
    raise SystemExit(1)
PY
}

stop_stale_launcher() {
    if [ -f "$PID_FILE" ]; then
        old_pid="$(cat "$PID_FILE" 2>/dev/null || true)"
        if [ -n "$old_pid" ] && kill -0 "$old_pid" 2>/dev/null; then
            echo "Stopping stale SMROS VM launcher pid $old_pid"
            kill "$old_pid" 2>/dev/null || true
            sleep 0.2
        fi
        rm -f "$PID_FILE"
    fi
    if command -v ss >/dev/null 2>&1; then
        for port_pid in $(ss -ltnp 2>/dev/null | sed -n "s/.*:$PORT .*pid=\([0-9][0-9]*\).*/\1/p" | sort -u); do
            if kill -0 "$port_pid" 2>/dev/null; then
                echo "Stopping stale process on TCP port $PORT pid $port_pid"
                kill "$port_pid" 2>/dev/null || true
                sleep 0.2
            fi
        done
    fi
}

launcher_action() {
    python3 "$SCRIPT_DIR/smros-vm-launcher.py" --action "$PROBE_HOST" "$PORT"
}

drain_host_jobs() {
    local attempt action
    attempt=1
    while [ "$attempt" -le 360 ]; do
        action="$(launcher_action || echo down)"
        if [ "$action" != "drain" ]; then
            return 0
        fi
        echo "Draining in-flight Hermes host jobs before replacing the launcher"
        sleep 1
        attempt=$((attempt + 1))
    done
    echo "SMROS VM launcher still has in-flight host jobs; not replacing it" >&2
    exit 1
}

if [ "${SMROS_VM_LAUNCHER_RESTART:-0}" != "1" ]; then
    action="$(launcher_action || echo down)"
    if [ "$action" = "keep" ]; then
        echo "SMROS VM launcher already reachable on $PROBE_HOST:$PORT (fingerprint matches)"
        exit 0
    fi
fi

drain_host_jobs
if [ "${SMROS_VM_LAUNCHER_RESTART:-0}" = "1" ]; then
    echo "Restarting SMROS VM launcher on $HOST:$PORT"
else
    echo "Replacing SMROS VM launcher on $HOST:$PORT (missing, old, or fingerprint mismatch)"
fi
stop_stale_launcher

# Drop the parent make's directory and locale. `make run` otherwise leaves
# MAKEFLAGS/MAKELEVEL on this daemon, and a zh_CN make then reports host jobs
# in Chinese.
LAUNCHER_ENV=(
    env
    -u MAKEFLAGS
    -u MFLAGS
    -u MAKELEVEL
    -u GNUMAKEFLAGS
    -u MAKEFILES
    -u MAKEOVERRIDES
    -u CURDIR
    LC_ALL=C
    LANG=C
    LANGUAGE=C
)
echo "Starting SMROS VM launcher on $HOST:$PORT"
if command -v setsid >/dev/null 2>&1; then
    nohup setsid "${LAUNCHER_ENV[@]}" python3 scripts/smros-vm-launcher.py --host "$HOST" --port "$PORT" >"$LOG_FILE" 2>&1 < /dev/null &
else
    nohup "${LAUNCHER_ENV[@]}" python3 scripts/smros-vm-launcher.py --host "$HOST" --port "$PORT" >"$LOG_FILE" 2>&1 < /dev/null &
fi
echo "$!" > "$PID_FILE"

sleep 0.5

launcher_pid="$(cat "$PID_FILE" 2>/dev/null || true)"
if [ -z "$launcher_pid" ] || ! kill -0 "$launcher_pid" 2>/dev/null; then
    echo "SMROS VM launcher exited during startup; see $LOG_FILE" >&2
    exit 1
fi

if probe_launcher
then
    sleep 0.2
    if ! kill -0 "$launcher_pid" 2>/dev/null; then
        echo "SMROS VM launcher exited after probe; see $LOG_FILE" >&2
        exit 1
    fi
    echo "SMROS VM launcher log: $LOG_FILE"
else
    echo "SMROS VM launcher did not become reachable; see $LOG_FILE" >&2
    exit 1
fi
