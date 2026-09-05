# DataType

The owned logical type of one value, with Arrow kept out of the value model.

## Contract

| | |
| --- | --- |
| Owns | 56 variants: every Arrow logical type, Variant, geometry, geography, UUID, six ASCII widths, four registered codes |
| Parses | Arrow, SQL, Hive, Spark, and FIX logical-name spellings; `to_string` is the one canonical, re-parseable form |
| Identity | `id()` names the variant, `kind()` the family: 56 ids, 17 kinds, both parameter-free |
| Serializes | one structural model (`into_value`, `into_dict`); JSON, YAML, and TOML are three writers over it |
| Defaults | one canonical non-null default per variant, freshly allocated on every call |
| Limits | `PARSE_RECURSION_LIMIT` = 64 bounds parsing, defaults, and compatibility walks; a default above 64 MiB errors |
| Compatibility | `arrow`, `spark`, `polars`, `pandas`, `iceberg`: layout-only rewrites, meaning changes refused |
| Immutable | cloning never allocates; a changed type is a new value |
| Rust only | the public enum, `validate`, `{:#}`; YAML, TOML, and `pretty` are not yet in JavaScript |

## Use

Parse any spelling, display the canonical one, and round-trip both text forms.

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

A FIX datatype name is one more spelling of an ordinary datatype, resolved by `from_logical_name` and the grammar alike.

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

The registry is the FIX Latest datatype table plus `mic` and `cfi`; `currency`, `country`, and `mic` also name a [prebuilt vocabulary](ascii.md).

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

## Identity and family

`id` names the variant and `kind` its family; both drop parameters, so branch on them without touching nested state.

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

Both vocabularies and their predicates live with [Scalar](scalar.md); Python and JavaScript receive them as lowercase strings.

## Arrow projection

Rust and Python exchange a real Arrow type; Node reads an Arrow JS type through its `toString`.

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

Every conversion re-checks parameters, so a hand-built enum value cannot escape; whole schemas cross through [Schema](../arrow/schema.md).

## Default values

The core computes one default from the schema; Rust yields a `Scalar`, Python a dataclass or `pyarrow.Scalar`, JavaScript a plain value.

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

`is_default_value` checks a candidate without building the default; a nullable slot's default is a [Field](field.md) question.

## Serializing a schema

Three formats write one structural model, so a schema embeds inline in any configuration document.

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

| call | form |
| --- | --- |
| `into_json`, `into_yaml`, `into_toml` | text, with the shared [Formatting](../text/index.md) option (`indent=` in Python) |
| `into_json_bytes`, `toJSONBytes` | the same JSON document, encoded |
| `from_json` | bytes, text, or the object `json.loads` and `JSON.parse` made |

## A readable rendering

`Display`, `str`, and `repr` stay the compact round-tripping form; the alternate `{:#}` and `pretty()` render one fact per line, one indent per level.

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

`into_scheme_compat` walks the type recursively and applies only the layout rewrites one target needs.

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

| target | rewrite |
| --- | --- |
| `arrow` | validated clone |
| `spark` | `uint8` -> `int16`, `uint64` -> `decimal128(20,0)`, fixed-size list -> list |
| `polars` | keeps unsigned integers and fixed-size lists; no map, names key/value structs instead |
| `pandas` | no map, names key/value structs instead |
| `iceberg` | `uint8`, `uint16`, `int8`, `int16` -> `int32`; keeps `fixed[n]` and both timestamp widths; no duration or interval |

Applied to a [Field](field.md), the call keeps name, nullability, and metadata. [Iceberg](../media/iceberg/index.md) is a closed primitive vocabulary rather than an engine.

## Building the enum directly

Rust only. The public enum admits states no constructor would produce; `validate` is the backstop, and every binding entry point validates first.

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

## Edges

- `DataType::Time32(TimeUnit::Nanosecond)` built directly -> `validate`, `into_arrow`, and `into_arrow_ffi` fail; `DataType::time32(...)` refuses at construction.
- `FixedSizeBinary(64 * 1024 * 1024 + 1).default_value()` -> error, not null; nesting past 64 levels -> error.
- `into_scheme_compat("duckdb")` -> refused by name; the message lists the accepted targets.
- `datetime64(ns)` to `spark` -> refused with `got ns` and the node path (`$["a.b"]`, `$[].item`); a negative scale is never clamped, extension metadata never relabeled.
- `DataType.fromArrow({})` in JavaScript -> `own textual representation` error, never `[object Object]`.
- `int`, `float`, `char`, `String`, `Boolean` -> grammar meanings (`int32`, `float32`, `utf8`, `utf8`, `boolean`), not FIX ones.
- `TZTimestamp` -> the instant under `"UTC"`, offset dropped; read under `datetime64(ns,"<zone>")` for the local value.
- `AsciiEnum::prebuilt_values("tenor")` -> empty; only `currency`, `country`, and `mic` prebuild a vocabulary.
- Rust `into_arrow` and `into_arrow_ffi` consume the source -> clone first to keep it.
- `dictionary_id` of 0, unset `dictionary_is_ordered`, empty metadata -> omitted by every format and by `pretty`, never emitted as null.
- Bare `DataType` -> the non-null default; Python `repr` -> `DataType.from_str(...)`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::parser datatype::serde datatype::logical datatype::default datatype::compatibility datatype::arrow datatype::scalar default_scalar:: value_bounds::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::tests::logical types::tests::vocabulary types::tests::datatype_ types::arrow::
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/(scalar_sql|nested_sql_hive|near_limit_nested|logical_)'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^datatype_(default|compatibility)/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^arrow/datatype_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_defaults.py
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/types/datatype.test.js node/tests/types/defaults.test.js
    npm run --prefix node bench:types:defaults
    ```
