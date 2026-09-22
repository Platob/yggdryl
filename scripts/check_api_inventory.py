#!/usr/bin/env python3
"""Check that the hand-maintained API inventories describe what the crate has.

`.api-inventory.txt` and `.api-bindings.txt` are written by hand - their own
header says "there is no generator" - so nothing verified them for a long
time. The paths were authored on Windows and carried backslashes, which no
POSIX check could resolve even if one had existed.

Four guards, cheapest first:

1. A section header naming a source file that is gone: the names under it
   describe something the crate no longer has.
2. A name listed under a section that the named file does not define. This is
   the drift a rename or a deletion leaves behind, and it is the one that has
   actually happened here - `Value::id`, `Value::into_family`,
   `DataTypeKind::is_wrapper`, `Scalar::as_enum` and twenty-seven `XField`
   aliases were all listed after they stopped existing, because an edit
   appended the new spelling instead of replacing the old one.
3. A *type* an entry names in the signature it documents. Guard 2 reads the
   name an entry declares and nothing else, so a wrapper's inner type, a
   return type, or an alias's right-hand side could rot untouched: 135 lines
   still said `MimeTypeValue`, `SchemeValue`, `TypedField` and `ValueIter`
   long after all four became `MimeTypeWire`, `SchemeWire`, `FieldOf` and
   `ScalarIter`. Only the code part of a line is read - the prose after two
   spaces and an open parenthesis is prose - and only names that appear
   nowhere in the crate are reported.
4. `.api-bindings.txt`, which until now was read and then skipped in its
   entirety: its two section headers name a language rather than a file, so
   the crate under them resolved to nothing and all 477 lines passed without
   being looked at. Its entries have their own shape - `ClassName: member,
   member` - so they are checked against their own binding's surface: the
   stubs and package for Python, the generated declarations for JavaScript,
   each beside the binding crate that answers them.

The reverse direction - a public item the inventory omits - is deliberately
reported as a count rather than an error, and now actually is one. The
inventories have never been complete, and failing CI on that would mean
transcribing hundreds of signatures before any other work could land; a
number says how far off they are without blocking the work that noticed.
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
# A type named in a signature. Only the code part of an entry is read, so this
# never sees the prose a line ends with.
TYPE = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\b")
# The inventories end a line's code and begin its prose with two spaces and an
# open parenthesis, which is also how a tuple struct would never be written.
PROSE = "  ("
# A binding entry: `  ClassName: member, member` at the top level of a section,
# or a deeper line whose leading word only labels the group of names under it.
BINDING_SECTION = re.compile(r"^###\s+(python|javascript):")
BINDING_ENTRY = re.compile(r"^(\s+)([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$")
# A member the check can be sure of: a bare name, not a signature, a sentence,
# or a spelling with punctuation in it.
BARE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
# Where each binding's public surface is written down. The stubs and the
# generated declarations are what a caller reads; the binding crate beside them
# is where a name that only reaches the caller through a macro still appears.
# The package's own JavaScript belongs here too: `fields` and the other
# hand-written helpers are declared in `node/fields.js` and reach the caller
# without ever appearing in a `.d.ts` class.
SURFACES = {
    "python": ("python/yggdryl/**/*.pyi", "python/yggdryl/**/*.py", "python/src/**/*.rs"),
    "javascript": ("node/*.d.ts", "node/*.js", "node/src/**/*.rs"),
}
_TOKENS: dict[Path, set[str]] = {}
_SURFACES: dict[str, set[str]] = {}


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


def surface(language: str) -> set[str]:
    """Every identifier a binding's public surface spells.

    The same reasoning as `tokens`: an occurrence anywhere in the stubs, the
    package, or the binding crate is enough, because a name can reach the
    caller through a `#[pyclass]` attribute or a napi macro without ever being
    written as a declaration.
    """
    if language not in _SURFACES:
        found: set[str] = set()
        for pattern in SURFACES[language]:
            for path in ROOT.glob(pattern):
                if path.is_file():
                    found.update(WORD.findall(path.read_text(errors="replace")))
        _SURFACES[language] = found
    return _SURFACES[language]


def check_bindings(inventory: Path) -> list[str]:
    """Check `.api-bindings.txt` against the surface each section names.

    A two-space entry declares a class, so its own name is checked too; a
    deeper one only labels a group - `statics:`, `sections:`, `writable:` -
    so only the names it lists are. Either way just the bare names in the
    line's first clause are read, because everything past the first `;`, ` - `
    or `(` is prose about them.
    """
    problems: list[str] = []
    language = ""
    for number, line in enumerate(inventory.read_text().splitlines(), start=1):
        header = BINDING_SECTION.match(line)
        if header:
            language = header.group(1)
            continue
        entry = BINDING_ENTRY.match(line) if language else None
        if not entry:
            continue
        indent, name, rest = len(entry.group(1)), entry.group(2), entry.group(3)
        known = surface(language)
        if indent == 2 and name not in known:
            problems.append(
                f"{inventory.name}:{number}: {language} class {name!r} "
                f"appears nowhere in that binding's surface"
            )
        for member in re.split(r";| - |\(", rest)[0].split(","):
            member = member.strip()
            if BARE.match(member) and member not in known:
                problems.append(
                    f"{inventory.name}:{number}: {language} {name}.{member} "
                    f"appears nowhere in that binding's surface"
                )
    return problems


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
        for named in TYPE.findall(line.split(PROSE)[0]):
            if named not in tokens(crate):
                problems.append(
                    f"{inventory.name}:{number}: type {named!r} "
                    f"(named under {section}) appears nowhere in {crate.relative_to(ROOT)}"
                )
    return problems


def omitted(inventory: Path) -> tuple[int, int]:
    """How far short of the crate the Rust inventory falls.

    Two numbers, neither an error: source files no section names, and `pub`
    items whose name the inventory never spells. Both are approximations - a
    `pub` item inside a private module is not public API, and a name shared
    with a documented one counts as present - so they are reported as a
    direction of travel, not a target to reach.
    """
    listed_paths = set()
    for line in inventory.read_text().splitlines():
        match = SECTION.match(line)
        if match:
            listed_paths.add(match.group(1).replace("\\", "/"))
    spelled = set(WORD.findall(inventory.read_text()))
    declaration = re.compile(
        r"^\s*pub\s+(?:unsafe\s+)?(?:const\s+)?(?:async\s+)?"
        r"(?:fn|struct|enum|trait|type|union)\s+([A-Za-z_][A-Za-z0-9_]*)"
    )
    files = 0
    names: set[str] = set()
    for path in sorted((ROOT / "rust" / "src").rglob("*.rs")):
        if path.relative_to(ROOT).as_posix() not in listed_paths:
            files += 1
        for line in path.read_text(errors="replace").splitlines():
            found = declaration.match(line)
            if found and found.group(1) not in spelled:
                names.add(found.group(1))
    return files, len(names)


def main() -> int:
    problems: list[str] = []
    rust = ROOT / ".api-inventory.txt"
    bindings = ROOT / ".api-bindings.txt"
    if rust.is_file():
        problems.extend(check(rust))
    if bindings.is_file():
        problems.extend(check(bindings))
        problems.extend(check_bindings(bindings))
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print(f"{len(problems)} stale inventory reference(s)", file=sys.stderr)
        return 1
    if rust.is_file():
        files, names = omitted(rust)
        print(
            f"inventories are current; {files} source file(s) and {names} "
            f"`pub` name(s) are not described yet"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
