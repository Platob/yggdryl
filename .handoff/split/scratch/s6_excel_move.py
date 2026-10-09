#!/usr/bin/env python3
"""S6b: move the Excel medium into `yggdryl-excel` at `rust/excel/`.

Usage:
    python3 -I s6_excel_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch after S4 (the amended order:
excel lands right after S4, a commit of its own, before xmla and the avro,
parquet, s3 and iceberg batch) - and it leaves the tree uncommitted: `git
status` shows the moves as renames and every rewrite as a modification, for
the lane manager's compiler loop (`cargo check --workspace --all-targets
--all-features --keep-going`). It runs no cargo command. A second run on a
tree it already moved stops at step 0 and changes nothing.

What it does, in order (each step says what it touched; `--residue` gets
every site it could not rewrite mechanically, `file:line` each):

 1. test split - before the moves, every core test item that reaches Excel -
                 a `yggdryl::excel` path, a name imported from it, a handle
                 whose name declares a workbook, `MimeType::XLSX`, the
                 medium's name - moves byte-identical into `rust/excel/tests/`
                 at the same relative path, its helpers copied (a macro
                 invocation with the items it defines), each half's imports
                 pruned - a trait whose methods a half calls kept - the
                 harness written beside it from the core's (D39: a test lives
                 in the lowest crate that can name everything it uses). Files
                 that name `.xlsx` or `MimeType::XLSX` as routing vocabulary
                 alone (`mime_type`, `uri`) and the XML for Analysis captures
                 of Excel the client stay.
 2. register   - `RESERVED_RANKS` and the claim rule (D39) pinned in the
                 core's `media_register.rs` over test-only media.
 3. bench      - the core's `media` bench loses its Excel group; the group's
                 file becomes `yggdryl-excel`'s `excel` bench root, its
                 `main` installing the crate.
 4. moves      - `git mv` by glob: `rust/src/excel/` to `rust/excel/src/`
                 (`mod.rs` the crate root `lib.rs`), `rust/tests/excel{,.rs}`
                 to `rust/excel/tests/`, `rust/tests/interop/excel.rs` to
                 `rust/excel/tests/interop/excel.rs`,
                 `rust/benchmarks/media/excel.rs` to
                 `rust/excel/benchmarks/excel.rs`;
                 `rust/tests/support/excel_package.rs` stays in support and
                 the moved harness reaches it by `#[path]`.
 5. crate      - `rust/excel/src/lib.rs`: the doc, `#![deny(unsafe_code)]`,
                 `install()` claiming `EXCEL_CODEC` at its reserved rank 6
                 through `yggdryl::media::codec::claim`, idempotent (D7); no
                 `implementer` of its own, since no crate reaches Excel.
 6. core       - `pub mod excel` and the seed claim gone; `RESERVED_RANKS`
                 and the claim rule (D39); `yggdryl::implementer` grown by
                 S3's routes - R raises in `iobase`, `iobase::hierarchy`,
                 `iomedia` and `temporal::scalars`, F/A forwarders for the
                 published modules and the two associated items; the
                 package's `exclude`.
 7. paths      - every `crate::` path of the moved sources by owner, every
                 `yggdryl::excel` path and `use` tree in tests, benches,
                 bindings, docs and skills, file paths in every text file,
                 `#[path]` re-anchored; then the four method calls on core
                 types the path rewrite cannot see re-spelled through their
                 forwarders.
 8. interop    - `rust/excel/tests/interop.rs`, the exchange directory kept
                 at `rust/target/excel-interop`, the driver's cargo line
                 `-p yggdryl-excel`.
 9. install    - every excel test opens with `crate::install::installed()`,
                 the bench `main` installs, the crate root's rustdoc example
                 and every page block reading a workbook through the register
                 install, the bindings' init and the CLI's `main` install.
10. manifests  - `rust/excel/Cargo.toml` and its README, the workspace
                 members and `[workspace.dependencies]`, the bindings', the
                 CLI's.
11. tooling    - `generate_internals.py` and `check_api_inventory.py` per
                 crate (S4's edits, where S4 has not landed), the generator.
12. CI         - the `excel` leaf line, `x-excel` reading the crate, the
                 planner test, the workflow's comments.
13. docs       - the Excel page, the media overview's register section, the
                 benchmark index, the skills, contributing, architecture,
                 AGENTS.md, the README, `.api-inventory.txt` re-homed.
14. residue    - what is left for the compiler loop, by file and line.

Helpers: S4's lexer, `use` trees, rewriter, re-anchoring and git lock
(`s4_move.py`), and the avro define's generic ones (`s6_avro_move.py`) - its
import prune that keeps a trait whose methods a file calls, the names a
macro invocation defines, the reserved-rank refusal and its test, the raise
and `use`-tree helpers, the per-crate tooling edits. Both are imported and
never edited; their residue and progress report into this script's.
"""

from __future__ import annotations

import argparse
import collections
import os
import pathlib
import posixpath
import re
import sys

SCRATCH = pathlib.Path(
    os.environ.get(
        "S",
        "/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad",
    )
)
sys.path.insert(0, str(SCRATCH))
import s4_move as s4  # noqa: E402  (S4's lexer, use trees, rewriter, lock)
import s6_avro_move as av  # noqa: E402  (the avro define's generic helpers)

CORE, EXCEL = "core", "excel"
PACKAGE = "yggdryl-excel"
CRATE = "yggdryl_excel"
CRATE_DIR = "rust/excel"
XLSX_TYPE = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-excel] {what}", flush=True)


# Both helper modules report into this script's residue; S4's rewriter learns
# the crate.
s4.residue = residue
av.residue = residue
av.done = done
s4.CRATE_NAME[EXCEL] = CRATE

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, UseParseError = s4.parse_use, s4.UseParseError
line_of = s4.line_of
prune_imports = av.prune_imports
replace_once = s4.replace_once
edit = av.edit
tidy_head = av.tidy_head


# ---------------------------------------------------------------------------
# Who owns a path
# ---------------------------------------------------------------------------

# The core's crate-private items the moved sources reach, by the path they
# spell after `crate::`, and the name `yggdryl::implementer` publishes them
# under (S3's routes; the map's section 2, re-read on this tree: D34 and D35
# rewrote `media.rs`, which now reads `container_origin`, `field_under` and
# the cache clock too).
CORE_ROUTES: dict[tuple[str, ...], str] = {
    ("TemporalKind",): "TemporalKind",
    ("arrow", "arrow_schema_from_field"): "arrow_schema_from_field",
    ("arrow", "field_from_arrow_schema"): "field_from_arrow_schema",
    ("arrow", "rows", "result_reader"): "result_reader",
    ("boolean", "BOOLEAN_SPELLINGS"): "BOOLEAN_SPELLINGS",
    ("boolean", "bool_from_text"): "bool_from_text",
    ("floating", "f64_from_text"): "f64_from_text",
    ("iobase", "append_arrow_reader_default"): "append_arrow_reader_default",
    ("iobase", "leaf_writer"): "leaf_writer",
    ("iobase", "merge_arrow_reader_default"): "merge_arrow_reader_default",
    ("iobase", "overwrite_arrow_reader_default_with_field"): "overwrite_arrow_reader_default_with_field",
    ("iobase", "owned_handle"): "owned_handle",
    ("iomedia", "container_field"): "container_field",
    ("iomedia", "container_origin"): "container_origin",
    ("iomedia", "container_row_size"): "container_row_size",
    ("iomedia", "dimension_options"): "dimension_options",
    ("iomedia", "field_under"): "field_under",
    ("iomedia", "own_options"): "own_options",
    ("iomedia", "read_record_serie"): "read_record_serie",
    ("media", "cache", "now"): "cache_now",
    ("temporal", "format_datetime"): "format_datetime",
    ("temporal", "parse_date"): "parse_date",
    ("temporal", "parse_time"): "parse_time",
    ("temporal", "scalars", "nanoseconds_per"): "nanoseconds_per",
    ("text", "ERROR_TEXT_LIMIT"): "ERROR_TEXT_LIMIT",
    ("text", "elide_to"): "elide_to",
    ("text", "expected_got"): "expected_got",
    ("text", "prepare_text"): "with_field",
    ("xml", "decode_x_escapes"): "decode_x_escapes",
    ("xml", "write_attribute_text"): "write_attribute_text",
    ("xml", "write_element_text"): "write_element_text",
    ("xml", "write_leaf_text"): "write_leaf_text",
    ("xml", "write_x_escape"): "write_x_escape",
}


class ExcelTables:
    """`s4.Rewriter`'s resolution, for one module leaving the core."""

    def __init__(self, public: set[str]) -> None:
        self.public = public
        # Inside the crate an Excel path is the crate's own module path.
        self.inside = False

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        if segs[0] == "excel":
            rest = segs[1:]
            if not self.inside and rest and rest[0] not in self.public:
                residue("paths", f"`excel::{'::'.join(rest)}` names nothing the crate root publishes")
            return EXCEL, rest
        for cut in range(len(segs), 0, -1):
            name = CORE_ROUTES.get(tuple(segs[:cut]))
            if name:
                return CORE, ["implementer", name, *segs[cut:]]
        return CORE, segs


def excel_public(text: str) -> set[str]:
    """The names the Excel module publishes at its root."""
    names = set(re.findall(r"(?m)^pub mod (\w+);", text))
    names |= set(re.findall(r"(?m)^pub (?:const|fn|static|struct|enum) (\w+)", text))
    for m in re.finditer(r"(?m)^pub use \w+::(\{[^;]*\}|\w+);", text):
        names.update(s4.Tables.brace_names(m.group(1)) if m.group(1).startswith("{") else [m.group(1)])
    return names | {"install", "internals"}


ALIAS_USE = re.compile(r"(?m)^[ \t]*(?:#[ \t]+)?use yggdryl_excel as excel;[ \t]*\n")
EXCEL_ROOTED = re.compile(r"(?<![\w:$])excel::")


def unalias(text: str) -> str:
    """`use yggdryl_excel as excel;` dropped and every `excel::` path in code
    spelled `yggdryl_excel::`: the module a caller imported whole is its crate."""
    if not ALIAS_USE.search(text):
        return text
    text = ALIAS_USE.sub("", text)
    code = code_only(text)
    out, pos = [], 0
    for m in EXCEL_ROOTED.finditer(code):
        out.append(text[pos:m.start()])
        out.append("yggdryl_excel::")
        pos = m.end()
    out.append(text[pos:])
    return "".join(out)


# ---------------------------------------------------------------------------
# The move table and file paths
# ---------------------------------------------------------------------------


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    moves: list[tuple[str, str]] = []
    for f in tracked(root, "rust/src/excel", "rust/tests/excel", "rust/tests/excel.rs",
                     "rust/tests/interop/excel.rs", "rust/benchmarks/media/excel.rs"):
        if f.startswith("rust/src/excel/"):
            rel = f[len("rust/src/excel/"):]
            new = "rust/excel/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        elif f == "rust/tests/excel.rs" or f.startswith("rust/tests/excel/"):
            new = "rust/excel/tests/" + f[len("rust/tests/"):]
        elif f == "rust/tests/interop/excel.rs":
            new = "rust/excel/tests/interop/excel.rs"
        else:
            new = "rust/excel/benchmarks/excel.rs"
        moves.append((f, new))
    return moves


