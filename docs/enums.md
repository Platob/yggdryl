# Shared enums

`yggdryl::enums` holds the small copyable vocabulary every other module reuses to name a type, a representation, a location, or a coding.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    let value = DataType::from_str("int64")?;
    assert_eq!(value.id(), DataTypeId::Int64);
    assert_eq!(value.kind(), DataTypeKind::Integer);
    assert_eq!(value.id().as_str(), "int64");
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    value = DataType("int64")
    assert value.id == "int64"
    assert value.kind == "integer"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const value = new DataType('int64')
    assert.equal(value.id, 'int64')
    assert.equal(value.kind, 'integer')
    ```

Two names describe one type. [`DataTypeId`](datatype.md) is the parameter-free identity of a
variant - 44 of them - and `DataTypeKind` is the family that variant belongs to, 16 of them.
Behavior that is uniform across a family dispatches on the kind instead of re-listing variants.
Python and JavaScript have no separate class for either: both arrive as the canonical lowercase
strings `DataType.id` and `DataType.kind` return.

## Identity carries no parameters

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId};

    let stamp = DataType::from_str("timestamp(us, UTC)")?;
    assert_eq!(stamp.id(), DataTypeId::Timestamp);
    assert_eq!(stamp.to_string(), "timestamp(us,\"UTC\")");
    assert!(DataTypeId::Timestamp.is_parameterized());
    assert!(!DataTypeId::Int32.is_parameterized());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    stamp = DataType("timestamp(us, UTC)")
    assert stamp.id == "timestamp"
    assert str(stamp) == 'timestamp(us,"UTC")'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const stamp = new DataType('timestamp(us, UTC)')
    assert.equal(stamp.id, 'timestamp')
    assert.equal(stamp.toString(), 'timestamp(us,"UTC")')
    ```

The identity of a timestamp is `timestamp`; its unit and timezone live in the
[`DataType`](datatype.md), not in the id. That is what makes the id cheap: it compares and hashes
without touching nested state. `DataTypeId::from_str` accepts only the bare variant name, so
`decimal128(10, 2)` parses through `DataType::from_str` and never through the id.

The predicates over the id itself are Rust-only.

```rust
use yggdryl::{DataTypeId, DataTypeKind};

assert_eq!(DataTypeId::ALL.len(), 45);
assert_eq!(DataTypeKind::ALL.len(), 16);

assert_eq!(DataTypeId::Int32.fixed_byte_width(), Some(4));
assert_eq!(DataTypeId::Utf8.fixed_byte_width(), None);
assert!(DataTypeId::Int32.is_signed_integer() && !DataTypeId::Int32.is_unsigned_integer());

// A wrapper is nested only when the value it encodes is, so it reports neither.
assert!(DataTypeKind::Dictionary.is_wrapper());
assert!(!DataTypeKind::Dictionary.is_nested());
assert!(DataTypeKind::Struct.is_nested());
```

## MIME types

=== "Rust"

    ```rust
    use yggdryl::MimeType;

    let parquet = MimeType::from_extension("parquet")?;
    assert_eq!(parquet, MimeType::PARQUET);
    assert_eq!(parquet.as_str(), "application/vnd.apache.parquet");
    assert_eq!(parquet.top_level(), "application");
    assert!(parquet.is_tabular() && parquet.is_binary());

    let custom = MimeType::from_str("Application/Vnd.Example+JSON")?;
    assert_eq!(custom.as_str(), "application/vnd.example+json");
    assert_eq!(custom.structured_suffix(), Some("json"));
    assert!(!custom.is_known());
    assert!(custom.is_structured());
    ```

