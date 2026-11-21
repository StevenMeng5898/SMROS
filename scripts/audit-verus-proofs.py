#!/usr/bin/env python3
"""Cross-check 已证明 files against raw Verus JSON. Existing shell checks stay separate."""

import hashlib
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
LOGS = [
    ROOT / "verification/logs/verus-syscall.log",
    ROOT / "verification/logs/verus-kernel-objects.log",
    ROOT / "verification/logs/verus-kernel-lowlevel.log",
    ROOT / "verification/logs/verus-user-level.log",
    ROOT / "verification/logs/verus-services.log",
]
HARNESS_SOURCES = [
    ROOT / "verification/syscall/src/lib.rs",
    ROOT / "verification/kernel_objects/src/lib.rs",
    ROOT / "verification/kernel_lowlevel/src/lib.rs",
    ROOT / "verification/user_level/src/lib.rs",
    ROOT / "verification/services/src/lib.rs",
]

# Runtime wrappers the raw log does not verify. The macros in the same files are
# still required to have specified func-details entries. This list is printed.
UNVERIFIED_WRAPPERS = {
    "src/kernel_lowlevel/kernel_budget_logic_shared.rs": {
        "admit_heap",
        "admit_process",
        "mmap_metadata_bytes",
        "uncharge",
    },
    "src/syscall/uaccess_logic_shared.rs": {"el0_user_range_plausible"},
}

EXTRA_SHARED = [
    "src/syscall/syscall_bridge_shared.rs",
    "src/syscall/linux_runtime_lock_shared.rs",
]


def fail(message):
    print(message, file=sys.stderr)
    return 1


def verus_spans(text):
    spans = []
    cursor = 0
    while True:
        start = text.find("verus!", cursor)
        if start < 0:
            break
        brace = text.find("{", start)
        if brace < 0:
            break
        depth = 0
        for index in range(brace, len(text)):
            char = text[index]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    spans.append((brace, index))
                    cursor = index + 1
                    break
        else:
            break
    return spans


def inside(pos, spans):
    return any(left <= pos <= right for left, right in spans)


def load_log(path):
    text = path.read_text()
    header, _, raw = text.partition("\n{")
    if not raw:
        raise ValueError(f"{path} has no JSON object")
    data, _ = json.JSONDecoder().raw_decode("{" + raw)
    return header, data


def function_headers(text):
    """Map function name to the signature text before its body brace."""
    headers = {}
    for match in re.finditer(r"(?:proof\s+)?fn\s+([A-Za-z0-9_]+)\b", text):
        name = match.group(1)
        window = text[match.start() : match.start() + 1200]
        brace = window.find("{")
        headers.setdefault(name, window[: brace if brace >= 0 else len(window)])
    return headers


def nonempty_spec(header):
    clauses = re.findall(
        r"\b(?:ensures|requires)\b(.*?)(?=\b(?:ensures|requires|decreases)\b|$)",
        header,
        re.S,
    )
    compact = re.sub(r"\s+", "", " ".join(clauses))
    if re.fullmatch(r"(?:true,?|out==out,?)*", compact or ""):
        return False
    atoms = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", " ".join(clauses))
    return any(atom != "true" for atom in atoms)


def git(*args):
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "git failed")
    return result.stdout


