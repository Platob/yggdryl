# Parquet compression

Parquet compresses its own pages, so the codec is a write setting the footer records - and a coding around the whole file is refused rather than silently applied.

## Contract

| Item | Behaviour |
| --- | --- |
| Setting | `ParquetOptions::compression`, default Zstandard at the crate's default level |
| Spellings | the text the `parquet` crate parses: `uncompressed`, `snappy`, `gzip(6)`, `lzo`, `brotli(1)`, `lz4`, `zstd(3)`, `lz4_raw` |
| Direction | a write setting only; the footer records the codec, so every runtime reads every file without naming one |
| Scope | pages inside the file, per column chunk; the magic, the footer and the row-group layout stay readable |
| `level` | unused: the shared compression level belongs to an outer coding, and Parquet has none |
| Refused | a handle whose media type declares a coding - `trades.parquet.gz` - on reads and writes alike, before anything is encoded; the handle is left untouched |
| Refused | a spelling the writer does not accept, as an invalid record option naming `$.compression` |
| Bindings | Python `options.compression = "zstd(1)"`, JavaScript `withCompression('zstd(1)')`, Rust `ParquetOptions::with_compression(Compression::ZSTD(..))` |

## Use

The bindings name page compression as the text the `parquet` crate parses: `zstd(3)`, `snappy`, `uncompressed`. Compression is a write setting only; the footer records the codec, so every runtime reads every file. A coding around the whole file would move the footer out of reach, so a coded name is refused before anything is encoded, and the handle is left untouched.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use parquet::basic::Compression;
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType, StructType, Url};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");

    let ids: Vec<i64> = (0..1_024).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let arrow_schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )?;

    let mut sizes = Vec::new();
    for compression in [
        Compression::UNCOMPRESSED,
        Compression::SNAPPY,
        Compression::ZSTD(Default::default()),
    ] {
        // One batch per read, so the comparison is not split by the default bound.
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
            .with_options(
                ParquetOptions::new()
                    .with_compression(compression)
                    .with_batch_row_size(batch.num_rows()),
            );
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]),
            &options,
        )?;

        // Nothing on the read side names the compression: the footer records it.
        let read = media
            .read_arrow_reader(&options)?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(read, [batch.clone()], "{compression:?}");
        sizes.push(media.handle().size());
    }

    assert!(sizes[0] > sizes[1] && sizes[0] > sizes[2], "{sizes:?}");

    // A coding around the whole file is refused, and nothing is published.
    let coded = Url::from_str("file:///trades.parquet.gz")?;
    let mut media = Parquet::new(Buffer::new().with_media_type(coded.media_type()));
    let options = media.record_options()?;
    let message = media
        .overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch]),
            &options,
        )
        .unwrap_err()
        .to_string();

    assert!(message.contains("parquet compresses"), "{message}");
    assert!(message.contains("ParquetOptions::compression"), "{message}");
    assert!(media.handle().is_empty());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    rows = 1_024
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    table = pa.table(
        {"id": list(range(rows)), "symbol": ["AAPL"] * rows}, schema=schema
    )

    sizes = []
    for compression in ("uncompressed", "snappy", "zstd(1)"):
        handle = IOBase(root / f"trades-{compression}.parquet")
        # One batch per read, so the comparison is not split by the default bound.
        options = handle.record_options()
        options.compression = compression
        options.batch_row_size = rows
        handle.overwrite_arrow_table(table, options=options)

        # Nothing on the read side names the compression: the footer records it.
        read = handle.read_arrow_reader(options=options).read_all()
        assert read.num_rows == rows, compression
        sizes.append(handle.size)

    assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes

    # A coding around the whole file is refused, and nothing is published.
    coded = IOBase(root / "trades.parquet.gz")
    with pytest.raises(ValueError, match="parquet compresses"):
        coded.overwrite_arrow_table(table)
    assert coded.size == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const ids = Array.from({ length: 1_024 }, (_, index) => BigInt(index))
    const table = new arrow.Table({
      id: arrow.vectorFromArray(ids, new arrow.Int64()),
      symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
    })

    const sizes = []
    for (const compression of ['uncompressed', 'snappy', 'zstd(1)']) {
      const handle = new IOBase(path.join(root, `trades-${compression}.parquet`))
      // One batch per read, so the comparison is not split by the default bound.
      const options = handle
        .recordOptions()
        .withCompression(compression)
        .withBatchRowSize(table.numRows)
      handle.overwriteArrowTable(table, options)

      // Nothing on the read side names the compression: the footer records it.
      const read = handle.readArrowReader(options).intoTable()
      assert.equal(read.numRows, 1_024, compression)
      sizes.push(handle.size)
    }

    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    // A coding around the whole file is refused, and nothing is published.
    const coded = new IOBase(path.join(root, 'trades.parquet.gz'))
    assert.throws(() => coded.overwriteArrowTable(table), /parquet compresses/)
    assert.equal(coded.size, 0)

    fs.rmSync(root, { recursive: true, force: true })
    ```

| setting | effect |
| --- | --- |
| `compression` | codec applied to pages inside the file |
| `level` | nothing; Parquet has no outer coding to apply it to |

## Edges

- `trades.parquet.gz`, or any non-identity coding -> refused on reads and writes with `parquet compresses`, naming `ParquetOptions::compression`; nothing is published.
- other encodings, such as [Arrow IPC](../ipc/index.md), take a coded name through the handle's [coding](../../coding/index.md); Parquet alone refuses one.
- `level` -> ignored; `compression` decides how the file compresses.
- a foreign file -> read under the codec its footer names; nothing on the read side names one.
- an unknown spelling -> refused as an invalid record option under `$.compression`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib parquet::tests
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_parquet.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
