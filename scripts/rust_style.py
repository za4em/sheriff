#!/usr/bin/env python3
"""Fail the style mistakes from the score-window change.

Import a type instead of writing a path of two or more '::'. The last
segment of that path is the type, or the segment before it when the last
segment is an enum variant. assert! belongs in a test. A function that
contains two of if, match, and for/while/loop and runs past 48 lines does
not fit on one screen. run, screen, and Screen do not say what happens.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
SCREEN_LINES = 48
BANNED_FUNCTIONS = {
    "run": "score_group scores one group, watch_pending republishes stale decisions",
    "screen": "judge_window asks the provider, classify_windows walks the windows",
}
BANNED_TYPES = {"Screen": "WindowVerdict"}
TYPE_PATH = re.compile(
    r"\b(?:[A-Za-z_][A-Za-z0-9_]*::){2,}[A-Z][A-Za-z0-9_]*\b"
)
KIND = {"if": "if", "match": "match", "for": "for", "while": "for", "loop": "for"}


def at_string(text, i):
    if text[i] == '"':
        return True
    rest = text[i:]
    for head in ("br", "b", "r"):
        if rest.startswith(head) and len(rest) > len(head) and rest[len(head)] in '#"':
            return True
    return False


def end_of_string(text, i):
    n = len(text)
    hashes = 0
    j = i
    while j < n and text[j] in "bru#":
        if text[j] == "#":
            hashes += 1
        j += 1
        if j < n and text[j] == '"':
            break
    if j >= n or text[j] != '"':
        return i + 1
    if hashes:
        closer = '"' + ("#" * hashes)
        end = text.find(closer, j + 1)
        return n if end < 0 else end + len(closer)
    k = j + 1
    while k < n:
        if text[k] == "\\":
            k += 2
            continue
        if text[k] == '"':
            return k + 1
        k += 1
    return n


def end_of_char_or_lifetime(text, i):
    n = len(text)
    rest = text[i + 1 :]
    if rest.startswith("\\") and len(rest) >= 3 and rest[2] == "'":
        return i + 4
    if len(rest) >= 2 and rest[1] == "'" and rest[0] != "\\":
        return i + 3
    j = i + 1
    while j < n and (text[j].isalnum() or text[j] == "_"):
        j += 1
    return j


def strip_comments_and_strings(text):
    out = []
    i = 0
    n = len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            if j < 0:
                out.append(" " * (n - i))
                break
            out.append(" " * (j - i))
            i = j
            continue
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            if j < 0:
                out.append(" " * (n - i))
                break
            chunk = text[i : j + 2]
            out.append("".join("\n" if ch == "\n" else " " for ch in chunk))
            i = j + 2
            continue
        if at_string(text, i):
            j = end_of_string(text, i)
            chunk = text[i:j]
            out.append("".join("\n" if ch == "\n" else " " for ch in chunk))
            i = j
            continue
        if text[i] == "'":
            j = end_of_char_or_lifetime(text, i)
            chunk = text[i:j]
            out.append("".join("\n" if ch == "\n" else " " for ch in chunk))
            i = j
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def line_of(text, index):
    return text.count("\n", 0, index) + 1


def skip_uses(text):
    chars = list(text)
    i = 0
    n = len(text)
    while i < n:
        if re.match(r"\buse\b", text[i:]):
            j = i
            depth = 0
            while j < n:
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth = max(0, depth - 1)
                elif text[j] == ";" and depth == 0:
                    j += 1
                    break
                j += 1
            for k in range(i, j):
                if chars[k] != "\n":
                    chars[k] = " "
            i = j
            continue
        i += 1
    return "".join(chars)


def test_ranges(text):
    ranges = []
    pattern = re.compile(r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+\w+|mod\s+tests\b")
    for match in pattern.finditer(text):
        brace = text.find("{", match.end())
        if brace < 0:
            continue
        depth = 0
        for i in range(brace, len(text)):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    ranges.append((match.start(), i + 1))
                    break
    return ranges


def in_ranges(index, ranges):
    return any(start <= index < end for start, end in ranges)


def functions(text):
    found = []
    for match in re.finditer(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b", text):
        brace = text.find("{", match.end())
        if brace < 0:
            continue
        depth = 0
        end = None
        for i in range(brace, len(text)):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    end = i + 1
                    break
        if end is None:
            continue
        found.append((match.group(1), match.start(), brace, end))
    return found


def imported_type(path):
    parts = path.split("::")
    if len(parts) >= 2 and parts[-1][:1].isupper() and parts[-2][:1].isupper():
        return parts[-2]
    return parts[-1]


def control_kinds(body):
    found = set()
    for match in re.finditer(r"\b(if|for|while|loop|match)\b", body):
        found.add(KIND[match.group(1)])
    return found


def type_names(text):
    names = []
    for match in re.finditer(
        r"\b(?:enum|struct|type|trait)\s+([A-Za-z_][A-Za-z0-9_]*)\b", text
    ):
        names.append((match.group(1), match.start()))
    return names


def check_text(path, text):
    clean = strip_comments_and_strings(text)
    tests = test_ranges(clean)
    failures = []
    visible = skip_uses(clean)
    for match in TYPE_PATH.finditer(visible):
        if in_ranges(match.start(), tests):
            continue
        path_text = match.group(0)
        failures.append(
            f"{path}:{line_of(text, match.start())}: import {imported_type(path_text)} instead of writing {path_text}"
        )
    for match in re.finditer(r"\bassert(?:_eq|_ne)?!\s*\(", clean):
        if in_ranges(match.start(), tests):
            continue
        failures.append(
            f"{path}:{line_of(text, match.start())}: assert belongs in a test. Delete it or move it under #[cfg(test)]"
        )
    for name, start, brace, end in functions(clean):
        if in_ranges(start, tests):
            continue
        if name in BANNED_FUNCTIONS:
            failures.append(
                f"{path}:{line_of(text, start)}: rename {name}. {BANNED_FUNCTIONS[name]}"
            )
        span = line_of(text, end - 1) - line_of(text, start) + 1
        kinds = control_kinds(clean[brace:end])
        if span > SCREEN_LINES and len(kinds) >= 2:
            listed = ", ".join(sorted(kinds))
            failures.append(
                f"{path}:{line_of(text, start)}: {name} is {span} lines and nests {listed}. Split it so it fits on one screen ({SCREEN_LINES} lines)"
            )
    for name, start in type_names(clean):
        if in_ranges(start, tests):
            continue
        if name in BANNED_TYPES:
            failures.append(
                f"{path}:{line_of(text, start)}: rename {name} to {BANNED_TYPES[name]}"
            )
    return failures


def long_branchy(name):
    lines = [f"fn {name}() {{", "    if true {", "        let _ = 1;", "    }"]
    lines.extend(["    let _ = 1;"] * 45)
    lines.extend(["    for _ in 0..1 {", "        let _ = 1;", "    }", "}"])
    return "\n".join(lines) + "\n"


BAD = {
    "inline type": (
        "fn keep() {\n    let covered: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();\n    let _ = covered;\n}\n",
        "import BTreeSet",
    ),
    "production assert": ("fn keep() {\n    assert!(true);\n}\n", "assert belongs in a test"),
    "vague function": ("fn screen() {}\nfn run() {}\n", "rename screen"),
    "vague type": ("enum Screen { Hit }\n", "rename Screen to WindowVerdict"),
    "long function": (long_branchy("keep"), "fits on one screen"),
}

GOOD = (
    """
