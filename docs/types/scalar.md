# Scalar

`Scalar` is the one value every layer speaks; the shared vocabulary and `FieldScalar` sit beside it.

## Contract

| Key | Value |
| --- | --- |
| `DataTypeId`, `DataTypeKind` | [Datatype](datatype.md) identity, family |
| `Codec`, `Level` | Content coding, compression scale 0 to 9 |
| `DigestAlgorithm`, `Digest`, `Digester` | Hash identity, value, streaming state |
| `MimeType`, `MediaType` | Representation, ordered codings; suffix, coding, and `MAGIC_PROBE_LEN`-bounded content inference |
| `Scheme`, `IOKind`, `IOMode` | Scheme, resource kind, intent: `overwrite`, `append`, `merge`, `readonly`, `random` |
| `TimeUnit`, `Timezone`, `UnionMode`, `EdgeAlgorithm` | Resolution, zone, union layout, edge model |
| `Enum` | The vocabulary: kind, spelling, ordinal. A member's datatype is `string`, so its value is the spelling and no `Scalar` variant holds it |
| Widths | one flat enum: every width is its own variant (`Scalar::Int32`, `Scalar::Date32`, ...), matched directly and named by `kind()` |
| `Scalar::Arrow` | an [`ArrowScalar`](../arrow/values.md) behind one shared pointer: a columnar value crossing a boundary as the scalar it is, buffers shared; `into_native` reads it as rows, `as_arrow` borrows it, and the narrowing readers answer `None` |
| Readers | across widths: `as_i128`, `as_u128`, `as_i64`, `as_u64`, `as_f64`, `as_decimal`; `temporal_family`, `temporal_unit`, `temporal_timezone`, `temporal_count`, `None` for a non-temporal |
| Identity | total equality, ordering, hash, cross-width: `I32(7)` is `U8(7)`, `F32(1.5)` is `F64(1.5)`, `D32(1250, 2)` is `D256(125, 1)`; kinds stay apart, `I32(1)` is not `F64(1.0)` |
| Bindings | `yggdryl.enums`, `enums`; `FieldScalar` and the `wkb` reader Rust only |

## Use

One spelling per member at every boundary.

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

## Enum members

An enum member's datatype is `string` - there is no `DataType::Enum` - so a
member **is** its canonical name, and which vocabulary it belongs to is the
column's business. `Enum` is the vocabulary: `from_parts` validates a name
against it, `kind`, `as_str` and `ordinal` read the member, and building a
`Scalar` from one answers the text a column holds.

=== "Rust"

    ```rust
    use yggdryl::{Enum, IOMode, Scalar};

    let member = Enum::from_parts("IOMode", "append").expect("a known member");
    assert_eq!(member, Enum::IOMode(IOMode::Append));
    assert_eq!((member.kind(), member.as_str(), member.ordinal()), ("IOMode", "append", 1));

    // The value is the name, so it is the same scalar the text is.
    assert_eq!(Scalar::from(IOMode::Append), Scalar::from("append"));
    ```

=== "Python"

    ```python
    from yggdryl import Scalar

    value = Scalar.from_enum("IOMode", "append")
    assert value.kind == "string"
    assert value.as_py() == "append"
    assert value == Scalar.from_("append")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    const value = Scalar.fromEnum('IOMode', 'append')
    assert.equal(value.kind, 'string')
    assert.equal(value.asJs(), 'append')
    assert.deepEqual(value, Scalar.from('append'))
    ```

## Widths and readers

Every width is a `Scalar` variant: units pick the date and time width, a duration takes the narrowest width holding its count, a decimal a 128-bit coefficient unless it needs 256, and a datetime stays 64-bit.
The readers answer across widths and `None` for another kind; an interval's `temporal_count` is its nanosecond component, and its three components are matched as `Scalar::Interval` directly.
Rust only.

