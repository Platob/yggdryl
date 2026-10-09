#!/usr/bin/env python3
"""S6: move the Parquet medium into `yggdryl-parquet` at `rust/parquet/`.

Usage:
    python3 -I s6_parquet_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch after S4 and after the Avro move
(D39's order: avro, parquet, excel, xmla) - and it leaves the tree
uncommitted: `git status` shows the moves as renames and every rewrite as a
modification, for the lane manager's compiler loop (`cargo check --workspace
--all-targets --all-features --keep-going`). It runs no cargo command. A
second run on a tree it already moved stops at step 0 and changes nothing.
It also runs on a tree neither S4 nor the Avro move has reached: what those
add that this move needs too - `RESERVED_RANKS`, the media section of the
core's `implementer`, the per-crate tooling - it adds itself, the same items
under the same names, and each anchor it cannot find in either state is a
residue line rather than a guess.

What it does, in order (`--residue` gets every site it could not rewrite
mechanically, `file:line` each):

 1. moves      - `git mv` by glob: `rust/src/parquet/` to `rust/parquet/src/`
                 (`mod.rs` the crate root `lib.rs`), `rust/tests/parquet{,.rs}`
                 to `rust/parquet/tests/`. There is no Parquet exchange of its
                 own: Parquet is exchanged through the Iceberg exchange.
 2. test split - before the moves, every core test item that needs the
                 medium - a `yggdryl::parquet` path, a name imported from it,
                 a `yggdryl::internals::parquet*` path, a positive
                 `feature = "parquet"` gate (the core could read Parquet only
                 under it) - moves byte-identical into `rust/parquet/tests/`
                 at the same relative path, its helpers copied, a harness
                 written beside it from the core's, and the helpers a moved
                 item reaches in a sibling module of its harness (the object
                 store's fixtures) copied into that module's place (D39);
                 `FakeS3` stays under `rust/tests/support/`, `#[path]`-included.
                 The `parquet` gates of what moved are resolved - the medium
                 is the crate's whatever a feature says.
 3. bench split - the Parquet rows of the core's shared record benchmarks
                 (`media/io/{dimensions,write,pushdown}.rs`, `holder/calls.rs`,
                 `holder/fs/record.rs`, `holder/s3/records.rs`) leave the core
                 for `rust/parquet/benchmarks/parquet/{io,calls,fs,s3}.rs`,
                 built from the core's own items byte-identical plus the glue
                 that registers the Parquet rows alone; the core's `io` group
                 loses its `parquet` gate.
 4. crate      - `rust/parquet/src/lib.rs` (the doc, `#![deny(unsafe_code)]`,
                 `install()` claiming `PARQUET_CODEC` at its reserved rank 1,
                 the hidden `implementer` the Iceberg crate reaches the
                 crate-private reader, footer, encoder and bounds through,
                 its own `internals` holding the generated re-exports), the
                 plan cache and the statistics landing re-spelled through the
                 core's forwarders.
 5. core       - `pub mod parquet`, `DEFAULT_CRS`'s re-export and the seed
                 claim gone; `RESERVED_RANKS` and the claim rule where the
                 Avro move has not added them (D39); `yggdryl::implementer`
                 grown by S3's routes (the shared media section where absent;
                 `projection_indices`, `same_columns`, `land_unproven` and three
                 constants for Parquet); the core's Iceberg re-spelled onto
                 `yggdryl_parquet` (it compiles once Iceberg is its own crate);
                 the `parquet` feature kept, `dep:parquet` alone where Avro has
                 left, because `iceberg` implies it until the Iceberg move.
 6. paths      - every `crate::` path of the moved sources by owner, every
                 `yggdryl::parquet` path and `use` tree in tests, benches,
                 bindings, docs and skills, `yggdryl::internals::parquet*`,
                 file paths in every text file, `#[path]` re-anchored.
 7. siblings   - a leaf crate's test reaching Parquet (Avro's, the market's
                 after S4) stays where it is, installs the crate, gains it as
                 a dev-dependency, its `parquet` gates resolved and the
                 `parquet` feature forward dropped where nothing reads it.
 8. install    - every Parquet test opens with `crate::install::installed()`,
                 the bench `main` installs, the crate's rustdoc examples and a
                 page's Rust block reading Parquet install, the bindings' init
                 and the CLI's `main` (behind its new `parquet` feature) install.
 9. manifests  - `rust/parquet/Cargo.toml` and its README, the workspace
                 members and `[workspace.dependencies]`, the core's, the
                 bindings', the CLI's.
10. tooling    - `generate_internals.py` (S4's per-crate form where absent,
                 and a crate root declaring its own `internals` holding the
                 generated re-exports), `check_api_inventory.py` per crate,
                 the generator run.
11. CI         - the `parquet` leaf line, the Iceberg exchange's cargo lines
                 (the core's `parquet` feature no longer names the medium).
12. docs       - the Parquet page, the media overview, the holder page's
                 commands, the skills, AGENTS.md, the READMEs,
                 `.api-inventory.txt` re-homed.
13. residue    - what is left for the compiler loop, by file and line.
"""

from __future__ import annotations

import argparse
import ast
import collections
import json
import os
import pathlib
import posixpath
import re
import subprocess
import sys

SCRATCH = pathlib.Path(
    os.environ.get(
        "S",
        "/tmp/s",
    )
)
sys.path.insert(0, str(SCRATCH))
import s4_move as s4  # noqa: E402  (S4's lexer, use trees, rewriter, lock)

CORE, PARQUET = "core", "parquet"
PACKAGE = "yggdryl-parquet"
CRATE = "yggdryl_parquet"
CRATE_DIR = "rust/parquet"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-parquet] {what}", flush=True)


# S4's helpers report into this script's residue; its rewriter learns the crate.
s4.residue = residue
s4.CRATE_NAME[PARQUET] = CRATE
s4.PACKAGE[PARQUET] = PACKAGE

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, render_use, UseParseError = s4.parse_use, s4.render_use, s4.UseParseError
line_of = s4.line_of


def no_comments(text: str) -> str:
    """`text` with comments blanked and string literals kept: a `cfg` gate's
    feature name is a literal."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and (text.startswith("//", i) or text.startswith("/*", i)):
            j = s4.skip_comment(text, i)
            out.append(" " * (j - i))
            i = j
            continue
        if c in "\"'brc" and not (i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_")):
            j = s4.skip_string(text, i)
            if j != i:
                out.append(text[i:j])
                i = j
                continue
        out.append(c)
        i += 1
    return "".join(out)


# The methods of every trait a file may import only to call them: the tree's
# own traits, read off their definitions by `load_trait_methods`, and the
# outside ones the moved tests and benches import (the Avro move's table).
TRAIT_METHODS: dict[str, set[str]] = {
    "Read": {"read", "read_exact", "read_to_end", "read_to_string", "take", "bytes", "by_ref"},
    "Write": {"write", "write_all", "flush", "write_fmt", "by_ref"},
    "Seek": {"seek", "rewind", "stream_position"},
    "BufRead": {"read_line", "lines", "fill_buf", "consume", "read_until", "split"},
    "FromStr": {"from_str"},
    "Hash": {"hash"},
    "Hasher": {"finish", "write", "write_u8", "write_u16", "write_u32", "write_u64", "write_usize"},
    "RecordBatchReader": {"schema"},
    "Array": {"len", "is_empty", "is_null", "is_valid", "null_count", "data_type", "as_any", "slice",
              "offset", "to_data", "into_data", "nulls", "logical_nulls", "get_array_memory_size"},
    "AsArray": {"as_primitive", "as_primitive_opt", "as_string", "as_string_opt", "as_string_view",
                "as_binary", "as_binary_opt", "as_binary_view", "as_boolean", "as_boolean_opt",
                "as_struct", "as_struct_opt", "as_list", "as_list_opt", "as_list_view",
                "as_fixed_size_list", "as_fixed_size_binary", "as_map", "as_map_opt",
                "as_dictionary", "as_dictionary_opt", "as_any_dictionary", "as_union", "as_run"},
    "Error": {"source", "description", "cause"},
    "Datelike": {"year", "month", "day", "ordinal", "weekday", "iso_week"},
    "Timelike": {"hour", "minute", "second", "nanosecond"},
    "Borrow": {"borrow"},
}


def load_trait_methods(root: pathlib.Path) -> None:
    """Every trait the tree defines, with the methods it declares."""
    for path in sorted((root / "rust").glob("**/src/**/*.rs")):
        if "/target/" in str(path):
            continue
        code = code_only(read(path))
        for m in re.finditer(r"\btrait\s+([A-Z]\w*)[^;{]*\{", code):
            depth, i = 1, m.end()
            while i < len(code) and depth:
                depth += {"{": 1, "}": -1}.get(code[i], 0)
                i += 1
            TRAIT_METHODS.setdefault(m.group(1), set()).update(re.findall(r"\bfn\s+(\w+)", code[m.end():i]))


def traits_called(code: str) -> set[str]:
    """The traits whose methods `code` calls - a trait imported only to call
    its methods is named nowhere else, so a name-based prune would drop it."""
    called = set(re.findall(r"\.\s*(\w+)\s*(?:::<[^()]*>)?\s*\(", code))
    called |= set(re.findall(r"\b[A-Za-z_]\w*::(\w+)\s*(?:::<[^()]*>)?\s*\(", code))
    if re.search(r"\b(?:write|writeln)!\s*\(", code):
        called.add("write_fmt")
    return {name for name, methods in TRAIT_METHODS.items() if methods & called}


def prune_imports(text: str) -> str:
    """S4's prune, keeping a trait whose methods the file calls."""
    head, items = top_items(text)
    used: set[str] = set()
    code = ""
    for item in items:
        if item.kind != "use":
            item_code = code_only(item.text)
            code += item_code
            used |= set(re.findall(r"[A-Za-z_]\w*", item_code))
    used |= traits_called(code)
    out = [head]
    pos = len(head)
    for item in items:
        out.append(text[pos:item.start])
        pos = item.end
        if item.kind != "use":
            body = inline_body(item)
            if body is None:
                out.append(item.text)
            else:
                start, end = body
                out.append(item.text[:start] + prune_imports(item.text[start:end]) + item.text[end:])
            continue
        tree = s4.use_tree_of(item.text)
        try:
            leaves = parse_use(re.sub(r"\s+", " ", tree or ""))
        except UseParseError:
            out.append(item.text)
            continue
        keep = [leaf for leaf in leaves if leaf[2] == "glob"
                or (leaf[1] == "_" and (not leaf[0] or leaf[0][-1] not in TRAIT_METHODS or leaf[0][-1] in used))
                or (leaf[1] != "_" and (leaf[1] or (leaf[0][-1] if leaf[0] else "")) in used)]
        if len(keep) == len(leaves):
            out.append(item.text)
            continue
        if not keep:
            continue
        m = re.search(r"(?m)^([ \t]*)((?:pub(?:\([^)]*\))?[ \t]+)?)use\b", item.text)
        prefix = item.text[: m.start()] if m else ""
        lead, vis = (m.group(1), m.group(2)) if m else ("", "")
        groups: dict[str, list] = collections.OrderedDict()
        for path, alias, kind in keep:
            leading = "::" if path and path[0] == "" else ""
            segs = path[1:] if leading else path
            groups.setdefault(leading + segs[0], []).append((segs[1:], alias, kind))
        rendered = [f"{lead}{vis}use {r};" for word, items_ in groups.items() for r in render_use(word, items_)]
        out.append(prefix + "\n".join(rendered))
    out.append(text[pos:])
    return "".join(out)


s4.prune_imports = prune_imports


def replace_once(text: str, old: str, new: str, where: str) -> str:
    return s4.replace_once(text, old, new, where)


def edit(root: pathlib.Path, rel: str, *pairs, optional: bool = False) -> bool:
    """Exact-string edits of one file, each anchor asserted to match once.

    An anchor is a string or a tuple of alternatives - the spellings the
    file has before and after a move this one may follow - the first that
    matches once taking the edit; an edit whose result is already there is
    skipped, so a second pass changes nothing."""
    path = root / rel
    if not path.exists():
        if not optional:
            residue(rel, "absent in this tree; its edits were not made")
        return False
    text = read(path)
    for old, new in pairs:
        olds = old if isinstance(old, tuple) else (old,)
        for candidate in olds:
            if text.count(candidate) == 1:
                text = text.replace(candidate, new, 1)
                break
        else:
            if new and new in text:
                continue
            counts = ", ".join(str(text.count(c)) for c in olds)
            residue(rel, f"anchor matched {counts} times, not edited: {olds[0].strip()[:90]!r}")
    write(path, text)
    return True


def dedent(text: str, width: int = 4) -> str:
    prefix = " " * width
    return "\n".join(line[width:] if line.startswith(prefix) else line.lstrip(" ") if not line.strip() else line
                     for line in text.split("\n"))


# ---------------------------------------------------------------------------
# Who owns a path
# ---------------------------------------------------------------------------

# The core's crate-private items the moved sources reach, by the path they
# spell after `crate::`, and the name `yggdryl::implementer` publishes them
# under (S3's routes; D39's list, as the tree stands after S3, P2 and the
# Avro move).
CORE_ROUTES: dict[tuple[str, ...], str] = {
    ("arrow", "arrow_schema_from_field"): "arrow_schema_from_field",
    ("arrow", "field_from_arrow_schema"): "field_from_arrow_schema",
    ("arrow", "projection_indices"): "projection_indices",
    ("arrow", "same_columns"): "same_columns",
    ("cast", "PlanCache"): "PlanCache",
    ("iobase", "append_arrow_reader_default"): "append_arrow_reader_default",
    ("iobase", "merge_arrow_reader_default"): "merge_arrow_reader_default",
    ("iobase", "overwrite_arrow_reader_default_with_field"): "overwrite_arrow_reader_default_with_field",
    ("iomedia", "container_origin"): "container_origin",
    ("iomedia", "container_row_size"): "container_row_size",
    ("iomedia", "dimension_options"): "dimension_options",
    ("iomedia", "held_arrow_field"): "held_arrow_field",
    ("iomedia", "own_options"): "own_options",
    ("media", "cache", "now"): "cache_now",
    ("text", "elide_display"): "elide_display",
    ("text", "expected_got"): "expected_got",
    ("DEFAULT_CRS",): "DEFAULT_CRS",
    ("GEOARROW_WKB_EXTENSION_NAME",): "GEOARROW_WKB_EXTENSION_NAME",
    ("UUID_EXTENSION_NAME",): "UUID_EXTENSION_NAME",
    ("is_variant_storage",): "is_variant_storage",
}

# The Parquet modules the crate root does not publish.
PARQUET_PRIVATE_MODULES = {"geospatial", "metadata"}

# What the Iceberg crate reaches of this crate's crate-private root, through
# the crate's hidden `implementer`.
CRATE_IMPLEMENTER = {
    "read_batch_reader_with", "overwrite_buffered", "load_metadata", "schema_from_metadata",
    "READ_AHEAD_BATCHES", "WHOLE_READ_BYTES", "WRITE_BUFFER_BYTES",
}


class ParquetTables:
    """`s4.Rewriter`'s resolution, for one module leaving the core."""

    def __init__(self, public: set[str], internals: dict[str, list[str]]) -> None:
        self.public = public
        self.internals = internals
        # Inside the crate a Parquet path is the crate's own module path.
        self.inside = False

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        if segs[0] == "parquet":
            rest = segs[1:]
            if not self.inside and rest and rest[0] in CRATE_IMPLEMENTER:
                return PARQUET, ["implementer", *rest]
            if not self.inside and rest and rest[0] not in self.public | PARQUET_PRIVATE_MODULES:
                residue("paths", f"`parquet::{'::'.join(rest)}` names nothing the crate root publishes")
            return PARQUET, rest
        if segs[0] == "internals" and len(segs) > 1 and segs[1] in self.internals:
            return PARQUET, ["internals", *self.internals[segs[1]], *segs[2:]]
        for cut in range(len(segs), 0, -1):
            name = CORE_ROUTES.get(tuple(segs[:cut]))
            if name:
                return CORE, ["implementer", name, *segs[cut:]]
        return CORE, segs


def parquet_public(text: str) -> set[str]:
    """The names the Parquet module publishes at its root."""
    names = set(re.findall(r"(?m)^pub (?:struct|enum|fn|static|const|type|trait|mod) (\w+)", text))
    for m in re.finditer(r"(?m)^pub use \w+::(\{[^;]*\}|\w+);", text):
        names.update(s4.Tables.brace_names(m.group(1)) if m.group(1).startswith("{") else [m.group(1)])
    return names | {"install", "implementer", "internals"}


ALIAS_USE = re.compile(r"(?m)^[ \t]*(?:#[ \t]+)?use yggdryl_parquet as parquet;[ \t]*\n")
PARQUET_ROOTED = re.compile(r"(?<![\w:$])parquet::")


def unalias(text: str) -> str:
    """`use yggdryl_parquet as parquet;` dropped and every `parquet::` path in
    code spelled `yggdryl_parquet::`: the module a caller imported whole is
    its crate - and `parquet::` alone would name the external crate."""
    if not ALIAS_USE.search(text):
        return text
    text = ALIAS_USE.sub("", text)
    code = code_only(text)
    out, pos = [], 0
    for m in PARQUET_ROOTED.finditer(code):
        out.append(text[pos:m.start()])
        out.append("yggdryl_parquet::")
        pos = m.end()
    out.append(text[pos:])
    return "".join(out)


# ---------------------------------------------------------------------------
# The move table and file paths
# ---------------------------------------------------------------------------


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    moves: list[tuple[str, str]] = []
    for f in tracked(root, "rust/src/parquet", "rust/tests/parquet", "rust/tests/parquet.rs"):
        if f.startswith("rust/src/parquet/"):
            rel = f[len("rust/src/parquet/"):]
            new = "rust/parquet/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        else:
            new = "rust/parquet/tests/" + f[len("rust/tests/"):]
        moves.append((f, new))
    return moves


class PathMap:
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/parquet/", "rust/parquet/src/"),
            ("rust/tests/parquet/", "rust/parquet/tests/parquet/"),
        ]

    def __call__(self, path: str) -> str:
        path = posixpath.normpath(path)
        if path in self.files:
            return self.files[path]
        probe = path + "/"
        for old, new in self.dirs:
            if probe == old:
                return new.rstrip("/")
        for old, new in self.dirs:
            if path.startswith(old):
                return new + path[len(old):]
        return path

    def moved(self, path: str) -> bool:
        return self(path) != posixpath.normpath(path)

    def with_files(self, extra: dict[str, str]) -> "PathMap":
        other = PathMap([])
        other.files = {**self.files, **extra}
        other.dirs = self.dirs
        return other


