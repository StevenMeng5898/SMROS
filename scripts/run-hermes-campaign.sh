#!/usr/bin/env bash
# Boot one SMROS guest and run hermes test-all.
# The serial header is written only by scripts/smros-vm-launcher.py.
# Usage: scripts/run-hermes-campaign.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KERNEL="${KERNEL_IMAGE:-$ROOT/kernel8.img}"
DISK="${FXFS_DISK:-$ROOT/target/hermes-campaign-fxfs.img}"
LOG="${SMROS_ST_LOG:-$ROOT/target/hermes-campaign-serial.log}"
TIMEOUT="${SMROS_ST_TIMEOUT:-14400}"
ITERATIONS="${SMROS_HERMES_ITERATIONS:-100}"
QEMU="${QEMU_SYSTEM:-qemu-system-aarch64}"

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

qemu_argv=(
    "$QEMU"
    -M virt,gic-version=4,virtualization=on
    -cpu cortex-a710
    -smp 4
    -m 2G
    -nographic
    -kernel "$KERNEL"
    -drive "file=$DISK,if=none,format=raw,id=fxfs,cache=writethrough"
    -device virtio-blk-device,drive=fxfs
    -netdev user,id=smrosnet
    -device virtio-net-device,netdev=smrosnet
)
export PYTHONDONTWRITEBYTECODE=1
python3 "$ROOT/scripts/smros-vm-launcher.py" \
    --write-campaign-preamble "$LOG" -- "${qemu_argv[@]}"
"${qemu_argv[@]}" <"$fifo" >>"$LOG" 2>&1 &
qemu_pid=$!

deadline=$((SECONDS + 180))
while [ "$SECONDS" -lt "$deadline" ]; do
    if grep -Fq "smros:/>" "$LOG"; then
        break
    fi
    if ! kill -0 "$qemu_pid" >/dev/null 2>&1; then
        echo "qemu exited before prompt" >&2
        exit 1
    fi
    sleep 1
done
if ! grep -Fq "smros:/>" "$LOG"; then
    echo "no prompt" >&2
    exit 1
fi
echo "prompt_seen utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf 'hermes test-all seed=1 iterations=%s\n' "$ITERATIONS" >&3

end=$((SECONDS + TIMEOUT))
while [ "$SECONDS" -lt "$end" ]; do
    if grep -Fq "Hermes test-all result:" "$LOG"; then
        echo "campaign_finished utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        grep -F "Hermes test-all summary" "$LOG" | tail -n 1
        grep -F "Hermes test-all result:" "$LOG" | tail -n 1
        exit 0
    fi
    if ! kill -0 "$qemu_pid" >/dev/null 2>&1; then
        echo "qemu exited before campaign result" >&2
        tail -n 40 "$LOG" >&2
        exit 1
    fi
    sleep 5
done
echo "campaign timeout" >&2
tail -n 40 "$LOG" >&2
exit 1
