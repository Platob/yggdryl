#!/usr/bin/env python3
"""S6d: move the object-store backend into `yggdryl-s3` at `rust/s3/`.

Usage:
    python3 -I s6_s3_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch with D36's register in place
(P3), alone or in the S6 batch after the Avro and Parquet moves and before
the Iceberg one - and it leaves the tree uncommitted: `git status` shows the
moves as renames and every rewrite as a modification, for the lane manager's
compiler loop (`cargo check --workspace --all-targets --all-features
--keep-going`). It runs no cargo command. A second run on a tree it already
moved stops at step 0 and changes nothing.

The core does not build between this move and the Iceberg one: the core's
`s3tables/` (Iceberg's, D15) opens a table's store through
`yggdryl_s3::located_with`, which the core cannot depend on; the Iceberg move
takes `s3tables/` into `yggdryl-iceberg`, whose `s3tables` feature depends on
`yggdryl-s3` (D36.6).

What it does, in order (`--residue` gets every site it could not rewrite
mechanically, `file:line` each):

 1. test split - before the moves, every core test item that builds an
                 object store's handle - a `yggdryl::s3` path, a name
                 imported from it, `yggdryl::internals::s3_*`, a positive
                 `feature = "s3"` gate - moves into `rust/s3/tests/` at the
                 same relative path with the helpers it names (D39), the
                 harness written beside it; `FakeS3` stays under
                 `rust/tests/support/`, `#[path]`-included. What builds an
                 Iceberg table over the store stays for the Iceberg move:
                 `accounting::iceberg` of `rust/tests/s3/mod_.rs` is kept in
                 the core at that path with its fixtures (D36.7), and the
                 core's Iceberg suites and bench keep their object-store
                 modules, their `s3` gates re-keyed to `s3tables`.
 2. moves      - `git mv`: `rust/src/s3/` to `rust/s3/src/` (`mod.rs` the
                 crate root `lib.rs`), `rust/tests/s3{,.rs}` to
                 `rust/s3/tests/`, `rust/tests/interop/s3/` to
                 `rust/s3/tests/interop/s3/`, `rust/benchmarks/holder/s3/`
                 to `rust/s3/benchmarks/s3/`.
 3. crate      - `rust/s3/src/lib.rs`: the doc, `#![deny(unsafe_code)]`,
                 `install()` claiming `S3_BACKEND` through
                 `yggdryl::holder::claim_backend` as `yggdryl-s3`, idempotent
                 (D7); the session's crate-private doors and two associated
                 ones called through `yggdryl::implementer`, the two core
                 modules the client imported whole flattened to the names it
                 reads, links to what the core keeps private turned to code.
 4. core       - `pub mod s3` and the register's seed gone (no backend is the
                 core's own any more); the 23 `feature = "s3"` sites of
                 `aws/`, `auth/`, `http/` and `xml/` re-keyed to the feature
                 of the module that holds them, or deleted (D36.5); the 58
                 crate-private items the backend reaches published in
                 `yggdryl::implementer` (functions forwarded, types raised in
                 their private modules and re-exported, associated items
                 forwarded taking their receiver first); `s3tables/`
                 re-spelled onto `yggdryl_s3`; the logging facade's `CRATES`
                 gains `yggdryl_s3`; the `s3` feature, `md-5` and the S3
                 bench baseline (`object_store`, `futures`) leave the core.
 5. paths      - every `crate::` path of the moved sources by owner, every
                 `yggdryl::s3` path and `use` tree in tests, benches,
                 bindings, docs and skills, `yggdryl::internals::s3_*`, file
                 paths in every text file, `#[path]` re-anchored.
 6. siblings   - a leaf crate's test or bench reaching the backend (the
                 Parquet crate's, in the batch) stays, its `s3` gates
                 resolved, the crate a dev-dependency and installed, its
                 `s3` feature forward gone and its `[leaves]` line after `s3`.
 7. install    - every test of the crate opens with
                 `crate::install::installed()`, the bench `main` installs, a
                 rustdoc example and a page's Rust block reaching the backend
                 install, the bindings' init and the CLI's `main` install.
 8. manifests  - `rust/s3/Cargo.toml` and its README, the workspace members
                 and `[workspace.dependencies]`, the core's, the bindings',
                 the CLI's.
 9. tooling    - `generate_internals.py` and `check_api_inventory.py` per
                 crate where S4 has not made them so, the generator run.
10. CI         - the `s3` leaf line with its three exchanges, the `iceberg`
                 line after `s3`, `x-s3`/`x-azure`/`x-gcs` reading the crate,
                 the planner's leaf grammar and exchange tests, the exchange
                 lane's build step, the three drivers' cargo lines, the docs
                 runner's features.
11. docs       - the holder page's object-store and register sections, the
                 architecture, contributing, testing and benchmark pages, the
                 skills, AGENTS.md, the READMEs, `.api-inventory.txt`.
12. residue    - what is left for the compiler loop, by file and line.
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
        "/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad",
    )
)
sys.path.insert(0, str(SCRATCH))
import s4_move as s4  # noqa: E402  (S4's lexer, use trees, rewriter, lock)

CORE, S3 = "core", "s3"
PACKAGE = "yggdryl-s3"
CRATE = "yggdryl_s3"
CRATE_DIR = "rust/s3"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-s3] {what}", flush=True)


# S4's helpers report into this script's residue; its rewriter learns the crate.
s4.residue = residue
s4.CRATE_NAME[S3] = CRATE
s4.PACKAGE[S3] = PACKAGE

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, render_use, UseParseError = s4.parse_use, s4.render_use, s4.UseParseError
line_of = s4.line_of


# ---------------------------------------------------------------------------
# Helpers the Avro and Parquet moves wrote, copied (importing either script
# would point S4's globals at its crate)
# ---------------------------------------------------------------------------


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
    "StreamExt": {"next", "collect", "map", "for_each", "try_collect"},
    "ObjectStoreExt": {"put", "get", "get_range", "head", "delete", "copy", "rename"},
    "ObjectStore": {"list", "list_with_delimiter", "put", "get", "get_range", "head", "delete"},
    "Engine": {"encode", "decode", "encode_string", "decode_vec"},
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
            body = inline_body(item) if item.kind == "mod" else None
            if body is not None and not re.search(r"\bsuper::\*", item_code[body[0]:body[1]]):
                # An inline module sees the parent's imports only through
                # `super::` (or `crate::` at a harness root): what it calls on
                # its own imports is no use of the parent's.
                inner = item_code[body[0]:body[1]]
                item_code = item_code[:body[0]] + item_code[body[1]:]
                for m in re.finditer(r"\b(?:super|crate)::(?:(\{)|(\w+))", inner):
                    if m.group(2):
                        used.add(m.group(2))
                        continue
                    close = s4.matching_brace(inner, m.start(1))
                    used |= set(re.findall(r"[A-Za-z_]\w*", inner[m.start(1):close if close > 0 else None]))
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


def edit_text(text: str, where: str, *pairs) -> str:
    """`edit` over text in hand."""
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
            residue(where, f"anchor matched {counts} times, not edited: {olds[0].strip()[:90]!r}")
    return text


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


# ---------------------------------------------------------------------------
# Feature gates: what a crate that holds the backend resolves, what the core
# keeps
# ---------------------------------------------------------------------------


def gate_re(feature: str) -> re.Pattern:
    return re.compile(r'feature\s*=\s*"' + re.escape(feature) + '"')


def positive_gate(text: str, feature: str) -> bool:
    """A `feature = "<feature>"` gate in code that is not `not(..)`."""
    nc = no_comments(text)
    return any(not re.search(r"not\(\s*$", nc[max(0, m.start() - 8):m.start()])
               for m in gate_re(feature).finditer(nc))


def resolve_gates(text: str, where: str, held: bool, feature: str = "s3") -> str:
    """The `feature` gates of `text`, resolved (the Parquet move's rule): with
    `held`, what the gate names is always there - a positive gate holds, a
    negative one does not; without it, it never is."""
    gate = gate_re(feature)
    out = text

    def all_list(m: re.Match) -> str:
        inner = m.group(2)
        terms = [t.strip() for t in re.split(r",(?![^()]*\))", inner) if t.strip()]
        rest = [t for t in terms if not gate.fullmatch(t)]
        if len(rest) == len(terms):
            return m.group(0)
        if not held:
            return m.group(1) + 'cfg(any())]'
        if len(rest) == 1:
            return m.group(1) + f"cfg({rest[0]})]"
        return m.group(1) + f"cfg(all({', '.join(rest)}))]"

    out = re.sub(r'(#\[)cfg\(all\(((?:[^()]|\([^()]*\))*)\)\)\]', all_list, out)
    plain, negated = f'#[cfg(feature = "{feature}")]', f'#[cfg(not(feature = "{feature}"))]'
    lines = out.split("\n")
    result: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        if stripped == plain or stripped == negated:
            keep = (stripped == plain) == held
            if keep:
                i += 1
                continue
            # The gated statement or item never compiles, nor do the comments
            # and attributes that state it.
            while result and re.match(r"[ \t]*(?:///|#\[)", result[-1]):
                result.pop()
            rest = "\n".join(lines[i + 1:])
            end = statement_end(rest, 0)
            skip = rest[:end].count("\n") if end > 0 else 0
            i += 1 + skip
            continue
        m = re.fullmatch(r'([ \t]*)#\[cfg_attr\(not\(feature = "' + re.escape(feature) + r'"\), (.*)\)\]', line)
        if m:
            if not held:
                result.append(f"{m.group(1)}#[{m.group(2)}]")
            i += 1
            continue
        result.append(line)
        i += 1
    out = "\n".join(result)
    while True:
        m = re.search(r'(?m)^([ \t]*)if cfg!\(feature = "' + re.escape(feature) + r'"\) \{\n', out)
        if not m:
            break
        open_at = m.end() - 2
        close = s4.matching_brace(code_only(out), open_at)
        if close < 0 or re.match(r"\s*else\b", out[close + 1:]):
            residue(where, f"an `if cfg!(feature = \"{feature}\")` with no plain block, left")
            break
        body = out[m.end():close]
        end = out.find("\n", close)
        end = len(out) if end < 0 else end + 1
        out = out[:m.start()] + (dedent(body.rstrip(" \t")) if held else "") + out[end:]
    if f'cfg!(feature = "{feature}")' in out or gate.search(no_comments(out)):
        for m in gate.finditer(no_comments(out)):
            residue(f"{where}:{line_of(out, m.start())}", f"a `{feature}` gate is left: resolve it by hand")
    return out


# ---------------------------------------------------------------------------
# The item split (S4's, as the Parquet move wrote it), for one reach at a time
# ---------------------------------------------------------------------------


class Reach:
    """What makes an item leave its file: a path it names, a name it imports
    from one, or a positive gate on a feature."""

    def __init__(self, path: re.Pattern, feature: str, roots: list[list[str]]) -> None:
        self.path = path
        self.feature = feature
        self.roots = roots


S3_REACH = Reach(
    re.compile(r"\byggdryl(?:_s3\b|::s3\b|::internals::s3_\w+\b)"),
    "s3",
    [["yggdryl", "s3"], ["yggdryl_s3"]],
)
# What builds an Iceberg table over the store, in the backend's own suite.
ICEBERG_REACH = Reach(
    re.compile(r"\byggdryl(?:_iceberg\b|::iceberg\b|::internals::iceberg\w*\b)"),
    "iceberg",
    [["yggdryl", "iceberg"], ["yggdryl_iceberg"]],
)
REACH = S3_REACH
AWAY, HOME = "away", "home"


def file_imports(items: list) -> set[str]:
    """Names a level imports from what the reach names."""
    names: set[str] = set()
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", s4.use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            if any(leaf[:len(root)] == root for root in REACH.roots) and kind == "self":
                names.add(alias or leaf[-1])
    return names


def reaches(item, imports: set[str]) -> bool:
    code = code_only(item.text)
    if REACH.path.search(code):
        return True
    if set(re.findall(r"[A-Za-z_]\w*", code)) & imports:
        return True
    return positive_gate(item.text, REACH.feature)


def header_gated(item) -> bool:
    """An inline module whose own attributes gate it on the reach's feature."""
    body = inline_body(item)
    return body is not None and positive_gate(item.text[:body[0]], REACH.feature)


def macro_names(items: list) -> None:
    """A top-level macro invocation defines what its arguments name and no
    other item does, so it travels with the items that name it."""
    defined = {name for item in items if item.kind != "use" for name in item.names}
    for item in items:
        if item.kind.startswith("macro:") and item.kind != "macro:macro_rules" and not item.names:
            m = re.search(r"!\s*[(\[{](.*)[)\]}]", code_only(item.text), re.S)
            if m:
                item.names |= set(re.findall(r"\b([A-Z]\w*)\b", m.group(1))) - defined


def tidy_head(text: str) -> str:
    """One blank line between a file's own documentation and its first item."""
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
        owner[id(item)] = AWAY if reaches(item, imports) else HOME
    changed = True
    while changed:
        changed = False
        for item in body:
            if owner[id(item)] == HOME and any(owner[other] == AWAY for other in refs[id(item)]
                                               if inline_body(next(i for i in body if id(i) == other)) is None):
                owner[id(item)] = AWAY
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
    """S4's `split_level` with the reach the one side above the core; an
    inline module gated on the reach's feature moves whole."""
    head, items = top_items(text)
    imports = inherited | file_imports(items)
    owner, refs = item_owners(items, inherited)
    moved = 0
    dest: dict[int, str] = {}
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    for item in items:
        if item.kind == "use" or owner[id(item)] != AWAY:
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
    away_text = "".join(out).rstrip("\n") + "\n"
    roots = {
        id(item) for item in body_items
        if id(item) not in dropped
        and (re.search(r"#\[test\]|#\[global_allocator\]", item.text) or inline_body(item) is not None
             or published(item.text))
    }
    stay = (closure(roots) - dropped) | set(replaced)
    out = [head]
    for item in items:
        if item.kind == "use":
            out.append(item.text)
        elif id(item) in replaced:
            out.append(replaced[id(item)])
        elif id(item) in stay:
            out.append(item.text)
    return "".join(out).rstrip("\n") + "\n", away_text, moved


def all_items(text: str, base: int = 0) -> list:
    """Every non-`use` item of `text`, inline modules' included, with its span."""
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
    """Whether `code` names `item` the way a use of it reads."""
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
    those `old` (the file they came from) named."""
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


# ---------------------------------------------------------------------------
# Who owns a path
# ---------------------------------------------------------------------------

# The core's crate-private items the moved sources reach, by the path they
# spell after `crate::`, and the name `yggdryl::implementer` publishes them
# under (S3's routes, D36.5's 58 items as the tree stands after P3).
_RETRY = ["RETRY_BACKOFF", "RETRY_COST", "RETRY_REFUND", "RetryBudget", "backoff", "delay",
          "fresh_jitter", "is_resumable", "is_retryable_transport", "is_unsent", "retry_after"]
_SIGV4 = ["EMPTY_PAYLOAD_SHA256", "Signer", "UNSIGNED_PAYLOAD", "canonical_query", "encode_key",
          "encode_query_component", "sha256_hex", "signed_access_key"]
_PROPERTIES = ["EndpointName", "Identity", "count", "flag", "refusal", "seconds"]
CORE_ROUTES: dict[tuple[str, ...], str] = {
    ("iobase", "oversized"): "oversized",
    ("iobase", "read_upload"): "read_upload",
    ("iobase", "short_upload"): "short_upload",
    ("holder", "sibling"): "sibling",
    ("holder", "system_time_ns"): "system_time_ns",
    ("uri", "percent_decode"): "percent_decode",
    ("uri", "percent_encode_segment"): "percent_encode_segment",
    ("integer", "BYTE_COUNT_SPELLINGS"): "BYTE_COUNT_SPELLINGS",
    ("integer", "byte_count_from_text"): "byte_count_from_text",
    ("integer", "integer_from_scalar_as"): "integer_from_scalar_as",
    ("boolean", "bool_from_text"): "bool_from_text",
    ("xxhash", "stream", "read_range_digest"): "read_range_digest",
    ("http", "record_process"): "record_process",
    ("http", "client", "agent_for"): "agent_for",
    ("aws", "environment", "is_native"): "is_native",
    ("auth", "Bearer"): "Bearer",
    ("auth", "Expiring"): "Expiring",
    ("auth", "Lease"): "Lease",
    ("auth", "instant"): "instant",
    ("auth", "variable"): "variable",
    ("xml", "scanner", "Element"): "ScannedElement",
    ("xml", "scanner", "XmlError"): "XmlError",
    ("xml", "scanner", "parse_document"): "parse_document",
    ("xml", "scanner", "parse_root"): "parse_root",
    ("ArnPartition", "check_region"): "arn_partition_check_region",
    ("ByteStream", "from_handle"): "byte_stream_from_handle",
    **{("http", "retry", name): name for name in _RETRY},
    **{("aws", "sigv4", name): name for name in _SIGV4},
    **{("aws", "properties", name): name for name in _PROPERTIES},
}
# Public at the crate root although spelled through a private module.
ROOT_NAMES: dict[tuple[str, ...], str] = {
    ("iobase", "not_empty"): "not_empty",
}


class S3Tables:
    """`s4.Rewriter`'s resolution, for one module leaving the core."""

    def __init__(self, internals: dict[str, list[str]]) -> None:
        self.internals = internals

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        if segs[0] == "s3":
            return S3, segs[1:]
        if segs[0] == "internals" and len(segs) > 1 and segs[1] in self.internals:
            return S3, ["internals", *self.internals[segs[1]], *segs[2:]]
        for cut in range(len(segs), 0, -1):
            name = ROOT_NAMES.get(tuple(segs[:cut]))
            if name:
                return CORE, [name, *segs[cut:]]
            name = CORE_ROUTES.get(tuple(segs[:cut]))
            if name:
                return CORE, ["implementer", name, *segs[cut:]]
        return CORE, segs


# A whole-module import becomes `use yggdryl_s3 as s3;`: dropped, and every
# `s3::` path the code or a doc example spells named by the crate instead.
ALIAS_LINE = re.compile(r"(?m)^[ \t]*(?://[/!][ \t]?)?(?:#[ \t]+)?use yggdryl_s3 as s3;[ \t]*\n")
S3_ROOTED = re.compile(r"(?<![\w:$])s3::")
DOC_LINE = re.compile(r"^([ \t]*//[/!][ \t]?)(.*)$")


def unalias(text: str) -> str:
    if not ALIAS_LINE.search(text):
        return text
    text = ALIAS_LINE.sub("", text)
    code = code_only(text)
    out, pos = [], 0
    for m in S3_ROOTED.finditer(code):
        out.append(text[pos:m.start()])
        out.append("yggdryl_s3::")
        pos = m.end()
    out.append(text[pos:])
    text = "".join(out)
    # A doc example's code is a comment to `code_only`: its fenced lines are
    # read one by one.
    lines = text.split("\n")
    fence = None
    for k, line in enumerate(lines):
        m = DOC_LINE.match(line)
        if not m:
            fence = None
            continue
        body = m.group(2)
        opening = re.match(r"\s*(`{3,})", body)
        if opening:
            fence = None if fence else opening.group(1)
            continue
        if fence:
            lines[k] = m.group(1) + S3_ROOTED.sub("yggdryl_s3::", body)
    return "\n".join(lines)


def markdown_unalias(text: str) -> str:
    def block(m: re.Match) -> str:
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().lstrip("{").strip().startswith((".rust", "rust")) or "yggdryl_s3 as s3" not in body:
            return m.group(0)
        if indent:
            stripped = "\n".join(l[len(indent):] if l.startswith(indent) else l for l in body.split("\n"))
            new = unalias(stripped)
            body = "\n".join((indent + l) if l else l for l in new.split("\n"))
        else:
            body = unalias(body)
        return f"{indent}```{info}\n{body}{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text)


# ---------------------------------------------------------------------------
# The move table and file paths
# ---------------------------------------------------------------------------

# Iceberg's accounting over the store stays where it was until the Iceberg
# move (D36.7): a text naming this file means that suite.
ICEBERG_REMNANT = "rust/tests/s3/mod_.rs"


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    moves: list[tuple[str, str]] = []
    for f in tracked(root, "rust/src/s3", "rust/tests/s3", "rust/tests/s3.rs", "rust/tests/interop/s3",
                     "rust/benchmarks/holder/s3"):
        if f.startswith("rust/src/s3/"):
            rel = f[len("rust/src/s3/"):]
            new = f"{CRATE_DIR}/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        elif f.startswith("rust/benchmarks/holder/s3/"):
            new = f"{CRATE_DIR}/benchmarks/s3/" + f[len("rust/benchmarks/holder/s3/"):]
        else:
            new = f"{CRATE_DIR}/tests/" + f[len("rust/tests/"):]
        moves.append((f, new))
    return moves


class PathMap:
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/s3/", f"{CRATE_DIR}/src/"),
            ("rust/tests/s3/", f"{CRATE_DIR}/tests/s3/"),
            ("rust/tests/interop/s3/", f"{CRATE_DIR}/tests/interop/s3/"),
            ("rust/benchmarks/holder/s3/", f"{CRATE_DIR}/benchmarks/s3/"),
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


def rewrite_text_paths(text: str, moves: list[tuple[str, str]]) -> str:
    """Moved file paths wherever a text spells them - but the Iceberg
    remnant's, which a text names for the suite that stays."""
    for old, new in sorted(moves, key=lambda pair: -len(pair[0])):
        if old == ICEBERG_REMNANT or old not in text:
            continue
        text = re.sub(r"(?<![\w./-])" + re.escape(old) + r"(?![\w-])", new, text)
    for old, new in PathMap([]).dirs:
        guard = r"(?!mod_\.rs)" if old == "rust/tests/s3/" else ""
        text = re.sub(r"(?<![\w./-])" + re.escape(old) + guard, new, text)
    text = re.sub(r"(?<![\w./-])rust/src/s3(?![\w/.-])", f"{CRATE_DIR}/src", text)
    text = re.sub(r"(?<![\w./-])rust/tests/s3(?![\w/.-])", f"{CRATE_DIR}/tests/s3", text)
    # A command running the backend's harness runs the crate's, whose own
    # features are not the core's.
    text = re.sub(r"(?m)(-p )yggdryl( [^\n]*?--test s3\b)", r"\1yggdryl-s3\2", text)
    return text


# ---------------------------------------------------------------------------
# Step 1: the core's tests that build an object store's handle, item by item
# ---------------------------------------------------------------------------

# What the split never cuts: what moves whole, Iceberg's own (it moves with
# `yggdryl-iceberg`, whose `s3tables` edge links this crate), the fixtures,
# and the two suites whose `s3` gates were the core's own items' (re-keyed).
SPLIT_EXCLUDED = (
    "rust/tests/s3/", "rust/tests/s3.rs", "rust/tests/interop/", "rust/tests/interop.rs",
    "rust/tests/iceberg/", "rust/tests/iceberg.rs", "rust/tests/s3tables", "rust/tests/medallion_ledger.rs",
    "rust/tests/support/", "rust/tests/aws/sigv4.rs", "rust/tests/xml.rs",
)
# What reaches the backend for Iceberg's sake: its gates name the feature the
# Iceberg crate links the backend under.
ICEBERG_KEPT = ("rust/tests/iceberg/", "rust/tests/iceberg.rs", "rust/benchmarks/media/iceberg.rs")


def is_harness(root: pathlib.Path, rel: str) -> bool:
    """A `rust/tests/<name>.rs` that declares the suites of `rust/tests/<name>/`."""
    m = re.fullmatch(r"rust/tests/(\w+)\.rs", rel)
    return bool(m) and (root / "rust/tests" / m.group(1)).is_dir()


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


def pre_split_edits(root: pathlib.Path) -> None:
    """The core's own `s3` gates on what stays: `aws::environment`'s pins read
    the session's own list, `encode_key` and the scanner's two readers are
    `aws`'s (D36.5) - only the sweep's one test needs the backend."""
    edit(root, "rust/tests/aws/environment.rs",
         ('#[cfg(all(feature = "internals", feature = "s3"))]\nmod internal {\n',
          '#[cfg(feature = "internals")]\nmod internal {\n'))
    path = root / "rust/tests/aws/sigv4.rs"
    if path.exists():
        text = read(path)
        text = text.replace(
            "// The object key encoder is the S3 client's, and built with it.\n",
            "// The object key encoder the object-store crate spells its keys with.\n", 1)
        n = text.count('#[cfg(feature = "s3")]\n')
        if n != 3:
            residue("rust/tests/aws/sigv4.rs", f"{n} `s3` gates where three were expected: re-key them by hand")
        text = text.replace('#[cfg(feature = "s3")]\n', "")
        write(path, text)
    edit(root, "rust/tests/xml.rs",
         ('#[cfg(all(feature = "s3", feature = "internals"))]\n#[path = "xml/scanner.rs"]\n',
          '#[cfg(all(feature = "aws", feature = "internals"))]\n#[path = "xml/scanner.rs"]\n'))


def split_tests(root: pathlib.Path, moves: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Every core test file's items building an object store's handle, moved
    byte-identical into `rust/s3/tests/` at the same relative path (D39)."""
    global REACH
    REACH = S3_REACH
    move_map = dict(moves)
    written: list[tuple[str, str]] = []
    needs: dict[str, list[str]] = collections.defaultdict(list)
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED) or is_harness(root, f):
            continue
        rel = f[len("rust/tests/"):]
        single = "/" not in rel
        home_text, away_text, moved = split_level(read(root / f), True, single, set())
        if away_text is None:
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
            + f" that build an\n//! object store's handle and so need `{PACKAGE}`, moved byte-identical (S6d,\n"
            "//! D39): a test lives in the lowest crate that can name everything it uses.\n//!\n"
        )
        original = read(root / f)
        write(root / dest_rel, tidy_head(prune_imports(prune_imports(drop_orphans(header + away_text, original, set())))))
        written.append((f, dest_rel))
        if not single:
            needs["rust/tests/" + rel.split("/", 1)[0] + ".rs"].append(f)
        write(root / f, tidy_head(prune_imports(prune_imports(drop_orphans(home_text, original, set())))))
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
            f"//! The `{PACKAGE}` half of `{harness}`: the modules whose tests build an\n"
            f"//! object store's handle, split off by S6d (D39), with the helpers they name.\n//!\n" + head
        ]
        parts += [item.text for item in items if item.kind == "use" or id(item) in chosen or id(item) in decls]
        if (root / dest).exists() or dest in move_map.values():
            residue(dest, f"the harness exists or arrives by a move: declare {', '.join(modules)} in it by hand")
            continue
        write(root / dest, tidy_head(prune_imports("".join(parts).rstrip("\n") + "\n")))
        written.append((harness, dest))
    # What stays in the core holds no `s3` gate: the backend is never the
    # core's own.
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith(SPLIT_EXCLUDED):
            continue
        text = read(root / f)
        if gate_re("s3").search(no_comments(text)):
            write(root / f, resolve_gates(text, f, held=False))
    return written


ICEBERG_REMNANT_HEAD = """//! The tests of `rust/tests/s3/mod_.rs` that build an Iceberg table over the
//! object stores' in-process store - `accounting::iceberg`, what a table
//! costs per operation - with the fixtures they name, left in the core by
//! the object stores' move (S6d) for the Iceberg move, which takes them to
//! `rust/iceberg/tests/` (D36.7): the `s3tables` edge is what links
//! `yggdryl-s3` there, and its gate is what they compile under until then.
//!
"""

ICEBERG_REMNANT_HARNESS = """//! The object stores' suite's Iceberg half: what an Iceberg table costs over
//! the in-process store (`accounting::iceberg` in `s3/mod_.rs`), left in the
//! core by the object stores' move (S6d) for the Iceberg move, which takes
//! this harness and its one module to `rust/iceberg/tests/` (D36.7). The
//! backend's own suites are `yggdryl-s3`'s, under `rust/s3/tests/`.

#[cfg(feature = "s3tables")]
#[path = "support/server.rs"]
mod server;

#[cfg(feature = "s3tables")]
#[path = "s3/mod_.rs"]
mod mod_;
"""


def split_iceberg_remnant(root: pathlib.Path) -> str | None:
    """`rust/tests/s3/mod_.rs` less what builds an Iceberg table, written back
    before it moves; the Iceberg half answered, written after the moves."""
    global REACH
    rel = ICEBERG_REMNANT
    if not (root / rel).exists():
        return None
    REACH = ICEBERG_REACH
    try:
        original = read(root / rel)
        home, away, moved = split_level(original, True, False, set())
    finally:
        REACH = S3_REACH
    if away is None:
        return None
    write(root / rel, tidy_head(prune_imports(prune_imports(home))))
    done(f"{rel}: {moved} Iceberg test(s) kept in the core for the Iceberg move")
    return ICEBERG_REMNANT_HEAD + away


def rekey_iceberg_gates(root: pathlib.Path) -> list[str]:
    """Iceberg's suites and bench over the store keep their modules: the core
    has no `s3` feature, and the Iceberg crate links the backend under its
    `s3tables` feature (D36.6), so that is the gate they wait under."""
    touched = []
    files = [f for f in tracked(root, "rust/tests", "rust/benchmarks") if f.startswith(ICEBERG_KEPT)]
    for f in files:
        text = read(root / f)
        new = re.sub(r'feature\s*=\s*"s3"', 'feature = "s3tables"', text)
        new = new.replace('all(feature = "s3tables", feature = "s3tables")', 'feature = "s3tables"')
        if new != text:
            write(root / f, new)
            touched.append(f)
    return touched


# ---------------------------------------------------------------------------
# Step 3: the crate's own sources
# ---------------------------------------------------------------------------

# Before the rewrite: what the moved sources spell that the rewrite cannot
# re-own - the two core modules the client imported whole, the session's
# crate-private doors called as methods, and links to what the core keeps
# private - in the core's own terms, so the rewrite takes them from there.
SOURCE_EDITS: dict[str, list[tuple[str, str]]] = {
    "client.rs": [
        ("use crate::aws::sigv4::{self, Signer};\n",
         "use crate::aws::sigv4::{\n    EMPTY_PAYLOAD_SHA256, Signer, UNSIGNED_PAYLOAD, canonical_query, encode_key, sha256_hex,\n"
         "    signed_access_key,\n};\n"),
        ("use crate::http::retry::{\n    self, RETRY_COST, RETRY_REFUND, RetryBudget, fresh_jitter, is_resumable,\n"
         "    is_retryable_transport, is_unsent,\n};\n",
         "use crate::http::retry::{\n    RETRY_COST, RETRY_REFUND, RetryBudget, delay, fresh_jitter, is_resumable,\n"
         "    is_retryable_transport, is_unsent, retry_after,\n};\n"),
        ("    retry::retry_after(value).filter(", "    retry_after(value).filter("),
        ("            path.push_str(&sigv4::encode_key(account));\n", "            path.push_str(&encode_key(account));\n"),
        ("            path.push_str(&sigv4::encode_key(container));\n", "            path.push_str(&encode_key(container));\n"),
        ("        let key = sigv4::encode_key(key);\n", "        let key = encode_key(key);\n"),
        ("    /// from the backoff window ([`retry::delay`]).\n", "    /// from the backoff window (`http::retry::delay`).\n"),
        ("        std::thread::sleep(retry::delay(attempt, asked, &self.jitter));\n",
         "        std::thread::sleep(delay(attempt, asked, &self.jitter));\n"),
        (".and_then(sigv4::signed_access_key)", ".and_then(signed_access_key)"),
        ("        let mut query = sigv4::canonical_query(&request.query);\n",
         "        let mut query = canonical_query(&request.query);\n"),
        ("None | Some([]) => sigv4::EMPTY_PAYLOAD_SHA256.to_owned(),", "None | Some([]) => EMPTY_PAYLOAD_SHA256.to_owned(),"),
        ("                        sigv4::sha256_hex(body)\n", "                        sha256_hex(body)\n"),
        ("Some(_) => sigv4::UNSIGNED_PAYLOAD.to_owned(),", "Some(_) => UNSIGNED_PAYLOAD.to_owned(),"),
        ("        self.session.signer(\"s3\", region, now)\n",
         "        crate::implementer::session_signer(&self.session, \"s3\", region, now)\n"),
        ("        let another = self.session.answers_another(\n            signed,\n",
         "        let another = crate::implementer::session_answers_another(\n            &self.session,\n            signed,\n"),
        ("        let tls = session.tls_config()?;\n", "        let tls = crate::implementer::session_tls_config(&session)?;\n"),
        ("        match self.session.bucket_region(partition, bucket) {\n",
         "        match crate::implementer::session_bucket_region(&self.session, partition, bucket) {\n"),
        ("            self.session.learn_bucket_region(bucket, region);\n",
         "            crate::implementer::session_learn_bucket_region(&self.session, bucket, region);\n"),
        ("(options.region(), session.stated_region())", "(options.region(), crate::implementer::session_stated_region(&session))"),
        ("[`crate::http::retry`]'s", "`http::retry`'s"),
        ("rule is `crate::http::retry`'s, re-exported here", "rule is `http::retry`'s, re-exported here"),
        ("[`Session::bucket_region`]", "`Session::bucket_region`"),
        ("[`Session::learn_bucket_region`]", "`Session::learn_bucket_region`"),
    ],
    "properties.rs": [
        ("        match self.session().given_variables() {\n",
         "        match crate::implementer::session_given_variables(self.session()) {\n"),
        ("!self.session().states_identity()", "!crate::implementer::session_states_identity(self.session())"),
        ("        let session = self.session().under(ambient.session());\n",
         "        let session = crate::implementer::session_under(self.session(), ambient.session());\n"),
    ],
    "xml.rs": [
        ("pub(crate) use crate::xml::scanner::{Element, XmlError, parse_document, parse_root};\n",
         "pub(crate) use crate::xml::scanner::{Element as Element, XmlError, parse_document, parse_root};\n"),
        ("[`crate::xml::scanner`]", "`xml::scanner`"),
        ("    //! `yggdryl::internals::xml`.\n", "    //! `yggdryl_s3::internals::xml`.\n"),
    ],
    "aws/xml.rs": [
        ("[`crate::xml::scanner`]", "`xml::scanner`"),
    ],
}
# Every `[`crate::ArnPartition::check_region`]` the client's docs link: the
# rule is the core's own, reached through the implementer.
CHECK_REGION_LINK = "[`crate::ArnPartition::check_region`]"

LIB_DOC_EDITS = [
    ("//! The ten schemes are [`S3_BACKEND`]'s, the\n//! [`StorageBackend`] the core claims under\n"
     "//! them until `yggdryl-s3` does: [`Holder::from_url`] holds a location of any\n"
     "//! of them as the [`S3Path`] it names, [`Holder::Registered`] as every\n"
     "//! claimed backend's handle is, and [`Holder::downcast_ref`] answers it as\n"
     "//! the role it is.\n//!\n",
     "//! The ten schemes are [`S3_BACKEND`]'s, the [`StorageBackend`] [`install`]\n"
     "//! claims under them: [`Holder::from_url`] holds a location of any of them\n"
     "//! as the [`S3Path`] it names, [`Holder::Registered`] as every claimed\n"
     "//! backend's handle is, and [`Holder::downcast_ref`] answers it as the role\n"
     "//! it is. Every binding's init and the `yggdryl` command's `main` call\n"
     "//! [`install`], and so does a Rust program before the core reads a location\n"
     "//! of the ten schemes, which is refused naming the crate to install until\n"
     "//! then; a handle built through this crate's own doors - [`file()`],\n"
     "//! [`folder()`], [`located()`] - needs no claim.\n//!\n"),
    ("/// The core claims it itself, before the register answers anything, until\n"
     "/// `yggdryl-s3`'s `install()` does. A location's query states the store's\n"
     "/// properties in the names [`S3Options::with_properties`] reads, and\n",
     "/// [`install`] claims it, once for the life of the process. A location's\n"
     "/// query states the store's properties in the names\n"
     "/// [`S3Options::with_properties`] reads, and\n"),
    ('.expect("the core claims `gs`");', '.expect("`install()` claimed `gs`");'),
    ("use std::sync::Arc;\n", "use std::sync::{Arc, Mutex, OnceLock, PoisonError};\n"),
]

INSTALL_FN = '''
/// What this crate claims the object stores' backend as.
const CLAIMANT: &str = "yggdryl-s3";

/// Claims the object stores' backend on the core's register of storage
/// backends - [`S3_BACKEND`] under its ten schemes, `s3`, `s3a`, `s3n`,
/// `gs`, `gcs`, `az`, `abfs`, `abfss`, `wasb` and `wasbs` - once for the life
/// of the process; a later call returns at once. Every binding's init and
/// the `yggdryl` command's `main` call it, and so does a Rust caller before
/// [`Holder::from_url`] reads a location of those schemes: until the claim
/// such a location is refused, naming the crate to install.
///
/// ```
/// use yggdryl::Scheme;
/// use yggdryl::holder::backend_for;
///
/// # fn main() -> yggdryl::Result<()> {
/// yggdryl_s3::install()?;
/// yggdryl_s3::install()?;
/// let claimed = backend_for(&Scheme::AZ).expect("`install()` claimed `az`");
/// assert_eq!(claimed.name(), "yggdryl-s3");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed one of the ten
/// schemes first, naming it.
pub fn install() -> Result<()> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    static INSTALLING: Mutex<()> = Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    let _installing = INSTALLING.lock().unwrap_or_else(PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    crate::holder::claim_backend(&S3_BACKEND, CLAIMANT)?;
    let _ = INSTALLED.set(());
    Ok(())
}
'''


def build_lib(root: pathlib.Path) -> None:
    """The crate root: the module's own doc and items, `install()` beside the
    backend it claims."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    text = edit_text(read(root / rel), rel, *LIB_DOC_EDITS)
    # `#![deny(unsafe_code)]` after the crate's own documentation.
    head = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if head and "#![deny(unsafe_code)]" not in text:
        text = text[:head.end()] + "\n#![deny(unsafe_code)]\n" + text[head.end():]
    anchor = "use client::Client;\n"
    if text.count(anchor) != 1:
        residue(rel, "no `use client::Client;` to place `install()` after: add it by hand")
    elif "pub fn install()" not in text:
        text = text.replace(anchor, anchor + INSTALL_FN, 1)
    write(root / rel, text)


def edit_crate_sources(root: pathlib.Path) -> None:
    base = f"{CRATE_DIR}/src"
    for rel, pairs in SOURCE_EDITS.items():
        edit(root, f"{base}/{rel}", *pairs)
    rel = f"{base}/client.rs"
    text = read(root / rel)
    if CHECK_REGION_LINK in text:
        text = text.replace(CHECK_REGION_LINK, "`ArnPartition::check_region`")
        write(root / rel, text)
    done("crate: lib.rs and install(), the client's flattened imports, the session's doors through the implementer")


def finish_crate_text(root: pathlib.Path) -> None:
    """After the rewrite: the scanner's element under the name the implementer
    publishes it by, aliased back to what the backend calls it."""
    rel = f"{CRATE_DIR}/src/xml.rs"
    text = read(root / rel)
    new = text.replace("ScannedElement as Element as Element", "ScannedElement as Element")
    if "ScannedElement as Element" not in new:
        residue(rel, "the scanner's element is not imported as `ScannedElement as Element`: fix the `use` by hand")
    if new != text:
        write(root / rel, new)


# ---------------------------------------------------------------------------
# Step 4: the core
# ---------------------------------------------------------------------------


def edit_core_root(root: pathlib.Path) -> None:
    edit(root, "rust/src/lib.rs", ('#[cfg(feature = "s3")]\npub mod s3;\n', ""))
    # No backend is the core's own any more: the register has nothing to seed.
    edit(
        root, "rust/src/holder/backend.rs",
        ("//! states. A scheme a core arm holds is never a backend's to claim. The core\n"
         "//! claims the object stores' backend itself under the `s3` feature, until\n"
         "//! `yggdryl-s3`'s `install()` does; a scheme no claim answers is refused\n"
         "//! naming the crate to install.\n",
         "//! states. A scheme a core arm holds is never a backend's to claim, and no\n"
         "//! backend is the core's own: the object stores' is `yggdryl-s3`'s, claimed\n"
         "//! by its `install()`; a scheme no claim answers is refused naming the crate\n"
         "//! to install.\n"),
        ("use std::sync::{Mutex, OnceLock};\n", "use std::sync::Mutex;\n"),
        ("static SEEDED: OnceLock<()> = OnceLock::new();\n", ""),
        ("/// Claim the core's own backends once, before the register answers\n/// anything.\nfn seed() {\n"
         "    SEEDED.get_or_init(|| {\n        // The core's claims cannot conflict: each scheme is stated once in\n"
         "        // the crate.\n        #[cfg(feature = \"s3\")]\n        claim_unseeded(&crate::s3::S3_BACKEND, CORE)\n"
         "            .expect(\"the core's own storage backends claim cleanly\");\n    });\n}\n\n", ""),
        ("    seed();\n    if by == CORE {\n", "    if by == CORE {\n"),
        ("    claim_unseeded(backend, by)\n}\n\nfn claim_unseeded(backend: &'static dyn StorageBackend, by: &'static str) -> Result<()> {\n",
         ""),
        ("    seed();\n    BACKENDS.get(scheme)\n", "    BACKENDS.get(scheme)\n"),
        ("    seed();\n    let mut backends", "    let mut backends"),
    )
    edit(
        root, "rust/src/holder/mod.rs",
        ("    /// `gs:`, `az:` and their aliases, by the backend the core claims under\n"
         "    /// the `s3` feature until `yggdryl-s3` does, configured by the properties\n",
         "    /// `gs:`, `az:` and their aliases, by the backend `yggdryl-s3`'s\n"
         "    /// `install()` claims, configured by the properties\n"),
    )
    edit(
        root, "rust/src/holder/counted.rs",
        ("`StatsSnapshot` (behind the `s3` feature) counts", "`StatsSnapshot` (`yggdryl-s3`'s) counts"),
    )
    edit(
        root, "rust/src/aws/mod.rs",
        ("//! The module is behind the non-default `aws` feature, which the `s3` feature\n//! implies.\n",
         "//! The module is behind the non-default `aws` feature, which `yggdryl-s3`\n"
         "//! and the `s3tables` feature turn on.\n"),
    )
    edit(
        root, "rust/src/xml/scanner.rs",
        ("//! reads for itself: `s3::aws::xml` reads `ListBucketResult`,\n//! `s3::azure::xml` reads",
         "//! reads for itself: `yggdryl-s3`'s `aws::xml` reads `ListBucketResult`,\n//! its `azure::xml` reads"),
    )


# The 23 `feature = "s3"` sites of `aws/`, `auth/`, `http/` and `xml/` (D36.5):
# each module holding one is already behind `aws` (or `http`), so the gate
# goes; a re-export only the backend read goes with it.
CFG_SITES: dict[str, tuple[int, list[tuple[str, str]]]] = {
    "rust/src/aws/mod.rs": (1, [('#[cfg(feature = "s3")]\npub(crate) mod environment;\n', "pub(crate) mod environment;\n")]),
    "rust/src/aws/session.rs": (11, []),
    "rust/src/aws/sigv4.rs": (3, [
        ('    #[cfg(all(feature = "internals", feature = "s3"))]\n    pub(crate) fn access_key_id(',
         '    #[cfg(feature = "internals")]\n    pub fn access_key_id('),
    ]),
    "rust/src/auth/lease.rs": (4, []),
    "rust/src/auth/mod.rs": (1, [('#[cfg(feature = "s3")]\npub(crate) use lease::Bearer;\n', "")]),
    "rust/src/http/mod.rs": (1, [('#[cfg(feature = "s3")]\npub(crate) use client::record_process;\n', "")]),
    "rust/src/xml/scanner.rs": (2, []),
}


def rekey_cfg_sites(root: pathlib.Path) -> int:
    count = 0
    for rel, (expected, pairs) in CFG_SITES.items():
        path = root / rel
        text = read(path)
        found = len(re.findall(r'feature\s*=\s*"s3"', text))
        if found != expected:
            residue(rel, f"{found} `s3` gates where {expected} were counted (D36.5): re-key them by hand")
        text = edit_text(text, rel, *pairs) if pairs else text
        text, n = re.subn(r'(?m)^[ \t]*#\[cfg\(feature = "s3"\)\]\n', "", text)
        text, m = re.subn(r'(?m)^[ \t]*#\[cfg_attr\(not\(feature = "s3"\), expect\(unused_variables, reason = "s3-only"\)\)\]\n',
                          "", text)
        write(path, text)
        left = len(re.findall(r'feature\s*=\s*"s3"', text))
        if left:
            residue(rel, f"{left} `s3` gate(s) left after the re-key")
        count += found - left
    return count


# Types the backend names, raised to `pub` inside the private module that
# holds them (route R: the module publishes nothing); the methods it calls
# with them; their links to what stays private turned to code.
RAISES: dict[str, list[tuple[str, str]]] = {
    "rust/src/http/retry.rs": [
        ("pub(crate) struct RetryBudget {", "pub struct RetryBudget {"),
        ("    pub(crate) fn withdraw(&self) -> bool {", "    pub fn withdraw(&self) -> bool {"),
        ("    pub(crate) fn refund(&self, tokens: i64) {", "    pub fn refund(&self, tokens: i64) {"),
        ("    pub(crate) fn remaining(&self) -> i64 {", "    pub fn remaining(&self) -> i64 {"),
        ("/// [`RETRY_COST`] tokens, a request that succeeds without one refunds\n/// [`RETRY_REFUND`], and",
         "/// `RETRY_COST` tokens, a request that succeeds without one refunds\n/// `RETRY_REFUND`, and"),
    ],
    "rust/src/aws/sigv4.rs": [
        ("pub(crate) struct Signer {", "pub struct Signer {"),
        ("    pub(crate) fn sign(\n", "    pub fn sign(\n"),
        ("    ///   canonical URI of it by its service's rule ([`Self::canonical_uri`]). \"/\" for the root.\n",
         "    ///   canonical URI of it by its service's rule (`Self::canonical_uri`). \"/\" for the root.\n"),
        ("    /// * `query`: raw (unencoded) name/value pairs; the canonical query string is built with\n"
         "    ///   [`canonical_query`].\n",
         "    /// * `query`: raw (unencoded) name/value pairs; the canonical query string is built with\n"
         "    ///   `canonical_query`.\n"),
        ("    /// * `payload_hash`: `sha256_hex(body)` or [`EMPTY_PAYLOAD_SHA256`]; the S3 family also\n"
         "    ///   accepts [`UNSIGNED_PAYLOAD`], which",
         "    /// * `payload_hash`: `sha256_hex(body)` or `EMPTY_PAYLOAD_SHA256`; the S3 family also\n"
         "    ///   accepts `UNSIGNED_PAYLOAD`, which"),
    ],
    "rust/src/aws/properties.rs": [
        ("pub(crate) struct Identity {", "pub struct Identity {"),
        ("    pub(crate) fn read(&mut self, key: &str, name: &str, value: &str) -> Result<bool> {",
         "    pub fn read(&mut self, key: &str, name: &str, value: &str) -> Result<bool> {"),
        ("    pub(crate) fn apply(&self, session: &Session) -> Result<Session> {",
         "    pub fn apply(&self, session: &Session) -> Result<Session> {"),
        ("pub(crate) enum EndpointName {", "pub enum EndpointName {"),
        ("    pub(crate) fn of(key: &str) -> Option<Self> {", "    pub fn of(key: &str) -> Option<Self> {"),
        ("    /// Any other service, by the service id after [`SERVICE_ENDPOINT`]\n",
         "    /// Any other service, by the service id after `SERVICE_ENDPOINT`\n"),
    ],
    "rust/src/xml/scanner.rs": [
        ("#[derive(Debug, Default)]\npub struct Element {\n",
         "/// One element of a small fixed-shape document: its name without a\n"
         "/// namespace prefix, its own text and its children in document order.\n"
         "#[derive(Debug, Default)]\npub struct Element {\n"),
        ("    /// The text of the first child named `name`, whose absence is an error.\n"
         "    pub fn required(&self, name: &str) -> Result<&str, XmlError> {\n",
         "    /// The text of the first child named `name`, whose absence is an error.\n"
         "    ///\n    /// # Errors\n    ///\n    /// An [`XmlError`] naming both elements where no such child is there.\n"
         "    pub fn required(&self, name: &str) -> Result<&str, XmlError> {\n"),
    ],
}


def raise_items(root: pathlib.Path) -> None:
    for rel, pairs in RAISES.items():
        edit(root, rel, *pairs)


IMPLEMENTER_DOC = [
    (("//! workspace: an item is listed because a crate split off the core - the\n"
      "//! market crate, the FIX crate, the media crates - needs it, and an item no\n"),
     "//! workspace: an item is listed because a crate split off the core - the\n"
     "//! market crate, the FIX crate, the media crates, the object-store crate -\n"
     "//! needs it, and an item no\n"),
]

IMPLEMENTER_SECTION = '''
// ------------------------------------------------------------------------
// Object stores: what the object-store crate reaches - the HTTP client's
// pool and retry rules, Signature Version 4 and the AWS property reader,
// the bearer lease, the XML scanner, the session's own doors, and the
// handle helpers a store's roles share with every backend.
// ------------------------------------------------------------------------

/// `iobase::oversized`, for the object-store crate: the refusal of a value
/// too large for the platform's addressable memory.
#[inline]
#[must_use]
pub fn oversized(size: u64) -> crate::Error {
    crate::iobase::oversized(size)
}

/// `iobase::read_upload`, for the object-store crate: exactly `length` bytes
/// of `source`, what an upload of a stated length sends.
///
/// # Errors
///
/// Returns the reader's failure, or the refusal of a source that ends short
/// of `length`.
#[inline]
pub fn read_upload(source: &mut dyn std::io::Read, length: u64) -> Result<Vec<u8>> {
    crate::iobase::read_upload(source, length)
}

/// `iobase::short_upload`, for the object-store crate: the refusal of an
/// upload whose source ended at `got` bytes of the `expected`.
#[inline]
#[must_use]
pub fn short_upload(expected: u64, got: u64) -> crate::Error {
    crate::iobase::short_upload(expected, got)
}

/// `holder::sibling`, for the object-store crate: the handle beside
/// `handle` that its parent holds under its own name, where it has both.
///
/// # Errors
///
/// Returns the parent's refusal to resolve the child.
#[inline]
pub fn sibling<H: IOBase + ?Sized>(handle: &H) -> Result<Option<crate::holder::Holder>> {
    crate::holder::sibling(handle)
}

/// `holder::system_time_ns`, for the object-store crate: an instant as
/// nanoseconds from the epoch, counting backwards before it.
#[inline]
#[must_use]
pub fn system_time_ns(value: std::time::SystemTime) -> Option<i64> {
    crate::holder::system_time_ns(value)
}

/// `uri::percent_encode_segment`, for the object-store crate: one path
/// segment escaped, its `/` included.
#[inline]
#[must_use]
pub fn percent_encode_segment(value: &str) -> Cow<'_, str> {
    crate::uri::percent_encode_segment(value)
}

/// `integer::BYTE_COUNT_SPELLINGS`, for the object-store crate: what every
/// refusal of a byte count names.
pub const BYTE_COUNT_SPELLINGS: &str = crate::integer::BYTE_COUNT_SPELLINGS;

/// `integer::byte_count_from_text`, for the object-store crate: a byte count
/// with an optional `KiB`, `MiB` or `GiB` suffix.
#[inline]
#[must_use]
pub fn byte_count_from_text(text: &str) -> Option<u64> {
    crate::integer::byte_count_from_text(text)
}

/// `integer::integer_from_scalar_as`, for the object-store crate: an
/// integer a value states, read at one native width.
#[inline]
#[must_use]
pub fn integer_from_scalar_as<T: TryFrom<i128> + TryFrom<u128>>(value: &Scalar) -> Option<T> {
    crate::integer::integer_from_scalar_as(value)
}

/// `xxhash::stream::read_range_digest`, for the object-store crate: the
/// digest of `length` bytes of a handle from `offset`, clamped as a ranged
/// read is.
///
/// # Errors
///
/// Returns the refusal of a container, or the handle's read failure.
#[inline]
pub fn read_range_digest<H: IOBase + ?Sized>(
    handle: &H,
    offset: u64,
    length: usize,
    algorithm: crate::DigestAlgorithm,
) -> Result<crate::Digest> {
    crate::xxhash::stream::read_range_digest(handle, offset, length, algorithm)
}

/// `ByteStream::from_handle`, for the object-store crate: a handle's bytes
/// from `position` in chunks of `batch_size`, read positionally.
///
/// # Errors
///
/// Returns the handle's refusal to be read.
#[inline]
pub fn byte_stream_from_handle<'source, H: IOBase + ?Sized>(
    handle: &'source H,
    position: u64,
    batch_size: usize,
) -> Result<crate::ByteStream<'source>> {
    crate::ByteStream::from_handle(handle, position, batch_size)
}

/// `http::client::agent_for`, for the object-store crate: the one door every
/// client in the build takes its connections through.
///
/// # Errors
///
/// Returns the refusal of a proxy or a TLS setting the options state.
#[cfg(feature = "http")]
#[inline]
pub fn agent_for(
    options: &crate::http::HttpOptions,
    tls: Option<ureq::tls::TlsConfig>,
) -> Result<ureq::Agent> {
    crate::http::client::agent_for(options, tls)
}

/// `http::client::record_process`, for the object-store crate: one request
/// to `host` counted in the process ledger every client records into.
#[cfg(feature = "http")]
#[inline]
pub fn record_process(host: &str, method: &str) {
    crate::http::client::record_process(host, method);
}

/// The retry budget every retrying client spends, for the object-store
/// crate's client.
#[cfg(feature = "http")]
pub use crate::http::retry::RetryBudget;

/// `http::retry::RETRY_BACKOFF`, for the object-store crate: the pause
/// before the first retry.
#[cfg(feature = "http")]
pub const RETRY_BACKOFF: std::time::Duration = crate::http::retry::RETRY_BACKOFF;

/// `http::retry::RETRY_COST`, for the object-store crate: what one retry
/// costs the budget.
#[cfg(feature = "http")]
pub const RETRY_COST: i64 = crate::http::retry::RETRY_COST;

/// `http::retry::RETRY_REFUND`, for the object-store crate: what a
/// first-attempt success refunds.
#[cfg(feature = "http")]
pub const RETRY_REFUND: i64 = crate::http::retry::RETRY_REFUND;

/// `http::retry::backoff`, for the object-store crate: the window attempt
/// `attempt + 1` is drawn from.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn backoff(attempt: u32) -> std::time::Duration {
    crate::http::retry::backoff(attempt)
}

/// `http::retry::delay`, for the object-store crate: the pause before
/// attempt `attempt`, a server's own short ask honoured.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn delay(
    attempt: u32,
    asked: Option<std::time::Duration>,
    jitter: &std::sync::atomic::AtomicU64,
) -> std::time::Duration {
    crate::http::retry::delay(attempt, asked, jitter)
}

/// `http::retry::fresh_jitter`, for the object-store crate: a client's
/// jitter seed.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn fresh_jitter() -> u64 {
    crate::http::retry::fresh_jitter()
}

/// `http::retry::retry_after`, for the object-store crate: the pause a
/// `Retry-After` value asks for.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn retry_after(value: Option<&str>) -> Option<std::time::Duration> {
    crate::http::retry::retry_after(value)
}

/// `http::retry::is_retryable_transport`, for the object-store crate:
/// whether a transport failure may be sent again.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_retryable_transport(error: &ureq::Error) -> bool {
    crate::http::retry::is_retryable_transport(error)
}

/// `http::retry::is_unsent`, for the object-store crate: whether a failure
/// happened before any connection took the request.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_unsent(error: &ureq::Error) -> bool {
    crate::http::retry::is_unsent(error)
}

/// `http::retry::is_resumable`, for the object-store crate: whether a read
/// failure is the transport's rather than the server's verdict.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_resumable(error: &std::io::Error) -> bool {
    crate::http::retry::is_resumable(error)
}

/// The text that renders as `<redacted>`, for the object-store crate: what a
/// bearer token holds.
#[cfg(feature = "http")]
pub use crate::auth::secret::Secret;

/// `auth::variable`, for the object-store crate: a non-empty process
/// environment variable, trimmed.
#[cfg(feature = "http")]
pub use crate::auth::environment::variable;

/// The bearer token, its lease and its expiry, for the object-store crate's
/// Google and Azure dialects; `instant` reads an expiry a tool wrote.
#[cfg(feature = "aws")]
pub use crate::auth::lease::{Bearer, Expiring, Lease, instant};

/// The AWS property reader's collection and its endpoint names, for the
/// object-store crate's property reader.
#[cfg(feature = "aws")]
pub use crate::aws::properties::{EndpointName, Identity};

/// The signer of one credential set bound to a region and a service, for
/// the object-store crate's client.
#[cfg(feature = "aws")]
pub use crate::aws::sigv4::Signer;

/// `aws::sigv4::EMPTY_PAYLOAD_SHA256`, for the object-store crate: the
/// payload hash of an empty body.
#[cfg(feature = "aws")]
pub const EMPTY_PAYLOAD_SHA256: &str = crate::aws::sigv4::EMPTY_PAYLOAD_SHA256;

/// `aws::sigv4::UNSIGNED_PAYLOAD`, for the object-store crate: the payload
/// hash of a body sent unsigned.
#[cfg(feature = "aws")]
pub const UNSIGNED_PAYLOAD: &str = crate::aws::sigv4::UNSIGNED_PAYLOAD;

/// `aws::sigv4::canonical_query`, for the object-store crate: the canonical
/// query string of raw name and value pairs.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn canonical_query(query: &[(String, String)]) -> String {
    crate::aws::sigv4::canonical_query(query)
}

/// `aws::sigv4::encode_key`, for the object-store crate: one raw object key
/// percent-encoded for the request path, its `/` kept.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn encode_key(key: &str) -> String {
    crate::aws::sigv4::encode_key(key)
}

/// `aws::sigv4::encode_query_component`, for the object-store crate: one
/// query name or value percent-encoded.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn encode_query_component(text: &str) -> String {
    crate::aws::sigv4::encode_query_component(text)
}

/// `aws::sigv4::sha256_hex`, for the object-store crate: lowercase hex
/// SHA-256 of `bytes`.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::aws::sigv4::sha256_hex(bytes)
}

/// `aws::sigv4::signed_access_key`, for the object-store crate: the access
/// key an `Authorization` header was signed with.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn signed_access_key(authorization: &str) -> Option<&str> {
    crate::aws::sigv4::signed_access_key(authorization)
}

/// `aws::properties::count`, for the object-store crate: a whole number a
/// property states, refused naming it.
///
/// # Errors
///
/// Returns the refusal naming `name` where `value` is no count.
#[cfg(feature = "aws")]
#[inline]
pub fn count(name: &str, value: &str) -> Result<u32> {
    crate::aws::properties::count(name, value)
}

/// `aws::properties::flag`, for the object-store crate: a boolean a
/// property states, read through the one boolean table.
///
/// # Errors
///
/// Returns the refusal naming `name` where `value` is no boolean.
#[cfg(feature = "aws")]
#[inline]
pub fn flag(name: &str, value: &str) -> Result<bool> {
    crate::aws::properties::flag(name, value)
}

/// `aws::properties::refusal`, for the object-store crate: the error a
/// property reader refuses a value with.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn refusal(message: &str) -> crate::Error {
    crate::aws::properties::refusal(message)
}

/// `aws::properties::seconds`, for the object-store crate: a duration a
/// property states in seconds.
///
/// # Errors
///
/// Returns the refusal naming `name` where `value` is no duration.
#[cfg(feature = "aws")]
#[inline]
pub fn seconds(name: &str, value: &str) -> Result<std::time::Duration> {
    crate::aws::properties::seconds(name, value)
}

/// `aws::environment::is_native`, for the object-store crate: whether a
/// variable is one the AWS tools read for themselves, which the S3 options'
/// sweep leaves to the session.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn is_native(name: &str) -> bool {
    crate::aws::environment::is_native(name)
}

/// The element of a small fixed-shape document and its refusal, for the
/// object-store crate's XML vocabularies.
#[cfg(feature = "aws")]
pub use crate::xml::scanner::{Element as ScannedElement, XmlError};

/// `xml::scanner::parse_document`, for the object-store crate: the root
/// element of `xml`.
///
/// # Errors
///
/// Returns the refusal of malformed XML or anything outside the root.
#[cfg(feature = "aws")]
#[inline]
pub fn parse_document(xml: &[u8]) -> std::result::Result<ScannedElement, XmlError> {
    crate::xml::scanner::parse_document(xml)
}

/// `xml::scanner::parse_root`, for the object-store crate: the root of
/// `xml`, which must be named `expected`.
///
/// # Errors
///
/// Returns the refusal of malformed XML or of another root.
#[cfg(feature = "aws")]
#[inline]
pub fn parse_root(xml: &[u8], expected: &str) -> std::result::Result<ScannedElement, XmlError> {
    crate::xml::scanner::parse_root(xml, expected)
}

/// `ArnPartition::check_region`, for the object-store crate: a region read
/// off a location, a header or a profile refused unless it is one a host can
/// be spelled with.
///
/// # Errors
///
/// Returns the refusal naming `source` and the region.
#[cfg(feature = "aws")]
#[inline]
pub fn arn_partition_check_region(region: &str, source: &str) -> Result<()> {
    crate::ArnPartition::check_region(region, source)
}

/// `Session::signer`, for the object-store crate: the signer of the set in
/// hand at `now` for `service` in `region`; `None` for unsigned requests.
///
/// # Errors
///
/// Returns the credential chain's refusal.
#[cfg(feature = "aws")]
#[inline]
pub fn session_signer(
    session: &crate::aws::Session,
    service: &str,
    region: &str,
    now: std::time::SystemTime,
) -> Result<Option<std::sync::Arc<Signer>>> {
    session.signer(service, region, now)
}

/// `Session::answers_another`, for the object-store crate: whether a store's
/// refusal of the key `signed` leaves the session another set to sign with.
///
/// # Errors
///
/// Returns the session's own refusal once nothing answers any more.
#[cfg(feature = "aws")]
#[inline]
pub fn session_answers_another(
    session: &crate::aws::Session,
    signed: &str,
    code: Option<&str>,
    endpoint: &str,
    region: &str,
    now: std::time::SystemTime,
) -> Result<bool> {
    session.answers_another(signed, code, endpoint, region, now)
}

/// `Session::given_variables`, for the object-store crate: the environment a
/// caller handed the session instead of the process's.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_given_variables(
    session: &crate::aws::Session,
) -> Option<&std::collections::BTreeMap<String, String>> {
    session.given_variables()
}

/// `Session::tls_config`, for the object-store crate: the TLS setup the
/// session's certificate bundle states.
///
/// # Errors
///
/// Returns the refusal of a bundle that cannot be read.
#[cfg(feature = "aws")]
#[inline]
pub fn session_tls_config(session: &crate::aws::Session) -> Result<Option<ureq::tls::TlsConfig>> {
    session.tls_config()
}

/// `Session::bucket_region`, for the object-store crate: the region a
/// redirect found `bucket` in on `partition`'s hosts, when learned.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_bucket_region(session: &crate::aws::Session, partition: &str, bucket: &str) -> Option<String> {
    session.bucket_region(partition, bucket)
}

/// `Session::learn_bucket_region`, for the object-store crate: remember that
/// `bucket` answers in `region`, for every session sharing this one's.
#[cfg(feature = "aws")]
#[inline]
pub fn session_learn_bucket_region(session: &crate::aws::Session, bucket: &str, region: &str) {
    session.learn_bucket_region(bucket, region);
}

/// `Session::stated_region`, for the object-store crate: the region the
/// session was told, nothing resolved.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_stated_region(session: &crate::aws::Session) -> Option<&str> {
    session.stated_region()
}

/// `Session::states_identity`, for the object-store crate: whether the
/// session was told who to sign as.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_states_identity(session: &crate::aws::Session) -> bool {
    session.states_identity()
}

/// `Session::under`, for the object-store crate: `session` with what it
/// leaves unstated taken from `ambient`.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_under(session: &crate::aws::Session, ambient: &crate::aws::Session) -> crate::aws::Session {
    session.under(ambient)
}
'''


def edit_implementer(root: pathlib.Path) -> None:
    rel = "rust/src/implementer.rs"
    edit(root, rel, *IMPLEMENTER_DOC)
    text = read(root / rel)
    if "// Object stores: what the object-store crate reaches" in text:
        return
    for name in ("Cow", "IOBase", "Scalar", "Result"):
        if not re.search(rf"\b{name}\b", text.split("// ----", 1)[0]):
            residue(rel, f"`{name}` is not imported at the top: the object-store section names it")
    write(root / rel, text.rstrip("\n") + "\n" + IMPLEMENTER_SECTION)


def edit_s3tables(root: pathlib.Path) -> list[str]:
    """The core's `s3tables/` (Iceberg's, D15) opens a table's store through
    the backend: re-spelled onto the crate, which the Iceberg crate's
    `s3tables` feature links (D36.6); the core cannot until it moves."""
    touched = []
    for path in sorted((root / "rust/src/s3tables").rglob("*.rs")):
        rel = str(path.relative_to(root))
        text = read(path)
        new = re.sub(r"(?<![\w:])crate::s3::", "yggdryl_s3::", text)
        if new != text:
            write(path, new)
            touched.append(rel)
    return touched


def edit_facade(root: pathlib.Path) -> None:
    rel = "rust/src/logging/facade.rs"
    text = read(root / rel)
    if '("yggdryl_s3", "yggdryl.s3")' not in text:
        m = re.search(r"const CRATES: \[\(&str, &str\); (\d+)\] = \[\n((?:    \([^\n]*\),\n)+)\];", text)
        if not m:
            residue(rel, "no `CRATES` table to add `yggdryl_s3` to")
            return
        rows = m.group(2).splitlines(keepends=True)
        rows.append('    ("yggdryl_s3", "yggdryl.s3"),\n')
        rows.sort(key=lambda row: re.search(r'"([^"]+)"', row).group(1))
        new = f"const CRATES: [(&str, &str); {len(rows)}] = [\n" + "".join(rows) + "];"
        text = text[:m.start()] + new + text[m.end():]
        write(root / rel, text)
    edit(root, "rust/tests/logging/facade.rs",
         ('        ("yggdryl_parquet::reader", "yggdryl.parquet.reader"),\n',
          '        ("yggdryl_parquet::reader", "yggdryl.parquet.reader"),\n'
          '        // The object stores are the `s3` folder they left.\n'
          '        ("yggdryl_s3::client", "yggdryl.s3.client"),\n'))


# ---------------------------------------------------------------------------
# Step 5: paths everywhere
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


S3_NAMED = re.compile(r"\byggdryl::(?:s3\b|internals::s3_|\{[^;]*\bs3\b)")


def sibling_leaves(root: pathlib.Path) -> list[pathlib.Path]:
    return [m for m in sorted((root / "rust").glob("*/Cargo.toml"))
            if m.parent.name not in CORE_FOLDERS and m.parent.name != S3]


def markdown(rewriter, text: str, where: str) -> str:
    outside = s4.Context(None, False)
    out: list[str] = []
    pos = 0
    for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
        prose = text[pos:m.start(3)]
        out.append(rewriter.inline(prose, outside, where) if S3_NAMED.search(prose) else prose)
        body = m.group(3)
        info = m.group(2).strip().lstrip("{").strip()
        if info.startswith((".rust", "rust")) and S3_NAMED.search(body):
            indent = m.group(1)
            lines = body.split("\n")
            if indent and not all(l.startswith(indent) or not l.strip() for l in lines):
                residue(f"{where}:{line_of(text, m.start(3))}", "a Rust block with a line outside its indent names the backend: re-spell it by hand")
            elif indent:
                stripped = "\n".join(l[len(indent):] for l in lines)
                new = rewriter.rust(stripped, outside, where)
                body = "\n".join((indent + l) if l else l for l in new.split("\n"))
            else:
                body = rewriter.rust(body, outside, where)
        elif S3_NAMED.search(body):
            body = rewriter.inline(body, outside, where)
        out.append(body)
        pos = m.end(3)
    rest = text[pos:]
    out.append(rewriter.inline(rest, outside, where) if S3_NAMED.search(rest) else rest)
    return "".join(out)


# A module path a page or a skill spells beside the crate's free doors.
MODULE_DOORS = re.compile(
    r"(?<![\w:$/.])s3::(file_at_with|folder_at_with|path_at_with|located_with|file_with|folder_with|"
    r"file_at|folder_at|path_at|located|file|folder)\b"
)


def comment_doors(text: str) -> str:
    """The crate's free doors a comment spells through the module it was,
    named by the crate."""
    if not MODULE_DOORS.search(text):
        return text
    return "\n".join(MODULE_DOORS.sub(r"yggdryl_s3::\1", line) if re.match(r"\s*//", line) else line
                     for line in text.split("\n"))


def rewrite_paths(root: pathlib.Path, rewriter, tables: S3Tables, paths: PathMap, moves, splits) -> None:
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new[new] = old
    crate_ctx = s4.Context(S3, True)
    outside = s4.Context(None, False)
    count = 0
    local = paths.with_files({o: n for o, n in splits})
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        old = moved_new.get(rel, rel)
        text = read(path)
        new = s4.reanchor(text, old, rel, local)
        ctx = crate_ctx if rel.startswith(f"{CRATE_DIR}/src/") else outside
        new = comment_doors(unalias(rewriter.rs_file(new, ctx, rel)))
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
            # Only a file naming the backend is read by the rewriter: the core's
            # routes re-own nothing a file of the core spells for itself.
            if S3_NAMED.search(new):
                new = comment_doors(unalias(rewriter.rs_file(new, outside, rel)))
            if new != text:
                write(path, new)
                count += 1
    for leaf in sibling_leaves(root):
        for path in sorted(leaf.parent.rglob("*.rs")):
            rel = str(path.relative_to(root))
            if "/target/" in rel:
                continue
            text = read(path)
            new = comment_doors(unalias(rewriter.rs_file(text, outside, rel))) if S3_NAMED.search(text) else text
            if new != text:
                write(path, new)
                count += 1
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text = read(root / f)
            new = text
            if S3_NAMED.search(text):
                new = markdown_unalias(markdown(rewriter, text, f))
            new = MODULE_DOORS.sub(r"yggdryl_s3::\1", new)
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
    for f in text_files(root):
        text = read(root / f)
        new = rewrite_text_paths(text, moves)
        if new != text:
            write(root / f, new)
            count += 1
    done(f"file paths re-pointed in {count} files")


S3_HARNESS_DOC = """//! One test file per file under `rust/s3/src/` that pins something of its
//! own, mirrored file for file: [`client`], [`encryption`], [`file`],
//! [`folder`], [`options`], [`path`], [`properties`], [`provider`] and
//! [`xml`] here, and the files each dialect owns under [`aws`],
//! [`azure`] and [`google`]. Who a request signs as - the credential chain,
//! the profile, a role, a sign-in - and the Signature Version 4 signing
//! itself are the core's `aws` module's, so they are pinned under the core's
//! `rust/tests/aws/` (the signing in `rust/tests/aws/sigv4.rs`); what stays
//! here is how the backend wires a session in, which the suites above drive
//! over a socket.
//!
//! The suites that pin something a caller cannot reach - the two XML
//! vocabularies, the addressing, the payload-signing policy - carry the
//! `internals` cfg, on the module or on a module inside the file, and reach
//! the crate through `yggdryl_s3::internals`. Everything else reaches it
//! through `yggdryl_s3::` like any other caller, after
//! [`install::installed`] has claimed the backend.
//!
//! [`server`] is not a suite either: it is the in-process store that speaks
//! all three dialects, and answers STS's `AssumeRole` on the same endpoint,
//! the core's `rust/tests/support/server.rs` declared here once so every
//! suite over a socket shares one fixture, and [`mod_`] is the
//! handle-building that goes with it.
"""

OLD_HARNESS_PARAGRAPH = (
    "//! The whole backend is behind the `s3` feature, so every module here\n"
    "//! carries that cfg; the ones that pin something a caller cannot reach - the\n"
    "//! two XML vocabularies, the addressing, the payload-signing policy - carry\n"
    "//! the `internals` cfg beside it, on the module or on a module inside the\n"
    "//! file, and reach the crate through `yggdryl::internals`. Everything else\n"
    "//! reaches it through `yggdryl::` like any other caller.\n"
)
NEW_HARNESS_PARAGRAPH = (
    "//! The ones that pin something a caller cannot reach - the two XML\n"
    "//! vocabularies, the addressing, the payload-signing policy - carry the\n"
    "//! `internals` cfg, on the module or on a module inside the file, and reach\n"
    "//! `yggdryl-s3` through `yggdryl_s3::internals`. Everything else reaches it\n"
    "//! through `yggdryl_s3::` like any other caller.\n"
)


BACKEND_CLAIM_DOC = """//! The tests of `rust/tests/holder/backend.rs`, which pins
//! `rust/src/holder/backend.rs`, that pin the object stores' claim on the
//! core's register of storage backends, moved (S6d, D39) with the fixture
//! backend a rival claim is refused through: the claim is `yggdryl-s3`'s
//! `install()`, so its tests live in `yggdryl-s3`.
//!
//! A claim is process-wide and never withdrawn: each test opens with
//! `crate::install::installed`, and a refused claim leaves the register as
//! it was.
"""


def finish_tests(root: pathlib.Path) -> None:
    """The crate's tests: the backend is the crate's whatever a feature says,
    the harness's sentences about itself, the claim they pin now the crate's."""
    for path in sorted((root / CRATE_DIR).rglob("*.rs")):
        rel = str(path.relative_to(root))
        if rel.startswith(f"{CRATE_DIR}/src/"):
            continue
        text = read(path)
        new = text.replace("`yggdryl::internals`", "`yggdryl_s3::internals`") if "/tests/s3" in rel else text
        if rel == f"{CRATE_DIR}/tests/s3.rs":
            new = re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", S3_HARNESS_DOC, new, count=1)
        new = resolve_gates(new, rel, held=True)
        # The crate links the core with `aws` whatever a feature says.
        if gate_re("aws").search(no_comments(new)):
            new = resolve_gates(new, rel, held=True, feature="aws")
        if new != text:
            write(path, new)
    rel = f"{CRATE_DIR}/tests/holder/backend.rs"
    if (root / rel).exists():
        edit(
            root, rel,
            ("/// The object stores' backend, which the core claims itself under the `s3`\n"
             "/// feature until `yggdryl-s3` does.\n",
             "/// The object stores' backend, which `yggdryl-s3`'s `install()` claims.\n"),
            ("mod core_claim {", "mod claim {"),
            ("    fn the_object_stores_ten_schemes_are_the_cores_own_claim() {",
             "    fn the_object_stores_ten_schemes_are_yggdryl_s3s_claim() {"),
            ('.expect("the core claims `s3`");', '.expect("`yggdryl-s3` claims `s3`");'),
            ("    fn a_second_claim_of_an_object_store_scheme_names_the_core() {",
             "    fn a_second_claim_of_an_object_store_scheme_names_yggdryl_s3() {"),
            ('            "expected to create a storage backend at \\"s3\\", got an existing yggdryl"\n',
             '            "expected to create a storage backend at \\"s3\\", got an existing yggdryl-s3"\n'),
            ("        // and the claimed one stays the core's.\n", "        // and the claimed one stays `yggdryl-s3`'s.\n"),
            ('            "expected to create a storage backend at \\"gs\\", got an existing yggdryl"\n',
             '            "expected to create a storage backend at \\"gs\\", got an existing yggdryl-s3"\n'),
            optional=True,
        )
        text = read(root / rel)
        text = re.sub(r"\A(?:[ \t]*//![^\n]*\n)+", BACKEND_CLAIM_DOC, text, count=1)
        write(root / rel, text)
    edit(root, "rust/tests/holder/backend.rs",
         ("//! The refusals come first and claim nothing - a refused claim leaves the\n"
          "//! register as it was - so they hold whatever else the process claimed.\n"
          "//! Then the core's own claim, the object stores' ten schemes under the `s3`\n"
          "//! feature. Then one test-only backend no crate claims, `ygshelf`: a\n",
          "//! The refusals come first and claim nothing - a refused claim leaves the\n"
          "//! register as it was - so they hold whatever else the process claimed;\n"
          "//! no backend is the core's own (the object stores' claim is pinned in\n"
          "//! `yggdryl-s3`). Then one test-only backend no crate claims, `ygshelf`: a\n"),
         optional=True)
    edit(root, "rust/tests/holder/backend.rs",
         ("    /// Without the feature that claims them, an object store's location is\n"
          "    /// a scheme no claim answers, refused as every other is.\n",
          "    /// With no crate claiming them, an object store's location is a scheme\n"
          "    /// no claim answers, refused as every other is.\n"),
         optional=True)


def edit_interop(root: pathlib.Path) -> None:
    edit(root, "rust/tests/interop.rs",
         ('#[cfg(feature = "s3")]\n#[path = "interop/s3/mod.rs"]\nmod s3;\n', ""))
    write(root / CRATE_DIR / "tests/interop.rs", INTEROP_HARNESS)
    for script, dialect in (("check_object_interop.py", "aws"), ("check_azure_interop.py", "azure"),
                            ("check_gcs_interop.py", "gcs")):
        edit(
            root, f"scripts/{script}",
            (f"``cargo test --features s3 --test interop s3::{dialect}::``",
             f"``cargo test -p yggdryl-s3 --test interop s3::{dialect}::``"),
            ('        str(REPO / "rust" / "Cargo.toml"),\n        "--features",\n        "s3",\n',
             '        str(REPO / "rust" / "s3" / "Cargo.toml"),\n'),
        )


INTEROP_HARNESS = """//! The object stores' exchanges with a real implementation of each store:
//! MinIO with boto3, Azurite with azure-storage-blob, `fake-gcs-server` with
//! google-cloud-storage. Each driver under `scripts/` starts its server and
//! runs its dialect's module here - `s3::aws::`, `s3::azure::`, `s3::gcs::`.

#[path = "interop/s3/mod.rs"]
mod s3;
"""


# ---------------------------------------------------------------------------
# Step 6: a sibling leaf's tests and benches reaching the backend
# ---------------------------------------------------------------------------

SIBLING_INSTALL = f'{CRATE}::install().expect("{PACKAGE} installs");'


def install_in_siblings(root: pathlib.Path) -> tuple[list[str], list[str]]:
    """A sibling leaf's test or bench reaching the backend stays where it is
    (D39: neither crate is below the other), its `s3` gates resolved - the
    backend is there through the dev-dependency - and each test that names
    the crate installing it first."""
    touched: list[str] = []
    linking: list[str] = []
    for manifest in sibling_leaves(root):
        leaf = manifest.parent
        files = [p for folder in ("tests", "benchmarks") if (leaf / folder).exists()
                 for p in sorted((leaf / folder).rglob("*.rs"))]
        reads = False
        for path in files:
            rel = str(path.relative_to(root))
            if "/target/" in rel:
                continue
            text = read(path)
            new = text
            if gate_re("s3").search(no_comments(new)):
                new = resolve_gates(new, rel, held=True)
            if re.search(rf"\b{CRATE}\b", code_only(new)):
                reads = True
                if "/tests/" in rel and "/support/" not in rel:
                    for at in reversed(test_fn_bodies(new)):
                        line_end = new.find("\n", at)
                        if SIBLING_INSTALL in new[at:at + 400]:
                            continue
                        line_start = new.rfind("\n", 0, at) + 1
                        indent = re.match(r"[ \t]*", new[line_start:]).group(0) + "    "
                        new = new[:at] + f"\n{indent}{SIBLING_INSTALL}" + new[at:]
            if new != text:
                write(path, new)
                touched.append(rel)
        # A bench root whose `main` installs a crate installs this one beside it.
        for path in files:
            text = read(path)
            m = re.search(r'(?m)^([ \t]*)(yggdryl_\w+)::install\(\)\.expect\("[^"]*"\);\n(?![ \t]*yggdryl_s3::install)', text)
            if m and "fn main()" in text and reads and SIBLING_INSTALL not in text:
                at = m.end()
                write(path, text[:at] + f"{m.group(1)}{SIBLING_INSTALL}\n" + text[at:])
                touched.append(str(path.relative_to(root)))
        manifest_text = read(manifest)
        if reads:
            linking.append(leaf.name)
            if PACKAGE not in manifest_text:
                if "[dev-dependencies]\n" in manifest_text:
                    manifest_text = manifest_text.replace(
                        "[dev-dependencies]\n", f"[dev-dependencies]\n{PACKAGE}.workspace = true\n", 1)
                else:
                    manifest_text = manifest_text.rstrip("\n") + f"\n\n[dev-dependencies]\n{PACKAGE}.workspace = true\n"
        # The core has no `s3` feature to forward to.
        manifest_text = re.sub(r'(?m)^s3 = \["yggdryl/s3"\]\n', "", manifest_text)
        if re.search(r'"yggdryl/s3"', manifest_text):
            residue(str(manifest.relative_to(root)), "forwards `yggdryl/s3`, which no longer exists: drop it")
        write(manifest, manifest_text)
    # The Parquet crate's object-store bench and suite, by name.
    edit(root, "rust/parquet/benchmarks/parquet.rs",
         ("//! `io_pushdown`, `calls/records`, `fs_record` and, with the `s3` feature,\n//! `object_records` - measured the same way.",
          "//! `io_pushdown`, `calls/records`, `fs_record` and `object_records` over\n"
          "//! `yggdryl-s3`'s stores - measured the same way."),
         ("/// The object-store group, with the backend compiled in.\n"
          "fn object_store_benchmarks(_criterion: &mut Criterion) {\n    s3::record_benchmarks(_criterion);\n}\n",
          "/// The object-store group, over `yggdryl-s3`'s in-process store.\n"
          "fn object_store_benchmarks(criterion: &mut Criterion) {\n    s3::record_benchmarks(criterion);\n}\n"),
         optional=True)
    for path in [root / "rust/parquet/tests/s3.rs"]:
        if path.exists():
            text = read(path)
            new = text.replace(OLD_HARNESS_PARAGRAPH, NEW_HARNESS_PARAGRAPH)
            if new != text:
                write(path, new)
    return sorted(set(touched)), linking


def leaf_after_s3(root: pathlib.Path, leaf: str) -> None:
    """A leaf whose tests link the crate names `s3` among what its `[leaves]`
    line is after (the Avro move's rule)."""
    rel = ".github/ci/rows.toml"
    text = read(root / rel)
    m = re.search(rf"(?m)^{re.escape(leaf)} = \{{ (.*) \}}$", text)
    if not m:
        residue(rel, f"no listed `[leaves]` line for `{leaf}`: say it is after `s3` by hand")
        return
    body = m.group(1)
    after = re.search(r"after = \[([^\]]*)\]", body)
    if after and '"s3"' in after.group(1):
        return
    if after:
        listed = after.group(1).strip()
        body = body[:after.start(1)] + (f'{listed}, "s3"' if listed else '"s3"') + body[after.end(1):]
    else:
        body = re.sub(r'(package = "[^"]+")', r'\1, after = ["s3"]', body, count=1)
    write(root / rel, text[:m.start(1)] + body + text[m.end(1):])


# ---------------------------------------------------------------------------
# Step 7: install
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = """//! The claim every harness of `yggdryl-s3` makes before a test reads a
//! location: the core's register answers the object stores' ten schemes
//! only once `yggdryl-s3` has claimed them (D7, D36), so each test opens
//! with [`installed`].

/// Claims the object stores' backend, once for the process.
pub fn installed() {
    yggdryl_s3::install().expect("yggdryl-s3 claims its backend");
}
"""

INSTALL_TESTS = """
/// `install()` is idempotent (D7): however often and from however many
/// threads it is called, `yggdryl-s3` claims the backend once, and the
/// register lists it once.
#[test]
fn install_claims_the_backend_once_however_often_it_is_called() {
    let calls: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| yggdryl_s3::install().is_ok()))
        .collect();
    for call in calls {
        assert!(call.join().expect("an install thread"), "every install answers Ok");
    }
    yggdryl_s3::install().expect("a later install returns at once");
    let backend: &dyn yggdryl::holder::StorageBackend = &yggdryl_s3::S3_BACKEND;
    let listed = yggdryl::holder::backends()
        .into_iter()
        .filter(|held| std::ptr::addr_eq(*held, backend))
        .count();
    assert_eq!(listed, 1);
    let claimed = yggdryl::holder::backend_for(&yggdryl::Scheme::S3).expect("`s3` is claimed");
    assert!(std::ptr::addr_eq(claimed, backend));
    assert_eq!(claimed.name(), "yggdryl-s3");
}

/// A second claim of the backend itself, under another claimant's name, is
/// refused naming `yggdryl-s3`, the first claimant (D7), and leaves the claim
/// standing.
#[test]
fn a_second_claim_of_the_backend_is_refused_naming_yggdryl_s3() {
    yggdryl_s3::install().expect("the backend is claimed");
    let refused = yggdryl::holder::claim_backend(&yggdryl_s3::S3_BACKEND, "another")
        .expect_err("the schemes are claimed");
    assert!(matches!(refused, yggdryl::Error::Conflict { .. }), "{refused}");
    assert_eq!(
        refused.to_string(),
        "expected to create a storage backend at \\"s3\\", got an existing yggdryl-s3"
    );
    let claimed = yggdryl::holder::backend_for(&yggdryl::Scheme::S3).expect("`s3` is claimed");
    assert_eq!(claimed.name(), "yggdryl-s3");
}
"""

S3_BENCH_ROOT = """//! The object stores' benchmarks: the S3 backend measured against
//! `object_store` on one in-process store, its byte, listing and record
//! groups. `main` claims the backend before any group runs.

#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

#[path = "s3/mod.rs"]
mod s3;

use criterion::criterion_group;

criterion_group!(
    object_stores,
    s3::bytes::byte_benchmarks,
    s3::listing::listing_benchmarks,
    s3::records::record_benchmarks,
);

fn main() {
    // The backend the benchmarks reach by scheme is claimed before any runs.
    yggdryl_s3::install().expect("yggdryl-s3 installs");
    object_stores();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
"""

# A Rust block reaching the backend through the core's register: a door
# that reads a location handed an object-store literal, or a variable bound
# to one; a plan reading one; or the register itself.
STORE_LITERAL = r'"(?:s3|s3a|s3n|gs|gcs|az|abfs|abfss|wasb|wasbs)://'
LOCATION_DOORS = (r"(?:Holder::from_url|Catalog::from_url|IcebergTable::from_url|"
                  r"IcebergTable::open_or_create_from_url|IcebergTable::create_from_url|from_location)")
CRATE_INSTALL = f"{CRATE}::install()"


def reaches_backend(body: str) -> bool:
    """The crate's own doors (`file`, `folder`, `located`, the options) build
    a handle with no claim, so they install nothing."""
    if re.search(r"\b(?:backend_for|backends|claim_backend)\b", body):
        return True
    bound = set(re.findall(r"let\s+(?:mut\s+)?(\w+)\s*(?::[^=]+)?=\s*[^;]*" + STORE_LITERAL, body))
    for m in re.finditer(LOCATION_DOORS + r"\s*\(\s*&?\s*([^,)]*)", body):
        argument = m.group(1).strip()
        if re.match(STORE_LITERAL, argument) or argument.split(".")[0] in bound:
            return True
    return bool(re.search(r"(?:from|into|to)\s+'(?:s3|s3a|s3n|gs|gcs|az|abfs|abfss|wasb|wasbs)://", body)
                and re.search(r"\bexecute(?:_in)?\b", body))


def markdown_installs(text: str) -> tuple[str, int]:
    """A Rust block reaching the backend through the register installs it
    first - beside an install the block already makes (the Parquet move's
    placement)."""
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        kind = info.strip().lstrip("{").strip()
        if not kind.startswith((".rust", "rust")) or CRATE_INSTALL in body or not reaches_backend(body):
            return m.group(0)
        lines = body.split("\n")
        main = next((k for k, line in enumerate(lines) if re.match(r"^\s*fn main\(", line)), None)
        others = [k for k, line in enumerate(lines) if re.match(r"^\s*yggdryl_\w+::install\(\)", line)]
        if others:
            k = others[-1]
            lead = re.match(r"[ \t]*", lines[k]).group(0)
            ends = "?;" if lines[k].rstrip().endswith("?;") else f'.expect("{PACKAGE} installs");'
            lines.insert(k + 1, f"{lead}{CRATE_INSTALL}{ends}")
        elif main is not None:
            k = main
            while k < len(lines) and not lines[k].rstrip().endswith("{"):
                k += 1
            if k >= len(lines):
                return m.group(0)
            ends = "?;" if "->" in "".join(lines[main:k + 1]) else f'.expect("{PACKAGE} installs");'
            inner = re.match(r"[ \t]*", lines[k]).group(0)
            lines.insert(k + 1, f"{inner}    {CRATE_INSTALL}{ends}")
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
                lines[at + 1:at + 1] = [f"{indent}{CRATE_INSTALL}?;", ""]
            else:
                lines.insert(at, f"{indent}{CRATE_INSTALL}?;")
        count += 1
        return f"{indent}```{info}\n" + "\n".join(lines) + f"{indent}```"

    return re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text), count


def insert_doc_installs(text: str) -> tuple[str, int]:
    """S4's hidden `install()` line opening every Rust example of a moved
    source, but where the example installs the crate itself."""
    lines = text.split("\n")
    out = []
    i = 0
    count = 0
    doc = re.compile(r"^([ \t]*//[/!])( ?)(.*)$")
    call = f"{CRATE}::install().unwrap();"
    while i < len(lines):
        m = doc.match(lines[i])
        fence = re.match(r"^\s*(`{3,}|~{3,})(.*)$", m.group(3)) if m else None
        if not (m and fence):
            out.append(lines[i])
            i += 1
            continue
        out.append(lines[i])
        marker, space = m.group(1), m.group(2) or " "
        ticks, rust = fence.group(1), s4.is_rust_fence(fence.group(2))
        i += 1
        block_start = len(out)
        body = []
        while i < len(lines):
            mm = doc.match(lines[i])
            if not mm or re.match(r"^\s*" + re.escape(ticks) + r"\s*$", mm.group(3)):
                break
            body.append(mm.group(3))
            out.append(lines[i])
            i += 1
        block_end = len(out)
        if i < len(lines):
            out.append(lines[i])
            i += 1
        if not rust or block_end == block_start or any(CRATE_INSTALL in b for b in body):
            continue
        main = next((k for k, b in enumerate(body) if re.match(r"^\s*(#\s*)?fn main\(", b)), None)
        if main is None:
            if body and body[0].lstrip().startswith("#!["):
                continue
            out.insert(block_start, f"{marker}{space}# {call}")
        else:
            k = main
            while k < len(body) and not body[k].rstrip().endswith("{"):
                k += 1
            if k >= len(body):
                continue
            out.insert(block_start + k + 1, f"{marker}{space}#     {call}")
        count += 1
    return "\n".join(out), count


def install_everywhere(root: pathlib.Path) -> None:
    base = root / CRATE_DIR
    write(base / "tests/support/install.rs", INSTALL_SUPPORT)
    harnesses = tests = 0
    for path in sorted((base / "tests").glob("*.rs")):
        text = s4.declare_install(read(path))
        write(path, re.sub(r'\n{3,}(#\[path = "support/install\.rs"\])', r"\n\n\1", text, count=1))
        harnesses += 1
    for path in sorted((base / "tests").rglob("*.rs")):
        if "/support/" in str(path):
            continue
        text, n = s4.insert_test_installs(read(path))
        tests += n
        write(path, text)
    # The claim's own tests, which install themselves.
    mod_ = base / "tests/s3/mod_.rs"
    if mod_.exists() and "install_claims_the_backend_once_however_often_it_is_called" not in read(mod_):
        write(mod_, read(mod_).rstrip("\n") + "\n" + INSTALL_TESTS)
    write(base / "benchmarks/s3.rs", S3_BENCH_ROOT)
    docs = 0
    for path in sorted((base / "src").rglob("*.rs")):
        text, n = insert_doc_installs(read(path))
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


def install_at_init(root: pathlib.Path) -> None:
    """The bindings' init and the CLI's `main` claim the backend after what
    they claim before it, in dependency order (D39)."""
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
            residue(rel, "no `module_init` function to install the backend in")
        else:
            line = f'    {CRATE}::install().expect("{PACKAGE} claims its backend");\n'
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
            "    // The object stores are linked where a feature asks for them: the\n"
            "    // default build stays the schema-only core.\n"
            '    #[cfg(feature = "s3")]\n'
            f"    if let Err(refusal) = {CRATE}::install() {{\n"
            "        style::bad(&refusal.to_string());\n"
            "        return ExitCode::FAILURE;\n"
            "    }\n"
        )
        write(root / rel, replace_once(text, anchor, block + anchor, rel))


