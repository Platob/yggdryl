

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
