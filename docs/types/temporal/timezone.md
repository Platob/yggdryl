# Time zone

One way to name a zone everywhere in the project, and the datatype a column of them declares.

## Contract

| | |
| --- | --- |
| Owned | the `Timezone` value, its bundled IANA rules registry, `DataType::Timezone`, the `TimezoneField` marker, and `Scalar::Timezone` |
| Validated | at parsing: an alias resolves to what it stands for, case folds, and a fixed offset normalizes to `+HH:MM`, so two spellings of one zone are one value. An offset beyond 24 hours of UTC or finer than a whole minute is refused |
| Lazy | nothing; the value is a four-byte handle and the rules are compiled in |
| Cached | the handle is interned for the process lifetime, so a zone is four bytes wherever it is carried |
| Refused | an empty name, a name holding a control character, an offset out of range, and merging into text, which would drop the canonicalization |

A zone is not a bounded [code](../codes/index.md) and not a static vocabulary -
an IANA name is as long as the registry says and a fixed offset is generated
rather than enumerated - so it is its own datatype, the way a URL and a version
are.

## DataType

`DataType::Timezone` takes no parameter. Its kind is `text`, because what it
stores is a canonical name; the column it declares holds the same value a
[datetime](datetime.md) declares, so a zone read out of a table is a zone a
`datetime64` column can be built with rather than text that happens to name
one.

