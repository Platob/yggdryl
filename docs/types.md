# Types

Datatypes, fields, scalar values, and their shared vocabulary live in one type layer.

## Datatypes


`yggdryl::types` holds `DataType`, the owned logical type of one value - every Arrow logical
type, with Arrow itself kept out of the value model.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let value = DataType::from_str("decimal(18, 4)")?;
    assert_eq!(value, DataType::decimal64(18, 4)?);

    // Display is canonical and both text forms round-trip.
    assert_eq!(value.to_string(), "decimal64(18,4)");
    assert_eq!(DataType::from_str(&value.to_string())?, value);
    assert_eq!(DataType::from_json(&value.clone().into_json()?)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    value = DataType("decimal(18, 4)")
    assert value == DataType.decimal(18, 4)

    assert str(value) == "decimal64(18,4)"
    assert DataType(str(value)) == value
    assert DataType.from_json(value.into_json()) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const value = DataType.from('decimal(18, 4)')
    assert.equal(value.id, 'decimal64')

    assert.equal(value.toString(), 'decimal64(18,4)')
    assert.ok(DataType.fromString(value.toString()).equals(value))
    assert.ok(DataType.fromJSON(value.toJSON()).equals(value))
    ```

There are 57 variants: one per Arrow logical type, plus Variant, the geospatial pair, UUID, Version,
the six ASCII widths, and the four registered codes, which cross Arrow as extension-typed storage.
The parser accepts the Arrow,
SQL, Hive, and Spark spellings of all of them - `bigint`, `varchar(255)`, `array<string>`,
`row(...)`, `double precision` - and normalizes to one canonical form, so `to_string` is a
losslessly re-parseable value rather than a debug rendering. `into_json` is a separate, structural
encoding: tagged objects that name every parameter, which is what a schema written to disk should
be.

Scalar variants are inline and nested children sit behind shared allocations, so cloning a
`DataType` never allocates. The value is immutable; changing a type means building another one.

### Children

Subscripting a `DataType` reaches a nested child - by name or by position - which is exactly what
subscripting a [`Field`](types.md#item-access-reaches-a-child-never-metadata) does, so one semantic
covers every node in a schema graph. A `DataType` is immutable and hashable, so it is a *read-only*
child collection: assignment and deletion belong on the `Field` that carries it, which owns the
cache-aware mutation.


=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let quote = DataType::from_fields([
        Field::new("symbol", DataType::Utf8, false),
        Field::new("levels", DataType::list(DataType::Float64.nullable_field("item")), true),
    ])?;

    assert_eq!(quote.field_len(), 2);
    assert_eq!(quote.get_field(0).map(Field::name), Some("symbol"));
    assert_eq!(quote.get_field_by_path("levels").unwrap().dtype().field_len(), 1);
    assert!(quote.get_field_by_path("missing").is_none());

    // Every child-bearing type answers the same two questions.
    let lookup = DataType::map_of(DataType::Utf8, DataType::Int64, true)?;
    assert_eq!(lookup.field_len(), 1);
    assert_eq!(lookup.get_field(0).map(Field::name), Some("entries"));
    assert!(lookup.as_fields().is_none() && quote.as_fields().is_some());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, types

    quote = DataType.from_fields([
        Field("symbol", "utf8", nullable=False),
        types.list("levels", types.float64("item")),
    ])

    assert len(quote) == 2
    assert [field.name for field in quote] == ["symbol", "levels"]
    assert quote[0].name == "symbol"
    assert quote[-1].name == "levels"
    assert len(quote["levels"].dtype) == 1
    assert "levels" in quote and "missing" not in quote

    lookup = types.map_of("lookup", "utf8", "int64", keys_sorted=True).dtype
    assert len(lookup) == 1
    assert lookup[0].name == "entries"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const quote = DataType.fromFields([
      fields.utf8('symbol'),
      fields.list('levels', fields.float64('item'), { nullable: true }),
    ])

    assert.equal(quote.length, 2)
    assert.deepEqual(quote.keys(), ['symbol', 'levels'])
    assert.equal(quote.getFieldAt(-1).name, 'levels')
    assert.equal(quote.getField('levels').dtype.length, 1)
    assert.equal(quote.contains('missing'), false)
    assert.deepEqual([...quote].map((field) => field.name), ['symbol', 'levels'])

    const lookup = fields.mapOf('lookup', 'utf8', 'int64', true).dtype
    assert.equal(lookup.length, 1)
    assert.equal(lookup.getFieldAt(0).name, 'entries')
    ```

Children are positional and read-only. A list has one child, a map has one - its `entries` struct -
a run-end encoding has two, and a struct or union has as many as it declares, so a walker never
branches on the variant to descend. `as_fields` is the exception: it returns a borrowed slice only
for a struct, because only a struct's children are a flat named collection.

Construction rejects duplicate names in a struct, so `from_fields` can fail. The Python and
JavaScript `fields` factories build a [`Field`](types.md) rather than a bare type, which is what
nesting actually needs - a child is a name, a type, and a nullability flag.

### Precision and resolution pick the width

=== "Rust"

    ```rust
    use yggdryl::{DataType, TimeUnit};

    assert_eq!(DataType::decimal(38, 4)?, DataType::decimal128(38, 4)?);
    assert_eq!(DataType::decimal(39, 4)?, DataType::decimal256(39, 4)?);
    assert_eq!(DataType::time(TimeUnit::Second)?, DataType::Time32(TimeUnit::Second));
    assert_eq!(DataType::time(TimeUnit::Nanosecond)?, DataType::Time64(TimeUnit::Nanosecond));

    // Out-of-range parameters and mismatched unit categories are refused.
    assert!(DataType::decimal(2, 3).is_err());
    assert!(DataType::decimal128(39, 0).is_err());
    assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
    assert!(DataType::time(TimeUnit::YearMonth).is_err());
    assert!(DataType::fixed_size_binary(-1).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
    assert DataType.decimal(39, 4) == DataType("decimal256(39,4)")
    assert DataType.time("s") == DataType("time32(s)")
    assert DataType.time("nano seconds") == DataType("time64(ns)")

    with pytest.raises(ValueError, match="positive scale cannot exceed precision"):
        DataType.decimal(2, 3)
    with pytest.raises(ValueError, match="temporal resolution"):
        DataType.time("year_month")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    assert.equal(fields.decimal('amount', 38, 4).dtype.toString(), 'decimal128(38,4)')
    assert.equal(fields.decimal('wide', 39, 4).dtype.toString(), 'decimal256(39,4)')
    assert.equal(DataType.time('s').toString(), 'time32(s)')
    assert.equal(DataType.time('nano seconds').toString(), 'time64(ns)')

    assert.throws(() => fields.decimal('bad', 2, 3), /positive scale cannot exceed precision/)
    assert.throws(() => DataType.time('year_month'), /temporal resolution/)
    ```

`decimal` and `time` are selectors, not extra types: precision 1-9 lands on `decimal32`, 10-18 on
`decimal64`, 19-38 on `decimal128`, and 39-76 on `decimal256`; seconds and milliseconds land on
`time32`, and micro/nanoseconds on `time64`. The exact constructors stay available for when the
physical width is part of the contract. Whichever you call, the parameters are validated once at
construction - a precision of zero, a positive scale larger than the precision, a nanosecond
`time32`, or a negative fixed width never becomes a value.

Units are parsed from the shared [`TimeUnit`](types.md) vocabulary, so `s`, `sec`, `MILLIS`, `µs`,
and `nano seconds` all work, and the calendar interval layouts - which are not time-of-day
resolutions - are rejected here rather than silently accepted. JavaScript has no
`DataType.decimal`; decimals arrive through the `fields` factories.

### Encodings that wrap a value

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, Field};

    let codes = DataType::dictionary(DataType::Int16, DataType::Utf8)?;
    let runs = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Utf8, true),
    )?;

    assert_eq!(codes.kind(), DataTypeKind::Nested);
    assert_eq!(runs.kind(), DataTypeKind::Nested);
    // A wrapper reports the shape of what it encodes, not of its own storage.
    assert!(!codes.is_nested() && !runs.is_nested());

    let DataType::Dictionary(dictionary) = &codes else { panic!("dictionary") };
    assert_eq!(dictionary.key(), &DataType::Int16);
    assert_eq!(dictionary.value(), &DataType::Utf8);

    // The key must be an integer; run ends must be a non-null int16, int32, or int64.
    assert!(DataType::dictionary(DataType::Utf8, DataType::Utf8).is_err());
    assert!(DataType::run_end_encoded(
        Field::new("run_ends", DataType::UInt32, false),
        Field::new("values", DataType::Utf8, true),
    ).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import types

    codes = types.dictionary("codes", "int16", "utf8").dtype
    runs = types.run_end_encoded(
        "runs",
        types.int16("run_ends", nullable=False),
        types.utf8("values"),
    ).dtype

    assert codes.kind == "nested"
    assert runs.kind == "nested"
    assert not codes.is_nested and not runs.is_nested
    assert str(codes) == "dictionary(int16,utf8)"

    with pytest.raises(ValueError, match="integer key datatype"):
        types.dictionary("bad", "utf8", "utf8")
    with pytest.raises(ValueError, match="int16, int32, or int64"):
        types.run_end_encoded(
            "bad", types.uint32("run_ends", nullable=False), types.utf8("values")
        )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const codes = fields.dictionary('codes', 'int16', 'utf8').dtype
    const runs = fields
      .runEndEncoded('runs', fields.int16('run_ends', { nullable: false }), fields.utf8('values'))
      .dtype

    assert.equal(codes.kind, 'nested')
    assert.equal(runs.kind, 'nested')
    assert.equal(codes.nested, false)
    assert.equal(runs.nested, false)
    assert.equal(codes.toString(), 'dictionary(int16,utf8)')

    assert.throws(() => fields.dictionary('bad', 'utf8', 'utf8'), /integer key datatype/)
    assert.throws(
      () => fields.runEndEncoded('bad', fields.uint32('run_ends', { nullable: false }), fields.utf8('values')),
      /int16, int32, or int64/,
    )
    ```

A dictionary and a run-end encoding are storage decisions, not logical shapes: a dictionary of
`utf8` still holds strings. That is why `is_nested` resolves through them and answers for the
encoded value - a dictionary of strings is not nested, a dictionary of structs is - while `kind`
still reports the wrapper. Dictionary ordering and the Arrow dictionary id live on the
[`Field`](types.md), not here, because two fields can share a type and disagree about them.

Run-end children keep Arrow's names and constraints: `run_ends` must be non-null and one of the
three signed widths, while `values` carries the actual type and its own nullability.

### Unions and the dense-union sugar

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, UnionMode};

    let members = [
        Field::new("number", DataType::Int64, false),
        Field::new("text", DataType::Utf8, true),
    ];
    let members_union = DataType::dense_union(members.clone())?;

    // The member sugar is not a second logical type: it is the dense union
    // with IDs 0.. - and bare `variant` is a datatype of its own, so the
    // parenthesis is what disambiguates the two spellings.
    assert_eq!(
        members_union,
        DataType::union(
            [(0, members[0].clone()), (1, members[1].clone())],
            UnionMode::Dense,
        )?
    );
    assert_eq!(members_union.name(), "union");
    assert!(members_union.to_string().starts_with("union(dense,"));
    assert_eq!(DataType::from_str("variant(number:int64,text:string)")?.name(), "union");
    assert_eq!(DataType::from_str("variant")?, DataType::Variant);

    // An explicit union picks its own mode and its own non-negative type IDs.
    let sparse = DataType::union(
        [(7, Field::new("only", DataType::Int32, false))],
        UnionMode::Sparse,
    )?;
    assert_eq!(sparse.get_field(0).map(Field::name), Some("only"));
    assert!(DataType::union(
        [(0, members[0].clone()), (0, members[1].clone())],
        UnionMode::Dense,
    ).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field

    variant = DataType.variant([
        Field("number", "int64", nullable=False),
        Field("text", "utf8"),
    ])

    assert variant.id == "union"
    assert str(variant).startswith("union(dense,")
    assert [field.name for field in variant] == ["number", "text"]
    assert DataType("variant(number:int64,text:string)").id == "union"

    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.variant([Field("same", "int64"), Field("same", "utf8")])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const variant = DataType.variant([
      fields.int64('number'),
      fields.utf8('text', { nullable: true }),
    ])

    assert.equal(variant.id, 'union')
    assert.ok(variant.toString().startsWith('union(dense,'))
    assert.deepEqual(variant.keys(), ['number', 'text'])
    assert.equal(DataType.from('variant(number:int64,text:string)').id, 'union')

    assert.throws(
      () => DataType.variant([fields.int64('same'), fields.utf8('same')]),
      /duplicate field name/,
    )
    ```

A union pairs each member with an explicit Arrow type ID; the sugar is the case where those IDs are
just `0..` in declaration order, so a caller lists members and nothing else. It is not a second
logical type: `DataType::dense_union` builds a `DataType::Union` with `UnionMode::Dense`, and
display, JSON, Arrow projection, and defaults all go through the union contract - which is why the
canonical form reads `union(dense, ...)`. In Python and JavaScript the sugar is the member-taking
call `DataType.variant(fields)` and the `fields.dense_union` / `fields.denseUnion` factories (the
typed alias is `DenseUnionField`); an explicit mode goes through the `fields.union` factories
there, and through `DataType::union` in Rust.

The `variant` spelling collides with a datatype of its own, and the parenthesis is the whole
disambiguation: bare `variant` is the self-describing [Variant datatype](#variant-geometry-and-geography),
while `variant(...)` with members stays this dense-union input sugar - in the grammar and in the
`DataType.variant` call of both bindings alike.

Type IDs are `i8` and must be unique and non-negative, so the sugar caps at 128 members. The parser
accepts `variant(...)`, `dense_union(...)`, and `sparse_union(...)` and canonicalizes all of them.

### Variant, geometry, and geography

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

A variant is a self-describing tree: each value declares its own types, which is why the datatype
takes no parameters - shredding is a physical layout, not part of the logical type - and why the
grammar's bare `variant` parses to it while `variant(...)` stays the
[dense-union sugar](#unions-and-the-dense-union-sugar) above. A variant *value* is the one
[`Scalar`](types.md) model; its Parquet binary encoding lands with the Iceberg v3 layer, so paths
that would need it today refuse by name.

The geospatial pair shares one parameter value: a coordinate reference system, and - only on a
geography - the edge interpolation. Bare `geometry` and `geography` fill the defaults Parquet's
`GEOMETRY`/`GEOGRAPHY` logical types and Iceberg v3 share (`OGC:CRS84`, spherical edges), and
display emits a parameter exactly when it differs from that default, so every spelling round-trips
through `from_str`. An empty CRS is refused - the absent spelling is `None`, which fills the
default - and a geometry given an edge algorithm is refused by name: straight planar lines need
none. The algorithm vocabulary is the five canonical lowercase names of
[`EdgeAlgorithm`](types.md#shared-vocabulary) - `spherical`, `vincenty`, `thomas`, `andoyer`,
`karney` - parsed case-insensitively; the Python and JavaScript `algorithm` arguments accept those
strings.

Across an Arrow boundary the three are extension-typed storage, as are the
[ASCII widths](#ascii-widths-and-the-registered-codes) below: a variant is a struct of
non-nullable `metadata` and `value` binaries under the canonical `arrow.parquet.variant` extension
name, and both geospatial types are WKB bytes under the community `geoarrow.wkb` name, whose
GeoArrow JSON document carries the CRS and, for a geography, the edge algorithm. The identities
ride the `ARROW:extension:name` / `ARROW:extension:metadata` field-metadata keys. GeoArrow's own
documentation says the specification is not finalized, so the `geoarrow.wkb` mapping is a
community choice that may be revisited when it stabilizes. The geospatial *values* travel as
Well-Known Binary through [`Scalar::Geospatial`](types.md#the-wkb-reader), read back for display
and bounds by the one WKB reader documented there.

### ASCII widths and the registered codes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{ArrowCast, DataType, DataTypeKind, Field, Scalar};

    // Two shapes: text of any length, and text padded to one fixed width.
    assert_eq!(DataType::from_str("ascii")?, DataType::Ascii);
    assert_eq!(DataType::ascii(3)?, DataType::FixedAscii(3));
    assert_eq!(DataType::from_str("ascii(12)")?, DataType::FixedAscii(12));
    assert_eq!(DataType::FixedAscii(4).to_string(), "ascii(4)");
    assert_eq!(DataType::Ascii.kind(), DataTypeKind::Ascii);
    assert_eq!(DataType::FixedAscii(8).ascii_width(), Some(8));
    // Variable ASCII has no width to report, and neither has anything else.
    assert_eq!(DataType::Ascii.ascii_width(), None);
    assert_eq!(DataType::Utf8.ascii_width(), None);
    assert!(DataType::ascii(0).is_err());

    // A registered code is a datatype, not a name over a width: it stores the
    // width its standard fixes and displays as itself.
    let currency = DataType::currency();
    assert_eq!(DataType::from_str("currency")?, currency);
    assert_eq!(currency.to_string(), "currency");
    assert_eq!(currency.ascii_width(), Some(3));
    assert_ne!(currency, DataType::FixedAscii(3));
    assert_eq!(currency.kind(), DataTypeKind::Ascii);
    assert_eq!(
        DataType::CODES,
        &[
            ("country", DataType::Country, 2),
            ("currency", DataType::Currency, 3),
            ("mic", DataType::Mic, 4),
            // Six bytes: `cfi` stores what it is, not the eight some other
            // width would pad it to.
            ("cfi", DataType::Cfi, 6),
        ]
    );

    // A code rides its own Arrow extension, so the identity survives the trip.
    let venue = Field::new("venue", DataType::Mic, false);
    let arrow = venue.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
    assert_eq!(Field::from_arrow(&arrow)?, venue);

    // Storage pads to the width; every string rendering trims the padding.
    let ccy = Field::new("ccy", DataType::FixedAscii(4), false);
    let stored = scalar_array(&ccy, &Scalar::from("USD"))?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), b"USD\0");
    assert_eq!(
        scalar_value(&ccy, stored.as_ref())?,
        DataType::FixedAscii(4).scalar("USD")?
    );

    // The Arrow field is `fixed_size_binary(4)` under the `yggdryl.ascii` name.
    let arrow = ccy.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ascii");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow(&arrow)?, ccy);

    // The variable form is the same extension over Arrow's `Binary`: no width,
    // so no padding, and the storage is the bytes the value is.
    let note = Field::new("note", DataType::Ascii, false);
    let arrow = note.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ascii");
    assert_eq!(Field::from_arrow(&arrow)?, note);
    let free = scalar_array(&note, &Scalar::from("a note of any length at all"))?;
    let free = free.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(free.value(0), b"a note of any length at all");

    // A cast into the width pads; the stored column read under `utf8` trims.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EU"]));
    let padded = ccy.cast_arrow_array(text, false)?;
    let bytes = padded.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(1), b"EU\0\0");
    let row = DataType::from_fields([ccy.clone()])?.required_field("row");
    let batch = RecordBatch::try_new(row.into_arrow_schema()?, vec![padded])?;
    let text = DataType::from_fields([DataType::Utf8.required_field("ccy")])?.required_field("row");
    let trimmed = text.cast_arrow_batch(batch, false)?;
    let trimmed = trimmed.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(trimmed.value(1), "EU");

    // A width merged with the variable form drops the width, and either
    // merged with text is text.
    assert_eq!(
        DataType::FixedAscii(4).merge_with(&DataType::Ascii, true)?,
        DataType::Ascii
    );
    assert_eq!(DataType::Ascii.merge_with(&DataType::Utf8, true)?, DataType::Utf8);

    let long: ArrayRef = Arc::new(StringArray::from(vec!["EURO!"]));
    let refused = ccy.cast_arrow_array(long, false).unwrap_err().to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, types

    # Two shapes: text of any length, and text padded to one fixed width.
    note = DataType("ascii")
    ascii32 = DataType.ascii(4)
    assert note.id == "ascii"
    assert ascii32.id == "fixed_ascii"
    assert str(ascii32) == "ascii(4)"
    assert DataType("ascii(12)") == DataType.ascii(12)
    assert ascii32.kind == note.kind == "ascii"
    assert ascii32.ascii_width == 4
    assert DataType.ascii(12).ascii_width == 12
    # Variable ASCII has no width to report, and neither has anything else.
    assert note.ascii_width is None
    assert DataType("utf8").ascii_width is None
    assert types.fixed_ascii("ccy", 3).dtype == DataType.ascii(3)
    assert types.ascii("note").dtype == note

    # A registered code is a datatype, not a name over a width: it stores the
    # width its standard fixes and displays as itself.
    currency = DataType("currency")
    assert str(currency) == "currency"
    assert currency.ascii_width == 3
    assert currency != DataType.ascii(3)
    assert currency.kind == "ascii"
    assert [(DataType(name).id, DataType(name).ascii_width) for name in
            ("country", "currency", "mic", "cfi")] == [
        ("country", 2), ("currency", 3), ("mic", 4), ("cfi", 6)
    ]

    # A code rides its own Arrow extension, so the identity survives the trip.
    venue = types.mic("venue", nullable=False)
    venue_arrow = venue.into_arrow()
    assert venue_arrow.type == pa.binary(4)
    assert venue_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
    assert Field.from_arrow(venue_arrow) == venue

    # Storage pads to the width; every string rendering trims the padding.
    ccy = Field("ccy", ascii32, nullable=False)
    assert ccy.arrow_scalar("USD") == pa.scalar(b"USD\x00", pa.binary(4))
    assert ccy.default_pyvalue() == ""

    # The Arrow field is `fixed_size_binary(4)` under the `yggdryl.ascii` name.
    arrow = ccy.into_arrow()
    assert arrow.type == pa.binary(4)
    assert arrow.metadata == {
        b"ARROW:extension:name": b"yggdryl.ascii",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow) == ccy

    # The variable form is the same extension over Arrow's variable binary: no
    # width, so no padding, and the storage is the bytes the value is.
    free = types.ascii("note", nullable=False)
    free_arrow = free.into_arrow()
    assert free_arrow.type == pa.binary()
    assert free_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.ascii"
    assert Field.from_arrow(free_arrow) == free
    assert free.arrow_scalar("a note of any length at all") == pa.scalar(
        b"a note of any length at all", pa.binary()
    )

    # A cast into the width pads; the stored column read under `utf8` trims.
    padded = ccy.cast_arrow_array(pa.array(["USD", "EU"]))
    assert padded.to_pylist() == [b"USD\x00", b"EU\x00\x00"]
    stored = pa.record_batch([padded], schema=pa.schema([arrow]))
    text = DataType.from_fields([types.utf8("ccy")])
    assert text.cast_arrow_batch(stored).column(0).to_pylist() == ["USD", "EU"]

    # A width merged with the variable form drops the width, and either merged
    # with text is text.
    assert ascii32.merge_with(note) == note
    assert note.merge_with("utf8") == DataType("utf8")

    with pytest.raises(ValueError, match="at most 4 bytes"):
        ccy.cast_arrow_array(pa.array(["EURO!"]))
    with pytest.raises(ValueError, match="at least 1 byte, got 0"):
        DataType.ascii(0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, fields } = require('yggdryl')

    // Two shapes: text of any length, and text padded to one fixed width.
    const note = new DataType('ascii')
    const ascii32 = DataType.ascii(4)
    assert.equal(note.id, 'ascii')
    assert.equal(ascii32.id, 'fixed_ascii')
    assert.equal(ascii32.toString(), 'ascii(4)')
    assert.ok(DataType.from('ascii(12)').equals(DataType.ascii(12)))
    assert.equal(ascii32.kind, 'ascii')
    assert.equal(ascii32.asciiWidth, 4)
    // Variable ASCII has no width to report, and neither has anything else.
    assert.equal(note.asciiWidth, null)
    assert.equal(new DataType('utf8').asciiWidth, null)
    assert.ok(fields.fixedAscii('ccy', 3).dtype.equals(DataType.ascii(3)))
    assert.equal(fields.ascii('note').dtype.id, 'ascii')

    // A registered code is a datatype, not a name over a width: it stores the
    // width its standard fixes and displays as itself.
    const currency = new DataType('currency')
    assert.equal(currency.id, 'currency')
    assert.equal(currency.toString(), 'currency')
    assert.equal(currency.asciiWidth, 3)
    assert.ok(!currency.equals(DataType.ascii(3)))
    assert.deepEqual(
      ['country', 'currency', 'mic', 'cfi'].map((name) => new DataType(name).asciiWidth),
      [2, 3, 4, 6],
    )

    // A code rides its own Arrow extension, so the identity survives the trip.
    const venue = fields.struct('row', [fields.mic('venue', { nullable: false })], {
      nullable: false,
    })
    const venueArrow = venue.castArrow(
      new arrow.Table({ venue: arrow.vectorFromArray(['XPAR'], new arrow.Utf8()) }),
    ).schema.fields[0]
    assert.equal(String(venueArrow.type), 'FixedSizeBinary[4]')
    assert.equal(venueArrow.metadata.get('ARROW:extension:name'), 'yggdryl.mic')

    // Storage pads to the width; every string rendering trims the padding.
    const row = fields.struct('row', [fields.fixedAscii('ccy', 4, { nullable: false })], {
      nullable: false,
    })
    assert.equal(row.getField('ccy').defaultJSValue(), '')
    const codes = (values) =>
      new arrow.Table({ ccy: arrow.vectorFromArray(values, new arrow.Utf8()) })
    const stored = row.castArrow(codes(['USD', 'EU']))
    assert.deepEqual([...stored.getChild('ccy').get(1)], [0x45, 0x55, 0, 0])

    // The Arrow field is `FixedSizeBinary[4]` under the `yggdryl.ascii` name,
    // and a column carrying that identity reads under `utf8` as trimmed text.
    const field = stored.schema.fields[0]
    assert.equal(String(field.type), 'FixedSizeBinary[4]')
    assert.equal(field.metadata.get('ARROW:extension:name'), 'yggdryl.ascii')
    const text = fields.struct('row', [fields.utf8('ccy', { nullable: false })], {
      nullable: false,
    })
    assert.deepEqual([...text.castArrow(stored).getChild('ccy')], ['USD', 'EU'])

    // The variable form is the same extension over Arrow's `Binary`: no width,
    // so no padding, and the storage is the bytes the value is.
    const notes = fields.struct('row', [fields.ascii('note', { nullable: false })], {
      nullable: false,
    })
    const free = notes.castArrow(
      new arrow.Table({
        note: arrow.vectorFromArray(['a note of any length at all'], new arrow.Utf8()),
      }),
    )
    assert.equal(String(free.schema.fields[0].type), 'Binary')
    assert.equal(
      Buffer.from(free.getChild('note').get(0)).toString(),
      'a note of any length at all',
    )

    // A width merged with the variable form drops the width, and either merged
    // with text is text.
    assert.equal(ascii32.mergeWith(note).id, 'ascii')
    assert.equal(note.mergeWith('utf8').id, 'utf8')

    assert.throws(() => row.castArrow(codes(['EURO!'])), /ASCII text of at most 4 bytes/)
    assert.throws(() => DataType.ascii(0), /at least 1 byte, got 0/)
    ```

ASCII is two datatypes. `ascii` is text of any length, stored as the bytes it is; `ascii(n)` is that
same text padded with trailing NUL to exactly `n` bytes, for any `n` of at least one byte. The fixed
form is what the codes of the trade take - two bytes for a country, three for a currency, four for a
venue, six for a CFI code, twelve for an ISIN - and it buys a column of fixed-size slots, a packed
integer, and a schema that says how long a value may be; the variable form buys none of those and
takes a value of any length. A value is ASCII - every byte at most `0x7F` - with no NUL byte, and at
most the width when there is one; storage pads it to exactly that width, and every string rendering
trims the padding, so a column reads back as the text that went in. The canonical scalar is the
trimmed string; bytes and a string carrying trailing NULs are accepted on the way in under the same
rule and canonicalize to it. `ascii_width` answers the storage width, `None` for the variable form,
and a value that is not ASCII, holds a NUL, or is longer than the width is refused naming the width
and, in a cast, the row.

`country`, `currency`, `mic` and `cfi` are datatypes of their own, not names over a width. Each is
one published registry - ISO 3166-1 alpha-2, ISO 4217, ISO 10383, ISO 10962 - with exactly one
storage width: two, three, four and six bytes. `CODES` is that listing, `code_name` answers a code's
identity, and `is_code` distinguishes the four from the widths. The value contract is unchanged: a
code answers `ascii_width`, so `ascii_packed` and `AsciiEnum` work over it with nothing added, and
an enum member is still the integer its value packs into. What a code adds over the width that would
hold it is identity and a constant - the identity crosses Arrow, and the width is known at compile
time, so the ingest and render paths are monomorphized per code rather than reading a length out of
the datatype on every row. Six bytes is the point of `cfi`: it stores what a CFI code is rather than
padding into some wider slot.

Across Arrow the two ASCII shapes share the `yggdryl.ascii` extension name with an empty document,
and the storage says which: `binary` is the variable form, `fixed_size_binary(n)` is the width `n`.
A code is `fixed_size_binary(2|3|4|6)` under its own `yggdryl.country`, `yggdryl.currency`,
`yggdryl.mic` or `yggdryl.cfi`, because the name is what says which registry the bytes belong to.
Three bytes under `yggdryl.currency` read back a currency and three bytes under `yggdryl.ascii` read
back an `ascii(3)`; a plain binary or fixed binary, or one carrying a document, imports as it is.
Two schemas that agree on a code keep it, and a code [merged](types.md#merging-two-schemas) with
anything else answers the plain text both fit in - a currency beside a country is `ascii(3)`, never
one standard's code carrying the other's values; a width beside the variable form is the variable
form, which is the narrowest shape holding both. Every other exchange sees text: Iceberg, Spark,
Polars, and pandas [rewrite](#compatibility-rewriting) either shape to `string`/`utf8`, Avro writes
`string`, and a filter such as `ccy = 'USD'` meets the literal at `utf8`. One boundary shows storage:
JavaScript's `readRecords` hands out Arrow JS rows, which carry no extension identity, so an ASCII
column arrives there as its stored bytes; read it under a declared `utf8` field, as above, for the
text.

#### The declared vocabulary and its generated enum

=== "Rust"

    ```rust
    use yggdryl::{AsciiEnum, DataType, Field};

    // A value's integer is its own storage bytes read big-endian, so it is the
    // same integer in every process and orders exactly as the text does.
    assert_eq!(DataType::FixedAscii(4).ascii_packed(b"USD")?, 0x5553_4400);
    assert_eq!(DataType::FixedAscii(4).ascii_packed(b"USD\0")?, 0x5553_4400);
    assert_eq!(DataType::FixedAscii(4).ascii_value(0x5553_4400)?, "USD");
    assert_eq!(DataType::Currency.ascii_packed(b"USD")?, 0x0055_5344);
    // Sixteen bytes fill the whole `i128`; a wider width has no packed code.
    assert_eq!(
        DataType::FixedAscii(16).ascii_packed(b"US0378331005")?,
        0x5553_3033_3738_3333_3130_3035_0000_0000
    );
    assert!(DataType::FixedAscii(17).ascii_packed(b"US").is_err());
    assert!(DataType::Ascii.ascii_packed(b"US").is_err());

    // An enum is that naming as a value: one ASCII value per member name.
    let venues = AsciiEnum::from_members("Venue", [("XNAS", "XNAS"), ("N_A", "n/a")])?;
    assert_eq!(venues.get("N_A"), Some("n/a"));
    assert_eq!(
        venues.into_members(&DataType::Mic)?,
        [("N_A".into(), 0x6E2F_6100), ("XNAS".into(), 0x584E_4153)]
    );

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert_eq!(AsciiEnum::member_name("n/a").as_str(), "N_A");

    // The ISO listings ship with the package, so a code column declares the
    // vocabulary it draws from without a copy per language.
    let currencies = AsciiEnum::from_logical_name("currency")?;
    assert_eq!(currencies.len(), AsciiEnum::CURRENCIES.len());
    assert_eq!(currencies.get("USD"), Some("USD"));
    assert_eq!(AsciiEnum::from_logical_name("Exchange")?.len(), AsciiEnum::MICS.len());
    // A registered name with no listing answers an enum of no members.
    assert!(AsciiEnum::from_logical_name("tenor")?.is_empty());

    // A field declares the enum its values name, as one metadata document, so
    // the enum crosses Arrow and comes back the enum that was written.
    let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
    let field = Field::new("side", DataType::FixedAscii(4), false).try_with_ascii_enum(&side)?;
    assert_eq!(side.into_members(&DataType::FixedAscii(4))?[0], ("BUY".into(), 0x4200_0000));
    assert_eq!(Field::from_arrow(&field.into_arrow()?)?.ascii_enum()?, Some(side));
    ```

=== "Python"

    ```python
    import enum

    import pytest

    from yggdryl import AsciiEnum, DataType, Field

    # A value's integer is its own storage bytes read big-endian, so it is the
    # same integer in every process and orders exactly as the text does.
    ascii32 = DataType.ascii(4)
    assert ascii32.ascii_packed("USD") == 0x55534400
    assert ascii32.ascii_packed("USD\x00") == 0x55534400
    assert ascii32.ascii_value(0x55534400) == "USD"
    assert DataType("currency").ascii_packed("USD") == 0x555344
    # Sixteen bytes fill the whole 128-bit integer, which Python holds natively.
    assert DataType.ascii(16).ascii_packed("US0378331005") == (
        0x55533033373833333130303500000000
    )
    with pytest.raises(ValueError, match="at most 16 bytes"):
        DataType("ascii").ascii_packed("US")

    # An enum is that naming as a value: one ASCII value per member name.
    venues = AsciiEnum("Venue", {"XNAS": "XNAS", "N_A": "n/a"})
    assert venues.get("N_A") == "n/a"
    assert venues.into_members("mic") == [("N_A", 0x6E2F6100), ("XNAS", 0x584E4153)]

    # ... and as a Python `IntEnum`, keyed by the same integers.
    Venue = venues.into_intenum("mic")
    assert issubclass(Venue, enum.IntEnum)
    assert Venue(0x584E4153).name == "XNAS"

    # The same rule names one value at a time, for a vocabulary declared
    # member by member rather than generated from a whole listing.
    assert AsciiEnum.member_name("n/a") == "N_A"

    # The ISO listings ship with the package, so a code column declares the
    # vocabulary it draws from without a copy per language.
    currencies = AsciiEnum.from_logical_name("currency")
    assert len(currencies) == len(AsciiEnum.prebuilt()["currency"])
    assert currencies.get("USD") == "USD"
    assert len(AsciiEnum.from_logical_name("Exchange")) == len(AsciiEnum.prebuilt()["mic"])
    # A registered name with no listing answers an enum of no members.
    assert len(AsciiEnum.from_logical_name("tenor")) == 0

    # A field declares the enum its values name, as one metadata document, so
    # the enum crosses Arrow and comes back the enum that was written.
    side = AsciiEnum("Side", {"BUY": "B", "SELL": "S"})
    field = Field("side", ascii32, nullable=False)
    field.set_ascii_enum(side)
    assert side.into_members(ascii32)[0] == ("BUY", 0x42000000)
    assert Field.from_arrow(field.into_arrow()).ascii_enum == side
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { AsciiEnum, DataType, Field } = require('yggdryl')

    // A value's integer is its own storage bytes read big-endian, so it is the
    // same integer in every process and orders exactly as the text does.
    const ascii32 = DataType.ascii(4)
    assert.equal(ascii32.asciiPacked('USD'), 0x55534400n)
    assert.equal(ascii32.asciiPacked('USD\0'), 0x55534400n)
    assert.equal(ascii32.asciiValue(0x55534400n), 'USD')
    assert.equal(new DataType('currency').asciiPacked('USD'), 0x555344n)
    // Sixteen bytes fill the whole 128-bit integer, so every code is a bigint.
    assert.equal(
      DataType.ascii(16).asciiPacked('US0378331005'),
      0x55533033373833333130303500000000n,
    )
    assert.throws(() => new DataType('ascii').asciiPacked('US'), /at most 16 bytes/)

    // An enum is that naming as a value: one ASCII value per member name.
    const venues = new AsciiEnum('Venue', { XNAS: 'XNAS', N_A: 'n/a' })
    assert.equal(venues.get('N_A'), 'n/a')
    assert.deepEqual(venues.intoMembers('mic'), { XNAS: 0x584e4153n, N_A: 0x6e2f6100n })

    // ... and as the generated enum: a frozen name-to-code object, tagged with
    // the enum's own name.
    const Venue = venues.intoEnum('mic')
    assert.equal(Venue.XNAS, new DataType('mic').asciiPacked('XNAS'))
    assert.equal(Object.prototype.toString.call(Venue), '[object Venue]')

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert.equal(AsciiEnum.memberName('n/a'), 'N_A')

    // The ISO listings ship with the package, so a code column declares the
    // vocabulary it draws from without a copy per language.
    const currencies = AsciiEnum.fromLogicalName('currency')
    assert.equal(currencies.length, AsciiEnum.prebuilt().currency.length)
    assert.equal(currencies.get('USD'), 'USD')
    assert.equal(AsciiEnum.fromLogicalName('tenor').length, 0)

    // A field declares the enum its values name, as one metadata document, so
    // every serialization carries it and it comes back the enum that wrote it.
    const side = new AsciiEnum('Side', { BUY: 'B', SELL: 'S' })
    const field = new Field('side', ascii32, false)
    field.setAsciiEnum(side)
    assert.deepEqual(side.intoMembers(ascii32), { BUY: 0x42000000n, SELL: 0x53000000n })
    assert.ok(Field.fromJSON(field.toJSON()).asciiEnum.equals(side))
    ```

An ASCII value has one integer, and it is the value itself. `ascii_packed` is the value's **own
storage bytes** read big-endian, so it is the same integer in every process, orders exactly as the
text does, is what a stable hash hashes, and fills an `i32`, an `i64`, or a whole `i128` by width.
An ASCII byte never sets the sign bit, so a packed code is never negative. Only a fixed width has
one: the variable form takes a value of any length and so has no integer its bytes always fit, and
neither has a width past the sixteen bytes an `i128` holds.

The generated enum names its members by that packed code, so a member survives a process, a file,
and another runtime; the name comes once from the core listing - an ASCII letter kept uppercased, a
digit kept, every other byte `_`, a leading digit prefixed with `_`, and a name that both opens and
closes with `_` dropping its trailing underscores, because that is the shape Python reserves for
`_sunder_` and `__dunder__` names.

`AsciiEnum` is that naming as a value: an enum's own name and one ASCII value per member, which a
field stores under the reserved `field:enum` key. The width is the field's datatype and is never
copied into the document, so a member's code is the packed value under that width and one enum is
one canonical text however it was built. Ordinary field metadata is what carries it, so the
declaration reaches Arrow, a file, and either binding with the field, and the `field:` protocol view
reads it beside `field:init` and `field:partition`. `from_logical_name` builds one from the ISO
listings the package ships - `CURRENCIES`, `COUNTRIES`, `MICS`, reachable as `prebuilt()` from
either binding - so a code column declares the vocabulary it draws from without a copy per language.

### The UUID

=== "Rust"

    ```rust
    use arrow_array::{Array, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, DataTypeKind, Field, Scalar};

    // One 128-bit identifier, one canonical spelling, and no parameters.
    let uuid = DataType::uuid();
    assert_eq!(DataType::from_str("uuid")?, uuid);
    assert_eq!(uuid.to_string(), "uuid");
    assert_eq!(uuid.kind(), DataTypeKind::Uuid);

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let packed = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab_u128;
    assert_eq!(uuid.uuid_packed(text.as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(text.to_uppercase().as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(text.replace('-', "").as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(&packed.to_be_bytes())?, packed);
    assert_eq!(uuid.uuid_value(packed)?, text);

    // Storage is the canonical `arrow.uuid` extension over sixteen bytes, and
    // the value reads back spelled out.
    let id = Field::new("id", DataType::Uuid, false);
    let value = id.scalar(text)?;
    let stored = scalar_array(&id, &value)?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), packed.to_be_bytes());
    assert_eq!(scalar_value(&id, stored.as_ref())?, value);

    let arrow = id.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(16));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "arrow.uuid");
    assert_eq!(Field::from_arrow(&arrow)?, id);

    assert!(uuid.uuid_packed(b"not-a-uuid").is_err());
    ```

=== "Python"

    ```python
    import uuid

    import pyarrow as pa

    from yggdryl import DataType, Field

    # One 128-bit identifier, one canonical spelling, and no parameters.
    uuid_type = DataType("uuid")
    assert DataType("uuid") == uuid_type
    assert str(uuid_type) == "uuid"
    assert uuid_type.kind == "uuid"

    # The identity is the sixteen bytes; every spelling is a rendering of them.
    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    id = Field("id", uuid_type, nullable=False)
    assert id.arrow_scalar(text) == pa.scalar(packed.to_bytes(16, "big"), pa.binary(16))
    assert id.arrow_scalar(text.upper()) == id.arrow_scalar(text)
    assert id.arrow_scalar(packed.to_bytes(16, "big")) == id.arrow_scalar(text)
    assert id.default_pyvalue() == "00000000-0000-0000-0000-000000000000"

    # Storage is the canonical `arrow.uuid` extension, which PyArrow registers
    # itself, so a column of them reads back as `uuid.UUID`.
    arrow = id.into_arrow()
    assert arrow.type == pa.uuid()
    assert arrow.type.storage_type == pa.binary(16)
    assert Field.from_arrow(arrow) == id
    stored = id.cast_arrow_array(pa.array([text, text.upper()]))
    assert stored.to_pylist() == [uuid.UUID(text)] * 2

    # A recognized identifier column renders as its spelling.
    batch = pa.record_batch([stored], schema=pa.schema([arrow]))
    spelled = DataType.from_fields([Field("id", "utf8")])
    assert spelled.cast_arrow_batch(batch).column(0).to_pylist() == [text, text]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    // One 128-bit identifier, one canonical spelling, and no parameters.
    const uuid = new DataType('uuid')
    assert.ok(DataType.from('uuid').equals(uuid))
    assert.equal(uuid.toString(), 'uuid')
    assert.equal(uuid.kind, 'uuid')

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    const id = new Field('id', uuid, false)
    assert.equal(id.defaultJSValue(), '00000000-0000-0000-0000-000000000000')
    assert.equal(fields.uuid('id').dtype.id, 'uuid')
    assert.ok(Field.fromJSON(id.toJSON()).equals(id))
    ```

A UUID is the ASCII widths' sibling: one fixed-width value whose integer is its own storage bytes
read big-endian, so it is the same integer in every process and is what a stable hash hashes. It is
a `u128` rather than an `i128` because every one of the sixteen bytes carries identity. Storage is
`FixedSizeBinary(16)` under the canonical `arrow.uuid` extension - the name Arrow itself registers,
taken as-is rather than re-spelled - and Iceberg's `uuid` maps straight onto it, so the spelling
survives a metadata rewrite in the datatype instead of a marker beside the column.

Where an ASCII width canonicalizes toward its text, because the value *is* text and the padding is
layout, a UUID canonicalizes toward its 36-character lowercase hyphenated spelling, because that is
what a reader means by an identifier. The 32-digit bare-hex text, upper case, and the sixteen stored
bytes are all accepted on the way in and rewrite to that one spelling; anything else is refused by
the one rule that field validation, Arrow ingest, and every cast tier all call.

### Versions

`Version` is a generic, numerically ordered value. Its native layout is exactly 16 bytes: required
`u8` major, nullable `u8` minor, and a nullable fixed 14-byte patch/qualifier tail. Parsing trims
trailing numeric zeroes and canonicalizes appended, dot-introduced, and hyphen-introduced
qualifiers. Hyphens are pre-release; the other forms are post-release, so `5.0-rc1 < 5 < 5.0SP1`
and `SP2 < SP10`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Version};

    let version = "5.0.SP1".parse::<Version>()?;
    assert_eq!(version.to_string(), "5.0SP1");
    assert_eq!(std::mem::size_of::<Version>(), 16);

    let field = Field::new("version", DataType::Version, false);
    assert_eq!(field.scalar("5.0.SP1")?, Scalar::from(version));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, types
    from yggdryl.text import json

    dtype = DataType("version")
    field = types.version("version", nullable=False)
    value = json.loads('"5.0.SP1"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == "5.0SP1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const dtype = new DataType('version')
    const value = json.loads('"5.0.SP1"', {
      field: fields.version('version', { nullable: false }),
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.equal(value.asJs(), '5.0SP1')
    ```

Arrow stores the canonical spelling as `Utf8` and preserves the datatype through the
`yggdryl.version` extension name. Arrow string sorting remains lexicographic; `Version::cmp` is the
version-ordering contract. Inputs whose first two components exceed `u8`, later components exceed
`u16`, or whose canonical patch tail exceeds 14 ASCII bytes are refused at the first bad byte.

On an AMD Ryzen 5 150 with Rust 1.96.1, the release benchmark
`cargo bench -p yggdryl --bench types -- version --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10`
measured parse at 35.1 ns and compare at 49.3 ns. These deliberately light two-case measurements
cover only the two hot operations this value owns.

### Regular-expression captures

`DataType::from_regex` builds one Struct from a byte regex's named captures,
in capture order. Every field is nullable because either the whole expression
or an optional branch may miss. With autotyping enabled, a capture whose syntax
names a boolean, integer, finite float, ISO date, time, or datetime receives
that datatype; a broad capture such as `\S+` stays UTF-8. Inference reads no
rows, so text media can publish its schema before opening its source.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let dtype = DataType::from_regex(
        r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)",
        true,
    )?;
    assert_eq!(dtype.field("level")?.dtype(), &DataType::Utf8);
    assert_eq!(dtype.field("id")?.dtype(), &DataType::Int64);
    assert!(dtype.field("id")?.is_nullable());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType.from_regex(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)")
    assert dtype["level"].dtype == DataType("utf8")
    assert dtype["id"].dtype == DataType("int64")
    assert dtype["id"].nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dtype = DataType.fromRegex(
      '\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)',
    )
    assert.equal(dtype.field('level').dtype.id, 'utf8')
    assert.equal(dtype.field('id').dtype.id, 'int64')
    assert.equal(dtype.field('id').nullable, true)
    ```

Passing `false` as the second argument keeps every capture UTF-8. Invalid
syntax and expressions beyond the shared datatype recursion limit are typed
datatype errors.

### Logical names

=== "Rust"

    ```rust
    use yggdryl::{AsciiEnum, DataType, TimeUnit, Timezone};

    // A name is one more spelling of a datatype, so it displays as that datatype.
    let price = DataType::from_logical_name("Price")?;
    assert_eq!(price, DataType::decimal64(18, 8)?);
    assert_eq!(price.to_string(), "decimal64(18,8)");

    // The same lookup backs the grammar, so a FIX declaration types a row.
    let row = DataType::from_str(
        "struct<ccy: Currency, venue: Exchange, px: Price, qty: Qty, at: UTCTimestamp>",
    )?;
    assert_eq!(
        row.get_field_by_path("venue").map(|field| field.dtype().clone()),
        Some(DataType::Mic)
    );
    assert_eq!(
        row.get_field_by_path("at").map(|field| field.dtype().clone()),
        Some(DataType::DateTime64 { unit: TimeUnit::Nanosecond, timezone: Timezone::UTC })
    );

    // Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    assert_eq!(DataType::from_str("utc_date_only")?, DataType::Date32);
    assert_eq!(DataType::LOGICAL_NAMES[0], ("currency", DataType::Currency));

    // Three of the names also prebuild the vocabulary their codes come from.
    assert_eq!(AsciiEnum::prebuilt_values("MIC"), AsciiEnum::MICS);
    assert!(AsciiEnum::prebuilt_values("tenor").is_empty());

    // The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert_eq!(DataType::from_str("int")?, DataType::Int32);
    assert_eq!(DataType::from_str("float")?, DataType::Float32);
    ```

=== "Python"

    ```python
    from yggdryl import AsciiEnum, DataType

    # A name is one more spelling of a datatype, so it displays as that datatype.
    price = DataType.from_logical_name("Price")
    assert price == DataType("decimal64(18,8)")
    assert str(price) == "decimal64(18,8)"

    # The same lookup backs the grammar, so a FIX declaration types a row.
    row = DataType("struct<ccy: Currency, venue: Exchange, px: Price, at: UTCTimestamp>")
    assert row["venue"].dtype == DataType("mic")
    assert row["at"].dtype == DataType('datetime64(ns,"UTC")')

    # Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    assert DataType("utc_date_only") == DataType("date32")
    assert DataType.logical_names()["currency"] == DataType("currency")

    # Three of the names also prebuild the vocabulary their codes come from.
    assert AsciiEnum.prebuilt()["mic"] == AsciiEnum.prebuilt()["exchange"]
    assert "tenor" not in AsciiEnum.prebuilt()

    # The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert DataType("int") == DataType("int32")
    assert DataType("float") == DataType("float32")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { AsciiEnum, DataType } = require('yggdryl')

    // A name is one more spelling of a datatype, so it displays as that datatype.
    const price = DataType.fromLogicalName('Price')
    assert.ok(price.equals(DataType.from('decimal64(18,8)')))
    assert.equal(price.toString(), 'decimal64(18,8)')

    // The same lookup backs the grammar, so a FIX declaration types a row.
    const row = DataType.from('struct<ccy: Currency, venue: Exchange, px: Price, at: UTCTimestamp>')
    assert.equal(row.getField('venue').dtype.id, 'mic')
    assert.equal(row.getField('at').dtype.toString(), 'datetime64(ns,"UTC")')

    // Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    assert.equal(DataType.from('utc_date_only').id, 'date32')
    assert.equal(DataType.logicalNames().currency.id, 'currency')

    // Three of the names also prebuild the vocabulary their codes come from.
    assert.deepEqual(AsciiEnum.prebuilt().mic, AsciiEnum.prebuilt().exchange)
    assert.equal(AsciiEnum.prebuilt().tenor, undefined)

    // The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert.equal(DataType.from('int').id, 'int32')
    assert.equal(DataType.from('float').id, 'float32')
    ```

A logical name is a *name*, never a type: `Price` resolves to `decimal64(18,8)` and displays as
`decimal64(18,8)`, so the grammar keeps one canonical spelling per datatype and nothing downstream
learns a new variant. `LOGICAL_NAMES` is the registry, `from_logical_name` resolves it, and the
name folds the way every other datatype word does - trimmed, ASCII case-insensitive, with `_`, `-`,
and spaces ignored - so `UTCTimestamp`, `utc_timestamp`, and `UTC Timestamp` are one name.

The vocabulary is the [FIX Latest](https://www.fixtrading.org) datatype table, so a FIX field
declaration types a column directly, plus `mic` and `cfi` - ISO 10383's and ISO 10962's own names
for two codes FIX itself carries as plain `String` fields.

| FIX | base | resolves to | why |
| --- | --- | --- | --- |
| `Currency` | String | `currency` | ISO 4217 alpha-3, exactly 3 bytes |
| `Country` | String | `country` | ISO 3166-1 alpha-2, exactly 2 bytes |
| `Exchange`, `mic` | String | `mic` | ISO 10383 MIC, exactly 4 bytes |
| `cfi` | - | `cfi` | ISO 10962, exactly 6 bytes |
| `Language` | String | `ascii(2)` | ISO 639-1 alpha-2 |
| `MonthYear` | String | `ascii(8)` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
| `Tenor` | Pattern | `ascii(8)` | `D5`, `W2`, `M3`, `Y1` |
| `Pattern` | - | `utf8` | the abstract base of `Tenor` and the reserved ranges |
| `Length` | int | `int32` | a byte count |
| `TagNum` | int | `int32` | a FIX tag |
| `SeqNum` | int | `int64` | a session sequence number outgrows `int32` |
| `NumInGroup` | int | `int32` | a repeating-group counter |
| `DayOfMonth` | int | `int8` | 1 through 31 |
| `Reserved100Plus` | Pattern | `int32` | a user-defined enumeration value |
| `Reserved1000Plus` | Pattern | `int32` | as above |
| `Reserved4000Plus` | Pattern | `int32` | as above |
| `Qty` | float | `decimal64(18,8)` | exact, 8 bytes |
| `Price` | float | `decimal64(18,8)` | exact, 8 bytes |
| `PriceOffset` | float | `decimal64(18,8)` | exact and signed |
| `Percentage` | float | `decimal64(18,8)` | `0.0525` is 5.25% |
| `Amt` | float | `decimal128(38,8)` | a notional outgrows 10 integer digits |
| `UTCTimestamp` | String | `datetime64(ns,"UTC")` | the instant, at the finest FIX width |
| `TZTimestamp` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant |
| `UTCTimeOnly` | String | `time64(ns)` | a time of day with a fraction |
| `LocalMktTime` | String | `time32(s)` | `HH:MM:SS`, no fraction |
| `UTCDateOnly` | String | `date32` | a calendar day |
| `LocalMktDate` | String | `date32` | a calendar day |
| `TZTimeOnly` | String | `ascii(16)` | a time of day plus an offset has no Arrow type |
| `MultipleCharValue` | char | `utf8` | space-delimited members |
| `MultipleStringValue` | String | `utf8` | space-delimited members |
| `XID` | String | `utf8` | an XML identifier |
| `XIDREF` | String | `utf8` | a reference to one |
| `data` | - | `binary` | opaque bytes |
| `XMLData` | data | `binary` | an XML document, opaque here |

Five FIX base types already have a meaning in the Arrow/SQL grammar and keep it, because a stored
schema string must not change meaning under a reader: `int` is `int32`, `float` is `float32`, `char`
and `String` are `utf8`, and `Boolean` is `boolean`. The FIX types derived from `int` and `float`
carry the precision those base types do not, which is what the registrations above add.

The float family is exact rather than binary floating point because a price that does not
round-trip is a broken trade. `decimal64` holds 18 digits in 8 bytes, which is 10 integer digits
beside the 8 fractional ones a listed venue's tick fits in; a notional needs more integer room, so
`Amt` widens to `decimal128`. A venue outside those bounds declares its own `decimal(p,s)`, which
is why these are names over the ordinary constructors rather than a second numeric model.
`TZTimestamp` keeps the instant and drops the local offset, because an Arrow column carries one
zone for every row; read it under `datetime64(ns,"<zone>")` when the local reading is the value.

`currency`, `country`, and `mic` also name a [prebuilt vocabulary](#the-declared-vocabulary-and-its-generated-enum):
`AsciiEnum::from_logical_name` builds the enum of the codes those standards assign, in sorted
order, named for the registration. A member's code is the value's own bytes under the datatype the
name resolved to, so every process reading this version answers the same integers. The MICs are the
common venues rather than the whole ISO 10383 registry, which is thousands of segment codes.

### Identity and family

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    let stamp = DataType::from_str("datetime64(ns, Europe/Paris)")?;
    assert_eq!(stamp.id(), DataTypeId::DateTime64);
    assert_eq!(stamp.kind(), DataTypeKind::Temporal);
    assert_eq!(stamp.name(), "datetime64");

    // The id drops parameters, so two resolutions share one identity ...
    assert_eq!(DataType::from_str("datetime64(s)")?.id(), stamp.id());
    // ... while the values themselves stay distinct.
    assert_ne!(DataType::from_str("datetime64(s)")?, stamp);

    // Foreign Arrow/SQL input canonicalizes to the core spelling.
    assert_eq!(DataType::from_str("timestamp(us)")?.to_string(), "datetime64(us)");

    assert_eq!(DataType::decimal(38, 4)?.id(), DataTypeId::Decimal128);
    assert_eq!(DataType::decimal(38, 4)?.kind(), DataTypeKind::Decimal);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    stamp = DataType("datetime64(ns, Europe/Paris)")
    assert stamp.id == "datetime64"
    assert stamp.kind == "temporal"

    assert DataType("datetime64(s)").id == stamp.id
    assert DataType("datetime64(s)") != stamp
    assert str(DataType("timestamp(us)")) == "datetime64(us)"

    assert DataType.decimal(38, 4).id == "decimal128"
    assert DataType.decimal(38, 4).kind == "decimal"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const stamp = DataType.from('datetime64(ns, Europe/Paris)')
    assert.equal(stamp.id, 'datetime64')
    assert.equal(stamp.kind, 'temporal')

    assert.equal(DataType.from('datetime64(s)').id, stamp.id)
    assert.equal(DataType.from('datetime64(s)').equals(stamp), false)
    assert.equal(DataType.from('timestamp(us)').toString(), 'datetime64(us)')

    assert.equal(fields.decimal('amount', 38, 4).dtype.id, 'decimal128')
    assert.equal(fields.decimal('amount', 38, 4).dtype.kind, 'decimal')
    ```

`id` names the variant and `kind` names the family it belongs to - 56 ids across 17 kinds. Both are
parameter-free, so they compare and hash without touching nested state, which is what makes them the
cheap way to branch. Dispatch on the kind when the behavior is uniform across a family, on the id
when it is not; `name()` is the Rust spelling of `id().as_str()`. The two vocabularies and the
predicates over them are documented in [shared enums](types.md). Python and JavaScript have no
separate class for either: both arrive as the canonical lowercase strings.

### Arrow projection

=== "Rust"

    ```rust
    use yggdryl::{DataType, TimeUnit};

    let value = DataType::from_str("map<string,array<decimal(38,18)>>")?;
    let arrow = value.clone().into_arrow()?;

    assert_eq!(DataType::from_arrow(&arrow)?, value);
    assert_eq!(value.clone().into_arrow()?, arrow);
    assert_eq!(DataType::try_from(arrow)?, value);

    // Projection re-checks parameters, so a directly built enum value cannot escape.
    assert!(DataType::Time32(TimeUnit::Nanosecond).into_arrow().is_err());
    assert!(DataType::Time32(TimeUnit::Nanosecond).into_arrow_ffi().is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    value = DataType("map<string,array<decimal(38,18)>>")
    arrow = value.into_arrow()

    assert DataType.from_arrow(arrow) == value
    assert DataType(arrow) == value
    assert value.into_arrow() == arrow

    assert DataType(pa.int64()) == DataType("int64")
    assert DataType("int64").into_arrow() == pa.int64()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // Any Apache Arrow JS type is read through its own textual form.
    const arrowLike = { toString: () => 'map<string,array<decimal(38,18)>>' }
    const value = DataType.fromArrow(arrowLike)

    assert.equal(value.id, 'map')
    assert.ok(DataType.fromArrow(value).equals(value))
    assert.throws(() => DataType.fromArrow({}), /own textual representation/)
    ```

Rust and Python hand over a real Arrow type - an `arrow_schema::DataType` and a `pyarrow.DataType`
respectively - and Python crosses the boundary through the Arrow C Data Interface rather than
rebuilding the value. Rust's `into_arrow` and `into_arrow_ffi` consume the source so ownership is
explicit; clone first when the source must be retained.

Node's `fromArrow` reads an Arrow JS type through its standard `toString`; the canonical display is
the bridge back out. Nothing is inferred from a generic string
coercion, so an object without its own textual form is an error rather than `[object Object]`.

Projection is a validation boundary. Because `DataType` is a public Rust enum, a caller can
construct a variant with parameters no constructor would accept; every conversion repeats the checks
before materializing foreign state. Whole schemas cross the same boundary through
[`arrow`](arrow.md), which reads a struct-rooted `Field` as an Arrow `Schema`.

### Default values

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};

    let value = DataType::from_fields([
        Field::new("id", DataType::Int32, false),
        Field::new("note", DataType::Utf8, true),
    ])?;

    // One positional slot per child, each honoring its own nullability.
    assert_eq!(
        value.default_value()?.as_sequence().unwrap(),
        &[Scalar::from(0_i64), Scalar::Null]
    );
    assert!(value.is_default_value(&value.default_value()?)?);
    assert_eq!(DataType::Utf8.default_value()?, Scalar::from(""));

    // A default is bounded: a layout too large to materialize is an error, not a null.
    assert!(DataType::FixedSizeBinary(64 * 1024 * 1024 + 1).default_value().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    value = DataType.from_fields([
        Field("id", "int32", nullable=False),
        Field("note", "utf8", nullable=True),
    ])
    row = value.default_pyvalue()

    assert (row.id, row.note) == (0, None)
    assert DataType("utf8").default_pyvalue() == ""
    assert DataType("int64").default_pyhint() is int
    assert value.default_arrow_scalar().as_py() == {"id": 0, "note": None}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const value = DataType.fromFields([
      fields.int32('id', { nullable: false }),
      fields.utf8('note', { nullable: true }),
    ])

    // A nullable field defaults to null; a required one defaults to its zero.
    assert.deepEqual(value.defaultJSValue(), [0, null])
    assert.equal(new DataType('utf8').defaultJSValue(), '')
    assert.equal(new DataType('int64').defaultJSHint().constructor, BigInt)
    assert.equal(new DataType('int32').defaultArrowScalar(), 0)
    ```

Every one of the 56 variants has a canonical default, and it is computed from the schema rather than
looked up per language: the core produces one value and each binding projects it. Rust yields a
[`Scalar`](types.md); Python yields a generated field dataclass or a Python scalar from `default_pyvalue`
and a `pyarrow.Scalar` from `default_arrow_scalar`; JavaScript yields a plain array, `Buffer`,
`Map`, or `{ typeId, value }`. Mutable containers are freshly allocated on every call, so a default
is never shared state.

The walk is bounded in both depth and bytes. A fixed-size binary or fixed-size list whose default
would exceed the byte safety limit, and a nesting deeper than the parser limit, both fail loudly
instead of degrading to a null. `is_default_value` answers the reverse question - is this value the
canonical default - without allocating the default first.

A bare `DataType` has no nullability, so its default is the non-null one. Ask a
[`Field`](types.md) instead when the answer depends on whether the slot may be null.

### Serializing a schema

`DataType` reads and writes the three structured-text formats through **one** structural model. There
is exactly one `DataType` ⇄ `Scalar` mapping - `into_value`/`from_value` in Rust, `into_dict`/`from_dict` in
Python - and JSON, YAML, and TOML are three writers over it, so the three agree by construction
rather than by three sets of tests. That is also what makes a schema *embeddable*: a configuration
document can carry a declared schema inline beside the rest of its settings, with no
JSON-string-inside-YAML awkwardness.

The shape is what the JSON emit carries: `name`, `dtype`, `nullable`, then
`dictionary_id` only when it is non-zero and `dictionary_is_ordered` only when it is set, then
`metadata`. An unset optional attribute is **omitted**, never emitted as null - which is also why
TOML, which has no null, loses nothing on the way out.

Each format takes the shared [`Formatting`](text.md#formatting) option; Python spells it as an
`indent` keyword.

Nesting is carried, not flattened: a struct of a list of a struct of a map comes back exactly as it
went in, in every format, because all three write the one structural model rather than a shape of
their own.

JSON is also readable and writable as bytes. `into_json_bytes` renders the document `into_json`
renders, encoded rather than decoded, for a caller writing straight to a file or a socket -
`toJSONBytes` in JavaScript, and `Scalar` has spelled the same pair `as_json_bytes` and
`as_json_utf8` all along. Reading needs no such choice: `from_json` takes whichever shape the
caller already holds - the bytes it was read from, the text it was read as, or the object
`json.loads` and `JSON.parse` already made - and dispatches on what it gets.

=== "Rust"

    ```rust
    use yggdryl::DataType;
    use yggdryl::types::Scalar;

    let dtype = DataType::decimal(9, 2)?;

    // One structural model, three formats over it.
    assert_eq!(DataType::from_value(dtype.clone().into_value())?, dtype);
    assert_eq!(DataType::from_json(&dtype.clone().into_json()?)?, dtype);
    assert_eq!(DataType::from_yaml(&dtype.clone().into_yaml()?)?, dtype);
    assert_eq!(DataType::from_toml(&dtype.clone().into_toml()?)?, dtype);

    let shape = dtype.into_value();
    assert_eq!(shape.get_key_str("type").and_then(Scalar::as_utf8), Some("decimal32"));
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType.decimal(9, 2)

    assert DataType.from_dict(dtype.into_dict()) == dtype
    assert DataType.from_json(dtype.into_json()) == dtype
    assert DataType.from_yaml(dtype.into_yaml()) == dtype
    assert DataType.from_toml(dtype.into_toml()) == dtype

    assert dtype.into_dict()["type"] == "decimal32"
    ```

=== "JavaScript"

    !!! note "Rust first"
        The YAML and TOML pair lands in the JavaScript binding once the core surface settles;
        `toJSON` is already there.

### A readable rendering

`Display` - and Python's `str`/`repr` - is the compact constructor form, and it stays exactly as it
is: it round-trips through `from_str`, and the error messages, the documentation, and Python's
`repr` all depend on that. It is also unreadable the moment a struct nests three levels deep.

The readable form is the **alternate**: `{:#}` in Rust, or the named `pretty()` adapter that backs
it, and `pretty()` in Python. One fact per line, one indent per nesting level, and only the
attributes that are actually set - a `dictionary_id` of `0` or empty metadata is noise the compact
form already omits. Metadata renders as indented `@key = value` lines rather than one braced blob.
The output is stable across runs; nothing in it iterates a hash map.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let rows = DataType::list(
        DataType::from_fields([DataType::Utf8.nullable_field("venue")])?.nullable_field("item"),
    );

    // Compact still round-trips.
    assert_eq!(DataType::from_str(&rows.to_string())?, rows);

    // Readable is the alternate, or the named adapter - one implementation.
    assert_eq!(format!("{rows:#}"), rows.pretty().to_string());
    assert_eq!(
        format!("{rows:#}"),
        "list\n  item: struct[1], nullable\n    venue: utf8, nullable",
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    rows = DataType.from_fields([Field("venue", "utf8")])

    # `repr` is unchanged - the eval-round-trip form Python expects.
    assert repr(rows).startswith("DataType.from_str(")
    assert DataType.from_str(str(rows)) == rows

    assert rows.pretty() == "struct[1]\n  venue: utf8, nullable"
    ```

=== "JavaScript"

    !!! note "Rust first"
        `pretty` lands in the JavaScript binding once the core surface settles.


### Compatibility rewriting

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme, TimeUnit, Timezone};

    let source = DataType::from_fields([
        Field::new("small", DataType::UInt8, false),
        Field::new("wide", DataType::UInt64, true),
        Field::new(
            "text",
            DataType::large_list(DataType::Utf8View.nullable_field("item")),
            false,
        ),
    ])?;

    let spark = source.clone().into_scheme_compat(&Scheme::SPARK)?;
    let rewritten = spark.as_fields().unwrap();
    assert_eq!(rewritten[0].dtype(), &DataType::Int16);
    assert_eq!(rewritten[1].dtype(), &DataType::decimal128(20, 0)?);
    assert_eq!(
        rewritten[2].dtype(),
        &DataType::list(DataType::Utf8.nullable_field("item"))
    );

    // Arrow is a validated clone; Polars keeps the unsigned integers Spark has to widen.
    assert_eq!(source.clone().into_scheme_compat(&Scheme::ARROW)?, source);
    assert_eq!(DataType::UInt32.into_scheme_compat(&Scheme::POLARS)?, DataType::UInt32);

    // A rewrite that would reinterpret values is refused, and the path is named.
    let error = DataType::from_fields([Field::new(
        "created",
        DataType::DateTime64 { unit: TimeUnit::Nanosecond, timezone: Timezone::NAIVE },
        false,
    )])?
    .into_scheme_compat(&Scheme::SPARK)
    .unwrap_err()
    .to_string();
    assert!(error.contains("created") && error.contains("got ns"));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field

    source = DataType.from_fields([
        Field("small", "uint8", nullable=False),
        Field("wide", "uint64", nullable=True),
    ])

    spark = source.into_scheme_compat("spark")
    assert str(spark["small"].dtype) == "int16"
    assert str(spark["wide"].dtype) == "decimal128(20,0)"

    assert source.into_scheme_compat("arrow") == source
    assert DataType("uint32").into_scheme_compat("polars") == DataType("uint32")

    with pytest.raises(ValueError, match="got ns"):
        DataType("datetime64(ns)").into_scheme_compat("spark")
    with pytest.raises(ValueError, match="arrow, spark, polars, pandas"):
        DataType("int32").into_scheme_compat("duckdb")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const source = DataType.fromFields([
      fields.uint8('small'),
      fields.uint64('wide', { nullable: true }),
    ])

    const spark = source.intoSchemeCompat('spark')
    assert.equal(spark.getField('small').dtype.toString(), 'int16')
    assert.equal(spark.getField('wide').dtype.toString(), 'decimal128(20,0)')

    assert.ok(source.intoSchemeCompat('arrow').equals(source))
    assert.ok(DataType.from('uint32').intoSchemeCompat('polars').equals(DataType.from('uint32')))

    assert.throws(() => DataType.from('datetime64(ns)').intoSchemeCompat('spark'), /got ns/)
    assert.throws(
      () => DataType.from('int32').intoSchemeCompat('duckdb'),
      /arrow, spark, polars, pandas/,
    )
    ```

Five targets are accepted - `arrow`, `spark`, `polars`, `pandas`, `iceberg` - and anything else is
refused by name. Arrow validates and clones. The other four walk the type recursively and apply only
layout-only rewrites: widths that the engine cannot address, offset variants it does not implement,
and container shapes it lacks. Spark has no unsigned integers, so `uint8` widens to `int16` and
`uint64` becomes `decimal128(20, 0)`; Polars has both natively and keeps them, and keeps a
fixed-size list where Spark degrades to a list. Polars and pandas have no first-class map and say
so, naming key/value structs as the alternative. [Iceberg](media.md) is a closed primitive
vocabulary rather than an engine, so every narrow or unsigned integer widens to the signed one that
holds it - `uint8`, `uint16`, `int8`, and `int16` all become `int32` - it keeps `fixed[n]` and both
its microsecond and nanosecond timestamps, and it has no elapsed-time or calendar-interval type at
all.

Anything that would change what a value means is an error rather than a rewrite. A nanosecond
datetime is not silently truncated to Spark's microseconds, a negative decimal scale is not
clamped, and a node carrying Arrow extension metadata is not physically relabeled - the message
names the offending node with a path like `$["a.b"]` or `$[].item`. Applied to a
[`Field`](types.md), the same call keeps the name, nullability, and metadata, and rebuilds the Arrow
projection cache only when something actually changed.

### Building the enum directly

`DataType` is a public Rust enum, so pattern matching and direct construction are available - and so
is a state no constructor would produce. `validate` is the backstop, and it is Rust-only: every
entry point the bindings expose validates before it hands back a value.

```rust
use yggdryl::{DataType, Field, TimeUnit};

let broken = DataType::Time32(TimeUnit::Nanosecond);
assert!(broken.validate().is_err());
assert!(DataType::time32(TimeUnit::Nanosecond).is_err());

// A valid value validates without allocating, recursing through every child.
let value = DataType::list(Field::new(
    "item",
    DataType::Decimal128 { precision: 18, scale: 4 },
    true,
));
value.validate()?;
assert!(DataType::Decimal128 { precision: 0, scale: 0 }.validate().is_err());

assert_eq!(DataType::PARSE_RECURSION_LIMIT, 64);
```

The same limit bounds parsing, default construction, and compatibility walks, so neither a nested
type expression nor a structural JSON schema can buy unbounded recursion.

## Fields


`yggdryl::types` is the one schema value: a name, a datatype, a nullability flag, and metadata.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let field = Field::new("price", DataType::from_str("decimal(18, 6)")?, false);

    assert_eq!(field.name(), "price");
    assert_eq!(field.dtype(), &DataType::decimal(18, 6)?);
    assert!(!field.is_nullable());
    assert!(field.is_metadata_empty());

    // The canonical text round-trips, and shorthand parses into the same value.
    assert_eq!(Field::from_str(&field.to_string())?, field);
    assert_eq!(Field::from_str("price decimal(18, 6) NOT NULL")?, field);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "decimal(18, 6)", nullable=False)

    assert field.name == "price"
    assert field.dtype == DataType("decimal(18, 6)")
    assert field.nullable is False
    assert len(field) == 0

    assert Field.from_str(str(field)) == field
    assert Field.from_str("price decimal(18, 6) NOT NULL") == field
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const field = new Field('price', 'decimal(18, 6)', false)

    assert.equal(field.name, 'price')
    assert.ok(field.dtype.equals(DataType.from('decimal(18, 6)')))
    assert.equal(field.nullable, false)
    assert.equal(field.size, 0)

    assert.ok(Field.from(field.toString()).equals(field))
    assert.ok(Field.from('price decimal(18, 6) NOT NULL').equals(field))
    ```

A field owns exactly those four things. Everything else on the type is a view over them: a typed
accessor for a reserved metadata key, an [Arrow](arrow.md) projection, a comparison, a cast. There
is no separate schema type anywhere in the library, so learning this page is learning the schema
model.

`Field::new` takes a [`DataType`](types.md) and validates nothing; `Field::from_parts` takes a
metadata snapshot as well and validates the whole value. The bindings have a single constructor that
takes the metadata inline and always validates, and it infers the datatype from whatever is in that
position - a string in any accepted syntax, a native `DataType`, and in Python a PyArrow type.

### A non-null struct field is the schema

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("trade");

    schema.validate_struct_root()?;
    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.index_of("symbol"), Some(1));
    assert_eq!(schema.get_field_by_path("id").map(Field::name), Some("id"));

    // A nullable root is not a schema: a whole row cannot be logically absent.
    assert!(schema.with_nullable(true).validate_struct_root().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, types

    schema = Field(
        "trade",
        DataType.from_fields([
            types.int64("id", nullable=False),
            types.utf8("symbol"),
        ]),
        nullable=False,
    )

    children = schema.dtype
    assert len(children) == 2
    assert "symbol" in children
    assert children["id"].nullable is False
    assert children[1].name == "symbol"
    assert [child.name for child in children] == ["id", "symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    const schema = new Field(
      'trade',
      DataType.fromFields([
        fields.int64('id', { nullable: false }),
        fields.utf8('symbol', { nullable: true }),
      ]),
      false,
    )

    const children = schema.dtype
    assert.equal(children.length, 2)
    assert.equal(children.getFieldAt(0).nullable, false)
    assert.equal(children.getFieldByPath('symbol').name, 'symbol')
    assert.deepEqual(children.keys(), ['id', 'symbol'])
    ```

The columns of a table are the children of a struct field, so a schema is a field whose datatype is
a struct and whose `nullable` is false. `validate_struct_root` is the check every reader and writer
makes: `require_struct` accepts a nullable struct, because a nullable struct is a perfectly good
*column*, while a nullable *root* would make an entire row absent and no row-oriented reader can
represent that. This root is what [`ipc`](media.md), [`parquet`](media.md), and
[`iceberg`](media.md) take and return.

Rust reaches the children through the field, and `DataType` answers the same calls, so a caller
walking a schema never has to ask which node it is holding. Each lookup comes in three forms - by
position, by path, and by whichever the key turns out to be - and each of those raises or answers
`None`:

| | position | path | either |
|---|---|---|---|
| raising | `field_at` | `field_by_path` | `field` |
| optional | `get_field_at` | `get_field_by_path` | `get_field` |
| replacing | `set_field_at` | `set_field_by_path` | `set_field` |
| removing | `remove_field_at` | `remove_field_by_path` | `remove_field` |

`fields`, `field_len`, and `index_of` round the struct root out.

Python and JavaScript carry the same family under their own casing - `get_field_by_path` and
`getFieldByPath` - and each adds what its language expects. Python spells the inferring form
`field(x, *, idx=..., path=...)`, refusing a call that names more than one of the three. JavaScript
keeps positions Array-compatible, so a negative index counts from the end, and the optional forms
answer `null` where Python answers `None` and Rust answers `Option`.

Two names moved so that `field` could mean this on every class: the datatype constructor that was
`DataType::field(name, nullable)` is `named_field`, beside the `nullable_field` and `required_field`
it already builds; and the `field:` property view is `as_field_properties` in Rust,
`field_properties` in Python and `fieldProperties` in JavaScript. `as_field` could not be the Rust
spelling because it already means "the `Field` inside" on a typed field and on a protocol view, and
one receiver-dependent name for two returns is what this page exists to avoid. The namespace itself
is unchanged, so `field:init` and `field:partition` are what they were.

#### Flattening and expanding

Two projections answer the two questions a nested schema raises, and they are
deliberately separate because they treat collections in opposite ways.

`unnest_fields` flattens **struct** nesting to its leaves, each named by the
dotted path that reaches it: `struct<id, line: struct<px>>` answers `id` and
`line.px`. A leaf under a nullable ancestor is nullable, because a null parent
leaves it with no value to carry, and every name it answers is one
[`field_by_path`](#item-access-reaches-a-child-never-metadata) resolves - so a flattened column list and
the tree it came from address children the same way. A list or a map is a
**leaf** here: unnesting says what a flat column list looks like, and a list is
one column.

`explode_fields` is what reaches inside one. It replaces each **collection**
child with what it holds - a list answers its item, a map its entries, a
dictionary or run-end node the values it encodes - and returns anything else
unchanged, so the result names the same columns in the same order. The column
keeps its own name, and is nullable when either the collection or its element
is, because an absent list yields no element. Only one level is unwrapped, so a
list of lists answers a list; calling it again reaches the next one, which
makes the depth the caller's decision.

Both answer a list of fields rather than a node, the way `partition_fields`
does; `DataType::from_fields` builds a node from either when you want one.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let row = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("px")])?
            .nullable_field("line"),
        DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
    ])?;

    // Structs flatten to leaves; the list stays one column.
    let leaves = row.unnest_fields();
    let names: Vec<&str> = leaves.iter().map(|field| field.name()).collect();
    assert_eq!(names, ["id", "line.px", "levels"]);

    // The nullable parent makes its leaf nullable, and the name resolves.
    assert!(leaves[1].is_nullable());
    assert!(row.get_field_by_path("line.px").is_some());

    // Exploding reaches inside the collection, keeping the column's name.
    let exploded = row.explode_fields();
    assert_eq!(exploded[2].name(), "levels");
    assert_eq!(exploded[2].dtype(), &DataType::Float64);
    ```

#### Merging two schemas

`merge_with` answers the type that describes both sides, and it is the only
promotion table in the crate: expression typing and value inference call the
same code, so two callers reading one pair of types can never disagree.

The rules are tried in order:

1. two equal types are that type;
2. `null` yields to whatever is defined beside it, so a column read as all-null
   takes the shape the other side gives it;
3. two nested layouts of the same family recurse - a struct takes the **union**
   of its fields, and lists, maps, unions and run-end nodes merge their
   children;
4. **bytes win**, because every other encoding fits inside them;
5. **text wins next**, over numbers and temporals; two ASCII widths meet at
   the wider (or, narrowing, the narrower) width, and a width beside
   variable text meets at the variable text when widening and at the width
   when narrowing;
6. numbers meet by width, and temporals by unit.

Anything left is refused rather than guessed: a boolean and a datetime have no
meeting point that is not a re-encoding, and an exact decimal beside an
approximate float would trade exactness for range without saying so.

`upscale` picks the direction width resolves in. Widening is the default and
loses nothing - `int32` and `int64` meet at `int64`. Passing `false` meets at
the tightest type that names both, which is what a caller wants when the
narrower type is the one the data actually fits.

A struct child only one side declares becomes **nullable**, because the rows
the other side described do not carry it, and field order is the receiver's
with additions appended, so merging never reorders columns a caller depends on.

`Field.merge_with` adds only what a field carries beyond a type: the name is
the receiver's, the result is nullable when either side is, dictionary options
survive only where both sides encode, and metadata is the union of both with
the receiver winning any key they disagree on.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let left = DataType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::Utf8.required_field("venue"),
    ])?;
    let right = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?;

    let merged = left.merge_with(&right, true)?;

    // The shared column widens; the two unshared ones arrive nullable.
    assert_eq!(merged["id"].dtype(), &DataType::Int64);
    assert!(merged["venue"].is_nullable());
    assert!(merged["price"].is_nullable());

    // Narrowing meets at the tightest type naming both.
    assert_eq!(
        DataType::Int32.merge_with(&DataType::Int64, false)?,
        DataType::Int32,
    );

    // Bytes win over text, and text over numbers.
    assert_eq!(DataType::Utf8.merge_with(&DataType::Binary, true)?, DataType::Binary);
    assert_eq!(DataType::Int64.merge_with(&DataType::Utf8, true)?, DataType::Utf8);

    // A field merge carries nullability and metadata across.
    let a = Field::new("price", DataType::Int32, false);
    let b = Field::new("price", DataType::Int64, true);
    let field = a.merge_with(&b, true)?;
    assert_eq!(field.dtype(), &DataType::Int64);
    assert!(field.is_nullable());
    ```

