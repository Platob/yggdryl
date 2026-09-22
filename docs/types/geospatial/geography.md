# Geography

Geospatial features on a sphere or a spheroid: the same Well-Known Binary, read with edges that follow the surface rather than a plane.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Geography(Arc<GeospatialParameters>)`, `GeographyField` / `GeographyType`, and the value `Geography` behind `Scalar::Geography` |
| Parameters | A coordinate reference system and an edge algorithm; `None` fills `OGC:CRS84` and `spherical` |
| Validates | The CRS at construction, the algorithm against the shared vocabulary; the payload at the value door, read whole as WKB |
| Lazy | Nothing - the payload is read on the way in and never re-read to compare or hash |
| Cached | The Arrow projection of a [`Field`](../field.md); the payload is one shared `Arc<[u8]>` |
| Refuses | An empty CRS and an algorithm outside `spherical`, `vincenty`, `thomas`, `andoyer`, `karney` |
| Kind | `DataTypeKind::Geospatial`, id `geography` (`0xb2`) |
| Bindings | The datatype and the field factory are in all three languages; the `Geography` value is Rust only, and a cell crosses a binding as WKB bytes |

## DataType

`DataType::geography(crs, algorithm)` takes both parameters, and `None` fills
each: `OGC:CRS84` and [`EdgeAlgorithm::Spherical`](#the-edge-algorithm), the
defaults Parquet and Iceberg v3 share. The algorithm field is never absent on a
geography and never present on a [geometry](geometry.md), so a bare `geography`
is a complete declaration. Both defaults display as nothing, and the algorithm
is written only when it is not `spherical` - `geography("EPSG:4326")` is a
non-default reference system read with default edges.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, EdgeAlgorithm};

    // The bare spelling fills both defaults.
    let geography = DataType::geography(None, None)?;
    assert_eq!(geography.to_string(), "geography");
    assert_eq!(
        geography,
        DataType::geography(Some("OGC:CRS84"), Some(EdgeAlgorithm::Spherical))?
    );
    assert_eq!(geography.kind(), DataTypeKind::Geospatial);
    assert_eq!(geography.id(), DataTypeId::Geography);
    assert_eq!(DataTypeId::Geography.as_u8(), 0xb2);
    assert_eq!(DataType::from_str("geography")?, geography);
    assert_eq!(DataType::from_str("geography()")?, geography);

    // A parameter is written exactly when it says something.
    let vincenty = DataType::geography(None, Some(EdgeAlgorithm::Vincenty))?;
    assert_eq!(vincenty.to_string(), "geography(\"OGC:CRS84\",\"vincenty\")");
    assert_eq!(DataType::from_str("geography('OGC:CRS84', 'vincenty')")?, vincenty);
    assert_eq!(
        DataType::geography(Some("EPSG:4326"), None)?.to_string(),
        "geography(\"EPSG:4326\")"
    );

    // An unknown algorithm reports the accepted vocabulary.
    assert!(DataType::from_str("geography('OGC:CRS84', 'euclidean')").is_err());
    assert!(DataType::geography(Some(""), None).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    # The bare spelling fills both defaults.
    geography = DataType.geography()
    assert geography.id == "geography"
    assert geography.kind == "geospatial"
    assert str(geography) == "geography"
    assert geography == DataType.geography("OGC:CRS84", "spherical")
    assert DataType("geography") == geography
    assert geography.crs == "OGC:CRS84"
    assert geography.edge_algorithm == "spherical"

    # A parameter is written exactly when it says something.
    vincenty = DataType.geography("OGC:CRS84", "vincenty")
    assert str(vincenty) == 'geography("OGC:CRS84","vincenty")'
    assert DataType(str(vincenty)) == vincenty
    assert str(DataType.geography("EPSG:4326")) == 'geography("EPSG:4326")'

    # An unknown algorithm reports the accepted vocabulary.
    with pytest.raises(ValueError, match="expected one of spherical"):
        DataType.geography("OGC:CRS84", "euclidean")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // The bare spelling fills both defaults.
    const geography = DataType.geography()
    assert.equal(geography.id, 'geography')
    assert.equal(geography.kind, 'geospatial')
    assert.equal(geography.toString(), 'geography')
    assert.ok(DataType.geography('OGC:CRS84', 'spherical').equals(geography))
    assert.ok(new DataType('geography').equals(geography))

    // A parameter is written exactly when it says something.
    const vincenty = DataType.geography('OGC:CRS84', 'vincenty')
    assert.equal(vincenty.toString(), 'geography("OGC:CRS84","vincenty")')
    assert.ok(DataType.fromString(vincenty.toString()).equals(vincenty))
    assert.equal(DataType.geography('EPSG:4326').toString(), 'geography("EPSG:4326")')

    // An unknown algorithm reports the accepted vocabulary.
    assert.throws(
      () => DataType.geography('OGC:CRS84', 'euclidean'),
      /expected one of spherical/,
    )
    ```

