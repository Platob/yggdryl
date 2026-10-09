

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
