# Testing

Every check runs from the repository root, which owns the Cargo workspace.

## Full pass

=== "Rust"

    ```bash
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-targets --all-features
    cargo test --workspace --doc --all-features
    ```

=== "Python"

    ```bash
    cd python
    .venv/bin/python -m maturin develop
    .venv/bin/python -m pytest
    .venv/bin/python -m mypy --strict yggdryl tests/typing_bindings.py tests/typing_fields.py
    ```

=== "JavaScript"

    ```bash
    npm ci --prefix node
    npm run --prefix node build:debug
    cargo build --locked -p yggdryl-cli   # the command node/tests/book.test.js spawns
    npm test --prefix node
    ```

| Pass | Toolchain |
| --- | --- |
| Every feature, both bindings | Rust 1.94 or newer |

## By entry

`rust/tests/` mirrors `rust/src/` file for file, and one target declares each
top-level entry: a source folder is declared by `rust/tests/<folder>.rs`, and
the files the crate root holds by `rust/tests/root.rs`. Five targets stand
outside the mirror because what they pin is a cost or an exchange rather than a
file. `--all-features` is what turns `internals` on, and with it a target runs
everything it declares.

=== "Rust"

    ```bash
    cargo test -p yggdryl --all-features --test root
    cargo test -p yggdryl --all-features --test arrow
    cargo test -p yggdryl --all-features --test auth
    cargo test -p yggdryl --all-features --test avro
    cargo test -p yggdryl --all-features --test aws
    cargo test -p yggdryl --all-features --test charset
    cargo test -p yggdryl --all-features --test coding
    cargo test -p yggdryl --all-features --test csv
    cargo test -p yggdryl --all-features --test expression
    cargo test -p yggdryl --all-features --test fix
    cargo test -p yggdryl --all-features --test fs
    cargo test -p yggdryl --all-features --test graph
    cargo test -p yggdryl --all-features --test hashing
    cargo test -p yggdryl --all-features --test holder
    cargo test -p yggdryl --all-features --test http
    cargo test -p yggdryl --all-features --test iceberg
    cargo test -p yggdryl --all-features --test iobase
    cargo test -p yggdryl --all-features --test ipc
    cargo test -p yggdryl --all-features --test json
    cargo test -p yggdryl --all-features --test local
    cargo test -p yggdryl --all-features --test media
    cargo test -p yggdryl --all-features --test media_type
    cargo test -p yggdryl --all-features --test metadata
    cargo test -p yggdryl --all-features --test mime_type
    cargo test -p yggdryl --all-features --test parquet
    cargo test -p yggdryl --all-features --test s3
    cargo test -p yggdryl --all-features --test s3tables
    cargo test -p yggdryl --all-features --test serie
    cargo test -p yggdryl --all-features --test soap
    cargo test -p yggdryl --all-features --test text
    cargo test -p yggdryl --all-features --test toml
    cargo test -p yggdryl --all-features --test txhash
    cargo test -p yggdryl --all-features --test uri
    cargo test -p yggdryl --all-features --test value
    cargo test -p yggdryl --all-features --test warehouse
    cargo test -p yggdryl --all-features --test xml
    cargo test -p yggdryl-xmla --all-features --test root          # the XML for Analysis crate
    cargo test -p yggdryl-xmla --all-features --test iobase_calls  # its pinned `IOBase` call counts
    cargo test -p yggdryl-xmla --all-features --test allocations   # its counting allocator
    cargo test -p yggdryl --all-features --test xxhash
    cargo test -p yggdryl --all-features --test yaml
    cargo test -p yggdryl --all-features --test zip
    cargo test -p yggdryl --all-features --test allocations     # the counting allocator
    cargo test -p yggdryl --all-features --test iobase_calls    # the pinned `IOBase` call counts
    cargo test -p yggdryl --all-features --test benchmark_mode  # every benchmark at its smoke corpus
    cargo test -p yggdryl --all-features --test docs_index      # the landing-page example
    cargo test -p yggdryl --all-features --test interop         # the exchanges with an outside implementation
    cargo test -p yggdryl --all-features --test spill_doors     # the doors that settle under the process spill bound, in a process of their own
    cargo test -p yggdryl --all-features --test scale_ulbridge  # the capture pipeline on series, table to table, three copies of the capture
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py   # one source file
    python/.venv/bin/python -m pytest python/tests/text               # one source folder
    python/.venv/bin/python -m pytest python/tests                    # the whole binding
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/datatype.test.js      # one source file
    node --test "node/tests/text/*.test.js"      # one source folder
    npm test --prefix node                       # the whole binding, and `tsc --noEmit`
    ```

## The capture path at scale

```bash
YGGDRYL_SCALE_BYTES=21474836480 cargo test --release -p yggdryl \
    --test scale_ulbridge --features iceberg -- --ignored --nocapture
```

