#!/usr/bin/env python3
"""S6: move the Iceberg table format into `yggdryl-iceberg` at `rust/iceberg/`,
with Amazon S3 Tables as its `s3tables` feature.

Usage:
    python3 -I s6_iceberg_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the batch tree of the amended S6 order: the program
branch after S4, excel and xmla, with `s6_avro_move.py`, `s6_parquet_move.py`
and `s6_s3_move.py` run on it in that order (regroup.md, "S6 order amended";
one dependency-closed commit) - and it leaves the tree uncommitted: `git
status` shows the moves as renames and every rewrite as a modification, for
the lane manager's compiler loop (`cargo check --workspace --all-targets
--all-features --keep-going`). Run on a tree where Avro, Parquet or the object
stores are still the core's, it writes the same final form - the crate names
`yggdryl_avro`, `yggdryl_parquet` and `yggdryl_s3` - and the residue says the
tree builds once their moves have run. It runs no cargo command. A second run
on a tree it already moved stops at step 0 and changes nothing.

What it does, in order (each step says what it touched; `--residue` gets
every site it could not rewrite mechanically, `file:line` each):

 1. test split  - before the moves, every core test item and every sibling
                  leaf's test item that reaches Iceberg - a `yggdryl::iceberg`
                  or `yggdryl::s3tables` path, a name imported from one, the
                  crate's `internals`, a positive `iceberg` or `s3tables`
                  gate, an Iceberg view's own vocabulary - moves byte-identical
                  into `rust/iceberg/tests/` at the path it had below its
                  crate's `tests/`, its helpers copied; an item whose body
                  reads both sides of an `iceberg` gate is kept where it was
                  with the absent side and copied to the crate with the
                  present side; every gate is resolved in each half (D39: a
                  test lives in the lowest crate that can name everything it
                  uses; `PrimitiveType` is the core's, D17).
 2. benches     - the Iceberg block of `rust/benchmarks/types/field/value.rs`
                  and the S3 Tables block of `rust/benchmarks/holder/aws.rs`
                  carved into `yggdryl-iceberg`'s `iceberg` bench beside the
                  `media` bench's Iceberg group, the core's `media` bench
                  losing it.
 3. moves       - `git mv` by glob: `rust/src/iceberg/` to `rust/iceberg/src/`
                  (`mod.rs` the crate root), `rust/src/s3tables/` to
                  `rust/iceberg/src/s3tables/`, `rust/src/iceberg/types.rs`
                  to `rust/src/iceberg.rs` (the core keeps `PrimitiveType`,
                  D17), the Iceberg and S3 Tables test targets, the interop
                  half, `medallion_ledger.rs`, `scale_ulbridge.rs`.
 4. crate       - `rust/iceberg/src/lib.rs`: the doc, `#![deny(unsafe_code)]`,
                  `s3tables` a module under its feature, the `IcebergField`
                  view minted by `protocol_field_types!` in `field.rs` (D39),
                  `install()` claiming `ICEBERG_FORMAT`, `HADOOP_FACTORY` and,
                  under `s3tables`, `S3TABLES_FACTORY` and `S3TABLES_LOCATOR`,
                  idempotent (D7), after the crates it reads; the official
                  model's two doors the core held (`PrimitiveType`'s and
                  `Error`'s) as the crate's own free functions.
 5. core        - `pub mod iceberg` the type vocabulary alone,
                  `pub mod s3tables` and the Iceberg `internals` gone; the
                  three seed claims gone; `as_iceberg`/`as_iceberg_mut` and
                  `Metadata::as_iceberg` gone with the `ICEBERG` entry of the
                  well-known protocols (`Scheme::ICEBERG` stays); every
                  `cfg(iceberg|s3tables)` arm made unconditional where the
                  crate reaches it through `implementer`, deleted where
                  nothing does; `yggdryl::implementer` grown by S3's routes;
                  the `parquet`, `iceberg` and `s3tables` features and the
                  `iceberg-official`, `uuid` and `parquet` dependencies gone.
 6. paths       - every `crate::` path of the moved sources by owner, the
                  crate-private methods they call through their forwarders,
                  every `yggdryl::iceberg`/`yggdryl::s3tables` path and `use`
                  tree in tests, benches, bindings, the CLI, docs and skills,
                  `yggdryl::internals::iceberg_*`, file paths, `#[path]`
                  re-anchored; every `as_iceberg` site swept to
                  `IcebergField::new` (a core test reading only the generic
                  view reads it through `protocol(&Scheme::ICEBERG)`).
 7. interop     - `rust/iceberg/tests/interop.rs`, the exchange directory
                  kept at `target/iceberg-interop`, the driver's cargo line
                  `-p yggdryl-iceberg`.
 8. install     - every moved test opens with `crate::install::installed()`,
                  the bench `main` installs, a page's Rust block reaching the
                  crate installs, the bindings' init and the CLI's `main`
                  install it after Avro, Parquet and the object stores.
 9. manifests   - `rust/iceberg/Cargo.toml` and its README, the workspace
                  members and `[workspace.dependencies]`, the core, the
                  bindings (with `s3tables`), the CLI.
10. tooling     - `generate_internals.py` and `check_api_inventory.py` per
                  crate (S4's edits, where S4 has not landed), the generator
                  run, the logging facade's row.
11. CI          - the `iceberg` leaf line, `x-iceberg` reading the crate, the
                  MSRV job and the PyIceberg build re-pointed, the planner
                  test, the workflow's comments.
12. docs        - the Iceberg page's overview, the "claimed by the core until"
                  sentences, AGENTS.md, the skills, the READMEs,
                  `.api-inventory.txt` re-homed under `### yggdryl_iceberg::..`.
13. residue     - what is left for the compiler loop, by file and line.

Helpers: S4's lexer, `use` trees, rewriter, re-anchoring and git lock
(`s4_move.py`), and the avro define's generic ones (`s6_avro_move.py`) - its
import prune that keeps a trait whose methods a file calls, the names a
macro invocation defines, the shared media section of `implementer` and its
raises, the manifest's container reads, the per-crate tooling edits. Both are
imported and never edited; their residue and progress report into this
script's.
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
        "/tmp/s",
    )
)
sys.path.insert(0, str(SCRATCH))
import s4_move as s4  # noqa: E402  (S4's lexer, use trees, rewriter, lock)
import s6_avro_move as av  # noqa: E402  (the avro define's generic helpers)

CORE, ICEBERG, AVRO, PARQUET, S3 = "core", "iceberg", "avro", "parquet", "s3"
PACKAGE = "yggdryl-iceberg"
CRATE = "yggdryl_iceberg"
CRATE_DIR = "rust/iceberg"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-iceberg] {what}", flush=True)


# Both helper modules report into this script's residue; S4's rewriter learns
# every crate this one names.
s4.residue = residue
av.residue = residue
av.done = done
s4.CRATE_NAME.update({ICEBERG: CRATE, AVRO: "yggdryl_avro", PARQUET: "yggdryl_parquet", S3: "yggdryl_s3"})
s4.prune_imports = av.prune_imports
s4.manifest_dir = av.manifest_dir

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, render_use, UseParseError = s4.parse_use, s4.render_use, s4.UseParseError
line_of = s4.line_of
prune_imports = av.prune_imports
tidy_head = av.tidy_head
CORE_FOLDERS = av.CORE_FOLDERS


def replace_once(text: str, old: str, new: str, where: str) -> str:
    return s4.replace_once(text, old, new, where)


def replace_n(text: str, old: str, new: str, count: int, where: str) -> str:
    found = text.count(old)
    if found != count:
        residue(where, f"anchor matched {found} times, expected {count}, not edited: {old.strip()[:90]!r}")
        return text
    return text.replace(old, new)


def edit(root: pathlib.Path, rel: str, *pairs: tuple[str, str], optional: bool = False) -> bool:
    """Exact-string edits of one file, each anchor asserted to match once; an
    edit whose replacement is there already is done (a second run, or a
    sibling define that made it first)."""
    path = root / rel
    if not path.exists():
        if not optional:
            residue(rel, "absent in this tree; its edits were not made")
        return False
    text = read(path)
    for old, new in pairs:
        if text.count(old) == 1:
            text = text.replace(old, new, 1)
        elif new and new in text:
            continue
        elif not optional or old in text:
            residue(rel, f"anchor matched {text.count(old)} times, not edited: {old.strip()[:90]!r}")
    write(path, text)
    return True


def leaf_exists(root: pathlib.Path, name: str) -> bool:
    return (root / "rust" / name / "Cargo.toml").exists()


def sibling_leaves(root: pathlib.Path) -> list[str]:
    """Every leaf crate under `rust/` but this one, by folder name."""
    return [m.parent.name for m in sorted((root / "rust").glob("*/Cargo.toml"))
            if m.parent.name not in CORE_FOLDERS and m.parent.name != ICEBERG]


# ---------------------------------------------------------------------------
# Who owns a path
# ---------------------------------------------------------------------------

# The core's crate-private items the moved sources reach, by the path they
# spell after `crate::`, and the name `yggdryl::implementer` publishes them
# under (S3's routes; the map's section 2, re-read on this tree: P2's
# `compose` and P3's `Site::Opened` are among them). A path one of the
# sibling defines re-points first - Avro's `PlanCache` - reads as it lands.
CORE_ROUTES: dict[tuple[str, ...], str] = {
    ("arrow", "arrow_schema_from_field"): "arrow_schema_from_field",
    ("arrow", "field_from_arrow_schema"): "field_from_arrow_schema",
    ("aws", "Answer"): "Answer",
    ("aws", "credentials", "Refusal"): "Refusal",
    ("aws", "error_code"): "error_code",
    ("aws", "sigv4", "canonical_query"): "canonical_query",
    ("aws", "sigv4", "encode_query_component"): "encode_query_component",
    ("cast", "PlanCache"): "PlanCache",
    ("decimal", "decimal_parameters"): "decimal_parameters",
    ("decimal", "decimal_text"): "decimal_text",
    ("enums", "read_enum_code"): "read_enum_code",
    ("expression", "eval", "EpochPeriod"): "EpochPeriod",
    ("expression", "eval", "TimeBucket"): "TimeBucket",
    ("expression", "eval", "epoch_value"): "epoch_value",
    ("expression", "eval", "order"): "order",
    ("http", "is_unanswered"): "is_unanswered",
    ("integer", "integer_from_text_as"): "integer_from_text_as",
    ("iobase", "oversized"): "oversized",
    ("iobase", "non_empty_arrow_reader"): "non_empty_arrow_reader",
    ("iobase", "prepare_arrow_write_deriving"): "prepare_arrow_write_deriving",
    ("iomedia", "field_under"): "field_under",
    ("iomedia", "own_options"): "own_options",
    ("media", "merge", "absent"): "absent",
    ("media", "merge", "merged"): "merged",
    ("media", "Cadence"): "Cadence",
    ("media_serie", "Scan"): "Scan",
    ("media_serie", "compose"): "compose",
    ("metadata", "PARQUET_FIELD_ID_KEY"): "PARQUET_FIELD_ID_KEY",
    ("metadata", "SORT_BY_KEY"): "SORT_BY_KEY",
    ("metadata", "sorted_pairs"): "sorted_pairs",
    ("metadata", "sorted_values"): "sorted_values",
    ("metadata", "parse_by_list"): "parse_by_list",
    ("metadata", "parse_by_ordering"): "parse_by_ordering",
    ("serie", "Closing"): "PartitionClosing",
    ("serie", "Partitions"): "Partitions",
    ("string", "is_text_storage"): "is_text_storage",
    ("structure", "partition_column_name"): "partition_column_name",
    ("temporal", "per_second"): "per_second",
    ("uuid", "UUID_SPELLINGS"): "UUID_SPELLINGS",
    ("uuid_bytes",): "uuid_bytes",
    ("uuid_parse",): "uuid_parse",
    ("uuid_text",): "uuid_text",
    ("variant", "is_null_value"): "is_null_value",
    ("warehouse", "Site"): "Site",
    ("warehouse", "entry_name"): "entry_name",
    ("warehouse", "extended"): "extended",
    ("warehouse", "implementation_name"): "implementation_name",
    ("warehouse", "path_text"): "path_text",
    ("warehouse", "table_layout"): "table_layout",
}

# The Parquet crate's hidden door: what Iceberg reads of a file's footer and
# encoder, spelled as the Parquet define spells it (where Parquet is still the
# core's, this script writes the same final form).
PARQUET_IMPLEMENTER = {
    "read_batch_reader_with", "overwrite_buffered", "load_metadata", "schema_from_metadata",
    "READ_AHEAD_BATCHES", "WHOLE_READ_BYTES", "WRITE_BUFFER_BYTES",
}
# The Avro modules the Avro crate's root does not publish.
AVRO_PRIVATE_MODULES = {"arrow", "batch", "container", "datum", "resolve", "schema", "single"}
# What stays the core's of `iceberg/`: the type vocabulary (D17).
CORE_ICEBERG = {"PrimitiveType"}


class IcebergTables:
    """`s4.Rewriter`'s resolution, for `iceberg/` and `s3tables/` leaving the
    core together as one crate."""

    def __init__(self, public: set[str], internals: dict[str, str]) -> None:
        self.public = public
        self.internals = internals
        # Inside the crate an Iceberg path is the crate's own module path.
        self.inside = False
        # A file of the crate - its sources, tests and benches - spells the
        # crates it depends on by their final names; every other file's
        # Avro, Parquet and object-store paths are their own moves' to
        # rewrite, and stay as they are here.
        self.crate_file = False
        # The core's public paths, read when the paths are rewritten.
        self.core: CorePublic | None = None

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        head, rest = segs[0], segs[1:]
        if head == "iceberg":
            if rest[:1] == ["types"]:
                rest = rest[1:]
            if rest and rest[0] in CORE_ICEBERG:
                return CORE, ["iceberg", *rest]
            if not self.inside and rest and rest[0] not in self.public:
                residue("paths", f"`iceberg::{'::'.join(rest)}` names nothing the crate root publishes")
            return ICEBERG, rest
        if head == "s3tables":
            return ICEBERG, segs
        if head == "internals" and rest and rest[0] in self.internals:
            return ICEBERG, ["internals", self.internals[rest[0]], *rest[1:]]
        if head in ("avro", "parquet", "s3") and not self.crate_file:
            return CORE, segs
        if head == "avro":
            if rest and rest[0] in AVRO_PRIVATE_MODULES and len(rest) > 1:
                return AVRO, ["implementer", *rest[1:]]
            return AVRO, rest
        if head == "parquet":
            if rest and rest[0] in PARQUET_IMPLEMENTER:
                return PARQUET, ["implementer", *rest]
            return PARQUET, rest
        if head == "s3":
            return S3, rest
        for cut in range(len(segs), 0, -1):
            name = CORE_ROUTES.get(tuple(segs[:cut]))
            if name:
                return CORE, ["implementer", name, *segs[cut:]]
        if self.inside and self.core is not None and segs[0] not in ("implementer", "internals"):
            return CORE, self.core.route(segs)
        return CORE, segs


class CorePublic:
    """Where a core item the moved sources spelled by its module path is
    public: the path itself where every module on it and the item are
    `pub`; else the name `yggdryl::implementer` publishes it under (read off
    the implementer as it stands when the paths are rewritten), else its
    crate-root re-export. What no public path reaches is residue."""

    ITEM = r"(?m)^pub (?:const |unsafe |async )*(?:fn|struct|enum|trait|type|const|static|mod|union) {name}\b"

    def __init__(self, root: pathlib.Path) -> None:
        self.src = root / "rust/src"
        self.cache: dict[tuple[str, ...], list[str] | None] = {}
        implementer = self.src / "implementer.rs"
        self.routed = published_names(read(implementer)) if implementer.exists() else set()
        lib = read(self.src / "lib.rs")
        self.root_names: set[str] = set()
        for m in re.finditer(r"(?m)^pub use [^;]*;", lib):
            tree = re.sub(r"\s+", " ", m.group(0))
            for leaf in re.findall(r"(\w+)(?: as (\w+))?\s*[,};]", tree):
                self.root_names.add(leaf[1] or leaf[0])

    @staticmethod
    def module_file(folder: pathlib.Path, name: str) -> pathlib.Path | None:
        for candidate in (folder / f"{name}.rs", folder / name / "mod.rs"):
            if candidate.exists():
                return candidate
        return None

    def public_item(self, text: str, name: str) -> bool:
        if re.search(self.ITEM.format(name=re.escape(name)), text):
            return True
        for m in re.finditer(r"(?m)^pub use ([^;]*);", text):
            tree = re.sub(r"\s+", " ", m.group(1))
            if tree.endswith("*") or re.search(rf"(?:\b{re.escape(name)}\s*[,}}]|::{re.escape(name)}$|^{re.escape(name)}$| as {re.escape(name)}\b)", tree):
                return True
        return False

    def route(self, segs: list[str]) -> list[str]:
        key = tuple(segs)
        if key in self.cache:
            found = self.cache[key]
            return list(found) if found is not None else segs
        declaring, folder, public = self.src / "lib.rs", self.src, True
        index = 0
        while index < len(segs):
            text = read(declaring)
            d = re.search(rf"(?m)^(pub(?:\([^)]*\))?\s+)?mod {re.escape(segs[index])};", text)
            if not d:
                break
            if (d.group(1) or "").strip() != "pub":
                public = False
            child = self.module_file(folder, segs[index])
            if child is None:
                break
            declaring, folder = child, folder / segs[index]
            index += 1
        if index >= len(segs):
            self.cache[key] = None
            return segs
        name, rest = segs[index], segs[index + 1:]
        if public and self.public_item(read(declaring), name):
            self.cache[key] = None
            return segs
        # A module the root re-exports whole spells its items through a
        # macro the literal search does not read: public all the same.
        if public and index == 1 and re.search(rf"(?m)^pub use {re.escape(segs[0])}::\*;", read(self.src / "lib.rs")):
            self.cache[key] = None
            return segs
        if name in self.routed:
            found = ["implementer", name, *rest]
        elif name in self.root_names:
            found = [name, *rest]
        else:
            residue("paths", f"`crate::{'::'.join(segs)}` names a core item no public path reaches; "
                             "add it to `yggdryl::implementer`")
            found = None
        self.cache[key] = found
        return list(found) if found is not None else segs


class Rewriter(s4.Rewriter):
    """S4's rewriter over this crate's resolution."""


def iceberg_public(lib: str, s3tables: str | None) -> set[str]:
    """The names the Iceberg module publishes at its root, less what stays the
    core's, with the crate's own additions."""
    names = set(re.findall(r"(?m)^pub (?:struct|enum|fn|static|const|type|trait|mod) (\w+)", lib))
    for m in re.finditer(r"(?m)^pub use \w+::(\{[^;]*\}|\w+);", lib):
        names.update(s4.Tables.brace_names(m.group(1)) if m.group(1).startswith("{") else [m.group(1)])
    names -= CORE_ICEBERG
    # A module the inventory and the docs name by its path, public or not.
    names |= set(re.findall(r"(?m)^(?:pub(?:\([^)]*\))? )?mod (\w+);", lib)) - {"types"}
    return names | {"install", "internals", "s3tables", "IcebergField", "IcebergFieldMut"}


def iceberg_internals(core_lib: str) -> dict[str, str]:
    """`yggdryl::internals::iceberg_<module>` to the crate's `internals::<module>`."""
    found = {}
    for m in re.finditer(r"pub use crate::iceberg::([\w:]+)::internals as (\w+);", core_lib):
        found[m.group(2)] = m.group(1).replace("::", "_")
    for m in re.finditer(r"pub use crate::s3tables::([\w:]+)::internals as (\w+);", core_lib):
        found[m.group(2)] = "s3tables_" + m.group(1).replace("::", "_")
    return found


# ---------------------------------------------------------------------------
# The `iceberg` and `s3tables` gates: what the crate resolves, what the core
# keeps
# ---------------------------------------------------------------------------

FEATURES = ("iceberg", "s3tables")
GATE_TERM = re.compile(r'feature\s*=\s*"(iceberg|s3tables)"')


def no_comments(text: str) -> str:
    """`text` with comments blanked and string literals kept: a `cfg` gate's
    feature name is a literal (the Parquet define's rule)."""
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


def dedent(text: str, width: int = 4) -> str:
    prefix = " " * width
    return "\n".join(line[width:] if line.startswith(prefix) else line.lstrip(" ") if not line.strip() else line
                     for line in text.split("\n"))


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
            rest = re.match(r"[ \t]*[;,]", code[i:])
            if rest:
                i += rest.end()
            break
        if c in ";,":
            i += 1
            break
        i += 1
    nl = text.find("\n", i)
    return len(text) if nl < 0 else nl + 1


class Gates:
    """What a half knows of the two features: `True` there, `False` never
    there, `None` the half's own feature, left as written."""

    def __init__(self, iceberg: bool | None, s3tables: bool | None) -> None:
        self.state = {"iceberg": iceberg, "s3tables": s3tables}

    def term(self, term: str) -> bool | None:
        """The value of one cfg term, or `None` where it is not decided here."""
        term = term.strip()
        m = re.fullmatch(r'feature\s*=\s*"(\w+)"', term)
        if m:
            return self.state.get(m.group(1))
        m = re.fullmatch(r"not\((.*)\)", term, re.S)
        if m:
            inner = self.term(m.group(1))
            return None if inner is None else not inner
        m = re.fullmatch(r"(all|any)\((.*)\)", term, re.S)
        if m:
            parts = split_terms(m.group(2))
            values = [self.term(p) for p in parts]
            if m.group(1) == "all":
                if any(v is False for v in values):
                    return False
                if all(v is True for v in values):
                    return True
            else:
                if any(v is True for v in values):
                    return True
                if all(v is False for v in values):
                    return False
            return None
        return None

    def simplify(self, term: str) -> str | None:
        """The term with every decided part taken out, or `None` where it is
        decided whole (the caller reads `term()` then)."""
        term = term.strip()
        if self.term(term) is not None:
            return None
        m = re.fullmatch(r"(all|any)\((.*)\)", term, re.S)
        if not m:
            m2 = re.fullmatch(r"not\((.*)\)", term, re.S)
            if m2:
                inner = self.simplify(m2.group(1))
                return term if inner is None else f"not({inner})"
            return term
        kept = []
        for part in split_terms(m.group(2)):
            value = self.term(part)
            if value is None:
                kept.append(self.simplify(part) or part.strip())
            # A decided part of `all` that holds, or of `any` that does not,
            # says nothing more.
        if len(kept) == 1:
            return kept[0]
        return f"{m.group(1)}({', '.join(kept)})"


def split_terms(text: str) -> list[str]:
    parts, depth, start = [], 0, 0
    for i, c in enumerate(text):
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        elif c == "," and depth == 0:
            parts.append(text[start:i])
            start = i + 1
    if text[start:].strip():
        parts.append(text[start:])
    return [p.strip() for p in parts if p.strip()]


CFG_LINE = re.compile(r"^([ \t]*)#(!?)\[cfg\((.*)\)\][ \t]*$")
CFG_ATTR_LINE = re.compile(r"^([ \t]*)#\[cfg_attr\((.*)\)\][ \t]*$")
IF_CFG = re.compile(r'\bif (!?)cfg!\(feature = "(iceberg|s3tables)"\) \{')