CORE_FOLDERS = {"src", "tests", "benchmarks", "examples", "target"}


def manifest_dir(path: str) -> str | None:
    m = re.match(r"rust/([a-z0-9_]+)/", path)
    if m and m.group(1) not in CORE_FOLDERS:
        return f"rust/{m.group(1)}"
    for prefix in ("rust", "cli", "python", "node"):
        if path.startswith(prefix + "/"):
            return prefix
    return None


s4.manifest_dir = manifest_dir


def rewrite_text_paths(text: str, paths: PathMap, moves: list[tuple[str, str]]) -> str:
    for old, new in sorted(moves, key=lambda pair: -len(pair[0])):
        if old in text:
            text = re.sub(r"(?<![\w./-])" + re.escape(old) + r"(?![\w-])", new, text)
    for old, new in paths.dirs:
        text = re.sub(r"(?<![\w./-])" + re.escape(old), new, text)
    text = re.sub(r"(?<![\w./-])rust/src/parquet(?![\w/.-])", "rust/parquet/src", text)
    text = re.sub(r"(?<![\w./-])rust/tests/parquet(?![\w/.-])", "rust/parquet/tests/parquet", text)
    # A command running the Parquet harness runs the crate's, whose own
    # features are not the core's.
    text = re.sub(r"(?m)(-p )yggdryl( [^\n]*?--test parquet\b)", r"\1yggdryl-parquet\2", text)
    text = re.sub(r'(?m)^(.*?)--features "[^"\n]*" (.*-p yggdryl-parquet\b.*)$', r"\1\2", text)
    text = re.sub(r'(?m)^(.*-p yggdryl-parquet\b.*?) --features "[^"\n]*"', r"\1", text)
    return text


# ---------------------------------------------------------------------------
# The `parquet` gates: what the medium's crate resolves, what the core keeps
# ---------------------------------------------------------------------------

GATE = re.compile(r'feature\s*=\s*"parquet"')
NEGATED_GATE = re.compile(r'not\(\s*feature\s*=\s*"parquet"\s*\)')


def positive_gate(text: str) -> bool:
    """A `feature = "parquet"` gate in code that is not `not(..)`: what the
    core could only do with the medium compiled in."""
    nc = no_comments(text)
    return any(not re.search(r"not\(\s*$", nc[max(0, m.start() - 8):m.start()]) for m in GATE.finditer(nc))


def statement_end(text: str, start: int) -> int:
    """The end of the statement or item starting at `start`: its `;` at depth
    zero, or the close of the block it opens, with the rest of that line."""
    code = code_only(text)
    i, n = start, len(code)
    while i < n:
        c = code[i]
        if c in "([":
            close = s4.matching_brace(code, i)
            if close < 0:
                return -1
            i = close + 1
            continue
        if c == "{":
            close = s4.matching_brace(code, i)
            if close < 0:
                return -1
            i = close + 1
            rest = re.match(r"[ \t]*;", code[i:])
            if rest:
                i += rest.end()
            break
        if c == ";":
            i += 1
            break
        i += 1
    nl = text.find("\n", i)
    return len(text) if nl < 0 else nl + 1


def resolve_gates(text: str, where: str, medium: bool) -> str:
    """The `parquet` gates of `text`, resolved: with `medium`, the medium is
    there (the crate's own tests, a sibling that installs it) - a positive
    gate holds, a negative one does not; without it, the core's - the medium
    is never there, whatever its features say."""
    out = text
    # `#[cfg(all(.., feature = "parquet", ..))]`: the other terms remain.
    def all_list(m: re.Match) -> str:
        inner = m.group(2)
        terms = [t.strip() for t in re.split(r",(?![^()]*\))", inner) if t.strip()]
        rest = [t for t in terms if not GATE.fullmatch(t)]
        if len(rest) == len(terms):
            return m.group(0)
        if not medium:
            return m.group(1) + 'cfg(any())]'
        if len(rest) == 1:
            return m.group(1) + f"cfg({rest[0]})]"
        return m.group(1) + f"cfg(all({', '.join(rest)}))]"

    out = re.sub(r'(#\[)cfg\(all\(((?:[^()]|\([^()]*\))*)\)\)\]', all_list, out)
    lines = out.split("\n")
    result: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        if stripped == '#[cfg(feature = "parquet")]':
            if medium:
                i += 1
                continue
            # The core: the gated statement or item never compiles, nor do the
            # comments and attributes that state it.
            while result and re.match(r"[ \t]*(?:///|#\[)", result[-1]):
                result.pop()
            rest = "\n".join(lines[i + 1:])
            end = statement_end(rest, 0)
            skip = rest[:end].count("\n") if end > 0 else 0
            i += 1 + skip
            continue
        if stripped == '#[cfg(not(feature = "parquet"))]':
            if not medium:
                i += 1
                continue
            rest = "\n".join(lines[i + 1:])
            end = statement_end(rest, 0)
            skip = rest[:end].count("\n") if end > 0 else 0
            i += 1 + skip
            continue
        m = re.fullmatch(r'([ \t]*)#\[cfg_attr\(not\(feature = "parquet"\), (.*)\)\]', line)
        if m:
            if not medium:
                result.append(f"{m.group(1)}#[{m.group(2)}]")
            i += 1
            continue
        result.append(line)
        i += 1
    out = "\n".join(result)
    # `if cfg!(feature = "parquet") { .. }`, and the skip of a leg without it.
    while True:
        m = re.search(r'(?m)^([ \t]*)if cfg!\(feature = "parquet"\) \{\n', out)
        if not m:
            break
        open_at = m.end() - 2
        close = s4.matching_brace(code_only(out), open_at)
        if close < 0 or re.match(r"\s*else\b", out[close + 1:]):
            residue(where, "an `if cfg!(feature = \"parquet\")` with no plain block, left")
            break
        body = out[m.end():close]
        end = out.find("\n", close)
        end = len(out) if end < 0 else end + 1
        out = out[:m.start()] + (dedent(body.rstrip(" \t")) if medium else "") + out[end:]
    out = re.sub(
        r'(?m)^[ \t]*// Parquet is a non-default feature; skip its leg when it is not in\.\n'
        r'([ \t]*)if [^\n{]*&& !cfg!\(feature = "parquet"\) \{\n[ \t]*continue;\n[ \t]*\}\n',
        "", out)
    out = re.sub(r'(?m)^[ \t]*if [^\n{]*&& !cfg!\(feature = "parquet"\) \{\n[ \t]*continue;\n[ \t]*\}\n', "", out)
    # A boolean a test compares against the gate: the medium is there or not.
    out = re.sub(
        r'assert_eq!\(\n([ \t]*)([^\n]*),\n[ \t]*cfg!\(feature = "parquet"\),\n',
        (lambda m: f"assert!(\n{m.group(1)}{m.group(2)},\n") if medium
        else (lambda m: f"assert!(\n{m.group(1)}!{m.group(2)},\n"),
        out)
    if medium:
        out = out.replace("(feature `parquet`)", "(`yggdryl-parquet`, installed)")
    if 'cfg!(feature = "parquet")' in out or GATE.search(no_comments(out)):
        for m in GATE.finditer(no_comments(out)):
            residue(f"{where}:{line_of(out, m.start())}", "a `parquet` gate is left: resolve it by hand")
    return out


# ---------------------------------------------------------------------------
# Step 2: the core's tests that reach Parquet, item by item (S4's split)
# ---------------------------------------------------------------------------

# What stays with Iceberg, the table format that writes Parquet data files:
# it moves with `yggdryl-iceberg`, which installs the medium.
SPLIT_EXCLUDED = (
    "rust/tests/iceberg/", "rust/tests/iceberg.rs", "rust/tests/interop/iceberg.rs",
    "rust/tests/s3tables", "rust/tests/parquet/", "rust/tests/parquet.rs",
    "rust/tests/support/", "rust/tests/allocations.rs", "rust/tests/medallion_ledger.rs",
    # Avro's snappy blocks rode the core's `parquet` feature for `snap` (D16):
    # the Avro move resolves those gates, never this one.
    "rust/tests/avro/", "rust/tests/avro.rs",
)
PARQUET_PATH = re.compile(r"\byggdryl(?:_parquet\b|::parquet\b|::internals::parquet(?:_\w+)?\b)")


def file_imports(items: list) -> set[str]:
    """Names a level imports from `yggdryl::parquet`."""
    names: set[str] = set()
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            if (leaf[:2] == ["yggdryl", "parquet"] or leaf[:1] == ["yggdryl_parquet"]) and kind == "self":
                names.add(alias or leaf[-1])
    return names


def reaches(item, imports: set[str]) -> bool:
    code = code_only(item.text)
    if PARQUET_PATH.search(code):
        return True
    if set(re.findall(r"[A-Za-z_]\w*", code)) & imports:
        return True
    return positive_gate(item.text)


def header_gated(item) -> bool:
    """An inline module whose own attributes gate it on the medium."""
    body = inline_body(item)
    return body is not None and positive_gate(item.text[:body[0]])


def macro_names(items: list) -> None:
    """A top-level macro invocation defines what its arguments name and no
    other item does - `test_options!(TestOptions, TEST_CODEC)` is where
    `TestOptions` comes from - so it travels with the items that name it
    (the Avro move's rule)."""
    defined = {name for item in items if item.kind != "use" for name in item.names}
    for item in items:
        if item.kind.startswith("macro:") and item.kind != "macro:macro_rules" and not item.names:
            m = re.search(r"!\s*[(\[{](.*)[)\]}]", code_only(item.text), re.S)
            if m:
                item.names |= set(re.findall(r"\b([A-Z]\w*)\b", m.group(1))) - defined


def tidy_head(text: str) -> str:
    """One blank line between a file's own documentation and its first item,
    however many a removed `use` left behind (the Avro move's rule)."""
    return re.sub(r"\A((?:[ \t]*//![^\n]*\n)+)\n{2,}", r"\1\n", text, count=1)


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
        owner[id(item)] = PARQUET if reaches(item, imports) else CORE
    changed = True
    while changed:
        changed = False
        for item in body:
            if owner[id(item)] == CORE and any(owner[other] == PARQUET for other in refs[id(item)]
                                               if inline_body(next(i for i in body if id(i) == other)) is None):
                owner[id(item)] = PARQUET
                changed = True
    return owner, refs


def published(text: str) -> bool:
    for line in text.split("\n"):
        stripped = line.strip()
        if not stripped or stripped.startswith(("//", "#[", "#![")):
            continue
        return stripped.startswith("pub ") or stripped.startswith("pub(")
    return False


def split_level(text: str, top: bool, single: bool, inherited: set[str]) -> tuple[str, str | None, int]:
    """S4's `split_level` with Parquet the one crate above the core; an inline
    module gated on the medium moves whole."""
    head, items = top_items(text)
    imports = inherited | file_imports(items)
    owner, refs = item_owners(items, inherited)
    moved = 0
    dest: dict[int, str] = {}
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    for item in items:
        if item.kind == "use" or owner[id(item)] != PARQUET:
            continue
        body = inline_body(item)
        if body is not None and not header_gated(item):
            start, end = body
            inner_home, inner_dest, n = split_level(item.text[start:end], False, False, imports)
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
        moved += 1 if body is None else len(re.findall(r"#\[test\]", item.text))
    if not dest:
        return text, None, 0
    body_items = [item for item in items if item.kind != "use"]
    always = {id(item) for item in body_items if top and "#[global_allocator]" in item.text}
    containers = {id(item) for item in body_items if inline_body(item) is not None and not header_gated(item)}

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
    parquet_text = "".join(out).rstrip("\n") + "\n"
    roots = {
        id(item) for item in body_items
        if id(item) not in dropped
        and (re.search(r"#\[test\]|#\[global_allocator\]", item.text) or inline_body(item) is not None
             or published(item.text))
    }
    # A moved item never comes back by a name a remaining one shares with it.
    stay = (closure(roots) - dropped) | set(replaced)
    out = [head]
    for item in items:
        if item.kind == "use":
            out.append(item.text)
        elif id(item) in replaced:
            out.append(replaced[id(item)])
        elif id(item) in stay:
            out.append(item.text)
    return "".join(out).rstrip("\n") + "\n", parquet_text, moved


def pinned_source(rel: str) -> str | None:
    """The core source file a test file under `rust/tests/` pins."""
    parts = rel.split("/")
    if len(parts) == 1:
        return None
    if parts[0] == "root":
        return "rust/src/" + parts[1]
    if parts[-1] == "mod_.rs":
        parts[-1] = "mod.rs"
    return "rust/src/" + "/".join(parts)


def harness_module_files(text: str, harness_rel: str) -> dict[str, tuple[str, object]]:
    """A harness's `#[path]` modules: name to (file, item)."""
    found = {}
    _, items = top_items(text)
    base = posixpath.dirname(harness_rel)
    for item in items:
        if item.kind != "mod" or inline_body(item) is not None:
            continue
        m = re.search(r'#\[path\s*=\s*"([^"]+)"\]', item.text)
        if m and item.name:
            found[item.name] = (posixpath.normpath(posixpath.join(base, m.group(1))), item)
    return found


