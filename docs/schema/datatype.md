# DataType

`DataType` is the logical type of a value — the heart of `yggdryl-schema`, a compact
**Arrow-compatible** type system built to back a dataframe. It has three
[categories](#categories-physical-attributes) — **primitive**, **logical**,
**nested** — plus an `any` wildcard. The fixed-width numerics are **concrete types**
(`int8` … `uint64`, `float16` / `float32` / `float64`, `decimal32` … `decimal256`),
each backed by a native Rust storage type (`i8` / `f32` / `i128` / … — and the
created `f16` / `i256` where Rust has no builtin); the variable-width string / binary /
list still carry their offset/layout as uniform attributes (`large` / `view`), and all
strings are one `Varchar` with a charset.

## Construct

Parse a canonical string, or use a named constructor.

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.DataType("int64") == yggdryl.DataType.int(64)
    assert yggdryl.DataType.int(8, signed=False) == yggdryl.DataType("uint8")
    assert yggdryl.DataType.varchar() == yggdryl.DataType("string")
    yggdryl.DataType.timestamp("us", "UTC")          # timestamp[us, UTC]
    yggdryl.DataType.struct_([
        yggdryl.Field("id", yggdryl.DataType.int(64), nullable=False),
        yggdryl.Field("name", yggdryl.DataType.varchar()),
    ])
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    yggdryl.DataType.int(64).equals(yggdryl.DataType.fromStr("int64")); // true
    yggdryl.DataType.int(8, false).toString();                          // "uint8"
    yggdryl.DataType.timestamp("us", "UTC");
    yggdryl.DataType.struct([
      new yggdryl.Field("id", yggdryl.DataType.int(64), false),
      new yggdryl.Field("name", yggdryl.DataType.varchar()),
    ]);
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};
    use yggdryl_core::{TimeUnit, Timezone};

    assert_eq!(DataType::from_str("int64")?, DataType::int(64, true));
    let _ = DataType::timestamp(TimeUnit::Microsecond, Some(Timezone::from_str("UTC")?));
    let _ = DataType::struct_(vec![Field::new("id", DataType::int(64, true), false)]);
    ```

!!! tip "SQL & Hive forms"
    `from_str` also accepts common **SQL** and **Hive/Spark** spellings, so you can
    paste a DDL type: `BIGINT`, `INTEGER`, `VARCHAR(255)`, `CHAR(10)`, `DOUBLE
    PRECISION`, `DECIMAL(10,2)`, `TIMESTAMP WITH TIME ZONE`, `UUID`, `JSON`, `BSON`,
    and the `( )` / `< >` bracket styles — `array<int>`, `struct<a: int, b: string>`,
    `map<string, int>`. (Per SQL, a bare `int`/`integer` is 32-bit and `bigint` is
    64-bit; `varchar(n)` stays variable-length while `char(n)` is fixed.) Integers are
    the **fixed** widths only — `int8`/`int16`/`int32`/`int64` and their `uint`
    counterparts — so a non-standard width like `int24` is not a type.

## Categories & physical attributes

`category` is `"primitive"` / `"logical"` / `"nested"` / `"any"`. The physical
layout is read uniformly: `byte_size` (bytes for byte-aligned fixed-width types,
else null), `is_large`, `is_view`, `is_fixed_size`, and `charset` for strings.

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.DataType.int(32).category == "primitive"
    assert yggdryl.DataType.date().category == "logical"
    assert yggdryl.DataType.struct_([]).category == "nested"
    assert yggdryl.DataType.int(32).byte_size == 4
    assert yggdryl.DataType.boolean().byte_size is None   # sub-byte
    assert yggdryl.DataType.varchar().byte_size is None
    assert yggdryl.DataType.varchar(large=True).is_large
    assert yggdryl.DataType.varchar(charset="latin1").charset == "latin1"
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    yggdryl.DataType.int(32).category;       // "primitive"
    yggdryl.DataType.date().category;        // "logical"
    yggdryl.DataType.int(32).byteSize;       // 4
    yggdryl.DataType.varchar().byteSize;     // null
    yggdryl.DataType.varchar(undefined, true).isLarge; // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, TypeCategory};

    assert_eq!(DataType::int(32, true).category(), TypeCategory::Primitive);
    assert_eq!(DataType::int(32, true).byte_size(), Some(4));
    assert_eq!(DataType::varchar().byte_size(), None);
    ```

## Fixed numerics, native storage & JSON/BSON

