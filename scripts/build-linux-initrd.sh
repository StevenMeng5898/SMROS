#!/bin/bash
# Build the Linux VM initramfs used by host_shared/vm-linux-demo.xml.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SOURCE_TAR="${1:-$ROOT_DIR/host_shared/ubuntu-alpineamr64.tar}"
OUTPUT="${2:-$ROOT_DIR/host_shared/linux/initrd.img}"

if [ ! -f "$SOURCE_TAR" ]; then
    echo "missing rootfs tar: $SOURCE_TAR" >&2
    exit 1
fi

for tool in busybox fakeroot gzip tar; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 1
    fi
done

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/docker" "$TMP_DIR/root" "$(dirname "$OUTPUT")"

tar -xf "$SOURCE_TAR" -C "$TMP_DIR/docker"
LAYER_TAR="$(find "$TMP_DIR/docker" -maxdepth 2 -type f -name layer.tar | head -n 1)"
if [ -z "$LAYER_TAR" ]; then
    echo "missing Docker layer.tar in $SOURCE_TAR" >&2
    exit 1
fi

tar -xf "$LAYER_TAR" -C "$TMP_DIR/root"

mkdir -p "$TMP_DIR/root/dev" "$TMP_DIR/root/proc" "$TMP_DIR/root/sys" "$TMP_DIR/root/tmp" "$TMP_DIR/root/run"
chmod 1777 "$TMP_DIR/root/tmp"

cat > "$TMP_DIR/root/init" <<'INIT'
#!/bin/sh

export PATH=/sbin:/bin:/usr/sbin:/usr/bin

[ -c /dev/console ] || mknod -m 600 /dev/console c 5 1
[ -c /dev/null ] || mknod -m 666 /dev/null c 1 3

mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sysfs /sys 2>/dev/null || true
mount -t devtmpfs devtmpfs /dev 2>/dev/null || true

[ -c /dev/console ] || mknod -m 600 /dev/console c 5 1
[ -c /dev/null ] || mknod -m 666 /dev/null c 1 3

echo
echo "SMROS Linux VM initramfs"
echo "Kernel: $(uname -srmo)"
echo "Console: ttyAMA0"
echo

exec /bin/sh < /dev/console > /dev/console 2>&1
INIT
chmod 0755 "$TMP_DIR/root/init"

fakeroot -- sh -c '
set -e
root="$1"
output="$2"
cd "$root"
rm -f dev/console dev/null
busybox mknod -m 600 dev/console c 5 1
busybox mknod -m 666 dev/null c 1 3
busybox find . -print0 | busybox cpio -0 -o -H newc -R 0:0 | gzip -9 > "$output"
' sh "$TMP_DIR/root" "$OUTPUT"

echo "built $OUTPUT ($(wc -c < "$OUTPUT") bytes)"