class PathMap(av.PathMap):
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/excel/", "rust/excel/src/"),
            ("rust/tests/excel/", "rust/excel/tests/excel/"),
        ]

    def with_files(self, extra: dict[str, str]) -> "PathMap":
        other = PathMap([])
        other.files = {**self.files, **extra}
        other.dirs = self.dirs
        return other


s4.manifest_dir = av.manifest_dir


def rewrite_text_paths(text: str, paths: PathMap, moves: list[tuple[str, str]]) -> str:
    for old, new in sorted(moves, key=lambda pair: -len(pair[0])):
        if old in text:
            text = re.sub(r"(?<![\w./-])" + re.escape(old) + r"(?![\w-])", new, text)
    for old, new in paths.dirs:
        text = re.sub(r"(?<![\w./-])" + re.escape(old), new, text)
    text = re.sub(r"(?<![\w./-])rust/src/excel(?![\w/.-])", "rust/excel/src", text)
    text = re.sub(r"(?<![\w./-])rust/tests/excel(?![\w/.-])", "rust/excel/tests/excel", text)
    # A command running the Excel harness runs the crate's.
    text = re.sub(r"(?m)(-p )yggdryl( [^\n]*?--test excel\b)", r"\1yggdryl-excel\2", text)
    return text


# ---------------------------------------------------------------------------
# Step 1: the core's tests that reach Excel, item by item (S4's split)
# ---------------------------------------------------------------------------

# Files whose `.xlsx` names are the routing vocabulary alone - a suffix a
# MIME type is inferred from, a location's spelling - or Excel the client of
# the XML for Analysis provider, never a workbook read through the register:
# they stay.
SPLIT_EXCLUDED = (
    "rust/tests/excel/", "rust/tests/excel.rs", "rust/tests/interop/", "rust/tests/support/",
    "rust/tests/xmla/", "rust/tests/mime_type/", "rust/tests/root/mime_type.rs", "rust/tests/uri/",
    "rust/tests/iceberg/", "rust/tests/s3/", "rust/tests/s3tables",
)
EXCEL_PATH = re.compile(r"\byggdryl(?:_excel\b|::excel\b|::internals::excel_)|\bMimeType::XLSX\b")


def literal_reads_excel(literal: str) -> bool:
    body = re.sub(r'^b?r?#*"|"#*$', "", literal)
    return bool(re.search(r"\.xlsx\b", body)) or body in ("excel", XLSX_TYPE)


def file_imports(items: list) -> set[str]:
    """Names a level imports from `yggdryl::excel`."""
    names: set[str] = set()
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            if leaf[:2] == ["yggdryl", "excel"] and kind == "self":
                names.add(alias or leaf[-1])
    return names


def reaches_excel(text: str, code: str) -> bool:
    if EXCEL_PATH.search(code):
        return True
    return any(literal_reads_excel(m.group(0)) for m in s4.STRING_LITERAL.finditer(text))


def item_owners(items: list, inherited: set[str]) -> tuple[dict[int, str], dict[int, set[int]]]:
    av.macro_names(items)
    imports = inherited | file_imports(items)
    body = [item for item in items if item.kind != "use"]
    code = {id(item): code_only(item.text) for item in body}
    tokens = {id(item): set(re.findall(r"[A-Za-z_]\w*", code[id(item)])) for item in body}
    by_name: dict[str, list] = collections.defaultdict(list)
    for item in body:
        for name in item.names:
            by_name[name].append(item)
    refs = {
        id(item): {id(other) for name in tokens[id(item)] for other in by_name.get(name, []) if other is not item}
        for item in body
    }
    owner: dict[int, str] = {}
    for item in body:
        reaches = reaches_excel(item.text, code[id(item)]) or bool(tokens[id(item)] & imports)
        owner[id(item)] = EXCEL if reaches else CORE
    by_id = {id(item): item for item in body}
    changed = True
    while changed:
        changed = False
        for item in body:
            if owner[id(item)] == CORE and any(owner[other] == EXCEL for other in refs[id(item)]
                                               if inline_body(by_id[other]) is None):
                owner[id(item)] = EXCEL
                changed = True
    return owner, refs


def split_level(text: str, top: bool, inherited: set[str]) -> tuple[str, str | None, int]:
    """S4's `split_level` with Excel the one crate above the core."""
    head, items = top_items(text)
    imports = inherited | file_imports(items)
    owner, refs = item_owners(items, inherited)
    moved = 0
    dest: dict[int, str] = {}
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    for item in items:
        if item.kind == "use" or owner[id(item)] != EXCEL:
            continue
        body = inline_body(item)
        if body is not None:
            start, end = body
            inner_home, inner_dest, n = split_level(item.text[start:end], False, imports)
            moved += n
            if inner_dest is None:
                continue
            if not any(i.kind != "use" for i in top_items(inner_home)[1]):
                dropped.add(id(item))
            else:
                replaced[id(item)] = item.text[:start] + inner_home + item.text[end:]
            dest[id(item)] = item.text[:start] + inner_dest + item.text[end:]
            continue
        dropped.add(id(item))
        dest[id(item)] = item.text
        moved += 1
    if not dest:
        return text, None, 0
    body_items = [item for item in items if item.kind != "use"]
    always = {id(item) for item in body_items if top and "#[global_allocator]" in item.text}
    containers = {id(item) for item in body_items if inline_body(item) is not None}

    def closure(seed: set[int]) -> set[int]:
        found, stack = set(seed), list(seed)
        while stack:
            for other in refs[stack.pop()]:
                if other not in found and other not in containers:
                    found.add(other)
                    stack.append(other)
        return found

    names = {n for text_ in dest.values() for n in re.findall(r"[A-Za-z_]\w*", code_only(text_))}
    seed = {
        id(item) for item in body_items
        if item.names & names and id(item) not in dropped and id(item) not in replaced and id(item) not in containers
    }
    helpers = closure(seed | always) - set(dest)
    out = [head]
    for item in items:
        if item.kind == "use":
            out.append(item.text)
        elif id(item) in dest:
            out.append(dest[id(item)])
        elif id(item) in helpers:
            out.append(item.text)
    excel_text = "".join(out).rstrip("\n") + "\n"
    # What stays is what a remaining test, module or published item names: a
    # helper only the moved tests used leaves the core rather than dying there.
    roots = {
        id(item) for item in body_items
        if id(item) not in dropped
        and (re.search(r"#\[test\]|#\[global_allocator\]", item.text) or inline_body(item) is not None
             or av.published(item.text))
    }
    stay = closure(roots) | set(replaced)
    out = [head]
    for item in items:
        if item.kind == "use":
            out.append(item.text)
        elif id(item) in replaced:
            out.append(replaced[id(item)])
        elif id(item) in stay:
            out.append(item.text)
    return "".join(out).rstrip("\n") + "\n", excel_text, moved


def kept_for_calls(text: str) -> list[str]:
    """The top-level `use` leaves of `text` named nowhere else in it but kept
    because a method a trait of that name declares is called: the avro
    define's prune keeps them, and a common method name keeps one the file
    does not need, which the compiler loop drops as `unused_imports`."""
    _, items = top_items(text)
    code = "".join(code_only(item.text) for item in items if item.kind != "use")
    tokens = set(re.findall(r"[A-Za-z_]\w*", code))
    kept = []
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            name = alias or (leaf[-1] if leaf else "")
            if kind == "self" and name != "_" and name not in tokens and leaf and leaf[-1] in av.TRAIT_METHODS:
                kept.append(name)
    return kept


def method_calls(text: str, trait: str) -> int:
    """How many calls `text` makes to a method the trait `trait` declares."""
    code = code_only(text)
    return sum(len(re.findall(rf"\.\s*{re.escape(m)}\s*(?:::<[^()]*>)?\s*\(", code))
               for m in av.TRAIT_METHODS.get(trait, ()))


def note_kept_traits(root: pathlib.Path, splits: list[tuple[str, str]]) -> None:
    """A trait import a split kept for a method call, where the split moved
    such calls: every one in a new file, and in a core file one whose calls
    the split took (a trait the file imported for calls it still makes is as
    it was)."""
    for core, dest in splits:
        before = git(root, "show", f"HEAD:{core}", check=False)
        for rel in (core, dest):
            if not (root / rel).exists():
                continue
            text = read(root / rel)
            kept = kept_for_calls(text)
            if rel == core:
                kept = [k for k in kept if method_calls(text, k) < method_calls(before, k)]
            if kept:
                residue(rel, f"trait import(s) {', '.join(f'`{k}`' for k in kept)} kept only for a method call: "
                             "the compiler loop drops each it finds unused")