The fixed-width numerics are concrete types with **explicit constructors** — `int8()`
… `uint64()`, `float16()` / `float32()` / `float64()`, `decimal32()` … `decimal256()`
— while `int(bits, signed)` / `float(bits)` / `decimal(precision, scale, bits)` are the
width builders (`integer()` / `floating()` default to `int64` / `float64`; a
non-standard width rounds up to the next fixed one). Each names its **native Rust
storage type** via `name` — a builtin (`i8` / `f32` / `i128` / …) or the type created
where Rust has none (`f16` for `float16`, `i256` for `decimal256`). In Rust each is a
struct generic over that storage type, defaulting to the natural one (`Int64<i64>`,
`Float16<f16>`, `Decimal256<i256>`). The numeric types share the **`Numeric`**
interface (a common `signed`). `Json` (string-backed) and `Bson` (binary-backed) are
logical types. Strings and binaries can be fixed- or variable-length (`is_fixed_size`).

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D("int8") == D.int8() == D.int(8)
    assert D.int() == D.int(64) and D.integer() == D.int(64)   # default width
    assert D.int(24) == D.int32()                              # rounds up to a fixed width
    assert D.int32().name == "i32"                             # native Rust storage type
    assert D.float16().name == "f16"                           # created half float
    assert D.decimal256(76, 0).name == "i256"                  # created 256-bit int
    assert D.int(32, signed=False).signed is False             # Numeric interface
    assert D.float(64).signed is True
    assert D("json").is_json() and D("bson").is_bson()
    assert D("char[10]").is_fixed_size and not D.varchar().is_fixed_size
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.fromStr("int8").equals(D.int8());                        // true
    D.int().equals(D.int(64));                                 // default width
    D.int(24).equals(D.int32());                               // rounds up to a fixed width
    D.int32().name;                                            // "i32" (native Rust storage)
    D.float16().name;                                          // "f16" (created half float)
    D.decimal256(76, 0).name;                                  // "i256" (created 256-bit int)
    D.int(32, false).signed;                                   // false (Numeric)
    D.float(64).signed;                                        // true
    D.fromStr("char[10]").isFixedSize;                         // true
    D.varchar().isFixedSize;                                   // false
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Int32, Decimal128, f16, i256};

    use yggdryl_schema::Numeric;
    assert_eq!(DataType::from_str("int8")?, DataType::int8());
    assert_eq!(DataType::integer(), DataType::int64());
    assert_eq!(DataType::int(24, true), DataType::int32());      // rounds up to a fixed width
    assert_eq!(DataType::int32().name(), Some("i32"));          // native Rust storage type
    // Each fixed type is a struct generic over its native storage (default = natural).
    assert_eq!(DataType::from(Int32::<i32>::new()), DataType::int32());
    assert_eq!(DataType::from(Decimal128::new(10, 2)), DataType::decimal(10, 2));
    assert_eq!(DataType::int(32, false).signed(), Some(false));  // Numeric interface
    assert_eq!(DataType::int32().to_string(), "int32");          // canonical via Display
    // the two native types Rust has no builtin for, created in `fixed`:
    assert_eq!(f16::from_f32(0.5).to_f32(), 0.5);
    assert_eq!(i256::from_i128(-5).to_str(), "-5");
    ```

## Type checks

Cheap predicates for routing values: `is_numeric`, `is_integer`,
`is_signed_integer`, `is_floating`, `is_string`, `is_temporal`, `is_decimal`,
`is_json`, `is_bson`, `is_nested`, `is_struct`, `is_fixed_size`, …

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.DataType.int(32).is_signed_integer()
    assert yggdryl.DataType.float(32).is_numeric()
    assert not yggdryl.DataType.decimal(10, 2).is_numeric()   # decimals are logical
    assert yggdryl.DataType.timestamp("s").is_temporal()
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    yggdryl.DataType.int(32).isSignedInteger(); // true
    yggdryl.DataType.float(32).isNumeric();     // true
    yggdryl.DataType.timestamp("s").isTemporal(); // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::DataType;

    assert!(DataType::int(32, true).is_signed_integer());
    assert!(DataType::float(32).is_numeric());
    assert!(DataType::date().is_temporal());
    ```

## Coercion & merge

`can_cast_to` is a broad cast-feasibility check; `common_type` is the
type-promotion lattice (integer widening, int→float, decimal growth, string
widening, struct field-union); `merge` applies a strategy when unifying a column's
type across batches — `"strict"` (must match), `"promote"` (widen, else error) or
`"permissive"` (widen, else fall back to `any`).

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D.int(8).common_type(D.int(32)) == D.int(32)
    assert D.int(32).common_type(D.float(32)) == D.float(64)
    assert D.int(32).common_type(D.varchar()) is None
    assert D.int(8).merge(D.int(64), "promote") == D.int(64)
    assert D.int(8).merge(D.varchar(), "permissive") == D.any()
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.int(8).commonType(D.int(32)).equals(D.int(32));       // true
    D.int(32).commonType(D.float(32)).equals(D.float(64));  // true
    D.int(8).merge(D.int(64), "promote").equals(D.int(64)); // true
    D.int(8).merge(D.varchar(), "permissive").equals(D.any()); // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, MergeStrategy};

    assert_eq!(DataType::int(8, true).common_type(&DataType::int(32, true)), Some(DataType::int(32, true)));
    assert_eq!(DataType::int(8, true).merge(&DataType::int(64, true), MergeStrategy::Promote)?, DataType::int(64, true));
    ```

## Serialize

Every type round-trips through a string, a component map, JSON and bytes, and is
hashable (`pickle` in Python, `JSON.stringify` in Node, `serde` in Rust). In Rust,
the `arrow` feature adds `to_arrow` / `from_arrow` conversion to `arrow-schema`. The
mapping is structural and near-total — a few attributes the simplified model does not
carry are normalised rather than preserved on the round-trip: a non-UTF-8 charset
maps to UTF-8, a union's type ids are reassigned `0, 1, …`, a map's key/value
entry-field nullability follows the Arrow convention, and an unrecognised Arrow
timestamp timezone falls back to UTC (with a `warn` log).

=== "Python"

    ```python
    import yggdryl, pickle
    dt = yggdryl.DataType.struct_([yggdryl.Field("id", yggdryl.DataType.int(64))])
    assert yggdryl.DataType.from_json(dt.to_json()) == dt
    assert pickle.loads(pickle.dumps(dt)) == dt
    ```

=== "Node"

    ```javascript
    const { DataType, Field } = require("yggdryl");
    const dt = DataType.struct([new Field("id", DataType.int(64))]);
    DataType.fromJSON(dt.toJSON()).equals(dt); // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};
    let dt = DataType::struct_(vec![Field::new("id", DataType::int(64, true), true)]);
    let _arrow = dt.to_arrow()?;     // arrow_schema::DataType (the `arrow` feature)
    ```

## Next

- [Field](field.md) — naming a `DataType`, building a schema, the graph
- Back to [Getting started](../getting-started.md)
