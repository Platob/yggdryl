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
    assert_eq!(DataType::from_json(&value.to_json()?)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    value = DataType("decimal(18, 4)")
    assert value == DataType.decimal(18, 4)

    assert str(value) == "decimal128(18,4)"
    assert DataType(str(value)) == value
    assert DataType.from_json(value.to_json()) == value
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

There are 41 variants, one per Arrow logical type. The parser accepts the Arrow, SQL, Hive, and
Spark spellings of all of them - `bigint`, `varchar(255)`, `array<string>`, `row(...)`,
`double precision` - and normalizes to one canonical form, so `to_string` is a losslessly
re-parseable value rather than a debug rendering. `to_json` is a separate, structural encoding:
tagged objects that name every parameter, which is what a schema written to disk should be.

Scalar variants are inline and nested children sit behind shared allocations, so cloning a
`DataType` never allocates. The value is immutable; changing a type means building another one.

## Children

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let quote = DataType::from_fields([
        Field::new("symbol", DataType::Utf8, false),
        Field::new("levels", DataType::list(DataType::Float64.nullable_field("item")), true),
    ])?;

    assert_eq!(quote.field_len(), 2);
    assert_eq!(quote.get_field(0).map(Field::name), Some("symbol"));
    assert_eq!(quote.get_field_by_name("levels").unwrap().data_type().field_len(), 1);
    assert!(quote.get_field_by_name("missing").is_none());

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
    assert len(quote["levels"].data_type) == 1
    assert "levels" in quote and "missing" not in quote

    lookup = fields.map_of("lookup", "utf8", "int64", keys_sorted=True).data_type
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
    assert.equal(quote.at(-1).name, 'levels')
    assert.equal(quote.get('levels').dataType.length, 1)
    assert.equal(quote.contains('missing'), false)
    assert.deepEqual([...quote].map((field) => field.name), ['symbol', 'levels'])

    const lookup = fields.mapOf('lookup', 'utf8', 'int64', true).dataType
    assert.equal(lookup.length, 1)
    assert.equal(lookup.at(0).name, 'entries')
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

    assert.equal(fields.decimal('amount', 38, 4).dataType.toString(), 'decimal128(38,4)')
    assert.equal(fields.decimal('wide', 39, 4).dataType.toString(), 'decimal256(39,4)')
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

Units are parsed from the shared [`TimeUnit`](enums.md) vocabulary, so `s`, `sec`, `MILLIS`, `µs`,
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

    codes = fields.dictionary("codes", "int16", "utf8").data_type
    runs = fields.run_end_encoded(
        "runs",
        fields.int16("run_ends", nullable=False),
        fields.utf8("values"),
    ).data_type

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

    const codes = fields.dictionary('codes', 'int16', 'utf8').dataType
    const runs = fields
      .runEndEncoded('runs', fields.int16('run_ends', { nullable: false }), fields.utf8('values'))
      .dataType

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

## Unions and the variant alias

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, UnionMode};

    let members = [
        Field::new("number", DataType::Int64, false),
        Field::new("text", DataType::Utf8, true),
    ];
    let variant = DataType::variant(members.clone())?;

    // `variant` is not a second logical type: it is the dense union with IDs 0..
    assert_eq!(
        variant,
        DataType::union(
            [(0, members[0].clone()), (1, members[1].clone())],
            UnionMode::Dense,
        )?
    );
    assert_eq!(variant.name(), "union");
    assert!(variant.to_string().starts_with("union(dense,"));
    assert_eq!(DataType::from_str("variant(number:int64,text:string)")?.name(), "union");

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

A union pairs each member with an explicit Arrow type ID; a variant is the case where those IDs are
just `0..` in declaration order, so a caller lists members and nothing else. It is not a second
logical type: `variant` builds a `DataType::Union` with `UnionMode::Dense`, and display, JSON, Arrow
projection, and defaults all go through the union contract - which is why the canonical form of a
variant reads `union(dense, ...)`. It is also the only union constructor on `DataType` itself in
Python and JavaScript; an explicit mode goes through the `fields.union` factories there, and through
`DataType::union` in Rust.

