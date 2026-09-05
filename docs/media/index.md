# Media

`Media` binds a handle to the record encoding that its declared media type names.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Media`, `Media::open`, `open_as`, `ipc`, `parquet` |
| Variants | `Ipc`, `Parquet`, `Avro` |
| Selects on | the handle's declared media type; nothing is read to decide |
| Every variant | implements [`IOMedia`](../holder/iobase/records.md): `record_options`, `read_arrow_field`, `read_arrow_reader`, three write methods |
| Writes take | an [`arrow::BatchReader`](../arrow/readers.md); signatures and validation rules are documented once in [Records](../holder/iobase/records.md) |
| Settings | one shared [`RecordOptions`](options.md) behind every encoding |
| Plain text | no wrapper; every `IOBase` reaches it through `IOMedia` and [`RecordOptions`](options.md) |
| Content coding | the handle's business, not the encoding's |
| Errors | an encoding with no implementation in this build is reported, never guessed at |
| Bindings | Rust only; Python and JavaScript reach an encoding through the handle |

## Use

Rust only. `Media::open` binds the IPC, Parquet, or Avro implementation the name declares.

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
    ```

## Pages

| Page | Owns |
| --- | --- |
| [Arrow IPC](ipc.md) | Arrow IPC streams over any handle, schema carriage, the one-stream contract |
| [Apache Parquet](parquet.md) | The Parquet record surface, pushdown, compression, coded-handle refusal |
| [Parquet footer](parquet-footer.md) | Footer metadata, statistics, and the caching `Parquet<H>` wrapper |
| [Apache Avro](avro.md) | Avro as streamed Arrow batches, block options, schema resolution |
| [Plain-text records](text.md) | `TextOptions`, the url/rownum/body schema, autotyping |
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

A name that declares an encoding and a coding gives the same calls and different bytes underneath.

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

## Unimplemented encodings

The error names the media type that was found and the ones that would have worked.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::Media;
    use yggdryl::holder::Buffer;
    use yggdryl::Url;

    let url = Url::from_str("file:///trades.csv")?;
    let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));

    let message = Media::open(handle).unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
    ```

## Edges

- `text/csv` on `Media::open` -> error naming the found media type; no encoding is guessed.
- Encoding already known -> `Media::ipc` and `Media::parquet` name a variant directly.
- Handle name not trustworthy -> `Media::open_as` takes an explicit `MimeType`.
- `trades.arrows.gz` -> the same calls, an IPC stream behind gzip framing.
- Plain text -> no variant; `IOMedia` and [`RecordOptions`](options.md) on any handle.
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

`io_write_stateful/media_ipc` drives the generic enum over its IPC variant with a 4,096-row, four-column fixture. Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23).

| operation through `Media::Ipc` | estimate | throughput |
| --- | ---: | ---: |
| overwrite | 82.2 us | 49.8M rows/s |
| append | 424 us | 9.67M rows/s |
| keyed merge (upsert) | 6.41 ms | 639k rows/s |

Criterion prepares the stored side for append and keyed merge outside the timer. Sub-millisecond estimates carry allocator variance and are regression anchors; the enum redirects to the same IPC implementation.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/media_ipc
```