`rust/tests/scale_ulbridge.rs` runs the capture pipeline end to end on series, over a ULBridge capture of any size - the 144 lines of `rust/tests/fix/ulbridge.log` repeated, every clock and identifier stepped per copy, written through the crate's own Zstandard encoder into several `.log.zst` files. The folder of files is read as one stream of text rows (`read_serie`) and appended into an Iceberg table (`append_serie`); that table is read back in its order, parsed and walked (`parse_text_serie`, `lifecycle_serie`) and written over a second table (`overwrite_serie`); and that one is read back in its order into books, the complete book of every quarter of an hour written over a third table, and between them every book's `delta` - the orders and quotes it applied - and its `events` - the executions and snapshot controls it recorded - each flattened to `marketdata` rows over a table of its own. Every table is created from the schema of the stream written to it, partitioned by `partunix` - `time_bucket('15 minutes', currunix)`, a column the table computes - and sorted by `partunix, currunix, seqnum, currhashcode`. The ordinary pass runs three copies and checks every table row by row: each row in the quarter its instant falls in, a read in the table's order, the identity columns typed `uuid`, a second run of the FIX stage leaving the table as it was, and a run over one window of the text rewriting the partitions that window's rows reach and no other. The scale run asserts that the process's `RssAnon` stays where it stood a quarter of the way in, where the platform states one; it is `#[ignore]`d and a no-op printing `SKIPPED` unless `YGGDRYL_SCALE_BYTES` names the uncompressed size to generate, so no CI job runs it. `YGGDRYL_SCALE_STAGE` (`lines`, `text`, `parse`, `lifecycle`, `fix`, `books`) ends the path early to put a growth on the stage that owns it, and `YGGDRYL_SCALE_FOLDER` is where the input and the tables are written.

## The documentation is tested too