Type IDs are `i8` and must be unique and non-negative, so a variant caps at 128 members. The parser
accepts `variant(...)`, `dense_union(...)`, and `sparse_union(...)` and canonicalizes all of them.

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

    assert.equal(fields.decimal('amount', 38, 4).dataType.id, 'decimal128')
    assert.equal(fields.decimal('amount', 38, 4).dataType.kind, 'decimal')
    ```

`id` names the variant and `kind` names the family it belongs to - 41 ids across 14 kinds. Both are
parameter-free, so they compare and hash without touching nested state, which is what makes them the
cheap way to branch. Dispatch on the kind when the behavior is uniform across a family, on the id
when it is not; `name()` is the Rust spelling of `id().as_str()`. The two vocabularies and the
predicates over them are documented in [shared enums](enums.md). Python and JavaScript have no
separate class for either: both arrive as the canonical lowercase strings.

## Arrow projection

=== "Rust"

    ```rust
    use yggdryl::{DataType, TimeUnit};

    let value = DataType::from_str("map<string,array<decimal(38,18)>>")?;
    let arrow = value.to_arrow()?;

    assert_eq!(DataType::from_arrow(&arrow)?, value);
    assert_eq!(value.clone().into_arrow()?, arrow);
    assert_eq!(DataType::try_from(arrow)?, value);

    // Projection re-checks parameters, so a directly built enum value cannot escape.
    assert!(DataType::Time32(TimeUnit::Nanosecond).to_arrow().is_err());
    assert!(DataType::Time32(TimeUnit::Nanosecond).to_arrow_ffi().is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    value = DataType("map<string,array<decimal(38,18)>>")
    arrow = value.to_arrow()

    assert DataType.from_arrow(arrow) == value
    assert DataType(arrow) == value
    assert value.into_arrow() == arrow

    assert DataType(pa.int64()) == DataType("int64")
    assert DataType("int64").to_arrow() == pa.int64()
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
rebuilding the value. In Rust `to_arrow` borrows and `into_arrow` consumes, while `to_arrow_ffi`
produces the `FFI_ArrowSchema` a foreign runtime imports.

The Node package has no `toArrow`: `fromArrow` reads any Arrow JS type through its own `toString`,
and the canonical display is the bridge back out. Nothing is inferred from a generic string
coercion, so an object without its own textual form is an error rather than `[object Object]`.

Projection is a validation boundary. Because `DataType` is a public Rust enum, a caller can
construct a variant with parameters no constructor would accept; every conversion repeats the checks
before materializing foreign state. Whole schemas cross the same boundary through
[`arrow`](arrow.md), which reads a struct-rooted `Field` as an Arrow `Schema`.

## Default values

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Value};

    let value = DataType::from_fields([
        Field::new("id", DataType::Int32, false),
        Field::new("note", DataType::Utf8, true),
    ])?;

    // One positional slot per child, each honoring its own nullability.
    assert_eq!(
        value.default_value()?.as_sequence().unwrap(),
        &[Value::I64(0), Value::Null]
    );
    assert!(value.is_default_value(&value.default_value()?)?);
    assert_eq!(DataType::Utf8.default_value()?, Value::String("".into()));

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

Every one of the 41 variants has a canonical default, and it is computed from the schema rather than
looked up per language: the core produces one value and each binding projects it. Rust yields a
[`Value`](generic.md); Python yields a dataclass record or a Python scalar from `default_pyvalue`
and a `pyarrow.Scalar` from `default_arrow_scalar`; JavaScript yields a plain array, `Buffer`,
`Map`, or `{ typeId, value }`. Mutable containers are freshly allocated on every call, so a default
is never shared state.

The walk is bounded in both depth and bytes. A fixed-size binary or fixed-size list whose default
would exceed the byte safety limit, and a nesting deeper than the parser limit, both fail loudly
instead of degrading to a null. `is_default_value` answers the reverse question - is this value the
canonical default - without allocating the default first.

A bare `DataType` has no nullability, so its default is the non-null one. Ask a
[`Field`](field.md) instead when the answer depends on whether the slot may be null.

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

    let spark = source.to_scheme_compat(&Scheme::SPARK)?;
    let rewritten = spark.as_fields().unwrap();
    assert_eq!(rewritten[0].data_type(), &DataType::Int16);
    assert_eq!(rewritten[1].data_type(), &DataType::decimal128(20, 0)?);
    assert_eq!(
        rewritten[2].data_type(),
        &DataType::list(DataType::Utf8.nullable_field("item"))
    );

    // Arrow is a validated clone; Polars keeps the unsigned integers Spark has to widen.
    assert_eq!(source.to_scheme_compat(&Scheme::ARROW)?, source);
    assert_eq!(DataType::UInt32.to_scheme_compat(&Scheme::POLARS)?, DataType::UInt32);

    // A rewrite that would reinterpret values is refused, and the path is named.
    let error = DataType::from_fields([Field::new(
        "created",
        DataType::Timestamp(TimeUnit::Nanosecond, None),
        false,
    )])?
    .to_scheme_compat(&Scheme::SPARK)
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

    spark = source.to_scheme_compat("spark")
    assert str(spark["small"].data_type) == "int16"
    assert str(spark["wide"].data_type) == "decimal128(20,0)"

    assert source.to_scheme_compat("arrow") == source
    assert DataType("uint32").to_scheme_compat("polars") == DataType("uint32")

    with pytest.raises(ValueError, match="got ns"):
        DataType("timestamp(ns)").to_scheme_compat("spark")
    with pytest.raises(ValueError, match="arrow, spark, polars, pandas"):
        DataType("int32").to_scheme_compat("duckdb")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const source = DataType.fromFields([
      fields.uint8('small'),
      fields.uint64('wide', { nullable: true }),
    ])

    const spark = source.toSchemeCompat('spark')
    assert.equal(spark.get('small').dataType.toString(), 'int16')
    assert.equal(spark.get('wide').dataType.toString(), 'decimal128(20,0)')

    assert.ok(source.toSchemeCompat('arrow').equals(source))
    assert.ok(DataType.from('uint32').toSchemeCompat('polars').equals(DataType.from('uint32')))

    assert.throws(() => DataType.from('timestamp(ns)').toSchemeCompat('spark'), /got ns/)
    assert.throws(
      () => DataType.from('int32').toSchemeCompat('duckdb'),
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

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/datatype-rust.ipynb){ download },
[Python](notebooks/datatype-python.ipynb){ download },
[JavaScript](notebooks/datatype-javascript.ipynb){ download }.

<!-- /notebooks -->
