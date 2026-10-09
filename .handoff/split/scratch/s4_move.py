#!/usr/bin/env python3
"""S4: move the market vocabulary into `yggdryl-market` and FIX into `yggdryl-fix`.

Usage:
    python3 -I s4_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch after P4 (D38) and P5 (D37) have
landed - and it leaves the tree uncommitted: `git status` shows the moves as
renames and every rewrite as a modification, for the lane manager's compiler
loop (`cargo check --workspace --all-targets --keep-going`).

What it does, in order (each step says what it touched; `--residue` gets
every site it could not rewrite mechanically, `file:line` each):

 1. tables      - the names each crate owns, read off the tree as it stands:
                  the core root re-exports of the moved modules, the FIX
                  module's own re-exports, the graph's re-exports less the
                  event vocabulary the core keeps (D4).
 2. moves       - `git mv` by glob, so a file P4 or P5 added or deleted under
                  a moved folder (`graph/message.rs`, `graph/anomaly.rs`,
                  `fix/anomaly.rs` gone) moves or not with it; the shared
                  capture to `rust/tests/support/ulbridge.log` (D14); the two
                  `rust/examples` files deleted (AGENTS: no `examples/`).
 3. graph split - `rust/src/graph/mod.rs` keeps the event vocabulary;
                  `rust/market/src/graph/mod.rs` is written from its market
                  half, the three `delegate_*` macros exported for FIX.
 4. crate roots - `rust/market/src/lib.rs` (the core's declarations of the
                  moved modules, `pub mod graph`, `install()`), FIX's `mod.rs`
                  as `rust/fix/src/lib.rs` (`fix_category`, `install()`), the
                  core's declarations deleted from `rust/src/lib.rs`.
 5. core seams  - the register's seed of the four kinds replaced by the
                  numbers it holds for them (`market.rs`), the FIX Latest
                  names leave the logical-name seed (`vocabulary.rs`), the
                  four kinds' Arrow extension types leave `arrow/extension.rs`
                  for an exported `market_extension!` their files invoke.
 6. paths       - every `crate::`, `$crate::`, `yggdryl::` path and `use`
                  tree in the moved sources, the moved tests and benches, the
                  bindings, the CLI, the docs and the skills re-pointed by
                  owner; `super::` paths to the event vocabulary in the
                  market's graph files; `#[path]`, `include_*!` and
                  `CARGO_MANIFEST_DIR` paths re-anchored through the move
                  table; file paths in every text file.
 7. cost rows   - `allocations.rs` and `iobase_calls.rs` split item by item:
                  a test reaching a moved name moves byte-identical into the
                  crate's own target, its helpers copied where both sides use
                  them, the imports each side no longer uses pruned.
 8. install     - each moved test calls its harness's `install::installed()`,
                  each moved bench's `main` installs, each rustdoc example of
                  the moved crates installs on a hidden line, each docs block
                  naming a moved crate installs, the bindings' init and the
                  CLI's `main` install both crates in dependency order.
 9. manifests   - `rust/market/Cargo.toml`, `rust/fix/Cargo.toml`, the
                  workspace members and `[workspace.dependencies]` pinned
                  `=<version>`, the bindings and the CLI.
10. tooling     - `generate_internals.py` and `check_api_inventory.py` per
                  crate (and the generator run), the FIX dictionary generator,
                  the ISIN seed check and the docs runner (D13) re-pointed.
11. CI          - `[leaves]` market and fix uncommented, the moved targets'
                  shards with them, the capture's path in `rows.libs`, the
                  planner test's capture path.
12. inventory   - `.api-inventory.txt` sections re-homed by file.
13. residue     - what is left for the compiler loop, by file and line.

It runs no cargo command. A second run on a tree it already moved stops at
step 0 and changes nothing.
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import pathlib
import posixpath
import re
import subprocess
import sys
import time

SCRATCH = pathlib.Path(
    os.environ.get(
        "S",
        "/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad",
    )
)

# ---------------------------------------------------------------------------
# The crate map (DESIGN.md "The crate map", D4, D25)
# ---------------------------------------------------------------------------

CORE, MARKET, FIX = "core", "market", "fix"
CRATE_NAME = {CORE: "yggdryl", MARKET: "yggdryl_market", FIX: "yggdryl_fix"}
PACKAGE = {CORE: "yggdryl", MARKET: "yggdryl-market", FIX: "yggdryl-fix"}
CRATE_DIR = {CORE: "rust", MARKET: "rust/market", FIX: "rust/fix"}

# The root files the market crate takes (D25: the seventeen codes, `code.rs`,
# `market.rs` - the register and the four root variants - stay core).
MARKET_ROOT_FILES = [
    "marketdatakind", "marketdatatype", "side", "timeinforce",
    "idkey", "identifier", "idtype", "idsource", "securityid", "eusipa",
    "limit", "isin_registry",
]
MARKET_DIRS = ["isin_registry"]
FIX_ROOT_FILES = ["fix_category"]
# `graph/` less the event vocabulary the core keeps (D4); `mod.rs` splits.
CORE_GRAPH_FILES = {"mod.rs", "element.rs", "column.rs", "element_column.rs"}
CORE_GRAPH_NAMES = {
    "Element", "Event", "EventColumn", "ElementColumn",
    "element", "column", "element_column",
}
# The four kinds the market crate claims, by their statics.
MARKET_KINDS = ["MARKETDATAKIND_KIND", "SIDE_KIND", "MARKETDATATYPE_KIND", "TIMEINFORCE_KIND"]
# Market macros FIX invokes; exported through `yggdryl_market::implementer`.
MARKET_MACROS = ["delegate_event", "delegate_market", "delegate_operation"]

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s4] {what}", flush=True)


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------


def read(path: pathlib.Path) -> str:
    # Line endings as the file has them: `.gitignore` is CRLF.
    with open(path, encoding="utf-8", newline="") as handle:
        return handle.read()


def write(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(text)


def git(root: pathlib.Path, *args: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=False
    )
    if check and result.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


def tracked(root: pathlib.Path, *pathspecs: str) -> list[str]:
    out = git(root, "ls-files", "-z", "--", *pathspecs)
    return sorted(p for p in out.split("\0") if p)


def replace_once(text: str, old: str, new: str, where: str) -> str:
    count = text.count(old)
    if count != 1:
        residue(where, f"anchor matched {count} times, not edited: {old.strip()[:90]!r}")
        return text
    return text.replace(old, new, 1)


def line_of(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


class GitLock:
    """`mkdir $S/git.lock` ... `rmdir` around every step that touches `.git`."""

    def __init__(self, enabled: bool) -> None:
        self.enabled = enabled
        self.path = SCRATCH / "git.lock"

    def __enter__(self) -> "GitLock":
        if not self.enabled:
            return self
        deadline = time.time() + 900
        while True:
            try:
                self.path.mkdir()
                return self
            except FileExistsError:
                if time.time() > deadline:
                    raise SystemExit(f"{self.path} held for 15 minutes; pass --no-git-lock if you hold it")
                time.sleep(2)

    def __exit__(self, *exc: object) -> None:
        if self.enabled:
            self.path.rmdir()


# ---------------------------------------------------------------------------
# A Rust lexer good enough to find items, strings and comments
# ---------------------------------------------------------------------------


def skip_string(text: str, i: int) -> int:
    """Index just past the string or char literal starting at `i`, else `i`."""
    n = len(text)
    j = i
    if text.startswith("br", j) or text.startswith("cr", j):
        j += 1
    if j < n and text[j] == "r" and j + 1 < n and text[j + 1] in "#\"":
        k = j + 1
        hashes = 0
        while k < n and text[k] == "#":
            hashes += 1
            k += 1
        if k < n and text[k] == '"':
            end = text.find('"' + "#" * hashes, k + 1)
            return n if end < 0 else end + 1 + hashes
        return i
    if j < n and text[j] == "b" and j + 1 < n and text[j + 1] in "\"'":
        j += 1
    if j < n and text[j] == '"':
        k = j + 1
        while k < n:
            if text[k] == "\\":
                k += 2
                continue
            if text[k] == '"':
                return k + 1
            k += 1
        return n
    if j < n and text[j] == "'":
        # A char literal, never a lifetime: `'x'`, `'\n'`, `'\u{..}'`.
        if j + 2 < n and text[j + 1] == "\\":
            end = text.find("'", j + 2)
            return i if end < 0 or end - j > 12 else end + 1
        if j + 2 < n and text[j + 2] == "'":
            return j + 3
        return i
    return i


def skip_comment(text: str, i: int) -> int:
    if text.startswith("//", i):
        end = text.find("\n", i)
        return len(text) if end < 0 else end
    if text.startswith("/*", i):
        depth, k = 0, i
        while k < len(text):
            if text.startswith("/*", k):
                depth += 1
                k += 2
            elif text.startswith("*/", k):
                depth -= 1
                k += 2
                if depth == 0:
                    return k
            else:
                k += 1
        return len(text)
    return i


def matching_brace(text: str, open_index: int) -> int:
    """The index of the brace closing the one at `open_index` (`{`, `(` or `[`)."""
    pairs = {"{": "}", "(": ")", "[": "]"}
    opener = text[open_index]
    closer = pairs[opener]
    depth = 0
    i = open_index
    n = len(text)
    while i < n:
        c = text[i]
        if c in "/" and (text.startswith("//", i) or text.startswith("/*", i)):
            i = skip_comment(text, i)
            continue
        if c in "\"'brc":
            if c in "brc" and i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_"):
                i += 1
                continue
            j = skip_string(text, i)
            if j != i:
                i = j
                continue
        if c == opener:
            depth += 1
        elif c == closer:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


# ---------------------------------------------------------------------------
# Who owns a path: the resolution every rewrite reads
# ---------------------------------------------------------------------------


class Tables:
    """The names each crate owns, read off the tree before or after the moves."""

    def __init__(self, root: pathlib.Path) -> None:
        self.root = root
        self.market_names: set[str] = set()
        self.fix_names: set[str] = set()
        self.fix_modules: set[str] = set()
        self.graph_market_modules: set[str] = set()
        self.internals: dict[str, tuple[str, str]] = {}  # core alias -> (owner, new alias)
        # Market items FIX reaches through `yggdryl_market::implementer`.
        self.private_routes: dict[tuple[str, ...], str] = {}
        # Core items of an unpublished module the core's implementer re-exports.
        self.core_routes: dict[tuple[str, ...], str] = {}
        self.read()
        self.read_core_routes()

    @staticmethod
    def brace_names(text: str) -> list[str]:
        names = []
        for part in re.split(r"[,{}\s]+", text):
            part = part.strip()
            if " as " in part:
                part = part.split(" as ")[-1]
            if part and re.fullmatch(r"[A-Za-z_]\w*", part) and part != "self":
                names.append(part)
        return names

    def glob_names(self, text: str) -> set[str]:
        names = set()
        for m in re.finditer(r"^pub (?:const fn|fn|enum|struct|trait|type|const|static) (\w+)", text, re.M):
            names.add(m.group(1))
        for m in re.finditer(r"^\s{4}pub enum (\w+):", text, re.M):
            names.add(m.group(1))
        for m in re.finditer(r"market = (\w+_KIND) \[", text):
            names.add(m.group(1))
        for m in re.finditer(r"define_field_types!\(\s*(\w+),\s*(\w+),", text):
            names.update(m.groups())
        return names

    def read(self) -> None:
        root = self.root
        core_lib = root / "rust/src/lib.rs"
        market_lib = root / "rust/market/src/lib.rs"
        fix_lib = root / "rust/fix/src/lib.rs"
        fix_mod = root / "rust/src/fix/mod.rs"
        # Market root names: the core's re-exports of the moved modules, or
        # the market crate's own once it exists.
        lib = read(market_lib) if market_lib.exists() else read(core_lib)
        base = (root / "rust/market/src") if market_lib.exists() else (root / "rust/src")
        for m in re.finditer(r"^pub use (\w+)::(\{[^;]*\}|\w+|\*);", lib, re.M):
            module, rest = m.group(1), m.group(2)
            if module not in MARKET_ROOT_FILES:
                continue
            if rest == "*":
                self.market_names |= self.glob_names(read(base / f"{module}.rs"))
            else:
                self.market_names.update(self.brace_names(rest))
        # FIX names: everything the FIX module re-exports, and its own items.
        fix_text = read(fix_lib) if fix_lib.exists() else read(fix_mod)
        for m in re.finditer(r"^pub use (\w+)::(\{[^;]*\}|\w+);", fix_text, re.M):
            self.fix_names.update(self.brace_names(m.group(2)))
        for m in re.finditer(r"^pub (?:const fn|fn|enum|struct|trait|type|const|static) (\w+)", fix_text, re.M):
            self.fix_names.add(m.group(1))
        self.fix_names.add("FixCategory")
        # The FIX Latest datatype names, which only `yggdryl_fix::install()` claims.
        table = re.search(r"LOGICAL_NAMES: &\[\(&str, DataType\)\] = &\[(.*?)\n\];", fix_text, re.S)
        self.fix_logical = set(re.findall(r'\(\s*"(\w+)"', table.group(1))) if table else set()
        for m in re.finditer(r"^(?:pub(?:\([^)]*\))? )?mod (\w+);", fix_text, re.M):
            self.fix_modules.add(m.group(1))
        self.fix_modules.add("fix_category")
        # The graph modules the market takes.
        graph = root / ("rust/market/src/graph" if market_lib.exists() else "rust/src/graph")
        for path in graph.glob("*.rs"):
            if path.name not in CORE_GRAPH_FILES:
                self.graph_market_modules.add(path.stem)
        # `yggdryl::internals::<alias>` of a moved module and what it becomes.
        core_text = read(core_lib)
        for m in re.finditer(r"pub use crate::([\w:]+)::internals as (\w+);", core_text):
            path, alias = m.group(1).split("::"), m.group(2)
            owner, new = self.resolve(path)
            if owner != CORE:
                self.internals[alias] = (owner, "_".join(new) if new else "lib")
        # The market's exported macros sit at its root.
        self.market_names |= set(MARKET_MACROS)
        # A core name must never be read as a moved one.
        self.market_names -= {"Metadata"}

    def read_core_routes(self) -> None:
        """`yggdryl::implementer`'s re-exports of items whose module the core
        does not publish: a moved file spelling one by its module path (S3's
        sweep left a few inline) reaches it through the implementer."""
        core = self.root / "rust/src"
        path = core / "implementer.rs"
        if not path.exists():
            return
        text = read(path)
        for m in re.finditer(r"(?m)^pub use crate::((?:\w+::)+)(\{[^;]*\}|\w+);", text):
            modules = [s for s in m.group(1).split("::") if s]
            names = self.brace_names(m.group(2)) if m.group(2).startswith("{") else [m.group(2)]
            # Published all the way down, it needs no route.
            declaring, folder, public = core / "lib.rs", core, True
            for seg in modules:
                d = re.search(rf"(?m)^(pub(?:\([^)]*\))?\s+)?mod {seg};", read(declaring)) if declaring.exists() else None
                if not d or (d.group(1) or "").strip() != "pub":
                    public = False
                    break
                candidate = folder / f"{seg}.rs"
                declaring = candidate if candidate.exists() else folder / seg / "mod.rs"
                folder = folder / seg
            if public:
                continue
            for name in names:
                self.core_routes[tuple(modules) + (name,)] = name

    # -- resolution ---------------------------------------------------------

    def resolve(self, segs: list[str], routed: bool = False) -> tuple[str, list[str]]:
        """The owner of `yggdryl::<segs>` as the core held it, and its path there;
        `routed`, a market item no `pub` reaches is spelled through the market's
        implementer."""
        owner, new = self.resolve_plain(segs)
        routes = (self.private_routes if routed else {}) if owner == MARKET else self.core_routes if owner == CORE else {}
        if routes:
            for cut in range(len(new), 1, -1):
                name = routes.get(tuple(new[:cut]))
                if name:
                    return owner, ["implementer", name, *new[cut:]]
        return owner, new

    def resolve_plain(self, segs: list[str]) -> tuple[str, list[str]]:
        if not segs:
            return CORE, segs
        s0 = segs[0]
        if s0 == "fix":
            return FIX, segs[1:]
        if s0 in FIX_ROOT_FILES:
            return FIX, segs
        if s0 in MARKET_ROOT_FILES:
            return MARKET, segs
        if s0 == "graph":
            if len(segs) == 1:
                return CORE, segs
            if segs[1] in CORE_GRAPH_NAMES:
                return CORE, segs
            if segs[1] in MARKET_MACROS:
                return MARKET, ["implementer", segs[1]]
            return MARKET, segs
        if s0 == "internals" and len(segs) > 1 and segs[1] in self.internals:
            owner, alias = self.internals[segs[1]]
            return owner, ["internals", alias, *segs[2:]]
        if s0 in self.fix_names:
            return FIX, segs
        if s0 in self.market_names:
            return MARKET, segs
        return CORE, segs


# ---------------------------------------------------------------------------
# `use` trees: parse, flatten, re-own, render
# ---------------------------------------------------------------------------

USE_TOKEN = re.compile(r"\s*(::|\{|\}|,|\*|\$crate|r#\w+|[A-Za-z_]\w*)")


class UseParseError(Exception):
    pass


def parse_use(tree: str) -> list[tuple[list[str], str | None, str]]:
    """Flatten a use tree into leaves: (path, alias, kind) with kind `self` or `glob`.

    `a::b::C as D` is (["a","b","C"], "D", "self"); `a::{self}` is (["a"], None,
    "self"); `a::*` is (["a"], None, "glob"). A leading `::` is kept as an
    empty first segment.
    """
    tokens: list[str] = []
    i = 0
    while i < len(tree):
        m = USE_TOKEN.match(tree, i)
        if not m:
            if tree[i:].strip() == "":
                break
            raise UseParseError(tree[i:i + 20])
        tokens.append(m.group(1))
        i = m.end()
    pos = 0
    leaves: list[tuple[list[str], str | None, str]] = []

    def peek() -> str | None:
        return tokens[pos] if pos < len(tokens) else None

    def take() -> str:
        nonlocal pos
        if pos >= len(tokens):
            raise UseParseError("end")
        pos += 1
        return tokens[pos - 1]

    def tree_at(prefix: list[str]) -> None:
        path = list(prefix)
        if peek() == "::":
            take()
            if not path:
                path.append("")
        while True:
            t = peek()
            if t == "{":
                take()
                while peek() != "}":
                    tree_at(path)
                    if peek() == ",":
                        take()
                take()
                return
            if t == "*":
                take()
                leaves.append((path, None, "glob"))
                return
            if t is None or t in ("}", ",", "::"):
                raise UseParseError(str(t))
            name = take()
            if name == "self" and path:
                alias = None
                if peek() == "as":
                    take()
                    alias = take()
                leaves.append((path, alias, "self"))
                return
            path = path + [name]
            if peek() == "::":
                take()
                continue
            alias = None
            if peek() == "as":
                take()
                alias = take()
            leaves.append((path, alias, "self"))
            return

    tree_at([])
    if pos != len(tokens):
        raise UseParseError(" ".join(tokens[pos:]))
    return leaves


def render_use(root_word: str, items: list[tuple[list[str], str | None, str]]) -> list[str]:
    """Render leaves under one root word as `use` trees; one tree, or two where
    the crate root itself is imported (`use yggdryl_fix as fix;`)."""
    out: list[str] = []
    node: dict = {"children": {}, "terms": []}
    for path, alias, kind in items:
        if not path:
            out.append(f"{root_word} as {alias}" if alias else root_word)
            continue
        cursor = node
        for seg in path:
            cursor = cursor["children"].setdefault(seg, {"children": {}, "terms": []})
        cursor["terms"].append((kind, alias))

    def render(n: dict) -> list[str]:
        parts: list[str] = []
        for kind, alias in n["terms"]:
            parts.append("*" if kind == "glob" else ("self" + (f" as {alias}" if alias else "")))
        for name, child in n["children"].items():
            sub = render(child)
            if len(sub) == 1:
                s = sub[0]
                if s == "self" or s.startswith("self as "):
                    parts.append(name + s[4:])
                else:
                    parts.append(f"{name}::{s}")
            else:
                parts.append(f"{name}::{{{', '.join(sub)}}}")
        return parts

    rendered = render(node)
    if rendered:
        if len(rendered) == 1:
            out.append(f"{root_word}::{rendered[0]}")
        else:
            out.append(f"{root_word}::{{{', '.join(rendered)}}}")
    return out


# ---------------------------------------------------------------------------
# The rewrite: every path in a text, by the context its file is in
# ---------------------------------------------------------------------------


class Context:
    """Where a text sits: which crate `crate::` names, and whether `crate::`
    and `super::` paths are the core's paths to re-own (a moved source file)."""

    def __init__(self, crate: str | None, moved_source: bool, graph_file: bool = False) -> None:
        self.crate = crate  # MARKET or FIX for a moved source, None outside
        self.moved_source = moved_source
        self.graph_file = graph_file