# ---------------------------------------------------------------------------
# Step 8: manifests
# ---------------------------------------------------------------------------

LIB_IDENTS = {"md5": "md-5", "iceberg_official": "iceberg-official", "object_store": "object_store"}

# Why each dependency the crate takes from the core's manifest is there.
DEPENDENCY_NOTES = {
    "md-5": "# MD5 for the one S3 request that still demands `Content-MD5`: the bulk\n"
            "# delete. Same RustCrypto generation as `sha2`, so no second digest stack.\n",
    "ureq": "# The synchronous HTTP/1.1 client the core's `http` client is: the one pool\n"
            "# the core's `agent_for` builds, and the request and error types the retry\n"
            "# rules read, at the core's pin.\n",
    "object_store": "# The trusted Arrow-ecosystem S3 client the benchmarks are measured\n"
                    "# against, on the same in-process server and the same payloads; its\n"
                    "# streams are consumed through `futures`' combinators.\n",
}


def toml_value(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{k} = {toml_value(v)}" for k, v in value.items()) + " }"
    return str(value)


def dependency_line(name: str, spec: object) -> str:
    if isinstance(spec, dict):
        spec = {k: v for k, v in spec.items() if k != "optional"}
        if spec == {"workspace": True}:
            return f"{name}.workspace = true"
        return f"{name} = {toml_value(spec)}"
    return f"{name} = {toml_value(spec)}"