## Field

`GeographyField` is the typed marker - a field carrying `GeographyType`, whose
payload is the shared `GeospatialParameters`, so the CRS and the algorithm are
read off the marker rather than matched out of a root datatype. The bindings
spell it `yggdryl.geography(name, crs, algorithm)` and
`fields.geography(name, crs, algorithm)`, nullable unless the call says
otherwise.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, EdgeAlgorithm, Field, GeographyField};

    let region = GeographyField::try_new(
        "region",
        DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Vincenty))?,
        false,
    )?;
    assert_eq!(region.name(), "region");
    assert_eq!(region.id(), DataTypeId::Geography);
    assert_eq!(region.typed_dtype_ref().parameters().crs(), "EPSG:4326");
    assert_eq!(
        region.typed_dtype_ref().parameters().algorithm(),
        Some(EdgeAlgorithm::Vincenty)
    );
    assert!(!region.is_nullable());

    // A bare geography fills both defaults on the marker too.
    let bare = GeographyField::try_new("region", DataType::geography(None, None)?, true)?;
    assert!(bare.typed_dtype_ref().parameters().has_default_crs());
    assert_eq!(
        bare.typed_dtype_ref().parameters().algorithm(),
        Some(EdgeAlgorithm::Spherical)
    );
    assert_eq!(bare.to_field(), Field::new("region", DataType::geography(None, None)?, true));

    // A geometry is not a geography, and the refusal says so.
    let refused = GeographyField::try_new("region", DataType::geometry(None)?, true)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("geography"), "{refused}");
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    region = yggdryl.geography("region", "OGC:CRS84", "vincenty", nullable=False)
    assert isinstance(region, Field)
    assert region.name == "region"
    assert region.dtype == DataType.geography("OGC:CRS84", "vincenty")
    assert region.nullable is False

    # A bare geography fills both defaults.
    assert yggdryl.geography("region").dtype == DataType.geography()

    # Metadata rides beside the datatype, never inside it.
    tagged = yggdryl.geography("region", metadata={"source": "feed"})
    assert tagged.metadata["source"] == "feed"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    const region = fields.geography('region', 'OGC:CRS84', 'vincenty', { nullable: false })
    assert.ok(region instanceof Field)
    assert.equal(region.name, 'region')
    assert.ok(region.dtype.equals(DataType.geography('OGC:CRS84', 'vincenty')))
    assert.equal(region.nullable, false)

    // A bare geography fills both defaults.
    assert.ok(fields.geography('region').dtype.equals(DataType.geography()))

    // Metadata rides beside the datatype, never inside it.
    const tagged = fields.geography('region', { metadata: { source: 'feed' } })
    assert.equal(tagged.get('source'), 'feed')
    ```

## Scalar

`Scalar::Geography(Geography)` is the value, and it holds exactly what
[`Scalar::Geometry`](geometry.md#scalar) holds: the validated payload. The CRS
and the algorithm stay on the column, so a cell read out of
`geography("EPSG:4326","vincenty")` answers `geography`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, EdgeAlgorithm, Geography, Scalar};

    // A little-endian XY point: order byte, type code 1, then x and y.
    let mut point = vec![1_u8, 1, 0, 0, 0];
    point.extend(10.0_f64.to_le_bytes());
    point.extend(20.0_f64.to_le_bytes());

    let declared = DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Vincenty))?;
    let value = declared.scalar(point.clone())?;
    assert_eq!(value, Scalar::Geography(Geography::new(point.clone())?));
    assert_eq!(value.kind(), "geography");
    assert_eq!(value.as_wkb(), Some(point.as_slice()));
    assert_eq!(value.dtype()?, DataType::geography(None, None)?);
    assert!(declared.scalar(vec![1_u8, 1, 0]).is_err());

    // The payload is the value: the geometry reading of the same bytes is
    // an equal scalar, and the byte payload is not.
    assert_eq!(value, DataType::geometry(None)?.scalar(point.clone())?);
    assert_ne!(value, Scalar::from(point));
    ```

