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
REQUIRED_VERSION=3

cd "$ROOT_DIR"

probe_launcher() {
python3 - "$PROBE_HOST" "$PORT" "$REQUIRED_VERSION" <<'PY'
import socket
import sys

host = sys.argv[1]
port = int(sys.argv[2])
required_version = int(sys.argv[3])
request = b"SMROS_VM_PING 1\nname=__smros_probe__\nend\n"
try:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.2)
        sock.connect((host, port))
        sock.sendall(request)
        response = sock.recv(128)
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
if fields.get("monitor_none") != "1":
    raise SystemExit(1)
if fields.get("trace_sync") != "1":
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

if probe_launcher
then
    echo "SMROS VM launcher already reachable on $PROBE_HOST:$PORT"
    exit 0
fi

stop_stale_launcher

echo "Starting SMROS VM launcher on $HOST:$PORT"
if command -v setsid >/dev/null 2>&1; then
    nohup setsid python3 scripts/smros-vm-launcher.py --host "$HOST" --port "$PORT" >"$LOG_FILE" 2>&1 < /dev/null &
else
    nohup python3 scripts/smros-vm-launcher.py --host "$HOST" --port "$PORT" >"$LOG_FILE" 2>&1 < /dev/null &
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
