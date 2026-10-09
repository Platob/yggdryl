#!/usr/bin/env python3
"""S6c: move the XML for Analysis medium, with SOAP, into `yggdryl-xmla` at
`rust/xmla/`.

Usage:
    python3 -I s6_xmla_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch after S4 and the Excel move
(DESIGN.md "S6 order amended": S4, excel, xmla, then the avro, parquet, s3 and
iceberg batch) - and it leaves the tree uncommitted: `git status` shows the
moves as renames and every rewrite as a modification, for the lane manager's
compiler loop (`cargo check --workspace --all-targets --all-features
--keep-going`). It runs no cargo command. A second run on a tree it already
moved stops at step 0 and changes nothing. It holds on the tree as it stands
before S4 too, which is how it was simulated.

D33 is the decision it carries out: `soap/` - the SOAP 1.1 envelope, which
only XML for Analysis speaks - leaves with the medium, as the crate's own
`soap` module; D39 fixes the rank, the door and the tests.

What it does, in order (each step says what it touched; `--residue` gets
every site it could not rewrite mechanically, `file:line` each):

 1. test split - before the moves, every core test item that reaches XMLA or
                 SOAP - a `yggdryl::xmla`/`yggdryl::soap` path, a name
                 imported from one, a handle whose name declares `.xmla`,
                 `MimeType::XMLA`, the medium's name - moves byte-identical
                 into `rust/xmla/tests/` at the same relative path, its
                 helpers copied, each half's imports pruned (a trait whose
                 methods a half calls kept, a macro invocation travelling
                 with the items it defines), the harness written beside it
                 from the core's (D39: a test lives in the lowest crate that
                 can name everything it uses). The catalog `type` word `xmla`
                 `Catalog::from_url` refuses stays core (D10).
 2. moves      - `git mv`: `rust/src/xmla/` to `rust/xmla/src/` (`mod.rs`
                 the crate root `lib.rs`), `rust/src/soap/mod.rs` to
                 `rust/xmla/src/soap/mod.rs`, `rust/tests/{xmla,soap}{,.rs}`
                 with the Excel wire fixtures to `rust/xmla/tests/`,
                 `rust/benchmarks/media/xmla.rs` to the crate's bench root
                 `rust/xmla/benchmarks/xmla.rs`.
 3. crate      - `lib.rs` (the doc, `#![deny(unsafe_code)]`, `pub mod soap`,
                 `install()` claiming `XMLA_CODEC` at its reserved rank 4 -
                 idempotent, D7), the rowset read landing its canonical rows
                 through `Serie::from_scalars` (a proof never crosses the
                 crate boundary), the crate-private `Serie` and `Plan`
                 methods reached through the core's `<type>_<item>`
                 forwarders.
 4. core       - `pub mod xmla` and `pub mod soap` gone, the seed claim of
                 `XMLA_CODEC` gone; `RESERVED_RANKS` and the claim rule (D39)
                 where no earlier media move wrote them; `yggdryl::implementer`
                 grown item by item where absent, by S3's routes (R raises in
                 `iobase` and `iomedia`; F forwarders; A `plan_map_sources`);
                 `exclude = ["/xmla"]`.
 5. paths      - every `crate::` path of the moved sources by owner, every
                 `yggdryl::xmla`/`yggdryl::soap` path and `use` tree in tests,
                 benches, the CLI, sibling leaves, docs and the inventory,
                 file paths in every text file, `#[path]`/`CARGO_MANIFEST_DIR`
                 re-anchored, docs.rs links.
 6. install    - every crate test opens with `crate::install::installed()`,
                 the install test pins the rank, every rustdoc example of the
                 crate installs first, the bench `main` installs, a page's
                 Rust block reading XMLA installs, the bindings' init and the
                 CLI's `main` install after whatever installs there already; a
                 sibling leaf's test reaching XMLA (S4's market half of
                 `xmla/dbtype.rs`) stays where S4 left it and installs.
 7. manifests  - `rust/xmla/Cargo.toml` and its README, the workspace members
                 and `[workspace.dependencies]`, the core's `exclude`, the
                 bindings' (no feature) and the CLI's (`http`).
 8. tooling    - `generate_internals.py` and `check_api_inventory.py` per
                 crate (S4's edits, where S4 has not landed), the generator.
 9. CI         - the `xmla` leaf line, `after = ["xmla"]` on a sibling whose
                 tests link it.
10. docs       - the XMLA page, the media overview, the READMEs, the
                 contributing page, AGENTS.md (the `xmla/` and `soap/` rows in
                 `yggdryl-xmla`, the register sentences), `.gitattributes`,
                 `.api-inventory.txt` re-homed under `### yggdryl_xmla..`.
11. residue    - what is left for the compiler loop, by file and line.
"""

from __future__ import annotations

import argparse
import collections
import os
import pathlib
import posixpath
import re
import subprocess
import sys

SCRATCH = pathlib.Path(
    os.environ.get(
        "S",
        "/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad",
    )
)
sys.path.insert(0, str(SCRATCH))
import s4_move as s4  # noqa: E402  (S4's lexer, use trees, rewriter, lock)
import s6_avro_move as s6a  # noqa: E402  (S6a's prune, split, register edits)

CORE, XMLA = "core", "xmla"
PACKAGE = "yggdryl-xmla"
CRATE = "yggdryl_xmla"
CRATE_DIR = "rust/xmla"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-xmla] {what}", flush=True)


# The helpers of S4 and S6a report into this script's residue and log; S4's
# rewriter learns the crate.
s4.residue = residue
s6a.residue = residue
s6a.done = done
s4.CRATE_NAME[XMLA] = CRATE

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, render_use, UseParseError = s4.parse_use, s4.render_use, s4.UseParseError
line_of = s4.line_of
# S6a's prune keeps a trait whose methods a file calls (S4's defect).
prune_imports = s6a.prune_imports
s4.prune_imports = prune_imports
edit = s6a.edit
tidy_head, published, pinned_source = s6a.tidy_head, s6a.published, s6a.pinned_source
test_fn_bodies, macro_names = s6a.test_fn_bodies, s6a.macro_names


def replace_once(text: str, old: str, new: str, where: str) -> str:
    return s4.replace_once(text, old, new, where)


def sub_once(text: str, pattern: str, repl, where: str, flags: int = 0, done_if: str | None = None) -> str:
    """One regex substitution, asserted to match once; a text that already
    holds `done_if` is left as it is."""
    if done_if is not None and done_if in text:
        return text
    found = list(re.finditer(pattern, text, flags))
    if len(found) != 1:
        residue(where, f"pattern matched {len(found)} times, not edited: {pattern[:90]!r}")
        return text
    return re.sub(pattern, repl, text, count=1, flags=flags)


# ---------------------------------------------------------------------------
# Who owns a path
# ---------------------------------------------------------------------------

# The core's crate-private items the moved sources reach, by the path they
# spell after `crate::`, and the name `yggdryl::implementer` answers them
# under (read off the tree by the S6 reach tool, D39's list for XMLA).
CORE_ROUTES: dict[tuple[str, ...], str] = {
    ("arrow", "arrow_schema_from_field"): "arrow_schema_from_field",
    ("arrow", "field_from_arrow_schema"): "field_from_arrow_schema",
    ("bytes", "into_base64"): "into_base64",
    ("http", "server", "normalize_path"): "normalize_path",
    ("integer", "integer_from_text_as"): "integer_from_text_as",
    ("iobase", "append_arrow_reader_default"): "append_arrow_reader_default",
    ("iobase", "leaf_writer"): "leaf_writer",
    ("iobase", "merge_arrow_reader_default"): "merge_arrow_reader_default",
    ("iobase", "overwrite_arrow_reader_default_with_field"): "overwrite_arrow_reader_default_with_field",
    ("iomedia", "container_field"): "container_field",
    ("iomedia", "container_origin"): "container_origin",
    ("iomedia", "container_row_size"): "container_row_size",
    ("iomedia", "dimension_options"): "dimension_options",
    ("iomedia", "field_under"): "field_under",
    ("iomedia", "own_options"): "own_options",
    ("iomedia", "read_record_serie"): "read_record_serie",
    ("media", "cache", "now"): "cache_now",
    ("temporal", "format_timestamp"): "format_timestamp",
    ("temporal", "parse_timestamp"): "parse_timestamp",
    ("text", "ERROR_TEXT_LIMIT"): "ERROR_TEXT_LIMIT",
    ("text", "elide_to"): "elide_to",
    ("text", "expected_got"): "expected_got",
    ("uuid_parse",): "uuid_parse",
    ("warehouse", "holds"): "holds",
    ("warehouse", "no_catalog"): "no_catalog",
    ("warehouse", "path_text"): "path_text",
    ("xml", "decode_x_escapes"): "decode_x_escapes",
    ("xml", "is_name_char"): "is_name_char",
    ("xml", "is_name_start"): "is_name_start",
    ("xml", "shaped"): "shaped",
    ("xml", "write_attribute_text"): "write_attribute_text",
    ("xml", "write_element_text"): "write_element_text",
    ("xml", "write_fragment"): "write_fragment",
    ("xml", "write_leaf_text"): "write_leaf_text",
    ("xml", "write_x_escape"): "write_x_escape",
}


class XmlaTables:
    """`s4.Rewriter`'s resolution for the two modules leaving the core: a
    `xmla::` path is the crate's root, a `soap::` path the crate's `soap`."""

    def __init__(self) -> None:
        self.inside = False

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        if segs[0] == "xmla":
            return XMLA, segs[1:]
        if segs[0] == "soap":
            return XMLA, segs
        for cut in range(len(segs), 0, -1):
            name = CORE_ROUTES.get(tuple(segs[:cut]))
            if name:
                return CORE, ["implementer", name, *segs[cut:]]
        return CORE, segs


# ---------------------------------------------------------------------------
# The move table and file paths
# ---------------------------------------------------------------------------


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    moves: list[tuple[str, str]] = []
    for f in tracked(root, "rust/src/xmla", "rust/src/soap", "rust/tests/xmla", "rust/tests/xmla.rs",
                     "rust/tests/soap", "rust/tests/soap.rs", "rust/benchmarks/media/xmla.rs"):
        if f.startswith("rust/src/xmla/"):
            rel = f[len("rust/src/xmla/"):]
            new = "rust/xmla/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        elif f.startswith("rust/src/soap/"):
            new = "rust/xmla/src/soap/" + f[len("rust/src/soap/"):]
        elif f.startswith("rust/tests/"):
            new = "rust/xmla/tests/" + f[len("rust/tests/"):]
        else:
            new = "rust/xmla/benchmarks/xmla.rs"
        moves.append((f, new))
    return moves


