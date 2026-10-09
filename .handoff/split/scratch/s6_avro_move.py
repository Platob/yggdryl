#!/usr/bin/env python3
"""S6a: move the Avro medium into `yggdryl-avro` at `rust/avro/`.

Usage:
    python3 -I s6_avro_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch after S4 (D39's order: avro
first of the media) - and it leaves the tree uncommitted: `git status` shows
the moves as renames and every rewrite as a modification, for the lane
manager's compiler loop (`cargo check --workspace --all-targets
--all-features --keep-going`). It runs no cargo command. A second run on a
tree it already moved stops at step 0 and changes nothing.

What it does, in order (each step says what it touched; `--residue` gets
every site it could not rewrite mechanically, `file:line` each):

 1. moves      - `git mv` by glob: `rust/src/avro/` to `rust/avro/src/`
                 (`mod.rs` the crate root `lib.rs`), `rust/tests/avro{,.rs}`
                 to `rust/avro/tests/`, `rust/tests/interop/avro.rs` to
                 `rust/avro/tests/interop/avro.rs`, `rust/benchmarks/media/
                 avro{,.rs}` to `rust/avro/benchmarks/avro{,.rs}`.
 2. test split - before the moves, every core test item that reaches Avro -
                 a `yggdryl::avro` path, a name imported from it, a handle
                 whose name declares Avro, `MimeType::AVRO`, the medium's
                 name - moves byte-identical into `rust/avro/tests/` at the
                 same relative path, its helpers copied (a macro invocation
                 with the items it defines), each half's imports pruned - a
                 trait whose methods a half calls kept - the harness written
                 beside it from the core's (S4's split, D39: a test lives in
                 the lowest crate that can name everything it uses); a sibling
                 leaf's test reaching Avro (`rust/market/tests/graph/serve.rs`
                 after S4) stays and installs the crate, its manifest gaining
                 the dev-dependency and its `[leaves]` line `after = ["avro"]`.
 3. bench split - the Avro rows of the core's shared record benchmarks
                 (`holder/calls.rs` records, `media/io/{dimensions,write,
                 pushdown}.rs`) leave the core for `rust/avro/benchmarks/avro/
                 {calls,io}.rs`, built from the core's own items byte-identical
                 plus the glue that registers the Avro rows alone.
 4. crate      - `rust/avro/src/lib.rs` (the doc, `#![deny(unsafe_code)]`,
                 `install()` claiming `AVRO_CODEC` at its reserved rank 2, the
                 hidden `implementer` for what Iceberg reaches), the container,
                 datum and schema items Iceberg reads raised to `pub` inside
                 their crate-private modules, the `parquet` (snappy) and
                 `iceberg` gates made unconditional (D16), the plan-cache
                 call re-spelled through the core's forwarders.
 5. core       - `pub mod avro` and the seed claim gone; `RESERVED_RANKS` and
                 the claim rule (D39); `yggdryl::implementer` grown by S3's
                 routes (R raises in `iobase`, `iomedia`, `media::options`,
                 `metadata`; F/A forwarders; `PlanCache` moved in, M);
                 Iceberg's manifest re-spelled onto `yggdryl_avro`; `snap`
                 and the `parquet` feature's `dep:snap` gone (D16).
 6. paths      - every `crate::` path of the moved sources by owner, every
                 `yggdryl::avro` path and `use` tree in tests, benches,
                 bindings, docs and skills, `yggdryl::internals::avro_*`, file
                 paths in every text file, `#[path]`/`CARGO_MANIFEST_DIR`
                 re-anchored.
 7. interop    - `rust/avro/tests/interop.rs`, the exchange directory kept at
                 `rust/target/avro-interop`, the driver's cargo line
                 `-p yggdryl-avro`.
 8. install    - every avro test opens with `crate::install::installed()`,
                 the bench `main` installs, a page's Rust block reaching Avro
                 installs, the bindings' init and the CLI's `main` install.
 9. manifests  - `rust/avro/Cargo.toml` and its README, the workspace
                 members and `[workspace.dependencies]`, the core's, the
                 bindings', the CLI's.
10. tooling    - `generate_internals.py` and `check_api_inventory.py` per
                 crate (S4's edits, where S4 has not landed), the generator run.
11. CI         - the `avro` leaf line, `x-avro` reading the crate, the
                 planner test, the workflow's comments.
12. docs       - the Avro page, the media overview's register section, the
                 testing page, the skills, the contributing page, AGENTS.md,
                 the READMEs, `.api-inventory.txt` re-homed.
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

CORE, AVRO = "core", "avro"
PACKAGE = "yggdryl-avro"
CRATE = "yggdryl_avro"
CRATE_DIR = "rust/avro"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-avro] {what}", flush=True)


# S4's helpers report into this script's residue; its rewriter learns the crate.
s4.residue = residue
s4.CRATE_NAME[AVRO] = CRATE

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, render_use, UseParseError = s4.parse_use, s4.render_use, s4.UseParseError
line_of = s4.line_of

# The methods of every trait a file may import only to call them: the tree's
# own traits, read off their definitions by `load_trait_methods`, and the
# outside ones the moved tests and benches import.
TRAIT_METHODS: dict[str, set[str]] = {
    "Read": {"read", "read_exact", "read_to_end", "read_to_string", "take", "bytes", "by_ref"},
    "Write": {"write", "write_all", "flush", "write_fmt", "write_str", "write_char", "by_ref"},
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


def replace_n(text: str, old: str, new: str, count: int, where: str) -> str:
    found = text.count(old)
    if found != count:
        residue(where, f"anchor matched {found} times, expected {count}, not edited: {old.strip()[:90]!r}")
        return text
    return text.replace(old, new)


def edit(root: pathlib.Path, rel: str, *pairs: tuple[str, str], optional: bool = False) -> bool:
    """Exact-string edits of one file, each anchor asserted to match once."""
    path = root / rel
    if not path.exists():
        if not optional:
            residue(rel, "absent in this tree; its edits were not made")
        return False
    text = read(path)
    for old, new in pairs:
        if old in text and text.count(old) == 1:
            text = text.replace(old, new, 1)
        elif new and new in text:
            continue
        else:
            residue(rel, f"anchor matched {text.count(old)} times, not edited: {old.strip()[:90]!r}")
    write(path, text)
    return True


# ---------------------------------------------------------------------------
# Who owns a path
# ---------------------------------------------------------------------------

# The core's crate-private items the moved sources reach, by the path they
# spell after `crate::`, and the name `yggdryl::implementer` publishes them
# under (S3's routes; D39's list, as the tree stands after P2).
CORE_ROUTES: dict[tuple[str, ...], str] = {
    ("arrow", "arrow_schema_from_field"): "arrow_schema_from_field",
    ("arrow", "field_from_arrow_schema"): "field_from_arrow_schema",
    ("cast", "PlanCache"): "PlanCache",
    ("decimal", "decimal_parameters"): "decimal_parameters",
    ("hashing", "stable_hash_of"): "stable_hash_of",
    ("iobase", "append_arrow_reader_default"): "append_arrow_reader_default",
    ("iobase", "merge_arrow_reader_default"): "merge_arrow_reader_default",
    ("iobase", "overwrite_arrow_reader_default_with_field"): "overwrite_arrow_reader_default_with_field",
    ("iomedia", "container_origin"): "container_origin",
    ("iomedia", "container_row_size"): "container_row_size",
    ("iomedia", "dimension_options"): "dimension_options",
    ("iomedia", "held_arrow_field"): "held_arrow_field",
    ("iomedia", "own_options"): "own_options",
    ("is_variant_storage",): "is_variant_storage",
    ("media", "cache", "now"): "cache_now",
    ("media", "options", "FileThreads"): "FileThreads",
    ("metadata", "sorted_pairs"): "sorted_pairs",
    ("string", "is_text_storage"): "is_text_storage",
    ("text", "expected_got"): "expected_got",
    ("uuid_bytes",): "uuid_bytes",
    ("uuid_parse",): "uuid_parse",
    ("uuid_text",): "uuid_text",
}

# The Avro modules the crate root does not publish: a path through one is the
# crate's own, never a caller's.
AVRO_PRIVATE_MODULES = {"arrow", "batch", "container", "datum", "resolve", "schema", "single"}


class AvroTables:
    """`s4.Rewriter`'s resolution, for one module leaving the core."""

    def __init__(self, public: set[str], internals: dict[str, str]) -> None:
        self.public = public
        self.internals = internals
        # Inside the crate an Avro path is the crate's own module path.
        self.inside = False

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        if segs[0] == "avro":
            rest = segs[1:]
            if not self.inside and rest and rest[0] in AVRO_PRIVATE_MODULES and len(rest) > 1:
                return AVRO, ["implementer", *rest[1:]]
            if not self.inside and rest and rest[0] not in self.public | AVRO_PRIVATE_MODULES:
                residue("paths", f"`avro::{'::'.join(rest)}` names nothing the crate root publishes")
            return AVRO, rest
        if segs[0] == "internals" and len(segs) > 1 and segs[1] in self.internals:
            return AVRO, ["internals", self.internals[segs[1]], *segs[2:]]
        for cut in range(len(segs), 0, -1):
            name = CORE_ROUTES.get(tuple(segs[:cut]))
            if name:
                return CORE, ["implementer", name, *segs[cut:]]
        return CORE, segs


def avro_public(text: str) -> set[str]:
    """The names the Avro module re-exports at its root."""
    names = set()
    for m in re.finditer(r"(?m)^pub use \w+::(\{[^;]*\}|\w+);", text):
        names.update(s4.Tables.brace_names(m.group(1)) if m.group(1).startswith("{") else [m.group(1)])
    return names | {"install", "implementer", "internals"}


ALIAS_USE = re.compile(r"(?m)^[ \t]*(?:#[ \t]+)?use yggdryl_avro as avro;[ \t]*\n")
AVRO_ROOTED = re.compile(r"(?<![\w:$])avro::")


def unalias(text: str) -> str:
    """`use yggdryl_avro as avro;` dropped and every `avro::` path in code
    spelled `yggdryl_avro::`: the module a caller imported whole is its crate."""
    if not ALIAS_USE.search(text):
        return text
    text = ALIAS_USE.sub("", text)
    code = code_only(text)
    out, pos = [], 0
    for m in AVRO_ROOTED.finditer(code):
        out.append(text[pos:m.start()])
        out.append("yggdryl_avro::")
        pos = m.end()
    out.append(text[pos:])
    return "".join(out)


# ---------------------------------------------------------------------------
# The move table and file paths
# ---------------------------------------------------------------------------


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    moves: list[tuple[str, str]] = []
    for f in tracked(root, "rust/src/avro", "rust/tests/avro", "rust/tests/avro.rs", "rust/tests/interop/avro.rs",
                     "rust/benchmarks/media/avro", "rust/benchmarks/media/avro.rs"):
        if f.startswith("rust/src/avro/"):
            rel = f[len("rust/src/avro/"):]
            new = "rust/avro/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        elif f == "rust/tests/avro.rs" or f.startswith("rust/tests/avro/"):
            new = "rust/avro/tests/" + f[len("rust/tests/"):]
        elif f == "rust/tests/interop/avro.rs":
            new = "rust/avro/tests/interop/avro.rs"
        elif f == "rust/benchmarks/media/avro.rs":
            new = "rust/avro/benchmarks/avro.rs"
        else:
            new = "rust/avro/benchmarks/avro/" + f[len("rust/benchmarks/media/avro/"):]
        moves.append((f, new))
    return moves