=== "Python"

    ```python
    import struct

    import pytest

    from yggdryl import DataType

    # A little-endian XY point: order byte, type code 1, then x and y.
    point = b"\x01\x01\x00\x00\x00" + struct.pack("<dd", 10.0, 20.0)

    value = DataType.geography("EPSG:4326", "vincenty").scalar(point)
    assert value.as_py() == point
    assert value.kind == "geography"
    assert value.family == "geospatial"
    # The parameters stay on the column; the cell is a geography.
    assert value.dtype == DataType.geography()

    with pytest.raises(ValueError, match="invalid wkb data"):
        DataType.geography().scalar(b"\x01\x01\x00")
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

    const value = DataType.geography('EPSG:4326', 'vincenty').scalar(point)
    assert.deepEqual(Buffer.from(value.asJs()), point)
    assert.equal(value.kind, 'geography')
    assert.equal(value.family, 'geospatial')
    // The parameters stay on the column; the cell is a geography.
    assert.equal(value.dtype.toString(), 'geography')

    assert.throws(() => DataType.geography().scalar(Buffer.from([1, 1, 0])), /invalid wkb data/)
    ```

## Arrow storage

The storage is the same `Binary` array of WKB under the same `geoarrow.wkb`
name; what differs is the document. A geography writes
`{"crs":<crs>,"edges":<algorithm>}`, and that `"edges"` key is the whole
distinction on the wire: a document carrying one imports as a geography, and one
without it as a [geometry](geometry.md). Both values are always written,
defaults included, so the projection never depends on what a reader would fill.