def split_tests(root: pathlib.Path, moves: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Every core test file's items reaching Parquet, moved byte-identical into
    `rust/parquet/tests/` at the same relative path (S4's split, D39)."""
    move_map = dict(moves)
    written: list[tuple[str, str]] = []
    needs: dict[str, list[str]] = collections.defaultdict(list)
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED):
            continue
        rel = f[len("rust/tests/"):]
        single = "/" not in rel
        home_text, parquet_text, moved = split_level(read(root / f), True, single, set())
        if parquet_text is None:
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
        original = read(root / f)
        write(root / dest_rel, tidy_head(prune_imports(prune_imports(drop_orphans(header + parquet_text, original, set())))))
        written.append((f, dest_rel))
        if not single:
            needs["rust/tests/" + rel.split("/", 1)[0] + ".rs"].append(f)
        write(root / f, tidy_head(prune_imports(prune_imports(drop_orphans(home_text, original, set())))))
        done(f"{f}: {moved} item(s) moved to {dest_rel}")
    # The helpers a moved item reaches in a sibling module of its harness
    # (`crate::mod_::store`), copied into that module's place in the crate.
    for harness, modules in sorted(needs.items()):
        if not (root / harness).exists():
            continue
        declared = harness_module_files(read(root / harness), harness)
        dest_files = {d for _, d in written}
        reached: dict[str, set[str]] = collections.defaultdict(set)
        for module in modules:
            dest = f"{CRATE_DIR}/tests/{module[len('rust/tests/'):]}"
            code = code_only(read(root / dest))
            for m in re.finditer(r"\bcrate::(\w+)::(\{[^;}]*\}|\w+)", code):
                name = m.group(1)
                leaves = s4.Tables.brace_names(m.group(2)) if m.group(2).startswith("{") else [m.group(2)]
                reached[name].update(leaves)
        for name, leaves in sorted(reached.items()):
            if name not in declared:
                continue
            source, _item = declared[name]
            if source.startswith("rust/tests/support/") or not source.startswith("rust/tests/"):
                continue
            dest = f"{CRATE_DIR}/tests/{source[len('rust/tests/'):]}"
            if dest in dest_files:
                continue
            text = read(root / source)
            copied = copy_items({source: text}, sorted(leaves), source)
            head, _ = top_items(text)
            body = (
                f"//! The helpers of `{source}` the `{PACKAGE}` half of its suite\n"
                "//! names, copied byte-identical (S6, D39).\n//!\n" + head
                + "".join(i.text for i in top_items(text)[1] if i.kind == "use")
                + "\n\n" + "\n\n".join(copied) + "\n"
            )
            write(root / dest, tidy_head(prune_imports(prune_imports(body))))
            written.append((source, dest))
            needs[harness].append(source)
            done(f"{source}: the helpers {', '.join(sorted(leaves))} copied to {dest}")
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


def test_fn_bodies(text: str) -> list[int]:
    """The offset just inside each `#[test]` function's opening brace."""
    code = code_only(text)
    found = []
    for m in re.finditer(r"#\[test\]", code):
        fn = re.compile(r"\bfn\s+\w+").search(code, m.end())
        if not fn:
            continue
        paren = code.find("(", fn.end())
        close = s4.matching_brace(code, paren) if paren >= 0 else -1
        brace = code.find("{", close) if close >= 0 else -1
        if brace >= 0:
            found.append(brace + 1)
    return found


def sibling_leaves(root: pathlib.Path) -> list[pathlib.Path]:
    return [m for m in sorted((root / "rust").glob("*/Cargo.toml"))
            if m.parent.name not in CORE_FOLDERS and m.parent.name != PARQUET]


def install_in_sibling_tests(root: pathlib.Path) -> tuple[list[str], list[str]]:
    """A sibling leaf's test reaching Parquet stays where it is, the crate a
    dev-dependency of its own and installed by the test (D39: the lowest crate
    that names everything it uses; neither sibling is below the other)."""
    touched: list[str] = []
    linking: list[str] = []
    for manifest in sibling_leaves(root):
        leaf = manifest.parent
        changed = False
        files = [p for folder in ("tests", "benchmarks") if (leaf / folder).exists()
                 for p in sorted((leaf / folder).rglob("*.rs"))]
        for path in files:
            rel = str(path.relative_to(root))
            if "/support/" in rel or "/target/" in rel:
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
                    if item.kind != "use" and owner[id(item)] == PARQUET:
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
            new = resolve_gates(new, rel, medium=True)
            if new != text:
                write(path, new)
                touched.append(rel)
                changed = True
        manifest_text = read(manifest)
        reads_crate = any(re.search(rf"\b{CRATE}\b", read(p)) for p in files)
        if changed or reads_crate:
            linking.append(leaf.name)
        if (changed or reads_crate) and PACKAGE not in manifest_text:
            if "[dev-dependencies]\n" in manifest_text:
                manifest_text = manifest_text.replace(
                    "[dev-dependencies]\n", f"[dev-dependencies]\n{PACKAGE}.workspace = true\n", 1
                )
            else:
                manifest_text = manifest_text.rstrip("\n") + f"\n\n[dev-dependencies]\n{PACKAGE}.workspace = true\n"
        manifest_text = drop_feature_forward(root, leaf, manifest_text)
        write(manifest, manifest_text)
    return touched, linking


def drop_feature_forward(root: pathlib.Path, leaf: pathlib.Path, manifest_text: str) -> str:
    """A leaf's `parquet` feature forward, gone where nothing of the leaf reads
    it: the medium is a crate now, never a feature of the core."""
    m = re.search(r'(?m)^parquet = \[[^\]]*\]\n', manifest_text)
    if not m:
        return manifest_text
    reads = any(GATE.search(no_comments(read(p))) for p in leaf.rglob("*.rs") if "/target/" not in str(p))
    if reads:
        return manifest_text
    name = leaf.name
    # Another leaf's forward of this one's `parquet` must go with it.
    for other in sibling_leaves(root):
        if other.parent == leaf:
            continue
        if f'"yggdryl-{name}/parquet"' in read(other):
            residue(str(other.relative_to(root)), f"forwards `yggdryl-{name}/parquet`, which no longer exists: drop it")
    text = manifest_text[:m.start()] + manifest_text[m.end():]
    text = re.sub(r'(?m)^(\w+ = \[)"parquet", ', r"\1", text)
    text = re.sub(r'(?m)^(\w+ = \[[^\]\n]*), "parquet"\]', r"\1]", text)
    text = re.sub(r', "yggdryl-[a-z]+/parquet"', "", text)
    text = re.sub(r'"yggdryl-[a-z]+/parquet", ', "", text)
    return text


# ---------------------------------------------------------------------------
# Step 3: the Parquet rows of the core's shared record benchmarks
# ---------------------------------------------------------------------------


def item_named(text: str, name: str, where: str):
    _, items = top_items(text)
    found = [item for item in items if name in item.names and item.kind != "use"]
    if len(found) != 1:
        residue(where, f"{len(found)} items named `{name}`, expected one")
        return None
    return found[0]


def merged_uses(texts: list[str]) -> str:
    """Every top-level `use` of `texts` as one deduplicated set, `use
    super::..` left out: the module copies what it named."""
    groups: dict[tuple, list] = collections.OrderedDict()
    seen: set[tuple] = set()
    for text in texts:
        _, items = top_items(text)
        for item in items:
            if item.kind != "use":
                continue
            try:
                leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
            except UseParseError:
                continue
            for leaf, alias, kind in leaves:
                if not leaf or leaf[0] in ("super", "crate", "self"):
                    continue
                key = (tuple(leaf), alias, kind)
                if key in seen:
                    continue
                seen.add(key)
                if kind == "glob":
                    parent, term = tuple(leaf), "*"
                else:
                    parent, term = tuple(leaf[:-1]), leaf[-1] + (f" as {alias}" if alias else "")
                groups.setdefault(parent, []).append(term)
    lines = []
    for parent, terms in groups.items():
        terms = sorted(set(terms), key=lambda term: (term != "self", term))
        path = "::".join(parent)
        if not path:
            lines += [f"use {term};" for term in terms]
        elif len(terms) == 1:
            lines.append(f"use {path}::{terms[0]};")
        else:
            lines.append(f"use {path}::{{{', '.join(terms)}}};")
    std = sorted(line for line in lines if re.match(r"use (?:std|core|alloc)::", line))
    rest = sorted(line for line in lines if line not in std)
    return "\n".join(std) + ("\n\n" if std and rest else "") + "\n".join(rest) + "\n"


def copy_items(sources: dict[str, str], names: list[str], where: str, inner: bool = False) -> list[str]:
    """The named items of `sources`, byte-identical, with every item of the
    same sources they name, in the order the sources hold them."""
    pool = []
    for rel, text in sources.items():
        for item in top_items(text)[1]:
            if item.kind not in ("use", "mod") and inline_body(item) is None:
                pool.append(item)
    by_name: dict[str, list] = collections.defaultdict(list)
    for item in pool:
        for name in item.names:
            by_name[name].append(item)
    chosen: list = []
    stack = list(names)
    seen: set[str] = set()
    while stack:
        name = stack.pop()
        if name in seen:
            continue
        seen.add(name)
        candidates = by_name.get(name, [])
        if not candidates:
            if name in names:
                residue(where, f"no item named `{name}` to copy")
            continue
        for item in candidates:
            if item not in chosen:
                chosen.append(item)
                stack.extend(re.findall(r"[A-Za-z_]\w*", code_only(item.text)))
    return [item.text.strip("\n") for item in pool if item in chosen]


def all_items(text: str, base: int = 0) -> list:
    """Every non-`use` item of `text`, inline modules' included, with its span
    in `text`."""
    found = []
    _, items = top_items(text)
    for item in items:
        if item.kind == "use":
            continue
        body = inline_body(item)
        if body is not None:
            start, end = body
            for inner, at, stop in all_items(item.text[start:end], base + item.start + start):
                found.append((inner, at, stop))
            continue
        found.append((item, base + item.start, base + item.end))
    return found


def referenced(code: str, item) -> bool:
    """Whether `code` names `item` the way a use of it reads: a function
    called, reached by a path, listed in a `use` or handed over as a value -
    never a variable that happens to share its name."""
    for name in item.names:
        n = re.escape(name)
        if item.kind != "fn":
            if re.search(rf"\b{n}\b", code):
                return True
            continue
        if (re.search(rf"(?<![\w.]){n}\s*(?:::<[^()]*>)?\s*\(", code)
                or re.search(rf"::\s*{n}\b", code)
                or re.search(rf"[(\[,:=]\s*{n}\s*[,)\];}}]", code)
                or re.search(rf"\buse\b[^;]*\b{n}\b[^;]*;", code)):
            return True
    return False


def drop_orphans(text: str, old: str | None, keep: set[str], bench: bool = False) -> str:
    """The helpers `text` no longer names, dropped until none is left - only
    those `old` (the file they came from) named, so nothing a use the reading
    cannot see is touched: a closure over names reads a variable as the item
    it shadows, and an unused helper is a warning the lanes refuse."""
    old_code = code_only(old) if old is not None else None
    while True:
        code = code_only(text)
        drop = []
        for item, start, end in all_items(text):
            if (item.kind not in ("fn", "const", "static") or not item.names or item.names & keep
                    or re.search(r"#\[test\]|#\[global_allocator\]", item.text)
                    or (published(item.text) and old is not None and not bench)):
                continue
            rest = code[:start] + " " * (end - start) + code[end:]
            if referenced(rest, item):
                continue
            if old_code is not None and not referenced(old_code, item):
                continue
            drop.append((start, end))
        if not drop:
            return text
        for start, end in sorted(drop, reverse=True):
            text = text[:start] + text[end:]


def take_statement(text: str, anchor: str, where: str) -> tuple[str, str]:
    """The one statement starting at the line `anchor` opens, taken out."""
    at = text.find(anchor)
    if at < 0 or text.count(anchor) != 1:
        residue(where, f"anchor matched {text.count(anchor)} times, statement not taken: {anchor.strip()[:80]!r}")
        return "", text
    end = statement_end(text, at)
    return text[at:end], text[:at] + text[end:]


IO_DOC = """//! The Parquet rows of the core's shared record benchmarks, measured as the
//! core measures its own media: `io_dimensions` (the footer statistics, a
//! projected WKB scan and the metadata counters against the decode they
//! avoid), `io_write_stateful` (the three write intents), `io_write_shape` (a
//! cast and a selection, and a wide nested row) and `io_pushdown` (a declared
//! subset against a whole read: column chunks are separately addressable, so
//! a masked read never decompresses what it skipped). The fixtures and the
//! helpers are the core's `rust/benchmarks/media/io/` items, copied
//! byte-identical (S6, D39).

"""

CALLS_DOC = """//! What the Parquet record surfaces cost, in calls to storage and in time:
//! the `calls/records/parquet` rows the core's `rust/benchmarks/holder/calls.rs`
//! measures for the media it keeps, over the same in-memory handle and the
//! same helpers, copied byte-identical (S6, D39). The assertions live in
//! `rust/parquet/tests/iobase_calls.rs`.

"""

FS_DOC = """//! Parquet round trips over a foreign filesystem, against the same encoding
//! over the native in-memory handle: the `fs_record/*/parquet` rows the
//! core's `rust/benchmarks/holder/fs/record.rs` measures for Arrow IPC, over
//! the same filesystem and the same helpers, copied byte-identical (S6, D39).

"""

S3_DOC = """//! Parquet reads and writes through the object-store backend on one
//! in-process store: the `object_records/*/parquet` rows the core's
//! `rust/benchmarks/holder/s3/records.rs` measures for Arrow IPC, over the
//! same store and the same helpers, copied byte-identical (S6, D39).

"""


def carve_benches(root: pathlib.Path) -> list[str]:
    """The Parquet rows leave the core's shared benches for four modules of
    the crate's own bench target."""
    written = []
    io = "rust/benchmarks/media/io"
    sources = {rel: read(root / rel) for rel in (f"{io}/mod.rs", f"{io}/dimensions.rs", f"{io}/write.rs", f"{io}/pushdown.rs")
               if (root / rel).exists()}
    if len(sources) == 4:
        dims = sources[f"{io}/dimensions.rs"]
        write_text = sources[f"{io}/write.rs"]
        push = sources[f"{io}/pushdown.rs"]
        statistics, dims = take_statement(dims, "    parquet_statistics_cases(\n", f"{io}/dimensions.rs")
        cases, dims = take_statement(dims, '    media_cases(\n        &mut group,\n        "parquet",\n', f"{io}/dimensions.rs")
        stateful_rows, write_text = take_statement(write_text, '    stateful_triplet(\n        &mut stateful,\n        "parquet",\n', f"{io}/write.rs")
        copied = copy_items(
            {f"{io}/mod.rs": sources[f"{io}/mod.rs"], f"{io}/dimensions.rs": sources[f"{io}/dimensions.rs"],
             f"{io}/write.rs": sources[f"{io}/write.rs"], f"{io}/pushdown.rs": sources[f"{io}/pushdown.rs"]},
            ["ROWS", "batch", "handle", "reader", "stored_with", "stored", "narrow", "wide", "materialized",
             "decoded_rows", "media_cases", "parquet_statistics_cases", "geospatial_fixture", "STATEFUL_ROWS",
             "stateful_triplet", "parquet_target", "cast_source", "cast_field", "nested_wide", "SHAPE_ROWS",
             "WIDE_COLUMNS"],
            f"{io}/*.rs",
        )
        shape = item_named(sources[f"{io}/write.rs"], "shape_benchmarks", f"{io}/write.rs")
        projection = item_named(push, "projection_benchmarks", f"{io}/pushdown.rs")
        glue = []
        list_re = re.compile(r"    for \(label, name\) in \[\n(?:        \([^\n]*\),\n)+    \] \{\n")
        if shape is not None:
            shape_text, n = list_re.subn('    for (label, name) in [("parquet", "shape.parquet")] {\n', shape.text.strip("\n"), count=1)
            if n != 1:
                residue(f"{io}/write.rs shape_benchmarks", "its encoding list was not found")
            glue.append(re.sub(r"(?m)^fn shape_benchmarks\(", "pub(crate) fn shape_benchmarks(", shape_text, count=1))
        if projection is not None:
            projection_text, n = list_re.subn('    for (label, name) in [("parquet", "bench.parquet")] {\n', projection.text.strip("\n"), count=1)
            if n != 1:
                residue(f"{io}/pushdown.rs projection_benchmarks", "its encoding list was not found")
            projection_text = re.sub(r'(?m)^[ \t]*if label == "parquet" && !cfg!\(feature = "parquet"\) \{\n[ \t]*continue;\n[ \t]*\}\n', "", projection_text)
            glue.append(projection_text)
        if statistics and cases and stateful_rows:
            glue.append(
                "/// The Parquet rows of `io_dimensions`.\n"
                "pub(crate) fn dimension_benchmarks(criterion: &mut Criterion) {\n"
                "    let source = batch();\n"
                '    let parquet = stored_with("bench-dimensions.parquet", &source);\n'
                "    let geospatial = geospatial_fixture();\n\n"
                '    let mut group = criterion.benchmark_group("io_dimensions");\n'
                "    group.sample_size(10);\n"
                + statistics + cases
                + "    group.finish();\n}"
            )
            glue.append(
                "/// The Parquet rows of `io_write_stateful`.\n"
                "pub(crate) fn stateful_benchmarks(criterion: &mut Criterion) {\n"
                "    let source = batch().slice(0, STATEFUL_ROWS);\n"
                '    let mut stateful = criterion.benchmark_group("io_write_stateful");\n'
                "    stateful.sample_size(10);\n"
                "    stateful.throughput(Throughput::Elements(STATEFUL_ROWS as u64));\n\n"
                + stateful_rows
                + "    stateful.finish();\n}"
            )
            uses = merged_uses(list(sources.values()) + ["use criterion::{Criterion, Throughput};\n"])
            body = IO_DOC + uses + "\n" + "\n\n".join(copied + glue) + "\n"
            body = drop_orphans(body, "\n".join(sources.values()), {
                "dimension_benchmarks", "stateful_benchmarks", "shape_benchmarks", "projection_benchmarks"}, bench=True)
            write(root / CRATE_DIR / "benchmarks/parquet/io.rs", tidy_head(prune_imports(body)))
            written.append(f"{CRATE_DIR}/benchmarks/parquet/io.rs")
            # The core keeps every other medium's rows.
            for line in ('    let parquet = stored_with("bench-dimensions.parquet", &source);\n',
                         "    let geospatial = geospatial_fixture();\n",
                         "use yggdryl::parquet::Parquet;\n"):
                dims = replace_once(dims, line, "", f"{io}/dimensions.rs")
            for name in ("parquet_statistics_cases", "geospatial_fixture"):
                item = item_named(dims, name, f"{io}/dimensions.rs")
                if item is not None:
                    dims = dims[:item.start] + dims[item.end:]
            write(root / f"{io}/dimensions.rs", prune_imports(dims))
            target = item_named(write_text, "parquet_target", f"{io}/write.rs")
            if target is not None:
                write_text = write_text[:target.start] + write_text[target.end:]
            write_text = replace_once(write_text, "use yggdryl::parquet::Parquet;\n", "", f"{io}/write.rs")
            write_text = replace_once(write_text, '        ("parquet", "shape.parquet"),\n', "", f"{io}/write.rs")
            write(root / f"{io}/write.rs", prune_imports(write_text))
            push = replace_once(push, '        ("parquet", "bench.parquet"),\n', "", f"{io}/pushdown.rs")
            push = replace_once(
                push,
                '        if label == "parquet" && !cfg!(feature = "parquet") {\n            continue;\n        }\n',
                "", f"{io}/pushdown.rs")
            push = replace_once(
                push,
                "//! inference from elapsed time. The Parquet pair is the one where the saving is\n",
                "//! inference from elapsed time. The Parquet pair - `yggdryl-parquet`'s own\n"
                "//! `io_pushdown` rows - is the one where the saving is\n",
                f"{io}/pushdown.rs",
            )
            write(root / f"{io}/pushdown.rs", push)
    else:
        residue(io, "the shared record benchmarks are not all here: carve their Parquet rows by hand")
    # The media bench's `io` group compiles with no feature now.
    edit(
        root, "rust/benchmarks/media.rs",
        ('#[cfg(feature = "parquet")]\n#[path = "media/io.rs"]\nmod io;\n', '#[path = "media/io.rs"]\nmod io;\n'),
    )
    media = read(root / "rust/benchmarks/media.rs")
    m = re.search(r'fn io_benchmarks\(_criterion: &mut Criterion\) \{\n    #\[cfg\(feature = "parquet"\)\]\n    \{\n((?:        [^\n]*\n)+)    \}\n\}', media)
    if m:
        calls = m.group(1).replace("(_criterion)", "(criterion)")
        media = media[:m.start()] + "fn io_benchmarks(criterion: &mut Criterion) {\n" + dedent(calls) + "}" + media[m.end():]
        write(root / "rust/benchmarks/media.rs", media)
    elif 'feature = "parquet"' in media:
        residue("rust/benchmarks/media.rs", "`io_benchmarks` keeps a `parquet` gate: ungate it by hand")
    # The record encodings' call benchmark.
    calls_rel = "rust/benchmarks/holder/calls.rs"
    if (root / calls_rel).exists():
        calls = read(root / calls_rel)
        records = item_named(calls, "records", calls_rel)
        measured = item_named(calls, "measured", calls_rel)
        line = '        #[cfg(feature = "parquet")]\n        surfaces(criterion, "parquet", "file:///lake/part.parquet");\n'
        if records is None or measured is None or calls.count(line) != 1:
            residue(calls_rel, "the records call bench was not found: carve its Parquet row by hand")
        else:
            start, end = inline_body(records)
            inner = records.text[start:end]
            _, inner_items = top_items(inner)
            kept = [dedent(item.text.strip("\n")) for item in inner_items
                    if item.kind != "use" and "record_call_benchmarks" not in item.names]
            uses = merged_uses([calls, dedent(inner)])
            glue = (
                "/// The Parquet row of the record encodings' call benchmark.\n"
                "pub(crate) fn record_call_benchmarks(criterion: &mut Criterion) {\n"
                '    surfaces(criterion, "parquet", "file:///lake/part.parquet");\n'
                "}"
            )
            body = CALLS_DOC + uses + "\n" + "\n\n".join([measured.text.strip("\n"), *kept, glue]) + "\n"
            body = drop_orphans(body, calls, {"record_call_benchmarks"}, bench=True)
            write(root / CRATE_DIR / "benchmarks/parquet/calls.rs", tidy_head(prune_imports(body)))
            written.append(f"{CRATE_DIR}/benchmarks/parquet/calls.rs")
            write(root / calls_rel, calls.replace(line, "", 1))
    # The record round trips over a foreign filesystem.
    fs_rel, fs_mod = "rust/benchmarks/holder/fs/record.rs", "rust/benchmarks/holder/fs/mod.rs"
    if (root / fs_rel).exists() and (root / fs_mod).exists():
        record = read(root / fs_rel)
        names = ('    let mut names = vec!["bucket/bench.arrows"];\n'
                 '    if cfg!(feature = "parquet") {\n'
                 '        names.push("bucket/bench.parquet");\n'
                 "    }\n\n"
                 "    for name in names {\n")
        bench = item_named(record, "record_benchmarks", fs_rel)
        if record.count(names) != 1 or bench is None:
            residue(fs_rel, "the record bench's encodings were not found: carve its Parquet leg by hand")
        else:
            fs_text = read(root / fs_mod)
            helpers = copy_items({fs_mod: fs_text}, ["ROWS", "batch", "buffer", "memory", "store", "wide"], fs_mod)
            leg = bench.text.strip("\n").replace(names, '    for name in ["bucket/bench.parquet"] {\n', 1)
            uses = merged_uses([fs_text, record])
            head = re.match(r"(?:[ \t]*//![^\n]*\n|[ \t]*#!\[[^\n]*\]\n|[ \t]*\n)*", fs_text).group(0)
            inner_attrs = "".join(l + "\n" for l in head.split("\n") if l.startswith("#!["))
            body = FS_DOC + inner_attrs + uses + "\n" + "\n\n".join([*helpers, leg]) + "\n"
            body = drop_orphans(body, fs_text + record, {"record_benchmarks"}, bench=True)
            write(root / CRATE_DIR / "benchmarks/parquet/fs.rs", tidy_head(prune_imports(body)))
            written.append(f"{CRATE_DIR}/benchmarks/parquet/fs.rs")
            write(root / fs_rel, record.replace(names, '    for name in ["bucket/bench.arrows"] {\n', 1))
    # The object-store record round trips.
    s3_rel, s3_mod = "rust/benchmarks/holder/s3/records.rs", "rust/benchmarks/holder/s3/mod.rs"
    if (root / s3_rel).exists() and (root / s3_mod).exists():
        records = read(root / s3_rel)
        loop = ('    for name in ["bench/rows.arrows", "bench/rows.parquet"] {\n'
                "        let encoding = name.rsplit('.').next().expect(\"an extension\");\n"
                "        // Parquet is a non-default feature; skip its leg when it is not in.\n"
                '        if encoding == "parquet" && !cfg!(feature = "parquet") {\n'
                "            continue;\n"
                "        }\n")
        bench = item_named(records, "record_benchmarks", s3_rel)
        if records.count(loop) != 1 or bench is None:
            residue(s3_rel, "the object-store record bench's encodings were not found: carve its Parquet leg by hand")
        else:
            s3_text = read(root / s3_mod)
            own = [i.names for i in top_items(records)[1] if i.kind not in ("use", "mod")]
            local = sorted({n for names_ in own for n in names_} - {"record_benchmarks"})
            helpers = copy_items({s3_mod: s3_text, s3_rel: records},
                                 ["store", "options", "location", *local], s3_rel)
            leg = bench.text.strip("\n").replace(
                loop,
                '    for name in ["bench/rows.parquet"] {\n'
                "        let encoding = name.rsplit('.').next().expect(\"an extension\");\n",
                1)
            uses = merged_uses([s3_text, records])
            server = re.search(r'(?m)^#\[path = "([^"]*server\.rs)"\]\n(?:pub(?:\(crate\))? )?mod server;\n', s3_text)
            server_decl = ""
            if server:
                server_decl = '#[path = "../../../tests/support/server.rs"]\nmod server;\n\n'
            body = S3_DOC + server_decl + uses + "\n" + "\n\n".join([*helpers, leg]) + "\n"
            body = body.replace("use super::server::", "use self::server::").replace("super::server::", "server::")
            body = drop_orphans(body, s3_text + records, {"record_benchmarks"}, bench=True)
            write(root / CRATE_DIR / "benchmarks/parquet/s3.rs", tidy_head(prune_imports(body)))
            written.append(f"{CRATE_DIR}/benchmarks/parquet/s3.rs")
            write(root / s3_rel, records.replace(
                loop,
                '    for name in ["bench/rows.arrows"] {\n'
                "        let encoding = name.rsplit('.').next().expect(\"an extension\");\n",
                1))
    edit(root, "rust/Cargo.toml",
         (("# adds no dependency, so this target compiles in a default build; the Parquet\n"
           "# leg of the record group is compiled in only when that feature is, and the\n"
           "# S3 groups - measured against `object_store` on one in-process server -\n"
           "# only with the `s3` feature.\n"),
          "# adds no dependency, so this target compiles in a default build; the Parquet\n"
          "# legs of the record groups are `yggdryl-parquet`'s own `parquet` target, and\n"
          "# the S3 groups - measured against `object_store` on one in-process server -\n"
          "# compile only with the `s3` feature.\n"),
         optional=True)
    return written


PARQUET_BENCH_ROOT = """//! The Parquet medium's benchmarks: the Parquet rows of the core's shared
//! record benchmarks - `io_dimensions`, `io_write_stateful`, `io_write_shape`,
//! `io_pushdown`, `calls/records`, `fs_record` and, with the `s3` feature,
//! `object_records` - measured the same way. `main` claims the medium before
//! any group runs, so a handle whose name declares Parquet reads through the
//! core's register.

#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

#[path = "parquet/calls.rs"]
mod calls;
#[path = "parquet/fs.rs"]
mod fs;
#[path = "parquet/io.rs"]
mod io;
#[cfg(feature = "s3")]
#[path = "parquet/s3.rs"]
mod s3;

use criterion::{Criterion, criterion_group};

/// The object-store group, with the backend compiled in.
fn object_store_benchmarks(_criterion: &mut Criterion) {
    #[cfg(feature = "s3")]
    s3::record_benchmarks(_criterion);
}

criterion_group!(
    parquet,
    io::dimension_benchmarks,
    io::stateful_benchmarks,
    io::shape_benchmarks,
    io::projection_benchmarks,
    calls::record_call_benchmarks,
    fs::record_benchmarks,
    object_store_benchmarks,
);

fn main() {
    // The medium the benchmarks read by name is claimed before any runs.
    yggdryl_parquet::install().expect("yggdryl-parquet installs");
    parquet();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
"""


# ---------------------------------------------------------------------------
# Step 4: the crate's own sources
# ---------------------------------------------------------------------------

INSTALL_FN = '''
/// What this crate claims the Parquet medium as.
const CRATE: &str = "yggdryl-parquet";

/// Claims the Parquet medium on the core's register of media -
/// [`PARQUET_CODEC`] under `application/vnd.apache.parquet`, at rank 1, which
/// [`RESERVED_RANKS`](yggdryl::media::RESERVED_RANKS) keeps for it since the
/// core held it there - once for the life of the process; a later call
/// returns at once. Every binding's init and the `yggdryl` command's `main`
/// call it, and so does a Rust caller before the core reads a handle whose
/// name declares Parquet: until the claim such a handle's options are
/// refused, naming the crate to install.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed
/// `application/vnd.apache.parquet` or the name `parquet` first.
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
    yggdryl::media::codec::claim(&PARQUET_CODEC, CRATE)?;
    let _ = INSTALLED.set(());
    Ok(())
}
'''

PARQUET_IMPLEMENTER = """//! The one door the Iceberg crate reaches this crate's crate-private items
//! through, as `yggdryl::implementer` is the core's: nothing here is API, and
//! an item is listed because `yggdryl-iceberg` reads or writes a data file
//! with it - the footer a scan reads first and the reader it hands that
//! footer to, the encoder whose closed footer and length a manifest records
//! and the statistics it projects from them, and the bounds a scan and a
//! commit size their work by. Each item is an `#[inline]` forwarder for a
//! crate-private item of this crate's root, where raising it would publish
//! it, or a constant restating one; `ParquetOptions`, `FileStatistics` and
//! `ColumnStatistics` are the crate's own API and need no door.

use std::sync::Arc;

use arrow_schema::Schema;
use parquet::file::metadata::ParquetMetaData;
use yggdryl::arrow::{BatchReader, Result};
use yggdryl::{Field, IOBase};

use crate::{FileStatistics, ParquetOptions};

/// `READ_AHEAD_BATCHES`, for the Iceberg crate: the batches one unit decodes
/// ahead of its consumer before it waits - a row group's decoder here, a
/// file's worker in an Iceberg scan.
pub const READ_AHEAD_BATCHES: usize = crate::READ_AHEAD_BATCHES;

/// `WHOLE_READ_BYTES`, for the Iceberg crate: the size up to which a file
/// arrives whole in a read's one request.
pub const WHOLE_READ_BYTES: usize = crate::WHOLE_READ_BYTES;

/// `WRITE_BUFFER_BYTES`, for the Iceberg crate: the Arrow input a write
/// gathers before it feeds the column writers.
pub const WRITE_BUFFER_BYTES: usize = crate::WRITE_BUFFER_BYTES;

/// `read_batch_reader_with`, for the Iceberg crate: a read of the file
/// `handle` holds over `footer`, its decoded footer where the caller already
/// holds it, so no byte of the file's end is read again.
///
/// # Errors
///
/// Returns a read, footer, or decoding failure.
#[inline]
pub fn read_batch_reader_with<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &ParquetOptions,
    footer: Option<Arc<ParquetMetaData>>,
) -> Result<BatchReader> {
    crate::read_batch_reader_with(handle, field, options, footer)
}

/// `load_metadata`, for the Iceberg crate: the decoded footer of the file
/// `handle` holds, read from its end.
///
/// # Errors
///
/// Returns a read failure, a file too short to end in a footer, a file that
/// does not end in the Parquet magic, a footer length reaching before the
/// file's start, or a footer that does not decode.
#[inline]
pub fn load_metadata<H: IOBase + ?Sized>(handle: &H) -> Result<Arc<ParquetMetaData>> {
    crate::load_metadata(handle)
}

/// `schema_from_metadata`, for the Iceberg crate: the embedded Arrow schema
/// of a footer already in hand.
///
/// # Errors
///
/// Returns the refusal of a footer whose schema does not read.
#[inline]
pub fn schema_from_metadata(metadata: Arc<ParquetMetaData>) -> Result<Arc<Schema>> {
    crate::schema_from_metadata(metadata)
}

/// `overwrite_buffered`, for the Iceberg crate: the file `handle` holds
/// replaced as [`crate::overwrite_arrow_reader`] replaces it, the column
/// writers fed every `buffer` Arrow bytes, answering the footer the writer
/// closed the file with and the bytes it wrote - what a manifest states,
/// read back from nothing.
///
/// # Errors
///
/// Returns what [`crate::overwrite_arrow_reader`] returns.
#[inline]
pub fn overwrite_buffered<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &ParquetOptions,
    buffer: usize,
) -> Result<(ParquetMetaData, u64)> {
    crate::overwrite_buffered(handle, batches, options, buffer)
}

/// `FileStatistics::from_metadata`, for the Iceberg crate: a footer - read,
/// or the one a write closed its file with - projected into the statistics
/// a manifest records.
#[inline]
#[must_use]
pub fn file_statistics_from_metadata(metadata: &ParquetMetaData) -> FileStatistics {
    FileStatistics::from_metadata(metadata)
}
"""

GENERATED_START = "// GENERATED by scripts/generate_internals.py - do not edit by hand.\n"
GENERATED_END = "// END GENERATED\n"

LIB_DOC_EDITS = [
    ("//! Apache Parquet data files over [`IOBase`] handles.\n",
     "//! Apache Parquet data files over the yggdryl core's [`IOBase`] handles -\n"
     "//! the `yggdryl-parquet` crate.\n"),
    ("compression, so unlike [`crate::ipc`] this module does **not** apply the\n",
     "compression, so unlike [`crate::ipc`] this crate does **not** apply the\n"),
    ("//! The medium is [`PARQUET_CODEC`], claimed on the record register under\n"
     "//! `application/vnd.apache.parquet`: a Parquet handle is\n",
     "//! The medium is [`PARQUET_CODEC`], claimed on the core's record register\n"
     "//! under `application/vnd.apache.parquet` by [`install`] at the rank the\n"
     "//! core held it at: every binding's init and the `yggdryl` command's `main`\n"
     "//! call it, and a Rust program that links this crate calls it before the\n"
     "//! core reads a handle whose name declares Parquet, which is refused naming\n"
     "//! the crate to install until then. [`Parquet`], [`ParquetOptions`] and the\n"
     "//! free functions below answer without it. A Parquet handle is\n"),
]


def build_lib(root: pathlib.Path) -> None:
    """`mod.rs` as the crate root: its doc rewritten for the crate, its module
    declarations and re-exports as they were, `implementer`, `install()`, its
    own `internals` holding the generated re-exports of the modules below."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    path = root / rel
    text = read(path)
    for old, new in LIB_DOC_EDITS:
        text = replace_once(text, old, new, rel)
    text = replace_once(text, "pub(crate) mod geospatial;\nmod metadata;\n",
                        "mod geospatial;\n#[doc(hidden)]\npub mod implementer;\nmod metadata;\n", rel)
    doc = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if not doc:
        residue(rel, "no crate doc to place the attributes after")
    else:
        text = text[:doc.end()] + "\n#![deny(unsafe_code)]\n" + text[doc.end():]
    # The imports `install()` needs.
    text = replace_once(text, "use std::sync::Arc;\n", "use std::sync::{Arc, Mutex, OnceLock, PoisonError};\n", rel)
    # `install()` before the root's own `internals`, which holds the
    # generated re-exports of the modules below it at its end.
    anchor = '#[cfg(feature = "internals")]\n#[doc(hidden)]\npub mod internals {\n'
    at = text.find(anchor)
    if at < 0 or text.count(anchor) != 1:
        residue(rel, "the root's own `internals` was not found: place `install()` and the generated markers by hand")
        text = text.rstrip("\n") + "\n" + INSTALL_FN
    else:
        close = s4.matching_brace(text, at + len(anchor) - 2)
        body_end = text.rfind("\n", 0, close) + 1
        text = (text[:at] + INSTALL_FN.lstrip("\n") + "\n" + text[at:body_end]
                + "\n    " + GENERATED_START + "    " + GENERATED_END + text[body_end:])
    write(path, text)


PLAN_OLD = """                .get_or_compile(stored.fields(), || {
                    crate::cast::ArrowCastPlan::compile_schema(
                        &stored,
                        &root,
                        options,
                        crate::cast::Deferred::default(),
                    )
                })
                .and_then(|plan| plan.reconcile_batch(batch))"""

PLAN_NEW = """                .get_or_compile(stored.fields(), || {
                    crate::implementer::arrow_cast_plan_compile_schema(&stored, &root, options)
                })
                .and_then(|plan| crate::implementer::arrow_cast_plan_reconcile_batch(plan, batch))"""


def edit_crate_sources(root: pathlib.Path) -> None:
    """Before the rewrite, while the moved sources still spell `crate::` for
    the core: the plan cache's compile and reconcile and the statistics
    landing through the core's forwarders."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    edit(
        root, rel,
        (PLAN_OLD, PLAN_NEW),
        (("let mut plans = crate::cast::PlanCache::new();", "let mut plans = crate::implementer::PlanCache::new();"),
         "let mut plans = crate::implementer::PlanCache::new();"),
        ("crate::serie::land(Arc::clone(&field), values, &crate::serie::Proof::Unproven).ok()",
         "crate::implementer::land_unproven(Arc::clone(&field), values).ok()"),
    )