def crates_named(root: pathlib.Path, folders: list[str], files: list[str] = ()) -> set[str]:
    found = s4.crates_named(root, folders)
    for rel in files:
        if (root / rel).exists():
            code = code_only(read(root / rel))
            found |= set(re.findall(r"(?<![\w:$])([a-z_][a-z0-9_]*)::", code))
            found |= set(re.findall(r"\buse\s+([a-z_][a-z0-9_]*)\b", code))
    return found


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
    # The fixtures the tests and benches `#[path]`-include are theirs too.
    support = sorted({
        posixpath.normpath(posixpath.join(posixpath.dirname(str(p.relative_to(root))), m.group(1)))
        for folder in ("tests", "benchmarks") for p in (root / CRATE_DIR / folder).rglob("*.rs")
        for m in re.finditer(r'#\[path\s*=\s*"([^"]*support/[^"]+)"\]', read(p))
    })
    used_dev = crates_named(root, [f"{CRATE_DIR}/tests", f"{CRATE_DIR}/benchmarks"], support) - used
    lines = ['yggdryl = { workspace = true, features = ["aws"] }']
    for name in sorted(used - skip):
        if name in by_ident:
            package, spec = by_ident[name]
            lines.append(DEPENDENCY_NOTES.get(package, "") + dependency_line(package, spec))
        elif name not in {"aws", "azure", "client", "encryption", "file", "folder", "google", "options", "path",
                          "provider", "retry", "sigv4", "xml", "clippy", "collect", "downcast_ref", "parse",
                          "str", "i64", "u128", "u16", "u32", "u64", "usize", "u8", "char", "io", "range"}:
            residue(f"{CRATE_DIR}/Cargo.toml", f"the crate's sources name `{name}::`, which the core's manifest does not list")
    dev_lines = []
    for name in sorted(used_dev - skip):
        if name in dev_by_ident:
            package, spec = dev_by_ident[name]
            dev_lines.append(DEPENDENCY_NOTES.get(package, "") + dependency_line(package, spec))
        elif name in by_ident:
            package, spec = by_ident[name]
            dev_lines.append(dependency_line(package, spec))
    if not any(l.startswith("criterion") for l in dev_lines):
        dev_lines.append(dependency_line("criterion", dev.get("criterion", "0.7")))
    for sibling in sorted(n for n in used_dev if n.startswith("yggdryl_") and n != CRATE):
        dev_lines.append(f"{sibling.replace('_', '-')}.workspace = true")
    # The core's gates the crate's tests and benches read, forwarded.
    core_features = core_manifest.get("features", {})
    found: set[str] = set()
    for path in (root / CRATE_DIR).rglob("*.rs"):
        found |= set(re.findall(r'feature\s*=\s*"([\w-]+)"', no_comments(read(path))))
    found -= {"internals", "s3"}
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
        'description = "Object stores for yggdryl: Amazon S3, Google Cloud Storage and Azure Blob Storage behind the core\'s IOBase handles, each REST API spoken directly"\n'
        "version.workspace = true\n"
        "edition.workspace = true\n"
        "rust-version.workspace = true\n"
        "license.workspace = true\n"
        "repository.workspace = true\n"
        'readme = "README.md"\n'
        'keywords = ["s3", "gcs", "azure", "object-store", "storage"]\n'
        'categories = ["filesystem", "network-programming"]\n'
        "\n[features]\n"
        "# The core's gates this crate's tests and benchmarks read, forwarded; the\n"
        "# crate's own code reads none. `internals` makes `yggdryl_s3::internals`\n"
        "# exist for `tests/` and turns the core's on.\n"
        + feature_text + "\n"
        "\n[dependencies]\n"
        "# The core with `aws`: who this process is to AWS, Signature Version 4, and\n"
        "# the HTTP client every store is spoken to over.\n"
        + "\n".join(lines) + "\n"
        "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
        "\n# The object stores against `object_store` on one in-process store; `main`\n"
        "# claims the backend first.\n"
        '[[bench]]\nname = "s3"\npath = "benchmarks/s3.rs"\nharness = false\n'
    )
    write(root / CRATE_DIR / "Cargo.toml", manifest)
    readme = root / CRATE_DIR / "README.md"
    if not readme.exists():
        write(
            readme,
            f"# {PACKAGE}\n\nObject stores for yggdryl: Amazon S3, Google Cloud Storage and Azure Blob Storage "
            "behind the core's `IOBase` handles, each store's REST API spoken directly over synchronous "
            "HTTP/1.1 - no SDK, no async runtime.\n\n"
            "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the project's site, "
            "and `install()` claims the ten object-store schemes on the core's register of storage backends.\n",
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
    # The bindings link the crate and enable no `s3` of the core's.
    for member in ("python", "node"):
        rel = f"{member}/Cargo.toml"
        text = read(root / rel)
        text = text.replace('    "s3",\n', "", 1) if '    "s3",\n' in text else text
        if PACKAGE not in text:
            m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:yggdryl-[a-z0-9]+(?:\.workspace)? = [^\n]*\n)*", text)
            if not m:
                residue(rel, "no `yggdryl` dependency line to place the crate after")
            else:
                text = text[:m.end()] + f"{PACKAGE}.workspace = true\n" + text[m.end():]
        write(root / rel, text)
    edit_cli_manifest(root)
    done("manifests: rust/s3, the workspace, the bindings and the CLI")