=== "Python"

    ```python
    from yggdryl import MimeType

    parquet = MimeType.from_extension("parquet")
    assert parquet == MimeType.PARQUET
    assert str(parquet) == "application/vnd.apache.parquet"
    assert parquet.top_level == "application"
    assert parquet.is_tabular() and parquet.is_binary()

    custom = MimeType("Application/Vnd.Example+JSON")
    assert str(custom) == "application/vnd.example+json"
    assert custom.structured_suffix == "json"
    assert not custom.is_known()
    assert custom.is_structured()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MimeType } = require('yggdryl')

    const parquet = MimeType.fromExtension('parquet')
    assert.ok(parquet.equals(MimeType.PARQUET))
    assert.equal(parquet.toString(), 'application/vnd.apache.parquet')
    assert.equal(parquet.topLevel, 'application')
    assert.ok(parquet.isTabular() && parquet.isBinary())

    const custom = new MimeType('Application/Vnd.Example+JSON')
    assert.equal(custom.toString(), 'application/vnd.example+json')
    assert.equal(custom.structuredSuffix, 'json')
    assert.equal(custom.isKnown(), false)
    assert.equal(custom.isStructured(), true)
    ```

Fifty-six MIME values are constants with no allocation behind them; anything else that satisfies the
RFC restricted-name grammar is accepted and stored once in canonical ASCII lowercase, which is what
`is_known` distinguishes. Parsing takes an extension, a canonical name, or a registered alias, so
`yml`, `text/yaml`, and `application/x-yaml` all land on `MimeType::YAML`.

Parameters are not part of the value. An HTTP header goes through `from_content_type`, which
validates the parameters - quoting, duplicates - and then discards them.

=== "Rust"

    ```rust
    use yggdryl::MimeType;

    assert_eq!(
        MimeType::from_content_type("Application/JSON; charset=\"utf-8\"")?,
        MimeType::JSON
    );
    assert!(MimeType::from_content_type("application/json; charset").is_err());

    // Content codings map both directions, and `identity` is not one of them.
    assert_eq!(MimeType::from_content_coding("x-gzip")?, MimeType::GZIP);
    assert_eq!(MimeType::GZIP.content_coding(), Some("gzip"));
    assert!(MimeType::from_content_coding("identity").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import MimeType

    assert MimeType.from_content_type('Application/JSON; charset="utf-8"') == MimeType.JSON
    with pytest.raises(ValueError):
        MimeType.from_content_type("application/json; charset")

    assert MimeType.from_content_coding("x-gzip") == MimeType.GZIP
    assert MimeType.GZIP.content_coding == "gzip"
    with pytest.raises(ValueError):
        MimeType.from_content_coding("identity")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MimeType } = require('yggdryl')

    assert.ok(
      MimeType.fromContentType('Application/JSON; charset="utf-8"').equals(MimeType.JSON),
    )
    assert.throws(() => MimeType.fromContentType('application/json; charset'))

    assert.ok(MimeType.fromContentCoding('x-gzip').equals(MimeType.GZIP))
    assert.equal(MimeType.GZIP.contentCoding, 'gzip')
    assert.throws(() => MimeType.fromContentCoding('identity'))
    ```

## Media types are a base plus its codings

=== "Rust"

    ```rust
    use yggdryl::{MediaType, MimeType};

    let media = MediaType::from_file_name("trades.json.gz");
    assert_eq!(media.base(), &MimeType::JSON);
    assert_eq!(media.encodings(), &[MimeType::GZIP]);
    assert_eq!(media.encoding(), Some(&MimeType::GZIP));
    assert_eq!(media.extensions().collect::<Vec<_>>(), ["json", "gz"]);
    assert!(media.is_encoded());
    assert_eq!(media.to_string(), "application/json;encodings=application/gzip");
    ```

=== "Python"

    ```python
    from yggdryl import MediaType, MimeType

    media = MediaType.from_file_name("trades.json.gz")
    assert media.base == MimeType.JSON
    assert media.encodings == (MimeType.GZIP,)
    assert media.encoding == MimeType.GZIP
    assert media.extensions == ["json", "gz"]
    assert media.is_encoded()
    assert str(media) == "application/json;encodings=application/gzip"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MediaType, MimeType } = require('yggdryl')

    const media = MediaType.fromFileName('trades.json.gz')
    assert.ok(media.base.equals(MimeType.JSON))
    assert.deepEqual(media.encodings.map((value) => value.toString()), ['application/gzip'])
    assert.ok(media.encoding.equals(MimeType.GZIP))
    assert.deepEqual(media.extensions, ['json', 'gz'])
    assert.equal(media.isEncoded(), true)
    assert.equal(media.toString(), 'application/json;encodings=application/gzip')
    ```