#### Item access reaches a child, never metadata

Subscripting a `Field` or a `DataType` means one thing: reach a nested child. A `str` is a child
name, an `int` is a position, and `len`, iteration, and membership all speak children. Both classes
answer identically, so a caller walking one object graph never gets a child from one node and a
metadata string from the next. Metadata is reached through [its own view](#metadata-is-a-mapping) -
`field.metadata[...]` in Python, `get_metadata` and friends in Rust - because a view whose keys *are*
keys is where item syntax legitimately means "a key".

Chained subscripts still descend - `order["line"]["price"]` reaches two levels, because each
subscript answers a node that subscripts again - and a single string may also spell the whole route:
`order["line.price"]`. A string is resolved **name first**, so a child whose own name contains a dot
stays reachable: `order["a.b"]` finds a child literally called `a.b` before it considers `a` then
`b`. Only when nothing carries the whole string is it decomposed, each `.` tried as a boundary from
the left, so `"a.b.c"` still resolves through a child named `a.b` that carries `c`.

Assignment is dict-like *by path* and list-like *by position*: a path that resolves is replaced in
place keeping its position, a string that resolves to nothing **appends** a new child, and a
position only ever replaces - past the end is an error, never a silent grow. `del` removes and
closes the gap by either form. Only a struct may grow or shrink: a list holds exactly one child and
a run-end node exactly two, so those refuse rather than quietly becoming a struct. In Python this
routes through the core's cache-aware child mutation, which is also why a `DataType` -
immutable and hashable - refuses assignment and points at the `Field` that carries it.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])?
            .required_field("line"),
    ])?
    .required_field("order");
    order.insert_metadata("owner", "trading")?;

    // A child by name, by position, and two levels down.
    assert_eq!(order["id"].dtype(), &DataType::Int64);
    assert_eq!(order[1].name(), "line");
    assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);

    // An unknown name appends; a position replaces.
    order.set_field_by_path("venue", DataType::Utf8.nullable_field("venue"))?;
    assert_eq!(order.field_len(), 3);
    order.set_field(0, DataType::Utf8.required_field("id"))?;
    assert_eq!(order["id"].dtype(), &DataType::Utf8);
    assert_eq!(order.remove_field_by_path("venue")?.name(), "venue");

    // Metadata keeps its own named surface.
    assert_eq!(order.get_metadata("owner"), Some("trading"));
    assert!(order.get_field_by_path("owner").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    order = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
                nullable=False,
            ),
        ]),
        nullable=False,
        metadata={"owner": "trading"},
    )

    # A child by name, by position, negatively, and two levels down.
    assert order["id"].dtype == DataType("int64")
    assert order[-1].name == "line"
    assert order["line"]["price"].dtype == DataType("float64")

    # The DataType answers the same way, and children drive len/iter/in.
    assert order.dtype["id"].name == "id"
    assert len(order) == 2
    assert [child.name for child in order] == ["id", "line"]
    assert "line" in order

    # An unknown name appends; a position replaces only.
    order["venue"] = Field("venue", "utf8")
    assert len(order) == 3
    order[0] = Field("id", "utf8", nullable=False)
    assert order["id"].dtype == DataType("utf8")
    del order["venue"]
    assert len(order) == 2

    # Metadata is reached through its view, never by subscripting the node.
    assert order.metadata["owner"] == "trading"
    try:
        order["owner"]
    except KeyError:
        pass
    ```

=== "JavaScript"

    !!! note "Rust first"
        The JavaScript binding reaches children through `dtype` with `at`, `getByName`, and
        `keys`; the shared subscript vocabulary lands with the rest of the lifecycle surface.


### Metadata is a mapping

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
    field.insert_metadata("currency", "EUR")?;
    field.update_metadata([("source", "exchange")])?;

    assert_eq!(field.metadata_len(), 3);
    assert_eq!(field.get_metadata("venue"), Some("XPAR"));
    assert!(field.has_metadata("currency"));
    assert_eq!(
        field.metadata_iter().collect::<Vec<_>>(),
        [("currency", "EUR"), ("source", "exchange"), ("venue", "XPAR")]
    );
    assert_eq!(field.remove_metadata("venue").as_deref(), Some("XPAR"));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    # Metadata lives on `field.metadata`, a live mapping view. Subscripting the
    # field itself reaches a nested *child*, not a metadata key.
    field.metadata["currency"] = "EUR"
    field.metadata.update(source="exchange")

    assert len(field.metadata) == 3
    assert "venue" in field.metadata
    assert field.metadata["venue"] == "XPAR"
    assert field.metadata.get("missing") is None
    assert list(field.metadata.items()) == [
        ("currency", "EUR"),
        ("source", "exchange"),
        ("venue", "XPAR"),
    ]

    del field.metadata["venue"]
    assert list(field.metadata.keys()) == ["currency", "source"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'float64', false, { venue: 'XPAR' })
    field.set('currency', 'EUR')
    field.update(new Map([['source', 'exchange']]))

    assert.equal(field.size, 3)
    assert.equal(field.has('venue'), true)
    assert.equal(field.get('venue'), 'XPAR')
    assert.equal(field.get('missing'), null)
    assert.deepEqual([...field], [
      ['currency', 'EUR'],
      ['source', 'exchange'],
      ['venue', 'XPAR'],
    ])

    assert.equal(field.delete('venue'), true)
    assert.deepEqual(field.keys(), ['currency', 'source'])
    ```

