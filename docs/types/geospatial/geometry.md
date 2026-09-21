# Geometry

Planar geospatial features: Well-Known Binary under a coordinate reference system, with straight lines between vertices.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Geometry(Arc<GeospatialParameters>)`, `GeometryField` / `GeometryType`, and the value `Geometry` behind `Scalar::Geometry` |
| Validates | The CRS at construction; the payload at the value door, read whole as WKB |
| Lazy | Nothing - the payload is read on the way in and never re-read to compare or hash |
| Cached | The Arrow projection of a [`Field`](../field.md); the payload is one shared `Arc<[u8]>` |
| Refuses | An empty CRS, an edge algorithm - a geometry's lines are straight, so it has none to state - and any payload that is not one whole geometry |
| Kind | `DataTypeKind::Geospatial`, id `geometry` (`0xb1`) |
| Bindings | The datatype and the field factory are in all three languages; the `Geometry` value is Rust only, and a cell crosses a binding as WKB bytes |

## DataType

`DataType::geometry(crs)` is the whole declaration: one optional coordinate
reference system, and nothing else. `None` fills `OGC:CRS84`
([the family's default](index.md#the-coordinate-reference-system)), which
displays as nothing, so `geometry` round-trips as itself and a parameter appears
exactly when it says something. The grammar takes `geometry`, `geometry()` and
`geometry('crs')`, in either quote; there is no logical name in front of it and
no second parameter, because an edge algorithm is [the geography's](geography.md).

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    // The bare spelling fills the default Parquet and Iceberg share.
    let geometry = DataType::geometry(None)?;
    assert_eq!(geometry.to_string(), "geometry");
    assert_eq!(geometry, DataType::geometry(Some("OGC:CRS84"))?);
    assert_eq!(geometry.kind(), DataTypeKind::Geospatial);
    assert_eq!(geometry.id(), DataTypeId::Geometry);
    assert_eq!(DataTypeId::Geometry.as_u8(), 0xb1);
    assert_eq!(DataType::from_str("geometry")?, geometry);
    assert_eq!(DataType::from_str("geometry()")?, geometry);

    // A stated reference system is quoted, and reads back as itself.
    let projected = DataType::geometry(Some("EPSG:3857"))?;
    assert_eq!(projected.to_string(), "geometry(\"EPSG:3857\")");
    assert_eq!(DataType::from_str("geometry('EPSG:3857')")?, projected);

    // A geometry has no edge algorithm, and an empty CRS names nothing.
    assert!(DataType::from_str("geometry('OGC:CRS84', 'vincenty')").is_err());
    assert!(DataType::geometry(Some("")).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    # The bare spelling fills the default Parquet and Iceberg share.
    geometry = DataType.geometry()
    assert geometry.id == "geometry"
    assert geometry.kind == "geospatial"
    assert str(geometry) == "geometry"
    assert geometry == DataType.geometry("OGC:CRS84")
    assert DataType("geometry") == geometry
    assert geometry.crs == "OGC:CRS84"
    assert geometry.edge_algorithm is None

    # A stated reference system is quoted, and reads back as itself.
    projected = DataType.geometry("EPSG:3857")
    assert str(projected) == 'geometry("EPSG:3857")'
    assert DataType(str(projected)) == projected
    assert not projected.has_default_crs

    # A geometry has no edge algorithm, and an empty CRS names nothing.
    with pytest.raises(ValueError, match="expected no edge algorithm"):
        DataType("geometry('OGC:CRS84', 'vincenty')")
    with pytest.raises(ValueError, match="expected a coordinate reference system"):
        DataType.geometry("")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // The bare spelling fills the default Parquet and Iceberg share.
    const geometry = DataType.geometry()
    assert.equal(geometry.id, 'geometry')
    assert.equal(geometry.kind, 'geospatial')
    assert.equal(geometry.toString(), 'geometry')
    assert.ok(DataType.geometry('OGC:CRS84').equals(geometry))
    assert.ok(new DataType('geometry').equals(geometry))

    // A stated reference system is quoted, and reads back as itself.
    const projected = DataType.geometry('EPSG:3857')
    assert.equal(projected.toString(), 'geometry("EPSG:3857")')
    assert.ok(DataType.fromString("geometry('EPSG:3857')").equals(projected))

    // A geometry has no edge algorithm, and an empty CRS names nothing.
    assert.throws(
      () => new DataType("geometry('OGC:CRS84', 'vincenty')"),
      /expected no edge algorithm/,
    )
    assert.throws(() => DataType.geometry(''), /expected a coordinate reference system/)
    ```

