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
