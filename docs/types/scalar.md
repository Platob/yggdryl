# Scalar

`Scalar` is the one value every layer speaks, and this page owns the shared vocabulary, families, and `TypedScalar` beside it.

## Contract

| Key | Value |
| --- | --- |
| Shared enums | One named root file each, re-exported at the crate root; `yggdryl.enums` (Python), `enums` (JavaScript) |
| `DataTypeId`, `DataTypeKind` | Exact [datatype](datatype.md) identity and family |
| `Codec`, `Level` | Content coding and the shared 0–9 compression scale |
| `DigestAlgorithm`, `Digest`, `Digester` | Hash algorithm identity, the value it answers, and the runtime-selected streaming state |
| `MimeType`, `MediaType` | Base representation and ordered content codings; they own suffix and coding inference, bounded by `MAGIC_PROBE_LEN` |
| `Scheme`; `IOKind`, `IOMode` | URI and compatibility schemes; resource kind and I/O intent (`overwrite`, `append`, `merge`, `readonly`, `random`) |
| `TimeUnit`, `Timezone`; `UnionMode`, `EdgeAlgorithm` | Temporal resolution and zone; union layout and geospatial edge model |
| `Enum` | Vocabulary, spelling, and compact member index; text and host projections emit the spelling |
| `TypedScalar` | One value and one datatype checked against each other, one alias per datatype; Rust only |
| Views | `as_integer`, `as_float`, `as_decimal`, `as_temporal` keep total equality, ordering, and hash semantics |

## Use

Every enum spells itself the same way at each boundary.

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

An `Enum` scalar keeps the member's kind, spelling, and ordinal across the bindings.

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

Units select date and time widths, duration the narrowest fitting count, decimal the coefficient; datetime stays 64-bit like Arrow timestamps.
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

Exact constructors remain in Rust when a physical Arrow identity is required; the width rules live under [Numeric & temporal](numeric.md).

## TypedScalar

`TypedScalar` pairs one value with one datatype, with one alias per datatype for a caller who knows which is coming.
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

Without a schema, `Scalar` exposes the `Field` the core already inferred: `value` for a scalar, `item` for a sequence, `row` for named record rows.

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

The markers match a [typed field](field.md); the Arrow projection lives under [Scalars](../arrow/scalars.md) and the parsers under [Text](../text/index.md).

## Edges

- `IOMode::ReadOnly` or `IOMode::Random` at a write entry point -> refused; only the three write modes write.
- Empty or positional rows -> ambiguous; `inferred_struct_field` needs a declared `Field`.
- `Int64Scalar::new(Scalar::from("seven"))` -> `Err`; the value must match the datatype.
- Duration count above `i32::MAX` -> 64-bit; the narrowest fitting width is chosen.
- `MimeType::PUFFIN` -> `application/vnd.apache.puffin`, `.puffin`, `PFA1` magic; the Puffin specification assigns no MIME name.
- Geospatial value across a binding -> plain WKB bytes; `TypedScalar` and the `wkb` reader stay [Rust only](geospatial.md).
- [ASCII](ascii.md) bases in `yggdryl.enums` -> [Python only](../extensions/python.md); the declaration they build is the shared `AsciiEnum`.
- Field inference -> Rust only; neither binding reimplements it.

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

Release measurements on Windows x86_64, AMD Ryzen 5 150, rustc 1.96.1, CPython 3.12.13, and Node 24.18.0 (2026-08-24); no Node scalar benchmark is checked in.

| Rust | 28.9 ns | 2.59 ns | 5.01 ns | 3.94 ns |
| Python | 200 ns | 98.8 ns | 100 ns | 73.7 ns |
| JavaScript | 3 us | 2 us | 2 us | 1 us |

Regenerate with `cargo bench --bench datatype --all-features -- "value/enum_"`,
`python benchmarks/scalars.py --iterations 10000`, and

Field inference is Rust only; the same host with rustc 1.96.1 (2026-08-23) gave these Criterion point estimates.

| inferred field | estimate |
| --- | ---: |
| scalar | 80.1 ns |
| array | 240 ns |
| one-record Struct | 985 ns |

```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(stable_hash_|from_float32|family_constructors|as_|temporal_|enum_|infer_|record_field_update|json_|checked_)'
python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
```