```rust
use yggdryl::{i256, Scalar, TemporalFamily, TimeUnit, Timezone};

let date = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
let time = Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE)?;
let duration = Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)?;
let decimal = Scalar::from_decimal(i256::from_i128(1_250), 2);

// The variant is the width, and `kind()` names it.
assert!(matches!(date, Scalar::Date32(_)));
assert_eq!(time.kind(), "time64");
assert!(matches!(duration, Scalar::Duration64(_)));

assert_eq!(time.temporal_family(), Some(TemporalFamily::Time));
assert_eq!(time.temporal_unit(), Some(TimeUnit::Nanosecond));
assert_eq!(date.temporal_timezone(), Some(Timezone::NAIVE));
assert_eq!(duration.temporal_count(), Some(i64::from(i32::MAX) + 1));
assert_eq!(decimal.temporal_family(), None);

// Numbers read across widths, and one number at two widths is one value.
assert_eq!(decimal.as_decimal(), Some((i256::from_i128(1_250), 2)));
assert_eq!(decimal, Scalar::d256(i256::from_i128(125), 1));
assert_eq!(Scalar::from(7_u8).as_i128(), Some(7));
assert_eq!(Scalar::from(7_u8), Scalar::from(7_i32));
```

## Truthiness and length

`is_truthy` is a coercion and answers for every value; `as_bool` is a reading
and answers only for a boolean, keeping the `None` a filter's three-valued
logic walks. Nothing falls back between them.

Falsy is absence and emptiness: `Null`, `false`, a zero of any width, empty
text or bytes, and a container with nothing set in it. That last one is wider
than `is_empty`, which only counts entries - a record of three nulls has three
fields and nothing set, so it is not empty but it is falsy.

Text is the one place this is wider than Python. `false`, `no`, `off`, `f`,
`n` and `0` read as false, case-insensitively and trimmed, where Python calls
every non-empty string true. Values arrive as text from CSV, FIX and query
strings, and a column that spells false is not asking to be read as true.
[`Boolean`](numeric.md)'s own text reader stays strict, because that
one is the String-to-Boolean *cast*, not a coercion.

`len` counts a container's direct children and answers zero for everything
else, so it is not a text or byte length and never a truthiness test.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    assert!(Scalar::from(5).is_truthy());
    assert!(!Scalar::from(0).is_truthy());
    assert!(!Scalar::from("OFF").is_truthy());
    assert!(Scalar::from("anything else").is_truthy());
    ```

=== "Python"

    ```python
    from yggdryl import Scalar

    assert bool(Scalar.from_(5))
    assert not bool(Scalar.from_(0))
    assert not bool(Scalar.from_("off"))
    assert not bool(Scalar.from_({"a": None, "b": ""}))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.equal(Scalar.from(5).isTruthy(), true)
    assert.equal(Scalar.from(0).isTruthy(), false)
    assert.equal(Scalar.from('off').isTruthy(), false)
    ```

## Variants and arithmetic

Every width is a direct `Scalar` variant, with no family enum between (`Scalar::Int32(Int32(2))`). Every `Scalar` is hashable and totally ordered; equal numeric or temporal values compare and hash equal across storage widths (`Int32(7)` equals `UInt8(7)`). Width stays available for datatype and Arrow projection.

| group | variants |
| --- | --- |
| absence and logic | `Null`, `Boolean` |
| integers | `I8`, `I16`, `I32`, `I64`, `I128`, `U8`, `U16`, `U32`, `U64`, `U128` |
| floats | `F16`, `F32`, `F64` |
| decimals | `D32`, `D64`, `D128`, `D256`, each a coefficient and a scale |
| text and binary | `String`, `Code`, `Bytes`, `Geometry`, `Geography` |
| identifiers | `Uuid`, `Version`, `Url` |
| date and time | `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64` |
| elapsed time | `Duration32`, `Duration64`, `Interval` |
| containers | `Sequence`, `Mapping`, `Record` |

Arithmetic is checked in the Rust value model, both bindings redirect to it, and only unambiguous typed results exist.

| operands | supported operations | result rule |
| --- | --- | --- |
| integers | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | keep a shared width; mixed signed/unsigned inputs promote only when lossless |
| floats | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | retain the widest float input; mixing an integer uses `F64` |
| exact decimals | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | preserve an exact coefficient and scale; an inexact quotient is refused |
| temporal and duration | temporal `+/-` duration, temporal `-` temporal, duration `+/-` duration, duration `*` integer, duration `/` integer | preserve the temporal kind or return an exact duration in the finest required unit |
| text, bytes, sequences | `+` only | concatenation - the join a repertoire with no sum has. Both sides one repertoire; a code joins as its text and stops being a code; a WKB payload does not join |
| null | every binary operation above | propagate `Null` |

Rust has `checked_add`, `checked_sub`, `checked_mul`, `checked_div`, `checked_rem`, `checked_neg`, `checked_abs`, and `Result<Scalar>` operator traits; Python adds operators, JavaScript only the named methods.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    assert_eq!(
        Scalar::from(-1_i8).checked_add(&Scalar::from(2_u8))?,
        Scalar::from(1_i16),
    );
    assert_eq!(
        Scalar::d128(1, 0).checked_div(&Scalar::d128(2, 0))?,
        Scalar::d128(5, 1),
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar

    assert (Scalar.from_(40) + 2).as_py() == 42
    assert Scalar.decimal(1, 0).divide(Scalar.decimal(2, 0)) == Scalar.decimal(5, 1)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.equal(Scalar.from(40).add(2).asJs(), 42)
    assert.ok(Scalar.decimal(1n).divide(Scalar.decimal(2n)).equals(Scalar.decimal(5n, 1)))
    ```

