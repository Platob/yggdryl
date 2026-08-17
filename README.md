# Yggdryl

Yggdryl is a focused Rust core for Arrow-native schemas, validated resource
identifiers, byte-oriented structured text, and record I/O. `DataType`, `Field`,
immutable `Metadata`, `MimeType`, `MediaType`, the unified `TimeUnit`, `Scheme`,
`Uri`, `Url`, `Urn`, and the codec `Value` own parsing, validation, comparison,
hashing, and serialization in one place. Python and JavaScript are runtime views
of those native values; neither maintains a parallel schema or codec model.

A struct `Field` is the schema. There is no separate record or schema type: a
non-null `Struct` field describes rows, and a row is one ordered
`Value::Sequence` with a value per child field.

Query execution, network clients, and transport protocols are outside the
project's scope.

## Documentation

Start with the [Yggdryl documentation](https://platob.github.io/yggdryl/) for
copyable Rust, Python, and JavaScript examples. The same pages live in
[`docs/`](docs/index.md), including the
[getting-started guide](docs/getting-started.md) and the
[architecture reference](docs/architecture.md). One page documents one core
module, so the site tree and the source tree are the same tree:

| Area | Pages |
| --- | --- |
| Schema | [enums](docs/core/enums.md), [datatype](docs/core/datatype.md), [field](docs/core/field.md), [arrow](docs/core/arrow.md) |
| Storage | [io](docs/core/io.md), [generic](docs/core/generic.md), [local](docs/core/local.md) |
| Content codings | [gzip](docs/core/gzip.md), [zlib](docs/core/zlib.md), [zstd](docs/core/zstd.md) |
| Record encodings | [ipc](docs/core/ipc.md), [parquet](docs/core/parquet.md) |
| Table format | [iceberg](docs/core/iceberg.md) |
| Identifiers | [uri](docs/core/uri.md) |
| Structured text | [text](docs/core/text.md), [json](docs/core/json.md), [yaml](docs/core/yaml.md), [toml](docs/core/toml.md) |
| Extensions | [Python](docs/extensions/python.md), [JavaScript](docs/extensions/javascript.md) |

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
  src/datatype/          Categorized datatype implementation
  src/field/             Field state, Arrow projection, casting, parsing, diffing
  src/enums/             Shared value vocabularies: units, schemes, MIME values
  src/metadata.rs        Immutable shared metadata value
  src/arrow/             Arrow scalars, arrays, batches, and IPC readers/writers
  src/io/                The IOBase storage trait, Buffer, and Coded
  src/generic/           Holder, Media, RecordOptions, and the shared Value
  src/local/             Local Path, Folder, and memory-mapped File
  src/{gzip,zlib,zstd}/  Content codings, whole-buffer and streaming
  src/{ipc,parquet}/     Record encodings over any handle
  src/iceberg/           Apache Iceberg tables over one container handle
  src/uri.rs             Identifier domain
  src/text/              Shared value, format dispatch, limits, byte positions
  src/{json,yaml,toml}/  Format-specific parsers, streams, and emitters
  tests/                 Edge tests, categorized like the source
  benchmarks/            Criterion targets, categorized like the source
python/                  The Python extension
  src/                   PyO3 views over the matching core domains
  yggdryl/               The Python package, including records and annotations
node/                    The JavaScript extension
  src/                   Node-API views over the matching core domains
  *.js                   The loader and its convenience protocols
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
let data_type = DataType::from_str(
    "struct<id:bigint,items:array<struct<sku:string,price:decimal(18,4)>>>",
)?;

let field = Field::from_str(
    r#"field("orders",struct<id:bigint>,nullable=false,metadata={"source":"warehouse"})"#,
)?;

assert_eq!(DataType::from_str(&data_type.to_string())?, data_type);
assert_eq!(Field::from_str(&field.to_string())?, field);
# Ok(())
# }
```

Balanced outer `()`, `[]`, `{}`, single quotes, and double quotes are optional.
Nested separators are depth-aware, quoted Unicode names are retained, and errors
report their byte position and parsing context.

## Records over a handle

The record surface is exactly three methods, and streaming is the only shape
they have: `IOBase::read_arrow_batch_reader` returns an `arrow::BatchReader`,
`IOBase::write_arrow_batch_reader` replaces or merges by a match key, and
`IOBase::append_arrow_batch_reader` adds after what is there. The encoding comes
from the handle's media type rather than an argument, a declared schema selects
and casts in one pass, and a handle addressing a folder reads and writes across
the partitions beneath it.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::MimeType;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
// A resource that does not exist yet holds no batches rather than failing.
let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
assert_eq!(empty.read_arrow_batch_reader(&empty.record_options()?)?.count(), 0);
# Ok(())
# }
```

The Python package exchanges records as `pyarrow.RecordBatchReader` values over
the Arrow C Stream interface, so a read and a write both stay lazy. The Node
package crosses into Apache Arrow JS through copied IPC, because Arrow JS does
not expose a C Data consumer.

## Resource identifiers

URI components are validated owned values. A scheme is always present; authority
and path are always concrete (possibly empty where the syntax allows it), while
query and fragment are optional. Canonical display round-trips without
platform-dependent behavior:

```rust
use yggdryl::{MediaType, MimeType, Uri, Urn, Url};

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
# Ok(())
# }
```

Windows drive paths and UNC paths normalize to `file:` URIs with forward slashes
regardless of the host operating system. Path segments, file names, stems, and
extensions are borrowed views and do not allocate. URI-family mutators validate a
complete replacement before changing the identifier; MIME and media setters use
the same preferred-extension table as inference.

## JSON, TOML, and YAML bytes

```rust
use yggdryl::text::{self, Format};
use yggdryl::Value;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let value = Value::from_mapping([
    (Value::from("id"), Value::from(42_i64)),
    (Value::from("active"), Value::from(true)),
])?;
let bytes = text::to_vec(&value, Format::Json)?;

assert_eq!(text::from_slice(&bytes, Format::Json)?, value);
# Ok(())
# }
```

The shared native value preserves bytes, wide integers, exact decimals, the four
temporals, non-finite floats, and arbitrary mapping keys across JSON, TOML, and
YAML.
Slice, reader, writer, JSON Lines, TOML document, and YAML document APIs apply
explicit byte, depth, node, and document limits. See the
[shared text](docs/core/text.md), [JSON](docs/core/json.md),
[TOML](docs/core/toml.md), and [YAML](docs/core/yaml.md) guides.

## Native value behavior

- All core values implement equality, total ordering, hashing, deterministic
  stable hashing, canonical display, recursive parsing, and tagged structural
  Serde serialization (`{"type":"int64"}` for a scalar).
- Scalar datatypes remain inline. Nested children and sorted metadata use
  immutable shared storage, so clones do not recursively allocate.
- `Metadata` is a public immutable shared snapshot. Cache-aware mutation stays on
  `Field`; bulk replacement or overlay validates once and publishes one
  deterministic copy-on-write map.
- Borrowed `to_arrow*` methods reuse cached projections. Consuming `into_arrow*`
  methods move uniquely owned state when possible.
- Arrow imports and projections preserve every Arrow 59.2 schema datatype, nested
  shared Field reference, dictionary ID and order flag, and temporal or interval
  unit category.

Python adds native comparison, hashing, pickle and JSON support, child-sequence
and metadata-mapping protocols, inferred string and PyArrow conversion, cached
dataclass schemas through `yggdryl.records`, precise `Annotated` Arrow and Field
overrides, and byte-first `yggdryl.json`, `yggdryl.toml`, and `yggdryl.yaml`
modules. JavaScript provides the equivalent value protocols plus Buffer-first
codecs and safe, explicit class registries. The URI family wrappers expose the
same canonical components and resource-path views in both languages.

## Build and test

The core supports Rust 1.85. Building either extension requires the newer
toolchain its binding dependencies need. The Parquet encoding and the Iceberg
table format over it are behind non-default features, so the test and Clippy
passes run twice: once with default features and once with both enabled.

```console
cargo fmt --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg" -- -D warnings
cargo test --manifest-path rust/Cargo.toml --features "parquet iceberg"
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

Binding-specific commands are documented in [`rust/README.md`](rust/README.md).
Yggdryl is licensed under the [Apache License 2.0](LICENSE).
