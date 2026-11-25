#!/usr/bin/env python3
"""Semantic gate for functions labeled 已证明.

The pattern scan in audit-verus-proofs.py misses specs that are true for every
return value, such as `out ==> deadline < deadline + 1` and `!(out && !out)`.
This script mutates each proved harness body and requires Verus to reject at
least one mutation. A spec that survives every mutation is vacuous.

Results are cached in verification/logs/verus-mutation-summary.txt by the hash
of the harness function text. That cache is gitignored. `--harness NAME`
limits the run to one log. Functions that cannot be mutated automatically
need a justified line in verification/verus-mutation-signoff.txt.

The function table is verification/logs/verus-functions.txt, which
audit-verus-proofs.py rewrites and which is also gitignored. If that file
is absent, the published table under verification/evidence/ is used only
when every source hash it records still matches this tree. This script
never writes verification/evidence/.
"""

import hashlib
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SUMMARY = ROOT / "verification/logs/verus-mutation-summary.txt"
FUNCTIONS = ROOT / "verification/logs/verus-functions.txt"
PUBLISHED_FUNCTIONS = (
    ROOT
    / "verification/evidence/751b308560119ceb3de3571cfaf90787cd8de44d/verus-functions.txt"
)
SIGNOFF = ROOT / "verification/verus-mutation-signoff.txt"
HARNESS = {
    "verus-syscall.log": ROOT / "verification/syscall/src/lib.rs",
    "verus-kernel-objects.log": ROOT / "verification/kernel_objects/src/lib.rs",
    "verus-kernel-lowlevel.log": ROOT / "verification/kernel_lowlevel/src/lib.rs",
    "verus-user-level.log": ROOT / "verification/user_level/src/lib.rs",
    "verus-services.log": ROOT / "verification/services/src/lib.rs",
}
INTEGERS = {"u8", "u16", "u32", "u64", "u128", "usize", "i32", "i64", "isize"}
MAX_LITERAL = {
    "u8": "0xff",
    "u16": "0xffff",
    "u32": "0xffff_ffff",
    "u64": "0xffff_ffff_ffff_ffff",
    "u128": "0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff",
    "usize": "usize::MAX",
    "i32": "i32::MAX",
    "i64": "i64::MAX",
    "isize": "isize::MAX",
}


def fail(message):
    print(message, file=sys.stderr)
    return 1


def toolchain():
    env_path = ROOT / "verification/verus-toolchain.env"
    name = "1.95.0-x86_64-unknown-linux-gnu"
    if env_path.is_file():
        for line in env_path.read_text().splitlines():
            if line.startswith("VERUS_RUST_TOOLCHAIN="):
                name = line.split("=", 1)[1].strip()
    return name


def verus_bin():
    candidate = ROOT / ".tools/verus/current/verus"
    if candidate.is_file():
        return candidate
    setup = ROOT / "scripts/setup-verus.sh"
    subprocess.run([str(setup)], cwd=ROOT, check=True)
    if not candidate.is_file():
        raise RuntimeError("verus binary missing after setup")
    return candidate


def git(*args):
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return ""
    return result.stdout.strip()


def split_commas(text):
    parts = []
    depth = 0
    start = 0
    for index, char in enumerate(text):
        if char in "<(":
            depth += 1
        elif char in ">)":
            depth = max(0, depth - 1)
        elif char == "," and depth == 0:
            parts.append(text[start:index].strip())
            start = index + 1
    tail = text[start:].strip()
    if tail:
        parts.append(tail)
    return parts


def zero_expr(rtype):
    rtype = rtype.strip()
    if rtype == "bool":
        return "false"
    if rtype in INTEGERS:
        return "0"
    if rtype.startswith("Option<") and rtype.endswith(">"):
        return "None"
    if rtype.startswith("(") and rtype.endswith(")"):
        inner = ", ".join(zero_expr(part) for part in split_commas(rtype[1:-1]))
        return f"({inner})"
    return None


def alt_expr(rtype, kind):
    """A value other than zero. kind is 'one' or 'max'."""
    rtype = rtype.strip()
    if rtype == "bool":
        return "true"
    if rtype in INTEGERS:
        if kind == "max":
            return MAX_LITERAL[rtype]
        return "1"
    if rtype.startswith("Option<") and rtype.endswith(">"):
        inner = rtype[len("Option<") : -1]
        payload = alt_expr(inner, kind)
        if payload is None:
            return None
        return f"Some({payload})"
    if rtype.startswith("(") and rtype.endswith(")"):
        rendered = []
        for part in split_commas(rtype[1:-1]):
            item = alt_expr(part, kind)
            if item is None:
                return None
            rendered.append(item)
        return "(" + ", ".join(rendered) + ")"
    return None


