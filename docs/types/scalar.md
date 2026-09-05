# Scalar

`Scalar` is the one value every layer speaks; the vocabulary, families, and `TypedScalar` sit beside it.

## Contract

| Key | Value |
| --- | --- |
| `DataTypeId`, `DataTypeKind` | [Datatype](datatype.md) identity, family |
| `Codec`, `Level` | Content coding, compression scale 0 to 9 |
| `DigestAlgorithm`, `Digest`, `Digester` | Hash identity, value, streaming state |
| `MimeType`, `MediaType` | Representation, ordered codings; suffix, coding, and `MAGIC_PROBE_LEN`-bounded content inference |
| `Scheme`, `IOKind`, `IOMode` | Scheme, resource kind, intent: `overwrite`, `append`, `merge`, `readonly`, `random` |
| `TimeUnit`, `Timezone`, `UnionMode`, `EdgeAlgorithm` | Resolution, zone, union layout, edge model |
| `Enum` | Kind, spelling, ordinal; projections emit the spelling |
| Views | `as_integer`, `as_float`, `as_decimal`, `as_temporal`; total equality, ordering, hash |
| Bindings | `yggdryl.enums`, `enums`; `TypedScalar` Rust only |

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

## Enum scalars

`Scalar.from_enum` keeps kind, spelling, and ordinal.

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

## Scalar families

Units pick date and time widths, duration the narrowest fitting count, decimal the coefficient; datetime stays 64-bit.
Rust only.

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

## TypedScalar

Rust only.

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

See [Field](field.md), [Arrow scalars](../arrow/scalars.md), and [Text](../text/index.md).

## Edges

- `readonly` or `random` at a write entry point -> refused.
- Empty or positional rows -> ambiguous; declare the `Field`.
- Physical Arrow identity -> exact constructors, [Rust only](numeric.md).
- `MimeType::PUFFIN` -> `application/vnd.apache.puffin`, `.puffin`, `PFA1`; the specification names no MIME type.
- Geospatial value across a binding -> WKB bytes; `wkb` reader [Rust only](geospatial.md).
- [ASCII](ascii.md) bases in `yggdryl.enums` -> [Python only](../extensions/python.md), building the shared `AsciiEnum`.
- Field inference -> Rust only; no binding reimplements it.

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

Release builds, Windows x86_64, AMD Ryzen 5 150, rustc 1.96.1, CPython 3.12.13, Node 24.18.0 (2026-08-24); no Node benchmark exists.

| Rust | 28.9 ns | 2.59 ns | 5.01 ns | 3.94 ns |
| Python | 200 ns | 98.8 ns | 100 ns | 73.7 ns |
| JavaScript | 3 us | 2 us | 2 us | 1 us |

Regenerate with `cargo bench --bench datatype --all-features -- "value/enum_"`,
`python benchmarks/scalars.py --iterations 10000`, and

Inference, same host, rustc 1.96.1 (2026-08-23), Criterion point estimates:

| inferred field | estimate |
| --- | ---: |
| scalar | 80.1 ns |
| array | 240 ns |
| one-record Struct | 985 ns |

```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(stable_hash_|from_float32|family_constructors|as_|temporal_|enum_|infer_|record_field_update|json_|checked_)'
python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
```