class PathMap(s6a.PathMap):
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/xmla/", "rust/xmla/src/"),
            ("rust/src/soap/", "rust/xmla/src/soap/"),
            ("rust/tests/xmla/", "rust/xmla/tests/xmla/"),
            ("rust/tests/soap/", "rust/xmla/tests/soap/"),
        ]

    def with_files(self, extra: dict[str, str]) -> "PathMap":
        other = PathMap([])
        other.files = {**self.files, **extra}
        other.dirs = self.dirs
        return other


def rewrite_text_paths(text: str, paths: PathMap, moves: list[tuple[str, str]]) -> str:
    for old, new in sorted(moves, key=lambda pair: -len(pair[0])):
        if old in text:
            text = re.sub(r"(?<![\w./-])" + re.escape(old) + r"(?![\w-])", new, text)
    for old, new in paths.dirs:
        text = re.sub(r"(?<![\w./-])" + re.escape(old), new, text)
    for old, new in (("rust/src/xmla", "rust/xmla/src"), ("rust/src/soap", "rust/xmla/src/soap"),
                     ("rust/tests/xmla", "rust/xmla/tests/xmla"), ("rust/tests/soap", "rust/xmla/tests/soap")):
        text = re.sub(r"(?<![\w./-])" + re.escape(old) + r"(?![\w/.-])", new, text)
    # A command running the medium's harnesses runs the crate's.
    text = re.sub(r"(?m)(-p )yggdryl( [^\n]*?--test (?:xmla|soap)\b)", r"\1yggdryl-xmla\2", text)
    return text


DOCS_RS = re.compile(r"https://docs\.rs/yggdryl/latest/yggdryl/(xmla|soap)(/|\b)")


def docs_rs(text: str) -> str:
    """A docs.rs link into a moved module names the crate's own pages."""
    def sub(m: re.Match) -> str:
        base = "https://docs.rs/yggdryl-xmla/latest/yggdryl_xmla/"
        return base if m.group(1) == "xmla" else base + "soap" + m.group(2)
    return DOCS_RS.sub(sub, text)


# ---------------------------------------------------------------------------
# Step 1: the core's tests that reach XMLA, item by item (S4's split, S6a's
# fixes)
# ---------------------------------------------------------------------------

# Files that move whole, the shared fixtures, and the one file whose `xmla`
# is the catalog `type` word `Catalog::from_url` refuses (D10): they stay.
SPLIT_EXCLUDED = (
    "rust/tests/xmla/", "rust/tests/xmla.rs", "rust/tests/soap/", "rust/tests/soap.rs",
    "rust/tests/support/", "rust/tests/warehouse/catalog.rs",
)
XMLA_PATH = re.compile(
    r"\byggdryl(?:_xmla\b|::(?:xmla|soap)\b|::internals::(?:xmla|soap)_)|\bMimeType::XMLA\b"
)


def literal_reads_xmla(literal: str) -> bool:
    """A string naming a handle the register reads as XMLA, or the medium."""
    body = re.sub(r'^b?r?#*"|"#*$', "", literal)
    return bool(re.search(r"\.xmla\b", body)) or body in ("xmla", "application/xmla+xml")


def file_imports(items: list) -> set[str]:
    """Names a level imports from the two moving modules, or the crate."""
    names: set[str] = set()
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            moving = leaf[:2] in (["yggdryl", "xmla"], ["yggdryl", "soap"]) or leaf[:1] == [CRATE]
            if moving and kind == "self" and len(leaf) > 1:
                names.add(alias or leaf[-1])
    return names


def item_owners(items: list, inherited: set[str]) -> tuple[dict[int, str], dict[int, set[int]]]:
    macro_names(items)
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
        reaches = bool(XMLA_PATH.search(code[id(item)])) or bool(tokens[id(item)] & imports)
        if not reaches:
            reaches = any(literal_reads_xmla(m.group(0)) for m in s4.STRING_LITERAL.finditer(item.text))
        owner[id(item)] = XMLA if reaches else CORE
    by_id = {id(item): item for item in body}
    changed = True
    while changed:
        changed = False
        for item in body:
            if owner[id(item)] == CORE and any(
                owner[other] == XMLA for other in refs[id(item)] if inline_body(by_id[other]) is None
            ):
                owner[id(item)] = XMLA
                changed = True
    return owner, refs