def edit_cli_manifest(root: pathlib.Path) -> None:
    rel = "cli/Cargo.toml"
    text = read(root / rel)
    if PACKAGE in text:
        return
    m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n(?:yggdryl-[a-z0-9]+(?: = \{[^\n]*\}|\.workspace = true)\n)*", text)
    if not m:
        residue(rel, "no `yggdryl` dependency line to place the crate after")
        return
    text = text[:m.end()] + f"{PACKAGE} = {{ workspace = true, optional = true }}\n" + text[m.end():]
    text = edit_text(
        text, rel,
        ("# table (`s3tables://<bucket>/<namespace>/<table>`): the core's object store and\n"
         "# its table-bucket catalog. The staged binary enables `s3tables`, which implies\n"
         "# both, beside `iceberg`.\n"
         's3 = ["yggdryl/s3"]\n'
         's3tables = ["yggdryl/s3tables", "iceberg"]\n',
         "# table (`s3tables://<bucket>/<namespace>/<table>`): the object stores of\n"
         "# `yggdryl-s3`, linked and installed at `main`, and the core's table-bucket\n"
         "# catalog. The staged binary enables `s3tables`, which implies both, beside\n"
         "# `iceberg`.\n"
         's3 = ["dep:yggdryl-s3"]\n'
         's3tables = ["s3", "yggdryl/s3tables", "iceberg"]\n'),
    )
    dev = (
        "# The pages' Rust blocks name the object stores whatever the CLI's own\n"
        "# features, so the docs runner and the CLI's tests link them.\n"
        f"{PACKAGE}.workspace = true\n"
    )
    if "[dev-dependencies]\n" in text:
        text = text.replace("[dev-dependencies]\n", "[dev-dependencies]\n" + dev, 1)
    else:
        text = text.replace("\n[features]\n", "\n[dev-dependencies]\n" + dev + "\n[features]\n", 1)
    write(root / rel, text)


