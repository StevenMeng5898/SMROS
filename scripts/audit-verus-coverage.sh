#!/usr/bin/env bash
# Check that source files are classified for Verus and shared logic is wired.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

status=0

if [ -f verification/logs/verus-syscall.log ]; then
    python3 - <<'PY'
import json, pathlib, re
root = pathlib.Path("verification/logs")
names = [
    "verus-syscall",
    "verus-kernel-objects",
    "verus-kernel-lowlevel",
    "verus-user-level",
    "verus-services",
]
first = (root / "verus-syscall.log").read_text()
header = first.split("\n{", 1)[0]
lines = ["===== make verus summary ====="]
for key in ("utc", "git_head", "git_dirty"):
    match = re.search(rf"^{key}=(.*)$", header, re.M)
    if match:
        lines.append(f"{key}={match.group(1)}")
lines.append("Per-harness totals from Verus --output-json. Full JSON stays local and is not committed.")
for name in names:
    text = (root / f"{name}.log").read_text()
    data, _ = json.JSONDecoder().raw_decode(text[text.find("{") :])
    result = data.get("verification-results", {})
    lines.append(
        f"{name}: verified={result.get('verified')} errors={result.get('errors')} success={result.get('success')}"
    )
(root / "make-verus-summary.txt").write_text("\n".join(lines) + "\n")
PY
fi

contains_fixed() {
    local needle="$1"
    local path="$2"
    grep -R -F -q -- "$needle" "$path"
}

contains_regex() {
    local pattern="$1"
    local path="$2"
    grep -R -E -q -- "$pattern" "$path"
}

macro_lines() {
    local path="$1"
    grep -E -o '^macro_rules! [A-Za-z0-9_]+ ' "$path" || true
}

shared_files="$(
    find src -type f -name '*_logic_shared.rs'
    printf '%s\n' \
        src/syscall/syscall_bridge_shared.rs \
        src/syscall/linux_runtime_lock_shared.rs
)"
shared_files="$(printf '%s\n' "$shared_files" | sort -u)"

for src_file in $shared_files; do
    include_path="../../../$src_file"
    if ! contains_fixed "include!(\"$include_path\")" verification; then
        echo "missing Verus include for shared logic: $src_file" >&2
        status=1
    fi
done

while IFS= read -r src_file; do
    while IFS= read -r macro_line; do
        [ -n "$macro_line" ] || continue
        macro_name="${macro_line#macro_rules! }"
        macro_name="${macro_name%% *}"
        case "$macro_name" in
            smros_ko_align_up_checked_body)
                # Verus currently reports Rust's usize::is_power_of_two as unsupported.
                # Keep this exception explicit so the gap is visible and removable.
                continue
                ;;
        esac

        if ! contains_regex "${macro_name}!" verification; then
            echo "missing Verus macro use: $src_file $macro_name" >&2
            status=1
        fi
    done < <(macro_lines "$src_file")
done < <(printf '%s\n' "$shared_files")

classified_file="$(mktemp)"
generated_file="$(mktemp)"
cleanup() {
    rm -f "$classified_file" "$generated_file"
}
trap cleanup EXIT

sed -n 's/^- `\(src\/.*\.\(rs\|S\)\)`$/\1/p' docs/VERUS_COVERAGE.md | sort >"$classified_file"
find src -type f \( -name '*.rs' -o -name '*.S' \) | sort >"$generated_file"

if ! diff -u "$generated_file" "$classified_file"; then
    echo "docs/VERUS_COVERAGE.md must classify every src/*.rs and src/*.S file" >&2
    status=1
fi

# Each inventory line must carry exactly one of the six coverage labels.
# This check is in addition to the path-set diff above.
if ! python3 - <<'PY'
import pathlib, re, sys
text = pathlib.Path("docs/VERUS_COVERAGE.md").read_text()
lines = text.splitlines()
path_re = re.compile(r"^- `(src/.*\.(?:rs|S))`$")
label_re = re.compile(r"^  - (已证明|待证明|公理或可信计算基|规范假设|超出范围|未覆盖)$")
missing = []
for index, line in enumerate(lines):
    if not path_re.match(line):
        continue
    if index + 1 >= len(lines) or not label_re.match(lines[index + 1]):
        missing.append(line)
if missing:
    print("docs/VERUS_COVERAGE.md entry is missing exactly one coverage label:", file=sys.stderr)
    for line in missing:
        print(line, file=sys.stderr)
    sys.exit(1)
PY
then
    status=1
fi

if ! python3 scripts/audit-verus-proofs.py --self-test; then
    status=1
fi

if ! python3 scripts/audit-verus-mutation.py --self-test; then
    status=1
fi

if ! python3 scripts/audit-verus-proofs.py; then
    status=1
fi

if [ "$status" -eq 0 ]; then
    echo "Verus coverage audit passed."
fi

exit "$status"
