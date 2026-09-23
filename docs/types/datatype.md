# DataType

The owned logical type of one value: immutable, and cloning never allocates.

## Contract

| | |
| --- | --- |
| Owns | 48 variants: every Arrow logical type plus Variant, geospatial, UUID, Version, the URI family, the [string and byte families](text/index.md), the twelve [codes](codes/index.md) |
| Parses | Arrow, SQL, Hive, Spark, FIX spellings; `to_string` re-parses losslessly, including `figi` as ANSI X9.145's checked identifier |
| Identity | `id()`, `kind()`: 84 ids, 12 kinds, parameter-free; a string's id is its leaf, a byte column's its leaf |
| Serializes | one structural model under JSON, YAML, TOML |
| Defaults | one non-null default per variant, freshly allocated |
| Limits | recursion 64; a default above 64 MiB errors |
| Compatibility | `arrow`, `spark`, `polars`, `pandas`, `iceberg`; layout rewrites only |
| Rust only | the enum itself |
| JavaScript | the model as JSON only: no YAML, TOML or `pretty` |
| Serializes strings, bytes | one `string` tag and one `binary` tag with `layout` naming the leaf and `fixed` or `max` beside it ([String](text/string.md#serialized-shape), [Bytes](text/bytes.md#serialized-shape)) |

## Use

Parse any spelling, display the canonical one, round-trip both text forms.

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

## Logical names

A FIX name resolves to, and displays as, an ordinary datatype.

=== "Rust"

    ```rust
    use yggdryl::{DataType, StringEnum, TimeUnit, Timezone};

    // A name is one more spelling of a datatype, so it displays as that datatype.
    let price = DataType::from_logical_name("Price")?;
    assert_eq!(price, DataType::Float64);
    assert_eq!(price.to_string(), "float64");

    // The same lookup backs the grammar, so a FIX declaration types a row.
    let row = DataType::from_str(
        "struct<ccy: Currency, venue: Exchange, px: Price, qty: Qty, at: UTCTimestamp>",
    )?;
    assert_eq!(
        row.get_field_by_path("venue").map(|field| field.dtype().clone()),
        Some(DataType::MicCode)
    );
    assert_eq!(
        row.get_field_by_path("at").map(|field| field.dtype().clone()),
        Some(DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?)
    );

    // Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    // A FIX date is that day's midnight, so it resolves to an instant rather
    // than to a day a consumer would have to cast before comparing it.
    assert_eq!(
        DataType::from_str("utc_date_only")?,
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?,
    );
    assert_eq!(DataType::LOGICAL_NAMES[0], ("currency", DataType::Currency));

    // Three of the names also prebuild the vocabulary their codes come from.
    assert_eq!(StringEnum::prebuilt_values("MIC"), StringEnum::MICS);
    assert!(StringEnum::prebuilt_values("tenor").is_empty());
    // A name that is a width rather than a code resolves to the fixed string.
    assert_eq!(DataType::from_logical_name("Language")?, DataType::fixed_ascii(2)?);

    // The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert_eq!(DataType::from_str("int")?, DataType::Int32);
    assert_eq!(DataType::from_str("float")?, DataType::Float32);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, StringEnum

    # A name is one more spelling of a datatype, so it displays as that datatype.
    price = DataType.from_logical_name("Price")
    assert price == DataType("float64")
    assert str(price) == "float64"

    # The same lookup backs the grammar, so a FIX declaration types a row.
    row = DataType("struct<ccy: Currency, venue: Exchange, px: Price, at: UTCTimestamp>")
    assert row["venue"].dtype == DataType("mic")
    assert row["at"].dtype == DataType('datetime64(ns,"UTC")')

    # Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    # A FIX date is that day's midnight, so it resolves to an instant.
    assert DataType("utc_date_only") == DataType("datetime64(ns, UTC)")
    assert DataType.logical_names()["currency"] == DataType("currency")

    # Three of the names also prebuild the vocabulary their codes come from.
    assert StringEnum.prebuilt()["mic"] == StringEnum.prebuilt()["exchange"]
    assert "tenor" not in StringEnum.prebuilt()
    # A name that is a width rather than a code resolves to the fixed string.
    assert DataType.from_logical_name("Language") == DataType.fixed_ascii(2)

    # The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert DataType("int") == DataType("int32")
    assert DataType("float") == DataType("float32")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, StringEnum } = require('yggdryl')

    // A name is one more spelling of a datatype, so it displays as that datatype.
    const price = DataType.fromLogicalName('Price')
    assert.ok(price.equals(DataType.from('float64')))
    assert.equal(price.toString(), 'float64')

    // The same lookup backs the grammar, so a FIX declaration types a row.
    const row = DataType.from('struct<ccy: Currency, venue: Exchange, px: Price, at: UTCTimestamp>')
    assert.equal(row.getField('venue').dtype.id, 'mic')
    assert.equal(row.getField('at').dtype.toString(), 'datetime64(ns,"UTC")')

    // Case, `_`, `-`, and spaces fold, exactly as elsewhere in the grammar.
    // A FIX date is that day's midnight, so it resolves to an instant.
    assert.equal(DataType.from('utc_date_only').id, 'datetime64')
    assert.equal(DataType.logicalNames().currency.id, 'currency')

    // Three of the names also prebuild the vocabulary their codes come from.
    assert.deepEqual(StringEnum.prebuilt().mic, StringEnum.prebuilt().exchange)
    assert.equal(StringEnum.prebuilt().tenor, undefined)
    // A name that is a width rather than a code resolves to the fixed string.
    assert.ok(DataType.fromLogicalName('Language').equals(DataType.fixedAscii(2)))

    // The five base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert.equal(DataType.from('int').id, 'int32')
    assert.equal(DataType.from('float').id, 'float32')
    ```

The registry is the FIX Latest table plus `mic`, `cfi`, `isin`, `cusip` and `sedol`; `currency`, `country`, `mic` also name a [prebuilt vocabulary](codes/index.md).

| FIX | base | resolves to | why |
| --- | --- | --- | --- |
| `Currency` | String | `currency` | ISO 4217 alpha-3, at most 3 bytes |
| `Country` | String | `country` | ISO 3166-1 alpha-2, at most 2 bytes |
| `Exchange`, `mic` | String | `mic` | ISO 10383 MIC, at most 4 bytes |
| `cfi` | - | `cfi` | ISO 10962, at most 6 bytes |
| `isin` | - | `isin` | ISO 6166, twelve bytes closed by a check digit |
| `cusip` | - | `cusip` | CUSIP, nine bytes closed by a check digit |
| `sedol` | - | `sedol` | SEDOL, seven bytes closed by a check digit |
| `Language` | String | `fixed_ascii(2)` | ISO 639-1 alpha-2 |
| `MonthYear` | String | `fixed_ascii(8)` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
| `Tenor` | Pattern | `fixed_ascii(8)` | `D5`, `W2`, `M3`, `Y1` |
| `Pattern` | - | `utf8` | the abstract base of `Tenor` and the reserved ranges |
| `Length` | int | `int32` | a byte count |
| `TagNum` | int | `int32` | a FIX tag |
| `SeqNum` | int | `int64` | a session sequence number outgrows `int32` |
| `NumInGroup` | int | `int32` | a repeating-group counter |
| `DayOfMonth` | int | `int8` | 1 through 31 |
| `Reserved100Plus` | Pattern | `int32` | a user-defined enumeration value |
| `Reserved1000Plus` | Pattern | `int32` | as above |
| `Reserved4000Plus` | Pattern | `int32` | as above |
| `Qty` | float | `float64` | the specification states no scale |
| `Price` | float | `float64` | as above |
| `PriceOffset` | float | `float64` | as above, signed |
| `Percentage` | float | `float64` | `0.0525` is 5.25% |
| `Amt` | float | `float64` | one width, so the family is arithmetic |
| `UTCTimestamp` | String | `datetime64(ns,"UTC")` | the instant, at the finest FIX width |
| `TZTimestamp` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant |
| `UTCTimeOnly` | String | `time64(ns)` | a time of day with a fraction |
| `LocalMktTime` | String | `time64(ns)` | a time of day, one type with `UTCTimeOnly` |
| `UTCDateOnly`, `utcdate` | String | `datetime64(ns, UTC)` | that day at midnight, in UTC |
| `LocalMktDate` | String | `datetime64(ns)` | that day at midnight, stating no zone |
| `LocalMktDatetime` | String | `datetime64(ns)` | a local instant, stating no zone |
| `TZTimeOnly` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant, on the epoch day |
| `MultipleCharValue` | char | `utf8` | space-delimited members |
| `MultipleStringValue` | String | `utf8` | space-delimited members |
| `XID` | String | `utf8` | an XML identifier |
| `XIDREF` | String | `utf8` | a reference to one |
| `data` | - | `binary` | opaque bytes |
| `XMLData` | data | `binary` | an XML document, opaque here |

## Identity and family

`id` names the variant, `kind` its family; both drop parameters and touch no nested state.

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

Both vocabularies live on [Scalar](scalar.md); the bindings see lowercase strings. `DataTypeId::as_u8` is the identifier as one byte, laid out by family - `DataTypeKind::id` is the family's own number, the start of the range its leaves take - and `DataTypeId::from_u8` and `DataTypeKind::of_u8` read a byte back; the [value stream](value-stream.md) and the [digest feed](../hashing.md#encoding) write that byte.

## Arrow projection

Rust and Python exchange a real Arrow type; Node reads Arrow JS through `toString`.
Python crosses through the Arrow C Data Interface rather than rebuilding the value.

An extension identity is field metadata, so `into_arrow` answers the storage a type is written
over and [the field](field.md) is what carries the name back. A C schema is a field node, so
`into_arrow_ffi` keeps the identity - under a dictionary encoding too, where the entries belong
to the outer node and Arrow's values are a bare datatype.

=== "Rust"

    ```rust
    use yggdryl::{DataType, TimeUnit};

    let value = DataType::from_str("map<string,array<decimal(38,18)>>")?;
    let arrow = value.clone().into_arrow_datatype()?;

    assert_eq!(DataType::from_arrow_datatype(&arrow)?, value);
    assert_eq!(value.clone().into_arrow_datatype()?, arrow);
    assert_eq!(DataType::try_from(arrow)?, value);

    // Projection re-checks parameters, so a directly built leaf cannot escape.
    let broken = DataType::Time32(TimeUnit::Nanosecond);
    assert!(broken.clone().into_arrow_datatype().is_err());
    assert!(broken.into_arrow_datatype_ffi().is_err());
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

Every conversion re-checks parameters; whole schemas cross through [Schema](../arrow/schema.md).

## Default values

The core computes one default; each binding projects it.

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let value = DataType::from(StructType::from_fields([
        Field::new("id", DataType::Int32, false),
        Field::new("note", DataType::utf8(), true),
    ])?);

    // One positional slot per child, each honoring its own nullability.
    assert_eq!(
        value.default_value()?.as_sequence().unwrap(),
        &[Scalar::from(0_i64), Scalar::Null]
    );
    assert!(value.is_default_value(&value.default_value()?)?);
    assert_eq!(DataType::utf8().default_value()?, Scalar::from(""));

    // A column of defaults crosses into Arrow like any other column.
    let defaults = Serie::from_default(value.clone().required_field("value"), 2)?;
    assert_eq!(defaults.require_arrow_array()?.len(), 2);

    // A default is bounded: a layout too large to materialize is an error, not a null.
    assert!(DataType::fixed_binary(64 * 1024 * 1024 + 1)?.default_value().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, Serie

    value = DataType.from_fields([
        Field("id", "int32", nullable=False),
        Field("note", "utf8", nullable=True),
    ])
    # A default is a value, so it answers as one `Scalar`; `as_py` is the one
    # conversion, and a struct reads as its ordered children.
    row = value.default_scalar()

    assert row.as_py() == [0, None]
    assert DataType("utf8").default_scalar().as_py() == ""
    assert DataType("int64").default_pyhint() is int

    # A column of defaults crosses into Arrow like any other column.
    defaults = Serie.from_default(Field("value", value, nullable=False))
    assert defaults.into_arrow_scalar().as_py() == {"id": 0, "note": None}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, Serie, fields } = require('yggdryl')

    const value = DataType.fromFields([
      fields.int32('id', { nullable: false }),
      fields.utf8('note', { nullable: true }),
    ])

    // A nullable field defaults to null; a required one defaults to its zero.
    assert.deepEqual(value.defaultJSValue(), [0, null])
    assert.equal(new DataType('utf8').defaultJSValue(), '')
    assert.equal(new DataType('int64').defaultJSHint().constructor, BigInt)

    // A column of defaults crosses into Arrow like any other column.
    assert.equal(Serie.fromDefault(new Field('value', 'int32', false)).intoArrowScalar(), 0)
    ```

`is_default_value` checks a candidate without building the default; nullability is a [Field](field.md) question.

## Serializing a schema

One structural model, three writers; a schema embeds inline in configuration.
Nesting is carried, not flattened, so every format round-trips it.

=== "Rust"

    ```rust
    use yggdryl::DataType;
    use yggdryl::Scalar;

    let dtype = DataType::decimal(9, 2)?;

    // One structural model, three formats over it.
    assert_eq!(DataType::from_value(dtype.clone().into_value())?, dtype);
    assert_eq!(DataType::from_json(&dtype.clone().into_json()?)?, dtype);
    assert_eq!(DataType::from_yaml(&dtype.clone().into_yaml()?)?, dtype);
    assert_eq!(DataType::from_toml(&dtype.clone().into_toml()?)?, dtype);

    let shape = dtype.into_value();
    assert_eq!(shape.get_key_str("type").and_then(Scalar::as_str), Some("decimal32"));
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

    !!! note "Rust and Python only"
        JavaScript has no YAML or TOML writer; it reads and writes the same model as JSON
        through `toJSON`, `toJSONBytes`, `DataType.fromJSON`, and `DataType.fromJSONBytes`,
        as [Use](#use) shows.

| call | form |
| --- | --- |
| `into_json`, `into_yaml`, `into_toml` | text; shared [Formatting](../media/index.md#json), `indent=` in Python |
| `into_json_bytes`, `toJSONBytes` | the same JSON, encoded |
| `from_json` | bytes, text, or a parsed object |

## A readable rendering

Compact still round-trips; `{:#}` and `pretty()` render one fact per line, one indent per level.

=== "Rust"

    ```rust
    use yggdryl::{DataType, StructType};

    let rows = DataType::list(
        DataType::from(StructType::from_fields([DataType::utf8().nullable_field("venue")])?).nullable_field("item"),
    );

    // Compact still round-trips.
    assert_eq!(DataType::from_str(&rows.to_string())?, rows);

    // Readable is the alternate, or the named adapter - one implementation.
    assert_eq!(format!("{rows:#}"), rows.into_pretty_str().to_string());
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

    !!! note "Rust and Python only"
        JavaScript has no `pretty`; `toString` is the compact form that round-trips.

## Compatibility rewriting

`into_scheme_compat` applies only the layout rewrites one target needs.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme, StructType, TimeUnit, Timezone};

    let source = DataType::from(StructType::from_fields([
        Field::new("small", DataType::UInt8, false),
        Field::new("wide", DataType::UInt64, true),
        Field::new(
            "text",
            DataType::large_list(DataType::utf8_view().nullable_field("item")),
            false,
        ),
    ])?);

    let spark = source.clone().into_scheme_compat(&Scheme::SPARK)?;
    let rewritten = spark.as_fields().unwrap();
    assert_eq!(rewritten[0].dtype(), &DataType::Int16);
    assert_eq!(rewritten[1].dtype(), &DataType::decimal128(20, 0)?);
    assert_eq!(
        rewritten[2].dtype(),
        &DataType::list(DataType::utf8().nullable_field("item"))
    );

    // Arrow is a validated clone; Polars keeps the unsigned integers Spark has to widen.
    assert_eq!(source.clone().into_scheme_compat(&Scheme::ARROW)?, source);
    assert_eq!(DataType::UInt32.into_scheme_compat(&Scheme::POLARS)?, DataType::UInt32);

    // A rewrite that would reinterpret values is refused, and the path is named.
    let error = DataType::from(StructType::from_fields([Field::new(
        "created",
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE)?,
        false,
    )])?)
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

| target | rewrite |
| --- | --- |
| `arrow` | validated clone |
| `spark` | `uint8` -> `int16`, `uint64` -> `decimal128(20,0)`, fixed-size list -> list |
| `polars`, `pandas` | no map, and the error names key/value structs; Polars keeps unsigned and fixed-size list |
| `iceberg` | `int8`, `int16`, `uint8`, `uint16` -> `int32`; keeps `fixed[n]`, us/ns timestamps; no duration or interval |

On a [Field](field.md) the call keeps name, nullability, and metadata, and rebuilds the Arrow projection cache only when something changed.
[Iceberg](../media/index.md#iceberg) is a closed primitive vocabulary, not an engine.

## Building the enum directly

Building the enum by hand is Rust only; `validate` is in Python too. It catches states the public enum admits but no constructor produces.

```rust
use yggdryl::{DataType, Field, TimeUnit};

let broken = DataType::Time32(TimeUnit::Nanosecond);
assert!(broken.validate().is_err());
assert!(DataType::time32(TimeUnit::Nanosecond).is_err());

// A valid value validates without allocating, recursing through every child.
let value = DataType::list(Field::new(
    "item",
    DataType::decimal128(18, 4)?,
    true,
));
value.validate()?;
assert!(DataType::decimal128(0, 0).is_err());

assert_eq!(DataType::PARSE_RECURSION_LIMIT, 64);
```

## Edges

- `Time32(Nanosecond)` built directly -> `validate`, `into_arrow`, `into_arrow_ffi` fail; `DataType::time32` refuses.
- `fixed_size_binary(64 * 1024 * 1024 + 1).default_value()` -> error, not null; a fixed-size list default over that byte limit fails the same way.
- nesting past 64 -> error, in parsing, default construction, and compatibility walks alike.
- `into_scheme_compat("duckdb")` -> refused by name, listing the accepted targets.
- `datetime64(ns)` to `spark` -> refused with `got ns` and the node path; scale never clamped, extension metadata never relabeled.
- `DataType.fromArrow({})` -> `own textual representation` error, never `[object Object]`.
- `int`, `float`, `char`, `String`, `Boolean` -> grammar meanings (`int32`, `float32`, `utf8`, `boolean`), not FIX.
- `TZTimestamp` -> the instant, offset dropped; read under `datetime64(ns,"<zone>")` for the local value.
- `TZTimeOnly` -> the same instant under the date it does not state: the epoch day supplies one, so `07:39+05:30` is `1970-01-01T02:09:00Z` and `00:30+05:30` is the evening of 1969-12-31. The date is not data and a reading is not confined to one day, so a day filter is the wrong tool on the column; two readings still subtract.
- A `TZTimeOnly` stating no offset -> null, not a guess. FIX means local time by omitting one and an instant cannot hold that; the text stays in the message's own entries. It is also what keeps a dateless `UTCTimestamp` - a malformed one - from reading as an instant on the epoch day.
- A FIX temporal the ISO reading refuses -> null, and the raw text stays in the message's own entries. A leap second (`23:59:60Z`, which FIX permits) is such a value: it was text under `fixed_ascii(16)` and is null now, which is the cost of being typed. The converse holds too: a wire spelling is read by this crate's [shared ISO reader](../media/index.md#json), not a second parser of FIX's own, so `20240102-10:15:30,000` reads the instant its dotted twin reads where it was null before - FIX gains no spelling, the reader simply has one more. A bare `20240102` goes the same way now that a date is a reading of a datetime: a `LocalMktDate` is that day's midnight straight from the wire text, and the FIX layer states only what FIX leaves out, which for a `UTCDateOnly` column is the `Z` its name already says.
- `into_arrow`, `into_arrow_ffi` consume the source -> clone first.
- `DataType::from_arrow(currency.into_arrow())` -> `utf8`: an Arrow datatype has no metadata to name an extension with. `Field`, a schema, an IPC stream, and `into_arrow_ffi` all keep it, `dictionary(int32, <extension>)` included.
- a logical name folds -> trimmed, ASCII case-insensitive, `_`, `-`, and spaces ignored.
- prebuilt `currency`, `country`, `mic` -> codes in sorted order, so every process on this version answers the same integers.
- prebuilt `mic` -> the common venues, not the whole ISO 10383 registry.
- a JavaScript default -> a plain array, `Buffer`, `Map`, or `{ typeId, value }`.
- JSON emit order -> `name`, `dtype`, `nullable`, `dictionary_id` when non-zero, `dictionary_is_ordered` when set, then `metadata`.
- unset optional attributes -> omitted by every format and by `pretty`, never null.
- `pretty()` -> stable across runs; nothing in it iterates a hash map.
- a value from Python or JavaScript -> already validated at the entry point, so `validate` only ever re-checks; JavaScript does not bind it.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- budget compatibility datatype::arrow datatype_id datatype_kind::names default::datatypes default::scalars parser::aliases parser::grammar serde::datatypes string::listings vocabulary::logical vocabulary::rows
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/(scalar_sql|nested_sql_hive|near_limit_nested|logical_)'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^datatype_(default|compatibility)/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^arrow/datatype_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py python/tests/test__defaults.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/datatype.test.js node/tests/defaults.test.js
    npm run --prefix node bench:types:defaults
    ```
