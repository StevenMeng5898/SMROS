#!/usr/bin/env bash
# Convert the QEMU smoke-test serial log into a small HTML report artifact.

set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 <smoke-log> <html-report>" >&2
    exit 2
fi

SMOKE_LOG="$1"
HTML_REPORT="$2"
SMROS_ST_PROMPT="${SMROS_ST_PROMPT:-smros:/>}"
DEFAULT_SMROS_ST_REQUIRED_PATTERNS="SMROS-A Distributed AI-Native Operating System|[OK] Kernel initialized successfully!|[OK] Serial console initialized|[SYSCALL] Syscall handler initialized|[CHANNEL] Channel subsystem initialized|[INFO] Fast boot complete. Starting shell|[SHELL] Starting shell as scheduled thread...|$SMROS_ST_PROMPT"
SMROS_ST_REQUIRED_PATTERNS="${SMROS_ST_REQUIRED_PATTERNS:-$DEFAULT_SMROS_ST_REQUIRED_PATTERNS}"

mkdir -p "$(dirname "$HTML_REPORT")"

html_escape() {
    sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g'
}

required_patterns=()
IFS='|' read -r -a required_patterns <<< "$SMROS_ST_REQUIRED_PATTERNS"
found=0
total=0
rows=""

for pattern in "${required_patterns[@]}"; do
    if [ -z "$pattern" ]; then
        continue
    fi

    total=$((total + 1))
    escaped_pattern="$(printf '%s\n' "$pattern" | html_escape)"
    if [ -f "$SMOKE_LOG" ] && grep -Fq "$pattern" "$SMOKE_LOG"; then
        found=$((found + 1))
        rows="${rows}<tr><td>found</td><td><code>${escaped_pattern}</code></td></tr>"
    else
        rows="${rows}<tr><td>missing</td><td><code>${escaped_pattern}</code></td></tr>"
    fi
done

if [ "$total" -eq 0 ]; then
    percent="100.00"
else
    percent="$(awk -v found="$found" -v total="$total" 'BEGIN { printf "%.2f", (found * 100) / total }')"
fi

{
    printf '%s\n' '<!doctype html>'
    printf '%s\n' '<html lang="en">'
    printf '%s\n' '<head>'
    printf '%s\n' '<meta charset="utf-8">'
    printf '%s\n' '<title>SMROS System Smoke Report</title>'
    printf '%s\n' '<style>body{font:14px/1.45 system-ui,sans-serif;margin:24px;background:#f7f8fa;color:#111827}main{max-width:1100px;margin:0 auto}pre{background:#111827;color:#e5e7eb;padding:16px;overflow:auto;border-radius:6px}code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}.note{color:#4b5563}.score{font-size:28px;font-weight:700}table{border-collapse:collapse;width:100%;background:white}td,th{border:1px solid #d1d5db;padding:8px;text-align:left}td:first-child{width:90px;font-weight:700}</style>'
    printf '%s\n' '</head>'
    printf '%s\n' '<body><main>'
    printf '%s\n' '<h1>SMROS System Smoke Report</h1>'
    printf '%s\n' '<p class="note">QEMU system smoke coverage is reported as required serial milestones, not cargo-tarpaulin line coverage.</p>'
    printf '<p class="score">%s%% milestone coverage (%s/%s)</p>\n' "$percent" "$found" "$total"
    printf '%s\n' '<h2>Required Milestones</h2>'
    printf '%s\n' '<table><thead><tr><th>Status</th><th>Serial Pattern</th></tr></thead><tbody>'
    printf '%s\n' "$rows"
    printf '%s\n' '</tbody></table>'
    printf '%s\n' '<h2>Serial Log Tail</h2>'
    printf '%s\n' '<pre><code>'
    if [ -f "$SMOKE_LOG" ]; then
        tail -n 160 "$SMOKE_LOG" | html_escape
    else
        printf 'missing smoke log: %s\n' "$SMOKE_LOG" | html_escape
    fi
    printf '%s\n' '</code></pre>'
    printf '%s\n' '</main></body></html>'
} > "$HTML_REPORT"

echo "SMROS system smoke HTML report: $HTML_REPORT"
