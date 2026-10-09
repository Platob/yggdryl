

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