def split_level(text: str, top: bool, inherited: set[str]) -> tuple[str, str | None, int]:
    """S6a's `split_level`, the crate's items the ones reaching XMLA."""
    head, items = top_items(text)
    imports = inherited | file_imports(items)
    owner, refs = item_owners(items, inherited)
    moved = 0
    dest: dict[int, str] = {}
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    for item in items:
        if item.kind == "use" or owner[id(item)] != XMLA:
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
    xmla_text = "".join(out).rstrip("\n") + "\n"
    # What stays is what a remaining test, module or published item names: a
    # helper only the moved tests used leaves the core rather than dying there.
    roots = {
        id(item) for item in body_items
        if id(item) not in dropped
        and (re.search(r"#\[test\]|#\[global_allocator\]", item.text) or inline_body(item) is not None
             or published(item.text))
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
    return "".join(out).rstrip("\n") + "\n", xmla_text, moved


def split_tests(root: pathlib.Path, moves: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Every core test file's items reaching XMLA, moved byte-identical into
    `rust/xmla/tests/` at the same relative path (S4's split, D39)."""
    move_map = dict(moves)
    written: list[tuple[str, str]] = []
    needs: dict[str, list[str]] = collections.defaultdict(list)
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED):
            continue
        rel = f[len("rust/tests/"):]
        single = "/" not in rel
        home_text, xmla_text, moved = split_level(read(root / f), True, set())
        if xmla_text is None:
            continue
        dest_rel = f"{CRATE_DIR}/tests/{rel}"
        if (root / dest_rel).exists() or dest_rel in move_map.values():
            residue(dest_rel, f"a file of this name exists or arrives by a move; the split tests of `{f}` were not written")
            continue
        pinned = pinned_source(rel)
        what = "rows" if single else "tests"
        header = (
            f"//! The {what} of `{f}`"
            + (f", which pins `{pinned}`," if pinned else "")
            + f" that need\n//! `{PACKAGE}`, moved byte-identical (S6, D39): a test lives in the lowest\n"
            "//! crate that can name everything it uses.\n//!\n"
        )
        write(root / dest_rel, tidy_head(prune_imports(prune_imports(header + xmla_text))))
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


def install_in_sibling_tests(root: pathlib.Path) -> list[str]:
    """A sibling leaf's test reaching XMLA stays where it is - S4's market half
    of `xmla/dbtype.rs` among them, where S4 left it - the crate a
    dev-dependency of the leaf and installed by the test (D39: neither
    sibling is below the other)."""
    touched: list[str] = []
    for manifest in sorted((root / "rust").glob("*/Cargo.toml")):
        leaf = manifest.parent.name
        if leaf in s6a.CORE_FOLDERS or leaf == XMLA:
            continue
        tests_dir = manifest.parent / "tests"
        paths = sorted(tests_dir.rglob("*.rs")) if tests_dir.exists() else []
        for path in paths:
            rel = str(path.relative_to(root))
            if "/support/" in rel:
                continue
            text = read(path)

            def level(text: str, inherited: set[str]) -> str:
                head, items = top_items(text)
                owner, _ = item_owners(items, inherited)
                imports = inherited | file_imports(items)
                out = [head]
                pos = len(head)
                for item in items:
                    out.append(text[pos:item.start])
                    pos = item.end
                    piece = item.text
                    if item.kind != "use" and owner[id(item)] == XMLA:
                        body = inline_body(item)
                        if body is not None:
                            start, end = body
                            piece = piece[:start] + level(piece[start:end], imports) + piece[end:]
                        else:
                            for at in reversed(test_fn_bodies(piece)):
                                if f"{CRATE}::install()" in piece[at:at + 300]:
                                    continue
                                line_start = piece.rfind("\n", 0, at) + 1
                                indent = re.match(r"[ \t]*", piece[line_start:]).group(0) + "    "
                                piece = piece[:at] + f'\n{indent}{CRATE}::install().expect("{PACKAGE} installs");' + piece[at:]
                    out.append(piece)
                out.append(text[pos:])
                return "".join(out)

            new = level(text, set())
            if new != text:
                write(path, new)
                touched.append(rel)
                manifest_text = read(manifest)
                if PACKAGE not in manifest_text:
                    if "[dev-dependencies]\n" in manifest_text:
                        manifest_text = manifest_text.replace(
                            "[dev-dependencies]\n", f"[dev-dependencies]\n{PACKAGE}.workspace = true\n", 1
                        )
                    else:
                        manifest_text = manifest_text.rstrip("\n") + f"\n\n[dev-dependencies]\n{PACKAGE}.workspace = true\n"
                    write(manifest, manifest_text)
                leaf_after(root, leaf, XMLA)
    return touched


def leaf_after(root: pathlib.Path, leaf: str, upstream: str) -> None:
    """A leaf whose tests link `upstream` runs again when it changes: its
    `[leaves]` line names `upstream` among what it is after."""
    rel = ".github/ci/rows.toml"
    if not (root / rel).exists():
        residue(rel, f"absent: say `{leaf}` is after `{upstream}` by hand")
        return
    text = read(root / rel)
    m = re.search(rf"(?m)^{re.escape(leaf)} = \{{ (.*) \}}$", text)
    if not m:
        residue(rel, f"no listed `[leaves]` line for `{leaf}`: say it is after `{upstream}` by hand")
        return
    body = m.group(1)
    after = re.search(r"after = \[([^\]]*)\]", body)
    if after and f'"{upstream}"' in after.group(1):
        return
    if after:
        listed = after.group(1).strip()
        body = body[:after.start(1)] + (f'{listed}, "{upstream}"' if listed else f'"{upstream}"') + body[after.end(1):]
    else:
        body = re.sub(r'(package = "[^"]+")', rf'\1, after = ["{upstream}"]', body, count=1)
    write(root / rel, text[:m.start(1)] + body + text[m.end(1):])


# ---------------------------------------------------------------------------
# Step 3: the crate's own sources
# ---------------------------------------------------------------------------

LIB_HEAD = """//! XML for Analysis 1.1 for the yggdryl core: the `.xmla` rowset medium over
//! the core's handles and Arrow, the SOAP 1.1 envelope it travels in, and the
//! tabular provider that serves a warehouse's catalogs over it - the
//! `yggdryl-xmla` crate.
//!
//! The record medium is [`XMLA_CODEC`], claimed on the core's register of
//! media by [`install`] at the rank the core held it at: every binding's init
//! and the `yggdryl` command's `main` call it, and a Rust program that links
//! this crate calls it before the core reads a handle whose name declares an
//! XML for Analysis document, which is refused naming the crate to install
//! until then. [`Xmla`], [`XmlaOptions`], the rowset document, [`soap`] and
//! the [`Service`] answer without it.
//!
"""

INSTALL_FN = '''
/// What this crate claims the XML for Analysis medium as.
const CRATE: &str = "yggdryl-xmla";

/// Claims the XML for Analysis medium on the core's register of media -
/// [`XMLA_CODEC`] under `application/xmla+xml`, at rank 4, which
/// [`RESERVED_RANKS`](yggdryl::media::RESERVED_RANKS) keeps for it since the
/// core held it there - once for the life of the process; a later call
/// returns at once. Every binding's init and the `yggdryl` command's `main`
/// call it, and so does a Rust caller before the core reads a handle whose
/// name declares an XML for Analysis document: until the claim such a
/// handle's options are refused, naming the crate to install.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed
/// `application/xmla+xml` or the name `xmla` first.
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
    yggdryl::media::codec::claim(&XMLA_CODEC, CRATE)?;
    let _ = INSTALLED.set(());
    Ok(())
}
'''

INTERNALS_PLACEHOLDER = "// GENERATED by scripts/generate_internals.py - do not edit by hand.\n// END GENERATED\n"


def build_lib(root: pathlib.Path) -> None:
    """`mod.rs` as the crate root: its doc led by the crate's, its table
    naming `soap`, its declarations with `pub mod soap`, `install()`."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    text = read(root / rel)
    m = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if not m:
        residue(rel, "no crate doc to replace")
        return
    paragraphs = m.group(0).split("//!\n")
    # The first paragraph is the one-line summary the crate's head restates.
    doc = "//!\n".join(paragraphs[1:])
    doc = replace_once(
        doc,
        "//! | [`service`] |",
        "//! | [`soap`] | the SOAP 1.1 envelope, its header blocks, its body and the fault - the protocol vocabulary only XML for Analysis speaks here, so it is this crate's |\n//! | [`service`] |",
        rel,
    )
    doc = replace_once(
        doc,
        "//! The SOAP envelope and fault are XML's own, in [`crate::soap`], and the\n"
        "//! HTTP it travels over is the crate's `http` server and\n"
        "//! client; this module speaks XMLA over them.\n",
        "//! The SOAP envelope and fault are XML's own, in [`soap`], and the HTTP it\n"
        "//! travels over is the core's `http` server and client; this crate speaks\n"
        "//! XMLA over them.\n",
        rel,
    )
    body = text[m.end():].lstrip("\n")
    body = replace_once(body, "pub mod service;\n", "pub mod service;\npub mod soap;\n", rel)
    new = (
        LIB_HEAD + doc.rstrip("\n") + "\n\n#![deny(unsafe_code)]\n\n"
        + "use std::sync::{Mutex, OnceLock, PoisonError};\n\n"
        + body.rstrip("\n") + "\n" + INSTALL_FN + "\n" + INTERNALS_PLACEHOLDER
    )
    write(root / rel, new)


ROWSET_OLD = """        let borrowed: Vec<&Scalar> = rows.iter().collect();
        crate::serie::from_canonical_rows(Arc::clone(&self.field), &borrowed)
    }"""

ROWSET_NEW = """        // The field's own contract typed every row above, and the core's one
        // value door lands them, proving each against the field itself: no
        // proof crosses into the core from outside it.
        Serie::from_scalars(Arc::clone(&self.field), rows)
    }"""


def edit_crate_sources(root: pathlib.Path) -> None:
    """What the crate reaches of the core that no path names: a proof the
    core alone may hold, and crate-private methods, through the core's
    `<type>_<item>` forwarders; the sentences that said `the core`."""
    src = f"{CRATE_DIR}/src"
    edit(
        root, f"{src}/rowset.rs",
        (ROWSET_OLD, ROWSET_NEW),
        ("        Shape::Text | Shape::FixedText if child.is_string_storage() => {\n",
         "        Shape::Text | Shape::FixedText if crate::implementer::serie_is_string_storage(child) => {\n"),
        ("        Shape::Bytes if child.is_byte_storage() => {\n",
         "        Shape::Bytes if crate::implementer::serie_is_byte_storage(child) => {\n"),
    )
    path = root / src / "rowset.rs"
    text = read(path)
    text = s6a.replace_n(text, "            let Some(bytes) = child.value_bytes(row) else {\n",
                         "            let Some(bytes) = crate::implementer::serie_value_bytes(child, row) else {\n",
                         2, f"{src}/rowset.rs")
    write(path, text)
    edit(
        root, f"{src}/service.rs",
        ("        plan.map_sources(|source| match source {\n",
         "        crate::implementer::plan_map_sources(plan, |source| match source {\n"),
    )
    edit(
        root, f"{src}/soap/mod.rs",
        ("//! rather than held whole. The HTTP a SOAP endpoint is reached over is the\n"
         "//! crate's `http` server and client; what SOAP 1.1 states of\n",
         "//! rather than held whole. The HTTP a SOAP endpoint is reached over is the\n"
         "//! core's `http` server and client; what SOAP 1.1 states of\n"),
    )
    if "from_canonical_rows" in read(root / src / "rowset.rs"):
        residue(f"{src}/rowset.rs", "`from_canonical_rows` is left: land the rows through `Serie::from_scalars`")


def bench_root(root: pathlib.Path) -> None:
    """The moved bench module is the crate's bench root: the profile beside
    it, one group, and a `main` that claims the medium first."""
    rel = f"{CRATE_DIR}/benchmarks/xmla.rs"
    path = root / rel
    if not path.exists():
        residue(rel, "the moved XMLA bench is missing")
        return
    text = read(path)
    head = re.match(r"(?:[ \t]*//![^\n]*\n)+\n?", text)
    at = head.end() if head else 0
    text = (
        text[:at]
        + "#[path = \"../../benchmarks/bench_profile.rs\"]\nmod bench_profile;\n\n"
        + text[at:].rstrip("\n")
        + "\n\ncriterion::criterion_group!(xmla, xmla_benchmarks);\n\n"
        "fn main() {\n"
        "    // The medium the benchmarks read by name is claimed before any runs.\n"
        f'    {CRATE}::install().expect("{PACKAGE} installs");\n'
        "    xmla();\n"
        "    criterion::Criterion::default()\n"
        "        .configure_from_args()\n"
        "        .final_summary();\n"
        "}\n"
    )
    text = replace_once(text, "pub(crate) fn xmla_benchmarks(", "fn xmla_benchmarks(", rel)
    write(path, text)


def edit_core_media_bench(root: pathlib.Path) -> None:
    edit(
        root, "rust/benchmarks/media.rs",
        ('#[path = "media/xmla.rs"]\nmod xmla;\n', ""),
        ("    xmla::xmla_benchmarks,\n", ""),
    )


# ---------------------------------------------------------------------------
# Step 4: the core's seams
# ---------------------------------------------------------------------------


def edit_codec(root: pathlib.Path) -> None:
    """The seed claim leaves; `RESERVED_RANKS` and the claim rule (D39)
    where no earlier media move wrote them - S6a's text, so the moves agree."""
    rel = "rust/src/media/codec.rs"
    edit(root, rel, ("            &crate::xmla::XMLA_CODEC,\n", ""))
    if "RESERVED_RANKS" in read(root / rel):
        done("media/codec.rs: the seed claim of XMLA_CODEC gone; RESERVED_RANKS was there")
        return
    edit(
        root, rel,
        ("/// The rank a medium outside the core takes at least: the core's seven hold\n"
         "/// the positions below it, in the order their options sort.\n"
         "pub const EXTERNAL_RANK: u8 = 32;\n",
         "/// The rank a medium outside the core takes at least: the positions below\n"
         "/// it are the core's own media's and the [`RESERVED_RANKS`] of the media\n"
         "/// split off it, in the order their options sort.\n"
         "pub const EXTERNAL_RANK: u8 = 32;\n" + s6a.RESERVED),
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
        (s6a.CLAIM_OLD, s6a.CLAIM_NEW),
    )
    edit(root, "rust/src/media/mod.rs",
         ("pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper,",
          "pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper, RESERVED_RANKS,"))
    done("media/codec.rs: the seed claim of XMLA_CODEC gone, RESERVED_RANKS and the claim rule written")


def edit_core_lib_and_manifest(root: pathlib.Path) -> None:
    edit(root, "rust/src/lib.rs", ("pub mod soap;\n", ""), ("pub mod xmla;\n", ""))
    rel = "rust/Cargo.toml"
    text = read(root / rel)
    head = text.split("[features]")[0]
    m = re.search(r'exclude = \[([^\]]*)\]', head)
    if m:
        if '"/xmla"' not in m.group(1):
            text = text[:m.start(1)] + m.group(1) + ', "/xmla"' + text[m.end(1):]
    else:
        text = replace_once(
            text,
            'categories = ["data-structures", "encoding"]\n',
            'categories = ["data-structures", "encoding"]\n'
            "# The split crates sit inside this folder; each is its own package.\n"
            'exclude = ["/xmla"]\n',
            rel,
        )
    write(root / rel, text)


# The door: what `yggdryl-xmla` reaches of the core, each item added once -
# by this move, or by an earlier media move that reached it too.
IMPLEMENTER_HEADER = """
// ------------------------------------------------------------------------
// Media: what the media crates reach - the record doors a wrapper redirects
// to, the dimensions and the schema it answers through, and the helpers one
// medium's codec reads. An item is added once, by the first move reaching it.
// ------------------------------------------------------------------------
"""

R_IOBASE = ["append_arrow_reader_default", "leaf_writer", "merge_arrow_reader_default",
            "overwrite_arrow_reader_default_with_field"]
R_IOMEDIA = ["container_field", "container_origin", "container_row_size", "dimension_options",
             "field_under", "own_options", "read_record_serie"]

R_IOBASE_DOC = """/// The default record writes a media wrapper redirects to once its options
/// are proven its own, and the one encode of a leaf's whole contents, for the
/// media crates: raised inside the crate-private `iobase`.
"""
R_IOMEDIA_DOC = """/// What a media wrapper answers its options, its origin, its dimensions and
/// its schema through, for the media crates: the handle's own options where
/// none were given, a container's root, origin and row count, the options a
/// dimension is read under, the one schema answer under the options' clauses,
/// and the composed record read; raised inside the crate-private `iomedia`.
"""