A filename says two things at once - what the payload is, and what was done to it - so `MediaType`
keeps them apart. Inference reads the suffixes left to right, keeps the trailing run of known
codings, and takes the nearest suffix before them as the base. An unrecognized suffix hides
everything to its left, and a name with no usable base falls back to `application/octet-stream`.

Encodings are stored in application order, matching HTTP `Content-Encoding`: the last one listed is
the outermost and the first that a reader must remove.

=== "Rust"

    ```rust
    use yggdryl::{MediaType, MimeType};

    let media = MediaType::from_content_headers(Some("text/csv; charset=utf-8"), Some("gzip"))?;
    assert_eq!(media.base(), &MimeType::CSV);
    assert_eq!(media.encoding(), Some(&MimeType::GZIP));

    // Compound suffixes name both halves at once.
    assert_eq!(
        MediaType::from_extension("tgz"),
        MediaType::from_parts(MimeType::TAR, [MimeType::GZIP])?
    );

    // Only a coding may be pushed; anything else leaves the value untouched.
    let mut stacked = MediaType::from_file_name("events.json");
    assert!(stacked.push_encoding(MimeType::ZIP).is_err());
    stacked.push_encoding(MimeType::ZSTD)?;
    assert_eq!(stacked.extension(), Some("zst"));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import MediaType, MimeType

    media = MediaType.from_content_headers("text/csv; charset=utf-8", "gzip")
    assert media.base == MimeType.CSV
    assert media.encoding == MimeType.GZIP

    assert MediaType.from_extension("tgz") == MediaType.from_parts(MimeType.TAR, [MimeType.GZIP])

    stacked = MediaType.from_file_name("events.json")
    with pytest.raises(ValueError):
        stacked.push_encoding(MimeType.ZIP)
    stacked.push_encoding(MimeType.ZSTD)
    assert stacked.extensions == ["json", "zst"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MediaType, MimeType } = require('yggdryl')

    const media = MediaType.fromContentHeaders('text/csv; charset=utf-8', 'gzip')
    assert.ok(media.base.equals(MimeType.CSV))
    assert.ok(media.encoding.equals(MimeType.GZIP))

    assert.ok(
      MediaType.fromExtension('tgz').equals(
        MediaType.fromParts(MimeType.TAR, [MimeType.GZIP]),
      ),
    )

    const stacked = MediaType.fromFileName('events.json')
    assert.throws(() => stacked.pushEncoding(MimeType.ZIP))
    stacked.pushEncoding(MimeType.ZSTD)
    assert.deepEqual(stacked.extensions, ['json', 'zst'])
    ```

A ZIP archive is rejected because it is a container, not a transparent coding: `is_archive` and
`is_encoding` are different questions, and only the second admits a payload that can be unwrapped
back to the base. Mutation is atomic - a rejected `set_encodings` or `push_encoding` leaves the
value exactly as it was. Python keeps a `MediaType` editable until its first built-in hash, then
locks that wrapper so a dictionary key cannot change equality; copying or unpickling creates an
independent unlocked wrapper. The explicit `stable_hash()` only computes the current deterministic
identity and never locks it.

## Directories and files

`MimeType::DIRECTORY` and `MimeType::FILE`, and the filesystem accessors over them, are Rust-only.