def split_tests(root: pathlib.Path, moves: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Every core test file's items reaching Excel, moved byte-identical into
    `rust/excel/tests/` at the same relative path (S4's split, D39)."""
    move_map = dict(moves)
    written: list[tuple[str, str]] = []
    needs: dict[str, list[str]] = collections.defaultdict(list)
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED):
            continue
        rel = f[len("rust/tests/"):]
        single = "/" not in rel
        home_text, excel_text, moved = split_level(read(root / f), True, set())
        if excel_text is None:
            continue
        dest_rel = f"{CRATE_DIR}/tests/{rel}"
        if (root / dest_rel).exists() or dest_rel in move_map.values():
            residue(dest_rel, f"a file of this name exists or arrives by a move; the split tests of `{f}` were not written")
            continue
        pinned = av.pinned_source(rel)
        what = "rows" if single else "tests"
        header = (
            f"//! The {what} of `{f}`"
            + (f", which pins `{pinned}`," if pinned else "")
            + f" that need\n//! `{PACKAGE}`, moved byte-identical (S6, D39): a test lives in the lowest\n"
            "//! crate that can name everything it uses.\n//!\n"
        )
        write(root / dest_rel, tidy_head(prune_imports(prune_imports(header + excel_text))))
        written.append((f, dest_rel))
        if not single:
            needs["rust/tests/" + rel.split("/", 1)[0] + ".rs"].append(f)
        write(root / f, tidy_head(prune_imports(prune_imports(home_text))))
        done(f"{f}: {moved} item(s) moved to {dest_rel}")
    for harness, modules in sorted(needs.items()):
        if not (root / harness).exists():
            residue(harness, "the harness of a split module is missing")
            continue
        text = read(root / harness)
        head, items = top_items(text)
        wanted: set[str] = set()
        decls = []
        for module in modules:
            relpath = module[len("rust/tests/"):]
            decl = next((item for item in items if item.kind == "mod" and f'"{relpath}"' in item.text), None)
            if decl is None:
                residue(harness, f"no `#[path]` declaration of `{relpath}` to copy into the {PACKAGE} harness")
                continue
            decls.append(id(decl))
            wanted |= set(re.findall(r"[A-Za-z_]\w*", code_only(read(root / CRATE_DIR / "tests" / relpath))))
        helpers = {
            id(item) for item in items
            if item.kind not in ("use", "mod") or (item.kind == "mod" and '"support/' in item.text)
        }
        chosen = {id(item) for item in items if id(item) in helpers and (item.names & wanted)}
        names = {n for item in items if id(item) in chosen for n in re.findall(r"[A-Za-z_]\w*", code_only(item.text))}
        chosen |= {id(item) for item in items if id(item) in helpers and item.names & names}
        dest = f"{CRATE_DIR}/tests/{posixpath.basename(harness)}"
        parts = [
            f"//! The `{PACKAGE}` half of `{harness}`: the modules whose tests need\n"
            f"//! `{PACKAGE}`, split off by S6 (D39), with the helpers they name.\n//!\n" + head
        ]
        parts += [item.text for item in items if item.kind == "use" or id(item) in chosen or id(item) in decls]
        if (root / dest).exists() or dest in move_map.values():
            residue(dest, f"the harness exists or arrives by a move: declare {', '.join(modules)} in it by hand")
            continue
        write(root / dest, tidy_head(prune_imports("".join(parts).rstrip("\n") + "\n")))
        written.append((harness, dest))
    return written


def edit_media_register(root: pathlib.Path) -> None:
    """The claim rule's two refusals, pinned in the core over test-only media
    (the avro define's statics and test, byte for byte)."""
    rel = "rust/tests/media_register.rs"
    path = root / rel
    if not path.exists():
        residue(rel, "absent: pin the reserved-rank refusals by hand")
        return
    text = read(path)
    if "a_reserved_rank_is_claimed_under_its_own_name_alone" in text:
        return
    text = replace_once(text, "/// A medium naming no MIME type.\nstatic NO_TYPE_CODEC: TestCodec",
                        av.RESERVED_TEST_STATICS + "/// A medium naming no MIME type.\nstatic NO_TYPE_CODEC: TestCodec", rel)
    write(path, text.rstrip("\n") + "\n" + av.RESERVED_TEST)


# ---------------------------------------------------------------------------
# Step 3: the bench
# ---------------------------------------------------------------------------


def edit_core_media_bench(root: pathlib.Path) -> None:
    edit(
        root, "rust/benchmarks/media.rs",
        ('#[path = "media/excel.rs"]\nmod excel;\n', ""),
        ("    excel::excel_benchmarks,\n", ""),
    )


BENCH_DOC = """//!
//! `yggdryl-excel`'s `excel` target: `main` claims the medium before the
//! group runs, so a handle whose name declares a workbook reads through the
//! core's register.

#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;
"""

BENCH_MAIN = """
criterion::criterion_group!(excel, excel_benchmarks);

fn main() {
    // The medium the benchmarks read by name is claimed before any runs.
    yggdryl_excel::install().expect("yggdryl-excel installs");
    excel();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
"""


def build_bench_root(root: pathlib.Path) -> None:
    """The moved group file is the crate's bench root: the shared corpus
    profile by `#[path]`, the group, and a `main` that installs."""
    rel = f"{CRATE_DIR}/benchmarks/excel.rs"
    path = root / rel
    if not path.exists():
        residue(rel, "the moved bench file is missing: write the bench root by hand")
        return
    text = read(path)
    m = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if not m or "excel_benchmarks" not in text:
        residue(rel, "no crate doc or no `excel_benchmarks` to build the bench root around")
        return
    text = text[:m.end()] + BENCH_DOC + text[m.end():]
    write(path, text.rstrip("\n") + "\n" + BENCH_MAIN)


# ---------------------------------------------------------------------------
# Step 5: the crate's own root
# ---------------------------------------------------------------------------

LIB_HEAD = """//! Office Open XML spreadsheets (`.xlsx`) for the yggdryl core: a workbook
//! of worksheets as record media, and as cells a caller reaches one by one -
//! the `yggdryl-excel` crate.
//!
//! The record medium is [`EXCEL_CODEC`], claimed on the core's register of
//! media by [`install`] at the rank the core held it at: every binding's init
//! and the `yggdryl` command's `main` call it, and a Rust program that links
//! this crate calls it before the core reads a handle whose name declares a
//! workbook, which is refused naming the crate to install until then.
//! [`Excel`], [`ExcelOptions`], [`Workbook`], [`Sheet`] and [`Cell`] answer
//! without it.
//!
"""

INSTALL_FN = '''
/// What this crate claims the Excel medium as.
const CRATE: &str = "yggdryl-excel";

/// Claims the Excel medium on the core's register of media - [`EXCEL_CODEC`]
/// under the spreadsheetml sheet MIME type, at rank 6, which
/// [`RESERVED_RANKS`](yggdryl::media::RESERVED_RANKS) keeps for it since the
/// core held it there - once for the life of the process; a later call
/// returns at once. Every binding's init and the `yggdryl` command's `main`
/// call it, and so does a Rust caller before the core reads a handle whose
/// name declares a workbook: until the claim such a handle's options are
/// refused, naming the crate to install.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed the
/// workbook's MIME type or the name `excel` first.
pub fn install() -> yggdryl::Result<()> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    static INSTALLING: Mutex<()> = Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    let _installing = INSTALLING.lock().unwrap_or_else(PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    yggdryl::media::codec::claim(&EXCEL_CODEC, CRATE)?;
    let _ = INSTALLED.set(());
    Ok(())
}
'''


def build_lib(root: pathlib.Path) -> None:
    """`mod.rs` as the crate root: its doc led by the crate's, its module
    declarations, re-exports, constants and the coding refusal as they were,
    then `install()` and the generated `internals` block."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    path = root / rel
    text = read(path)
    m = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if not m:
        residue(rel, "no crate doc to replace")
        return
    paragraphs = m.group(0).split("//!\n")
    # The first paragraph is what the crate's head restates.
    tail = "//!\n".join(paragraphs[1:])
    tail = replace_once(tail, "read through the crate's own\n//! [`zip`]", "read through the core's own\n//! [`zip`]", rel)
    body = text[m.end():].lstrip("\n")
    write(
        path,
        LIB_HEAD + tail.rstrip("\n") + "\n\n#![deny(unsafe_code)]\n\n"
        + "use std::sync::{Mutex, OnceLock, PoisonError};\n\n"
        + body.rstrip("\n") + "\n" + INSTALL_FN + "\n" + av.INTERNALS_PLACEHOLDER,
    )


# ---------------------------------------------------------------------------
# Step 6: the core's seams
# ---------------------------------------------------------------------------


def edit_codec(root: pathlib.Path) -> None:
    """The seed loses Excel; `RESERVED_RANKS` and the claim rule (D39) - the
    avro define's text, so the media that leave after Excel find it said."""
    rel = "rust/src/media/codec.rs"
    pairs = [("            &crate::excel::EXCEL_CODEC,\n", "")]
    if "pub const RESERVED_RANKS" not in read(root / rel):
        pairs += [
            ("/// The rank a medium outside the core takes at least: the core's seven hold\n"
             "/// the positions below it, in the order their options sort.\n"
             "pub const EXTERNAL_RANK: u8 = 32;\n",
             "/// The rank a medium outside the core takes at least: the positions below\n"
             "/// it are the core's own media's and the [`RESERVED_RANKS`] of the media\n"
             "/// split off it, in the order their options sort.\n"
             "pub const EXTERNAL_RANK: u8 = 32;\n" + av.RESERVED),
            ("    /// Where the medium's options sort among every medium's: the core's are\n"
             "    /// `ipc` 0, `parquet` 1, `avro` 2, `text` 3, `xmla` 4, `csv` 5 and\n"
             "    /// `excel` 6; a medium outside the core takes [`EXTERNAL_RANK`] or more.\n",
             "    /// Where the medium's options sort among every medium's: `ipc` 0,\n"
             "    /// `parquet` 1, `avro` 2, `text` 3, `xmla` 4, `csv` 5 and `excel` 6 -\n"
             "    /// those a crate split off the core holds kept as [`RESERVED_RANKS`] -\n"
             "    /// and any other medium [`EXTERNAL_RANK`] or more.\n"),
            ("/// is claimed already, and [`Error::InvalidRecord`] at `$.encoding` for a\n"
             "/// claim in the core's own name, a codec naming no MIME type, or a rank\n"
             "/// below [`EXTERNAL_RANK`].\n",
             "/// is claimed already, and [`Error::InvalidRecord`] at `$.encoding` for a\n"
             "/// claim in the core's own name, a codec naming no MIME type, a reserved\n"
             "/// rank under another name or a reserved name at another rank\n"
             "/// ([`RESERVED_RANKS`]), or any other rank below [`EXTERNAL_RANK`].\n"),
            (av.CLAIM_OLD, av.CLAIM_NEW),
        ]
    edit(root, rel, *pairs)
    if "RESERVED_RANKS" not in read(root / "rust/src/media/mod.rs"):
        edit(root, "rust/src/media/mod.rs",
             ("pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper,",
              "pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper, RESERVED_RANKS,"))


IMPLEMENTER_SECTION = """
// ------------------------------------------------------------------------
// Media: what the media crates reach - the record doors a wrapper redirects
// to, the dimensions and the schema it answers through, the cache clock it
// stamps its entry with, and the value and text helpers one medium's codec
// reads.
// ------------------------------------------------------------------------

/// The default record writes a media wrapper redirects to once its options
/// are proven its own, for the media crates: append and merge over a leaf or
/// a folder, an overwrite answering the field it published, and the one
/// encode of a leaf's whole contents.
pub use crate::iobase::{
    append_arrow_reader_default, leaf_writer, merge_arrow_reader_default,
    overwrite_arrow_reader_default_with_field,
};

/// An owned handle on the resource a borrowed one addresses, for the Excel
/// crate's package: reopened at its location where it has one, else copied
/// once.
pub use crate::iobase::hierarchy::owned_handle;

/// What a media wrapper answers its options, its origin, its dimensions and
/// its schema through, for the media crates: the handle's own options where
/// none were given, a container's root, origin and row count, the options a
/// dimension is read under, the one schema answer under the options'
/// clauses, and the one composed record read.
pub use crate::iomedia::{
    container_field, container_origin, container_row_size, dimension_options, field_under,
    own_options, read_record_serie,
};

/// The temporal family a cell's datatype is read by, and the nanoseconds one
/// count of a fixed-length unit holds, for the Excel crate.
pub use crate::temporal::scalars::{TemporalKind, nanoseconds_per};

/// `DataTypeId::temporal_kind`, for the Excel crate: the temporal family an
/// identifier is a leaf of, `None` outside the temporal range.
#[inline]
pub const fn datatype_id_temporal_kind(id: crate::DataTypeId) -> Option<TemporalKind> {
    id.temporal_kind()
}

/// `media::cache::now`, for the media crates: the instant every cache door
/// reads, which a test sets by hand under `internals`.
#[inline]
#[must_use]
pub fn cache_now() -> std::time::Instant {
    crate::media::cache::now()
}

/// `arrow::arrow_schema_from_field`, for the media crates: the Arrow schema a
/// record root projects to, its dictionary-ID sidecar written.
///
/// # Errors
///
/// Returns an error when the root is not a record or a child has no Arrow
/// projection.
#[inline]
pub fn arrow_schema_from_field(field: &Field) -> crate::arrow::Result<arrow_schema::SchemaRef> {
    crate::arrow::arrow_schema_from_field(field)
}

/// `floating::f64_from_text`, for the Excel crate: a float read out of its
/// canonical spelling through the one grammar every float in the crate reads.
#[inline]
pub fn f64_from_text(text: &str) -> Option<f64> {
    crate::floating::f64_from_text(text)
}

/// `temporal::format_datetime`, for the Excel crate: a naive reading spelled
/// `YYYY-MM-DDTHH:MM:SS[.fraction]`, `None` for a unit with no clock.
#[inline]
pub fn format_datetime(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    crate::temporal::format_datetime(count, unit)
}

/// `temporal::parse_date`, for the Excel crate: `YYYY-MM-DD` or `YYYYMMDD`
/// read into a day count since the Unix epoch.
///
/// # Errors
///
/// Returns the ISO 8601 date reader's refusal, trailing text included.
#[inline]
pub fn parse_date(text: &str) -> Result<i32> {
    crate::temporal::parse_date(text)
}

/// `temporal::parse_time`, for the Excel crate: `HH:MM:SS[.fraction]` read
/// into a count of the unit its digits spell since midnight.
///
/// # Errors
///
/// Returns the ISO 8601 clock reader's refusal, trailing text included.
#[inline]
pub fn parse_time(text: &str) -> Result<(i64, TimeUnit)> {
    crate::temporal::parse_time(text)
}

/// `xml::write_leaf_text`, for the Excel crate: one leaf value written as an
/// element's character data, in the spelling the XML writer gives it.
///
/// # Errors
///
/// Returns the writer's refusal of a value that is not a leaf, or of a leaf
/// with no XML spelling, named by `context`.
#[inline]
pub fn write_leaf_text<W: std::io::Write>(
    writer: &mut W,
    value: &Scalar,
    context: &str,
) -> Result<()> {
    crate::xml::write_leaf_text(writer, value, context)
}

/// `xml::write_element_text`, for the Excel crate: text written as escaped
/// element character data.
///
/// # Errors
///
/// Returns the writer's refusal of a character XML 1.0 cannot carry.
#[inline]
pub fn write_element_text<W: std::io::Write>(writer: &mut W, text: &str) -> Result<()> {
    crate::xml::write_element_text(writer, text)
}

/// `xml::write_attribute_text`, for the Excel crate: text written as a
/// quoted attribute's escaped value, without the quotes.
///
/// # Errors
///
/// Returns the writer's refusal of a character XML 1.0 cannot carry.
#[inline]
pub fn write_attribute_text<W: std::io::Write>(writer: &mut W, text: &str) -> Result<()> {
    crate::xml::write_attribute_text(writer, text)
}

/// `xml::write_x_escape`, for the Excel crate: one character spelled as the
/// `_xHHHH_` escape Office Open XML gives a character a name or a string
/// cannot carry.
#[inline]
pub fn write_x_escape(target: &mut String, character: char) {
    crate::xml::write_x_escape(target, character);
}

/// `xml::decode_x_escapes`, for the Excel crate: every `_xHHHH_` escape read
/// back into the character it spells, text carrying none borrowed.
#[inline]
pub fn decode_x_escapes(encoded: &str) -> Cow<'_, str> {
    crate::xml::decode_x_escapes(encoded)
}

/// `ZipArchive::member_reader`, for the Excel crate: one member of an
/// archive streamed by its path, `None` where the archive holds none.
///
/// # Errors
///
/// Returns the read or format failure the index parse hit, or the refusal of
/// an encrypted member or a compression method this build cannot decode.
#[inline]
pub fn zip_archive_member_reader(
    archive: &std::sync::Arc<crate::zip::ZipArchive>,
    path: &str,
) -> Result<Option<Box<dyn std::io::Read + Send>>> {
    archive.member_reader(path)
}
"""

TRANSFER_NAMES = [
    "append_arrow_reader_default", "leaf_writer", "merge_arrow_reader_default",
    "overwrite_arrow_reader_default_with_field",
]
IOMEDIA_NAMES = [
    "container_field", "container_origin", "container_row_size", "dimension_options", "field_under",
    "own_options", "read_record_serie",
]


def edit_implementer(root: pathlib.Path) -> None:
    rel = "rust/src/implementer.rs"
    text = read(root / rel)
    if "pub fn zip_archive_member_reader(" in text:
        return
    write(root / rel, text.rstrip("\n") + "\n" + IMPLEMENTER_SECTION)
    # R: the raises behind the `pub use`s, each inside a module the crate root
    # does not publish, so the raise publishes nothing else.
    av.raise_in(root, "rust/src/iobase/transfer.rs", TRANSFER_NAMES)
    iobase = read(root / "rust/src/iobase.rs")
    iobase = av.move_use_names(
        iobase,
        re.compile(r"(pub\(crate\) use transfer::)\{([^}]*)\};"),
        TRANSFER_NAMES,
        "pub use transfer::{",
        "rust/src/iobase.rs",
    )
    write(root / "rust/src/iobase.rs", iobase)
    av.raise_in(root, "rust/src/iobase/hierarchy.rs", ["owned_handle"])
    av.raise_in(root, "rust/src/iomedia.rs", IOMEDIA_NAMES)
    # A raised item's doc names no private item by link.
    edit(
        root, "rust/src/iomedia.rs",
        ("/// Returns what [`container_field`] returns.\n", "/// Returns what `container_field` returns.\n"),
        ("/// [`compose`](crate::media_serie::compose): the medium's native reader is\n",
         "/// `compose`: the medium's native reader is\n"),
    )
    edit(
        root, "rust/src/temporal.rs",
        ("    pub(crate) enum TemporalKind {", "    pub enum TemporalKind {"),
        ("    pub(crate) const fn nanoseconds_per(unit: TimeUnit)", "    pub const fn nanoseconds_per(unit: TimeUnit)"),
    )


def edit_core_lib_and_manifest(root: pathlib.Path) -> None:
    edit(root, "rust/src/lib.rs", ("pub mod excel;\n", ""))
    rel = "rust/Cargo.toml"
    text = read(root / rel)
    head = text.split("[features]")[0]
    if "exclude = [" in head:
        m = re.search(r"exclude = \[([^\]]*)\]", text)
        if '"/excel"' not in m.group(1):
            text = text[:m.start(1)] + m.group(1) + ', "/excel"' + text[m.end(1):]
    else:
        text = replace_once(
            text,
            'categories = ["data-structures", "encoding"]\n',
            'categories = ["data-structures", "encoding"]\n'
            "# The split crates sit inside this folder; each is its own package.\n"
            'exclude = ["/excel"]\n',
            rel,
        )
    write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 7: the four calls the path rewrite cannot see
# ---------------------------------------------------------------------------

# A method of a core type that is crate-private reads as a method call, never
# as a path: each is re-spelled through the forwarder `implementer` gives it.
CALL_EDITS: dict[str, list[tuple[str, str, int]]] = {
    "src/workbook.rs": [
        ("        archive\n            .member_reader(part)?\n",
         "        yggdryl::implementer::zip_archive_member_reader(archive, part)?\n", 1),
    ],
    "src/writer.rs": [
        ("Kind::Text if child.is_string_storage() && utf8_stored(child) => {",
         "Kind::Text if yggdryl::implementer::serie_is_string_storage(child) && utf8_stored(child) => {", 1),
        ("let Some(bytes) = child.value_bytes(index) else {",
         "let Some(bytes) = yggdryl::implementer::serie_value_bytes(child, index) else {", 1),
        ("        if dtype\n            .id()\n            .temporal_kind()\n",
         "        if yggdryl::implementer::datatype_id_temporal_kind(dtype.id())\n", 1),
    ],
    "src/cell.rs": [
        ("match dtype.id().temporal_kind() {",
         "match yggdryl::implementer::datatype_id_temporal_kind(dtype.id()) {", 2),
        ("if is_text_target || dtype.id().temporal_kind().is_some() {",
         "if is_text_target || yggdryl::implementer::datatype_id_temporal_kind(dtype.id()).is_some() {", 1),
    ],
}


def edit_calls(root: pathlib.Path) -> None:
    count = 0
    for rel, edits in CALL_EDITS.items():
        path = root / CRATE_DIR / rel
        text = read(path)
        for old, new, n in edits:
            before = text
            text = av.replace_n(text, old, new, n, f"{CRATE_DIR}/{rel}")
            count += n if text != before else 0
        write(path, text)
    left = []
    for path in sorted((root / CRATE_DIR / "src").glob("*.rs")):
        code = code_only(read(path))
        for name in ("member_reader", "temporal_kind", "is_string_storage", "value_bytes"):
            for m in re.finditer(rf"\.{name}\s*\(", code):
                left.append(f"{path.relative_to(root)}:{line_of(code, m.start())}")
    for where in left:
        residue(where, "a crate-private core method is still called directly")
    done(f"calls: {count} method call(s) on core types re-spelled through their forwarders")


# ---------------------------------------------------------------------------
# Step 7: paths, everywhere
# ---------------------------------------------------------------------------

EXCEL_NAMED = re.compile(r"\byggdryl::(?:excel\b|internals::excel_|\{[^;]*\bexcel\b)")
DOCS_RS = re.compile(r"https://docs\.rs/yggdryl/latest/yggdryl/excel/")


def rewrite_paths(root: pathlib.Path, rewriter, paths: PathMap, moves, splits) -> None:
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new[new] = old
    crate_ctx = s4.Context(EXCEL, True)
    outside = s4.Context(None, False)
    local = paths.with_files({o: n for o, n in splits})
    count = 0
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        old = moved_new.get(rel, rel)
        text = read(path)
        new = s4.reanchor(text, old, rel, local)
        inside = rel.startswith(f"{CRATE_DIR}/src/")
        rewriter.t.inside = inside
        new = unalias(rewriter.rs_file(new, crate_ctx if inside else outside, rel))
        rewriter.t.inside = False
        if new != text:
            write(path, new)
            count += 1
    for folder in ("rust/src", "rust/tests", "rust/benchmarks", "python/src", "node/src", "cli"):
        base = root / folder
        if not base.exists():
            continue
        for path in sorted(base.rglob("*.rs")):
            rel = str(path.relative_to(root))
            if "/target/" in rel:
                continue
            text = read(path)
            if folder == "rust/src":
                new = s4.reanchor(text, rel, rel, paths)
            else:
                new = unalias(rewriter.rs_file(s4.reanchor(text, rel, rel, paths), outside, rel))
            if new != text:
                write(path, new)
                count += 1
    # A sibling leaf's sources and tests (after S4): what they name of Excel.
    for leaf in sorted((root / "rust").glob("*/Cargo.toml")):
        if leaf.parent.name in av.CORE_FOLDERS or leaf.parent.name == EXCEL:
            continue
        for path in sorted(leaf.parent.rglob("*.rs")):
            rel = str(path.relative_to(root))
            if "/target/" in rel:
                continue
            text = read(path)
            new = unalias(rewriter.rs_file(text, outside, rel))
            if new != text:
                write(path, new)
                count += 1
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text = read(root / f)
            new = text
            if EXCEL_NAMED.search(text):
                new = markdown_unalias(markdown(rewriter, text, f))
            new = DOCS_RS.sub("https://docs.rs/yggdryl-excel/latest/yggdryl_excel/", new)
            if new != text:
                write(root / f, new)
                count += 1
    for f in ("AGENTS.md", "README.md", "rust/README.md", ".api-inventory.txt"):
        if (root / f).exists():
            text = read(root / f)
            new = DOCS_RS.sub("https://docs.rs/yggdryl-excel/latest/yggdryl_excel/", rewriter.inline(text, outside, f))
            if new != text:
                write(root / f, new)
                count += 1
    done(f"paths: {rewriter.stats['use']} use statements and {rewriter.stats['inline']} paths re-owned, {count} files")
    count = 0
    for f in av.text_files(root):
        text = read(root / f)
        new = rewrite_text_paths(text, paths, moves)
        if new != text:
            write(root / f, new)
            count += 1
    done(f"file paths re-pointed in {count} files")


def markdown(rewriter, text: str, where: str) -> str:
    """S4's markdown rewrite, touching only the Rust blocks and the prose that
    name Excel, and refusing a block whose lines do not all carry its indent -
    a string continued at column 0 is content, never indentation."""
    outside = s4.Context(None, False)
    out: list[str] = []
    pos = 0
    for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
        prose = text[pos:m.start(3)]
        out.append(rewriter.inline(prose, outside, where) if EXCEL_NAMED.search(prose) else prose)
        body = m.group(3)
        if m.group(2).strip().startswith("rust") and EXCEL_NAMED.search(body):
            indent = m.group(1)
            lines = body.split("\n")
            if indent and not all(l.startswith(indent) or not l.strip() for l in lines):
                residue(f"{where}:{line_of(text, m.start(3))}", "a Rust block with a line outside its indent names Excel: re-spell it by hand")
            elif indent:
                stripped = "\n".join(l[len(indent):] for l in lines)
                new = rewriter.rust(stripped, outside, where)
                body = "\n".join((indent + l) if l else l for l in new.split("\n"))
            else:
                body = rewriter.rust(body, outside, where)
        elif EXCEL_NAMED.search(body):
            body = rewriter.inline(body, outside, where)
        out.append(body)
        pos = m.end(3)
    rest = text[pos:]
    out.append(rewriter.inline(rest, outside, where) if EXCEL_NAMED.search(rest) else rest)
    return "".join(out)


def markdown_unalias(text: str) -> str:
    def block(m: re.Match) -> str:
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or "yggdryl_excel as excel" not in body:
            return m.group(0)
        if indent:
            stripped = "\n".join(l[len(indent):] if l.startswith(indent) else l for l in body.split("\n"))
            new = unalias(stripped)
            body = "\n".join((indent + l) if l else l for l in new.split("\n"))
        else:
            body = unalias(body)
        return f"{indent}```{info}\n{body}{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text)


EXCEL_HARNESS_DOC = """//! One test file per file under `rust/excel/src/`, under `tests/excel/`.
//!
//! `rust/excel/tests/` mirrors `rust/excel/src/`: a source file has exactly
//! one test file at the matching path, and this target is the harness for the
//! Excel medium - the workbook, its sheets and cells, and the record path
//! over a worksheet. A test reaches the crate through `yggdryl_excel::` and
//! the core through `yggdryl::`, and nothing else; the hand-written package
//! the readers are fed is the core's `rust/tests/support/excel_package.rs`.
"""


def finish_crate_text(root: pathlib.Path) -> None:
    """The moved harness's sentences about itself."""
    path = root / CRATE_DIR / "tests/excel.rs"
    if not path.exists():
        residue(f"{CRATE_DIR}/tests/excel.rs", "the moved harness is missing")
        return
    text = read(path)
    new = re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", EXCEL_HARNESS_DOC, text, count=1)
    if new != text:
        write(path, new)


# ---------------------------------------------------------------------------
# Step 8: the exchange
# ---------------------------------------------------------------------------

INTEROP_HARNESS = """//! The Excel exchange with an outside implementation: openpyxl, through
//! `scripts/check_excel_interop.py`. The driver runs this target twice
//! around its round trip and refuses the word `SKIPPED`, so a skipped half
//! never reads as a pass.

#[path = "interop/excel.rs"]
mod excel;
"""


def edit_interop(root: pathlib.Path) -> None:
    write(root / CRATE_DIR / "tests/interop.rs", INTEROP_HARNESS)
    edit(
        root, f"{CRATE_DIR}/tests/interop/excel.rs",
        ("    let mut path = std::env::current_dir().expect(\"a working directory\");\n"
         "    // Under `cargo test` the working directory is `rust/`.\n"
         "    path.push(\"target\");\n",
         "    let mut path = std::env::current_dir().expect(\"a working directory\");\n"
         "    // Under `cargo test` the working directory is the crate's, `rust/excel/`;\n"
         "    // the exchange lives beside the driver's, under `rust/target/`.\n"
         "    path.push(\"..\");\n"
         "    path.push(\"target\");\n"),
    )
    edit(
        root, "scripts/check_excel_interop.py",
        ("1. ``cargo test --test interop excel::`` writes ``target/excel-interop/from-rust.xlsx``.",
         "1. ``cargo test -p yggdryl-excel --test interop excel::`` writes\n   ``target/excel-interop/from-rust.xlsx``."),
        ('["cargo", "test", "--locked", "--test", "interop", "excel::", "--", "--nocapture"],',
         '["cargo", "test", "--locked", "-p", "yggdryl-excel", "--test", "interop", "excel::", "--", "--nocapture"],'),
    )


# ---------------------------------------------------------------------------
# Step 9: install
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = """//! The claim every harness of `yggdryl-excel` makes before a test reads a
//! name: the core's register answers the Excel medium only once
//! `yggdryl-excel` has claimed it (D7), so each test opens with [`installed`].

/// Claims the Excel medium, once for the process.
pub fn installed() {
    yggdryl_excel::install().expect("yggdryl-excel claims its medium");
}
"""

INSTALL_TESTS = """
/// `install()` claims the medium under its own name at the rank the core held
/// it at (D39), once: a second call returns at once, and the claim stands
/// against another crate's.
#[test]
fn install_claims_excel_at_its_reserved_rank_once() {
    yggdryl_excel::install().expect("the first install claims the medium");
    yggdryl_excel::install().expect("a later install returns at once");
    let held = yggdryl::media::codec_for(&MimeType::XLSX).expect("the medium is claimed");
    assert_eq!((held.name(), held.title(), held.rank()), ("excel", "Excel", 6));
    assert!(std::ptr::addr_eq(
        held,
        &yggdryl_excel::EXCEL_CODEC as &dyn yggdryl::media::MediaCodec
    ));
    let refused = yggdryl::media::codec::claim(&yggdryl_excel::EXCEL_CODEC, "another")
        .expect_err("a second claim of the medium is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-excel"), "{refused}");
}
"""


def install_everywhere(root: pathlib.Path) -> None:
    base = root / CRATE_DIR
    write(base / "tests/support/install.rs", INSTALL_SUPPORT)
    harnesses = tests = 0
    for path in sorted((base / "tests").glob("*.rs")):
        text = s4.declare_install(read(path))
        text = re.sub(r'\n{3,}(#\[path = "support/install\.rs"\])', r"\n\n\1", text, count=1)
        # The declaration stands apart from the item after it.
        write(path, re.sub(r'(#\[path = "support/install\.rs"\]\nmod install;\n)(?=[^\n])', r"\1\n", text, count=1))
        harnesses += 1
    mod_ = base / "tests/excel/mod_.rs"
    if mod_.exists() and "install_claims_excel_at_its_reserved_rank_once" not in read(mod_):
        text = read(mod_).rstrip("\n") + "\n" + INSTALL_TESTS
        if not re.search(r"(?m)^use yggdryl::\{[^}]*\bMimeType\b", text) and "use yggdryl::MimeType;" not in text:
            residue(f"{CRATE_DIR}/tests/excel/mod_.rs", "the install test names `MimeType`, which the file does not import at its top")
        write(mod_, text)
    for path in sorted((base / "tests").rglob("*.rs")):
        if "/support/" in str(path):
            continue
        text, n = s4.insert_test_installs(read(path))
        tests += n
        write(path, text)
    # The crate root's example writes a workbook through the register.
    lib = base / "src/lib.rs"
    text, examples = s4.insert_doc_installs(read(lib), EXCEL)
    write(lib, text)
    pages = 0
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text, n = markdown_installs(read(root / f))
            if n:
                write(root / f, text)
                pages += n
    install_at_init(root)
    done(f"install: {harnesses} harnesses, {tests} tests, the bench main, {examples} crate example(s), "
         f"{pages} page blocks, the bindings and the CLI")


def block_reads_excel(body: str) -> bool:
    return bool(re.search(r"\byggdryl_excel\b", body)) or "MimeType::XLSX" in body or any(
        literal_reads_excel(l.group(0)) for l in s4.STRING_LITERAL.finditer(body)
    )


def markdown_installs(text: str) -> tuple[str, int]:
    """A Rust block naming the crate, or reading a handle whose name declares
    a workbook through the register, installs it first."""
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or f"{CRATE}::install()" in body or not block_reads_excel(body):
            return m.group(0)
        lines = body.split("\n")
        main = next((k for k, line in enumerate(lines) if re.match(r"^\s*fn main\(", line)), None)
        if main is not None:
            k = main
            while k < len(lines) and not lines[k].rstrip().endswith("{"):
                k += 1
            if k >= len(lines):
                return m.group(0)
            ends = "?;" if "->" in "".join(lines[main:k + 1]) else f'.expect("{PACKAGE} installs");'
            inner = re.match(r"[ \t]*", lines[k]).group(0)
            lines.insert(k + 1, f"{inner}    {CRATE}::install(){ends}")
        else:
            k = 0
            last_use = -1
            while k < len(lines):
                stripped = lines[k].strip()
                if stripped.startswith("use ") or stripped.startswith("pub use "):
                    while k < len(lines) and not lines[k].rstrip().endswith(";"):
                        k += 1
                    last_use = k
                elif stripped and not stripped.startswith("//"):
                    break
                k += 1
            at = last_use + 1
            if last_use >= 0 and at < len(lines) and not lines[at].strip():
                # Its own paragraph between the imports and the example.
                lines[at + 1:at + 1] = [f"{indent}{CRATE}::install()?;", ""]
            else:
                lines.insert(at, f"{indent}{CRATE}::install()?;")
        count += 1
        return f"{indent}```{info}\n" + "\n".join(lines) + f"{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text), count


def install_at_init(root: pathlib.Path) -> None:
    """The bindings' init and the CLI's `main` claim the medium, after the
    crates S4 split off where it has landed (dependency order)."""
    rel = "python/src/lib.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        fix = "    yggdryl_fix::install().map_err(value_error)?;\n"
        if fix in text:
            text = replace_once(text, fix, fix + f"    {CRATE}::install().map_err(value_error)?;\n", rel)
        else:
            text = replace_once(
                text, "    logging::install(module.py())?;\n",
                "    logging::install(module.py())?;\n"
                "    // The crates split off the core claim what they register before a\n"
                "    // class can read a name of theirs, in dependency order.\n"
                f"    {CRATE}::install().map_err(value_error)?;\n", rel)
        write(root / rel, text)
    rel = "node/src/lib.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        fix = '    yggdryl_fix::install().expect("yggdryl-fix claims its names");\n'
        line = f'    {CRATE}::install().expect("{PACKAGE} claims its medium");\n'
        if fix in text:
            text = replace_once(text, fix, fix + line, rel)
        else:
            text = replace_once(
                text, "fn install_logging() {\n    logging::install();\n}",
                "fn install_logging() {\n    logging::install();\n"
                "    // The crates split off the core claim what they register before an\n"
                "    // export can read a name of theirs, in dependency order; a refusal is\n"
                "    // a build linking two claimants, which no caller can repair.\n"
                + line + "}", rel)
        write(root / rel, text)
    rel = "cli/src/main.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        chain = "yggdryl_market::install().and_then(|()| yggdryl_fix::install())"
        if chain in text:
            text = replace_once(text, chain, chain + f".and_then(|()| {CRATE}::install())", rel)
        else:
            text = replace_once(
                text, "    warnings::install();\n",
                "    warnings::install();\n"
                "    // The crates split off the core claim what they register before an\n"
                "    // argument can name a medium of theirs, in dependency order.\n"
                f"    if let Err(refusal) = {CRATE}::install() {{\n"
                "        style::bad(&refusal.to_string());\n"
                "        return ExitCode::FAILURE;\n"
                "    }\n", rel)
        write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 10: manifests
# ---------------------------------------------------------------------------

LIB_IDENTS = {"md5": "md-5", "iceberg_official": "iceberg-official"}


def crate_manifest(root: pathlib.Path, core_manifest: dict, version: str) -> None:
    deps = core_manifest.get("dependencies", {})
    dev = core_manifest.get("dev-dependencies", {})

    def ident(package: str) -> str:
        for lib, pkg in LIB_IDENTS.items():
            if pkg == package:
                return lib
        return package.replace("-", "_")

    by_ident = {ident(name): (name, spec) for name, spec in deps.items()}
    dev_by_ident = {ident(name): (name, spec) for name, spec in dev.items()}
    skip = {"yggdryl", CRATE, "crate", "self", "super", "std", "core", "alloc"}
    used = s4.crates_named(root, [f"{CRATE_DIR}/src"])
    used_dev = s4.crates_named(root, [f"{CRATE_DIR}/tests", f"{CRATE_DIR}/benchmarks"]) - used
    lines = ["yggdryl.workspace = true"]
    for name in sorted(used - skip):
        if name in by_ident:
            lines.append(av.dependency_line(*by_ident[name]))
    dev_lines = []
    for name in sorted(used_dev - skip):
        if name in by_ident:
            dev_lines.append(av.dependency_line(*by_ident[name]))
        elif name in dev_by_ident:
            dev_lines.append(av.dependency_line(*dev_by_ident[name]))
    if not any(l.startswith("criterion") for l in dev_lines):
        dev_lines.append(av.dependency_line("criterion", dev.get("criterion", "0.7")))
    for leaf in ("yggdryl_market", "yggdryl_fix", "yggdryl_avro"):
        if leaf in used_dev:
            dev_lines.append(f"{leaf.replace('_', '-')}.workspace = true")
    # The core's gates the crate's tests read, forwarded.
    core_features = core_manifest.get("features", {})
    found: set[str] = set()
    for path in (root / CRATE_DIR).rglob("*.rs"):
        found |= set(re.findall(r'feature\s*=\s*"([\w-]+)"', read(path)))
    found.discard("internals")
    unknown = sorted(found - set(core_features))
    if unknown:
        residue(f"{CRATE_DIR}/Cargo.toml", f"the crate's code reads features the core does not have: {unknown}")
    if "iceberg" in found:
        residue(f"{CRATE_DIR}/Cargo.toml", "a moved test reads the `iceberg` gate: it builds an Iceberg object and belongs to "
                "`yggdryl-iceberg`, which no crate forwards `iceberg` for (regroup, S6 order amended)")
    features = [("default", [])]
    for name in sorted(found & set(core_features)):
        implied = [g for g in core_features[name] if g in found]
        features.append((name, [*implied, f"yggdryl/{name}"]))
    features.append(("internals", ["yggdryl/internals"]))
    feature_text = "\n".join(f"{name} = {av.toml_value(value)}" for name, value in features)
    manifest = (
        "[package]\n"
        f'name = "{PACKAGE}"\n'
        'description = "Office Open XML workbooks for yggdryl: one worksheet read and written as records over the core\'s handles and Arrow, and the workbook, its sheets and cells for random access"\n'
        "version.workspace = true\n"
        "edition.workspace = true\n"
        "rust-version.workspace = true\n"
        "license.workspace = true\n"
        "repository.workspace = true\n"
        'readme = "README.md"\n'
        'keywords = ["excel", "xlsx", "spreadsheet", "arrow", "ooxml"]\n'
        'categories = ["encoding", "parser-implementations"]\n'
        "\n[features]\n"
        "# The core's gates this crate's tests read, forwarded; the crate's own code\n"
        "# reads none. `internals` makes `yggdryl_excel::internals` exist for\n"
        "# `tests/` and turns the core's on.\n"
        + feature_text + "\n"
        "\n[dependencies]\n" + "\n".join(lines) + "\n"
        "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
        "\n# A workbook's records written and read, one cell through `Workbook`, a\n"
        "# sheet into and from a `Serie`; `main` claims the medium first.\n"
        '[[bench]]\nname = "excel"\npath = "benchmarks/excel.rs"\nharness = false\n'
    )
    write(root / CRATE_DIR / "Cargo.toml", manifest)
    readme = root / CRATE_DIR / "README.md"
    if not readme.exists():
        write(
            readme,
            f"# {PACKAGE}\n\nOffice Open XML workbooks (`.xlsx`) for yggdryl: one worksheet read and "
            "written as records over the core's handles and Arrow, and the workbook, its sheets and "
            "cells for random access.\n\n"
            "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the project's "
            "site, and `install()` claims the medium on the core's register.\n",
        )
    # The workspace: members and the one pinned version.
    ws = read(root / "Cargo.toml")
    m = re.search(r"members = \[([^\]]*)\]", ws)
    members = [s.strip().strip('"') for s in m.group(1).split(",") if s.strip()]
    if CRATE_DIR not in members:
        at = max([i for i, s in enumerate(members) if s == "rust" or s.startswith("rust/")] or [0]) + 1
        members.insert(at, CRATE_DIR)
        ws = ws[:m.start(1)] + ", ".join(f'"{s}"' for s in members) + ws[m.end(1):]
    if f"{PACKAGE} = {{" not in ws:
        if re.search(r"(?m)^yggdryl = \{ path = \"rust\"", ws):
            tail = list(re.finditer(r'(?m)^yggdryl(?:-[a-z]+)? = \{ path = "rust[^\n]*\n', ws))[-1]
            ws = ws[:tail.end()] + f'{PACKAGE} = {{ path = "{CRATE_DIR}", version = "={version}" }}\n' + ws[tail.end():]
        else:
            ws = replace_once(
                ws,
                "[workspace.dependencies]\n",
                "[workspace.dependencies]\n"
                "# The workspace's own crates, pinned to the one version every artifact\n"
                "# carries, so a published crate names exactly the core it was built with.\n"
                f'yggdryl = {{ path = "rust", version = "={version}" }}\n'
                f'{PACKAGE} = {{ path = "{CRATE_DIR}", version = "={version}" }}\n',
                "Cargo.toml",
            )
    write(root / "Cargo.toml", ws)
    # The bindings and the CLI link the crate.
    for member in ("python", "node", "cli"):
        path = root / member / "Cargo.toml"
        text = read(path)
        if PACKAGE in text:
            continue
        m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:yggdryl-[a-z]+ = [^\n]*\n)*", text)
        if not m:
            residue(f"{member}/Cargo.toml", "no `yggdryl` dependency line to place the crate after")
            continue
        text = text[: m.end()] + f"{PACKAGE}.workspace = true\n" + text[m.end():]
        write(path, text)
    done(f"manifests: {CRATE_DIR}, the workspace, the bindings and the CLI")


