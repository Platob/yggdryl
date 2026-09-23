# Geospatial

Two datatypes over one payload - planar `geometry` and spherical `geography`, both Well-Known Binary - and the dependency-free reader every display, cast and statistic goes through.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Geometry` and `DataType::Geography`, each carrying a shared `GeospatialParameters`; the values `Geometry` and `Geography`, the `GeospatialValue` contract both answer, and the `wkb` reader beside them |
| Constructors | `DataType::geometry(crs)` and `DataType::geography(crs, algorithm)`; `None` fills the defaults, so the bare spelling is the common column |
| Defaults | CRS `OGC:CRS84`, edges `spherical` - the defaults [Parquet](../../media/index.md#parquet) and [Iceberg](../../media/index.md#iceberg) v3 share; the display omits them |
| Validates | The CRS at construction - never empty; the payload at the value door, by reading it whole as WKB |
| Lazy | Nothing - a payload is read once, on the way in, and never re-read to be compared or hashed |
| Cached | The Arrow projection of a [`Field`](../field.md); the payload is one shared `Arc<[u8]>`, so restating a geometry as a geography clones a handle rather than copying bytes |
| Kinds | `DataTypeKind::Geospatial`, the range `0xb0..=0xbf`; ids `geometry` (`0xb1`) and `geography` (`0xb2`) |
| Arrow | A `Binary` column of WKB under the `geoarrow.wkb` extension name, with the CRS and the edges in a GeoArrow JSON document on `ARROW:extension:metadata` |
| Refuses | An empty CRS, an edge algorithm on a geometry, an algorithm outside the vocabulary, a payload the reader cannot read whole, and text on the way in - there is no WKT parser |
| Bindings | The datatypes and the field factories are in all three languages; `Geometry`, `Geography`, `GeospatialValue` and the `wkb` module are Rust only, and a value crosses a binding as plain WKB bytes |

## Pages

| page | owns |
| --- | --- |
| [Geometry](geometry.md) | `geometry(crs)`, `GeometryField`, `Scalar::Geometry`, the planar reading of the payload and the `{"crs":...}` document |
| [Geography](geography.md) | `geography(crs, algorithm)`, `GeographyField`, `Scalar::Geography`, the five edge algorithms and the `{"crs":...,"edges":...}` document |

The bare `variant` spelling is parsed beside these two in the grammar and is not
geospatial: its value and its encoding are [the variant's](../variant.md).

## The value both hold

The payload is the value and the datatype is the reading of it. Both types store
the same validated bytes, so a cell reads back as WKB in every language, and the
column - not the cell - says whether those bytes are planar or spherical.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    // A little-endian XY point: order byte, type code 1, then x and y.
    let mut point = vec![1_u8, 1, 0, 0, 0];
    point.extend(10.0_f64.to_le_bytes());
    point.extend(20.0_f64.to_le_bytes());

    let planar = DataType::geometry(None)?.scalar(point.clone())?;
    let spherical = DataType::geography(None, None)?.scalar(point.clone())?;
    assert_eq!(planar.kind(), "geometry");
    assert_eq!(spherical.kind(), "geography");
    assert_eq!(planar.as_wkb(), Some(point.as_slice()));
    assert_eq!(spherical.as_wkb(), Some(point.as_slice()));

    // The bytes are the value, so the two readings of one payload compare
    // equal - and neither is the same value as the plain byte payload.
    assert_eq!(planar, spherical);
    assert_ne!(planar, Scalar::from(point));
    ```

=== "Python"

    ```python
    import struct

    from yggdryl import DataType

    # A little-endian XY point: order byte, type code 1, then x and y.
    point = b"\x01\x01\x00\x00\x00" + struct.pack("<dd", 10.0, 20.0)

    planar = DataType.geometry().scalar(point)
    spherical = DataType.geography().scalar(point)
    assert planar.kind == "geometry"
    assert spherical.kind == "geography"
    assert planar.family == spherical.family == "geospatial"
    assert planar.as_py() == spherical.as_py() == point
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

    const planar = DataType.geometry().scalar(point)
    const spherical = DataType.geography().scalar(point)
    assert.equal(planar.kind, 'geometry')
    assert.equal(spherical.kind, 'geography')
    assert.equal(planar.family, 'geospatial')
    assert.equal(spherical.family, 'geospatial')
    assert.deepEqual(Buffer.from(planar.asJs()), point)
    assert.deepEqual(Buffer.from(spherical.asJs()), point)
    ```