Keys are strings, values are strings, and iteration is in lexical key order, so two independently
built fields with the same entries compare and hash identically. Clones share one metadata map until
a write forces a copy.

Every write validates before it changes anything. `set_metadata` and `update_metadata` in Rust,
`update` in the bindings, build and check the whole batch first, so a bad entry in the middle of a
thousand leaves the field exactly as it was. Equality, ordering, and hashing all include metadata
and dictionary state. In Python, the first `hash(field)` therefore locks every equality-affecting
mutation on that wrapper; `copy.copy(field)` makes an independent unlocked wrapper when an edit is
needed. `stable_hash()` computes the same complete identity without locking it. The bindings' live
metadata and protocol views remain unhashable because they can change through their owning field.
Rust's borrowed protocol view hashes only the properties it exposes, never the field behind it,
which is why it is not `Borrow<Field>`.

### Reserved keys and protocol properties

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MimeType, Scheme};

    let mut field = Field::new("payload", DataType::Binary, false);

    field.set_parquet_field_id(17);
    field.set_init(false);
    field.set_display("Raw payload")?;
    field.as_http_mut().set_content_type("application/json; charset=utf-8")?;
    field.set_property(&Scheme::POSTGRES, "type", "jsonb")?;

    assert_eq!(field.parquet_field_id()?, Some(17));
    assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));
    assert!(!field.is_init()?);
    assert_eq!(field.get_metadata("field:init"), Some("false"));

    // A straight key belongs to no protocol, so every protocol falls back to it.
    assert_eq!(field.display(), Some("Raw payload"));
    assert_eq!(field.as_postgres().display(), Some("Raw payload"));

    // An http: property answers to either scheme and to a raw key lookup, and its
    // parsing accessors live on the field's http: view.
    assert_eq!(field.as_http().mime_type()?, MimeType::JSON);
    assert_eq!(
        field.get_property(&Scheme::HTTPS, "Content-Type"),
        field.as_http().content_type()
    );
    assert_eq!(
        field.get_metadata("http:content-type"),
        field.as_http().content_type()
    );
    assert_eq!(
        field.property_iter(&Scheme::POSTGRES).collect::<Vec<_>>(),
        [("type", "jsonb")]
    );
    ```

=== "Python"

    ```python
    from yggdryl import Field, MimeType

    field = Field("payload", "binary", nullable=False)

    field.set_parquet_field_id(17)
    field.metadata["field:init"] = "false"
    field.set_display("Raw payload")
    field.set_content_type("application/json; charset=utf-8")
    field.set_property("postgres", "type", "jsonb")

    assert field.parquet_field_id == 17
    assert field.metadata["PARQUET:field_id"] == "17"
    assert field.metadata["field:init"] == "false"

    # A straight key belongs to no protocol, so every protocol falls back to it.
    assert field.display == "Raw payload"
    assert field.postgres.display == "Raw payload"

    assert field.mime_type == MimeType.JSON
    assert field.get_property("https", "Content-Type") == field.content_type
    assert field.metadata["http:content-type"] == field.content_type
    assert list(field.property_iter("postgres")) == [("type", "jsonb")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, MimeType } = require('yggdryl')

    const field = new Field('payload', 'binary', false)

    field.setParquetFieldId(17)
    field.set('field:init', 'false')
    field.setDisplay('Raw payload')
    field.setContentType('application/json; charset=utf-8')
    field.setProperty('postgres', 'type', 'jsonb')

    assert.equal(field.parquetFieldId, 17)
    assert.equal(field.get('PARQUET:field_id'), '17')
    assert.equal(field.get('field:init'), 'false')

    // A straight key belongs to no protocol, so every protocol falls back to it.
    assert.equal(field.display, 'Raw payload')
    assert.equal(field.postgres.display, 'Raw payload')

    assert.ok(field.mimeType.equals(MimeType.JSON))
    assert.equal(field.getProperty('https', 'Content-Type'), field.contentType)
    assert.equal(field.get('http:content-type'), field.contentType)
    assert.deepEqual(field.propertyIter('postgres'), [{ key: 'type', value: 'jsonb' }])
    ```

A handful of keys mean something to the library, and each one has a typed accessor that parses and
canonicalizes on the way in and out. `PARQUET:field_id` is a signed 32-bit integer and is what
`parquet_field_id` reads; writing `"+00017"` through the mapping stores `"17"`, and writing
`"2147483648"` fails.
`field:init` is a reserved boolean: it is absent for an ordinary field, and setting it to `false`
marks a field a schema still declares but a constructor must not accept. `location` parses as a
[`Url`](uri.md), and `alias`, `comment` and `display` carry validated text: another name for the
column, a description, and a human-readable label. None of the three belongs to a protocol, and a
protocol view falls back to the straight `comment` and `display` when it names none of its own.
Catalog coordinates - a catalog, schema or table name - are protocol
properties rather than straight keys, because which catalog names a column is the protocol's
business: write them as `iceberg:table_name` or `glue:table_name`.

Anything shaped `scheme:name` is a protocol property, keyed by a known [`Scheme`](types.md). The
prefix is canonicalized, so `HTTPS:Content-Type`, `HTTP:content-type`, and `http:content-type` are
one entry, and `get_property` matches HTTP names case-insensitively. The `http:` family is the one
with parsing accessors on top - `content_type`, `content_length`, `mime_type`, `media_type`,
`location` - because a field is also how a remote resource describes itself. In Rust they belong to
the field's HTTP view, `as_http()` and `as_http_mut()`, not to `Field`: `as_http().location()` reads
`http:location` while `field.location()` stays the straight `location` key, and the receiver is what
says which. Python and JavaScript keep them as validated attributes on the field itself, where the
two are told apart by name instead - `content_type` and `http_location`, `contentType` and
`httpLocation`.

Setting `field:init` has named methods in Rust only (`set_init`, `is_init`, `with_init`); Python and
JavaScript write the reserved key through the mapping, which validates it exactly the same way.
`display` is named in all three (`set_display`/`display`/`remove_display`, plus Rust's consuming
`try_with_display`), on the field and on every protocol view.

### One protocol at a time

Spelling `scheme:name` at a call site means spelling it right in every branch it appears in. A
protocol view remembers the protocol instead, so the caller writes the bare name:

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme};

    let mut field = Field::new("price", DataType::Int64, false);

    field.as_iceberg_mut().insert("doc", "closing price")?;
    field.as_iceberg_mut().update([("schema-id", "3"), ("field-id", "7")])?;
    field.as_postgres_mut().insert("type", "numeric")?;
    field.as_digest_mut().set_component()?;
    field.as_identity_mut().update([("role", "primary"), ("nulls", "distinct")])?;
    field.as_partition_mut().update([("transform", "bucket[16]"), ("order", "0")])?;

    assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.as_iceberg().key("doc"), "iceberg:doc");
    assert_eq!(field.as_iceberg().len(), 3);
    assert!(field.as_mysql().is_empty());
    assert!(field.as_digest().is_component());
    assert_eq!(field.as_identity().get("role"), Some("primary"));
    assert_eq!(field.as_partition().get("transform"), Some("bucket[16]"));

    // It is a view of the one metadata map, not a copy of part of it.
    assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    assert_eq!(field.get_metadata("digest:role"), Some("component"));
    assert_eq!(field.metadata_len(), 9);

    // A protocol-scoped replacement leaves every other protocol alone.
    field.as_iceberg_mut().set([("doc", "close")])?;
    assert_eq!(field.as_iceberg().iter().collect::<Vec<_>>(), [("doc", "close")]);
    assert_eq!(field.as_postgres().get("type"), Some("numeric"));

    // The view is the field: it dereferences to one, and `as_field` hands back a
    // borrow that outlives the view rather than one that dies with it.
    assert_eq!(field.as_iceberg().dtype(), &DataType::Int64);
    let name = field.as_iceberg().as_field().name();
    assert_eq!(name, "price");

    // The protocol can also come from a value rather than from the code.
    assert_eq!(field.protocol(&Scheme::POSTGRES).get("type"), Some("numeric"));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("price", "int64", nullable=False)

    field.iceberg["doc"] = "closing price"
    field.iceberg.update({"schema-id": "3", "field-id": "7"})
    field.postgres["type"] = "numeric"
    field.digest["role"] = "component"
    field.identity.update({"role": "primary", "nulls": "distinct"})
    field.partition.update({"transform": "bucket[16]", "order": "0"})

    assert field.iceberg["doc"] == "closing price"
    assert field.iceberg.key("doc") == "iceberg:doc"
    assert len(field.iceberg) == 3
    assert not field.mysql
    assert field.digest["role"] == "component"
    assert field.identity["role"] == "primary"
    assert field.partition["transform"] == "bucket[16]"

    # It is a view of the one metadata mapping, not a copy of part of it.
    assert field.metadata["iceberg:doc"] == "closing price"
    assert field.metadata["digest:role"] == "component"
    assert len(field.metadata) == 9
    assert dict(field.iceberg.items())["field-id"] == "7"

    del field.iceberg["field-id"]
    assert "field-id" not in field.iceberg
    assert field.protocol("postgres")["type"] == "numeric"
    ```

    !!! note "Rust-only"
        The per-protocol view *types* - `HttpField`, `IcebergField`, `FixField`, `DigestField`,
        `IdentityField`, `PartitionField`, and the fifteen others, each
        carrying its protocol's typed vocabulary - are Rust-only for now. `field.iceberg` answers the
        generic property mapping shown here, and the validated HTTP values stay attributes on the
        field itself.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'int64', false)

    field.iceberg.set('doc', 'closing price')
    field.iceberg.update({ 'schema-id': '3', 'field-id': '7' })
    field.postgres.set('type', 'numeric')
    field.digest.set('role', 'component')
    field.identity.update({ role: 'primary', nulls: 'distinct' })
    field.partition.update({ transform: 'bucket[16]', order: '0' })

    assert.equal(field.iceberg.get('doc'), 'closing price')
    assert.equal(field.iceberg.key('doc'), 'iceberg:doc')
    assert.equal(field.iceberg.size, 3)
    assert.equal(field.mysql.size, 0)
    assert.equal(field.digest.get('role'), 'component')
    assert.equal(field.identity.get('role'), 'primary')
    assert.equal(field.partition.get('transform'), 'bucket[16]')

    // It is a view of the one metadata map, not a copy of part of it.
    assert.equal(field.get('iceberg:doc'), 'closing price')
    assert.equal(field.get('digest:role'), 'component')
    assert.equal(field.size, 9)
    assert.deepEqual([...field.iceberg].sort(), [['doc', 'closing price'], ['field-id', '7'], ['schema-id', '3']])

    assert.equal(field.iceberg.delete('field-id'), true)
    assert.equal(field.iceberg.has('field-id'), false)
    assert.equal(field.protocol('postgres').get('type'), 'numeric')
    ```

    !!! note "Rust-only"
        The per-protocol view *types* - `HttpField`, `IcebergField`, `FixField`, `DigestField`,
        `IdentityField`, `PartitionField`, and the fifteen others, each
        carrying its protocol's typed vocabulary - are Rust-only for now. `field.iceberg` answers the
        generic property `Map` shown here, and the validated HTTP values stay accessors on the field
        itself.

The view is a borrow, not a snapshot: it reads out of the field's own metadata and writes through the
field's own cache-aware mutation, so two views of one field see each other's writes and a protocol
write invalidates a populated Arrow projection exactly as a direct metadata write does. In Rust the
borrow is of the whole field: the view dereferences to `Field`, so one value answers both the
protocol's properties and everything a field answers, and `as_field` is the spelling whose `&Field`
outlives the view rather than dying with it. Every well-known protocol has a named accessor -
`as_iceberg`, `as_digest`, `as_identity`, `as_partition`, `as_postgres`, `as_http`,
`as_arrow_properties`, `as_spark`, `as_s3`, and the rest of the [`Scheme`](types.md) vocabulary,
spelled `iceberg`, `digest`, `identity`, `partition`, `postgres`, `http` and so on in the bindings -
and `protocol` takes one that is only known at runtime. There is no `https` accessor, because HTTPS
shares the canonical `http:` namespace; the view for either scheme reports `http` as its prefix.

`IdentityField` and `PartitionField` are deliberately generic protocol views. They carry arbitrary
inert text under `identity:<property>` and `partition:<property>` for systems that need more than a
single key flag: role, ordering, comparison, transform, source, or another protocol-owned fact all
use the same lookup, iteration, replacement, and cache-aware mutation contract. These annotations
do not by themselves change row-layout behavior. The separate `field:partition` boolean remains the
authoritative marker read by `partition_fields` and storage routing.

A protocol that has a typed vocabulary carries it on its own view and nowhere else. Digest roles,
holder paths, and holder algorithms are on `DigestField` and `DigestFieldMut`; the `http:` headers
are on `HttpField` and `HttpFieldMut`;
Iceberg's `doc`, `schema_id`, `spec_id` and `transform` are on
[`IcebergField` and `IcebergFieldMut`](media.md#a-field-carries-its-own-iceberg-vocabulary),
and FIX's `branch`, `id`, `tag`, `tags`, `aliases` and `description` are on
[`FixField` and `FixFieldMut`](fix.md),
which is why deleting a protocol's namespace never touches `Field`. `Field` keeps only what is its
own state - `field:init`, `field:partition`, `alias`, `comment`, `display`, `location`, and the
reserved `PARQUET:field_id` - whatever key that state is stored under.

Rust's `set` is the one operation that is not a plain map write: it replaces exactly this protocol's
properties and leaves every other key untouched, which is what a protocol-scoped assignment has to
mean when one map holds them all. The bindings expose the mapping and `update` but not that
replacement, for the same reason they expose no whole-metadata `set`: in Python `set` on a mapping
means one key, and in JavaScript `Map.set` does too.

### Digest components and holders

`digest:role` has exactly two values. A `component` contributes to a row digest; a `holder` stores
the result and never contributes to its own recomputation. One or more explicit components are the
exact input set. With no explicit component, every non-holder field contributes. Rust names the
namespace `Scheme::DIGEST`. `DigestField` answers `is_holder`, `is_component`, `algorithm`, and
`paths`; `DigestFieldMut` answers `set_holder`, `set_component`, `set_algorithm`, `remove_algorithm`,
`set_paths`, `remove_paths`, and `remove_role`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let id = Field::new("id", DataType::Int64, false);
    let price = Field::new("price", DataType::Float64, false);
    let mut stored = Field::new("row_digest", DataType::UInt64, false);
    stored.as_digest_mut().set_holder()?;

    let fallback = DataType::from_fields([id.clone(), price.clone(), stored.clone()])?
        .required_field("row");
    assert_eq!(fallback.digest_field_names().collect::<Vec<_>>(), ["id", "price"]);

    let mut id = id;
    id.as_digest_mut().set_component()?;
    let explicit = DataType::from_fields([id, price, stored])?.required_field("row");
    assert!(explicit.has_digest_components());
    assert_eq!(explicit.digest_field_names().collect::<Vec<_>>(), ["id"]);
    assert_eq!(explicit.only_digest_fields()?.field_len(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    identifier = Field("id", "int64", nullable=False)
    price = Field("price", "float64", nullable=False)
    stored = Field("row_digest", "uint64", nullable=False)
    stored.digest["role"] = "holder"

    fallback = Field(
        "row", DataType.from_fields([identifier, price, stored]), nullable=False
    )
    assert fallback.digest_field_names == ["id", "price"]

    identifier.digest["role"] = "component"
    explicit = Field(
        "row", DataType.from_fields([identifier, price, stored]), nullable=False
    )
    assert explicit.has_digest_components
    assert explicit.digest_field_names == ["id"]
    assert len(explicit.only_digest_fields().dtype) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const identifier = new Field('id', 'int64', false)
    const price = new Field('price', 'float64', false)
    const stored = new Field('row_digest', 'uint64', false)
    stored.digest.set('role', 'holder')

    const fallback = new Field(
      'row', DataType.fromFields([identifier, price, stored]), false,
    )
    assert.deepEqual(fallback.digestFieldNames(), ['id', 'price'])

    identifier.digest.set('role', 'component')
    const explicit = new Field(
      'row', DataType.fromFields([identifier, price, stored]), false,
    )
    assert.equal(explicit.hasDigestComponents, true)
    assert.deepEqual(explicit.digestFieldNames(), ['id'])
    assert.equal(explicit.onlyDigestFields().dtype.length, 1)
    ```