## Field

`GeometryField` is the typed marker - a field carrying `GeometryType`, whose
payload is the shared `GeospatialParameters`, so the CRS is read off the marker
rather than matched out of a root datatype. The bindings spell it
`types.geometry(name, crs)` and `fields.geometry(name, crs)`, nullable unless the
call says otherwise, with metadata riding beside the datatype as on every field.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, Field, GeometryField};

    let shape = GeometryField::try_new("shape", DataType::geometry(None)?, false)?;
    assert_eq!(shape.name(), "shape");
    assert_eq!(shape.id(), DataTypeId::Geometry);
    assert_eq!(shape.typed_dtype_ref().parameters().crs(), "OGC:CRS84");
    assert!(shape.typed_dtype_ref().parameters().has_default_crs());
    assert_eq!(shape.typed_dtype_ref().parameters().algorithm(), None);
    assert!(!shape.is_nullable());

    // Widened, it is the same column the root constructor builds.
    assert_eq!(
        shape.to_field(),
        Field::new("shape", DataType::geometry(None)?, false)
    );

    // A datatype from another family is refused by name.
    let refused = GeometryField::try_new("shape", DataType::binary(), true)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("geometry"), "{refused}");

    // Metadata rides beside the datatype, never inside it.
    let tagged = Field::from_parts(
        "shape",
        DataType::geometry(Some("EPSG:3857"))?,
        true,
        [("source", "feed")],
    )?;
    assert_eq!(tagged.get_metadata("source"), Some("feed"));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    shape = yggdryl.geometry("shape", nullable=False)
    assert isinstance(shape, Field)
    assert shape.name == "shape"
    assert shape.dtype == DataType.geometry()
    assert shape.nullable is False

    # The CRS is the factory's second argument.
    assert yggdryl.geometry("shape", "EPSG:3857").dtype == DataType.geometry("EPSG:3857")

    # Metadata rides beside the datatype, never inside it.
    tagged = yggdryl.geometry("shape", metadata={"source": "feed"})
    assert tagged.metadata["source"] == "feed"
    assert tagged.nullable is True
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    const shape = fields.geometry('shape', { nullable: false })
    assert.ok(shape instanceof Field)
    assert.equal(shape.name, 'shape')
    assert.ok(shape.dtype.equals(DataType.geometry()))
    assert.equal(shape.nullable, false)

    // The CRS is the factory's second argument.
    assert.ok(fields.geometry('shape', 'EPSG:3857').dtype.equals(DataType.geometry('EPSG:3857')))

    // Metadata rides beside the datatype, never inside it.
    const tagged = fields.geometry('shape', { metadata: { source: 'feed' } })
    assert.equal(tagged.get('source'), 'feed')
    assert.equal(tagged.nullable, true)
    ```

## Scalar

`Scalar::Geometry(Geometry)` is the value: the validated payload and nothing
else. The value door reads the bytes as WKB before it stores them, so a cell
that is not one whole geometry never becomes one, and the CRS stays on the
column - a cell read out of `geometry("EPSG:3857")` answers `geometry`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Geometry, Scalar};

    // A little-endian XY point: order byte, type code 1, then x and y.
    let mut point = vec![1_u8, 1, 0, 0, 0];
    point.extend(10.0_f64.to_le_bytes());
    point.extend(20.0_f64.to_le_bytes());

    // The value door reads the payload before it stores it.
    let value = DataType::geometry(Some("EPSG:3857"))?.scalar(point.clone())?;
    assert_eq!(value, Scalar::Geometry(Geometry::new(point.clone())?));
    assert_eq!(value.kind(), "geometry");
    assert_eq!(value.as_wkb(), Some(point.as_slice()));
    assert_eq!(value.dtype()?, DataType::geometry(None)?);
    assert!(DataType::geometry(None)?.scalar(vec![1_u8, 1, 0]).is_err());

    // The kind is part of the identity: the same bytes as a plain payload
    // are a different value, and order follows the bytes.
    assert_ne!(value, Scalar::from(point.clone()));
    assert!(Geometry::new(point)?.to_string().starts_with("0101000000"));
    ```

