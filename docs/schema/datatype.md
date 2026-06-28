# DataType

`DataType` is the logical type of a value — the heart of `yggdryl-schema`. It is a
small, three-way scaffold: every type is exactly one of three **categories** —
[**primitive**](#primitive-types), [**logical**](#logical-types) or
[**nested**](#nested-types) — and every type carries two universal accessors, a
stable `type_id` (a `u8` [`DataTypeId`](#the-type_id-registry)) and a `name`.

## Construct

Build a type with a named constructor. The primitive widths are concrete
(`int8` … `uint64`, `float16` / `float32` / `float64`); the logical and nested types
carry their parameters (a decimal's precision/scale, a list's element field, …).

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    D.int32()                              # a primitive
    D.decimal(10, 2)                       # a logical type
    D.timestamp("us", "UTC")
    D.struct_([                            # a nested type
        yggdryl.Field("id", D.int64()),
        yggdryl.Field("name", D.utf8()),
    ])
    ```

=== "Node"

    ```javascript
    const { DataType, Field } = require("yggdryl");

    DataType.int32();                      // a primitive
    DataType.decimal(10, 2);               // a logical type
    DataType.timestamp("us", "UTC");
    DataType.struct([                      // a nested type
      new Field("id", DataType.int64()),
      new Field("name", DataType.utf8()),
    ]);
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};
    use yggdryl_core::TimeUnit;

    let _ = DataType::int32();                              // a primitive
    let _ = DataType::decimal(10, 2);                       // a logical type
    let _ = DataType::timestamp(TimeUnit::Microsecond, None);
    let _ = DataType::struct_(vec![                         // a nested type
        Field::new("id", DataType::int64()),
        Field::new("name", DataType::utf8()),
    ]);
    ```

## The `type_id` registry

Every type carries a stable `u8` id (`DataTypeId`) and a `name`. The id is the single
registry the variants map onto; parameters live on the `DataType`, not the id.

| id | name | category | id | name | category |
| --- | --- | --- | --- | --- | --- |
| 0 | `null` | primitive | 15 | `decimal` | logical |
| 1 | `bool` | primitive | 16 | `date` | logical |
| 2–5 | `int8`…`int64` | primitive | 17 | `time` | logical |
| 6–9 | `uint8`…`uint64` | primitive | 18 | `timestamp` | logical |
| 10–12 | `float16`…`float64` | primitive | 19 | `duration` | logical |
| 13 | `utf8` | primitive | 20 | `interval` | logical |
| 14 | `binary` | primitive | 21–22 | `json` / `bson` | logical |
| | | | 23–28 | `list`…`run_end_encoded` | nested |

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D.int32().type_id == 4
    assert D.int32().name == "int32"
    assert D.int32().category == "primitive"
    assert D.boolean().name == "bool"
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.int32().typeId;    // 4
    D.int32().name;      // "int32"
    D.int32().category;  // "primitive"
    D.boolean().name;    // "bool"
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, DataTypeId, TypeCategory};

    assert_eq!(DataType::int32().type_id(), DataTypeId::Int32);
    assert_eq!(DataType::int32().type_id().as_u8(), 4);
    assert_eq!(DataType::int32().name(), "int32");
    assert_eq!(DataType::int32().category(), TypeCategory::Primitive);
    ```

## Primitive types

The fixed/variable-width scalars: `null`, `boolean`, the signed (`int8` … `int64`) and
unsigned (`uint8` … `uint64`) integers, the floats (`float16` / `float32` / `float64`),
`utf8` and `binary`. Each constructor is parameter-less.

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D.uint64().type_id == 9
    assert D.float64().category == "primitive"
    assert D.utf8().is_primitive()
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.uint64().typeId;        // 9
    D.float64().category;     // "primitive"
    D.utf8().isPrimitive();   // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::DataType;

    assert_eq!(DataType::uint64().type_id().as_u8(), 9);
    assert!(DataType::utf8().is_primitive());
    // The inner `PrimitiveType` answers width/sign questions.
    assert!(DataType::int32().as_primitive().unwrap().is_integer());
    assert!(DataType::float64().as_primitive().unwrap().is_float());
    ```

## Logical types

Richer meaning over a physical storage: `decimal(precision, scale)`, the temporal
family (`date`, `time(unit)`, `timestamp(unit, tz)`, `duration(unit)`,
`interval(unit)`) and the document types `json` (string-backed) / `bson`
(binary-backed). The temporal types reuse the core `TimeUnit` / `Timezone`; an interval
unit is `"year_month"` / `"day_time"` / `"month_day_nano"`. A decimal exposes its
`(precision, scale)` via `decimal_parts`.

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D.decimal(10, 2).category == "logical"
    assert D.decimal(10, 2).decimal_parts == (10, 2)
    assert D.decimal(10) == D.decimal(10, 0)          # scale defaults to 0
    assert D.utf8().decimal_parts is None
    assert D.interval("month_day_nano").name == "interval"
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.decimal(10, 2).category;          // "logical"
    D.decimal(10, 2).decimalParts;      // [10, 2]
    D.decimal(10).equals(D.decimal(10, 0)); // true (scale defaults to 0)
    D.utf8().decimalParts;              // null
    D.interval("month_day_nano").name;  // "interval"
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, IntervalUnit, LogicalType};

    assert_eq!(DataType::decimal(10, 2).decimal_parts(), Some((10, 2)));
    assert_eq!(DataType::utf8().decimal_parts(), None);
    assert_eq!(DataType::interval(IntervalUnit::MonthDayNano).name(), "interval");
    assert!(matches!(
        DataType::decimal(10, 2).as_logical(),
        Some(LogicalType::Decimal { .. })
    ));
    ```

## Nested types

Containers of other fields or types: `list(field)`, `struct(fields)`,
`map(key, value)`, `union(fields)`, `dictionary(key, value)` and
`run_end_encoded(run_ends, values)`. The field-bearing containers (list, struct, union)
expose their immediate children through `fields`; the key/value containers hold child
*types* and report no fields.

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    s = D.struct_([
        yggdryl.Field("a", D.int32()),
        yggdryl.Field("b", D.utf8()),
    ])
    assert s.is_nested()
    assert [f.name for f in s.fields()] == ["a", "b"]
    assert D.int32().fields() == []                    # scalars have no children
    assert D.list(yggdryl.Field("item", D.int32())).fields()[0].name == "item"
    ```

=== "Node"

    ```javascript
    const { DataType, Field } = require("yggdryl");

    const s = DataType.struct([
      new Field("a", DataType.int32()),
      new Field("b", DataType.utf8()),
    ]);
    s.isNested();                                  // true
    s.fields().map((f) => f.name);                 // ["a", "b"]
    DataType.int32().fields();                      // []
    DataType.list(new Field("item", DataType.int32())).fields()[0].name; // "item"
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let s = DataType::struct_(vec![
        Field::new("a", DataType::int32()),
        Field::new("b", DataType::utf8()),
    ]);
    assert!(s.is_nested());
    assert_eq!(s.fields().iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert!(DataType::int32().fields().is_empty());
    ```

## Equality & hashing

Every type is `==`-comparable and hashable, so it can key a set or map.

=== "Python"

    ```python
    import yggdryl
    D = yggdryl.DataType

    assert D.int64() == D.int64()
    assert D.int64() != D.int32()
    assert hash(D.int64()) == hash(D.int64())
    assert str(D.int32()) == "int32"
    assert {D.int32(), D.int32(), D.utf8()} == {D.int32(), D.utf8()}
    ```

=== "Node"

    ```javascript
    const D = require("yggdryl").DataType;

    D.int64().equals(D.int64());                 // true
    D.int64().hashCode() === D.int64().hashCode(); // true
    D.int32().toString();                        // "int32"
    ```

=== "Rust"

    ```rust
    use std::collections::HashSet;
    use yggdryl_schema::DataType;

    assert_eq!(DataType::int64(), DataType::int64());
    assert_ne!(DataType::int64(), DataType::int32());
    let set: HashSet<_> = [DataType::int32(), DataType::int32(), DataType::utf8()].into();
    assert_eq!(set.len(), 2);
    ```

## Next

- [Field](field.md) — naming a `DataType` to build a schema
- Back to [Getting started](../getting-started.md)