# ---------------------------------------------------------------------------
# Step 5: the core's seams
# ---------------------------------------------------------------------------

RESERVED = """
/// The ranks the media split off the core keep, each under the one name
/// beside it: `parquet` 1, `avro` 2, `xmla` 4 and `excel` 6.
///
/// An options value hashes and orders by its medium's rank, so a rank is a
/// wire contract that never moves: [`claim`] admits a medium at one of these
/// ranks only under the name it is reserved for, refuses that name at any
/// other rank, and holds every other medium at or above [`EXTERNAL_RANK`].
pub const RESERVED_RANKS: [(&str, u8); 4] =
    [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)];
"""

CLAIM_OLD = """    if codec.rank() < EXTERNAL_RANK {
        return Err(invalid(format_smolstr!(
            "a medium outside the core ranks at or above {EXTERNAL_RANK}, got {} for `{}`",
            codec.rank(),
            codec.name()
        )));
    }
    claim_unseeded(codec, by)"""

CLAIM_NEW = """    // A medium the core held keeps its rank, the identity its options hash
    // and order by, under its own name and no other.
    if let Some(&(name, rank)) = RESERVED_RANKS
        .iter()
        .find(|(name, rank)| *name == codec.name() || *rank == codec.rank())
    {
        if name != codec.name() {
            return Err(invalid(format_smolstr!(
                "rank {rank} is reserved for `{name}`, got `{}`",
                codec.name()
            )));
        }
        if rank != codec.rank() {
            return Err(invalid(format_smolstr!(
                "`{name}` is reserved at rank {rank}, got {}",
                codec.rank()
            )));
        }
    } else if codec.rank() < EXTERNAL_RANK {
        return Err(invalid(format_smolstr!(
            "a medium outside the core ranks at or above {EXTERNAL_RANK}, got {} for `{}`",
            codec.rank(),
            codec.name()
        )));
    }
    claim_unseeded(codec, by)"""


def edit_codec(root: pathlib.Path) -> bool:
    """The seed claim gone; `RESERVED_RANKS` and the claim rule where the Avro
    move has not added them. Answers whether this run added them."""
    rel = "rust/src/media/codec.rs"
    edit(root, rel, ("            #[cfg(feature = \"parquet\")]\n            &crate::parquet::PARQUET_CODEC,\n", ""))
    if "pub const RESERVED_RANKS" in read(root / rel):
        return False
    edit(
        root, rel,
        ("/// The rank a medium outside the core takes at least: the core's seven hold\n"
         "/// the positions below it, in the order their options sort.\n"
         "pub const EXTERNAL_RANK: u8 = 32;\n",
         "/// The rank a medium outside the core takes at least: the positions below\n"
         "/// it are the core's own media's and the [`RESERVED_RANKS`] of the media\n"
         "/// split off it, in the order their options sort.\n"
         "pub const EXTERNAL_RANK: u8 = 32;\n" + RESERVED),
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
        (CLAIM_OLD, CLAIM_NEW),
    )
    edit(root, "rust/src/media/mod.rs",
         ("pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper,",
          "pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper, RESERVED_RANKS,"))
    return True


RESERVED_TEST_STATICS = """/// A medium taking a reserved rank under a name other than the one it is
/// reserved for.
static SQUATTER_CODEC: TestCodec = TestCodec {
    name: "squatter",
    title: "Squatter",
    rank: 2,
    types: late_types,
    defaults: late_defaults,
};

/// A medium taking a reserved name at a rank other than its own.
static MISRANKED_CODEC: TestCodec = TestCodec {
    name: "parquet",
    title: "Parquet",
    rank: EXTERNAL_RANK + 4,
    types: late_types,
    defaults: late_defaults,
};

"""