# Forwarders (F) and the one associated item (A), by the name they answer.
FORWARDERS: dict[str, tuple[str, str]] = {
    "arrow_schema_from_field": ('''/// `arrow::arrow_schema_from_field`, for the media crates: the Arrow schema a
/// record root projects to, its dictionary-ID sidecar written.
///
/// # Errors
///
/// Returns an error when the root is not a record or a child has no Arrow
/// projection.
#[inline]
pub fn arrow_schema_from_field(
    field: &crate::Field,
) -> crate::arrow::Result<arrow_schema::SchemaRef> {
    crate::arrow::arrow_schema_from_field(field)
}
''', "pub fn arrow_schema_from_field(field: &crate::Field) -> crate::arrow::Result<arrow_schema::SchemaRef>  (`arrow::arrow_schema_from_field`: the Arrow schema a record root projects to)"),
    "cache_now": ('''/// `media::cache::now`, for the media crates: the instant every cache door
/// reads, which a test sets by hand under `internals`.
#[inline]
#[must_use]
pub fn cache_now() -> std::time::Instant {
    crate::media::cache::now()
}
''', "pub fn cache_now() -> std::time::Instant  (`media::cache::now`: the instant every cache door reads)"),
    "uuid_parse": ('''/// `uuid::uuid_parse`, for the media crates: bytes validated as one
/// identifier, answered as its sixteen storage bytes.
///
/// # Errors
///
/// Returns an error naming the accepted spellings when the bytes are neither.
#[inline]
pub fn uuid_parse(value: &[u8]) -> crate::Result<[u8; 16]> {
    crate::uuid_parse(value)
}
''', "pub fn uuid_parse(value: &[u8]) -> crate::Result<[u8; 16]>  (`uuid::uuid_parse`: bytes validated as one identifier)"),
    "into_base64": ('''/// `bytes::into_base64`, for the XMLA crate: a payload as the one base64
/// string the crate's byte spellings write.
#[inline]
#[must_use]
pub fn into_base64(payload: &[u8]) -> String {
    crate::bytes::into_base64(payload)
}
''', "pub fn into_base64(payload: &[u8]) -> String  (`bytes::into_base64`: a payload as the one base64 string)"),
    "parse_timestamp": ('''/// `temporal::parse_timestamp`, for the XMLA crate: a zoned instant read as
/// its count, its unit and its zone, the zone required.
///
/// # Errors
///
/// Returns the reader's refusal of text that is no zoned instant.
#[inline]
pub fn parse_timestamp(text: &str) -> crate::Result<(i64, crate::TimeUnit, crate::Timezone)> {
    crate::temporal::parse_timestamp(text)
}
''', "pub fn parse_timestamp(text: &str) -> crate::Result<(i64, crate::TimeUnit, crate::Timezone)>  (`temporal::parse_timestamp`: a zoned instant, the zone required)"),
    "format_timestamp": ('''/// `temporal::format_timestamp`, for the XMLA crate: a zoned instant
/// spelled as its local reading plus its offset.
#[inline]
#[must_use]
pub fn format_timestamp(
    count: i64,
    unit: crate::TimeUnit,
    zone: &crate::Timezone,
) -> Option<smol_str::SmolStr> {
    crate::temporal::format_timestamp(count, unit, zone)
}
''', "pub fn format_timestamp(count: i64, unit: crate::TimeUnit, zone: &crate::Timezone) -> Option<smol_str::SmolStr>  (`temporal::format_timestamp`: a zoned instant as its local reading plus its offset)"),
    "normalize_path": ('''/// `http::server::normalize_path`, for the XMLA crate: the canonical
/// spelling of a mount prefix or a route path.
///
/// # Errors
///
/// Returns a parse error for a query, a fragment or a control byte in it.
#[cfg(feature = "http")]
#[inline]
pub fn normalize_path(path: &str) -> crate::Result<String> {
    crate::http::server::normalize_path(path)
}
''', "pub fn normalize_path(path: &str) -> crate::Result<String>  (`http::server::normalize_path`: the canonical spelling of a mount prefix or a route path)"),
    "holds": ('''/// `warehouse::holds`, for the XMLA crate: whether `prefix` holds `url` on a
/// path boundary, and how long the match is - the one containment rule every
/// location check reads.
#[inline]
#[must_use]
pub fn holds(prefix: &str, url: &str) -> Option<usize> {
    crate::warehouse::holds(prefix, url)
}
''', "pub fn holds(prefix: &str, url: &str) -> Option<usize>  (`warehouse::holds`: whether a prefix holds a URL on a path boundary)"),
    "no_catalog": ('''/// `warehouse::no_catalog`, for the XMLA crate: the absence of a catalog
/// called `name`, as the warehouse refuses it.
#[inline]
#[must_use]
pub fn no_catalog(name: &str) -> crate::Error {
    crate::warehouse::no_catalog(name)
}
''', "pub fn no_catalog(name: &str) -> crate::Error  (`warehouse::no_catalog`: the absence of a catalog by name)"),
    "path_text": ('''/// `warehouse::path_text`, for the XMLA crate: a path rendered as the plan
/// grammar spells it, which is how every error names one.
#[inline]
#[must_use]
pub fn path_text(parts: &[smol_str::SmolStr]) -> String {
    crate::warehouse::path_text(parts)
}
''', "pub fn path_text(parts: &[smol_str::SmolStr]) -> String  (`warehouse::path_text`: a path as the plan grammar spells it)"),
    "write_fragment": ('''/// `xml::write_fragment`, for the XMLA crate: `<name>` holding a natural
/// value as one element, spelled exactly as the document writer spells it.
///
/// # Errors
///
/// Returns the writer's refusals for the value: what has no XML spelling.
#[inline]
pub fn write_fragment<W: std::io::Write>(
    writer: &mut W,
    name: &str,
    value: &crate::Scalar,
) -> crate::Result<()> {
    crate::xml::write_fragment(writer, name, value)
}
''', "pub fn write_fragment<W: std::io::Write>(writer: &mut W, name: &str, value: &crate::Scalar) -> crate::Result<()>  (`xml::write_fragment`: one element holding a natural value)"),
    "write_leaf_text": ('''/// `xml::write_leaf_text`, for the media crates: one leaf value as an
/// element's character data in the document writer's spelling, `context`
/// naming the column or element a refusal is reported for.
///
/// # Errors
///
/// Returns [`crate::Error::Codec`] for a value that is not a leaf, or a leaf
/// with no XML spelling.
#[inline]
pub fn write_leaf_text<W: std::io::Write>(
    writer: &mut W,
    value: &crate::Scalar,
    context: &str,
) -> crate::Result<()> {
    crate::xml::write_leaf_text(writer, value, context)
}
''', "pub fn write_leaf_text<W: std::io::Write>(writer: &mut W, value: &crate::Scalar, context: &str) -> crate::Result<()>  (`xml::write_leaf_text`: one leaf as character data)"),
    "write_x_escape": ('''/// `xml::write_x_escape`, for the media crates: `character` as the
/// `_xHHHH_` escape Office Open XML gives a character a name cannot carry.
#[inline]
pub fn write_x_escape(target: &mut String, character: char) {
    crate::xml::write_x_escape(target, character);
}
''', "pub fn write_x_escape(target: &mut String, character: char)  (`xml::write_x_escape`: a character as its `_xHHHH_` escape)"),
    "decode_x_escapes": ('''/// `xml::decode_x_escapes`, for the media crates: every `_xHHHH_` escape
/// read back into the character it spells, text carrying none borrowed.
#[inline]
#[must_use]
pub fn decode_x_escapes(encoded: &str) -> std::borrow::Cow<'_, str> {
    crate::xml::decode_x_escapes(encoded)
}
''', "pub fn decode_x_escapes(encoded: &str) -> std::borrow::Cow<'_, str>  (`xml::decode_x_escapes`: the `_xHHHH_` escapes read back)"),
    "write_element_text": ('''/// `xml::write_element_text`, for the media crates: text as element
/// character data, escaped.
///
/// # Errors
///
/// Returns [`crate::Error::Codec`] for a character XML 1.0 cannot carry.
#[inline]
pub fn write_element_text<W: std::io::Write>(writer: &mut W, text: &str) -> crate::Result<()> {
    crate::xml::write_element_text(writer, text)
}
''', "pub fn write_element_text<W: std::io::Write>(writer: &mut W, text: &str) -> crate::Result<()>  (`xml::write_element_text`: text as escaped character data)"),
    "write_attribute_text": ('''/// `xml::write_attribute_text`, for the media crates: text as a quoted
/// attribute's value, escaped, without the quotes.
///
/// # Errors
///
/// Returns [`crate::Error::Codec`] for a character XML 1.0 cannot carry.
#[inline]
pub fn write_attribute_text<W: std::io::Write>(writer: &mut W, text: &str) -> crate::Result<()> {
    crate::xml::write_attribute_text(writer, text)
}
''', "pub fn write_attribute_text<W: std::io::Write>(writer: &mut W, text: &str) -> crate::Result<()>  (`xml::write_attribute_text`: text as an escaped attribute value)"),
    "is_name_start": ('''/// `xml::is_name_start`, for the XMLA crate: XML 1.0's `NameStartChar`.
#[inline]
#[must_use]
pub const fn is_name_start(character: char) -> bool {
    crate::xml::is_name_start(character)
}
''', "pub const fn is_name_start(character: char) -> bool  (`xml::is_name_start`: XML 1.0's NameStartChar)"),
    "is_name_char": ('''/// `xml::is_name_char`, for the XMLA crate: XML 1.0's `NameChar`.
#[inline]
#[must_use]
pub const fn is_name_char(character: char) -> bool {
    crate::xml::is_name_char(character)
}
''', "pub const fn is_name_char(character: char) -> bool  (`xml::is_name_char`: XML 1.0's NameChar)"),
    "shaped": ('''/// `xml::shaped`, for the XMLA crate: a natural value restated in the shape
/// a field's XML reading takes - a repeated element read once one item,
/// text trimmed and empty text null under a leaf that is not text - before
/// the field's contract types it.
#[inline]
#[must_use]
pub fn shaped(value: crate::Scalar, field: &crate::Field) -> crate::Scalar {
    crate::xml::shaped(value, field)
}
''', "pub fn shaped(value: crate::Scalar, field: &crate::Field) -> crate::Scalar  (`xml::shaped`: a natural value in the shape a field's XML reading takes)"),
    "plan_map_sources": ('''/// `Plan::map_sources`, for the XMLA crate: the plan with every source it
/// reads - its `from`, then each join's - replaced by what `map` answers for
/// it, the one door that rewrites where a plan reads.
///
/// # Errors
///
/// The first error `map` answers, the plan dropped with it.
#[inline]
pub fn plan_map_sources(
    plan: crate::expression::Plan,
    map: impl FnMut(crate::expression::Source) -> crate::Result<crate::expression::Source>,
) -> crate::Result<crate::expression::Plan> {
    plan.map_sources(map)
}
''', "pub fn plan_map_sources(plan: crate::expression::Plan, map: impl FnMut(crate::expression::Source) -> crate::Result<crate::expression::Source>) -> crate::Result<crate::expression::Plan>  (`Plan::map_sources`: every source a plan reads replaced by what the map answers)"),
}


def exported_names(text: str) -> set[str]:
    """Every name `implementer.rs` answers: its `pub use` leaves, its `pub`
    items."""
    names: set[str] = set()
    for m in re.finditer(r"(?ms)^pub use (.*?);", code_only(text)):
        try:
            for leaf, alias, kind in parse_use(re.sub(r"\s+", " ", m.group(1))):
                if kind == "self" and leaf:
                    names.add(alias or leaf[-1])
        except UseParseError:
            continue
    names |= set(re.findall(r"(?m)^pub (?:const )?(?:fn|struct|enum|const|static|type|trait) (\w+)", text))
    return names