# ---------------------------------------------------------------------------
# Step 12: CI
# ---------------------------------------------------------------------------


def edit_ci(root: pathlib.Path) -> None:
    rel = ".github/ci/rows.toml"
    if not (root / rel).exists():
        residue(rel, "absent in this tree: list the `excel` leaf by hand")
        return
    edit(
        root, rel,
        ('# excel = { package = "yggdryl-excel", jobs = ["excel-interop"] }\n',
         'excel = { package = "yggdryl-excel", jobs = ["excel-interop"] }\n'),
        ('[rows.x-excel]\nextends = ["interop"]\n', '[rows.x-excel]\nextends = ["crate-excel"]\n'),
    )
    rows = read(root / rel)
    old = ("# One `interop` target holds every exchange's Rust half, so a change to any\n"
           "# of them can break the build of all seven.\n")
    if old in rows:
        write(root / rel, rows.replace(old, (
            "# One `interop` target holds the core's exchanges' Rust half, so a change to\n"
            "# any of them can break the build of all six; a leaf's exchange is its own\n"
            "# crate's target, read through the leaf's row.\n"), 1))
    else:
        residue(rel, "the `[rows.interop]` comment counting the exchanges was not found: recount it by hand")
    edit(
        root, "scripts/tests/test_ci_plan.py",
        ('        self.assertTrue({"zip-interop", "avro-interop", "excel-interop", "object-interop",\n'
         '                         "azure-interop", "gcs-interop", "pyiceberg-interop"} <= jobs)\n',
         '        self.assertTrue({"zip-interop", "avro-interop", "object-interop",\n'
         '                         "azure-interop", "gcs-interop", "pyiceberg-interop"} <= jobs)\n'
         "        # The Excel exchange's Rust half is `yggdryl-excel`'s own target.\n"
         '        self.assertNotIn("excel-interop", jobs)\n'),
    )
    edit(
        root, ".github/workflows/ci.yml",
        ("    # every default-feature test target links and the `interop` target the\n"
         "    # ZIP, Avro and Excel drivers run, and hands both on as `lane-default`:\n",
         "    # every default-feature test target links and the `interop` target the\n"
         "    # ZIP and Avro drivers run, and hands both on as `lane-default`:\n"),
    )