class PathMap:
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/avro/", "rust/avro/src/"),
            ("rust/tests/avro/", "rust/avro/tests/avro/"),
            ("rust/benchmarks/media/avro/", "rust/avro/benchmarks/avro/"),
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
    text = re.sub(r"(?<![\w./-])rust/src/avro(?![\w/.-])", "rust/avro/src", text)
    text = re.sub(r"(?<![\w./-])rust/tests/avro(?![\w/.-])", "rust/avro/tests/avro", text)
    # A command running the Avro harness runs the crate's.
    text = re.sub(r"(?m)(-p )yggdryl( [^\n]*?--test avro\b)", r"\1yggdryl-avro\2", text)
    return text


# ---------------------------------------------------------------------------
# Step 2: the core's tests that reach Avro, item by item (S4's split)
# ---------------------------------------------------------------------------

# Files whose `.avro` names are a table format's own files or a location's
# spelling, never a handle read through the register: they stay.
SPLIT_EXCLUDED = (
    "rust/tests/iceberg/", "rust/tests/interop/iceberg.rs", "rust/tests/s3/", "rust/tests/s3tables",
    "rust/tests/medallion_ledger.rs", "rust/tests/uri/", "rust/tests/media/magic.rs",
    "rust/tests/allocations.rs", "rust/tests/interop/avro.rs", "rust/tests/avro/", "rust/tests/avro.rs",
    "rust/tests/support/",
)
RANK = {CORE: 0, AVRO: 1}
AVRO_PATH = re.compile(r"\byggdryl(?:_avro\b|::avro\b|::internals::avro_)|\bMimeType::AVRO\b")


def literal_reads_avro(literal: str) -> bool:
    body = re.sub(r'^b?r?#*"|"#*$', "", literal)
    return bool(re.search(r"\.avro\b", body)) or body in ("avro", "application/avro")


def file_imports(items: list) -> set[str]:
    """Names a level imports from `yggdryl::avro`."""
    names: set[str] = set()
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            if leaf[:2] == ["yggdryl", "avro"] and kind == "self":
                names.add(alias or leaf[-1])
    return names


def tidy_head(text: str) -> str:
    """One blank line between a file's own documentation and its first item,
    however many a removed `use` left behind."""
    return re.sub(r"\A((?:[ \t]*//![^\n]*\n)+)\n{2,}", r"\1\n", text, count=1)


def macro_names(items: list) -> None:
    """A top-level macro invocation defines what its arguments name and no
    other item does - `test_options!(TestOptions, TEST_CODEC)` is where
    `TestOptions` comes from - so it travels with the items that name it."""
    defined = {name for item in items if item.kind != "use" for name in item.names}
    for item in items:
        if item.kind.startswith("macro:") and item.kind != "macro:macro_rules" and not item.names:
            m = re.search(r"!\s*[(\[{](.*)[)\]}]", code_only(item.text), re.S)
            if m:
                item.names |= set(re.findall(r"\b([A-Z]\w*)\b", m.group(1))) - defined


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
        reaches = bool(AVRO_PATH.search(code[id(item)])) or bool(tokens[id(item)] & imports)
        if not reaches:
            reaches = any(literal_reads_avro(m.group(0)) for m in s4.STRING_LITERAL.finditer(item.text))
        owner[id(item)] = AVRO if reaches else CORE
    changed = True
    while changed:
        changed = False
        for item in body:
            if owner[id(item)] == CORE and any(owner[other] == AVRO for other in refs[id(item)]
                                               if inline_body(next(i for i in body if id(i) == other)) is None):
                owner[id(item)] = AVRO
                changed = True
    return owner, refs


def split_level(text: str, top: bool, single: bool, inherited: set[str]) -> tuple[str, str | None, int]:
    """S4's `split_level` with Avro the one crate above the core."""
    head, items = top_items(text)
    imports = inherited | file_imports(items)
    owner, refs = item_owners(items, inherited)
    moved = 0
    dest: dict[int, str] = {}
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    for item in items:
        if item.kind == "use" or owner[id(item)] != AVRO:
            continue
        body = inline_body(item)
        if body is not None:
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
    avro_text = "".join(out).rstrip("\n") + "\n"
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
    return "".join(out).rstrip("\n") + "\n", avro_text, moved


def published(text: str) -> bool:
    """Whether an item is visible beyond its own module."""
    for line in text.split("\n"):
        stripped = line.strip()
        if not stripped or stripped.startswith(("//", "#[", "#![")):
            continue
        return stripped.startswith("pub ") or stripped.startswith("pub(")
    return False


def pinned_source(rel: str) -> str | None:
    """The core source file a test file under `rust/tests/` pins."""
    parts = rel.split("/")
    if len(parts) == 1:
        return None
    if parts[0] == "root":
        return "rust/src/" + parts[1]
    name = parts[-1]
    if name == "mod_.rs":
        parts[-1] = "mod.rs"
    return "rust/src/" + "/".join(parts)


