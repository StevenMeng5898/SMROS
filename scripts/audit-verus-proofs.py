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


def split_top(expr, op):
    parts = []
    depth = 0
    start = 0
    i = 0
    while i < len(expr):
        char = expr[i]
        if char == "(":
            depth += 1
        elif char == ")":
            depth = max(0, depth - 1)
        elif depth == 0 and expr.startswith(op, i):
            parts.append(expr[start:i])
            i += len(op)
            start = i
            continue
        i += 1
    parts.append(expr[start:])
    return parts


def is_tautology(expr):
    """Syntactic patterns only.

    Catches true, x == x, x ==> x, x ==> true, x <= x, x >= x, and
    x || !x in either order. A disjunction is tautological when any
    disjunct is, and a conjunction when every conjunct is. This does
    not decide general arithmetic or boolean equivalence.
    """
    expr = re.sub(r"[\s,{}]+", " ", expr).strip()
    if expr in {"", "true"}:
        return True
    ors = split_top(expr, "||")
    if len(ors) > 1:
        if len(ors) == 2 and complementary(ors[0], ors[1]):
            return True
        return any(is_tautology(part) for part in ors)
    ands = split_top(expr, "&&")
    if len(ands) > 1:
        return all(is_tautology(part) for part in ands)
    compact = re.sub(r"\s+", "", expr)
    if re.fullmatch(r"([A-Za-z_][A-Za-z0-9_]*)(?:==|==>|<=|>=)\1", compact):
        return True
    return re.fullmatch(r"([A-Za-z_][A-Za-z0-9_]*)==>true", compact) is not None


def complementary(left, right):
    a = re.sub(r"\s+", "", left)
    b = re.sub(r"\s+", "", right)
    return (a.startswith("!") and a[1:] == b) or (b.startswith("!") and b[1:] == a)


def ensures_clauses(header):
    return re.findall(
        r"\bensures\b(.*?)(?=\b(?:ensures|requires|decreases)\b|$)",
        header,
        re.S,
    )


def nonempty_spec(header):
    clauses = ensures_clauses(header)
    if not clauses:
        return False
    if all(is_tautology(clause) for clause in clauses):
        return False
    atoms = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", " ".join(clauses))
    return any(atom != "true" for atom in atoms)


def constrains_result(header):
    """True when ensures mentions the named result other than is_some/is_none/is_ok/is_err."""
    named = re.search(r"->\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*:", header)
    if not named:
        return True
    clauses = ensures_clauses(header)
    if not clauses:
        return False
    name = named.group(1)
    stripped = re.sub(
        rf"\b{name}\s*\.\s*(?:is_some|is_none|is_ok|is_err)\s*\(\s*\)",
        " ",
        " ".join(clauses),
    )
    return re.search(rf"\b{name}\b", stripped) is not None


