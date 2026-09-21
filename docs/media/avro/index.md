# Apache Avro

`yggdryl::avro` reads and writes Avro object containers, as streamed Arrow batches or as native values.

## Contract

| | |
| --- | --- |
| Owns | `avro::Avro` (stateful handle form), `avro::AvroOptions`, `Schema`, `Resolution`, `read_container`/`write_container`, `read_blocks`, single-object framing |
| Bindings | Python `yggdryl.avro`: `Schema`, `loads`/`dumps`, `loads_single`/`dumps_single`, `blocks`; JavaScript `avro`: `Schema`, `loads`/`dumps`, `loadsSingle`/`dumpsSingle`, `blocks` |
| Rust only | `Resolution`; a binding `reader_schema` option compiles and reuses it internally |
| Selects | A name whose media type says `avro`, on any handle, with no format argument |
| Reads | [Read](read.md): a container whole, block by block, or resolved onto a reader schema, as native scalars or as Arrow batches |
| Writes | [Write](write.md): containers and single framed datums as native scalars, the three intents as Arrow batches |
| Decodes | Columnar, one builder per leaf, no `Scalar` tree; an unselected top-level column is skipped, not decoded, but its row bytes are still read |
| Schemas | [Schemas](schemas.md): the retained JSON document, the Parsing Canonical Form, the CRC-64-AVRO fingerprint, and the logical types |
| Block codec | `null`, `deflate` (default), `zstandard`; `snappy` in builds with the `parquet` feature: [Blocks](blocks.md) |
| Sync marker | Absent (a fresh marker per write) or exactly 16 bytes: [Blocks](blocks.md) |
| Record surface refuses | A union wider than `null` plus one branch, a recursive schema, a datatype Avro cannot spell; the `Scalar` functions have no such limits |
| Cached | `open` keeps the inferred wrapper, schema, and dimensions until `close`; writes invalidate |
| Limits | Every Rust reader has a `_with_limits` form over [`Limits`](../structured.md); Python snake-case keywords, JavaScript camel-case options: [Blocks](blocks.md#codecs-and-limits) |

## Pages

Avro answers the two surfaces every medium answers - rows as native scalars and rows as Arrow batches - and both sit on the direction pages, so one page holds every spelling of a read and one holds every spelling of a write. It also carries a raw container codec of its own, implemented here with no Avro crate underneath: `read_container`, `read_blocks`, and `write_container` over any handle, which is what an [Iceberg](../iceberg/index.md) manifest is read with.

| Page | Owns |
| --- | --- |
| [Read](read.md) | rows out: a container as native scalars, writer/reader resolution, lazy blocks, then Arrow batches |
| [Write](write.md) | rows in: containers and single-object datums as native scalars, then the three Arrow intents and the datatype mapping a write has to spell |
| [Schemas](schemas.md) | the schema value: canonical form, fingerprint, identity, logical types |
| [Blocks](blocks.md) | the block codec, the synchronization marker, and the limits every decode carries |

## Use

`row_size` walks block counts and encoded lengths, jumps each payload positionally, and validates its sync marker without allocating, decompressing, or decoding rows. `column_size` reads only the header schema; both describe the whole container, ignoring selections, filters, and limits.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::avro::Avro;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;

    let mut media = Avro::new(Buffer::new().with_media_type(MimeType::AVRO.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    // Dimensions describe every block, and an opened handle answers from cache.
    media.open()?;
    assert_eq!((media.row_size()?, media.column_size()?), (2, 1));
    assert_eq!(media.read_arrow_field(&options)?.name(), "row");
    media.close()?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.avro")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2]}))
    with handle:
        assert (handle.row_size, handle.column_size) == (2, 1)
        assert handle.read_arrow_field().name == "row"
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
    const handle = new IOBase(path.join(root, 'trades.avro'))
    handle.overwriteArrowTable(new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    }))
    handle.open()
    assert.deepEqual([handle.rowSize, handle.columnSize], [2, 1])
    assert.equal(handle.readArrowField().name, 'row')
    handle.close()
    fs.rmSync(root, { recursive: true, force: true })
    ```

| form | role |
| --- | --- |
| `avro::Avro` | Handle, options, and the metadata cache that [`IOBase::open`](../../holder/iobase/bytes.md) fills and `close` releases |
| `avro::AvroOptions` | The shared record options plus the block codec name and an optional fixed sync marker for byte-reproducible writes |

## Edges

- `trades.avro.gz` -> refused rather than double-compressed: Avro compresses inside its blocks, like [Parquet](../parquet/index.md) and unlike [IPC](../ipc/index.md).
- `row_size` and `column_size` -> whole-container counts; a selection, a filter, or a row limit does not move them.
- an opened handle -> answers from the wrapper, schema, and dimensions `open` cached; any write invalidates them, and `close` drops them.
- an unselected top-level column -> skipped rather than decoded, but its row bytes are still read: Avro interleaves its columns per record.
- the block codec and the sync marker -> [Blocks](blocks.md); the logical types and the fingerprint -> [Schemas](schemas.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib avro::tests
    cargo test --features "parquet iceberg" -p yggdryl --test media avro::
    cargo test --features "parquet iceberg" -p yggdryl --test interop avro::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/avro
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_avro.py
    python/.venv/bin/python python/benchmarks/media.py --filter avro
    python/.venv/bin/python scripts/bench_avro_baseline.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/avro.test.js
    YGGDRYL_BENCH_FILTER=records/avro npm run --prefix node bench:media
    ```

## Performance

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

`scripts/bench_avro_baseline.py` writes one deterministic ten-thousand-entry [Iceberg](../iceberg/index.md) manifest (112,246 bytes, statistics included) from Rust, then times three readers over those exact bytes. One containerized x86_64 Linux run: rustc stable release build, CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1.

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
