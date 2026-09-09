# Media

`Media` binds a handle to the record encoding that its declared media type names.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Media`, `Media::open`, `open_as`, `ipc`, `parquet`, `avro`, `text`, `csv`, `handle`, `into_handle` |
| Variants | `Ipc`, `Parquet`, `Avro`, `Text`, `Csv` |
| Selects on | the handle's declared media type; nothing is read to decide |
| Every variant | implements [`IOMedia`](../holder/iobase/records.md): `record_options`, `read_arrow_field`, `read_arrow_reader`, three write methods |
| Writes take | an [`arrow::BatchReader`](../arrow/readers.md); signatures and validation live in [Records](../holder/iobase/records.md) |
| Settings | one shared [`RecordOptions`](options.md) behind every encoding |
| Plain text | `Media::Text`, retaining [`TextOptions`](text.md); any other handle still reaches rows through `IOMedia` and [`RecordOptions`](options.md) |
| Delimited text | `Media::Csv`, retaining [`CsvOptions`](csv.md) and adding positional row and cell access |
| Content coding | the handle's business, not the encoding's |
| Errors | an encoding with no implementation in this build is reported, never guessed |
| Bindings | Rust: the enum; Python: `yggdryl.media.Media` with `Ipc`, `Parquet`, `Avro` under it and `Text` beside it; JavaScript: one `IOBase` class |

## Use

`Media::open` binds the implementation the name declares. Python applies the same
choice when the handle is built, so `type(handle)` names it; JavaScript has one
`IOBase` class.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::Media;
    use yggdryl::holder::Buffer;
    use yggdryl::Url;

    fn named(name: &str) -> Result<Holder, Box<dyn std::error::Error>> {
        let url = Url::from_str(&format!("file:///{name}"))?;
        Ok(Holder::buffer(Buffer::new().with_media_type(url.media_type())))
    }

    assert!(matches!(Media::open(named("trades.arrows")?)?, Media::Ipc(_)));
    assert!(matches!(Media::open(named("trades.parquet")?)?, Media::Parquet(_)));
    assert!(matches!(Media::open(named("trades.log")?)?, Media::Text(_)));
    assert!(matches!(Media::open(named("trades.csv")?)?, Media::Csv(_)));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.media import Ipc, Parquet, Text

    root = pathlib.Path(tempfile.mkdtemp())

    # Composed at construction, and nothing is read to decide it.
    assert isinstance(IOBase(root / "trades.arrows"), Ipc)
    assert isinstance(IOBase(root / "trades.parquet"), Parquet)
    assert isinstance(IOBase(root / "trades.log"), Text)
    ```

## Pages

| Page | Owns |
| --- | --- |
| [Arrow IPC](ipc.md) | Arrow IPC streams over any handle, schema carriage, the one-stream contract |
| [Apache Parquet](parquet.md) | The Parquet record surface, pushdown, compression, coded-handle refusal |
| [Parquet footer](parquet-footer.md) | Footer metadata, statistics, and the caching `Parquet<H>` wrapper |
| [Apache Avro](avro.md) | Avro as streamed Arrow batches, block options, schema resolution |
| [Plain-text records](text.md) | `TextOptions`, the url/rownum/body schema, autotyping |
| [Delimited-text records](csv.md) | `CsvOptions`, the header and cell schema, positional row and cell access |
| [RecordOptions](options.md) | The declared root, `batch_row_size`, identity, shared by every encoding |
| [Iceberg](iceberg/index.md) | Table anatomy: metadata, snapshots, manifests, partition specs |
| [Iceberg schema](iceberg/schema.md) | Evolution, field ids, `SchemaUpdate`, the type mappings |
| [Iceberg reads](iceberg/read.md) | Scan planning, pushdown, time travel, parallel multi-file reads |
| [Iceberg writes](iceberg/write.md) | The three record methods, size targets, commits, branches and tags |
| [Iceberg catalog](iceberg/catalog.md) | The warehouse over one folder, as namespaces of tables |

## Shared IOMedia calls

Choosing the encoding is the only thing that changes.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::holder::Holder;
    use yggdryl::media::Media;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
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
    assert_eq!(media.read_range_bytes(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
    ```

## Content coding

The handle owns the coding: the same calls, different bytes underneath.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::holder::Holder;
    use yggdryl::media::Media;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
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
    assert_eq!(media.read_range_bytes(0, 2)?, [0x1F, 0x8B]);
    ```

## The handle underneath

`handle` borrows the byte handle the encoding is layered over; `into_handle`
consumes the media and answers it.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::Media;
    use yggdryl::holder::Buffer;
    use yggdryl::Url;

    let url = Url::from_str("file:///trades.arrows")?;
    let media = Media::open(Holder::buffer(Buffer::new().with_media_type(url.media_type())))?;

    assert!(matches!(media.handle(), Holder::Buffer(_)));
    assert!(matches!(media.into_handle(), Holder::Buffer(_)));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.coding import Gzip
    from yggdryl.holder import Path
    from yggdryl.media import Ipc

    root = pathlib.Path(tempfile.mkdtemp())
    assert isinstance(IOBase(root / "trades.arrows").into_handle(), Path)

    # A coded name composes one more layer: the coding goes under the encoding.
    coded = IOBase(root / "trades.arrows.gz")
    assert isinstance(coded, Ipc)
    gzip_handle = coded.into_handle()
    assert isinstance(gzip_handle, Gzip)
    assert isinstance(gzip_handle.into_handle(), Path)
    ```

## Unimplemented encodings

The error names the media type found and the ones that would have worked.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::Media;
    use yggdryl::holder::Buffer;
    use yggdryl::Url;

    let url = Url::from_str("file:///trades.orc")?;
    let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));

    let message = Media::open(handle).unwrap_err().to_string();
    assert!(message.contains("orc"), "{message}");
    ```

## Edges

- An encoding with no implementation (`trades.orc`) on `Media::open` -> error naming the found media type; no encoding is guessed.
- Encoding already known -> `Media::ipc` and `Media::parquet` name a variant directly.
- Handle name not trustworthy -> `Media::open_as` takes an explicit `MimeType`.
- Plain text -> `Media::Text`; delimited text -> `Media::Csv`; any other handle still reaches rows through `IOMedia` and [`RecordOptions`](options.md).
- Python name with no implementation (`trades.orc`) -> nothing is composed and the handle stays a `Path`; only `Media::open` reports it.
- `--lib media::tests` -> the enum's own module only; `media::ipc::tests` needs its own filter.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/media_ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_media.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/media.test.js
    ```

## Performance

`io_write_stateful/media_ipc` drives the enum over its IPC variant with a 4,096-row, four-column fixture. Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23).

| operation through `Media::Ipc` | estimate | throughput |
| --- | ---: | ---: |
| overwrite | 82.2 us | 49.8M rows/s |
| append | 424 us | 9.67M rows/s |
| keyed merge (upsert) | 6.41 ms | 639k rows/s |

Criterion prepares the stored side for append and merge outside the timer. Sub-millisecond estimates are regression anchors; the enum redirects to the same IPC implementation.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/media_ipc
```
