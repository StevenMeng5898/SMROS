#!/usr/bin/env bash
# Run one Verus harness and append an inspectable log.
# Usage: verus-recorded-run.sh <log-path> <harness-src>

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

log_path="$1"
src_path="$2"

# shellcheck disable=SC1091
source "$ROOT_DIR/verification/verus-toolchain.env"
"$ROOT_DIR/scripts/setup-verus.sh" >/dev/null

export PATH="$ROOT_DIR/.tools/verus/current:$PATH"
export RUSTUP_TOOLCHAIN="$VERUS_RUST_TOOLCHAIN"

mkdir -p "$(dirname "$log_path")"

git_head="$(git rev-parse HEAD)"
git_dirty="false"
if [ -n "$(git status --porcelain --untracked-files=no -- . ':(exclude)verification/logs')" ]; then
    git_dirty="true"
fi
utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
verus_bin="$ROOT_DIR/.tools/verus/current/verus"

{
    echo "===== verification log ====="
    echo "utc=$utc"
    echo "git_head=$git_head"
    echo "git_dirty=$git_dirty"
    echo "git_status_before_log_write<<END"
    git status --porcelain --untracked-files=no -- . ':(exclude)verification/logs' || true
    echo "END"
    echo "Status above was recorded before this log file was written."
    echo "A later commit whose diff is only verification/logs/ publishes this run and does not change the sources it checked."
    echo "===== source sha256 ====="
    python3 - "$src_path" <<'PY'
import hashlib, pathlib, re, sys
root = pathlib.Path(".").resolve()
paths = [sys.argv[1]]
doc = pathlib.Path("docs/VERUS_COVERAGE.md").read_text().splitlines()
for index, line in enumerate(doc):
    match = re.match(r"^- `(src/.*\.(?:rs|S))`$", line)
    if not match or index + 1 >= len(doc):
        continue
    if doc[index + 1].strip() == "- 已证明":
        paths.append(match.group(1))
seen = []
for rel in paths:
    if rel in seen:
        continue
    seen.append(rel)
    data = pathlib.Path(rel).read_bytes()
    print(f"{hashlib.sha256(data).hexdigest()}  {rel}")
PY
    echo "===== toolchain ====="
    cat verification/verus-toolchain.env
    echo "===== verus --version ====="
    "$verus_bin" --version
    echo "===== command ====="
    echo "$verus_bin $src_path --crate-type=lib --triggers-mode=silent --no-report-long-running --time-expanded --output-json"
    echo "===== output ====="
} >"$log_path"

tmp_dir="$(mktemp -d)"
cleanup() {
    rm -rf "$tmp_dir"
}
trap cleanup EXIT

set +e
(
    cd "$tmp_dir"
    "$verus_bin" \
        "$ROOT_DIR/$src_path" \
        --crate-type=lib \
        --triggers-mode=silent \
        --no-report-long-running \
        --time-expanded \
        --output-json
) >>"$log_path" 2>&1
status=$?
set -e
echo "===== exit $status =====" >>"$log_path"
exit "$status"