def raise_fns(root: pathlib.Path, rel: str, names: list[str]) -> None:
    """`pub(crate) fn <name>` raised to `pub fn` (R): the module holding it is
    one the crate root does not publish."""
    path = root / rel
    text = read(path)
    for name in names:
        if re.search(rf"(?m)^pub fn {name}\b", text):
            continue
        pattern = re.compile(rf"(?m)^pub\(crate\) fn {name}\b")
        if len(pattern.findall(text)) != 1:
            residue(rel, f"`pub(crate) fn {name}` matched {len(pattern.findall(text))} times, not raised")
            continue
        text = pattern.sub(f"pub fn {name}", text)
    write(path, text)


def publish_transfer(root: pathlib.Path, names: list[str]) -> None:
    """`iobase.rs` re-exports the raised `transfer` items `pub` - which
    publishes nothing, `iobase` being private - the rest `pub(crate)`."""
    rel = "rust/src/iobase.rs"
    text = read(root / rel)
    crate_use = re.search(r"pub\(crate\) use transfer::\{([^}]*)\};", text)
    pub_use = re.search(r"(?m)^pub use transfer::\{([^}]*)\};", text)
    if not crate_use or not pub_use:
        residue(rel, "no `pub(crate) use transfer::{..}` and `pub use transfer::{..}` pair to move the raised names between")
        return
    held = [n.strip() for n in crate_use.group(1).split(",") if n.strip()]
    public = [n.strip() for n in pub_use.group(1).split(",") if n.strip()]
    moving = [n for n in names if n in held]
    if not moving:
        return
    kept = [n for n in held if n not in moving]
    merged = sorted(set(public) | set(moving), key=lambda n: (not n[0].isupper(), n))
    new_pub = "pub use transfer::{" + ", ".join(merged) + "};"
    new_crate = ("pub(crate) use transfer::{" + ", ".join(kept) + "};") if len(kept) > 1 else (
        f"pub(crate) use transfer::{kept[0]};" if kept else "")
    # Replace the later span first so the earlier offsets hold.
    spans = sorted([(crate_use.start(), crate_use.end(), new_crate), (pub_use.start(), pub_use.end(), new_pub)], reverse=True)
    for start, end, new in spans:
        text = text[:start] + new + text[end:]
    write(root / rel, text)


def edit_implementer(root: pathlib.Path) -> list[str]:
    """The door, item by item: an item an earlier move added is used as it
    is; a missing one is added by its route. Answers the inventory lines of
    the items added."""
    rel = "rust/src/implementer.rs"
    text = read(root / rel)
    have = exported_names(text)
    added_code: list[str] = []
    inventory: list[str] = []
    missing_iobase = [n for n in R_IOBASE if n not in have]
    if missing_iobase:
        raise_fns(root, "rust/src/iobase/transfer.rs", missing_iobase)
        publish_transfer(root, missing_iobase)
        added_code.append(R_IOBASE_DOC + "pub use crate::iobase::{" + ", ".join(missing_iobase) + "};\n")
        inventory.append("pub use iobase::{" + ", ".join(missing_iobase) + "}  (raised inside the crate-private `iobase`: the default record writes a media wrapper redirects to once its options are proven its own, and the one encode of a leaf's whole contents)")
    missing_iomedia = [n for n in R_IOMEDIA if n not in have]
    if missing_iomedia:
        raise_fns(root, "rust/src/iomedia.rs", missing_iomedia)
        # A raised item's doc names no crate-private item by link.
        pairs = []
        if "container_origin" in missing_iomedia:
            pairs.append(("/// Returns what [`container_field`] returns.\n", "/// Returns what `container_field` returns.\n"))
        if "read_record_serie" in missing_iomedia:
            pairs.append(("/// [`compose`](crate::media_serie::compose): the medium's native reader is\n",
                          "/// `compose`: the medium's native reader is\n"))
        if pairs:
            edit(root, "rust/src/iomedia.rs", *pairs)
        added_code.append(R_IOMEDIA_DOC + "pub use crate::iomedia::{" + ", ".join(missing_iomedia) + "};\n")
        inventory.append("pub use iomedia::{" + ", ".join(missing_iomedia) + "}  (raised inside the crate-private `iomedia`: the handle's own options where none were given, a container's root, origin and row count, the options a dimension is read under, the one schema answer under the options' clauses, the composed record read)")
    for name, (code, line) in FORWARDERS.items():
        if name in have:
            continue
        added_code.append(code)
        inventory.append(line)
    if not added_code:
        done("implementer.rs: every item XMLA reaches was there")
        return []
    if "// Media: what the media crates reach" not in text:
        text = text.rstrip("\n") + "\n" + IMPLEMENTER_HEADER
    text = text.rstrip("\n") + "\n\n" + "\n".join(added_code)
    write(root / rel, text)
    done(f"implementer.rs: {len(inventory)} entr(ies) added for XMLA")
    return inventory


# ---------------------------------------------------------------------------
# Step 5: paths, everywhere
# ---------------------------------------------------------------------------

XMLA_NAMED = re.compile(r"\byggdryl::(?:xmla\b|soap\b|internals::(?:xmla|soap)_|\{[^;]*\b(?:xmla|soap)\b)")


def markdown(rewriter, text: str, where: str) -> str:
    """S6a's markdown rewrite with XMLA's names: only the Rust blocks and the
    prose that name a moving module, a block whose lines leave its indent
    refused."""
    outside = s4.Context(None, False)
    out: list[str] = []
    pos = 0
    for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
        prose = text[pos:m.start(3)]
        out.append(rewriter.inline(prose, outside, where) if XMLA_NAMED.search(prose) else prose)
        body = m.group(3)
        if m.group(2).strip().startswith("rust") and XMLA_NAMED.search(body):
            indent = m.group(1)
            lines = body.split("\n")
            if indent and not all(l.startswith(indent) or not l.strip() for l in lines):
                residue(f"{where}:{line_of(text, m.start(3))}", "a Rust block with a line outside its indent names XMLA: re-spell it by hand")
            elif indent:
                stripped = "\n".join(l[len(indent):] for l in lines)
                new = rewriter.rust(stripped, outside, where)
                body = "\n".join((indent + l) if l else l for l in new.split("\n"))
            else:
                body = rewriter.rust(body, outside, where)
        elif XMLA_NAMED.search(body):
            body = rewriter.inline(body, outside, where)
        out.append(body)
        pos = m.end(3)
    rest = text[pos:]
    out.append(rewriter.inline(rest, outside, where) if XMLA_NAMED.search(rest) else rest)
    return "".join(out)


def rewrite_paths(root: pathlib.Path, rewriter, paths: PathMap, moves, splits) -> None:
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new[new] = old
    crate_ctx = s4.Context(XMLA, True)
    outside = s4.Context(None, False)
    count = 0
    local = paths.with_files({o: n for o, n in splits})
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        if "/fixtures/" in rel:
            continue
        old = moved_new.get(rel, rel)
        text = read(path)
        new = s4.reanchor(text, old, rel, local)
        ctx = crate_ctx if rel.startswith(f"{CRATE_DIR}/src/") else outside
        new = rewriter.rs_file(new, ctx, rel)
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
            new = s4.reanchor(text, rel, rel, paths)
            if folder != "rust/src":
                new = rewriter.rs_file(new, outside, rel)
            if new != text:
                write(path, new)
                count += 1
    # A sibling leaf's sources and tests name the crate as any caller does.
    for leaf in sorted((root / "rust").glob("*/Cargo.toml")):
        if leaf.parent.name in s6a.CORE_FOLDERS or leaf.parent.name == XMLA:
            continue
        for path in sorted(leaf.parent.rglob("*.rs")):
            rel = str(path.relative_to(root))
            if "/target/" in rel:
                continue
            text = read(path)
            new = rewriter.rs_file(s4.reanchor(text, rel, rel, paths), outside, rel)
            if new != text:
                write(path, new)
                count += 1
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text = read(root / f)
            new = docs_rs(text)
            if XMLA_NAMED.search(new):
                new = markdown(rewriter, new, f)
            if new != text:
                write(root / f, new)
                count += 1
    for f in ("AGENTS.md", "README.md", "rust/README.md", ".api-inventory.txt"):
        if (root / f).exists():
            text = read(root / f)
            new = docs_rs(rewriter.inline(text, outside, f))
            if new != text:
                write(root / f, new)
                count += 1
    done(f"paths: {rewriter.stats['use']} use statements and {rewriter.stats['inline']} paths re-owned, {count} files")
    count = 0
    for f in s6a.text_files(root):
        if f.startswith(f"{CRATE_DIR}/tests/xmla/fixtures/"):
            continue
        text = read(root / f)
        new = rewrite_text_paths(text, paths, moves)
        if new != text:
            write(root / f, new)
            count += 1
    path = root / ".gitattributes"
    if path.exists():
        text = read(path)
        new = text.replace("rust/tests/xmla/fixtures/** -text", f"{CRATE_DIR}/tests/xmla/fixtures/** -text")
        if new != text:
            write(path, new)
            count += 1
        elif f"{CRATE_DIR}/tests/xmla/fixtures/" not in text:
            residue(".gitattributes", "no `rust/tests/xmla/fixtures/** -text` line to re-point")
    done(f"file paths re-pointed in {count} files")


XMLA_HARNESS_DOC = """//! One test file per file under `rust/xmla/src/`, under `tests/xmla/`.
//!
//! `rust/xmla/tests/` mirrors `rust/xmla/src/`: a source file has exactly one
//! test file at the matching path - `lib.rs` pinned by `xmla/mod_.rs`, the
//! module it was in the core - and this target is the harness for the XML
//! for Analysis medium, its provider and its server. A test reaches the crate
//! through `yggdryl_xmla::` and the core through `yggdryl::`, and nothing
//! else.
"""

SOAP_HARNESS_DOC = """//! One test file per file under `rust/xmla/src/soap/`, under `tests/soap/`.
//!
//! `rust/xmla/tests/` mirrors `rust/xmla/src/`: a source file has exactly one
//! test file at the matching path, and this target is the harness for the
//! SOAP 1.1 envelope the crate speaks XML for Analysis in. A test reaches the
//! crate through `yggdryl_xmla::` and the core through `yggdryl::`, and
//! nothing else.
"""


def finish_crate_text(root: pathlib.Path) -> None:
    """What the rewrite must not read as the core's - a link to the crate's
    own `install` - and the moved harnesses' sentences about themselves."""
    edit(
        root, f"{CRATE_DIR}/src/media.rs",
        ("/// The XMLA rowset medium, claimed by the core under its MIME type.\n",
         "/// The XMLA rowset medium, claimed under its MIME type by\n"
         "/// [`install`](crate::install) at the rank the core held it at.\n"),
    )
    for name, doc in (("xmla.rs", XMLA_HARNESS_DOC), ("soap.rs", SOAP_HARNESS_DOC)):
        path = root / CRATE_DIR / "tests" / name
        if not path.exists():
            residue(f"{CRATE_DIR}/tests/{name}", "the moved harness is missing")
            continue
        text = read(path)
        new = re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", doc, text, count=1)
        if new != text:
            write(path, new)


