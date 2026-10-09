

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