RESERVED_TEST = """
#[test]
fn a_reserved_rank_is_claimed_under_its_own_name_alone() {
    installed();
    // A medium split off the core keeps its rank (D39): the rank under its own
    // name and no other, the name at that rank and no other.
    assert_eq!(yggdryl::media::RESERVED_RANKS[0], ("parquet", 1));
    let rank = codec::claim(&SQUATTER_CODEC, "another")
        .unwrap_err()
        .to_string();
    assert!(rank.contains("invalid record value at $.encoding"), "{rank}");
    assert!(rank.contains("rank 2 is reserved for `avro`, got `squatter`"), "{rank}");
    let name = codec::claim(&MISRANKED_CODEC, "another")
        .unwrap_err()
        .to_string();
    assert!(name.contains("`parquet` is reserved at rank 1, got 36"), "{name}");
    let names: Vec<&str> = codecs().iter().map(|medium| medium.name()).collect();
    assert!(!names.contains(&"squatter"), "{names:?}");
    if let Ok(held) = codec_for(&late_mime()) {
        assert_eq!(held.name(), "latemedium");
    }
}
"""


def edit_media_register(root: pathlib.Path) -> None:
    """The claim rule's two refusals, pinned in the core over test-only media,
    where the Avro move has not pinned them."""
    rel = "rust/tests/media_register.rs"
    path = root / rel
    if not path.exists():
        residue(rel, "absent: pin the reserved-rank refusals by hand")
        return
    text = read(path)
    if "a_reserved_rank_is_claimed_under_its_own_name_alone" in text:
        return
    text = replace_once(text, "/// A medium naming no MIME type.\nstatic NO_TYPE_CODEC: TestCodec",
                        RESERVED_TEST_STATICS + "/// A medium naming no MIME type.\nstatic NO_TYPE_CODEC: TestCodec", rel)
    write(path, text.rstrip("\n") + "\n" + RESERVED_TEST)


def raise_in(root: pathlib.Path, rel: str, names: list[str]) -> None:
    """`pub(crate) fn <name>` raised to `pub fn` (R): the module holding it is
    one the crate root does not publish."""
    path = root / rel
    text = read(path)
    for name in names:
        pattern = re.compile(rf"(?m)^pub\(crate\) fn {name}\b")
        if re.search(rf"(?m)^pub fn {name}\b", text):
            continue
        if len(pattern.findall(text)) != 1:
            residue(rel, f"`pub(crate) fn {name}` matched {len(pattern.findall(text))} times, not raised")
            continue
        text = pattern.sub(f"pub fn {name}", text)
    write(path, text)


def move_use_names(text: str, statement: re.Pattern, names: list[str], into: str, where: str) -> str:
    """Take `names` out of the one `use` tree `statement` matches and add them to
    the tree `into` is the head of (`pub use transfer::{`)."""
    m = statement.search(text)
    if not m:
        residue(where, f"no `{statement.pattern[:40]}` statement to take {names} out of")
        return text
    leaves = [n.strip() for n in m.group(2).split(",") if n.strip()]
    kept = [n for n in leaves if n not in names]
    if len(kept) + len(names) != len(leaves):
        residue(where, f"the statement does not name all of {names}")
        return text
    rendered = m.group(1) + ("{" + ", ".join(kept) + "}" if len(kept) > 1 else kept[0]) + ";"
    text = text[:m.start()] + rendered + text[m.end():]
    target = re.search(re.escape(into) + r"([^}]*)\};", text)
    if not target:
        residue(where, f"no `{into}` tree to add {names} to")
        return text
    merged = sorted([n.strip() for n in target.group(1).split(",") if n.strip()] + names, key=lambda n: (not n[0].isupper(), n))
    return text[:target.start()] + into + ", ".join(merged) + "};" + text[target.end():]


def move_plan_cache(root: pathlib.Path) -> str:
    """`PlanCache` moves into `implementer.rs` (M): raising it inside the
    published `cast` module would publish it. The core reaches it there."""
    rel = "rust/src/cast.rs"
    text = read(root / rel)
    start = text.find("/// One compiled plan per distinct source layout")
    anchor = text.find("impl<P> Default for PlanCache<P> {")
    if start < 0 or anchor < 0 or text.count("pub(crate) struct PlanCache<P>") != 1:
        residue(rel, "`PlanCache` was not found where S6 moves it from")
        return ""
    end = s4.matching_brace(text, text.find("{", anchor)) + 1
    end = text.find("\n", end) + 1
    moved = text[start:end]
    text = text[:start] + text[end:].lstrip("\n")
    write(root / rel, text)
    moved = moved.replace("pub(crate) struct PlanCache<P>", "pub struct PlanCache<P>", 1)
    moved = moved.replace("    pub(crate) const fn new() -> Self {", "    pub const fn new() -> Self {", 1)
    moved = moved.replace("    pub(crate) fn get_or_compile(", "    pub fn get_or_compile(", 1)
    moved = moved.replace("compile: impl FnOnce() -> Result<P>,\n    ) -> Result<&P> {",
                          "compile: impl FnOnce() -> crate::arrow::Result<P>,\n    ) -> crate::arrow::Result<&P> {", 1)
    if "crate::arrow::Result<&P>" not in moved:
        residue("rust/src/implementer.rs", "`PlanCache::get_or_compile`'s result type was not re-spelled `crate::arrow::Result`")
    users = 0
    for path in sorted((root / "rust/src").rglob("*.rs")):
        src = read(path)
        new = src.replace("crate::cast::PlanCache", "crate::implementer::PlanCache")

        def strip(m: re.Match) -> str:
            indent, names = m.group(1), [n.strip() for n in m.group(2).split(",") if n.strip()]
            if "PlanCache" not in names:
                return m.group(0)
            kept = [n for n in names if n != "PlanCache"]
            head = f"{indent}use crate::cast::" + ("{" + ", ".join(kept) + "}" if len(kept) > 1 else kept[0]) + ";"
            return head + f"\n{indent}use crate::implementer::PlanCache;"

        new = re.sub(r"(?m)^([ \t]*)use crate::cast::\{([^}]*)\};", strip, new)
        if new != src:
            write(path, new)
            users += 1
    done(f"PlanCache moved into implementer.rs, {users} core file(s) re-pointed")
    return moved


# The media section of the core's implementer, where the Avro move has not
# written it: the items every media crate shares and Parquet reaches, under
# the names and with the documents that move gives them.
SHARED_MEDIA_SECTION = """
// ------------------------------------------------------------------------
// Media: what the media crates reach - the record doors a wrapper redirects
// to, the dimensions and the schema it answers through, the cache clock and
// the plan cache it writes with, and the value helpers one medium's codec
// reads.
// ------------------------------------------------------------------------

/// The default record writes a media wrapper redirects to once its options
/// are proven its own, for the media crates: append and merge over a leaf or
/// a folder, and an overwrite answering the field it published.
pub use crate::iobase::{
    append_arrow_reader_default, merge_arrow_reader_default,
    overwrite_arrow_reader_default_with_field,
};

/// What a media wrapper answers its options, its origin, its dimensions and
/// its schema through, for the media crates: the handle's own options where
/// none were given, a container's origin and row count, the options a
/// dimension is read under, and the declared or held root a schema read
/// answers.
pub use crate::iomedia::{
    container_origin, container_row_size, dimension_options, held_arrow_field, own_options,
};

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

/// `ArrowCastPlan::compile_schema` with no holder deferred, for the media
/// crates: the cast from one batch schema to one non-null struct root, which
/// a writer holds per distinct source layout in a [`PlanCache`].
///
/// # Errors
///
/// Returns the plan's refusal where the target is no non-null struct root or
/// a source column cannot reach its target.
#[inline]
pub fn arrow_cast_plan_compile_schema(
    source: &Schema,
    target: &Field,
    options: crate::ArrowCastOptions,
) -> crate::arrow::Result<crate::ArrowCastPlan> {
    crate::ArrowCastPlan::compile_schema(source, target, options, crate::cast::Deferred::default())
}

/// `ArrowCastPlan::reconcile_batch`, for the media crates: one batch of the
/// plan's source layout reconciled to its target root as transport, an
/// exact batch answered as itself.
///
/// # Errors
///
/// Returns the plan's refusal of a value its target cannot hold.
#[inline]
pub fn arrow_cast_plan_reconcile_batch(
    plan: &crate::ArrowCastPlan,
    batch: RecordBatch,
) -> crate::arrow::Result<RecordBatch> {
    plan.reconcile_batch(batch)
}

/// `variant::is_variant_storage`, for the media crates: whether an Arrow
/// datatype is the variant's storage.
#[inline]
#[must_use]
pub fn is_variant_storage(dtype: &arrow_schema::DataType) -> bool {
    crate::is_variant_storage(dtype)
}

"""

PARQUET_CORE_SECTION = """
/// `arrow::projection_indices`, for the Parquet crate: the stored columns a
/// read decodes - a declared root's, else the ones listed - as ascending
/// positions in the stored schema, `None` where the read takes them all.
#[inline]
#[must_use]
pub fn projection_indices(
    field: Option<&Field>,
    columns: Option<&[String]>,
    stored: &Schema,
) -> Option<Vec<usize>> {
    crate::arrow::projection_indices(field, columns, stored)
}

/// `arrow::same_columns`, for the Parquet crate: whether two schemas name the
/// same columns in the same order, whatever their nullability and metadata.
#[inline]
#[must_use]
pub fn same_columns(left: &Schema, right: &Schema) -> bool {
    crate::arrow::same_columns(left, right)
}

/// `serie::land` with nothing taken on trust, for the Parquet crate: one
/// array - a footer's statistics column - landed under `field`, every leaf
/// whose layout is not its datatype's whole contract read once.
///
/// # Errors
///
/// Returns the landing's refusal, naming the row and the path below it.
#[inline]
pub fn land_unproven(field: Arc<Field>, array: ArrayRef) -> crate::arrow::Result<Serie> {
    crate::serie::land(field, array, &crate::serie::Proof::Unproven)
}

/// `geospatial::DEFAULT_CRS`, for the Parquet crate: the coordinate
/// reference system a geospatial column states when it states none.
pub const DEFAULT_CRS: &str = crate::geospatial::DEFAULT_CRS;

/// `geospatial::GEOARROW_WKB_EXTENSION_NAME`, for the Parquet crate: the
/// Arrow extension name a WKB geometry or geography column rides.
pub const GEOARROW_WKB_EXTENSION_NAME: &str = crate::geospatial::GEOARROW_WKB_EXTENSION_NAME;

/// `uuid::UUID_EXTENSION_NAME`, for the Parquet crate: the canonical Arrow
/// extension name a UUID column rides.
pub const UUID_EXTENSION_NAME: &str = crate::uuid::UUID_EXTENSION_NAME;
"""


def edit_implementer(root: pathlib.Path) -> None:
    rel = "rust/src/implementer.rs"
    text = read(root / rel)
    if "pub fn land_unproven(" in text:
        return
    if "pub fn arrow_cast_plan_compile_schema(" not in text:
        # The Avro move has not run: the shared media section, the plan cache
        # moved in, and the raises behind the `pub use`s, as it writes them.
        plan_cache = move_plan_cache(root)
        text = read(root / rel)
        text = replace_once(
            text,
            "//! - a definition moved here, where raising it inside a published module\n"
            "//!   would publish it: [`InstantSequence`] and [`Staged`];\n",
            "//! - a definition moved here, where raising it inside a published module\n"
            "//!   would publish it: [`InstantSequence`], [`Staged`] and [`PlanCache`];\n",
            rel,
        )
        text = text.rstrip("\n") + "\n" + SHARED_MEDIA_SECTION + plan_cache.rstrip("\n") + "\n"
        raise_in(root, "rust/src/iobase/transfer.rs",
                 ["append_arrow_reader_default", "merge_arrow_reader_default", "overwrite_arrow_reader_default_with_field"])
        iobase = read(root / "rust/src/iobase.rs")
        iobase = move_use_names(
            iobase,
            re.compile(r"(pub\(crate\) use transfer::)\{([^}]*)\};"),
            ["append_arrow_reader_default", "merge_arrow_reader_default", "overwrite_arrow_reader_default_with_field"],
            "pub use transfer::{",
            "rust/src/iobase.rs",
        )
        write(root / "rust/src/iobase.rs", iobase)
        raise_in(root, "rust/src/iomedia.rs",
                 ["held_arrow_field", "container_origin", "own_options", "dimension_options", "container_row_size"])
        edit(
            root, "rust/src/iomedia.rs",
            ("/// [`field_under`]. A media wrapper", "/// `field_under`. A media wrapper"),
            ("/// Returns the origin's read failure, [`no_schema`], or a clause that does\n",
             "/// Returns the origin's read failure, `no_schema`, or a clause that does\n"),
            ("/// Returns what [`container_field`] returns.\n", "/// Returns what `container_field` returns.\n"),
        )
    # Parquet's own: what the Parquet crate alone reaches.
    text = text.rstrip("\n") + "\n" + PARQUET_CORE_SECTION
    for old, new in (("use arrow_array::RecordBatch;\n", "use arrow_array::{ArrayRef, RecordBatch};\n"),
                     ("use std::hash::Hasher;\n", "use std::hash::Hasher;\nuse std::sync::Arc;\n")):
        if new not in text:
            text = replace_once(text, old, new, rel)
    write(root / rel, text)
    # What the constants' modules held only for Parquet.
    edit(root, "rust/src/lib.rs",
         ('#[cfg(feature = "parquet")]\npub(crate) use geospatial::DEFAULT_CRS;\n', ""),
         ('#[cfg(feature = "parquet")]\npub mod parquet;\n', ""))


# The core's Iceberg reaches the crate the core cannot depend on: re-spelled
# for `yggdryl-iceberg`, which depends on `yggdryl-parquet` (D39).
ICEBERG_FILES = ("scan.rs", "table.rs", "statistics.rs")


def edit_iceberg(root: pathlib.Path, rewriter, tables: ParquetTables) -> list[str]:
    touched = []
    for folder in ("rust/src/iceberg", "rust/iceberg/src"):
        for name in ICEBERG_FILES:
            rel = f"{folder}/{name}"
            path = root / rel
            if not path.exists():
                continue
            text = read(path)
            if "crate::parquet" not in text and "yggdryl::parquet" not in text:
                continue
            new = text.replace("crate::parquet::FileStatistics::from_metadata(",
                               "yggdryl_parquet::implementer::file_statistics_from_metadata(")
            new = new.replace("crate::parquet::", "yggdryl::parquet::")
            tables.inside = False
            new = rewriter.rs_file(new, s4.Context(None, False), rel)
            # A link to the crate's hidden door is a name, never a link.
            new = re.sub(r"\[`(yggdryl_parquet::implementer::\w+)`\]", r"`\1`", new)
            if "crate::parquet" in new or "yggdryl::parquet" in new:
                residue(rel, "a `parquet` path is left")
            if new != text:
                write(path, new)
                touched.append(rel)
    return touched


def edit_core_manifest(root: pathlib.Path) -> None:
    rel = "rust/Cargo.toml"
    text = read(root / rel)
    m = re.search(r"(?m)^((?:#[^\n]*\n)*)parquet = \[([^\]]*)\]\n", text)
    if not m:
        residue(rel, "no `parquet` feature to re-state")
    else:
        snap = '"dep:snap"' in m.group(2)
        comment = (
            "# The Parquet crate the core's own Iceberg tables read their data files'\n"
            "# footers with, until Iceberg is `yggdryl-iceberg`: `iceberg` implies it and\n"
            "# nothing else reads it. The Parquet medium is `yggdryl-parquet`.\n"
        )
        if snap:
            comment += ("# `snap` rides here because the parquet crate already compiles it, so\n"
                        "# Avro's snappy blocks cost no crate a build was not paying.\n")
        text = text[:m.start()] + comment + f"parquet = [{m.group(2)}]\n" + text[m.end():]
    # Two comments, each in the spelling the Avro move left or the one before it.
    for slot in (
        [("# Shared, reference-counted byte views: the column chunks a Parquet read\n"
          "# fetches, shared by the threads that decode them. Arrow's buffer crate\n"
          "# already compiles it, so naming it costs no crate.\n",
          "# Shared, reference-counted byte views: the frames the HTTP/2 and HTTP/3\n"
          "# clients and server hand on. Arrow's buffer crate already compiles it, so\n"
          "# naming it costs no crate.\n"),
         ("# Shared, reference-counted byte views: the column chunks a Parquet read\n"
          "# fetches and the container an Avro read decodes, each shared by the threads\n"
          "# that decode it. Arrow's buffer crate already compiles it, so naming it costs\n"
          "# no crate.\n",
          "# Shared, reference-counted byte views: the container an Avro read decodes,\n"
          "# shared by the threads that decode it, and the frames the HTTP/2 and HTTP/3\n"
          "# clients and server hand on. Arrow's buffer crate already compiles it, so\n"
          "# naming it costs no crate.\n")],
        [("# Record media: CSV, Excel and XML for Analysis are unconditional;\n"
          "# Parquet-backed I/O and Iceberg groups compile only with their owning\n"
          "# features. Avro's groups are `yggdryl-avro`'s own `avro` target.\n",
          "# Record media: the shared record groups, CSV, Excel and XML for Analysis\n"
          "# are unconditional, and the Iceberg groups compile only with that feature.\n"
          "# Avro's and Parquet's groups are their crates' own targets.\n"),
         ("# Record media: Avro is unconditional; Parquet-backed I/O and Iceberg groups\n"
          "# compile only with their owning features.\n",
          "# Record media: Avro and the shared record groups are unconditional, and the\n"
          "# Iceberg groups compile only with that feature. Parquet's groups are\n"
          "# `yggdryl-parquet`'s own `parquet` target.\n")],
    ):
        for old, new in slot:
            if text.count(old) == 1:
                text = text.replace(old, new, 1)
                break
        else:
            if not any(new in text for _, new in slot):
                residue(rel, f"comment not found: {slot[0][0][:70]!r}")
    if "exclude = [" in text.split("[features]")[0]:
        m = re.search(r'exclude = \[([^\]]*)\]', text)
        if '"/parquet"' not in m.group(1):
            text = text[:m.start(1)] + m.group(1) + ', "/parquet"' + text[m.end(1):]
    else:
        text = replace_once(
            text,
            'categories = ["data-structures", "encoding"]\n',
            'categories = ["data-structures", "encoding"]\n'
            "# The split crates sit inside this folder; each is its own package.\n"
            'exclude = ["/parquet"]\n',
            rel,
        )
    write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 6: paths, everywhere
# ---------------------------------------------------------------------------

TEXT_SUFFIXES = {".rs", ".md", ".py", ".pyi", ".js", ".mjs", ".ts", ".toml", ".yml", ".yaml", ".txt", ".cfg"}


