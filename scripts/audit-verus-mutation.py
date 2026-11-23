#!/usr/bin/env python3
"""Semantic gate for functions labeled 已证明.

The pattern scan in audit-verus-proofs.py misses specs that are true for every
return value, such as `out ==> deadline < deadline + 1` and `!(out && !out)`.
This script mutates each proved harness body and requires Verus to reject at
least one mutation. A spec that survives every mutation is vacuous.

Results are cached in verification/logs/verus-mutation-summary.txt by the hash
of the harness function text. `--harness NAME` limits the run to one log.
Functions that cannot be mutated automatically need a justified line in
verification/verus-mutation-signoff.txt.
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


def mutants_for(rtype, body):
    """Ordered mutants. The first successful verification failure wins."""
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
        mutants.append(("wrapping-add-one", "({\n" + body + "\n}).wrapping_add(1)"))
    return mutants


def function_span(text, name):
    pattern = re.compile(rf"(?m)^(?:pub(?:\([^)]*\))?\s+)?fn {re.escape(name)}\b")
    found = list(pattern.finditer(text))
    if len(found) != 1:
        return None
    start = found[0].start()
    index = found[0].end()
    paren = 0
    body = None
    while index < len(text):
        char = text[index]
        if char == "(":
            paren += 1
        elif char == ")":
            paren = max(0, paren - 1)
        elif char == "{" and paren == 0:
            body = index
            break
        index += 1
    if body is None:
        return None
    depth = 0
    for index in range(body, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return start, body, index
    return None


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
    if code == 0 and re.search(r"\b0 errors\b", output):
        return "survived"
    compile_error = (
        re.search(r"error\[E\d+\]", output) is not None
        or "mismatched types" in output
        or "cannot find value" in output
        or "expected expression" in output
    )
    proof_error = "postcondition" in output or "assertion failed" in output
    if compile_error and not proof_error:
        return "invalid"
    if code != 0:
        return "killed"
    return "invalid"


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
            out = (exc.stdout or "") + (exc.stderr or "")
            return 1, out + "\nerror: mutation run timed out\n"
    return result.returncode, result.stdout + result.stderr


def verus_env():
    env = os.environ.copy()
    env["PATH"] = str(ROOT / ".tools/verus/current") + os.pathsep + env.get("PATH", "")
    env["RUSTUP_TOOLCHAIN"] = toolchain()
    return env


def load_functions():
    rows = []
    lines = FUNCTIONS.read_text().splitlines()
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
        if len(parts) != 5 or not parts[0].startswith("lib::"):
            continue
        qualified, harness, mutation, digest, result = parts
        cached[(harness, qualified)] = {
            "mutation": mutation,
            "digest": digest,
            "result": result,
        }
    return cached


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
        f"functions={len(rows)}",
        f"killed={killed}",
        f"signed_off={signed}",
        f"vacuous={vacuous}",
        "A matching body_sha256 skips Verus. Full prover output is not stored.",
        "function\tharness\tmutation\tbody_sha256\tresult",
    ]
    for row in rows:
        lines.append(
            "\t".join(
                [
                    row["qualified"],
                    row["harness"],
                    row["mutation"],
                    row["digest"],
                    row["result"],
                ]
            )
        )
    SUMMARY.parent.mkdir(parents=True, exist_ok=True)
    SUMMARY.write_text("\n".join(lines) + "\n")


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
            "result": "vacuous",
            "detail": "harness function not found",
        }
    start, body_open, body_close = span
    function_text = original[start : body_close + 1]
    digest = hashlib.sha256(function_text.encode()).hexdigest()
    rtype = return_type(original[start:body_open])
    body = original[body_open + 1 : body_close]
    if rtype is None:
        return {
            "qualified": row["qualified"],
            "harness": row["harness"],
            "mutation": "missing-type",
            "digest": digest,
            "result": "vacuous",
            "detail": "return type not parsed",
        }
    chosen = None
    invalid = []
    try:
        for label, expr in mutants_for(rtype, body):
            path.write_text(apply_mutant(original, body_open, body_close, expr))
            code, output = run_verus(bin_path, path, row["name"], env, timeout=90)
            kind = classify(code, output)
            if kind == "killed":
                chosen = label
                break
            if kind == "invalid":
                invalid.append(label)
        result = "killed" if chosen else "vacuous"
        detail = chosen or ("invalid:" + ",".join(invalid) if invalid else "all-survived")
        return {
            "qualified": row["qualified"],
            "harness": row["harness"],
            "mutation": chosen or "none",
            "digest": digest,
            "result": result,
            "detail": detail,
        }
    finally:
        path.write_text(original)


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
    for qualified in signoff:
        if qualified not in known:
            status |= fail(f"sign-off names a function that is not 已证明: {qualified}")
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
            if not signed:
                status |= fail(f"sign-off is missing a justification: {row['qualified']}")
            finished.append(
                {
                    "qualified": row["qualified"],
                    "harness": row["harness"],
                    "mutation": "sign-off",
                    "digest": digest,
                    "result": "signed-off",
                }
            )
            print(f"signed-off {row['qualified']}")
            continue
        if cached and cached["digest"] == digest and cached["result"] == "killed" and digest:
            finished.append(
                {
                    "qualified": row["qualified"],
                    "harness": row["harness"],
                    "mutation": cached["mutation"],
                    "digest": digest,
                    "result": "killed",
                }
            )
            print(f"cached {row['qualified']} mutation={cached['mutation']}")
            continue
        if bin_path is None:
            bin_path = verus_bin()
            env = verus_env()
        proved = prove_function(row, bin_path, env)
        finished.append(
            {
                "qualified": proved["qualified"],
                "harness": proved["harness"],
                "mutation": proved["mutation"],
                "digest": proved["digest"],
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
    print(
        f"mutation gate functions={len(finished)} killed={killed} signed_off={signed_count} status={status}"
    )
    return status


def self_test():
    """The two vacuous specs must survive mutation. A tight spec must not."""
    source = r"""