```rust
use yggdryl::MimeType;

assert_eq!(MimeType::DIRECTORY.as_str(), "inode/directory");
assert_eq!(MimeType::FILE.as_str(), "inode/file");
assert!(MimeType::DIRECTORY.is_directory());
assert!(MimeType::FILE.is_filesystem() && !MimeType::FILE.is_directory());
assert!(!MimeType::DIRECTORY.is_io());
assert!(MimeType::FILE.is_io() && MimeType::CSV.is_io());
assert_eq!(MimeType::DIRECTORY.extension(), None);

let directory = std::env::temp_dir();
assert_eq!(MimeType::from_local_path(&directory), MimeType::DIRECTORY);
assert_eq!(MimeType::from_local_path(directory.join("report.csv")), MimeType::CSV);
// A name that says nothing is still known to be a leaf.
assert_eq!(MimeType::from_local_path(directory.join("payload")), MimeType::FILE);
```

`inode/file` says the resource holds bytes, not what is in them; a file whose type is recognized
reports that type instead. The pair exists so a handle can answer "container or leaf" with the same
vocabulary it uses for everything else - which is what [`local::Path`](local.md) reads before
deciding whether to become a `Folder` or a `File`.

`is_io` is derived from that same distinction. Every known or custom content MIME value, including
`inode/file`, can describe an I/O value; only `inode/directory` describes a container instead.
`MediaType::is_io` delegates to its unencoded base because transparent content codings do not turn a
leaf into a directory.

## Content inference from bytes

Magic-byte inference is Rust-only.

```rust
use yggdryl::enums::MAGIC_PROBE_LEN;
use yggdryl::{MediaType, MimeType, gzip, zstd};

assert_eq!(MimeType::from_magic_bytes(b"PAR1"), Some(MimeType::PARQUET));
assert_eq!(MimeType::from_magic_bytes(b"ARROW1\0\0"), Some(MimeType::ARROW_FILE));

// Text has no signature, so it is sniffed structurally and only when unambiguous.
assert_eq!(MimeType::from_bytes(b"  {\"symbol\": \"AAPL\"}"), Some(MimeType::JSON));
assert_eq!(MimeType::from_bytes(b"key = 1"), None);

// Codings are peeled recursively and reported in application order.
let payload = gzip::dump(&zstd::dump(br#"{"symbol":"AAPL"}"#)?)?;
let media = MediaType::from_magic_bytes(&payload).expect("gzip of zstd of json");
assert_eq!(media.base(), &MimeType::JSON);
assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);

assert_eq!(MAGIC_PROBE_LEN, 64);
```

Filename inference answers what a location claims to be; this answers what the payload is, and the
two disagree often enough that a reader holding bytes should prefer the second. Only the first
`MAGIC_PROBE_LEN` bytes are examined and peeling stops after four layers, so an adversarial input
cannot buy unbounded work. A coding whose payload stays unidentifiable is reported as the base,
because the coding is still a fact.

## Schemes

`Scheme` is Rust-only; the bindings take the compatibility targets as strings through
`into_scheme_compat`.

```rust
use yggdryl::Scheme;

assert_eq!(Scheme::HTTPS.default_port(), Some(443));
assert_eq!(Scheme::POSTGRES.default_port(), Some(5432));
assert_eq!(Scheme::S3.default_port(), None);
assert!(Scheme::S3.is_storage() && !Scheme::ICEBERG.is_storage());

assert_eq!(Scheme::COMPATIBILITY_TARGETS.len(), 5);
assert!(Scheme::SPARK.is_compatibility_target());
assert!(Scheme::ICEBERG.is_compatibility_target());
assert!(!Scheme::HTTPS.is_compatibility_target());

// Any RFC-valid scheme parses; only the listed ones are allocation-free.
let custom = Scheme::from_str("Acme+Wire")?;
assert_eq!(custom.as_str(), "acme+wire");
assert!(!custom.is_known() && Scheme::HTTPS.is_known());
```

