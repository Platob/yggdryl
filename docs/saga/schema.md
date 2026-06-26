# Schema

A `Schema` is an ordered list of [`Field`](field.md)s plus string metadata — the
header shared by every [`Frame`](frame.md). It mirrors `arrow_schema::Schema`;
metadata is kept in an ordered map so rendering and serialisation are stable.

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## Build & query

=== "Rust"

    ```rust
    use yggdryl_saga::{Field, PrimitiveType, Schema};

    let schema = Schema::new(vec![
        Field::new("id", PrimitiveType::Int64.into(), false),
        Field::new("name", PrimitiveType::Utf8.into(), true),
    ]);
    assert_eq!(schema.len(), 2);
    assert_eq!(schema.names(), ["id", "name"]);
    assert_eq!(schema.index_of("name"), Some(1));
    ```

## The string grammar

`from_str` parses a comma-separated list of `name: type` fields (each as
`Field::from_str`); the empty string is the empty schema.

=== "Rust"

    ```rust
    use yggdryl_saga::Schema;

    let s = "ts: timestamp(ns, UTC) not null, px: float64";
    let schema = Schema::from_str(s).unwrap();
    assert_eq!(schema.to_str(), s);
    ```

## Convert to/from Arrow

Under the `arrow` feature, `to_arrow` / `from_arrow` carry fields **and** metadata
across the boundary.

=== "Rust"

    ```rust
    use yggdryl_saga::Schema;

    let schema = Schema::from_str("id: int64 not null, px: float64").unwrap();
    let arrow = schema.to_arrow();                 // arrow_schema::Schema
    assert_eq!(Schema::from_arrow(&arrow), schema);
    ```

## Next

- [Frame](frame.md) — the trait every table backing satisfies
- [Column](column.md) — the trait every column backing satisfies