# ---------------------------------------------------------------------------
# Step 13: docs, skills, AGENTS.md, README, inventory
# ---------------------------------------------------------------------------


def edit_docs(root: pathlib.Path) -> None:
    edit(
        root, "docs/media/excel.md",
        ("| Build | default |",
         "| Build | the `yggdryl-excel` crate, which every binding and the `yggdryl` command link and install |"),
        ("| Rust | `yggdryl_excel`: `Excel<H>` over any handle",
         "| Rust | `yggdryl_excel`, whose `install()` claims the medium at its reserved rank: `Excel<H>` over any handle"),
        ("One release run of the `media` Criterion target on one Linux x86_64 container",
         "One release run of the `media/excel` Criterion group on one Linux x86_64 container"),
        ("cargo bench -p yggdryl --bench media -- media/excel\n",
         "cargo bench -p yggdryl-excel --bench excel -- media/excel\n"),
        # The cost rows the Excel page cites moved with the medium.
        ("`rust/tests/iobase_calls.rs` pins the record doors' call counts and `rust/excel/tests/excel/workbook.rs` the workbook's, and `rust/tests/allocations.rs` pins",
         "`rust/excel/tests/iobase_calls.rs` pins the record doors' call counts and `rust/excel/tests/excel/workbook.rs` the workbook's, and `rust/excel/tests/allocations.rs` pins"),
    )
    edit(
        root, "docs/media/index.md",
        ("| [Excel](excel.md) | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | default |",
         "| [Excel](excel.md) | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | the `yggdryl-excel` crate |"),
        ("the core claims its own - Arrow IPC, Parquet, Avro, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`",
         "the core claims its own - Arrow IPC, Parquet, Avro, plain text, XML for Analysis and CSV - before the register answers anything, and a crate that brings another claims it from its `install()`, as `yggdryl-excel` claims Excel"),
        ("a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's seven hold - are refused at `$.encoding`.",
         "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's media hold - are refused at `$.encoding`, but for `RESERVED_RANKS`: the ranks the media split off the core keep, `parquet` 1, `avro` 2, `xmla` 4 and `excel` 6, each claimed under its own name and no other, so an options value hashes and sorts as it did when the core held its medium."),
        ("The core claims its own seven media before the register answers anything, until they move to the crates that hold them.",
         "The core claims its own six media before the register answers anything, until they move to the crates that hold them; `yggdryl_excel::install()` claims Excel, and every binding and the `yggdryl` command call it at load."),
        ("use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RecordOptions};",
         "use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RESERVED_RANKS, RecordOptions};"),
        ("// The core's ranks and names are never another crate's.\nassert_eq!(EXTERNAL_RANK, 32);\n",
         "// The core's ranks and names are never another crate's; a medium split off\n"
         "// the core keeps its rank under its own name.\nassert_eq!(EXTERNAL_RANK, 32);\n"
         'assert_eq!(RESERVED_RANKS, [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)]);\n'),
    )
    edit(
        root, "docs/benchmarks.md",
        ("| `media` | record round trips, text projection, Avro, Parquet, Iceberg, CSV, Excel, XML for Analysis, and pushdown |",
         "| `media` | record round trips, text projection, Avro, Parquet, Iceberg, CSV, XML for Analysis, and pushdown |\n"
         "| `excel` | `yggdryl-excel`'s target: a workbook's records written and read under a declared and an inferred field, one cell through `Workbook`, and a sheet into and from a `Serie` |"),
        ('    cargo bench --bench media --features "parquet iceberg"\n',
         '    cargo bench --bench media --features "parquet iceberg"\n'
         "    cargo bench -p yggdryl-excel --bench excel\n"),
    )
    edit(
        root, "docs/contributing.md",
        ("and one root folder per medium: `rust/src/ipc/`, `parquet/`, `avro/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/` |",
         "and one root folder per medium: `rust/src/ipc/`, `parquet/`, `avro/`, `csv/`, `iceberg/`, `text/`, `xmla/`; and `rust/excel/`, the `yggdryl-excel` crate |"),
    )
    edit(
        root, "docs/architecture.md",
        ("`xmla/` (XML for Analysis rowsets, and the provider serving them), `excel/` (Office Open XML workbooks:",
         "`xmla/` (XML for Analysis rowsets, and the provider serving them), `rust/excel/`, the `yggdryl-excel` crate (Office Open XML workbooks:"),
    )
    edit(
        root, "skills/yggdryl-records/references/formats.md",
        ("| Excel workbook | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | default |",
         "| Excel workbook | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | `yggdryl-excel`, linked by every binding |"),
    )
    edit(
        root, "skills/yggdryl/SKILL.md",
        ("`aws` (implies `http`), `s3` (implies `aws`) | everything built in | everything built in |",
         "`aws` (implies `http`), `s3` (implies `aws`); Excel workbooks are the `yggdryl-excel` crate, claimed once by `yggdryl_excel::install()` | everything built in | everything built in |"),
    )
    edit(
        root, "README.md",
        ("  src/{ipc,parquet,avro,csv,iceberg,xmla,excel}/\n"
         "                         One folder per record medium; xmla/ also holds the\n"
         "                         XML for Analysis provider and its HTTP server, and\n"
         "                         excel/ the workbook, sheet and cell model\n",
         "  src/{ipc,parquet,avro,csv,iceberg,xmla}/\n"
         "                         One folder per record medium; xmla/ also holds the\n"
         "                         XML for Analysis provider and its HTTP server\n"),
        ("  benchmarks/            Criterion targets, grouped by theme\n",
         "  benchmarks/            Criterion targets, grouped by theme\n"
         "  excel/                 yggdryl-excel: the workbook medium, its sheet and cell\n"
         "                         model, its own src/, tests/ and benchmarks/\n"),
    )
    for binding, frozen in (("python/src/excel.rs", ", frozen."), ("node/src/excel.rs", ". A cell's value")):
        edit(
            root, binding,
            ("//! Every value here is the core's: a `Workbook` is [`yggdryl_excel::Workbook`]",
             "//! Every value here is `yggdryl-excel`'s: a `Workbook` is [`yggdryl_excel::Workbook`]"),
            (f"//! `CellRange` are the core values{frozen}", f"//! `CellRange` are that crate's values{frozen}"),
        )
    edit_agents(root)
    done("docs: the Excel page, the media overview, the benchmark index, contributing, architecture, the skills, the README, AGENTS.md")


