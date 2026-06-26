# DataType

`DataType` is the logical type of a column in **yggdryl-saga**. It mirrors
`arrow_schema::DataType` exactly, split — as Arrow's own types are — into three
families, each its own module:

- **`PrimitiveType`** — the flat, child-less scalars: `Null`, `Boolean`, the
  signed/unsigned integers, `Float16/32/64`, the binary/string buffers
  (`Binary`, `LargeBinary`, `BinaryView`, `FixedSizeBinary(n)`, `Utf8`,
  `LargeUtf8`, `Utf8View`).
- **`LogicalType`** — semantic types over a physical layout: `Date32`/`Date64`,
  `Time32`/`Time64`, `Timestamp` (with optional timezone), `Duration`,
  `Interval`, and `Decimal32/64/128/256`.
- **`NestedType`** — types carrying child `Field`s: `List` (and its `Large` /
  `View` variants), `FixedSizeList`, `Struct`, `Map`, `Union`, `Dictionary`,
  `RunEndEncoded`.

Plus one type outside the Arrow families:

- **`Any`** (`any` / `object`) — the **dynamic** type: a value whose concrete type
  is not yet known. It has no Arrow counterpart (it converts to `Null`); its job is
  to carry an untyped literal — a filter value written as a string — until a
  [`Frame`](frame.md) resolves the target column's type and casts it for pushdown
  (see [Predicate](predicate.md)). Every type `can_cast_to` and from `Any`.

The three Arrow families form a total, disjoint partition, so the Arrow bridge
(`to_arrow` / `from_arrow`, under the on-by-default `arrow` feature) is a lossless
bijection for every Arrow type. (`Any` is the one non-Arrow type — it maps to
`Null` one-way.)

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth. They will gain synced `Python` / `Node` tabs once the
    bindings land.

## Parse

`from_str` accepts any canonical type string, trying the primitive, logical and
nested families in turn. Names are recognised first, so a recognised name with bad
parameters is an *invalid* error (not *unknown*).

=== "Rust"

    ```rust
    use yggdryl_saga::DataType;

    assert!(DataType::from_str("int64").unwrap().is_primitive());
    assert!(DataType::from_str("timestamp(us, UTC)").unwrap().is_logical());
    assert!(DataType::from_str("list<item: int64>").unwrap().is_nested());

    // Aliases resolve to the canonical variant.
    assert_eq!(DataType::from_str("string").unwrap().to_str(), "utf8");
    ```

## Construct

Build directly from a family value; `From` makes the conversion implicit.

=== "Rust"

    ```rust
    use yggdryl_saga::{DataType, LogicalType, PrimitiveType, TimeUnit};

    let i = DataType::from(PrimitiveType::Int64);
    let t = DataType::from(LogicalType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())));
    assert!(i.is_numeric());
    ```

## The string grammar

Every type renders to a canonical string and parses back from it. Nested types
render each child with `Field::to_str`, so the round-trip is lossless (including
child names and nullability):

=== "Rust"

    ```rust
    use yggdryl_saga::DataType;

    let s = "struct<id: int64 not null, tags: list<item: utf8>>";
    let dt = DataType::from_str(s).unwrap();
    assert_eq!(dt.to_str(), s);
    assert_eq!(DataType::from_str(&dt.to_str()).unwrap(), dt);
    ```

| family | examples |
| --- | --- |
| primitive | `int64`, `uint8`, `float64`, `utf8`, `fixed_size_binary(16)` |
| logical | `date32`, `time64(us)`, `timestamp(ns, UTC)`, `decimal128(38, 10)` |
| nested | `list<item: int64>`, `struct<a: int64, b: utf8 not null>`, `map<entries: …>`, `dictionary<int32, utf8>` |

## Convert to/from Arrow

Under the `arrow` feature, `to_arrow` / `from_arrow` (and the `From` impls they
delegate to) cross the `arrow-schema` boundary at zero cost.

=== "Rust"

    ```rust
    use yggdryl_saga::DataType;

    let dt = DataType::from_str("list<item: struct<ts: timestamp(ns, UTC) not null, px: float64>>").unwrap();
    let arrow = dt.to_arrow();                       // arrow_schema::DataType
    assert_eq!(DataType::from_arrow(&arrow), dt);    // lossless round-trip
    ```

## Serialize

Under the `serde` feature every schema type derives `Serialize` / `Deserialize`
*structurally* (not as a lossy string), so metadata and child nullability survive a
round-trip through any serde format.

## Next

- [Field](field.md) — a named, nullable `DataType` with metadata
