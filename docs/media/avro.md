# Avro

The Apache Avro object container format: a header naming the writer schema and the block codec, then blocks of rows - read and written as records through the shared calls, and as plain values through the raw codec.

## Overview

| | |
| --- | --- |
| Declared by | `application/avro`, `.avro` |
| Build | default; the record surface rides Arrow, and the `snappy` block codec needs the `parquet` feature |
| Rust | `yggdryl::avro`: `Avro<H>` over any handle with `AvroOptions` and the free `read_field`, `read_batch_reader` and `overwrite_arrow_reader` for records; `read_container`, `read_container_resolved`, `write_container`, `read_blocks` and `Schema` for the raw codec, and `into_single_object_vec`/`from_single_object_slice` for single-object encoding |
| Python | any `IOBase` whose name declares Avro; `yggdryl.avro`: `loads`, `dumps`, `blocks`, `loads_single`, `dumps_single`, `Schema` |
| JavaScript | any `IOBase` whose name declares Avro; `avro`: `loads`, `dumps`, `blocks`, `loadsSingle`, `dumpsSingle`, `Schema` |
| Settings | `block_codec` - `deflate` unless set, or `null`, `snappy`, `zstandard` - and `sync_marker`, sixteen bytes, beside the shared [`RecordOptions`](index.md#options) |

## Read

A container carries its writer schema, so a record read needs no declaration and `read_arrow_field` answers the field that schema maps to. The raw codec reads the rows as plain values, and a reader schema resolves renames, promotions and defaults against the writer's: a writer field the reader does not name is jumped by its length rather than decoded. A container's blocks are independent once their headers are walked, so runs of whole blocks decompress and decode on every thread, batches returned in file order; a read under a row limit stays on one thread.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::holder::Buffer;
    use yggdryl::Scalar;
    use yggdryl::json;
    use yggdryl::avro;

    let writer = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string"},
            {"name":"qty","type":"int"},
            {"name":"venue","type":"string"}]}"#,
    )?;
    let reader = Schema::from_str(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"quantity","aliases":["qty"],"type":"long"},
            {"name":"note","type":"string","default":"none"}]}"#,
    )?;
    let row = json::from_utf8(r#"{"symbol":"AAPL","qty":100,"venue":"XNAS"}"#)?;
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &writer, &[], &[row])?;

    let decoded = avro::read_container_resolved(&handle, &reader)?;
    assert_eq!(
        decoded.rows[0].get_key_str("quantity").and_then(Scalar::as_i64),
        Some(100),
    );
    assert_eq!(decoded.rows[0].len(), 2, "unwanted writer fields are skipped");
    ```

=== "Python"

    ```python
    from yggdryl import avro

    writer = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "int"},
            {"name": "venue", "type": "string"},
        ],
    }
    reader = avro.Schema({
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "quantity", "aliases": ["qty"], "type": "long"},
            {"name": "note", "type": "string", "default": "none"},
        ],
    })
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100, "venue": "XNAS"}], writer)

    assert avro.loads(encoded, reader_schema=reader).rows == [
        {"note": "none", "quantity": 100}
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const writer = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'int' },
        { name: 'venue', type: 'string' },
      ],
    }
    const reader = new avro.Schema({
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'quantity', aliases: ['qty'], type: 'long' },
        { name: 'note', type: 'string', default: 'none' },
      ],
    })
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', qty: 100, venue: 'XNAS' }],
      writer,
    )

    assert.deepEqual(avro.loads(encoded, { readerSchema: reader }).rows, [
      { note: 'none', quantity: 100 },
    ])
    ```

## Write

A record write derives the writer schema from the field and writes the header - that schema, the `block_codec` and a sync marker, fresh and random unless `sync_marker` pins one for byte-for-byte reproducible output - then cuts the rows into blocks of about a megabyte, never more rows than a reader with default limits accepts, and encodes and compresses the blocks on every thread. Avro has no dictionary encoding, so a dictionary column is written as the values it encodes.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::types::Int32Type;
    use arrow_array::{ArrayRef, DictionaryArray, Int64Array, RecordBatch};
    use yggdryl::holder::Buffer;
    use yggdryl::{arrow, avro, IOBase, IOMedia, MimeType, Scalar};

    let venues: DictionaryArray<Int32Type> = std::iter::repeat_n("XNAS", 512).collect();
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from_iter_values(0..512)) as ArrayRef),
        ("venue", Arc::new(venues) as ArrayRef),
    ])?;

    let mut sizes = Vec::new();
    for codec in ["null", "deflate", "zstandard"] {
        let mut handle = Buffer::new().with_media_type(MimeType::AVRO.into());
        let mut options = handle.record_options()?;
        options.set_avro_block_codec(codec)?;
        handle.overwrite_arrow_reader(arrow::batch_reader(batch.schema(), [batch.clone()]), &options)?;
        sizes.push(handle.size());

        // Avro has no dictionary encoding: the column is written as its values.
        let read = handle.read_arrow_reader(&options)?.next().expect("one batch")?;
        assert_eq!(read.column(1).data_type(), &arrow_schema::DataType::Utf8);

        // The raw codec reads the same container as plain values.
        let container = avro::read_container(&handle)?;
        assert_eq!(container.rows.len(), 512);
        assert_eq!(container.rows[0].get_key_str("venue").and_then(Scalar::as_str), Some("XNAS"));
    }

    // Each block is compressed by the codec its header names for the reader.
    assert!(sizes[0] > sizes[1] && sizes[0] > sizes[2], "{sizes:?}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, avro

    root = pathlib.Path(tempfile.mkdtemp())
    table = pa.table({
        "id": pa.array(range(512), pa.int64()),
        "venue": pa.array(["XNAS"] * 512).dictionary_encode(),
    })

    sizes = []
    for codec in ("null", "deflate", "zstandard"):
        handle = IOBase(root / f"trades-{codec}.avro")
        handle.overwrite_arrow_table(table, block_codec=codec)
        sizes.append(handle.size())

        # Avro has no dictionary encoding: the column is written as its values.
        read = handle.read_arrow_reader().read_all()
        assert read.column("venue").type == pa.string()
        assert read.column("venue").to_pylist() == ["XNAS"] * 512

    # Each block is compressed by the codec its header names for the reader.
    assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes

    # The raw codec reads the same container as plain values.
    container = avro.loads((root / "trades-deflate.avro").read_bytes())
    assert container.rows[0] == {"id": 0, "venue": "XNAS"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase, avro } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const ids = Array.from({ length: 512 }, (_, index) => BigInt(index))
    const table = new arrow.Table({
      id: arrow.vectorFromArray(ids, new arrow.Int64()),
      venue: arrow.vectorFromArray(
        ids.map(() => 'XNAS'),
        new arrow.Dictionary(new arrow.Utf8(), new arrow.Int32()),
      ),
    })

    const sizes = []
    for (const codec of ['null', 'deflate', 'zstandard']) {
      const handle = new IOBase(path.join(root, `trades-${codec}.avro`))
      handle.overwriteArrowTable(table, { blockCodec: codec })
      sizes.push(handle.size())

      // Avro has no dictionary encoding: the column is written as its values.
      const read = handle.readArrowReader().intoTable()
      assert.ok(read.schema.fields[1].type instanceof arrow.Utf8)
    }

    // Each block is compressed by the codec its header names for the reader.
    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    // The raw codec reads the same container as plain values.
    const container = avro.loads(fs.readFileSync(path.join(root, 'trades-deflate.avro')))
    assert.equal(container.rows[0].venue, 'XNAS')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Performance

### Against polars and fastavro

One run of `python/benchmarks/media/avro.py --repeat 5` on a containerized four-core x86_64 Linux host, release wheel: CPython 3.11.15, PyArrow 25.0.1, polars 1.44.2, fastavro 1.12.2. Every read case is one container fastavro wrote in blocks of about 64,000 bytes, the Java writer's default, read three ways: `polars.read_avro`, `fastavro.reader` drained row by row, and `read_arrow_reader(...).read_all()`. The key and price columns polars reads are checked against this crate's, value for value, before anything is timed. Every write case is one table written by `DataFrame.write_avro`, `fastavro.writer` and `overwrite_arrow_table` with the same block codec. The ratios are the other library's best time over this crate's, so above one is in this crate's favor. polars cannot read Zstandard blocks - it refuses them or misreads them - so that row has no polars column.

| read | polars | fastavro | yggdryl | x polars | x fastavro |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1M trade rows, null | 177.31 ms | 2,706.5 ms | 40.14 ms | 4.42 | 67.42 |
| 1M trade rows, deflate | 386.79 ms | 2,674.2 ms | 63.51 ms | 6.09 | 42.11 |
| 1M trade rows, snappy | 295.19 ms | 2,751.4 ms | 55.66 ms | 5.30 | 49.43 |
| 1M trade rows, zstandard | - | 3,276.3 ms | 51.84 ms | - | 63.20 |
| 1M trade rows, 2 of 6 columns, snappy | 218.95 ms | 2,833.3 ms | 40.41 ms | 5.42 | 70.12 |
| 64K trade rows, deflate | 24.45 ms | 204.8 ms | 5.73 ms | 4.26 | 35.72 |

| write | polars | fastavro | yggdryl | x polars | x fastavro |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1M trade rows, null | 123.48 ms | 2,571.3 ms | 85.28 ms | 1.45 | 30.15 |
| 1M trade rows, deflate | 3,812.26 ms | 4,007.9 ms | 346.61 ms | 11.00 | 11.56 |
| 1M trade rows, snappy | 320.55 ms | 2,597.8 ms | 97.61 ms | 3.28 | 26.61 |
| 64K trade rows, deflate | 115.58 ms | 225.3 ms | 28.54 ms | 4.05 | 7.89 |

What the reader does with the time: a container's blocks are independent once their headers are walked - a length read and jumped per block - so runs of whole blocks decompress and decode on every thread, batches returned in file order; an uncompressed block is decoded where the read's own copy holds it; a varint whose ten bytes are in the buffer is read without a bounds check per byte; and a string column validates its UTF-8 once per batch rather than once per value. A read under a row limit stays on one thread, and a table hands each file its share of `read.parallelism` and `write.parallelism`. The writer cuts rows into blocks of about a megabyte - never more rows than a reader with default limits accepts - resolves each column's Arrow values once per block rather than per cell, and encodes and compresses the blocks on every thread.

```bash
python/.venv/bin/python python/benchmarks/media/avro.py --repeat 5
```

### Record surface

Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23). The read fixture holds 65,536 rows and four columns; the write fixture holds 4,096 rows, its append and merge base prepared outside the timer.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 26.0 ms | 2.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 6.10 ms | 671k rows/s |
| `append_arrow_reader` | 4,096 | 12.9 ms | 318k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 11.4 ms | 358k rows/s |

Opened calls answer from the cache `open` fills; closed calls derive fresh metadata, on the same 65,536-row fixture.

| dimension | fresh | opened |
| --- | ---: | ---: |
| `row_size` | 20.6 us | 83.6 ns |
| `column_size` | 22.3 us | 87.8 ns |
| `read_arrow_field` | 101 us | 72.5 us |

The generic options enum redirects both Avro settings without downcasting or allocation.

| options operation | estimate |
| --- | ---: |
| read the block codec | 9.65 ns |
| set the block codec and a fixed marker | 49.8 ns |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/avro
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
```