=== "Python"

    ```python
    import struct

    import pytest

    from yggdryl import DataType

    # A little-endian XY point: order byte, type code 1, then x and y.
    point = b"\x01\x01\x00\x00\x00" + struct.pack("<dd", 10.0, 20.0)

    value = DataType.geometry("EPSG:3857").scalar(point)
    assert value.as_py() == point
    assert value.kind == "geometry"
    assert value.family == "geospatial"
    # The CRS stays on the column; the cell is a geometry.
    assert value.dtype == DataType.geometry()

    with pytest.raises(ValueError, match="invalid wkb data"):
        DataType.geometry().scalar(b"\x01\x01\x00")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // A little-endian XY point: order byte, type code 1, then x and y.
    const point = Buffer.alloc(21)
    point[0] = 1
    point[1] = 1
    point.writeDoubleLE(10, 5)
    point.writeDoubleLE(20, 13)

    const value = DataType.geometry('EPSG:3857').scalar(point)
    assert.deepEqual(Buffer.from(value.asJs()), point)
    assert.equal(value.kind, 'geometry')
    assert.equal(value.family, 'geospatial')
    // The CRS stays on the column; the cell is a geometry.
    assert.equal(value.dtype.toString(), 'geometry')

    assert.throws(() => DataType.geometry().scalar(Buffer.from([1, 1, 0])), /invalid wkb data/)
    ```

`Geometry` itself is Rust only: a `#[repr(transparent)]` handle over one
`Arc<[u8]>`, sixteen bytes wide, validated once by `new` and borrowed with
`as_bytes`. `storage` hands back the shared handle, which is what lets a
[geography](geography.md) be spelled from the same payload without copying it,
and `Display` writes the payload as lowercase hex.

```rust
use yggdryl::{Geography, Geometry, GeospatialValue};

let mut point = vec![1_u8, 1, 0, 0, 0];
point.extend(1.5_f64.to_le_bytes());
point.extend(2.5_f64.to_le_bytes());

let value = Geometry::new(point.clone())?;
assert_eq!(value.as_bytes(), point.as_slice());
assert_eq!(std::mem::size_of::<Geometry>(), 16);

// The payload is already validated, so the other reading of it shares the
// handle rather than copying and re-reading the bytes.
let spherical = Geography::new(std::sync::Arc::clone(GeospatialValue::storage(&value)))?;
assert_eq!(spherical.as_bytes(), value.as_bytes());

// An invalid payload never becomes a value.
assert!(Geometry::new(vec![1_u8, 1, 0]).is_err());
```

## Arrow storage

A geometry column is a `Binary` array of WKB payloads, and the reading rides
beside it: `ARROW:extension:name` is `geoarrow.wkb` and
`ARROW:extension:metadata` is the GeoArrow document `{"crs":<crs>}`, with the
CRS always written, defaults included, so the projection never depends on what a
reader would fill. A geometry writes no `"edges"` key, and that absence is
exactly what makes the column import back as a geometry rather than as
[a geography](geography.md). The keys are transport: they never reach
[`Field`](../field.md) metadata.

| datatype | Arrow storage | extension name | document |
| --- | --- | --- | --- |
| `geometry` | `Binary` | `geoarrow.wkb` | `{"crs":"OGC:CRS84"}` |
| `geometry("EPSG:3857")` | `Binary` | `geoarrow.wkb` | `{"crs":"EPSG:3857"}` |

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let field = Field::new("shape", DataType::geometry(None)?, true);
    let arrow = field.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "geoarrow.wkb");
    assert_eq!(
        arrow.metadata()["ARROW:extension:metadata"],
        r#"{"crs":"OGC:CRS84"}"#
    );

    // The document is transport: it reimports as the datatype, not as metadata.
    let imported = Field::from_arrow_field(&arrow)?;
    assert_eq!(imported, field);
    assert!(imported.as_metadata().is_empty());

    // A bare `geoarrow.wkb` name, with no document at all, is the default
    // geometry - which is what a writer that filled the defaults wrote.
    let bare = arrow_schema::Field::new("shape", ArrowDataType::Binary, true).with_metadata(
        std::collections::HashMap::from([(
            "ARROW:extension:name".to_owned(),
            "geoarrow.wkb".to_owned(),
        )]),
    );
    assert_eq!(Field::from_arrow_field(&bare)?.dtype(), &DataType::geometry(None)?);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import DataType, Field

    projected = yggdryl.geometry("shape").into_arrow()
    assert projected.type == pa.binary()
    assert projected.metadata == {
        b"ARROW:extension:name": b"geoarrow.wkb",
        b"ARROW:extension:metadata": b'{"crs":"OGC:CRS84"}',
    }

    # The document is transport: it reimports as the datatype, not as metadata.
    imported = Field.from_arrow(projected)
    assert imported == yggdryl.geometry("shape")
    assert imported.dtype == DataType.geometry()
    assert dict(imported.metadata) == {}

    # A stated CRS travels in the document.
    assert yggdryl.geometry("shape", "EPSG:3857").into_arrow().metadata[
        b"ARROW:extension:metadata"
    ] == b'{"crs":"EPSG:3857"}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // A cast through a struct root answers the Arrow field a column is written as.
    const point = Uint8Array.from(Buffer.alloc(21, 0))
    point[0] = 1
    point[1] = 1
    const projected = (field) =>
      fields
        .struct('row', [field], { nullable: false })
        .castArrow(
          new arrow.Table({ [field.name]: arrow.vectorFromArray([point], new arrow.Binary()) }),
        ).schema.fields[0]

    const shape = projected(fields.geometry('shape'))
    assert.equal(shape.type.toString(), 'Binary')
    assert.equal(shape.metadata.get('ARROW:extension:name'), 'geoarrow.wkb')
    assert.equal(shape.metadata.get('ARROW:extension:metadata'), '{"crs":"OGC:CRS84"}')

    // A stated CRS travels in the document.
    assert.equal(
      projected(fields.geometry('shape', 'EPSG:3857')).metadata.get('ARROW:extension:metadata'),
      '{"crs":"EPSG:3857"}',
    )
    ```

## The WKB payload

The bytes are the simple-feature Well-Known Binary spelling, and the type code
says the shape and the axes. A geometry column takes all seven shapes, in either
byte order, with ISO or EWKB codes, and the reader that decides is
[the family's](index.md#the-wkb-reader) - it is not a geometry engine, so
nothing here measures, joins or reprojects.

```rust
use yggdryl::wkb::{self, Geometry as Shape};
use yggdryl::{DataType, Geometry};

