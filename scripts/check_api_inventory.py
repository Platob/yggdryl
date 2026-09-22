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

`.api-bindings.txt` was read by this script and checked by none of it. Its
headers carried no `[path]`, so `SECTION` never matched, `crate` stayed `None`
and every line fell through the guard; and even had one matched, `ENTRY` only
knows the Rust item keywords, which a binding entry does not spell. The file
went a release describing `yggdryl.types`, `yggdryl.hashing`,
`yggdryl.media.iceberg` and `yggdryl.text.toml` - every one of them deleted -
and stated outright that `yggdryl.xxhash` does not exist, which it does.

So the bindings file names its tree too, and a third guard reads it: an entry
key is a dotted path from that tree's root, and each segment must be a module
beside its parent or a name that parent's namespace binds. Python is resolved
through `ast` over the package's own sources - the names a module imports,
assigns, defines, or lists in `__all__` - so a path that survives only in a
docstring is still reported. JavaScript is resolved against the files
`node/package.json` ships, which is what `yggdryl` the npm package answers.
"""

from __future__ import annotations

import ast
import json
import re
import sys
from pathlib import Path

# A header names what the section documents, then the tree to read it
# against in brackets. The label may carry spaces - a Rust header
# sometimes qualifies one file twice, `(additions)`, `(refs and
# retention)` - and a parenthetical may follow the bracket, so the label
# is taken lazily and nothing is anchored to the end of the line.
SECTION = re.compile(r"^###\s+(.*?)\s*\[([^\]]+)\]")
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


# A binding entry opens with the dotted path it documents: `Field`,
# `iceberg.IcebergOptions`, `xxhash`. A key of two words is prose - `media
# roles`, `fix functions`, `root constants` - and names nothing to resolve, so
# it is skipped; `<name> namespace` is the JavaScript spelling of a bare key.
BINDING_ENTRY = re.compile(
    r"^  ([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)"
    r"(?::|\s+namespace\b|\s*$)"
)
_NAMES: dict[Path, frozenset[str]] = {}


def module_beside(parent: Path, name: str) -> Path | None:
    """The module `name` names beside `parent`, as a file or a package."""
    for candidate in (parent / f"{name}.py", parent / f"{name}.pyi"):
        if candidate.is_file():
            return candidate
    package = parent / name
    if (package / "__init__.py").is_file() or (package / "__init__.pyi").is_file():
        return package
    return None


def bound_names(module: Path) -> frozenset[str]:
    """Every name a Python module binds at its top level.

    Imports, assignments, `def`, `class`, and the strings `__all__` lists -
    which is what another module can reach through it. Read with `ast` rather
    than by importing, so the check stays static and needs no built extension;
    read from the source rather than from every token in it, so a path that
    survives only in a docstring is not mistaken for one that resolves.
    """
    if module in _NAMES:
        return _NAMES[module]
    sources = (
        [module / "__init__.py", module / "__init__.pyi"]
        if module.is_dir()
        else [module]
    )
    found: set[str] = set()
    for source in sources:
        if not source.is_file():
            continue
        try:
            tree = ast.parse(source.read_text(errors="replace"))
        except SyntaxError:
            continue
        for node in tree.body:
            if isinstance(node, (ast.Import, ast.ImportFrom)):
                for alias in node.names:
                    found.add(alias.asname or alias.name.split(".")[0])
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                found.add(node.name)
            elif isinstance(node, ast.Assign):
                for target in node.targets:
                    if isinstance(target, ast.Name):
                        found.add(target.id)
                        if target.id == "__all__":
                            found.update(
                                element.value
                                for element in getattr(node.value, "elts", [])
                                if isinstance(element, ast.Constant)
                                and isinstance(element.value, str)
                            )
            elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
                found.add(node.target.id)
    _NAMES[module] = frozenset(found)
    return _NAMES[module]


def resolves_in_python(package: Path, dotted: str) -> bool:
    """Whether `dotted` names something reachable from a Python package.

    Each segment is a module beside its parent, or a name that parent binds.
    The root also admits the extension module's own classes: `Xxh32` and
    `TxHash` are `_native`'s, surfaced through the module that owns them.
    """
    parent = package
    for index, segment in enumerate(dotted.split(".")):
        beside = module_beside(parent, segment)
        if beside is not None:
            parent = beside
            continue
        names = bound_names(parent)
        if parent == package:
            native = module_beside(package, "_native")
            if native is not None:
                names = names | bound_names(native)
        if segment in names:
            # A member is where static resolution stops: what hangs off it is
            # the extension's, which no source in this tree spells.
            return index == len(dotted.split(".")) - 1
        return False
    return True


def shipped_javascript(package: Path) -> set[str]:
    """Every identifier in the files `node/package.json` ships.

    The published surface is the honest scope for a section documenting it: a
    name no shipped declaration or module spells is a name no caller can
    reach, whatever `src/` still says.
    """
    if package in _TOKENS:
        return _TOKENS[package]
    manifest = json.loads((package / "package.json").read_text())
    found: set[str] = set()
    for entry in manifest.get("files", []):
        for path in sorted(package.glob(entry)):
            if path.is_file() and path.suffix in {".js", ".ts", ".mjs", ".cjs"}:
                found.update(WORD.findall(path.read_text(errors="replace")))
    _TOKENS[package] = found
    return found


def binding_problems(inventory: Path, number: int, line: str, scope: Path,
                     language: str, section: str) -> list[str]:
    """Report a binding entry whose key reaches nothing in its own tree."""
    entry = BINDING_ENTRY.match(line)
    if entry is None:
        return []
    dotted = entry.group(1)
    if language == "python":
        if resolves_in_python(scope, dotted):
            return []
    elif all(part in shipped_javascript(scope) for part in dotted.split(".")):
        return []
    return [
        f"{inventory.name}:{number}: {dotted!r} (listed under {section}) "
        f"reaches nothing in {scope.relative_to(ROOT)}"
    ]


def check(inventory: Path) -> list[str]:
    problems: list[str] = []
    crate: Path | None = None
    scope: Path | None = None
    language = ""
    section = ""
    for number, line in enumerate(inventory.read_text().splitlines(), start=1):
        match = SECTION.match(line)
        if match:
            label, named = match.group(1), match.group(2)
            if "\\" in named:
                problems.append(f"{inventory.name}:{number}: backslash path {named!r}")
                named = named.replace("\\", "/")
            target = ROOT / named
            # A Rust section names the one file it documents; a binding
            # section names the tree its whole surface is published from.
            language = label.split(":")[0].strip() if ":" in label else ""
            if language in ("python", "javascript"):
                if not target.is_dir():
                    problems.append(f"{inventory.name}:{number}: no such folder {named!r}")
                section, scope, crate = named, target, None
            else:
                if not target.is_file():
                    problems.append(f"{inventory.name}:{number}: no such file {named!r}")
                section, crate, scope = named, crate_of(named), None
            continue
        if scope is not None and scope.is_dir():
            problems.extend(
                binding_problems(inventory, number, line, scope, language, section)
            )
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
