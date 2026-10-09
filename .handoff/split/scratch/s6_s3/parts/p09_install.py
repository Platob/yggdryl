

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
