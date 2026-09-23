# Yggdryl

Yggdryl is a focused Rust core for Arrow-native schemas, validated resource
identifiers, byte-oriented structured text, and record I/O. `DataType`, `Field`,
immutable `Metadata`, `MimeType`, `MediaType`, the unified `TimeUnit`, `Scheme`,
`Uri`, `Url`, `Urn`, and `Scalar` own parsing, validation, comparison,
hashing, and serialization in one place. Python and JavaScript are runtime views
of those native values; neither maintains a parallel schema or codec model.

A struct `Field` is the schema. There is no separate record or schema type: a
non-null `Struct` field describes rows, and a row is one ordered
`Scalar::Sequence` with one value per child field.

Query execution, network clients, and transport protocols are outside the
project's scope.

## Documentation

Start with the [Yggdryl documentation](https://platob.github.io/yggdryl/) for
copyable Rust, Python, and JavaScript examples. The same pages live in
[`docs/`](docs/index.md), including the
[getting-started guide](docs/getting-started.md) and the
[architecture reference](docs/architecture.md). One tab per core layer and one
page per family in that layer, so the site tree and source tree agree:

| Layer | Tab |
| --- | --- |
| Datatypes, fields, scalars, casting, families | [types](docs/types/index.md) |
| Storage handles and backends | [holder](docs/holder/index.md) |
| Record encodings, tables, documents, codings, charsets | [media](docs/media/index.md) |
| Identifiers | [uri](docs/uri/index.md) |
| Arrow, expressions, hashing, FIX | [arrow](docs/arrow/index.md), [expression](docs/expression/index.md), [hashing](docs/hashing.md), [fix](docs/fix/index.md) |

Cross-runtime examples use linked tabs: choose Rust, Python, or JavaScript once
and the site keeps that context while you move between pages.

```console
python -m pip install --requirement requirements-docs.txt
python -m mkdocs serve --strict
```

Documentation changes in pull requests build the site in strict mode. Matching
pushes to `main` publish the result to GitHub Pages.

## Layout

```text
rust/                    The core crate
  src/*.rs               One type with its datatype, field and scalar, or
                         one shared trait, enum or value, per root file
  src/holder/            What every storage backend shares; local/, fs/,
                         zip/ and s3/ are one root folder each
  src/coding/            What every codec shares; gzip.rs, zlib.rs and
                         zstd.rs are one root file each
  src/charset/           What every code page shares; utf8.rs, ascii.rs
                         and cp1252.rs are one root file each
  src/media/             What every medium shares; avro/, ipc/, parquet/
                         and iceberg/ are one root folder each
  src/text/              Plain text and what the structured codecs share;
                         json/, toml/ and yaml/ are one root folder each
  src/hashing/           The private stable-hash adapters; xxhash/ and
                         txhash/ are one root folder each
  src/{uri,arrow,expression,graph,fix}/
                         The remaining core layers
  tests/                 One test file per source file, at the mirrored path
  benchmarks/            Criterion targets, grouped by theme
python/                  The Python extension
  src/                   PyO3 views over the matching core domains
  yggdryl/               The Python package, including field classes and annotations
  tests/                 The mirror of both, file for file
node/                    The JavaScript extension
  src/                   Node-API views over the matching core domains
  *.js                   The loader and its convenience protocols
  tests/                 The mirror of both, file for file
docs/                    The MkDocs site sources
scripts/                 Documentation and interoperability checkers
```

The repository root owns the workspace manifest, the shared dependency pins, and
the shared lints. Repository-wide implementation rules are in
[`AGENTS.md`](AGENTS.md).

## Parsing

The Rust parser accepts one canonical lossless syntax plus familiar SQL, Arrow,
Hive, and Spark spellings:

```rust
use yggdryl::{DataType, Field};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let dtype = DataType::from_str(
    "struct<id:bigint,items:array<struct<sku:string,price:decimal(18,4)>>>",
)?;

let field = Field::from_str(
    r#"field("orders",struct<id:bigint>,nullable=false,metadata={"source":"warehouse"})"#,
)?;

assert_eq!(DataType::from_str(&dtype.to_string())?, dtype);
assert_eq!(Field::from_str(&field.to_string())?, field);
# Ok(())
# }
```

Balanced outer `()`, `[]`, `{}`, single quotes, and double quotes are optional.
Nested separators are depth-aware, quoted Unicode names are retained, and errors
report their byte position and parsing context.

## Records over a handle

The record surface is one streaming read and three explicit write intents:
`IOMedia::read_arrow_reader` returns an `arrow::BatchReader`, while
`IOMedia::overwrite_arrow_reader`, `IOMedia::append_arrow_reader`, and
`IOMedia::merge_arrow_reader` consume one. The encoding comes from the handle's
media type rather than an argument, `options.field` selects and casts in one
pass, and a handle addressing a folder reads and writes across the partitions
beneath it. The canonical signatures and intent rules live on the
[records page](docs/holder/index.md#records).

```rust
use yggdryl::IOMedia;
use yggdryl::holder::Buffer;
use yggdryl::MimeType;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
// A resource that does not exist yet holds no batches rather than failing.
let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
assert_eq!(empty.read_arrow_reader(&empty.record_options()?)?.count(), 0);
# Ok(())
# }
```

The Python package exchanges records as `pyarrow.RecordBatchReader` values over
the Arrow C Stream interface, so a read and a write both stay lazy. The Node
package crosses into Apache Arrow JS through copied IPC, because Arrow JS does
not expose a C Data consumer.

A caller who does not know which of Arrow's four shapes they are holding reaches
for `ArrowScalar` instead. It regroups a pinned one-row array, a column, a held
table, and a one-shot stream behind one value carrying the exact `Field` that
types it, and `IOMedia::read_arrow`/`write_arrow` read and write it
whatever the handle holds - a record encoding as a batch stream, a JSON, JSON
Lines, YAML, or TOML document as the batch its rows parse into. In Python it is
also the one entry point every columnar runtime crosses:
`yggdryl.ArrowScalar.from_py` takes a `pyarrow` container, a pandas or polars
frame or series, a NumPy array, or anything exporting the Arrow C data or
stream protocol, and the declared `Field` casts it in Rust. The
[values page](docs/arrow/values.md) has the shapes and their edges.

## Resource identifiers

URI components are validated owned values. A scheme is always present; authority
and path are always concrete (possibly empty where the syntax allows it), while
query and fragment are optional. Canonical display round-trips without
platform-dependent behavior:

```rust
use yggdryl::{Arn, MediaType, MimeType, Uri, Urn, Url};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let windows = Uri::from_path(r"C:\Users\Ada\orders.parquet")?;
assert_eq!(windows.to_string(), "file:///C:/Users/Ada/orders.parquet");
assert_eq!(windows.extension(), Some("parquet"));
assert_eq!(windows.media_type().base(), &MimeType::PARQUET);

let encoded = MediaType::from_str("orders.csv.gz.zst")?;
assert_eq!(encoded.base(), &MimeType::CSV);
assert_eq!(encoded.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);

let network = Url::from_str("https://example.test/trades/42?view=full")?;
assert_eq!(network.authority().as_str(), "example.test");

let name = Urn::from_str("urn:isbn:9780131103627")?;
assert_eq!(name.namespace(), "isbn");

let resource = Arn::from_str("arn:aws:s3:::market-data/2026/trades.parquet")?;
assert_eq!(resource.service(), "s3");
assert_eq!(resource.bucket(), Some("market-data"));

// A name says where it is: `locator` answers the URL any identifier names.
assert_eq!(resource.locator()?.to_string(), "s3://market-data/2026/trades.parquet");
assert_eq!(
    Urn::from_str("urn:lake:trades:2026:part.parquet")?.locator_path()?.as_str(),
    "lake/trades/2026/part.parquet"
);
# Ok(())
# }
```

`Url`, `Urn` and `Arn` are the three narrowings of one canonical `Uri`, and the
scheme decides which: a location, a name, and the name AWS writes for one of its
resources. `locator()` answers the `Url` any of them names — a location locates
itself, a URN resolves to the path it spells, an Amazon S3 ARN maps to its `s3:`
URL — so everything that takes a location takes a name too. In Python the three
are subclasses of `Uri`, so `Uri(value)` answers the one that value is.

Windows drive paths and UNC paths normalize to `file:` URIs with forward slashes
regardless of the host operating system. Path segments, file names, stems, and
extensions are borrowed views and do not allocate. URI-family mutators validate a
complete replacement before changing the identifier; MIME and media setters use
the same preferred-extension table as inference.

## JSON, TOML, and YAML bytes

```rust
use yggdryl::text::{self, Format};
use yggdryl::Scalar;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let value = Scalar::from_mapping([
    (Scalar::from("id"), Scalar::from(42_i64)),
    (Scalar::from("active"), Scalar::from(true)),
])?;
let bytes = text::into_bytes(&value, Format::Json)?;

assert_eq!(text::from_bytes(&bytes, Format::Json)?, value);
# Ok(())
# }
```

The shared native value preserves bytes, wide integers, exact decimals, the four
temporals, non-finite floats, and arbitrary mapping keys across JSON, TOML, and
YAML.
Slice, reader, writer, JSON Lines, TOML document, and YAML document APIs apply
explicit byte, depth, node, and document limits. See the
[JSON](docs/media/index.md#json), [TOML](docs/media/index.md#toml), and
[YAML](docs/media/index.md#yaml) sections of the media page.

## Native value behavior

- All core values implement equality, total ordering, hashing, deterministic
  stable hashing, canonical display, recursive parsing, and tagged structural
  Serde serialization (`{"type":"int64"}` for a scalar).
- Scalar datatypes remain inline. Nested children and sorted metadata use
  immutable shared storage, so clones do not recursively allocate.
- `Metadata` is a public immutable shared snapshot. Cache-aware mutation stays on
  `Field`; bulk replacement or overlay validates once and publishes one
  deterministic copy-on-write map.
- Consuming `into_arrow*` methods reuse cached projections and move uniquely
  owned state when possible; clone explicitly when the source must be retained.
- Arrow imports and projections preserve every Arrow 59.2 schema datatype, nested
  shared Field reference, dictionary ID and order flag, and temporal or interval
  unit category.

Python adds native comparison, hashing, pickle and JSON support, child-sequence
and metadata-mapping protocols, inferred string and PyArrow conversion, and
cached native fields for ordinary dataclasses through `@scalar` and the static
`Class.into_field()` accessor; `field(value, name=None)` remains a pure builder.
It also provides precise `Annotated` Arrow and Field overrides and byte-first
`yggdryl.json`, `yggdryl.toml`, and `yggdryl.yaml` modules. JavaScript provides
the equivalent value protocols plus Buffer-first codecs and safe, explicit
class registries. The URI family wrappers expose the same canonical components
and resource-path views in both languages.

## Build and test

Default and schema-only core builds support Rust 1.85. The optional `iceberg`
feature and both bindings require Rust 1.94 because they include official
Iceberg 0.10.1. Root Cargo commands select only the core; CI checks its default
surface and the all-feature workspace separately.

```console
cargo fmt --all -- --check
cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings
cargo test -p yggdryl --all-targets
cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings
cargo test -p yggdryl --all-targets --all-features
cargo check -p yggdryl --profile bench --benches --all-features
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

Binding-specific commands are documented in [`rust/README.md`](rust/README.md).
Yggdryl is licensed under the [Apache License 2.0](LICENSE).