use vstd::prelude::*;
verus! {
fn vacuous_bound(flag: bool, deadline: u64) -> (out: bool)
    ensures
        out ==> deadline < deadline + 1,
{
    flag
}

fn vacuous_neg(flag: bool) -> (out: bool)
    ensures
        !(out && !out),
{
    flag
}

fn tight(flag: bool) -> (out: bool)
    ensures
        out == flag,
{
    flag
}
}
"""
    bin_path = verus_bin()
    env = verus_env()
    failures = 0
    with tempfile.TemporaryDirectory(prefix="smros-verus-mutation-") as tmp:
        path = Path(tmp) / "vacuous.rs"
        path.write_text(source)
        code, output = run_verus(bin_path, path, "vacuous_bound", env, timeout=60)
        if classify(code, output) != "survived":
            print("counterexample baseline did not verify", file=sys.stderr)
            failures += 1
        cases = [
            ("vacuous_bound", "false", "survived", "out ==> deadline < deadline + 1"),
            ("vacuous_bound", "true", "survived", "out ==> deadline < deadline + 1"),
            ("vacuous_neg", "false", "survived", "!(out && !out)"),
            ("vacuous_neg", "true", "survived", "!(out && !out)"),
            ("tight", "false", "killed", "out == flag"),
        ]
        for function, expr, expect, label in cases:
            text = source
            span = function_span(text, function)
            if span is None:
                print(f"self-test missing {function}", file=sys.stderr)
                failures += 1
                continue
            _start, body_open, body_close = span
            path.write_text(apply_mutant(text, body_open, body_close, expr))
            code, output = run_verus(bin_path, path, function, env, timeout=60)
            got = classify(code, output)
            if got != expect:
                print(
                    f"self-test {function} {expr}: expected {expect} got {got}",
                    file=sys.stderr,
                )
                failures += 1
            elif expect == "survived":
                print(f"counterexample semantic: {label}: audit exit 1")
            else:
                print(f"self-test killed tight spec with {expr}")
    if failures:
        print("mutation self-test failed", file=sys.stderr)
        return 1
    print("mutation self-test passed; vacuous counterexamples fail the audit")
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    if "--self-test" not in sys.argv:
        probe = self_test()
        if probe != 0:
            sys.exit(probe)
    sys.exit(audit(sys.argv))
