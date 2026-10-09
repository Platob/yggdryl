

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
