# Date

One calendar day, held either as a count of days or as the milliseconds of its midnight.

## Contract

| | |
| --- | --- |
| Owned | `DateType` and its two leaves, `DataType::Date32` and `DataType::Date64` - `DateType` the family's view over them - the `DateField` marker, and the `Date32` and `Date64` values |
| Validated | nothing at the datatype: a date takes no parameter, so `date32()` and `date64()` are `const` and `DateType::validate` never refuses. The value is where the rules live - a `Date32` counts days, a `Date64` milliseconds, and both are naive |
| Lazy | nothing; the leaf is `Copy` and the value is a count, a unit and a zone |
| Cached | the [field](../field.md)'s Arrow projection; the datatype caches nothing |
| Refused | a zone, any other unit, a calendar date that does not exist, a `Date64` count that is not a whole midnight, and a day count outside 32 bits |

## DataType

`date32` is Arrow's `Date32` and `date64` its `Date64`. Neither takes a
parameter, because the unit *is* what the width means, so `DataTypeId::Date32`
answers `is_parameterized()` with `false` and the grammar has nowhere to put
one.

| leaf | `DataTypeId` | `as_u8()` | holds | unit | Arrow |
| --- | --- | --- | --- | --- | --- |
| `date32` | `Date32` | `0x32` | `i32` days since the Unix epoch | `d` | `Date32` |
| `date64` | `Date64` | `0x33` | `i64` milliseconds since the Unix epoch, always a midnight | `ms` | `Date64` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `date32` | `DataType::Date32` | `date` |
| `date64` | `DataType::Date64` | `date_millisecond` |

No logical name answers a date column: FIX's `LocalMktDate` and `UTCDateOnly`
resolve to that day's midnight as a [datetime](datetime.md), so a settlement
date and a transact time sit in comparable columns rather than in two shapes a
reader has to cast between. The [datatype page](../datatype.md) lists the whole
logical vocabulary.

=== "Rust"

    ```rust
    use yggdryl::DateType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit};

    // Two leaves, no parameter: the unit is what the width means.
    assert_eq!(DataType::date32(), DataType::Date32);
    assert_eq!(DataType::date64().date_type(), Some(DateType::Date64));
    assert_eq!(DateType::ALL, [DateType::Date32, DateType::Date64]);
    assert_eq!(DateType::default(), DateType::Date32);

    // The leaf answers its own unit, width, family and identifier.
    assert_eq!(DateType::Date32.unit(), TimeUnit::Day);
    assert_eq!(DateType::Date64.unit(), TimeUnit::Millisecond);
    assert_eq!(DateType::Date64.bit_width(), 64);
    assert_eq!(DateType::ALL.map(DateType::family), ["date"; 2]);
    assert_eq!(DateType::Date32.id(), DataTypeId::Date32);
    assert_eq!(DateType::from_id(DataTypeId::Date64), Some(DateType::Date64));
    assert_eq!(DateType::from_id(DataTypeId::Time32), None);
    assert_eq!(DataTypeId::Date32.as_u8(), 0x32);
    assert!(!DataTypeId::Date32.is_parameterized());

    // Every spelling reads back as itself, and another family answers `None`.
    assert_eq!(DataType::from_str("date")?, DataType::date32());
    assert_eq!(DataType::from_str("date_millisecond")?, DataType::date64());
    assert_eq!(DataType::date64().to_string(), "date64");
    assert_eq!(DataType::date32().kind(), DataTypeKind::Temporal);
    assert!(DataType::date32().validate().is_ok());
    assert_eq!(DataType::Int64.date_type(), None);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    # One datatype, two leaves; `date` is the narrow one.
    assert DataType("date") == DataType("date32")
    assert DataType("date_millisecond") == DataType("date64")
    assert DataType("date32").id == "date32"
    assert DataType("date32").kind == "temporal"
    assert str(DataType("date64")) == "date64"

    # Arrow's two date storages, and the import back.
    assert DataType("date32").into_arrow() == pa.date32()
    assert DataType.from_arrow(pa.date64()) == DataType("date64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // One datatype, two leaves; `date` is the narrow one.
    assert.ok(DataType.from('date').equals(DataType.from('date32')))
    assert.ok(DataType.from('date_millisecond').equals(DataType.from('date64')))
    assert.equal(new DataType('date32').id, 'date32')
    assert.equal(new DataType('date32').kind, 'temporal')
    assert.equal(DataType.from('date64').toString(), 'date64')

    // A date takes no parameter, so the grammar has nowhere to put one.
    assert.throws(() => new DataType('date32(s)'), /unexpected/)
    assert.throws(() => new DataType('date32(d,"UTC")'), /unexpected/)
    ```

