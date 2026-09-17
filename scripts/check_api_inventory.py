#!/usr/bin/env python3
"""Check that the hand-maintained API inventories point at files that exist.

`.api-inventory.txt` and `.api-bindings.txt` are written by hand - their own
header says "there is no generator" - so nothing has ever verified them. The
paths were authored on Windows and carried backslashes, which no POSIX check
could resolve even if one had existed.

This is the cheapest guard that catches the drift that matters: a section
header naming a source file that is gone, which means the names under it
describe something the crate no longer has.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SECTION = re.compile(r"^###\s+\S+\s+\[([^\]]+)\]")
ROOT = Path(__file__).resolve().parent.parent


def check(inventory: Path) -> list[str]:
    problems: list[str] = []
    for number, line in enumerate(inventory.read_text().splitlines(), start=1):
        match = SECTION.match(line)
        if not match:
            continue
        named = match.group(1)
        if "\\" in named:
            problems.append(f"{inventory.name}:{number}: backslash path {named!r}")
            named = named.replace("\\", "/")
        if not (ROOT / named).is_file():
            problems.append(f"{inventory.name}:{number}: no such file {named!r}")
    return problems


def main() -> int:
    problems: list[str] = []
    for name in (".api-inventory.txt", ".api-bindings.txt"):
        path = ROOT / name
        if path.is_file():
            problems.extend(check(path))
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print(f"{len(problems)} stale inventory reference(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