def split_tests(root: pathlib.Path, moves: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Every core test file's items reaching Avro, moved byte-identical into
    `rust/avro/tests/` at the same relative path (S4's split, D39)."""
    move_map = dict(moves)
    written: list[tuple[str, str]] = []
    needs: dict[str, list[str]] = collections.defaultdict(list)
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED):
            continue
        rel = f[len("rust/tests/"):]
        single = "/" not in rel
        home_text, avro_text, moved = split_level(read(root / f), True, single, set())
        if avro_text is None:
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
        write(root / dest_rel, tidy_head(prune_imports(prune_imports(header + avro_text))))
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


def install_in_sibling_tests(root: pathlib.Path) -> list[str]:
    """A sibling leaf's test reaching Avro stays where it is, the crate a
    dev-dependency of its own and installed by the test (D39: the lowest crate
    that names everything it uses; neither sibling is below the other)."""
    touched: list[str] = []
    for manifest in sorted((root / "rust").glob("*/Cargo.toml")):
        leaf = manifest.parent.name
        if leaf in CORE_FOLDERS or leaf == "avro":
            continue
        for path in sorted((manifest.parent / "tests").rglob("*.rs")) if (manifest.parent / "tests").exists() else []:
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
                    if item.kind != "use" and owner[id(item)] == AVRO:
                        body = inline_body(item)
                        if body is not None:
                            start, end = body
                            piece = piece[:start] + level(piece[start:end], imports) + piece[end:]
                        else:
                            for at in reversed(test_fn_bodies(piece)):
                                if "yggdryl_avro::install()" in piece[at:at + 200]:
                                    continue
                                line_start = piece.rfind("\n", 0, at) + 1
                                indent = re.match(r"[ \t]*", piece[line_start:]).group(0) + "    "
                                piece = piece[:at] + f'\n{indent}yggdryl_avro::install().expect("{PACKAGE} installs");' + piece[at:]
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
                leaf_after_avro(root, leaf)
    return touched


def leaf_after_avro(root: pathlib.Path, leaf: str) -> None:
    """A leaf whose tests link the crate runs again when Avro changes: its
    `[leaves]` line names `avro` among what it is after."""
    rel = ".github/ci/rows.toml"
    text = read(root / rel)
    m = re.search(rf"(?m)^{re.escape(leaf)} = \{{ (.*) \}}$", text)
    if not m:
        residue(rel, f"no listed `[leaves]` line for `{leaf}`: say it is after `avro` by hand")
        return
    body = m.group(1)
    after = re.search(r"after = \[([^\]]*)\]", body)
    if after and '"avro"' in after.group(1):
        return
    if after:
        listed = after.group(1).strip()
        body = body[:after.start(1)] + (f'{listed}, "avro"' if listed else '"avro"') + body[after.end(1):]
    else:
        body = re.sub(r'(package = "[^"]+")', r'\1, after = ["avro"]', body, count=1)
    write(root / rel, text[:m.start(1)] + body + text[m.end(1):])


# ---------------------------------------------------------------------------
# Step 3: the Avro rows of the core's shared record benchmarks
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
                # One `use` per module, as the repository writes them.
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


def copy_items(sources: dict[str, str], names: list[str], where: str) -> list[str]:
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


IO_DOC = """//! The Avro rows of the core's shared record benchmarks, measured as the core
//! measures its own media: `io_dimensions` (the metadata counters against the
//! decode they avoid, and the options redirection to the medium's settings),
//! `io_write_stateful` (the three write intents), `io_write_shape` (a cast
//! and a selection, and a wide nested row) and `io_pushdown` (a declared
//! subset against a whole read: rows interleave columns, so a block is still
//! decompressed whole, but an unselected column's bytes are skipped rather
//! than decoded). The fixtures and the helpers are the core's
//! `rust/benchmarks/media/io/` items, copied byte-identical (S6, D39).

"""

CALLS_DOC = """//! What the Avro record surfaces cost, in calls to storage and in time: the
//! `calls/records/avro` rows the core's `rust/benchmarks/holder/calls.rs`
//! measures for the media it keeps, over the same in-memory handle and the
//! same helpers, copied byte-identical (S6, D39). The assertions live in
//! `rust/avro/tests/iobase_calls.rs`.

"""


def carve_benches(root: pathlib.Path) -> list[str]:
    """The Avro rows leave the core's shared benches for two modules of the
    crate's own bench target, written before the moves into the folder the
    moved Avro benches land in."""
    written = []
    io = "rust/benchmarks/media/io"
    sources = {rel: read(root / rel) for rel in (f"{io}/mod.rs", f"{io}/dimensions.rs", f"{io}/write.rs", f"{io}/pushdown.rs")
               if (root / rel).exists()}
    if len(sources) == 4:
        dims = sources[f"{io}/dimensions.rs"]
        start = dims.find('    media_cases(&mut group, "avro"')
        end = dims.find('    media_cases(&mut group, "text"')
        stateful_anchor = '    stateful_triplet(\n        &mut stateful,\n        "avro",'
        write_text = sources[f"{io}/write.rs"]
        s_start = write_text.find(stateful_anchor)
        s_end = write_text.find("    );\n", s_start) + len("    );\n") if s_start >= 0 else -1
        if start < 0 or end < start or s_start < 0:
            residue(f"{io}/dimensions.rs, write.rs", "the Avro rows were not found: carve them into `rust/avro/benchmarks/avro/io.rs` by hand")
        else:
            dims_rows = dims[start:end]
            stateful_rows = write_text[s_start:s_end]
            copied = copy_items(
                sources,
                ["ROWS", "batch", "handle", "reader", "stored_with", "stored", "narrow", "materialized",
                 "decoded_rows", "media_cases", "STATEFUL_ROWS", "stateful_triplet", "avro_target",
                 "cast_source", "cast_field", "nested_wide", "SHAPE_ROWS", "WIDE_COLUMNS"],
                f"{io}/*.rs",
            )
            shape = item_named(write_text, "shape_benchmarks", f"{io}/write.rs")
            projection = item_named(sources[f"{io}/pushdown.rs"], "projection_benchmarks", f"{io}/pushdown.rs")
            glue = []
            if shape is not None:
                shape_text = replace_once(
                    shape.text.strip("\n"),
                    '    for (label, name) in [\n        ("ipc", "shape.arrows"),\n        ("parquet", "shape.parquet"),\n        ("avro", "shape.avro"),\n    ] {',
                    '    for (label, name) in [("avro", "shape.avro")] {',
                    f"{io}/write.rs shape_benchmarks",
                )
                glue.append(shape_text.replace("fn shape_benchmarks(", "pub(crate) fn shape_benchmarks(", 1))
            if projection is not None:
                projection_text = replace_once(
                    projection.text.strip("\n"),
                    '    for (label, name) in [\n        ("ipc", "bench.arrows"),\n        ("parquet", "bench.parquet"),\n        ("avro", "bench.avro"),\n    ] {\n        if label == "parquet" && !cfg!(feature = "parquet") {\n            continue;\n        }\n',
                    '    for (label, name) in [("avro", "bench.avro")] {\n',
                    f"{io}/pushdown.rs projection_benchmarks",
                )
                glue.append(projection_text)
            glue.append(
                "/// The Avro rows of `io_dimensions`.\n"
                "pub(crate) fn dimension_benchmarks(criterion: &mut Criterion) {\n"
                "    let source = batch();\n"
                '    let avro = stored_with("bench-dimensions.avro", &source);\n'
                "    let mut avro_options = RecordOptions::from(yggdryl_avro::AvroOptions::new());\n"
                '    let sync_marker = *b"0123456789abcdef";\n\n'
                '    let mut group = criterion.benchmark_group("io_dimensions");\n'
                "    group.sample_size(10);\n"
                + dims_rows
                + "    group.finish();\n}"
            )
            glue.append(
                "/// The Avro rows of `io_write_stateful`.\n"
                "pub(crate) fn stateful_benchmarks(criterion: &mut Criterion) {\n"
                "    let source = batch().slice(0, STATEFUL_ROWS);\n"
                '    let mut stateful = criterion.benchmark_group("io_write_stateful");\n'
                "    stateful.sample_size(10);\n"
                "    stateful.throughput(Throughput::Elements(STATEFUL_ROWS as u64));\n\n"
                + stateful_rows
                + "    stateful.finish();\n}"
            )
            uses = merged_uses(list(sources.values()) + ["use criterion::{Criterion, Throughput};\nuse yggdryl::media::RecordOptions;\n"])
            body = IO_DOC + uses + "\n" + "\n\n".join(copied + glue) + "\n"
            write(root / "rust/avro/benchmarks/avro/io.rs", tidy_head(prune_imports(body)))
            written.append("rust/avro/benchmarks/avro/io.rs")
            # The core keeps every other medium's rows.
            dims = dims[:start] + dims[end:]
            for line in ('    let avro = stored_with("bench-dimensions.avro", &source);\n',
                         "    let mut avro_options = RecordOptions::from(yggdryl::avro::AvroOptions::new());\n",
                         '    let sync_marker = *b"0123456789abcdef";\n',
                         "use yggdryl::avro::Avro;\n"):
                dims = replace_once(dims, line, "", f"{io}/dimensions.rs")
            write(root / f"{io}/dimensions.rs", prune_imports(dims))
            write_text = write_text[:s_start] + write_text[s_end:]
            avro_target = item_named(write_text, "avro_target", f"{io}/write.rs")
            if avro_target is not None:
                write_text = write_text[:avro_target.start] + write_text[avro_target.end:]
            write_text = replace_once(write_text, "use yggdryl::avro::Avro;\n", "", f"{io}/write.rs")
            write_text = replace_once(write_text, '        ("avro", "shape.avro"),\n', "", f"{io}/write.rs")
            write(root / f"{io}/write.rs", prune_imports(write_text))
            push = sources[f"{io}/pushdown.rs"]
            push = replace_once(push, '        ("avro", "bench.avro"),\n', "", f"{io}/pushdown.rs")
            push = replace_once(
                push,
                "record batch is one contiguous message. The Avro pair sits between the two:\n"
                "//! rows interleave columns, so the whole block is still decompressed, but an\n"
                "//! unselected column's bytes are skipped instead of decoded.\n",
                "record batch is one contiguous message. The Avro pair is `yggdryl-avro`'s own\n"
                "//! `io_pushdown` rows.\n",
                f"{io}/pushdown.rs",
            )
            write(root / f"{io}/pushdown.rs", push)
    else:
        residue(io, "the shared record benchmarks are not all here: carve their Avro rows by hand")
    calls_rel = "rust/benchmarks/holder/calls.rs"
    if (root / calls_rel).exists():
        calls = read(root / calls_rel)
        records = item_named(calls, "records", calls_rel)
        measured = item_named(calls, "measured", calls_rel)
        line = '        surfaces(criterion, "avro", "file:///lake/part.avro");\n'
        if records is None or measured is None or calls.count(line) != 1:
            residue(calls_rel, "the records call bench was not found: carve its Avro row by hand")
        else:
            start, end = inline_body(records)
            inner = records.text[start:end]
            _, inner_items = top_items(inner)
            kept = [item.text.strip("\n") for item in inner_items
                    if item.kind != "use" and "record_call_benchmarks" not in item.names]
            uses = merged_uses([calls, inner])
            glue = (
                "/// The Avro row of the record encodings' call benchmark.\n"
                "pub(crate) fn record_call_benchmarks(criterion: &mut Criterion) {\n"
                + line
                + "}"
            )
            body = CALLS_DOC + uses + "\n" + "\n\n".join([measured.text.strip("\n"), *kept, glue]) + "\n"
            write(root / "rust/avro/benchmarks/avro/calls.rs", tidy_head(prune_imports(body)))
            written.append("rust/avro/benchmarks/avro/calls.rs")
            write(root / calls_rel, calls.replace(line, "", 1))
    return written


AVRO_BENCH_ROOT = """//! The Avro medium's benchmarks: the raw codec's groups (`codec/avro*`),
//! and the Avro rows of the core's shared record benchmarks - `io_dimensions`,
//! `io_write_stateful`, `io_write_shape`, `io_pushdown` and `calls/records` -
//! measured the same way. `main` claims the medium before any group runs, so a
//! handle whose name declares Avro reads through the core's register.

#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

#[path = "avro/calls.rs"]
mod calls;
#[path = "avro/codecs.rs"]
mod codecs;
#[path = "avro/container.rs"]
mod container;
#[path = "avro/format.rs"]
mod format;
#[path = "avro/io.rs"]
mod io;
#[path = "avro/projection.rs"]
mod projection;
#[path = "avro/resolution.rs"]
mod resolution;

use criterion::criterion_group;

criterion_group!(
    avro,
    container::avro_benchmarks,
    format::format_benchmarks,
    codecs::codec_benchmarks,
    projection::projection_benchmarks,
    resolution::resolution_benchmarks,
    io::dimension_benchmarks,
    io::stateful_benchmarks,
    io::shape_benchmarks,
    io::projection_benchmarks,
    calls::record_call_benchmarks,
);

fn main() {
    // The medium the benchmarks read by name is claimed before any runs.
    yggdryl_avro::install().expect("yggdryl-avro installs");
    avro();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
"""


def edit_core_media_bench(root: pathlib.Path) -> None:
    path = "rust/benchmarks/media.rs"
    edit(
        root, path,
        ('#[path = "media/avro.rs"]\nmod avro;\n', ""),
        ("    avro::container::avro_benchmarks,\n"
         "    avro::format::format_benchmarks,\n"
         "    avro::codecs::codec_benchmarks,\n"
         "    avro::projection::projection_benchmarks,\n"
         "    avro::resolution::resolution_benchmarks,\n", ""),
    )


# ---------------------------------------------------------------------------
# Step 4: the crate's own sources
# ---------------------------------------------------------------------------

LIB_HEAD = """//! Apache Avro for the yggdryl core: schemas, datums, object containers and
//! schema resolution, and the Avro record medium over the core's handles and
//! Arrow - the `yggdryl-avro` crate.
//!
//! Avro is the exchange format Iceberg keeps its manifests in and one other
//! systems hand over on its own, so it is implemented here as a first-class
//! codec - a sibling of [`yggdryl::json`] on the byte side and of
//! [`yggdryl::ipc`] on the record side - with no Avro crate underneath, which
//! is what keeps the table format's promise that no dependency is added for
//! the format itself.
//!
//! The record medium is [`AVRO_CODEC`], claimed on the core's register of
//! media by [`install`] at the rank the core held it at: every binding's init
//! and the `yggdryl` command's `main` call it, and a Rust program that links
//! this crate calls it before the core reads a handle whose name declares
//! Avro, which is refused naming the crate to install until then. [`Avro`],
//! [`AvroOptions`] and the raw codec below answer without it.
//!
"""

INSTALL_FN = '''
/// What this crate claims the Avro medium as.
const CRATE: &str = "yggdryl-avro";

/// Claims the Avro medium on the core's register of media - [`AVRO_CODEC`]
/// under `application/avro`, at rank 2, which
/// [`RESERVED_RANKS`](yggdryl::media::RESERVED_RANKS) keeps for it since the
/// core held it there - once for the life of the process; a later call
/// returns at once. Every binding's init and the `yggdryl` command's `main`
/// call it, and so does a Rust caller before the core reads a handle whose
/// name declares Avro: until the claim such a handle's options are refused,
/// naming the crate to install.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed
/// `application/avro` or the name `avro` first.
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
    yggdryl::media::codec::claim(&AVRO_CODEC, CRATE)?;
    let _ = INSTALLED.set(());
    Ok(())
}
'''

AVRO_IMPLEMENTER = """//! The one door the Iceberg crate reaches this crate's crate-private items
//! through, as `yggdryl::implementer` is the core's: nothing here is API, and
//! an item is listed because `yggdryl-iceberg` reads a manifest's container
//! with it before the official parser does - the header it checks and parses,
//! the cursor it walks the blocks with, and the datum codec one row decodes
//! under. Each item takes one of the core's routes: a `pub use` of an item
//! raised to `pub` inside a module this crate's root does not publish, or a
//! free function over a crate-private part of a public type, named
//! `<type>_<item>`.

use std::collections::HashMap;

use smol_str::SmolStr;
use yggdryl::IOBase;

use crate::{Blocks, Schema};

/// The container's framing - its magic, the schema key and the marker
/// length - its header, parsed or as raw entries, and the block coding it
/// names, for the Iceberg crate.
pub use crate::container::{
    BlockCoding, Header, HeaderEntries, MAGIC, SCHEMA_KEY, SYNC_LEN, check_magic, header_entry,
    parse_header, parse_header_entries,
};

/// The cursor a container is walked with and the codec one datum decodes
/// under, for the Iceberg crate.
pub use crate::datum::{Cursor, DatumCodec};

/// One node of a parsed schema, which a datum decodes against, for the
/// Iceberg crate.
pub use crate::schema::Node;

/// `Schema::names`, for the Iceberg crate: every named type the schema
/// resolved, by fullname - what a [`DatumCodec`] resolves a reference
/// through.
#[must_use]
pub fn schema_names(schema: &Schema) -> &HashMap<SmolStr, Node> {
    &schema.names
}

/// `Schema::node`, for the Iceberg crate: the root node a row decodes
/// against.
#[must_use]
pub fn schema_node(schema: &Schema) -> &Node {
    &schema.node
}

/// `Blocks::metadata_bytes`, for the Iceberg crate: the header's metadata
/// entries exactly as written, which the official manifest parser reads.
#[must_use]
pub fn blocks_metadata_bytes<'blocks, H: IOBase + ?Sized>(
    blocks: &'blocks Blocks<'_, H>,
) -> &'blocks [(SmolStr, Vec<u8>)] {
    blocks.metadata_bytes()
}
"""

INTERNALS_PLACEHOLDER = "// GENERATED by scripts/generate_internals.py - do not edit by hand.\n// END GENERATED\n"


def build_lib(root: pathlib.Path) -> None:
    """`mod.rs` as the crate root: its doc rewritten for the crate, its module
    declarations and re-exports as they were, `implementer`, `install()`."""
    path = root / CRATE_DIR / "src/lib.rs"
    text = read(path)
    m = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if not m:
        residue(f"{CRATE_DIR}/src/lib.rs", "no crate doc to replace")
        return
    old_doc = m.group(0)
    # The doc's own paragraphs past the first two, which the head restates.
    paragraphs = old_doc.split("//!\n")
    tail = "//!\n".join(paragraphs[2:]) if len(paragraphs) > 2 else ""
    body = text[m.end():]
    body = body.lstrip("\n")
    body = replace_once(body, "mod single;\n", "mod single;\n\n#[doc(hidden)]\npub mod implementer;\n",
                        f"{CRATE_DIR}/src/lib.rs")
    new = (
        LIB_HEAD + tail.rstrip("\n") + "\n\n#![deny(unsafe_code)]\n\n"
        + "use std::sync::{Mutex, OnceLock, PoisonError};\n\n"
        + body.rstrip("\n") + "\n" + INSTALL_FN + "\n" + INTERNALS_PLACEHOLDER
    )
    write(path, new)


# What Iceberg reads of the container, the datum codec and the schema, raised
# to `pub` inside this crate's private modules (R).
RAISES = {
    "container.rs": [
        ("pub(crate) const MAGIC:", "pub const MAGIC:"),
        ("pub(crate) const SCHEMA_KEY:", "pub const SCHEMA_KEY:"),
        ("pub(crate) const SYNC_LEN:", "pub const SYNC_LEN:"),
        ("pub(crate) enum BlockCoding {", "pub enum BlockCoding {"),
        ("    pub(crate) fn load_into(", "    pub fn load_into("),
        ("pub(crate) struct Header {\n    /// The writer schema.\n    pub(crate) schema: Schema,\n"
         "    /// Metadata minus the reserved keys.\n    pub(crate) metadata: Vec<(SmolStr, SmolStr)>,\n"
         "    /// The block compression.\n    pub(crate) coding: BlockCoding,\n",
         "pub struct Header {\n    /// The writer schema.\n    pub schema: Schema,\n"
         "    /// Metadata minus the reserved keys.\n    pub metadata: Vec<(SmolStr, SmolStr)>,\n"
         "    /// The block compression.\n    pub coding: BlockCoding,\n"),
        ("    pub(crate) sync: [u8; SYNC_LEN],", "    pub sync: [u8; SYNC_LEN],"),
        ("pub(crate) type HeaderEntries", "pub type HeaderEntries"),
        ("pub(crate) fn parse_header_entries(", "pub fn parse_header_entries("),
        ("pub(crate) fn header_entry<'entries>(", "pub fn header_entry<'entries>("),
        ("pub(crate) fn parse_header(", "pub fn parse_header("),
        ("pub(crate) fn check_magic(", "pub fn check_magic("),
        # D16: snappy and the raw header bytes are the crate's, whatever the core builds.
        ('fn implemented_codecs() -> &\'static str {\n    if cfg!(feature = "parquet") {\n'
         '        "null, deflate, snappy, zstandard"\n    } else {\n        "null, deflate, zstandard"\n    }\n}',
         'fn implemented_codecs() -> &\'static str {\n    "null, deflate, snappy, zstandard"\n}'),
    ],
    "datum.rs": [
        ("pub(crate) struct Cursor<'bytes> {", "pub struct Cursor<'bytes> {"),
        ("    pub(crate) const fn new(bytes: &'bytes [u8]) -> Self {", "    pub const fn new(bytes: &'bytes [u8]) -> Self {"),
        ("    pub(crate) fn take(&mut self, count: usize)", "    pub fn take(&mut self, count: usize)"),
        ("    pub(crate) fn long(&mut self)", "    pub fn long(&mut self)"),
        ("    pub(crate) fn bytes(&mut self)", "    pub fn bytes(&mut self)"),
        ("    pub(crate) const fn is_exhausted(&self)", "    pub const fn is_exhausted(&self)"),
        ("pub(crate) struct DatumCodec<'schema> {\n    /// The schema's named types, for resolving references.\n"
         "    pub(crate) names: &'schema HashMap<SmolStr, Node>,\n    /// The nesting and allocation bounds.\n"
         "    pub(crate) limits: Limits,",
         "pub struct DatumCodec<'schema> {\n    /// The schema's named types, for resolving references.\n"
         "    pub names: &'schema HashMap<SmolStr, Node>,\n    /// The nesting and allocation bounds.\n"
         "    pub limits: Limits,"),
        ("    pub(crate) fn decode<'node>(", "    pub fn decode<'node>("),
        ("    pub(crate) fn budget(&self) -> usize {", "    pub fn budget(&self) -> usize {"),
    ],
    "schema.rs": [
        ("pub(crate) enum Node {", "pub enum Node {"),
        ("pub(crate) struct RecordType {", "pub struct RecordType {"),
        ("pub(crate) struct EnumType {", "pub struct EnumType {"),
        ("pub(crate) struct FixedType {", "pub struct FixedType {"),
        ("pub(crate) struct DecimalType {", "pub struct DecimalType {"),
    ],
}

BATCH_PLAN_OLD = """        let batch = plans
            .get_or_compile(batch.schema_ref().fields(), || {
                ArrowCastPlan::compile_schema(
                    batch.schema_ref(),
                    &canonical,
                    ArrowCastOptions::new().with_safe(false),
                    Deferred::default(),
                )
            })?
            .reconcile_batch(batch)?;"""

BATCH_PLAN_NEW = """        let plan = plans.get_or_compile(batch.schema_ref().fields(), || {
            arrow_cast_plan_compile_schema(
                batch.schema_ref(),
                &canonical,
                ArrowCastOptions::new().with_safe(false),
            )
        })?;
        let batch = arrow_cast_plan_reconcile_batch(plan, batch)?;"""


def edit_crate_sources(root: pathlib.Path) -> None:
    src = root / CRATE_DIR / "src"
    for name, pairs in RAISES.items():
        edit(root, f"{CRATE_DIR}/src/{name}", *pairs)
    # The `parquet` (snappy) and `iceberg` (raw header bytes) gates leave with
    # the crate: what they guarded is the crate's whatever the core builds.
    for path in sorted(src.glob("*.rs")):
        text = read(path)
        new = re.sub(r'(?m)^[ \t]*#\[cfg\(feature = "(?:parquet|iceberg)"\)\]\n', "", text)
        if new != text:
            write(path, new)
    edit(root, f"{CRATE_DIR}/tests/avro/container.rs",
         ('    #[cfg(feature = "parquet")]\n    mod snappy {', "    mod snappy {"))
    edit(root, f"{CRATE_DIR}/benchmarks/avro/codecs.rs",
         ('    let mut codecs = vec!["null", "deflate", "zstandard"];\n    if cfg!(feature = "parquet") {\n'
          '        codecs.push("snappy");\n    }\n',
          '    let codecs = ["null", "deflate", "zstandard", "snappy"];\n'))
    if 'cfg!(feature = "parquet")' in read(src / "container.rs"):
        residue(f"{CRATE_DIR}/src/container.rs", "a `cfg!(feature = \"parquet\")` is left: make it unconditional")
    # The writer's plan cache, through the core's forwarders.
    edit(
        root, f"{CRATE_DIR}/src/batch.rs",
        ("use crate::cast::{ArrowCastPlan, Deferred, PlanCache};\n",
         "use crate::cast::PlanCache;\nuse yggdryl::implementer::{arrow_cast_plan_compile_schema, arrow_cast_plan_reconcile_batch};\n"),
        (BATCH_PLAN_OLD, BATCH_PLAN_NEW),
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


def edit_codec(root: pathlib.Path) -> None:
    rel = "rust/src/media/codec.rs"
    edit(
        root, rel,
        ("            &crate::avro::AVRO_CODEC,\n", ""),
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


def raise_in(root: pathlib.Path, rel: str, names: list[str]) -> None:
    """`pub(crate) fn <name>` raised to `pub fn` (R): the module holding it is
    one the crate root does not publish."""
    path = root / rel
    text = read(path)
    for name in names:
        pattern = re.compile(rf"(?m)^pub\(crate\) fn {name}\b")
        if len(pattern.findall(text)) != 1:
            residue(rel, f"`pub(crate) fn {name}` matched {len(pattern.findall(text))} times, not raised")
            continue
        text = pattern.sub(f"pub fn {name}", text)
    write(path, text)


def move_use_names(text: str, statement: re.Pattern, names: list[str], into: str, where: str) -> str:
    """Take `names` out of the one `use` tree `statement` matches and add them to
    the tree `into` is the head of (`pub use transfer::{`), or a new line."""
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
    # Every core user reaches it through the implementer.
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


IMPLEMENTER_SECTION = """
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

/// The threads one file decodes or encodes on, for the media crates: outside
/// every options value's identity.
pub use crate::media::options::FileThreads;

/// Map-shaped pairs borrowed in key order, for the Avro crate.
pub use crate::metadata::sorted_pairs;

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

/// `string::is_text_storage`, for the Avro crate: whether a string leaf's
/// bytes ride Arrow's text layouts rather than its binary ones.
#[inline]
#[must_use]
pub const fn is_text_storage(parameters: crate::StringType) -> bool {
    crate::string::is_text_storage(parameters)
}

/// `variant::is_variant_storage`, for the Avro crate: whether an Arrow
/// datatype is the variant's storage.
#[inline]
#[must_use]
pub fn is_variant_storage(dtype: &arrow_schema::DataType) -> bool {
    crate::is_variant_storage(dtype)
}

/// `decimal::decimal_parameters`, for the Avro crate: a decimal's precision
/// and scale narrowed to their stored widths, or the reason they do not fit.
///
/// # Errors
///
/// `expected a decimal precision fitting u8, got {precision}`, else
/// `expected a decimal scale fitting i8, got {scale}`.
#[inline]
pub fn decimal_parameters<P, S>(precision: P, scale: S) -> std::result::Result<(u8, i8), SmolStr>
where
    P: TryInto<u8> + fmt::Display + Copy,
    S: TryInto<i8> + fmt::Display + Copy,
{
    crate::decimal::decimal_parameters(precision, scale)
}

/// `uuid::uuid_bytes`, for the Avro crate: the bytes a UUID value carries, in
/// either accepted spelling.
#[inline]
#[must_use]
pub fn uuid_bytes(value: &Scalar) -> Option<&[u8]> {
    crate::uuid_bytes(value)
}

/// `uuid::uuid_parse`, for the Avro crate: bytes validated as one identifier,
/// answered as its sixteen storage bytes.
///
/// # Errors
///
/// Returns an error naming the accepted spellings when the bytes are neither.
#[inline]
pub fn uuid_parse(value: &[u8]) -> Result<[u8; 16]> {
    crate::uuid_parse(value)
}

/// `uuid::uuid_text`, for the Avro crate: the canonical 36-character
/// lowercase rendering of one identifier.
#[inline]
#[must_use]
pub fn uuid_text(stored: &[u8; 16]) -> SmolStr {
    crate::uuid_text(stored)
}

"""


def edit_implementer(root: pathlib.Path, plan_cache: str) -> None:
    rel = "rust/src/implementer.rs"
    text = read(root / rel)
    if "pub fn arrow_cast_plan_compile_schema(" in text:
        return
    text = replace_once(
        text,
        "//! - a definition moved here, where raising it inside a published module\n"
        "//!   would publish it: [`InstantSequence`] and [`Staged`];\n",
        "//! - a definition moved here, where raising it inside a published module\n"
        "//!   would publish it: [`InstantSequence`], [`Staged`] and [`PlanCache`];\n",
        rel,
    )
    text = text.rstrip("\n") + "\n" + IMPLEMENTER_SECTION + plan_cache.rstrip("\n") + "\n"
    write(root / rel, text)
    # R: the raises behind the `pub use`s.
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
    # A raised item's doc names no private item by link.
    edit(
        root, "rust/src/iomedia.rs",
        ("/// [`field_under`]. A media wrapper", "/// `field_under`. A media wrapper"),
        ("/// Returns the origin's read failure, [`no_schema`], or a clause that does\n",
         "/// Returns the origin's read failure, `no_schema`, or a clause that does\n"),
        ("/// Returns what [`container_field`] returns.\n", "/// Returns what `container_field` returns.\n"),
    )
    edit(
        root, "rust/src/media/options.rs",
        ("pub(crate) struct FileThreads(pub(crate) Option<usize>);", "pub struct FileThreads(pub Option<usize>);"),
        ("    pub(crate) fn resolve(self) -> usize {", "    pub fn resolve(self) -> usize {"),
    )
    edit(root, "rust/src/metadata/pairs.rs",
         ("pub(crate) fn sorted_pairs<K: Ord, V>", "pub fn sorted_pairs<K: Ord, V>"))
    edit(root, "rust/src/metadata.rs",
         ("pub(crate) use pairs::sorted_pairs;", "pub use pairs::sorted_pairs;"))


MANIFEST_SUBS = [
    ("    official_manifest_metadata(crate::avro::read_blocks(handle)?.metadata_bytes())",
     "    official_manifest_metadata(yggdryl_avro::implementer::blocks_metadata_bytes(\n        &yggdryl_avro::read_blocks(handle)?,\n    ))", 1),
    ("names: &header.schema.names,", "names: yggdryl_avro::implementer::schema_names(&header.schema),", 1),
    ("datum.decode(&header.schema.node,", "datum.decode(yggdryl_avro::implementer::schema_node(&header.schema),", 1),
    ("    use crate::avro::container;\n", "", 3),
    ("    use crate::avro::datum::Cursor;\n", "    use yggdryl_avro::implementer::Cursor;\n", 2),
    ("    use crate::avro::datum::{Cursor, DatumCodec};\n", "    use yggdryl_avro::implementer::{Cursor, DatumCodec};\n", 1),
    ("crate::avro::container::Header", "yggdryl_avro::implementer::Header", 2),
    ("crate::avro::write_container(", "yggdryl_avro::write_container(", 3),
    ("crate::avro::read_container(", "yggdryl_avro::read_container(", 2),
    ("[`crate::avro`]", "[`yggdryl_avro`]", 1),
]


def edit_manifest(root: pathlib.Path) -> None:
    """Iceberg's manifest reads the container through `yggdryl_avro` (D39:
    `yggdryl-iceberg` depends on `yggdryl-avro` and reaches its hidden
    implementer); wherever Iceberg lives when this runs."""
    for rel in ("rust/src/iceberg/manifest.rs", "rust/iceberg/src/manifest.rs"):
        path = root / rel
        if not path.exists():
            continue
        text = read(path)
        if "crate::avro" not in text and "yggdryl::avro" not in text:
            continue
        text = text.replace("yggdryl::avro::", "crate::avro::")
        for old, new, count in MANIFEST_SUBS:
            text = replace_n(text, old, new, count, rel)
        code = code_only(text)
        out, pos = [], 0
        for m in re.finditer(r"(?<![\w:$])container::", code):
            out.append(text[pos:m.start()])
            out.append("yggdryl_avro::implementer::")
            pos = m.end()
        out.append(text[pos:])
        text = "".join(out)
        if "crate::avro" in text:
            residue(rel, "a `crate::avro` path is left")
        write(path, text)
        done(f"{rel}: the manifest's container reads re-spelled onto `yggdryl_avro`")


def edit_core_lib_and_manifest(root: pathlib.Path) -> None:
    edit(root, "rust/src/lib.rs", ("pub mod avro;\n", ""))
    rel = "rust/Cargo.toml"
    text = read(root / rel)
    text = replace_once(
        text,
        "# consumer does not need. `snap` rides here because the parquet crate already\n"
        "# compiles it, so Avro's snappy blocks cost no crate a build was not paying.\n"
        'parquet = ["dep:parquet", "dep:snap"]\n',
        '# consumer does not need.\nparquet = ["dep:parquet"]\n',
        rel,
    )
    text = replace_once(
        text,
        "# Raw snappy blocks for Avro containers; optional because the parquet crate is\n"
        "# what already pulls it, so it rides that feature rather than adding a crate.\n"
        'snap = { version = "1.1", optional = true }\n',
        "",
        rel,
    )
    text = replace_once(
        text,
        "# Shared, reference-counted byte views: the column chunks a Parquet read\n"
        "# fetches and the container an Avro read decodes, each shared by the threads\n"
        "# that decode it. Arrow's buffer crate already compiles it, so naming it costs\n"
        "# no crate.\n",
        "# Shared, reference-counted byte views: the column chunks a Parquet read\n"
        "# fetches, shared by the threads that decode them. Arrow's buffer crate\n"
        "# already compiles it, so naming it costs no crate.\n",
        rel,
    )
    text = replace_once(
        text,
        "# Record media: Avro is unconditional; Parquet-backed I/O and Iceberg groups\n"
        "# compile only with their owning features.\n",
        "# Record media: CSV, Excel and XML for Analysis are unconditional;\n"
        "# Parquet-backed I/O and Iceberg groups compile only with their owning\n"
        "# features. Avro's groups are `yggdryl-avro`'s own `avro` target.\n",
        rel,
    )
    if "exclude = [" in text.split("[features]")[0]:
        m = re.search(r'exclude = \[([^\]]*)\]', text)
        if '"/avro"' not in m.group(1):
            text = text[:m.start(1)] + m.group(1) + ', "/avro"' + text[m.end(1):]
    else:
        text = replace_once(
            text,
            'categories = ["data-structures", "encoding"]\n',
            'categories = ["data-structures", "encoding"]\n'
            "# The split crates sit inside this folder; each is its own package.\n"
            'exclude = ["/avro"]\n',
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


def rewrite_paths(root: pathlib.Path, rewriter, paths: PathMap, moves, splits) -> None:
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new[new] = old
    crate_ctx = s4.Context(AVRO, True)
    outside = s4.Context(None, False)
    count = 0
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        old = moved_new.get(rel, rel)
        text = read(path)
        local = paths.with_files({o: n for o, n in splits})
        new = s4.reanchor(text, old, rel, local)
        ctx = crate_ctx if rel.startswith(f"{CRATE_DIR}/src/") else outside
        rewriter.t.inside = ctx is crate_ctx
        new = unalias(rewriter.rs_file(new, ctx, rel))
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
            new = text
            if folder != "rust/src" or rel.startswith(("rust/src/iceberg/", "rust/src/s3tables/")):
                new = unalias(rewriter.rs_file(s4.reanchor(text, rel, rel, paths), outside, rel))
            else:
                new = s4.reanchor(text, rel, rel, paths)
            if new != text:
                write(path, new)
                count += 1
    for leaf in sorted((root / "rust").glob("*/Cargo.toml")):
        if leaf.parent.name in CORE_FOLDERS or leaf.parent.name == "avro":
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
            if not AVRO_NAMED.search(text):
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


AVRO_NAMED = re.compile(r"\byggdryl::(?:avro\b|internals::avro_|\{[^;]*\bavro\b)")


def markdown(rewriter, text: str, where: str) -> str:
    """S4's markdown rewrite, touching only the Rust blocks and the prose that
    name Avro, and refusing a block whose lines do not all carry its indent -
    a string continued at column 0 is content, never indentation."""
    outside = s4.Context(None, False)
    out: list[str] = []
    pos = 0
    for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
        prose = text[pos:m.start(3)]
        out.append(rewriter.inline(prose, outside, where) if AVRO_NAMED.search(prose) else prose)
        body = m.group(3)
        if m.group(2).strip().startswith("rust") and AVRO_NAMED.search(body):
            indent = m.group(1)
            lines = body.split("\n")
            if indent and not all(l.startswith(indent) or not l.strip() for l in lines):
                residue(f"{where}:{line_of(text, m.start(3))}", "a Rust block with a line outside its indent names Avro: re-spell it by hand")
            elif indent:
                stripped = "\n".join(l[len(indent):] for l in lines)
                new = rewriter.rust(stripped, outside, where)
                body = "\n".join((indent + l) if l else l for l in new.split("\n"))
            else:
                body = rewriter.rust(body, outside, where)
        elif AVRO_NAMED.search(body):
            body = rewriter.inline(body, outside, where)
        out.append(body)
        pos = m.end(3)
    rest = text[pos:]
    out.append(rewriter.inline(rest, outside, where) if AVRO_NAMED.search(rest) else rest)
    return "".join(out)


def markdown_unalias(text: str) -> str:
    def block(m: re.Match) -> str:
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or "yggdryl_avro as avro" not in body:
            return m.group(0)
        if indent:
            stripped = "\n".join(l[len(indent):] if l.startswith(indent) else l for l in body.split("\n"))
            new = unalias(stripped)
            body = "\n".join((indent + l) if l else l for l in new.split("\n"))
        else:
            body = unalias(body)
        return f"{indent}```{info}\n{body}{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text)


def finish_crate_text(root: pathlib.Path) -> None:
    """What the rewrite must not read as the core's: the crate's implementer,
    written past it, and its own harness's sentences about itself."""
    write(root / CRATE_DIR / "src/implementer.rs", AVRO_IMPLEMENTER)
    for path in sorted((root / CRATE_DIR / "tests").glob("avro*")) + sorted((root / CRATE_DIR / "tests/avro").glob("*.rs")):
        if path.is_dir():
            continue
        text = read(path)
        new = text.replace("`yggdryl::internals`", "`yggdryl_avro::internals`")
        if path.name == "avro.rs" and path.parent.name == "tests":
            new = re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", AVRO_HARNESS_DOC, new, count=1)
        if new != text:
            write(path, new)


AVRO_HARNESS_DOC = """//! One test file per file under `rust/avro/src/`, under `tests/avro/`.
//!
//! `rust/avro/tests/` mirrors `rust/avro/src/`: a source file has exactly one
//! test file at the matching path, and this target is the harness for the
//! crate's own files. A test reaches the crate through `yggdryl_avro::`; where
//! what it pins is not reachable that way, it reaches
//! `yggdryl_avro::internals`, which exists only under the `internals` feature,
//! and the file that reaches it is declared behind that feature here.
"""


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
    """The claim rule's two refusals, pinned in the core over test-only media."""
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


# ---------------------------------------------------------------------------
# Step 7: the exchange
# ---------------------------------------------------------------------------

INTEROP_HARNESS = """//! The Avro exchange with outside implementations: fastavro, and the
//! `apache-avro` crate where `scripts/check_avro_interop.py`'s probe builds.
//! The driver runs this target twice around its round trips and refuses the
//! word `SKIPPED`, so a skipped half never reads as a pass.

#[path = "interop/avro.rs"]
mod avro;
"""


def edit_interop(root: pathlib.Path) -> None:
    write(root / CRATE_DIR / "tests/interop.rs", INTEROP_HARNESS)
    edit(
        root, f"{CRATE_DIR}/tests/interop/avro.rs",
        ("    let mut path = std::env::current_dir().expect(\"a working directory\");\n"
         "    // Under `cargo test` the working directory is `rust/`.\n"
         "    path.push(\"target\");\n",
         "    let mut path = std::env::current_dir().expect(\"a working directory\");\n"
         "    // Under `cargo test` the working directory is the crate's, `rust/avro/`;\n"
         "    // the exchange lives beside the driver's, under `rust/target/`.\n"
         "    path.push(\"..\");\n"
         "    path.push(\"target\");\n"),
    )
    edit(
        root, "scripts/check_avro_interop.py",
        ("1. ``cargo test --test interop avro::`` writes ``target/avro-interop/from-rust.avro``.",
         "1. ``cargo test -p yggdryl-avro --test interop avro::`` writes\n   ``target/avro-interop/from-rust.avro``."),
        ('            "--locked",\n            "--test",\n            "interop",\n',
         '            "--locked",\n            "-p",\n            "yggdryl-avro",\n            "--test",\n            "interop",\n'),
    )


# ---------------------------------------------------------------------------
# Step 8: install
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = """//! The claim every harness of `yggdryl-avro` makes before a test reads a
//! name: the core's register answers the Avro medium only once
//! `yggdryl-avro` has claimed it (D7), so each test opens with [`installed`].

/// Claims the Avro medium, once for the process.
pub fn installed() {
    yggdryl_avro::install().expect("yggdryl-avro claims its medium");
}
"""

INSTALL_TESTS = """
/// `install()` claims the medium under its own name at the rank the core held
/// it at (D39), once: a second call returns at once, and the claim stands
/// against another crate's.
#[test]
fn install_claims_avro_at_its_reserved_rank_once() {
    yggdryl_avro::install().expect("the first install claims the medium");
    yggdryl_avro::install().expect("a later install returns at once");
    let held = yggdryl::media::codec_for(&MimeType::AVRO).expect("the medium is claimed");
    assert_eq!((held.name(), held.title(), held.rank()), ("avro", "Avro", 2));
    assert!(std::ptr::addr_eq(
        held,
        &yggdryl_avro::AVRO_CODEC as &dyn yggdryl::media::MediaCodec
    ));
    let refused = yggdryl::media::codec::claim(&yggdryl_avro::AVRO_CODEC, "another")
        .expect_err("a second claim of the medium is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-avro"), "{refused}");
}
"""


def install_everywhere(root: pathlib.Path) -> None:
    base = root / CRATE_DIR
    write(base / "tests/support/install.rs", INSTALL_SUPPORT)
    harnesses = tests = 0
    for path in sorted((base / "tests").glob("*.rs")):
        text = s4.declare_install(read(path))
        write(path, re.sub(r'\n{3,}(#\[path = "support/install\.rs"\])', r"\n\n\1", text, count=1))
        harnesses += 1
    mod_ = base / "tests/avro/mod_.rs"
    if mod_.exists() and "install_claims_avro_at_its_reserved_rank_once" not in read(mod_):
        text = read(mod_).rstrip("\n") + "\n" + INSTALL_TESTS
        if not re.search(r"(?m)^use yggdryl::\{[^}]*\bMimeType\b", text) and "use yggdryl::MimeType;" not in text:
            residue(f"{CRATE_DIR}/tests/avro/mod_.rs", "the install test names `MimeType`, which the file does not import at its top")
        write(mod_, text)
    for path in sorted((base / "tests").rglob("*.rs")):
        if "/support/" in str(path):
            continue
        text, n = s4.insert_test_installs(read(path))
        tests += n
        write(path, text)
    write(base / "benchmarks/avro.rs", AVRO_BENCH_ROOT)
    pages = 0
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text, n = markdown_installs(read(root / f))
            if n:
                write(root / f, text)
                pages += n
    install_at_init(root)
    done(f"install: {harnesses} harnesses, {tests} tests, the bench main, {pages} page blocks, the bindings and the CLI")


def markdown_installs(text: str) -> tuple[str, int]:
    """A Rust block naming the crate, or reading a handle whose name declares
    Avro through the register, installs it first."""
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or "yggdryl_avro::install()" in body:
            return m.group(0)
        if not (re.search(r"\byggdryl_avro\b", body) or "MimeType::AVRO" in body
                or any(literal_reads_avro(l.group(0)) for l in s4.STRING_LITERAL.finditer(body))):
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
    rel = "python/src/lib.rs"
    text = read(root / rel)
    if "yggdryl_avro::install()" not in text:
        if "    yggdryl_fix::install().map_err(value_error)?;\n" in text:
            text = replace_once(text, "    yggdryl_fix::install().map_err(value_error)?;\n",
                                "    yggdryl_fix::install().map_err(value_error)?;\n"
                                "    yggdryl_avro::install().map_err(value_error)?;\n", rel)
        else:
            text = replace_once(
                text, "    logging::install(module.py())?;\n",
                "    logging::install(module.py())?;\n"
                "    // The crates split off the core claim what they register before a\n"
                "    // class can read a name of theirs, in dependency order.\n"
                "    yggdryl_avro::install().map_err(value_error)?;\n", rel)
        write(root / rel, text)
    rel = "node/src/lib.rs"
    text = read(root / rel)
    if "yggdryl_avro::install()" not in text:
        fix_line = '    yggdryl_fix::install().expect("yggdryl-fix claims its names");\n'
        if fix_line in text:
            text = replace_once(text, fix_line, fix_line + '    yggdryl_avro::install().expect("yggdryl-avro claims its medium");\n', rel)
        else:
            text = replace_once(
                text, "fn install_logging() {\n    logging::install();\n}",
                "fn install_logging() {\n    logging::install();\n"
                "    // The crates split off the core claim what they register before an\n"
                "    // export can read a name of theirs, in dependency order; a refusal is\n"
                "    // a build linking two claimants, which no caller can repair.\n"
                '    yggdryl_avro::install().expect("yggdryl-avro claims its medium");\n}', rel)
        write(root / rel, text)
    rel = "cli/src/main.rs"
    text = read(root / rel)
    if "yggdryl_avro::install()" not in text:
        chain = "yggdryl_market::install().and_then(|()| yggdryl_fix::install())"
        if chain in text:
            text = replace_once(text, chain, chain + ".and_then(|()| yggdryl_avro::install())", rel)
        else:
            text = replace_once(
                text, "    warnings::install();\n",
                "    warnings::install();\n"
                "    // The crates split off the core claim what they register before an\n"
                "    // argument can name a medium of theirs, in dependency order.\n"
                "    if let Err(refusal) = yggdryl_avro::install() {\n"
                "        style::bad(&refusal.to_string());\n"
                "        return ExitCode::FAILURE;\n"
                "    }\n", rel)
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
    skip = {"yggdryl", "yggdryl_avro", "crate", "self", "super", "std", "core", "alloc"}
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
    for f in ("yggdryl_market", "yggdryl_fix"):
        if f in used_dev:
            dev_lines.append(f"{f.replace('_', '-')}.workspace = true")
    # The core's gates the crate's tests and benches read, forwarded.
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
    feature_text = "\n".join(f"{name} = {toml_value(value)}" for name, value in features)
    manifest = (
        "[package]\n"
        f'name = "{PACKAGE}"\n'
        'description = "Apache Avro for yggdryl: schemas, datums, object containers, schema resolution and single-object framing, and the Avro record medium over the core\'s handles and Arrow"\n'
        "version.workspace = true\n"
        "edition.workspace = true\n"
        "rust-version.workspace = true\n"
        "license.workspace = true\n"
        "repository.workspace = true\n"
        'readme = "README.md"\n'
        'keywords = ["avro", "arrow", "codec", "schema", "serialization"]\n'
        'categories = ["encoding", "parser-implementations"]\n'
        "\n[features]\n"
        "# The core's gates this crate's tests and benchmarks read, forwarded; the\n"
        "# crate's own code reads none - snappy blocks and a header's raw bytes are\n"
        "# the crate's whatever the core builds (D16). `internals` makes\n"
        "# `yggdryl_avro::internals` exist for `tests/` and turns the core's on.\n"
        + feature_text + "\n"
        "\n[dependencies]\n" + "\n".join(lines) + "\n"
        "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
        "\n# The raw codec's groups and the Avro rows of the core's shared record\n"
        "# benchmarks; `main` claims the medium first.\n"
        '[[bench]]\nname = "avro"\npath = "benchmarks/avro.rs"\nharness = false\n'
    )
    write(root / CRATE_DIR / "Cargo.toml", manifest)
    readme = root / CRATE_DIR / "README.md"
    if not readme.exists():
        write(
            readme,
            f"# {PACKAGE}\n\nApache Avro for yggdryl: schemas, datums, object containers, schema "
            "resolution and single-object framing, and the Avro record medium over the core's "
            "handles and Arrow.\n\n"
            "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the "
            "project's site, and `install()` claims the medium on the core's register.\n",
        )
    # The workspace: members and the one pinned version.
    ws = read(root / "Cargo.toml")
    m = re.search(r"members = \[([^\]]*)\]", ws)
    members = [s.strip().strip('"') for s in m.group(1).split(",") if s.strip()]
    if "rust/avro" not in members:
        at = max([i for i, s in enumerate(members) if s == "rust" or s.startswith("rust/")] or [0]) + 1
        members.insert(at, "rust/avro")
        ws = ws[:m.start(1)] + ", ".join(f'"{s}"' for s in members) + ws[m.end(1):]
    if f"{PACKAGE} = {{" not in ws:
        if re.search(r"(?m)^yggdryl = \{ path = \"rust\"", ws):
            tail = list(re.finditer(r'(?m)^yggdryl(?:-[a-z]+)? = \{ path = "rust[^\n]*\n', ws))[-1]
            ws = ws[:tail.end()] + f'{PACKAGE} = {{ path = "rust/avro", version = "={version}" }}\n' + ws[tail.end():]
        else:
            ws = replace_once(
                ws,
                "[workspace.dependencies]\n",
                "[workspace.dependencies]\n"
                "# The workspace's own crates, pinned to the one version every artifact\n"
                "# carries, so a published crate names exactly the core it was built with.\n"
                f'yggdryl = {{ path = "rust", version = "={version}" }}\n'
                f'{PACKAGE} = {{ path = "rust/avro", version = "={version}" }}\n',
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
    done("manifests: rust/avro, the workspace, the core, the bindings and the CLI")


# ---------------------------------------------------------------------------
# Step 10: tooling
# ---------------------------------------------------------------------------


def edit_tooling(root: pathlib.Path) -> None:
    """Where S4 has not landed, its per-crate edits of the two inventory and
    internals tools, read off `s4_move.py` itself."""
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
            write(path, text[:start] + ast.literal_eval(m.group(1)) + text[end:])
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


# ---------------------------------------------------------------------------
# Step 11: CI
# ---------------------------------------------------------------------------


def edit_ci(root: pathlib.Path) -> None:
    rel = ".github/ci/rows.toml"
    if not (root / rel).exists():
        residue(rel, "absent in this tree: list the `avro` leaf by hand")
        return
    edit(
        root, rel,
        ('# avro = { package = "yggdryl-avro", jobs = ["avro-interop"] }\n',
         'avro = { package = "yggdryl-avro", jobs = ["avro-interop"] }\n'),
        ("# One `interop` target holds every exchange's Rust half, so a change to any\n"
         "# of them can break the build of all seven.\n",
         "# One `interop` target holds the core's exchanges' Rust half, so a change to\n"
         "# any of them can break the build of all six; a leaf's exchange is its own\n"
         "# crate's target, read through the leaf's row.\n"),
        ('[rows.x-avro]\nextends = ["interop"]\n', '[rows.x-avro]\nextends = ["crate-avro"]\n'),
    )
    edit(
        root, "scripts/tests/test_ci_plan.py",
        ('        self.assertTrue({"zip-interop", "avro-interop", "excel-interop", "object-interop",\n'
         '                         "azure-interop", "gcs-interop", "pyiceberg-interop"} <= jobs)\n',
         '        self.assertTrue({"zip-interop", "excel-interop", "object-interop",\n'
         '                         "azure-interop", "gcs-interop", "pyiceberg-interop"} <= jobs)\n'
         "        # The Avro exchange's Rust half is `yggdryl-avro`'s own target.\n"
         '        self.assertNotIn("avro-interop", jobs)\n'),
    )
    edit(
        root, ".github/workflows/ci.yml",
        ("    # every default-feature test target links and the `interop` target the\n"
         "    # ZIP, Avro and Excel drivers run, and hands both on as `lane-default`:\n",
         "    # every default-feature test target links and the `interop` target the\n"
         "    # ZIP and Excel drivers run, and hands both on as `lane-default`:\n"),
        ("    # `apache-avro` crate. Same shape as the ZIP job and the same reason:\n"
         "    # `rust/tests/interop/avro.rs` is ungated, so its reading half has been\n",
         "    # `apache-avro` crate. Same shape as the ZIP job and the same reason:\n"
         "    # `rust/avro/tests/interop/avro.rs` is ungated, so its reading half has been\n"),
        optional=False,
    )


# ---------------------------------------------------------------------------
# Step 12: docs, skills, AGENTS.md, READMEs, inventory
# ---------------------------------------------------------------------------


def edit_docs(root: pathlib.Path) -> None:
    edit(
        root, "docs/media/avro.md",
        ("| Build | default; the record surface rides Arrow, and the `snappy` block codec needs the `parquet` feature |",
         "| Build | the `yggdryl-avro` crate, which every binding and the `yggdryl` command link and install; the record surface rides Arrow, and every block codec, `snappy` included, is built in |"),
        ("| Rust | `yggdryl_avro`: `Avro<H>` over any handle with `AvroOptions`, `AVRO_CODEC` the [registered medium](index.md#registering-a-medium)",
         "| Rust | `yggdryl_avro`, whose `install()` claims the medium at its reserved rank: `Avro<H>` over any handle with `AvroOptions`, `AVRO_CODEC` the [registered medium](index.md#registering-a-medium)"),
        ('cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/avro\n'
         'cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro\n',
         "cargo bench -p yggdryl-avro --bench avro -- io_dimensions/avro\n"
         "cargo bench -p yggdryl-avro --bench avro -- io_write_stateful/avro\n"),
        ('cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro\n',
         "cargo bench -p yggdryl-avro --bench avro -- codec/avro\n"),
    )
    edit(
        root, "docs/media/index.md",
        ("the core claims its own - Arrow IPC, Parquet, Avro, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`",
         "the core claims its own - Arrow IPC, Parquet, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()`, as `yggdryl-avro` claims Avro"),
        ("a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's seven hold - are refused at `$.encoding`.",
         "a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's media hold - are refused at `$.encoding`, but for `RESERVED_RANKS`: the ranks the media split off the core keep, `parquet` 1, `avro` 2, `xmla` 4 and `excel` 6, each claimed under its own name and no other, so an options value hashes and sorts as it did when the core held its medium."),
        ("The core claims its own seven media before the register answers anything, until they move to the crates that hold them.",
         "The core claims its own six media before the register answers anything, until they move to the crates that hold them; `yggdryl_avro::install()` claims Avro, and every binding and the `yggdryl` command call it at load."),
        ("use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RecordOptions};",
         "use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RESERVED_RANKS, RecordOptions};"),
        ("// The core's ranks and names are never another crate's.\nassert_eq!(EXTERNAL_RANK, 32);\n",
         "// The core's ranks and names are never another crate's; a medium split off\n"
         "// the core keeps its rank under its own name.\nassert_eq!(EXTERNAL_RANK, 32);\n"
         'assert_eq!(RESERVED_RANKS, [("parquet", 1), ("avro", 2), ("xmla", 4), ("excel", 6)]);\n'),
    )
    edit(
        root, "skills/yggdryl-records/references/formats.md",
        ("| default (`snappy` blocks need `parquet`) |", "| `yggdryl-avro`, linked by every binding (every block codec built in) |"),
    )
    edit(
        root, "skills/yggdryl-records/SKILL.md",
        ("| `avro::read_container_resolved(&h, &schema)?` |", "| `yggdryl_avro::read_container_resolved(&h, &schema)?` |"),
    )
    edit(
        root, "docs/contributing.md",
        ("and one root folder per medium: `rust/src/ipc/`, `parquet/`, `avro/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/` |",
         "and one root folder per medium: `rust/src/ipc/`, `parquet/`, `csv/`, `iceberg/`, `text/`, `xmla/`, `excel/`; and `rust/avro/`, the `yggdryl-avro` crate |"),
    )
    edit(root, "rust/README.md", ("src/{ipc,parquet,avro}/\n", "src/{ipc,parquet}/\n"))
    edit(
        root, "README.md",
        ("  src/{ipc,parquet,avro,csv,iceberg,xmla,excel}/\n", "  src/{ipc,parquet,csv,iceberg,xmla,excel}/\n"),
        ("  benchmarks/            Criterion targets, grouped by theme\n",
         "  benchmarks/            Criterion targets, grouped by theme\n"
         "  avro/                  yggdryl-avro: the Avro codec and record medium, its\n"
         "                         own src/, tests/ and benchmarks/\n"),
    )
    edit_agents(root)
    done("docs: the Avro page, the media overview, the skills, contributing, the READMEs, AGENTS.md")


def edit_agents(root: pathlib.Path) -> None:
    rel = "AGENTS.md"
    text = read(root / rel)
    pairs = [
        ("| `ipc/`, `parquet/`, `avro/`, `csv/` | one root folder per record medium;",
         "| `ipc/`, `parquet/`, `csv/`; `avro/` in `yggdryl-avro` | one root folder per record medium;"),
        (", `avro/` the `AvroOptions` block codec and sync marker as the struct's own methods |",
         ", `avro/` - in `yggdryl-avro` at `rust/avro/`, its `install()` claiming `AVRO_CODEC` at the reserved rank 2 - the `AvroOptions` block codec and sync marker as the struct's own methods, and the container, cursor and datum codec Iceberg reads through the crate's own hidden `implementer` |"),
        ("the core claims its own seven media and its table format before a register answers anything, until they move to the crates that hold them |",
         "the core claims its own six media and its table format before a register answers anything, until they move to the crates that hold them; `RESERVED_RANKS` keeps the rank of each medium split off it, under its own name |"),
        ("the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`)",
         "the register (`claim`, `codec_for`, `codecs`, `EXTERNAL_RANK`, `RESERVED_RANKS`)"),
        ("Parquet is feature-gated; Avro's scalar codec is unconditional and its record\nsurface uses Arrow;",
         "Parquet is feature-gated; Avro is `yggdryl-avro`, its scalar codec and its\nrecord surface over Arrow;"),
        ("`rust/tests/` mirrors `rust/src/`, file for file: `rust/avro/src/schema.rs` is\npinned by `rust/avro/tests/avro/schema.rs`,",
         "`rust/tests/` mirrors `rust/src/`, file for file: `rust/src/csv/reader.rs` is\npinned by `rust/tests/csv/reader.rs`,"),
        ("no module named after itself: `avro::schema::schema::x` says the name twice, so",
         "no module named after itself: `csv::reader::reader::x` says the name twice, so"),
        ("an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK`,",
         "an `install()` calling `media::codec::claim(&<NAME>_CODEC, \"<crate>\")` at a rank at or above `EXTERNAL_RANK` - a medium the core held at its `RESERVED_RANKS` rank, under its own name -"),
        ("  manifests = core Avro, data = core Parquet, and no Iceberg/Avro/catalog",
         "  manifests = `yggdryl-avro`'s Avro, data = core Parquet, and no Iceberg/Avro/catalog"),
    ]
    for old, new in pairs:
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
        elif new not in text:
            residue(rel, f"anchor matched {text.count(old)} times, not edited: {old[:90]!r}")
    bench = re.search(r"(?m)^cargo bench -p yggdryl --bench <[^>\n]*>\n", text)
    if bench and "cargo bench -p yggdryl-avro --bench avro" not in text:
        text = text[:bench.end()] + "cargo bench -p yggdryl-avro --bench avro\n" + text[bench.end():]
    elif not bench:
        residue(rel, "no `cargo bench -p yggdryl --bench <...>` line to list the crate's target beside")
    write(root / rel, text)


AVRO_LIB_SECTION = """### yggdryl_avro  [rust/avro/src/lib.rs]  (the crate root: the codec, the record medium and the claim; `implementer` and `internals` hidden)
pub fn install() -> yggdryl::Result<()>  (claims AVRO_CODEC under `application/avro` at rank 2, which yggdryl::media::RESERVED_RANKS keeps for it, once for the process; every binding's init and the CLI's `main` call it; a later call returns at once; the register's refusal where another crate claimed the type or the name first)

### yggdryl_avro::implementer  [rust/avro/src/implementer.rs]  (the door yggdryl-iceberg reaches the crate's crate-private items through, as yggdryl::implementer is the core's; `#[doc(hidden)]`, no API: the container's framing and header, the cursor and the datum codec one manifest row decodes under)
pub use container::{BlockCoding, Header, HeaderEntries, MAGIC, SCHEMA_KEY, SYNC_LEN, check_magic, header_entry, parse_header, parse_header_entries}  (raised to `pub` inside the crate-private `container`)
pub use datum::{Cursor, DatumCodec}  (raised inside `datum`: Cursor's new, take, long, bytes and is_exhausted, DatumCodec's names and limits, budget and decode)
pub use schema::Node  (raised inside `schema`, with the record, enum, fixed and decimal types its variants hold)
pub fn schema_names(schema: &Schema) -> &HashMap<SmolStr, Node>  (`Schema::names`: every named type the schema resolved, by fullname)
pub fn schema_node(schema: &Schema) -> &Node  (`Schema::node`: the root node a row decodes against)
pub fn blocks_metadata_bytes<'blocks, H: IOBase + ?Sized>(blocks: &'blocks Blocks<'_, H>) -> &'blocks [(SmolStr, Vec<u8>)]  (`Blocks::metadata_bytes`: the header's metadata entries exactly as written)

"""

IMPLEMENTER_INVENTORY = """  (Media: what the media crates reach - the record doors a wrapper redirects to, the dimensions and the schema it answers through, the cache clock and the plan cache it writes with, and the value helpers one medium's codec reads)
pub use iobase::{append_arrow_reader_default, merge_arrow_reader_default, overwrite_arrow_reader_default_with_field}  (raised inside the crate-private `iobase`: the default record writes a media wrapper redirects to once its options are proven its own)
pub use iomedia::{container_origin, container_row_size, dimension_options, held_arrow_field, own_options}  (raised inside the crate-private `iomedia`: the handle's own options where none were given, a container's origin and row count, the options a dimension is read under, the declared or held root a schema read answers)
pub use media::options::FileThreads  (raised inside the crate-private `media::options`, its bound and `resolve` with it: the threads one file decodes or encodes on, outside every options value's identity)
pub use metadata::sorted_pairs  (raised inside the crate-private `metadata`: map-shaped pairs borrowed in key order)
pub fn cache_now() -> std::time::Instant  (`media::cache::now`: the instant every cache door reads)
pub fn arrow_schema_from_field(field: &Field) -> crate::arrow::Result<arrow_schema::SchemaRef>  (`arrow::arrow_schema_from_field`: the Arrow schema a record root projects to)
pub fn arrow_cast_plan_compile_schema(source: &Schema, target: &Field, options: crate::ArrowCastOptions) -> crate::arrow::Result<crate::ArrowCastPlan>  (`ArrowCastPlan::compile_schema` with no holder deferred)
pub fn arrow_cast_plan_reconcile_batch(plan: &crate::ArrowCastPlan, batch: RecordBatch) -> crate::arrow::Result<RecordBatch>  (`ArrowCastPlan::reconcile_batch`: one batch reconciled to the plan's target as transport)
pub const fn is_text_storage(parameters: crate::StringType) -> bool  (`string::is_text_storage`)
pub fn is_variant_storage(dtype: &arrow_schema::DataType) -> bool  (`variant::is_variant_storage`)
pub fn decimal_parameters<P, S>(precision: P, scale: S) -> std::result::Result<(u8, i8), SmolStr>  (`decimal::decimal_parameters`)
pub fn uuid_bytes(value: &Scalar) -> Option<&[u8]>  (`uuid::uuid_bytes`)
pub fn uuid_parse(value: &[u8]) -> Result<[u8; 16]>  (`uuid::uuid_parse`)
pub fn uuid_text(stored: &[u8; 16]) -> SmolStr  (`uuid::uuid_text`)
pub struct PlanCache<P>  (moved here from `cast`, where raising it would publish it: one compiled plan per distinct source layout, the pointer compared first, then the fields)
  pub const fn new() -> Self
  pub fn get_or_compile(&mut self, fields: &arrow_schema::Fields, compile: impl FnOnce() -> crate::arrow::Result<P>) -> crate::arrow::Result<&P>
"""


def edit_inventory(root: pathlib.Path) -> None:
    rel = ".api-inventory.txt"
    text = read(root / rel)
    if "### yggdryl_avro  [" in text:
        return
    # The moved sections, re-homed by file.
    text = re.sub(r"(?m)^### yggdryl_avro::(batch|container|resolve|schema|single)  \[rust/avro/src/\1\.rs\]",
                  r"### yggdryl_avro::\1  [rust/avro/src/\1.rs]", text)
    text = replace_once(
        text,
        "  pub static AVRO_CODEC: AvroCodec  (claimed by the core under `application/avro`, rank 2, named `avro`, titled `Avro`; options held as RecordOptions::Registered)",
        "  pub static AVRO_CODEC: AvroCodec  (claimed by `install()` under `application/avro` at its reserved rank 2, named `avro`, titled `Avro`; options held as RecordOptions::Registered)",
        rel,
    )
    anchor = text.find("### yggdryl_avro::batch  [")
    if anchor < 0:
        residue(rel, "no `yggdryl_avro::batch` section to place the crate root's beside")
    else:
        text = text[:anchor] + AVRO_LIB_SECTION + text[anchor:]
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
        "the core claims its own six before the register answers anything, until they move to the crates that hold them - yggdryl-avro's install() claims Avro at its reserved rank;",
        rel,
    )
    text = replace_once(text, "a definition moved here, `InstantSequence` and `Staged`;",
                        "a definition moved here, `InstantSequence`, `Staged` and `PlanCache`;", rel)
    # The media crates' additions close the implementer's section.
    start = text.find("### yggdryl::implementer  [")
    end = text.find("\n### ", start + 1) if start >= 0 else -1
    if start < 0 or end < 0:
        residue(rel, "no `yggdryl::implementer` section to list the media additions under")
    else:
        section = text[start:end].rstrip("\n")
        text = text[:start] + section + "\n" + IMPLEMENTER_INVENTORY + text[end:]
    write(root / rel, text)
    done(".api-inventory.txt: the Avro sections re-homed under yggdryl_avro, the crate root, its implementer, the core's additions")


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
            for m in re.finditer(r"\byggdryl::avro\b|\bcrate::avro\b|yggdryl::internals::avro_", text):
                residue(f"{rel}:{line_of(text, m.start())}", f"`{m.group(0)}` is left")
    # The core's Iceberg reaches the crate it cannot depend on (D39's order).
    for rel in ("rust/src/iceberg/manifest.rs",):
        if (root / rel).exists() and "yggdryl_avro" in read(root / rel):
            residue(rel, "the core's `iceberg` feature names `yggdryl_avro`, which the core cannot depend on (a cycle): "
                         "it compiles once Iceberg is `yggdryl-iceberg`; until then every `--all-features` build, "
                         "the bindings (which enable `yggdryl/iceberg`) and the core's Iceberg tests fail - see the report")
    # Core tests and benches still reading Avro through the register.
    for folder in ("rust/tests", "rust/benchmarks"):
        for path in sorted((root / folder).rglob("*.rs")):
            rel = str(path.relative_to(root))
            if rel.startswith(SPLIT_EXCLUDED) and not rel.startswith(("rust/tests/iceberg/", "rust/tests/interop/iceberg.rs")):
                continue
            text = read(path)
            hits = [line_of(text, m.start()) for m in s4.STRING_LITERAL.finditer(text) if literal_reads_avro(m.group(0))]
            hits += [line_of(text, m.start()) for m in re.finditer(r"\bMimeType::AVRO\b|\byggdryl_avro\b", code_only(text))]
            if hits:
                hits = sorted(set(hits))
                shown = ", ".join(str(n) for n in hits)
                residue(rel, f"a core test or bench reads Avro at {len(hits)} line(s) ({shown}): Iceberg's own "
                             "(moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)")
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
    for folder in ("rust/src", "rust/avro/src", "python/src", "node/src", "cli/src"):
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
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s6_avro" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    # Step 0: a tree already moved is left as it is.
    if (root / CRATE_DIR / "Cargo.toml").exists():
        print(f"[s6-avro] {CRATE_DIR}/Cargo.toml exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/avro/mod.rs").exists():
        raise SystemExit("rust/src/avro/mod.rs is missing: this is not the tree S6a moves")
    if git(root, "status", "--porcelain").strip():
        raise SystemExit("the tree has changes; run S6a on a clean tree")
    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)
    import tomllib

    core_manifest = tomllib.loads(read(root / "rust/Cargo.toml"))
    public = avro_public(read(root / "rust/src/avro/mod.rs"))
    internals = {
        m.group(2): m.group(1).split("::", 1)[1].replace("::", "_")
        for m in re.finditer(r"pub use crate::(avro::[\w:]+)::internals as (\w+);", read(root / "rust/src/lib.rs"))
    }
    tables = AvroTables(public, internals)
    load_trait_methods(root)
    moves = plan_moves(root)
    paths = PathMap(moves)

    # Step 2-3 read the core's files where they stand.
    splits = split_tests(root, moves)
    edit_media_register(root)
    benches = carve_benches(root)
    edit_core_media_bench(root)
    edit(root, "rust/tests/interop.rs", ('#[path = "interop/avro.rs"]\nmod avro;\n', ""))
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
    edit_codec(root)
    plan_cache = move_plan_cache(root)
    edit_implementer(root, plan_cache)
    edit_manifest(root)
    edit_core_lib_and_manifest(root)
    done("crate: lib.rs, implementer.rs, the raises for Iceberg, the gates; core: the seed, RESERVED_RANKS, the implementer")

    # Step 6.
    rewriter = s4.Rewriter(tables)
    rewrite_paths(root, rewriter, paths, moves, splits)
    finish_crate_text(root)
    siblings = install_in_sibling_tests(root)
    if siblings:
        done(f"sibling leaf tests reaching Avro install it: {', '.join(siblings)}")

    # Step 7-12.
    edit_interop(root)
    install_everywhere(root)
    crate_manifest(root, core_manifest, version)
    edit_tooling(root)
    edit_ci(root)
    edit_docs(root)
    edit_inventory(root)

    # Step 13.
    scan_residue(root)
    with s4.GitLock(not arguments.no_git_lock):
        git(root, "add", "-A", "--", CRATE_DIR)
    report = ["# S6a residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE] + ["", "## Residue", ""] + (RESIDUE or ["- none"])
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s6-avro] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
