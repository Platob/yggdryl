

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