Selection reads direct Struct children in declaration order. The effective fields, names, and count
come from `digest_fields`, `digest_field_names`, and `digest_field_len`; `only_digest_fields`
projects the same selection. A root whose children are all holders selects no values, which is the
empty ordered sequence. Any metadata entry point rejects a `digest:role` other than `holder` or
`component`, leaving a failed mutation unchanged. Arrow row hashing uses this same contract in
[`row_digests`](xxhash.md#arrow-row-digests).

`digest:paths` is a holder-local ordered JSON array; absence retains the component fallback while
`[]` selects the empty sequence. `digest:algorithm` is an optional canonical
[`DigestAlgorithm`](xxhash.md#digest-values); its storage must match the result width:
XXH32 uses `uint32`, XXH64 and XXH3-64 use `uint64`, and XXH3-128 uses
`fixed_size_binary(16)`. Typed setters require `holder`, validate the width, and fail atomically.
Changing or removing that role first requires removing `algorithm` and `paths`. Generic metadata
writes canonicalize the algorithm token and path JSON; the Arrow fill operation validates their
holder ownership and datatype together. Path resolution, nested-holder reuse, algorithm fallback,
and batch filling are specified in [Filling digest holders](xxhash.md#filling-digest-holders).

### A field can be a partition column

Nothing in a batch says which of its columns belong in a directory name, so a schema that means to be
stored partitioned says so on the columns themselves:

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let schema = DataType::from_fields([
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("venue"),
        DataType::Int64.required_field("price"),
    ])?
    .required_field("row")
    .with_partition_fields(&["year", "venue"])?;

    assert!(schema.has_partition_fields());
    assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year", "venue"]);
    assert!(schema.get_field_by_path("year").expect("the column").is_partition());

    // The two halves of the layout: what a path spells, and what a leaf stores.
    assert_eq!(schema.without_partition_fields()?.field_len(), 1);
    assert_eq!(schema.only_partition_fields()?.field_len(), 2);

    // The mark is reserved metadata, so it round-trips like any other.
    assert_eq!(
        schema.get_field_by_path("year").expect("the column").get_metadata("field:partition"),
        Some("true")
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    schema = Field(
        "row",
        DataType.from_fields([
            Field("year", "int32", nullable=False),
            Field("venue", "string", nullable=False),
            Field("price", "int64", nullable=False),
        ]),
        nullable=False,
    ).with_partition_fields(["year", "venue"])

    assert schema.has_partition_fields
    assert schema.partition_field_names == ["year", "venue"]
    assert schema.dtype["year"].is_partition
    assert not schema.dtype["price"].is_partition

    assert len(schema.without_partition_fields().dtype) == 1
    assert len(schema.only_partition_fields().dtype) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('year', 'int32', false),
        new Field('venue', 'string', false),
        new Field('price', 'int64', false),
      ]),
      false,
    ).withPartitionFields(['year', 'venue'])

    assert.equal(schema.hasPartitionFields, true)
    assert.deepEqual(schema.partitionFieldNames(), ['year', 'venue'])
    assert.equal(schema.dtype.getFieldByPath('year').isPartition, true)
    assert.equal(schema.dtype.getFieldByPath('price').isPartition, false)

    assert.equal(schema.withoutPartitionFields().dtype.length, 1)
    assert.equal(schema.onlyPartitionFields().dtype.length, 2)
    ```

The mark is the reserved `field:partition` key, so it travels wherever field metadata travels - into
Arrow, into a Parquet footer, through a JSON round trip - and a field that is not a partition column
carries no marker at all, which keeps two schemas that partition the same way exactly equal. A folder
write reads the marks to lay an empty tree out, a folder read puts them back on the columns it
restored from the path, and Iceberg builds an identity spec from them; that whole story is in
[storage](holder.md#partition-columns-in-the-data) and [Iceberg](media.md#partition-specs-and-the-hive-layout).

### Typed field aliases

=== "Rust"

    ```rust
    use yggdryl::types::{Int64Field, DateTime64Field, Utf8Field, integer};
    use yggdryl::{DataType, Field, TimeUnit, Timezone};

    let id = Int64Field::new("id", false);
    let symbol = Utf8Field::from_parts("symbol", true, [("source", "feed")])?;
    let at = DateTime64Field::try_new("at", DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }, false)?;

    // A typed field derefs to the field it wraps.
    assert_eq!(id.name(), "id");
    assert_eq!(symbol.get_metadata("source"), Some("feed"));
    assert_eq!(at.dtype().to_string(), "datetime64(us)");

    // The marker is checked, never assumed.
    assert!(
        Field::new("id", DataType::Utf8, false)
            .try_into_typed::<integer::Int64Type>()
            .is_err()
    );
    assert_eq!(id.into_field().dtype(), &DataType::Int64);
    ```

=== "Python"

    ```python
    from yggdryl import Field, types

    id_field = types.int64("id", nullable=False)
    symbol = types.utf8("symbol", metadata={"source": "feed"})
    at = types.datetime64("at", "us", nullable=False)

    assert isinstance(id_field, Field)
    assert str(id_field.dtype) == "int64"
    assert symbol.metadata["source"] == "feed"
    assert str(at.dtype) == "datetime64(us)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const id = fields.int64('id')
    const symbol = fields.utf8('symbol', { nullable: true, metadata: { source: 'feed' } })
    const at = fields.datetime64('at', 'us')

    assert.ok(id instanceof Field)
    assert.equal(id.dtype.toString(), 'int64')
    assert.equal(symbol.get('source'), 'feed')
    assert.equal(at.dtype.toString(), 'datetime64(us)')
    ```

`Int64Field` and the fifty-six aliases beside it are `TypedField<K>`, one `Field` plus a
zero-sized sealed marker, `repr(transparent)` and exactly the size of the field it holds. The
marker constrains the variant only: a decimal's precision, a datetime's unit, a list's child all
stay in the wrapped field, so the typed view never duplicates schema state. `try_as_typed`
borrows a `TypedFieldRef` without allocating, `try_into_typed` consumes, and there is no
`DerefMut` - replacing the datatype through a generic reference could violate `K`, so `set_dtype`
on a typed field re-checks the marker and leaves the value untouched when it fails.

Aliases with a statically known datatype get a `new(name, nullable)` that cannot fail, plus
`from_parts(name, nullable, metadata)`; parameterized ones take the datatype through `try_new`. In
Python and JavaScript the aliases are static views over the same native class: `yggdryl.types.int64`
returns an ordinary `Field`, typed as `Int64Field` for a checker only. Watch the default -
`nullable` defaults to `True` in Python and `true` in JavaScript.

[Variant, geometry, and geography](types.md#variant-geometry-and-geography) follow the same
pattern: `yggdryl::types::VariantField` is parameterless and gets the static `new(name, nullable)`,
while `GeometryField` and `GeographyField` are parameterized by CRS and edge algorithm, so they
take their datatype through `try_new`. The binding-side `VariantField`, `GeometryField`, and
`GeographyField` aliases beside `fields.variant`, `fields.geometry`, and `fields.geography` are
checker-level views over the one native class exactly like every alias above.
The [ASCII widths](types.md#ascii-widths-and-the-registered-codes) are parameterless
too: `Ascii16Field`, `Ascii24Field`, `Ascii32Field`, `Ascii64Field`, `Ascii96Field`, and
`Ascii128Field` get the static `new`, and the bindings add `fields.ascii(name, width)` over
`DataType.ascii` beside the six per-width factories. The four registered codes are parameterless
too: `CountryField`, `CurrencyField`, `MicField`, and `CfiField` get the static `new` beside
`fields.country`, `fields.currency`, `fields.mic`, and `fields.cfi`, each building the code's own
datatype rather than the ASCII width that would hold the same bytes.
[The UUID](types.md#the-uuid) is parameterless in the same way: `UuidField` gets the static
`new`, and the bindings spell it `fields.uuid`.
[Versions](types.md#versions) follow that parameterless shape through `VersionField`,
`types.version`, and `fields.version`.

### Converting to one native field

Python's canonical runtime signature is `field(value, name=None) -> Field`; JavaScript spells it
`intoField(value, name = null)`. The value comes first and the optional replacement name comes
second. Omitting the name, passing `None`/`null`, or passing the field's existing name returns the
cached/native value itself; another name returns a renamed clone. A non-string name or a value that
cannot describe a field is a `TypeError`.

A scalar-decorated or otherwise structured class exposes its root separately. Python uses the
zero-argument cached staticmethod `Class.field() -> StructField`; JavaScript requires the actual
static getter `Class.intoStructField`. It must answer a non-null Struct field. Python's `@scalar`
decorator installs the owner-capturing accessor; JavaScript's global `intoField` validates the
getter descriptor and memoizes its result. Rust stays precise rather than dynamically inferring: `TypedField<K>::into_field(self)`
returns the generic field, and `StructField::into_struct_field(self)` is the return-typed Struct
spelling. None of these accessors introduces a second schema object.

#### What cached field access costs

The Rust target measures the two consuming typed accessors with construction
outside the timer. The binding targets keep the cached class or native Field
alive across calls and price a renamed clone separately:

```console
cargo bench -p yggdryl --bench field --all-features -- into_ --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
cd python && .venv/Scripts/python benchmarks/types.py --iterations 10000
npm run --prefix node bench:schema
```

One local Windows x86_64 release run (Criterion point estimates for Rust;
median time per call for Python; whole-loop rate for JavaScript):

| runtime operation | estimate |
| --- | ---: |
| Rust `TypedField::into_field` | 41.5 ns |
| Rust `StructField::into_struct_field` | 34.7 ns |
| Python cached `Class.field()` | 677 ns |
| Python global `field(Class)` | 1.27 us |
| Python renamed `field(Class, name=...)` | 9.26 us |
| JavaScript `intoField(nativeField)` | 40.0 ns (25.0M calls/s) |
| JavaScript cached `intoField(Class)` | 72.0 ns (13.9M calls/s) |
| JavaScript renamed `intoField(Class, name)` | 33.6 us (29.7k calls/s) |

The cached class paths return the same native value rather than rebuilding its
annotation graph. A replacement name must clone and validate the Field, which
is why it remains visible as a separate case.

### Row values are validated against the root

Validating and canonicalizing a [`Scalar`](types.md) against a struct root is Rust-only. Python and
JavaScript reconcile Arrow data instead.

```rust
use yggdryl::{DataType, Field, Scalar};

let schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Float32.nullable_field("price"),
])?
.required_field("trade");

// A row is one ordered sequence with one value per struct child.
let row = Scalar::from_sequence([Scalar::from(7u64), Scalar::from(0.1f64)]);
schema.validate_value(&row)?;

// Canonicalizing narrows every value into the representation the root declares.
let canonical = schema.canonicalize_value(row)?;
assert_eq!(canonical.get(0), Some(&Scalar::from(7_i64)));
assert_eq!(
    canonical.get(1).and_then(Scalar::as_f64),
    Some(f64::from(0.1f32))
);

// A value that does not fit names the path walked to reach it.
let wrong = Scalar::from_sequence([Scalar::from("seven"), Scalar::Null]);
let message = schema.validate_value(&wrong).unwrap_err().to_string();
assert!(message.contains("$.trade.id"), "{message}");
```

The two calls answer different questions. `validate_value` asks whether the row is *representable*:
right arity, no null in a required column, every scalar inside its declared range. It accepts a
`U64` where an `Int64` is declared, because the value fits. `canonicalize_value` then rewrites the
row into the exact representation - that `U64` becomes an `I64`, an `f64` bound for a `Float32`
column is rounded through `f32` - and returns the input untouched when nothing needed changing, so
a correctly built row costs nothing. Both walk the schema, not the value, and both report the
dot/bracket path of the first thing that does not fit.

### Serializing a schema

`Field` reads and writes the three structured-text formats through **one** structural model. There
is exactly one `Field` ⇄ `Scalar` mapping - `into_value`/`from_value` in Rust, `into_dict`/`from_dict` in
Python - and JSON, YAML, and TOML are three writers over it, so the three agree by construction
rather than by three sets of tests. That is also what makes a schema *embeddable*: a configuration
document can carry a declared schema inline beside the rest of its settings, with no
JSON-string-inside-YAML awkwardness.

The shape is what the JSON emit has always been: `name`, `dtype`, `nullable`, then
`dictionary_id` only when it is non-zero and `dictionary_is_ordered` only when it is set, then
`metadata`. An unset optional attribute is **omitted**, never emitted as null - which is also why
TOML, which has no null, loses nothing on the way out.

Each format takes the shared [`Formatting`](text.md#formatting) option; Python spells it as an
`indent` keyword.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};
    use yggdryl::types::Scalar;

    let field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;

    // One structural model, three formats over it.
    assert_eq!(Field::from_value(field.clone().into_value())?, field);
    assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
    assert_eq!(Field::from_yaml(&field.clone().into_yaml()?)?, field);
    assert_eq!(Field::from_toml(&field.clone().into_toml()?)?, field);

    // The mapping is the shared `Scalar`, so it drops into any document.
    let shape = field.into_value();
    assert_eq!(shape.get_key_str("name").and_then(Scalar::as_utf8), Some("price"));
    // Unset optional attributes are absent rather than null.
    assert!(shape.get_key_str("dictionary_id").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})

    assert Field.from_dict(field.into_dict()) == field
    assert Field.from_json(field.into_json()) == field
    assert Field.from_yaml(field.into_yaml()) == field
    assert Field.from_toml(field.into_toml()) == field

    shape = field.into_dict()
    assert shape["name"] == "price"
    assert "dictionary_id" not in shape
    ```