BLOCK_OPENERS = {"match", "if", "while", "for", "loop", "unsafe"}
RLIMIT = "10"


def mutants_for(rtype, body):
    """Candidate bodies. Bool and Option need both sides killed."""
    rtype = rtype.strip()
    zero = zero_expr(rtype)
    one = alt_expr(rtype, "one")
    high = alt_expr(rtype, "max")
    mutants = []
    if zero is not None:
        mutants.append(("return-zero", zero))
    if one is not None and one != zero:
        mutants.append(("return-one", one))
    if high is not None and high not in {zero, one}:
        mutants.append(("return-max", high))
    if rtype in INTEGERS:
        # zero and max are both required. Drop the extra wrapping mutant.
        return [item for item in mutants if item[0] in {"return-zero", "return-max"}]
    if rtype.startswith("(") and rtype.endswith(")"):
        # Flip one component and keep the others from the real result.
        # Zeroing the other fields kills a spec that only constrains one field.
        parts = split_commas(rtype[1:-1])
        component_mutants = []
        block = "{" + body.strip() + "}"
        for index, part in enumerate(parts):
            part = part.strip()
            if part == "bool":
                flips = [("not", f"!__o.{index}")]
            else:
                flips = [
                    ("zero", zero_expr(part) or "0"),
                    ("max", alt_expr(part, "max") or alt_expr(part, "one") or "1"),
                ]
            for suffix, flipped in flips:
                fields = []
                for other_index, _other in enumerate(parts):
                    if other_index == index:
                        fields.append(flipped)
                    else:
                        fields.append(f"__o.{other_index}")
                expr = "let __o = " + block + "; (" + ", ".join(fields) + ")"
                component_mutants.append((f"component-{index}-{suffix}", expr))
        return component_mutants
    return mutants


def skip_trivia(text, index):
    while index < len(text):
        if text.startswith("//", index):
            newline = text.find("\n", index)
            if newline < 0:
                return len(text)
            index = newline + 1
            continue
        if text.startswith("/*", index):
            end = text.find("*/", index + 2)
            if end < 0:
                return len(text)
            index = end + 2
            continue
        if text[index] == '"':
            index += 1
            while index < len(text) and text[index] != '"':
                index += 2 if text[index] == "\\" else 1
            index += 1
            continue
        return index
    return index


def read_word(text, index):
    if index >= len(text) or not (text[index].isalpha() or text[index] == "_"):
        return None, index
    end = index + 1
    while end < len(text) and (text[end].isalnum() or text[end] == "_"):
        end += 1
    return text[index:end], end


def body_open_index(text, index):
    """Index of the function-body brace after the signature and spec.

    A `{` that opens `match`/`if`/loop in an ensures clause is not the body.
    """
    paren = brace = bracket = 0
    seen_arrow = False
    signature_done = False
    pending = None
    while index < len(text):
        index = skip_trivia(text, index)
        if index >= len(text):
            break
        if not signature_done and text.startswith("->", index) and paren == 0 and brace == 0:
            seen_arrow = True
            index += 2
            continue
        word, next_index = read_word(text, index)
        if word is not None:
            if (
                signature_done
                and word in BLOCK_OPENERS
                and brace == 0
                and paren == 0
                and bracket == 0
            ):
                pending = word
            index = next_index
            continue
        char = text[index]
        if char == "{":
            if brace == 0 and paren == 0 and bracket == 0 and pending:
                pending = None
                brace = 1
            elif signature_done and brace == 0 and paren == 0 and bracket == 0:
                return index
            else:
                brace += 1
        elif char == "}":
            brace = max(0, brace - 1)
        elif char == "(":
            paren += 1
        elif char == ")":
            paren = max(0, paren - 1)
            if paren == 0 and brace == 0 and seen_arrow:
                signature_done = True
        elif char == "[":
            bracket += 1
        elif char == "]":
            bracket = max(0, bracket - 1)
        index += 1
    return None


def matching_brace(text, body):
    depth = 0
    index = body
    while index < len(text):
        index = skip_trivia(text, index)
        if index >= len(text):
            return None
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index
        index += 1
    return None


def function_span(text, name):
    pattern = re.compile(rf"(?m)^(?:pub(?:\([^)]*\))?\s+)?fn {re.escape(name)}\b")
    found = list(pattern.finditer(text))
    if len(found) != 1:
        return None
    start = found[0].start()
    body = body_open_index(text, found[0].end())
    if body is None:
        return None
    close = matching_brace(text, body)
    if close is None:
        return None
    return start, body, close


def return_type(signature):
    arrow = signature.rfind("->")
    if arrow < 0:
        return None
    rest = signature[arrow + 2 :].lstrip()
    if not rest.startswith("("):
        return rest.split()[0]
    depth = 0
    for index, char in enumerate(rest):
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                inner = rest[1:index]
                colon = inner.find(":")
                if colon < 0:
                    return None
                return inner[colon + 1 :].strip()
    return None