## Field

`DateField` is the typed marker: one field carrying `DateType` itself, so the
width is read off the payload rather than matched out of a root datatype.
`try_new` takes a root `DataType` and refuses another family by name; `new`
takes the payload directly. The bindings have one factory per leaf, nullable
unless the call says otherwise.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, Field, TimeUnit};
    use yggdryl::{DateField, DateType};

    // The marker carries the family's own payload; the root redirects to it.
    let settlement = DateField::new("settlement", DateType::Date64, false);
    assert_eq!(settlement.name(), "settlement");
    assert_eq!(settlement.dtype(), &DataType::date64());
    assert_eq!(settlement.typed_dtype_ref().unit(), TimeUnit::Millisecond);
    assert_eq!(settlement.id(), DataTypeId::Date64);
    assert!(!settlement.is_nullable());

    // A datatype from another family is refused by name.
    let refused = DateField::try_new("settlement", DataType::time32(TimeUnit::Second)?, false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("date"), "{refused}");

    // Widened, it is the same column the root constructor builds.
    assert_eq!(settlement.to_field(), Field::new("settlement", DataType::date64(), false));
    assert_eq!(DataType::date32().required_field("day").dtype(), &DataType::date32());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    settlement = yggdryl.date64("settlement", nullable=False)
    assert settlement.name == "settlement"
    assert str(settlement.dtype) == "date64"
    assert settlement.nullable is False

    # The factory is a typed spelling of the one native `Field`.
    assert isinstance(settlement, Field)
    assert settlement.dtype == Field("settlement", "date64", nullable=False).dtype

    # Metadata rides beside the datatype, never inside it.
    day = yggdryl.date32("day", metadata={"source": "feed"})
    assert day.metadata["source"] == "feed"
    assert day.nullable is True
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const settlement = fields.date64('settlement', { nullable: false })
    assert.equal(settlement.name, 'settlement')
    assert.equal(settlement.dtype.toString(), 'date64')
    assert.equal(settlement.nullable, false)
    assert.ok(settlement instanceof Field)

    // Nullable unless the options say otherwise; metadata rides beside it.
    const day = fields.date32('day', { metadata: { source: 'feed' } })
    assert.equal(day.nullable, true)
    assert.equal(day.get('source'), 'feed')
    ```

## Scalar

`Scalar::Date32` counts days and `Scalar::Date64` milliseconds; both carry
their unit and `Timezone::NAIVE`, so the shared temporal readers never ask the
width. `Scalar::from_date(count, unit, zone)` picks the width from the unit,
`date32`/`date64` build one directly, and `date32_in`/`date64_in` validate the
unit and zone first.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    // The family constructor picks the width from the unit.
    let day = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
    assert_eq!(day.as_date32().map(|(count, ..)| count), Some(20_000));
    assert_eq!(
        Scalar::from_date(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE)?,
        Scalar::date64(86_400_000)
    );
    assert_eq!(Scalar::from_date(1, TimeUnit::Day, Timezone::NAIVE)?, Scalar::date32(1));

    // Unit and zone are checked where they are stated.
    assert!(Scalar::from_date(1, TimeUnit::Second, Timezone::NAIVE).is_err());
    assert!(Scalar::from_date(i64::MAX, TimeUnit::Day, Timezone::NAIVE).is_err());
    assert!(Scalar::date32_in(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
    assert!(Scalar::date64_in(1, TimeUnit::Second, Timezone::NAIVE).is_err());

    // The readers answer across both widths.
    assert_eq!(day.temporal_unit(), Some(TimeUnit::Day));
    assert_eq!(day.temporal_timezone(), Some(Timezone::NAIVE));
    assert_eq!(day.temporal_dtype(), Some(DataType::date32()));
    assert_eq!(Scalar::date64(86_400_000).temporal_count(), Some(86_400_000));
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import DataType, Scalar

    # A column's datatype reads the value; a `date` crosses as itself.
    assert DataType("date32").scalar(1).kind == "date32"
    assert DataType("date64").scalar(86_400_000).kind == "date64"
    assert DataType("date32").scalar(1).as_py() == dt.date(1970, 1, 2)
    assert Scalar.from_(dt.date(2026, 8, 15)).kind == "date32"
    assert Scalar.from_(dt.date(2026, 8, 15)).as_py() == dt.date(2026, 8, 15)

    # A date carries a unit and the zone-free marker, never a zone.
    value = DataType("date32").scalar(1)
    assert (value.count, value.unit, value.zone) == (1, "d", "NAIVE")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // JavaScript has no date value, so a date reads back as the Scalar itself
    // and `count`, `unit` and `zone` are the readers.
    const day = new DataType('date32').scalar(7)
    assert.equal(day.kind, 'date32')
    assert.equal(day.count, 7n)
    assert.equal(day.unit, 'd')
    assert.equal(day.zone, 'NAIVE')

    // A `date64` counts whole days in milliseconds, and the type checks it.
    const midnight = new DataType('date64').scalar(86_400_000n)
    assert.equal(midnight.kind, 'date64')
    assert.equal(midnight.count, 86_400_000n)
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `date32` | `Date32` | `date32` |
| `date64` | `Date64` | `date64` |

Both projections are Arrow's own and both round-trip, so a date column crosses
a boundary as the width it declared. A `date32` column casts to a `Date32Array`
and a `date64` one to a `Date64Array`: the family has no single array type, so
[the cast](../cast.md) answers an `ArrayRef` and the leaf says which to narrow
to.

## Text

Text is `YYYY-MM-DD`, and the compact `YYYYMMDD` a wire writes reads as the
same day. A year outside four digits has no spelling and keeps its count.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    // One day, two spellings, both widths.
    assert_eq!(DataType::date32().scalar("1970-01-02")?, Scalar::date32(1));
    assert_eq!(DataType::date64().scalar("1970-01-02")?, Scalar::date64(86_400_000));
    assert_eq!(
        DataType::date32().scalar("20240102")?,
        DataType::date32().scalar("2024-01-02")?
    );

    // A cast into text writes the spelling back.
    assert_eq!(DataType::utf8().scalar(Scalar::date32(1))?, Scalar::from("1970-01-02"));

    // A date that no calendar has is refused, not guessed.
    assert!(DataType::date32().scalar("2026-02-30").is_err());
    ```

