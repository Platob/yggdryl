# Parquet footer

Owns the Parquet footer: field identifiers, `FileStatistics`, geospatial and variant columns, and the `Parquet<H>` wrapper that caches it between `open` and `close`.

## Contract

| Item | Behaviour |
| --- | --- |
| Footer read | validates the leaf is Parquet, range-reads the footer, decodes no rows |
| Field ids | `Field::with_parquet_field_id` writes the `PARQUET:field_id` key into the file schema; `parquet_field_id()` reads it back |
| `FileStatistics` | `num_rows`, writer, ordered `key_value_metadata` entries (duplicate keys kept), `row_groups` in file order with counts, sizes and an optional split offset, one column per leaf path (`address.zip`) |
| Bounds | `min_bytes` / `max_bytes` are Parquet's encoded bytes; missing bounds and counts stay null, never zero |
| `null_count(column)` | sums across row groups; `None` when no row group recorded any |
| Geospatial column | no min/max; `bounding_box` plus sorted ISO `geometry_types`; a geography records no box |
| Variant column | the metadata/value storage struct under a schema-level `VARIANT` logical type only |
| Cache | `open` parses the footer once, `close` drops it, any write invalidates it |
| Bindings | `read_parquet_statistics` and `read_parquet_geospatial_statistics(column)` (camelCase in JavaScript) return native records through [`Scalar`](../types/scalar.md) |
| Rust only | `Parquet<H>` and the `parquet::*` free functions |

## Use

Read the footer of a file written with two row groups and one key/value entry.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType, Scalar};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?.required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("MSFT"), None])),
        ],
    )?;
    let mut media = Parquet::new(
        Buffer::new().with_media_type(MimeType::PARQUET.into()),
    ).with_options(
        ParquetOptions::new()
            .with_max_row_group_size(2)
            .with_key_value("writer", "rust"),
    );
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = IOMedia::read_parquet_statistics(&media)?;
    assert_eq!(statistics.num_rows, 4);
    assert_eq!(statistics.row_groups.len(), 2);
    assert_eq!(statistics.null_count("symbol"), Some(2));
    let native = Scalar::from(statistics);
    assert_eq!(native.get_key_str("num_rows").and_then(Scalar::as_i64), Some(4));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    scratch = tempfile.TemporaryDirectory()
    handle = IOBase(pathlib.Path(scratch.name) / "trades.parquet")
    options = handle.record_options()
    options.max_row_group_size = 2
    options.key_value_metadata = {"writer": "python"}
    handle.overwrite_arrow_table(pa.table({"id": [1, 2, 3, 4]}), options=options)

    statistics = handle.read_parquet_statistics()
    assert statistics["num_rows"] == 4
    assert len(statistics["row_groups"]) == 2
    assert next(
        entry for entry in statistics["key_value_metadata"] if entry["key"] == "writer"
    ) == {
        "key": "writer",
        "value": "python",
    }
    assert isinstance(statistics["row_groups"][0]["columns"][0]["min_bytes"], bytes)
    scratch.cleanup()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-parquet-stats-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    const options = handle
      .recordOptions()
      .withMaxRowGroupSize(2)
      .withKeyValue('writer', 'javascript')
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2, 3, 4] }), options)

    const statistics = handle.readParquetStatistics()
    assert.equal(statistics.num_rows, 4)
    assert.equal(statistics.row_groups.length, 2)
    assert.deepEqual(statistics.key_value_metadata.find(({ key }) => key === 'writer'), {
      key: 'writer',
      value: 'javascript',
    })
    assert.ok(Buffer.isBuffer(statistics.row_groups[0].columns[0].min_bytes))
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Field identifiers