def text_files(root: pathlib.Path) -> list[str]:
    files = []
    untracked = [p for p in git(root, "ls-files", "-z", "--others", "--exclude-standard").split("\0") if p]
    for f in sorted(set(tracked(root)) | set(untracked)):
        if f.startswith((".handoff/", "config/", "docs/assets/")) or "/fixtures/" in f:
            continue
        if pathlib.PurePosixPath(f).suffix in TEXT_SUFFIXES and (root / f).exists():
            files.append(f)
    return files


PARQUET_NAMED = re.compile(r"\byggdryl::(?:parquet\b|internals::parquet|\{[^;]*\bparquet\b)")


def rewrite_paths(root: pathlib.Path, rewriter, tables: ParquetTables, paths: PathMap, moves, splits) -> None:
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new[new] = old
    crate_ctx = s4.Context(PARQUET, True)
    outside = s4.Context(None, False)
    count = 0
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        old = moved_new.get(rel, rel)
        text = read(path)
        local = paths.with_files({o: n for o, n in splits})
        new = s4.reanchor(text, old, rel, local)
        ctx = crate_ctx if rel.startswith(f"{CRATE_DIR}/src/") else outside
        tables.inside = ctx is crate_ctx
        new = unalias(rewriter.rs_file(new, ctx, rel))
        tables.inside = False
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
            if folder != "rust/src":
                new = unalias(rewriter.rs_file(s4.reanchor(text, rel, rel, paths), outside, rel))
            else:
                new = s4.reanchor(text, rel, rel, paths)
            if new != text:
                write(path, new)
                count += 1
    for leaf in sibling_leaves(root):
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
            if not PARQUET_NAMED.search(text):
                continue
            new = markdown_unalias(markdown(rewriter, text, f))
            if new != text:
                write(root / f, new)
                count += 1
    for f in ("AGENTS.md", "README.md", "rust/README.md", ".api-inventory.txt"):
        if (root / f).exists():
            text = read(root / f)
            new = rewriter.inline(text, outside, f)
            if new != text:
                write(root / f, new)
                count += 1
    done(f"paths: {rewriter.stats['use']} use statements and {rewriter.stats['inline']} paths re-owned, {count} files")
    count = 0
    for f in text_files(root):
        text = read(root / f)
        new = rewrite_text_paths(text, paths, moves)
        if new != text:
            write(root / f, new)
            count += 1
    done(f"file paths re-pointed in {count} files")


def markdown(rewriter, text: str, where: str) -> str:
    outside = s4.Context(None, False)
    out: list[str] = []
    pos = 0
    for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
        prose = text[pos:m.start(3)]
        out.append(rewriter.inline(prose, outside, where) if PARQUET_NAMED.search(prose) else prose)
        body = m.group(3)
        if m.group(2).strip().startswith("rust") and PARQUET_NAMED.search(body):
            indent = m.group(1)
            lines = body.split("\n")
            if indent and not all(l.startswith(indent) or not l.strip() for l in lines):
                residue(f"{where}:{line_of(text, m.start(3))}", "a Rust block with a line outside its indent names Parquet: re-spell it by hand")
            elif indent:
                stripped = "\n".join(l[len(indent):] for l in lines)
                new = rewriter.rust(stripped, outside, where)
                body = "\n".join((indent + l) if l else l for l in new.split("\n"))
            else:
                body = rewriter.rust(body, outside, where)
        elif PARQUET_NAMED.search(body):
            body = rewriter.inline(body, outside, where)
        out.append(body)
        pos = m.end(3)
    rest = text[pos:]
    out.append(rewriter.inline(rest, outside, where) if PARQUET_NAMED.search(rest) else rest)
    return "".join(out)


def markdown_unalias(text: str) -> str:
    def block(m: re.Match) -> str:
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or "yggdryl_parquet as parquet" not in body:
            return m.group(0)
        if indent:
            stripped = "\n".join(l[len(indent):] if l.startswith(indent) else l for l in body.split("\n"))
            new = unalias(stripped)
            body = "\n".join((indent + l) if l else l for l in new.split("\n"))
        else:
            body = unalias(body)
        return f"{indent}```{info}\n{body}{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text)


PARQUET_HARNESS_DOC = """//! One test file per file under `rust/parquet/src/`, under `tests/parquet/`.
//!
//! `rust/parquet/tests/` mirrors `rust/parquet/src/`: a source file has
//! exactly one test file at the matching path - `geospatial.rs` is pinned
//! inside `parquet/mod_.rs`, which reaches it through
//! `yggdryl_parquet::internals::geospatial` - and this target is the harness
//! for the crate's own files. A test reaches the crate through
//! `yggdryl_parquet::`; where what it pins is not reachable that way, it
//! reaches `yggdryl_parquet::internals`, which exists only under the
//! `internals` feature, and the file that reaches it is declared behind that
//! feature too.
"""


def finish_crate_text(root: pathlib.Path) -> None:
    """What the rewrite must not read as the core's: the crate's implementer,
    written past it, the harness's sentences about itself, and the gates the
    crate's own code and tests no longer need."""
    write(root / CRATE_DIR / "src/implementer.rs", PARQUET_IMPLEMENTER)
    edit(
        root, f"{CRATE_DIR}/tests/holder/mod_.rs",
        ("        // A name composes to the implementation this build carries. Parquet is an\n"
         "        // opt-in feature, so without it `rows.parquet` names a record encoding\n"
         "        // nothing here can read and the handle is left as the bytes it is.\n",
         "        // A name composes to the implementation this build carries: Parquet's,\n"
         "        // once `yggdryl-parquet` has claimed it, where without the claim\n"
         "        // `rows.parquet` names a record encoding nothing can read and the\n"
         "        // handle is left as the bytes it is.\n"),
        optional=True,
    )
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        text = read(path)
        new = text.replace("`yggdryl::internals`", "`yggdryl_parquet::internals`") if "/tests/parquet" in rel else text
        if rel == f"{CRATE_DIR}/tests/parquet.rs":
            new = re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", PARQUET_HARNESS_DOC, new, count=1)
        new = resolve_gates(new, rel, medium=True)
        if new != text:
            write(path, new)


# ---------------------------------------------------------------------------
# Step 8: install
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = """//! The claim every harness of `yggdryl-parquet` makes before a test reads a
//! name: the core's register answers the Parquet medium only once
//! `yggdryl-parquet` has claimed it (D7), so each test opens with
//! [`installed`].

/// Claims the Parquet medium, once for the process.
pub fn installed() {
    yggdryl_parquet::install().expect("yggdryl-parquet claims its medium");
}
"""

INSTALL_TESTS = """
/// `install()` claims the medium under its own name at the rank the core held
/// it at (D39), once: a second call returns at once, and the claim stands
/// against another crate's.
#[test]
fn install_claims_parquet_at_its_reserved_rank_once() {
    yggdryl_parquet::install().expect("the first install claims the medium");
    yggdryl_parquet::install().expect("a later install returns at once");
    let held = yggdryl::media::codec_for(&yggdryl::MimeType::PARQUET).expect("the medium is claimed");
    assert_eq!((held.name(), held.title(), held.rank()), ("parquet", "Parquet", 1));
    assert!(std::ptr::addr_eq(
        held,
        &yggdryl_parquet::PARQUET_CODEC as &dyn yggdryl::media::MediaCodec
    ));
    let refused = yggdryl::media::codec::claim(&yggdryl_parquet::PARQUET_CODEC, "another")
        .expect_err("a second claim of the medium is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-parquet"), "{refused}");
}
"""

# A Rust block reading Parquet through the core's register: a name or a type
# that declares Parquet beside a door that asks the register.
REGISTER_DOORS = re.compile(
    r"\b(?:read_arrow_reader|read_arrow_field|read_serie|read_records|read_field|record_options|row_size|"
    r"column_size|(?:write|overwrite|append|merge)_(?:arrow\w*|serie|records)|codec_for|codecs|"
    r"for_media_type|for_mime_type|Media::open|into_media|into_declared_media|read_media_statistics|"
    r"read_media_geospatial_statistics|execute|execute_in)\b"
)


def names_parquet(body: str) -> bool:
    if re.search(rf"\b{CRATE}\b", body) or "MimeType::PARQUET" in body:
        return True
    return any(re.search(r"\.(?:parquet|pq)\b", lit.group(0)) for lit in s4.STRING_LITERAL.finditer(body))


def markdown_installs(text: str) -> tuple[str, int]:
    """A Rust block naming the crate, or reading Parquet through the register,
    installs it first - beside an install the block already makes."""
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or f"{CRATE}::install()" in body:
            return m.group(0)
        if not (re.search(rf"\b{CRATE}\b", body) or (names_parquet(body) and REGISTER_DOORS.search(body))):
            return m.group(0)
        lines = body.split("\n")
        main = next((k for k, line in enumerate(lines) if re.match(r"^\s*fn main\(", line)), None)
        others = [k for k, line in enumerate(lines) if re.match(r"^\s*yggdryl_\w+::install\(\)", line)]
        if others:
            k = others[-1]
            lead = re.match(r"[ \t]*", lines[k]).group(0)
            ends = "?;" if lines[k].rstrip().endswith("?;") else f'.expect("{PACKAGE} installs");'
            lines.insert(k + 1, f"{lead}{CRATE}::install(){ends}")
        elif main is not None:
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


def install_everywhere(root: pathlib.Path) -> None:
    base = root / CRATE_DIR
    write(base / "tests/support/install.rs", INSTALL_SUPPORT)
    harnesses = tests = 0
    for path in sorted((base / "tests").glob("*.rs")):
        text = s4.declare_install(read(path))
        write(path, re.sub(r'\n{3,}(#\[path = "support/install\.rs"\])', r"\n\n\1", text, count=1))
        harnesses += 1
    mod_ = base / "tests/parquet/mod_.rs"
    if mod_.exists() and "install_claims_parquet_at_its_reserved_rank_once" not in read(mod_):
        write(mod_, read(mod_).rstrip("\n") + "\n" + INSTALL_TESTS)
    for path in sorted((base / "tests").rglob("*.rs")):
        if "/support/" in str(path):
            continue
        text, n = s4.insert_test_installs(read(path))
        tests += n
        write(path, text)
    write(base / "benchmarks/parquet.rs", PARQUET_BENCH_ROOT)
    docs = 0
    for path in sorted((base / "src").rglob("*.rs")):
        text, n = s4.insert_doc_installs(read(path), PARQUET)
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
    done(f"install: {harnesses} harnesses, {tests} tests, the bench main, {docs} rustdoc examples, "
         f"{pages} page blocks, the bindings and the CLI")


CLI_BLOCK = re.compile(
    r"(?m)^    if let Err\(refusal\) = [^\n]*install\(\)[^\n]* \{\n"
    r"        style::bad\(&refusal\.to_string\(\)\);\n"
    r"        return ExitCode::FAILURE;\n"
    r"    \}\n"
)


def install_at_init(root: pathlib.Path) -> None:
    rel = "python/src/lib.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        others = list(re.finditer(r"(?m)^    yggdryl_\w+::install\(\)\.map_err\(value_error\)\?;\n", text))
        if others:
            at = others[-1].end()
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
        others = list(re.finditer(r'(?m)^    yggdryl_\w+::install\(\)\.expect\("[^"\n]*"\);\n', text))
        if others:
            at = others[-1].end()
            text = text[:at] + f'    {CRATE}::install().expect("{PACKAGE} claims its medium");\n' + text[at:]
        else:
            text = replace_once(
                text, "fn install_logging() {\n    logging::install();\n}",
                "fn install_logging() {\n    logging::install();\n"
                "    // The crates split off the core claim what they register before an\n"
                "    // export can read a name of theirs, in dependency order; a refusal is\n"
                "    // a build linking two claimants, which no caller can repair.\n"
                f'    {CRATE}::install().expect("{PACKAGE} claims its medium");\n}}', rel)
        write(root / rel, text)
    rel = "cli/src/main.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        block = (
            "    // Parquet is linked where a feature asks for it: the default build stays\n"
            "    // the schema-only core.\n"
            '    #[cfg(feature = "parquet")]\n'
            f"    if let Err(refusal) = {CRATE}::install() {{\n"
            "        style::bad(&refusal.to_string());\n"
            "        return ExitCode::FAILURE;\n"
            "    }\n"
        )
        others = list(CLI_BLOCK.finditer(text))
        if others:
            at = others[-1].end()
            text = text[:at] + block + text[at:]
        else:
            text = replace_once(
                text, "    warnings::install();\n",
                "    warnings::install();\n"
                "    // The crates split off the core claim what they register before an\n"
                "    // argument can name a medium of theirs, in dependency order.\n" + block, rel)
        write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 9: manifests
# ---------------------------------------------------------------------------

LIB_IDENTS = {"md5": "md-5", "iceberg_official": "iceberg-official"}


def toml_value(value: object) -> str:
    return s4.toml_value(value)


def dependency_line(name: str, spec: object) -> str:
    """S4's line, with a table that states a version alone written as the
    version: what the core held `optional` is plain in the crate."""
    if isinstance(spec, dict):
        rest = {k: v for k, v in spec.items() if k != "optional"}
        if set(rest) == {"version"}:
            return f"{name} = {toml_value(rest['version'])}"
        return s4.dependency_line(name, rest)
    return s4.dependency_line(name, spec)


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
            lines.append(dependency_line(*by_ident[name]))
    dev_lines = []
    for name in sorted(used_dev - skip):
        if name in by_ident:
            dev_lines.append(dependency_line(*by_ident[name]))
        elif name in dev_by_ident:
            dev_lines.append(dependency_line(*dev_by_ident[name]))
    if not any(l.startswith("criterion") for l in dev_lines):
        dev_lines.append(dependency_line("criterion", dev.get("criterion", "0.7")))
    for sibling in ("yggdryl_avro", "yggdryl_market", "yggdryl_fix"):
        if sibling in used_dev:
            dev_lines.append(f"{sibling.replace('_', '-')}.workspace = true")
    # The core's gates the crate's tests and benches read, forwarded.
    core_features = core_manifest.get("features", {})
    found: set[str] = set()
    for path in (root / CRATE_DIR).rglob("*.rs"):
        found |= set(re.findall(r'feature\s*=\s*"([\w-]+)"', no_comments(read(path))))
    found.discard("internals")
    found.discard("parquet")
    unknown = sorted(found - set(core_features))
    if unknown:
        residue(f"{CRATE_DIR}/Cargo.toml", f"the crate's code reads features the core does not have: {unknown}")
    features = [("default", [])]
    for name in sorted(found & set(core_features)):
        implied = [g for g in core_features[name] if g in found]
        features.append((name, [*implied, f"yggdryl/{name}"]))
    features.append(("internals", ["yggdryl/internals"]))
    feature_text = "\n".join(f"{name} = {toml_value(value)}" for name, value in features)
    manifest = (
        "[package]\n"
        f'name = "{PACKAGE}"\n'
        'description = "Apache Parquet for yggdryl: the Parquet record medium over the core\'s handles and Arrow, its footer statistics and its geospatial and variant columns"\n'
        "version.workspace = true\n"
        "edition.workspace = true\n"
        "rust-version.workspace = true\n"
        "license.workspace = true\n"
        "repository.workspace = true\n"
        'readme = "README.md"\n'
        'keywords = ["parquet", "arrow", "columnar", "geospatial", "statistics"]\n'
        'categories = ["encoding", "parser-implementations"]\n'
        "\n[features]\n"
        "# The core's gates this crate's tests and benchmarks read, forwarded; the\n"
        "# crate's own code reads none. `internals` makes `yggdryl_parquet::internals`\n"
        "# exist for `tests/` and turns the core's on.\n"
        + feature_text + "\n"
        "\n[dependencies]\n" + "\n".join(lines) + "\n"
        "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
        "\n# The Parquet rows of the core's shared record benchmarks; `main` claims\n"
        "# the medium first.\n"
        '[[bench]]\nname = "parquet"\npath = "benchmarks/parquet.rs"\nharness = false\n'
    )
    write(root / CRATE_DIR / "Cargo.toml", manifest)
    readme = root / CRATE_DIR / "README.md"
    if not readme.exists():
        write(
            readme,
            f"# {PACKAGE}\n\nApache Parquet for yggdryl: the Parquet record medium over the core's handles "
            "and Arrow, its footer statistics and its geospatial and variant columns.\n\n"
            "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the "
            "project's site, and `install()` claims the medium on the core's register.\n",
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
    # The bindings link the crate; the CLI behind a feature of its own.
    for member in ("python", "node", "cli"):
        path = root / member / "Cargo.toml"
        text = read(path)
        if PACKAGE in text:
            continue
        m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:yggdryl-[a-z]+(?:\.workspace)? = [^\n]*\n)*", text)
        if not m:
            residue(f"{member}/Cargo.toml", "no `yggdryl` dependency line to place the crate after")
            continue
        if member != "cli":
            text = text[: m.end()] + f"{PACKAGE}.workspace = true\n" + text[m.end():]
        else:
            text = text[: m.end()] + f"{PACKAGE} = {{ workspace = true, optional = true }}\n" + text[m.end():]
            text = cli_features(text)
        write(path, text)
    done("manifests: rust/parquet, the workspace, the core, the bindings and the CLI")


def cli_features(text: str) -> str:
    """The CLI's `parquet` feature - the medium linked and installed - which
    its `iceberg` feature implies, as the core's `iceberg` implied its
    `parquet`; the pages' Rust blocks, compiled here, link it always."""
    rel = "cli/Cargo.toml"
    m = re.search(r'(?m)^((?:#[^\n]*\n)*)iceberg = \[([^\]]*)\]\n', text)
    if not m:
        residue(rel, "no `iceberg` feature to imply `parquet` from")
        return text
    terms = [t.strip() for t in m.group(2).split(",") if t.strip()]
    if '"parquet"' not in terms:
        terms.insert(0, '"parquet"')
    feature = (
        "# The Parquet medium, `yggdryl-parquet`, linked and installed at `main`:\n"
        "# a record leaf named `.parquet` is read and written with it, and refused\n"
        "# naming the crate without it. `iceberg` implies it, since a table's data\n"
        "# files are Parquet.\n"
        'parquet = ["dep:yggdryl-parquet"]\n'
    )
    text = text[:m.start()] + feature + m.group(1) + f"iceberg = [{', '.join(terms)}]\n" + text[m.end():]
    dev = (
        "# The pages' Rust blocks name the Parquet medium whatever the CLI's own\n"
        "# features, so the docs runner and the CLI's tests link it.\n"
        f"{PACKAGE}.workspace = true\n"
    )
    if "[dev-dependencies]\n" in text:
        text = text.replace("[dev-dependencies]\n", "[dev-dependencies]\n" + dev, 1)
    else:
        text = text.replace("\n[features]\n", "\n[dev-dependencies]\n" + dev + "\n[features]\n", 1)
    return text