=== "Python"

    ```python
    import datetime as dt

    import pyarrow as pa

    from yggdryl import Field

    day = Field("day", "date32", nullable=False)
    assert day.scalar("1970-01-02").as_py() == dt.date(1970, 1, 2)
    assert day.scalar("19700102").as_py() == dt.date(1970, 1, 2)
    assert day.scalar("1970-01-02").into_arrow_scalar() == pa.scalar(1, pa.date32())
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const day = fields.date32('day', { nullable: false })
    assert.ok(day.scalar('1970-01-02').equals(new DataType('date32').scalar(1)))
    assert.ok(day.scalar('19700102').equals(new DataType('date32').scalar(1)))
    assert.throws(() => day.scalar('2026-02-30'))
    ```

## A date under a datetime column

A reading that states no clock is that day's midnight, so a settlement date
read into a [datetime](datetime.md) column is an instant that compares with a
transact time directly rather than a second shape to cast through. The column
owns the zone, so a zoned one wants the offset stated and a naive one refuses
it; [the datetime page](datetime.md#a-date-under-a-datetime-column) shows both.

## Edges

- `date32(s)`, `date32(d,"UTC")` -> refused by the grammar as unexpected: a date has no parameter to spell.
- `Scalar::from_date(count, unit, zone)` -> `date unit must be day or millisecond`; a day count outside 32 bits is refused rather than widened.
- `Date32::new` and `Date64::new` refuse any zone but `NAIVE`, so a date never claims an instant.
- `2026-02-30` -> refused, not a guess; a year outside four digits keeps its count and has no text spelling.
- [Merged](../field.md) with the other width -> `date64`, the wider storage; merged with a [datetime](datetime.md) -> refused, two families.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- date::temporal temporal::fields
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^date/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_scalar.py -k "width_unit_scale_and_zone"
    python/.venv/bin/python -m pytest python/tests/test_scalar.py -k "temporals_cross"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="temporal families" node/tests/text/codec.test.js
    node --test --test-name-pattern="typed field factories" node/tests/fields.test.js
    ```