# ---------------------------------------------------------------------------
# Step 6: install
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = """//! The claim every harness of `yggdryl-xmla` makes before a test reads a
//! name: the core's register answers the XML for Analysis medium only once
//! `yggdryl-xmla` has claimed it (D7), so each test opens with [`installed`].

/// Claims the XML for Analysis medium, once for the process.
pub fn installed() {
    yggdryl_xmla::install().expect("yggdryl-xmla claims its medium");
}
"""

INSTALL_TEST = """
/// `install()` claims the medium under its own name at the rank the core held
/// it at (D39), once: a later call returns at once, and the claim stands
/// against another crate's.
#[test]
fn install_claims_xmla_at_its_reserved_rank_once() {
    yggdryl_xmla::install().expect("the medium is claimed");
    yggdryl_xmla::install().expect("a later install returns at once");
    let held = yggdryl::media::codec_for(&yggdryl::MimeType::XMLA).expect("the medium is claimed");
    assert_eq!((held.name(), held.title(), held.rank()), ("xmla", "XMLA", 4));
    assert!(std::ptr::addr_eq(
        held,
        &yggdryl_xmla::XMLA_CODEC as &dyn yggdryl::media::MediaCodec
    ));
    let refused = yggdryl::media::codec::claim(&yggdryl_xmla::XMLA_CODEC, "another")
        .expect_err("a second claim of the medium is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-xmla"), "{refused}");
}
"""


def markdown_installs(text: str) -> tuple[str, int]:
    """A Rust block naming the crate, or reading a handle whose name declares
    XMLA through the register, installs it first (S6a's rule)."""
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or f"{CRATE}::install()" in body:
            return m.group(0)
        if not (re.search(rf"\b{CRATE}\b", body) or "MimeType::XMLA" in body
                or any(literal_reads_xmla(l.group(0)) for l in s4.STRING_LITERAL.finditer(body))):
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
                lines[at + 1:at + 1] = [f"{indent}{CRATE}::install()?;", ""]
            else:
                lines.insert(at, f"{indent}{CRATE}::install()?;")
        count += 1
        return f"{indent}```{info}\n" + "\n".join(lines) + f"{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text), count


def install_at_init(root: pathlib.Path) -> None:
    """The bindings' init and the CLI's `main` claim the medium after every
    crate that installs there already, before anything reads a name."""
    rel = "python/src/lib.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        found = list(re.finditer(r"(?m)^    yggdryl_\w+::install\(\)\.map_err\(value_error\)\?;\n", text))
        if found:
            at = found[-1].end()
            text = text[:at] + f"    {CRATE}::install().map_err(value_error)?;\n" + text[at:]
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
        found = list(re.finditer(r'(?m)^    yggdryl_\w+::install\(\)\.expect\("[^"\n]*"\);\n', text))
        line = f'    {CRATE}::install().expect("{PACKAGE} claims its medium");\n'
        if found:
            at = found[-1].end()
            text = text[:at] + line + text[at:]
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
        chain = re.search(r"if let Err\(refusal\) = (yggdryl_\w+::install\(\)(?:\.and_then\(\|\(\)\| yggdryl_\w+::install\(\)\))*) \{", text)
        if chain:
            text = text[:chain.end(1)] + f".and_then(|()| {CRATE}::install())" + text[chain.end(1):]
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


def install_everywhere(root: pathlib.Path) -> None:
    base = root / CRATE_DIR
    write(base / "tests/support/install.rs", INSTALL_SUPPORT)
    harnesses = tests = docs = 0
    for path in sorted((base / "tests").glob("*.rs")):
        text = s4.declare_install(read(path))
        text = re.sub(r'\n{3,}(#\[path = "support/install\.rs"\])', r"\n\n\1", text, count=1)
        # The declaration closes its group: a `use` or an item after it is a
        # paragraph of its own.
        text = re.sub(r'(#\[path = "support/install\.rs"\]\nmod install;\n)(?=[^\n#m])', r"\1\n", text, count=1)
        write(path, text)
        harnesses += 1
    for path in sorted((base / "tests").rglob("*.rs")):
        rel = str(path.relative_to(root))
        if "/support/" in rel or "/fixtures/" in rel:
            continue
        text, n = s4.insert_test_installs(read(path))
        tests += n
        write(path, text)
    # The install test claims the medium itself, after every other opens with it.
    mod_ = base / "tests/xmla/mod_.rs"
    if mod_.exists() and "install_claims_xmla_at_its_reserved_rank_once" not in read(mod_):
        write(mod_, read(mod_).rstrip("\n") + "\n" + INSTALL_TEST)
    elif not mod_.exists():
        residue(f"{CRATE_DIR}/tests/xmla/mod_.rs", "missing: pin `install()`'s rank by hand")
    for path in sorted((base / "src").rglob("*.rs")):
        text, n = s4.insert_doc_installs(read(path), XMLA)
        if n:
            write(path, text)
            docs += n
    pages = 0
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text, n = markdown_installs(read(root / f))
            if n:
                write(root / f, text)
                pages += n
    install_at_init(root)
    done(f"install: {harnesses} harnesses, {tests} tests, {docs} rustdoc examples, the bench main, {pages} page blocks, the bindings and the CLI")


# ---------------------------------------------------------------------------
# Step 7: manifests
# ---------------------------------------------------------------------------