def edit_agents(root: pathlib.Path) -> None:
    rel = "AGENTS.md"
    text = read(root / rel)
    pairs = [
        ("| `excel/` | the Office Open XML workbook medium over `zip/` and the XML parser:",
         "| `excel/` in `yggdryl-excel` | the Office Open XML workbook medium - in `yggdryl-excel` at `rust/excel/`, its `install()` claiming `EXCEL_CODEC` at the reserved rank 6 - over the core's `zip/` and XML parser, which stay the core's:"),
        ("the core claims its own seven media and its table format before a register answers anything, until they move to the crates that hold them |",
         "the core claims its own six media and its table format before a register answers anything, until they move to the crates that hold them; `RESERVED_RANKS` keeps the rank of each medium split off it, under its own name |"),
        ("the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`)",
         "the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`, `RESERVED_RANKS`)"),
        ("XML for Analysis and the workbook are registered media, held as",
         "XML for Analysis and the workbook - `yggdryl-excel`'s - are registered media, held as"),
        ("an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK`,",
         "an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK` - a medium the core held at its `RESERVED_RANKS` rank, under its own name -"),
        ("Sections: both crates, market, FIX, and what the media folders share with them.",
         "Sections: both crates, market, FIX, what the media folders share with them, and what the media crates reach."),
    ]
    for old, new in pairs:
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
        elif new not in text:
            residue(rel, f"anchor matched {text.count(old)} times, not edited: {old[:90]!r}")
    bench = re.search(r"(?m)^cargo bench -p yggdryl --bench <[^>\n]*>\n", text)
    if bench and f"cargo bench -p {PACKAGE} --bench excel" not in text:
        text = text[:bench.end()] + f"cargo bench -p {PACKAGE} --bench excel\n" + text[bench.end():]
    elif not bench:
        residue(rel, "no `cargo bench -p yggdryl --bench <...>` line to list the crate's target beside")
    write(root / rel, text)