Rust names that shared payload once: `GeospatialValue` is the leaf contract
both answer - the bytes and the shared handle behind them. The family is the
geospatial range of identifiers, not a type: each value is its own `Scalar`
variant, and the leaf's `from_scalar` borrows it back out
([Scalar](../scalar.md#families)). Neither crosses a binding.

```rust
use yggdryl::{DataTypeKind, Geometry, GeospatialValue, Scalar, Value};

let mut point = vec![1_u8, 1, 0, 0, 0];
point.extend(10.0_f64.to_le_bytes());
point.extend(20.0_f64.to_le_bytes());

let value = Geometry::new(point.clone())?;
assert_eq!(GeospatialValue::as_bytes(&value), point.as_slice());

// The family is a range of identifiers; the value is its leaf's own variant.
let held = value.clone().into_scalar();
assert_eq!(held, Scalar::Geometry(value.clone()));
assert_eq!(held.family(), DataTypeKind::Geospatial);
assert!(DataTypeKind::Geospatial.contains(held.id()));
assert_eq!(Value::dtype(&value)?.kind(), DataTypeKind::Geospatial);
assert_eq!(<Geometry as Value>::from_scalar(&held), Some(&value));
assert_eq!(<Geometry as Value>::from_scalar(&Scalar::from(1_i64)), None);
```

## The coordinate reference system

Both types carry one, and `None` fills `OGC:CRS84` - longitude/latitude on
WGS 84, the default Parquet's `GEOMETRY`/`GEOGRAPHY` logical types and Iceberg
v3's geospatial types share. The default displays as nothing, so a parameter
appears exactly when it says something, and an empty string is refused rather
than filled: absence is spelled `None`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, EdgeAlgorithm};

    // Bare is the default, and the default round-trips as itself.
    assert_eq!(DataType::geometry(None)?, DataType::geometry(Some("OGC:CRS84"))?);
    assert_eq!(DataType::geometry(None)?.to_string(), "geometry");
    assert_eq!(DataType::geography(None, None)?.to_string(), "geography");

    // A stated CRS is quoted, on either type.
    assert_eq!(
        DataType::geometry(Some("EPSG:3857"))?.to_string(),
        "geometry(\"EPSG:3857\")"
    );
    assert_eq!(
        DataType::geography(Some("EPSG:4326"), None)?.to_string(),
        "geography(\"EPSG:4326\")"
    );
    assert_eq!(
        DataType::geography(Some("OGC:CRS84"), Some(EdgeAlgorithm::Vincenty))?.to_string(),
        "geography(\"OGC:CRS84\",\"vincenty\")"
    );

    // An empty reference system names nothing, so it is refused, not filled.
    assert!(DataType::geometry(Some("")).is_err());
    assert!(DataType::geography(Some(""), None).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    # Bare is the default, and the default round-trips as itself.
    assert DataType.geometry() == DataType.geometry("OGC:CRS84")
    assert str(DataType.geometry()) == "geometry"
    assert str(DataType.geography()) == "geography"

    # The parameters read back one at a time.
    assert DataType.geometry().crs == "OGC:CRS84"
    assert DataType.geometry().has_default_crs
    assert not DataType.geometry("EPSG:4326").has_default_crs
    assert DataType.geography().edge_algorithm == "spherical"
    assert DataType.geometry().edge_algorithm is None
    assert DataType("int64").crs is None

    # A stated CRS is quoted, on either type.
    assert str(DataType.geometry("EPSG:3857")) == 'geometry("EPSG:3857")'
    assert str(DataType.geography("EPSG:4326")) == 'geography("EPSG:4326")'

    # An empty reference system names nothing, so it is refused, not filled.
    with pytest.raises(ValueError, match="expected a coordinate reference system"):
        DataType.geometry("")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // Bare is the default, and the default round-trips as itself.
    assert.ok(DataType.geometry().equals(DataType.geometry('OGC:CRS84')))
    assert.equal(DataType.geometry().toString(), 'geometry')
    assert.equal(DataType.geography().toString(), 'geography')

    // A stated CRS is quoted, on either type.
    assert.equal(DataType.geometry('EPSG:3857').toString(), 'geometry("EPSG:3857")')
    assert.equal(DataType.geography('EPSG:4326').toString(), 'geography("EPSG:4326")')

    // An empty reference system names nothing, so it is refused, not filled.
    assert.throws(() => DataType.geometry(''), /expected a coordinate reference system/)
    ```

Reading the parameters back is Rust and Python: Rust matches the datatype and
asks `GeospatialParameters` for `crs`, `has_default_crs` and `algorithm`, Python
reads `crs`, `has_default_crs` and `edge_algorithm` off the datatype, and
JavaScript reads the canonical spelling instead.

## The WKB reader

Rust only. Display, the [text cast](#the-casts-the-two-share), and Parquet and
Iceberg statistics share this decoder, and nothing here needs a geometry engine.
`Geometry::from_slice` reads the seven simple-feature shapes in either byte
order, with ISO (Z, M, ZM add 1000, 2000, 3000) or EWKB type codes;
`bounding_box` folds min/max in one pass without materializing a geometry; and
`into_wkt` prints the shortest round-trip decimal. There is deliberately no WKT
*parser*: the workspace displays and bounds geometries, it does not accept text
geometry input.

```rust
use yggdryl::wkb::{self, Geometry};

// A little-endian XY point: order byte, type code 1, then x and y.
let mut point = vec![1, 1, 0, 0, 0];
point.extend(10.0_f64.to_le_bytes());
point.extend(20.0_f64.to_le_bytes());

let decoded = Geometry::from_slice(&point)?;
assert_eq!(decoded.clone().into_wkt(), "POINT (10 20)");
assert_eq!(decoded.type_id(), 1);
assert!(!decoded.is_empty());

// The free functions answer without materializing the geometry.
assert_eq!(wkb::into_wkt(&point)?, "POINT (10 20)");
assert_eq!(wkb::geometry_type_ids(&point)?, [1]);
let bounds = wkb::bounding_box(&point)?;
assert_eq!((bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax), (10.0, 10.0, 20.0, 20.0));

// Truncated input: the error names the byte position.
let error = wkb::bounding_box(&point[..5]).unwrap_err();
assert!(error.to_string().contains("byte 5"), "{error}");
```

The dimensionality travels with the geometry rather than with its coordinates,
which is what keeps `POINT ZM EMPTY` distinguishable from `POINT EMPTY` once the
coordinates are gone. `geometry_type_ids` reports the distinct ISO codes a
payload holds, sorted, which is the vocabulary
[Parquet's footer](../../media/index.md#parquet) records.

```rust
use yggdryl::wkb::{self, Dimensions, Geometry};

// `POINT EMPTY` has no zero-count spelling, so WKB writes NaN coordinates:
// the reader gives back an absent coordinate rather than a NaN one.
let mut empty = vec![1_u8, 1, 0, 0, 0];
empty.extend(f64::NAN.to_le_bytes());
empty.extend(f64::NAN.to_le_bytes());
let decoded = Geometry::from_slice(&empty)?;
assert_eq!(decoded, Geometry::Point { dimensions: Dimensions::Xy, coordinate: None });
assert!(decoded.is_empty());
assert_eq!(wkb::into_wkt(&empty)?, "POINT EMPTY");

// An empty geometry bounds nothing, and the box says so instead of storing
// the fold identity as a statistic.
assert!(wkb::bounding_box(&empty)?.is_empty());

// The axes are the geometry's, and ISO spells them in the type code.
assert!(Dimensions::new(true, true).has_z());
assert!(Dimensions::Xyzm.has_m());
assert!(!Dimensions::Xy.has_z());
```

## The casts the two share

One tier for both, on [Cast](../cast.md): bytes entering either column are read
as WKB and refused by field and row when they are not one whole geometry; either
column renders as text through the reader; and neither crosses into the other,
because a CRS or an edge model is a statement about the coordinates and not a
layout the bytes can be re-read under.

| from | to | result |
| --- | --- | --- |
| `Binary` | `geometry` / `geography` | validated as WKB, same bytes |
| `geometry` / `geography` | `utf8`, `large_utf8`, `utf8_view` | WKT, rendered per cell |
| `geometry` / `geography` | `binary` | the payload, lossless |
| `geometry` | `geography`, or either under another CRS | refused, naming both readings |
| `utf8` | `geometry` / `geography` | refused, naming the absent WKT parser |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, BinaryArray, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    // A little-endian XY point: order byte, type code 1, then x and y.
    let mut point = vec![1_u8, 1, 0, 0, 0];
    point.extend(1.0_f64.to_le_bytes());
    point.extend(2.0_f64.to_le_bytes());
    let strict = ArrowCastOptions::new().with_safe(false);

    // Bytes entering the column are read as WKB; a truncated payload names
    // the field, the row and what the reader wanted.
    let shape = Field::new("shape", DataType::geometry(None)?, true);
    let source: ArrayRef = Arc::new(BinaryArray::from(vec![Some(point.as_slice()), None]));
    let stored = Serie::from_arrow_array(Some(&shape), source, strict)?;
    assert_eq!(stored.as_binary().expect("a WKB column").value(0), Some(point.as_slice()));
    let broken: ArrayRef = Arc::new(BinaryArray::from(vec![Some([1_u8, 1, 0].as_slice())]));
    let refused = Serie::from_arrow_array(Some(&shape), broken, strict).unwrap_err().to_string();
    assert!(refused.contains("shape") && refused.contains("row 0"), "{refused}");

    // The column renders as WKT through the same reader.
    let text = stored.cast(&Field::new("shape", DataType::utf8(), true), strict)?;
    assert_eq!(text.as_utf8().expect("a utf8 column").value(0), Some("POINT (1 2)"));
    assert!(text.is_null(1)?);

    // Text has no way in: the reader decodes, it does not parse.
    let words: ArrayRef = Arc::new(StringArray::from(vec!["POINT (1 2)"]));
    let refused = Serie::from_arrow_array(Some(&shape), words, strict).unwrap_err().to_string();
    assert!(refused.contains("WKT parser"), "{refused}");
    ```

=== "Python"

    ```python
    import struct

    import pyarrow as pa
    import pytest

    import yggdryl

    from yggdryl import Serie

    # A little-endian XY point: order byte, type code 1, then x and y.
    point = b"\x01\x01\x00\x00\x00" + struct.pack("<dd", 1.0, 2.0)

    # Bytes entering the column are read as WKB; a truncated payload names
    # the field and the row.
    shape = yggdryl.geometry("shape")
    stored = Serie.from_arrow_array(pa.array([point, None], pa.binary()), shape)
    assert stored.into_arrow_array().to_pylist() == [point, None]
    with pytest.raises(ValueError, match="row 0"):
        Serie.from_arrow_array(pa.array([b"\x01\x01\x00"], pa.binary()), shape, safe=False)

    # The column renders as WKT through the same reader.
    text = stored.cast(yggdryl.utf8("shape"), safe=False)
    assert text.as_py() == ["POINT (1 2)", None]

    # Text has no way in: the reader decodes, it does not parse.
    with pytest.raises(ValueError, match="WKT parser"):
        Serie.from_arrow_array(pa.array(["POINT (1 2)"]), shape, safe=False)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    // A little-endian XY point: order byte, type code 1, then x and y.
    const point = Buffer.alloc(21)
    point[0] = 1
    point[1] = 1
    point.writeDoubleLE(1, 5)
    point.writeDoubleLE(2, 13)
    const payload = arrow.vectorFromArray([Uint8Array.from(point)], new arrow.Binary())

    // Bytes entering the column are read as WKB; a truncated payload names
    // the field and the row.
    const shape = fields.geometry('shape')
    const stored = Serie.fromArrowArray(payload, shape)
    assert.deepEqual(Buffer.from(stored.intoArrowArray().get(0)), point)
    assert.throws(
      () =>
        Serie.fromArrowArray(
          arrow.vectorFromArray([Uint8Array.of(1, 1, 0)], new arrow.Binary()),
          shape,
          { safe: false },
        ),
      /row 0/,
    )

    // The column renders as WKT through the same reader.
    const text = stored.cast(fields.utf8('shape'), { safe: false })
    assert.equal(text.intoArrowArray().get(0), 'POINT (1 2)')

    // Text has no way in: the reader decodes, it does not parse.
    assert.throws(
      () =>
        Serie.fromArrowArray(arrow.vectorFromArray(['POINT (1 2)'], new arrow.Utf8()), shape, {
          safe: false,
        }),
      /WKT parser/,
    )
    ```

## The default value

Both types default to `POINT EMPTY` - twenty-one bytes of little-endian WKB
whose coordinates are the conventional NaNs - because a geospatial column's
present-but-empty value is a geometry that bounds nothing, not an empty payload
the reader would refuse.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    let default = DataType::geometry(None)?.default_value()?;
    assert!(matches!(default, Scalar::Geometry(_)));
    assert_eq!(default.as_wkb().map(<[u8]>::len), Some(21));
    assert!(DataType::geometry(None)?.is_default_value(&default)?);
    assert_eq!(
        yggdryl::wkb::into_wkt(default.as_wkb().unwrap())?,
        "POINT EMPTY"
    );

    // The geography defaults to the same empty geometry, read spherically.
    let spherical = DataType::geography(None, None)?.default_value()?;
    assert!(matches!(spherical, Scalar::Geography(_)));
    assert_eq!(spherical.as_wkb(), default.as_wkb());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    default = DataType.geometry().default_scalar()
    assert default.kind == "geometry"
    assert len(default.as_py()) == 21
    assert DataType.geometry().is_default_value(default)

    # The geography defaults to the same empty geometry, read spherically.
    spherical = DataType.geography().default_scalar()
    assert spherical.kind == "geography"
    assert spherical.as_py() == default.as_py()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // The default is read off the datatype, as the WKB bytes themselves.
    const value = Buffer.from(DataType.geometry().defaultJSValue())
    assert.equal(value.length, 21)
    assert.equal(value.readUInt32LE(1), 1)
    assert.ok(Number.isNaN(value.readDoubleLE(5)))
    assert.ok(Number.isNaN(value.readDoubleLE(13)))

    // The geography defaults to the same empty geometry, read spherically.
    assert.deepEqual(Buffer.from(DataType.geography().defaultJSValue()), value)
    ```

Rust and Python answer with a `Scalar`, whose kind names the column it came
from; `defaultJSValue` hands JavaScript the payload alone, which is what a
geospatial cell crosses a binding as.

## Edges

- `DataType::geometry(Some(""))` -> refused; absent is `None`, which fills `OGC:CRS84`.
- `geometry('OGC:CRS84', 'vincenty')` -> refused, `expected no edge algorithm`; straight planar lines need none.
- `geography('OGC:CRS84', 'euclidean')` -> refused, `expected one of spherical, vincenty, thomas, andoyer, karney`.
- Truncated or trailing WKB bytes -> error naming the byte position; two payloads laid end to end are not one geometry.
- Collections nesting past `DataType::PARSE_RECURSION_LIMIT` -> refused by both the decoder and the streaming bounds pass.
- EWKB SRID -> read past, not modeled: bounds and text are the same in every reference system.
- `POINT EMPTY` -> NaN coordinates decode as `coordinate: None`, and the dimension marker survives: `POINT ZM EMPTY`.
- An empty geometry -> `bounding_box` is the fold identity, and `BoundingBox::is_empty` names it so a statistics writer can skip the box.
- `Scalar::Bytes` holding WKB under either field -> canonicalized to `Scalar::Geometry` or `Scalar::Geography`; `as_wkb` reads all three spellings.
- The same payload as a geometry and as a geography -> equal scalars; as a plain byte value -> a different value, because the kind is part of the identity.
- A geospatial value in arithmetic -> refused: it reads as bytes, but two WKB payloads do not join.
- `bytes_parameters` on either type -> `None`; a geospatial value is bytes with an identity, like a [UUID](../uuid.md) ([Strings & bytes](../text/index.md)).
- Row digests hash a geospatial value like any other payload ([Hashing](../../hashing.md)).
- `geoarrow.wkb` -> a community mapping; GeoArrow's own documents say it is not finalized, so the spelling is revisitable.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cast::typed diff::comparison geospatial wkb::bounds wkb::empties wkb::ewkb wkb::exactness wkb::identity wkb::nesting wkb::reading wkb::refusals wkb::type_ids
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^geospatial/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/geospatial_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "geometry or geography"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="geometry|geography" node/tests/datatype.test.js node/tests/fields.test.js
    ```
