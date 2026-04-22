#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT_DIR/verification/verus-toolchain.env"

host_os="$(uname -s | tr '[:upper:]' '[:lower:]')"
host_arch="$(uname -m)"

case "${host_os}:${host_arch}" in
    linux:x86_64)
        platform="x86-linux"
        ;;
    darwin:x86_64)
        platform="x86-macos"
        ;;
    darwin:arm64|darwin:aarch64)
        platform="arm64-macos"
        ;;
    *)
        echo "Unsupported Verus host platform: ${host_os}:${host_arch}" >&2
        exit 1
        ;;
esac

install_root="$ROOT_DIR/.tools/verus"
release_dir="$install_root/${VERUS_RELEASE_ID}-${platform}"
archive_path="$install_root/${VERUS_RELEASE_ID}-${platform}.zip"
current_link="$install_root/current"
download_url="https://github.com/verus-lang/verus/releases/download/${VERUS_RELEASE_TAG}/verus-${VERUS_RELEASE_ID}-${platform}.zip"

mkdir -p "$install_root"

if [ ! -d "$release_dir" ]; then
    rm -f "$archive_path"
    curl -fsSL "$download_url" -o "$archive_path"

    extracted_root="$(
        python3 - "$archive_path" "$install_root" <<'PY'
import sys
import zipfile

archive_path = sys.argv[1]
install_root = sys.argv[2]

with zipfile.ZipFile(archive_path) as archive:
    root = archive.namelist()[0].split("/")[0]
    archive.extractall(install_root)
    print(root)
PY
    )"

    rm -rf "$release_dir"
    mv "$install_root/$extracted_root" "$release_dir"
    rm -f "$archive_path"
fi

ln -sfn "$release_dir" "$current_link"
chmod +x "$current_link/verus" "$current_link/cargo-verus" "$current_link/rust_verify" "$current_link/z3"

verus_output="$("$current_link/verus" 2>&1 || true)"
clean_output="$(printf '%s\n' "$verus_output" | sed -E 's/\x1B\[[0-9;]*[A-Za-z]//g')"
required_toolchain="$(
    printf '%s\n' "$clean_output" \
        | sed -n 's/^verus: required rust toolchain \([^ ]*\) not found$/\1/p' \
        | head -n 1
)"

toolchain_to_install="${VERUS_RUST_TOOLCHAIN:-$required_toolchain}"

if [ -n "$toolchain_to_install" ]; then
    if ! rustup toolchain list | sed 's/ (default)//' | grep -Fqx "$toolchain_to_install"; then
        rustup toolchain install "$toolchain_to_install"
    fi

    if ! rustup component list --installed --toolchain "$toolchain_to_install" | grep -Fqx "rust-src"; then
        rustup component add rust-src --toolchain "$toolchain_to_install"
    fi
fi

echo "Verus is available at $current_link"