EXCEL_INSTALL_ENTRY = (
    "pub fn install() -> yggdryl::Result<()>  (claims EXCEL_CODEC under the spreadsheetml sheet MIME type "
    "at rank 6, which yggdryl::media::RESERVED_RANKS keeps for it, once for the process; every binding's "
    "init and the CLI's `main` call it; a later call returns at once; the register's refusal where another "
    "crate claimed the type or the name first)"
)

IMPLEMENTER_INVENTORY = """  (Media: what the media crates reach - the record doors a wrapper redirects to, the dimensions and the schema it answers through, the cache clock it stamps its entry with, and the value and text helpers one medium's codec reads)
pub use iobase::{append_arrow_reader_default, leaf_writer, merge_arrow_reader_default, overwrite_arrow_reader_default_with_field}  (raised inside the crate-private `iobase`: the default record writes a media wrapper redirects to once its options are proven its own, and the one encode of a leaf's whole contents)
pub use iobase::hierarchy::owned_handle  (raised inside the crate-private `iobase::hierarchy`: an owned handle on the resource a borrowed one addresses, reopened at its location, else copied once)
pub use iomedia::{container_field, container_origin, container_row_size, dimension_options, field_under, own_options, read_record_serie}  (raised inside the crate-private `iomedia`: the handle's own options where none were given, a container's root, origin and row count, the options a dimension is read under, the one schema answer under the options' clauses, the one composed record read)
pub use temporal::scalars::{TemporalKind, nanoseconds_per}  (raised inside the crate-private `temporal::scalars`: the temporal family a cell's datatype is read by, and the nanoseconds one count of a fixed-length unit holds)
pub const fn datatype_id_temporal_kind(id: crate::DataTypeId) -> Option<TemporalKind>  (`DataTypeId::temporal_kind`: the temporal family an identifier is a leaf of)
pub fn cache_now() -> std::time::Instant  (`media::cache::now`: the instant every cache door reads)
pub fn arrow_schema_from_field(field: &Field) -> crate::arrow::Result<arrow_schema::SchemaRef>  (`arrow::arrow_schema_from_field`: the Arrow schema a record root projects to)
pub fn f64_from_text(text: &str) -> Option<f64>  (`floating::f64_from_text`: the one float grammar)
pub fn format_datetime(count: i64, unit: TimeUnit) -> Option<SmolStr>  (`temporal::format_datetime`: a naive reading spelled ISO 8601)
pub fn parse_date(text: &str) -> Result<i32>  (`temporal::parse_date`: an ISO 8601 date as days since the epoch)
pub fn parse_time(text: &str) -> Result<(i64, TimeUnit)>  (`temporal::parse_time`: an ISO 8601 clock as a count since midnight)
pub fn write_leaf_text<W: std::io::Write>(writer: &mut W, value: &Scalar, context: &str) -> Result<()>  (`xml::write_leaf_text`: one leaf as element character data)
pub fn write_element_text<W: std::io::Write>(writer: &mut W, text: &str) -> Result<()>  (`xml::write_element_text`: escaped element character data)
pub fn write_attribute_text<W: std::io::Write>(writer: &mut W, text: &str) -> Result<()>  (`xml::write_attribute_text`: an escaped attribute value)
pub fn write_x_escape(target: &mut String, character: char)  (`xml::write_x_escape`: the Office Open XML `_xHHHH_` escape)
pub fn decode_x_escapes(encoded: &str) -> Cow<'_, str>  (`xml::decode_x_escapes`: every `_xHHHH_` escape read back)
pub fn zip_archive_member_reader(archive: &std::sync::Arc<crate::zip::ZipArchive>, path: &str) -> Result<Option<Box<dyn std::io::Read + Send>>>  (`ZipArchive::member_reader`: one archive member streamed by its path)
"""