The scheme does double duty: it names a protocol for [`Uri`](uri.md), and it namespaces metadata
properties on a [`Field`](field.md), which is why `arrow`, `iceberg`, `fix`, and `dtype` sit in the
same list as `https` and `s3`. `default_port` returns `None` for metadata namespaces and for
object-storage protocols with no fixed listening port. Only the five compatibility targets are
accepted by `into_scheme_compat`, and `iceberg` is both: it namespaces
[table-format](iceberg.md) metadata and names the schema subset that format can express.

## Content codings

`Codec` and `Level` are Rust-only.

```rust
use yggdryl::{Codec, Level, Url};

let plain = b"symbol,price\nAAPL,1\n";
let compressed = Codec::Gzip.dump_with_level(plain, Level::BEST)?;
assert_eq!(Codec::Gzip.load(&compressed)?, plain.to_vec());
assert_eq!(Codec::Identity.dump(plain)?, plain.to_vec());

// The coding is recoverable from a filename alone.
let url = Url::from_str("file:///trades.csv.gz")?;
assert_eq!(Codec::from_url(&url), Codec::Gzip);
assert_eq!(Codec::from_mime_type(&yggdryl::MimeType::ZSTD), Codec::Zstd);
assert_eq!(Codec::Gzip.extension(), Some("gz"));
assert!(Codec::Identity.is_identity());
```

`Codec::from_url` is what lets `trades.json.gz` decode without a caller naming the codec: it reads
the media type off the compound filename and takes the outermost encoding. `load`/`dump` handle
whole buffers, while `reader`/`writer` stream - nothing buffers an object to compress it. A
`writer` must be finished with `Encoder::finish`; dropping it omits the trailer and the output is
not a valid member of its format. See [`gzip`](gzip.md), [`zlib`](zlib.md), and [`zstd`](zstd.md)
for the per-format entry points.

`Level` is one 0-to-9 scale mapped onto each codec's native range, so raising compression does not
mean learning three numbering schemes.

```rust
use yggdryl::Level;

assert_eq!(Level::NONE.get(), 0);
assert_eq!(Level::DEFAULT.get(), 6);
assert_eq!(Level::BEST.get(), 9);
assert_eq!(Level::default(), Level::DEFAULT);
// Out-of-range levels clamp rather than fail.
assert_eq!(Level::new(200), Level::BEST);
```

## What a handle addresses

`IOKind` is Rust-only.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::IOKind;

assert_eq!(Buffer::new().kind(), IOKind::Memory);
assert_eq!(IOKind::from_str("directory")?, IOKind::Directory);
assert_eq!(IOKind::default(), IOKind::File);

assert!(IOKind::Memory.is_leaf() && IOKind::File.is_leaf());
assert!(IOKind::Directory.is_container());
assert!(!IOKind::Unknown.is_known());
```

Every backend has the same three roles - bytes with no location, a leaf holding bytes, a container
holding other resources - plus `Unknown`, the honest answer for a location that does not exist yet.
`Unknown` is not an error: reading it yields nothing and writing it creates, which is the laziness
contract [`IOBase`](io.md) is built on. Adding a backend means answering this question, not
inventing new vocabulary.

A table format names three container roles of its own, because answering "directory" about them
loses what a caller most needs to know. A `Table` is one tabular value spread over many files, so it
is read through the record surface rather than by listing it; a `Namespace` holds tables and further
namespaces; a `Catalog` is the warehouse those namespaces live under. All three contain others, so
`is_container` stays the one question a walk asks.

```rust
use yggdryl::IOKind;

for container in [IOKind::Table, IOKind::Namespace, IOKind::Catalog] {
    assert!(container.is_container());
    assert!(!container.is_leaf());
}
assert_eq!(IOKind::from_str("CATALOG")?, IOKind::Catalog);
// The parser names the whole vocabulary when it refuses one.
let refused = IOKind::from_str("warehouse").unwrap_err().to_string();
assert!(refused.contains("namespace"));
```

Storage cannot tell one folder from another, so each role is answered by the value that adds the
framing: [`iceberg::Catalog::kind`](iceberg.md), `iceberg::Namespace::kind`, and - because a table
*is* a handle - `IOBase::kind` on [`iceberg::Table`](iceberg.md). A plain folder handle over the
same location still answers `Directory`, and still reads as the table beneath it.

## Record write intent

`WriteMode` is the required Rust vocabulary used by generic record-write entry points. It chooses
the operation; a schema, row limit, commit cadence, or merge key can refine that operation but can
never choose it implicitly.

```rust
use yggdryl::WriteMode;

