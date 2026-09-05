# Geospatial

Variant, geometry, and geography datatypes plus the dependency-free WKB reader behind display, casts, and bounds.

## Contract

| | |
| --- | --- |
| Owns | `variant`, `geometry(crs)`, `geography(crs, algorithm)`, `Scalar::Geospatial`, `types::geospatial::wkb` |
| Defaults | CRS `OGC:CRS84`; edges `spherical`; display omits defaults |
| Algorithms | `spherical`, `vincenty`, `thomas`, `andoyer`, `karney`; case-insensitive ([`EdgeAlgorithm`](scalar.md)) |
| Arrow | variant: struct of non-nullable `metadata`, `value` binaries under `arrow.parquet.variant`; pair: WKB binary under `geoarrow.wkb`, CRS and algorithm in GeoArrow JSON; both ride `ARROW:extension:name`/`ARROW:extension:metadata` |
| Variant value | the [`Scalar`](scalar.md) tree itself; no `Scalar::Variant` |
| WKB reader | Rust only; no dependency; no WKT parser |

## Use

Bare spellings fill the defaults [Parquet](../media/parquet.md) and [Iceberg](../media/iceberg/index.md) v3 share.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, EdgeAlgorithm};

    // Bare `variant` is the self-describing semi-structured datatype; the
    // parenthesis selects the dense-union sugar instead.
    let variant = DataType::variant();
    assert_eq!(variant.to_string(), "variant");
    assert_eq!(variant.kind(), DataTypeKind::Nested);
    assert_eq!(DataType::from_str("variant")?, variant);
    assert_eq!(DataType::from_str("variant(n:int64)")?.name(), "union");

    // The bare geospatial spellings fill the defaults Parquet and Iceberg
    // share - `OGC:CRS84`, and spherical edges for a geography - so a
    // parameter appears exactly when it says something.
    let geometry = DataType::geometry(None)?;
    assert_eq!(geometry.to_string(), "geometry");
    assert_eq!(geometry, DataType::geometry(Some("OGC:CRS84"))?);
    assert_eq!(geometry.kind(), DataTypeKind::Geospatial);
    assert_eq!(
        DataType::geometry(Some("EPSG:3857"))?.to_string(),
        "geometry(\"EPSG:3857\")"
    );

    let geography = DataType::geography(None, None)?;
    assert_eq!(geography.to_string(), "geography");
    assert_eq!(
        geography,
        DataType::geography(Some("OGC:CRS84"), Some(EdgeAlgorithm::Spherical))?
    );

    let vincenty = DataType::geography(None, Some(EdgeAlgorithm::Vincenty))?;
    assert_eq!(vincenty.to_string(), "geography(\"OGC:CRS84\",\"vincenty\")");
    assert_eq!(DataType::from_str("geography('OGC:CRS84', 'vincenty')")?, vincenty);

    // A geometry has no edge algorithm, and an empty CRS names nothing.
    assert!(DataType::from_str("geometry('OGC:CRS84', 'vincenty')").is_err());
    assert!(DataType::geometry(Some("")).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, types

    variant = DataType.variant()
    assert variant.id == "variant"
    assert variant.kind == "nested"
    assert str(variant) == "variant"
    assert DataType("variant") == variant
    assert DataType("variant(n:int64)").id == "union"
    assert types.variant("payload").dtype == variant

    geometry = DataType.geometry()
    assert str(geometry) == "geometry"
    assert geometry == DataType.geometry("OGC:CRS84")
    assert geometry.kind == "geospatial"
    assert str(DataType.geometry("EPSG:3857")) == 'geometry("EPSG:3857")'

    geography = DataType.geography()
    assert str(geography) == "geography"
    assert geography == DataType.geography("OGC:CRS84", "spherical")

    vincenty = DataType.geography("OGC:CRS84", "vincenty")
    assert str(vincenty) == 'geography("OGC:CRS84","vincenty")'
    assert DataType(str(vincenty)) == vincenty
    assert types.geography("region", "OGC:CRS84", "vincenty").dtype == vincenty

    with pytest.raises(ValueError, match="expected no edge algorithm"):
        DataType("geometry('OGC:CRS84', 'vincenty')")
    with pytest.raises(ValueError, match="expected one of spherical"):
        DataType.geography("OGC:CRS84", "euclidean")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const variant = DataType.variant()
    assert.equal(variant.id, 'variant')
    assert.equal(variant.kind, 'nested')
    assert.equal(variant.toString(), 'variant')
    assert.ok(new DataType('variant').equals(variant))
    assert.equal(DataType.fromString('variant(n:int64)').id, 'union')
    assert.ok(fields.variant('payload').dtype.equals(variant))

    const geometry = DataType.geometry()
    assert.equal(geometry.toString(), 'geometry')
    assert.ok(DataType.geometry('OGC:CRS84').equals(geometry))
    assert.equal(geometry.kind, 'geospatial')
    assert.equal(DataType.geometry('EPSG:3857').toString(), 'geometry("EPSG:3857")')

    const geography = DataType.geography()
    assert.equal(geography.toString(), 'geography')
    assert.ok(DataType.geography('OGC:CRS84', 'spherical').equals(geography))

    const vincenty = DataType.geography('OGC:CRS84', 'vincenty')
    assert.equal(vincenty.toString(), 'geography("OGC:CRS84","vincenty")')
    assert.ok(DataType.fromString(vincenty.toString()).equals(vincenty))
    assert.ok(
      fields.geography('region', 'OGC:CRS84', 'vincenty').dtype.equals(vincenty),
    )

    assert.throws(
      () => new DataType("geometry('OGC:CRS84', 'vincenty')"),
      /expected no edge algorithm/,
    )
    assert.throws(
      () => DataType.geography('OGC:CRS84', 'euclidean'),
      /expected one of spherical/,
    )
    ```

## The WKB reader

Rust only. Display, the [text cast](cast.md), and Parquet and Iceberg statistics share this decoder.

```rust
use yggdryl::types::geospatial::wkb::{self, Geometry};

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
```

`Geometry::from_slice` reads the seven simple-feature shapes in either byte order, with ISO (Z, M, ZM add 1000, 2000, 3000) or EWKB type codes. `bounding_box` folds min/max in one pass; `into_wkt` prints the shortest round-trip decimal.

```rust
use yggdryl::types::geospatial::wkb;

// Truncated input: the error names the byte position.
let error = wkb::bounding_box(&[1, 1, 0, 0, 0]).unwrap_err();
assert!(error.to_string().contains("byte 5"), "{error}");
```

## Edges

- `variant(n:int64)` -> the [dense-union sugar](nested.md), id `union`.
- `geometry('OGC:CRS84', 'vincenty')` -> refused, `expected no edge algorithm`.
- `DataType::geometry(Some(""))` -> refused; absent CRS is `None`.
- `geography('OGC:CRS84', 'euclidean')` -> refused, `expected one of spherical`.
- Variant value on a Parquet path -> refused by name until Iceberg v3.
- Truncated or trailing WKB bytes -> error naming the byte position.
- EWKB SRID -> read past, not modeled.
- `POINT EMPTY` -> NaN coordinates decode as `coordinate: None`.
- Empty geometry -> `bounding_box` is the fold identity; `BoundingBox::is_empty` skips the statistic.
- `Scalar::Bytes` holding WKB -> canonicalized to `Scalar::Geospatial`; `as_wkb` reads both.
- `geoarrow.wkb` -> community mapping; GeoArrow is not finalized.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::geospatial types::tests::semi_structured_and_geospatial
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- field::arrow::geospatial field::comparison::geospatial
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^geospatial/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/geospatial_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "variant or geometry"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="variant|geometry" node/tests/types/datatype.test.js
    ```