# ---------------------------------------------------------------------------
# Step 10: tooling
# ---------------------------------------------------------------------------

OWN_INTERNALS_RULE = '''

def own_internals(text: str) -> bool:
    """Whether a crate root declares its own `internals` outside the block this
    script writes - a crate whose root is the implementation, a medium split
    off the core: the block then holds the re-exports alone, written inside
    that module between the two markers, so the root has one `internals`."""
    outside = text
    if START in text:
        start = text.index(START)
        outside = text[:start] + text[text.index(END, start) + len(END):]
    return bool(DECLARATION.search(outside))
'''


def edit_tooling(root: pathlib.Path) -> None:
    """Where S4 has not landed, its per-crate edits of the two inventory and
    internals tools, read off `s4_move.py` itself; then the rule a crate root
    declaring its own `internals` needs."""
    path = root / "scripts/generate_internals.py"
    text = read(path)
    if "def crates()" not in text:
        source = read(SCRATCH / "s4_move.py")
        m = re.search(r"            tail = ('''def crates\(\).*?''')\n", source, re.S)
        start = text.find("def block() -> str:")
        end = text.find('if __name__ == "__main__":')
        if not m or start < 0 or end < 0:
            residue("scripts/generate_internals.py", "S4's per-crate generator was not found: make it per crate by hand")
        else:
            text = text[:start] + ast.literal_eval(m.group(1)) + text[end:]
    if "def own_internals(" not in text:
        text = replace_once(
            text,
            "def block() -> str:\n",
            OWN_INTERNALS_RULE.lstrip("\n") + "\n\ndef block(nested: bool = False) -> str:\n",
            "scripts/generate_internals.py",
        )
        text = replace_once(
            text,
            "    tests = SRC.parent.relative_to(ROOT).as_posix() + \"/tests/\"\n",
            "    if nested:\n"
            "        return START + \"\".join(f\"{line}\\n\" for line in body) + \"    \" + END\n"
            "    tests = SRC.parent.relative_to(ROOT).as_posix() + \"/tests/\"\n",
            "scripts/generate_internals.py",
        )
        text = replace_once(
            text,
            "        text = LIB.read_text(encoding=\"utf-8\")\n        generated = block()\n",
            "        text = LIB.read_text(encoding=\"utf-8\")\n"
            "        nested = own_internals(text)\n"
            "        if nested and START not in text:\n"
            "            print(f\"{LIB.relative_to(ROOT)} declares its own `internals`: put the markers at its end\", file=sys.stderr)\n"
            "            return 1\n"
            "        generated = block(nested)\n",
            "scripts/generate_internals.py",
        )
    write(path, text)
    path = root / "scripts/check_api_inventory.py"
    text = read(path)
    if 'glob("*/src")' not in text:
        text = replace_once(
            text,
            'for path in sorted((ROOT / "rust" / "src").rglob("*.rs")):',
            'for path in sorted(p for src in [ROOT / "rust" / "src", *sorted((ROOT / "rust").glob("*/src"))] for p in src.rglob("*.rs")):',
            "scripts/check_api_inventory.py",
        )
        text = replace_once(
            text,
            '        for path in crate.rglob("*.rs"):\n',
            "        # A leaf's surface holds what the core's macros write in it.\n"
            '        core = ROOT / "rust" / "src"\n'
            '        for path in [p for tree in {crate, core} for p in tree.rglob("*.rs")]:\n',
            "scripts/check_api_inventory.py",
        )
        write(path, text)
    result = subprocess.run([sys.executable, "-I", str(root / "scripts/generate_internals.py")],
                            capture_output=True, text=True, cwd=root)
    done(f"generate_internals.py: {(result.stdout or result.stderr).strip()}")
    if result.returncode:
        residue("scripts/generate_internals.py", f"the generator failed: {result.stderr.strip()[:200]}")
    # The docs runner compiles the pages' Rust blocks where it stands.
    runner = read(root / "scripts/check_docs_examples.py")
    if '"--test", "docs_examples"' in runner and '"-p", "yggdryl-cli"' not in runner:
        residue("scripts/check_docs_examples.py",
                "the Rust pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner to "
                "`cli/tests/` (D13): a block naming `yggdryl_parquet` compiles only there")


# ---------------------------------------------------------------------------
# Step 11: CI
# ---------------------------------------------------------------------------


def edit_ci(root: pathlib.Path, linking: list[str]) -> None:
    rel = ".github/ci/rows.toml"
    if not (root / rel).exists():
        residue(rel, "absent in this tree: list the `parquet` leaf by hand")
    else:
        edit(
            root, rel,
            ('# parquet = { package = "yggdryl-parquet", jobs = ["pyiceberg-interop", "spark-interop"] }\n',
             'parquet = { package = "yggdryl-parquet", jobs = ["pyiceberg-interop", "spark-interop"] }\n'),
        )
        # A leaf whose tests link the crate runs again when Parquet changes.
        for leaf in linking:
            leaf_after_parquet(root, leaf)
    # The core's `parquet` feature no longer names the medium: `iceberg`
    # implies what is left of it.
    edit(
        root, "scripts/check_iceberg_interop.py",
        ('1. ``cargo test --features "parquet iceberg" --test interop iceberg::`` writes a\n',
         "1. ``cargo test --features iceberg --test interop iceberg::`` writes a\n"),
        ('        "--features",\n        "parquet iceberg",\n',
         '        "--features",\n        "iceberg",\n'),
    )
    edit(
        root, ".github/workflows/ci.yml",
        ("    # Google, `parquet iceberg` from `rust/` for PyIceberg - so the jobs that\n",
         "    # Google, `iceberg` from `rust/` for PyIceberg - so the jobs that\n"),
        ('        run: cargo test --locked --features "parquet iceberg" --test interop --no-run\n',
         "        run: cargo test --locked --features iceberg --test interop --no-run\n"),
    )


def leaf_after_parquet(root: pathlib.Path, leaf: str) -> None:
    """A leaf whose tests link the crate names `parquet` among what its
    `[leaves]` line is after (the Avro move's rule)."""
    rel = ".github/ci/rows.toml"
    text = read(root / rel)
    m = re.search(rf"(?m)^{re.escape(leaf)} = \{{ (.*) \}}$", text)
    if not m:
        residue(rel, f"no listed `[leaves]` line for `{leaf}`: say it is after `parquet` by hand")
        return
    body = m.group(1)
    after = re.search(r"after = \[([^\]]*)\]", body)
    if after and '"parquet"' in after.group(1):
        return
    if after:
        listed = after.group(1).strip()
        body = body[:after.start(1)] + (f'{listed}, "parquet"' if listed else '"parquet"') + body[after.end(1):]
    else:
        body = re.sub(r'(package = "[^"]+")', r'\1, after = ["parquet"]', body, count=1)
    write(root / rel, text[:m.start(1)] + body + text[m.end(1):])


# ---------------------------------------------------------------------------
# Step 12: docs, skills, AGENTS.md, READMEs, inventory
# ---------------------------------------------------------------------------

BENCH_COMMANDS = [
    ('cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/parquet/read_rows\n'
     'cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/parquet\n',
     "cargo bench -p yggdryl-parquet --bench parquet -- io_dimensions/parquet/read_rows\n"
     "cargo bench -p yggdryl-parquet --bench parquet -- io_write_stateful/parquet\n"),
    ("cargo bench --features \"parquet iceberg\" -p yggdryl --bench media -- 'io_dimensions/parquet/(row_size|column_size|read_arrow_field)'\n",
     "cargo bench -p yggdryl-parquet --bench parquet -- 'io_dimensions/parquet/(row_size|column_size|read_arrow_field)'\n"),
]


def edit_docs(root: pathlib.Path) -> None:
    edit(
        root, "docs/media/parquet.md",
        ("| Build | the `parquet` feature |",
         "| Build | the `yggdryl-parquet` crate, which every binding links and installs and the `yggdryl` command links with its `parquet` feature |"),
        ("| Rust | `yggdryl_parquet`: `Parquet<H>` over any handle,",
         "| Rust | `yggdryl_parquet`, whose `install()` claims the medium at its reserved rank: `Parquet<H>` over any handle,"),
        *BENCH_COMMANDS,
    )
    edit(
        root, "docs/media/index.md",
        ("| [Parquet](parquet.md) | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature |",
         "| [Parquet](parquet.md) | `application/vnd.apache.parquet`, `.parquet` | `yggdryl-parquet` |"),
    )
    # The overview's register sentences, whichever moves ran before.
    path = root / "docs/media/index.md"
    text = read(path)
    for old, new in (
        ("the core claims its own - Arrow IPC, Parquet, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`, as `yggdryl-avro` claims Avro",
         "the core claims its own - Arrow IPC, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`, as `yggdryl-avro` claims Avro and `yggdryl-parquet` Parquet"),
        ("the core claims its own - Arrow IPC, Parquet, Avro, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`",
         "the core claims its own - Arrow IPC, Avro, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`, as `yggdryl-parquet` claims Parquet"),
        ("The core claims its own six media before the register answers anything, until they move to the crates that hold them; `yggdryl_avro::install()` claims Avro, and every binding and the `yggdryl` command call it at load.",
         "The core claims its own five media before the register answers anything, until they move to the crates that hold them; `yggdryl_avro::install()` claims Avro and `yggdryl_parquet::install()` Parquet, and every binding and the `yggdryl` command call them at load."),
        ("The core claims its own seven media before the register answers anything, until they move to the crates that hold them.",
         "The core claims its own six media before the register answers anything, until they move to the crates that hold them; `yggdryl_parquet::install()` claims Parquet, and every binding and the `yggdryl` command call it at load."),
        ("// The core's media are claimed before the register answers anything, in rank order.\n",
         "// The core's media are claimed before the register answers anything, and the\n// crates that hold the others claim theirs at install, in rank order.\n"),
    ):
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
    if "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's seven hold - are refused at `$.encoding`." in text:
        text = text.replace(
            "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's seven hold - are refused at `$.encoding`.",
            "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's media hold - are refused at `$.encoding`, but for `RESERVED_RANKS`: the ranks the media split off the core keep, `parquet` 1, `avro` 2, `xmla` 4 and `excel` 6, each claimed under its own name and no other, so an options value hashes and sorts as it did when the core held its medium.",
            1)
        text = text.replace("use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RecordOptions};",
                            "use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RESERVED_RANKS, RecordOptions};", 1)
        text = text.replace(
            "// The core's ranks and names are never another crate's.\nassert_eq!(EXTERNAL_RANK, 32);\n",
            "// The core's ranks and names are never another crate's; a medium split off\n"
            "// the core keeps its rank under its own name.\nassert_eq!(EXTERNAL_RANK, 32);\n"
            'assert_eq!(RESERVED_RANKS, [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)]);\n', 1)
    write(path, text)
    edit(
        root, "docs/holder/index.md",
        ("cargo bench --bench holder --features parquet -- fs_record\n",
         "cargo bench --bench holder -- fs_record\ncargo bench -p yggdryl-parquet --bench parquet -- fs_record\n"),
        ("cargo bench -p yggdryl --features parquet --bench holder -- calls/\n",
         "cargo bench -p yggdryl --bench holder -- calls/\ncargo bench -p yggdryl-parquet --bench parquet -- calls/\n"),
        optional=True,
    )
    # A holder or media bench group no longer needs the medium's feature.
    path = root / "docs/holder/index.md"
    if path.exists():
        text = read(path)
        new = re.sub(r"(?m)^cargo bench --bench (media|holder) --features parquet -- ", r"cargo bench --bench \1 -- ", text)
        if new != text:
            write(path, new)
    edit(
        root, "docs/arrow/readers.md",
        ("| Feature flag | `parquet::read_batch_reader` needs the non-default `parquet` feature |",
         "| Feature flag | none: `yggdryl_parquet::read_batch_reader` is the `yggdryl-parquet` crate's |"),
        optional=True,
    )
    edit(
        root, "skills/yggdryl-records/references/formats.md",
        ("| Parquet | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature |",
         "| Parquet | `application/vnd.apache.parquet`, `.parquet` | `yggdryl-parquet`, linked by every binding |"),
        optional=True,
    )
    edit(
        root, "skills/yggdryl-expressions/references/rust.md",
        ("- `Plan::execute` needs the `parquet` feature to read `.parquet` sources and\n  `s3` for object-store URLs.",
         "- `Plan::execute` reads `.parquet` sources once `yggdryl_parquet::install()`\n  has claimed the medium, and object-store URLs with the `s3` feature."),
        optional=True,
    )
    edit(
        root, "skills/yggdryl-records/references/rust.md",
        ("Parquet needs `features = [\"parquet\"]`, Iceberg `features = [\"iceberg\"]` (implies `parquet`).",
         "Parquet is the `yggdryl-parquet` crate - `yggdryl_parquet::install()` claims the medium before a handle names it - and Iceberg `features = [\"iceberg\"]`."),
        optional=True,
    )
    edit(
        root, "skills/yggdryl/references/rust.md",
        ("`yggdryl::local::LocalFile`, `yggdryl_parquet`, `yggdryl::json`).",
         "`yggdryl::local::LocalFile`, `yggdryl::json`); a medium split off the core is its crate's (`yggdryl_parquet`)."),
        optional=True,
    )
    edit(
        root, "skills/yggdryl/references/rust.md",
        ("# Parquet files, Iceberg tables (needs Rust 1.94), object stores:\n"
         "# yggdryl = { version = \"0.1\", features = [\"parquet\", \"iceberg\", \"s3\"] }\n",
         "# Iceberg tables (needs Rust 1.94), object stores:\n"
         "# yggdryl = { version = \"0.1\", features = [\"iceberg\", \"s3\"] }\n"
         "# Parquet files, the medium's own crate - `yggdryl_parquet::install()`\n"
         "# claims it before a handle names it:\n"
         "# yggdryl-parquet = \"0.1\"\n"),
        (("| `parquet` | Parquet reader/writer, Avro snappy blocks | - |",
          "| `parquet` | Parquet reader/writer | - |"),
         "| `parquet` | the Parquet crate the core's Iceberg tables read their data files' footers with, until Iceberg is its own crate; the medium is `yggdryl-parquet` | - |"),
        ("| `iceberg` | Iceberg tables over the crate's own Parquet | `parquet` |",
         "| `iceberg` | Iceberg tables over `yggdryl-parquet`'s Parquet | `parquet` |"),
        optional=True,
    )
    edit(
        root, "docs/index.md",
        ('    # yggdryl = { version = "0.1", features = ["parquet", "iceberg"] }\n',
         '    # yggdryl = { version = "0.1", features = ["iceberg"] }\n    # yggdryl-parquet = "0.1"\n'),
        optional=True,
    )
    readme_edits = (
        ("  src/{ipc,parquet,csv,iceberg,xmla,excel}/\n", "  src/{ipc,csv,iceberg,xmla,excel}/\n"),
        ("  src/{ipc,parquet,avro,csv,iceberg,xmla,excel}/\n", "  src/{ipc,avro,csv,iceberg,xmla,excel}/\n"),
    )
    path = root / "README.md"
    if path.exists():
        text = read(path)
        for old, new in readme_edits:
            if text.count(old) == 1:
                text = text.replace(old, new, 1)
        bench = "  benchmarks/            Criterion targets, grouped by theme\n"
        if "  parquet/               yggdryl-parquet" not in text and text.count(bench) == 1:
            text = text.replace(
                bench,
                bench + "  parquet/               yggdryl-parquet: the Parquet record medium, its own\n"
                        "                         src/, tests/ and benchmarks/\n", 1)
        write(path, text)
    path = root / "rust/README.md"
    if path.exists():
        text = read(path)
        for old, new in (("src/{ipc,parquet}/\n", "src/ipc/\n"), ("src/{ipc,parquet,avro}/\n", "src/{ipc,avro}/\n")):
            if text.count(old) == 1:
                text = text.replace(old, new, 1)
        write(path, text)
    path = root / "docs/contributing.md"
    if path.exists():
        text = read(path)
        for old, new in (
            ("and one root folder per medium: `rust/src/ipc/`, `parquet/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/`; and `rust/avro/`, the `yggdryl-avro` crate |",
             "and one root folder per medium: `rust/src/ipc/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/`; and `rust/avro/` and `rust/parquet/`, the `yggdryl-avro` and `yggdryl-parquet` crates |"),
            ("and one root folder per medium: `rust/src/ipc/`, `parquet/`, `avro/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/` |",
             "and one root folder per medium: `rust/src/ipc/`, `avro/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/`; and `rust/parquet/`, the `yggdryl-parquet` crate |"),
        ):
            if text.count(old) == 1:
                text = text.replace(old, new, 1)
        write(path, text)
    edit_agents(root)
    done("docs: the Parquet page, the media overview, the holder page, the skills, contributing, the READMEs, AGENTS.md")