assert_eq!(WriteMode::from_str("OVERWRITE")?, WriteMode::Overwrite);
assert_eq!(WriteMode::Append.as_str(), "append");
assert_eq!(WriteMode::Merge.to_string(), "merge");
assert_eq!(
    WriteMode::ALL.map(WriteMode::as_str),
    ["overwrite", "append", "merge"],
);
```

Only those three names are accepted. There is no `write`, `upsert`, default mode, or compatibility
alias: intent must remain visible at every generic call site.

## Time units and union modes

=== "Rust"

    ```rust
    use yggdryl::{DataType, TimeUnit};

    assert_eq!(TimeUnit::from_str("microseconds")?, TimeUnit::Microsecond);
    assert_eq!(TimeUnit::from_str("MICRO SECONDS")?, TimeUnit::Microsecond);
    assert_eq!(TimeUnit::Microsecond.as_str(), "us");

    // The unit picks the physical width.
    assert_eq!(DataType::time(TimeUnit::Second)?.to_string(), "time32(s)");
    assert_eq!(DataType::time(TimeUnit::Microsecond)?.to_string(), "time64(us)");
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    assert str(DataType.time("microseconds")) == "time64(us)"
    assert str(DataType.time("MICRO SECONDS")) == "time64(us)"
    assert str(DataType.time("s")) == "time32(s)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.time('microseconds').toString(), 'time64(us)')
    assert.equal(DataType.time('MICRO SECONDS').toString(), 'time64(us)')
    assert.equal(DataType.time('s').toString(), 'time32(s)')
    ```

Arrow splits temporal resolutions and calendar interval layouts across two enums; `TimeUnit` holds
both and `is_temporal`/`is_interval` tell them apart. Projection is checked, not coerced: an
interval layout cannot become an `arrow_schema::TimeUnit`.

```rust
use yggdryl::TimeUnit;

assert!(TimeUnit::Nanosecond.is_temporal() && !TimeUnit::Nanosecond.is_interval());
assert!(TimeUnit::MonthDayNano.is_interval());
assert_eq!(TimeUnit::MonthDayNano.as_str(), "month_day_nano");

assert_eq!(TimeUnit::Nanosecond.into_arrow_time()?, arrow_schema::TimeUnit::Nanosecond);
assert!(TimeUnit::MonthDayNano.into_arrow_time().is_err());
assert!(TimeUnit::Nanosecond.into_arrow_interval().is_err());
```

Parsing is case-insensitive and accepts the Arrow, SQL, Hive, and Spark spellings - `sec`,
`MILLIS`, `µs`, `YEAR TO MONTH`, `days to seconds` - while `as_str` and `Display` always return the
canonical short form.

`UnionMode` is the last of the set: the physical layout of a tagged union, Rust-only as a value.

```rust
use yggdryl::UnionMode;

assert_eq!(UnionMode::Sparse.as_str(), "sparse");
assert_eq!(UnionMode::Dense.to_string(), "dense");
assert_ne!(UnionMode::Sparse, UnionMode::Dense);
```

`DataType::union` takes the mode explicitly; `DataType::dense_union` - the member-list sugar the
bindings expose as `DataType.variant(fields)` - always builds a dense union. Bare
`DataType::variant` is the self-describing [Variant datatype](datatype.md#variant-geometry-and-geography);
the parenthesis disambiguates.

## Edge algorithms

`EdgeAlgorithm` names how the edge between two geography vertices is interpolated - the vocabulary
Parquet's `GEOGRAPHY` logical type and Iceberg v3 share. A geometry connects vertices with straight
planar lines, so it needs no algorithm, and a
[geography](datatype.md#variant-geometry-and-geography) given none fills `Spherical`, the default
both formats fill.

!!! note "Rust only"
    The bindings take the canonical lowercase name as the `algorithm` string of
    `DataType.geography(crs, algorithm)`; the enum itself does not cross.

```rust
use yggdryl::{DataType, EdgeAlgorithm};

