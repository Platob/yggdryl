# Generic enums

One enum per contract, so a caller can hold *some* handle, *some* coding, *some* encoding, or *some* settings as a concrete, matchable value.

!!! note "Rust only"
    The bindings hold one handle class and one record settings value rather
    than the enums behind them; `RecordOptions` is the one section below that
    crosses, and it says so. `TypedValue` is Rust-only too.

```rust
use yggdryl::generic::Holder;
use yggdryl::io::{Buffer, IOBase};

// A value that could have been any handle. The calls do not change.
let mut handle = Holder::buffer(Buffer::new());
handle.write_all_bytes(b"AAPL,1\n")?;

assert_eq!(handle.read_all()?, b"AAPL,1\n");
assert_eq!(handle.kind(), yggdryl::IOKind::Memory);
```

A trait says what an implementation must do; the enum beside it says which implementations exist. The two are not interchangeable: `Box<dyn IOBase>` erases the concrete type, and [`IOBase::parent`, `child_by`, and `ls`](io.md) have to return *some* handle that a caller can still match on. Their signatures name `Holder`, so the enum has to be sized, `Send`, and a full implementation of the contract itself.

That last part is what makes the enums invisible in use. Each one delegates every method of its contract to the variant it holds, so code written against the enum behaves exactly as code written against the implementation would - and a variant is still there to match when the concrete type matters.

## Holder: every storage handle

`Holder` names the four [`IOBase`](io.md) implementations the core ships: `Buffer`, `Folder`, `Path`, and `File`.

```rust
use yggdryl::generic::Holder;

// An existing directory is a folder; anything else is a mapped file.
let directory = Holder::local(std::env::temp_dir())?;
assert!(matches!(directory, Holder::Folder(_)));

let missing = Holder::local(std::env::temp_dir().join("yggdryl-generic-doc.bin"))?;
assert!(matches!(missing, Holder::File(_)));
```

`Holder::local` is the only constructor that inspects the filesystem, and it does so once, to pick a role. `Holder::buffer`, `Holder::folder`, and `Holder::file` commit to a role without touching anything, which is what keeps construction lazy. None of them produce `Holder::Path`: that variant holds [`local::Path`](local.md), the role that has not resolved to a file or a folder yet.

Walking a tree stays in one type, because every hierarchy accessor returns `Holder` too.

```rust
use yggdryl::generic::Holder;
use yggdryl::io::IOBase;

let root = Holder::folder(std::env::temp_dir())?;
assert!(root.is_container());

// A child need not exist. Naming one yields a leaf handle, and nothing is created.
let leaf = root.child_by("yggdryl-generic-child.bin")?;
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
assert_eq!(handle.read_all()?, b"symbol,price\nAAPL,1\nAAPL,2\n");
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

`Media` is to a record encoding what `Holder` is to a handle. `Media::open` reads the handle's declared media type and binds the implementation it names - [`ipc`](ipc.md) for Arrow IPC, [`parquet`](parquet.md) for Parquet. Nothing is read to decide.

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
```

Choosing the encoding is the only thing that changes. Every variant answers the same questions - what is the schema, what are the batches, what are the bytes - and both directions stream, through [`arrow::BatchReader`](arrow.md): `Media::read_batch_reader` returns one and `Media::write_batch_reader` consumes one. `Media` is the encoding bound to a handle; the three methods a caller reaches for on a bare handle are [io.md](io.md)'s `read_arrow_batch_reader`, `write_arrow_batch_reader`, and `append_arrow_batch_reader`.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::generic::{Holder, Media};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let url = Url::from_str("file:///trades.arrows")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
let mut media = Media::open(handle)?.with_schema(schema.clone());

media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;
assert_eq!(media.read_batch_reader(None)?.count(), 1);
assert_eq!(media.schema()?, schema);

// A Media is also the bytes it encodes: an Arrow IPC stream opens with its
// continuation marker.
assert_eq!(media.read_range(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
```

The content coding is the handle's business, not the encoding's. A name that declares both gives the same calls and different bytes underneath.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::generic::{Holder, Media};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![9]))],
)?;

let url = Url::from_str("file:///trades.arrows.gz")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
let mut media = Media::open(handle)?.with_schema(schema.clone());

media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;
assert_eq!(media.read_batch_reader(None)?.count(), 1);

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

=== "Rust"

    ```rust
    use yggdryl::generic::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_schema(schema.clone())
        .with_batch_size(1024);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.schema(), Some(&schema));
    assert_eq!(options.batch_size(), Some(1024));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import RecordOptions

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The media type names the encoding, so there is no format argument.
    options = RecordOptions("trades.parquet")
    options.schema = schema
    options.batch_size = 1024

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.schema is not None
    assert options.batch_size == 1024

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
      .withSchema(schema)
      .withBatchSize(1024)

    assert.equal(String(options.mimeType), 'application/vnd.apache.parquet')
    assert.ok(options.schema.equals(schema))
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
    .with_schema(declared.clone())
    .with_select_by_names(["price"]);

// One call is the whole pipeline: the declared cast, then the selection.
// Passing a stored root as the second argument adds the completion layer.
let batch = RecordBatch::new_empty(yggdryl::arrow::schema_from_field(&declared)?);
let cast = options.cast_arrow_batch(batch, None)?;
assert_eq!(cast.num_columns(), 1);
```

There is no shared settings struct threaded through the encodings. Each one stores those five settings as its own flat public fields and implements `IORecordOptions` over them, so a concrete options value takes the same builders the enum does and converts into it. A setting an encoding has no use for is still there and still ignored: [`ParquetOptions::level`](parquet.md) is unused, because Parquet compresses pages inside the file and an outer content coding would produce something no Parquet reader can open.

```rust
use yggdryl::generic::{IORecordOptions, RecordOptions};
use yggdryl::ipc::IpcOptions;
use yggdryl::MimeType;

let options: RecordOptions = IpcOptions::new().with_root_name("trade").with_safe(false).into();

assert_eq!(options.mime_type(), MimeType::ARROW_STREAM);
assert_eq!(options.root_name(), "trade");
assert!(!options.safe());
```

A schema is the one setting with no default. `require_schema` is what a write calls, and it fails by naming the builder that supplies one rather than inventing a schema from the first batch.

```rust
use yggdryl::generic::{IORecordOptions, RecordOptions};
use yggdryl::MimeType;

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?;
assert!(options.schema().is_none());

let message = options.require_schema().unwrap_err().to_string();
assert!(message.contains("with_schema"), "{message}");
```

Content codings are ignored when deriving options: `for_media_type` looks only at the base type, because the coding belongs to the handle. This is the same derivation [`IOBase::record_options`](io.md) performs, which is how a record call on a bare handle knows its encoding without a format argument.

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

The markers are the same family a [typed field](field.md) uses, so a value and a field spell one datatype the same way. Both are covered in full under [structured text](text.md#a-typed-value-per-datatype). Behind the default `arrow` feature the pairing is also the one scalar Arrow projection - `to_arrow_array` materializes one row and `from_arrow_array` decodes one back - documented with the rest of the array boundary in [arrow.md](arrow.md).

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/generic-rust.ipynb){ download },
[Python](notebooks/generic-python.ipynb){ download },
[JavaScript](notebooks/generic-javascript.ipynb){ download }.

<!-- /notebooks -->
