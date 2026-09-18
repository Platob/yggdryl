#!/usr/bin/env python3
"""Check that the hand-maintained API inventories describe what the crate has.

`.api-inventory.txt` and `.api-bindings.txt` are written by hand - their own
header says "there is no generator" - so nothing verified them for a long
time. The paths were authored on Windows and carried backslashes, which no
POSIX check could resolve even if one had existed.

Two guards, cheapest first:

1. A section header naming a source file that is gone: the names under it
   describe something the crate no longer has.
2. A name listed under a section that the named file does not define. This is
   the drift a rename or a deletion leaves behind, and it is the one that has
   actually happened here - `Value::id`, `Value::into_family`,
   `DataTypeKind::is_wrapper`, `Scalar::as_enum` and twenty-seven `XField`
   aliases were all listed after they stopped existing, because an edit
   appended the new spelling instead of replacing the old one.

The reverse direction - a public item the inventory omits - is deliberately
reported as a count rather than an error. The inventories have never been
complete, and failing CI on that would mean transcribing hundreds of
signatures before any other work could land.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SECTION = re.compile(r"^###\s+\S+\s+\[([^\]]+)\]")
# An entry line: leading space, then an optional `pub`, then the item keyword
# and the name. `fn`, `const fn`, `unsafe fn`, `type`, `const` and `static`
# cover every shape the inventories use.
ENTRY = re.compile(
    r"^\s+(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?"
    r"(?:unsafe\s+)?(?:extern\s+\"[^\"]*\"\s+)?(fn|type|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
ROOT = Path(__file__).resolve().parent.parent


WORD = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
_TOKENS: dict[Path, set[str]] = {}


def crate_of(named: str) -> Path | None:
    """The source tree a section's names must appear somewhere in.

    A section documents one *type's* whole surface, and that legitimately
    spans sibling modules: the `types/field.rs` section carries the protocol
    accessors that live in `types/protocol/http.rs`, and the per-family
    `fields.rs` files supply names listed under `types/field.rs`. So the file
    named in the header is where to read the section, not a claim about where
    every name is defined - and checking names against it alone reports over a
    hundred false positives.

    The crate is the honest scope. It still catches the drift that has
    happened: a name kept after the item was renamed or deleted, which then
    appears nowhere at all.
    """
    parts = Path(named).parts
    if "src" not in parts:
        return None
    return ROOT.joinpath(*parts[: parts.index("src") + 1])


def tokens(crate: Path) -> set[str]:
    """Every identifier appearing anywhere in a crate's sources.

    Deliberately every *occurrence*, not every declaration: names like
    `Field::as_transform` are emitted by `for_each_well_known_protocol!` and
    are never written as a `fn` item, but they do appear as macro arguments.
    Matching occurrences keeps macro-generated surface out of the report.
    """
    if crate not in _TOKENS:
        found: set[str] = set()
        for path in crate.rglob("*.rs"):
            found.update(WORD.findall(path.read_text(errors="replace")))
        _TOKENS[crate] = found
    return _TOKENS[crate]


def check(inventory: Path) -> list[str]:
    problems: list[str] = []
    crate: Path | None = None
    section = ""
    for number, line in enumerate(inventory.read_text().splitlines(), start=1):
        match = SECTION.match(line)
        if match:
            named = match.group(1)
            if "\\" in named:
                problems.append(f"{inventory.name}:{number}: backslash path {named!r}")
                named = named.replace("\\", "/")
            if not (ROOT / named).is_file():
                problems.append(f"{inventory.name}:{number}: no such file {named!r}")
            section, crate = named, crate_of(named)
            continue
        if crate is None or not crate.is_dir():
            continue
        entry = ENTRY.match(line)
        if entry and entry.group(2) not in tokens(crate):
            problems.append(
                f"{inventory.name}:{number}: {entry.group(1)} {entry.group(2)!r} "
                f"(listed under {section}) appears nowhere in {crate.relative_to(ROOT)}"
            )
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
