#!/usr/bin/env python3
"""Spotifast Free's renames of upstream Spotifast, kept in one rules file.

Usage:
  fork-rename.py check
      Fail if any rule's upstream phrase is in a file it covers (a rename
      went missing) or a rule matches nothing at all (upstream changed the
      phrase: update or drop the rule).
  fork-rename.py apply [PATH ...]
      Rename upstream phrases in place, in PATHs or in every covered file.
  fork-rename.py merge-driver BASE OURS THEIRS PATH
      A git merge driver: rename upstream's side and the common ancestor,
      then merge them with ours in three ways. A conflict that was only the
      name goes away; any other conflict stays for a person.
  fork-rename.py merge REF
      Merge REF (upstream) into the current branch with the driver, then
      apply the rules to every covered file the merge touched.
"""

from __future__ import annotations

import fnmatch
import os
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RULES = ROOT / ".github" / "fork-rename.toml"


def rules() -> list[dict]:
    with RULES.open("rb") as handle:
        loaded = tomllib.load(handle)["rule"]
    # Longest first, so a phrase inside a longer one never fires early.
    return sorted(loaded, key=lambda rule: len(rule["from"]), reverse=True)


def covers(rule: dict, path: str) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in rule["files"])


def rename(text: str, path: str, active: list[dict]) -> str:
    for rule in active:
        if covers(rule, path):
            text = protect_replace(text, rule["from"], rule["to"])
    return text


def protect_replace(text: str, old: str, new: str) -> str:
    """Replace `old` with `new`, leaving text that already reads `new`.

    When `new` contains `old` (as "Spotifast Free" contains "Spotifast"),
    a plain replace would rename already-renamed text twice.
    """
    if old not in new:
        return text.replace(old, new)
    marker = "\0FORK-RENAME\0"
    return text.replace(new, marker).replace(old, new).replace(marker, new)


def covered_files(active: list[dict]) -> list[str]:
    tracked = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.splitlines()
    return [path for path in tracked if any(covers(rule, path) for rule in active)]


def remaining(text: str, rule: dict) -> bool:
    """Whether the upstream phrase still stands where the rule renames it."""
    if rule["from"] not in rule["to"]:
        return rule["from"] in text
    return rule["from"] in text.replace(rule["to"], "")


def check() -> int:
    active = rules()
    problems = []
    for rule in active:
        paths = [path for path in covered_files([rule])]
        if not paths:
            problems.append(f"no file matches {rule['files']} for {rule['from']!r}")
            continue
        seen = False
        for path in paths:
            text = (ROOT / path).read_text(encoding="utf-8")
            if rule["to"] in text:
                seen = True
            if remaining(text, rule):
                problems.append(f"{path}: upstream {rule['from']!r} is not renamed to {rule['to']!r}")
        if not seen:
            problems.append(
                f"{rule['from']!r} -> {rule['to']!r} matches nothing in {rule['files']}: "
                "upstream changed it; update or drop the rule"
            )
    for problem in problems:
        print(problem)
    print("fork renames: " + ("FAILED" if problems else f"{len(active)} rules hold"))
    return 1 if problems else 0


def apply(paths: list[str]) -> int:
    active = rules()
    changed = 0
    for path in paths or covered_files(active):
        file = ROOT / path
        if not file.is_file():
            continue
        text = file.read_text(encoding="utf-8")
        renamed = rename(text, path, active)
        if renamed != text:
            file.write_text(renamed, encoding="utf-8")
            changed += 1
            print(f"renamed {path}")
    print(f"fork renames applied to {changed} file(s)")
    return 0


def merge_driver(base: str, ours: str, theirs: str, path: str) -> int:
    active = rules()
    for side in (base, theirs):
        file = Path(side)
        try:
            text = file.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            break
        file.write_text(rename(text, path, active), encoding="utf-8")
    # git merge-file writes the result over OURS and exits with the number of
    # conflicts, which is what git expects from a merge driver.
    result = subprocess.run(
        ["git", "merge-file", "-L", "fork", "-L", "base", "-L", "upstream", ours, base, theirs]
    )
    return 0 if result.returncode == 0 else 1


def merge(ref: str) -> int:
    active = rules()
    with tempfile.TemporaryDirectory() as scratch:
        attributes = Path(scratch) / "attributes"
        attributes.write_text(
            "".join(f"{path} merge=fork-rename\n" for path in covered_files(active)),
            encoding="utf-8",
        )
        driver = f"{sys.executable} {Path(__file__).resolve()} merge-driver %O %A %B %P"
        merged = subprocess.run(
            [
                "git",
                "-c", f"core.attributesFile={attributes}",
                "-c", f"merge.fork-rename.driver={driver}",
                "-c", "merge.fork-rename.name=Spotifast Free renames",
                "merge", "--no-ff", "--no-commit", ref,
            ],
            cwd=ROOT,
        )
    # Upstream's new lines in files that merged without the driver get the
    # rules too; conflicted files keep their markers for a person.
    unmerged = set(
        subprocess.run(
            ["git", "diff", "--name-only", "--diff-filter=U"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout.splitlines()
    )
    touched = [path for path in covered_files(active) if path not in unmerged]
    apply(touched)
    subprocess.run(["git", "add", "--", *touched], cwd=ROOT, check=True)
    return merged.returncode


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    command, rest = argv[1], argv[2:]
    if command == "check":
        return check()
    if command == "apply":
        return apply(rest)
    if command == "merge-driver" and len(rest) == 4:
        return merge_driver(*rest)
    if command == "merge" and len(rest) == 1:
        return merge(rest[0])
    print(__doc__)
    return 2


if __name__ == "__main__":
    os.chdir(ROOT)
    sys.exit(main(sys.argv))
