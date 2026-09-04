# DataType

`yggdryl::datatype` holds `DataType`, the owned logical type of one value - every Arrow logical
type, with Arrow itself kept out of the value model.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let value = DataType::from_str("decimal(18, 4)")?;
    assert_eq!(value, DataType::decimal128(18, 4)?);

    // Display is canonical and both text forms round-trip.
    assert_eq!(value.to_string(), "decimal128(18,4)");
    assert_eq!(DataType::from_str(&value.to_string())?, value);
    assert_eq!(DataType::from_json(&value.clone().into_json()?)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    value = DataType("decimal(18, 4)")
    assert value == DataType.decimal(18, 4)

    assert str(value) == "decimal128(18,4)"
    assert DataType(str(value)) == value
    assert DataType.from_json(value.into_json()) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const value = DataType.from('decimal(18, 4)')
    assert.equal(value.id, 'decimal128')

    assert.equal(value.toString(), 'decimal128(18,4)')
    assert.ok(DataType.fromString(value.toString()).equals(value))
    assert.ok(DataType.fromJSON(value.toJSON()).equals(value))
    ```

There are 56 variants: one per Arrow logical type, plus Variant, the geospatial pair, the GUID, the
six ASCII widths, and the four registered codes, which cross Arrow as extension-typed storage.
The parser accepts the Arrow,
SQL, Hive, and Spark spellings of all of them - `bigint`, `varchar(255)`, `array<string>`,
`row(...)`, `double precision` - and normalizes to one canonical form, so `to_string` is a
losslessly re-parseable value rather than a debug rendering. `into_json` is a separate, structural
encoding: tagged objects that name every parameter, which is what a schema written to disk should
be.

Scalar variants are inline and nested children sit behind shared allocations, so cloning a
`DataType` never allocates. The value is immutable; changing a type means building another one.

## Children

Subscripting a `DataType` reaches a nested child - by name or by position - which is exactly what
subscripting a [`Field`](field.md#item-access-reaches-a-child-never-metadata) does, so one semantic
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
    from yggdryl import DataType, Field, fields

    quote = DataType.from_fields([
        Field("symbol", "utf8", nullable=False),
        fields.list("levels", fields.float64("item")),
    ])

    assert len(quote) == 2
    assert [field.name for field in quote] == ["symbol", "levels"]
    assert quote[0].name == "symbol"
    assert quote[-1].name == "levels"
    assert len(quote["levels"].dtype) == 1
    assert "levels" in quote and "missing" not in quote

    lookup = fields.map_of("lookup", "utf8", "int64", keys_sorted=True).dtype
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
JavaScript `fields` factories build a [`Field`](field.md) rather than a bare type, which is what
nesting actually needs - a child is a name, a type, and a nullability flag.

## Precision and resolution pick the width

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

`decimal` and `time` are selectors, not extra types: precision 1-38 lands on `decimal128` and 39-76
on `decimal256`, seconds and milliseconds land on `time32` and micro/nanoseconds on `time64`. The
exact constructors stay available for when the physical width is part of the contract. Whichever you
call, the parameters are validated once at construction - a precision of zero, a positive scale
larger than the precision, a nanosecond `time32`, or a negative fixed width never becomes a value.

Units are parsed from the shared [`TimeUnit`](generic.md) vocabulary, so `s`, `sec`, `MILLIS`, `µs`,
and `nano seconds` all work, and the calendar interval layouts - which are not time-of-day
resolutions - are rejected here rather than silently accepted. JavaScript has no
`DataType.decimal`; decimals arrive through the `fields` factories.

## Encodings that wrap a value

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, Field};

    let codes = DataType::dictionary(DataType::Int16, DataType::Utf8)?;
    let runs = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Utf8, true),
    )?;

    assert_eq!(codes.kind(), DataTypeKind::Dictionary);
    assert_eq!(runs.kind(), DataTypeKind::RunEndEncoded);
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

    from yggdryl import fields

    codes = fields.dictionary("codes", "int16", "utf8").dtype
    runs = fields.run_end_encoded(
        "runs",
        fields.int16("run_ends", nullable=False),
        fields.utf8("values"),
    ).dtype

    assert codes.kind == "dictionary"
    assert runs.kind == "run_end_encoded"
    assert not codes.is_nested and not runs.is_nested
    assert str(codes) == "dictionary(int16,utf8)"

    with pytest.raises(ValueError, match="integer key datatype"):
        fields.dictionary("bad", "utf8", "utf8")
    with pytest.raises(ValueError, match="int16, int32, or int64"):
        fields.run_end_encoded(
            "bad", fields.uint32("run_ends", nullable=False), fields.utf8("values")
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

    assert.equal(codes.kind, 'dictionary')
    assert.equal(runs.kind, 'run_end_encoded')
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
[`Field`](field.md), not here, because two fields can share a type and disagree about them.

Run-end children keep Arrow's names and constraints: `run_ends` must be non-null and one of the
three signed widths, while `values` carries the actual type and its own nullability.

## Unions and the dense-union sugar

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

## Variant, geometry, and geography

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, EdgeAlgorithm};

    // Bare `variant` is the self-describing semi-structured datatype; the
    // parenthesis selects the dense-union sugar instead.
    let variant = DataType::variant();
    assert_eq!(variant.to_string(), "variant");
    assert_eq!(variant.kind(), DataTypeKind::Variant);
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

    from yggdryl import DataType, fields

    variant = DataType.variant()
    assert variant.id == "variant"
    assert variant.kind == "variant"
    assert str(variant) == "variant"
    assert DataType("variant") == variant
    assert DataType("variant(n:int64)").id == "union"
    assert fields.variant("payload").dtype == variant

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
    assert fields.geography("region", "OGC:CRS84", "vincenty").dtype == vincenty

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
    assert.equal(variant.kind, 'variant')
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
[`Scalar`](generic.md) model; its Parquet binary encoding lands with the Iceberg v3 layer, so paths
that would need it today refuse by name.

The geospatial pair shares one parameter value: a coordinate reference system, and - only on a
geography - the edge interpolation. Bare `geometry` and `geography` fill the defaults Parquet's
`GEOMETRY`/`GEOGRAPHY` logical types and Iceberg v3 share (`OGC:CRS84`, spherical edges), and
display emits a parameter exactly when it differs from that default, so every spelling round-trips
through `from_str`. An empty CRS is refused - the absent spelling is `None`, which fills the
default - and a geometry given an edge algorithm is refused by name: straight planar lines need
none. The algorithm vocabulary is the five canonical lowercase names of
[`EdgeAlgorithm`](generic.md#shared-vocabulary) - `spherical`, `vincenty`, `thomas`, `andoyer`,
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
Well-Known Binary through [`Scalar::Geospatial`](generic.md#the-wkb-reader), read back for display
and bounds by the one WKB reader documented there.

## ASCII widths and the registered codes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{ArrowCast, DataType, DataTypeKind, Field, Scalar};

    // Six widths named by their bits; the family constructor selects the
    // narrowest one that holds the bytes.
    assert_eq!(DataType::ascii(2)?, DataType::Ascii16);
    assert_eq!(DataType::ascii(3)?, DataType::Ascii24);
    assert_eq!(DataType::ascii(6)?, DataType::Ascii64);
    assert_eq!(DataType::from_str("ascii(12)")?, DataType::Ascii96);
    assert_eq!(DataType::Ascii32.kind(), DataTypeKind::String);
    assert_eq!(DataType::Ascii64.ascii_width(), Some(8));
    assert_eq!(DataType::Utf8.ascii_width(), None);
    assert!(DataType::ascii(17).is_err());

    // A registered code is a datatype, not a name over a width: it stores the
    // width its standard fixes and displays as itself.
    let currency = DataType::currency();
    assert_eq!(DataType::from_str("currency")?, currency);
    assert_eq!(currency.to_string(), "currency");
    assert_eq!(currency.ascii_width(), Some(3));
    assert_ne!(currency, DataType::Ascii24);
    assert_eq!(currency.kind(), DataTypeKind::String);
    assert_eq!(
        DataType::CODES,
        &[
            ("country", DataType::Country, 2),
            ("currency", DataType::Currency, 3),
            ("mic", DataType::Mic, 4),
            // Six bytes: `cfi` stores what it is, not the eight the next
            // ASCII width would pad it to.
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
    let ccy = Field::new("ccy", DataType::Ascii32, false);
    let stored = scalar_array(&ccy, &Scalar::from("USD"))?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), b"USD\0");
    assert_eq!(scalar_value(&ccy, stored.as_ref())?, Scalar::from("USD"));

    // The Arrow field is `fixed_size_binary(4)` under the `yggdryl.ascii` name.
    let arrow = ccy.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ascii");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow(&arrow)?, ccy);

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

    assert_eq!(DataType::Ascii32.merge_with(&DataType::Utf8, true)?, DataType::Utf8);

    let long: ArrayRef = Arc::new(StringArray::from(vec!["EURO!"]));
    let refused = ccy.cast_arrow_array(long, false).unwrap_err().to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, fields

    ascii32 = DataType("ascii32")
    assert DataType.ascii(2) == DataType("ascii16")
    assert DataType.ascii(3) == DataType("ascii24")
    assert DataType.ascii(6) == DataType("ascii64")
    assert DataType("ascii(12)") == DataType("ascii96")
    assert ascii32.kind == "string"
    assert ascii32.ascii_width == 4
    assert DataType("ascii24").ascii_width == 3
    assert DataType("ascii96").ascii_width == 12
    assert DataType("utf8").ascii_width is None
    assert fields.ascii("ccy", 3).dtype == DataType("ascii24")

    # A registered code is a datatype, not a name over a width: it stores the
    # width its standard fixes and displays as itself.
    currency = DataType("currency")
    assert str(currency) == "currency"
    assert currency.ascii_width == 3
    assert currency != DataType("ascii24")
    assert currency.kind == "string"
    assert [(DataType(name).id, DataType(name).ascii_width) for name in
            ("country", "currency", "mic", "cfi")] == [
        ("country", 2), ("currency", 3), ("mic", 4), ("cfi", 6)
    ]

    # A code rides its own Arrow extension, so the identity survives the trip.
    venue = fields.mic("venue", nullable=False)
    venue_arrow = venue.into_arrow()
    assert venue_arrow.type == pa.binary(4)
    assert venue_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
    assert Field.from_arrow(venue_arrow) == venue

    # Storage pads to the width; every string rendering trims the padding.
    ccy = Field("ccy", "ascii32", nullable=False)
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

    # A cast into the width pads; the stored column read under `utf8` trims.
    padded = ccy.cast_arrow_array(pa.array(["USD", "EU"]))
    assert padded.to_pylist() == [b"USD\x00", b"EU\x00\x00"]
    stored = pa.record_batch([padded], schema=pa.schema([arrow]))
    text = DataType.from_fields([fields.utf8("ccy")])
    assert text.cast_arrow_batch(stored).column(0).to_pylist() == ["USD", "EU"]

    assert ascii32.merge_with("utf8") == DataType("utf8")

    with pytest.raises(ValueError, match="at most 4 bytes"):
        ccy.cast_arrow_array(pa.array(["EURO!"]))
    with pytest.raises(ValueError, match="from 1 to 16 bytes, got 17"):
        DataType.ascii(17)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, fields } = require('yggdryl')

    const ascii32 = new DataType('ascii32')
    assert.equal(DataType.ascii(2).id, 'ascii16')
    assert.equal(DataType.ascii(3).id, 'ascii24')
    assert.equal(DataType.ascii(6).id, 'ascii64')
    assert.equal(DataType.from('ascii(12)').id, 'ascii96')
    assert.equal(ascii32.kind, 'string')
    assert.equal(ascii32.asciiWidth, 4)
    assert.equal(new DataType('ascii24').asciiWidth, 3)
    assert.equal(new DataType('ascii96').asciiWidth, 12)
    assert.equal(new DataType('utf8').asciiWidth, null)
    assert.equal(fields.ascii('ccy', 3).dtype.id, 'ascii24')

    // A registered code is a datatype, not a name over a width: it stores the
    // width its standard fixes and displays as itself.
    const currency = new DataType('currency')
    assert.equal(currency.id, 'currency')
    assert.equal(currency.toString(), 'currency')
    assert.equal(currency.asciiWidth, 3)
    assert.ok(!currency.equals(new DataType('ascii24')))
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
    const row = fields.struct('row', [fields.ascii32('ccy', { nullable: false })], {
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

    assert.equal(ascii32.mergeWith('utf8').id, 'utf8')

    assert.throws(() => row.castArrow(codes(['EURO!'])), /ASCII text of at most 4 bytes/)
    assert.throws(() => DataType.ascii(17), /from 1 to 16 bytes, got 17/)
    ```

`ascii16`, `ascii24`, `ascii32`, `ascii64`, `ascii96`, and `ascii128` are ASCII text padded with
trailing NUL to 2, 3, 4, 8, 12, and 16 bytes, named by their bit width like `int32` and
`decimal128`. The six are the widths the codes of the trade actually take: two bytes for a country,
three for a currency, four for a venue, six inside eight for a CFI code, twelve for an ISIN, and
sixteen for whatever is longer. A value is ASCII - every byte at most `0x7F` - of at most the width
in bytes, with no NUL byte; storage pads it to exactly the width, and every string rendering trims
the padding, so a column reads back as the text that went in. The canonical scalar is the trimmed
string; bytes and a string carrying trailing NULs are accepted on the way in under the same rule and
canonicalize to it. `ascii(n)` selects the narrowest width that holds `n` bytes, `ascii_width`
answers the storage width, and a value that is not ASCII, holds a NUL, or is longer than the width
is refused naming the width and, in a cast, the row.

`country`, `currency`, `mic` and `cfi` are datatypes of their own, not names over a width. Each is
one published registry - ISO 3166-1 alpha-2, ISO 4217, ISO 10383, ISO 10962 - with exactly one
storage width: two, three, four and six bytes. `CODES` is that listing, `code_name` answers a code's
identity, and `is_code` distinguishes the four from the widths. The value contract is unchanged: a
code answers `ascii_width`, so `ascii_packed`, `AsciiEnum` and `AsciiDictionary` work over it with
nothing added, and an enum member is still the integer its value packs into. What a code adds over
the width that would hold it is identity and a constant - the identity crosses Arrow, and the width
is known at compile time, so the ingest and render paths are monomorphized per code rather than
reading a length out of the datatype on every row. Six bytes is the point of `cfi`: it stores what a
CFI code is rather than padding into `ascii64`'s eight.

Across Arrow a width is `fixed_size_binary(2|3|4|8|12|16)` under the `yggdryl.ascii` extension name
with an empty document, because the storage width says the width; a code is
`fixed_size_binary(2|3|4|6)` under its own `yggdryl.country`, `yggdryl.currency`, `yggdryl.mic` or
`yggdryl.cfi`, because the name is what says which registry the bytes belong to. Three bytes under
`yggdryl.currency` read back a currency and three bytes under `yggdryl.ascii` read back an
`ascii24`; a plain fixed binary of any width, or one carrying a document, imports as it is. Two
schemas that agree on a code keep it, and a code [merged](field.md#merging-two-schemas) with
anything else answers the plain text both fit in - a currency beside a country is `ascii24`, never
one standard's code carrying the other's values. Every other exchange sees text: Iceberg, Spark, Polars, and
pandas [rewrite](#compatibility-rewriting) a width to `string`/`utf8`, Avro writes `string`, and a
filter such as `ccy = 'USD'` meets the literal at `utf8`. Widths
[merge](field.md#merging-two-schemas) as text. One boundary shows storage: JavaScript's
`readRecords` hands out Arrow JS rows, which carry no extension identity, so an ASCII column
arrives there as its padded bytes; read it under a declared `utf8` field, as above, for the text.

### The dictionary vocabulary and its generated enum

=== "Rust"

    ```rust
    use arrow_array::types::Int32Type;
    use arrow_array::{Array, DictionaryArray, FixedSizeBinaryArray};
    use yggdryl::{AsciiDictionary, AsciiEnum, DataType, Field};

    // Register as you encode: an unseen value takes the next code, a seen one
    // answers the code it already has.
    let mut currencies = AsciiDictionary::new(DataType::Ascii32)?;
    assert_eq!(currencies.push("USD")?, 0);
    assert_eq!(currencies.push("EUR")?, 1);
    assert_eq!(currencies.push("USD")?, 0);
    assert_eq!(currencies.as_values(), ["USD", "EUR"]);
    assert_eq!(currencies.get(1), Some("EUR"));
    assert_eq!(currencies.get_code("USD\0"), Some(0));

    // An encoded column carries the key type over the width.
    assert_eq!(currencies.dtype()?.to_string(), "dictionary(int32,ascii32)");
    assert_eq!(currencies.key(), &DataType::Int32);
    assert_eq!(currencies.values_dtype(), &DataType::Ascii32);

    // The keys are the codes, a `None` is a null key, and the values are the
    // vocabulary in the width's padded storage.
    let column = currencies.into_arrow_array([Some("USD"), None, Some("JPY"), Some("EUR")])?;
    let column = column
        .as_any()
        .downcast_ref::<DictionaryArray<Int32Type>>()
        .unwrap();
    assert_eq!(
        column.keys().iter().collect::<Vec<_>>(),
        [Some(0), None, Some(2), Some(1)]
    );
    let stored = column
        .values()
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(stored.value(2), b"JPY\0");
    assert_eq!(AsciiDictionary::from_arrow_array(column)?, currencies);

    // A prebuilt vocabulary starts from a constant and auto-registers past it,
    // so a code below the constant's length names one value in every process.
    let mut countries = AsciiDictionary::from_logical_name("Country")?;
    assert_eq!(countries.len(), AsciiDictionary::COUNTRIES.len());
    let france = countries.get_code("FR").expect("FR is prebuilt");
    assert_eq!(AsciiDictionary::COUNTRIES[france as usize], "FR");
    // `ZZ` is ISO 3166's user-assigned range, so it registers after the seed.
    assert_eq!(countries.push("ZZ")?, AsciiDictionary::COUNTRIES.len() as i64);

    // A dictionary code is a position. The other integer a value has is its
    // own storage bytes, which is the same integer in every process and
    // orders exactly as the text does.
    assert_eq!(DataType::Ascii32.ascii_packed(b"USD")?, 0x5553_4400);
    assert_eq!(DataType::Ascii32.ascii_packed(b"USD\0")?, 0x5553_4400);
    assert_eq!(DataType::Ascii32.ascii_value(0x5553_4400)?, "USD");

    // That is what an enum member is, at every width - sixteen bytes fill the
    // whole `i128`.
    let venues = AsciiDictionary::from_values(DataType::Ascii32, ["XNAS", "n/a"])?;
    assert_eq!(
        venues.into_members()?,
        [("XNAS".into(), 0x584E_4153), ("N_A".into(), 0x6E2F_6100)]
    );
    let isins = AsciiDictionary::from_values(DataType::Ascii128, ["US0378331005"])?;
    assert_eq!(
        isins.into_members()?,
        [("US0378331005".into(), 0x5553_3033_3738_3333_3130_3035_0000_0000)]
    );

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert_eq!(AsciiDictionary::member_name("n/a").as_str(), "N_A");

    // A field declares the enum its values name, as one metadata document, so
    // the enum crosses Arrow and comes back the enum that was written.
    let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
    let field = Field::new("side", DataType::Ascii32, false).try_with_ascii_enum(&side)?;
    assert_eq!(side.into_members(&DataType::Ascii32)?[0], ("BUY".into(), 0x4200_0000));
    assert_eq!(Field::from_arrow(&field.into_arrow()?)?.ascii_enum()?, Some(side));
    ```

=== "Python"

    ```python
    import enum

    import pyarrow as pa

    from yggdryl import AsciiDictionary, AsciiEnum, DataType, Field

    # Register as you encode: an unseen value takes the next code, a seen one
    # answers the code it already has.
    currencies = AsciiDictionary("ascii32")
    assert currencies.push("USD") == 0
    assert currencies.push("EUR") == 1
    assert currencies.push("USD") == 0
    assert currencies.values == ["USD", "EUR"]
    assert currencies.get(1) == "EUR"
    assert currencies.get_code("USD\x00") == 0

    # An encoded column carries the key type over the width.
    assert currencies.dtype == DataType("dictionary(int32,ascii32)")
    assert currencies.key == DataType("int32")
    assert currencies.values_dtype == DataType("ascii32")

    # The indices are the codes, `None` is a null key, and the dictionary is the
    # vocabulary in the width's padded storage.
    column = currencies.into_arrow_array(["USD", None, "JPY", "EUR"])
    assert column.type == pa.dictionary(pa.int32(), pa.binary(4))
    assert column.indices.to_pylist() == [0, None, 2, 1]
    assert column.dictionary.to_pylist() == [b"USD\x00", b"EUR\x00", b"JPY\x00"]
    assert AsciiDictionary.from_arrow_array(column) == currencies

    # A prebuilt vocabulary starts from a constant and auto-registers past it,
    # so a code below the constant's length names one value in every process.
    seed = AsciiDictionary.prebuilt()["country"]
    countries = AsciiDictionary.from_logical_name("Country")
    assert len(countries) == len(seed)
    assert seed[countries.get_code("FR")] == "FR"
    # `ZZ` is ISO 3166's user-assigned range, so it registers after the seed.
    assert countries.push("ZZ") == len(seed)

    # The enum is the value list and the code is the position.
    Venue = AsciiDictionary.from_values("ascii32", ["XNAS", "n/a"]).into_intenum("Venue")
    assert issubclass(Venue, enum.IntEnum)
    assert [(member.name, member.value) for member in Venue] == [
        ("XNAS", 0x584E4153),
        ("N_A", 0x6E2F6100),
    ]
    assert Venue(0x584E4153).name == "XNAS"

    # Sixteen bytes fill the whole 128-bit integer, which Python holds natively.
    Isin = AsciiDictionary.from_values("ascii128", ["US0378331005"]).into_intenum("Isin")
    assert Isin.US0378331005 == 0x55533033373833333130303500000000

    # The same rule names one value at a time, for a vocabulary declared
    # member by member rather than generated from a whole listing.
    assert AsciiDictionary.member_name("n/a") == "N_A"

    # A field declares the enum its values name, as one metadata document, so
    # the enum crosses Arrow and comes back the enum that was written.
    side = AsciiEnum("Side", {"BUY": "B", "SELL": "S"})
    field = Field("side", "ascii32", nullable=False)
    field.set_ascii_enum(side)
    assert side.into_members("ascii32")[0] == ("BUY", 0x42000000)
    assert Field.from_arrow(field.into_arrow()).ascii_enum == side
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { AsciiDictionary, AsciiEnum, DataType, Field } = require('yggdryl')

    // Register as you encode: an unseen value takes the next code, a seen one
    // answers the code it already has.
    const currencies = new AsciiDictionary('ascii32')
    assert.equal(currencies.push('USD'), 0)
    assert.equal(currencies.push('EUR'), 1)
    assert.equal(currencies.push('USD'), 0)
    assert.deepEqual(currencies.values(), ['USD', 'EUR'])
    assert.equal(currencies.get(1), 'EUR')
    assert.equal(currencies.getCode('USD\0'), 0)

    // An encoded column carries the key type over the width.
    assert.equal(currencies.dtype.toString(), 'dictionary(int32,ascii32)')
    assert.equal(currencies.key.toString(), 'int32')
    assert.equal(currencies.valuesDtype.toString(), 'ascii32')

    // The keys are the codes, a `null` is a null key, and the values are the
    // vocabulary in the width's padded storage.
    const column = currencies.intoArrowArray(['USD', null, 'JPY', 'EUR'])
    assert.deepEqual(Array.from(column.data[0].values), [0, 0, 2, 1])
    assert.equal(column.get(1), null)
    assert.deepEqual(Array.from(column.get(2)), [0x4a, 0x50, 0x59, 0])
    assert.ok(AsciiDictionary.fromArrowArray(column).equals(currencies))

    // A prebuilt vocabulary starts from a constant and auto-registers past it,
    // so a code below the constant's length names one value in every process.
    const seed = AsciiDictionary.prebuilt().country
    const countries = AsciiDictionary.fromLogicalName('Country')
    assert.equal(countries.length, seed.length)
    assert.equal(seed[countries.getCode('FR')], 'FR')
    // `ZZ` is ISO 3166's user-assigned range, so it registers after the seed.
    assert.equal(countries.push('ZZ'), seed.length)

    // The enum is the value list and the code is the position.
    const Venue = AsciiDictionary.fromValues('ascii32', ['XNAS', 'n/a']).intoEnum('Venue')
    assert.deepEqual({ ...Venue }, { XNAS: 0x584e4153n, N_A: 0x6e2f6100n })
    assert.equal(Venue.N_A, new DataType('ascii32').asciiPacked('n/a'))

    // Sixteen bytes fill the whole 128-bit integer, so every code is a bigint.
    const Isin = AsciiDictionary.fromValues('ascii128', ['US0378331005']).intoEnum('Isin')
    assert.equal(Isin.US0378331005, 0x55533033373833333130303500000000n)

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert.equal(AsciiDictionary.memberName('n/a'), 'N_A')

    // A field declares the enum its values name, as one metadata document, so
    // every serialization carries it and it comes back the enum that wrote it.
    const side = new AsciiEnum('Side', { BUY: 'B', SELL: 'S' })
    const field = new Field('side', 'ascii32', false)
    field.setAsciiEnum(side)
    assert.deepEqual(side.intoMembers('ascii32'), { BUY: 0x42000000n, SELL: 0x53000000n })
    assert.ok(Field.fromJSON(field.toJSON()).asciiEnum.equals(side))
    ```

An ASCII value has two integers and they answer different questions.
`AsciiDictionary` is the vocabulary of one column, a value the caller carries
and never a process-global registry, and its code is a **position**: stable
exactly as far as that dictionary travels, agreed between two encodes only
where the same dictionary crossed both, and never registered by the write path
on its own. `ascii_packed` is the value's **own storage bytes** read
big-endian, so it is the same integer in every process, orders exactly as the
text does, is what a stable hash hashes, and fills an `i32`, an `i64`, or a
whole `i128` by width. An ASCII byte never sets the sign bit, so a packed code
is never negative.

The generated enum names its members by the packed code, so a member survives a
process, a file, and another runtime; the name comes once from the core listing
- an ASCII letter kept uppercased, a digit kept, every other byte `_`, a leading
digit prefixed with `_`, and a name that both opens and closes with `_` dropping
its trailing underscores, because that is the shape Python reserves for
`_sunder_` and `__dunder__` names - which refuses any two values that would name
one member, and answers name to code because the vocabulary is already the code
to value direction.

`AsciiEnum` is that naming as a value: an enum's own name and one ASCII value
per member, which a field stores under the reserved `field:enum` key. The width
is the field's datatype and is never copied into the document, so a member's
code is the packed value under that width and one enum is one canonical text
however it was built. Ordinary field metadata is what carries it, so the
declaration reaches Arrow, a file, and either binding with the field, and the
`field:` protocol view reads it beside `field:init` and `field:partition`.

## The GUID

=== "Rust"

    ```rust
    use arrow_array::{Array, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, DataTypeKind, Field, Scalar};

    // One 128-bit identifier and no parameters. `uuid` is what every other
    // system calls it, so both spellings parse to the one type.
    let guid = DataType::guid();
    assert_eq!(DataType::from_str("guid")?, guid);
    assert_eq!(DataType::from_str("uuid")?, guid);
    assert_eq!(guid.to_string(), "guid");
    assert_eq!(guid.kind(), DataTypeKind::Guid);

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let packed = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab_u128;
    assert_eq!(guid.guid_packed(text.as_bytes())?, packed);
    assert_eq!(guid.guid_packed(text.to_uppercase().as_bytes())?, packed);
    assert_eq!(guid.guid_packed(text.replace('-', "").as_bytes())?, packed);
    assert_eq!(guid.guid_packed(&packed.to_be_bytes())?, packed);
    assert_eq!(guid.guid_value(packed)?, text);

    // Storage is the canonical `arrow.uuid` extension over sixteen bytes, and
    // the value reads back spelled out.
    let id = Field::new("id", DataType::Guid, false);
    let stored = scalar_array(&id, &Scalar::from(text))?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), packed.to_be_bytes());
    assert_eq!(scalar_value(&id, stored.as_ref())?, Scalar::from(text));

    let arrow = id.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(16));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "arrow.uuid");
    assert_eq!(Field::from_arrow(&arrow)?, id);

    assert!(guid.guid_packed(b"not-a-guid").is_err());
    ```

=== "Python"

    ```python
    import uuid

    import pyarrow as pa

    from yggdryl import DataType, Field

    # One 128-bit identifier and no parameters. `uuid` is what every other
    # system calls it, so both spellings parse to the one type.
    guid = DataType("guid")
    assert DataType("uuid") == guid
    assert str(guid) == "guid"
    assert guid.kind == "guid"

    # The identity is the sixteen bytes; every spelling is a rendering of them.
    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    id = Field("id", guid, nullable=False)
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

    // One 128-bit identifier and no parameters. `uuid` is what every other
    // system calls it, so both spellings parse to the one type.
    const guid = new DataType('guid')
    assert.ok(DataType.from('uuid').equals(guid))
    assert.equal(guid.toString(), 'guid')
    assert.equal(guid.kind, 'guid')

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    const id = new Field('id', guid, false)
    assert.equal(id.defaultJSValue(), '00000000-0000-0000-0000-000000000000')
    assert.equal(fields.guid('id').dtype.id, 'guid')
    assert.ok(Field.fromJSON(id.toJSON()).equals(id))
    ```

A GUID is the ASCII widths' sibling: one fixed-width value whose integer is its own storage bytes
read big-endian, so it is the same integer in every process and is what a stable hash hashes. It is
a `u128` rather than an `i128` because every one of the sixteen bytes carries identity. Storage is
`FixedSizeBinary(16)` under the canonical `arrow.uuid` extension - the name Arrow itself registers,
taken as-is rather than re-spelled - and Iceberg's `uuid` maps straight onto it, so the spelling
survives a metadata rewrite in the datatype instead of a marker beside the column.

Where an ASCII width canonicalizes toward its text, because the value *is* text and the padding is
layout, a GUID canonicalizes toward its 36-character lowercase hyphenated spelling, because that is
what a reader means by an identifier. The 32-digit bare-hex text, upper case, and the sixteen stored
bytes are all accepted on the way in and rewrite to that one spelling; anything else is refused by
the one rule that field validation, Arrow ingest, and every cast tier all call.

## Logical names

=== "Rust"

    ```rust
    use yggdryl::{AsciiDictionary, DataType, TimeUnit, Timezone};

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
        Some(DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)))
    );

    // Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    assert_eq!(DataType::from_str("utc_date_only")?, DataType::Date32);
    assert_eq!(DataType::LOGICAL_NAMES[0], ("currency", DataType::Currency));

    // Three of the names also prebuild the vocabulary their codes come from.
    assert_eq!(AsciiDictionary::prebuilt_values("MIC"), AsciiDictionary::MICS);
    assert!(AsciiDictionary::prebuilt_values("tenor").is_empty());

    // The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert_eq!(DataType::from_str("int")?, DataType::Int32);
    assert_eq!(DataType::from_str("float")?, DataType::Float32);
    ```

=== "Python"

    ```python
    from yggdryl import AsciiDictionary, DataType

    # A name is one more spelling of a datatype, so it displays as that datatype.
    price = DataType.from_logical_name("Price")
    assert price == DataType("decimal64(18,8)")
    assert str(price) == "decimal64(18,8)"

    # The same lookup backs the grammar, so a FIX declaration types a row.
    row = DataType("struct<ccy: Currency, venue: Exchange, px: Price, at: UTCTimestamp>")
    assert row["venue"].dtype == DataType("mic")
    assert row["at"].dtype == DataType('timestamp(ns,"UTC")')

    # Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    assert DataType("utc_date_only") == DataType("date32")
    assert DataType.logical_names()["currency"] == DataType("currency")

    # Three of the names also prebuild the vocabulary their codes come from.
    assert AsciiDictionary.prebuilt()["mic"] == AsciiDictionary.prebuilt()["exchange"]
    assert "tenor" not in AsciiDictionary.prebuilt()

    # The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert DataType("int") == DataType("int32")
    assert DataType("float") == DataType("float32")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { AsciiDictionary, DataType } = require('yggdryl')

    // A name is one more spelling of a datatype, so it displays as that datatype.
    const price = DataType.fromLogicalName('Price')
    assert.ok(price.equals(DataType.from('decimal64(18,8)')))
    assert.equal(price.toString(), 'decimal64(18,8)')

    // The same lookup backs the grammar, so a FIX declaration types a row.
    const row = DataType.from('struct<ccy: Currency, venue: Exchange, px: Price, at: UTCTimestamp>')
    assert.equal(row.getField('venue').dtype.id, 'mic')
    assert.equal(row.getField('at').dtype.toString(), 'timestamp(ns,"UTC")')

    // Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    assert.equal(DataType.from('utc_date_only').id, 'date32')
    assert.equal(DataType.logicalNames().currency.id, 'currency')

    // Three of the names also prebuild the vocabulary their codes come from.
    assert.deepEqual(AsciiDictionary.prebuilt().mic, AsciiDictionary.prebuilt().exchange)
    assert.equal(AsciiDictionary.prebuilt().tenor, undefined)

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
declaration types a column directly, plus `mic` - ISO 10383's own name for what FIX calls
`Exchange`.

| FIX | base | resolves to | why |
| --- | --- | --- | --- |
| `Currency` | String | `ascii32` | ISO 4217 alpha-3, stored `USD\0` |
| `Country` | String | `ascii32` | ISO 3166-1 alpha-2, stored `FR\0\0` |
| `Exchange`, `mic` | String | `ascii32` | ISO 10383 MIC, exactly 4 bytes |
| `Language` | String | `ascii32` | ISO 639-1 alpha-2 |
| `MonthYear` | String | `ascii64` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
| `Tenor` | Pattern | `ascii64` | `D5`, `W2`, `M3`, `Y1` |
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
| `UTCTimestamp` | String | `timestamp(ns,"UTC")` | the instant, at the finest FIX width |
| `TZTimestamp` | String | `timestamp(ns,"UTC")` | the offset resolves into the instant |
| `UTCTimeOnly` | String | `time64(ns)` | a time of day with a fraction |
| `LocalMktTime` | String | `time32(s)` | `HH:MM:SS`, no fraction |
| `UTCDateOnly` | String | `date32` | a calendar day |
| `LocalMktDate` | String | `date32` | a calendar day |
| `TZTimeOnly` | String | `ascii128` | a time of day plus an offset has no Arrow type |
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
zone for every row; read it under `timestamp(ns,"<zone>")` when the local reading is the value.

`currency`, `country`, and `mic` also name a [prebuilt vocabulary](#the-dictionary-vocabulary-and-its-generated-enum):
`AsciiDictionary::from_logical_name` seeds the codes those standards assign, in sorted order, and
auto-registers past them. A code below the constant's length therefore names one value in every
process reading this version, which a vocabulary that only auto-registers cannot promise. The MICs
are the common venues rather than the whole ISO 10383 registry, which is thousands of segment
codes.

## Identity and family

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    let stamp = DataType::from_str("timestamp(ns, Europe/Paris)")?;
    assert_eq!(stamp.id(), DataTypeId::Timestamp);
    assert_eq!(stamp.kind(), DataTypeKind::Temporal);
    assert_eq!(stamp.name(), "timestamp");

    // The id drops parameters, so two resolutions share one identity ...
    assert_eq!(DataType::from_str("timestamp(s)")?.id(), stamp.id());
    // ... while the values themselves stay distinct.
    assert_ne!(DataType::from_str("timestamp(s)")?, stamp);

    assert_eq!(DataType::decimal(38, 4)?.id(), DataTypeId::Decimal128);
    assert_eq!(DataType::decimal(38, 4)?.kind(), DataTypeKind::Decimal);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    stamp = DataType("timestamp(ns, Europe/Paris)")
    assert stamp.id == "timestamp"
    assert stamp.kind == "temporal"

    assert DataType("timestamp(s)").id == stamp.id
    assert DataType("timestamp(s)") != stamp

    assert DataType.decimal(38, 4).id == "decimal128"
    assert DataType.decimal(38, 4).kind == "decimal"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const stamp = DataType.from('timestamp(ns, Europe/Paris)')
    assert.equal(stamp.id, 'timestamp')
    assert.equal(stamp.kind, 'temporal')

    assert.equal(DataType.from('timestamp(s)').id, stamp.id)
    assert.equal(DataType.from('timestamp(s)').equals(stamp), false)

    assert.equal(fields.decimal('amount', 38, 4).dtype.id, 'decimal128')
    assert.equal(fields.decimal('amount', 38, 4).dtype.kind, 'decimal')
    ```

`id` names the variant and `kind` names the family it belongs to - 56 ids across 17 kinds. Both are
parameter-free, so they compare and hash without touching nested state, which is what makes them the
cheap way to branch. Dispatch on the kind when the behavior is uniform across a family, on the id
when it is not; `name()` is the Rust spelling of `id().as_str()`. The two vocabularies and the
predicates over them are documented in [shared enums](generic.md). Python and JavaScript have no
separate class for either: both arrive as the canonical lowercase strings.

## Arrow projection

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

## Default values

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
        &[Scalar::I64(0), Scalar::Null]
    );
    assert!(value.is_default_value(&value.default_value()?)?);
    assert_eq!(DataType::Utf8.default_value()?, Scalar::String("".into()));

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
[`Scalar`](generic.md); Python yields a generated field dataclass or a Python scalar from `default_pyvalue`
and a `pyarrow.Scalar` from `default_arrow_scalar`; JavaScript yields a plain array, `Buffer`,
`Map`, or `{ typeId, value }`. Mutable containers are freshly allocated on every call, so a default
is never shared state.

The walk is bounded in both depth and bytes. A fixed-size binary or fixed-size list whose default
would exceed the byte safety limit, and a nesting deeper than the parser limit, both fail loudly
instead of degrading to a null. `is_default_value` answers the reverse question - is this value the
canonical default - without allocating the default first.

A bare `DataType` has no nullability, so its default is the non-null one. Ask a
[`Field`](field.md) instead when the answer depends on whether the slot may be null.

## Serializing a schema

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
    use yggdryl::generic::Scalar;

    let dtype = DataType::decimal128(9, 2)?;

    // One structural model, three formats over it.
    assert_eq!(DataType::from_value(dtype.clone().into_value())?, dtype);
    assert_eq!(DataType::from_json(&dtype.clone().into_json()?)?, dtype);
    assert_eq!(DataType::from_yaml(&dtype.clone().into_yaml()?)?, dtype);
    assert_eq!(DataType::from_toml(&dtype.clone().into_toml()?)?, dtype);

    let shape = dtype.into_value();
    assert_eq!(shape.get_key_str("type").and_then(Scalar::as_utf8), Some("decimal128"));
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType.decimal(9, 2)

    assert DataType.from_dict(dtype.into_dict()) == dtype
    assert DataType.from_json(dtype.into_json()) == dtype
    assert DataType.from_yaml(dtype.into_yaml()) == dtype
    assert DataType.from_toml(dtype.into_toml()) == dtype

    assert dtype.into_dict()["type"] == "decimal128"
    ```

=== "JavaScript"

    !!! note "Rust first"
        The YAML and TOML pair lands in the JavaScript binding once the core surface settles;
        `toJSON` is already there.

## A readable rendering

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


## Compatibility rewriting

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme, TimeUnit};

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
        DataType::Timestamp(TimeUnit::Nanosecond, None),
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
        DataType("timestamp(ns)").into_scheme_compat("spark")
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

    assert.throws(() => DataType.from('timestamp(ns)').intoSchemeCompat('spark'), /got ns/)
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
so, naming key/value structs as the alternative. [Iceberg](iceberg.md) is a closed primitive
vocabulary rather than an engine, so every narrow or unsigned integer widens to the signed one that
holds it - `uint8`, `uint16`, `int8`, and `int16` all become `int32` - it keeps `fixed[n]` and both
its microsecond and nanosecond timestamps, and it has no elapsed-time or calendar-interval type at
all.

Anything that would change what a value means is an error rather than a rewrite. A nanosecond
timestamp is not silently truncated to Spark's microseconds, a negative decimal scale is not
clamped, and a node carrying Arrow extension metadata is not physically relabeled - the message
names the offending node with a path like `$["a.b"]` or `$[].item`. Applied to a
[`Field`](field.md), the same call keeps the name, nullability, and metadata, and rebuilds the Arrow
projection cache only when something actually changed.

## Building the enum directly

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