LIB_IDENTS = {"md5": "md-5", "iceberg_official": "iceberg-official"}
FEATURE_NOTES = {
    "http": "# `http` puts the provider on the core's HTTP server - `Service::route`, which\n"
            "# `yggdryl xmla serve` answers through.",
}


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
    # Only a crate a source names as its own path root: `use x::` or `x::` at
    # the start of a path, never a module segment inside a `use` tree.
    used = s4.crates_named(root, [f"{CRATE_DIR}/src"])
    used_dev = s4.crates_named(root, [f"{CRATE_DIR}/tests", f"{CRATE_DIR}/benchmarks"]) - used
    lines = ["yggdryl.workspace = true"]
    for name in sorted(used - skip):
        if name in by_ident:
            lines.append(s6a.dependency_line(*by_ident[name]))
    dev_lines = []
    for name in sorted(used_dev - skip):
        if name in by_ident:
            dev_lines.append(s6a.dependency_line(*by_ident[name]))
        elif name in dev_by_ident:
            dev_lines.append(s6a.dependency_line(*dev_by_ident[name]))
    if not any(l.startswith("criterion") for l in dev_lines):
        dev_lines.append(s6a.dependency_line("criterion", dev.get("criterion", "0.7")))
    for leaf in ("yggdryl_market", "yggdryl_fix", "yggdryl_excel", "yggdryl_avro", "yggdryl_parquet"):
        if leaf in used_dev | used:
            dev_lines.append(f"{leaf.replace('_', '-')}.workspace = true")
            residue(f"{CRATE_DIR}/Cargo.toml", f"the crate's tests name `{leaf}`: a dev-dependency of a sibling leaf, its install and `after` to check")
    core_features = core_manifest.get("features", {})
    found: set[str] = set()
    for path in (root / CRATE_DIR).rglob("*.rs"):
        found |= set(re.findall(r'feature\s*=\s*"([\w-]+)"', read(path)))
    found.discard("internals")
    unknown = sorted(found - set(core_features))
    if unknown:
        residue(f"{CRATE_DIR}/Cargo.toml", f"the crate's code reads features the core does not have: {unknown}")
    features = [("default", [])]
    for name in sorted(found & set(core_features)):
        implied = [g for g in core_features[name] if g in found]
        features.append((name, [*implied, f"yggdryl/{name}"]))
    features.append(("internals", ["yggdryl/internals"]))
    feature_text = "\n".join(f"{name} = {s6a.toml_value(value)}" for name, value in features)
    others = sorted(found - {"http"})
    manifest = (
        "[package]\n"
        f'name = "{PACKAGE}"\n'
        'description = "XML for Analysis 1.1 for yggdryl: the `.xmla` rowset medium over the core\'s handles and Arrow, the SOAP 1.1 envelope it travels in, and the tabular provider that serves a warehouse\'s catalogs over HTTP"\n'
        "version.workspace = true\n"
        "edition.workspace = true\n"
        "rust-version.workspace = true\n"
        "license.workspace = true\n"
        "repository.workspace = true\n"
        'readme = "README.md"\n'
        'keywords = ["xmla", "olap", "soap", "arrow", "xml"]\n'
        'categories = ["encoding", "network-programming", "database"]\n'
        "\n[features]\n"
        + FEATURE_NOTES["http"] + "\n"
        + (f"# {', '.join(f'`{n}`' for n in others)}: the core's gate{'s' if len(others) > 1 else ''} this crate's tests read, forwarded.\n" if others else "")
        + "# `internals` makes `yggdryl_xmla::internals` exist for `tests/` and turns the\n"
        "# core's on.\n"
        + feature_text + "\n"
        "\n[dependencies]\n" + "\n".join(lines) + "\n"
        "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
        "\n# The rowset documents, the requests and the provider's answers; `main`\n"
        "# claims the medium first.\n"
        '[[bench]]\nname = "xmla"\npath = "benchmarks/xmla.rs"\nharness = false\n'
    )
    write(root / CRATE_DIR / "Cargo.toml", manifest)
    readme = root / CRATE_DIR / "README.md"
    if not readme.exists():
        write(
            readme,
            f"# {PACKAGE}\n\nXML for Analysis 1.1 for yggdryl: the `.xmla` rowset medium over the core's "
            "handles and Arrow, the SOAP 1.1 envelope it travels in, and the tabular provider that "
            "serves a warehouse's catalogs over HTTP.\n\n"
            "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the "
            "project's site, and `install()` claims the medium on the core's register.\n",
        )
    ws = read(root / "Cargo.toml")
    m = re.search(r"members = \[([^\]]*)\]", ws)
    members = [s.strip().strip('"') for s in m.group(1).split(",") if s.strip()]
    if CRATE_DIR not in members:
        at = max([i for i, s in enumerate(members) if s == "rust" or s.startswith("rust/")] or [0]) + 1
        members.insert(at, CRATE_DIR)
        ws = ws[:m.start(1)] + ", ".join(f'"{s}"' for s in members) + ws[m.end(1):]
    if f"{PACKAGE} = {{" not in ws:
        if re.search(r'(?m)^yggdryl = \{ path = "rust"', ws):
            tail = list(re.finditer(r'(?m)^yggdryl(?:-[a-z0-9]+)? = \{ path = "rust[^\n]*\n', ws))[-1]
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
    # The bindings link the crate for its claim alone; the CLI serves it.
    for member, line in (
        ("python", f"{PACKAGE}.workspace = true\n"),
        ("node", f"{PACKAGE}.workspace = true\n"),
        ("cli", "# `yggdryl xmla serve`: the XML for Analysis provider, on the core's HTTP\n"
                f'# server.\n{PACKAGE} = {{ workspace = true, features = ["http"] }}\n'),
    ):
        path = root / member / "Cargo.toml"
        text = read(path)
        if PACKAGE in text:
            continue
        m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:(?:#[^\n]*\n)*yggdryl-[a-z0-9]+(?:\.workspace)? = [^\n]*\n)*", text)
        if not m:
            residue(f"{member}/Cargo.toml", "no `yggdryl` dependency line to place the crate after")
            continue
        text = text[: m.end()] + line + text[m.end():]
        write(path, text)
    done("manifests: rust/xmla, the workspace, the core's exclude, the bindings and the CLI")



# ---------------------------------------------------------------------------
# Step 8: tooling
# ---------------------------------------------------------------------------

GENERATOR_OLD = (
    '        + "pub mod internals {\\n"\n'
    '        + ("\\n".join(body) + "\\n" if body else "")\n'
    '        + "}\\n"\n'
)
GENERATOR_NEW = (
    "        # A crate with nothing behind the feature states the empty module on\n"
    "        # one line, as rustfmt writes it.\n"
    '        + ("pub mod internals {\\n" + "\\n".join(body) + "\\n}\\n" if body else "pub mod internals {}\\n")\n'
)


def edit_tooling(root: pathlib.Path) -> None:
    """S4's per-crate generator and inventory reader (S6a's step where S4 has
    not landed), then the empty block a crate with no `internals` module of
    its own gets - this is the first - spelled as rustfmt keeps it."""
    s6a.edit_tooling(root)
    edit(root, "scripts/generate_internals.py", (GENERATOR_OLD, GENERATOR_NEW))
    result = subprocess.run([sys.executable, "-I", str(root / "scripts/generate_internals.py")],
                            capture_output=True, text=True, cwd=root)
    done(f"generate_internals.py, the empty block on one line: {(result.stdout or result.stderr).strip()}")


# ---------------------------------------------------------------------------
# Step 9: CI
# ---------------------------------------------------------------------------


def edit_ci(root: pathlib.Path) -> None:
    rel = ".github/ci/rows.toml"
    if not (root / rel).exists():
        residue(rel, "absent in this tree: list the `xmla` leaf by hand")
        return
    edit(root, rel, ('# xmla = { package = "yggdryl-xmla" }\n', 'xmla = { package = "yggdryl-xmla" }\n'))


# ---------------------------------------------------------------------------
# Step 10: docs, AGENTS.md, READMEs, inventory
# ---------------------------------------------------------------------------

NUMBER_DOWN = {"seven": "six", "six": "five", "five": "four", "four": "three", "three": "two"}


def one_fewer(text: str, pattern: str, where: str) -> str:
    """The count word the pattern's group 1 holds, one fewer: a medium left."""
    found = list(re.finditer(pattern, text))
    if len(found) != 1 or found[0].group(1) not in NUMBER_DOWN:
        residue(where, f"the count sentence matched {len(found)} times, not edited: {pattern[:80]!r}")
        return text
    m = found[0]
    return text[:m.start(1)] + NUMBER_DOWN[m.group(1)] + text[m.end(1):]


def drop_from_brace_lists(text: str, word: str) -> str:
    """`src/{a,word,b}/` without `word`: the folder left the core."""
    def sub(m: re.Match) -> str:
        items = [i for i in m.group(1).split(",") if i != word]
        return "src/{" + ",".join(items) + "}/"
    return re.sub(r"src/\{([a-z0-9_,]+)\}/", lambda m: sub(m) if word in m.group(1).split(",") else m.group(0), text)


def edit_docs(root: pathlib.Path) -> None:
    rel = "docs/media/xmla.md"
    edit(
        root, rel,
        ("| Build | default; the provider's HTTP route needs the `http` feature |",
         "| Build | the `yggdryl-xmla` crate, which every binding and the `yggdryl` command link and install; the provider's HTTP route needs its `http` feature |"),
        ("| Rust | `yggdryl_xmla`: `Xmla<H>` over any handle",
         "| Rust | `yggdryl_xmla`, whose `install()` claims the medium at its reserved rank: `Xmla<H>` over any handle"),
        ("The documents are the ten-thousand-row table of the `media` bench;",
         "The documents are the ten-thousand-row table of `yggdryl-xmla`'s `xmla` bench;"),
        ("cargo bench -p yggdryl --bench media -- media/xmla\n", "cargo bench -p yggdryl-xmla --bench xmla -- media/xmla\n"),
        # The HTTP server the provider is put on stays the core's.
        ("puts the service on the crate's [`http::Server`]", "puts the service on the core's [`http::Server`]"),
        ("the exchange trace - is the crate's [HTTP server]", "the exchange trace - is the core's [HTTP server]"),
    )
    edit(
        root, "cli/src/xmla.rs",
        ("//! `serve` routes the provider in [`yggdryl_xmla`] on the crate's HTTP\n",
         "//! `serve` routes the provider in [`yggdryl_xmla`] on the core's HTTP\n"),
    )
    rel = "docs/media/index.md"
    path = root / rel
    if path.exists():
        text = read(path)
        text = sub_once(
            text,
            r"\| (default|[^|\n]*); the provider's route `http` feature \|",
            "| `yggdryl-xmla`, which every binding and the `yggdryl` command link and install; the provider's route its `http` feature |",
            rel, done_if="| `yggdryl-xmla`, which every binding",
        )
        # The list of the core's own media loses XML for Analysis.
        listed = re.search(r"the core claims its own - ([^\n]*?) - before the register answers anything", text)
        if listed and "XML for Analysis" in listed.group(1):
            media = [m.strip() for m in re.split(r", | and ", listed.group(1)) if m.strip() != "XML for Analysis"]
            spelled = ", ".join(media[:-1]) + " and " + media[-1] if len(media) > 1 else media[0]
            text = text[:listed.start(1)] + spelled + text[listed.end(1):]
        elif not listed:
            residue(rel, "no `the core claims its own - .. -` list to take XML for Analysis out of")
        if "`yggdryl-xmla` claims XML for Analysis" not in text:
            # Beside the clause an earlier media move wrote there, else alone.
            earlier = re.search(r"claims it from its `install\(\)`, as `yggdryl-[a-z0-9]+` claims [^(\n]*?(?= \(\[Registering)", text)
            if earlier:
                text = text[:earlier.end()] + " and `yggdryl-xmla` claims XML for Analysis" + text[earlier.end():]
            else:
                text = sub_once(text, r"claims it from its `install\(\)`", "claims it from its `install()`, as `yggdryl-xmla` claims XML for Analysis", rel)
        if "RESERVED_RANKS" not in text:
            text = replace_once(
                text,
                "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's seven hold - are refused at `$.encoding`.",
                "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's media hold - are refused at `$.encoding`, but for `RESERVED_RANKS`: the ranks the media split off the core keep, `parquet` 1, `avro` 2, `xmla` 4 and `excel` 6, each claimed under its own name and no other, so an options value hashes and sorts as it did when the core held its medium.",
                rel,
            )
            text = replace_once(text, "use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RecordOptions};",
                                "use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RESERVED_RANKS, RecordOptions};", rel)
            text = replace_once(
                text,
                "// The core's ranks and names are never another crate's.\nassert_eq!(EXTERNAL_RANK, 32);\n",
                "// The core's ranks and names are never another crate's; a medium split off\n"
                "// the core keeps its rank under its own name.\nassert_eq!(EXTERNAL_RANK, 32);\n"
                'assert_eq!(RESERVED_RANKS, [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)]);\n',
                rel,
            )
        text = one_fewer(text, r"The core claims its own (\w+) media before the register answers anything", rel)
        if "`yggdryl_xmla::install()` claims XML for Analysis" not in text:
            earlier = re.search(r"(The core claims its own \w+ media before the register answers anything, until they move to the crates that hold them; `yggdryl_[a-z0-9]+::install\(\)` claims [^,;.]+?)(?=, and every binding)", text)
            if earlier:
                text = text[:earlier.end()] + " and `yggdryl_xmla::install()` XML for Analysis" + text[earlier.end():]
            else:
                text = sub_once(
                    text,
                    r"(The core claims its own \w+ media before the register answers anything, until they move to the crates that hold them)",
                    r"\1; `yggdryl_xmla::install()` claims XML for Analysis, and every binding and the `yggdryl` command call it at load",
                    rel,
                )
        write(path, text)
    rel = "docs/contributing.md"
    path = root / rel
    if path.exists():
        text = read(path)
        m = re.search(r"(?m)^\| `rust/src/media_type\.rs`, `mime_type\.rs`, `rust/src/media/`, and one root folder per medium: ([^|\n]*) \|", text)
        if m and "`rust/xmla/`" not in m.group(1):
            cell = m.group(1).replace("`xmla/`, ", "").replace(", `xmla/`", "")
            # Beside the crates an earlier media move named there, else alone.
            cell += (", and `rust/xmla/`, the `yggdryl-xmla` crate with its `soap/`" if "; and `rust/" in cell
                     else "; and `rust/xmla/`, the `yggdryl-xmla` crate, with its `soap/`")
            text = text[:m.start(1)] + cell + text[m.end(1):]
            write(path, text)
        elif not m:
            residue(rel, "no media row naming one root folder per medium to take `xmla/` out of")
    rel = "README.md"
    path = root / rel
    if path.exists():
        text = read(path)
        text = drop_from_brace_lists(text, "xmla")
        whole = re.compile(r"xmla/ also holds the\n {25}XML for Analysis provider and its HTTP server, and\n {25}excel/ the workbook, sheet and cell model")
        alone = re.compile(r"; xmla/ also holds the\n {25}XML for Analysis provider and its HTTP server")
        if whole.search(text):
            text = whole.sub("excel/ also holds the\n" + " " * 25 + "workbook, sheet and cell model", text, count=1)
        elif alone.search(text):
            text = alone.sub("", text, count=1)
        elif "xmla/ also holds" in text:
            residue(rel, "the layout's `xmla/ also holds ..` sentence was not found as written: take it out by hand")
        if "yggdryl-xmla:" not in text:
            bench = re.search(r"(?m)^  benchmarks/ +Criterion targets, grouped by theme\n(?:  [a-z0-9]+/ +yggdryl-[a-z0-9]+:[^\n]*\n(?: {25}[^\n]*\n)*)*", text)
            if bench:
                text = text[:bench.end()] + (
                    "  xmla/                  yggdryl-xmla: the XML for Analysis medium, SOAP and\n"
                    "                         the provider, its own src/, tests/ and benchmarks/\n"
                ) + text[bench.end():]
            else:
                residue(rel, "no `benchmarks/` line to list the crate's folder after")
        write(path, text)
    edit_agents(root)
    done("docs: the XMLA page, the media overview, contributing, the README, AGENTS.md")


def edit_agents(root: pathlib.Path) -> None:
    rel = "AGENTS.md"
    text = read(root / rel)
    pairs = [
        ("| `soap/` | the SOAP 1.1 envelope over the XML codec - `Envelope`, `Fault`, `Fragment`, the streaming `EnvelopeWriter`; protocol vocabulary no medium owns, a root folder of its own name like every implementation, which `xmla/` speaks over, the HTTP it travels on being `http/`'s |",
         "| `soap/` in `yggdryl-xmla` | the SOAP 1.1 envelope over the XML codec - `Envelope`, `Fault`, `Fragment`, the streaming `EnvelopeWriter`; the protocol vocabulary only XML for Analysis speaks here, so it is `yggdryl-xmla`'s (D33) - `rust/xmla/src/soap/`, `yggdryl_xmla::soap` - beside the medium that speaks over it, the HTTP it travels on being the core's `http/` |"),
        ("| `xmla/` | the XML for Analysis 1.1 medium and provider: ",
         "| `xmla/` in `yggdryl-xmla` | the XML for Analysis 1.1 medium and provider, the `yggdryl-xmla` crate at `rust/xmla/` - its `src/` the folder, `lib.rs` its `mod.rs`, `soap/` beside it: "),
        ("and `XMLA_CODEC`, which the core claims under `application/xmla+xml` until `yggdryl-xmla` does,",
         "and `XMLA_CODEC`, which the crate's `install()` claims under `application/xmla+xml` at the reserved rank 4,"),
        ("which `soap/` and `xmla/` read through |", "which `yggdryl-xmla`'s `soap/` and medium read through |"),
        ("the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`)",
         "the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`, `RESERVED_RANKS`)"),
        ("an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK`,",
         "an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK` - a medium the core held at its `RESERVED_RANKS` rank, under its own name -"),
    ]
    for old, new in pairs:
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
        elif new not in text:
            residue(rel, f"anchor matched {text.count(old)} times, not edited: {old[:90]!r}")
    text = one_fewer(
        text,
        r"the core claims its own (\w+) media and its table format before a register answers anything, until they move to the crates that hold them",
        rel,
    )
    if "`RESERVED_RANKS` keeps the rank of each medium split off it" not in text:
        text = sub_once(
            text,
            r"(the core claims its own \w+ media and its table format before a register answers anything, until they move to the crates that hold them) \|",
            r"\1; `RESERVED_RANKS` keeps the rank of each medium split off it, under its own name |",
            rel,
        )
    if "cargo bench -p yggdryl-xmla --bench xmla" not in text:
        bench = re.search(r"(?m)^cargo bench -p yggdryl --bench <[^>\n]*>\n(?:cargo bench -p yggdryl-[a-z0-9]+ --bench [^\n]*\n)*", text)
        if bench:
            text = text[:bench.end()] + "cargo bench -p yggdryl-xmla --bench xmla\n" + text[bench.end():]
        else:
            residue(rel, "no `cargo bench -p yggdryl --bench <...>` line to list the crate's target beside")
    write(root / rel, text)


LIB_SECTION_LINE = "pub fn install() -> yggdryl::Result<()>  (claims XMLA_CODEC under `application/xmla+xml` at rank 4, which yggdryl::media::RESERVED_RANKS keeps for it, once for the process; every binding's init and the CLI's `main` call it; a later call returns at once; the register's refusal where another crate claimed the type or the name first)"
IMPLEMENTER_INVENTORY_HEAD = "  (Media: what the media crates reach - the record doors a wrapper redirects to, the dimensions and the schema it answers through, and the helpers one medium's codec reads; an item added once, by the first move reaching it)"


def edit_inventory(root: pathlib.Path, rewriter, inventory: list[str]) -> None:
    rel = ".api-inventory.txt"
    text = read(root / rel)
    if "### yggdryl_xmla  [" in text and LIB_SECTION_LINE in text:
        return
    # The moved sections: a signature's `crate::` path is the core's from the
    # crate, `yggdryl::`, but for the crate's own `soap`.
    lines = text.split("\n")
    out: list[str] = []
    inside = False
    crate_ctx = s4.Context(XMLA, True)
    for line in lines:
        header = re.match(r"^###\s+(\S+)\s+\[([^\]]+)\]", line)
        if header:
            inside = header.group(2).startswith(f"{CRATE_DIR}/src/")
            out.append(line)
            continue
        out.append(rewriter.inline(line, crate_ctx, rel) if inside else line)
    text = "\n".join(out)
    text = replace_once(
        text,
        "pub static XMLA_CODEC: XmlaCodec  (claimed by the core under `application/xmla+xml`, rank 4, named `xmla`, titled `XMLA`; options held as RecordOptions::Registered)",
        "pub static XMLA_CODEC: XmlaCodec  (claimed by `install()` under `application/xmla+xml` at its reserved rank 4, named `xmla`, titled `XMLA`; options held as RecordOptions::Registered)",
        rel,
    )
    root_header = re.search(r"(?m)^### yggdryl_xmla  \[rust/xmla/src/lib\.rs\][^\n]*\n", text)
    if root_header:
        text = (text[:root_header.start()]
                + "### yggdryl_xmla  [rust/xmla/src/lib.rs]  (the crate root: the medium, the provider and the claim; `soap` the SOAP 1.1 envelope it speaks in; `internals` hidden)\n"
                + LIB_SECTION_LINE + "\n" + text[root_header.end():])
    else:
        residue(rel, "no `### yggdryl_xmla  [rust/xmla/src/lib.rs]` section to list `install()` under")
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
            "Error::InvalidRecord at `$.encoding` for a claim in the core's own name, a codec naming no MIME type, or a rank below EXTERNAL_RANK)",
            "Error::InvalidRecord at `$.encoding` for a claim in the core's own name, a codec naming no MIME type, a reserved rank under another name or a reserved name at another rank (RESERVED_RANKS), or any other rank below EXTERNAL_RANK)",
            rel,
        )
    if inventory:
        start = text.find("### yggdryl::implementer  [")
        end = text.find("\n### ", start + 1) if start >= 0 else -1
        if start < 0 or end < 0:
            residue(rel, "no `yggdryl::implementer` section to list the XMLA additions under")
        else:
            section = text[start:end].rstrip("\n")
            head = "" if IMPLEMENTER_INVENTORY_HEAD in section else IMPLEMENTER_INVENTORY_HEAD + "\n"
            text = text[:start] + section + "\n" + head + "\n".join(inventory) + "\n" + text[end:]
    write(root / rel, text)
    done(".api-inventory.txt: the XMLA and SOAP sections re-homed under yggdryl_xmla, the crate root, the core's additions")