def main():
    status = 0
    doc = (ROOT / "docs/VERUS_COVERAGE.md").read_text()
    lines = doc.splitlines()
    proved = []
    for index, line in enumerate(lines):
        match = re.match(r"^- `(src/.*\.(?:rs|S))`$", line)
        if not match or index + 1 >= len(lines):
            continue
        if lines[index + 1].strip() == "- 已证明":
            proved.append(match.group(1))

    for needle in (
        "- `third_party/posixtest/**`",
        "- `target/posix/**`",
    ):
        pos = doc.find(needle)
        if pos < 0:
            status |= fail(f"missing upstream out-of-scope entry: {needle}")
            continue
        following = doc[pos:].splitlines()
        if len(following) < 2 or following[1].strip() != "- 超出范围":
            status |= fail(f"{needle} must be labeled 超出范围")
        if "Not kernel TCB" not in "\n".join(following[:4]):
            status |= fail(f"{needle} is missing the not-kernel-TCB reason")

    if "smros_ko_align_up_checked_body" not in doc or not re.search(
        r"smros_ko_align_up_checked_body`\n  - 待证明", doc
    ):
        status |= fail(
            "smros_ko_align_up_checked_body must be explicitly labeled 待证明"
        )

    harness_text = [(path, path.read_text()) for path in HARNESS_SOURCES]
    harness_spans = [(path, text, verus_spans(text)) for path, text in harness_text]
    includes = {}
    include_re = re.compile(r'include!\("(?:\.\./)+([^"]+)"\)')
    for path, text, spans in harness_spans:
        for match in include_re.finditer(text):
            rel = match.group(1)
            where = "inside" if inside(match.start(), spans) else "outside"
            includes.setdefault(rel, set()).add(where)
            print(f"include {where} verus!: {rel} ({path.relative_to(ROOT)})")

    shared = sorted((ROOT / "src").rglob("*_logic_shared.rs"))
    required = [str(path.relative_to(ROOT)) for path in shared] + EXTRA_SHARED
    for rel in required:
        if rel not in includes:
            status |= fail(f"missing Verus include for shared file: {rel}")

    for rel in proved:
        if "inside" not in includes.get(rel, set()):
            status |= fail(f"已证明 file is not included inside verus!: {rel}")

    headers = {}
    func_names = set()
    recorded_heads = set()
    for log in LOGS:
        if not log.is_file():
            status |= fail(f"missing Verus JSON log: {log.relative_to(ROOT)}")
            continue
        header, data = load_log(log)
        results = data.get("verification-results", {})
        if not results.get("success") or results.get("errors") != 0:
            status |= fail(f"{log.name} is not a successful Verus run: {results}")
        head_match = re.search(r"^git_head=([0-9a-f]{40})$", header, re.M)
        if not head_match:
            status |= fail(f"{log.name} has no git_head binding")
        else:
            recorded_heads.add(head_match.group(1))
        if "only verification/logs/" not in header and "only verification/logs" not in header:
            status |= fail(f"{log.name} does not state that a log-only commit publishes the run")
        for key in data.get("func-details", {}):
            func_names.add(key)
        for path, text in harness_text:
            headers.setdefault(path, function_headers(text))

    if len(recorded_heads) > 1:
        status |= fail(f"Verus logs disagree on git_head: {sorted(recorded_heads)}")
    if recorded_heads:
        recorded = next(iter(recorded_heads))
        current = git("rev-parse", "HEAD").strip()
        if recorded != current:
            diff = [
                line
                for line in git("diff", "--name-only", recorded, current).splitlines()
                if line
            ]
            if not diff or any(not line.startswith("verification/logs/") for line in diff):
                status |= fail(
                    "Verus logs are bound to "
                    f"{recorded}, but HEAD {current} changes more than verification/logs/: {diff}"
                )
        for log in LOGS:
            if not log.is_file():
                continue
            header, _ = load_log(log)
            for line in header.splitlines():
                if not re.match(r"^[0-9a-f]{64}  ", line):
                    continue
                digest, rel = line.split("  ", 1)
                target = ROOT / rel
                if not target.is_file():
                    status |= fail(f"logged source is missing: {rel}")
                    continue
                actual = hashlib.sha256(target.read_bytes()).hexdigest()
                if actual != digest:
                    status |= fail(f"source hash changed since the Verus log: {rel}")

    combined = "\n".join(text for _, text in harness_text)
    combined_spans = []
    offset = 0
    pieces = []
    for _, text, spans in harness_spans:
        pieces.append(text)
        combined_spans.extend((left + offset, right + offset) for left, right in spans)
        offset += len(text) + 1
    combined = "\n".join(pieces)
    all_headers = {}
    for mapping in headers.values():
        all_headers.update(mapping)

    for rel in proved:
        source = (ROOT / rel).read_text()
        functions = re.findall(
            r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?fn\s+([A-Za-z0-9_]+)",
            source,
            re.M,
        )
        methods = re.findall(
            r"^\s+(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?fn\s+([A-Za-z0-9_]+)",
            source,
            re.M,
        )
        macros = re.findall(r"^macro_rules!\s+([A-Za-z0-9_]+)", source, re.M)
        exempt = UNVERIFIED_WRAPPERS.get(rel, set())
        for name in functions + methods:
            if name in exempt:
                print(f"documented unverified runtime wrapper: {rel} {name}")
                continue
            qualified = f"lib::{name}"
            if qualified not in func_names or not nonempty_spec(all_headers.get(name, "")):
                status |= fail(
                    f"已证明 function has no specified func-details entry: {rel} {name}"
                )
        for macro in macros:
            needle = macro + "!"
            found = False
            start = 0
            while True:
                pos = combined.find(needle, start)
                if pos < 0:
                    break
                start = pos + len(needle)
                if not inside(pos, combined_spans):
                    continue
                previous = list(re.finditer(r"(?:proof\s+)?fn\s+([A-Za-z0-9_]+)\b", combined[:pos]))
                if not previous:
                    continue
                name = previous[-1].group(1)
                if not inside(previous[-1].start(), combined_spans):
                    continue
                if f"lib::{name}" in func_names and nonempty_spec(all_headers.get(name, "")):
                    found = True
                    break
            if not found:
                status |= fail(
                    f"已证明 macro has no specified func-details function: {rel} {macro}"
                )

    if status == 0:
        print(
            f"Verus proof audit passed for {len(proved)} 已证明 files "
            f"against {len(func_names)} func-details names."
        )
    return status


if __name__ == "__main__":
    sys.exit(main())