def edit_core_manifest(root: pathlib.Path) -> None:
    rel = "rust/Cargo.toml"
    text = read(root / rel)
    text = edit_text(
        text, rel,
        ("# Object stores behind `IOBase`: Amazon S3, Google Cloud Storage, Azure Blob\n"
         "# Storage. Not default: the signed HTTPS client and its TLS stack are a cost a\n"
         "# local or schema-only consumer never pays. Every protocol is spoken directly -\n"
         "# SigV4, OAuth 2.0 bearer tokens, and Azure Shared Key over a synchronous\n"
         "# HTTP/1.1 client - so no SDK, runtime, or async executor rides in.\n", ""),
        ("# exchanges, and Signature Version 4. Not default for the same reason as\n"
         "# `s3`: it is the signed HTTPS client. `s3` implies it, because every S3\n"
         "# request signs with what it answers.\n",
         "# exchanges, and Signature Version 4. Not default: it is the signed HTTPS\n"
         "# client. `yggdryl-s3` turns it on, because every object-store request signs\n"
         "# with what it answers, and so does `s3tables`.\n"),
        ('s3 = ["aws", "dep:md-5"]\n', ""),
        ("# one over the HTTP session `aws` already holds. It implies `s3`, because a\n"
         "# table's files live at an `s3:` warehouse location, and `iceberg`, because\n"
         "# what it catalogs are Iceberg tables and their schemas.\n"
         's3tables = ["s3", "iceberg"]\n',
         "# one over the HTTP session `aws` already holds. It implies `aws`, and\n"
         "# `iceberg`, because what it catalogs are Iceberg tables and their schemas;\n"
         "# a table's files live at an `s3:` warehouse location, which `yggdryl-s3`\n"
         "# holds - the Iceberg crate's `s3tables` links it (D36).\n"
         's3tables = ["aws", "iceberg"]\n'),
        ("# MD5 for the one S3 request that still demands `Content-MD5`: the bulk\n"
         "# delete. Same RustCrypto generation as `sha2`, so no second digest stack.\n"
         'md-5 = { version = "0.11", optional = true }\n', ""),
        ("# The trusted Arrow-ecosystem S3 client the S3 benchmarks are measured\n"
         "# against, on the same in-process server and the same payloads. Pinned to\n"
         "# the release that shares the reqwest and tokio versions the workspace\n"
         "# already locks, so the baseline adds no second HTTP stack.\n"
         "# The streams `object_store` answers with are consumed through its own\n"
         "# ecosystem's combinators; nothing here reaches a public signature.\n"
         'futures = "0.3"\n', ""),
        ('object_store = { version = "=0.13.1", features = ["aws"] }\n', ""),
    )
    # The holder bench's comment, as the core wrote it or as the Parquet move left it.
    bench_notes = [
        ("# S3 groups - measured against `object_store` on one in-process server -\n# only with the `s3` feature.\n",
         "# S3 groups - measured against `object_store` on one in-process server -\n# are `yggdryl-s3`'s own `s3` target.\n"),
        ("# the S3 groups - measured against `object_store` on one in-process server -\n# compile only with the `s3` feature.\n",
         "# the S3 groups - measured against `object_store` on one in-process server -\n# are `yggdryl-s3`'s own `s3` target.\n"),
    ]
    if not any(old in text for old, _ in bench_notes) and "are `yggdryl-s3`'s own `s3` target" not in text:
        residue(rel, "the holder bench's comment on the S3 groups was not found: re-word it by hand")
    for old, new in bench_notes:
        text = text.replace(old, new, 1)
    if "/s3" not in (re.search(r"(?m)^exclude = \[[^\]]*\]", text) or [""])[0]:
        m = re.search(r'(?m)^exclude = \[([^\]]*)\]', text)
        if m:
            text = text[:m.end(1)] + ', "/s3"' + text[m.end(1):]
        else:
            text = replace_once(
                text,
                'categories = ["data-structures", "encoding"]\n',
                'categories = ["data-structures", "encoding"]\n'
                "# The split crates sit inside this folder; each is its own package.\n"
                'exclude = ["/s3"]\n',
                rel,
            )
    # The S3 bench baseline was the core's alone: no other target names it.
    for crate in ("object_store", "futures", "md5"):
        users = [f for f in tracked(root, "rust/src", "rust/tests", "rust/benchmarks")
                 if f.endswith(".rs") and re.search(rf"(?<![\w:$]){crate}::|\buse\s+{crate}\b", code_only(read(root / f)))]
        if users:
            residue(rel, f"`{crate}` left the core's manifest but core targets still name it: {', '.join(users)}")
    write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 9: tooling
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
    """Where S4 (or an earlier S6 move) has not made them so, the two tools
    per crate, read off `s4_move.py` itself (the Parquet move's edits)."""
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
    # The docs runner compiles the pages' Rust blocks with features the core
    # no longer has, in whichever member S4 left it.
    rel = "scripts/check_docs_examples.py"
    runner = read(root / rel)
    m = re.search(r'"--features", "([^"]*)"', runner)
    if m:
        words = [w for w in m.group(1).split() if w not in ("s3", "yggdryl/s3")]
        runner = runner[:m.start(1)] + " ".join(words) + runner[m.end(1):]
        write(root / rel, runner)
    if '"--test", "docs_examples"' in runner and '"-p", "yggdryl-cli"' not in runner:
        residue(rel, "the Rust pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner "
                     "to `cli/tests/` (D13): a block naming `yggdryl_s3` compiles only there")


