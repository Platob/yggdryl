# Field

A `Field` is a named, nullable [`DataType`](datatype.md) with optional string
metadata — the header of a column, and the child element of every nested type. It
mirrors `arrow_schema::Field`.

Metadata is held in an ordered map (so rendering and serialisation are stable). The
string form carries name, type and nullability; use `serde` or the Arrow bridge to
preserve metadata too.

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## Construct

`new` takes the nullability flag; `nullable` / `required` are the readable
shorthands a CSV/Parquet schema inference reaches for.

=== "Rust"

    ```rust
    use yggdryl_saga::{DataType, Field, PrimitiveType};

    let f = Field::new("price", DataType::from(PrimitiveType::Float64), false);
    assert_eq!(f.name(), "price");
    assert!(!f.is_nullable());

    let a = Field::nullable("a", PrimitiveType::Int64.into());   // nullable
    let b = Field::required("b", PrimitiveType::Utf8.into());    // not null
    ```

## Update & cast

`with_name` / `with_data_type` / `with_nullable` / `with_metadata` /
`with_metadata_entry` each return a new value and never mutate the original.
`cast` re-types the field **validated** by [`DataType::can_cast_to`](predicate.md)
(keeping name, nullability and metadata) — the field-level mirror of
[`Column::cast`](column.md).

=== "Rust"

    ```rust
    use yggdryl_saga::{DataType, Field, PrimitiveType};

    let id = Field::new("id", PrimitiveType::Int64.into(), false)
        .with_metadata_entry("source", "csv");
    let key = id.clone().with_name("key").with_nullable(true);
    assert_eq!(key.name(), "key");
    assert_eq!(id.name(), "id"); // unchanged

    // Re-type a utf8 field to timestamp (metadata/nullability preserved).
    let ts = Field::new("ts", PrimitiveType::Utf8.into(), true)
        .cast(DataType::from_str("timestamp(ns, UTC)").unwrap())
        .unwrap();
    assert!(ts.data_type().is_temporal());
    ```

## The string grammar

`from_str` parses `name: type`, with an optional trailing `not null` marking the
field non-nullable (the default is nullable). The type after the `:` is parsed by
`DataType::from_str`, and the separator is found at the top level — so a `:` inside a
nested type is not mistaken for it.

=== "Rust"

    ```rust
    use yggdryl_saga::Field;

    let f = Field::from_str("ts: timestamp(ns, UTC) not null").unwrap();
    assert_eq!(f.name(), "ts");
    assert!(!f.is_nullable());
    assert_eq!(f.to_str(), "ts: timestamp(ns, UTC) not null");

    // The ':' inside the struct is not the field separator.
    let col = Field::from_str("col: struct<a: int64, b: utf8 not null>").unwrap();
    assert!(col.data_type().is_nested());
    ```

## Convert to/from Arrow

Under the `arrow` feature, `to_arrow` / `from_arrow` carry name, type, nullability
**and metadata** across the boundary.

=== "Rust"

    ```rust
    use std::collections::BTreeMap;
    use yggdryl_saga::{Field, PrimitiveType};

    let meta = BTreeMap::from([("unit".to_string(), "bps".to_string())]);
    let f = Field::new("spread", PrimitiveType::Float64.into(), false).with_metadata(meta);

    let arrow = f.to_arrow();                        // arrow_schema::Field
    assert_eq!(arrow.metadata().get("unit"), Some(&"bps".to_string()));
    assert_eq!(Field::from_arrow(&arrow), f);
    ```

## Next

- [DataType](datatype.md) — the logical type a field carries
