

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