def resolve_gates(text: str, where: str, gates: Gates) -> str:
    """`text`'s `iceberg` and `s3tables` gates resolved for one half: an
    attribute that holds dropped, the statement or item one that never holds
    gates dropped with the documentation above it, a `cfg!` branch chosen."""
    if not GATE_TERM.search(no_comments(text)):
        return text
    lines = text.split("\n")
    result: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        m = CFG_LINE.match(line)
        if m and GATE_TERM.search(m.group(3)):
            value = gates.term(m.group(3))
            if value is True:
                i += 1
                continue
            if value is False:
                if m.group(2) == "!":
                    residue(where, "a whole file gated off in this half; it should have moved")
                    result.append(line)
                    i += 1
                    continue
                while result and re.match(r"[ \t]*(?:///|#\[)", result[-1]):
                    result.pop()
                rest = "\n".join(lines[i + 1:])
                # The statement gated may itself open with attributes.
                skip_attrs = re.match(r"(?:[ \t]*#\[[^\n]*\]\n)*", rest)
                end = statement_end(rest, skip_attrs.end())
                skip = rest[:end].count("\n") if end > 0 else 0
                i += 1 + skip
                continue
            simplified = gates.simplify(m.group(3))
            result.append(f"{m.group(1)}#{m.group(2)}[cfg({simplified})]")
            i += 1
            continue
        m = CFG_ATTR_LINE.match(line)
        if m and GATE_TERM.search(m.group(2)):
            parts = split_terms(m.group(2))
            value = gates.term(parts[0])
            if value is True:
                result.append(f"{m.group(1)}#[{', '.join(parts[1:])}]")
            elif value is None:
                result.append(line)
            i += 1
            continue
        result.append(line)
        i += 1
    out = "\n".join(result)
    # `if cfg!(..) { .. } else { .. }` and `if !cfg!(..) { .. }`.
    guard = 0
    while True:
        guard += 1
        code = no_comments(out)
        m = next((mm for mm in IF_CFG.finditer(code) if gates.state[mm.group(2)] is not None), None)
        if not m or guard > 200:
            break
        holds = gates.state[m.group(2)]
        if m.group(1) == "!":
            holds = not holds
        open_then = m.end() - 1
        close_then = s4.matching_brace(out, open_then)
        if close_then < 0:
            residue(where, "an `if cfg!` whose block does not close, left")
            break
        after = re.match(r"\s*else\s*\{", code[close_then + 1:])
        if after and code[close_then + 1 + after.end() - 1] == "{":
            open_else = close_then + 1 + after.end() - 1
            close_else = s4.matching_brace(out, open_else)
        else:
            open_else = close_else = None
        chosen = (open_then, close_then) if holds else ((open_else, close_else) if open_else is not None else None)
        end = close_else if close_else is not None else close_then
        line_start = out.rfind("\n", 0, m.start()) + 1
        statement = out[line_start:m.start()].strip() == ""
        if statement:
            tail = out[end + 1:]
            nl = tail.find("\n")
            rest_of_line = tail[:nl] if nl >= 0 else tail
            if rest_of_line.strip() not in ("", ";"):
                residue(where, "an `if cfg!` statement followed by more code on its line, left")
                break
            stop = end + 1 + (nl + 1 if nl >= 0 else len(tail))
            body = "" if chosen is None else dedent(out[chosen[0] + 1:chosen[1]].strip("\n").rstrip(" \t") + "\n")
            body = body.lstrip("\n")
            out = out[:line_start] + body + out[stop:]
        else:
            if chosen is None:
                residue(where, "an `if cfg!` expression with no `else`, left")
                break
            inner = out[chosen[0] + 1:chosen[1]]
            single = ";" not in code_only(inner).strip().rstrip(";")
            replacement = dedent(inner.strip("\n").rstrip(" \t"), 4).strip() if single else out[chosen[0]:chosen[1] + 1]
            out = out[:m.start()] + replacement + out[end + 1:]
    # A boolean a test compares against the gate.
    for feature in FEATURES:
        value = gates.state[feature]
        if value is None:
            continue
        out = re.sub(
            r'assert_eq!\(\n([ \t]*)([^\n]*),\n[ \t]*cfg!\(feature = "' + feature + r'"\),\n',
            (lambda m: f"assert!(\n{m.group(1)}{m.group(2)},\n") if value
            else (lambda m: f"assert!(\n{m.group(1)}!{m.group(2)},\n"),
            out)
    for m in re.finditer(r'(?:#!?\[cfg[^\n]*|cfg!\()feature\s*=\s*"(iceberg|s3tables)"', no_comments(out)):
        if gates.state[m.group(1)] is not None:
            residue(f"{where}:{line_of(out, m.start())}", f"a `{m.group(1)}` gate is left: resolve it by hand")
    return out


CORE_GATES = Gates(iceberg=False, s3tables=False)
CRATE_GATES = Gates(iceberg=True, s3tables=None)


# ---------------------------------------------------------------------------
# Step 1: the tests that reach Iceberg, item by item (S4's split, D39)
# ---------------------------------------------------------------------------

# What moves whole with the crate (step 3) or stays as the shared fixtures
# the moved targets `#[path]` (D39): never split.
SPLIT_EXCLUDED = (
    "rust/tests/iceberg/", "rust/tests/iceberg.rs", "rust/tests/s3tables/", "rust/tests/s3tables.rs",
    "rust/tests/s3tables_handle.rs", "rust/tests/interop/iceberg.rs", "rust/tests/interop.rs", "rust/tests/medallion_ledger.rs",
    "rust/tests/scale_ulbridge.rs", "rust/tests/support/", "rust/tests/logging/",
)
ICEBERG_PATH = re.compile(
    r"\byggdryl(?:_iceberg\b|::s3tables\b|::internals::(?:iceberg|s3tables)_\w+"
    r"|::iceberg::(?!PrimitiveType\b)[\w{]|::iceberg\b(?!::))"
)
VIEW_METHODS = {
    "schema_id", "identifier_field_ids", "is_unknown", "initial_default", "write_default", "spec_id",
    "partition_source_id", "set_schema_id", "set_identifier_field_ids", "set_unknown",
    "set_initial_default", "set_write_default", "set_spec_id", "set_partition_source_id", "set_transform",
}
VIEW_SPECIFIC = re.compile(
    r"\.\s*(?:" + "|".join(sorted(VIEW_METHODS)) + r")\s*\(|as_iceberg(?:_mut)?\(\)\s*\.\s*transform\s*\("
)


def file_imports(items: list) -> set[str]:
    """The names a level imports from the crate's modules: anything a
    `yggdryl::iceberg` tree names but the core's `PrimitiveType`, a
    `yggdryl::s3tables` tree, the crate itself."""
    names: set[str] = set()
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            if kind != "self" or not leaf:
                continue
            if (leaf[:2] in (["yggdryl", "iceberg"], ["yggdryl", "s3tables"]) or leaf[0] == CRATE) \
                    and leaf[-1] not in CORE_ICEBERG:
                names.add(alias or leaf[-1])
    return names


def head_of(text: str) -> str:
    """The attributes and comments above an item's own first line."""
    lines = text.split("\n")
    head = []
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith(("//", "#[")):
            head.append(line)
            continue
        break
    return "\n".join(head)


def gate_of(text: str) -> bool | None:
    """Whether the item's own `cfg` holds only with Iceberg (`True`), only
    without it (`False`), or says nothing of it."""
    for m in re.finditer(r"#\[cfg\((.*)\)\]", head_of(text)):
        if not GATE_TERM.search(m.group(1)):
            continue
        if CORE_GATES.term(m.group(1)) is False:
            return True
        if CRATE_GATES.term(m.group(1)) is False:
            return False
    return None


def reaches(item, imports: set[str]) -> bool:
    code = code_only(item.text)
    if ICEBERG_PATH.search(no_comments(item.text)):
        return True
    if set(re.findall(r"[A-Za-z_]\w*", code)) & imports:
        return True
    return "as_iceberg" in code and bool(VIEW_SPECIFIC.search(code))


def reads_gate(text: str) -> bool:
    """An item whose body reads the Iceberg side of a gate: a positive `cfg`
    inside it, or a `cfg!` branch."""
    body = text[len(head_of(text)):]
    nc = no_comments(body)
    for m in re.finditer(r"#\[cfg\((.*)\)\]", nc):
        if GATE_TERM.search(m.group(1)) and CORE_GATES.term(m.group(1)) is False:
            return True
    return bool(re.search(r'cfg!\(feature = "(?:iceberg|s3tables)"\)', nc))


def primitive_only(text: str) -> bool:
    """An item gated on Iceberg that names nothing of it but the core's own
    `PrimitiveType` (D17): it stays the core's, its gate dropped."""
    nc = no_comments(text)
    return "PrimitiveType" in nc and not ICEBERG_PATH.search(nc)


MOVE, DUP, STAY = "move", "dup", "stay"


def classify(item, imports: set[str]) -> str:
    gate = gate_of(item.text)
    if gate is True:
        return STAY if primitive_only(item.text) else MOVE
    if gate is False:
        return STAY
    if reaches(item, imports):
        return MOVE
    if reads_gate(item.text):
        return DUP
    return STAY


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
    owner = {id(item): classify(item, imports) for item in body}
    containers = {id(item) for item in body if inline_body(item) is not None}
    changed = True
    while changed:
        changed = False
        for item in body:
            if owner[id(item)] == STAY and any(owner[other] == MOVE for other in refs[id(item)]
                                               if other not in containers):
                owner[id(item)] = MOVE
                changed = True
    return owner, refs


def header_gated(item) -> bool:
    body = inline_body(item)
    return body is not None and gate_of(item.text[:body[0]]) is True


def split_level(text: str, top: bool, inherited: set[str]) -> tuple[str, str | None, int, int]:
    """One level of a test file split: the core's half, the crate's half (or
    `None`), the items moved and the items copied with the gate's other side."""
    head, items = top_items(text)
    imports = inherited | file_imports(items)
    owner, refs = item_owners(items, inherited)
    moved = copied = 0
    dest: dict[int, str] = {}
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    ungated: dict[int, str] = {}
    for item in items:
        if item.kind == "use":
            continue
        role = owner[id(item)]
        body = inline_body(item)
        if body is not None and not header_gated(item) and role != STAY or body is not None and role == DUP:
            start, end = body
            inner_home, inner_dest, n, d = split_level(item.text[start:end], False, imports)
            moved += n
            copied += d
            if inner_dest is None:
                if inner_home != item.text[start:end]:
                    ungated[id(item)] = item.text[:start] + inner_home + item.text[end:]
                continue
            if not any(i.kind != "use" for i in top_items(inner_home)[1]):
                dropped.add(id(item))
            else:
                replaced[id(item)] = item.text[:start] + inner_home + item.text[end:]
            dest[id(item)] = item.text[:start] + inner_dest + item.text[end:]
            continue
        if body is not None and role == STAY:
            start, end = body
            inner_home, inner_dest, n, d = split_level(item.text[start:end], False, imports)
            if inner_dest is not None:
                moved += n
                copied += d
                replaced[id(item)] = item.text[:start] + inner_home + item.text[end:]
                dest[id(item)] = item.text[:start] + inner_dest + item.text[end:]
            elif inner_home != item.text[start:end]:
                ungated[id(item)] = item.text[:start] + inner_home + item.text[end:]
            continue
        if role == MOVE:
            dropped.add(id(item))
            dest[id(item)] = item.text
            moved += len(re.findall(r"#\[test\]", item.text)) if body is not None else 1
        elif role == DUP:
            dest[id(item)] = item.text
            copied += len(re.findall(r"#\[test\]", item.text))
        elif gate_of(item.text) is True:
            # Gated on Iceberg, naming only the core's own vocabulary.
            ungated[id(item)] = resolve_gates(item.text, "split", Gates(True, False))
    if not dest:
        if ungated:
            out = [head]
            for item in items:
                out.append(ungated.get(id(item), item.text))
            return "".join(out).rstrip("\n") + "\n", None, 0, 0
        return text, None, 0, 0
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
        if item.names & names and id(item) not in dest and id(item) not in replaced and id(item) not in containers
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
    crate_text = "".join(out).rstrip("\n") + "\n"
    roots = {
        id(item) for item in body_items
        if id(item) not in dropped
        and (re.search(r"#\[test\]|#\[global_allocator\]", item.text) or inline_body(item) is not None
             or av.published(item.text))
    }
    stay = (closure(roots) - dropped) | set(replaced)
    out = [head]
    for item in items:
        if item.kind == "use":
            out.append(item.text)
        elif id(item) in replaced:
            out.append(replaced[id(item)])
        elif id(item) in stay:
            out.append(ungated.get(id(item), item.text))
    return "".join(out).rstrip("\n") + "\n", crate_text, moved, copied


def pinned_source(rel: str, base: str) -> str | None:
    """The source file a test file pins: below `rust/tests/`, the core's;
    below a leaf's `tests/`, that leaf's."""
    parts = rel.split("/")
    if len(parts) == 1:
        return None
    src = base.rsplit("/tests", 1)[0] + "/src/"
    if parts[0] == "root":
        return src + parts[1]
    if parts[-1] == "mod_.rs":
        parts[-1] = "mod.rs"
    return src + "/".join(parts)


def merge_into(existing: str, incoming: str) -> str:
    """A second half landing on a file the crate already has: its `use`
    statements merged, its items the file does not define appended."""
    head, items = top_items(existing)
    _, new_items = top_items(incoming)
    defined = {n for item in items for n in item.names}
    uses = av.merged_uses([existing, incoming])
    body = [item.text.strip("\n") for item in items if item.kind != "use"]
    body += [item.text.strip("\n") for item in new_items if item.kind != "use" and not (item.names & defined)]
    return head + uses + "\n" + "\n\n".join(body) + "\n"


def split_tests(root: pathlib.Path, moves: list[tuple[str, str]]) -> tuple[list[tuple[str, str]], int, int]:
    """Every core and sibling leaf test file's items reaching Iceberg, moved
    byte-identical into `rust/iceberg/tests/` at the path they had below
    their own `tests/`; an item reading both sides of a gate keeps its absent
    side where it was and takes its present side to the crate."""
    move_targets = set(dict(moves).values())
    move_sources = set(dict(moves))
    written: list[tuple[str, str]] = []
    needs: dict[str, list[tuple[str, str]]] = collections.defaultdict(list)
    total_moved = total_copied = 0
    bases = ["rust/tests"] + [f"rust/{leaf}/tests" for leaf in sibling_leaves(root)]
    for base in bases:
        for f in tracked(root, base):
            if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED) or "/support/" in f or f in move_sources:
                continue
            rel = f[len(base) + 1:]
            single = "/" not in rel
            original = read(root / f)
            home_text, crate_text, moved, copied = split_level(original, True, set())
            if crate_text is None:
                resolved = resolve_gates(home_text, f, CORE_GATES)
                if resolved != original:
                    write(root / f, tidy_head(resolved))
                    done(f"{f}: its Iceberg gates resolved for a build without the crate")
                continue
            total_moved += moved
            total_copied += copied
            dest_rel = f"{CRATE_DIR}/tests/{rel}"
            if dest_rel in move_targets:
                residue(dest_rel, f"arrives by a move; the split tests of `{f}` were not written")
                continue
            pinned = pinned_source(rel, base)
            what = "rows" if single else "tests"
            header = (
                f"//! The {what} of `{f}`"
                + (f", which pins `{pinned}`," if pinned else "")
                + f" that need\n//! `{PACKAGE}`, moved byte-identical (S6, D39): a test lives in the lowest\n"
                "//! crate that can name everything it uses. A test reading both sides of\n"
                "//! an `iceberg` gate is here with the side `yggdryl-iceberg` installed\n"
                f"//! answers, and stays in `{f}` with the other.\n//!\n"
            )
            crate_text = resolve_gates(crate_text, dest_rel, CRATE_GATES)
            # A fixture it names by `#[path]` stays where it was.
            crate_text = s4.reanchor(crate_text, f, dest_rel, PathMap(moves))
            home_text = resolve_gates(home_text, f, CORE_GATES)
            body = tidy_head(prune_imports(prune_imports(header + crate_text)))
            if (root / dest_rel).exists():
                body = merge_into(read(root / dest_rel), body)
            write(root / dest_rel, body)
            written.append((f, dest_rel))
            if not single:
                needs[f"{base}/{rel.split('/', 1)[0]}.rs"].append((f, rel))
            write(root / f, tidy_head(prune_imports(prune_imports(home_text))))
            done(f"{f}: {moved} item(s) moved and {copied} copied to {dest_rel}")
    for harness, modules in sorted(needs.items()):
        if not (root / harness).exists():
            residue(harness, "the harness of a split module is missing")
            continue
        text = read(root / harness)
        head, items = top_items(text)
        wanted: set[str] = set()
        decls = []
        base = harness.rsplit("/", 1)[0]
        for module, rel in modules:
            relpath = module[len(base) + 1:]
            decl = next((item for item in items if item.kind == "mod" and f'"{relpath}"' in item.text), None)
            if decl is None:
                residue(harness, f"no `#[path]` declaration of `{relpath}` to copy into the {PACKAGE} harness")
                continue
            decls.append(id(decl))
            wanted |= set(re.findall(r"[A-Za-z_]\w*", code_only(read(root / CRATE_DIR / "tests" / rel))))
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
        decl_text = [resolve_gates(item.text, dest, CRATE_GATES) for item in items if id(item) in decls]
        # A fixture the harness declares stays where it was; the split modules
        # sit beside the new harness at the paths they had.
        parts += [s4.reanchor(item.text, harness, dest, PathMap(moves)) if item.kind == "mod" else item.text
                  for item in items if item.kind == "use" or id(item) in chosen]
        parts += decl_text
        body = tidy_head(prune_imports("".join(parts).rstrip("\n") + "\n"))
        if (root / dest).exists():
            existing = read(root / dest)
            for piece in decl_text:
                if piece.strip() not in existing:
                    existing = existing.rstrip("\n") + "\n" + piece.rstrip("\n") + "\n"
            body = existing
        elif dest in move_targets:
            residue(dest, f"the harness arrives by a move: declare {', '.join(m for m, _ in modules)} in it by hand")
            continue
        write(root / dest, body)
        written.append((harness, dest))
    done(f"test split: {total_moved} item(s) moved, {total_copied} copied with the gate's other side")
    return written, total_moved, total_copied


# ---------------------------------------------------------------------------
# Step 3: the move table and file paths
# ---------------------------------------------------------------------------

# The core's own targets that leave whole: a target pinning the crate as a
# whole (`medallion_ledger.rs`), a target owning its process over the fake
# control plane (`s3tables_handle.rs`), and the capture pipeline whose tables
# are Iceberg tables (`scale_ulbridge.rs`, wherever S4 left it).
WHOLE_TARGETS = ["s3tables_handle.rs", "medallion_ledger.rs", "scale_ulbridge.rs"]


REMNANT_MARK = "//! The object stores' suite's Iceberg half:"
REMNANT_HARNESS_DOC = """//! What an Iceberg table costs over the object stores' in-process store
//! (`accounting::iceberg` in `s3/mod_.rs`), under the `s3tables` feature that
//! links `yggdryl-s3`: the object stores' suite's Iceberg half, moved here
//! from the core with the table format (D36.7). The backend's own suites are
//! `yggdryl-s3`'s, under `rust/s3/tests/`.
"""
REMNANT_MODULE_DOC = """//! The Iceberg tables over the object stores' in-process store -
//! `accounting::iceberg`, what a table costs per operation - with the
//! fixtures they name, moved from the core's `rust/tests/s3/mod_.rs` with the
//! table format (D36.7); `yggdryl-s3` is what the `s3tables` feature links.
"""


def settle_remnant(root: pathlib.Path) -> None:
    """The moved Iceberg half of the object stores' suite says where it is."""
    harness = root / CRATE_DIR / "tests/s3.rs"
    if harness.exists() and read(harness).startswith(REMNANT_MARK):
        text = read(harness)
        head_end = re.match(r"(?:[ \t]*//![^\n]*\n)*", text).end()
        write(harness, REMNANT_HARNESS_DOC + text[head_end:])
    module = root / CRATE_DIR / "tests/s3/mod_.rs"
    if module.exists():
        text = read(module)
        m = re.match(r"//! The tests of `rust/tests/s3/mod_\.rs` that build an Iceberg table over the\n(?:[ \t]*//![^\n]+\n)*[ \t]*//!\n", text)
        if m:
            write(module, REMNANT_MODULE_DOC + "//!\n" + text[m.end():])


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    moves: list[tuple[str, str]] = []
    for f in tracked(root, "rust/src/iceberg", "rust/src/s3tables"):
        if f == "rust/src/iceberg/types.rs":
            new = "rust/src/iceberg.rs"
        elif f.startswith("rust/src/iceberg/"):
            rel = f[len("rust/src/iceberg/"):]
            new = f"{CRATE_DIR}/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        else:
            new = f"{CRATE_DIR}/src/s3tables/" + f[len("rust/src/s3tables/"):]
        moves.append((f, new))
    for f in tracked(root, "rust/tests/iceberg", "rust/tests/iceberg.rs", "rust/tests/s3tables",
                     "rust/tests/s3tables.rs", "rust/tests/interop/iceberg.rs"):
        moves.append((f, f"{CRATE_DIR}/tests/" + f[len("rust/tests/"):]))
    for name in WHOLE_TARGETS:
        for base in ["rust/tests", *(f"rust/{leaf}/tests" for leaf in sibling_leaves(root))]:
            f = f"{base}/{name}"
            if f in tracked(root, f):
                moves.append((f, f"{CRATE_DIR}/tests/{name}"))
    # What the object stores' move left in the core for this one: its suite's
    # Iceberg half, a harness and its one module (D36.7).
    harness = root / "rust/tests/s3.rs"
    if harness.exists() and read(harness).startswith(REMNANT_MARK):
        moves.append(("rust/tests/s3.rs", f"{CRATE_DIR}/tests/s3.rs"))
        for f in tracked(root, "rust/tests/s3"):
            moves.append((f, f"{CRATE_DIR}/tests/" + f[len("rust/tests/"):]))
    if tracked(root, "rust/benchmarks/media/iceberg.rs"):
        moves.append(("rust/benchmarks/media/iceberg.rs", f"{CRATE_DIR}/benchmarks/iceberg/media.rs"))
    return moves


class PathMap(av.PathMap):
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/iceberg/", f"{CRATE_DIR}/src/"),
            ("rust/src/s3tables/", f"{CRATE_DIR}/src/s3tables/"),
            ("rust/tests/iceberg/", f"{CRATE_DIR}/tests/iceberg/"),
            ("rust/tests/s3tables/", f"{CRATE_DIR}/tests/s3tables/"),
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
    text = re.sub(r"(?<![\w./-])rust/src/iceberg(?![\w/.-])", f"{CRATE_DIR}/src", text)
    text = re.sub(r"(?<![\w./-])rust/src/s3tables(?![\w/.-])", f"{CRATE_DIR}/src/s3tables", text)
    text = re.sub(r"(?<![\w./-])rust/tests/iceberg(?![\w/.-])", f"{CRATE_DIR}/tests/iceberg", text)
    text = re.sub(r"(?<![\w./-])rust/tests/s3tables(?![\w/.-])", f"{CRATE_DIR}/tests/s3tables", text)
    # A command running a moved target runs the crate's.
    text = re.sub(r"(?m)(-p )yggdryl( [^\n]*?--test (?:iceberg|s3tables|s3tables_handle|medallion_ledger|scale_ulbridge)\b)",
                  rf"\1{PACKAGE}\2", text)
    return text


def git_mv(root: pathlib.Path, moves: list[tuple[str, str]]) -> None:
    for old, new in moves:
        (root / new).parent.mkdir(parents=True, exist_ok=True)
        git(root, "mv", old, new)
    done(f"moves: {len(moves)} files moved by git")


# ---------------------------------------------------------------------------
# Step 4: the crate's own sources
# ---------------------------------------------------------------------------

LIB_HEAD = """//! Apache Iceberg tables for the yggdryl core - the `yggdryl-iceberg` crate -
//! with Amazon S3 Tables, the table buckets that keep them, as its
//! `s3tables` feature.
//!
//! A table is one container over the core's [`IOBase`]: metadata documents,
//! Avro manifests through `yggdryl-avro` and Parquet data files through
//! `yggdryl-parquet`. The core reads such a folder through the table format
//! [`ICEBERG_FORMAT`], a catalog of them through [`HADOOP_FACTORY`] and,
//! under `s3tables`, a table bucket's locations and ARNs through its
//! factory and locator - each claimed on the core's registers by
//! [`install`]: every binding's init and the `yggdryl` command's `main` call
//! it, and a Rust program that links this crate calls it before the core
//! reads a table, which is refused naming the crate to install until then.
//! [`IcebergTable`] and every type below answer without it. The type
//! vocabulary - `PrimitiveType`, the Iceberg type strings the datatype
//! grammar reads - is the core's own, `yggdryl::iceberg::PrimitiveType`.
//!
"""

LIB_DOC_EDITS = [
    ("//! Apache Iceberg tables over one [`IOBase`] handle.\n//!\n"
     "//! The optional `iceberg` feature delegates metadata and schema builders and\n"
     "//! manifest/list readers to official Iceberg 0.10.1. That dependency requires\n",
     "//! The crate delegates metadata and schema builders and manifest/list\n"
     "//! readers to official Iceberg 0.10.1. That dependency requires\n"),
]