### JavaScript raw codec, historical

One 4,580-byte container of 1,000 three-column rows, fixtures outside the loops, from `npm run bench:codec` on Node 24.18.0, an x86-64 Windows release build (AMD Ryzen 5 150). That script no longer exists and has no replacement, so these numbers stand as history.

| operation | ms/op |
| --- | ---: |
| schema parse / canonical form | 0.136 / 0.003 |
| container decode / resolve / encode | 13.075 / 10.549 / 18.667 |
| first compressed block / decode / resolve | 0.054 / 13.833 / 11.979 |
| single-object decode / encode | 0.018 / 0.087 |

The first-block row includes header parsing and the first lazy `next` but not row decompression; the block decode and resolve rows measure that separately.

### Against fastavro and PyIceberg, on identical bytes

`scripts/bench_avro_baseline.py` writes one deterministic ten-thousand-entry [Iceberg](iceberg.md) manifest (112,246 bytes, statistics included) from Rust, then times three readers over those exact bytes. One containerized x86_64 Linux run: rustc stable release build, CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1.

```text
fastavro 1.12.2:                    67,719 entries/s best (147.7 ms best of 7)
pyiceberg 0.11.1:                   46,790 entries/s best (213.7 ms best of 7)
yggdryl full (release):            101,937 entries/s best ( 98.1 ms best of 7)
yggdryl plan_stats (release):      203,252 entries/s best ( 49.2 ms best of 7)
yggdryl plan_identity (release):   438,596 entries/s best ( 22.8 ms best of 7)
```