=== "JavaScript"

    !!! note "Rust first"
        The YAML and TOML pair lands in the JavaScript binding once the core surface settles;
        `toJSON` is already there.

### A readable rendering

`Display` - and Python's `str`/`repr` - is the compact constructor form, and it stays exactly as it
is: it round-trips through `from_str`, and the error messages, the documentation, and Python's
`repr` all depend on that. It is also unreadable the moment a struct nests three levels deep.

The readable form is the **alternate**: `{:#}` in Rust, or the named `pretty()` adapter that backs
it, and `pretty()` in Python. One fact per line, one indent per nesting level, and only the
attributes that are actually set - a `dictionary_id` of `0` or empty metadata is noise the compact
form already omits. Metadata renders as indented `@key = value` lines rather than one braced blob.
The output is stable across runs; nothing in it iterates a hash map.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])?
            .nullable_field("line"),
    ])?
    .required_field("order");

    // Compact still round-trips.
    assert_eq!(Field::from_str(&order.to_string())?, order);

    // Readable is the alternate, or the named adapter - one implementation.
    assert_eq!(format!("{order:#}"), order.pretty().to_string());
    assert_eq!(
        format!("{order:#}"),
        concat!(
            "order: struct[2], required\n",
            "  id: int64, required\n",
            "  line: struct[1], nullable\n",
            "    price: float64, required",
        ),
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    order = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
            ),
        ]),
        nullable=False,
    )

    # `repr` is unchanged - the eval-round-trip form Python expects.
    assert repr(order).startswith("Field.from_str(")
    assert Field.from_str(str(order)) == order

    assert order.pretty() == (
        "order: struct[2], required\n"
        "  id: int64, required\n"
        "  line: struct[1], nullable\n"
        "    price: float64, required"
    )
    ```

=== "JavaScript"

    !!! note "Rust first"
        `pretty` lands in the JavaScript binding once the core surface settles.


### Comparing two fields

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let left = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
    let right = Field::from_parts("price", DataType::Float64, true, [("venue", "XNAS")])?;

    assert!(!left.equals(&right, true));
    assert_eq!(
        left.show_diffs(&right, true, false).collect::<Vec<_>>(),
        [
            "≠ $.nullable: false → true",
            "≠ $.metadata[\"venue\"]: \"XPAR\" → \"XNAS\"",
        ]
    );
    assert_eq!(left.show_diff(&left, true, true), "✓ equal");
    assert_eq!(left.show_diff(&left, true, false), "");
    ```