Projecting the root to Arrow before the write carries the ids into the file; reading reverses it. A downstream [Iceberg](iceberg/schema.md) or Delta layer resolves a renamed or moved column by that id.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id").with_parquet_field_id(1),
        DataType::Utf8.nullable_field("symbol").with_parquet_field_id(2),
    ])?
    .required_field("row");

    let arrow_schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(StringArray::from(vec![Some("AAPL")])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // The ids went into the file, so the Arrow schema carries them back.
    let schema = media.read_arrow_schema()?;
    assert_eq!(
        schema.field(0).metadata().get("PARQUET:field_id"),
        Some(&"1".to_owned())
    );

    // And the recovered Field answers by id rather than by position.
    let recovered = media.read_arrow_field(&options)?;
    assert_eq!(recovered.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(recovered.fields()[1].parquet_field_id()?, Some(2));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False, metadata={"PARQUET:field_id": "1"}),
        pa.field("symbol", pa.string(), metadata={"PARQUET:field_id": "2"}),
    ])
    batch = pa.record_batch({"id": [1], "symbol": ["AAPL"]}, schema=schema)

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_batch(batch)

    # The ids went into the file, so the recovered Field answers by id rather
    # than by position.
    recovered = handle.read_arrow_field()
    assert [child.parquet_field_id for child in recovered.dtype] == [1, 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    // Arrow JS carries the identifiers the same way Arrow does anywhere else:
    // as field metadata under the exact `PARQUET:field_id` key.
    const rows = new arrow.Table({
      id: arrow.vectorFromArray([1n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()),
    })
    const schema = new arrow.Schema(
      rows.schema.fields.map(
        (child, index) =>
          new arrow.Field(
            child.name,
            child.type,
            child.nullable,
            new Map([['PARQUET:field_id', String(index + 1)]]),
          ),
      ),
    )

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(new arrow.Table(schema, rows.batches[0].data))

    // The ids went into the file, so the recovered Field answers by id rather
    // than by position.
    const recovered = handle.readArrowField()
    assert.deepEqual([...recovered.dtype].map((child) => child.parquetFieldId), [1, 2])
    assert.equal(recovered.dtype.getFieldAt(0).get('PARQUET:field_id'), '1')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Geospatial and variant columns

A [geometry or geography](../types/geospatial.md) field writes Parquet's `GEOMETRY` or `GEOGRAPHY` logical type over `BYTE_ARRAY` WKB; the defaults `OGC:CRS84` and `spherical` write as absent. `read_parquet_geospatial_statistics(column)` rescans the stored WKB as a projected read, so it answers when the writer recorded nothing.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    }

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::geometry(None)?.nullable_field("shape"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(BinaryArray::from_opt_vec(vec![
                Some(&wkb_point(1.0, 2.0)[..]),
                None,
                Some(&wkb_point(-3.0, 7.0)[..]),
            ])),
        ],
    )?;
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = media.read_statistics()?;
    let columns = &statistics.row_groups[0].columns;
    let id = columns.iter().find(|column| column.path == "id").unwrap();
    let shape = columns.iter().find(|column| column.path == "shape").unwrap();
    assert!(id.min_bytes.is_some() && id.max_bytes.is_some());
    assert!(shape.min_bytes.is_none() && shape.max_bytes.is_none());
    assert_eq!(shape.null_count, Some(1));

    let geospatial = shape.geospatial.as_ref().unwrap();
    let bounds = geospatial.bounding_box.unwrap();
    assert_eq!(
        (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
        (-3.0, 1.0, 2.0, 7.0)
    );
    assert_eq!(geospatial.geometry_types, vec![1]);
    assert_eq!(
        IOMedia::read_parquet_geospatial_statistics(&media, "shape")?,
        *geospatial,
    );
    ```

=== "Python"

    ```python
    import pathlib
    import struct
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    def point(x: float, y: float) -> bytes:
        return b"\x01\x01\x00\x00\x00" + struct.pack("<dd", x, y)

    schema = pa.schema([
        pa.field(
            "shape",
            pa.binary(),
            metadata={
                b"ARROW:extension:name": b"geoarrow.wkb",
                b"ARROW:extension:metadata": b'{"crs":"OGC:CRS84"}',
            },
        )
    ])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "shapes.parquet")
    handle.overwrite_arrow_table(
        pa.table({"shape": [point(1, 2), None, point(-3, 7)]}, schema=schema)
    )

    scanned = handle.read_parquet_geospatial_statistics("shape")
    footer = handle.read_parquet_statistics()["row_groups"][0]["columns"][0][
        "geospatial"
    ]
    assert scanned == footer
    assert scanned["geometry_types"] == [1]
    assert scanned["bounding_box"]["xmin"] == -3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const point = (x, y) => {
      const bytes = Buffer.allocUnsafe(21)
      bytes.writeUInt8(1, 0)
      bytes.writeUInt32LE(1, 1)
      bytes.writeDoubleLE(x, 5)
      bytes.writeDoubleLE(y, 13)
      return bytes
    }

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-geo-'))
    const handle = new IOBase(path.join(root, 'shapes.parquet'))
    handle.overwriteArrowTable(new arrow.Table({
      shape: arrow.vectorFromArray(
        [point(1, 2), null, point(-3, 7)],
        new arrow.Binary(),
      ),
    }))

    const scanned = handle.readParquetGeospatialStatistics('shape')
    assert.deepEqual(scanned.geometry_types, [1])
    assert.equal(scanned.bounding_box.xmin, -3)
    assert.equal(scanned.bounding_box.ymax, 7)
    fs.rmSync(root, { recursive: true, force: true })
    ```

## The handle underneath

Rust only. `Parquet<H>` is an [`IOBase`](../holder/index.md) over the handle it owns; it forwards every byte method and keeps `open`, `opened`, and `close`.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::media::parquet::{self, Parquet, ParquetOptions};
use yggdryl::{DataType, MimeType};

let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = field.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

// The free functions take a handle and options; nothing is bound.
let options = ParquetOptions::new();
let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
parquet::overwrite_arrow_reader(
    &mut handle,
    arrow::batch_reader(arrow_schema, [batch]),
    &options,
)?;

assert_eq!(parquet::read_arrow_schema(&handle)?.fields().len(), 1);
assert_eq!(parquet::read_field(&handle, &options)?.name(), "row");
assert_eq!(parquet::read_batch_reader(&handle, None, &options)?.count(), 1);
assert_eq!(parquet::read_statistics(&handle)?.num_rows, 2);

// A Parquet is also the bytes it encodes, magic bytes included.
let mut media = Parquet::new(handle);
assert_eq!(media.read_range_bytes(0, 4)?, *b"PAR1");

// open caches the footer, close releases it.
assert!(!media.opened());
media.open()?;
assert!(media.opened());
assert_eq!(media.read_statistics()?.num_rows, 2);
assert_eq!(media.row_size()?, 2);
assert_eq!(media.column_size()?, 1);
media.close()?;
assert!(!media.opened());
```

### Absent and empty files

[`Media::parquet`](index.md) names the variant at run time and [`record_options`](options.md) derives `ParquetOptions` from the handle's media type.

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // Nothing has been written, so there is nothing to read.
    let empty = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
        .with_field(field.clone());
    let options = empty.record_options()?;
    let reader = empty.read_arrow_reader(&options)?;
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(reader.count(), 0);

    // An empty write still publishes a readable file with the schema in its footer.
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(
        arrow::batch_reader(
            field.into_arrow_schema()?,
            std::iter::empty::<RecordBatch>(),
        ),
        &options,
    )?;
    assert_eq!(media.read_arrow_reader(&options)?.count(), 0);
    assert_eq!(media.read_arrow_schema()?.fields().len(), 1);
    assert_eq!(media.read_statistics()?.num_rows, 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    # Nothing has been written, so there is nothing to read.
    empty = IOBase(root / "absent.parquet")
    assert empty.read_arrow_reader().read_all().num_rows == 0

    # An empty write still publishes a readable file with the schema in its footer.
    handle = IOBase(root / "written.parquet")
    handle.overwrite_arrow_table(pa.Table.from_batches([], schema=schema))
    assert handle.size > 0
    assert handle.read_arrow_reader().read_all().num_rows == 0
    assert len(handle.read_arrow_field().dtype) == 1
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

    // Nothing has been written, so there is nothing to read.
    const empty = new IOBase(path.join(root, 'absent.parquet'))
    assert.equal(empty.readArrowReader().intoTable().numRows, 0)

    // An empty write still publishes a readable file with the schema in its footer.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const handle = new IOBase(path.join(root, 'written.parquet'))
    handle.overwriteArrowTable(new arrow.Table(schema))
    assert.ok(handle.size > 0)
    assert.equal(handle.readArrowReader().intoTable().numRows, 0)
    assert.equal(handle.readArrowField().dtype.length, 1)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Edges

- non-Parquet leaf (for example `redirect.arrows`) -> `Error::InvalidRecord`, message `expected Parquet media`.
- absent file -> `read_arrow_reader` yields zero batches under the declared schema; no missing-footer error.
- write with no batches -> a real file; the footer holds the schema and `num_rows == 0`.
- geospatial column -> the writer records no min/max; a foreign writer's min/max is ignored on read.
- geometry / geography declaration -> the CRS, and a geography's edge algorithm, ride into the file's logical type.
- foreign `GEOMETRY` / `GEOGRAPHY` / `VARIANT` file -> plain `Binary` / `Struct` Arrow types without extension metadata; files written here round-trip through the embedded Arrow schema.
- variant value across an Arrow array boundary -> unsupported until the Iceberg v3 layer lands.
- `geoarrow.wkb` spelling -> revisitable, GeoArrow is not finalized.
- Python and JavaScript -> an opened `IOBase` retains the same wrapper, so the footer cache applies there too.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::parquet::tests
    cargo test --features "parquet iceberg" -p yggdryl --lib media::parquet::tests::geospatial
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- 'io_dimensions/parquet/.*statistics'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_parquet.py
    python/.venv/bin/python python/benchmarks/media.py --filter "parquet read statistics" --filter "parquet read geospatial stats" --filter "parquet read arrow field"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    YGGDRYL_BENCH_FILTER=records/read_parquet_statistics npm run --prefix node bench:media
    YGGDRYL_BENCH_FILTER=records/read_parquet_geospatial_statistics npm run --prefix node bench:media
    ```

## Performance

Local release-build spot-check of the Python and JavaScript binding boundary; fixtures differ, so rows are per-runtime anchors, not a comparison.

| operation | runtime | rows | estimate |
| --- | --- | ---: | ---: |
| footer to native record (`read_parquet_statistics`) | Python | 65,536 | 504 us |
| footer to native record (`readParquetStatistics`) | JavaScript | 10,000 | 728 us |
| projected WKB to native record (`read_parquet_geospatial_statistics`) | Python | 8,192 | 2.64 ms |
| projected WKB to native record (`readParquetGeospatialStatistics`) | JavaScript | 10,000 | 3.26 ms |

```bash
python/.venv/bin/python python/benchmarks/media.py --filter "parquet read statistics" --filter "parquet read geospatial stats" --filter "parquet read arrow field"
YGGDRYL_BENCH_FILTER=records/read_parquet_statistics npm run --prefix node bench:media
YGGDRYL_BENCH_FILTER=records/read_parquet_geospatial_statistics npm run --prefix node bench:media
```
