# Apache Avro

`yggdryl::avro` reads and writes Avro object containers, as streamed Arrow batches or as native values.

## Contract

| | |
| --- | --- |
| Owns | `avro::Avro` (stateful handle form), `avro::AvroOptions`, `Schema`, `Resolution`, `read_container`/`write_container`, `read_blocks`, single-object framing |
| Bindings | Python `yggdryl.media.avro`: `Schema`, `loads`/`dumps`, `loads_single`/`dumps_single`, `blocks`; JavaScript `avro`: `Schema`, `loads`/`dumps`, `loadsSingle`/`dumpsSingle`, `blocks` |
| Rust only | `Resolution`; a binding `reader_schema` option compiles and reuses it internally |
| Selects | A name whose media type says `avro`, on any handle, with no format argument |
| Decodes | Columnar, one builder per leaf, no `Scalar` tree; an unselected top-level column is skipped, not decoded, but its row bytes are still read |
| Block codec | `null`, `deflate` (default), `zstandard`; `snappy` in builds with the `parquet` feature |
| Sync marker | Absent (a fresh marker per write) or exactly 16 bytes |
| Record surface refuses | A union wider than `null` plus one branch, a recursive schema, a datatype Avro cannot spell; the `Scalar` functions have no such limits |
| Cached | `open` keeps the inferred wrapper, schema, and dimensions until `close`; writes invalidate |
| Limits | Every Rust reader has a `_with_limits` form over [`Limits`](../structured.md); Python snake-case keywords, JavaScript camel-case options |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | `read_container`, `write_container`, lazy blocks, schema resolution, single-object framing |
| [Arrow](arrow.md) | the record surface: batch readers, the three write intents, the datatype mapping |

## Use

`row_size` walks block counts and encoded lengths, jumps each payload positionally, and validates its sync marker without allocating, decompressing, or decoding rows. `column_size` reads only the header schema; both describe the whole container, ignoring selections, filters, and limits.

| form | role |
| --- | --- |
| `avro::Avro` | Handle, options, and the metadata cache that [`IOBase::open`](../../holder/iobase/bytes.md) fills and `close` releases |
| `avro::AvroOptions` | The shared record options plus the block codec name and an optional fixed sync marker for byte-reproducible writes |

## Block encoding options

The generic [`RecordOptions`](../options.md) exposes both Avro settings without downcasting, and the writer validates the codec name before pulling a row source.

=== "Rust"

    ```rust
    use yggdryl::media::RecordOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::AVRO)?;
    assert_eq!(options.avro_block_codec(), Some("deflate"));
    assert_eq!(options.avro_sync_marker(), None);

    options.set_avro_block_codec("zstandard")?;
    options.set_avro_sync_marker(Some(b"0123456789abcdef"))?;
    assert_eq!(options.avro_sync_marker(), Some(b"0123456789abcdef"));
    ```

=== "Python"

    ```python
    from yggdryl import RecordOptions

    options = RecordOptions("trades.avro")
    assert options.block_codec == "deflate"
    assert options.sync_marker is None

    options.block_codec = "zstandard"
    options.sync_marker = b"0123456789abcdef"
    assert options.sync_marker == b"0123456789abcdef"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { RecordOptions } = require('yggdryl')

    const options = RecordOptions.from('trades.avro')
      .withBlockCodec('zstandard')
      .withSyncMarker(Buffer.from('0123456789abcdef'))

    assert.equal(options.blockCodec, 'zstandard')
    assert.deepEqual(options.syncMarker, Buffer.from('0123456789abcdef'))
    ```

## Schemas, canonical form, and fingerprints

A `Schema` resolves namespaces, aliases, defaults, and recursive references at parse time; a named-type reference stays a reference, which keeps a recursive schema finite.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;

    let schema = Schema::from_str(
        r#"{"type": "record", "name": "trade", "doc": "one fill", "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2}
        ]}"#,
    )?;

    assert!(!schema.clone().into_canonical_form().contains("doc"));
    assert_eq!(schema.fingerprint().to_le_bytes()[0], 0xF5);
    let text = String::from_utf8(yggdryl::json::into_bytes(&schema.into_json())?)?;
    assert!(text.contains("field-id"));
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    document = {
        "type": "record",
        "name": "trade",
        "doc": "one fill",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2},
        ],
    }
    schema = avro.Schema(document)

    assert "doc" not in schema.into_canonical_form()
    assert schema.fingerprint().to_bytes(8, "little")[0] == 0xF5
    assert schema.into_json()["fields"][1]["field-id"] == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = new avro.Schema({
      type: 'record',
      name: 'trade',
      doc: 'one fill',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'long', 'field-id': 2 },
      ],
    })

    assert.ok(!schema.canonicalForm.includes('doc'))
    assert.equal(Number(schema.fingerprint & 0xffn), 0xf5)
    assert.equal(schema.intoJSON().fields[1]['field-id'], 2)
    ```

`fingerprint` hashes the Parsing Canonical Form with CRC-64-AVRO, which strips whitespace, attribute order, docs, unknown attributes, logical annotations, aliases, and defaults. Equality, total ordering, and `stable_hash` use the complete retained JSON document instead.

## Logical types

`date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`, `local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes and fixed, and `duration` decode as typed values.