| datatype | Arrow storage | extension name | document |
| --- | --- | --- | --- |
| `geography` | `Binary` | `geoarrow.wkb` | `{"crs":"OGC:CRS84","edges":"spherical"}` |
| `geography("EPSG:4326","vincenty")` | `Binary` | `geoarrow.wkb` | `{"crs":"EPSG:4326","edges":"vincenty"}` |

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, EdgeAlgorithm, Field};

    let field = Field::new(
        "region",
        DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Vincenty))?,
        false,
    );
    let arrow = field.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "geoarrow.wkb");
    assert_eq!(
        arrow.metadata()["ARROW:extension:metadata"],
        r#"{"crs":"EPSG:4326","edges":"vincenty"}"#
    );
    assert_eq!(Field::from_arrow_field(&arrow)?, field);

    // The defaults are written too, so `"edges"` is always the distinction.
    let bare = Field::new("region", DataType::geography(None, None)?, true)
        .into_arrow_field()?;
    assert_eq!(
        bare.metadata()["ARROW:extension:metadata"],
        r#"{"crs":"OGC:CRS84","edges":"spherical"}"#
    );
    assert_eq!(
        Field::from_arrow_field(&bare)?.dtype(),
        &DataType::geography(None, None)?
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import DataType, Field

    projected = yggdryl.geography("region", "EPSG:4326", "vincenty").into_arrow()
    assert projected.type == pa.binary()
    assert projected.metadata == {
        b"ARROW:extension:name": b"geoarrow.wkb",
        b"ARROW:extension:metadata": b'{"crs":"EPSG:4326","edges":"vincenty"}',
    }
    assert Field.from_arrow(projected) == yggdryl.geography("region", "EPSG:4326", "vincenty")

    # The defaults are written too, so `"edges"` is always the distinction.
    bare = yggdryl.geography("region").into_arrow()
    assert bare.metadata[b"ARROW:extension:metadata"] == b'{"crs":"OGC:CRS84","edges":"spherical"}'
    assert Field.from_arrow(bare).dtype == DataType.geography()
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

    const region = projected(fields.geography('region', 'EPSG:4326', 'vincenty'))
    assert.equal(region.metadata.get('ARROW:extension:name'), 'geoarrow.wkb')
    assert.equal(
      region.metadata.get('ARROW:extension:metadata'),
      '{"crs":"EPSG:4326","edges":"vincenty"}',
    )

    // The defaults are written too, so `"edges"` is always the distinction.
    assert.equal(
      projected(fields.geography('region')).metadata.get('ARROW:extension:metadata'),
      '{"crs":"OGC:CRS84","edges":"spherical"}',
    )
    ```

## The edge algorithm

The second parameter says how an edge between two vertices is interpolated
across the surface. It is the one thing a geography states that a
[geometry](geometry.md) cannot, which is why the two are separate datatypes
rather than one with a flag. Five names are accepted, matched without regard to
case and displayed in the canonical lowercase; `spherical` is the default both
Parquet and Iceberg fill.

| name | edges |
| --- | --- |
| `spherical` | great-circle edges on a perfect sphere |
| `vincenty` | geodesic edges on a spheroid, by Vincenty's iterative formulae |
| `thomas` | geodesic edges by the Thomas cubic-series approximation |
| `andoyer` | geodesic edges by the Andoyer first-order approximation |
| `karney` | geodesic edges by Karney's exact algorithm |

This workspace stores and transports the statement; it computes no distances, so
the algorithm is carried to the reader that does.

=== "Rust"

    ```rust
    use yggdryl::{DataType, EdgeAlgorithm};

    // The canonical order, the default first.
    assert_eq!(EdgeAlgorithm::ALL.len(), 5);
    assert_eq!(EdgeAlgorithm::default(), EdgeAlgorithm::Spherical);
    assert_eq!(EdgeAlgorithm::Karney.as_str(), "karney");

    // Names fold case on the way in and canonicalize on the way out.
    assert_eq!(EdgeAlgorithm::from_str("VINCENTY")?, EdgeAlgorithm::Vincenty);
    assert_eq!(
        DataType::from_str("geography('OGC:CRS84', 'VINCENTY')")?.to_string(),
        "geography(\"OGC:CRS84\",\"vincenty\")"
    );

    // Anything else reports the whole vocabulary.
    let refused = EdgeAlgorithm::from_str("euclidean").unwrap_err().to_string();
    for name in EdgeAlgorithm::ALL {
        assert!(refused.contains(name.as_str()), "{refused}");
    }
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, enums

    # The canonical order, the default first.
    assert enums.EDGE_ALGORITHMS == (
        "spherical",
        "vincenty",
        "thomas",
        "andoyer",
        "karney",
    )
    assert DataType.geography().edge_algorithm == "spherical"

    # Names fold case on the way in and canonicalize on the way out.
    assert str(DataType.geography("OGC:CRS84", "VINCENTY")) == 'geography("OGC:CRS84","vincenty")'
    assert DataType.geography("OGC:CRS84", "karney").edge_algorithm == "karney"

    # Anything else reports the whole vocabulary.
    with pytest.raises(ValueError) as refused:
        DataType.geography("OGC:CRS84", "euclidean")
    assert all(name in str(refused.value) for name in enums.EDGE_ALGORITHMS)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // The default is spherical, and a name folds case on the way in.
    assert.equal(DataType.geography('OGC:CRS84', 'spherical').toString(), 'geography')
    assert.equal(
      DataType.geography('OGC:CRS84', 'VINCENTY').toString(),
      'geography("OGC:CRS84","vincenty")',
    )
    assert.equal(
      DataType.geography('OGC:CRS84', 'karney').toString(),
      'geography("OGC:CRS84","karney")',
    )

    // Anything else reports the whole vocabulary.
    assert.throws(
      () => DataType.geography('OGC:CRS84', 'euclidean'),
      /spherical, vincenty, thomas, andoyer, karney/,
    )
    ```

JavaScript has no listing of the five names; Rust reads them off
`EdgeAlgorithm::ALL` and Python off `enums.EDGE_ALGORITHMS`.

## Edges

- `geography('OGC:CRS84', 'euclidean')` -> refused, `expected one of spherical, vincenty, thomas, andoyer, karney`.
- `DataType::geography(Some(""), None)` -> refused; absent is `None`, which fills `OGC:CRS84`.
- A geography is never without an algorithm: `None` fills `spherical`, so `algorithm()` is `Some` on every geography and `None` on every [geometry](geometry.md).
- `geography("EPSG:4326")` -> a stated reference system with default edges; the algorithm is written only when it is not `spherical`.
- A cell read out of `geography("EPSG:4326","vincenty")` -> a `geography`: the parameters are the column's statement, never the value's.
- A geography and a [geometry](geometry.md) over the same bytes -> equal scalars, refused casts, because changing the edge model is a statement about the coordinates ([the family](index.md#the-casts-the-two-share)).
- A GeoArrow document with no `"edges"` key -> a geometry; the key, not the name, is what separates the two on the wire.
- `{"edges":7}` or an unknown algorithm in a document -> refused naming `ARROW:extension:metadata`.
- The default value is `POINT EMPTY`, the same twenty-one bytes a geometry defaults to ([the family](index.md#the-default-value)).
- On a [Parquet](../../media/parquet/footer.md) path the column is `GEOGRAPHY` over `BYTE_ARRAY` WKB, and the defaults write as absent.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cast::typed edge_algorithm field::arrow geospatial wkb::bounds wkb::empties wkb::ewkb wkb::exactness wkb::identity wkb::nesting wkb::reading wkb::refusals wkb::type_ids
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/geospatial_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "geography"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="geography" node/tests/datatype.test.js node/tests/fields.test.js
    ```