# ---------------------------------------------------------------------------
# Step 10: CI
# ---------------------------------------------------------------------------

S3_LEAF = 's3 = { package = "yggdryl-s3", jobs = ["object-interop", "azure-interop", "gcs-interop"] }\n'
NUMBERS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]
S3_EXCHANGES = ("object-interop", "azure-interop", "gcs-interop")


def edit_ci(root: pathlib.Path, linking: list[str]) -> None:
    rel = ".github/ci/rows.toml"
    text = read(root / rel)
    if S3_LEAF not in text:
        # After the leaves already listed, before the ones still commented out.
        lines = text.split("\n")
        start = lines.index("[leaves]") if "[leaves]" in lines else -1
        if start < 0:
            residue(rel, "no `[leaves]` table to list `s3` in")
        else:
            k = start + 1
            while k < len(lines) and lines[k].strip() and not lines[k].startswith("["):
                k += 1
            lines.insert(k, S3_LEAF.rstrip("\n"))
            text = "\n".join(lines)
    # The Iceberg crate links the backend under its `s3tables` feature.
    m = re.search(r'(?m)^(# )?iceberg = \{ package = "yggdryl-iceberg", after = \[([^\]]*)\]', text)
    if m and '"s3"' not in m.group(2):
        text = text[:m.end(2)] + ', "s3"' + text[m.end(2):]
    elif not m:
        residue(rel, "no `iceberg` leaf line to say it is after `s3`")
    for row in ("x-s3", "x-azure", "x-gcs"):
        text = text.replace(f'[rows.{row}]\nextends = ["interop"]\n', f'[rows.{row}]\nextends = ["crate-s3"]\n', 1)
    m = re.search(r"can break the build of all (\w+)", text)
    if m and m.group(1) in NUMBERS and "crate's target, read through the leaf's row" not in text:
        left = NUMBERS[NUMBERS.index(m.group(1)) - 3]
        text = edit_text(
            text, rel,
            ("# One `interop` target holds every exchange's Rust half, so a change to any\n"
             f"# of them can break the build of all {m.group(1)}.\n",
             "# One `interop` target holds the core's exchanges' Rust half, so a change to\n"
             f"# any of them can break the build of all {left}; a leaf's exchange is its own\n"
             "# crate's target, read through the leaf's row.\n"),
        )
    elif m and m.group(1) in NUMBERS:
        left = NUMBERS[NUMBERS.index(m.group(1)) - 3]
        text = text.replace(f"can break the build of all {m.group(1)};", f"can break the build of all {left};", 1)
    write(root / rel, text)
    for leaf in linking:
        leaf_after_s3(root, leaf)
    edit_planner_tests(root)
    edit(
        root, ".github/workflows/ci.yml",
        ("    # with each driver's own invocation - `--features s3` for S3, Azure and\n    # Google,",
         "    # with each driver's own invocation - `yggdryl-s3`'s `interop` target for\n    # S3, Azure and Google,"),
        ("    # with `s3` and Iceberg compiled; this job only reads it.\n",
         "    # with the object stores and Iceberg compiled; this job only reads it.\n"),
        ("        run: cargo test --locked --manifest-path rust/Cargo.toml --features s3 --test interop --no-run\n",
         "        run: cargo test --locked --manifest-path rust/s3/Cargo.toml --test interop --no-run\n"),
        ("      # The driver runs the `interop` target with `--features s3`, which the\n",
         "      # The driver runs `yggdryl-s3`'s `interop` target, which the\n"),
        ("  # generates an integration target and compiles it against `parquet iceberg\n  # s3 s3tables http3`,",
         "  # generates an integration target and compiles it against `parquet iceberg\n  # s3tables http3`,"),
    )