| | |
| --- | --- |
| spelling | `timezone`, also parsed as `tz` and `timezone_name` |
| `DataTypeId` | `Timezone`, `as_u8()` `0x66` - in the text family's range, because `as_u8` is a wire contract laid out by family |
| kind | `text` |
| storage | `Utf8` holding the canonical name, extension name `yggdryl.timezone` |
| default | `NAIVE`, the zone-free marker every temporal already defaults to |

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Scalar, Timezone};

    let dtype = DataType::Timezone;
    assert_eq!(dtype.id(), DataTypeId::Timezone);
    assert_eq!(dtype.kind(), DataTypeKind::Text);
    assert_eq!(dtype.name(), "timezone");
    assert_eq!(dtype.to_string(), "timezone");
    assert_eq!(DataTypeId::Timezone.as_u8(), 0x66);
    assert!(!DataTypeId::Timezone.is_parameterized());

    // Three spellings reach the one datatype, and the document is structural.
    assert_eq!(DataType::from_str("TIMEZONE")?, dtype);
    assert_eq!(DataType::from_str("tz")?, dtype);
    assert_eq!(DataType::from_str("timezone_name")?, dtype);
    assert_eq!(dtype.clone().into_json()?, r#"{"type":"timezone"}"#);

    // The default is the zone-free marker, and merging keeps it a zone.
    assert_eq!(dtype.default_value()?, Scalar::Timezone(Timezone::NAIVE));
    assert_eq!(dtype.merge_with(&DataType::Timezone, true)?, DataType::Timezone);
    assert!(dtype.merge_with(&DataType::utf8(), true).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType("timezone")
    assert dtype.id == "timezone"
    assert dtype.kind == "text"
    assert str(dtype) == "timezone"
    assert DataType("tz") == dtype
    assert DataType("timezone_name") == dtype

    # The default is the zone-free marker.
    assert dtype.default_scalar().as_py() == "NAIVE"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dtype = new DataType('timezone')
    assert.equal(dtype.id, 'timezone')
    assert.equal(dtype.kind, 'text')
    assert.equal(dtype.toString(), 'timezone')
    assert.ok(DataType.from('tz').equals(dtype))
    assert.ok(DataType.from('timezone_name').equals(dtype))
    ```

## Field

`TimezoneField` carries no parameters, so it is built with `unit(name,
nullable)` - there is nothing to pass, and naming the datatype again would say
it twice. The bindings spell it `types.timezone` and `fields.timezone`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, FieldScalar, Scalar, Timezone, TimezoneField};

    let typed = TimezoneField::unit("zone", true);
    assert_eq!(typed.dtype(), &DataType::Timezone);
    assert!(typed.is_nullable());

    // The root field is the same column.
    let field = typed.to_field();
    assert_eq!(field, Field::new("zone", DataType::Timezone, true));
    assert_eq!(field.scalar("Asia/Calcutta")?, Scalar::Timezone(Timezone::from_str("Asia/Kolkata")?));

    // A value under the field carries the proof its contract answered.
    let held = FieldScalar::new(&field, field.scalar("UTC")?)?;
    assert_eq!(held.dtype(), &DataType::Timezone);
    assert!(FieldScalar::new(&field, 7_i64).is_err());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Scalar
    from yggdryl import json

    field = yggdryl.timezone("zone", nullable=False)
    assert str(field.dtype) == "timezone"
    assert field.nullable is False
    assert field.dtype == DataType("timezone")

    # A document read under the field is a zone, canonical on arrival.
    value = json.loads('"Asia/Calcutta"', field=field, cls=Scalar)
    assert value.as_py() == "Asia/Kolkata"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const field = fields.timezone('zone', { nullable: false })
    assert.equal(field.dtype.toString(), 'timezone')
    assert.equal(field.nullable, false)
    assert.ok(field.dtype.equals(new DataType('timezone')))

    // A document read under the field is a zone, canonical on arrival.
    const value = json.loads('"Asia/Calcutta"', { field, scalar: true })
    assert.equal(value.asJs(), 'Asia/Kolkata')
    ```

## Scalar

`Scalar::Timezone` holds `crate::Timezone` itself - not a second zone type -
so a zone read out of a column is the zone a temporal column declares, rules
and all. Parsing canonicalizes, so equal values hash equally, which is what a
key column needs.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    // An alias resolves to what it stands for, and case folds.
    assert_eq!(DataType::Timezone.scalar("Asia/Calcutta")?, DataType::Timezone.scalar("Asia/Kolkata")?);
    assert_eq!(DataType::Timezone.scalar("utc")?, Scalar::Timezone(Timezone::UTC));
    assert_eq!(DataType::Timezone.scalar("Z")?, DataType::Timezone.scalar("UTC")?);
    // A fixed offset normalizes to `+HH:MM`.
    assert_eq!(DataType::Timezone.scalar("-0800")?, Scalar::Timezone(Timezone::from_str("-08:00")?));
    // The zone-free marker is a value like any other.
    assert_eq!(DataType::Timezone.scalar("NAIVE")?, Scalar::Timezone(Timezone::NAIVE));

    // The value is the one a temporal column declares.
    let Scalar::Timezone(zone) = DataType::Timezone.scalar("Asia/Calcutta")? else {
        panic!("expected a timezone scalar")
    };
    assert_eq!(Scalar::datetime64(0, TimeUnit::Second, zone)?.temporal_timezone(), Some(zone));

    // An empty cell entering a non-text column is no value, and a bad one is refused.
    assert_eq!(DataType::Timezone.scalar("")?, Scalar::Null);
    assert!(DataType::Timezone.scalar("+99:00").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    dtype = DataType("timezone")
    assert dtype.scalar("Asia/Calcutta").as_py() == "Asia/Kolkata"
    assert dtype.scalar("utc").as_py() == "UTC"
    assert dtype.scalar("-0800").as_py() == "-08:00"
    assert dtype.scalar("NAIVE").as_py() == "NAIVE"
    assert dtype.scalar("Asia/Calcutta").kind == "timezone"

    # An empty cell is no value, and a bad one is refused.
    assert dtype.scalar("").is_null()
    with pytest.raises(ValueError):
        dtype.scalar("+99:00")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dtype = new DataType('timezone')
    assert.equal(dtype.scalar('Asia/Calcutta').asJs(), 'Asia/Kolkata')
    assert.equal(dtype.scalar('utc').asJs(), 'UTC')
    assert.equal(dtype.scalar('-0800').asJs(), '-08:00')
    assert.equal(dtype.scalar('Asia/Calcutta').kind, 'timezone')

    // A bad name is refused rather than kept.
    assert.throws(() => dtype.scalar('+99:00'))
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `timezone` | `Utf8` carrying the canonical name, under `ARROW:extension:name` = `yggdryl.timezone` | `timezone` |

The extension name is what keeps a zone a zone across a round trip; the
ordering is the canonical name's, which is Arrow's own string order. A text
column cast into a zone column is canonicalized row by row, and a row that
names no zone says which row it was.

```rust
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{DataType, Field, FieldValue as _};

let field = Field::new("zone", DataType::Timezone, true);
let arrow = field.clone().into_arrow_field()?;
assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
assert_eq!(
    arrow.metadata().get("ARROW:extension:name").map(String::as_str),
    Some("yggdryl.timezone")
);
assert_eq!(Field::from_arrow_field(&arrow)?.dtype(), &DataType::Timezone);
```

## The value and its registry

The rules are compiled in: no files, no environment, no network. A registered
zone answers its own offsets, abbreviations and daylight-saving state at an
instant; a zone this build has no rules for answers `None` rather than
guessing, which is recoverable where a plausible wrong offset is not.

=== "Rust"

    ```rust
    use yggdryl::Timezone;

    // Aliases and case canonicalize; a fixed offset needs no registry at all.
    assert_eq!(Timezone::from_str("Asia/Calcutta")?, Timezone::from_str("Asia/Kolkata")?);
    assert_eq!(Timezone::from_str("US/Eastern")?.as_str(), "America/New_York");
    assert_eq!(Timezone::from_str("Z")?, Timezone::UTC);
    assert_eq!(Timezone::from_str("-0800")?.as_str(), "-08:00");
    assert_eq!(Timezone::from_offset(5 * 3600 + 1800)?.as_str(), "+05:30");
    assert_eq!(Timezone::from_str("+05:30")?.offset_at(0), Some(5 * 3600 + 1800));

    // A registered zone knows its own rules.
    let new_york = Timezone::from_str("America/New_York")?;
    assert_eq!(new_york.offset_at(1_700_000_000), Some(-5 * 3600));  // November: EST
    assert_eq!(new_york.offset_at(1_688_000_000), Some(-4 * 3600));  // June: EDT
    assert_eq!(new_york.abbreviation_at(1_688_000_000), Some("EDT"));
    assert_eq!(new_york.is_saving_at(1_688_000_000), Some(true));
    assert_eq!(new_york.standard_offset(), Some(-5 * 3600));
    assert!(new_york.observes_saving());
    assert!(new_york.is_known());
    assert!(!new_york.is_fixed());

    // The markers, and the registry the build carries.
    assert!(Timezone::UTC.is_utc());
    assert!(Timezone::NAIVE.is_naive());
    assert!(Timezone::registered().len() > 60);
    assert!(Timezone::aliases().len() > 0);

    // An offset no real zone uses is refused.
    assert!(Timezone::from_offset(25 * 3600).is_err());
    assert!(Timezone::from_offset(30).is_err());
    assert!(Timezone::from_str("").is_err());
    ```

=== "Python"

    ```python
    import datetime
    import zoneinfo

    from yggdryl import Timezone

    def utc(year: int, month: int, day: int) -> int:
        moment = datetime.datetime(year, month, day, tzinfo=datetime.timezone.utc)
        return int(moment.timestamp())

    # Every way Python names a zone arrives at the same value.
    assert Timezone("Asia/Calcutta") == Timezone("Asia/Kolkata")
    assert Timezone(zoneinfo.ZoneInfo("Asia/Calcutta")) == Timezone("Asia/Kolkata")
    assert Timezone(datetime.timezone(datetime.timedelta(hours=5, minutes=30))) == Timezone("+05:30")
    assert Timezone(datetime.timezone.utc) == Timezone.UTC
    # Named `key` so a Timezone can stand in wherever only the name is read.
    assert Timezone("US/Eastern").key == "America/New_York"

    new_york = Timezone("America/New_York")
    assert new_york.offset_at(utc(2024, 1, 15)) == -5 * 3600
    assert new_york.offset_at(utc(2024, 7, 15)) == -4 * 3600
    assert new_york.abbreviation_at(utc(2024, 1, 15)) == "EST"
    assert new_york.is_saving_at(utc(2024, 7, 15))
    assert Timezone.UTC.into_utc(utc(2024, 1, 15)) == utc(2024, 1, 15)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Timezone } = require('yggdryl')

    const utc = (year, month, day) => Date.UTC(year, month - 1, day) / 1000

    // A name, an alias, and a fixed offset all arrive at one value.
    assert.ok(Timezone.fromString('Asia/Calcutta').equals(Timezone.from('Asia/Kolkata')))
    assert.equal(Timezone.from('US/Eastern').key, 'America/New_York')
    assert.equal(Timezone.fromOffset(5 * 3600 + 1800).key, '+05:30')
    assert.equal(Timezone.fromString('+0530').key, '+05:30')
    assert.ok(Timezone.from('Z').equals(Timezone.UTC))

    const newYork = Timezone.fromString('America/New_York')
    assert.equal(newYork.offsetAt(utc(2024, 1, 15)), -5 * 3600)
    assert.equal(newYork.abbreviationAt(utc(2024, 7, 15)), 'EDT')
    assert.equal(newYork.isSavingAt(utc(2024, 7, 15)), true)
    assert.ok(newYork.observesSaving())
    // The duck-typing half: minutes west, as `Date.getTimezoneOffset` reports.
    assert.equal(Timezone.from('Europe/Paris').getTimezoneOffset(utc(2024, 7, 15)), -120)

    // An offset no real zone uses is refused.
    assert.throws(() => Timezone.fromOffset(25 * 3600))
    assert.throws(() => Timezone.fromOffset(30))
    assert.throws(() => Timezone.fromString(''))
    ```

## Spellings

| Spelling | Canonical value |
| --- | --- |
| `Asia/Calcutta`, `US/Eastern` | `Asia/Kolkata`, `America/New_York` (`key` in Python and JavaScript) |
| `UTC`, `utc`, `Z`, `GMT`, `Etc/UTC`, `+00:00` | `UTC` (`is_utc`, `is_fixed`) |
| `+0530`, `from_offset(19800)`, a fixed-offset `tzinfo` | `+05:30` (`is_fixed`) |
| `zoneinfo.ZoneInfo("Europe/Paris")` (Python) | `Europe/Paris` (`observes_saving`) |
| `Timezone::NAIVE` | wall clock; `is_naive`, projects to Arrow as no timezone |

## Edges

- `Timezone("")`, `Timezone("+25:00")` -> `ValueError` (JavaScript throws); `Timezone.fromOffset(25 * 3600)` and `Timezone.fromOffset(30)` throw, an offset having to be within a day of UTC and a whole number of minutes; Python `Timezone(object())` -> `TypeError`.
- A name the build has no rules for is kept, not refused: `Custom/Accepted` parses, `is_known()` is false, and `offset_at`, `standard_offset`, `abbreviation_at` and `is_saving_at` all answer `None` rather than guessing. Converting a reading through it is an error.
- An empty text cell entering a zone column is `Null`, not an empty name; `+99:00` in a cast names the row it failed on and says it does not read as a time zone.
- Merging is only with itself: merging into text would drop the canonicalization.
- The value crosses a binding as its canonical name; JavaScript and Python each have a `Timezone` class, and a temporal value's `zone` reads the name.
- `NAIVE` is a value like any other and is the datatype's default, which is why a wall-clock [datetime](datetime.md) projects to Arrow with no zone at all.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- timezone::zones
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- timezone::internal
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_timezone.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/timezone.test.js
    ```