| item | rule |
| --- | --- |
| rows | `Record` is sorted name-to-value input; a Struct `Field` resolves it into one `Sequence` in child-field order; `Mapping` is insertion-ordered with any unique `Scalar` key |
| accessors | `as_bytes`, `as_str`, `into_json_bytes` / `into_json`, `as_decimal`, and the temporal readers `temporal_family`, `temporal_unit`, `temporal_timezone`, `temporal_count`; native `from_*` / `into_*` [Arrow](../arrow/scalars.md) conversions; binding read-only `count`, `unit`, `zone`, `unscaled`, `scale` |

## FieldScalar

One `Field` and the value its own contract answered, held together. The field is
borrowed, so a reader downstream takes the name, the datatype and the value from
one place and nothing copies the schema per value. `UncheckedFieldScalar` is the
same pairing before that proof: it holds whatever it was given and casts on
read, and `checked` is where it becomes a `FieldScalar`. Rust only.

```rust
use yggdryl::types::UncheckedFieldScalar;
use yggdryl::{DataType, Field, FieldScalar, Scalar};

let price = Field::new("price", DataType::Int32, false);
let held = FieldScalar::new(&price, 7_i64)?;
assert_eq!(held.name(), "price");
assert_eq!(held.dtype(), &DataType::Int32);
// The value was narrowed to the width the field declares.
assert!(matches!(held.value(), Scalar::Int32(_)));
assert_eq!(held.as_i64(), Some(7));

// Nullability is the field's rule, so a required column refuses a null.
assert!(FieldScalar::new(&price, Scalar::Null).is_err());

// Text reads under the field, which is the one door from text to a value.
assert_eq!(FieldScalar::parse_str(&price, "42")?.as_i64(), Some(42));

// A value that names its own datatype borrows the field the crate prebuilt
// for it, so inferring one copies nothing.
let inferred = FieldScalar::infer(Scalar::from(7_i64))?;
assert_eq!(inferred.dtype(), &DataType::Int64);

// Unchecked, the text is held as given and read on demand.
let raw = UncheckedFieldScalar::from_str(&price, "42");
assert_eq!(raw.as_i64(), Some(42));
assert!(matches!(raw.checked()?.value(), Scalar::Int32(_)));
```

## Inferred fields