// A little-endian linestring: order byte, type code 2, a vertex count,
// then the coordinates.
let mut line = vec![1_u8, 2, 0, 0, 0, 2, 0, 0, 0];
for value in [0.0_f64, 0.0, 3.0, 4.0] {
    line.extend(value.to_le_bytes());
}

let value = DataType::geometry(None)?.scalar(line.clone())?;
let bytes = value.as_wkb().unwrap();
assert_eq!(wkb::into_wkt(bytes)?, "LINESTRING (0 0, 3 4)");
assert_eq!(wkb::geometry_type_ids(bytes)?, [2]);
let bounds = wkb::bounding_box(bytes)?;
assert_eq!((bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax), (0.0, 3.0, 0.0, 4.0));
assert!(matches!(Shape::from_slice(bytes)?, Shape::LineString { .. }));

// Trailing bytes are not a second geometry: a cell is exactly what it decodes.
let mut doubled = line.clone();
doubled.extend_from_slice(&line);
assert!(Geometry::new(doubled).is_err());
```

## Edges

- `geometry('OGC:CRS84', 'vincenty')` -> refused at the algorithm's own position, `expected no edge algorithm for geometry`; [geography](geography.md) is the type whose edges take one.
- `DataType::geometry(Some(""))` -> refused; absent is `None`, which fills `OGC:CRS84`.
- A cell read out of `geometry("EPSG:3857")` -> a `geometry`: the reference system is the column's statement, never the value's.
- A payload that is not one whole geometry - truncated, trailing, or two payloads laid end to end -> refused naming the byte position.
- `Scalar::Bytes` holding WKB under a geometry field -> canonicalized to `Scalar::Geometry`; `as_wkb` reads both.
- A geometry and a [geography](geography.md) over the same bytes -> equal scalars, refused casts: the payload is shared, the reading is not ([the family](index.md#the-casts-the-two-share)).
- `geometry` -> `geometry("EPSG:3857")` as a cast -> refused naming both reference systems; a reprojection is not a cast.
- A `geoarrow.wkb` name over a storage that is not `Binary` -> a foreign field wearing our name, imported as that storage with the key kept as metadata.
- A caller-set `ARROW:extension:name` on a geometry field -> refused naming both the caller's name and `geoarrow.wkb`.
- A malformed GeoArrow document - `{"crs":7}` -> refused naming `ARROW:extension:metadata` and `crs`.
- The default value is `POINT EMPTY`, twenty-one bytes ([the family](index.md#the-default-value)).
- On a [Parquet](../../media/parquet/footer.md) path the column is `GEOMETRY` over `BYTE_ARRAY` WKB, and the defaults write as absent.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- geospatial:: cast::binary_bytes_entering_a_geometry_field cast::a_geometry_column cast::a_crs_change_between_geospatial_columns field::arrow::a_geometry_field field::arrow::a_bare_geoarrow_document scalar::a_geospatial_value
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^geospatial/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_factories.py -k "geometry"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="geometry" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    ```