INSTALL_FN = '''
/// What this crate claims the Iceberg table format, its catalog and the
/// Amazon S3 Tables locations as.
const CRATE: &str = "yggdryl-iceberg";

/// Claims what Iceberg registers on the core, once for the life of the
/// process; a later call returns at once. [`ICEBERG_FORMAT`] is the table
/// format the record doors locate a folder laid out as a table with, and
/// [`HADOOP_FACTORY`] the catalog a `type` of `hadoop` and an `iceberg:`
/// location ask `Catalog::from_url` for; under the `s3tables` feature the
/// table bucket's catalog factory, claimed under the `s3tables` scheme, and
/// the locator of `s3tables:` locations and of the service's ARNs. The
/// crates it reads are installed first - Avro for the manifests, Parquet for
/// the data files and, under `s3tables`, the object stores a warehouse
/// lives in.
/// Every binding's init and the `yggdryl` command's `main` call it, and so
/// does a Rust caller before the core reads a table: until the claim a
/// folder laid out as a table is refused, naming the crate to install.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed the format's
/// name, the factory's `type` word or scheme, or the locator's scheme first.
pub fn install() -> yggdryl::Result<()> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    static INSTALLING: Mutex<()> = Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    yggdryl_avro::install()?;
    yggdryl_parquet::install()?;
    #[cfg(feature = "s3")]
    yggdryl_s3::install()?;
    let _installing = INSTALLING.lock().unwrap_or_else(PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    // Each claim is made where this crate does not hold it yet, so a call
    // after one refused claims what is left rather than refusing what it
    // already holds.
    let format: &dyn TableFormat = &ICEBERG_FORMAT;
    if !yggdryl::media::format::format_named(ICEBERG_FORMAT.name())
        .is_some_and(|held| std::ptr::addr_eq(held, format))
    {
        yggdryl::media::format::claim(&ICEBERG_FORMAT, CRATE)?;
    }
    claim_factory(&HADOOP_FACTORY)?;
    #[cfg(feature = "s3tables")]
    {
        claim_factory(&s3tables::S3TABLES_FACTORY)?;
        let locator: &dyn yggdryl::holder::Locator = &s3tables::S3TABLES_LOCATOR;
        if !yggdryl::holder::locators()
            .into_iter()
            .any(|held| std::ptr::addr_eq(held, locator))
        {
            yggdryl::holder::claim_locator(&s3tables::S3TABLES_LOCATOR, CRATE)?;
        }
    }
    let _ = INSTALLED.set(());
    Ok(())
}

/// Claim `factory` for this crate where it holds it not yet.
fn claim_factory(factory: &'static dyn yggdryl::CatalogFactory) -> yggdryl::Result<()> {
    if yggdryl::warehouse::factories()
        .into_iter()
        .any(|held| std::ptr::addr_eq(held, factory))
    {
        return Ok(());
    }
    yggdryl::warehouse::claim_factory(factory, CRATE)
}
'''

INTERNALS_PLACEHOLDER = "// GENERATED by scripts/generate_internals.py - do not edit by hand.\n// END GENERATED\n"


def build_lib(root: pathlib.Path) -> None:
    """`mod.rs` as the crate root: its doc led by the crate's, `s3tables` a
    module under its feature, the view's types re-exported from `field.rs`,
    `PrimitiveType` left to the core, `install()`."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    path = root / rel
    text = read(path)
    for old, new in LIB_DOC_EDITS:
        text = replace_once(text, old, new, rel)
    m = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if not m:
        residue(rel, "no crate doc to lead")
        return
    doc, body = m.group(0), text[m.end():].lstrip("\n")
    body = replace_once(body, "mod types;\n", "", rel)
    body = replace_once(body, "pub use types::PrimitiveType;\n", "", rel)
    body = replace_once(body, "pub(crate) mod scan;\n",
                        '#[cfg(feature = "s3tables")]\npub mod s3tables;\npub(crate) mod scan;\n', rel)
    body = replace_once(body, "pub use evolve::{SchemaUpdate, can_promote};\n",
                        "pub use evolve::{SchemaUpdate, can_promote};\npub use field::{IcebergField, IcebergFieldMut};\n", rel)
    body = replace_once(body, "use crate::media::{LocatedTable, RecordOptions, TableFormat};\n",
                        "use crate::media::{LocatedTable, RecordOptions, TableFormat};\n"
                        "use std::sync::{Mutex, OnceLock, PoisonError};\n", rel)
    write(path, LIB_HEAD + doc + "\n#![deny(unsafe_code)]\n\n" + body.rstrip("\n") + "\n" + INSTALL_FN + "\n"
          + INTERNALS_PLACEHOLDER)


FIELD_DOC_OLD = """//! The impls live here rather than beside the other protocol views because the
//! property constants belong to the documents they are read from and written
//! to, in `schema.rs` and `partition.rs`, and because the whole vocabulary is
//! gated with the rest of the feature.
"""

FIELD_DOC_NEW = """//! [`IcebergField`] and [`IcebergFieldMut`] are minted here by the core's
//! protocol view builder under `Scheme::ICEBERG`, as `yggdryl-fix` mints its
//! own: the views and their keys belong to this crate alone, a caller
//! borrows one with `IcebergField::new(&field)` or
//! `IcebergFieldMut::new(&mut field)`, and the core's `Field` has no accessor
//! for them - `Scheme::ICEBERG` stays the core's, so a property is read with
//! no allocation whichever crate names the view. The property constants
//! belong to the documents they are read from and written to, in `schema.rs`
//! and `partition.rs`.
"""

FIELD_MINT = '''
crate::implementer::protocol_field_types!(
    pub,
    crate::Scheme::ICEBERG,
    IcebergField,
    IcebergFieldMut,
    "Apache Iceberg"
);

'''

OFFICIAL_ADDITIONS = '''

/// `PrimitiveType` as the official Iceberg crate spells it: the one door
/// from the core's primitive vocabulary to the official one, which every
/// manifest literal, partition transform and bound reads through.
///
/// # Errors
///
/// Returns an error for `unknown` and `variant`, which name no concrete
/// primitive a value is encoded under.
pub(crate) fn primitive_into_official(
    primitive: crate::iceberg::PrimitiveType,
) -> crate::Result<iceberg_official::spec::PrimitiveType> {
    use crate::iceberg::PrimitiveType;
    use iceberg_official::spec::PrimitiveType as OfficialPrimitiveType;

    Ok(match primitive {
        PrimitiveType::Boolean => OfficialPrimitiveType::Boolean,
        PrimitiveType::Int => OfficialPrimitiveType::Int,
        PrimitiveType::Long => OfficialPrimitiveType::Long,
        PrimitiveType::Float => OfficialPrimitiveType::Float,
        PrimitiveType::Double => OfficialPrimitiveType::Double,
        PrimitiveType::Decimal { precision, scale } => OfficialPrimitiveType::Decimal {
            precision: u32::from(precision),
            scale: u32::try_from(scale).map_err(|_| crate::Error::Codec {
                format: "iceberg",
                position: 0,
                reason: format_smolstr!("expected a non-negative Iceberg decimal scale, got {scale}"),
            })?,
        },
        PrimitiveType::Date => OfficialPrimitiveType::Date,
        PrimitiveType::Time => OfficialPrimitiveType::Time,
        PrimitiveType::Timestamp => OfficialPrimitiveType::Timestamp,
        PrimitiveType::Timestamptz => OfficialPrimitiveType::Timestamptz,
        PrimitiveType::TimestampNs => OfficialPrimitiveType::TimestampNs,
        PrimitiveType::TimestamptzNs => OfficialPrimitiveType::TimestamptzNs,
        PrimitiveType::String => OfficialPrimitiveType::String,
        PrimitiveType::Uuid => OfficialPrimitiveType::Uuid,
        PrimitiveType::Fixed(width) => OfficialPrimitiveType::Fixed(u64::from(width)),
        PrimitiveType::Binary => OfficialPrimitiveType::Binary,
        primitive @ (PrimitiveType::Unknown | PrimitiveType::Variant) => {
            return Err(crate::Error::Codec {
                format: "iceberg",
                position: 0,
                reason: format_smolstr!("expected a concrete Iceberg primitive type, got {primitive}"),
            });
        }
        // The core's vocabulary may grow a spelling this boundary has no
        // official twin for yet.
        #[allow(unreachable_patterns)]
        other => {
            return Err(crate::Error::Codec {
                format: "iceberg",
                position: 0,
                reason: format_smolstr!("expected an Iceberg primitive type the official model spells, got {other}"),
            });
        }
    })
}

/// An official Iceberg failure as the core's error: an `Error::External`
/// from `Iceberg`, the failure kept as its source.
pub(crate) fn official_error(value: iceberg_official::Error) -> Error {
    let reason = SmolStr::new(value.to_string());
    Error::external_with_source("Iceberg", reason, value)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/iceberg/tests/iceberg/official.rs` pins and a caller
    //! cannot reach.
    //!
    //! The official failure [`super::official_error`] wraps as an
    //! `Error::External` is a type no caller of this crate can name, so the
    //! pin asks for one by its message. The forwarder changes no visibility.

    use crate::Error;

    /// Wrap an official Iceberg failure, built from `message`, as a core
    /// error, so the source it keeps can be read back.
    #[must_use]
    pub fn invalid_iceberg_metadata(message: &'static str) -> Error {
        super::official_error(iceberg_official::Error::new(
            iceberg_official::ErrorKind::DataInvalid,
            message,
        ))
    }
}
'''


def edit_field_view(root: pathlib.Path) -> None:
    rel = f"{CRATE_DIR}/src/field.rs"
    text = read(root / rel)
    text = replace_once(text, FIELD_DOC_OLD, FIELD_DOC_NEW, rel)
    text = replace_once(
        text,
        "use crate::{DataType, Error, Field, IcebergField, IcebergFieldMut, Result, Scalar};\n",
        "use crate::{DataType, Error, Field, Result, Scalar};\n" + FIELD_MINT,
        rel,
    )
    write(root / rel, text)


def edit_official(root: pathlib.Path) -> None:
    rel = f"{CRATE_DIR}/src/official.rs"
    text = read(root / rel)
    if "fn primitive_into_official(" not in text:
        text = text.rstrip("\n") + "\n" + OFFICIAL_ADDITIONS
    write(root / rel, text)


# The crate-private methods of the core's public types the moved sources call,
# re-spelled through their `implementer` forwarders (S3's route: a free
# function taking the receiver first, `<type>_<item>`), and the two official
# doors the core held. Each anchor is asserted to match exactly so often.
METHOD_SUBS: list[tuple[str, str, str, int]] = [
    ("table.rs", "        self.stated = match self.root.url() {\n",
     "        self.stated = match crate::implementer::handle_url(&self.root) {\n", 1),
    ("table.rs", "                .filter(|(name, _)| !Holder::is_backend_property(url, name))\n",
     "                .filter(|(name, _)| !crate::implementer::holder_is_backend_property(url, name))\n", 1),
    ("table.rs", "        self.root\n            .set_properties(self.stated.inherit(&self.inherited));\n",
     "        crate::implementer::handle_set_properties(&mut self.root, self.stated.inherit(&self.inherited));\n", 2),
    ("table.rs", "            None => self.root.exists(),\n",
     "            None => crate::implementer::handle_exists(&self.root),\n", 1),
    ("table.rs", "        if location.names_s3_tables() {\n",
     "        if crate::implementer::uri_names_s3_tables(location) {\n", 3),
    ("table.rs", "        match crate::expression::Derivation::owning(self.schema()?)? {\n"
                 "            Some(derivation) => derivation.apply_arrow_reader(batches),\n"
                 "            None => Ok(batches),\n        }\n",
     "        crate::implementer::derivation_owning_apply(self.schema()?, batches)\n", 1),
    ("table.rs", "        let cuts = reader.map_landed(threads, move |record| {\n",
     "        let cuts = crate::implementer::stream_chunked_serie_map_landed(reader, threads, move |record| {\n", 1),
    ("table.rs", "            Ok(crate::StreamChunkedSerie::from_landed_iter(\n",
     "            Ok(crate::implementer::stream_chunked_serie_from_landed_iter(\n", 1),
    ("table.rs", "record.map(|record| record.into_relabeled(std::sync::Arc::clone(&relabel)))",
     "record.map(|record| crate::implementer::serie_into_relabeled(record, std::sync::Arc::clone(&relabel)))", 1),
    ("table.rs", "        let cadence = options.commit_cadence(Cadence::Once)?;\n",
     "        let cadence = crate::implementer::record_options_commit_cadence(options, Cadence::Once)?;\n", 1),
    ("table.rs", "        let mut commits = options.commit_arrow_readers(batches, cadence)?;\n",
     "        let mut commits = crate::implementer::record_options_commit_arrow_readers(options, batches, cadence)?;\n", 1),
    ("table.rs", "    let hold = if write.sort.is_empty() || hold.keeps_order(write.sort)? {\n",
     "    let hold = if write.sort.is_empty() || crate::implementer::chunked_serie_keeps_order(&hold, write.sort)? {\n", 1),
    ("table.rs", "        let sorted = hold.sorted_by_on(write.sort, threads)?;\n",
     "        let sorted = crate::implementer::chunked_serie_sorted_by_on(&hold, write.sort, threads)?;\n", 1),
    ("table.rs", "            options.set_file_threads(threads);\n",
     "            crate::implementer::record_options_set_file_threads(&mut options, threads);\n", 1),
    ("table.rs", "use crate::serie::{Closing, Partitions};\n",
     "use crate::implementer::{PartitionClosing as Closing, Partitions};\n", 1),
    ("scan.rs", "        options.set_file_threads(self.threads);\n",
     "        crate::implementer::record_options_set_file_threads(&mut options, self.threads);\n", 1),
    ("scan.rs", "    if select.unnests() {\n", "    if crate::implementer::selector_unnests(select) {\n", 1),
    ("scan.rs", "        let hold = if hold.keeps_order(&self.sorting)? {\n",
     "        let hold = if crate::implementer::chunked_serie_keeps_order(&hold, &self.sorting)? {\n", 1),
    ("scan.rs", "        crate::StreamChunkedSerie::from_landed_iter(Arc::clone(&self.root), self)\n",
     "        crate::implementer::stream_chunked_serie_from_landed_iter(Arc::clone(&self.root), self)\n", 1),
    ("scan.rs", "return Some(Ok(record.into_relabeled(Arc::clone(&self.root))));",
     "return Some(Ok(crate::implementer::serie_into_relabeled(record, Arc::clone(&self.root))));", 1),
    ("scan.rs", "        plans\n            .get_or_compile(batch.schema_ref().fields(), || {\n"
                "                ArrowCastPlan::compile_schema(\n                    batch.schema_ref(),\n"
                "                    root,\n                    ArrowCastOptions::new(),\n"
                "                    Deferred::default(),\n                )\n            })?\n"
                "            .reconcile_batch(batch)\n",
     "        let plan = plans.get_or_compile(batch.schema_ref().fields(), || {\n"
     "            crate::implementer::arrow_cast_plan_compile_schema(\n                batch.schema_ref(),\n"
     "                root,\n                ArrowCastOptions::new(),\n            )\n        })?;\n"
     "        crate::implementer::arrow_cast_plan_reconcile_batch(plan, batch)\n", 1),
    ("scan.rs", "use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred, PlanCache};\n",
     "use crate::cast::{ArrowCastOptions, ArrowCastPlan};\nuse crate::implementer::PlanCache;\n", -1),
    ("scan.rs", "use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred};\n",
     "use crate::cast::{ArrowCastOptions, ArrowCastPlan};\n", -1),
    ("partition.rs", "        match function.epoch_period(constants).ok()?? {\n",
     "        match crate::implementer::function_epoch_period(function, constants).ok()?? {\n", 1),
    ("catalog/mod.rs", "        self.handle.set_properties(self.stated.clone());\n",
     "        crate::implementer::handle_set_properties(&mut self.handle, self.stated.clone());\n", 1),
    ("catalog/mod.rs", "        self.handle.url()\n", "        crate::implementer::handle_url(&self.handle)\n", 1),
    ("catalog/namespace.rs", "        self.handle.url()\n", "        crate::implementer::handle_url(&self.handle)\n", 1),
    ("catalog/namespace.rs", "        self.handle\n            .set_properties(self.stated.inherit(&self.inherited));\n",
     "        crate::implementer::handle_set_properties(&mut self.handle, self.stated.inherit(&self.inherited));\n", 2),
    ("s3tables/mod.rs", "        location.names_s3_tables()\n",
     "        crate::implementer::uri_names_s3_tables(location)\n", 1),
    ("s3tables/catalog.rs", "            if arn.identified_table().is_some() {\n",
     "            if crate::implementer::arn_identified_table(&arn).is_some() {\n", 1),
    ("s3tables/catalog.rs", "            if arn.table_bucket().is_some() {\n",
     "            if crate::implementer::arn_table_bucket(&arn).is_some() {\n", 1),
    ("s3tables/catalog.rs", "    if bucket.table_bucket().is_none() {\n",
     "    if crate::implementer::arn_table_bucket(&bucket).is_none() {\n", 1),
    ("s3tables/catalog.rs", "            if session.stated_region() == Some(region) {\n",
     "            if crate::implementer::session_stated_region(&session) == Some(region) {\n", 1),
    ("s3tables/bucket.rs", "    if bucket.table_bucket().is_none() {\n",
     "    if crate::implementer::arn_table_bucket(&bucket).is_none() {\n", 1),
    ("s3tables/table.rs", "    let Some((bucket, _)) = table.identified_table() else {\n",
     "    let Some((bucket, _)) = crate::implementer::arn_identified_table(&table) else {\n", 1),
    ("s3tables/client.rs", "        ArnPartition::check_region(&region, source)?;\n",
     "        crate::implementer::arn_partition_check_region(&region, source)?;\n", 1),
    ("s3tables/client.rs", "                let message = if refuses_key && ArnPartition::is_opt_in(region) {\n",
     "                let message = if refuses_key && crate::implementer::arn_partition_is_opt_in(region) {\n", 1),
    ("s3tables/client.rs", "        let mut request = self\n            .session\n            .http()?\n",
     "        let mut request = crate::implementer::session_http(&self.session)?\n", 1),
]

# Regular substitutions over every moved source: the land with nothing taken
# on trust, the official doors, the core's own type vocabulary.
REGEX_SUBS: list[tuple[str, str]] = [
    (r"crate::serie::land\(\n((?:[^\n]*\n)*?)([ \t]*)&crate::serie::Proof::Unproven,\n",
     r"crate::implementer::land_unproven(\n\1"),
    (r"super::PrimitiveType::from_dtype\(([^()]*)\)\?\.into_official\(\)\?",
     r"crate::iceberg::official::primitive_into_official(super::PrimitiveType::from_dtype(\1)?)?"),
    (r"super::PrimitiveType::from_dtype\(dtype\)\n([ \t]*)\.ok\(\)\?\n[ \t]*\.into_official\(\)\n",
     r"crate::iceberg::official::primitive_into_official(super::PrimitiveType::from_dtype(dtype).ok()?)\n"),
    (r"\bError::from_iceberg\b", "crate::iceberg::official::official_error"),
    (r"\bError::iceberg\(", 'Error::external("Iceberg", '),
    (r"(?<![\w:])(?:crate::)?Handle::(at|bound)\(", r"crate::implementer::handle_\1("),
    (r"(?<![\w:])super::PrimitiveType\b", "crate::iceberg::PrimitiveType"),
    (r"(?<![\w:])super::types::PrimitiveType\b", "crate::iceberg::PrimitiveType"),
]


def edit_crate_sources(root: pathlib.Path) -> None:
    src = root / CRATE_DIR / "src"
    for rel, old, new, count in METHOD_SUBS:
        path = src / rel
        if not path.exists():
            residue(f"{CRATE_DIR}/src/{rel}", "absent: its method redirects were not made")
            continue
        text = read(path)
        if count < 0:
            if old in text:
                text = text.replace(old, new, 1)
        elif new.strip() and text.count(new) and not text.count(old):
            continue
        else:
            text = replace_n(text, old, new, count, f"{CRATE_DIR}/src/{rel}")
        write(path, text)
    for path in sorted(src.rglob("*.rs")):
        text = read(path)
        new = text
        for pattern, replacement in REGEX_SUBS:
            new = re.sub(pattern, replacement, new)
        if new != text:
            write(path, new)
    for rel in ("table.rs", "partition.rs", "value.rs", "manifest.rs", "metadata.rs", "scan.rs"):
        path = src / rel
        if path.exists() and re.search(r"\.into_official\(\)\?|from_iceberg\b|serie::land\(|Proof::Unproven",
                                       code_only(read(path))):
            for m in re.finditer(r"PrimitiveType::from_dtype\([^\n]*\.into_official\(\)|from_iceberg\b|serie::land\(",
                                 read(path)):
                residue(f"{CRATE_DIR}/src/{rel}:{line_of(read(path), m.start())}", "a door the core held is left")
    edit_field_view(root)
    edit_official(root)
    # Doc lines that name the accessors the core no longer has.
    edit(root, f"{CRATE_DIR}/src/schema.rs",
         ("//! through [`Field::as_iceberg`] and [`Field::as_iceberg_mut`], which own the\n",
          "//! through [`IcebergField`](crate::IcebergField) and\n"
          "//! [`IcebergFieldMut`](crate::IcebergFieldMut), which own the\n"))
    done("crate: lib.rs, the view minted in field.rs, the official doors, the method redirects")


# ---------------------------------------------------------------------------
# Step 2: the benches
# ---------------------------------------------------------------------------

FIELD_BENCH_DOC = """//! What the Iceberg protocol view a field answers costs: the typed reads of
//! its properties - the one whose assembled key outgrows the inline key
//! buffer among them - and a set that changes nothing beside one that does.
//! These are the `value` group's Iceberg rows, which left the core's `types`
//! benchmark with the view (S6, D39).

use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::DataType;

pub(crate) fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("value");
"""

AWS_BENCH_DOC = """//! What holding an Amazon S3 Tables table bucket's location costs: its
//! catalog and a namespace below it are descriptions, built from the
//! properties - who signs among them - with no request and no file read.
//! These are the `aws_session` group's S3 Tables rows, which left the core's
//! `holder` benchmark with the `s3tables` feature (S6).

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::Arn;

pub(crate) fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("aws_session");
"""

BENCH_ROOT = """//! The Iceberg table format's benchmarks: planning, metadata, manifests and
//! the partition renderer (`iceberg/media.rs`), the Iceberg protocol view a
//! field answers (`iceberg/field.rs`) and, under `s3tables`, what holding a
//! table bucket's location costs (`iceberg/aws.rs`). `main` installs the
//! crate before any group runs, so a folder laid out as a table reads
//! through the core's register.

#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

#[cfg(feature = "s3tables")]
#[path = "iceberg/aws.rs"]
mod aws;
#[path = "iceberg/field.rs"]
mod field;
#[path = "iceberg/media.rs"]
mod media;

use criterion::{Criterion, criterion_group};

fn s3tables_benchmarks(_criterion: &mut Criterion) {
    #[cfg(feature = "s3tables")]
    aws::benchmarks(_criterion);
}

criterion_group!(
    iceberg,
    media::benchmarks,
    field::benchmarks,
    s3tables_benchmarks,
);

