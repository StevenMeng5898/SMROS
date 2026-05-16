#!/bin/bash
# Configure Linux host ICMP echo sockets for QEMU user networking.

set -euo pipefail

TARGET_RANGE="${SMROS_PING_GROUP_RANGE:-0 2147483647}"
PERSISTENT_CONF="${SMROS_PING_GROUP_CONF:-/etc/sysctl.d/99-smros-qemu-icmp.conf}"
SYSCTL_KEY="net.ipv4.ping_group_range"
SYSCTL_KEY_PATTERN="net\\.ipv4\\.ping_group_range"
PROC_NODE="/proc/sys/net/ipv4/ping_group_range"

usage() {
    echo "Usage: $0 [--check|--ensure]"
    echo ""
    echo "  --check   Report whether the current Linux host allows QEMU ICMP echo"
    echo "  --ensure  Persist and apply ${SYSCTL_KEY} for QEMU user networking"
}

die() {
    echo "setup-qemu-icmp: $*" >&2
    exit 1
}

validate_range() {
    case "$TARGET_RANGE" in
        *[!0-9\ ]* | "" | *"  "* | " "* | *" ")
            die "invalid SMROS_PING_GROUP_RANGE: '$TARGET_RANGE'"
            ;;
    esac

    set -- $TARGET_RANGE
    [ "$#" -eq 2 ] || die "SMROS_PING_GROUP_RANGE must be two group ids"
    [ "$1" -le "$2" ] || die "SMROS_PING_GROUP_RANGE start must be <= end"
}

current_range() {
    if command -v sysctl >/dev/null 2>&1; then
        sysctl -n "$SYSCTL_KEY" 2>/dev/null || true
    elif [ -r "$PROC_NODE" ]; then
        tr '\t' ' ' < "$PROC_NODE"
    fi
}

range_contains_gid() {
    local range="$1"
    local gid="$2"
    set -- $range
    [ "$#" -eq 2 ] || return 1
    [ "$1" -le "$gid" ] && [ "$gid" -le "$2" ]
}

persistent_configured() {
    [ -r "$PERSISTENT_CONF" ] && grep -Eq "^[[:space:]]*${SYSCTL_KEY_PATTERN}[[:space:]]*=[[:space:]]*${TARGET_RANGE}[[:space:]]*$" "$PERSISTENT_CONF"
}

write_persistent_config() {
    local line="${SYSCTL_KEY} = ${TARGET_RANGE}"

    if [ "$(id -u)" -eq 0 ]; then
        printf '%s\n' "$line" > "$PERSISTENT_CONF"
    elif command -v sudo >/dev/null 2>&1; then
        printf '%s\n' "$line" | sudo tee "$PERSISTENT_CONF" >/dev/null
    else
        die "sudo is required to write $PERSISTENT_CONF"
    fi
}

apply_runtime_config() {
    if command -v sysctl >/dev/null 2>&1; then
        if [ "$(id -u)" -eq 0 ]; then
            sysctl -w "${SYSCTL_KEY}=${TARGET_RANGE}" >/dev/null
        elif command -v sudo >/dev/null 2>&1; then
            sudo sysctl -w "${SYSCTL_KEY}=${TARGET_RANGE}" >/dev/null
        else
            die "sudo is required to set ${SYSCTL_KEY}"
        fi
    elif [ "$(id -u)" -eq 0 ]; then
        printf '%s\n' "$TARGET_RANGE" > "$PROC_NODE"
    elif command -v sudo >/dev/null 2>&1; then
        printf '%s\n' "$TARGET_RANGE" | sudo tee "$PROC_NODE" >/dev/null
    else
        die "sudo is required to set ${SYSCTL_KEY}"
    fi
}

MODE="${1:---check}"
case "$MODE" in
    --check | --ensure)
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac

if [ "$(uname -s)" != "Linux" ]; then
    echo "setup-qemu-icmp: non-Linux host; ${SYSCTL_KEY} is not used"
    exit 0
fi

if [ ! -e "$PROC_NODE" ]; then
    echo "setup-qemu-icmp: ${SYSCTL_KEY} is unavailable on this host"
    exit 0
fi

validate_range

GID="$(id -g)"
CURRENT_RANGE="$(current_range)"

if range_contains_gid "$CURRENT_RANGE" "$GID" && persistent_configured; then
    exit 0
fi

if [ "$MODE" = "--check" ]; then
    echo "setup-qemu-icmp: ${SYSCTL_KEY} is '${CURRENT_RANGE:-unknown}'"
    if ! range_contains_gid "$CURRENT_RANGE" "$GID"; then
        echo "setup-qemu-icmp: gid $GID is not allowed to create ICMP echo sockets"
    fi
    if ! persistent_configured; then
        echo "setup-qemu-icmp: persistent config missing: $PERSISTENT_CONF"
    fi
    exit 1
fi

echo "setup-qemu-icmp: enabling persistent QEMU ICMP echo support"
echo "setup-qemu-icmp: writing $PERSISTENT_CONF"
write_persistent_config
echo "setup-qemu-icmp: applying ${SYSCTL_KEY}='${TARGET_RANGE}'"
apply_runtime_config

CURRENT_RANGE="$(current_range)"
if ! range_contains_gid "$CURRENT_RANGE" "$GID"; then
    die "${SYSCTL_KEY} is still '${CURRENT_RANGE:-unknown}', gid $GID is not allowed"
fi

echo "setup-qemu-icmp: ok (${SYSCTL_KEY}='${CURRENT_RANGE}')"