def edit_inventory(root: pathlib.Path) -> None:
    rel = ".api-inventory.txt"
    text = read(root / rel)
    if EXCEL_INSTALL_ENTRY in text:
        return
    m = re.search(r"(?m)^### yggdryl_excel  \[rust/excel/src/lib\.rs\]\n(?:(?!###)[^\n]*\n)*?pub const DEFAULT_SHEET_NAME[^\n]*\n", text)
    if not m:
        residue(rel, "no `### yggdryl_excel  [rust/excel/src/lib.rs]` section to add `install()` to")
    else:
        text = text[:m.end()] + EXCEL_INSTALL_ENTRY + "\n" + text[m.end():]
        text = text.replace(
            "### yggdryl_excel  [rust/excel/src/lib.rs]\n",
            "### yggdryl_excel  [rust/excel/src/lib.rs]  (the crate root: the workbook medium, its cell model and the claim; `internals` hidden)\n",
            1,
        )
    # The crate's own sections speak of the core as `yggdryl::`.
    text = replace_once(
        text,
        "options: &ExcelOptions, ) -> crate::arrow::Result<BatchReader>",
        "options: &ExcelOptions, ) -> yggdryl::arrow::Result<BatchReader>",
        rel,
    )
    text = replace_once(
        text,
        "pub static EXCEL_CODEC: ExcelCodec  (claimed by the core under the spreadsheetml sheet MIME type, rank 6,",
        "pub static EXCEL_CODEC: ExcelCodec  (claimed by `install()` under the spreadsheetml sheet MIME type at its reserved rank 6,",
        rel,
    )
    if "pub const RESERVED_RANKS" not in text:
        text = replace_once(
            text,
            "pub const EXTERNAL_RANK: u8 = 32  (the rank a medium outside the core takes at least; the core's seven hold the positions below it, in the order their options sort: ipc 0, parquet 1, avro 2, text 3, xmla 4, csv 5, excel 6)",
            "pub const EXTERNAL_RANK: u8 = 32  (the rank a medium outside the core takes at least; the positions below it, in the order their options sort - ipc 0, parquet 1, avro 2, text 3, xmla 4, csv 5, excel 6 - are the core's own media's and the RESERVED_RANKS of the media split off it)\n"
            'pub const RESERVED_RANKS: [(&str, u8); 4] = [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)]  (the ranks the media split off the core keep, a wire contract that never moves: claim admits a medium at one only under the name beside it, refuses that name at any other rank, and holds every other medium at or above EXTERNAL_RANK; re-exported from yggdryl::media)',
            rel,
        )
    text = replace_once(
        text,
        "the core claims its own seven before the register answers anything, until they move to the crates that hold them;",
        "the core claims its own six before the register answers anything, until they move to the crates that hold them - yggdryl-excel's install() claims Excel at its reserved rank;",
        rel,
    )
    text = replace_once(
        text,
        "Sections: both crates, market, FIX, and what the media folders share with them.",
        "Sections: both crates, market, FIX, what the media folders share with them, and what the media crates reach.",
        rel,
    )
    start = text.find("### yggdryl::implementer  [")
    end = text.find("\n### ", start + 1) if start >= 0 else -1
    if start < 0 or end < 0:
        residue(rel, "no `yggdryl::implementer` section to list the media additions under")
    else:
        section = text[start:end].rstrip("\n")
        text = text[:start] + section + "\n" + IMPLEMENTER_INVENTORY + text[end:]
    write(root / rel, text)
    done(".api-inventory.txt: the Excel sections re-homed under yggdryl_excel, its install, the core's additions")


# ---------------------------------------------------------------------------
# Step 14: what the compiler loop still has to see
# ---------------------------------------------------------------------------


def scan_residue(root: pathlib.Path) -> None:
    for folder in ("rust", "python/src", "node/src", "cli", "docs", "skills"):
        base = root / folder
        if not base.exists():
            continue
        for path in sorted(base.rglob("*")):
            if path.suffix not in (".rs", ".md") or "/target/" in str(path):
                continue
            rel = str(path.relative_to(root))
            text = read(path)
            # A binding's own `excel` module is `crate::excel` there.
            pattern = r"\byggdryl::excel\b|yggdryl::internals::excel_|docs\.rs/yggdryl/latest/yggdryl/excel"
            if folder not in ("python/src", "node/src"):
                pattern += r"|\bcrate::excel\b"
            for m in re.finditer(pattern, text):
                residue(f"{rel}:{line_of(text, m.start())}", f"`{m.group(0)}` is left")
    # Core tests and benches still reading a workbook through the register.
    for folder in ("rust/tests", "rust/benchmarks"):
        for path in sorted((root / folder).rglob("*.rs")):
            rel = str(path.relative_to(root))
            if rel.startswith(SPLIT_EXCLUDED):
                continue
            text = read(path)
            hits = [line_of(text, m.start()) for m in s4.STRING_LITERAL.finditer(text) if literal_reads_excel(m.group(0))]
            hits += [line_of(text, m.start()) for m in re.finditer(r"\bMimeType::XLSX\b|\byggdryl_excel\b", code_only(text))]
            if hits:
                shown = ", ".join(str(n) for n in sorted(set(hits)))
                residue(rel, f"a core test or bench reads a workbook at line(s) {shown}: move it by hand (D39)")
    # A sibling leaf's test reaching Excel (after S4): none known; the crate
    # would be its dev-dependency and its `[leaves]` line `after = ["excel"]`.
    for manifest in sorted((root / "rust").glob("*/Cargo.toml")):
        leaf = manifest.parent.name
        if leaf in av.CORE_FOLDERS or leaf == EXCEL:
            continue
        for path in sorted(manifest.parent.rglob("*.rs")):
            text = read(path)
            code = code_only(text)
            hits = [line_of(text, m.start()) for m in re.finditer(r"\bMimeType::XLSX\b|\byggdryl_excel\b", code)]
            hits += [line_of(text, m.start()) for m in s4.STRING_LITERAL.finditer(text) if literal_reads_excel(m.group(0))]
            if hits:
                residue(f"{path.relative_to(root)}", f"a sibling leaf reads a workbook at line(s) {sorted(set(hits))}: "
                        f"install `{PACKAGE}` there and list it as a dev-dependency")
    # Paths that no longer name a file.
    for path in sorted(list((root / "rust").rglob("*.rs")) + list((root / "cli").rglob("*.rs"))):
        rel = str(path.relative_to(root))
        if "/target/" in rel:
            continue
        text = read(path)
        for m in re.finditer(r'#\[path\s*=\s*"([^"]+)"\]|(?<![\w!])include_(?:str|bytes)!\(\s*"([^"]+)"', text):
            literal = m.group(1) or m.group(2)
            if not (path.parent / literal).exists():
                residue(f"{rel}:{line_of(text, m.start())}", f"`{literal}` names no file")
    for folder in ("rust/src", f"{CRATE_DIR}/src", "python/src", "node/src", "cli/src"):
        for path in sorted((root / folder).rglob("*.rs")):
            text = read(path)
            for m in re.finditer(r"#\[cfg\(test\)\]|#\[test\]|\bmod tests\b", text):
                residue(f"{path.relative_to(root)}:{line_of(text, m.start())}", "test code under `src/`")
    # The docs runner compiles the pages' Rust blocks as one target: under
    # `cli/` after S4 (D13), which links this crate; as a core test target
    # before it, which cannot name `yggdryl_excel`.
    runner = root / "scripts/check_docs_examples.py"
    if runner.exists() and 'ROOT / "rust" / "tests" / "docs_examples.rs"' in read(runner):
        residue("scripts/check_docs_examples.py", "the Rust page blocks still compile as a core test target, which cannot link "
                f"`{PACKAGE}` (a cycle): the blocks that install it compile once S4 moves the runner under `cli/` (D13)")


# ---------------------------------------------------------------------------
# The run
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=pathlib.Path)
    parser.add_argument("--no-git-lock", action="store_true", help="the caller holds $S/git.lock already")
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s6_excel" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    # Step 0: a tree already moved is left as it is.
    if (root / CRATE_DIR / "Cargo.toml").exists():
        print(f"[s6-excel] {CRATE_DIR}/Cargo.toml exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/excel/mod.rs").exists():
        raise SystemExit("rust/src/excel/mod.rs is missing: this is not the tree S6b moves")
    if git(root, "status", "--porcelain").strip():
        raise SystemExit("the tree has changes; run S6b on a clean tree")
    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)
    import tomllib

    core_manifest = tomllib.loads(read(root / "rust/Cargo.toml"))
    tables = ExcelTables(excel_public(read(root / "rust/src/excel/mod.rs")))
    av.load_trait_methods(root)
    moves = plan_moves(root)
    paths = PathMap(moves)

    # Step 1-3 read the core's files where they stand.
    splits = split_tests(root, moves)
    note_kept_traits(root, splits)
    edit_media_register(root)
    edit_core_media_bench(root)
    edit(root, "rust/tests/interop.rs", ('#[path = "interop/excel.rs"]\nmod excel;\n', ""))
    done(f"tests: {len(splits)} file(s) split off the core's; the media bench and the interop target lose Excel")

    # Step 4: the moves.
    with s4.GitLock(not arguments.no_git_lock):
        for old, new in moves:
            (root / new).parent.mkdir(parents=True, exist_ok=True)
            git(root, "mv", old, new)
    done(f"moves: {len(moves)} files moved by git")

    # Step 5-6.
    build_lib(root)
    build_bench_root(root)
    edit_codec(root)
    edit_implementer(root)
    edit_core_lib_and_manifest(root)
    done("crate: lib.rs and its install(), the bench root; core: the seed, RESERVED_RANKS, the implementer, the exclude")

    # Step 7.
    rewriter = s4.Rewriter(tables)
    rewrite_paths(root, rewriter, paths, moves, splits)
    edit_calls(root)
    finish_crate_text(root)

    # Step 8-13.
    edit_interop(root)
    install_everywhere(root)
    crate_manifest(root, core_manifest, version)
    av.edit_tooling(root)
    edit_ci(root)
    edit_docs(root)
    edit_inventory(root)

    # Step 14.
    scan_residue(root)
    with s4.GitLock(not arguments.no_git_lock):
        git(root, "add", "-A", "--", CRATE_DIR)
    report = ["# S6b residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE] + ["", "## Residue", ""] + (RESIDUE or ["- none"])
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s6-excel] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