def apply_mutant(text, body_open, body_close, expr):
    return text[: body_open + 1] + "\n    " + expr + "\n" + text[body_close:]


def classify(code, output):
    """Only a postcondition or assertion failure is a kill.

    Syntax errors, timeouts, a missing toolchain, rlimit, and panics are
    invalid. The gate fails on invalid; it does not count them as killed.
    """
    if code == 0 and re.search(r"\b0 errors\b", output):
        return "survived"
    if code != 0 and (
        "postcondition not satisfied" in output or "assertion failed" in output
    ):
        return "killed"
    return "invalid"


def gate_exit(kind):
    """Killed is the only passing mutant result. Everything else fails the gate."""
    return 0 if kind == "killed" else 1


def signed_off_fails_gate(signed_count):
    """A signed function is 待证明. Any sign-off fails the gate."""
    return signed_count > 0


def run_verus(bin_path, source, function, env, timeout):
    command = [
        str(bin_path),
        str(source),
        "--crate-type=lib",
        "--triggers-mode=silent",
        "--no-report-long-running",
        "--verify-root",
        "--verify-function",
        function,
        "--rlimit",
        "10",
        "--multiple-errors",
        "0",
    ]
    with tempfile.TemporaryDirectory(prefix="smros-verus-run-") as tmp:
        try:
            result = subprocess.run(
                command,
                cwd=tmp,
                env=env,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired as exc:
            out = _timeout_text(exc) + "\nerror: mutation run timed out\n"
            return 1, out
        except OSError as exc:
            return 1, f"error: toolchain missing: {exc}\n"
    return result.returncode, result.stdout + result.stderr


def _timeout_text(exc):
    """TimeoutExpired.stdout is bytes even when text=True was requested."""
    parts = []
    for value in (exc.stdout, exc.stderr):
        if value is None:
            continue
        if isinstance(value, bytes):
            parts.append(value.decode("utf-8", "replace"))
        else:
            parts.append(str(value))
    return "".join(parts)


def included_shared_blobs(harness_path):
    text = harness_path.read_text()
    rels = re.findall(r'include!\("(?:\.\./)+([^"]+)"\)', text)
    blobs = []
    for rel in sorted(set(rels)):
        path = ROOT / rel
        blobs.append(path.read_bytes() if path.is_file() else b"missing:" + rel.encode())
    return blobs


def verus_env():
    env = os.environ.copy()
    env["PATH"] = str(ROOT / ".tools/verus/current") + os.pathsep + env.get("PATH", "")
    env["RUSTUP_TOOLCHAIN"] = toolchain()
    return env


def _recorded_hashes_match(path):
    """True when every `sha256  relative/path` line in path matches this tree."""
    if not path.is_file():
        return False
    saw = False
    for line in path.read_text().splitlines():
        if len(line) < 66 or line[64:66] != "  ":
            continue
        digest, rel = line.split("  ", 1)
        if len(digest) != 64 or any(char not in "0123456789abcdef" for char in digest):
            continue
        target = ROOT / rel
        if not target.is_file():
            return False
        if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
            return False
        saw = True
    return saw


def function_table_path():
    """Local table first. The published snapshot is a read-only fallback."""
    if FUNCTIONS.is_file():
        return FUNCTIONS
    if _recorded_hashes_match(PUBLISHED_FUNCTIONS):
        return PUBLISHED_FUNCTIONS
    raise RuntimeError(
        "verus-functions.txt is missing. make verus writes "
        "verification/logs/verus-functions.txt, which is gitignored. "
        "The published snapshot is used only when its source hashes still match."
    )


def load_functions():
    rows = []
    lines = function_table_path().read_text().splitlines()
    try:
        start = lines.index("harness function mode success origin")
    except ValueError as exc:
        raise RuntimeError("verus-functions.txt has no function table") from exc
    for line in lines[start + 1 :]:
        if not line.strip():
            continue
        harness, qualified, mode, success, origin = line.split("\t")
        if success not in {"True", "true"}:
            continue
        name = qualified.split("::", 1)[1]
        rows.append(
            {
                "harness": harness,
                "qualified": qualified,
                "name": name,
                "mode": mode,
                "origin": origin,
            }
        )
    return rows


def documented_signoffs():
    text = (ROOT / "docs/VERUS_COVERAGE.md").read_text()
    marker = "## 签署（待证明）"
    start = text.find(marker)
    if start < 0:
        return None
    rest = text[start + len(marker) :]
    nxt = rest.find("\n## ")
    section = rest if nxt < 0 else rest[:nxt]
    return re.findall(r"`(lib::[A-Za-z0-9_]+)`", section)


def load_signoff():
    signed = {}
    if not SIGNOFF.is_file():
        return signed
    for line in SIGNOFF.read_text().splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        qualified, separator, reason = stripped.partition("\t")
        if separator and not reason.strip():
            qualified, separator, reason = stripped.partition(" ")
        signed[qualified.strip()] = reason.strip()
    return signed


def load_cache():
    cached = {}
    if not SUMMARY.is_file():
        return cached
    for line in SUMMARY.read_text().splitlines():
        parts = line.split("\t")
        if len(parts) != 6 or not parts[0].startswith("lib::"):
            continue
        qualified, harness, mutation, body_sha, cache_sha, result = parts
        cached[(harness, qualified)] = {
            "mutation": mutation,
            "digest": body_sha,
            "cache": cache_sha,
            "result": result,
        }
    return cached


def tool_fingerprint(bin_path):
    try:
        result = subprocess.run(
            [str(bin_path), "--version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
        text = (result.stdout + result.stderr).strip()
    except (OSError, subprocess.TimeoutExpired) as exc:
        text = f"missing:{exc}"
    return f"{text}\nrlimit={RLIMIT}".encode()


def cache_digest(parts):
    digest = hashlib.sha256()
    for part in parts:
        if isinstance(part, str):
            part = part.encode()
        digest.update(part)
        digest.update(b"\0")
    return digest.hexdigest()


def function_cache_key(row, function_text, bin_path):
    harness_path = HARNESS[row["harness"]]
    harness = harness_path.read_bytes()
    shared = included_shared_blobs(harness_path)
    return cache_digest(
        [
            harness,
            *shared,
            tool_fingerprint(bin_path),
            f"rlimit={RLIMIT}",
            "tuple-mutant=keep-other-components-zero-and-max",
            function_text,
        ]
    )


def harness_filter(argv):
    if "--harness" not in argv:
        return None
    value = argv[argv.index("--harness") + 1]
    for name, path in HARNESS.items():
        if value in {name, name.removesuffix(".log"), path.name, str(path)}:
            return name
    raise SystemExit(f"unknown harness {value}")


def write_summary(rows):
    killed = sum(1 for row in rows if row["result"] == "killed")
    signed = sum(1 for row in rows if row["result"] == "signed-off")
    vacuous = sum(1 for row in rows if row["result"] == "vacuous")
    proved = len(rows) - signed
    dirty = git(
        "status",
        "--porcelain",
        "--untracked-files=no",
        "--",
        ".",
        ":(exclude)verification/logs",
    )
    lines = [
        "===== verus mutation summary =====",
        "utc=" + subprocess.check_output(["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], text=True).strip(),
        "git_head=" + (git("rev-parse", "HEAD") or "unknown"),
        "git_dirty=" + ("true" if dirty else "false"),
        f"functions={proved}",
        f"killed={killed}",
        f"signed_off={signed}",
        f"vacuous={vacuous}",
        "body_sha256 covers the whole fn, including ensures. cache_sha256 covers the harness, every *_shared.rs it includes, and the Verus version plus rlimit.",
        "A cache hit still re-runs the unmutated baseline. Changing any included shared file, including futex, changes the key.",
        "Full prover output is not stored.",
        "function\tharness\tmutation\tbody_sha256\tcache_sha256\tresult",
    ]
    for row in rows:
        lines.append(
            "\t".join(
                [
                    row["qualified"],
                    row["harness"],
                    row["mutation"],
                    row["digest"],
                    row.get("cache", ""),
                    row["result"],
                ]
            )
        )
    SUMMARY.parent.mkdir(parents=True, exist_ok=True)
    SUMMARY.write_text("\n".join(lines) + "\n")


def run_mutant(path, original, body_open, body_close, expr, name, bin_path, env):
    path.write_text(apply_mutant(original, body_open, body_close, expr))
    code, output = run_verus(bin_path, path, name, env, timeout=90)
    return classify(code, output)


def prove_function(row, bin_path, env):
    path = HARNESS[row["harness"]]
    original = path.read_text()
    span = function_span(original, row["name"])
    if span is None:
        return {
            "qualified": row["qualified"],
            "harness": row["harness"],
            "mutation": "missing-function",
            "digest": "",
            "cache": "",
            "result": "invalid",
            "detail": "harness function not found",
        }
    start, body_open, body_close = span
    function_text = original[start : body_close + 1]
    # The body hash includes ensures. A `match out {` inside ensures must not
    # truncate the function.
    if "match out {" in original[start:body_open] and body_close <= body_open:
        return {
            "qualified": row["qualified"],
            "harness": row["harness"],
            "mutation": "span",
            "digest": "",
            "cache": "",
            "result": "invalid",
            "detail": "function body was not located after ensures",
        }
    digest = hashlib.sha256(function_text.encode()).hexdigest()
    cache = function_cache_key(row, function_text, bin_path)
    rtype = return_type(original[start:body_open])
    body = original[body_open + 1 : body_close]
    if rtype is None:
        return {
            "qualified": row["qualified"],
            "harness": row["harness"],
            "mutation": "missing-type",
            "digest": digest,
            "cache": cache,
            "result": "invalid",
            "detail": "return type not parsed",
        }
    kind_name = re.sub(r"\s+", "", rtype)
    try:
        baseline_code, baseline_out = run_verus(bin_path, path, row["name"], env, timeout=90)
        if classify(baseline_code, baseline_out) != "survived":
            return {
                "qualified": row["qualified"],
                "harness": row["harness"],
                "mutation": "baseline",
                "digest": digest,
                "cache": cache,
                "result": "invalid",
                "detail": "unmutated baseline did not verify",
            }
        kinds = []
        for label, expr in mutants_for(rtype, body):
            kind = run_mutant(
                path, original, body_open, body_close, expr, row["name"], bin_path, env
            )
            kinds.append((label, kind))
            if kind == "invalid":
                return {
                    "qualified": row["qualified"],
                    "harness": row["harness"],
                    "mutation": label,
                    "digest": digest,
                    "cache": cache,
                    "result": "invalid",
                    "detail": f"{label} was not a postcondition failure",
                }
        result, mutation, detail = judge_mutants(kind_name, kinds)
        return {
            "qualified": row["qualified"],
            "harness": row["harness"],
            "mutation": mutation,
            "digest": digest,
            "cache": cache,
            "result": result,
            "detail": detail,
        }
    finally:
        path.write_text(original)


def judge_mutants(rtype, kinds):
    """Bool needs both constants killed. Option needs None and some Some killed."""
    by_label = {label: kind for label, kind in kinds}
    if rtype == "bool":
        if by_label.get("return-zero") == "killed" and by_label.get("return-one") == "killed":
            return "killed", "return-false+return-true", "both-bool"
        return "one-sided", "bool-one-sided", "true and false were not both killed"
    if rtype.startswith("Option<"):
        some_killed = [
            label
            for label, kind in kinds
            if label != "return-zero" and kind == "killed"
        ]
        if by_label.get("return-zero") == "killed" and some_killed:
            return "killed", "none+" + "+".join(some_killed), "option-both"
        return "one-sided", "option-one-sided", "Some and None were not both killed"
    if rtype.startswith("(") and rtype.endswith(")"):
        component_labels = [label for label, _kind in kinds if label.startswith("component-")]
        if component_labels and all(by_label.get(label) == "killed" for label in component_labels):
            return "killed", "+".join(component_labels), "tuple-components"
        return "one-sided", "tuple-one-sided", "a tuple component mutant survived"
    if rtype in INTEGERS:
        if by_label.get("return-zero") == "killed" and by_label.get("return-max") == "killed":
            return "killed", "return-zero+return-max", "int-both"
        return "one-sided", "int-one-sided", "zero and max were not both killed"
    killed = [label for label, kind in kinds if kind == "killed"]
    if killed:
        return "killed", killed[0], killed[0]
    return "one-sided", "none", "no postcondition failure"


def audit(argv):
    selected = harness_filter(argv)
    rows = load_functions()
    if selected:
        rows = [row for row in rows if row["harness"] == selected]
    if not rows:
        return fail("no 已证明 functions selected")
    signoff = load_signoff()
    cache = load_cache()
    status = 0
    for qualified, reason in signoff.items():
        if not reason:
            status |= fail(f"sign-off is missing a justification: {qualified}")
    known = {row["qualified"] for row in load_functions()}
    documented = documented_signoffs()
    if documented is None:
        status |= fail("docs/VERUS_COVERAGE.md is missing the 签署（待证明） section")
        documented = []
    for qualified in signoff:
        if qualified not in known:
            status |= fail(f"sign-off names a function that is not 已证明: {qualified}")
        if qualified not in documented:
            status |= fail(f"sign-off is not listed as 待证明 in VERUS_COVERAGE.md: {qualified}")
    for qualified in documented:
        if qualified not in signoff:
            status |= fail(f"VERUS_COVERAGE.md lists a sign-off that is not in the file: {qualified}")
    bin_path = None
    env = None
    finished = []
    for row in rows:
        key = (row["harness"], row["qualified"])
        path = HARNESS[row["harness"]]
        original = path.read_text()
        span = function_span(original, row["name"])
        digest = ""
        if span is not None:
            start, _open, close = span
            digest = hashlib.sha256(original[start : close + 1].encode()).hexdigest()
        signed = signoff.get(row["qualified"], "")
        cached = cache.get(key)
        if signed:
            if bin_path is None:
                bin_path = verus_bin()
                env = verus_env()
            base_code, base_out = run_verus(bin_path, path, row["name"], env, timeout=90)
            if classify(base_code, base_out) != "survived":
                status |= fail(f"signed-off baseline did not verify: {row['qualified']}")
            finished.append(
                {
                    "qualified": row["qualified"],
                    "harness": row["harness"],
                    "mutation": "sign-off",
                    "digest": digest,
                    "cache": "",
                    "result": "signed-off",
                }
            )
            print(f"待证明 sign-off {row['qualified']}")
            continue
        if bin_path is None:
            bin_path = verus_bin()
            env = verus_env()
        function_text = ""
        if span is not None:
            function_text = original[span[0] : span[2] + 1]
        cache_sha = function_cache_key(row, function_text, bin_path) if function_text else ""
        if (
            cached
            and cached.get("cache") == cache_sha
            and cached["result"] == "killed"
            and cache_sha
        ):
            base_code, base_out = run_verus(bin_path, path, row["name"], env, timeout=90)
            if classify(base_code, base_out) != "survived":
                status |= fail(f"cached function baseline did not verify: {row['qualified']}")
                cached_result = "invalid"
            else:
                cached_result = "killed"
            finished.append(
                {
                    "qualified": row["qualified"],
                    "harness": row["harness"],
                    "mutation": cached["mutation"],
                    "digest": digest,
                    "cache": cache_sha,
                    "result": cached_result,
                }
            )
            print(f"cached {row['qualified']} mutation={cached['mutation']} baseline={cached_result}")
            continue
        proved = prove_function(row, bin_path, env)
        finished.append(
            {
                "qualified": proved["qualified"],
                "harness": proved["harness"],
                "mutation": proved["mutation"],
                "digest": proved["digest"],
                "cache": proved.get("cache", ""),
                "result": proved["result"],
            }
        )
        print(
            f"{proved['result']} {proved['qualified']} mutation={proved['mutation']} {proved['detail']}"
        )
        if proved["result"] != "killed":
            status |= fail(
                f"vacuous or unchecked spec, no killing mutation: {proved['qualified']} ({proved['detail']})"
            )
    if not selected:
        write_summary(finished)
    else:
        print("partial harness run does not rewrite the summary cache")
    killed = sum(1 for row in finished if row["result"] == "killed")
    signed_count = sum(1 for row in finished if row["result"] == "signed-off")
    # Signed functions are 待证明. They are not part of the proved count, and
    # a non-empty sign-off fails the gate instead of passing as status 0.
    proved = len(finished) - signed_count
    if signed_off_fails_gate(signed_count):
        status |= fail(
            f"signed-off functions are 待证明 and are excluded from the proved count: {signed_count}"
        )
    print(
        f"mutation gate functions={proved} killed={killed} signed_off={signed_count} status={status}"
    )
    return status


def self_test():
    """Span, classify, cache key, one-sided bool specs, and a match-ensures body."""
    failures = 0
    match_src = """
fn matched(n: u64) -> (out: Option<u64>)
    ensures
        match out {
            Some(value) => value == n,
            None => false,
        },
{
    Some(n)
}
"""
    span = function_span(match_src, "matched")
    if span is None:
        print("match-out span was not found", file=sys.stderr)
        failures += 1
    else:
        start, body, close = span
        ensures = match_src[start:body]
        body_text = match_src[body : close + 1]
        if "match out {" not in ensures or "Some(n)" not in body_text:
            print("match-out brace was treated as the function body", file=sys.stderr)
            failures += 1
        else:
            mutated = apply_mutant(match_src, body, close, "None")
            if "match out {" not in mutated or "\n    None\n" not in mutated:
                print("match-out mutation did not replace the body", file=sys.stderr)
                failures += 1
            else:
                print("match-out ensures kept; body mutation is after the spec")
    same = cache_digest([b"spec fn old", b"shared", b"verus", b"fn target"])
    changed = cache_digest([b"spec fn new", b"shared", b"verus", b"fn target"])
    if same == changed:
        print("spec-only edit did not change the cache key", file=sys.stderr)
        failures += 1
    else:
        print("spec fn edit changes cache_sha256 and must re-verify")
    futex_same = cache_digest([b"harness", b"fifo", b"futex-old", b"verus", b"fn target"])
    futex_new = cache_digest([b"harness", b"fifo", b"futex-new", b"verus", b"fn target"])
    if futex_same == futex_new:
        print("included futex shared file did not change the cache key", file=sys.stderr)
        failures += 1
    else:
        print("futex shared edit changes cache_sha256")
    int_one = judge_mutants("usize", [("return-zero", "survived"), ("return-max", "killed")])
    int_both = judge_mutants("usize", [("return-zero", "killed"), ("return-max", "killed")])
    if int_one[0] == "killed" or int_both[0] != "killed":
        print("integer zero/max rule is wrong", file=sys.stderr)
        failures += 1
    else:
        print("integer one-sided zero mutant: audit exit 1")
    tuple_one = judge_mutants(
        "(usize,usize)",
        [("component-0", "killed"), ("component-1", "survived")],
    )
    tuple_both = judge_mutants(
        "(usize,usize)",
        [("component-0", "killed"), ("component-1", "killed")],
    )
    if tuple_one[0] == "killed" or tuple_both[0] != "killed":
        print("tuple component rule is wrong", file=sys.stderr)
        failures += 1
    else:
        print("tuple one-sided component: audit exit 1")
    preserved = mutants_for("(usize, usize)", "\n    (7, x)\n")
    preserved_text = " ".join(expr for _label, expr in preserved)
    if (
        len(preserved) != 4
        or "let __o = {(7, x)}; (0, __o.1)" not in preserved_text
        or "let __o = {(7, x)}; (usize::MAX, __o.1)" not in preserved_text
        or "let __o = {(7, x)}; (__o.0, 0)" not in preserved_text
        or "let __o = {(7, x)}; (__o.0, usize::MAX)" not in preserved_text
        or "(usize::MAX, 0)" in preserved_text
    ):
        print(f"tuple mutant still zeroes other fields: {preserved}", file=sys.stderr)
        failures += 1
    else:
        print("tuple mutant keeps the other component")
    if not signed_off_fails_gate(1) or signed_off_fails_gate(0):
        print("signed-off still leaves the gate at status 0", file=sys.stderr)
        failures += 1
    else:
        print("signed-off function: audit exit 1")
    invalid_cases = [
        (1, "error: aborting due to 1 previous error\nexpected `{`, found `!`\n", "syntax-bad mutation"),
        (1, "error: mutation run timed out\n", "timeout"),
        (1, "error: verus: required rust toolchain 1.95 not found\n", "toolchain missing"),
        (1, "error: resource limit (rlimit) exceeded\n", "rlimit"),
        (1, "thread 'rustc' panicked at compiler\n", "panic"),
    ]
    for code, output, label in invalid_cases:
        kind = classify(code, output)
        if kind != "invalid" or gate_exit(kind) != 1:
            print(f"{label} was classified {kind}", file=sys.stderr)
            failures += 1
        else:
            print(f"{label}: audit exit 1")
    if classify(1, "error: postcondition not satisfied\n") != "killed":
        print("postcondition failure was not a kill", file=sys.stderr)
        failures += 1
    if classify(1, "error: assertion failed\n") != "killed":
        print("assertion failure was not a kill", file=sys.stderr)
        failures += 1

    one_false = judge_mutants("bool", [("return-zero", "survived"), ("return-one", "killed")])
    one_true = judge_mutants("bool", [("return-zero", "killed"), ("return-one", "survived")])
    both = judge_mutants("bool", [("return-zero", "killed"), ("return-one", "killed")])
    if one_false[0] == "killed" or one_true[0] == "killed" or both[0] != "killed":
        print("one-sided bool implication was accepted as killed", file=sys.stderr)
        failures += 1
    else:
        print("one-sided out ==> condition: audit exit 1")
        print("one-sided condition ==> out: audit exit 1")
    option_one = judge_mutants(
        "Option<u64>",
        [("return-zero", "killed"), ("return-one", "survived")],
    )
    option_both = judge_mutants(
        "Option<u64>",
        [("return-zero", "killed"), ("return-one", "killed")],
    )
    if option_one[0] == "killed" or option_both[0] != "killed":
        print("Option Some/None rule is wrong", file=sys.stderr)
        failures += 1

    bin_path = verus_bin()
    env = verus_env()
    source = r"""
use vstd::prelude::*;
verus! {
fn matched(n: u64) -> (out: Option<u64>)
    ensures
        match out {
            Some(value) => value == n,
            None => false,
        },
{
    Some(n)
}

fn implies_out(flag: bool) -> (out: bool)
    ensures
        out ==> flag,
{
    false
}

fn out_implies(flag: bool) -> (out: bool)
    ensures
        flag ==> out,
{
    true
}

fn tight(flag: bool) -> (out: bool)
    ensures
        out == flag,
{
    flag
}

fn pair(x: usize) -> (out: (usize, usize))
    ensures
        out.1 == x,
{
    (7, x)
}

fn bounded(x: usize) -> (out: (usize, usize))
    ensures
        out.0 <= 5,
        out.1 == x,
{
    (0, x)
}
}
"""
    with tempfile.TemporaryDirectory(prefix="smros-verus-mutation-") as tmp:
        path = Path(tmp) / "gate.rs"
        path.write_text(source)
        for function, expr, expect in (
            ("matched", None, "survived"),
            ("matched", "None", "killed"),
            ("implies_out", "false", "survived"),
            ("implies_out", "true", "killed"),
            ("out_implies", "true", "survived"),
            ("out_implies", "false", "killed"),
            ("tight", "false", "killed"),
            ("tight", "true", "killed"),
        ):
            text = source
            span = function_span(text, function)
            if span is None:
                print(f"self-test missing {function}", file=sys.stderr)
                failures += 1
                continue
            _start, body_open, body_close = span
            if expr is None:
                path.write_text(text)
            else:
                path.write_text(apply_mutant(text, body_open, body_close, expr))
            code, output = run_verus(bin_path, path, function, env, timeout=60)
            got = classify(code, output)
            if got != expect:
                print(
                    f"self-test {function} {expr}: expected {expect} got {got}",
                    file=sys.stderr,
                )
                failures += 1
            elif function == "matched" and expr == "None":
                print("match-out body mutation killed by postcondition")
        pair_span = function_span(source, "pair")
        pair_body = source[pair_span[1] + 1 : pair_span[2]]
        pair_results = []
        for label, expr in mutants_for("(usize, usize)", pair_body):
            path.write_text(apply_mutant(source, pair_span[1], pair_span[2], expr))
            code, output = run_verus(bin_path, path, "pair", env, timeout=60)
            pair_results.append((label, classify(code, output)))
        pair_judge = judge_mutants("(usize, usize)", pair_results)
        if pair_judge[0] != "one-sided" or gate_exit(1) != 1:
            print(f"one-sided tuple (7, x) judged {pair_judge} from {pair_results}", file=sys.stderr)
            failures += 1
        else:
            print("one-sided tuple out.1 == x: audit exit 1")
        bound_span = function_span(source, "bounded")
        bound_body = source[bound_span[1] + 1 : bound_span[2]]
        bound_results = []
        for label, expr in mutants_for("(usize, usize)", bound_body):
            path.write_text(apply_mutant(source, bound_span[1], bound_span[2], expr))
            code, output = run_verus(bin_path, path, "bounded", env, timeout=60)
            bound_results.append((label, classify(code, output)))
        bound_judge = judge_mutants("(usize, usize)", bound_results)
        if bound_judge[0] != "one-sided" or gate_exit(1) != 1:
            print(f"one-sided tuple (0, x) judged {bound_judge} from {bound_results}", file=sys.stderr)
            failures += 1
        else:
            print("one-sided tuple out.0 <= 5: audit exit 1")
        implies = judge_mutants(
            "bool",
            [("return-zero", "survived"), ("return-one", "killed")],
        )
        reverse = judge_mutants(
            "bool",
            [("return-zero", "killed"), ("return-one", "survived")],
        )
        if gate_exit(0 if implies[0] == "killed" else 1) != 1 or gate_exit(
            0 if reverse[0] == "killed" else 1
        ) != 1:
            print("one-sided implication gate did not exit 1", file=sys.stderr)
            failures += 1
        syntax = apply_mutant(source, *function_span(source, "tight")[1:], "!!!")
        path.write_text(syntax)
        code, output = run_verus(bin_path, path, "tight", env, timeout=60)
        if classify(code, output) != "invalid" or gate_exit("invalid") != 1:
            print("syntax-bad mutation was not rejected", file=sys.stderr)
            failures += 1
        else:
            print("syntax-bad mutation: audit exit 1")
        try:
            code, output = run_verus(
                bin_path,
                ROOT / "verification/services/src/lib.rs",
                "verified_notify_gate_allow_retry",
                env,
                timeout=0.05,
            )
        except Exception as exc:
            print(f"timeout raised {type(exc).__name__}: {exc}", file=sys.stderr)
            failures += 1
        else:
            if "Traceback" in output or classify(code, output) != "invalid" or gate_exit("invalid") != 1:
                print(f"real timeout was {classify(code, output)} not invalid", file=sys.stderr)
                failures += 1
            else:
                print("real timeout: audit exit 1")
        try:
            _timeout_text(
                subprocess.TimeoutExpired(cmd="verus", timeout=0.05, output=b"partial")
            )
        except TypeError as exc:
            print(f"timeout bytes join raised {exc}", file=sys.stderr)
            failures += 1
    if failures:
        print("mutation self-test failed", file=sys.stderr)
        return 1
    print("mutation self-test passed")
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    if "--self-test" not in sys.argv:
        probe = self_test()
        if probe != 0:
            sys.exit(probe)
    sys.exit(audit(sys.argv))
