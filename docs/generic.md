# Generic enums

One enum per contract, so a caller can hold *some* handle, *some* coding, *some* encoding, or *some* settings as a concrete, matchable value.

!!! note "Mostly Rust"
    The bindings hold one handle class and one record settings value rather
    than the enums behind them; `RecordOptions` crosses, and the [static enum
    vocabularies](enums.md#listing-the-vocabularies) cross as `yggdryl.enums`
    in Python and the `enums` export in JavaScript. `TypedValue` and the
    `wkb` reader are Rust-only; a geospatial value crosses the bindings as
    its plain WKB bytes.

```rust
use yggdryl::generic::Holder;
use yggdryl::io::{Buffer, IOBase};

// A value that could have been any handle. The calls do not change.
let mut handle = Holder::buffer(Buffer::new());
handle.write_all_bytes(b"AAPL,1\n")?;

assert_eq!(handle.read_all_bytes()?, b"AAPL,1\n");
assert_eq!(handle.kind(), yggdryl::IOKind::Memory);
```

A trait says what an implementation must do; the enum beside it says which implementations exist. The two are not interchangeable: `Box<dyn IOBase>` erases the concrete type, and [`IOBase::parent`, `child_by_path`, and `ls`](io.md) have to return *some* handle that a caller can still match on. Their signatures name `Holder`, so the enum has to be sized, `Send`, and a full implementation of the contract itself.

That last part is what makes the enums invisible in use. Each one delegates every method of its contract to the variant it holds, so code written against the enum behaves exactly as code written against the implementation would - and a variant is still there to match when the concrete type matters.

## Holder: every storage handle

`Holder` names every [`IOBase`](io.md) implementation the core ships: the local `Buffer`, `Folder`,
`Path`, and `File`, the [`arrowfs`](arrowfs.md) trio, and the generic `Buffered`, `Text`, and `Media`
wrappers. `into_text`, `buffered`, and `into_media` are idempotent, so a dynamic caller can request
the optimized view without stacking wrappers.

```rust
use yggdryl::generic::Holder;

// Generic construction records the location without probing its role.
let directory = Holder::local(std::env::temp_dir())?;
assert!(matches!(directory, Holder::Path(_)));

let missing = Holder::local(std::env::temp_dir().join("yggdryl-generic-doc.bin"))?;
assert!(matches!(missing, Holder::Path(_)));
```

`Holder::local` returns `Holder::Path`, the unresolved [`local::Path`](local.md)
role, so construction performs no filesystem call. `Holder::buffer`,
`Holder::folder`, and `Holder::file` commit to a role explicitly and also touch
nothing. The generic path resolves through the matching specialized handle
only when an operation needs to know what is there.

`Holder::open` first promotes IPC, Parquet, Avro, or plain text into its inferred media wrapper,
then opens it. This keeps schema, footer, and dimension caches behind one generic handle—the exact
route Python and JavaScript scopes use. JSON, directories, and unknown byte media remain raw.

Walking a tree stays in one type, because every hierarchy accessor returns `Holder` too.

```rust
use yggdryl::generic::Holder;
use yggdryl::io::IOBase;

let root = Holder::folder(std::env::temp_dir())?;
assert!(root.is_container());

// A child need not exist. Naming one yields a leaf handle, and nothing is created.
let leaf = root.child_by_path("yggdryl-generic-child.bin")?;
assert!(matches!(leaf, Holder::File(_)));
assert!(!leaf.is_container());
assert_eq!(leaf.size(), 0);
```

## Codec: a coding over a handle

[`yggdryl::Codec`](enums.md) says *which* content coding a payload uses. `generic::Codec` is that coding applied to a handle: reading through one decompresses, writing through one compresses, and everything downstream sees plain bytes.

`Codec::infer` takes the coding from the handle's own media type, so a name is all the configuration there is.

```rust
use yggdryl::generic::Codec;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::Url;

let named = Buffer::new().with_media_type(Url::from_str("file:///trades.csv.zst")?.media_type());
let mut handle = Codec::infer(named);
assert_eq!(handle.codec(), yggdryl::Codec::Zstd);

handle.write_all_bytes(b"symbol,price\nAAPL,1\nAAPL,2\n")?;
handle.flush()?;

// The coded handle reads plain bytes; the handle underneath holds the frame.
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\nAAPL,2\n");
assert_ne!(handle.handle().as_slice(), b"symbol,price\nAAPL,1\nAAPL,2\n");
```

The two `Codec` types share a name, so one of them is written out in full wherever both appear. Name the coding yourself with `Codec::wrap` when the handle's media type does not carry it.

```rust
use yggdryl::generic::Codec;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::Level;

let mut handle = Codec::wrap(Buffer::new(), yggdryl::Codec::Gzip).with_level(Level::BEST);
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;

// into_handle publishes the pending write, then gives back the compressed bytes.
let inner = handle.into_handle()?;
assert_eq!(yggdryl::gzip::load(inner.as_slice())?, b"symbol,price\nAAPL,1\n");
```

There are four variants for five codings. Raw DEFLATE carries no framing to detect, so it has no transparent handle of its own and wraps as [`Zlib`](zlib.md), the framed form of the same algorithm.

```rust
use yggdryl::generic::Codec;
use yggdryl::io::Buffer;

let handle = Codec::wrap(Buffer::new(), yggdryl::Codec::Deflate);
assert_eq!(handle.codec(), yggdryl::Codec::Zlib);
```

`Codec<H>` is generic over the handle it wraps, unlike the other three enums here: it composes over anything implementing `IOBase`, including a `Holder` or another coded handle. Levels reach [`gzip`](gzip.md), [`zlib`](zlib.md), and [`zstd`](zstd.md) and are ignored by `Identity`.

## Media: a record encoding over a handle

`Media` is to a record encoding what `Holder` is to a handle. `Media::open` reads the handle's declared media type and binds the implementation it names - [`ipc`](ipc.md) for Arrow IPC, [`parquet`](parquet.md) for Parquet, [`text`](text.md#text-media-and-arrow-batches) for plain text read and written as lines. Nothing is read to decide.

```rust
use yggdryl::generic::{Holder, Media};
use yggdryl::io::Buffer;
use yggdryl::Url;

fn named(name: &str) -> Result<Holder, Box<dyn std::error::Error>> {
    let url = Url::from_str(&format!("file:///{name}"))?;
    Ok(Holder::buffer(Buffer::new().with_media_type(url.media_type())))
}

assert!(matches!(Media::open(named("trades.arrows")?)?, Media::Ipc(_)));
assert!(matches!(Media::open(named("trades.parquet")?)?, Media::Parquet(_)));
assert!(matches!(Media::open(named("app.log")?)?, Media::Text(_)));
```

Choosing the encoding is the only thing that changes. Every variant implements
`IOMedia`: `record_options` returns its held defaults, `read_arrow_field` and
`read_arrow_reader` answer its shape and batches, and the three explicit write
methods consume an [`arrow::BatchReader`](arrow.md). Their signatures and
validation rules are documented once in
[io.md](io.md#canonical-record-write-signatures).

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::generic::{Holder, Media};
use yggdryl::io::{Buffer, IOBase, IOMedia};
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let url = Url::from_str("file:///trades.arrows")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
let mut media = Media::open(handle)?.with_field(schema.clone());
let options = media.record_options()?;

media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
assert_eq!(media.read_arrow_field(&options)?, schema);

// A Media is also the bytes it encodes: an Arrow IPC stream opens with its
// continuation marker.
assert_eq!(media.read_range(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
```

### Measured generic media redirection

`io_write_stateful/media_ipc` exercises the generic enum over its IPC variant with the same
4,096-row, four-column fixture as the concrete media pages. Criterion prepares the stored side for
append and keyed merge outside the timer, so the result covers the generic redirection and the
selected operation, not fixture construction.

| operation through `Media::Ipc` | estimate | throughput |
| --- | ---: | ---: |
| overwrite | 82.2 us | 49.8M rows/s |
| append | 424 us | 9.67M rows/s |
| keyed merge (upsert) | 6.41 ms | 639k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them with
`cargo bench -p yggdryl --bench io --all-features -- io_write_stateful/media_ipc`. Sub-millisecond
point estimates include allocator variance and are regression anchors, not a claim that enum
dispatch makes encoding faster; the enum redirects to the same IPC implementation.

The content coding is the handle's business, not the encoding's. A name that declares both gives the same calls and different bytes underneath.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::generic::{Holder, Media};
use yggdryl::io::{Buffer, IOBase, IOMedia};
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![9]))],
)?;

let url = Url::from_str("file:///trades.arrows.gz")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
let mut media = Media::open(handle)?.with_field(schema.clone());
let options = media.record_options()?;

media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(media.read_arrow_reader(&options)?.count(), 1);

// Still an Arrow IPC stream, now behind gzip framing.
assert_eq!(media.read_range(0, 2)?, [0x1F, 0x8B]);
```

An encoding with no implementation in this build is reported, never guessed at. The error names the media type that was found and the ones that would have worked.

```rust
use yggdryl::generic::{Holder, Media};
use yggdryl::io::Buffer;
use yggdryl::Url;

let url = Url::from_str("file:///trades.csv")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));

let message = Media::open(handle).unwrap_err().to_string();
assert!(message.contains("text/csv"), "{message}");
```

`Media::ipc` and `Media::parquet` name a variant directly when the encoding is already known, and `Media::open_as` takes an explicit `MimeType` when the handle's own name cannot be trusted.

## RecordOptions: every encoding's settings

!!! note "All three"
    Python and JavaScript expose this as `RecordOptions`, derived from a media
    type and carrying every encoding's settings on one value; the encoding-
    specific structs behind it stay in Rust.

Reading rows out of an Arrow IPC stream and out of a Parquet file need the same handful of answers: what schema, what to call an inferred root, how strict a cast may be, how many rows per batch, how hard to compress. `IORecordOptions` is that shared surface; `RecordOptions` is the enum naming every encoding's options.

`RecordOptions` is also a complete Rust value: it implements `Clone`, `Eq`,
`Ord`, and `Hash`, including the encoding variant in its identity.
`stable_hash()` is deterministic across runs and redirects to that variant's
full configuration. The `lines_identity/stable_hash/record_options` Criterion
case measures this path with setup outside the timed loop.

=== "Rust"

    ```rust
    use yggdryl::generic::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_field(schema.clone())
        .with_batch_size(1024);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.field(), Some(&schema));
    assert_eq!(options.batch_size(), Some(1024));
    assert_eq!(options.stable_hash(), options.clone().stable_hash());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import RecordOptions

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The media type names the encoding, so there is no format argument.
    options = RecordOptions("trades.parquet")
    options.field = schema
    options.batch_size = 1024
    options.commit_row_size = 10_000

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.field is not None
    assert options.batch_size == 1024
    assert options.commit_row_size == 10_000

    # A setting one encoding has reads as None on an encoding that has none.
    assert options.max_row_group_size == 1_048_576
    assert RecordOptions("trades.arrows").max_row_group_size is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const options = RecordOptions.from('trades.parquet')
      .withField(schema)
      .withBatchSize(1024)

    assert.equal(String(options.mimeType), 'application/vnd.apache.parquet')
    assert.ok(options.field.equals(schema))
    assert.equal(options.batchSize, 1024)

    // A setting one encoding has reads as null on an encoding that has none.
    assert.equal(options.maxRowGroupSize, 1_048_576)
    assert.equal(RecordOptions.from('trades.arrows').maxRowGroupSize, null)
    ```

The options are also where option-driven casting is *defined*, once. `cast_arrow_batch` - and its
streaming sibling `cast_arrow_reader` - applies three layers in order: the declared schema says
what the rows are meant to be, `select_by_names` narrows and orders the columns, and the optional
`existing` root - a holder's stored shape - is what the rows are finally cast onto, always safely,
so a value that will not convert into a stored column becomes null rather than redefining that
column for every reader. Every write path routes through this one definition, which is why a
declared schema, a selection, and a stored shape can never disagree about what a cast means.

```rust
use arrow_array::RecordBatch;
use yggdryl::generic::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, MimeType};

let declared = DataType::from_fields([
    DataType::Utf8.required_field("symbol"),
    DataType::Int64.required_field("price"),
])?
.required_field("row");

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
    .with_field(declared.clone())
    .with_select_by_names(["price"]);

// One call is the whole pipeline: the declared cast, then the selection.
// Passing a stored root as the second argument adds the completion layer.
let batch = RecordBatch::new_empty(declared.into_arrow_schema()?);
let cast = options.cast_arrow_batch(batch, None)?;
assert_eq!(cast.num_columns(), 1);
```

There is no shared settings struct threaded through the encodings. Each one stores the shared settings as its own flat public fields and implements `IORecordOptions` over them, so a concrete options value takes the same builders the enum does and converts into it. `commit_row_size` is the optional publication cadence shared by every encoding: unset publishes once, while non-zero `N` publishes complete `N`-row prefixes and the final remainder. A setting an encoding has no use for is still there and still ignored: [`ParquetOptions::level`](parquet.md) is unused, because Parquet compresses pages inside the file and an outer content coding would produce something no Parquet reader can open.

```rust
use yggdryl::generic::{IORecordOptions, RecordOptions};
use yggdryl::ipc::IpcOptions;
use yggdryl::MimeType;

let options: RecordOptions = IpcOptions::new()
    .with_root_name("trade")
    .with_safe(false)
    .with_commit_row_size(10_000)
    .into();

assert_eq!(options.mime_type(), MimeType::ARROW_STREAM);
assert_eq!(options.root_name(), "trade");
assert!(!options.safe());
assert_eq!(options.commit_row_size(), Some(10_000));
```

A field is the one setting with no default. `require_field` is what a write calls, and it fails by naming the builder that supplies one rather than inventing a schema from the first batch.

```rust
use yggdryl::generic::{IORecordOptions, RecordOptions};
use yggdryl::MimeType;

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?;
assert!(options.field().is_none());

let message = options.require_field().unwrap_err().to_string();
assert!(message.contains("with_field"), "{message}");
```

Content codings are ignored when deriving options: `for_media_type` looks only at the base type, because the coding belongs to the handle. This is the same derivation [`IOMedia::record_options`](io.md) performs, which is how a record call on a bare handle knows its encoding without a format argument.

## Exact numeric identity

`I256` and the `Float16`, `Float32`, and `Float64` wrappers are immutable,
totally ordered, hashable values. Their `stable_hash` methods use the core's
deterministic structural hasher. Float construction canonicalizes every NaN
payload; signed zero remains distinct, matching equality and total ordering.

```rust
use yggdryl::generic::{Float32, I256};

let wide: I256 = "170141183460469231731687303715884105728".parse()?;
assert_eq!(wide.stable_hash(), wide.stable_hash());

let zero = Float32::from_f32(0.0);
let negative_zero = Float32::from_f32(-0.0);
assert_ne!(zero, negative_zero);
assert_ne!(zero.stable_hash(), negative_zero.stable_hash());
# Ok::<(), yggdryl::Error>(())
```

Criterion covers all four stable-hash entry points in the datatype/value
benchmark group.

## TypedValue: one value and its datatype

This module also owns the value every part of the project speaks, and the pairing beside it. `Value` is documented with the [structured text](text.md) that parses into it; `TypedValue` is one value and one datatype, checked against each other, with one alias per datatype for a caller who knows which is coming.

```rust
use yggdryl::generic::{Int64Value, TypedValue};
use yggdryl::{DataType, Value};

let price = TypedValue::from_parts(DataType::Int64, Value::from(7_i64))?;
assert_eq!(price.data_type(), &DataType::Int64);

// The same pairing, with the datatype fixed at compile time.
let typed: Int64Value = price.try_into_typed()?;
assert_eq!(typed.value(), &Value::I64(7));
assert!(Int64Value::new(Value::from("seven")).is_err());
```

When no schema was supplied, `Value` can expose the exact `Field` the core
already inferred. The three names describe the expected shape and the returned
type: a scalar yields `value`, an outer sequence yields its `item`, and named
record rows yield a non-null Struct root named `row`. Empty or positional rows
remain ambiguous and require a declared Field; neither binding reimplements
this inference.

=== "Rust"

    ```rust
    use yggdryl::Value;

    let scalar = Value::from(42_i64).inferred_scalar_field()?;
    let array = Value::from_sequence([Value::from(1_i64), Value::Null]);
    let row = Value::from_record([("id", Value::from(1_i64))])?;
    let rows = Value::from_sequence([row]);

    assert_eq!(scalar.name(), "value");
    assert_eq!(array.inferred_array_field()?.name(), "item");
    assert_eq!(rows.inferred_struct_field()?.name(), "row");
    ```

=== "Python"

    ```python
    from dataclasses import dataclass

    from yggdryl import Value

    @dataclass
    class Row:
        id: int

    assert Value.from_python(42).into_field().name == "value"
    assert Value.from_python([1, None]).into_array_field().name == "item"
    assert Value.from_python([Row(1)]).into_struct_field().name == "row"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Value } = require('yggdryl')

    assert.equal(Value.fromJs(42).intoField().name, 'value')
    assert.equal(Value.fromJs([1, null]).intoArrayField().name, 'item')
    assert.equal(Value.fromJs([{ id: 1 }]).intoStructField().name, 'row')
    ```

The inference itself stays in Rust. A local Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23) measured the scalar, array, and one-record Struct paths at
80.1 ns, 240 ns, and 985 ns respectively. These are Criterion point estimates; regenerate them on
the deployment host with:

```console
cargo bench -p yggdryl --bench datatype --all-features -- "value/infer_.*_field"
```

The markers are the same family a [typed field](field.md) uses, so a value and a field spell one datatype the same way. Both are covered in full under [structured text](text.md#typed-value-families). Behind the default `arrow` feature the pairing is also the one scalar Arrow projection - `into_arrow_array` materializes one row and `from_arrow_array` decodes one back - documented with the rest of the array boundary in [arrow.md](arrow.md).

## The WKB reader

`generic::wkb` is the one Well-Known Binary reader: displaying a geometry column as WKT, casting
it to text, and bounding it for Parquet and Iceberg statistics all need the same decoding, and
none of them needs a geometry engine, so the workspace reads WKB with no dependency and adds no
second implementation anywhere else. A WKT *parser* is deliberately absent: the workspace
displays and bounds geometries; it does not accept text geometry input yet.

```rust
use yggdryl::generic::wkb::{self, Geometry};

// A little-endian XY point: order byte, type code 1, then x and y.
let mut point = vec![1, 1, 0, 0, 0];
point.extend(10.0_f64.to_le_bytes());
point.extend(20.0_f64.to_le_bytes());

let decoded = Geometry::from_slice(&point)?;
assert_eq!(decoded.clone().into_wkt(), "POINT (10 20)");
assert_eq!(decoded.type_id(), 1);
assert!(!decoded.is_empty());

// The free functions answer without materializing the geometry.
assert_eq!(wkb::into_wkt(&point)?, "POINT (10 20)");
assert_eq!(wkb::geometry_type_ids(&point)?, [1]);
let bounds = wkb::bounding_box(&point)?;
assert_eq!((bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax), (10.0, 10.0, 20.0, 20.0));
```

`Geometry::from_slice` decodes one geometry - the seven simple-feature shapes, in either byte
order, with both type-code spellings: the ISO one, where Z, M, and ZM add 1000, 2000, or 3000 to
the base code, and the PostGIS EWKB one, where high bits flag the extra axes and an embedded SRID
that is read past rather than modeled. The whole slice must be one geometry - trailing bytes are
refused - and malformed input errors name their byte position. `POINT EMPTY` has no zero-count
spelling in WKB, so its conventional NaN coordinates read back as the empty point, and emptiness
is a shape (`coordinate: None`) rather than a value to test for.

`wkb::bounding_box` streams coordinates through a min/max fold in one pass - nothing is
materialized per vertex - and an empty geometry yields the fold's identity, which
`BoundingBox::is_empty` names so a statistics writer can skip the box instead of storing it.
`wkb::geometry_type_ids` collects the distinct ISO type codes a payload holds, and `wkb::into_wkt`
spells canonical WKT whose coordinates print the shortest decimal that reads back as the same
double, so the text loses nothing.

```rust
use yggdryl::generic::wkb;

// Truncated input: the error names the byte position.
let error = wkb::bounding_box(&[1, 1, 0, 0, 0]).unwrap_err();
assert!(error.to_string().contains("byte 5"), "{error}");
```

The value these bytes travel in is `Value::Geospatial`: the canonical spelling of one WKB payload
inside the shared [`Value`](text.md) model, which
[geometry and geography columns](datatype.md#variant-geometry-and-geography) read back, which
canonicalization rewrites plain `Value::Bytes` into on the way in, and whose `as_wkb` accessor
reads both spellings so an inbound payload is never rejected for arriving as plain bytes. There is
deliberately no `Value::Variant` beside it: a variant value *is* the self-describing `Value` tree
itself, and its Parquet binary encoding lands with the Iceberg v3 layer. Across Arrow boundaries
the geospatial pair is Binary storage under the community `geoarrow.wkb` extension name, whose
GeoArrow JSON metadata carries the CRS and edge algorithm - GeoArrow's own documentation says the
specification is not finalized, so that mapping is a community choice the workspace may revisit
when it stabilizes.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/generic.ipynb){ download },
[Python](notebooks/python/generic.ipynb){ download },
[JavaScript](notebooks/javascript/generic.ipynb){ download }.

<!-- /notebooks -->