def edit_planner_tests(root: pathlib.Path) -> None:
    rel = "scripts/tests/test_ci_plan.py"
    text = read(root / rel)
    text = text.replace(
        'LEAF_LINE = re.compile(r"^(?:# )?([a-z]+) = \\{ package = .*\\}$", re.MULTILINE)',
        'LEAF_LINE = re.compile(r"^(?:# )?([a-z][a-z0-9]*) = \\{ package = .*\\}$", re.MULTILINE)', 1)
    if '"s3": {"object-interop", "azure-interop", "gcs-interop"},' not in text:
        text = edit_text(
            text, rel,
            ('        "iceberg": {"pyiceberg-interop", "spark-interop", "iceberg-msrv"},\n    }\n',
             '        "iceberg": {"pyiceberg-interop", "spark-interop", "iceberg-msrv"},\n'
             '        "s3": {"object-interop", "azure-interop", "gcs-interop"},\n    }\n'),
            ("    # What no leaf-only change runs.\n",
             "    # What no leaf-only change runs but the leaf's own exchanges.\n"),
            ("                self.assertFalse(self.NEVER & result.jobs)\n",
             "                self.assertFalse((self.NEVER - exchanges) & result.jobs)\n"),
        )
    m = re.search(
        r'(    def test_an_exchange_half_runs_every_exchange\(self\) -> None:\n'
        r'        jobs = planned\(\["rust/tests/interop/zip.rs"\]\).jobs\n'
        r'        self.assertTrue\(\{)([^}]*)(\} <= jobs\)\n)', text)
    if not m:
        residue(rel, "the exchange-half test was not found: say the object stores' exchanges are not the core's by hand")
    elif "yggdryl-s3" not in text:
        jobs = [j for j in re.findall(r'"([\w-]+)"', m.group(2)) if j not in S3_EXCHANGES]
        listed = ", ".join(f'"{j}"' for j in jobs)
        body = (m.group(1) + listed + m.group(3)
                + "        # The object stores' exchanges' Rust half is `yggdryl-s3`'s own target.\n"
                + "        for job in (\"object-interop\", \"azure-interop\", \"gcs-interop\"):\n"
                + "            self.assertNotIn(job, jobs)\n")
        text = text[:m.start()] + body + text[m.end():]
    write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 11: docs, skills, AGENTS.md, READMEs
# ---------------------------------------------------------------------------


def edit_docs(root: pathlib.Path) -> None:
    edit(
        root, "docs/holder/index.md",
        ("Azure Blob Storage, the storage backend the core claims | `s3` feature |",
         "Azure Blob Storage, the storage backend `yggdryl-s3` claims | the `yggdryl-s3` crate |"),
        ("the handle of the [storage backend](#storage-backends) the core claims under the `s3` feature - under the store's own properties",
         "the handle of the [storage backend](#storage-backends) `yggdryl-s3` claims - under the store's own properties"),
        ("The core claims the object stores' ten schemes itself under the `s3` feature ([Object stores](#object-stores)), until `yggdryl-s3` does.",
         "No backend is the core's own: `yggdryl-s3`'s `install()` claims the object stores' ten schemes ([Object stores](#object-stores))."),
        ("// The core claims the object stores' ten schemes itself, until `yggdryl-s3` does.\n",
         "// `install()` claims the object stores' ten schemes for `yggdryl-s3`.\n"),
        ('.expect("claimed under the s3 feature");', '.expect("claimed by `yggdryl_s3::install()`");'),
        ('assert_eq!(refused, "expected to create a storage backend at \\"s3\\", got an existing yggdryl");',
         'assert_eq!(refused, "expected to create a storage backend at \\"s3\\", got an existing yggdryl-s3");'),
        ("- no SDK, no async runtime. Behind the non-default `s3` feature. The scheme picks the store",
         "- no SDK, no async runtime. They are the `yggdryl-s3` crate, which every binding and the `yggdryl` "
         "command link and install; a Rust program calls `yggdryl_s3::install()` before `Holder::from_url` "
         "reads one of its locations. The scheme picks the store"),
        ("the [storage backend](#storage-backends) the core claims itself under the feature until `yggdryl-s3` does:",
         "the [storage backend](#storage-backends) `install()` claims:"),
        ("Behind the `aws` feature, which `s3` implies.", "Behind the `aws` feature, which `yggdryl-s3` turns on."),
        ("cargo bench --bench holder --features s3 -- object_ --noplot", "cargo bench -p yggdryl-s3 --bench s3 -- object_ --noplot"),
        ("HTTP/1.1 is behind the non-default `http` feature, which `aws` and `s3` imply;",
         "HTTP/1.1 is behind the non-default `http` feature, which `aws` implies and `yggdryl-s3` turns on;"),
    )
    edit(
        root, "docs/architecture.md",
        ("`local/`, `fs/`, `zip/`, `s3/`, the last the backend the core claims:",
         "`local/`, `fs/`, `zip/` - and the object stores' backend, the `yggdryl-s3` crate at `rust/s3/`:"),
        ("| `s3` | the Amazon S3, Google Cloud Storage and Azure Blob Storage backend; implies `aws` |\n", ""),
        ("Rust only); implies `s3` and `iceberg` |", "Rust only); implies `aws` and `iceberg`, a table's files `yggdryl-s3`'s |"),
        ("Every build - the default one, `s3`, `iceberg` and both bindings - compiles on Rust 1.94.",
         "Every build - the default one, `iceberg`, the leaf crates and both bindings - compiles on Rust 1.94."),
    )
    edit(
        root, "docs/contributing.md",
        ("one root folder per backend: `rust/src/local/`, `fs/`, `zip/`, `s3/` | [Holder](holder/index.md) |",
         "one root folder per backend: `rust/src/local/`, `fs/`, `zip/`, and the object stores' crate `rust/s3/` | [Holder](holder/index.md) |"),
    )
    edit(
        root, "docs/benchmarks.md",
        ('    cargo bench --bench holder --features "parquet s3"\n',
         "    cargo bench --bench holder --features parquet\n    cargo bench -p yggdryl-s3 --bench s3\n"),
    )
    edit(
        root, "docs/expression/plans.md",
        ("an object-store scheme needs the `s3` feature and reads its credentials from the properties",
         "an object-store scheme needs `yggdryl-s3`, installed, and reads its credentials from the properties"),
    )
    edit(
        root, "docs/media/iceberg.md",
        ("behind the `s3tables` feature (which implies `s3` and `iceberg`)",
         "behind the `s3tables` feature (which implies `aws` and `iceberg`; a table's files are `yggdryl-s3`'s)"),
        ("The client is behind the `s3tables` feature, which implies `s3` and `iceberg`.",
         "The client is behind the `s3tables` feature, which implies `aws` and `iceberg`."),
        ('cargo bench --features "iceberg s3" -p yggdryl --bench media', "cargo bench --features s3tables -p yggdryl --bench media"),
    )
    edit(
        root, "README.md",
        ("and the S3, Google Cloud Storage, and Azure Blob object stores behind the `s3`\nfeature)",
         "and the S3, Google Cloud Storage, and Azure Blob object stores in the\n`yggdryl-s3` crate)"),
        ("  src/{local,fs,zip,s3,http}/\n", "  src/{local,fs,zip,http}/\n"),
    )
    edit(root, "rust/README.md",
         ("src/s3/                S3Path, S3Folder, and S3File over the S3 dialect\n",
          "s3/                    The yggdryl-s3 crate: S3Path, S3Folder, and S3File\n"))
    edit(
        root, "skills/yggdryl-storage/SKILL.md",
        ("| object store (`s3` feature) |", "| object store (`yggdryl-s3`) |"),
    )
    edit(
        root, "skills/yggdryl-storage/references/backends.md",
        ("`yggdryl_s3::file/folder/located` (`s3` feature)", "`yggdryl_s3::file/folder/located` (`yggdryl-s3`)"),
        ("Rust: the non-default `s3` feature (implies `aws`). Python and Node.js: built\nin.",
         "Rust: the `yggdryl-s3` crate, its `install()` called before `Holder::from_url`\nreads one of its locations. Python and Node.js: built in."),
    )
    edit(
        root, "skills/yggdryl-storage/references/rust.md",
        ("object-safe: `&dyn IOBase`). Object stores need the `s3` feature, HTTP needs\n"
         "the `http` feature (`http2`/`http3` add the multiplexed versions; `aws` and\n"
         "`s3` already imply `http`), the AWS session needs `aws` (implied by `s3`);\n",
         "object-safe: `&dyn IOBase`). Object stores need the `yggdryl-s3` crate, HTTP\n"
         "needs the `http` feature (`http2`/`http3` add the multiplexed versions; `aws`\n"
         "already implies `http`), the AWS session needs `aws` (which `yggdryl-s3`\n"
         "turns on);\n"),
        ("- `Holder::from_url` with an `s3:`/`gs:`/`az:` scheme needs the `s3` feature,\n"
         "  under which the core claims the object stores' storage backend; without it\n",
         "- `Holder::from_url` with an `s3:`/`gs:`/`az:` scheme needs `yggdryl-s3`,\n"
         "  whose `install()` claims the object stores' storage backend; before it\n"),
    )
    rel = "skills/yggdryl/references/rust.md"
    if (root / rel).exists():
        text = read(root / rel)
        new = re.sub(r'(?m)^(# yggdryl = \{ version = "0\.1", features = \[[^\]]*?), "s3"\] \}\n',
                     r'\1] }\n'
                     "# Object stores - Amazon S3, Google Cloud Storage, Azure Blob Storage - their own\n"
                     "# crate; `yggdryl_s3::install()` claims their schemes before a location names one:\n"
                     '# yggdryl-s3 = "0.1"\n', text, count=1)
        new = new.replace("(needs Rust 1.94), object stores:\n", "(needs Rust 1.94):\n", 1)
        new = new.replace("| `s3` | Amazon S3, Google Cloud Storage, Azure Blob Storage handles | `aws` |\n", "", 1)
        if new == text:
            residue(rel, "the dependency block and feature table were not found: drop `s3` and name `yggdryl-s3` by hand")
        write(root / rel, new)


