# Media

A handle's declared media type names one scheme, and every scheme answers the same two surfaces: rows as native scalars, rows as Arrow batches.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Media`, `Media::open`, `open_as`, `ipc`, `parquet`, `avro`, `text`, `handle`, `into_handle` |
| Variants | `Ipc`, `Parquet`, `Avro`, `Text`, `Xml` |
| Selects on | the handle's declared media type; nothing is read to decide |
| Every variant | implements [`IOMedia`](../holder/iobase/records.md): `record_options`, `read_arrow_field`, `read_arrow_reader`, three write methods |
| Writes take | an [`arrow::BatchReader`](../arrow/readers.md); signatures and validation live in [Records](../holder/iobase/records.md) |
| Settings | one shared [`RecordOptions`](options.md) behind every record encoding |
| Documents | JSON, JSON Lines, YAML, and TOML are not record encodings; [Structured documents](structured.md) owns them and their one Arrow bridge |
| Content coding | the handle's business, not the scheme's |
| Errors | an encoding with no implementation in this build is reported, never guessed |
| Bindings | Rust: the enum; Python: `yggdryl.media.Media` with `Ipc`, `Parquet`, `Avro`, `Xml` under it and `Text` beside it; JavaScript: one `IOBase` class |

## Schemes

Each scheme owns three pages: what it is, how rows cross as native scalars, and how they cross as Arrow.

| Scheme | Declared by | Overview | Scalars | Arrow |
| --- | --- | --- | --- | --- |
| Arrow IPC | `application/vnd.apache.arrow.stream`, `.arrows` | [Arrow IPC](ipc/index.md) | [rows](ipc/scalar.md) | [batches](ipc/arrow.md) |
| Apache Parquet | `application/vnd.apache.parquet`, `.parquet` | [Parquet](parquet/index.md) | [rows](parquet/scalar.md) | [batches](parquet/arrow.md) |
| Apache Avro | `application/avro`, `.avro` | [Avro](avro/index.md) | [containers](avro/scalar.md) | [batches](avro/arrow.md) |
| XML | `application/xml`, `text/xml`, `.xml` | [XML](xml/index.md) | [documents](xml/scalar.md) | [rows](xml/arrow.md) |
| Plain text | `text/plain`, `.txt`, `.log` | [Plain-text records](text/index.md) | [lines](text/scalar.md) | [batches](text/arrow.md) |
| JSON | `application/json`, `application/x-ndjson`, `.json`, `.jsonl` | [JSON](json/index.md) | [documents](json/scalar.md) | [rows](json/arrow.md) |
| YAML | `application/yaml`, `.yaml` | [YAML](yaml/index.md) | [documents](yaml/scalar.md) | [rows](yaml/arrow.md) |
| TOML | `application/toml`, `.toml` | [TOML](toml/index.md) | [documents](toml/scalar.md) | [rows](toml/arrow.md) |
| Apache Iceberg | a table folder, not a leaf | [Iceberg](iceberg/index.md) | - | [reads](iceberg/read.md), [writes](iceberg/write.md) |

Iceberg is a table over a folder rather than one leaf, so both surfaces live on its own read and write pages.

| Shared page | Owns |
| --- | --- |
| [RecordOptions](options.md) | the declared root, `batch_row_size`, identity, shared by every record encoding |
| [Structured documents](structured.md) | the `Format` vocabulary, limits, formatting, and the facade over JSON, YAML, and TOML |
| [Placeholders](placeholders.md) | the Jinja-style `{{ }}` contract YAML and TOML share |
| [Parquet footer](parquet/footer.md) | footer metadata, statistics, and the caching `Parquet<H>` wrapper |
| [Iceberg schema](iceberg/schema.md), [catalog](iceberg/catalog.md) | evolution and field ids, the warehouse over one folder |

## The two surfaces

| Surface | Calls | Answers |
| --- | --- | --- |
| Scalars | `overwrite_records`, `append_records`, `merge_records`, `read_records`; `read_scalar` and `write_scalar` for a document | native rows: a tuple, a mapping, a dataclass, a plain object, a [`Scalar`](../types/scalar.md) |
| Arrow | `read_arrow_reader`, `read_arrow_field`, the three `*_arrow_reader` intents; `read_arrow` and `write_arrow` | an [`arrow::BatchReader`](../arrow/readers.md), one batch at a time |

Choosing the scheme is the only thing that changes; the calls stay the same.

## Use

`Media::open` binds the implementation the name declares, and reports the media
type it found when this build has none. Python applies the same choice when the
handle is built, so `type(handle)` names it; JavaScript has one `IOBase` class.

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

    // An encoding with no implementation in this build is named, never guessed.
    let message = Media::open(named("trades.csv")?).unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
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

## Shared IOMedia calls

Choosing the encoding is the only thing that changes, and the handle owns the content coding: the same calls, different bytes underneath. Rust only; Python and JavaScript make the same calls on the handle itself, as on [Arrow IPC](ipc/index.md#use).

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

// A Media is also the bytes it encodes: an Arrow IPC stream opens with its
// continuation marker, and the same stream behind gzip framing with gzip's.
for (name, magic) in [
    ("trades.arrows", &[0xFF_u8, 0xFF, 0xFF, 0xFF][..]),
    ("trades.arrows.gz", &[0x1F, 0x8B][..]),
] {
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;
    let url = Url::from_str(&format!("file:///{name}"))?;
    let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
    let mut media = Media::open(handle)?.with_field(schema.clone());
    let options = media.record_options()?;

    media.overwrite_arrow_reader(arrow::batch_reader(Arc::clone(&arrow_schema), [batch]), &options)?;
    assert_eq!(media.read_arrow_reader(&options)?.count(), 1, "{name}");
    assert_eq!(media.read_arrow_field(&options)?, schema, "{name}");
    assert_eq!(media.read_range_bytes(0, magic.len())?, magic, "{name}");
}
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

## Edges

- `text/csv` on `Media::open` -> error naming the found media type; no encoding is guessed.
- Encoding already known -> `Media::ipc` and `Media::parquet` name a variant directly.
- Handle name not trustworthy -> `Media::open_as` takes an explicit `MimeType`.
- Plain text -> `Media::Text`; any other handle still reaches rows through `IOMedia` and [`RecordOptions`](options.md).
- Python name with no implementation (`trades.csv`) -> nothing is composed and the handle stays a `Path`; only `Media::open` reports it.
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
