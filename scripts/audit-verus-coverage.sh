#!/usr/bin/env bash
# Check that source files are classified for Verus and shared logic is wired.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

status=0

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

shared_files="$(find src -type f -name '*_logic_shared.rs' | sort)"

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

if [ "$status" -eq 0 ]; then
    echo "Verus coverage audit passed."
fi

exit "$status"
