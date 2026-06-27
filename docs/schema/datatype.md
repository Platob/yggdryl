# DataType

`DataType` is the logical type of a value — the heart of `yggdryl-schema`, a compact
**Arrow-compatible** type system built to back a dataframe. It has three
[categories](#categories-physical-attributes) — **primitive**, **logical**,
**nested** — plus an `any` wildcard. Unlike Arrow's combinatorial variants, width /
offset / layout differences are uniform attributes (`bit_size` / `large` / `view`),
and all strings are one `Varchar` with a charset.

## Construct

Parse a canonical string, or use a named constructor.

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.DataType("int64") == yggdryl.DataType.int(64)
    assert yggdryl.DataType.int(8, signed=False) == yggdryl.DataType("uint8")
    assert yggdryl.DataType.varchar() == yggdryl.DataType("string")
    yggdryl.DataType.timestamp("us", "UTC")          # timestamp[us, UTC]
    yggdryl.DataType.struct([
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
    64-bit; `varchar(n)` stays variable-length while `char(n)` is fixed.) Integers
    take **any** width — `int24`, `uint128` — not just 8/16/32/64.

## Categories & physical attributes

`category` is `"primitive"` / `"logical"` / `"nested"` / `"any"`. The physical
layout is read uniformly: `bit_size` (bits for fixed-width types, else null),
`byte_size`, `is_large`, `is_view`, and `charset` for strings.

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.DataType.int(32).category == "primitive"
    assert yggdryl.DataType.date().category == "logical"
    assert yggdryl.DataType.struct_([]).category == "nested"
    assert yggdryl.DataType.int(32).bit_size == 32
    assert yggdryl.DataType.boolean().bit_size == 1
    assert yggdryl.DataType.varchar().bit_size is None
    assert yggdryl.DataType.varchar(large=True).is_large
    assert yggdryl.DataType.varchar(charset="latin1").charset == "latin1"
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    yggdryl.DataType.int(32).category;       // "primitive"
    yggdryl.DataType.date().category;        // "logical"
    yggdryl.DataType.int(32).bitSize;        // 32
    yggdryl.DataType.varchar().bitSize;      // null
    yggdryl.DataType.varchar(undefined, true).isLarge; // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, TypeCategory};

    assert_eq!(DataType::int(32, true).category(), TypeCategory::Primitive);
    assert_eq!(DataType::int(32, true).bit_size(), Some(32));
    assert_eq!(DataType::varchar().bit_size(), None);
    ```

## Integers, JSON/BSON & physical types

Integers take **any** bit width (not just 8/16/32/64): `integer()` is the default
`int64`, and `int_from_bytes` infers the width from a buffer's length. `Json`
(string-backed) and `Bson` (binary-backed) are logical types, and every logical type
reports its storage layout via `physical_type()`. Strings and binaries can be fixed-
or variable-length (`is_fixed_size`).

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D("int24") == D.int(24)
    assert D.int() == D.int(64) and D.integer() == D.int(64)   # default width
    assert D.int_from_bytes(bytes(4)) == D.int(32)             # 4 bytes -> int32
    assert D("json").physical_type() == D.varchar()            # logical -> physical
    assert D("bson").physical_type() == D.binary()
    assert D.date().physical_type() == D.int(32)
    assert D("char[10]").is_fixed_size and not D.varchar().is_fixed_size
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.fromStr("int24").equals(D.int(24));                      // true
    D.int().equals(D.int(64));                                 // default width
    D.intFromBytes(Buffer.alloc(4)).equals(D.int(32));         // 4 bytes -> int32
    D.json().physicalType().equals(D.varchar());               // logical -> physical
    D.bson().physicalType().equals(D.binary());
    D.fromStr("char[10]").isFixedSize;                         // true
    D.varchar().isFixedSize;                                   // false
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::DataType;

    assert_eq!(DataType::from_str("int24")?, DataType::int(24, true));
    assert_eq!(DataType::integer(), DataType::int(64, true));
    assert_eq!(DataType::int_from_bytes(&[0u8; 4], true), DataType::int(32, true));
    assert_eq!(DataType::json().physical_type(), DataType::varchar());
    assert_eq!(DataType::date().physical_type(), DataType::int(32, true));
    assert!(DataType::fixed_size_varchar(10).is_fixed_size());
    assert!(!DataType::varchar().is_fixed_size());
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
    dt = yggdryl.DataType.struct([yggdryl.Field("id", yggdryl.DataType.int(64))])
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
    # #[cfg(feature = "arrow")]
    let _arrow = dt.to_arrow()?;     // arrow_schema::DataType
    ```

## Next

- [Field](field.md) — naming a `DataType`, building a schema, the graph
- Back to [Getting started](../getting-started.md)