def useful_spec(header):
    return nonempty_spec(header) and constrains_result(header)


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

    recorded_heads = set()
    details_by_harness = {}
    breakdown = {}
    log_headers = {}
    for log in LOGS:
        if not log.is_file():
            status |= fail(f"missing Verus JSON log: {log.relative_to(ROOT)}")
            continue
        header, data = load_log(log)
        log_headers[log.name] = header
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
        details_by_harness[log.name] = set(data.get("func-details", {}))
        def walk(node):
            if isinstance(node, dict):
                rows = node.get("function-breakdown")
                if isinstance(rows, list):
                    for row in rows:
                        if isinstance(row, dict) and "function" in row:
                            breakdown[(log.name, row["function"])] = (
                                row.get("mode:", ""),
                                row.get("success"),
                            )
                for value in node.values():
                    walk(value)
            elif isinstance(node, list):
                for value in node:
                    walk(value)
        walk(data)

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

    harness_by_name = {}
    for (path, text, spans), log in zip(harness_spans, LOGS):
        harness_by_name[log.name] = (path, text, spans)

    evidence = []
    for rel in proved:
        source = (ROOT / rel).read_text()
        origin_headers = function_headers(source)
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
        owners = [
            log.name
            for log in LOGS
            if "inside" in includes.get(rel, set())
            and rel in {
                match.group(1)
                for match in include_re.finditer(harness_by_name[log.name][1])
                if inside(match.start(), harness_by_name[log.name][2])
            }
        ]
        if not owners:
            status |= fail(f"已证明 file has no inside-verus harness: {rel}")
            continue
        exempt = UNVERIFIED_WRAPPERS.get(rel, set())
        for name in functions + methods:
            if name in exempt:
                print(f"documented unverified runtime wrapper: {rel} {name}")
                continue
            qualified = f"lib::{name}"
            header = origin_headers.get(name, "")
            present = [owner for owner in owners if qualified in details_by_harness.get(owner, set())]
            if not present or not useful_spec(header):
                status |= fail(
                    f"已证明 function has no specified func-details entry in its harness: {rel} {qualified}"
                )
            for owner in present:
                mode, ok = breakdown.get((owner, qualified), ("", None))
                evidence.append((owner, qualified, mode, ok, rel))
        for macro in macros:
            needle = macro + "!"
            found = False
            rejected = False
            for owner in owners:
                _path, text, spans = harness_by_name[owner]
                headers = function_headers(text)
                start = 0
                while True:
                    pos = text.find(needle, start)
                    if pos < 0:
                        break
                    start = pos + len(needle)
                    if not inside(pos, spans):
                        continue
                    previous = list(re.finditer(r"(?:proof\s+)?fn\s+([A-Za-z0-9_]+)\b", text[:pos]))
                    if not previous or not inside(previous[-1].start(), spans):
                        continue
                    name = previous[-1].group(1)
                    qualified = f"lib::{name}"
                    header = headers.get(name, "")
                    if qualified not in details_by_harness.get(owner, set()) or not nonempty_spec(header):
                        continue
                    if not constrains_result(header):
                        status |= fail(
                            f"已证明 spec does not constrain the result value: {rel} {qualified}"
                        )
                        rejected = True
                        break
                    found = True
                    mode, ok = breakdown.get((owner, qualified), ("", None))
                    evidence.append((owner, qualified, mode, ok, f"{rel}::{macro}"))
                    break
                if found or rejected:
                    break
            if not found and not rejected:
                status |= fail(
                    f"已证明 macro has no specified func-details function in its harness: {rel} {macro}"
                )

    if evidence and status == 0:
        lines = ["===== verus function evidence ====="]
        if recorded_heads:
            lines.append(f"git_head={next(iter(recorded_heads))}")
        dirty = git("status", "--porcelain", "--untracked-files=no", "--", ".", ":(exclude)verification/logs")
        lines.append("git_dirty=" + ("true" if dirty.strip() else "false"))
        lines.append("dirty_files<<END")
        lines.append(dirty.rstrip())
        lines.append("END")
        lines.append("source_sha256:")
        for rel in proved:
            digest = hashlib.sha256((ROOT / rel).read_bytes()).hexdigest()
            lines.append(f"{digest}  {rel}")
        lines.append("harness_sha256:")
        for path in HARNESS_SOURCES:
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            lines.append(f"{digest}  {path.relative_to(ROOT)}")
        lines.append("harness function mode success origin")
        for row in evidence:
            lines.append("\t".join(str(part) for part in row))
        (ROOT / "verification/logs/verus-functions.txt").write_text("\n".join(lines) + "\n")

    if status == 0:
        print(
            f"Verus proof audit passed for {len(proved)} 已证明 files "
            f"with {len(evidence)} harness function rows."
        )
    return status


def self_test():
    """The two auditor counterexamples must be rejected."""
    failures = 0
    tautologies = [
        "fn checked_end() -> (out: bool)\n    ensures\n        out == out || armed,\n{",
        "fn checked_end() -> (out: bool)\n    ensures\n        x == x,\n{",
        "fn checked_end() -> (out: bool)\n    ensures\n        out ==> out,\n{",
        "fn checked_end() -> (out: bool)\n    ensures\n        !out || out,\n{",
        "fn checked_end() -> (out: bool)\n    ensures\n        out || !out,\n{",
        "fn checked_end() -> (out: bool)\n    ensures\n        out ==> true,\n{",
        "fn checked_end() -> (out: usize)\n    ensures\n        deadline <= deadline,\n{",
        "fn checked_end() -> (out: Option<usize>)\n    ensures\n        out.is_some(),\n{",
    ]
    for header in tautologies:
        if useful_spec(header):
            print("counterexample tautology was accepted", file=sys.stderr)
            failures += 1
        else:
            print("counterexample tautology: audit exit 1")
    # A kernel_objects 已证明 file defining an unspecified checked_end must not
    # borrow syscall's lib::checked_end.
    origin = "fn checked_end(addr: usize, len: usize) -> (out: Option<usize>)\n{\n"
    syscall_has = {"lib::checked_end"}
    kernel_objects_has = set()
    owners = ["verus-kernel-objects.log"]
    qualified = "lib::checked_end"
    present = [owner for owner in owners if qualified in {"verus-syscall.log": syscall_has, "verus-kernel-objects.log": kernel_objects_has}[owner]]
    accepted = bool(present) and useful_spec(origin)
    if accepted:
        print("counterexample harness-collision was accepted", file=sys.stderr)
        failures += 1
    else:
        print("counterexample harness-collision: audit exit 1")
    return 1 if failures else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    sys.exit(main())