assert_eq!(EdgeAlgorithm::ALL.len(), 5);
assert_eq!(EdgeAlgorithm::default(), EdgeAlgorithm::Spherical);
assert_eq!(EdgeAlgorithm::Vincenty.as_str(), "vincenty");

// Parsing is ASCII case-insensitive; display is the canonical lowercase name.
assert_eq!(EdgeAlgorithm::from_str("KARNEY")?, EdgeAlgorithm::Karney);
assert_eq!(EdgeAlgorithm::Andoyer.to_string(), "andoyer");

// An unknown name reports the input and the whole accepted vocabulary.
let error = EdgeAlgorithm::from_str("euclidean").unwrap_err();
assert!(error.to_string().contains("expected one of spherical"));

// The value lives on a geography datatype and nowhere else.
let vincenty = DataType::geography(None, Some(EdgeAlgorithm::Vincenty))?;
assert_eq!(vincenty.to_string(), "geography(\"OGC:CRS84\",\"vincenty\")");
```

`Spherical` is great-circle edges on a perfect sphere; the other four are geodesic edges on a
spheroid, named for their methods: Vincenty's iterative formulae, the Thomas cubic-series and
Andoyer first-order approximations, and Karney's exact algorithm. The workspace stores and spells
the choice; it does not evaluate geodesics.

## Listing the vocabularies

Pure enums cross the bindings as strings - a datatype id is `"int64"`, a codec is `"gzip"` - and
the vocabularies enumerate what those strings can be. Every listing is unpacked from one native
call, so it can never drift from the Rust constants it mirrors.

=== "Rust"

    ```rust
    use yggdryl::{Codec, DataTypeId, IOKind, TimeUnit, UnionMode};

    // Every core enum publishes its variants in canonical order. Check
    // representatives rather than pinning an extensible vocabulary's length.
    assert!(DataTypeId::ALL.contains(&DataTypeId::Int64));
    assert!(DataTypeId::ALL.contains(&DataTypeId::Struct));
    assert!(DataTypeId::ALL.contains(&DataTypeId::Geography));
    assert_eq!(UnionMode::ALL.map(UnionMode::as_str), ["sparse", "dense"]);
    assert!(TimeUnit::ALL.contains(&TimeUnit::Microsecond));
    assert!(Codec::ALL.contains(&Codec::Gzip));
    assert!(IOKind::ALL.contains(&IOKind::File));
    ```

=== "Python"

    ```python
    from yggdryl import enums

    assert "int64" in enums.DATA_TYPE_IDS
    assert enums.UNION_MODES == ("sparse", "dense")
    assert "us" in enums.TIME_UNITS
    assert "gzip" in enums.CODECS
    assert "file" in enums.IO_KINDS
    assert "arrow" in enums.COMPATIBILITY_SCHEMES
    assert enums.LEVELS["default"] == 6
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { enums } = require('yggdryl')

    assert.ok(enums.dataTypeIds.includes('int64'))
    assert.deepEqual([...enums.unionModes], ['sparse', 'dense'])
    assert.ok(enums.timeUnits.includes('us'))
    assert.ok(enums.codecs.includes('gzip'))
    assert.ok(enums.ioKinds.includes('file'))
    assert.ok(enums.compatibilitySchemes.includes('arrow'))
    assert.equal(enums.levels.default, 6)
    ```

## Timezone

`Timezone` is the one way to name a zone: an alias, a case variant, and a fixed offset all
canonicalize on arrival, and a registered zone can answer what its offset actually was at an
instant.

=== "Rust"

    ```rust
    use yggdryl::Timezone;

    // Two spellings of one zone are one value.
    assert_eq!(Timezone::from_str("Asia/Calcutta")?, Timezone::from_str("Asia/Kolkata")?);
    assert_eq!(Timezone::from_str("US/Eastern")?.as_str(), "America/New_York");
    assert_eq!(Timezone::from_str("Z")?, Timezone::UTC);
    assert_eq!(Timezone::from_str("+0530")?.as_str(), "+05:30");

    // A registered zone knows the rule in force today.
    let new_york = Timezone::from_str("America/New_York")?;
    assert_eq!(new_york.offset_at(1_705_000_000), Some(-5 * 3600));
    assert_eq!(new_york.offset_at(1_720_000_000), Some(-4 * 3600));
    assert_eq!(new_york.abbreviation_at(1_720_000_000), Some("EDT"));
    assert_eq!(new_york.standard_offset(), Some(-5 * 3600));

    // A zone with no known rules answers nothing rather than guessing.
    let custom = Timezone::from_str("Custom/Accepted")?;
    assert!(!custom.is_known());
    assert_eq!(custom.offset_at(0), None);
    ```

=== "Python"

    ```python
    import zoneinfo

    from yggdryl import Timezone

    # A name, an alias, and a zoneinfo all arrive at one value.
    assert Timezone("Asia/Calcutta") == Timezone("Asia/Kolkata")
    assert Timezone(zoneinfo.ZoneInfo("US/Eastern")) == Timezone("America/New_York")
    assert Timezone("Z") == Timezone.UTC

    sydney = Timezone("Australia/Sydney")
    # Sydney's saving period spans the new year, so January is +11.
    assert sydney.offset_at(1_705_000_000) == 11 * 3600
    assert sydney.offset_at(1_720_000_000) == 10 * 3600
    assert sydney.observes_saving()

    # It duck-types as a tzinfo wherever only the offset is needed.
    import datetime
    assert sydney.utcoffset(1_720_000_000) == datetime.timedelta(hours=10)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Timezone } = require('yggdryl')

    // A name, an alias, and a zone read out of Intl all arrive at one value.
    assert.ok(Timezone.from('Asia/Calcutta').equals(Timezone.from('Asia/Kolkata')))
    assert.equal(Timezone.from('US/Eastern').key, 'America/New_York')
    assert.ok(Timezone.from('Z').equals(Timezone.UTC))

    const sydney = Timezone.from('Australia/Sydney')
    // Sydney's saving period spans the new year, so January is +11.
    assert.equal(sydney.offsetAt(1_705_000_000), 11 * 3600)
    assert.equal(sydney.offsetAt(1_720_000_000), 10 * 3600)
    assert.ok(sydney.observesSaving())

    // It reports an offset the way `Date` does wherever that is what is read.
    assert.equal(sydney.getTimezoneOffset(1_720_000_000), -600)
    ```

A datatype retains Arrow's optional timezone spelling:
`DataType::Timestamp(unit, None)` is a naive column. A concrete temporal value
is never ambiguous: `Value::DateTime64(count, unit, timezone)` always carries
a zone, using `Timezone::NAIVE` for a wall-clock reading and a named zone for
an instant.

The registry carries the rules **in force today**, which is what a schema, a partition value, or a
freshly written batch needs. It is deliberately not a history: applying today's rule to a 1975
instant would answer confidently and wrongly. For the same reason a zone whose real rule is
historical, irregular, or politically volatile is left out of the table rather than approximated,
so it still parses and round-trips but reports its offset as unknown. Refusing to answer is
recoverable; a plausible wrong answer is not.

Zone names share their heap allocation with Arrow's own `Arc<str>` in both directions, so importing
a schema and re-exporting it moves no bytes.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/enums.ipynb){ download },
[Python](notebooks/python/enums.ipynb){ download },
[JavaScript](notebooks/javascript/enums.ipynb){ download }.

<!-- /notebooks -->