def edit_agents(root: pathlib.Path) -> None:
    edit(
        root, "AGENTS.md",
        ((' or `--features s3` | only when the change is under that gate |'),
         ' or `-p yggdryl-s3` | only when the change is under that gate |'),
        ("`aws` (implies `http`), `s3` (implies `aws`), `s3tables` (implies `s3` and\n`iceberg`)",
         "`aws` (implies `http`), `s3tables` (implies `aws` and\n`iceberg`)"),
        ("the object stores' ten schemes are the core's own claim under `s3` until `yggdryl-s3` does;",
         "the object stores' ten schemes are `yggdryl-s3`'s claim, made by its `install()`;"),
        ("`aws/`, `s3/google/` and `s3/azure/` carry only where their answer comes from",
         "`aws/` and `yggdryl-s3`'s `google/` and `azure/` carry only where their answer comes from"),
        ("under the non-default `aws` feature, which `s3` implies |",
         "under the non-default `aws` feature, which `yggdryl-s3` and `s3tables` turn on |"),
        ("`s3/` holds Amazon S3, Google Cloud Storage and Azure Blob Storage inside it, since all three answer that dialect, under the non-default `s3` feature - the one of the four that is no `Holder` arm: `S3_BACKEND`, the `StorageBackend` the core claims under the ten object-store schemes until `yggdryl-s3` does,",
         "`s3/` - in `yggdryl-s3`, at `rust/s3/` - holds Amazon S3, Google Cloud Storage and Azure Blob Storage inside it, since all three answer that dialect - the one of the four that is no `Holder` arm: `S3_BACKEND`, the `StorageBackend` `yggdryl_s3::install()` claims under the ten object-store schemes,"),
        ("under the non-default `s3tables` feature (`s3` and `iceberg`):", "under the non-default `s3tables` feature (`aws` and `iceberg`):"),
        ("its opener `s3::located_with` under", "its opener `yggdryl_s3::located_with` under"),
        ("### Object stores (`s3/`, non-default `s3` feature)\n", "### Object stores (`yggdryl-s3`, at `rust/s3/`)\n"),
        ("object-store schemes by the core itself until `yggdryl-s3`'s `install()` does,\n",
         "object-store schemes by `yggdryl_s3::install()`,\n"),
        ("| `excel` | Excel exchange |\n", "| `excel` | Excel exchange |\n| `s3` | S3 exchange, Azure exchange, Google exchange |\n"),
        ("claimed through `holder::claim_backend` - the core's own by the core itself until its crate's `install()` does - each role",
         "claimed through `holder::claim_backend` by its crate's `install()` - each role"),
        (("the market crate, the FIX crate and, from the media slice, the media crates; never a caller's API",),
         "the market crate, the FIX crate and, from the media slice, the media crates and the object-store crate; never a caller's API"),
    )
# ---------------------------------------------------------------------------
# Step 11 (cont.): the Rust inventory
# ---------------------------------------------------------------------------

S3_SECTION = re.compile(r"(?m)^### yggdryl(?:::|_)s3  \[rust/(?:src/s3/mod\.rs|s3/src/lib\.rs)\][^\n]*$")
S3_HEADER = (
    f"### {CRATE}  [{CRATE_DIR}/src/lib.rs]  (the crate root: the backend, its free doors, the three roles, "
    "the options and the claim; `internals` hidden)"
)
INSTALL_ENTRY = (
    "  pub fn install() -> Result<()>  (claims S3_BACKEND under the ten object-store schemes as `yggdryl-s3`, "
    "once for the life of the process, a later call returning at once; every binding's init and the `yggdryl` "
    "command's `main` call it; a scheme another crate claimed first refused naming it)\n"
)
INVENTORY_EDITS = [
    ("  Behind the non-default `s3` feature. Amazon S3, Google Cloud Storage,\n"
     "  and Azure Blob Storage through one location/container/leaf trio.\n",
     "  The `yggdryl-s3` crate. Amazon S3, Google Cloud Storage, and Azure Blob\n"
     "  Storage through one location/container/leaf trio.\n" + INSTALL_ENTRY),
    ("(claimed by the core itself under the ten object-store schemes before the register answers anything, "
     "until `yggdryl-s3` does)",
     "(claimed by `install()` under the ten object-store schemes)"),
    ("  pub use crate::aws::Credentials  (", "  pub use yggdryl::aws::Credentials  ("),
    ("(behind the `aws` feature, which `s3` implies: who this process is to AWS, and where AWS is)",
     "(behind the `aws` feature, which `yggdryl-s3` and the `s3tables` feature turn on: who this process is "
     "to AWS, and where AWS is)"),
    ("(behind the `s3tables` feature, which implies `s3` and `iceberg`: ",
     "(behind the `s3tables` feature, which implies `aws` and `iceberg`, a table's store being `yggdryl-s3`'s: "),
    ("  Behind the non-default `http` feature, which `aws` and `s3` imply.",
     "  Behind the non-default `http` feature, which `aws` implies."),
    ("; the core claims the object stores' backend under `s3` until `yggdryl-s3` does; re-exported from "
     "yggdryl::holder)",
     "; no backend is the core's own - the object stores' is `yggdryl-s3`'s, claimed by its `install()`; "
     "re-exported from yggdryl::holder)"),
    ("the object stores' ten schemes by the backend the core claims under `s3`;",
     "the object stores' ten schemes by the backend `yggdryl-s3` claims;"),
    ("(`Holder::is_backend_property`, crate-private: the object store options' under `s3`,",
     "(`Holder::is_backend_property`, crate-private: the object store options' once `yggdryl-s3` claims "
     "their schemes,"),
    ("- the market crate, the FIX crate and, from the media slice, the media crates;",
     "- the market crate, the FIX crate and, from the media slice, the media crates and the object-store "
     "crate;"),
    ("Sections: both crates, market, FIX, and what the media folders share with them.",
     "Sections: both crates, market, FIX, what the media folders share with them, and the object stores'."),
]


def implementer_entries() -> list[str]:
    """The inventory's lines for the implementer's object-store section, read
    off the section this script writes: each item's signature and the first
    paragraph of its doc."""
    entries = []
    lines = IMPLEMENTER_SECTION.split("\n")
    k = 0
    while k < len(lines):
        doc: list[str] = []
        while k < len(lines) and lines[k].startswith("///"):
            doc.append(lines[k][3:].strip())
            k += 1
        while k < len(lines) and lines[k].startswith("#["):
            k += 1
        if not doc or k >= len(lines) or not lines[k].startswith("pub "):
            k += 1
            continue
        head = lines[k]
        if head.startswith("pub fn"):
            sig = [head]
            while not sig[-1].rstrip().endswith("{"):
                k += 1
                sig.append(lines[k])
            text = " ".join(s.strip() for s in sig)
            text = re.sub(r"\s*\{$", "", text)
            text = re.sub(r"\(\s+", "(", text)
            text = re.sub(r",\s*\)", ")", text)
            text = re.sub(r"\s+", " ", text)
        elif head.startswith("pub const"):
            text = head.split(" = ", 1)[0]
        else:
            text = head.rstrip(";").replace("pub use crate::", "pub use ", 1)
        first = []
        for line in doc:
            if not line:
                break
            first.append(line)
        what = " ".join(first).rstrip(".")
        entries.append(f"{text}  ({what})")
        k += 1
    return entries


def edit_inventory(root: pathlib.Path) -> None:
    rel = ".api-inventory.txt"
    text = read(root / rel)
    m = S3_SECTION.search(text)
    if not m:
        residue(rel, "no `yggdryl::s3` section to re-home as the crate's")
    else:
        text = text[:m.start()] + S3_HEADER + text[m.end():]
    if INSTALL_ENTRY.strip() not in text:
        text = edit_text(text, rel, *INVENTORY_EDITS)
    marker = "  (Object stores: what the object-store crate reaches)\n"
    if marker not in text:
        start = text.find("### yggdryl::implementer  [rust/src/implementer.rs]")
        end = text.find("\n### ", start + 1) if start >= 0 else -1
        if start < 0 or end < 0:
            residue(rel, "no `yggdryl::implementer` section to list the object stores' items in")
        else:
            block = marker + "".join(f"{entry}\n" for entry in implementer_entries())
            body = text[:end + 1].rstrip("\n") + "\n"
            gap = text[len(body):end + 1]
            text = body + block + gap + text[end + 1:]
    write(root / rel, text)


# ---------------------------------------------------------------------------
# Step 4 (cont.): the core's holder bench
# ---------------------------------------------------------------------------


def edit_holder_bench(root: pathlib.Path) -> None:
    """The object stores' groups are the crate's own `s3` target: the core's
    holder bench drops the module, its stubs and the three groups."""
    edit(
        root, "rust/benchmarks/holder.rs",
        ("// The object stores against `object_store` on one in-process store. The\n"
         "// backend is a non-default feature, so the group compiles in only when it is.\n"
         '#[cfg(feature = "s3")]\n#[path = "holder/s3/mod.rs"]\nmod s3;\n', ""),
        ("/// The object-store groups when the backend is not compiled in: nothing to\n"
         "/// register.\n///\n"
         "/// Stubs rather than a second `criterion_group!` list, so the target's\n"
         "/// benchmarks are named in one place whatever the feature state.\n"
         '#[cfg(not(feature = "s3"))]\nmod s3 {\n'
         "    pub(crate) mod bytes {\n        pub(crate) fn byte_benchmarks(_: &mut criterion::Criterion) {}\n    }\n"
         "    pub(crate) mod listing {\n        pub(crate) fn listing_benchmarks(_: &mut criterion::Criterion) {}\n    }\n"
         "    pub(crate) mod records {\n        pub(crate) fn record_benchmarks(_: &mut criterion::Criterion) {}\n    }\n"
         "}\n\n", ""),
        ("    s3::bytes::byte_benchmarks,\n    s3::listing::listing_benchmarks,\n    s3::records::record_benchmarks,\n",
         ""),
    )


# ---------------------------------------------------------------------------
# Step 12: what is left for the compiler loop
# ---------------------------------------------------------------------------

OLD_PATH = re.compile(r"\byggdryl::s3\b|(?<![\w$])crate::s3::|\byggdryl::internals::s3_")
OLD_GATE = re.compile(r'feature\s*=\s*"s3"|"yggdryl/s3"')
FEATURES_FLAG = re.compile(r"""--features(?:\s+|=)(?:"([^"]*)"|'([^']*)'|([^\s"']+))""")


def names_retired_feature(line: str) -> bool:
    """A cfg, a forward or a `--features` list naming the core's `s3`."""
    if OLD_GATE.search(line):
        return True
    for m in FEATURES_FLAG.finditer(line):
        words = re.split(r"[\s,]+", next(g for g in m.groups() if g is not None))
        if "s3" in words or "yggdryl/s3" in words:
            return True
    return False


def scan_residue(root: pathlib.Path) -> dict[str, list[str]]:
    """Old spellings left anywhere but the history, and the Iceberg move's
    sites - `s3tables/` and the suites kept for it - listed for that move."""
    found: dict[str, list[str]] = {"iceberg": []}
    for f in text_files(root):
        if f.startswith((".handoff/", "config/")):
            continue
        text = read(root / f)
        # A core file naming the crate is one the core cannot compile: each
        # waits under `s3tables` for the Iceberg move.
        core = f.startswith(("rust/src/", "rust/tests/", "rust/benchmarks/")) and f.endswith(".rs")
        code = code_only(text) if core else ""
        if core and re.search(rf"\b{CRATE}::", code):
            lines = sorted({line_of(text, m.start()) for m in re.finditer(rf"\b{CRATE}::", code)})
            found["iceberg"].append(f"{f}:{','.join(map(str, lines))}")
        for k, line in enumerate(text.split("\n"), 1):
            where = f"{f}:{k}"
            if OLD_PATH.search(line):
                residue(where, "an old `s3` path: " + line.strip()[:100])
            if names_retired_feature(line):
                # The CLI's own `s3` feature gates what it links; the Iceberg
                # suites wait under `s3tables`.
                if f.startswith("cli/") or (f == "cli/Cargo.toml"):
                    continue
                residue(where, "the core's retired `s3` feature: " + line.strip()[:100])
    if (root / "rust/tests/s3.rs").exists():
        found["iceberg"].append("rust/tests/s3.rs (the remnant's harness)")
    return found


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=pathlib.Path)
    parser.add_argument("--no-git-lock", action="store_true", help="the caller holds $S/git.lock already")
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s6_s3" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    lock = not arguments.no_git_lock

    # Step 0: a tree already moved is left as it is.
    if (root / CRATE_DIR / "Cargo.toml").exists():
        print(f"[s6-s3] {CRATE_DIR}/Cargo.toml exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/s3/mod.rs").exists():
        raise SystemExit("rust/src/s3/mod.rs is missing: this is not the tree the object stores' move moves")
    if "pub fn claim(backend: &'static dyn StorageBackend" not in read(root / "rust/src/holder/backend.rs"):
        raise SystemExit("rust/src/holder/backend.rs has no storage-backend register: D36's P3 is not in this tree")
    with s4.GitLock(lock):
        dirty = git(root, "--no-optional-locks", "status", "--porcelain").strip()
    if dirty:
        raise SystemExit("the tree has changes; run the object stores' move on a clean tree")
    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)
    import tomllib

    core_manifest = tomllib.loads(read(root / "rust/Cargo.toml"))
    internals: dict[str, list[str]] = {}
    for m in re.finditer(r"pub use crate::s3::([\w:]+)::internals as (s3_\w+);", read(root / "rust/src/lib.rs")):
        internals[m.group(2)] = [m.group(1).replace("::", "_")]
    tables = S3Tables(internals)
    load_trait_methods(root)
    moves = plan_moves(root)
    paths = PathMap(moves)

    # Step 1: the tests, where they stand; the exchange harness before its
    # `#[path]` is re-anchored onto the move.
    pre_split_edits(root)
    edit_interop(root)
    splits = split_tests(root, moves)
    remnant = split_iceberg_remnant(root)
    rekeyed = rekey_iceberg_gates(root)
    done(f"Iceberg's suites and bench over the store wait under `s3tables`: {', '.join(rekeyed) or 'none'}")

    # Step 2: the moves.
    with s4.GitLock(lock):
        for old, new in moves:
            (root / new).parent.mkdir(parents=True, exist_ok=True)
            git(root, "mv", old, new)
    done(f"moves: {len(moves)} files moved by git")

    # Step 3-4.
    build_lib(root)
    edit_crate_sources(root)
    edit_core_root(root)
    count = rekey_cfg_sites(root)
    done(f"core: {count} `s3` gates of aws/, auth/, http/ and xml/ re-keyed or deleted")
    raise_items(root)
    edit_implementer(root)
    s3tables = edit_s3tables(root)
    done(f"the core's s3tables/ re-spelled onto `{CRATE}`: {', '.join(s3tables) or 'nothing'}")
    edit_facade(root)
    edit_core_manifest(root)
    edit_holder_bench(root)

    # Step 5.
    rewriter = s4.Rewriter(tables)
    rewrite_paths(root, rewriter, tables, paths, moves, splits)
    finish_crate_text(root)
    finish_tests(root)
    if remnant is not None:
        # Written after the rewrite pass, so the file paths it names stay its
        # own; its object-store paths spelled as the crate's, which the Iceberg
        # crate links under `s3tables` - the core cannot, and the gate keeps it
        # out of every core build until the Iceberg move takes it.
        text = unalias(rewriter.rs_file(remnant, s4.Context(None, False), ICEBERG_REMNANT))
        text = comment_doors(rewrite_text_paths(text, moves))
        write(root / ICEBERG_REMNANT, tidy_head(prune_imports(prune_imports(text))))
        write(root / "rust/tests/s3.rs", ICEBERG_REMNANT_HARNESS)

    # Step 6.
    siblings, linking = install_in_siblings(root)
    if siblings:
        done(f"sibling leaf tests and benches reaching the backend: {', '.join(siblings)}")

    # Step 7-11.
    install_everywhere(root)
    crate_manifest(root, core_manifest, version)
    edit_tooling(root)
    edit_ci(root, linking)
    edit_docs(root)
    edit_agents(root)
    edit_inventory(root)

    # Step 12.
    found = scan_residue(root)
    with s4.GitLock(lock):
        git(root, "add", "-A", "--", CRATE_DIR)
        if remnant is not None:
            git(root, "add", "--", "rust/tests/s3.rs", ICEBERG_REMNANT)
    report = (["# S6 object stores residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE]
              + ["", "## For the Iceberg move", ""] + ([f"- `{w}`" for w in found["iceberg"]] or ["- none"])
              + ["", "## Residue", ""] + (RESIDUE or ["- none"]))
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s6-s3] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
