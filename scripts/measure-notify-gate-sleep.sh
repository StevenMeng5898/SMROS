#!/usr/bin/env bash
# Boot SMROS and run `sectest notify-gate-sleep` N times.
# Prints one NOTIFY-GATE SLEEP line and one CASE line per run.
# Usage: scripts/measure-notify-gate-sleep.sh [runs]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNS="${1:-50}"
KERNEL="${KERNEL_IMAGE:-$ROOT/kernel8.img}"
DISK="${FXFS_DISK:-$ROOT/target/notify-gate-fxfs.img}"
LOG="${SMROS_ST_LOG:-$ROOT/target/notify-gate-sleep-serial.log}"
TIMEOUT="${SMROS_ST_TIMEOUT:-900}"
QEMU="${QEMU_SYSTEM:-qemu-system-aarch64}"

if [ ! -f "$KERNEL" ]; then
    echo "kernel image missing: $KERNEL" >&2
    exit 1
fi
mkdir -p "$(dirname "$DISK")" "$(dirname "$LOG")"
if [ ! -f "$DISK" ]; then
    qemu-img create -f raw "$DISK" 128M >/dev/null
fi

fifo="${LOG}.stdin"
rm -f "$LOG" "$fifo"
mkfifo "$fifo"
exec 3<>"$fifo"

qemu_pid=""
cleanup() {
    if [ -n "$qemu_pid" ] && kill -0 "$qemu_pid" >/dev/null 2>&1; then
        kill "$qemu_pid" >/dev/null 2>&1 || true
        wait "$qemu_pid" >/dev/null 2>&1 || true
    fi
    exec 3>&- || true
    rm -f "$fifo"
}
trap cleanup EXIT INT TERM

"$QEMU" \
    -M virt,gic-version=4,virtualization=on \
    -cpu cortex-a710 \
    -smp 4 \
    -m 2G \
    -nographic \
    -kernel "$KERNEL" \
    -drive file="$DISK",if=none,format=raw,id=fxfs,cache=writethrough \
    -device virtio-blk-device,drive=fxfs \
    -netdev user,id=smrosnet \
    -device virtio-net-device,netdev=smrosnet \
    <"$fifo" >"$LOG" 2>&1 &
qemu_pid=$!

deadline=$((SECONDS + TIMEOUT))
while [ "$SECONDS" -lt "$deadline" ]; do
    if grep -Fq "smros:/>" "$LOG"; then
        break
    fi
    if ! kill -0 "$qemu_pid" >/dev/null 2>&1; then
        echo "qemu exited before the shell prompt" >&2
        exit 1
    fi
    sleep 1
done
if ! grep -Fq "smros:/>" "$LOG"; then
    echo "timed out waiting for smros:/>" >&2
    exit 1
fi

echo "utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "git_head=$(git -C "$ROOT" rev-parse HEAD)"
echo "runs=$RUNS"
echo "boot_prompt_seen=1"

for run in $(seq 1 "$RUNS"); do
    before="$(wc -l < "$LOG")"
    printf 'sectest notify-gate-sleep\n' >&3
    found=""
    run_deadline=$((SECONDS + 30))
    while [ "$SECONDS" -lt "$run_deadline" ]; do
        if awk -v start="$before" 'NR > start && /CASE notify-gate-sleep RESULT / { found=1 } END { exit found ? 0 : 1 }' "$LOG"; then
            found=1
            break
        fi
        if ! kill -0 "$qemu_pid" >/dev/null 2>&1; then
            echo "qemu exited during run $run" >&2
            exit 1
        fi
        sleep 0.2
    done
    if [ -z "$found" ]; then
        echo "run=$run TIMEOUT" >&2
        exit 1
    fi
    awk -v start="$before" 'NR > start && (/NOTIFY-GATE SUMMARY / || /CASE notify-gate-sleep RESULT /) { print }' "$LOG"
done