def edit_agents(root: pathlib.Path) -> None:
    rel = "AGENTS.md"
    text = read(root / rel)
    parquet_row = (
        ", `parquet/` also `ParquetFooter` (the footer the wrapper's cache holds, answered through `IOMedia::as_any`) and `read_media_statistics`/`read_media_geospatial_statistics`, the footer statistics of any media that is one Parquet leaf,",
        ", `parquet/` - in `yggdryl-parquet` at `rust/parquet/`, its `install()` claiming `PARQUET_CODEC` at the reserved rank 1 - also `ParquetFooter` (the footer the wrapper's cache holds, answered through `IOMedia::as_any`) and `read_media_statistics`/`read_media_geospatial_statistics`, the footer statistics of any media that is one Parquet leaf, and the reader, footer, encoder and bounds Iceberg reaches through the crate's own hidden `implementer`,",
    )
    # The record-media rows: whichever spelling the Avro move left.
    for old, new in (
        ("| `ipc/`, `parquet/`, `csv/`; `avro/` in `yggdryl-avro` | one root folder per record medium;",
         "| `ipc/`, `csv/`; `avro/` in `yggdryl-avro`, `parquet/` in `yggdryl-parquet` | one root folder per record medium;"),
        ("| `ipc/`, `parquet/`, `avro/`, `csv/` | one root folder per record medium;",
         "| `ipc/`, `avro/`, `csv/`; `parquet/` in `yggdryl-parquet` | one root folder per record medium;"),
        ("Parquet is feature-gated; Avro is `yggdryl-avro`, its scalar codec and its\nrecord surface over Arrow;",
         "Parquet is `yggdryl-parquet`; Avro is `yggdryl-avro`, its scalar codec and its\nrecord surface over Arrow;"),
        ("Parquet is feature-gated; Avro's scalar codec is unconditional and its record\nsurface uses Arrow;",
         "Parquet is `yggdryl-parquet`; Avro's scalar codec is unconditional and its\nrecord surface uses Arrow;"),
        ("the core claims its own six media and its table format before a register answers anything, until they move to the crates that hold them; `RESERVED_RANKS` keeps the rank of each medium split off it, under its own name |",
         "the core claims its own five media and its table format before a register answers anything, until they move to the crates that hold them; `RESERVED_RANKS` keeps the rank of each medium split off it, under its own name |"),
        ("the core claims its own seven media and its table format before a register answers anything, until they move to the crates that hold them |",
         "the core claims its own six media and its table format before a register answers anything, until they move to the crates that hold them; `RESERVED_RANKS` keeps the rank of each medium split off it, under its own name |"),
        ("the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`)",
         "the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`, `RESERVED_RANKS`)"),
        ("an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK`,",
         "an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK` - a medium the core held at its `RESERVED_RANKS` rank, under its own name -"),
        ("  manifests = `yggdryl-avro`'s Avro, data = core Parquet, and no Iceberg/Avro/catalog",
         "  manifests = `yggdryl-avro`'s Avro, data = `yggdryl-parquet`'s Parquet, and no Iceberg/Avro/catalog"),
        ("  manifests = core Avro, data = core Parquet, and no Iceberg/Avro/catalog",
         "  manifests = core Avro, data = `yggdryl-parquet`'s Parquet, and no Iceberg/Avro/catalog"),
        ("| a gated path works | the loop above plus `--features \"parquet iceberg\"` or `--features s3` |",
         "| a gated path works | the loop above plus `--features iceberg` or `--features s3` |"),
        ("`default-members = [\"rust\"]`; features are `default = []`, `parquet`,\n`iceberg` (implies `parquet`),",
         "`default-members = [\"rust\"]`; features are `default = []`, `parquet` (the\nParquet crate the core's Iceberg reads its data files' footers with, until\nIceberg is its own crate - the medium is `yggdryl-parquet`), `iceberg`\n(implies `parquet`),"),
    ):
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
    if text.count(parquet_row[0]) == 1:
        text = text.replace(parquet_row[0], parquet_row[1], 1)
    elif "`parquet/` - in `yggdryl-parquet`" not in text:
        residue(rel, "the Parquet sentence of the media folders' row was not found")
    bench = re.search(r"(?m)^cargo bench -p yggdryl --bench <[^>\n]*>\n(?:cargo bench -p yggdryl-[a-z]+ --bench [a-z]+\n)*", text)
    if bench and "cargo bench -p yggdryl-parquet --bench parquet" not in text:
        text = text[:bench.end()] + "cargo bench -p yggdryl-parquet --bench parquet\n" + text[bench.end():]
    elif not bench:
        residue(rel, "no `cargo bench -p yggdryl --bench <...>` line to list the crate's target beside")
    write(root / rel, text)


PARQUET_LIB_SECTION = """### yggdryl_parquet  [rust/parquet/src/lib.rs]  (the crate root: the codec, the record medium, its free doors and the claim; `implementer` and `internals` hidden)
pub fn install() -> yggdryl::Result<()>  (claims PARQUET_CODEC under `application/vnd.apache.parquet` at rank 1, which yggdryl::media::RESERVED_RANKS keeps for it, once for the process; every binding's init and the CLI's `main` (its `parquet` feature) call it; a later call returns at once; the register's refusal where another crate claimed the type or the name first)
"""

PARQUET_IMPLEMENTER_INVENTORY = """### yggdryl_parquet::implementer  [rust/parquet/src/implementer.rs]  (the door yggdryl-iceberg reaches the crate's crate-private items through, as yggdryl::implementer is the core's; `#[doc(hidden)]`, no API: the reader over a held footer, the footer read, the encoder whose closed footer and length a manifest records, its statistics, and three bounds)
pub const READ_AHEAD_BATCHES: usize  (the batches one unit decodes ahead of its consumer)
pub const WHOLE_READ_BYTES: usize  (the size up to which a file arrives whole in one request)
pub const WRITE_BUFFER_BYTES: usize  (the Arrow input a write gathers before it feeds the column writers)
pub fn read_batch_reader_with<H: IOBase + ?Sized>(handle: &H, field: Option<&Field>, options: &ParquetOptions, footer: Option<Arc<ParquetMetaData>>) -> Result<BatchReader>  (a read over a footer already in hand)
pub fn load_metadata<H: IOBase + ?Sized>(handle: &H) -> Result<Arc<ParquetMetaData>>  (the decoded footer, read from the file's end)
pub fn schema_from_metadata(metadata: Arc<ParquetMetaData>) -> Result<Arc<Schema>>  (the embedded Arrow schema of a footer in hand)
pub fn overwrite_buffered<H: IOBase + ?Sized>(handle: &mut H, batches: BatchReader, options: &ParquetOptions, buffer: usize) -> Result<(ParquetMetaData, u64)>  (the overwrite answering the footer it closed the file with and its length)
pub fn file_statistics_from_metadata(metadata: &ParquetMetaData) -> FileStatistics  (`FileStatistics::from_metadata`)

"""

CORE_IMPLEMENTER_INVENTORY = """pub fn projection_indices(field: Option<&Field>, columns: Option<&[String]>, stored: &Schema) -> Option<Vec<usize>>  (`arrow::projection_indices`, for the Parquet crate: the stored columns a read decodes)
pub fn same_columns(left: &Schema, right: &Schema) -> bool  (`arrow::same_columns`, for the Parquet crate)
pub fn land_unproven(field: Arc<Field>, array: ArrayRef) -> crate::arrow::Result<Serie>  (`serie::land` with nothing taken on trust, for the Parquet crate: a footer's statistics column landed under the field a filter reads)
pub const DEFAULT_CRS: &str  (`geospatial::DEFAULT_CRS`, for the Parquet crate)
pub const GEOARROW_WKB_EXTENSION_NAME: &str  (`geospatial::GEOARROW_WKB_EXTENSION_NAME`, for the Parquet crate)
pub const UUID_EXTENSION_NAME: &str  (`uuid::UUID_EXTENSION_NAME`, for the Parquet crate)
"""

SHARED_IMPLEMENTER_INVENTORY = """  (Media: what the media crates reach - the record doors a wrapper redirects to, the dimensions and the schema it answers through, the cache clock and the plan cache it writes with, and the value helpers one medium's codec reads)
pub use iobase::{append_arrow_reader_default, merge_arrow_reader_default, overwrite_arrow_reader_default_with_field}  (raised inside the crate-private `iobase`: the default record writes a media wrapper redirects to once its options are proven its own)
pub use iomedia::{container_origin, container_row_size, dimension_options, held_arrow_field, own_options}  (raised inside the crate-private `iomedia`: the handle's own options where none were given, a container's origin and row count, the options a dimension is read under, the declared or held root a schema read answers)
pub fn cache_now() -> std::time::Instant  (`media::cache::now`: the instant every cache door reads)
pub fn arrow_schema_from_field(field: &Field) -> crate::arrow::Result<arrow_schema::SchemaRef>  (`arrow::arrow_schema_from_field`: the Arrow schema a record root projects to)
pub fn arrow_cast_plan_compile_schema(source: &Schema, target: &Field, options: crate::ArrowCastOptions) -> crate::arrow::Result<crate::ArrowCastPlan>  (`ArrowCastPlan::compile_schema` with no holder deferred)
pub fn arrow_cast_plan_reconcile_batch(plan: &crate::ArrowCastPlan, batch: RecordBatch) -> crate::arrow::Result<RecordBatch>  (`ArrowCastPlan::reconcile_batch`: one batch reconciled to the plan's target as transport)
pub fn is_variant_storage(dtype: &arrow_schema::DataType) -> bool  (`variant::is_variant_storage`)
pub struct PlanCache<P>  (moved here from `cast`, where raising it would publish it: one compiled plan per distinct source layout, the pointer compared first, then the fields)
  pub const fn new() -> Self
  pub fn get_or_compile(&mut self, fields: &arrow_schema::Fields, compile: impl FnOnce() -> crate::arrow::Result<P>) -> crate::arrow::Result<&P>
"""


def edit_inventory(root: pathlib.Path, added_reserved: bool) -> None:
    rel = ".api-inventory.txt"
    text = read(root / rel)
    if "### yggdryl_parquet::implementer  [" in text:
        return
    # The moved sections, re-homed by file (the path rewrite may have done it).
    for old, new in (
        ("### yggdryl::parquet::metadata  [rust/src/parquet/metadata.rs]", "### yggdryl_parquet::metadata  [rust/parquet/src/metadata.rs]"),
        ("### yggdryl::parquet  [rust/src/parquet/mod.rs]", "### yggdryl_parquet  [rust/parquet/src/lib.rs]"),
        ("### yggdryl_parquet::metadata  [rust/parquet/src/metadata.rs]", None),
        ("### yggdryl_parquet  [rust/parquet/src/lib.rs]", None),
    ):
        if new is None:
            continue
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
    text = text.replace(
        "### yggdryl_parquet  [rust/parquet/src/lib.rs]\n",
        PARQUET_LIB_SECTION.split("\n", 1)[0] + "\n" + PARQUET_LIB_SECTION.split("\n", 1)[1], 1)
    if "### yggdryl_parquet  [rust/parquet/src/lib.rs]" not in text:
        residue(rel, "no `yggdryl::parquet` section to re-home as the crate root's")
    text = replace_once(
        text,
        "  pub static PARQUET_CODEC: ParquetCodec  (claimed by the core under `application/vnd.apache.parquet`, rank 1, named `parquet`, titled `Parquet`, compresses internally; options held as RecordOptions::Registered)",
        "  pub static PARQUET_CODEC: ParquetCodec  (claimed by `install()` under `application/vnd.apache.parquet` at its reserved rank 1, named `parquet`, titled `Parquet`, compresses internally; options held as RecordOptions::Registered)",
        rel,
    )
    anchor = text.find("### yggdryl_parquet::metadata  [")
    if anchor < 0:
        residue(rel, "no `yggdryl_parquet::metadata` section to place the implementer's beside")
    else:
        text = text[:anchor] + PARQUET_IMPLEMENTER_INVENTORY + text[anchor:]
    if added_reserved:
        text = replace_once(
            text,
            "pub const EXTERNAL_RANK: u8 = 32  (the rank a medium outside the core takes at least; the core's seven hold the positions below it, in the order their options sort: ipc 0, parquet 1, avro 2, text 3, xmla 4, csv 5, excel 6)",
            "pub const EXTERNAL_RANK: u8 = 32  (the rank a medium outside the core takes at least; the positions below it, in the order their options sort - ipc 0, parquet 1, avro 2, text 3, xmla 4, csv 5, excel 6 - are the core's own media's and the RESERVED_RANKS of the media split off it)\n"
            'pub const RESERVED_RANKS: [(&str, u8); 4] = [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)]  (the ranks the media split off the core keep, a wire contract that never moves: claim admits a medium at one only under the name beside it, refuses that name at any other rank, and holds every other medium at or above EXTERNAL_RANK; re-exported from yggdryl::media)',
            rel,
        )
    for old, new in (
        ("the core claims its own six before the register answers anything, until they move to the crates that hold them - yggdryl-avro's install() claims Avro at its reserved rank;",
         "the core claims its own five before the register answers anything, until they move to the crates that hold them - yggdryl-avro's install() claims Avro and yggdryl-parquet's Parquet at their reserved ranks;"),
        ("the core claims its own seven before the register answers anything, until they move to the crates that hold them;",
         "the core claims its own six before the register answers anything, until they move to the crates that hold them - yggdryl-parquet's install() claims Parquet at its reserved rank;"),
        ("a definition moved here, `InstantSequence` and `Staged`;",
         "a definition moved here, `InstantSequence`, `Staged` and `PlanCache`;"),
    ):
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
    start = text.find("### yggdryl::implementer  [")
    end = text.find("\n### ", start + 1) if start >= 0 else -1
    if start < 0 or end < 0:
        residue(rel, "no `yggdryl::implementer` section to list the Parquet additions under")
    else:
        section = text[start:end].rstrip("\n")
        add = "" if "  (Media: what the media crates reach" in section else SHARED_IMPLEMENTER_INVENTORY
        text = text[:start] + section + "\n" + add + CORE_IMPLEMENTER_INVENTORY + text[end:]
    write(root / rel, text)
    # The bindings' inventory names the Python and Node sources it reads.
    done(".api-inventory.txt: the Parquet sections re-homed under yggdryl_parquet, the crate root, its implementer, the core's additions")


# ---------------------------------------------------------------------------
# Step 13: what the compiler loop still has to see
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
            for m in re.finditer(r"\byggdryl::parquet\b|\bcrate::parquet\b|yggdryl::internals::parquet", text):
                residue(f"{rel}:{line_of(text, m.start())}", f"`{m.group(0)}` is left")
    # The core's Iceberg reaches the crate it cannot depend on (D39's order).
    for name in ICEBERG_FILES:
        rel = f"rust/src/iceberg/{name}"
        if (root / rel).exists() and CRATE in read(root / rel):
            residue(rel, "the core's `iceberg` feature names `yggdryl_parquet`, which the core cannot depend on "
                         "(a cycle): it compiles once Iceberg is `yggdryl-iceberg`; until then every "
                         "`--all-features` build, the bindings (which enable `yggdryl/iceberg`) and the core's "
                         "Iceberg tests fail - see the report")
    # Core tests and benches still reading Parquet through the register.
    for folder in ("rust/tests", "rust/benchmarks"):
        for path in sorted((root / folder).rglob("*.rs")):
            rel = str(path.relative_to(root))
            text = read(path)
            gates = [line_of(text, m.start()) for m in GATE.finditer(no_comments(text))]
            if gates and rel.startswith(("rust/tests/avro/", "rust/benchmarks/media/avro/")):
                residue(rel, f"a `parquet` gate at line(s) {', '.join(map(str, gates))} is Avro's snappy (D16): "
                             "the Avro move makes it unconditional")
            elif gates:
                residue(rel, f"a `parquet` gate is left at line(s) {', '.join(map(str, gates[:12]))}: "
                             "Iceberg's own (moves with `yggdryl-iceberg`) or to resolve by hand")
            hits = [line_of(text, m.start()) for m in re.finditer(rf"\b{CRATE}\b", code_only(text))]
            if hits:
                residue(rel, f"a core test or bench names `{CRATE}` at line(s) {', '.join(map(str, sorted(set(hits))[:12]))}: "
                             "Iceberg's own (moves with `yggdryl-iceberg`, which installs Parquet) or to move by hand (D39)")
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
    # Pages reading a `.parquet` name in Rust whose block was judged not to
    # read it through the register: the docs runner says.
    for f in tracked(root, "docs", "skills"):
        if not f.endswith(".md"):
            continue
        text = read(root / f)
        for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
            body = m.group(3)
            if (m.group(2).strip().startswith("rust") and names_parquet(body) and f"{CRATE}::install()" not in body
                    and REGISTER_DOORS.search(body)):
                residue(f"{f}:{line_of(text, m.start())}", "a Rust block reads a Parquet name through a register door and installs nothing")


# ---------------------------------------------------------------------------
# The run
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=pathlib.Path)
    parser.add_argument("--no-git-lock", action="store_true", help="the caller holds $S/git.lock already")
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s6_parquet" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    # Step 0: a tree already moved is left as it is.
    if (root / CRATE_DIR / "Cargo.toml").exists():
        print(f"[s6-parquet] {CRATE_DIR}/Cargo.toml exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/parquet/mod.rs").exists():
        raise SystemExit("rust/src/parquet/mod.rs is missing: this is not the tree the Parquet move moves")
    if git(root, "status", "--porcelain").strip():
        raise SystemExit("the tree has changes; run the Parquet move on a clean tree")
    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)
    import tomllib

    core_manifest = tomllib.loads(read(root / "rust/Cargo.toml"))
    public = parquet_public(read(root / "rust/src/parquet/mod.rs"))
    internals: dict[str, list[str]] = {}
    for m in re.finditer(r"pub use crate::(parquet(?:::[\w:]+)?)::internals as (\w+);", read(root / "rust/src/lib.rs")):
        internals[m.group(2)] = m.group(1).split("::")[1:]
    tables = ParquetTables(public, internals)
    load_trait_methods(root)
    moves = plan_moves(root)
    paths = PathMap(moves)

    # Step 2-3 read the core's files where they stand.
    splits = split_tests(root, moves)
    for _old, new in splits:
        write(root / new, resolve_gates(read(root / new), new, medium=True))
    for old, _new in splits:
        if (root / old).exists() and GATE.search(no_comments(read(root / old))):
            write(root / old, resolve_gates(read(root / old), old, medium=False))
    benches = carve_benches(root)
    done(f"benches: {', '.join(benches) or 'none'} carved out of the core's shared record benches")

    # Step 1: the moves.
    with s4.GitLock(not arguments.no_git_lock):
        for old, new in moves:
            (root / new).parent.mkdir(parents=True, exist_ok=True)
            git(root, "mv", old, new)
    done(f"moves: {len(moves)} files moved by git")

    # Step 4-5.
    build_lib(root)
    edit_crate_sources(root)
    added_reserved = edit_codec(root)
    if added_reserved:
        edit_media_register(root)
    edit_implementer(root)
    edit_core_manifest(root)
    done("crate: lib.rs, implementer.rs, the plan cache and the landing; core: the seed, RESERVED_RANKS, the implementer")

    # Step 6.
    rewriter = s4.Rewriter(tables)
    iceberg = edit_iceberg(root, rewriter, tables)
    done(f"the core's Iceberg re-spelled onto `{CRATE}`: {', '.join(iceberg) or 'nothing'}")
    rewrite_paths(root, rewriter, tables, paths, moves, splits)
    finish_crate_text(root)
    siblings, linking = install_in_sibling_tests(root)
    if siblings:
        done(f"sibling leaf tests reaching Parquet install it: {', '.join(siblings)}")

    # Step 8-12.
    install_everywhere(root)
    crate_manifest(root, core_manifest, version)
    edit_tooling(root)
    edit_ci(root, linking)
    edit_docs(root)
    edit_inventory(root, added_reserved)

    # Step 13.
    scan_residue(root)
    with s4.GitLock(not arguments.no_git_lock):
        git(root, "add", "-A", "--", CRATE_DIR)
    report = ["# S6 Parquet residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE] + ["", "## Residue", ""] + (RESIDUE or ["- none"])
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s6-parquet] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