=== "Python"

    ```python
    from yggdryl import Field

    left = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    right = Field("price", "float64", metadata={"venue": "XNAS"})

    assert not left.equals(right)
    assert list(left.show_diffs(right)) == [
        "≠ $.nullable: false → true",
        '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
    ]
    assert left.show_diff(left) == "✓ equal"
    assert left.show_diff(left, return_equal=False) == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const left = new Field('price', 'float64', false, { venue: 'XPAR' })
    const right = new Field('price', 'float64', true, { venue: 'XNAS' })

    assert.equal(left.equals(right), false)
    assert.deepEqual([...left.showDiffs(right)], [
      '≠ $.nullable: false → true',
      '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
    ])
    assert.equal(left.showDiff(left), '✓ equal')
    assert.equal(left.showDiff(left, true, false), '')
    ```

`equals` answers yes or no and takes `with_metadata`, which drops metadata from the comparison at
every depth rather than only at the root. `show_diffs` answers *why*, one line at a time: it is a
lazy iterator (`Differences` in Rust, borrowing both sides; `OwnedDifferences` when the lines must
outlive them), so a thousand-key metadata difference streams instead of building a report.
`show_diff` joins the lines into one string.

Both diff calls take `return_equal`, which decides what an equal comparison reports: nothing at all,
or exactly one `✓ equal` line. `show_diffs` defaults it to false and `show_diff` to true in the
bindings, which is why an equal `show_diff` prints a marker and an equal `show_diffs` yields
nothing. Paths are `$`-rooted and name the part that changed - `$.nullable`, `$.dtype.length`,
`$.metadata["venue"]`, `$.fields[2]` - so a diff line is a place, not a prose sentence. The same
two calls exist on [`DataType`](types.md) with the same output.