fn main() {
    // The format, its catalog and the crates it reads are claimed before any
    // group runs.
    yggdryl_iceberg::install().expect("yggdryl-iceberg installs");
    iceberg();
    media::cleanup();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
"""


def carve_block(text: str, anchor: str, rel: str) -> tuple[str, str | None]:
    """Take the `#[cfg(feature = ...)]` block that `anchor` opens out of
    `text`: the text without it, and its body dedented one level."""
    start = text.find(anchor)
    if start < 0 or text.count(anchor) != 1:
        residue(rel, f"the block `{anchor.strip()[:60]}` was not found once; not carved")
        return text, None
    brace = text.find("{", start + len(anchor) - 2)
    end = s4.matching_brace(text, brace)
    body = text[brace + 1:end].strip("\n")
    line_end = text.find("\n", end) + 1
    return text[:start] + text[line_end:], dedent(body + "\n")


def carve_benches(root: pathlib.Path) -> list[str]:
    """The Iceberg rows of the `types` and `holder` benchmarks, carved into
    the crate's own `iceberg` bench beside the moved `media` group."""
    written = []
    rel = "rust/benchmarks/types/field/value.rs"
    if (root / rel).exists():
        text = read(root / rel)
        text, body = carve_block(text, '    #[cfg(feature = "iceberg")]\n    {\n', rel)
        if body is not None:
            write(root / rel, tidy_head(prune_imports(text)))
            dest = f"{CRATE_DIR}/benchmarks/iceberg/field.rs"
            write(root / dest, FIELD_BENCH_DOC + body + "    group.finish();\n}\n")
            written.append(dest)
    rel = "rust/benchmarks/holder/aws.rs"
    if (root / rel).exists():
        text = read(root / rel)
        text = replace_once(
            text,
            "    // What holding a table bucket's location costs: its catalog and a\n"
            "    // namespace below it are descriptions, built from the properties - who\n"
            "    // signs among them - with no request and no file read.\n"
            '    #[cfg(feature = "s3tables")]\n    {\n',
            '    #[cfg(feature = "s3tables")]\n    {\n',
            rel,
        )
        text, body = carve_block(text, '    #[cfg(feature = "s3tables")]\n    {\n', rel)
        if body is not None:
            text = replace_once(
                text,
                "//! attempt's signing at a stated instant). Under `s3tables`, what holding a\n"
                "//! table bucket's location costs sits beside them: its catalog and a\n"
                "//! namespace are descriptions, built from the properties with no request.\n",
                "//! attempt's signing at a stated instant).\n",
                rel,
            )
            write(root / rel, tidy_head(prune_imports(text)))
            dest = f"{CRATE_DIR}/benchmarks/iceberg/aws.rs"
            write(root / dest, AWS_BENCH_DOC + body + "    group.finish();\n}\n")
            written.append(dest)
    rel = "rust/benchmarks/media.rs"
    edit(
        root, rel,
        ('#[cfg(feature = "iceberg")]\n#[path = "media/iceberg.rs"]\nmod iceberg;\n', ""),
        ("fn iceberg_benchmarks(_criterion: &mut Criterion) {\n"
         '    #[cfg(feature = "iceberg")]\n    iceberg::benchmarks(_criterion);\n}\n\n', ""),
        ("    iceberg_benchmarks,\n", ""),
        ('    media();\n    #[cfg(feature = "iceberg")]\n    iceberg::cleanup();\n', "    media();\n"),
    )
    write(root / f"{CRATE_DIR}/benchmarks/iceberg.rs", BENCH_ROOT)
    written.append(f"{CRATE_DIR}/benchmarks/iceberg.rs")
    done(f"benches: {', '.join(written)} written; the core's `media`, `types` and `holder` lose their Iceberg rows")
    return written


# ---------------------------------------------------------------------------
# Step 5: the core
# ---------------------------------------------------------------------------

# The `implementer` entries the moved sources reach (S3's routes), each
# guarded by the name it publishes: a sibling define that wrote one first -
# the object stores' `canonical_query`, `oversized`, `session_stated_region`
# and `arn_partition_check_region`, the workbook's `field_under` - keeps it.
ICEBERG_SECTION_HEAD = '''
// ------------------------------------------------------------------------
// Iceberg: what the Iceberg crate reaches - a write's shaping, count and
// cadences, the one partitioner and the residual a read runs once, the
// warehouse handle its objects resolve, the time buckets and periods a
// partition bound reads, and, under `aws`, the identity service doors its
// Amazon S3 Tables client signs and reads its answers through.
// ------------------------------------------------------------------------
'''

ICEBERG_ENTRIES: list[tuple[str, str]] = [
    ("sorted_values", '''
/// `metadata::sorted_values`, for the Iceberg crate: values borrowed in
/// their order, what a document writes a set in.
#[inline]
#[must_use]
pub fn sorted_values<T: Ord>(values: &[T]) -> Vec<&T> {
    crate::metadata::sorted_values(values)
}
'''),
    ("PARQUET_FIELD_ID_KEY", '''
/// `metadata::PARQUET_FIELD_ID_KEY`, for the Iceberg crate: the key a
/// column's Parquet field id is stated under.
pub const PARQUET_FIELD_ID_KEY: &str = crate::metadata::PARQUET_FIELD_ID_KEY;
'''),
    ("SORT_BY_KEY", '''
/// `metadata::SORT_BY_KEY`, for the Iceberg crate: the key a record states
/// the order of its rows under.
pub const SORT_BY_KEY: &str = crate::metadata::SORT_BY_KEY;
'''),
    ("parse_by_list", '''
/// `metadata::parse_by_list`, for the Iceberg crate: a `by` property's JSON
/// array of expression texts, each entry as written.
///
/// # Errors
///
/// Returns the refusal of a value that is no JSON array of texts, naming
/// `key`.
#[inline]
pub fn parse_by_list(key: &str, value: &str) -> Result<Vec<String>> {
    crate::metadata::parse_by_list(key, value)
}
'''),
    ("parse_by_ordering", '''
/// `metadata::parse_by_ordering`, for the Iceberg crate: one `SORT:by` entry
/// read as the `order by` key it spells.
///
/// # Errors
///
/// Returns the grammar's refusal of the entry, naming `key`.
#[inline]
pub fn parse_by_ordering(key: &str, text: &str) -> Result<crate::expression::Ordering> {
    crate::metadata::parse_by_ordering(key, text)
}
'''),
    ("UUID_SPELLINGS", '''
/// `uuid::UUID_SPELLINGS`, for the Iceberg crate: what every refusal of a
/// UUID's bytes names.
pub const UUID_SPELLINGS: &str = crate::uuid::UUID_SPELLINGS;
'''),
    ("decimal_text", '''
/// `decimal::decimal_text`, for the Iceberg crate: an exact decimal's
/// canonical text, its coefficient at `scale`.
#[inline]
#[must_use]
pub fn decimal_text(coefficient: crate::i256, scale: i8) -> String {
    crate::decimal::decimal_text(coefficient, scale)
}
'''),
    ("read_enum_code", '''
/// `enums::read_enum_code`, for the Iceberg crate: the member an enum
/// datatype stores under `code`.
///
/// # Errors
///
/// Returns the refusal of a code the datatype names no member under.
#[inline]
pub fn read_enum_code(dtype: &DataType, code: i64) -> Result<Scalar> {
    crate::enums::read_enum_code(dtype, code)
}
'''),
    ("is_null_value", '''
/// `variant::is_null_value`, for the Iceberg crate: whether a variant value
/// payload encodes the variant null.
#[inline]
#[must_use]
pub fn is_null_value(value: &[u8]) -> bool {
    crate::variant::is_null_value(value)
}
'''),
    ("partition_column_name", '''
/// `structure::partition_column_name`, for the Iceberg crate: the column a
/// `PARTITION:by` entry names - its alias, else `{source}_{function}`.
///
/// # Errors
///
/// Returns the refusal of an entry that names no column.
#[inline]
pub fn partition_column_name(entry: &crate::expression::Projection) -> Result<SmolStr> {
    crate::structure::partition_column_name(entry)
}
'''),
    ("per_second", '''
/// `temporal::per_second`, for the Iceberg crate: how many of `unit` a
/// second holds, `None` for a unit longer than one.
#[inline]
#[must_use]
pub const fn per_second(unit: TimeUnit) -> Option<i64> {
    crate::temporal::per_second(unit)
}
'''),
    ("oversized", '''
/// `iobase::oversized`, for the Iceberg crate: the refusal of a value too
/// large for the platform's addressable memory.
#[inline]
#[must_use]
pub fn oversized(size: u64) -> crate::Error {
    crate::iobase::oversized(size)
}
'''),
    ("WriteCount", '''
/// What one write read, wrote and skipped, counted once in the shared
/// shaping, for the Iceberg crate: the count [`prepare_arrow_write_deriving`]
/// answers beside the shaped stream.
pub use crate::iobase::WriteCount;
'''),
    ("non_empty_arrow_reader", '''
/// `iobase::non_empty_arrow_reader`, for the Iceberg crate: the reader with
/// its first batch pulled to see that there is one, `None` where it yields
/// no row.
///
/// # Errors
///
/// Returns the reader's failure on that first batch.
#[inline]
pub fn non_empty_arrow_reader(
    batches: crate::arrow::BatchReader,
) -> Result<Option<crate::arrow::BatchReader>> {
    crate::iobase::non_empty_arrow_reader(batches)
}
'''),
    ("prepare_arrow_write_deriving", '''
/// `iobase::prepare_arrow_write_deriving`, for the Iceberg crate: a write's
/// one shaping pass over a stored field that derives columns of its own -
/// the derived columns computed before the `where` and the `select` read
/// them - and the count it keeps.
///
/// # Errors
///
/// Returns the shaping's refusal: a clause that does not bind, a derivation
/// that fails, a cast the stored field refuses.
#[inline]
pub fn prepare_arrow_write_deriving(
    batches: crate::arrow::BatchReader,
    options: &crate::media::RecordOptions,
    stored: &Field,
) -> Result<(crate::arrow::BatchReader, WriteCount)> {
    crate::iobase::prepare_arrow_write_deriving(batches, options, stored)
}
'''),
    ("absent", '''
/// `media::merge::absent`, for the Iceberg crate: which incoming rows a
/// stored side does not hold the key of, and how many it does.
///
/// # Errors
///
/// Returns an empty or unbound key, a key column with no row encoding, or
/// the first read or cast failure of either side.
#[inline]
pub fn absent(
    stored: crate::arrow::BatchReader,
    stored_root: &Field,
    incoming: &[RecordBatch],
    field: &Field,
    merge_by: &crate::Selector,
) -> Result<(arrow_array::BooleanArray, u64)> {
    crate::media::merge::absent(stored, stored_root, incoming, field, merge_by)
}
'''),
    ("merged", '''
/// `media::merge::merged`, for the Iceberg crate: `incoming` merged into
/// `stored` on the `merge_by` columns, and whether the rows changed.
///
/// # Errors
///
/// Returns an empty or unbound key, a key column with no row encoding, or
/// the first read or cast failure of either side.
#[inline]
pub fn merged(
    stored: crate::arrow::BatchReader,
    incoming: crate::arrow::BatchReader,
    field: &Field,
    merge_by: &crate::Selector,
    safe: bool,
) -> Result<Merged> {
    crate::media::merge::merged(stored, incoming, field, merge_by, safe)
}

/// What [`merged`] answers, for the Iceberg crate: the merged rows, and
/// whether they differ from what was stored.
pub use crate::media::merge::Merged;
'''),
    ("Cadence", '''
/// How a streamed write is cut into publications, for the Iceberg crate.
pub use crate::media::options::commit::Cadence;
'''),
    ("record_options_commit_cadence", '''
/// `RecordOptions::commit_cadence`, for the Iceberg crate: the cadence a
/// write publishes by - the batch count the options state, else `default`.
///
/// # Errors
///
/// Returns the refusal of a zero batch count.
#[inline]
pub fn record_options_commit_cadence(
    options: &crate::media::RecordOptions,
    default: Cadence,
) -> Result<Cadence> {
    options.commit_cadence(default)
}
'''),
    ("record_options_commit_arrow_readers", '''
/// `RecordOptions::commit_arrow_readers`, for the Iceberg crate: one shaped
/// stream split into bounded publication readers, `cadence` where the
/// options state no batch count; the type carrying them stays private.
///
/// # Errors
///
/// Returns the refusal of a zero batch count.
#[inline]
pub fn record_options_commit_arrow_readers(
    options: &crate::media::RecordOptions,
    batches: crate::arrow::BatchReader,
    cadence: Cadence,
) -> Result<impl Iterator<Item = Result<crate::arrow::BatchReader>>> {
    options.commit_arrow_readers(batches, cadence)
}
'''),
    ("record_options_set_file_threads", '''
/// `RecordOptions::set_file_threads`, for the Iceberg crate: one file's
/// share of a table's threads, handed down before it is read or written.
#[inline]
pub fn record_options_set_file_threads(options: &mut crate::media::RecordOptions, threads: usize) {
    options.set_file_threads(threads);
}
'''),
    ("compose", '''
/// The one composition of a record read, for the Iceberg crate: what a
/// table's scan is handed and the residual it runs once over the rows - the
/// serie's verbs (`Scan`, none for a read door), the composed options
/// (`Composed`) and what runs after the medium (`Residual`).
pub use crate::media_serie::{Composed, Residual, Scan, compose};
'''),
    ("PartitionClosing", '''
/// The one partitioner every split of a stream by partition runs through,
/// for the Iceberg crate: the pieces each batch was cut into, pushed to
/// their open partition and closed as the closing rule says.
pub use crate::serie::partition::{Closing as PartitionClosing, Partitions, Pieces};
'''),
    ("serie_into_relabeled", '''
/// `Serie::into_relabeled`, for the Iceberg crate: a record under another
/// field of the same layout, its leaf's field swapped where it stands.
#[inline]
#[must_use]
pub fn serie_into_relabeled(serie: Serie, field: std::sync::Arc<Field>) -> Serie {
    serie.into_relabeled(field)
}
'''),
    ("stream_chunked_serie_from_landed_iter", '''
/// `StreamChunkedSerie::from_landed_iter`, for the Iceberg crate: the stream
/// of record columns `records` lays out as it is pulled, each already
/// landed under `root`.
///
/// # Errors
///
/// Returns the refusal of a root with no Arrow projection.
#[inline]
pub fn stream_chunked_serie_from_landed_iter(
    root: std::sync::Arc<Field>,
    records: impl Iterator<Item = crate::arrow::Result<Serie>> + Send + 'static,
) -> crate::arrow::Result<crate::StreamChunkedSerie> {
    crate::StreamChunkedSerie::from_landed_iter(root, records)
}
'''),
    ("stream_chunked_serie_map_landed", '''
/// `StreamChunkedSerie::map_landed`, for the Iceberg crate: each batch
/// landed and handed to `work` on up to `threads` threads, the answers in
/// batch order.
#[inline]
pub fn stream_chunked_serie_map_landed<R, F>(
    reader: crate::StreamChunkedSerie,
    threads: usize,
    work: F,
) -> Box<dyn Iterator<Item = crate::arrow::Result<R>> + Send>
where
    R: Send + 'static,
    F: Fn(Serie) -> crate::arrow::Result<R> + Send + Sync + 'static,
{
    reader.map_landed(threads, work)
}
'''),
    ("chunked_serie_keeps_order", '''
/// `ChunkedSerie::keeps_order`, for the Iceberg crate: whether the rows are
/// in the order `by` states - proven by the root's declaration where it
/// states it, else read once chunk by chunk.
///
/// # Errors
///
/// Returns the binder's refusal of a key.
#[inline]
pub fn chunked_serie_keeps_order(
    chunked: &crate::ChunkedSerie,
    by: &[crate::expression::Ordering],
) -> Result<bool> {
    chunked.keeps_order(by)
}
'''),
    ("chunked_serie_sorted_by_on", '''
/// `ChunkedSerie::sorted_by_on`, for the Iceberg crate: the rows sorted by
/// `by`, the chunks sorted on up to `threads` threads before the one merge.
///
/// # Errors
///
/// Returns the binder's refusal of a key, or a sort's failure.
#[inline]
pub fn chunked_serie_sorted_by_on(
    chunked: &crate::ChunkedSerie,
    by: impl crate::expression::IntoOrderings,
    threads: usize,
) -> Result<crate::ChunkedSerie> {
    chunked.sorted_by_on(by, threads)
}
'''),
    ("EpochPeriod", '''
/// The calendar and clock periods a time transform floors to, and the
/// bucket `time_bucket` floors to, for the Iceberg crate: the one floor the
/// row, the batch and the statistics tiers share.
pub use crate::expression::eval::{EpochPeriod, TimeBucket};
'''),
    ("epoch_value", '''
/// `expression::eval::epoch_value`, for the Iceberg crate: the number of
/// whole periods from the epoch a date or a timestamp falls in.
#[inline]
#[must_use]
pub fn epoch_value(period: EpochPeriod, value: &Scalar) -> Scalar {
    crate::expression::eval::epoch_value(period, value)
}
'''),
    ("order", '''
/// `expression::eval::order`, for the Iceberg crate: how two values of
/// `dtype` compare, `None` where they do not.
#[inline]
#[must_use]
pub fn order(dtype: &DataType, left: &Scalar, right: &Scalar) -> Option<std::cmp::Ordering> {
    crate::expression::eval::order(dtype, left, right)
}
'''),
    ("function_epoch_period", '''
/// `Function::epoch_period`, for the Iceberg crate: the period a time
/// function floors to, given its constant arguments in order.
///
/// # Errors
///
/// Refuses a `minutes` call whose step is missing, or is not a literal
/// whole number from 1 to `u32::MAX`.
#[inline]
pub fn function_epoch_period<'value>(
    function: &crate::expression::Function,
    constants: impl Iterator<Item = Option<&'value Scalar>>,
) -> Result<Option<EpochPeriod>> {
    function.epoch_period(constants)
}
'''),
    ("derivation_owning_apply", '''
/// `Derivation::owning` applied to a reader, for the Iceberg crate: the
/// `TRANSFORM:` columns `root` declares computed for every row, whatever
/// the rows carry under a derived name; a root deriving nothing hands the
/// reader back.
///
/// # Errors
///
/// Returns the refusal of a root that is not a struct, or a derivation's
/// failure.
#[inline]
pub fn derivation_owning_apply(
    root: &Field,
    reader: crate::arrow::BatchReader,
) -> Result<crate::arrow::BatchReader> {
    match crate::expression::Derivation::owning(root)? {
        Some(derivation) => derivation.apply_arrow_reader(reader),
        None => Ok(reader),
    }
}
'''),
    ("selector_unnests", '''
/// `Selector::unnests`, for the Iceberg crate: whether a projection of the
/// selector is an `unnest`.
#[inline]
#[must_use]
pub fn selector_unnests(selector: &crate::Selector) -> bool {
    selector.unnests()
}
'''),
    ("field_under", '''
/// `iomedia::field_under`, for the media crates: the field a read under
/// `options` answers over `root`.
///
/// # Errors
///
/// Returns a clause that does not bind against `root`.
#[inline]
pub fn field_under(options: &crate::media::RecordOptions, root: &Field) -> Result<Field> {
    crate::iomedia::field_under(options, root)
}
'''),
    ("Site", '''
/// Where a warehouse object's storage is, and what opens a location its
/// owner opens, for the Iceberg crate.
pub use crate::warehouse::handle::{Opener, Site};
'''),
    ("handle_at", '''
/// `Handle::at`, for the Iceberg crate: a handle resolved from `site` when
/// first needed, under `properties`, belonging to the object at `what`.
#[inline]
#[must_use]
pub fn handle_at(
    site: Site,
    declared: bool,
    what: &[SmolStr],
    properties: crate::Properties,
) -> crate::Handle {
    crate::Handle::at(site, declared, what, properties)
}
'''),
    ("handle_bound", '''
/// `Handle::bound`, for the Iceberg crate: a handle already in hand, its
/// site read off it for a clone.
#[inline]
#[must_use]
pub fn handle_bound(
    holder: crate::holder::Holder,
    declared: bool,
    what: &[SmolStr],
    properties: crate::Properties,
) -> crate::Handle {
    crate::Handle::bound(holder, declared, what, properties)
}
'''),
    ("handle_set_properties", '''
/// `Handle::set_properties`, for the Iceberg crate: what the handle opens
/// with from now on.
#[inline]
pub fn handle_set_properties(handle: &mut crate::Handle, properties: crate::Properties) {
    handle.set_properties(properties);
}
'''),
    ("handle_url", '''
/// `Handle::url`, for the Iceberg crate: the location, when the site is one.
#[inline]
#[must_use]
pub fn handle_url(handle: &crate::Handle) -> Option<&crate::Url> {
    handle.url()
}
'''),
    ("handle_exists", '''
/// `Handle::exists`, for the Iceberg crate: whether anything is at the
/// location now.
#[inline]
#[must_use]
pub fn handle_exists(handle: &crate::Handle) -> bool {
    handle.exists()
}
'''),
    ("entry_name", '''
/// `warehouse::entry_name`, for the Iceberg crate: the name a folder entry
/// is listed under.
#[inline]
#[must_use]
pub fn entry_name(holder: &crate::holder::Holder) -> Option<SmolStr> {
    crate::warehouse::entry_name(holder)
}
'''),
    ("table_layout", '''
/// `warehouse::table_layout`, for the Iceberg crate: whether a folder is
/// laid out as a table format's table, by one listing of its `metadata/`.
///
/// # Errors
///
/// Returns the listing's failure.
#[inline]
pub fn table_layout(folder: &crate::holder::Holder) -> Result<bool> {
    crate::warehouse::table_layout(folder)
}
'''),
    ("path_text", '''
/// `warehouse::path_text`, for the Iceberg crate: an object path as the
/// location grammar spells it.
#[inline]
#[must_use]
pub fn path_text(parts: &[SmolStr]) -> String {
    crate::warehouse::path_text(parts)
}
'''),
    ("extended", '''
/// `warehouse::extended`, for the Iceberg crate: an object path with one
/// part more.
#[inline]
#[must_use]
pub fn extended(parts: &[SmolStr], name: &str) -> Vec<SmolStr> {
    crate::warehouse::extended(parts, name)
}
'''),
    ("implementation_name", '''
/// `warehouse::implementation_name`, for the Iceberg crate: the name a
/// refusal calls an implementation by.
#[inline]
#[must_use]
pub fn implementation_name<T: ?Sized>(value: &T) -> &'static str {
    crate::warehouse::implementation_name(value)
}
'''),
    ("holder_is_backend_property", '''
/// `Holder::is_backend_property`, for the Iceberg crate: whether `name` is
/// a property the store behind `url` reads itself.
#[inline]
#[must_use]
pub fn holder_is_backend_property(url: &crate::Url, name: &str) -> bool {
    crate::holder::Holder::is_backend_property(url, name)
}
'''),
    ("uri_names_s3_tables", '''
/// `Uri::names_s3_tables`, for the Iceberg crate: whether an identifier
/// names something an Amazon S3 Tables table bucket keeps.
#[inline]
#[must_use]
pub fn uri_names_s3_tables(uri: &crate::Uri) -> bool {
    uri.names_s3_tables()
}
'''),
    ("arn_identified_table", '''
/// `Arn::identified_table`, for the Iceberg crate: the bucket and the table
/// identifier of an Amazon S3 Tables table's own ARN.
#[inline]
#[must_use]
pub fn arn_identified_table(arn: &crate::Arn) -> Option<(&str, &str)> {
    arn.identified_table()
}
'''),
    ("arn_table_bucket", '''
/// `Arn::table_bucket`, for the Iceberg crate: the table bucket an Amazon
/// S3 Tables table bucket's own ARN names.
#[inline]
#[must_use]
pub fn arn_table_bucket(arn: &crate::Arn) -> Option<&str> {
    arn.table_bucket()
}
'''),
    ("arn_partition_check_region", '''
/// `ArnPartition::check_region`, for the object-store and Iceberg crates: a
/// region stated as one host label, `source` named where it is not.
///
/// # Errors
///
/// `Error::Parse` targeting `region`, at the first byte that breaks the
/// rule.
#[cfg(feature = "aws")]
#[inline]
pub fn arn_partition_check_region(region: &str, source: &str) -> Result<()> {
    crate::ArnPartition::check_region(region, source)
}
'''),
    ("arn_partition_is_opt_in", '''
/// `ArnPartition::is_opt_in`, for the Iceberg crate: whether `region` is one
/// an account must enable before AWS accepts any key there.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn arn_partition_is_opt_in(region: &str) -> bool {
    crate::ArnPartition::is_opt_in(region)
}
'''),
    ("Answer", '''
/// One answer an identity service gave, read whole, for the core's identity
/// sources and the Iceberg crate's Amazon S3 Tables client: its status, the
/// error type it names in `x-amzn-ErrorType`, and its body.
///
/// Every identity call goes out through the session's `http::Session`, so
/// the retries, the timeouts, the proxy rules and the CA bundle are the HTTP
/// client's, stated per request.
#[cfg(feature = "aws")]
pub struct Answer {
    /// The status code the service answered.
    pub status: u16,
    /// The error type the service named in `x-amzn-ErrorType`.
    pub error_type: Option<String>,
    /// The body, read whole.
    pub body: std::sync::Arc<[u8]>,
}

#[cfg(feature = "aws")]
impl Answer {
    /// Send `request` and read its answer whole.
    ///
    /// # Errors
    ///
    /// The HTTP client's: a transport failure no retry could mend
    /// ([`is_unanswered`] tells one where nothing answered), or a body past
    /// the session's bound.
    pub fn of(request: &crate::http::Request) -> Result<Self> {
        let response = request.send()?;
        let error_type = crate::aws::error_type(response.headers()).map(str::to_owned);
        Ok(Self {
            status: response.status().code(),
            error_type,
            body: response.bytes()?,
        })
    }
}
'''),
    ("error_code", '''
/// `aws::error_code`, for the Iceberg crate: the error code a refusing
/// answer names, wherever the AWS protocols let a service state one.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn error_code(headers: &crate::http::Headers, body: &[u8]) -> Option<String> {
    crate::aws::error_code(headers, body)
}
'''),
    ("Refusal", '''
/// What a store's refusal says of the keys a request was signed with, for
/// the Iceberg crate's Amazon S3 Tables client.
#[cfg(feature = "aws")]
pub use crate::aws::credentials::Refusal;
'''),
    ("canonical_query", '''
/// `aws::sigv4::canonical_query`, for the object-store and Iceberg crates:
/// the canonical query string Signature Version 4 signs.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn canonical_query(query: &[(String, String)]) -> String {
    crate::aws::sigv4::canonical_query(query)
}
'''),
    ("encode_query_component", '''
/// `aws::sigv4::encode_query_component`, for the object-store and Iceberg
/// crates: one query component escaped as Signature Version 4 escapes it.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn encode_query_component(text: &str) -> String {
    crate::aws::sigv4::encode_query_component(text)
}
'''),
    ("session_http", '''
/// `Session::http`, for the Iceberg crate: the HTTP session every identity
/// call of the session goes out through.
///
/// # Errors
///
/// A CA bundle that cannot be read or holds no certificate.
#[cfg(feature = "aws")]
#[inline]
pub fn session_http(session: &crate::aws::Session) -> Result<&crate::http::Session> {
    session.http()
}
'''),
    ("session_stated_region", '''
/// `Session::stated_region`, for the object-store and Iceberg crates: the
/// region stated on the session, before anything is resolved.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_stated_region(session: &crate::aws::Session) -> Option<&str> {
    session.stated_region()
}
'''),
    ("is_unanswered", '''
/// `http::is_unanswered`, for the Iceberg crate: whether a failure is a
/// transport failure in which no head was read from any answer.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_unanswered(error: &crate::Error) -> bool {
    crate::http::is_unanswered(error)
}
'''),
    ("land_unproven", '''
/// `serie::land` with nothing taken on trust, for the Parquet and Iceberg
/// crates: one array landed under `field`, every leaf whose layout is not
/// its datatype's whole contract read once.
///
/// # Errors
///
/// Returns the landing's refusal, naming the row and the path below it.
#[inline]
pub fn land_unproven(
    field: std::sync::Arc<Field>,
    array: arrow_array::ArrayRef,
) -> crate::arrow::Result<Serie> {
    crate::serie::land(field, array, &crate::serie::Proof::Unproven)
}
'''),
]


def published_names(text: str) -> set[str]:
    """Every name `implementer.rs` publishes: a function, a constant, a type
    or a `pub use` leaf (its alias where it has one)."""
    names = set(re.findall(r"(?m)^pub (?:const )?(?:fn|const|struct|enum|type|static) (\w+)", text))
    for m in re.finditer(r"(?m)^pub use [^;]*;", text):
        tree = re.sub(r"\s+", " ", m.group(0))
        for leaf in re.findall(r"(\w+)(?: as (\w+))?\s*[,};]", tree):
            names.add(leaf[1] or leaf[0])
    return names


# The raises behind the `pub use`s above (route R: the module holding each
# publishes nothing, so the raise publishes nothing else), and the one move.
RAISES: list[tuple[str, str, str]] = [
    ("rust/src/iobase/transfer.rs", "pub(crate) struct WriteCount(Arc<Rows>);", "pub struct WriteCount(Arc<Rows>);"),
    ("rust/src/iobase/transfer.rs", "    pub(crate) fn skip(&self, rows: u64) {", "    pub fn skip(&self, rows: u64) {"),
    ("rust/src/iobase/transfer.rs", "    pub(crate) fn result(&self) -> IOResult {", "    pub fn result(&self) -> IOResult {"),
    ("rust/src/media/merge.rs", "pub(crate) struct Merged {", "pub struct Merged {"),
    ("rust/src/media/merge.rs", "    pub(crate) rows: BatchReader,", "    pub rows: BatchReader,"),
    ("rust/src/media/merge.rs", "    pub(crate) changed: bool,", "    pub changed: bool,"),
    ("rust/src/media/options/commit.rs", "pub(crate) enum Cadence {", "pub enum Cadence {"),
    ("rust/src/media_serie.rs", "pub(crate) struct Scan {", "pub struct Scan {"),
    ("rust/src/media_serie.rs", "pub(crate) struct Composed {", "pub struct Composed {"),
    ("rust/src/media_serie.rs", "    pub(crate) handed: RecordOptions,", "    pub handed: RecordOptions,"),
    ("rust/src/media_serie.rs", "    pub(crate) residual: Residual,", "    pub residual: Residual,"),
    ("rust/src/media_serie.rs", "pub(crate) struct Residual {", "pub struct Residual {"),
    ("rust/src/media_serie.rs", "    pub(crate) fn over_units(&self) -> Self {", "    pub fn over_units(&self) -> Self {"),
    ("rust/src/media_serie.rs", "    pub(crate) fn apply_reader(\n", "    pub fn apply_reader(\n"),
    ("rust/src/media_serie.rs", "pub(crate) fn compose(\n", "pub fn compose(\n"),
    ("rust/src/serie.rs", "\nmod partition;\n", "\npub(crate) mod partition;\n"),
    ("rust/src/serie/partition.rs", "pub(crate) enum Closing {", "pub enum Closing {"),
    ("rust/src/serie/partition.rs", "pub(crate) type Pieces<K> =", "pub type Pieces<K> ="),
    ("rust/src/serie/partition.rs", "pub(crate) struct Partitions<K> {", "pub struct Partitions<K> {"),
    ("rust/src/serie/partition.rs", "    pub(crate) fn new(root: Arc<Field>, pieces: Pieces<K>, closing: Closing) -> Self {",
     "    pub fn new(root: Arc<Field>, pieces: Pieces<K>, closing: Closing) -> Self {"),
    ("rust/src/expression/eval.rs", "pub(crate) enum EpochPeriod {", "pub enum EpochPeriod {"),
    ("rust/src/expression/eval.rs", "    pub(crate) const fn seconds(self) -> Option<i64> {", "    pub const fn seconds(self) -> Option<i64> {"),
    ("rust/src/expression/eval.rs", "    pub(crate) const fn takes_date(self) -> bool {", "    pub const fn takes_date(self) -> bool {"),
    ("rust/src/expression/eval.rs", "    pub(crate) fn start_days(self, period: i64) -> Option<i64> {",
     "    pub fn start_days(self, period: i64) -> Option<i64> {"),
    ("rust/src/expression/eval.rs", "    pub(crate) fn start_count(self, period: i64, unit: TimeUnit) -> Option<i64> {",
     "    pub fn start_count(self, period: i64, unit: TimeUnit) -> Option<i64> {"),
    ("rust/src/expression/eval.rs", "pub(crate) struct TimeBucket {", "pub struct TimeBucket {"),
    ("rust/src/expression/eval.rs", "    pub(crate) fn new(width: &Scalar, dtype: &DataType) -> Result<Self> {",
     "    pub fn new(width: &Scalar, dtype: &DataType) -> Result<Self> {"),
    ("rust/src/expression/eval.rs", "    pub(crate) const fn step(self) -> i64 {", "    pub const fn step(self) -> i64 {"),
    ("rust/src/warehouse/mod.rs", "\nmod handle;\n", "\npub(crate) mod handle;\n"),
    ("rust/src/warehouse/handle.rs", "pub(crate) type Opener =", "pub type Opener ="),
    ("rust/src/warehouse/handle.rs", "pub(crate) enum Site {", "pub enum Site {"),
    ("rust/src/aws/credentials.rs", "pub(crate) enum Refusal {", "pub enum Refusal {"),
    ("rust/src/aws/credentials.rs", "    pub(crate) fn from_code(code: &str) -> Option<Self> {",
     "    pub fn from_code(code: &str) -> Option<Self> {"),
]
# The items whose doc blocks a raise makes public: each intra-doc link in
# them becomes a code span, since what it links may stay crate-private.
RAISED_DOCS: list[tuple[str, str]] = [
    ("rust/src/iobase/transfer.rs", "pub struct WriteCount("),
    ("rust/src/media/merge.rs", "pub struct Merged {"),
    ("rust/src/media/options/commit.rs", "pub enum Cadence {"),
    ("rust/src/media_serie.rs", "pub struct Scan {"),
    ("rust/src/media_serie.rs", "pub struct Composed {"),
    ("rust/src/media_serie.rs", "pub struct Residual {"),
    ("rust/src/media_serie.rs", "    pub fn over_units("),
    ("rust/src/media_serie.rs", "    pub fn apply_reader("),
    ("rust/src/media_serie.rs", "pub fn compose("),
    ("rust/src/serie/partition.rs", "pub enum Closing {"),
    ("rust/src/serie/partition.rs", "pub struct Partitions<K> {"),
    ("rust/src/serie/partition.rs", "    pub fn new(root: Arc<Field>"),
    ("rust/src/expression/eval.rs", "pub enum EpochPeriod {"),
    ("rust/src/expression/eval.rs", "    pub const fn seconds("),
    ("rust/src/expression/eval.rs", "    pub const fn takes_date("),
    ("rust/src/expression/eval.rs", "    pub fn start_days("),
    ("rust/src/expression/eval.rs", "    pub fn start_count("),
    ("rust/src/expression/eval.rs", "pub struct TimeBucket {"),
    ("rust/src/expression/eval.rs", "    pub fn new(width: &Scalar"),
    ("rust/src/expression/eval.rs", "    pub const fn step("),
    ("rust/src/warehouse/handle.rs", "pub type Opener ="),
    ("rust/src/warehouse/handle.rs", "pub enum Site {"),
    ("rust/src/aws/credentials.rs", "pub enum Refusal {"),
]
DOC_LINK = re.compile(r"\[`([^`\]]+)`\](?:\([^)\s]*\)|\[[^\]]*\])?")


def unlink_docs(text: str, anchor: str, where: str) -> str:
    """Every intra-doc link in the doc block above `anchor` - and, for a
    struct or an enum, in the docs of its fields or variants - made a code
    span."""
    at = text.find(anchor)
    if at < 0:
        residue(where, f"`{anchor.strip()}` not found to unlink its docs")
        return text
    line_start = text.rfind("\n", 0, at) + 1
    start = line_start
    while True:
        prev_end = start - 1
        if prev_end <= 0:
            break
        prev_start = text.rfind("\n", 0, prev_end) + 1
        stripped = text[prev_start:prev_end].strip()
        if stripped.startswith(("///", "#[")):
            start = prev_start
            continue
        break
    end = at
    if re.match(r"\s*pub (?:struct|enum) ", anchor):
        brace = text.find("{", at)
        semi = text.find(";", at)
        if brace >= 0 and (semi < 0 or brace < semi):
            end = s4.matching_brace(text, brace)
    region = text[start:end]
    region = "\n".join(
        DOC_LINK.sub(lambda m: f"`{m.group(1)}`", line) if line.lstrip().startswith("///") else line
        for line in region.split("\n")
    )
    return text[:start] + region + text[end:]


def move_answer(root: pathlib.Path) -> None:
    """`aws::Answer` moves into `implementer.rs` (M): `aws` is published, so
    a raise there would publish it. The identity sources reach it there."""
    rel = "rust/src/aws/mod.rs"
    text = read(root / rel)
    start = text.find("/// One answer an identity service gave, read whole:")
    anchor = text.find("impl Answer {")
    if start < 0 or anchor < 0:
        if "pub(crate) struct Answer" in text:
            residue(rel, "`Answer` was not found where S6 moves it from")
        return
    end = s4.matching_brace(text, text.find("{", anchor)) + 1
    end = text.find("\n", end) + 1
    text = text[:start] + text[end:].lstrip("\n")
    text = replace_once(text, "fn error_type(headers: &crate::http::Headers) -> Option<&str> {",
                        "pub(crate) fn error_type(headers: &crate::http::Headers) -> Option<&str> {", rel)
    write(root / rel, text)
    for path in sorted((root / "rust/src/aws").rglob("*.rs")):
        src = read(path)
        new = src.replace("super::Answer::of(", "crate::implementer::Answer::of(")
        new = re.sub(r"(?<![\w:])super::Answer\b", "crate::implementer::Answer", new)
        if new != src:
            write(path, new)
    done("aws::Answer moved into implementer.rs; the identity sources re-pointed")


IMPLEMENTER_SHARED_ENTRIES = ("arrow_cast_plan_compile_schema", "land_unproven")


def edit_implementer(root: pathlib.Path) -> None:
    rel = "rust/src/implementer.rs"
    text = read(root / rel)
    if "pub fn arrow_cast_plan_compile_schema(" not in text:
        # Neither the Avro move nor the Parquet move has run: the shared media
        # section, the plan cache moved in, and its raises, as they write them.
        av.edit_implementer(root, av.move_plan_cache(root))
        text = read(root / rel)
    held = published_names(text)
    if "// Iceberg: what the Iceberg crate reaches" in text and all(
            name in held for name, _ in ICEBERG_ENTRIES if name not in ("Answer",)):
        return
    added = [entry for name, entry in ICEBERG_ENTRIES if name not in held]
    if "// Iceberg: what the Iceberg crate reaches" not in text:
        text = text.rstrip("\n") + "\n" + ICEBERG_SECTION_HEAD
    text = text.rstrip("\n") + "\n" + "".join(added)
    # The route list names what moved here.
    for old, new in (
        ("//!   would publish it: [`InstantSequence`], [`Staged`] and [`PlanCache`];\n",
         "//!   would publish it: [`InstantSequence`], [`Staged`], [`PlanCache`]\n"
         "//!   and, under `aws`, `Answer`;\n"),
        ("//!   would publish it: [`InstantSequence`] and [`Staged`];\n",
         "//!   would publish it: [`InstantSequence`], [`Staged`] and, under `aws`,\n"
         "//!   `Answer`;\n"),
    ):
        if old in text:
            text = text.replace(old, new, 1)
            break
    m = re.search(r"(//! workspace: an item is listed because a crate split off the core - )((?:.|\n)*?)( - needs it)", text)
    if m and "the Iceberg crate" not in m.group(2):
        crates = re.sub(r"\s*\n//!\s*", " ", m.group(2)).strip()
        parts = [part.strip() for part in re.split(r",\s*", crates)]
        parts.append("the Iceberg crate")
        sentence = m.group(1)[4:] + ", ".join(parts) + m.group(3)
        # Re-wrap the run of the paragraph this sentence sits in.
        rest_end = text.find("\n//!\n", m.end())
        tail = text[m.end():rest_end] if rest_end > 0 else ""
        words = (sentence + re.sub(r"\n//!\s*", " ", tail)).split(" ")
        lines, line = [], "//!"
        for word in words:
            if len(line) + 1 + len(word) > 80 and line != "//!":
                lines.append(line)
                line = "//!"
            line += " " + word
        lines.append(line)
        text = text[:m.start()] + "\n".join(lines) + text[rest_end:]
    write(root / rel, text)
    # R: the raises, and the re-exports they need.
    by_file: dict[str, list[tuple[str, str]]] = collections.defaultdict(list)
    for path, old, new in RAISES:
        by_file[path].append((old, new))
    for path, pairs in by_file.items():
        edit(root, path, *pairs)
    edit(root, "rust/src/iobase.rs",
         ("pub use transfer::{ArrowWriteSession,", "pub use transfer::{ArrowWriteSession, WriteCount,"))
    for path, anchor in RAISED_DOCS:
        if (root / path).exists():
            write(root / path, unlink_docs(read(root / path), anchor, path))
    move_answer(root)
    # A gate the forwarders lift: `stated_region` was the object stores' alone.
    edit(root, "rust/src/aws/session.rs",
         ('    #[cfg(feature = "s3")]\n    pub(crate) fn stated_region(&self) -> Option<&str> {\n',
          "    pub(crate) fn stated_region(&self) -> Option<&str> {\n"), optional=True)
    done(f"implementer.rs: the Iceberg section, {len(added)} entr{'y' if len(added) == 1 else 'ies'} added, "
         f"{len(RAISES)} raises and the moved `Answer`")


ICEBERG_EMIT = """        $emit!(
            as_iceberg,
            as_iceberg_mut,
            ICEBERG,
            IcebergField,
            IcebergFieldMut,
            "Apache Iceberg"
        );
"""

SEED_STATIC = "static SEEDED: OnceLock<()> = OnceLock::new();\n"


def drop_seed(root: pathlib.Path, rel: str) -> None:
    """A register's seed - the core's own claims of what `yggdryl-iceberg`
    now claims - and every call of it, gone."""
    path = root / rel
    if not path.exists():
        residue(rel, "absent; its seed was not dropped")
        return
    text = read(path)
    if "fn seed()" not in text:
        return
    text = replace_once(text, SEED_STATIC, "", rel)
    m = re.search(r"(?m)^(?:///[^\n]*\n)*fn seed\(\) \{\n", text)
    if not m:
        residue(rel, "no `fn seed()` to drop")
        return
    end = s4.matching_brace(text, text.find("{", m.start()))
    end = text.find("\n", end) + 1
    text = text[:m.start()] + text[end:].lstrip("\n")
    text = re.sub(r"(?m)^[ \t]*seed\(\);\n", "", text)
    if not re.search(r"\bOnceLock\b", re.sub(r"(?m)^use [^;]*;\n", "", code_only(text))):
        text = text.replace("use std::sync::OnceLock;\n", "")
        text = text.replace("use std::sync::{Mutex, OnceLock};\n", "use std::sync::Mutex;\n")
    write(path, text)


def edit_core_sources(root: pathlib.Path) -> None:
    rel = "rust/src/lib.rs"
    edit(root, rel,
         ('#[cfg(feature = "iceberg")]\npub mod iceberg;\n#[cfg(not(feature = "iceberg"))]\n'
          '#[path = "iceberg/types.rs"]\npub mod iceberg;\n', "pub mod iceberg;\n"),
         ('#[cfg(feature = "s3tables")]\npub mod s3tables;\n', ""))
    text = read(root / rel)
    m = re.search(r"pub use protocol::\{[^}]*\};", text)
    if m and "IcebergField" in m.group(0):
        stmt = re.sub(r"\bIcebergField, IcebergFieldMut,\s*", "", m.group(0))
        text = text[:m.start()] + stmt + text[m.end():]
        write(root / rel, text)
    # The core keeps the type vocabulary alone (D17): the official door is
    # the crate's (`official::primitive_into_official`).
    rel = "rust/src/iceberg.rs"
    if (root / rel).exists():
        text = read(root / rel)
        text = text.replace('#[cfg(feature = "iceberg")]\nuse iceberg_official::spec::PrimitiveType as OfficialPrimitiveType;\n', "")
        m = re.search(r'(?m)^#\[cfg\(feature = "iceberg"\)\]\nimpl PrimitiveType \{\n', text)
        if m:
            end = s4.matching_brace(text, text.find("{", m.start()))
            end = text.find("\n", end) + 1
            text = text[:m.start()] + text[end:].lstrip("\n")
        if "iceberg_official" in text or "into_official" in text:
            residue(rel, "the official Iceberg model is still named in the core's type vocabulary")
        write(root / rel, text)
    # The view leaves the well-known protocols; `Scheme::ICEBERG` stays.
    if ICEBERG_EMIT in read(root / "rust/src/metadata.rs"):
        edit(root, "rust/src/metadata.rs", (ICEBERG_EMIT, ""))
        rel = "rust/tests/root/protocol.rs"
        text = read(root / rel)
        m = re.search(r'probes\.len\(\), (\d+), "the core\'s own protocol views"', text)
        if m:
            text = text[:m.start(1)] + str(int(m.group(1)) - 1) + text[m.end(1):]
            write(root / rel, text)
        else:
            residue(rel, "no count of the core's own protocol views to take the Iceberg view off")
    edit(root, "rust/src/metadata.rs",
         ("    /// assert_eq!(metadata.as_iceberg().get(\"doc\"), Some(\"closing price\"));\n"
          "    /// assert_eq!(metadata.as_iceberg().key(\"doc\"), \"ICEBERG:doc\");\n",
          "    /// assert_eq!(metadata.protocol(&Scheme::ICEBERG).key(\"doc\"), \"ICEBERG:doc\");\n"))
    edit(root, "rust/src/metadata/protocol.rs",
         ("/// [`Metadata::as_iceberg`] do.\n", "/// [`Metadata::as_glue`] do.\n"),
         ("/// use yggdryl::Metadata;\n", "/// use yggdryl::{Metadata, Scheme};\n"),
         ("/// let iceberg = metadata.as_iceberg();\n", "/// let iceberg = metadata.protocol(&Scheme::ICEBERG);\n"))
    text = read(root / "rust/src/protocol.rs")
    text = text.replace("`field.as_iceberg().name()` is E0716", "`field.as_glue().name()` is E0716")
    # The two examples read the generic view under `Scheme::ICEBERG`.
    for block in re.finditer(r"/// ```\n(?:///[^\n]*\n)*?/// ```\n", text):
        body = block.group(0)
        if "as_iceberg" not in body:
            continue
        new = body.replace(".as_iceberg_mut()", ".protocol_mut(&Scheme::ICEBERG)")
        new = new.replace(".as_iceberg()", ".protocol(&Scheme::ICEBERG)")
        new = new.replace("/// use yggdryl::DataType;\n", "/// use yggdryl::{DataType, Scheme};\n")
        text = text.replace(body, new, 1)
    if "as_iceberg" in text:
        residue("rust/src/protocol.rs", "an `as_iceberg` mention is left in the core's protocol docs")
    write(root / "rust/src/protocol.rs", text)
    # The two official doors and their pin leave with the crate.
    rel = "rust/src/error.rs"
    text = read(root / rel)
    for name in ("iceberg", "from_iceberg"):
        m = re.search(r'(?m)^    #\[cfg\(feature = "iceberg"\)\]\n    pub\(crate\) fn ' + name + r"\(", text)
        if m:
            end = s4.matching_brace(text, text.find("{", m.end()))
            end = text.find("\n", end) + 1
            text = text[:m.start()] + text[end:].lstrip("\n")
    m = re.search(r'(?m)^#\[cfg\(feature = "internals"\)\]\n#\[doc\(hidden\)\]\npub mod internals \{\n', text)
    if m:
        end = s4.matching_brace(text, text.find("{", m.end() - 2))
        body = text[m.end():end]
        if "invalid_iceberg_metadata" in body and body.count("pub fn ") == 1:
            text = text[:m.start()].rstrip("\n") + "\n" + text[end + 1:].lstrip("\n")
        else:
            residue(rel, "the `internals` module holds more than the Iceberg pin; its Iceberg half was not dropped")
    write(root / rel, text.rstrip("\n") + "\n")
    for rel in ("rust/src/media/format.rs", "rust/src/warehouse/catalog.rs", "rust/src/holder/locator.rs"):
        drop_seed(root, rel)
    edit(root, "rust/src/warehouse/catalog.rs",
         ("    claim_unseeded(factory, by)\n}\n\nfn claim_unseeded(factory: &'static dyn CatalogFactory, by: &'static str) -> Result<()> {\n",
          ""),
         ("//! states or for its scheme, claimed once on the register the core claims\n"
          "//! its own factories on before it answers anything.\n",
          "//! states or for its scheme, claimed once on the register by the crate that\n"
          "//! holds it - `yggdryl-iceberg`'s `install()` claims `hadoop` and, under its\n"
          "//! `s3tables` feature, the `s3tables` scheme.\n"))
    edit(root, "rust/src/warehouse/catalog.rs",
         ("    /// warehouse folder, PyIceberg's spelling, under the `iceberg` feature -\n",
          "    /// warehouse folder, PyIceberg's spelling, which `yggdryl-iceberg` claims -\n"),
         ("    /// bucket's `S3TablesCatalog` under the `s3tables` feature - and every\n",
          "    /// bucket's `S3TablesCatalog` under `yggdryl-iceberg`'s `s3tables` - and every\n"),
         optional=True)
    edit(root, "rust/src/uri/handle.rs",
         ("//! dispatcher itself states: under the `s3tables` feature a table an Amazon\n",
          "//! dispatcher itself states: with `yggdryl-iceberg`'s `s3tables` installed, a table an Amazon\n"),
         optional=True)
    edit(root, "rust/src/structure.rs",
         ("    /// (`iceberg::PartitionSpec::from_schema`, under the `iceberg` feature).\n",
          "    /// (`yggdryl_iceberg::PartitionSpec::from_schema`).\n"),
         optional=True)
    # Every remaining `iceberg` or `s3tables` gate of the core: what the crate
    # reaches through `implementer` made unconditional (the seeds, the only
    # arms nothing reaches, are gone above).
    gates = Gates(iceberg=True, s3tables=True)
    changed = 0
    for path in sorted((root / "rust/src").rglob("*.rs")):
        text = read(path)
        if not GATE_TERM.search(text):
            continue
        new = resolve_gates(text, str(path.relative_to(root)), gates)
        if new != text:
            write(path, new)
            changed += 1
    done(f"core: lib.rs, the type vocabulary, the protocol entry, error.rs, three seeds; {changed} file(s) un-gated")


def edit_core_manifest(root: pathlib.Path) -> None:
    rel = "rust/Cargo.toml"
    text = read(root / rel)
    text = text.replace("# with what it answers, and so does `s3tables`.\n",
                        "# with what it answers, and so does `yggdryl-iceberg`'s `s3tables`.\n")
    parquet_moved = not tracked(root, "rust/src/parquet")
    text = re.sub(r"(?m)^# Iceberg tables\.[^\n]*\n(?:#[^\n]*\n)*iceberg = \[[^\]]*\]\n", "", text)
    text = re.sub(r"(?m)^# Amazon S3 Tables:[^\n]*\n(?:#[^\n]*\n)*s3tables = \[[^\]]*\]\n", "", text)
    text = text.replace("# Metadata-only boundary: official Arrow 58 types never enter public APIs.\n", "")
    text = re.sub(r'(?m)^iceberg-official = \{[^\n]*\}\n', "", text)
    text = re.sub(r'(?m)^uuid = \{ version = "[^"]*", features = \["v7"\], optional = true \}\n', "", text)
    if parquet_moved:
        text = re.sub(r"(?m)^((?:#[^\n]*\n)*)parquet = \[\"dep:parquet\"\]\n", "", text)
        text = re.sub(r'(?m)^parquet = \{ version = "[^"]*", default-features = false, features = \[\n(?:    [^\n]*\n)*\], optional = true \}\n',
                      "", text)
        text = text.replace(
            "reference speed. The\n# `parquet` feature already compiles it, so a table build pays nothing new; the\n# four algorithm",
            "reference speed. The\n# four algorithm")
    else:
        residue(rel, "the core's Parquet medium is still the core's: its `parquet` feature and dependency stay "
                     "until `s6_parquet_move.py` runs")
    for old, new in (
        ("# Record media: the shared record groups, CSV, Excel and XML for Analysis\n"
         "# are unconditional, and the Iceberg groups compile only with that feature.\n"
         "# Avro's and Parquet's groups are their crates' own targets.\n",
         "# Record media: the shared record groups, CSV, Excel and XML for Analysis.\n"
         "# Avro's, Parquet's and Iceberg's groups are their crates' own targets.\n"),
        ("# Record media: Avro is unconditional; Parquet-backed I/O and Iceberg groups\n"
         "# compile only with their owning features.\n",
         "# Record media: Avro is unconditional and Parquet-backed I/O compiles only\n"
         "# with its feature; Iceberg's groups are `yggdryl-iceberg`'s own target.\n"),
    ):
        if old in text:
            text = text.replace(old, new, 1)
    m = re.search(r'(?m)^exclude = \[([^\]]*)\]\n', text)
    if m:
        if '"/iceberg"' not in m.group(1):
            text = text[:m.start()] + f'exclude = [{m.group(1)}, "/iceberg"]\n' + text[m.end():]
    else:
        text = re.sub(r'(?m)^(categories = [^\n]*\n)', r'\1exclude = ["/iceberg"]\n', text, count=1)
    for word in ("iceberg-official", "dep:uuid", "s3tables = ", "iceberg = ["):
        if word in text:
            residue(rel, f"`{word}` is still in the core's manifest")
    write(root / rel, text)
    done("core manifest: the `iceberg` and `s3tables` features, `iceberg-official` and `uuid` gone")


def edit_core_prose(root: pathlib.Path) -> None:
    """The core's sentences that named its own Iceberg module."""
    edit(root, "python/yggdryl/logging.py",
         ("host: a record raised in `yggdryl::iceberg::table` is handled by\n",
          "host: a record raised in `yggdryl_iceberg::table` is handled by\n"), optional=True)
    for rel in ("rust/src/logging/record.rs", "rust/src/logging/facade.rs", "rust/src/logging/mod.rs"):
        path = root / rel
        if path.exists():
            text = read(path)
            new = text.replace("`yggdryl::iceberg::table`", "`yggdryl::media::partition`").replace(
                "`yggdryl.iceberg.table`", "`yggdryl.media.partition`")
            if new != text:
                write(path, new)
    edit(root, "rust/src/xxhash/mod.rs",
         ("//! other reader would find them. [`crate::iceberg`] never calls this module for\n//! partitioning.\n",
          "//! other reader would find them. `yggdryl-iceberg` never calls this module\n//! for partitioning.\n"),
         optional=True)


# ---------------------------------------------------------------------------
# Step 6: paths
# ---------------------------------------------------------------------------

TEXT_SUFFIXES = av.TEXT_SUFFIXES
ICEBERG_NAMED = re.compile(r"\byggdryl::(?:iceberg::(?!PrimitiveType\b)|s3tables\b|internals::(?:iceberg|s3tables)_|"
                           r"internals::error::invalid_iceberg|\{[^;]*\b(?:iceberg|s3tables)\b)")


class Paths(IcebergTables):
    """The resolution, with the one core pin that leaves by name: the
    official failure's, now the crate's `official` internals."""

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if segs[:3] == ["internals", "error", "invalid_iceberg_metadata"]:
            return ICEBERG, ["internals", "official", "invalid_iceberg_metadata", *segs[3:]]
        return super().resolve(segs, routed)


def markdown(rewriter, text: str, where: str) -> str:
    """S4's markdown rewrite over the Rust blocks and the prose naming the
    crate's paths; a block whose lines do not all carry its indent is left
    for a hand."""
    outside = s4.Context(None, False)
    out: list[str] = []
    pos = 0
    for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
        prose = text[pos:m.start(3)]
        out.append(rewriter.inline(prose, outside, where) if ICEBERG_NAMED.search(prose) else prose)
        body = m.group(3)
        if m.group(2).strip().startswith("rust") and ICEBERG_NAMED.search(body):
            indent = m.group(1)
            lines = body.split("\n")
            if indent and not all(l.startswith(indent) or not l.strip() for l in lines):
                residue(f"{where}:{line_of(text, m.start(3))}", "a Rust block with a line outside its indent names Iceberg: re-spell it by hand")
            elif indent:
                stripped = "\n".join(l[len(indent):] for l in lines)
                new = rewriter.rust(stripped, outside, where)
                body = "\n".join((indent + l) if l else l for l in new.split("\n"))
            else:
                body = rewriter.rust(body, outside, where)
        elif ICEBERG_NAMED.search(body):
            body = rewriter.inline(body, outside, where)
        out.append(body)
        pos = m.end(3)
    rest = text[pos:]
    out.append(rewriter.inline(rest, outside, where) if ICEBERG_NAMED.search(rest) else rest)
    return "".join(out)


def rewrite_paths(root: pathlib.Path, rewriter, paths: PathMap, moves, splits) -> None:
    rewriter.t.core = CorePublic(root)
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new.setdefault(new, old)
    crate_ctx = s4.Context(ICEBERG, True)
    outside = s4.Context(None, False)
    count = 0
    local = paths.with_files(dict(splits))
    moved_targets = {new for _, new in moves}
    split_only = {new for _, new in splits} - moved_targets
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        old = moved_new.get(rel, rel)
        text = read(path)
        # A split's output was anchored where it was written.
        new = text if rel in split_only else s4.reanchor(text, old, rel, local)
        ctx = crate_ctx if rel.startswith(f"{CRATE_DIR}/src/") else outside
        rewriter.t.inside = ctx is crate_ctx
        rewriter.t.crate_file = True
        new = rewriter.rs_file(new, ctx, rel)
        rewriter.t.inside = False
        rewriter.t.crate_file = False
        if ctx is crate_ctx:
            # A link to another crate's hidden door is a name, never a link.
            new = re.sub(r"\[`((?:yggdryl|yggdryl_avro|yggdryl_parquet|yggdryl_s3)::implementer::\w+)`\]", r"`\1`", new)
        if new != text:
            write(path, new)
            count += 1
    leaves = [f"rust/{leaf}" for leaf in sibling_leaves(root)]
    for folder in ("rust/src", "rust/tests", "rust/benchmarks", "python/src", "node/src", "cli", *leaves):
        base = root / folder
        if not base.exists():
            continue
        for path in sorted(base.rglob("*.rs")):
            rel = str(path.relative_to(root))
            if "/target/" in rel or rel.startswith("rust/tests/logging/"):
                continue
            text = read(path)
            if folder == "rust/src":
                new = s4.reanchor(text, rel, rel, paths)
            else:
                new = rewriter.rs_file(s4.reanchor(text, rel, rel, paths), outside, rel)
            if new != text:
                write(path, new)
                count += 1
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text = read(root / f)
            if not ICEBERG_NAMED.search(text):
                continue
            new = markdown(rewriter, text, f)
            if new != text:
                write(root / f, new)
                count += 1
    for f in ("AGENTS.md", "README.md", "rust/README.md", ".api-inventory.txt", ".api-bindings.txt"):
        if (root / f).exists():
            text = read(root / f)
            new = rewriter.inline(text, outside, f)
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


REDIRECTED_NAMES = {"Handle", "ArnPartition", "Holder", "Cadence", "ArrowCastPlan", "ArrowCastOptions", "Session",
                    "Deferred", "Derivation", "Arn", "Uri", "RecordOptions"}


def drop_unused_names(root: pathlib.Path) -> None:
    """The private imports the method redirects left unused: a name one of
    them imported that the file's code no longer spells."""
    changed = 0
    for path in sorted((root / CRATE_DIR / "src").rglob("*.rs")):
        text = read(path)
        head, items = top_items(text)
        code = "".join(code_only(item.text) for item in items if item.kind != "use")
        spelled = set(re.findall(r"[A-Za-z_]\w*", code))
        out, pos, dropped = [head], len(head), False
        for item in items:
            out.append(text[pos:item.start])
            pos = item.end
            stripped = item.text.lstrip()
            if item.kind != "use" or stripped.startswith(("pub", "#")) or "use " not in item.text:
                out.append(item.text)
                continue
            try:
                leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
            except UseParseError:
                out.append(item.text)
                continue
            keep = [leaf for leaf in leaves
                    if not (leaf[2] == "self" and leaf[0] and (leaf[1] or leaf[0][-1]) in REDIRECTED_NAMES
                            and (leaf[1] or leaf[0][-1]) not in spelled)]
            if len(keep) == len(leaves):
                out.append(item.text)
                continue
            dropped = True
            if not keep:
                continue
            m = re.search(r"(?m)^([ \t]*)use\b", item.text)
            lead = m.group(1) if m else ""
            groups: dict[str, list] = collections.OrderedDict()
            for leaf_path, alias, kind in keep:
                groups.setdefault(leaf_path[0], []).append((leaf_path[1:], alias, kind))
            out.append((item.text[:m.start()] if m else "")
                       + "\n".join(f"{lead}use {r};" for word, its in groups.items() for r in render_use(word, its)))
        out.append(text[pos:])
        if dropped:
            write(path, "".join(out))
            changed += 1
    done(f"imports: {changed} crate file(s) lose the names the redirects left unused")


def resolve_crate_gates(root: pathlib.Path) -> None:
    """Inside the crate `iceberg` always holds, and the object stores are
    what its `s3tables` feature links (D36.6): an `s3` gate is that one."""
    changed = 0
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        text = read(path)
        new = re.sub(r'feature\s*=\s*"s3"', 'feature = "s3tables"', text)
        new = new.replace('all(feature = "s3tables", feature = "s3tables")', 'feature = "s3tables"')
        new = resolve_gates(new, rel, CRATE_GATES)
        if new != text:
            write(path, new)
            changed += 1
    done(f"crate gates: `iceberg` resolved and `s3` read as `s3tables` in {changed} file(s)")


# ---------------------------------------------------------------------------
# The view: every `as_iceberg` site
# ---------------------------------------------------------------------------

AS_ICEBERG = re.compile(r"\.(\s*)as_iceberg(_mut)?\(\)")


def receiver_start(text: str, dot: int) -> int:
    """Where the receiver of the method call whose `.` is at `dot` starts: a
    path, calls, indexing and `?`, never an operator."""
    k = dot
    while k > 0 and text[k - 1] in " \t\n":
        k -= 1
    while k > 0:
        c = text[k - 1]
        if c in ")]":
            depth, m = 0, k - 1
            while m >= 0:
                if text[m] in ")]":
                    depth += 1
                elif text[m] in "([":
                    depth -= 1
                    if depth == 0:
                        break
                m -= 1
            k = m
            continue
        if c.isalnum() or c in "_?":
            k -= 1
            continue
        if c == ":" and text[k - 2:k] == "::":
            k -= 2
            continue
        if c == ".":
            j = k - 1
            while j > 0 and text[j - 1] in " \t\n":
                j -= 1
            if j > 0 and (text[j - 1].isalnum() or text[j - 1] in "_)]?"):
                k = j
                continue
        break
    return k


def owned_binding(text: str, at: int, name: str) -> bool | None:
    """Whether the binding `name` reaching `at` holds a field (`True`), a
    `&mut Field` (`False`), or neither can be told (`None`)."""
    before = text[:at]
    owned = [m.start() for m in re.finditer(rf"\bmut\s+{name}\b", before)]
    borrowed = [m.start() for m in re.finditer(
        rf"\b{name}\s*:\s*&(?:'\w+\s+)?mut\b|\blet\s+{name}\s*=\s*&mut\b|\b(?:Some|Ok)\(\s*{name}\s*\)|"
        rf"\bfor\s+{name}\s+in\b|\|\s*{name}\s*\||\|\s*{name}\s*,|,\s*{name}\s*\|", before)]
    last_owned = max(owned, default=-1)
    last_borrowed = max(borrowed, default=-1)
    if last_owned < 0 and last_borrowed < 0:
        return None
    return last_owned > last_borrowed


def sweep_text(text: str, where: str, form: str, crate_path: str) -> tuple[str, int]:
    """Every `.as_iceberg()` and `.as_iceberg_mut()` call in `text`: the
    generic view under `Scheme::ICEBERG` (`form == "generic"`), or the
    crate's view built from the field (`form == "view"`)."""
    out: list[str] = []
    pos = 0
    count = 0
    # Spelled whole: an inline module of a test file imports for itself.
    scheme = "yggdryl::Scheme::ICEBERG"
    for m in AS_ICEBERG.finditer(text):
        if m.start() < pos:
            continue
        mut = bool(m.group(2))
        if form == "generic":
            out.append(text[pos:m.start()])
            out.append(f".{m.group(1)}protocol{'_mut' if mut else ''}(&{scheme})")
            pos = m.end()
            count += 1
            continue
        start = receiver_start(text, m.start())
        recv = text[start:m.start()].rstrip()
        if not recv:
            residue(f"{where}:{line_of(text, m.start())}", "an `as_iceberg` call with no receiver the sweep reads")
            continue
        flat = re.sub(r"\s+", "", recv)
        view = f"{crate_path}::IcebergField{'Mut' if mut else ''}::new"
        if flat.endswith("]"):
            arg = f"&mut {recv}" if mut else f"&{recv}"
        elif flat.endswith((")", "?")):
            arg = recv
        elif mut:
            name = flat.rsplit(".", 1)[-1] if "." in flat else flat
            if "." in flat:
                arg = f"&mut {recv}"
            else:
                held = owned_binding(text, start, name)
                arg = recv if held is False else f"&mut {recv}"
                if held is None:
                    residue(f"{where}:{line_of(text, m.start())}",
                            f"`{name}.as_iceberg_mut()` read as a field the binding owns (`&mut {name}`); "
                            "the compiler says if it is a `&mut Field`")
        else:
            arg = f"&{recv}"
        out.append(text[pos:start])
        out.append(f"{view}({arg})")
        pos = m.end()
        count += 1
    out.append(text[pos:])
    return "".join(out), count


SWEEP_VIEW = (f"{CRATE_DIR}/", "docs/media/iceberg.md", "skills/yggdryl-records/", "python/", "node/", "cli/")


def sweep_as_iceberg(root: pathlib.Path) -> None:
    files = [f for f in av.text_files(root) if f.endswith((".rs", ".md"))]
    total = 0
    for f in files:
        if f.startswith((".handoff/",)) or f == "AGENTS.md":
            continue
        text = read(root / f)
        if "as_iceberg" not in text:
            continue
        view = f.startswith(SWEEP_VIEW)
        form = "view" if view else "generic"
        if f.startswith("rust/src/"):
            for m in AS_ICEBERG.finditer(text):
                residue(f"{f}:{line_of(text, m.start())}", "an `as_iceberg` call left in the core's sources")
            continue
        if f.endswith(".rs") and f.startswith(f"{CRATE_DIR}/src/"):
            # Code reads the crate's own path; a doc example is compiled
            # outside it.
            new, n = sweep_doc_and_code(text, f)
            total += n
        else:
            new, n = sweep_text(text, f, form, "yggdryl_iceberg")
            total += n
        if new != text:
            write(root / f, new)
    done(f"view: {total} `as_iceberg` calls swept")


def sweep_doc_and_code(text: str, where: str) -> tuple[str, int]:
    """A crate source: its code through `crate::IcebergField`, its doc
    examples through `yggdryl_iceberg::IcebergField`, and its prose links
    to the accessors the core no longer has re-pointed."""
    text = text.replace("[`Field::as_iceberg`]", "[`IcebergField::new`](crate::IcebergField::new)")
    text = text.replace("[`Field::as_iceberg_mut`]", "[`IcebergFieldMut::new`](crate::IcebergFieldMut::new)")
    out, count = [], 0
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        doc = lines[i].lstrip().startswith(("//!", "///"))
        j = i
        while j < len(lines) and lines[j].lstrip().startswith(("//!", "///")) == doc:
            j += 1
        chunk = "\n".join(lines[i:j])
        new, n = sweep_text(chunk, where, "view", "yggdryl_iceberg" if doc else "crate")
        out.append(new)
        count += n
        i = j
    return "\n".join(out), count


# ---------------------------------------------------------------------------
# Step 7: interop
# ---------------------------------------------------------------------------

INTEROP_HARNESS = """//! The Iceberg exchange with outside implementations: PyIceberg, which
//! `scripts/check_iceberg_interop.py` drives around this target, reading the
//! tables it writes under `target/iceberg-interop/` and writing the ones it
//! reads back.

#[path = "interop/iceberg.rs"]
mod iceberg;
"""


def edit_interop(root: pathlib.Path) -> None:
    rel = f"{CRATE_DIR}/tests/interop.rs"
    if not (root / rel).exists():
        write(root / rel, INTEROP_HARNESS)
    edit(root, f"{CRATE_DIR}/tests/interop/iceberg.rs",
         ("    // `CARGO_MANIFEST_DIR` is `rust/`; the workspace target directory is beside it.\n"
          "    let mut path = std::path::PathBuf::from(env!(\"CARGO_MANIFEST_DIR\"));\n"
          "    path.pop();\n",
          "    // `CARGO_MANIFEST_DIR` is `rust/iceberg/`; the workspace target directory\n"
          "    // is beside `rust/`.\n"
          "    let mut path = std::path::PathBuf::from(env!(\"CARGO_MANIFEST_DIR\"));\n"
          "    path.pop();\n    path.pop();\n"))
    rel = "scripts/check_iceberg_interop.py"
    if (root / rel).exists():
        text = read(root / rel)
        new = re.sub(r'(\[\s*"cargo",\s*"test",\s*"--locked",)(\s*)("--features",\s*"[^"]*",\s*)?("--test",\s*"interop")',
                     lambda m: m.group(1) + m.group(2) + '"-p",' + m.group(2) + '"yggdryl-iceberg",' + m.group(2) + m.group(4),
                     text)
        new = new.replace("cargo test --locked --features iceberg --test interop", "cargo test --locked -p yggdryl-iceberg --test interop")
        new = new.replace('cargo test --locked --features "parquet iceberg" --test interop', "cargo test --locked -p yggdryl-iceberg --test interop")
        if new == text or "yggdryl-iceberg" not in new:
            residue(rel, "the driver's cargo line was not re-pointed at `-p yggdryl-iceberg`")
        write(root / rel, new)
    done("interop: the crate's `interop` target, the exchange kept at `target/iceberg-interop`, the driver re-pointed")


# ---------------------------------------------------------------------------
# Step 8: install
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = """//! The claim every harness of `yggdryl-iceberg` makes before a test reads a
//! table: the core's registers answer the Iceberg table format, its catalog
//! and the Amazon S3 Tables locations only once `yggdryl-iceberg` has claimed
//! them (D7), so each test opens with [`installed`].

/// Claims the table format, its catalog and the crates it reads, once for
/// the process.
pub fn installed() {
    yggdryl_iceberg::install().expect("yggdryl-iceberg claims its table format");
}
"""

INSTALL_TESTS = """
/// `install()` claims the table format and the `hadoop` catalog factory for
/// `yggdryl-iceberg`, once: a later call returns at once from any thread, and
/// the claims stand against another crate's, naming this one (D7, D39).
#[test]
fn install_claims_the_table_format_and_its_catalog_once() {
    crate::install::installed();
    let workers: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| yggdryl_iceberg::install().is_ok()))
        .collect();
    for worker in workers {
        assert!(worker.join().expect("an install thread"), "a later install returns at once");
    }
    let format: &dyn yggdryl::media::TableFormat = &yggdryl_iceberg::ICEBERG_FORMAT;
    let held = yggdryl::media::format::format_named("iceberg").expect("the format is claimed");
    assert!(std::ptr::addr_eq(held, format));
    let factory: &dyn yggdryl::CatalogFactory = &yggdryl_iceberg::HADOOP_FACTORY;
    assert!(
        yggdryl::warehouse::factories()
            .into_iter()
            .any(|claimed| std::ptr::addr_eq(claimed, factory)),
        "the hadoop factory is claimed"
    );
}

/// A second claim of the table format or of the `hadoop` factory is refused
/// naming `yggdryl-iceberg`, the crate that holds it.
#[test]
fn a_second_claim_of_the_table_format_is_refused_naming_yggdryl_iceberg() {
    crate::install::installed();
    let refused = yggdryl::media::format::claim(&yggdryl_iceberg::ICEBERG_FORMAT, "another")
        .expect_err("a second claim of the format is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-iceberg"), "{refused}");
    let refused = yggdryl::warehouse::claim_factory(&yggdryl_iceberg::HADOOP_FACTORY, "another")
        .expect_err("a second claim of the factory is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-iceberg"), "{refused}");
}
"""

S3TABLES_INSTALL_TESTS = """
/// Under `s3tables`, `install()` claims the table bucket's catalog factory and
/// the locator of `s3tables:` locations for `yggdryl-iceberg`, and a second
/// claim of either is refused naming it (D7, D39).
#[test]
fn install_claims_the_table_bucket_factory_and_locator_once() {
    crate::install::installed();
    yggdryl_iceberg::install().expect("a later install returns at once");
    let factory: &dyn yggdryl::CatalogFactory = &yggdryl_iceberg::s3tables::S3TABLES_FACTORY;
    assert!(
        yggdryl::warehouse::factories()
            .into_iter()
            .any(|claimed| std::ptr::addr_eq(claimed, factory)),
        "the table bucket's factory is claimed"
    );
    let locator: &dyn yggdryl::holder::Locator = &yggdryl_iceberg::s3tables::S3TABLES_LOCATOR;
    assert!(
        yggdryl::holder::locators()
            .into_iter()
            .any(|claimed| std::ptr::addr_eq(claimed, locator)),
        "the locator is claimed"
    );
    let refused = yggdryl::holder::claim_locator(&yggdryl_iceberg::s3tables::S3TABLES_LOCATOR, "another")
        .expect_err("a second claim of the locator is refused");
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains("yggdryl-iceberg"), "{refused}");
}
"""

VIEW_TESTS = """
/// The Iceberg view is the crate's, minted over `Scheme::ICEBERG` (D39): a
/// field borrows it with `IcebergField::new`, and what it writes is the
/// field's `ICEBERG:` metadata, which the core's generic view reads back.
#[test]
fn the_iceberg_view_is_built_from_the_field_and_reads_its_iceberg_keys() {
    crate::install::installed();
    let mut field = yggdryl::DataType::Int64.required_field("id");
    yggdryl_iceberg::IcebergFieldMut::new(&mut field)
        .set_spec_id(7)
        .expect("a spec id");
    assert_eq!(yggdryl_iceberg::IcebergField::new(&field).spec_id().expect("a spec id"), Some(7));
    assert_eq!(
        field.protocol(&yggdryl::Scheme::ICEBERG).as_field().name(),
        "id"
    );
    assert!(field.get_metadata("ICEBERG:spec-id").is_some());
}
"""


def markdown_installs(text: str) -> tuple[str, int]:
    """A Rust block naming the crate or an Iceberg table installs it first."""
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or f"{CRATE}::install()" in body:
            return m.group(0)
        if not re.search(r"\byggdryl_iceberg\b|\bs3tables\b|[Ii]ceberg", body):
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


def install_everywhere(root: pathlib.Path) -> None:
    base = root / CRATE_DIR
    write(base / "tests/support/install.rs", INSTALL_SUPPORT)
    harnesses = tests = 0
    for path in sorted((base / "tests").glob("*.rs")):
        text = s4.declare_install(read(path))
        write(path, re.sub(r'\n{3,}(#\[path = "support/install\.rs"\])', r"\n\n\1", text, count=1))
        harnesses += 1
    for rel, marker, extra in (
        ("tests/iceberg/mod_.rs", "install_claims_the_table_format_and_its_catalog_once", INSTALL_TESTS),
        ("tests/s3tables/mod_.rs", "install_claims_the_table_bucket_factory_and_locator_once", S3TABLES_INSTALL_TESTS),
        ("tests/iceberg/field.rs", "the_iceberg_view_is_built_from_the_field_and_reads_its_iceberg_keys", VIEW_TESTS),
    ):
        path = base / rel
        if path.exists() and marker not in read(path):
            write(path, read(path).rstrip("\n") + "\n" + extra)
        elif not path.exists():
            residue(f"{CRATE_DIR}/{rel}", "absent: its added tests were not written")
    for path in sorted((base / "tests").rglob("*.rs")):
        if "/support/" in str(path):
            continue
        text, n = s4.insert_test_installs(read(path))
        tests += n
        write(path, text)
    docs = 0
    for path in sorted((base / "src").rglob("*.rs")):
        text, n = s4.insert_doc_installs(read(path), ICEBERG)
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
    done(f"install: {harnesses} harnesses, {tests} tests, {docs} doc examples, {pages} page blocks, "
         "the bench main, the bindings and the CLI")


def install_at_init(root: pathlib.Path) -> None:
    """The bindings' init and the CLI's `main` claim the table format after
    what they claim before it, in dependency order (D39)."""
    rel = "python/src/lib.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        anchor = "    register_classes(module)?;\n"
        line = f"    {CRATE}::install().map_err(value_error)?;\n"
        if "The crates split off the core claim what they register" not in text:
            line = ("    // The crates split off the core claim what they register before a\n"
                    "    // class can read a name of theirs, in dependency order.\n" + line)
        write(root / rel, replace_once(text, anchor, line + anchor, rel))
    rel = "node/src/lib.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        m = re.search(r"(?s)#\[napi_derive::module_init\]\nfn install_logging\(\) \{\n.*?\n(\}\n)", text)
        if not m:
            residue(rel, "no `module_init` function to install the table format in")
        else:
            line = f'    {CRATE}::install().expect("{PACKAGE} claims its table format");\n'
            if "The crates split off the core claim what they register" not in m.group(0):
                line = ("    // The crates split off the core claim what they register before an\n"
                        "    // export can read a name of theirs, in dependency order; a refusal is\n"
                        "    // a build linking two claimants, which no caller can repair.\n" + line)
            write(root / rel, text[:m.start(1)] + line + text[m.start(1):])
    rel = "cli/src/main.rs"
    text = read(root / rel)
    if f"{CRATE}::install()" not in text:
        anchor = "    let cli = Cli::parse();\n"
        block = (
            "    // Iceberg tables are linked where a feature asks for them: the\n"
            "    // default build stays the schema-only core.\n"
            '    #[cfg(feature = "iceberg")]\n'
            f"    if let Err(refusal) = {CRATE}::install() {{\n"
            "        style::bad(&refusal.to_string());\n"
            "        return ExitCode::FAILURE;\n"
            "    }\n"
        )
        write(root / rel, replace_once(text, anchor, block + anchor, rel))


# ---------------------------------------------------------------------------
# Step 9: manifests
# ---------------------------------------------------------------------------

LIB_IDENTS = {"md5": "md-5", "iceberg_official": "iceberg-official", "object_store": "object_store"}
SIBLINGS = {"yggdryl_avro": "yggdryl-avro", "yggdryl_parquet": "yggdryl-parquet", "yggdryl_s3": "yggdryl-s3",
            "yggdryl_market": "yggdryl-market", "yggdryl_fix": "yggdryl-fix", "yggdryl_excel": "yggdryl-excel",
            "yggdryl_xmla": "yggdryl-xmla"}


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
    skip = {"yggdryl", CRATE, "crate", "self", "super", "std", "core", "alloc", *SIBLINGS}
    used = s4.crates_named(root, [f"{CRATE_DIR}/src"])
    used_dev = s4.crates_named(root, [f"{CRATE_DIR}/tests", f"{CRATE_DIR}/benchmarks"]) - used
    lines = ["yggdryl.workspace = true", "yggdryl-avro.workspace = true", "yggdryl-parquet.workspace = true",
             "yggdryl-s3 = { workspace = true, optional = true }"]
    for name in sorted(used - skip):
        if name in by_ident:
            lines.append(av.dependency_line(*by_ident[name]))
    if "parquet" in used and "parquet" not in by_ident:
        residue(f"{CRATE_DIR}/Cargo.toml", "the `parquet` dependency's spec was not found in the core's manifest")
    dev_lines = []
    for name in sorted(used_dev - skip):
        if name in by_ident:
            dev_lines.append(av.dependency_line(*by_ident[name]))
        elif name in dev_by_ident:
            dev_lines.append(av.dependency_line(*dev_by_ident[name]))
    if not any(l.startswith("criterion") for l in dev_lines):
        dev_lines.append(av.dependency_line("criterion", dev.get("criterion", "0.7")))
    for sibling in ("yggdryl_market", "yggdryl_fix", "yggdryl_excel", "yggdryl_xmla"):
        if sibling in used_dev:
            dev_lines.append(f"{SIBLINGS[sibling]}.workspace = true")
    core_features = core_manifest.get("features", {})
    found: set[str] = set()
    for path in (root / CRATE_DIR).rglob("*.rs"):
        found |= set(re.findall(r'feature\s*=\s*"([\w-]+)"', no_comments(read(path))))
    found -= {"internals", "iceberg", "s3tables", "parquet"}
    unknown = sorted(found - set(core_features))
    if unknown:
        residue(f"{CRATE_DIR}/Cargo.toml", f"the crate's code reads features the core does not have: {unknown}")
    features = [("default", []), ("s3tables", ["dep:yggdryl-s3", "yggdryl/aws"])]
    for name in sorted(found & set(core_features)):
        implied = [g for g in core_features[name] if g in found]
        features.append((name, [*implied, f"yggdryl/{name}"]))
    features.append(("internals", ["yggdryl/internals"]))
    feature_text = "\n".join(f"{name} = {av.toml_value(value)}" for name, value in features)
    manifest = (
        "[package]\n"
        f'name = "{PACKAGE}"\n'
        'description = "Apache Iceberg tables for yggdryl: metadata, manifests, scans, commits and catalogs over the core\'s handles, and Amazon S3 Tables under its `s3tables` feature"\n'
        "version.workspace = true\n"
        "edition.workspace = true\n"
        "rust-version.workspace = true\n"
        "license.workspace = true\n"
        "repository.workspace = true\n"
        'readme = "README.md"\n'
        'keywords = ["iceberg", "arrow", "table", "lakehouse", "catalog"]\n'
        'categories = ["database", "encoding"]\n'
        "\n[features]\n"
        "# Amazon S3 Tables: the control plane of a table bucket - its namespaces,\n"
        "# its tables, and the metadata location each table's commits move under a\n"
        "# version token. Not default: it links the object stores a table's files\n"
        "# live in (`yggdryl-s3`) and the AWS identity it signs with. The other\n"
        "# gates are the core's that this crate's tests and benchmarks read,\n"
        "# forwarded; `internals` makes `yggdryl_iceberg::internals` exist for\n"
        "# `tests/` and turns the core's on.\n"
        + feature_text + "\n"
        "\n[dependencies]\n" + "\n".join(lines) + "\n"
        "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
        "\n# Planning, metadata, manifests and the partition renderer, the Iceberg\n"
        "# protocol view and, under `s3tables`, a table bucket's location; `main`\n"
        "# installs the crate first.\n"
        '[[bench]]\nname = "iceberg"\npath = "benchmarks/iceberg.rs"\nharness = false\n'
    )
    write(root / CRATE_DIR / "Cargo.toml", manifest)
    readme = root / CRATE_DIR / "README.md"
    if not readme.exists():
        write(
            readme,
            f"# {PACKAGE}\n\nApache Iceberg tables for yggdryl: metadata, manifests, scans, commits and "
            "catalogs over the core's handles, and Amazon S3 Tables under its `s3tables` feature.\n\n"
            "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the project's "
            "site, and `install()` claims the table format, its catalog and, under `s3tables`, the table "
            "bucket's factory and locator on the core's registers.\n",
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
            tail = list(re.finditer(r'(?m)^yggdryl(?:-[a-z0-9]+)? = \{ path = "rust[^\n]*\n', ws))[-1]
            ws = ws[:tail.end()] + f'{PACKAGE} = {{ path = "{CRATE_DIR}", version = "={version}" }}\n' + ws[tail.end():]
        else:
            ws = replace_once(
                ws, "[workspace.dependencies]\n",
                "[workspace.dependencies]\n"
                "# The workspace's own crates, pinned to the one version every artifact\n"
                "# carries, so a published crate names exactly the core it was built with.\n"
                f'yggdryl = {{ path = "rust", version = "={version}" }}\n'
                f'{PACKAGE} = {{ path = "{CRATE_DIR}", version = "={version}" }}\n',
                "Cargo.toml",
            )
    if not leaf_exists(root, "s3") and "yggdryl-s3 = {" not in ws:
        residue("Cargo.toml", "`yggdryl-s3` is no workspace dependency yet: the crate's `s3tables` names it "
                              "once `s6_s3_move.py` has run")
    write(root / "Cargo.toml", ws)
    # The bindings link the crate with S3 Tables; the CLI behind its own
    # `iceberg` and `s3tables` features.
    for member in ("python", "node"):
        rel = f"{member}/Cargo.toml"
        text = read(root / rel)
        if PACKAGE in text:
            continue
        m = re.search(r'(?ms)^yggdryl = \{ path = "\.\./rust", default-features = false, features = \[\n(.*?)\] \}\n', text)
        if m:
            body = re.sub(r'(?m)^\s*"(?:iceberg|s3tables)",\n', "", m.group(1))
            text = text[:m.start(1)] + body + text[m.end(1):]
        else:
            residue(rel, "no `yggdryl` feature list to take `iceberg` and `s3tables` out of")
        m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:yggdryl-[a-z0-9]+(?:\.workspace)?(?: = [^\n]*)?\n)*", text)
        if not m:
            residue(rel, "no `yggdryl` dependency line to place the crate after")
            continue
        text = text[:m.end()] + f'{PACKAGE} = {{ workspace = true, features = ["s3tables"] }}\n' + text[m.end():]
        write(root / rel, text)
    rel = "cli/Cargo.toml"
    text = read(root / rel)
    if PACKAGE not in text:
        m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:yggdryl-[a-z0-9]+(?:\.workspace)?(?: = [^\n]*)?\n)*", text)
        if m:
            text = text[:m.end()] + f"{PACKAGE} = {{ workspace = true, optional = true }}\n" + text[m.end():]
        else:
            residue(rel, "no `yggdryl` dependency line to place the crate after")
        text = re.sub(r'(?m)^(iceberg = \[[^\]]*)"yggdryl/iceberg"', r'\1"dep:yggdryl-iceberg"', text)
        text = text.replace('"yggdryl/s3tables"', f'"{PACKAGE}/s3tables"')
        text = text.replace("# `yggdryl-s3`, linked and installed at `main`, and the core's table-bucket\n# catalog.",
                            "# `yggdryl-s3`, linked and installed at `main`, and the table-bucket catalog\n# of `yggdryl-iceberg`.")
        text = text.replace("# `yggdryl xmla serve` and `yggdryl market serve` over an Iceberg folder need the\n"
                            "# core's table reader,", "# `yggdryl xmla serve` and `yggdryl market serve` over an Iceberg folder need the\n"
                            "# table reader of `yggdryl-iceberg`,")
        dev = ("# The pages' Rust blocks name Iceberg tables whatever the CLI's own\n"
               "# features, so the docs runner and the CLI's tests link the crate.\n"
               f'{PACKAGE} = {{ workspace = true, features = ["s3tables"] }}\n')
        if "[dev-dependencies]\n" in text:
            text = text.replace("[dev-dependencies]\n", "[dev-dependencies]\n" + dev, 1)
        else:
            text = text.replace("\n[features]\n", "\n[dev-dependencies]\n" + dev + "\n[features]\n", 1)
        if '"yggdryl/iceberg"' in text or '"yggdryl/s3tables"' in text:
            residue(rel, "a feature still names the core's `iceberg` or `s3tables`")
        write(root / rel, text)
    # A sibling leaf forwarding the core's `iceberg` or `s3tables`, which its
    # code no longer reads once the tests that did moved here.
    for leaf in sibling_leaves(root):
        rel = f"rust/{leaf}/Cargo.toml"
        text = read(root / rel)
        reads = set()
        for path in (root / "rust" / leaf).rglob("*.rs"):
            reads |= set(re.findall(r'feature\s*=\s*"(iceberg|s3tables)"', no_comments(read(path))))
        for name in ("iceberg", "s3tables"):
            line = re.search(rf'(?m)^{name} = \[[^\]]*\]\n', text)
            if not line:
                continue
            if name in reads:
                residue(rel, f"the crate's code still reads `{name}`, which the core no longer has")
                continue
            text = text[:line.start()] + text[line.end():]
        write(root / rel, text)
    done("manifests: rust/iceberg, the workspace, the bindings, the CLI and the sibling leaves")


