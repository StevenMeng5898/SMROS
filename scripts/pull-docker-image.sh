#!/bin/bash
# Pull an image with the host Docker engine and export a SMROS-loadable archive.

set -euo pipefail

usage() {
    echo "Usage: $0 <image> [output.tar]"
    echo ""
    echo "Environment:"
    echo "  DOCKER_PLATFORM=linux/arm64   Target image platform for SMROS"
    echo ""
    echo "Example:"
    echo "  $0 docker.1ms.run/library/alpine:latest host_shared/alpine.tar"
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ] || [ "$#" -lt 1 ]; then
    usage
    exit 0
fi

IMAGE="$1"
OUT="${2:-host_shared/docker-image.tar}"
PLATFORM="${DOCKER_PLATFORM:-linux/arm64}"
ARCH="${PLATFORM#*/}"
ARCH="${ARCH%%/*}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if [[ "$OUT" = /* ]]; then
    OUT_ABS="$OUT"
else
    OUT_ABS="$ROOT_DIR/$OUT"
fi
WORK_DIR="$(mktemp -d)"
CONTAINER_ID=""

cleanup() {
    if [ -n "$CONTAINER_ID" ]; then
        docker rm -f "$CONTAINER_ID" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

if ! command -v docker >/dev/null 2>&1; then
    echo "pull-docker-image: docker command not found" >&2
    exit 1
fi

if ! docker info >/dev/null 2>&1; then
    echo "pull-docker-image: cannot access Docker daemon" >&2
    echo "Run Docker Desktop/service, add this user to the docker group, or invoke with sudo." >&2
    exit 1
fi

if ! command -v tar >/dev/null 2>&1; then
    echo "pull-docker-image: tar command not found" >&2
    exit 1
fi

mkdir -p "$(dirname "$OUT_ABS")"

echo "Pulling $IMAGE for $PLATFORM on host..."
docker pull --platform "$PLATFORM" "$IMAGE"

REPO_TAG="$IMAGE"
if [[ "$REPO_TAG" != *:* ]]; then
    REPO_TAG="${REPO_TAG}:latest"
fi
CONFIG_NAME="config.json"
LAYER_NAME="layer.tar"

CONTAINER_ID="$(docker create --platform "$PLATFORM" "$IMAGE" /bin/sh)"
docker export "$CONTAINER_ID" -o "$WORK_DIR/$LAYER_NAME"
docker rm "$CONTAINER_ID" >/dev/null
CONTAINER_ID=""

LAYER_SIZE="$(wc -c < "$WORK_DIR/$LAYER_NAME" | tr -d ' ')"
cat > "$WORK_DIR/manifest.json" <<EOF
[
  {
    "Config": "$CONFIG_NAME",
    "RepoTags": ["$REPO_TAG"],
    "Layers": ["$LAYER_NAME"]
  }
]
EOF

cat > "$WORK_DIR/$CONFIG_NAME" <<EOF
{
  "architecture": "$ARCH",
  "os": "linux",
  "config": {
    "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "Entrypoint": ["/bin/sh"],
    "Cmd": [],
    "WorkingDir": "/"
  },
  "rootfs": {
    "type": "layers",
    "diff_ids": ["sha256:smros-host-export"]
  },
  "history": [
    { "created_by": "scripts/pull-docker-image.sh", "comment": "layer bytes $LAYER_SIZE" }
  ]
}
EOF

echo "Saving SMROS-loadable archive to $OUT..."
(
    cd "$WORK_DIR"
    tar -cf "$OUT_ABS" manifest.json "$CONFIG_NAME" "$LAYER_NAME"
)

echo "Wrote $OUT ($(wc -c < "$OUT_ABS" | tr -d ' ') bytes)"
echo ""
echo "Rebuild SMROS so host_shared is embedded, then in SMROS run either:"
echo "  docker pull $IMAGE"
echo "  docker load /shared/$(basename "$OUT")"