### Casting Arrow data through a field

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
    use yggdryl::types::Int64Field;
    use yggdryl::{ArrowCast, DataType, Field};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));

    // Any field answers with an ArrayRef, because any field could be any datatype.
    let field = Field::new("id", DataType::Int64, false);
    let cast = field.cast_arrow_array(Arc::clone(&text), false)?;
    assert_eq!(cast.data_type(), &arrow_schema::DataType::Int64);

    // A typed field already knows its variant, so it answers with the array itself.
    let typed = Int64Field::new("id", false);
    let ids: Int64Array = typed.cast_arrow_array(text, false)?;
    assert_eq!(ids.values(), &[1, 2]);

    // safe nulls a failed conversion; a non-null field then defaults it.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    assert!(typed.cast_arrow_array(Arc::clone(&broken), false).is_err());
    let repaired: Int64Array = typed.cast_arrow_array(broken, true)?;
    assert_eq!(repaired.values(), &[1, 0]);
    assert_eq!(repaired.null_count(), 0);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    field = Field("id", "int64", nullable=False)

    ids = field.cast_arrow_array(pa.array(["1", "2"]))
    assert ids.equals(pa.array([1, 2], type=pa.int64()))

    # safe nulls a failed conversion; a non-null field then defaults it.
    repaired = field.cast_arrow_array(pa.array(["1", "not a number"]))
    assert repaired.equals(pa.array([1, 0], type=pa.int64()))
    assert repaired.null_count == 0

    try:
        field.cast_arrow_array(pa.array(["1", "not a number"]), safe=False)
    except ValueError:
        pass
    else:
        raise AssertionError("an unsafe cast must fail")
    ```

The field is always the *target*: an incoming array is reconciled to the field's datatype and
nullability, never the other way around. `ArrowCast` is implemented for both `Field` and
[`DataType`](types.md) and returns an `ArrayRef`, because a generic field could be any datatype.
A `TypedField` has already committed to a variant, so `Int64Field::cast_arrow_array` returns an
`Int64Array` and the caller reads values without a downcast; `cast_arrow_scalar` does the same for a
one-element array. A few variants keep an `ArrayRef` return - a datetime's unit and a dictionary's
key type decide the physical array, so there is no single concrete type to name.

`safe` is Arrow's own cast option. When it is true a supported conversion failure becomes null, and
a non-nullable field then replaces that null with its canonical default (`Field::default_value`);
when it is false the failure is an error. A nullable field keeps the null either way.

Text and temporals cross through this crate's own spellings rather than Arrow's, so a column and a
row answer alike. Reading text into a `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`,
`Duration32` or `Duration64` accepts everything [text](text.md#field-directed-parsing) accepts - a
grouped fraction, an hour past the end of the day, a bracketed zone name, a duration in either
spelling, which Arrow reads into no duration at all - and a reading this crate takes but the
declared unit or width cannot hold exactly is null, never a rounded value. Arrow's kernel answers
only the spellings this crate cannot read at all, such as a bare date entering a datetime, a
twelve-hour clock or a compact `YYYYMMDD`, so nothing that read before stops reading. The other
direction spells the classic form back, a zoned instant included, which Arrow's own formatter
refuses without its timezone database.

The same trait reconciles a whole record batch to a struct root.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCast, DataType, Field};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("trade");

    let source = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, true),
            ArrowField::new("id", ArrowDataType::Int32, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["ACME"])),
            Arc::new(Int32Array::from(vec![7])),
        ],
    )?;

    let batch = schema.cast_arrow_batch(source, false)?;
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");
    assert_eq!(batch.column(0).data_type(), &ArrowDataType::Int64);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import DataType, Field, types

    schema = Field(
        "trade",
        DataType.from_fields([
            types.int64("id", nullable=False),
            types.utf8("symbol"),
        ]),
        nullable=False,
    )

    source = pa.record_batch({
        "symbol": pa.array(["ACME"]),
        "id": pa.array([7], type=pa.int32()),
    })

    batch = schema.cast_arrow_batch(source)
    assert batch.schema.names == ["id", "symbol"]
    assert batch.column("id").type == pa.int64()
    ```

Children are selected in target order by ASCII-case-insensitive name, so column order in the source
does not matter. Extra source columns are dropped, a missing nullable column is null-filled, and a
missing required column is filled with its canonical default. An already exact batch is returned
unchanged - the same arrays, not copies - which is what makes a cast safe to put in front of every
read. A `RecordBatch` is a `StructArray` plus a schema, so there is no second engine here: the batch
goes through the same recursive field cast an array does.

### The generic cast

One name, the kind inferred and kept. In Python, `field.cast_arrow(value)` takes whatever
Arrow-shaped thing you hold and hands back the same kind, cast to the field: a `pyarrow` `Scalar`,
`Array`, `ChunkedArray`, `RecordBatch`, `Table`, `RecordBatchReader`, `Dataset`, or `Scanner`; a
`polars` `DataFrame`, which crosses at the newest compat level so view arrays stay view arrays; a
`polars` `LazyFrame`, which *stays lazy* - its schema is read with `collect_schema`, which computes
no rows, and the cast is mapped over the engine's batches, so nothing is collected until you
collect; and a `pandas` `DataFrame` or `Series`, which crosses through Arrow and comes back as
itself. Streams are cast batch by batch. `field.cast(value)` is the same dispatch with plain Python
values allowed - a bare `5` becomes the typed scalar the field declares. `cast_arrow_scalar` and
`cast_arrow_batch` are the spelled-out single-kind names.

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    schema = Field("row", DataType("struct<id: int64, symbol: string>"), False)
    table = pa.table({"id": pa.array([1, 2], pa.int32()), "symbol": ["AAPL", "MSFT"]})

    # A table comes back a table, a reader a reader, a frame a frame.
    cast = schema.cast_arrow(table)
    assert cast.schema.field("id").type == pa.int64()

    # The generic name also takes plain values, as the typed scalar.
    price = Field("price", DataType("int64"), False)
    assert price.cast(5).as_py() == 5
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, fields } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    })

    // Whatever Arrow JS holds casts batch by batch and comes back a Table.
    const cast = schema.castArrow(table)
    assert.equal(cast.numRows, 2)
    assert.ok(schema.cast(table).numRows === 2)
    ```

See [Python](extensions/python.md) and [JavaScript](extensions/javascript.md) for what each
binding adds on top of this field.

## Shared vocabulary


Each shared enum lives in its named root file and is re-exported at the crate root.

| Type | Contract |
| --- | --- |
| `DataTypeId`, `DataTypeKind` | Exact datatype identity and family |
| `Codec`, `Level` | Content coding and the shared 0–9 compression scale |
| `DigestAlgorithm`, `Digest`, `Digester` | Hash algorithm identity, the value it answers, and the runtime-selected streaming state |
| `MimeType`, `MediaType` | Base representation and ordered content codings |
| `Scheme` | URI and compatibility schemes |
| `IOKind`, `IOMode` | Resource kind and I/O intent |
| `TimeUnit`, `Timezone` | Temporal resolution and zone |
| `UnionMode`, `EdgeAlgorithm` | Union layout and geospatial edge model |

`IOMode` contains `overwrite`, `append`, `merge`, `readonly`, and `random`.
Write entry points reject the two non-write modes.

=== "Rust"

    ```rust
    use yggdryl::{DataTypeId, IOMode, TimeUnit};

    assert_eq!(DataTypeId::Int64.as_str(), "int64");
    assert_eq!(TimeUnit::Millisecond.as_str(), "ms");
    assert_eq!(IOMode::ReadOnly.as_str(), "readonly");
    ```

=== "Python"

    ```python
    from yggdryl import enums

    assert "int64" in enums.DATA_TYPE_IDS
    assert enums.IO_MODES == ("overwrite", "append", "merge", "readonly", "random")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { enums } = require('yggdryl')

    assert.ok(enums.dataTypeIds.includes('int64'))
    assert.deepEqual(enums.ioModes, ['overwrite', 'append', 'merge', 'readonly', 'random'])
    ```

`Enum` retains the vocabulary, spelling, and compact member index. Natural
JSON/YAML/TOML and host projections emit the spelling.

=== "Rust"

    ```rust
    use yggdryl::{Enum, IOMode, Scalar};

    let value = Scalar::from(IOMode::Append);
    let member = value.as_enum().expect("an enum scalar");
    assert_eq!(member, &Enum::IOMode(IOMode::Append));
    assert_eq!((member.kind(), member.as_str(), member.ordinal()), ("io_mode", "append", 1));
    ```

=== "Python"

    ```python
    from yggdryl import Scalar

    value = Scalar.from_enum("io_mode", "append")
    assert (value.enum_kind, value.enum_value, value.enum_ordinal) == ("io_mode", "append", 1)
    assert value.as_py() == "append"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    const value = Scalar.fromEnum('io_mode', 'append')
    assert.deepEqual([value.enumKind, value.enumValue, value.enumOrdinal], ['io_mode', 'append', 1])
    assert.equal(value.asJs(), 'append')
    ```

Release measurements on Windows x86_64, AMD Ryzen 5 150, rustc 1.96.1,
CPython 3.12.13, and Node 24.18.0 (2026-08-24):

| boundary | construct | kind | spelling | ordinal |
| --- | ---: | ---: | ---: | ---: |
| Rust | 28.9 ns | 2.59 ns | 5.01 ns | 3.94 ns |
| Python | 200 ns | 98.8 ns | 100 ns | 73.7 ns |
| JavaScript | 3 us | 2 us | 2 us | 1 us |

Regenerate with `cargo bench --bench datatype --all-features -- "value/enum_"`,
`python benchmarks/scalars.py --iterations 10000`, and
`node benchmarks/codec.js`.

`MAGIC_PROBE_LEN` bounds content inference. `Codec`, `MimeType`, and
`MediaType` own suffix and coding inference; consumers do not duplicate it.
`MimeType::PUFFIN` / `MimeType.PUFFIN` uses Yggdryl's canonical
`application/vnd.apache.puffin`, `.puffin`, and the specified `PFA1` magic;
the Puffin specification assigns no MIME name.

!!! note "Mostly Rust"
    The bindings expose static vocabularies through `yggdryl.enums` in Python
    and `enums` in JavaScript; `Scalar.from_enum` / `Scalar.fromEnum` preserves
    a member's core identity. `TypedScalar` and the
    `wkb` reader are Rust-only; a geospatial value crosses the bindings as
    its plain WKB bytes. `yggdryl.enums` also carries the ten
    [ASCII bases](extensions/python.md#ascii-vocabularies-as-enums) - the six
    widths and the four registered codes - a Python caller declares a
    vocabulary with; they are Python-only, because they are the packed ASCII
    codes behind that language's own enum protocol.
    The declaration they build is the shared `AsciiEnum`, which every runtime
    reads off the field that stores it.

```rust
use yggdryl::holder::Holder;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

// A value that could have been any handle. The calls do not change.
let mut handle = Holder::buffer(Buffer::new());
handle.write_all_bytes(b"AAPL,1\n")?;

assert_eq!(handle.read_all_bytes()?, b"AAPL,1\n");
assert_eq!(handle.kind(), yggdryl::IOKind::Memory);
```

A trait says what an implementation must do; the enum beside it says which implementations exist. The two are not interchangeable: `Box<dyn IOBase>` erases the concrete type, and [`IOBase::parent`, `child_by_path`, and `ls`](holder.md) have to return *some* handle that a caller can still match on. Their signatures name `Holder`, so the enum has to be sized, `Send`, and a full implementation of the contract itself.

That last part is what makes the enums invisible in use. Each one delegates every method of its contract to the variant it holds, so code written against the enum behaves exactly as code written against the implementation would - and a variant is still there to match when the concrete type matters.


## Scalar families


`Scalar` keeps exact physical variants for Arrow, but family constructors and
views avoid width cascades. Units select date/time widths, duration selects the
narrowest fitting count, decimal selects by coefficient, and datetime remains
64-bit because Arrow timestamps are 64-bit.

```rust
use yggdryl::{I256, Scalar, TemporalFamily, TimeUnit, Timezone};

let date = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
let time = Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE)?;
let duration = Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)?;
let decimal = Scalar::from_decimal(I256::from_i128(1_250), 2);

assert_eq!(date.as_date().unwrap().bit_width(), 32);
assert_eq!(time.as_time().unwrap().bit_width(), 64);
assert_eq!(duration.as_duration().unwrap().bit_width(), 64);
assert_eq!(time.as_temporal().unwrap().family(), TemporalFamily::Time);
assert_eq!(decimal.as_decimal(), Some((I256::from_i128(1_250), 2)));
```

`as_integer`, `as_float`, `as_decimal`, and `as_temporal` are the shared core
views used by validation, arithmetic, and both extensions. Exact constructors
remain available in Rust when a physical Arrow identity is required. All views
preserve total equality, ordering, and hash semantics.


## TypedScalar: one value and its datatype


This module also owns the value every part of the project speaks, and the pairing beside it. `Scalar` is documented with the [structured text](text.md) that parses into it; `TypedScalar` is one value and one datatype, checked against each other, with one alias per datatype for a caller who knows which is coming.

```rust
use yggdryl::types::{Int64Scalar, TypedScalar};
use yggdryl::{DataType, Scalar};

let price = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))?;
assert_eq!(price.dtype(), &DataType::Int64);

// The same pairing, with the datatype fixed at compile time.
let typed: Int64Scalar = price.try_into_typed()?;
assert_eq!(typed.value(), &Scalar::from(7_i64));
assert!(Int64Scalar::new(Scalar::from("seven")).is_err());
```

When no schema was supplied, `Scalar` can expose the exact `Field` the core
already inferred. The three names describe the expected shape and the returned
type: a scalar yields `value`, an outer sequence yields its `item`, and named
record rows yield a non-null Struct root named `row`. Empty or positional rows
remain ambiguous and require a declared Field; neither binding reimplements
this inference.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    let scalar = Scalar::from(42_i64).inferred_scalar_field()?;
    let array = Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null]);
    let row = Scalar::from_record([("id", Scalar::from(1_i64))])?;
    let rows = Scalar::from_sequence([row]);

    assert_eq!(scalar.name(), "value");
    assert_eq!(array.inferred_array_field()?.name(), "item");
    assert_eq!(rows.inferred_struct_field()?.name(), "row");
    ```

=== "Python"

    ```python
    from dataclasses import dataclass

    from yggdryl import Scalar

    @dataclass
    class Row:
        id: int

    assert Scalar.from_py(42).into_field().name == "value"
    assert Scalar.from_py([1, None]).into_array_field().name == "item"
    assert Scalar.from_py([Row(1)]).into_struct_field().name == "row"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.equal(Scalar.fromJs(42).intoField().name, 'value')
    assert.equal(Scalar.fromJs([1, null]).intoArrayField().name, 'item')
    assert.equal(Scalar.fromJs([{ id: 1 }]).intoStructField().name, 'row')
    ```

The inference itself stays in Rust. A local Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23) measured the scalar, array, and one-record Struct paths at
80.1 ns, 240 ns, and 985 ns respectively. These are Criterion point estimates; regenerate them on
the deployment host with:

```console
cargo bench -p yggdryl --bench datatype --all-features -- "value/infer_.*_field"
```

The markers are the same family a [typed field](types.md) uses, so a value and a field spell one datatype the same way. Both are covered in full under [structured text](text.md#typed-scalar-families). Behind the default `arrow` feature the pairing is also the one scalar Arrow projection - `into_arrow_array` materializes one row and `from_arrow_array` decodes one back - documented with the rest of the array boundary in [arrow.md](arrow.md).


## The WKB reader


`types::geospatial` owns the one Well-Known Binary reader: displaying a geometry column as WKT, casting
it to text, and bounding it for Parquet and Iceberg statistics all need the same decoding, and
none of them needs a geometry engine, so the workspace reads WKB with no dependency and adds no
second implementation anywhere else. A WKT *parser* is deliberately absent: the workspace
displays and bounds geometries; it does not accept text geometry input yet.

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

`Geometry::from_slice` decodes one geometry - the seven simple-feature shapes, in either byte
order, with both type-code spellings: the ISO one, where Z, M, and ZM add 1000, 2000, or 3000 to
the base code, and the PostGIS EWKB one, where high bits flag the extra axes and an embedded SRID
that is read past rather than modeled. The whole slice must be one geometry - trailing bytes are
refused - and malformed input errors name their byte position. `POINT EMPTY` has no zero-count
spelling in WKB, so its conventional NaN coordinates read back as the empty point, and emptiness
is a shape (`coordinate: None`) rather than a value to test for.

`wkb::bounding_box` streams coordinates through a min/max fold in one pass - nothing is
materialized per vertex - and an empty geometry yields the fold's identity, which
`BoundingBox::is_empty` names so a statistics writer can skip the box instead of storing it.
`wkb::geometry_type_ids` collects the distinct ISO type codes a payload holds, and `wkb::into_wkt`
spells canonical WKT whose coordinates print the shortest decimal that reads back as the same
double, so the text loses nothing.

```rust
use yggdryl::types::geospatial::wkb;

// Truncated input: the error names the byte position.
let error = wkb::bounding_box(&[1, 1, 0, 0, 0]).unwrap_err();
assert!(error.to_string().contains("byte 5"), "{error}");
```

The value these bytes travel in is `Scalar::Geospatial`: the canonical spelling of one WKB payload
inside the shared [`Scalar`](text.md) model, which
[geometry and geography columns](types.md#variant-geometry-and-geography) read back, which
canonicalization rewrites plain `Scalar::Bytes` into on the way in, and whose `as_wkb` accessor
reads both spellings so an inbound payload is never rejected for arriving as plain bytes. There is
deliberately no `Scalar::Variant` beside it: a variant value *is* the self-describing `Scalar` tree
itself, and its Parquet binary encoding lands with the Iceberg v3 layer. Across Arrow boundaries
the geospatial pair is Binary storage under the community `geoarrow.wkb` extension name, whose
GeoArrow JSON metadata carries the CRS and edge algorithm - GeoArrow's own documentation says the
specification is not finalized, so that mapping is a community choice the workspace may revisit
when it stabilizes.
