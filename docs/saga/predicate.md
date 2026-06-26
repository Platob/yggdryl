# Predicate & expressions

The filtering layer: an `Expression` resolves to a [`DataType`](datatype.md) against
a [`Schema`](schema.md) and reports the columns it touches; a `Predicate` is the
boolean expression a [`Frame`](frame.md) filters with. Its `optimize` **types every
literal against the target column** so the filter can be pushed into typed storage.

This is where the dynamic [`Any`](datatype.md) type earns its keep: a freshly-written
filter literal (`Scalar::any("2024-01-01")`) is untyped until the predicate is
optimised against the schema, at which point it is cast to the column's type — a
string ISO date becomes a `timestamp`.

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## Scalars

A `Scalar` is one typed literal. It may start as the dynamic `any` type and be cast:

=== "Rust"

    ```rust
    use yggdryl_saga::{DataType, Scalar};

    let raw = Scalar::any("2024-01-01");       // type: any
    assert!(raw.data_type().is_any());

    let ts = DataType::from_str("timestamp(ns, UTC)").unwrap();
    let typed = raw.cast(&ts).unwrap();          // ISO string -> epoch nanos
    assert_eq!(typed.as_i64(), Some(19723 * 86_400 * 1_000_000_000));

    // Numbers, booleans and strings interconvert too.
    assert_eq!(Scalar::utf8("3.5").cast(&DataType::from_str("float64").unwrap()).unwrap().as_f64(), Some(3.5));
    ```

## Casting rules

`DataType::can_cast_to` decides what may convert: `any` ↔ everything, numbers ↔
booleans ↔ strings, and strings/integers ↔ the temporal types (the ISO-date →
`timestamp` path). Nested types only cast to themselves.

=== "Rust"

    ```rust
    use yggdryl_saga::DataType;

    let utf8 = DataType::from_str("utf8").unwrap();
    let ts = DataType::from_str("timestamp(ns, UTC)").unwrap();
    assert!(utf8.can_cast_to(&ts));
    assert!(DataType::Any.can_cast_to(&ts));
    ```

## Predicates

Build a `Predicate` from comparisons and null checks, combined with `and` / `or` /
`not`. `optimize(&schema)` casts each literal to its column's type and checks every
column exists:

=== "Rust"

    ```rust
    use yggdryl_saga::{Field, Predicate, Scalar, Schema};

    let schema = Schema::from_str("ts: timestamp(ns, UTC) not null, px: float64").unwrap();

    // col("ts") >= "2024-01-01"  AND  col("px") > 100
    let predicate = Predicate::ge("ts", Scalar::any("2024-01-01"))
        .and(Predicate::gt("px", Scalar::utf8("100")));

    let typed = predicate.optimize(&schema).unwrap();
    // Both literals are now typed to their columns (timestamp, float64) — ready to push down.
    ```

A frame applies this for you via [`Frame::filter_typed`](frame.md): it optimises the
predicate against the schema, then filters with the typed predicate.

## Expression nodes

`Expression` is the underlying trait; its leaves are `col(name)` (a column
reference, whose type is resolved from the schema) and `lit(scalar)` (a literal).
`Predicate` implements `Expression` too — it always yields `boolean`.

=== "Rust"

    ```rust
    use yggdryl_saga::{col, lit, Expression, Scalar, Schema};

    let schema = Schema::from_str("px: float64").unwrap();
    assert_eq!(col("px").data_type(&schema).unwrap().to_str(), "float64");
    assert!(lit(Scalar::int64(1)).columns().is_empty());
    ```

## Next

- [Frame](frame.md) — `filter` / `filter_typed` consume these predicates
- [DataType](datatype.md) — the `any` type and `can_cast_to`