use std::collections::BTreeSet;

fn score_group() {
    let ids: BTreeSet<i64> = BTreeSet::new();
    if ids.is_empty() {
        for _ in 0..1 {
            let _ = 1;
        }
    }
}

enum WindowVerdict { Clear }

#[cfg(test)]
mod tests {
    #[test]
    fn covers_the_window() {
        assert!(true);
    }
}
"""
    + "fn name() {\n"
    + "    match 1 {\n        1 => {}\n        _ => {}\n    }\n" * 20
    + "}\n"
)


def self_test():
    problems = []
    for name, (snippet, expected) in BAD.items():
        found = check_text(f"fixture/{name}.rs", snippet)
        if not any(expected in item for item in found):
            problems.append(f"fixture {name} did not report {expected!r}: {found}")
    accepted = check_text("fixture/good.rs", GOOD)
    if accepted:
        problems.extend(f"good fixture rejected: {item}" for item in accepted)
    return problems


def main():
    if "--self-test" in sys.argv:
        problems = self_test()
        if problems:
            print("\n".join(problems))
            return 1
        print("style fixtures passed")
        return 0
    failures = []
    for path in sorted(SRC.rglob("*.rs")):
        failures.extend(check_text(path.relative_to(ROOT), path.read_text()))
    if failures:
        print("\n".join(failures))
        print(f"{len(failures)} style violations")
        return 1
    print("style check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