INLINE_PATH = re.compile(r"(?<![\w$])(::)?(\$crate|crate|yggdryl)((?:::(?:r#)?[A-Za-z_]\w*)+)")
SUPER_GRAPH = re.compile(r"(?<![\w:])((?:super::)+)(" + "|".join(sorted(CORE_GRAPH_NAMES)) + r")\b")
USE_START = re.compile(r"(?m)^([ \t]*(?:#[ \t]+)?)((?:pub(?:\([^)]*\))?[ \t]+)?)use[ \t]+")


class Rewriter:
    def __init__(self, tables: Tables) -> None:
        self.t = tables
        self.stats = collections.Counter()

    # -- one path -----------------------------------------------------------

    def owner_root(self, root_word: str, segs: list[str], ctx: Context, leading: str) -> tuple[str, list[str]] | None:
        """The root word and segments a path is spelled with after the move, or
        None where it stays as written."""
        routed = ctx.crate != MARKET
        if root_word == "yggdryl":
            owner, new = self.t.resolve(segs, routed)
            if owner == CORE and new == segs:
                return None
            word = CRATE_NAME[owner]
            if ctx.moved_source and owner == ctx.crate:
                word = "crate"
            return (leading + word, new)
        if root_word in ("crate", "$crate") and ctx.moved_source:
            owner, new = self.t.resolve(segs, routed)
            if owner == ctx.crate:
                return None if new == segs else (root_word, new)
            word = CRATE_NAME[owner]
            if root_word == "$crate":
                word = "::" + word
            return (word, new)
        return None

    def inline(self, text: str, ctx: Context, where: str) -> str:
        def sub(m: re.Match) -> str:
            leading, word, rest = m.group(1) or "", m.group(2), m.group(3)
            segs = [s for s in rest.split("::") if s]
            moved = self.owner_root(word, segs, ctx, leading)
            if moved is None:
                return m.group(0)
            new_word, new = moved
            self.stats["inline"] += 1
            if not new:
                return new_word
            return new_word + "::" + "::".join(new)

        text = INLINE_PATH.sub(sub, text)
        if ctx.graph_file:
            text = SUPER_GRAPH.sub(lambda m: "yggdryl::graph::" + m.group(2), text)
        return text

    # -- use statements -----------------------------------------------------

    def use_statement(self, lead: str, vis: str, tree: str, ctx: Context, attrs: str) -> str | None:
        """The statement(s) replacing `use <tree>;`, or None to leave it."""
        try:
            leaves = parse_use(tree)
        except UseParseError:
            return None
        groups: dict[str, list] = collections.OrderedDict()
        changed = False
        for path, alias, kind in leaves:
            leading = ""
            segs = list(path)
            if segs and segs[0] == "":
                leading = "::"
                segs = segs[1:]
            if not segs:
                return None
            word, rest = segs[0], segs[1:]
            moved = None
            if word in ("yggdryl", "crate", "$crate"):
                moved = self.owner_root(word, rest, ctx, leading)
            elif word == "super" and ctx.graph_file:
                k = 0
                while k < len(segs) and segs[k] == "super":
                    k += 1
                if k < len(segs) and segs[k] in CORE_GRAPH_NAMES:
                    moved = ("yggdryl", ["graph", *segs[k:]])
            if moved is None:
                groups.setdefault(leading + word, []).append((rest, alias, kind))
                continue
            changed = True
            new_word, new = moved
            if not new and alias is None and kind == "self" and rest:
                # A moved module imported whole is its crate, named as before.
                alias = rest[-1]
            groups.setdefault(new_word, []).append((new, alias, kind))
        if not changed:
            return None
        statements = []
        for root_word, items in groups.items():
            for rendered in render_use(root_word, items):
                statements.append(f"{lead}{vis}use {rendered};")
        self.stats["use"] += 1
        if attrs:
            return ("\n" + attrs).join(statements)
        return "\n".join(statements)

    def rust(self, text: str, ctx: Context, where: str) -> str:
        """Rewrite Rust source text: its `use` statements, then every other path."""
        out: list[str] = []
        pos = 0
        for m in USE_START.finditer(text):
            if m.start() < pos:
                continue
            start = m.end()
            # The tree runs to the `;` at brace depth zero.
            depth, i = 0, start
            while i < len(text):
                c = text[i]
                if c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                elif c == ";" and depth == 0:
                    break
                i += 1
            if i >= len(text):
                break
            tree_text = text[start:i]
            lead, vis = m.group(1), m.group(2)
            # A statement whose continuation lines carry a doctest's `# ` or a
            # comment is left for the compiler loop; a one-path statement is
            # rewritten inline below.
            body = tree_text
            if "//" in body or ("\n" in body and lead.strip().startswith("#")):
                if re.search(r"\b(yggdryl|crate)\b", body) and "{" in body:
                    residue(f"{where}:{line_of(text, m.start())}", "a `use` tree with a comment or a hidden continuation, not split")
                continue
            # Attributes on the line(s) above, repeated on every statement a
            # split adds.
            line_start = text.rfind("\n", 0, m.start()) + 1
            attrs = ""
            back = line_start
            while True:
                prev_end = back - 1
                if prev_end < 0:
                    break
                prev_start = text.rfind("\n", 0, prev_end) + 1
                prev = text[prev_start:prev_end]
                if re.fullmatch(r"[ \t]*(#[ \t]+)?#\[cfg[^\n]*\][ \t]*", prev):
                    attrs = prev + "\n" + attrs
                    back = prev_start
                else:
                    break
            replacement = self.use_statement(lead, vis, re.sub(r"\s+", " ", tree_text).strip(), ctx, attrs)
            if replacement is None:
                continue
            out.append(self.inline(text[pos:m.start()], ctx, where))
            out.append(replacement)
            pos = i + 1
        out.append(self.inline(text[pos:], ctx, where))
        return "".join(out)

    # -- whole files ----------------------------------------------------------

    DOC_LINE = re.compile(r"^([ \t]*//[/!])( ?)(.*)$")

    def rs_file(self, text: str, ctx: Context, where: str) -> str:
        """A `.rs` file: code through `rust`, doc comments by their fences."""
        lines = text.split("\n")
        out: list[str] = []
        code: list[str] = []
        i = 0

        def flush_code() -> None:
            if code:
                out.append(self.rust("\n".join(code), ctx, where))
                code.clear()

        while i < len(lines):
            m = self.DOC_LINE.match(lines[i])
            if not m:
                code.append(lines[i])
                i += 1
                continue
            flush_code()
            marker = m.group(1)
            block: list[tuple[str, str, str]] = []
            while i < len(lines):
                mm = self.DOC_LINE.match(lines[i])
                if not mm or mm.group(1) != marker:
                    break
                block.append((mm.group(1), mm.group(2), mm.group(3)))
                i += 1
            out.append("\n".join(self.doc_block(block, ctx, where)))
        flush_code()
        return "\n".join(out)

    def doc_block(self, block: list[tuple[str, str, str]], ctx: Context, where: str) -> list[str]:
        """A run of doc lines: fenced Rust through `rust` as an outside crate
        would read it, any other fence as it is, prose through the inline
        rewrite in the file's context."""
        result: list[str] = []
        k = 0
        outside = Context(None, False)
        while k < len(block):
            marker, space, body = block[k]
            fence = re.match(r"^\s*(`{3,}|~{3,})(.*)$", body)
            if not fence:
                result.append(marker + space + self.inline(body, ctx, where))
                k += 1
                continue
            ticks = fence.group(1)
            result.append(marker + space + body)
            k += 1
            chunk: list[tuple[str, str, str]] = []
            while k < len(block) and not re.match(r"^\s*" + re.escape(ticks) + r"\s*$", block[k][2]):
                chunk.append(block[k])
                k += 1
            if is_rust_fence(fence.group(2)):
                code = "\n".join(c[2] for c in chunk)
                spacer = " " if any(c[1] for c in chunk) else ""
                for line in self.rust(code, outside, where).split("\n") if chunk else []:
                    result.append(marker + (spacer if line else "") + line)
            else:
                result.extend(c[0] + c[1] + c[2] for c in chunk)
            if k < len(block):
                result.append(block[k][0] + block[k][1] + block[k][2])
                k += 1
        return result

    def markdown(self, text: str, where: str) -> str:
        outside = Context(None, False)
        out: list[str] = []
        pos = 0
        for m in re.finditer(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", text):
            out.append(self.inline(text[pos:m.start(3)], outside, where))
            body = m.group(3)
            if m.group(2).strip().startswith("rust"):
                indent = m.group(1)
                if indent:
                    stripped = "\n".join(l[len(indent):] if l.startswith(indent) else l for l in body.split("\n"))
                    new = self.rust(stripped, outside, where)
                    body = "\n".join((indent + l) if l else l for l in new.split("\n"))
                else:
                    body = self.rust(body, outside, where)
            else:
                body = self.inline(body, outside, where)
            out.append(body)
            pos = m.end(3)
        out.append(self.inline(text[pos:], outside, where))
        return "".join(out)


def is_rust_fence(info: str) -> bool:
    words = [w.strip() for w in re.split(r"[,\s]+", info) if w.strip()]
    if not words:
        return True
    non_rust = {"text", "json", "toml", "yaml", "bash", "sh", "console", "python", "js", "javascript", "xml", "csv", "sql", "txt", "plain"}
    return not any(w in non_rust for w in words)


# ---------------------------------------------------------------------------
# The move table, by glob over what the tree tracks
# ---------------------------------------------------------------------------

# The two cost targets whose rows move item by item (step 7).
SPLIT_TARGETS = ["rust/tests/allocations.rs", "rust/tests/iobase_calls.rs"]
DELETED = ["rust/examples/fix_capture.rs", "rust/examples/fix_schema.rs"]


# The test and bench targets that leave the core, by the crate that takes them.
MOVED_TARGETS = {
    ("test", "fix"): FIX, ("test", "scale_ulbridge"): FIX,
    ("bench", "fix"): FIX, ("bench", "fix_allocations"): FIX,
    ("test", "graph"): MARKET, ("test", "isin_registry"): MARKET, ("test", "market_register"): MARKET,
    ("bench", "graph"): MARKET,
}


def plan_moves(root: pathlib.Path) -> list[tuple[str, str]]:
    """Every tracked file that leaves the core, old path to new, in move order."""
    moves: list[tuple[str, str]] = []
    files = tracked(root, "rust")
    capture = "rust/tests/fix/ulbridge.log"
    if capture in files:
        moves.append((capture, "rust/tests/support/ulbridge.log"))
    market_roots = {f"rust/src/{name}.rs" for name in MARKET_ROOT_FILES}
    market_tests = {f"rust/tests/root/{name}.rs" for name in [*MARKET_ROOT_FILES, "market"]}
    for f in files:
        if f == capture:
            continue
        new = None
        if f.startswith("rust/src/fix/"):
            rel = f[len("rust/src/fix/"):]
            new = "rust/fix/src/" + ("lib.rs" if rel == "mod.rs" else rel)
        elif f == "rust/src/fix_category.rs":
            new = "rust/fix/src/fix_category.rs"
        elif f in market_roots:
            new = "rust/market/src/" + f[len("rust/src/"):]
        elif any(f.startswith(f"rust/src/{d}/") for d in MARKET_DIRS):
            new = "rust/market/src/" + f[len("rust/src/"):]
        elif f.startswith("rust/src/graph/"):
            rel = f[len("rust/src/graph/"):]
            if "/" not in rel and rel not in CORE_GRAPH_FILES:
                new = "rust/market/src/graph/" + rel
            elif "/" in rel:
                residue(f, "a file in a folder under `graph/`: no rule moves it")
        elif f == "rust/tests/fix.rs" or f.startswith("rust/tests/fix/"):
            new = "rust/fix/tests/" + f[len("rust/tests/"):]
        elif f in ("rust/tests/scale_ulbridge.rs",):
            new = "rust/fix/tests/" + f[len("rust/tests/"):]
        elif f in ("rust/tests/support/allocations.rs", "rust/tests/support/ulbridge.rs"):
            new = "rust/fix/tests/" + f[len("rust/tests/"):]
        elif f in ("rust/tests/isin_registry.rs", "rust/tests/graph.rs", "rust/tests/market_register.rs") or f.startswith(
            ("rust/tests/isin_registry/", "rust/tests/graph/")
        ):
            new = "rust/market/tests/" + f[len("rust/tests/"):]
        elif f in market_tests:
            new = "rust/market/tests/" + f[len("rust/tests/"):]
        elif f in ("rust/benchmarks/fix.rs", "rust/benchmarks/fix_allocations.rs") or f.startswith("rust/benchmarks/fix/"):
            new = "rust/fix/benchmarks/" + f[len("rust/benchmarks/"):]
        elif f == "rust/benchmarks/graph.rs" or f.startswith("rust/benchmarks/graph/"):
            new = "rust/market/benchmarks/" + f[len("rust/benchmarks/"):]
        if new:
            moves.append((f, new))
    return moves


class PathMap:
    """Old repository path to new: files by the table, folders by prefix."""

    def __init__(self, moves: list[tuple[str, str]]) -> None:
        self.files = dict(moves)
        self.dirs = [
            ("rust/src/fix/", "rust/fix/src/"),
            ("rust/tests/fix/", "rust/fix/tests/fix/"),
            ("rust/benchmarks/fix/", "rust/fix/benchmarks/fix/"),
            ("rust/benchmarks/graph/", "rust/market/benchmarks/graph/"),
            ("rust/tests/graph/", "rust/market/tests/graph/"),
            ("rust/tests/isin_registry/", "rust/market/tests/isin_registry/"),
            *((f"rust/src/{d}/", f"rust/market/src/{d}/") for d in MARKET_DIRS),
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
        """This map with `extra` files taking precedence: a crate's split copies."""
        other = PathMap([])
        other.files = {**self.files, **extra}
        other.dirs = self.dirs
        return other


def manifest_dir(path: str) -> str | None:
    for prefix in ("rust/market", "rust/fix"):
        if path.startswith(prefix + "/"):
            return prefix
    for prefix in ("rust", "cli", "python", "node"):
        if path.startswith(prefix + "/"):
            return prefix
    return None


def relative(target: str, base_dir: str) -> str:
    return posixpath.relpath(target, base_dir or ".")


def reanchor(text: str, old_file: str, new_file: str, paths: PathMap) -> str:
    """Re-point `#[path]`, `include_*!` and `CARGO_MANIFEST_DIR` paths written
    against `old_file`'s place to what they name from `new_file`'s."""
    old_dir, new_dir = posixpath.dirname(old_file), posixpath.dirname(new_file)

    def file_relative(m: re.Match) -> str:
        literal = m.group(2)
        target = paths(posixpath.join(old_dir, literal))
        new = relative(target, new_dir)
        return m.group(1) + new + m.group(3) if new != literal else m.group(0)

    text = re.sub(r'(#\[path\s*=\s*")([^"]+)("\])', file_relative, text)
    text = re.sub(r'((?<![\w!])include_(?:str|bytes)!\(\s*")([^"]+)("\s*\))', file_relative, text)

    old_manifest, new_manifest = manifest_dir(old_file), manifest_dir(new_file)
    if old_manifest is None or new_manifest is None:
        return text

    def concat_form(m: re.Match) -> str:
        literal = m.group(2)
        target = paths(posixpath.join(old_manifest, literal.lstrip("/")))
        new = "/" + relative(target, new_manifest)
        return m.group(1) + new + m.group(3) if new != literal else m.group(0)

    text = re.sub(
        r'(concat!\(\s*env!\("CARGO_MANIFEST_DIR"\)\s*,\s*")([^"]+)("\s*\))', concat_form, text
    )

    def join_form(m: re.Match) -> str:
        chain = m.group(2)
        joins = list(re.finditer(r'(\s*)\.join\("([^"]*)"\)', chain))
        literals = [j.group(2) for j in joins]
        old_rel = posixpath.join(*literals)
        target = paths(posixpath.join(old_manifest, old_rel))
        new_rel = relative(target, new_manifest)
        if posixpath.normpath(new_rel) == posixpath.normpath(old_rel):
            return m.group(0)
        if len(joins) == 1:
            j = joins[0]
            return m.group(1) + chain[: j.start(2)] + new_rel + chain[j.end(2):]
        if new_rel.split("/") == ["..", *old_rel.split("/")] or posixpath.normpath(
            posixpath.join("..", old_rel)
        ) == posixpath.normpath(new_rel):
            first = joins[0]
            return m.group(1) + chain[: first.start()] + f'{first.group(1)}.join("..")' + chain[first.start():]
        residue(new_file, f"a `CARGO_MANIFEST_DIR` join chain `{old_rel}` now names `{new_rel}`; re-spell it")
        return m.group(0)

    text = re.sub(r'(env!\("CARGO_MANIFEST_DIR"\)\s*\))((?:\s*\.join\("[^"]*"\))+)', join_form, text)
    return text


def rewrite_text_paths(text: str, paths: PathMap, moves: list[tuple[str, str]]) -> str:
    """Moved file paths wherever a text spells them: `a/b/c.rs`, and the
    quoted-segment forms Python (`"a" / "b"`) and JavaScript (`'a', 'b'`) join."""
    for old, new in sorted(moves, key=lambda pair: -len(pair[0])):
        if old in text:
            text = re.sub(r"(?<![\w./-])" + re.escape(old) + r"(?![\w-])", new, text)
    for old, new in paths.dirs:
        text = re.sub(r"(?<![\w./-])" + re.escape(old), new, text)
    # `rust/src/fix` as a folder name, without its slash.
    text = re.sub(r"(?<![\w./-])rust/src/fix(?![\w/.-])", "rust/fix/src", text)
    text = re.sub(r"(?<![\w./-])rust/tests/fix(?![\w/.-])", "rust/fix/tests/fix", text)

    def segments(m: re.Match, sep: str, quote: str) -> str:
        parts = re.findall(quote + r"([\w.-]+)" + quote, m.group(0))
        joined = "/".join(parts)
        mapped = paths(joined)
        if mapped == joined:
            return m.group(0)
        spacing = re.search(quote + r"(\s*" + re.escape(sep) + r"\s*)" + quote, m.group(0))
        glue = spacing.group(1) if spacing else sep
        return glue.join(quote + p + quote for p in mapped.split("/"))

    text = re.sub(r'"rust"(?:\s*/\s*"[\w.-]+")+', lambda m: segments(m, "/", '"'), text)
    text = re.sub(r"'rust'(?:\s*,\s*'[\w.-]+')+", lambda m: segments(m, ",", "'"), text)
    # A command naming a moved target names its crate's package.
    def command(m: re.Match) -> str:
        crate = MOVED_TARGETS.get((m.group(3), m.group(4)))
        return m.group(1) + PACKAGE[crate] + m.group(2) + m.group(4) if crate else m.group(0)

    text = re.sub(r"(-p\s+)yggdryl(\s+(?:--locked\s+)?--(test|bench)\s+)(\w+)", command, text)
    return text


# ---------------------------------------------------------------------------
# Top-level items of a Rust file: what the graph split and the cost-row
# split cut along
# ---------------------------------------------------------------------------


class Item:
    def __init__(self, start: int, end: int, text: str, name: str | None, kind: str) -> None:
        self.start, self.end, self.text, self.name, self.kind = start, end, text, name, kind
        self.names: set[str] = {name} if name else set()

    def __repr__(self) -> str:
        return f"Item({self.kind} {self.name} @{self.start})"


ITEM_HEAD = re.compile(
    r"(?:pub(?:\([^)]*\))?\s+)?(?:(?:unsafe|async|const|extern\s+\"[^\"]*\")\s+)*"
    r"(fn|mod|struct|enum|trait|impl|type|const|static|use|macro_rules!|union)\b\s*([A-Za-z_]\w*)?"
)


def code_only(text: str) -> str:
    """`text` with comments and string literals blanked, so tokens are code."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and (text.startswith("//", i) or text.startswith("/*", i)):
            j = skip_comment(text, i)
            out.append(" " * (j - i))
            i = j
            continue
        if c in "\"'brc" and not (i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_")):
            j = skip_string(text, i)
            if j != i:
                out.append(" " * (j - i))
                i = j
                continue
        out.append(c)
        i += 1
    return "".join(out)


def top_items(text: str) -> tuple[str, list[Item]]:
    """The file's leading inner doc and attributes, then its top-level items,
    each with the comments and attributes above it. The items and the gaps
    between them cover the text after the head exactly."""
    head = re.match(r"(?:[ \t]*//![^\n]*\n|[ \t]*#!\[[^\n]*\]\n|[ \t]*\n)*", text)
    pos = head.end()
    items: list[Item] = []
    n = len(text)
    while pos < n:
        # Leading trivia: blank lines, comments, attributes.
        start = pos
        i = pos
        while i < n:
            if text[i] in " \t\r\n":
                i += 1
                continue
            if text.startswith("//", i) or text.startswith("/*", i):
                i = skip_comment(text, i)
                continue
            if text.startswith("#[", i):
                close = matching_brace(text, i + 1)
                if close < 0:
                    break
                i = close + 1
                continue
            break
        if i >= n:
            break
        m = ITEM_HEAD.match(text, i)
        name, kind = None, "other"
        if m:
            kind = m.group(1)
            name = m.group(2)
            if kind == "impl":
                head_end = text.find("{", i)
                header = text[i:head_end] if head_end > 0 else ""
                target = re.search(r"\bfor\s+([A-Za-z_]\w*)", header) or re.search(
                    r"impl(?:<[^>]*>)?\s+([A-Za-z_]\w*)", header
                )
                name = target.group(1) if target else None
        else:
            mm = re.match(r"([A-Za-z_]\w*)!\s*([A-Za-z_]\w*)?", text[i:])
            if mm:
                kind = "macro:" + mm.group(1)
                if mm.group(1) == "macro_rules":
                    name = mm.group(2)
        # The item's end: a `;` at depth zero, or the block it opens.
        j = i
        end = -1
        while j < n:
            c = text[j]
            if c == "/" and (text.startswith("//", j) or text.startswith("/*", j)):
                j = skip_comment(text, j)
                continue
            if c in "\"'brc" and not (j > 0 and (text[j - 1].isalnum() or text[j - 1] == "_")):
                k = skip_string(text, j)
                if k != j:
                    j = k
                    continue
            if c in "([":
                close = matching_brace(text, j)
                if close < 0:
                    break
                j = close + 1
                continue
            if c == ";":
                end = j + 1
                break
            if c == "{":
                close = matching_brace(text, j)
                if close < 0:
                    break
                end = close + 1
                if kind in ("const", "static", "use", "type") or kind.startswith("macro:"):
                    # `const X: T = T { .. };`, `name! { .. }` may close with `;`.
                    rest = re.match(r"[ \t]*;", text[end:])
                    if kind in ("const", "static", "type") and not rest:
                        j = end
                        continue
                    if rest:
                        end += rest.end()
                break
            j += 1
        if end < 0:
            residue("top_items", f"an item at offset {i} has no end")
            break
        # Take the rest of the line (a trailing comment) with the item.
        nl = text.find("\n", end)
        if nl >= 0 and text[end:nl].strip().startswith("//"):
            end = nl
        item = Item(start, end, text[start:end], name, kind)
        if kind == "macro:thread_local":
            item.names = set(re.findall(r"static\s+([A-Za-z_]\w*)", item.text))
        items.append(item)
        pos = end
    return text[: head.end()], items


# ---------------------------------------------------------------------------
# Step 3-5: the crate roots, the graph split and the core's seams
# ---------------------------------------------------------------------------

MOVED_MODULES = set(MARKET_ROOT_FILES) | set(FIX_ROOT_FILES) | {"fix"}


def use_root_segment(item_text: str) -> str | None:
    m = re.search(r"\buse\s+(?:crate::)?([A-Za-z_]\w*)", code_only(item_text))
    return m.group(1) if m else None


def split_core_lib(root: pathlib.Path) -> str:
    """Delete the core's declarations of the moved modules; answer the market
    ones, which the market crate's root declares as the core did."""
    path = root / "rust/src/lib.rs"
    text = read(path)
    head, items = top_items(text)
    kept: list[str] = [head]
    market: list[str] = []
    pos = len(head)
    for item in items:
        kept.append(text[pos:item.start])
        pos = item.end
        target = None
        if item.kind == "mod" and item.name in MOVED_MODULES:
            target = item.name
        elif item.kind == "use":
            segment = use_root_segment(item.text)
            if segment in MOVED_MODULES:
                target = segment
        if target is None:
            kept.append(item.text)
        elif target in MARKET_ROOT_FILES:
            market.append(item.text.strip("\n"))
    kept.append(text[pos:])
    write(path, "".join(kept))
    done(f"rust/src/lib.rs: {len(items)} items read, the moved modules' declarations deleted")
    return "\n".join(market) + "\n"


def mark_graph_links(text: str) -> str:
    """Bare intra-doc links to the event vocabulary, which the market's graph
    module no longer holds, spelled to the core's."""
    names = "|".join(sorted(CORE_GRAPH_NAMES, key=len, reverse=True))
    return re.sub(
        r"\[`(" + names + r")((?:::\w+)*)`\](?!\()",
        lambda m: f"[`{m.group(1)}{m.group(2)}`](yggdryl::graph::{m.group(1)}{m.group(2)})",
        text,
    )


CORE_GRAPH_DOC = """//! The event vocabulary every element of a graph answers about itself,
//! whatever crate holds the element. [`Element`] is the node - its
//! [`Uuid`](crate::Uuid), the one it has elsewhere and the UUIDs of what it
//! was read from, read and written, the order it stands in, and how it
//! follows and merges - and [`Event`] is an element that also happened at
//! one instant and stands in one state. [`ElementColumn`] is the columns
//! every generated schema of an element opens with and [`EventColumn`] the
//! ones an event adds, one per fact the traits answer, so a text line's batch
//! and a market row open with the same columns and join on them without a
//! mapping. The market leaves, the walk over them and the columns a market
//! and an operation add are `yggdryl_market::graph`'s.
"""


def split_graph_mod(root: pathlib.Path) -> str:
    """Keep the event vocabulary in the core's `graph/mod.rs`; answer the
    market's `graph/mod.rs`, written from the rest."""
    path = root / "rust/src/graph/mod.rs"
    text = read(path)
    head, items = top_items(text)
    core_items: list[str] = []
    market_items: list[str] = []
    for item in items:
        body = item.text.strip("\n")
        if item.kind == "macro:macro_rules":
            before, after = body.split(f"macro_rules! {item.name}", 1)
            # A macro another crate invokes calls itself through `$crate`.
            after = re.sub(rf"(?<![\w!$:]){item.name}!\(", f"$crate::{item.name}!(", after)
            market_items.append(before + f"#[macro_export]\n#[doc(hidden)]\nmacro_rules! {item.name}" + after)
            continue
        if item.kind == "use":
            segment = use_root_segment(item.text)
            if segment in MARKET_MACROS:
                continue  # the market crate exports them; FIX reaches them by its implementer
            (core_items if segment in CORE_GRAPH_NAMES else market_items).append(body)
            continue
        if item.kind == "mod":
            (core_items if item.name in CORE_GRAPH_NAMES else market_items).append(body)
            continue
        residue("rust/src/graph/mod.rs", f"an item the split does not classify, kept in the core: {body[:80]!r}")
        core_items.append(body)
    write(path, CORE_GRAPH_DOC + "\n" + "\n".join(core_items) + "\n")
    market = mark_graph_links(head) + "\n".join(market_items) + "\n"
    done(f"graph/mod.rs split: {len(core_items)} items stay, {len(market_items)} go to the market crate")
    return market


MARKET_LIB_DOC = """//! Market data over the yggdryl core: the four market enum kinds a column
//! declares - [`MarketDataKind`], [`MarketDataType`], [`Side`] and
//! [`TimeInForce`] - each claimed on the core's register by [`install`]; the
//! identifiers an element states ([`IdKey`], [`Identifier`],
//! [`Identifiers`], [`IdType`], [`IdSource`]); the instrument registry
//! ([`IsinRegistry`]); a book's price level ([`Limit`]); a structured
//! product's category ([`Eusipa`]); and [`graph`], the market leaves, the
//! walk over them, the books and their Arrow rows, beside the event
//! vocabulary the core's `yggdryl::graph` keeps.

#![deny(unsafe_code)]

"""

MARKET_INSTALL = '''
/// What this crate claims its kinds as.
const CRATE: &str = "yggdryl-market";

/// The kinds this crate claims, in byte order.
const KINDS: [&yggdryl::MarketDescriptor; 4] = [
    &MARKETDATAKIND_KIND,
    &SIDE_KIND,
    &MARKETDATATYPE_KIND,
    &TIMEINFORCE_KIND,
];

/// Claims the four market kinds on the core's register - each one's byte,
/// name and Arrow extension name - once for the life of the process; a
/// later call returns at once. Every binding's init and the CLI's `main`
/// call it, and so does a Rust caller before the core reads a market name:
/// a parsed `side`, a serde tag, a value-stream byte or a `yggdryl.side`
/// extension name is refused until the kind is claimed.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed one of these
/// bytes, names or extension names first.
pub fn install() -> yggdryl::Result<()> {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    let _installing = INSTALLING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    for kind in KINDS {
        if yggdryl::market::kind_of(kind.id).is_some_and(|held| std::ptr::eq(held, kind)) {
            continue;
        }
        yggdryl::market::claim(kind, CRATE)?;
    }
    let _ = INSTALLED.set(());
    Ok(())
}
'''

MARKET_IMPLEMENTER = """//! The one door the FIX crate reaches this crate's crate-private items
//! through, as `yggdryl::implementer` is the core's: nothing here is API,
//! and an item is listed because `yggdryl-fix` needs it. The three
//! `delegate_*` macros are exported `#[doc(hidden)]` at the crate root and
//! re-exported here; their expansions name the core through `::yggdryl`.

pub use crate::{delegate_event, delegate_market, delegate_operation};
"""

FIX_INSTALL = '''
/// What this crate claims the FIX Latest datatype names as.
const CRATE: &str = "yggdryl-fix";

/// Claims what FIX registers on the core - the FIX Latest datatype names
/// among the logical names - once for the life of the process, after the
/// market crate's kinds, which a dictionary's fields are typed by; a later
/// call returns at once. Every binding's init and the CLI's `main` call it,
/// and so does a Rust caller before the core reads a FIX datatype name.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed a market kind
/// or one of these names first.
pub fn install() -> yggdryl::Result<()> {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    yggdryl_market::install()?;
    let _installing = INSTALLING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    for (name, dtype) in LOGICAL_NAMES.iter().cloned() {
        yggdryl::DataType::register_logical_name(name, dtype, CRATE)?;
    }
    let _ = INSTALLED.set(());
    Ok(())
}
'''

INTERNALS_PLACEHOLDER = "// GENERATED by scripts/generate_internals.py - do not edit by hand.\n// END GENERATED\n"


def write_market_lib(root: pathlib.Path, declarations: str) -> None:
    text = (
        MARKET_LIB_DOC
        + declarations
        + "pub mod graph;\n#[doc(hidden)]\npub mod implementer;\n"
        + MARKET_INSTALL
        + "\n"
        + INTERNALS_PLACEHOLDER
    )
    write(root / "rust/market/src/lib.rs", text)
    write(root / "rust/market/src/implementer.rs", MARKET_IMPLEMENTER)
    done("rust/market/src/lib.rs and implementer.rs written")


def convert_fix_lib(root: pathlib.Path) -> None:
    path = root / "rust/fix/src/lib.rs"
    text = read(path)
    head = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if head:
        text = text[: head.end()] + "\n#![deny(unsafe_code)]\n" + text[head.end():]
    else:
        residue("rust/fix/src/lib.rs", "no crate doc to place `#![deny(unsafe_code)]` after")
    # `pub(super)` at the crate root names nothing above it.
    text = re.sub(r"(?m)^pub\(super\) ", "pub(crate) ", text)
    mods = list(re.finditer(r"(?m)^(?:pub(?:\([^)]*\))? )?mod \w+;\n", text))
    if mods:
        last = mods[-1]
        text = text[: last.end()] + "mod fix_category;\n" + text[last.end():]
        uses = list(re.finditer(r"(?m)^pub use [^;]+;\n", text))
        anchor = uses[-1].end() if uses else last.end()
        text = text[:anchor] + "pub use fix_category::FixCategory;\n" + text[anchor:]
    else:
        residue("rust/fix/src/lib.rs", "no `mod` declaration to place `mod fix_category` after")
    if "LOGICAL_NAMES" not in text:
        residue("rust/fix/src/lib.rs", "no `LOGICAL_NAMES` table for `install()` to claim")
    text = text.rstrip("\n") + "\n" + FIX_INSTALL + "\n" + INTERNALS_PLACEHOLDER
    write(path, text)
    done("rust/fix/src/lib.rs: crate attributes, `fix_category`, `install()`")


def kind_numbers(root: pathlib.Path) -> list[tuple[int, str, int, int, int]]:
    rows = []
    for name in ("marketdatakind", "side", "marketdatatype", "timeinforce"):
        path = root / f"rust/market/src/{name}.rs"
        if not path.exists():
            path = root / f"rust/src/{name}.rs"
        text = read(path)
        m = re.search(r'kind = "(\w+)".*?market = \w+ \[(0x[0-9a-f]+), (\d+), (\d+), (\d+)\]', text, re.S)
        if not m:
            residue(str(path), "no `market = KIND [byte, value_rank, dtype_rank, shape]` to read")
            continue
        rows.append((int(m.group(2), 16), m.group(1), int(m.group(3)), int(m.group(4)), int(m.group(5))))
    return sorted(rows)


def edit_market_rs(root: pathlib.Path) -> None:
    """The register's seed of the four kinds goes; the numbers the core held
    for them become the one rule a claim of each is held to (`HELD`)."""
    path = root / "rust/src/market.rs"
    where = "rust/src/market.rs"
    text = read(path)
    rows = kind_numbers(root)
    table = "\n".join(f'    ({byte:#04x}, "{name}", {value}, {dtype}, {shape}),' for byte, name, value, dtype, shape in rows)
    held = (
        "/// The kinds the core held as its own before the market crate claimed\n"
        "/// them: each one's byte, name and the three numbers its values and its\n"
        "/// datatype order and hash by - wire contracts that never move, whichever\n"
        "/// crate claims the kind - in byte order. A claim of one states all five;\n"
        "/// every other kind takes the reserved numbers.\n"
        f"const HELD: [(u8, &str, u8, u8, isize); {len(rows)}] = [\n{table}\n];\n"
    )
    start = text.find("/// The core's own kinds, in byte order")
    seed_fn = text.find("pub(crate) fn seed()", start)
    if start < 0 or seed_fn < 0:
        residue(where, "the four kinds' seed was not found; `HELD` not written")
    else:
        close = matching_brace(text, text.find("{", seed_fn))
        text = text[:start] + held + text[close + 2:]
    text = replace_once(text, "static SEEDED: OnceLock<()> = OnceLock::new();\n", "", where)
    text = text.replace("    seed();\n", "")
    text = text.replace("claim_unseeded", "claim_kind")
    text = re.sub(r"which only the core's seeding\s*\n(\s*///)\s*makes", r"which no crate\n\1 makes", text)
    m = re.search(r"    // The numbers a kind orders and hashes by[^\n]*\n(?:    //[^\n]*\n)*    if by != CORE \{", text)
    if not m:
        residue(where, "the reserved-numbers check was not found; a held kind's claim is refused as before")
    else:
        close = matching_brace(text, m.end() - 1)
        check = '''    // The numbers a kind orders and hashes by are wire contracts: a kind the
    // core held states the ones it held, under its own byte and name, and any
    // other claim the reserved ones, so no rank or shape ever collides and
    // nothing sorts or hashes two kinds as one.
    let expected = match HELD.iter().find(|held| held.0 == byte || held.1 == kind.name) {
        Some(&(held_byte, held_name, value_rank, dtype_rank, shape)) => {
            if (held_byte, held_name) != (byte, kind.name) {
                return Err(refuse(format_smolstr!(
                    "expected `{held_name}` at {held_byte:#04x}, the pair the core holds together, got `{}` at {byte:#04x}",
                    kind.name
                )));
            }
            (value_rank, dtype_rank, shape)
        }
        None => (
            MarketDescriptor::RESERVED_ENUM_VALUE_RANK,
            MarketDescriptor::RESERVED_DTYPE_RANK,
            MarketDescriptor::RESERVED_SHAPE,
        ),
    };
    if (kind.value_rank, kind.dtype_rank, kind.shape) != expected {
        return Err(refuse(format_smolstr!(
            "expected the reserved (value_rank, dtype_rank, shape) {expected:?}, got {:?}",
            (kind.value_rank, kind.dtype_rank, kind.shape)
        )));
    }'''
        text = text[: m.start()] + check + text[close + 1:]
    m = re.search(r"    // The grammar reads the register, which is seeding[^\n]*\n(?:    //[^\n]*\n)*", text)
    if m:
        text = text[: m.start()] + "    // A name the grammar already reads would never reach the register.\n" + text[m.end():]
    text = replace_once(text, "if by != CORE && DataType::from_str(kind.name).is_ok()", "if DataType::from_str(kind.name).is_ok()", where)
    if "seed" in code_only(text):
        residue(where, "a `seed` name is left after the seed went")
    write(path, text)
    done("rust/src/market.rs: the seed of the four kinds replaced by `HELD`")


def edit_vocabulary(root: pathlib.Path) -> None:
    path = root / "rust/src/vocabulary.rs"
    where = "rust/src/vocabulary.rs"
    text = read(path)
    text = replace_once(text, "CORE_NAMES.iter().chain(crate::fix::LOGICAL_NAMES).cloned()", "CORE_NAMES.iter().cloned()", where)
    text = re.sub(
        r"/// The core's names claimed, once, before the register answers anything,\n/// and the FIX Latest names beside them:[^\n]*\n(?:///[^\n]*\n)*?(?=fn seed)",
        "/// The core's names claimed, once, before the register answers anything.\n",
        text,
    )
    text = re.sub(
        r"claimed first when the register is seeded, the FIX Latest\n/// names of `crate::fix::LOGICAL_NAMES` beside them\.",
        "claimed first when the register is seeded; `yggdryl-fix` claims the\n/// FIX Latest names beside them through its `install()`.",
        text,
    )
    text = re.sub(
        r"are the FIX module's own table, which the core's seed claims\n//! beside its names until the FIX crate claims it itself;",
        "are `yggdryl-fix`'s own table, which its `install()` claims beside\n//! the core's names;",
        text,
    )
    text = replace_once(text, "        crate::market::seed();\n", "", where)
    text = re.sub(
        r"        // The market register seeds under its own lock, so it is seeded\n        // before the lock is taken here; then, under that lock from the\n        // first read, a name claimed as a kind and as a logical name is a\n",
        "        // Under the market register's lock, a name claimed as a kind and as\n        // a logical name is a\n",
        text,
    )
    text = replace_once(text, '    /// assert!(names.contains(&("price", DataType::Float64)));\n', "", where)
    write(path, text)
    done("rust/src/vocabulary.rs: the FIX Latest names leave the seed")


def edit_extension(root: pathlib.Path) -> None:
    """The four kinds' `ExtensionType` impls leave the core: a foreign trait is
    implemented beside the type, so each kind file invokes the exported
    `market_extension!` over two helpers the core's markers share."""
    path = root / "rust/src/arrow/extension.rs"
    where = "rust/src/arrow/extension.rs"
    text = read(path)
    text = re.sub(r"    \(@one \$marker:ident => market \$leaf:ident\) => \{\n.*?\n    \};\n", "", text, count=1, flags=re.S)
    text, removed = re.subn(r"(?m)^    \w+Type => market \w+,\n", "", text)
    if removed != 4:
        residue(where, f"{removed} market marker lines removed, expected 4")
    text = replace_once(
        text,
        """            fn deserialize_metadata(metadata: Option<&str>) -> Result<Self::Metadata, ArrowError> {
                match metadata {
                    None | Some("") => Ok(()),
                    Some(document) => Err(unexpected_document(Self::NAME, document)),
                }
            }""",
        """            fn deserialize_metadata(metadata: Option<&str>) -> Result<Self::Metadata, ArrowError> {
                marker_metadata(Self::NAME, metadata)
            }""",
        where,
    )
    text = replace_once(
        text,
        """            fn supports_data_type(&self, data_type: &ArrowDataType) -> Result<(), ArrowError> {
                match recognized(Self::NAME, None, data_type)? {
                    Some(dtype) if dtype.id() == $id => Ok(()),
                    _ => Err(unsupported(Self::NAME, data_type)),
                }
            }""",
        """            fn supports_data_type(&self, data_type: &ArrowDataType) -> Result<(), ArrowError> {
                marker_supports(Self::NAME, $id, data_type)
            }""",
        where,
    )
    helpers = '''/// What a parameter-free marker's `deserialize_metadata` answers: no
/// document, or the refusal of the one stated.
pub fn marker_metadata(name: &str, metadata: Option<&str>) -> Result<(), ArrowError> {
    match metadata {
        None | Some("") => Ok(()),
        Some(document) => Err(unexpected_document(name, document)),
    }
}

/// What a parameter-free marker's `supports_data_type` answers: the storage
/// the recognizer reads as the datatype `id` names, under the name `name`.
pub fn marker_supports(name: &str, id: DataTypeId, data_type: &ArrowDataType) -> Result<(), ArrowError> {
    match recognized(name, None, data_type)? {
        Some(dtype) if dtype.id() == id => Ok(()),
        _ => Err(unsupported(name, data_type)),
    }
}

'''
    text = replace_once(text, "/// [`ExtensionType`] for parameter-free markers:", helpers + "/// [`ExtensionType`] for parameter-free markers:", where)
    exported = '''/// [`ExtensionType`] for a registered enum kind's marker, written by the
/// crate that claims the kind beside its type - a foreign trait is
/// implemented only in the type's crate - as
/// `market_extension!(SideType, Side)`: the name and the identifier are the
/// kind's own, the storage the one the recognizer reads as that kind.
#[macro_export]
#[doc(hidden)]
macro_rules! market_extension {
    ($marker:ident, $leaf:ident) => {
        impl $crate::implementer::ExtensionType for $marker {
            const NAME: &'static str = <$leaf>::EXTENSION_NAME;

            type Metadata = ();

            fn metadata(&self) -> &Self::Metadata {
                &()
            }

            fn serialize_metadata(&self) -> Option<String> {
                None
            }

            fn deserialize_metadata(
                metadata: Option<&str>,
            ) -> Result<Self::Metadata, $crate::implementer::ArrowError> {
                $crate::implementer::marker_metadata(Self::NAME, metadata)
            }

            fn supports_data_type(
                &self,
                data_type: &$crate::implementer::ArrowDataType,
            ) -> Result<(), $crate::implementer::ArrowError> {
                $crate::implementer::marker_supports(Self::NAME, <$leaf>::ID, data_type)
            }

            fn try_new(
                data_type: &$crate::implementer::ArrowDataType,
                (): Self::Metadata,
            ) -> Result<Self, $crate::implementer::ArrowError> {
                Self.supports_data_type(data_type).map(|()| Self)
            }
        }
    };
}

'''
    text = replace_once(text, "/// [`ExtensionType`] for a view whose leaf is its document", exported + "/// [`ExtensionType`] for a view whose leaf is its document", where)
    text = re.sub(
        r"A core marker reads its name off its identifier; a\n/// registered enum kind's marker - `SideType => market Side` - off the kind's\n/// own type, since a kind's name is the register's and not a core constant\.",
        "A core marker reads its name off its identifier; a\n/// registered enum kind's marker is the claiming crate's, written beside\n/// its type through `market_extension!` below.",
        text,
    )
    text = replace_once(text, "use crate::{BytesType, DataType, StringType};", "use crate::{BytesType, DataType, DataTypeId, StringType};", where)
    write(path, text)
    arrow_mod = root / "rust/src/arrow/mod.rs"
    write(arrow_mod, replace_once(read(arrow_mod), "\nmod extension;\n", "\npub(crate) mod extension;\n", "rust/src/arrow/mod.rs"))
    implementer = root / "rust/src/implementer.rs"
    write(
        implementer,
        read(implementer).rstrip("\n")
        + '''

// ------------------------------------------------------------------------
// Market: the Arrow extension type a registered kind's marker implements in
// the crate that claims it, through `market_extension!`.
// ------------------------------------------------------------------------

/// arrow-rs's extension trait, which `market_extension!` implements.
pub use arrow_schema::extension::ExtensionType;
/// The Arrow error and datatype the extension trait's methods take.
pub use arrow_schema::{ArrowError, DataType as ArrowDataType};
/// `arrow::extension::{marker_metadata, marker_supports}`: what a
/// parameter-free marker's document and storage checks answer, the core's
/// markers and a registered kind's alike.
pub use crate::arrow::extension::{marker_metadata, marker_supports};
/// `market_extension!`, for the market crate's four kind files.
pub use crate::market_extension;
''',
    )
    for name in ("marketdatakind", "side", "marketdatatype", "timeinforce"):
        kind = root / f"rust/market/src/{name}.rs"
        ktext = read(kind)
        m = re.search(r"define_field_types!\(\s*(\w+),\s*\w+,\s*market\s*=\s*\w+,\s*(\w+)\s*,?\s*\)", ktext)
        if not m:
            residue(f"rust/market/src/{name}.rs", "no `define_field_types!` to read the marker from; `market_extension!` not invoked")
            continue
        if "market_extension!" in ktext:
            continue
        ktext = ktext.rstrip("\n") + (
            "\n\n// The Arrow extension type of the kind's marker: a foreign trait, so it is\n"
            "// implemented here, beside the type, rather than in the core.\n"
            f"yggdryl::implementer::market_extension!({m.group(1)}, {m.group(2)});\n"
        )
        write(kind, ktext)
    done("arrow/extension.rs: the four kinds' extension types move to their files through `market_extension!`")


LINK = re.compile(r"\[([^\]\n]+)\]\(((?:crate|yggdryl_fix|yggdryl_market)::[\w:]+)\)|\[`(((?:crate|yggdryl_fix|yggdryl_market)::[\w:]+))`\](?!\()")


def unlink_foreign(text: str, crate: str, tables: Tables) -> tuple[str, int]:
    """A doc link whose target the crate can no longer reach - the core's to a
    moved item, the market's to FIX - spelled as the path it names, unlinked."""
    count = 0

    def sub(m: re.Match) -> str:
        nonlocal count
        target = m.group(2) or m.group(4)
        segs = target.split("::")
        if crate == CORE and segs[0] == "crate":
            owner, new = tables.resolve(segs[1:])
            if owner == CORE:
                return m.group(0)
            spelled = "::".join([CRATE_NAME[owner], *new])
        elif crate == MARKET and segs[0] == "yggdryl_fix":
            spelled = target
        else:
            return m.group(0)
        count += 1
        return f"`{spelled}`"

    return LINK.sub(sub, text), count


# ---------------------------------------------------------------------------
# Step 7: the cost rows, item by item
# ---------------------------------------------------------------------------


def use_tree_of(item_text: str) -> str | None:
    code = code_only(item_text)
    m = re.search(r"\buse\s+(.*?);", code, re.S)
    if not m:
        return None
    return item_text[m.start(1):m.end(1)]


def moved_owners_in(code: str, tables: Tables) -> set[str]:
    """The crates other than the core a stretch of test code names."""
    owners = set()
    for m in INLINE_PATH.finditer(code):
        if m.group(2) == "yggdryl":
            owner, _ = tables.resolve([s for s in m.group(3).split("::") if s])
            if owner != CORE:
                owners.add(owner)
    for m in USE_START.finditer(code):
        rest = code[m.end():]
        end = rest.find(";")
        if end < 0:
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", rest[:end]))
        except UseParseError:
            continue
        for path, _alias, _kind in leaves:
            if path and path[0] == "yggdryl":
                owner, _ = tables.resolve(path[1:])
                if owner != CORE:
                    owners.add(owner)
    return owners


def inline_body(item: Item) -> tuple[int, int] | None:
    """The span of an inline module's body inside its item text, or None."""
    if item.kind != "mod":
        return None
    code = code_only(item.text)
    m = re.search(r"\bmod\s+\w+\s*\{", code)
    if not m:
        return None
    close = matching_brace(code, m.end() - 1)
    return (m.end(), close) if close > 0 else None


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
    """Drop the top-level `use` leaves a file no longer names."""
    head, items = top_items(text)
    used: set[str] = set()
    code = ""
    for item in items:
        if item.kind != "use":
            item_code = code_only(item.text)
            code += item_code
            used |= set(re.findall(r"[A-Za-z_]\w*", item_code))
    # A trait imported only to call its methods is named nowhere else.
    used |= traits_called(code)
    out = [head]
    pos = len(head)
    for item in items:
        out.append(text[pos:item.start])
        pos = item.end
        if item.kind != "use":
            body = inline_body(item)
            # A module that imports to prove the names exist says so with
            # `#[allow(unused_imports)]`: its list is the test, never pruned.
            if body is None or "#[allow(unused_imports)]" in item.text[: body[0]]:
                out.append(item.text)
            else:
                start, end = body
                out.append(item.text[:start] + prune_imports(item.text[start:end]) + item.text[end:])
            continue
        tree = use_tree_of(item.text)
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
            # The item's own comments and attributes go with it.
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


RANK = {CORE: 0, MARKET: 1, FIX: 2}


def path_owner(segs: list[str], tables: Tables) -> str:
    """The crate a path rooted at a crate name reaches, before or after a rewrite."""
    if not segs:
        return CORE
    if segs[0] == "yggdryl":
        return tables.resolve(segs[1:])[0]
    if segs[0] == "yggdryl_market":
        return MARKET
    if segs[0] == "yggdryl_fix":
        return FIX
    return CORE


STRING_LITERAL = re.compile(r'r#*"(?:.|\n)*?"#*|"(?:[^"\\]|\\.)*"', re.S)
# FIX Latest names too common as field names to read a register use into.
GENERIC_FIX_NAMES = {"price", "qty", "length", "data", "pattern", "language", "amt", "percentage", "seqnum"}
# A field's type in a schema expression: after `name:`, or a type
# constructor's `<` (`struct<`, `map<`), never an XML tag or a regex group.
GRAMMAR_TYPE = re.compile(r"(?:(?<=\w)<|:)\s*([A-Za-z][A-Za-z0-9_ -]*?)\s*(?=[,>)]|$|\s*not\b|\s*null\b)")
PARSE_DOOR = re.compile(
    r'(?:DataType::from_str|from_logical_name|kind_named)\(\s*"([^"]+)"\s*\)|"([^"]+)"\.parse::<DataType>\(\)'
)
# The same doors over a name held in a variable: a table of names parsed in a
# loop (`for (name, rank) in [("side", 53), ..] { DataType::from_str(name) }`).
INDIRECT_DOOR = re.compile(r'(?:DataType::from_str|from_logical_name|kind_named)\(\s*[A-Za-z_&*]')


def fold(word: str) -> str:
    return re.sub(r"[\s_-]", "", word).lower()


def register_reads(text: str, tables: Tables) -> set[str]:
    """The crates whose claims a test needs because it reads a name through
    the core's register: a market kind or a FIX Latest datatype name, parsed
    alone or written as a field's type in a schema expression."""
    kinds = {"side", "timeinforce", "marketdatakind", "marketdatatype"}
    found: set[str] = set()

    def classify(word: str) -> None:
        folded = fold(word)
        if folded in kinds:
            found.add(MARKET)
        elif folded in tables.fix_logical:
            found.add(FIX)

    for m in PARSE_DOOR.finditer(text):
        classify(m.group(1) or m.group(2))
    indirect = INDIRECT_DOOR.search(text) is not None
    for literal in STRING_LITERAL.finditer(text):
        body = re.sub(r'^b?r?#*"|"#*$', "", literal.group(0))
        # A distinctive FIX name alone in a string is a name read by the register.
        bare = fold(body)
        if bare in tables.fix_logical and bare not in GENERIC_FIX_NAMES:
            found.add(FIX)
        # A kind's name as a value a parse door in the same item reads.
        if indirect and body in kinds:
            found.add(MARKET)
        if ":" not in body and "<" not in body:
            continue
        for m in GRAMMAR_TYPE.finditer(body):
            classify(m.group(1))
    return found


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


def item_owners(items: list[Item], tables: Tables) -> tuple[dict[int, str], dict[int, set[int]]]:
    """Each item's crate - the highest any name it reaches is owned by, through
    the file's imports, its own paths and `use` trees, or a helper of the
    file that reaches one - and the helpers each item names."""
    macro_names(items)
    imports: dict[str, str] = {}
    for item in items:
        if item.kind != "use":
            continue
        try:
            leaves = parse_use(re.sub(r"\s+", " ", use_tree_of(item.text) or ""))
        except UseParseError:
            continue
        for leaf, alias, kind in leaves:
            if kind == "self" and len(leaf) > 1:
                owner = path_owner(leaf, tables)
                if owner != CORE:
                    imports[alias or leaf[-1]] = owner
    body = [item for item in items if item.kind != "use"]
    code = {id(item): code_only(item.text) for item in body}
    tokens = {id(item): set(re.findall(r"[A-Za-z_]\w*", code[id(item)])) for item in body}
    by_name: dict[str, list[Item]] = collections.defaultdict(list)
    for item in body:
        for name in item.names:
            by_name[name].append(item)
    refs = {
        id(item): {id(other) for name in tokens[id(item)] for other in by_name.get(name, []) if other is not item}
        for item in body
    }
    owner: dict[int, str] = {}
    for item in body:
        found = {imports[name] for name in tokens[id(item)] & imports.keys()}
        for m in INLINE_PATH.finditer(code[id(item)]):
            if m.group(2) == "yggdryl":
                found.add(path_owner(["yggdryl", *[s for s in m.group(3).split("::") if s]], tables))
        for m in re.finditer(r"\b(yggdryl_market|yggdryl_fix)::", code[id(item)]):
            found.add(MARKET if m.group(1) == "yggdryl_market" else FIX)
        for m in USE_START.finditer(code[id(item)]):
            rest = code[id(item)][m.end():]
            end = rest.find(";")
            try:
                leaves = parse_use(re.sub(r"\s+", " ", rest[:end]))
            except UseParseError:
                continue
            found |= {path_owner(leaf, tables) for leaf, _a, _k in leaves}
        # A test reading a market kind or a FIX name through the register
        # needs it claimed.
        if KIND_WORDS.search(item.text):
            found.add(MARKET)
        found |= register_reads(item.text, tables)
        owner[id(item)] = max(found | {CORE}, key=RANK.__getitem__)
    changed = True
    while changed:
        changed = False
        for item in body:
            for other in refs[id(item)]:
                if RANK[owner[other]] > RANK[owner[id(item)]]:
                    owner[id(item)] = owner[other]
                    changed = True
    return owner, refs


def crate_of_path(path: str) -> str:
    if path.startswith("rust/market/"):
        return MARKET
    if path.startswith("rust/fix/"):
        return FIX
    return CORE


def split_level(text: str, home: str, tables: Tables, top: bool, single: bool) -> tuple[str, dict[str, str], int]:
    """Split one level of a test file - the file's items, or an inline
    module's - into what stays in `home` and, for each crate above it, what
    moves there: an item reaching that crate moves whole, an inline module
    holding some is split inside, a helper the moved items name is copied."""
    head, items = top_items(text)
    owner, refs = item_owners(items, tables)
    moved = 0
    dest: dict[str, dict[int, str]] = collections.defaultdict(dict)
    dropped: set[int] = set()
    replaced: dict[int, str] = {}
    for item in items:
        if item.kind == "use" or RANK[owner[id(item)]] <= RANK[home]:
            continue
        body = inline_body(item)
        if body is not None:
            start, end = body
            inner_home, inner_dest, n = split_level(item.text[start:end], home, tables, False, False)
            moved += n
            if not any(i.kind != "use" for i in top_items(inner_home)[1]):
                dropped.add(id(item))
            else:
                replaced[id(item)] = item.text[:start] + inner_home + item.text[end:]
            for crate, part in inner_dest.items():
                dest[crate][id(item)] = item.text[:start] + part + item.text[end:]
            continue
        dropped.add(id(item))
        dest[owner[id(item)]][id(item)] = item.text
        moved += 1
    if not dest:
        return text, {}, 0
    body_items = [item for item in items if item.kind != "use"]
    by_id = {id(item): item for item in body_items}
    always = {id(item) for item in body_items if top and "#[global_allocator]" in item.text}

    # An inline module is a container of tests, never a helper a name
    # reaches: a test naming `required` does not want `mod required { .. }`.
    containers = {id(item) for item in body_items if inline_body(item) is not None}

    def closure(seed: set[int]) -> set[int]:
        found, stack = set(seed), list(seed)
        while stack:
            for other in refs[stack.pop()]:
                if other not in found and other not in containers and RANK[owner[other]] <= RANK[home]:
                    found.add(other)
                    stack.append(other)
        return found

    def helpers_named(texts: list[str]) -> set[int]:
        names = {n for text_ in texts for n in re.findall(r"[A-Za-z_]\w*", code_only(text_))}
        seed = {
            id(item) for item in body_items
            if item.names & names and id(item) not in dropped and id(item) not in replaced and id(item) not in containers
        }
        return closure(seed | always)

    outs: dict[str, str] = {}
    for crate, parts in dest.items():
        helpers = helpers_named(list(parts.values())) - set(parts)
        out = [head]
        for item in items:
            if item.kind == "use":
                out.append(item.text)
            elif id(item) in parts:
                out.append(parts[id(item)])
            elif id(item) in helpers:
                out.append(item.text)
        outs[crate] = "".join(out).rstrip("\n") + "\n"
    if single and top:
        roots = {
            id(item) for item in body_items
            if id(item) not in dropped
            and (re.search(r"#\[test\]|#\[global_allocator\]", item.text) or inline_body(item) is not None)
        }
        stay = closure(roots) | set(replaced)
    else:
        # A module's helper may serve a sibling module; keep every one.
        stay = {id(item) for item in body_items if id(item) not in dropped}
    out = [head]
    for item in items:
        if item.kind == "use":
            out.append(item.text)
        elif id(item) in replaced:
            out.append(replaced[id(item)])
        elif id(item) in stay:
            out.append(item.text)
    return "".join(out).rstrip("\n") + "\n", dict(outs), moved


def split_tests(root: pathlib.Path, tables: Tables, moves: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Before the moves: every test file's items that reach a crate above the
    one the file lives in after the move - a core test naming `Side`, a
    market test naming `FixCodec` - move byte-identical into that crate's
    `tests/` at the same relative path, an inline module holding some split
    inside it, a helper both halves name copied, each half's imports pruned;
    the harness a moved module needs is written beside it from its own, with
    the helpers the module names (D39: a test lives in the lowest crate that
    can name everything it uses). Answers each file written, the old path it
    was cut from first, for the later passes."""
    move_map = dict(moves)
    written: list[tuple[str, str]] = []
    needs: dict[tuple[str, str], list[str]] = collections.defaultdict(list)
    for f in tracked(root, "rust/tests"):
        if not f.endswith(".rs") or f.startswith("rust/tests/support/"):
            continue
        home = crate_of_path(move_map.get(f, f))
        rel = f[len("rust/tests/"):]
        single = "/" not in rel
        home_text, dests, moved = split_level(read(root / f), home, tables, True, single)
        if not dests:
            continue
        what = "rows" if single else "tests"
        for crate in sorted(dests, key=RANK.__getitem__):
            dest_rel = f"{CRATE_DIR[crate]}/tests/{rel}"
            if (root / dest_rel).exists() or dest_rel in move_map.values():
                residue(dest_rel, f"a file of this name exists or arrives by a move; the split {what} of `{f}` were not written")
                continue
            header = (
                f"//! The {what} of `{f}` that reach what\n"
                f"//! `{PACKAGE[crate]}` owns, moved byte-identical (S4, D39): a test lives\n"
                "//! in the lowest crate that can name everything it uses.\n//!\n"
            )
            write(root / dest_rel, prune_imports(header + dests[crate]))
            written.append((f, dest_rel))
            if not single:
                needs[(crate, "rust/tests/" + rel.split("/", 1)[0] + ".rs")].append(f)
        write(root / f, prune_imports(home_text))
        done(f"{f}: {moved} items moved up to {', '.join(PACKAGE[c] for c in sorted(dests, key=RANK.__getitem__))}")
    for (crate, harness), modules in sorted(needs.items()):
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
                residue(harness, f"no `#[path]` declaration of `{relpath}` to copy into the {PACKAGE[crate]} harness")
                continue
            decls.append(id(decl))
            wanted |= set(re.findall(r"[A-Za-z_]\w*", code_only(read(root / CRATE_DIR[crate] / "tests" / relpath))))
        # A harness's helpers: its functions and constants, and the shared
        # fixtures it declares - never another test module.
        helpers = {
            id(item) for item in items
            if item.kind not in ("use", "mod") or (item.kind == "mod" and '"support/' in item.text)
        }
        chosen = {id(item) for item in items if id(item) in helpers and (item.names & wanted)}
        names = {n for item in items if id(item) in chosen for n in re.findall(r"[A-Za-z_]\w*", code_only(item.text))}
        chosen |= {id(item) for item in items if id(item) in helpers and item.names & names}
        dest = f"{CRATE_DIR[crate]}/tests/{posixpath.basename(harness)}"
        parts = [
            f"//! The `{PACKAGE[crate]}` half of `{harness}`: the modules whose tests\n"
            "//! reach what it owns, split off by S4 (D39), with the helpers they name.\n//!\n" + head
        ]
        parts += [item.text for item in items if item.kind == "use" or id(item) in chosen or id(item) in decls]
        if (root / dest).exists() or dest in move_map.values():
            residue(dest, f"the harness exists or arrives by a move: declare {', '.join(modules)} in it by hand")
            continue
        write(root / dest, prune_imports("".join(parts).rstrip("\n") + "\n"))
        written.append((harness, dest))
    return written


# ---------------------------------------------------------------------------
# Step 8: install, wherever a moved test, bench or example reads a name
# ---------------------------------------------------------------------------

INSTALL_SUPPORT = {
    MARKET: """//! The claim every market harness makes before a test reads a name: the
//! core's register answers a market kind only once `yggdryl-market` has
//! claimed it (D7), so each test opens with [`installed`].

/// Claims the market crate's kinds, once for the process.
pub fn installed() {
    yggdryl_market::install().expect("yggdryl-market claims its kinds");
}
""",
    FIX: """//! The claim every FIX harness makes before a test reads a name: the core's
//! register answers a market kind and a FIX Latest name only once
//! `yggdryl-fix` - and the market crate under it - has claimed them (D7), so
//! each test opens with [`installed`].

/// Claims the FIX crate's names and the market crate's kinds, once for the
/// process.
pub fn installed() {
    yggdryl_fix::install().expect("yggdryl-fix claims its names");
}
""",
}


def insert_test_installs(text: str) -> tuple[str, int]:
    code = code_only(text)
    inserts: list[tuple[int, str]] = []
    for m in re.finditer(r"#\[test\]", code):
        fn = re.compile(r"\bfn\s+\w+").search(code, m.end())
        if not fn:
            continue
        paren = code.find("(", fn.end())
        close = matching_brace(code, paren) if paren >= 0 else -1
        brace = code.find("{", close) if close >= 0 else -1
        if brace < 0:
            continue
        if re.match(r"\s*crate::install::installed\(\);", text[brace + 1:]):
            continue
        line_start = code.rfind("\n", 0, fn.start()) + 1
        indent = re.match(r"[ \t]*", text[line_start:]).group(0) + "    "
        inserts.append((brace + 1, f"\n{indent}crate::install::installed();"))
    for at, line in reversed(inserts):
        text = text[:at] + line + text[at:]
    return text, len(inserts)


def declare_install(text: str) -> str:
    if re.search(r"(?m)^mod install;", text):
        return text
    head = re.match(r"(?:[ \t]*//![^\n]*\n|[ \t]*#!\[[^\n]*\]\n|[ \t]*\n)*", text)
    inner = list(re.finditer(r"(?m)^[ \t]*#!\[[^\n]*\]\n", text))
    # After the inner attributes, wherever comments put them.
    at = max([head.end()] + [m.end() for m in inner])
    return text[:at] + '\n#[path = "support/install.rs"]\nmod install;\n' + text[at:]


def bench_main(text: str, crate: str) -> str:
    m = re.search(r"criterion_main!\(\s*([^)]*?)\s*\);", text)
    if not m:
        residue("bench", "no `criterion_main!` to install before")
        return text
    groups = [g.strip() for g in m.group(1).split(",") if g.strip()]
    body = (
        "fn main() {\n"
        "    // The kinds and names the benchmarks read are claimed before any runs.\n"
        f'    {CRATE_NAME[crate]}::install().expect("{PACKAGE[crate]} installs");\n'
        + "".join(f"    {group}();\n" for group in groups)
        + "    criterion::Criterion::default()\n        .configure_from_args()\n        .final_summary();\n}"
    )
    return prune_imports(text[: m.start()] + body + text[m.end():])


def insert_doc_installs(text: str, crate: str) -> tuple[str, int]:
    """A hidden `install()` line opening every Rust example of a moved source."""
    call = f"{CRATE_NAME[crate]}::install().unwrap();"
    lines = text.split("\n")
    out: list[str] = []
    count = 0
    i = 0
    doc = re.compile(r"^([ \t]*//[/!])( ?)(.*)$")
    while i < len(lines):
        m = doc.match(lines[i])
        fence = re.match(r"^\s*(`{3,}|~{3,})(.*)$", m.group(3)) if m else None
        if not (m and fence):
            out.append(lines[i])
            i += 1
            continue
        out.append(lines[i])
        marker, space = m.group(1), m.group(2) or " "
        ticks, rust = fence.group(1), is_rust_fence(fence.group(2))
        i += 1
        block_start = len(out)
        body: list[str] = []
        while i < len(lines):
            mm = doc.match(lines[i])
            if not mm or re.match(r"^\s*" + re.escape(ticks) + r"\s*$", mm.group(3)):
                break
            body.append(mm.group(3))
            out.append(lines[i])
            i += 1
        block_end = len(out)
        if i < len(lines):
            # The closing fence is the block's, never the next one's opening.
            out.append(lines[i])
            i += 1
        if not rust or block_end == block_start or any(call in b for b in body):
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


def insert_markdown_installs(text: str) -> tuple[str, int]:
    count = 0

    def block(m: re.Match) -> str:
        nonlocal count
        indent, info, body = m.group(1), m.group(2), m.group(3)
        if not info.strip().startswith("rust") or "install()" in body:
            return m.group(0)
        if "yggdryl_fix" in body:
            crate = FIX
        elif "yggdryl_market" in body:
            crate = MARKET
        else:
            return m.group(0)
        lines = body.split("\n")
        main = next((k for k, line in enumerate(lines) if re.match(r"^\s*fn main\(", line)), None)
        if main is not None:
            k = main
            while k < len(lines) and not lines[k].rstrip().endswith("{"):
                k += 1
            if k >= len(lines):
                return m.group(0)
            ends = "?;" if "->" in "".join(lines[main:k + 1]) else f'.expect("{PACKAGE[crate]} installs");'
            inner = re.match(r"[ \t]*", lines[k]).group(0)
            lines.insert(k + 1, f"{inner}    {CRATE_NAME[crate]}::install(){ends}")
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
            line_indent = indent
            lines.insert(last_use + 1, f"{line_indent}{CRATE_NAME[crate]}::install()?;")
        count += 1
        return f"{indent}```{info}\n" + "\n".join(lines) + f"{indent}```"

    text = re.sub(r"(?ms)^([ \t]*)```([^\n]*)\n(.*?)^\1```", block, text)
    return text, count


# ---------------------------------------------------------------------------
# Step 9: manifests
# ---------------------------------------------------------------------------

LIB_IDENTS = {"md5": "md-5", "iceberg_official": "iceberg-official"}


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


def crates_named(root: pathlib.Path, folders: list[str]) -> set[str]:
    found: set[str] = set()
    for folder in folders:
        for path in (root / folder).rglob("*.rs"):
            code = code_only(read(path))
            found |= set(re.findall(r"(?<![\w:$])([a-z_][a-z0-9_]*)::", code))
            found |= set(re.findall(r"\buse\s+([a-z_][a-z0-9_]*)\b", code))
    return found


def write_manifests(root: pathlib.Path, version: str) -> None:
    import tomllib

    core = tomllib.loads(read(root / "rust/Cargo.toml"))
    deps = core.get("dependencies", {})
    dev = core.get("dev-dependencies", {})

    def ident(package: str) -> str:
        for lib, pkg in LIB_IDENTS.items():
            if pkg == package:
                return lib
        return package.replace("-", "_")

    by_ident = {ident(name): (name, spec) for name, spec in deps.items()}
    dev_by_ident = {ident(name): (name, spec) for name, spec in dev.items()}
    core_text = read(root / "rust/Cargo.toml")

    def bench_blocks(names: list[str]) -> str:
        blocks = []
        for name in names:
            m = re.search(r'((?:#[^\n]*\n)*)\[\[bench\]\]\nname = "' + re.escape(name) + r'"\n(?:[a-z]+ = [^\n]*\n)*', core_text)
            if m:
                block = m.group(0).replace(f'path = "benchmarks/{name}.rs"', f'path = "benchmarks/{name}.rs"')
                blocks.append(block.rstrip("\n"))
            else:
                blocks.append(f'[[bench]]\nname = "{name}"\npath = "benchmarks/{name}.rs"\nharness = false')
        return "\n\n".join(blocks)

    specs = {
        MARKET: dict(
            description="Market data for yggdryl: the four market enum kinds, the identifiers, the ISIN registry and the market-data graph, its books and their Arrow rows",
            keywords=["arrow", "market-data", "isin", "orderbook", "finance"],
            categories=["data-structures", "finance"],
            features=[
                ("default", []),
                ("http", ["yggdryl/http"]),
                ("parquet", ["yggdryl/parquet"]),
                ("iceberg", ["parquet", "yggdryl/iceberg"]),
                ("internals", ["yggdryl/internals"]),
            ],
            own=["yggdryl.workspace = true"],
            skip={"yggdryl", "crate", "self", "super", "std", "core", "alloc"},
        ),
        FIX: dict(
            description="FIX for yggdryl: the FIX dictionary registry and its store, the codec, its messages, their lifecycle and their Arrow rows over the market crate's graph",
            keywords=["fix", "fix-protocol", "market-data", "arrow", "trading"],
            categories=["parser-implementations", "finance"],
            features=[
                ("default", []),
                ("http", ["yggdryl/http", "yggdryl-market/http"]),
                ("parquet", ["yggdryl/parquet", "yggdryl-market/parquet"]),
                ("iceberg", ["parquet", "yggdryl/iceberg", "yggdryl-market/iceberg"]),
                ("internals", ["yggdryl/internals", "yggdryl-market/internals"]),
            ],
            own=["yggdryl.workspace = true", "yggdryl-market.workspace = true"],
            skip={"yggdryl", "yggdryl_market", "crate", "self", "super", "std", "core", "alloc"},
        ),
    }
    benches = {
        MARKET: sorted(p.stem for p in (root / "rust/market/benchmarks").glob("*.rs")),
        FIX: sorted(p.stem for p in (root / "rust/fix/benchmarks").glob("*.rs")),
    }
    for crate, spec in specs.items():
        base = CRATE_DIR[crate]
        used = crates_named(root, [f"{base}/src"])
        used_dev = crates_named(root, [f"{base}/tests", f"{base}/benchmarks"]) - used
        lines = list(spec["own"])
        for name in sorted(used - spec["skip"]):
            if name in by_ident:
                lines.append(dependency_line(*by_ident[name]))
        dev_lines = []
        for name in sorted(used_dev - spec["skip"]):
            if name in by_ident and name not in used:
                dev_lines.append(dependency_line(*by_ident[name]))
            elif name in dev_by_ident:
                dev_lines.append(dependency_line(*dev_by_ident[name]))
        if any(b for b in benches[crate]) and not any(l.startswith("criterion") for l in dev_lines):
            dev_lines.append(dependency_line("criterion", dev.get("criterion", "0.7")))
        features = "\n".join(f"{name} = {toml_value(value)}" for name, value in spec["features"])
        manifest = (
            "[package]\n"
            f'name = "{PACKAGE[crate]}"\n'
            f"description = {json.dumps(spec['description'])}\n"
            "version.workspace = true\n"
            "edition.workspace = true\n"
            "rust-version.workspace = true\n"
            "license.workspace = true\n"
            "repository.workspace = true\n"
            'readme = "README.md"\n'
            f"keywords = {toml_value(spec['keywords'])}\n"
            f"categories = {toml_value(spec['categories'])}\n"
            "\n[features]\n" + features + "\n"
            "\n[dependencies]\n" + "\n".join(lines) + "\n"
            "\n[dev-dependencies]\n" + "\n".join(dev_lines) + "\n"
            + ("\n" + bench_blocks(benches[crate]) + "\n" if benches[crate] else "")
        )
        write(root / base / "Cargo.toml", manifest)
        readme = root / base / "README.md"
        if not readme.exists():
            write(
                readme,
                f"# {PACKAGE[crate]}\n\n{spec['description']}.\n\n"
                "Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is "
                "the project's site, and `install()` claims what this crate registers on the core.\n",
            )
    # The core: the moved benches leave, and the crate folders stay out of its package.
    text = core_text
    for name in benches[MARKET] + benches[FIX]:
        text = re.sub(r'\n(?:#[^\n]*\n)*\[\[bench\]\]\nname = "' + re.escape(name) + r'"\n(?:[a-z]+ = [^\n]*\n)*', "\n", text)
    text = re.sub(r"\n{3,}", "\n\n", text)
    if "exclude = " not in text.split("[features]")[0]:
        text = text.replace(
            'categories = ["data-structures", "encoding"]\n',
            'categories = ["data-structures", "encoding"]\n# The split crates sit inside this folder; each is its own package.\nexclude = ["/market", "/fix"]\n',
            1,
        )
    write(root / "rust/Cargo.toml", text)
    # The workspace: members and the one pinned version.
    ws = read(root / "Cargo.toml")
    ws = replace_once(ws, 'members = ["rust", "python", "node", "cli"]', 'members = ["rust", "rust/market", "rust/fix", "python", "node", "cli"]', "Cargo.toml")
    if "yggdryl-market = {" not in ws:
        ws = replace_once(
            ws,
            "[workspace.dependencies]\n",
            "[workspace.dependencies]\n"
            "# The workspace's own crates, pinned to the one version every artifact\n"
            "# carries, so a published crate names exactly the core it was built with.\n"
            f'yggdryl = {{ path = "rust", version = "={version}" }}\n'
            f'yggdryl-market = {{ path = "rust/market", version = "={version}" }}\n'
            f'yggdryl-fix = {{ path = "rust/fix", version = "={version}" }}\n',
            "Cargo.toml",
        )
    write(root / "Cargo.toml", ws)
    # The bindings and the CLI link both crates.
    for member, features in (("python", '["http"]'), ("node", '["http"]'), ("cli", '["http"]')):
        path = root / member / "Cargo.toml"
        text = read(path)
        if "yggdryl-market" in text:
            continue
        m = re.search(r"(?m)^yggdryl = (?:\{[^}]*\}|[^\n]*)\n", text)
        if not m:
            residue(f"{member}/Cargo.toml", "no `yggdryl` dependency line to place the split crates after")
            continue
        add = (
            f"yggdryl-market = {{ workspace = true, features = {features} }}\n"
            f"yggdryl-fix = {{ workspace = true, features = {features} }}\n"
        )
        text = text[: m.end()] + add + text[m.end():]
        if member == "cli":
            # The docs runner compiles every page's Rust blocks here (D13).
            dev = (
                "\n[dev-dependencies]\n"
                "# What the pages' Rust blocks name beside the crates, compiled by\n"
                "# `scripts/check_docs_examples.py` into `tests/docs_examples.rs`.\n"
                "arrow-array.workspace = true\n"
                "arrow-schema.workspace = true\n"
                "log.workspace = true\n"
                'serde_json = "1.0"\n'
                'smol_str = "=0.3.2"\n'
            )
            if "[dev-dependencies]" not in text:
                text = text.replace("\n[features]\n", dev + "\n[features]\n", 1)
            else:
                residue("cli/Cargo.toml", "has `[dev-dependencies]`; add the docs runner's crates by hand")
            text = text.replace('iceberg = ["yggdryl/iceberg"]', 'iceberg = ["yggdryl/iceberg", "yggdryl-market/iceberg", "yggdryl-fix/iceberg"]', 1)
        write(path, text)
    done("manifests: rust/market, rust/fix, the workspace, the bindings and the CLI")


# ---------------------------------------------------------------------------
# Step 10-12: tooling, CI, inventory
# ---------------------------------------------------------------------------


def edit_tooling(root: pathlib.Path) -> None:
    # generate_internals.py: one block per crate root.
    path = root / "scripts/generate_internals.py"
    text = read(path)
    if "def crates()" not in text:
        start = text.find("def block() -> str:")
        end = text.find('if __name__ == "__main__":')
        if start < 0 or end < 0:
            residue("scripts/generate_internals.py", "no `block`/`main` to make per crate")
        else:
            tail = '''def crates() -> list[pathlib.Path]:
    """Every crate's source tree: the core's, then each leaf's under `rust/`."""
    return [ROOT / "rust" / "src", *sorted(lib.parent for lib in (ROOT / "rust").glob("*/src/lib.rs"))]


def crate_name(src: pathlib.Path) -> str:
    return "yggdryl" if src.parent.name == "rust" else f"yggdryl_{src.parent.name}"


def block() -> str:
    owners = []
    for path in sorted(SRC.rglob("*.rs")):
        if path == LIB:
            continue
        if DECLARATION.search(path.read_text(encoding="utf-8")):
            owners.append(path)

    body = []
    for path in owners:
        crate_path = module_path(path)
        alias = crate_path.replace("::", "_")
        for cfg in guards(path):
            body.append(f"    {cfg}")
        body.append(f"    pub use crate::{crate_path}::internals as {alias};")

    tests = SRC.parent.relative_to(ROOT).as_posix() + "/tests/"
    return (
        START
        + "/// What the test suite pins and a caller cannot reach.\\n"
        + "///\\n"
        + f"/// Every test lives in `{tests}` and reaches the crate through\\n"
        + f"/// `{crate_name(SRC)}::`. The handful that pin something no caller can name reach it\\n"
        + "/// here instead, under a feature no published build turns on. This is not\\n"
        + "/// API: it carries no stability promise, and an item is `pub` only because\\n"
        + "/// its own module is unreachable without the feature.\\n"
        + '#[cfg(feature = "internals")]\\n'
        + "#[doc(hidden)]\\n"
        + "pub mod internals {\\n"
        + ("\\n".join(body) + "\\n" if body else "")
        + "}\\n"
        + END
    )


def main() -> int:
    global SRC, LIB
    stale = []
    for src in crates():
        SRC, LIB = src, src / "lib.rs"
        text = LIB.read_text(encoding="utf-8")
        generated = block()
        if START in text:
            start = text.index(START)
            end = text.index(END, start) + len(END)
            current = text[start:end]
            updated = text[:start] + generated + text[end:]
        else:
            current = ""
            updated = text.rstrip("\\n") + "\\n\\n" + generated
        if "--check" in sys.argv:
            if current != generated:
                stale.append(crate_name(src))
            continue
        if updated != text:
            LIB.write_text(updated, encoding="utf-8")
        print(f"{LIB.relative_to(ROOT)}: internals re-exports {generated.count('pub use')} module(s)")
    if "--check" in sys.argv:
        if stale:
            print(f"internals stale in {', '.join(stale)}; run scripts/generate_internals.py", file=sys.stderr)
            return 1
        print("internals are current")
    return 0


'''
            text = text[:start] + tail + text[end:]
            write(path, text)
    # check_api_inventory.py: count every crate's sources.
    path = root / "scripts/check_api_inventory.py"
    text = read(path)
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
    # check_docs_examples.py: the Rust target in the one member linking every crate (D13).
    path = root / "scripts/check_docs_examples.py"
    text = read(path)
    text = replace_once(text, 'RUST_TARGET = ROOT / "rust" / "tests" / "docs_examples.rs"', 'RUST_TARGET = ROOT / "cli" / "tests" / "docs_examples.rs"', "scripts/check_docs_examples.py")
    text, n = re.subn(
        r'\["cargo", "test",((?: "--locked",)?) "--features", "parquet iceberg s3 s3tables http3", "--test", "docs_examples"\]',
        r'["cargo", "test",\1 "-p", "yggdryl-cli", "--features", "yggdryl/parquet yggdryl/iceberg yggdryl/s3 yggdryl/s3tables yggdryl/http3", "--test", "docs_examples"]',
        text,
    )
    if n != 1:
        residue("scripts/check_docs_examples.py", "the `cargo test ... docs_examples` command was not found; run it under `-p yggdryl-cli` by hand")
    write(path, text)
    path = root / ".gitignore"
    write(path, replace_once(read(path), "rust/tests/docs_examples.rs", "cli/tests/docs_examples.rs", ".gitignore"))
    done("tooling: generate_internals.py and check_api_inventory.py per crate, the docs runner in cli/")


def edit_rows(root: pathlib.Path) -> None:
    path = root / ".github/ci/rows.toml"
    if not path.exists():
        residue(".github/ci/rows.toml", "absent in this tree: uncomment `market` and `fix` under `[leaves]` and move the shards by hand")
        return
    text = read(path)
    for name in ("market", "fix"):
        text = re.sub(rf"(?m)^# ({name} = \{{ package = [^\n]*\}})$", r"\1", text)
    # The moved targets take their shards with them.
    import tomllib

    data = tomllib.loads(text)
    core = data.get("shards", {}).get("yggdryl", {})
    moved_targets = {
        "test:fix": FIX, "bench:fix": FIX, "test:scale_ulbridge": FIX, "bench:fix_allocations": FIX,
        "test:graph": MARKET, "test:isin_registry": MARKET, "test:market_register": MARKET, "bench:graph": MARKET,
    }
    new: dict[str, dict[str, dict[str, list[str]]]] = {"yggdryl": {}, "yggdryl-market": {}, "yggdryl-fix": {}}
    for lane, shards in core.items():
        for shard, targets in shards.items():
            for target in targets:
                owner = moved_targets.get(target, CORE)
                new[PACKAGE[owner]].setdefault(lane, {}).setdefault(shard, []).append(target)
    # A split cost target is a target of each crate it went to.
    for crate in (MARKET, FIX):
        for rel in SPLIT_TARGETS:
            name = posixpath.basename(rel)[:-3]
            if (root / CRATE_DIR[crate] / "tests" / f"{name}.rs").exists():
                full = new[PACKAGE[crate]].setdefault("full", {})
                if not any(f"test:{name}" in targets for targets in full.values()) and name == "allocations":
                    full.setdefault("allocations", []).insert(0, "test:allocations")

    def render(package: str) -> str:
        lanes = new[package]
        lines = [f"[shards.{package}]"]
        for lane in ("default", "full"):
            shards = lanes.get(lane)
            if shards:
                body = ", ".join(f"{shard} = {toml_value(targets)}" for shard, targets in shards.items())
                lines.append(f"{lane} = {{ {body} }}")
        return "\n".join(lines) if len(lines) > 1 else ""

    m = re.search(r"(?m)^\[shards\.yggdryl\]\n(?:(?:default|full) = [^\n]*\n)*", text)
    if not m:
        residue(".github/ci/rows.toml", "no `[shards.yggdryl]` table to split")
    else:
        blocks = [render(p) for p in ("yggdryl", "yggdryl-market", "yggdryl-fix")]
        text = text[: m.start()] + "\n\n".join(b for b in blocks if b) + "\n" + text[m.end():]
    write(path, text)
    # The planner's own test reads every package's shards against its own targets.
    test = root / "scripts/tests/test_ci_plan.py"
    if test.exists():
        old = """    def test_the_named_shards_are_targets_the_core_has(self) -> None:
        manifest = tomllib.loads((ROOT / "rust" / "Cargo.toml").read_text(encoding="utf-8"))
        tests = [{"kind": ["test"], "name": path.stem} for path in (ROOT / "rust" / "tests").glob("*.rs")]
        benches = [{"kind": ["bench"], "name": bench["name"]} for bench in manifest["bench"]]
        targets = [{"kind": ["lib"], "name": "yggdryl"}, *tests, *benches]
        for lane in plan.LANES:
            arguments = {
                shard: plan.shard_arguments(CONFIG, "yggdryl", lane, shard, targets)
                for shard in [*CONFIG.shards["yggdryl"][lane], "rest"]
            }
            selected = [
                (flag, name)
                for flags in arguments.values()
                for flag, name in zip(flags, flags[1:])
                if flag in ("--test", "--bench")
            ]
            with self.subTest(lane=lane):
                self.assertEqual(len(selected), len(set(selected)))
                self.assertEqual(len(selected), len(tests) + len(benches))
                self.assertEqual(arguments["rest"][0], "--lib")
"""
        replacement = """    def test_the_named_shards_are_targets_their_package_has(self) -> None:
        folders = {"yggdryl": ROOT / "rust"}
        folders |= {leaf.package: ROOT / "rust" / name for name, leaf in CONFIG.leaves.items()}
        for package, lanes in CONFIG.shards.items():
            folder = folders[package]
            manifest = tomllib.loads((folder / "Cargo.toml").read_text(encoding="utf-8"))
            tests = [{"kind": ["test"], "name": path.stem} for path in (folder / "tests").glob("*.rs")]
            benches = [{"kind": ["bench"], "name": bench["name"]} for bench in manifest.get("bench", [])]
            targets = [{"kind": ["lib"], "name": package.replace("-", "_")}, *tests, *benches]
            for lane in plan.LANES:
                arguments = {
                    shard: plan.shard_arguments(CONFIG, package, lane, shard, targets)
                    for shard in [*lanes.get(lane, {}), "rest"]
                }
                selected = [
                    (flag, name)
                    for flags in arguments.values()
                    for flag, name in zip(flags, flags[1:])
                    if flag in ("--test", "--bench")
                ]
                with self.subTest(package=package, lane=lane):
                    self.assertEqual(len(selected), len(set(selected)))
                    self.assertEqual(len(selected), len(tests) + len(benches))
                    self.assertEqual(arguments["rest"][0], "--lib")
"""
        write(test, replace_once(read(test), old, replacement, "scripts/tests/test_ci_plan.py"))
    done("rows.toml: `market` and `fix` leaves listed, their targets' shards moved")


def edit_inventory(root: pathlib.Path, tables: Tables, rewriter: Rewriter, paths: PathMap) -> None:
    path = root / ".api-inventory.txt"
    text = read(path)
    lines = text.split("\n")
    out: list[str] = []
    graph_market: list[str] = []
    section = None
    in_graph = False
    for line in lines:
        m = re.match(r"^(###\s+)(\S+)(\s+\[)([^\]]+)(\].*)$", line)
        if m:
            label, file = m.group(2), m.group(4)
            new_file = paths(file)
            in_graph = file == "rust/src/graph/mod.rs"
            if new_file != file:
                segs = label.split("::")
                if segs[0] == "yggdryl" and len(segs) > 2 and segs[1] == "root":
                    owner = MARKET if segs[2] in MARKET_ROOT_FILES else FIX
                    segs = [CRATE_NAME[owner], *segs[1:]]
                elif segs[0] == "yggdryl":
                    owner, rest = tables.resolve(segs[1:])
                    if owner == CORE:
                        owner = MARKET if new_file.startswith("rust/market/") else FIX
                    segs = [CRATE_NAME[owner], *rest]
                line = m.group(1) + "::".join(segs) + m.group(3) + new_file + m.group(5)
            out.append(rewriter.inline(line, Context(None, False), ".api-inventory.txt"))
            continue
        if in_graph and line.strip():
            name = re.search(r"\b(?:mod|use)\s+(\w+)", line)
            macro = line.lstrip().startswith("macro_rules!")
            if macro or (name and name.group(1) not in CORE_GRAPH_NAMES):
                graph_market.append(line)
                continue
        out.append(rewriter.inline(line, Context(None, False), ".api-inventory.txt"))
    text = "\n".join(out)
    # The four kinds' markers and Arrow extension types are the market's now.
    market_lines = [
        "pub fn install() -> yggdryl::Result<()>  (claims the four market kinds on the core's register - byte, name, Arrow extension name - once for the process; every binding's init and the CLI's `main` call it)",
    ]
    marker = re.compile(r"(?m)^(pub struct MarketDataKindType  // [^\n]*\n  pub type MarketDataKindField = [^\n]*)\n")
    m = marker.search(text)
    if m:
        market_lines.append(m.group(1))
        text = text[: m.start()] + text[m.end():]
    else:
        residue(".api-inventory.txt", "the market markers' lines were not found under the string section; move them by hand")
    kinds = "MarketDataKindType, MarketDataTypeType, SideType, TimeInForceType, "
    ext = re.compile(r"(?m)^(impl ExtensionType for [^\n]*)" + re.escape(kinds) + r"([^\n]*)$")
    m = ext.search(text)
    if m:
        line = m.group(1) + m.group(2)
        line = line.replace(" - for the four market enums their kind's own `EXTENSION_NAME`, read in a const context -", "")
        text = text[: m.start()] + line + text[m.end():]
        market_lines.append(
            "impl ExtensionType for MarketDataKindType, MarketDataTypeType, SideType, TimeInForceType  (written beside each kind's marker by the core's exported `market_extension!`: NAME the kind's own `EXTENSION_NAME`, Metadata = (), no document)"
        )
    else:
        residue(".api-inventory.txt", "the `impl ExtensionType for ..` line naming the four kinds was not found; split it by hand")
    section = "### yggdryl_market  [rust/market/src/lib.rs]  (the crate root: the four kinds' markers, their Arrow extension types and the claim)\n" + "\n".join(market_lines) + "\n"
    fix_root = text.find("### yggdryl_fix  [rust/fix/src/lib.rs]")
    if fix_root >= 0:
        nl = text.find("\n", fix_root)
        text = text[: nl + 1] + "pub fn install() -> yggdryl::Result<()>  (claims the FIX Latest datatype names among the logical names, after `yggdryl_market::install()`, once for the process)\n" + text[nl + 1:]
    else:
        residue(".api-inventory.txt", "no `yggdryl_fix` section to list `install()` under")
    if graph_market:
        graph_section = "### yggdryl_market::graph  [rust/market/src/graph/mod.rs]\n" + "\n".join(graph_market) + "\n"
        anchor = text.find("### yggdryl::graph::element  [")
        text = (text[:anchor] + graph_section + "\n" + text[anchor:]) if anchor >= 0 else text + "\n" + graph_section
    anchor = text.find("### yggdryl_market::side  [")
    text = (text[:anchor] + section + "\n" + text[anchor:]) if anchor >= 0 else text.rstrip("\n") + "\n\n" + section
    write(path, text)
    done(".api-inventory.txt: the moved sections re-homed, the graph section split")


# ---------------------------------------------------------------------------
# What FIX reaches of the market's crate-private items
# ---------------------------------------------------------------------------


def module_chain_public(root: pathlib.Path, segs: list[str]) -> tuple[bool, pathlib.Path | None]:
    """Whether every module on `segs` is `pub` from the market's root, and the
    file holding the last one."""
    base = root / "rust/market/src"
    declaring = base / "lib.rs"
    public = True
    folder = base
    for seg in segs:
        text = read(declaring)
        m = re.search(rf"(?m)^(pub(?:\([^)]*\))?\s+)?mod {seg};", text)
        if not m:
            return public, None
        if (m.group(1) or "").strip() != "pub":
            public = False
        candidate = folder / f"{seg}.rs"
        declaring = candidate if candidate.exists() else folder / seg / "mod.rs"
        folder = folder / seg
        if not declaring.exists():
            return public, None
    return public, declaring


def reach_market_private(root: pathlib.Path, tables: Tables) -> list[str]:
    """Before any rewrite: FIX's `crate::` paths to market items no `pub`
    reaches. An item in a module the market's root does not publish is raised
    to `pub` where it stands and re-exported from `yggdryl_market::implementer`
    (AGENTS: a raise inside an unpublished module publishes nothing else), and
    `tables` routes the path there, so the rewrite spells it so in a `use` tree
    as well as inline; an item of a published module is residue - it needs a
    forwarder, whose signature only the compiler loop writes."""
    raised: list[str] = []
    implementer = root / "rust/market/src/implementer.rs"
    imp_text = read(implementer)
    found: dict[tuple[str, ...], str] = {}
    for path in sorted((root / "rust/fix/src").rglob("*.rs")):
        code = code_only(read(path))
        paths_named = [[s for s in m.group(3).split("::") if s] for m in INLINE_PATH.finditer(code) if m.group(2) == "crate"]
        for m in USE_START.finditer(code):
            rest = code[m.end():]
            end = rest.find(";")
            try:
                leaves = parse_use(re.sub(r"\s+", " ", rest[:end]))
            except UseParseError:
                continue
            paths_named += [leaf[1:] for leaf, _alias, kind in leaves if leaf[:1] == ["crate"] and kind == "self"]
        for segs in paths_named:
            owner, new = tables.resolve(segs)
            if owner == MARKET and len(new) >= 2 and new[0] != "implementer":
                found.setdefault(tuple(new), str(path.relative_to(root)))
    for full, where in sorted(found.items()):
        # The item is the longest prefix naming a definition in a module file.
        for cut in range(len(full) - 1, 0, -1):
            modules, name = list(full[:cut]), full[cut]
            public, file = module_chain_public(root, modules)
            if file is None:
                continue
            ftext = read(file)
            d = re.search(rf"(?m)^(pub(?:\([^)]*\))?\s+)?((?:const\s+)?(?:fn|struct|enum|trait|type|const|static|mod)\s+{name}\b)", ftext)
            if not d:
                continue
            vis = (d.group(1) or "").strip()
            if vis == "pub" or (d.group(2).startswith("mod ")):
                break
            label = f"{where} -> yggdryl_market::{'::'.join(modules)}::{name}"
            if public:
                residue(label, f"`{vis or 'private'}` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)")
                break
            ftext = ftext[: d.start()] + "pub " + ftext[d.start(2):]
            write(file, ftext)
            line = f"pub use crate::{'::'.join(modules)}::{name};"
            if line not in imp_text:
                imp_text += f"/// `{'::'.join(modules)}::{name}`, raised to `pub` in a module the root does not publish.\n{line}\n"
            tables.private_routes[tuple(modules) + (name,)] = name
            raised.append(f"{'::'.join(modules)}::{name}")
            break
    write(implementer, imp_text)
    return raised


# ---------------------------------------------------------------------------
# Step 13: what the compiler loop still has to see
# ---------------------------------------------------------------------------

_KINDS = "side|timeinforce|marketdatakind|marketdatatype"
# A core test reading a market kind through the register: a parse of its
# name, its extension name, its byte, a pinned pair or a listing of names.
KIND_WORDS = re.compile(
    rf'(?:from_str|parse|from_logical_name|kind_named|from_spelling)\(\s*"(?:{_KINDS})"'
    rf'|"yggdryl\.(?:{_KINDS})"'
    r"|\bDataTypeId::market\(0xc[2-5]\)"
    rf'|\(\s*"(?:{_KINDS})"\s*,\s*0x'
    rf'|(?m:^\s*"(?:{_KINDS})",\s*$)'
    # A helper handed a kind's name and a value (`text("side", "BUY")`).
    rf'|\b\w+\(\s*"(?:{_KINDS})"\s*,\s*"'
)


DOC_LINE = re.compile(r"^[ \t]*//[/!][ ]?(.*)$")


def core_doctest_reaches(text: str, tables: Tables) -> list[tuple[int, str, str]]:
    """The moved names a core rustdoc example reaches, by `use yggdryl::...`
    or an inline `yggdryl::` path, with the line each is on."""
    found: list[tuple[int, str, str]] = []
    fence: str | None = None
    rust = False
    for number, line in enumerate(text.split("\n"), 1):
        doc = DOC_LINE.match(line)
        if doc is None:
            fence = None
            continue
        body = doc.group(1)
        opening = re.match(r"^(`{3,})(\w*)", body.strip())
        if opening:
            if fence is None:
                fence = opening.group(1)
                rust = opening.group(2) in ("", "rust", "no_run", "should_panic", "ignore", "compile_fail", "edition2024")
            elif body.strip() == fence:
                fence = None
            continue
        if fence is None or not rust:
            continue
        code = re.sub(r"^\s*#\s?", "", body)
        use = re.match(r"\s*(?:pub\s+)?use\s+(.*?);?\s*$", code)
        if use:
            try:
                leaves = parse_use(re.sub(r"\s+", " ", use.group(1)))
            except UseParseError:
                leaves = []
            for leaf, _alias, _kind in leaves:
                if leaf[:1] == ["yggdryl"] and len(leaf) > 1:
                    owner, _ = tables.resolve(leaf[1:])
                    if owner != CORE:
                        found.append((number, owner, "::".join(leaf)))
            continue
        for m in INLINE_PATH.finditer(code):
            if m.group(2) == "yggdryl":
                owner, _ = tables.resolve([s for s in m.group(3).split("::") if s])
                if owner != CORE:
                    found.append((number, owner, m.group(0)))
    return found


def scan_residue(root: pathlib.Path, tables: Tables) -> None:
    # Core sources naming a moved item by a `crate::` path (doc links mostly).
    for path in sorted((root / "rust/src").rglob("*.rs")):
        rel = str(path.relative_to(root))
        text = read(path)
        for m in INLINE_PATH.finditer(text):
            if m.group(2) not in ("crate", "$crate"):
                continue
            owner, _ = tables.resolve([s for s in m.group(3).split("::") if s])
            if owner != CORE:
                residue(f"{rel}:{line_of(text, m.start())}", f"the core names `{m.group(0)}`, which `{PACKAGE[owner]}` holds now: reword the doc link or drop it (D14)")
        for m in re.finditer(r"\b(?:yggdryl_market|yggdryl_fix)::", code_only(text)):
            residue(f"{rel}:{line_of(text, m.start())}", "the core's code names a split crate")
        for line, owner, spelled in core_doctest_reaches(text, tables):
            residue(f"{rel}:{line}", f"a core rustdoc example names `{spelled}`, which `{PACKAGE[owner]}` holds now: the core's doctests cannot name it - re-fixture the example on a core type or drop the line")
    # Core tests and benches still reaching a moved name: D14/D39, re-fixture or move.
    per_file: dict[str, list[int]] = collections.defaultdict(list)
    kinds_per_file: dict[str, list[int]] = collections.defaultdict(list)
    for folder in ("rust/tests", "rust/benchmarks", "rust/examples"):
        base = root / folder
        if not base.exists():
            continue
        for path in sorted(base.rglob("*.rs")):
            rel = str(path.relative_to(root))
            text = read(path)
            code = code_only(text)
            for m in INLINE_PATH.finditer(code):
                if m.group(2) == "yggdryl":
                    owner, _ = tables.resolve([s for s in m.group(3).split("::") if s])
                    if owner != CORE:
                        per_file[rel].append(line_of(text, m.start()))
            for m in USE_START.finditer(code):
                rest = code[m.end():]
                end = rest.find(";")
                try:
                    leaves = parse_use(re.sub(r"\s+", " ", rest[:end]))
                except UseParseError:
                    continue
                if any(l[:1] == ["yggdryl"] and tables.resolve(l[1:])[0] != CORE for l, _a, _k in leaves):
                    per_file[rel].append(line_of(text, m.start()))
            for m in KIND_WORDS.finditer(text):
                kinds_per_file[rel].append(line_of(text, m.start()))
    for rel, lines_ in sorted(per_file.items()):
        lines_ = sorted(set(lines_))
        shown = ", ".join(str(n) for n in lines_[:12]) + (" ..." if len(lines_) > 12 else "")
        residue(rel, f"a core test or bench names a moved item at {len(lines_)} line(s) ({shown}): move the test to the crate's own `tests/` (D39) or re-fixture it on the test-only kind (D14)")
    for rel, lines_ in sorted(kinds_per_file.items()):
        if rel in per_file:
            continue
        lines_ = sorted(set(lines_))
        shown = ", ".join(str(n) for n in lines_[:12]) + (" ..." if len(lines_) > 12 else "")
        residue(rel, f"a core test or bench reads a market kind by name at {len(lines_)} line(s) ({shown}): no core process claims one after S4 - move it or re-pin (runtime, not a compile error)")
    # The market crate may not name FIX: it is the dependency's dependency.
    for path in sorted((root / "rust/market").rglob("*.rs")):
        text = read(path)
        for m in re.finditer(r"\byggdryl_fix\b", code_only(text)):
            residue(f"{path.relative_to(root)}:{line_of(text, m.start())}", "the market crate names `yggdryl_fix` (a cycle): D9/D37 left this behind")
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
    # A graph module imported whole is two modules now.
    for path in sorted(list((root / "rust/market").rglob("*.rs")) + list((root / "rust/fix").rglob("*.rs")) + list((root / "python/src").rglob("*.rs")) + list((root / "node/src").rglob("*.rs")) + list((root / "cli").rglob("*.rs"))):
        text = read(path)
        for m in re.finditer(r"\buse\s+yggdryl::graph\s*;|\byggdryl::graph::\{\s*self\b", text):
            residue(f"{path.relative_to(root)}:{line_of(text, m.start())}", "`yggdryl::graph` imported whole: the market's half is `yggdryl_market::graph`")


# ---------------------------------------------------------------------------
# The run
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=pathlib.Path)
    parser.add_argument("--no-git-lock", action="store_true", help="the caller holds $S/git.lock already")
    parser.add_argument("--residue", type=pathlib.Path, default=SCRATCH / "s4" / "residue.md")
    arguments = parser.parse_args()
    root = arguments.tree.resolve()
    # Step 0: a tree already moved is left as it is.
    if (root / "rust/fix/Cargo.toml").exists() or (root / "rust/market/Cargo.toml").exists():
        print("[s4] rust/market or rust/fix exists: the move has run on this tree; nothing changed")
        return 0
    if not (root / "rust/src/fix/mod.rs").exists():
        raise SystemExit("rust/src/fix/mod.rs is missing: this is not the tree S4 moves")
    if git(root, "status", "--porcelain").strip():
        raise SystemExit("the tree has changes; run S4 on a clean tree")
    version = re.search(r'(?m)^version = "([^"]+)"', read(root / "Cargo.toml")).group(1)

    # Step 1: who owns what, read before anything moves.
    tables = Tables(root)
    done(f"tables: {len(tables.market_names)} market root names, {len(tables.fix_names)} FIX names, {len(tables.graph_market_modules)} graph modules leave")
    moves = plan_moves(root)
    paths = PathMap(moves)
    core_lib_market = split_core_lib(root)
    graph_market = split_graph_mod(root)
    # Step 7 runs before the moves: the split reads the core's paths, and
    # its import prune the methods of every trait the tree defines.
    load_trait_methods(root)
    splits = split_tests(root, tables, moves)

    # Step 2: the moves.
    with GitLock(not arguments.no_git_lock):
        for old, new in moves:
            (root / new).parent.mkdir(parents=True, exist_ok=True)
            git(root, "mv", old, new)
        for old in DELETED:
            if (root / old).exists():
                git(root, "rm", "-q", old)
    done(f"moves: {len(moves)} files moved by git, {len(DELETED)} examples deleted")
    split_root_harness(root, paths)
    write(root / "rust/market/src/graph/mod.rs", graph_market)
    write_market_lib(root, core_lib_market)
    convert_fix_lib(root)

    # Step 5: the core's seams.
    edit_market_rs(root)
    edit_vocabulary(root)
    raised = reach_market_private(root, tables)
    done(f"market items FIX reaches raised and routed through its implementer: {', '.join(raised) or 'none'}")

    # Step 6: every path, by the context its file is in.
    rewriter = Rewriter(tables)
    moved_new = {new: old for old, new in moves}
    for old, new in splits:
        moved_new[new] = old
    for path in sorted((root / "rust/market").rglob("*.rs")) + sorted((root / "rust/fix").rglob("*.rs")):
        rel = str(path.relative_to(root))
        crate = MARKET if rel.startswith("rust/market/") else FIX
        old = moved_new.get(rel, rel)
        text = read(path)
        # A split file's harness declares the crate's own copy of each module.
        local = paths.with_files({o: n for o, n in splits if crate_of_path(n) == crate})
        text = reanchor(text, old, rel, local)
        if "/src/" in rel:
            ctx = Context(crate, True, graph_file=rel.startswith("rust/market/src/graph/"))
        else:
            ctx = Context(None, False)
        text = rewriter.rs_file(text, ctx, rel)
        if rel.startswith("rust/market/src/"):
            text, n = unlink_foreign(text, MARKET, tables)
            if n:
                residue(rel, f"{n} doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword")
        write(path, text)
    edit_extension(root)
    for folder in ("python/src", "node/src", "cli"):
        for path in sorted((root / folder).rglob("*.rs")):
            rel = str(path.relative_to(root))
            if "/target/" in rel:
                continue
            text = read(path)
            new = rewriter.rs_file(reanchor(text, rel, rel, paths), Context(None, False), rel)
            if new != text:
                write(path, new)
    # Staying core files: only paths that name a moved file.
    for folder in ("rust/src", "rust/tests", "rust/benchmarks"):
        for path in sorted((root / folder).rglob("*.rs")):
            rel = str(path.relative_to(root))
            text = read(path)
            new = reanchor(text, rel, rel, paths)
            if folder == "rust/src":
                new, n = unlink_foreign(new, CORE, tables)
                if n:
                    residue(rel, f"{n} doc link(s) to moved items unlinked to their paths: reword (D14)")
            if new != text:
                write(path, new)
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text = read(root / f)
            new = rewriter.markdown(text, f)
            if new != text:
                write(root / f, new)
    for f in ("AGENTS.md", "README.md", "rust/README.md"):
        if (root / f).exists():
            text = read(root / f)
            write(root / f, rewriter.inline(text, Context(None, False), f))
    edit_inventory(root, tables, rewriter, paths)
    count = 0
    for f in text_files(root):
        text = read(root / f)
        new = rewrite_text_paths(text, paths, moves)
        if new != text:
            write(root / f, new)
            count += 1
    done(f"paths: {rewriter.stats['use']} use statements and {rewriter.stats['inline']} paths re-owned; file paths in {count} files")

    # Step 8: install.
    harnesses = 0
    tests = 0
    for crate in (MARKET, FIX):
        base = root / CRATE_DIR[crate]
        write(base / "tests/support/install.rs", INSTALL_SUPPORT[crate])
        for path in sorted((base / "tests").glob("*.rs")):
            write(path, declare_install(read(path)))
            harnesses += 1
        for path in sorted((base / "tests").rglob("*.rs")):
            if "/support/" in str(path):
                continue
            text, n = insert_test_installs(read(path))
            tests += n
            write(path, text)
        for path in sorted((base / "benchmarks").glob("*.rs")):
            write(path, bench_main(read(path), crate))
        for path in sorted((base / "src").rglob("*.rs")):
            text, n = insert_doc_installs(read(path), crate)
            if n:
                write(path, text)
    pages = 0
    for f in tracked(root, "docs", "skills"):
        if f.endswith(".md"):
            text, n = insert_markdown_installs(read(root / f))
            if n:
                write(root / f, text)
                pages += n
    install_at_init(root)
    done(f"install: {harnesses} harnesses, {tests} tests, the benches' mains, every moved rustdoc example, {pages} page blocks, the bindings and the CLI")

    # Step 9-12.
    write_manifests(root, version)
    edit_tooling(root)
    result = subprocess.run([sys.executable, "-I", str(root / "scripts/generate_internals.py")], capture_output=True, text=True)
    done(f"generate_internals.py: {result.stdout.strip() or result.stderr.strip()}")
    edit_rows(root)

    # Step 13.
    scan_residue(root, tables)
    with GitLock(not arguments.no_git_lock):
        git(root, "add", "-A", "--", "rust/market", "rust/fix")
    report = ["# S4 residue", "", f"Tree: `{root}`", "", "## Done", ""] + [f"- {d}" for d in DONE] + ["", "## Residue", ""] + (RESIDUE or ["- none"])
    arguments.residue.parent.mkdir(parents=True, exist_ok=True)
    arguments.residue.write_text("\n".join(report) + "\n", encoding="utf-8")
    print(f"[s4] residue: {len(RESIDUE)} item(s) in {arguments.residue}")
    return 0


def split_root_harness(root: pathlib.Path, paths: PathMap) -> None:
    """The root files the market took leave the core's `tests/root.rs` for the
    market's own, declared as the core declared them; a market `root.rs` the
    test split wrote already gains them."""
    path = root / "rust/tests/root.rs"
    text = read(path)
    head, items = top_items(text)
    # The root files that moved whole: a split file keeps its core half.
    moved = {
        posixpath.basename(old)[:-3]
        for old, new in paths.files.items()
        if old.startswith("rust/tests/root/") and new.startswith("rust/market/tests/root/")
    }
    keep = [head]
    take: list[str] = []
    pos = len(head)
    for item in items:
        keep.append(text[pos:item.start])
        pos = item.end
        if item.kind == "mod" and item.name in moved and f'"root/{item.name}.rs"' in item.text:
            take.append(item.text.strip("\n"))
        else:
            keep.append(item.text)
    keep.append(text[pos:])
    write(path, "".join(keep))
    market = root / "rust/market/tests/root.rs"
    declared = reanchor("\n".join(take) + "\n", "rust/tests/root.rs", "rust/market/tests/root.rs", paths)
    if market.exists():
        current = read(market)
        mine = {m.group(1) for m in re.finditer(r"(?m)^mod (\w+);", current)}
        extra = [d for d in take if not any(f"mod {n};" in d for n in mine)]
        text = current.rstrip("\n") + "\n" + reanchor("\n".join(extra) + "\n", "rust/tests/root.rs", "rust/market/tests/root.rs", paths)
    else:
        text = (
            "//! One test file per root file the market crate holds, under `tests/root/`,\n"
            "//! as `rust/tests/root.rs` is the core's; `root/market.rs` pins the core's\n"
            "//! register over the four kinds this crate claims (D39).\n\n" + declared
        )
    write(market, text)
    done(f"rust/market/tests/root.rs: {len(take)} root test files declared")


def install_at_init(root: pathlib.Path) -> None:
    path = root / "python/src/lib.rs"
    write(
        path,
        replace_once(
            read(path),
            "    logging::install(module.py())?;\n",
            "    logging::install(module.py())?;\n"
            "    // The crates split off the core claim what they register before a\n"
            "    // class can read a name of theirs, in dependency order.\n"
            "    yggdryl_market::install().map_err(value_error)?;\n"
            "    yggdryl_fix::install().map_err(value_error)?;\n",
            "python/src/lib.rs",
        ),
    )
    path = root / "node/src/lib.rs"
    write(
        path,
        replace_once(
            read(path),
            "fn install_logging() {\n    logging::install();\n}",
            "fn install_logging() {\n    logging::install();\n"
            "    // The crates split off the core claim what they register before an\n"
            "    // export can read a name of theirs, in dependency order; a refusal is\n"
            "    // a build linking two claimants, which no caller can repair.\n"
            '    yggdryl_market::install().expect("yggdryl-market claims its kinds");\n'
            '    yggdryl_fix::install().expect("yggdryl-fix claims its names");\n}',
            "node/src/lib.rs",
        ),
    )
    path = root / "cli/src/main.rs"
    write(
        path,
        replace_once(
            read(path),
            "    warnings::install();\n",
            "    warnings::install();\n"
            "    // The crates split off the core claim what they register before an\n"
            "    // argument can name a kind of theirs, in dependency order.\n"
            "    if let Err(refusal) = yggdryl_market::install().and_then(|()| yggdryl_fix::install()) {\n"
            "        style::bad(&refusal.to_string());\n"
            "        return ExitCode::FAILURE;\n"
            "    }\n",
            "cli/src/main.rs",
        ),
    )


if __name__ == "__main__":
    raise SystemExit(main())