# ---------------------------------------------------------------------------
# The core's type vocabulary keeps its own pins (D17)
# ---------------------------------------------------------------------------

TYPES_ROOT_DOC = """//! `rust/src/iceberg.rs`: the Iceberg type vocabulary the core keeps (D17) -
//! `PrimitiveType` and the Iceberg type strings the datatype grammar reads.
//! The table contract over these types is `yggdryl-iceberg`'s, pinned in
//! `rust/iceberg/tests/iceberg/types.rs`.

"""

TYPES_CRATE_DOC = """//! The v3 type contract a table keeps: expression-driven scans, partition
//! keys as the primary keys, sorted data files, parallel partition writes,
//! and the v3 `unknown` and `variant` types, both read as the variant
//! column. `PrimitiveType` itself is the core's (`rust/src/iceberg.rs`,
//! D17), pinned by `rust/tests/root/iceberg.rs`.
//!
//! Everything here reaches the crates through `yggdryl::` and
//! `yggdryl_iceberg::`.
"""


ICEBERG_HARNESS_DOC = """//! One test file per file under `rust/iceberg/src/`, under `tests/iceberg/`.
//!
//! `rust/iceberg/tests/` mirrors `rust/iceberg/src/`: a source file has
//! exactly one test file at the matching path, and this target is the harness
//! for the crate's own files; `tests/s3tables/` is the `s3tables` feature's.
//! A test reaches the crate through `yggdryl_iceberg::`; where what it pins
//! is not reachable that way, it reaches `yggdryl_iceberg::internals`, which
//! exists only under the `internals` feature, and the file that reaches it is
//! declared behind that feature here.
"""