Without a schema, `Scalar` exposes the inferred `Field`: `value`, `item`, or `row` by shape.

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

    assert Scalar.from_(42).into_field().name == "value"
    assert Scalar.from_([1, None]).into_array_field().name == "item"
    assert Scalar.from_([Row(1)]).into_struct_field().name == "row"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.equal(Scalar.from(42).intoField().name, 'value')
    assert.equal(Scalar.from([1, null]).intoArrayField().name, 'item')
    assert.equal(Scalar.from([{ id: 1 }]).intoStructField().name, 'row')
    ```

See [Field](field.md), [Arrow scalars](../arrow/scalars.md), and [Structured documents](../media/structured.md).

## Edges

- `readonly` or `random` at a write entry point -> refused.
- Overflow, division by zero, inexact decimal quotient, undefined operand pair -> four separate core errors.
- `+` on text, bytes or a sequence -> concatenation, the join a repertoire with
  no sum has. Both sides must be the same repertoire, and only `+`: `-`, `*`,
  `/` and `%` over text stay refusals. A code joins as its text and stops being
  a code. Geospatial values read as bytes but do not join - two WKB payloads
  end to end are not a geometry. This is not the expression language's
  `concat`, which is a variadic text function that also renders a version.
- `count`, `unit`, `zone`, `unscaled`, `scale`, or a Rust `temporal_*` reader on an unrelated kind -> `None` / `null`; an `Interval` answers `temporal_count` with its nanosecond component.
- Empty or positional rows -> ambiguous; declare the `Field`.
- Physical Arrow identity -> exact constructors, [Rust only](numeric.md).
- `MimeType::PUFFIN` -> `application/vnd.apache.puffin`, `.puffin`, `PFA1`; the specification names no MIME type.
- Geospatial value across a binding -> WKB bytes; `wkb` reader [Rust only](geospatial.md).
- [Code](codes.md) bases in `yggdryl.enums` -> Python only: the fixed US-ASCII widths `fixed_ascii(width)` builds and the four registered code bases, building the shared `StringEnum`.
- Field inference -> `Scalar.into_field` in Python, beside the `into_field` a `@scalar` class caches for its own struct root; no binding reimplements it.
- Named record rows -> a non-null Struct root named `row`.
- Default `arrow` feature -> `into_arrow_array` materializes one row, `from_arrow_array` decodes one back ([Arrow scalars](../arrow/scalars.md)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::scalar types::enumeration types::arithmetic types::decimal::scalars types::temporal::scalars
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- enums::
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(stable_hash_|from_float32|family_constructors|as_|temporal_|enum_|infer_|record_field_update|json_|checked_)'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(enum_accessors|mime_parse|media_infer)/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_native_scalar.py python/tests/types/test_scalar.py python/tests/test_enums.py
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/types/native-scalar-returns.test.js node/tests/enums.test.js
    ```

## Performance

### Enum and inference boundary

Enum boundary in release builds, Windows x86_64, AMD Ryzen 5 150, rustc 1.96.1, CPython 3.12.13, Node 24.18.0 (2026-08-24). No Node benchmark regenerates the JavaScript row.

| boundary | construct | kind | spelling | ordinal |
| --- | ---: | ---: | ---: | ---: |
| Rust | 28.9 ns | 2.59 ns | 5.01 ns | 3.94 ns |
| Python | 200 ns | 98.8 ns | 100 ns | 73.7 ns |
| JavaScript | 3 us | 2 us | 2 us | 1 us |

Inference, same host, rustc 1.96.1 (2026-08-23), Criterion point estimates.

| inferred field | estimate |
| --- | ---: |
| scalar | 80.1 ns |
| array | 240 ns |
| one-record Struct | 985 ns |

```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(stable_hash_|from_float32|family_constructors|as_|temporal_|enum_|infer_|record_field_update|json_|checked_)'
python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
```

### Scalar and Arrow boundary costs

Windows x86_64 release smoke runs, Criterion group `value` in `--bench types` and `python/benchmarks/types/scalars.py`, with conversion setup outside the timed loop. Regenerate on the deployment host before comparing releases.

| Rust core operation | estimate |
| --- | ---: |
| stable hash of a four-field `Record` | 227 ns |
| infer that Record's datatype | 675 ns |
| persistent Record field update | 273 ns |
| restate Date32 days as nanoseconds | 3.10 ns |
| `into_json_bytes` | 2.67 us |
| `into_json` | 2.67 us |

| CPython release boundary | estimate |
| --- | ---: |
| native Python into / from `Scalar` | 1.66 us / 596 ns |
| stable hash | 232 ns |
| JSON bytes / UTF-8 | 675 ns / 664 ns |
| Arrow scalar into / from `Scalar` | 16.3 us / 6.66 us |
| Arrow array, 4,096 values, into / from `Scalar` | 312 us / 1.76 ms |
| Arrow batch, 4,096 rows, into / from `Scalar` | 1.27 ms / 1.88 ms |
| Arrow table, 4,096 rows, into / from `Scalar` | 1.39 ms / 1.94 ms |

```bash
cargo bench -p yggdryl --bench types -- value
python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
```