```bash
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

The first command compiles every `rust` block under `docs/` and `skills/` as a test, runs every `python` block under `python/.venv`, and every `javascript` block under node with `yggdryl` rewired to this checkout. The second builds the site strictly, which validates every link.

A block that cannot stand alone is tagged `{ .rust .ignore }`, `{ .python .ignore }`, or `{ .javascript .ignore }`; the checker reports those instead of hiding them.

## The wheel is tested as installed

```bash
python scripts/check_wheel_smoke.py
```

What `pip install yggdryl` gives a reader, and nothing else asks: the extension
module loads, a handle opens a folder, and an Iceberg table takes rows and
gives them back. It imports `yggdryl` from the environment and never
`python/yggdryl`, so it reports on an installed distribution rather than the
source tree beside it - install a wheel first, which `maturin develop` also
satisfies.

It runs in one of two halves, and the caller says which. With no flag it runs
all of it: the Iceberg round trip and the table served over XMLA by the
`yggdryl` command the wheel installed. Under `--extension-only` it loads the installed
`yggdryl/_native*` extension module alone - `import yggdryl` imports PyArrow -
parses a datatype through it, finds the command beside the interpreter, and
prints one `SKIPPED` line naming what did not run and why. On a free-threaded
interpreter both halves assert the GIL is still off once the extension is
loaded, because an extension that does not declare itself safe without the GIL
turns it back on with nothing worse than a warning.

Every platform ships four wheels, Windows arm64 the last three:

| Wheel | Loaded by | Smoked |
| --- | --- | --- |
| `cp310-cp310` | CPython 3.10 | whole, but not on musllinux |
| `cp311-abi3` | every GIL CPython from 3.11 | whole, on 3.11 to 3.14 |
| `cp314-cp314t` | free-threaded CPython 3.14 | whole, on 3.14t |
| `cp315-abi3.abi3t` | every CPython from 3.15, GIL or free-threaded | the extension alone, on 3.15 and 3.15t |

A free-threaded CPython loads no `abi3` wheel, so 3.14t has a wheel of its
own, and from 3.15 PEP 803's free-threaded stable ABI serves both builds; one
maturin invocation builds one stable-ABI family, so the last two are a second
build. Free threading starts at 3.14: CPython declared it supported there and
PyO3 dropped the experimental 3.13t with it (PyO3 #5865), so every PyO3 that
builds abi3t refuses a free-threaded CPython below 3.14 (`pyo3-ffi`'s
`MIN_FREE_THREADED_VERSION`); 3.12 has no free-threaded build at all (PEP 703's
first is 3.13), and the abi3t wheel is 3.15's - no earlier CPython loads it -
so a GIL-enabled 3.12 or 3.13 loads the `cp311-abi3` wheel and a free-threaded
3.13 has none. PyArrow publishes a wheel for 3.14t on every platform it
publishes for at all and for 3.15 on none yet, which is why 3.15 runs the extension-only half
under `--extension-only` - the row's statement, never the environment's, so a
lane owing the whole smoke fails on a missing or broken PyArrow, and the day
PyArrow ships a cp315 wheel the row drops the flag.

The release runs it against every wheel it is about to publish that a runner can
load: the whole of it on each CPython PyArrow has a wheel for, and the extension
alone on 3.15 and 3.15t. Four wheels are never loaded - the Windows arm64
`cp311-abi3` and `cp314-cp314t` ones, whose platform PyArrow publishes no wheel
for (that row loads its abi3t extension on 3.15 and 3.15t alone), and the two
musllinux CPython 3.10 ones, whose extension the release reads for initial-exec
thread-locals instead. CI's Python lane runs it against the wheel that job
builds, under both PyArrow versions the binding supports, and CI's free-threaded
lane builds the `cp314-cp314t` and `cp315-abi3.abi3t` wheels, runs the script
and the whole suite on 3.14t - `polars` publishes no free-threaded wheel, so its
suites skip there and nowhere else - and loads the abi3t extension on both 3.15
builds.

The script lived in a `release.yml` heredoc until a renamed module reached 0.1.9
and stopped the release there, which is why it is a file both sides share. That
release stopped quietly - the wheels failed, the two jobs below them were
skipped rather than failed, and the version reached crates.io and npm without
reaching PyPI or growing a tag. It was finished four commits later, from the
tree `main` held by then, so 0.1.9's wheel is not built from the tree its crate
and its npm package are: one number came to name two libraries, which is the
whole reason the rules below exist. A release that was going to publish and did
not now files an issue naming what each registry holds, and `preflight` refuses
a branch push that would publish a version some registry already carries - the
tag is what pins a tree, so a half-published version is finished from the commit
it was built at.

## Exchange formats meet an outside implementation

```bash
python scripts/check_avro_interop.py
python scripts/check_object_interop.py
python scripts/check_azure_interop.py
python scripts/check_gcs_interop.py
python scripts/check_iceberg_interop.py
python scripts/setup_spark_interop.py
python -m pytest python/tests -m spark_interop
AVRO_FUZZ_ITERATIONS=200000 cargo test -p yggdryl --features internals --test avro -- mod_::fuzz_lite
```

| Script | Exchanges |
| --- | --- |
| `check_avro_interop.py` | Avro containers with fastavro both ways, logical types included, plus the `apache-avro` crate |
| `check_object_interop.py` | S3 objects with boto3 both ways against MinIO: awkward keys, ranged reads, and a multipart upload verified from the outside |
| `check_azure_interop.py` | Blobs with azure-storage-blob both ways against Azurite, which recomputes the Shared Key signature itself: awkward names, ranged reads, and a block-list upload verified from the outside |
| `check_gcs_interop.py` | Objects with google-cloud-storage both ways against fake-gcs-server: names escaped into one path segment, and a resumable upload verified from the outside. The emulator accepts any token, so this proves the dialect and not the identity |
| `check_iceberg_interop.py` | Whole Iceberg tables with PyIceberg both ways, format versions 1 to 3 |
| `setup_spark_interop.py` + the `spark_interop` marker | One Hadoop warehouse shared with Apache Spark, both directions |
| `AVRO_FUZZ_ITERATIONS` | Seeded Avro mutations; the ordinary pass runs a short sweep |

A skipped half fails its driver, so a skipped exchange never reads as a pass.

## What a test looks like here

- A name states the behaviour: `a_missing_stream_reads_as_empty_rather_than_failing`.
- One behaviour per test, and an assertion message that carries the case when the test loops.
- Refusals sit beside the happy path: a cast that must fail, a root that must be non-null, a Parquet handle that must reject an outer coding, a budget that must refuse before allocating.
- A claim that "this does not scale with N" is counted, not asserted: `rust/tests/allocations.rs` drains the same work at two corpus sizes under a counting allocator and asserts equal counts.

## Layout

| Where | What |
| --- | --- |
| `rust/tests/<entry>.rs` + `rust/tests/<entry>/` | One target per top-level source entry, declaring the mirror of each of its files |
| `rust/tests/root/<name>.rs` | The file `rust/src/<name>.rs` holds, pinned; a folder's own `mod.rs` is pinned by `mod_.rs` |
| `rust/tests/support/` | Fixtures several targets declare - a counting allocator, an in-process S3 |
| `rust/tests/allocations.rs`, `iobase_calls.rs`, `benchmark_mode.rs`, `docs_index.rs`, `interop/` | What is pinned as a cost or an exchange rather than as a file |
| `rust/tests/spill_doors.rs`, `scale_ulbridge.rs` | What must own its process: the spill doors install the process spill bound before anything reads it, and the scale run measures the process's `RssAnon` while a ULBridge capture streams from a `.log.zst` into an Iceberg table - three copies in the ordinary loop, the scale run `#[ignore]`d and a no-op printing `SKIPPED` unless `YGGDRYL_SCALE_BYTES` names the size to generate |
| `python/tests/**/test_<name>.py` | The mirror of `python/yggdryl/` and `python/src/`, which share one shape |
| `node/tests/**/<name>.test.js` | The mirror of `node/src/` and the JavaScript beside it, plus `tsc --noEmit` over the `.types.ts` files |
| `*/benchmarks/<theme>*` | [Benchmarks](benchmarks.md), which stay grouped by a caller's vocabulary |