=== "Rust"

    ```rust
    use yggdryl::TimeUnit;
    use yggdryl::holder::Buffer;
    use yggdryl::{Timezone, Scalar};
    use yggdryl::json;
    use yggdryl::avro;

    let schema = json::from_utf8(
        r#"{"type": "record", "name": "row", "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}}
        ]}"#,
    )?;
    let row = Scalar::from_struct([
        (
            "day",
            Scalar::date32_in(19_782, TimeUnit::Day, Timezone::NAIVE)?,
        ),
        ("at", Scalar::datetime64(
            1_700_000_000_000_000,
            TimeUnit::Microsecond,
            Timezone::UTC,
        )?),
        ("price", Scalar::d128(18_750, 2)),
    ])?;

    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &[row.clone()])?;
    assert_eq!(avro::read_container(&handle)?.rows[0], row);
    ```

=== "Python"

    ```python
    from datetime import date, datetime, timezone
    from decimal import Decimal

    from yggdryl.media import avro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}},
        ],
    }
    row = {
        "day": date(2024, 2, 29),
        "at": datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc),
        "price": Decimal("187.50"),
    }

    decoded = avro.loads(avro.dumps([row], schema)).rows[0]
    assert decoded == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, avro } = require('yggdryl')

    const decimal = {
      type: 'bytes',
      logicalType: 'decimal',
      precision: 10,
      scale: 2,
    }
    const value = Scalar.decimal(18750n, 2)
    const decoded = avro.loadsSingle(avro.dumpsSingle(value, decimal), decimal)

    assert.ok(decoded instanceof Scalar)
    assert.equal(decoded.kind, 'd64')
    assert.equal(decoded.unscaled, 18750n)
    assert.equal(decoded.scale, 2)
    ```

A date is `Date32`, a timestamp is `DateTime64` with `UTC`, and a decimal keeps its exact coefficient and scale.

## Codecs and limits

Input bytes bound the container and each decompressed block, depth bounds schema and datum nesting, and the node budget bounds rows and per-datum allocation.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Limits, Scalar};
    use yggdryl::json;
    use yggdryl::avro;

    let schema = json::from_utf8(r#""long""#)?;
    let mut bytes = Buffer::new();
    avro::write_container(&mut bytes, &schema, &[], &[Scalar::from(7_i64)])?;

    let limits = Limits::new(8, 1_024, 8, 1);
    assert_eq!(
        avro::read_container_with_limits(&bytes, limits)?.rows,
        [Scalar::from(7_i64)]
    );
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    encoded = avro.dumps([7], '"long"')
    decoded = avro.loads(
        encoded,
        max_depth=8,
        max_input_bytes=1_024,
        max_nodes=8,
    )

    assert decoded.rows == [7]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const encoded = avro.dumps([7], '"long"')
    const decoded = avro.loads(encoded, {
      maxDepth: 8,
      maxInputBytes: 1024,
      maxNodes: 8,
    })

    assert.deepEqual(decoded.rows, [7])
    ```

| header codec | implementation |
| --- | --- |
| `null`, `deflate`, `zstandard` | The crate's own [`Codec`](../../coding/index.md) implementations |
| `snappy` | Raw Snappy followed by a big-endian CRC-32 of the uncompressed block; builds carrying the `parquet` feature |
| `bzip2`, `xz`, any other name | Refused, naming it and listing what this build implements |

## Edges

- `trades.avro.gz` -> refused rather than double-compressed: Avro compresses inside its blocks, like [Parquet](../parquet/index.md) and unlike [IPC](../ipc/index.md).
- `set_avro_block_codec` or `set_avro_sync_marker` on options for another encoding -> typed record error.
- A sync marker of any length but 16 bytes -> refused; an absent marker generates a fresh one per write.
- An unknown logical annotation, or attributes invalid for its underlying type -> degrades to the underlying type, never an error.
- A decimal wider than 38 digits -> keeps its raw bytes; `duration` keeps its twelve bytes, being a month/day/millisecond triple.
- Same fingerprint, different retained JSON -> distinct schema values; the bindings' `equals`, comparison, and hash follow the JSON identity.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib avro::tests
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