| row | call | keeps |
| --- | --- | --- |
| `full` | `read_manifest` | Every field, the way the other two readers do |
| `plan_stats` | `read_manifest_for_plan(handle, true)`, what a filtered scan runs | The value counts, null counts, and bounds that pruning consults; the rest skipped as bytes |
| `plan_identity` | The unfiltered planning read | File identity, partition tuple, and sizes |

On this manifest the planning path is 2.1x the full decode with statistics kept and 4.4x without. The ratios hold at 1,000 and 100,000 entries: `manifest/decode_full`, `manifest/decode_plan_with_stats`, and `manifest/decode_plan_identity_only` in the `media` bench target.

The script needs `fastavro` and `pyiceberg` installed into `python/.venv` and regenerates its fixture itself.

```bash
python/.venv/bin/python scripts/bench_avro_baseline.py
```

### Codec groups

From the same machine, the five `codec/avro*` groups:

- **Types** (`codec/avro_types`, 10,000 rows each): primitives decode at ~2.9M rows/s and encode at ~3.5M rows/s. Two-string rows decode at ~3.3M rows/s, 18-digit decimals at ~6.7M rows/s, and array of records of maps at ~620K rows/s. The single-object varint floor sits at ~57-65 ns per framed datum.
- **Codec x block size** (`codec/avro_blocks`, 65,536 three-column rows): decode throughput is nearly flat from 1,024 to 65,536 rows per block for every codec. Below ~1,000 rows the per-block header and sync overhead shows. Encoded bytes decode at ~20 MiB/s for snappy, ~12 for deflate, ~9.6 for zstandard, and ~38 for null. Null's bytes are bigger, so compare row rates.
- **Projection** (`codec/avro_projection`, 40 columns, null codec so the skip itself is visible). Reading 3 of 40 columns takes 6.4 ms against 9.4 ms for all 40 over 8,192 rows. The saving is the decode and allocation of the 37 skipped columns, jumped by their length prefixes, never the row read.
- **Resolution** (`codec/avro_resolution`): compiling a five-field plan costs ~533 ns once. Executing it per row beats the direct decode on this shape: 4.12 ms against 4.74 ms for 10,000 rows. The plan skips two writer columns the reader never wanted.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
```