# ---------------------------------------------------------------------------
# Step 11: what the compiler loop still has to see
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
            for m in re.finditer(r"\byggdryl::(?:xmla|soap)\b|\bcrate::xmla\b|yggdryl::internals::(?:xmla|soap)_", text):
                residue(f"{rel}:{line_of(text, m.start())}", f"`{m.group(0)}` is left")
            if rel.startswith(f"{CRATE_DIR}/src/"):
                for m in re.finditer(r"\bcrate::(?:serie::from_canonical_rows|iobase::|iomedia::|xml::write|xml::is_name|xml::shaped|xml::decode)", text):
                    residue(f"{rel}:{line_of(text, m.start())}", f"`{m.group(0)}` reaches a core-private item by its core path")
    # Core tests and benches still reading XMLA through the register.
    for folder in ("rust/tests", "rust/benchmarks"):
        for path in sorted((root / folder).rglob("*.rs")):
            rel = str(path.relative_to(root))
            if rel.startswith(SPLIT_EXCLUDED):
                continue
            text = read(path)
            hits = [line_of(text, m.start()) for m in s4.STRING_LITERAL.finditer(text) if literal_reads_xmla(m.group(0))]
            hits += [line_of(text, m.start()) for m in re.finditer(rf"\bMimeType::XMLA\b|\b{CRATE}\b", code_only(text))]
            if hits:
                hits = sorted(set(hits))
                residue(rel, f"a core test or bench reads XMLA at {len(hits)} line(s) ({', '.join(map(str, hits))}): move it by hand (D39)")
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


# ---------------------------------------------------------------------------
# The run
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=pathlib.Path)
    parser.add_argument("--no-git-lock", action="store_true", help="the caller holds $S/git.lock already")
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s6_xmla" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    # Step 0: a tree already moved is left as it is.
    if (root / CRATE_DIR / "Cargo.toml").exists():
        print(f"[s6-xmla] {CRATE_DIR}/Cargo.toml exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/xmla/mod.rs").exists() or not (root / "rust/src/soap/mod.rs").exists():
        raise SystemExit("rust/src/xmla/mod.rs or rust/src/soap/mod.rs is missing: this is not the tree S6c moves")
    if git(root, "status", "--porcelain").strip():
        raise SystemExit("the tree has changes; run S6c on a clean tree")
    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)
    import tomllib

    core_manifest = tomllib.loads(read(root / "rust/Cargo.toml"))
    s6a.load_trait_methods(root)
    moves = plan_moves(root)
    paths = PathMap(moves)

    # Step 1 reads the core's files where they stand.
    splits = split_tests(root, moves)
    s6a.edit_media_register(root)
    edit_core_media_bench(root)

    # Step 2: the moves.
    with s4.GitLock(not arguments.no_git_lock):
        for old, new in moves:
            (root / new).parent.mkdir(parents=True, exist_ok=True)
            git(root, "mv", old, new)
    done(f"moves: {len(moves)} files moved by git")

    # Step 3-4.
    build_lib(root)
    edit_crate_sources(root)
    bench_root(root)
    edit_codec(root)
    inventory = edit_implementer(root)
    edit_core_lib_and_manifest(root)
    done("crate: lib.rs, the rowset landing, the forwarders' callers; core: the modules, the seed, the implementer")

    # Step 5.
    rewriter = s4.Rewriter(XmlaTables())
    rewrite_paths(root, rewriter, paths, moves, splits)
    finish_crate_text(root)
    siblings = install_in_sibling_tests(root)
    if siblings:
        done(f"sibling leaf tests reaching XMLA install it: {', '.join(siblings)}")

    # Step 6-10.
    install_everywhere(root)
    crate_manifest(root, core_manifest, version)
    edit_tooling(root)
    edit_ci(root)
    edit_docs(root)
    edit_inventory(root, rewriter, inventory)

    # Step 11.
    scan_residue(root)
    with s4.GitLock(not arguments.no_git_lock):
        git(root, "add", "-A", "--", CRATE_DIR)
    report = ["# S6c residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE] + ["", "## Residue", ""] + (RESIDUE or ["- none"])
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s6-xmla] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