def finish_crate_text(root: pathlib.Path) -> None:
    """The crate's harnesses say what they are; its tests' prose names the
    crate's `internals`, not the core's."""
    base = root / CRATE_DIR / "tests"
    harness = base / "iceberg.rs"
    if harness.exists():
        text = read(harness)
        write(harness, re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", ICEBERG_HARNESS_DOC, text, count=1))
    for folder in ("iceberg", "s3tables"):
        for path in sorted((base / folder).rglob("*.rs")) if (base / folder).exists() else []:
            text = read(path)
            new = text.replace("`yggdryl::internals`", "`yggdryl_iceberg::internals`")
            if new != text:
                write(path, new)


def relocate_error_pin(root: pathlib.Path) -> None:
    """The core's pin of the official failure follows the door it pins, now
    `official.rs`'s `internals`: into `tests/iceberg/official.rs`."""
    src = root / CRATE_DIR / "tests/root/error.rs"
    dest = root / CRATE_DIR / "tests/iceberg/official.rs"
    if not src.exists() or not dest.exists():
        return
    incoming = read(src)
    head_end = re.match(r"(?:[ \t]*//![^\n]*\n)*", incoming).end()
    write(dest, merge_into(read(dest), incoming[head_end:]))
    src.unlink()
    harness = root / CRATE_DIR / "tests/root.rs"
    if harness.exists():
        text = read(harness)
        text = re.sub(r'(?m)^(?:#\[cfg[^\n]*\n)?#\[path = "root/error\.rs"\]\nmod error;\n', "", text)
        if not re.search(r'#\[path = "root/', text):
            harness.unlink()
        else:
            write(harness, text)
    done(f"{CRATE_DIR}/tests/iceberg/official.rs: the official failure's pin, beside the door it pins")


def split_back_types(root: pathlib.Path) -> None:
    """The moved `tests/iceberg/types.rs`, whose source stays the core's: the
    items naming nothing of the crate return to `rust/tests/root/iceberg.rs`."""
    rel = f"{CRATE_DIR}/tests/iceberg/types.rs"
    path = root / rel
    if not path.exists():
        return
    text = read(path)
    core_half, crate_half, moved, copied = split_level(text, True, set())
    if crate_half is None:
        residue(rel, "nothing in it reaches the crate: move it back to `rust/tests/root/iceberg.rs` by hand")
        return
    head_end = re.match(r"(?:[ \t]*//![^\n]*\n)*", crate_half).end()
    write(path, TYPES_CRATE_DOC + crate_half[head_end:].lstrip("\n"))
    _, items = top_items(core_half)
    if not any(re.search(r"#\[test\]", item.text) for item in items):
        return
    head_end = re.match(r"(?:[ \t]*//![^\n]*\n)*", core_half).end()
    body = resolve_gates(core_half[head_end:].lstrip("\n"), "rust/tests/root/iceberg.rs", Gates(True, False))
    write(root / "rust/tests/root/iceberg.rs", tidy_head(prune_imports(TYPES_ROOT_DOC + body)))
    harness = root / "rust/tests/root.rs"
    text = read(harness)
    decl = '#[path = "root/iceberg.rs"]\nmod iceberg;\n'
    if decl not in text:
        decls = list(re.finditer(r'(?m)^(?:#\[cfg[^\n]*\n)?#\[path = "root/(\w+)\.rs"\]\nmod \w+;\n', text))
        after = [m for m in decls if m.group(1) < "iceberg"]
        at = after[-1].end() if after else (decls[0].start() if decls else len(text))
        text = text[:at] + decl + text[at:]
        write(harness, text)
    done(f"{rel}: its items naming only the core's vocabulary back in rust/tests/root/iceberg.rs")


# ---------------------------------------------------------------------------
# Commands: the features the core no longer has
# ---------------------------------------------------------------------------

FEATURE_ARG = re.compile(r'(\s)--features(?:=|\s+)("([^"]*)"|[\w,/-]+)')


def strip_features(line: str, drop: set[str]) -> str:
    def sub(m: re.Match) -> str:
        words = [w for w in re.split(r"[\s,]+", m.group(3) if m.group(3) is not None else m.group(2)) if w]
        kept = [w for w in words if w not in drop]
        if kept == words:
            return m.group(0)
        if not kept:
            return ""
        value = f'"{" ".join(kept)}"' if len(kept) > 1 else kept[0]
        return f"{m.group(1)}--features {value}"

    return FEATURE_ARG.sub(sub, line)


def edit_commands(root: pathlib.Path) -> None:
    """Every command a page, a script or a workflow spells against the core
    with a feature it no longer has, and the moved targets' commands."""
    parquet_moved = not tracked(root, "rust/src/parquet")
    core_drop = {"iceberg", "s3tables"} | ({"parquet"} if parquet_moved else set())
    crate_drop = {"iceberg", "parquet"}
    count = 0
    for f in av.text_files(root):
        if f.startswith((".handoff/", "docs/assets/")):
            continue
        text = read(root / f)
        if "cargo" not in text:
            continue
        lines = text.split("\n")
        for k, line in enumerate(lines):
            if "cargo " not in line or "--features" not in line:
                continue
            if re.search(r"-p\s+yggdryl-iceberg\b", line):
                lines[k] = strip_features(line, crate_drop)
            elif re.search(r"-p\s+yggdryl(?![-\w])", line) or ("rust/Cargo.toml" in line and "-p yggdryl-" not in line):
                lines[k] = strip_features(line, core_drop)
        new = "\n".join(lines)
        if new != text:
            write(root / f, new)
            count += 1
    edit(root, f"{CRATE_DIR}/tests/scale_ulbridge.rs",
         ("//! YGGDRYL_SCALE_BYTES=21474836480 cargo test --release -p yggdryl \\\n"
          "//!     --test scale_ulbridge --features iceberg -- --ignored --nocapture\n",
          "//! YGGDRYL_SCALE_BYTES=21474836480 cargo test --release -p yggdryl-iceberg \\\n"
          "//!     --test scale_ulbridge -- --ignored --nocapture\n"), optional=True)
    edit(root, "docs/types/protocol.md",
         ("cargo bench --manifest-path rust/Cargo.toml --features iceberg --bench types -- '^value/iceberg_'",
          "cargo bench -p yggdryl-iceberg --bench iceberg -- '^value/iceberg_'"), optional=True)
    for rel in ("scripts/bench_avro_baseline.py", "scripts/check_iceberg_interop.py", "scripts/stage_cli.py"):
        path = root / rel
        if not path.exists():
            continue
        text = read(path)
        new = re.sub(r'"--features",(\s*)"(?:parquet iceberg|iceberg)",', r'"-p",\1"yggdryl-iceberg",', text)
        new = new.replace("``cargo test --features iceberg --test interop iceberg::``",
                          "``cargo test -p yggdryl-iceberg --test interop iceberg::``")
        new = new.replace("yggdryl::iceberg::IcebergTable", "yggdryl_iceberg::IcebergTable")
        if new != text:
            write(path, new)
            count += 1
    rel = "scripts/check_docs_examples.py"
    if (root / rel).exists():
        runner = read(root / rel)
        m = re.search(r'"--features", "([^"]*)"', runner)
        if m:
            words = [w for w in m.group(1).split() if w not in core_drop]
            runner = runner[:m.start(1)] + " ".join(words) + runner[m.end(1):]
            write(root / rel, runner)
        if '"--test", "docs_examples"' in runner and '"-p", "yggdryl-cli"' not in runner:
            residue(rel, "the Rust pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner "
                         "to `cli/tests/` (D13): a block naming `yggdryl_iceberg` compiles only there")
    done(f"commands: {count} files' cargo lines lose the core's `iceberg`, `s3tables`"
         + (" and `parquet`" if parquet_moved else "") + " features")


# ---------------------------------------------------------------------------
# Step 10: tooling
# ---------------------------------------------------------------------------


def edit_tooling(root: pathlib.Path) -> None:
    av.edit_tooling(root)
    rel = "rust/src/logging/facade.rs"
    if '("yggdryl_iceberg", "yggdryl.iceberg")' not in read(root / rel):
        text = read(root / rel)
        m = re.search(r"const CRATES: \[\(&str, &str\); (\d+)\] = \[\n((?:    \([^\n]*\),\n)+)\];", text)
        if not m:
            residue(rel, "no `CRATES` table to add `yggdryl_iceberg` to")
        else:
            rows = m.group(2).splitlines(keepends=True)
            rows.append('    ("yggdryl_iceberg", "yggdryl.iceberg"),\n')
            rows.sort(key=lambda row: re.search(r'"([^"]+)"', row).group(1))
            text = text[:m.start()] + f"const CRATES: [(&str, &str); {len(rows)}] = [\n" + "".join(rows) + "];" + text[m.end():]
            write(root / rel, text)
    rel = "rust/tests/logging/facade.rs"
    if (root / rel).exists() and '"yggdryl_iceberg::' not in read(root / rel):
        residue(rel, "the logging facade's test names no `yggdryl_iceberg` module path")
    rel = "scripts/stage_cli.py"
    if (root / rel).exists():
        text = read(root / rel)
        if "s3tables" not in text or "iceberg" not in text:
            residue(rel, "the staged command's features do not name `iceberg` and `s3tables`; it must link `yggdryl-iceberg`")
    done("tooling: the per-crate generators, the internals re-exports regenerated, the logging facade's row")


# ---------------------------------------------------------------------------
# Step 11: CI
# ---------------------------------------------------------------------------


def edit_ci(root: pathlib.Path) -> None:
    rel = ".github/ci/rows.toml"
    if not (root / rel).exists():
        residue(rel, "absent in this tree: list the `iceberg` leaf by hand")
        return
    text = read(root / rel)
    listed = set(re.findall(r'(?m)^(\w+) = \{ package = "yggdryl-', text))
    after = [leaf for leaf in ("avro", "parquet", "s3") if leaf in listed]
    for leaf in ("avro", "parquet", "s3"):
        if leaf not in listed:
            residue(rel, f"the `{leaf}` leaf is not listed yet: `iceberg` is `after` it once its define has run")
    line = ('iceberg = { package = "yggdryl-iceberg", '
            + (f'after = [{", ".join(chr(34) + leaf + chr(34) for leaf in after)}], ' if after else "")
            + 'msrv = "--features s3tables", jobs = ["pyiceberg-interop", "spark-interop", "iceberg-msrv"] }\n')
    m = re.search(r'(?m)^(?:# )?iceberg = \{ package = "yggdryl-iceberg"[^\n]*\n', text)
    if m:
        text = text[:m.start()] + line + text[m.end():]
    else:
        residue(rel, "no `iceberg` leaf line to flip on")
    text = text.replace('[rows.x-iceberg]\nextends = ["interop"]\n', '[rows.x-iceberg]\nextends = ["crate-iceberg"]\n')
    text = re.sub(r'(?m)^(full = \{[^\n]*allocations = \[)"test:allocations", "test:scale_ulbridge", ',
                  r'\1"test:allocations", ', text)
    if '"test:scale_ulbridge"' in text.split("[leaves]")[0]:
        residue(rel, "`test:scale_ulbridge` is still a shard of the core's: it is `yggdryl-iceberg`'s target")
    write(root / rel, text)
    rel = ".github/workflows/ci.yml"
    edit(
        root, rel,
        ("      # The workspace's one declared MSRV: the core, its tests and the\n"
         "      # official Iceberg boundary all build at it.\n",
         "      # The workspace's one declared MSRV: `yggdryl-iceberg` - the core\n"
         "      # beneath it - its tests and the official Iceberg boundary all build\n"
         "      # at it.\n"),
        ("        run: cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg\n",
         "        run: cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl-iceberg --all-targets --features s3tables\n"),
        ("        working-directory: rust\n        run: cargo test --locked --features iceberg --test interop --no-run\n",
         "        working-directory: rust\n        run: cargo test --locked -p yggdryl-iceberg --test interop --no-run\n"),
        ("    # Google, `iceberg` from `rust/` for PyIceberg - so the jobs that\n",
         "    # Google, `-p yggdryl-iceberg` for PyIceberg - so the jobs that\n"),
        optional=True,
    )
    if "--features iceberg" in read(root / rel):
        residue(rel, "a step still passes `--features iceberg`")
    # The PyIceberg exchange's Rust half is the crate's own target now: a
    # change to the core's `interop` target no longer reaches it.
    rel = "scripts/tests/test_ci_plan.py"
    if (root / rel).exists():
        text = read(root / rel)
        m = re.search(r'(?ms)(    def test_an_exchange_half_runs_every_exchange\(self\) -> None:\n.*?)(?=\n    def )', text)
        if m and '"pyiceberg-interop"' in m.group(1) and 'assertNotIn("pyiceberg-interop"' not in m.group(1):
            body = re.sub(r',\s*"pyiceberg-interop"', "", m.group(1))
            body = body.rstrip("\n") + ("\n        # The PyIceberg exchange's Rust half is `yggdryl-iceberg`'s own target.\n"
                                       '        self.assertNotIn("pyiceberg-interop", jobs)\n')
            text = text[:m.start()] + body + text[m.end():]
            write(root / rel, text)
        elif not m:
            residue(rel, "no exchange-half test to take the PyIceberg exchange out of")
    done("CI: the `iceberg` leaf line on, `x-iceberg` reading the crate, the MSRV job and the PyIceberg build re-pointed")


# ---------------------------------------------------------------------------
# Step 12: docs, skills, AGENTS.md, READMEs, inventory
# ---------------------------------------------------------------------------

PROSE_SUBS: list[tuple[str, str]] = [
    (r"\(which implies `(?:s3|aws)` and `iceberg`[^)]*\)", "(of `yggdryl-iceberg`, which links `yggdryl-s3`)"),
    (r"which implies `(?:s3|aws)` and `iceberg`\.", "which links `yggdryl-s3`."),
    (r"the core claims as `iceberg` before the register answers anything", "`yggdryl_iceberg::install()` claims as `iceberg`"),
    (r"\| `iceberg` feature \|", "| `yggdryl-iceberg` |"),
    (r"`iceberg` feature \(implies `parquet`\)", "`yggdryl-iceberg`"),
    (r"\(`iceberg` feature\)", "(`yggdryl-iceberg`)"),
    (r"\(`s3tables` feature\)", "(`yggdryl-iceberg`'s `s3tables` feature)"),
    (r"\(the `iceberg` feature\)", "(`yggdryl-iceberg`)"),
    (r"Under the `iceberg` feature", "With `yggdryl-iceberg`"),
    (r"under the `iceberg` feature", "with `yggdryl-iceberg`"),
    (r"The `iceberg` feature", "`yggdryl-iceberg`"),
    (r"the `iceberg` feature", "`yggdryl-iceberg`"),
    (r"[Uu]nder the `s3tables` feature", "under `yggdryl-iceberg`'s `s3tables` feature"),
    (r"The `s3tables` feature", "`yggdryl-iceberg`'s `s3tables` feature"),
    (r"(?<![s'] )the `s3tables` feature", "`yggdryl-iceberg`'s `s3tables` feature"),
]


def prose(text: str) -> str:
    for pattern, replacement in PROSE_SUBS:
        text = re.sub(pattern, replacement, text)
    return text


def edit_docs(root: pathlib.Path) -> None:
    edit(root, "docs/media/iceberg.md",
         ("| Build | the `iceberg` feature, which implies `parquet` |",
          "| Build | the `yggdryl-iceberg` crate, which every binding and the `yggdryl` command link and install - "
          "`yggdryl_iceberg::install()` claims the format, its `hadoop` catalog and, under its `s3tables` feature, "
          "Amazon S3 Tables; the type vocabulary, `PrimitiveType`, is the core's |"),
         optional=True)
    edit(root, "docs/index.md",
         ('    # Parquet and Iceberg are opt-in; everything else is on by default.\n'
          '    # yggdryl = { version = "0.1", features = ["iceberg"] }\n',
          '    # Parquet and Iceberg are crates of their own; everything else is on by default.\n'
          '    # yggdryl-iceberg = "0.1"\n'), optional=True)
    edit(root, "skills/yggdryl/references/rust.md",
         ("# Iceberg tables (needs Rust 1.94), object stores:\n"
          '# yggdryl = { version = "0.1", features = ["iceberg", "s3"] }\n',
          "# Iceberg tables (needs Rust 1.94) - `yggdryl_iceberg::install()` claims\n"
          "# the table format before a folder is read as a table:\n"
          '# yggdryl-iceberg = "0.1"\n'),
         ("| `parquet` | the Parquet crate the core's Iceberg tables read their data files' footers with, until Iceberg is its own crate; the medium is `yggdryl-parquet` | - |\n", ""),
         ("| `iceberg` | Iceberg tables over `yggdryl-parquet`'s Parquet | `parquet` |\n", ""),
         optional=True)
    edit(root, "skills/yggdryl-records/references/rust.md",
         (' - and Iceberg `features = ["iceberg"]`.',
          " - and Iceberg the `yggdryl-iceberg` crate - `yggdryl_iceberg::install()` claims the table format before a folder is read as a table."),
         optional=True)
    edit(root, "skills/yggdryl-types/references/rust.md",
         ("- Protocol views are borrows (`as_iceberg`, `as_digest_mut`, ...)", "- Protocol views are borrows (`as_glue`, `as_digest_mut`, ...)"),
         optional=True)
    edit(root, "skills/yggdryl-types/SKILL.md",
         ('| one protocol\'s keys | `as_iceberg_mut().insert("doc", ..)?` |',
          '| one protocol\'s keys | `protocol_mut(&Scheme::ICEBERG).insert("doc", ..)?` |'), optional=True)
    edit(root, "docs/architecture.md",
         ("`field.as_iceberg()` and `FixField::new(&field)` borrow the same field",
          "`IcebergField::new(&field)` and `FixField::new(&field)` borrow the same field"),
         ("so `FixField` lives in `fix/field.rs` and `Field` carries no accessor for it.",
          "so `FixField` lives in `fix/field.rs`, `IcebergField` in `yggdryl-iceberg`'s `field.rs`, and `Field` carries no accessor for either."),
         optional=True)
    edit(root, "rust/README.md",
         ("tests/interop/iceberg.rs\n                       The Iceberg exchange with PyIceberg (`iceberg` feature)\n", ""),
         optional=True)
    count = 0
    for f in [*tracked(root, "docs", "skills"), "README.md", "rust/README.md"]:
        if not f.endswith(".md") or not (root / f).exists() or f.startswith("docs/assets/"):
            continue
        text = read(root / f)
        new = prose(text)
        if new != text:
            write(root / f, new)
            count += 1
    done(f"docs: the Iceberg page's overview, the feature tables, {count} pages' prose re-pointed at `yggdryl-iceberg`")


def edit_agents(root: pathlib.Path) -> None:
    rel = "AGENTS.md"
    text = read(root / rel)
    parquet_moved = not tracked(root, "rust/src/parquet")
    if parquet_moved:
        text = text.replace(
            "`default = []`, `parquet` (the\nParquet crate the core's Iceberg reads its data files' footers with, until\n"
            "Iceberg is its own crate - the medium is `yggdryl-parquet`), `iceberg`\n(implies `parquet`), `http`,",
            "`default = []`, `http`,")
    text = re.sub(r", `s3tables` \(implies `(?:s3|aws)` and\n`iceberg`\)", "", text)
    text = text.replace("`iceberg`\n(implies `parquet`), `http`,", "`http`,")
    text = text.replace("| a gated path works | the loop above plus `--features iceberg` or ",
                        "| a gated path works | the loop above plus `-p yggdryl-iceberg --features s3tables` or ")
    text = text.replace("| `iceberg/` | `mod.rs` `IcebergFormat` (`ICEBERG_FORMAT`), the `TableFormat` the core claims as `iceberg`:",
                        "| `iceberg.rs`; `iceberg/` in `yggdryl-iceberg` (`rust/iceberg/src/`) | `iceberg.rs` the type vocabulary the core keeps (`PrimitiveType`, D17); the crate's `lib.rs` `IcebergFormat` (`ICEBERG_FORMAT`), the `TableFormat` its `install()` claims as `iceberg`:")
    text = re.sub(r"\| `s3tables/` \| Amazon S3 Tables, under the non-default `s3tables` feature \(`(?:s3|aws)` and `iceberg`\):",
                  "| `s3tables/` in `yggdryl-iceberg` | Amazon S3 Tables, under the crate's non-default `s3tables` feature "
                  "(`yggdryl-s3` and the core's `aws`):", text)
    text = text.replace("| Iceberg Rust 1.94 | the declared MSRV, the workspace's one: the core with every target and the official Iceberg boundary | "
                        "`cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg` |",
                        "| Iceberg Rust 1.94 | the declared MSRV, the workspace's one: `yggdryl-iceberg` - the core beneath it - with every target and the official Iceberg boundary | "
                        "`cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl-iceberg --all-targets --features s3tables` |")
    for old in ("which `rust/tests/iceberg/types.rs` pins", "which `rust/iceberg/tests/iceberg/types.rs` pins"):
        text = text.replace(old, "which `rust/tests/root/iceberg.rs` pins")
    text = text.replace("# compiled against parquet iceberg s3 http3",
                        "# compiled against " + ("http3" if parquet_moved else "parquet http3"))
    text = text.replace("(`fix/field.rs`: `FixField::new(&field)`)",
                        "(`fix/field.rs`: `FixField::new(&field)`; `yggdryl-iceberg`'s `field.rs`: `IcebergField::new(&field)`)")
    text = prose(text)
    write(root / rel, text)
    for m in re.finditer(r"(?m)^.*(?:`iceberg` feature|feature = \"iceberg\"|until [^.\n]*Iceberg is its own crate|the core claims[^.\n]*(?:ICEBERG|HADOOP|S3TABLES|iceberg)).*$", text):
        residue(f"{rel}:{line_of(text, m.start())}", "a sentence still states the core's own Iceberg; re-word it")
    done("AGENTS.md: the features, the layout rows, the gated loop, the MSRV row")


ICEBERG_LIB_NOTE = ("(the crate root: the table format and its catalog, `install()` claiming ICEBERG_FORMAT as "
                    "`iceberg`, HADOOP_FACTORY under `hadoop` and, under `s3tables`, S3TABLES_FACTORY and "
                    "S3TABLES_LOCATOR; the IcebergField view minted over Scheme::ICEBERG; `internals` hidden)")


def implementer_inventory_lines() -> str:
    lines = ["  (Iceberg: what the Iceberg crate reaches - a write's shaping, count and cadences, the one "
             "partitioner and the residual a read runs once, the warehouse handle its objects resolve, the time "
             "buckets and periods a partition bound reads, and, under `aws`, the identity service doors its "
             "Amazon S3 Tables client signs and reads its answers through)"]
    for _name, entry in ICEBERG_ENTRIES:
        doc = " ".join(re.findall(r"(?m)^/// ?(.*)$", entry.split("\n#")[0].split("\npub")[0]))
        first = re.split(r"(?<=[.:])\s", doc, maxsplit=1)[0].rstrip(".:")
        for m in re.finditer(r"(?ms)^pub (?:const )?(?:fn|use|const|struct) .*?(?:\{\n|;\n|\n\{)", entry):
            sig = re.sub(r"\s+", " ", m.group(0)).strip().rstrip("{").strip()
            sig = sig.replace("( ", "(").replace(", )", ")").replace(" )", ")")
            lines.append(f"{sig}  ({first})")
    return "\n".join(lines) + "\n"


def edit_inventory(root: pathlib.Path) -> None:
    rel = ".api-inventory.txt"
    if not (root / rel).exists():
        return
    text = read(root / rel)
    if "### yggdryl_iceberg  [" in text and ICEBERG_LIB_NOTE in text:
        return
    m = re.search(r"(?m)^### yggdryl_iceberg  \[rust/iceberg/src/lib\.rs\][^\n]*\n", text)
    if m:
        text = text[:m.start()] + f"### yggdryl_iceberg  [rust/iceberg/src/lib.rs]  {ICEBERG_LIB_NOTE}\n" + text[m.end():]
        at = text.find("\n", text.find("### yggdryl_iceberg  [rust/iceberg/src/lib.rs]")) + 1
        text = text[:at] + "pub fn install() -> yggdryl::Result<()>  (claims what Iceberg registers on the core, once; the crates it reads installed first)\n" + text[at:]
    else:
        residue(rel, "no `yggdryl_iceberg` crate-root section after the path rewrite")
    m = re.search(r"(?m)^### yggdryl_iceberg  \[rust/src/iceberg\.rs\][^\n]*\n", text)
    if m:
        text = text[:m.start()] + "### yggdryl::iceberg  [rust/src/iceberg.rs]  (the Iceberg type vocabulary the core keeps, D17)\n" + text[m.end():]
    for old, new in (
        ("(claimed by the core under `iceberg` before the format register answers anything)", "(claimed by `install()` under `iceberg`)"),
        ("(claimed by the core under the `type` word `hadoop` before the factory register answers anything)", "(claimed by `install()` under the `type` word `hadoop`)"),
        ("(claimed by the core under the `s3tables` scheme before the factory register answers anything)", "(claimed by `install()` under the `s3tables` scheme)"),
        ("(claimed by the core under the `s3tables` scheme before the locator register answers anything)", "(claimed by `install()` under the `s3tables` scheme)"),
        ("; the core claims HADOOP_FACTORY under `iceberg` and S3TABLES_FACTORY under `s3tables` before the register answers anything)", ")"),
        ("; the core claims ICEBERG_FORMAT under `iceberg` before the register answers anything;", ";"),
        ("; the core claims S3TABLES_LOCATOR under `s3tables` before the register answers anything)", ")"),
        ("(the table format: the core claims ICEBERG_FORMAT as `iceberg` on the register", "(the table format: `install()` claims ICEBERG_FORMAT as `iceberg` on the register"),
        ("  pub fn as_iceberg(&self) -> ProtocolMetadata<'_>\n", ""),
        ("  pub fn as_iceberg(&self) -> protocol::IcebergField<'_>\n  pub fn as_iceberg_mut(&mut self) -> protocol::IcebergFieldMut<'_>\n", ""),
        ("  pub struct IcebergField<'field>(ProtocolField<'field>);\n  pub struct IcebergFieldMut<'field>(ProtocolFieldMut<'field>);\n  pub fn as_protocol(&self) -> IcebergField<'_>\n", ""),
    ):
        if old in text:
            text = text.replace(old, new, 1)
    m = re.search(r"(?m)^### yggdryl_iceberg::field  \[rust/iceberg/src/field\.rs\]\n", text)
    if m and "pub struct IcebergField<'field>" not in text[m.end():m.end() + 400]:
        text = text[:m.end()] + ("pub struct IcebergField<'field>  (minted by `protocol_field_types!` over Scheme::ICEBERG; Deref to ProtocolField)\n"
                                 "  pub fn new(field: &'field Field) -> Self\n"
                                 "pub struct IcebergFieldMut<'field>  (minted beside it; Deref to ProtocolFieldMut)\n"
                                 "  pub fn new(field: &'field mut Field) -> Self\n"
                                 "  pub fn as_protocol(&self) -> IcebergField<'_>\n") + text[m.end():]
    start = text.find("### yggdryl::implementer  [")
    end = text.find("\n### ", start + 1) if start >= 0 else -1
    if start < 0 or end < 0:
        residue(rel, "no `yggdryl::implementer` section to list the Iceberg additions under")
    elif "(Iceberg: what the Iceberg crate reaches" not in text[start:end]:
        section = text[start:end].rstrip("\n")
        text = text[:start] + section + "\n" + implementer_inventory_lines() + text[end:]
    write(root / rel, text)
    for m in re.finditer(r"(?m)^.*(?:as_iceberg|the core claims[^\n]*(?:ICEBERG|HADOOP|S3TABLES)).*$", text):
        residue(f"{rel}:{line_of(text, m.start())}", "an inventory line still names the core's own Iceberg")
    done(".api-inventory.txt: the Iceberg sections re-homed under yggdryl_iceberg, the core's type section, the view, the implementer additions")


# ---------------------------------------------------------------------------
# Step 13: what the compiler loop still has to see
# ---------------------------------------------------------------------------


def crate_modules(root: pathlib.Path) -> set[str]:
    lib = read(root / CRATE_DIR / "src/lib.rs")
    names = set(re.findall(r"(?m)^(?:pub(?:\([^)]*\))? )?mod (\w+);", lib))
    names |= set(re.findall(r"(?m)^(?:pub(?:\([^)]*\))? )?(?:struct|enum|fn|static|const|type|trait) (\w+)", lib))
    for m in re.finditer(r"(?m)^pub(?:\(crate\))? use \w+::(\{[^;]*\}|\w+);", lib):
        names.update(s4.Tables.brace_names(m.group(1)) if m.group(1).startswith("{") else [m.group(1)])
    return names | {"internals", "s3tables"}


def scan_residue(root: pathlib.Path) -> None:
    leftover = re.compile(r"\byggdryl::(?:iceberg::(?!PrimitiveType\b)\w|s3tables\b)|\bas_iceberg(?:_mut)?\(\)|"
                          r"yggdryl::internals::(?:iceberg|s3tables)_")
    for f in av.text_files(root):
        if f.startswith((".handoff/", "docs/assets/")) or not f.endswith((".rs", ".md", ".py", ".toml", ".yml", ".txt")):
            continue
        text = read(root / f)
        # A logger's name is data: `yggdryl::iceberg::table` there is a target
        # spelling the facade maps, whichever crate holds it.
        data = f.startswith(("rust/tests/logging/", "python/yggdryl/logging", "python/tests/logging"))
        for m in leftover.finditer("" if data else text):
            residue(f"{f}:{line_of(text, m.start())}", f"`{m.group(0)}` is left")
        if f.startswith(("rust/src/", "rust/tests/", "rust/benchmarks/")) or (
                f.startswith("rust/") and not f.startswith(f"{CRATE_DIR}/") and f.endswith(".rs")):
            for m in re.finditer(r'feature\s*=\s*"(?:iceberg|s3tables|parquet)"', no_comments(text)):
                if "parquet" in m.group(0) and tracked(root, "rust/src/parquet"):
                    continue
                residue(f"{f}:{line_of(text, m.start())}", f"`{m.group(0)}` names a feature the core no longer has")
    own = crate_modules(root)
    for path in sorted((root / CRATE_DIR / "src").rglob("*.rs")):
        rel = str(path.relative_to(root))
        code = code_only(read(path))
        for m in re.finditer(r"(?<![\w:$])crate::(\w+)", code):
            if m.group(1) not in own:
                residue(f"{rel}:{line_of(code, m.start())}", f"`crate::{m.group(1)}` names nothing the crate holds")
        if re.search(r"#\[(?:cfg\()?test\b", code):
            residue(rel, "test code under `src/`")
    missing = [leaf for leaf in ("avro", "parquet", "s3") if not leaf_exists(root, leaf)]
    if missing:
        residue("tree", f"`yggdryl-{'`, `yggdryl-'.join(missing)}` not moved yet: the crate names "
                        f"`{'`, `'.join('yggdryl_' + m for m in missing)}` and builds once "
                        f"`{'`, `'.join(f's6_{m}_move.py' for m in missing)}` have run")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=pathlib.Path)
    parser.add_argument("--no-git-lock", action="store_true", help="the caller holds $S/git.lock already")
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s6_iceberg" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    # Step 0: a tree already moved is left as it is.
    if (root / CRATE_DIR / "Cargo.toml").exists():
        print(f"[s6-iceberg] {CRATE_DIR}/Cargo.toml exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/iceberg/mod.rs").exists():
        raise SystemExit("rust/src/iceberg/mod.rs is missing: this is not the tree S6 moves Iceberg out of")
    if git(root, "status", "--porcelain").strip():
        raise SystemExit("the tree has changes; run the Iceberg move on a clean tree")
    import tomllib

    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)
    core_manifest = tomllib.loads(read(root / "rust/Cargo.toml"))
    s3tables_mod = root / "rust/src/s3tables/mod.rs"
    public = iceberg_public(read(root / "rust/src/iceberg/mod.rs"), read(s3tables_mod) if s3tables_mod.exists() else None)
    tables = Paths(public, iceberg_internals(read(root / "rust/src/lib.rs")))
    rewriter = Rewriter(tables)
    av.load_trait_methods(root)
    moves = plan_moves(root)
    paths = PathMap(moves)

    # Steps 1-2 read the core's files where they stand.
    splits, _moved, _copied = split_tests(root, moves)
    carve_benches(root)

    edit(root, "rust/tests/interop.rs",
         ('#[cfg(feature = "iceberg")]\n#[path = "interop/iceberg.rs"]\nmod iceberg;\n', ""))

    # Step 3: the moves.
    with s4.GitLock(not arguments.no_git_lock):
        git_mv(root, moves)
    split_back_types(root)
    relocate_error_pin(root)
    settle_remnant(root)

    # Steps 4-5.
    build_lib(root)
    av.edit_manifest(root)
    rel = f"{CRATE_DIR}/src/table.rs"
    if (root / rel).exists():
        text = read(root / rel)
        new = text.replace("crate::parquet::FileStatistics::from_metadata(",
                           "yggdryl_parquet::implementer::file_statistics_from_metadata(")
        if new != text:
            write(root / rel, new)
    edit_crate_sources(root)
    edit_core_sources(root)
    edit_core_prose(root)
    edit_core_manifest(root)
    edit_implementer(root)

    # Steps 6-7.
    rewrite_paths(root, rewriter, paths, moves, splits)
    finish_crate_text(root)
    drop_unused_names(root)
    resolve_crate_gates(root)
    sweep_as_iceberg(root)
    edit_interop(root)

    # Steps 8-12.
    install_everywhere(root)
    crate_manifest(root, core_manifest, version)
    edit_tooling(root)
    edit_ci(root)
    edit_docs(root)
    edit_agents(root)
    edit_inventory(root)
    # Last: every edit above anchors on the commands as they were.
    edit_commands(root)

    # Step 13.
    scan_residue(root)
    with s4.GitLock(not arguments.no_git_lock):
        git(root, "add", "-A", "--", *[p for p in (CRATE_DIR, "rust/tests/root/iceberg.rs") if (root / p).exists()])
    report = (["# S6 Iceberg residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE]
              + ["", "## Residue", ""] + (RESIDUE or ["- none"]))
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s6-iceberg] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
