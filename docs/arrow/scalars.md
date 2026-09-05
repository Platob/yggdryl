# Scalars

One value across the array boundary, both ways, under the materialization budgets.

## Contract

| Key | Value |
| --- | --- |
| Owns | `scalar_array`, `scalar_value`, `TypedScalar::into_arrow_array`, `from_arrow_array`, `try_from_arrow_array`, `arrow::StructScalar` |
| Validates | The exact `Field`: name, nullability, dictionary options, metadata, extension identity |
| Null rule | A non-nullable Field takes a logical null only as its datatype's canonical default |
| Copies | None; `StructScalar` column reads are slices |
| Limits | 1,000,000 expanded slots and 64 MiB fixed bytes, checked before allocation |
| Errors | `Error::IncompatibleSchema` (shape), `Error::PhysicalLimit` (budget), `Error::Allocation` (allocator) |
| Bindings | Rust; Python `DataType.arrow_scalar` and `Field.arrow_scalar`; JavaScript none |

## Use

Rust only.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array};
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, Field, Scalar};

    let field = Field::new("id", DataType::Int64, false);
    let array = scalar_array(&field, &Scalar::from(7_i64))?;
    assert_eq!(array.len(), 1);

    // The exact Field decodes the same one-row array back, unchanged.
    assert_eq!(scalar_value(&field, array.as_ref())?.as_i128(), Some(7));

    // A foreign array has to be one row of the Field's exact physical datatype.
    let two: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    assert!(scalar_value(&field, two.as_ref()).is_err());

    // And the value has to satisfy the Field, recursively.
    assert!(scalar_array(&field, &Scalar::Null).is_err());
    ```

## TypedScalar without a Field

A [`TypedScalar`](../types/scalar.md) projects through a synthetic non-nullable Field, with the same canonical-default exception.

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::types::Int64Scalar;
    use yggdryl::{DataType, TypedScalar, Scalar};

    let price = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))?;
    let array = price.clone().into_arrow_array()?;
    assert_eq!(array.len(), 1);
    assert_eq!(TypedScalar::from_arrow_array(DataType::Int64, array.as_ref())?, price);

    // The marker-narrowed decode checks the datatype at compile time too.
    let typed = Int64Scalar::try_from_arrow_array(DataType::Int64, array.as_ref())?;
    assert_eq!(typed.value(), &Scalar::from(7_i64));

    // A null projects only when the datatype's own default spells it...
    let nothing = TypedScalar::from_parts(DataType::Null, Scalar::Null)?;
    assert_eq!(nothing.into_arrow_array()?.logical_null_count(), 1);

    // ...an int64 null belongs to a nullable Field, so the pairing refuses it.
    let absent = TypedScalar::from_parts(DataType::Int64, Scalar::Null)?;
    assert!(absent.into_arrow_array().is_err());
    ```

## StructScalar

One present struct row with its root Field; `into_arrow_scalar` yields the `Datum` Arrow kernels take for one value.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Datum, Int64Array, StringArray, StructArray};
    use yggdryl::arrow::StructScalar;
    use yggdryl::{DataType, Field};

    let schema = Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])?,
        false,
    );

    let projected = schema.clone().into_arrow_schema()?;
    let array = StructArray::new(
        projected.fields().clone(),
        vec![
            Arc::new(Int64Array::from(vec![1])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("AAPL")])) as ArrayRef,
        ],
        None,
    );

    let row = StructScalar::from_parts(schema, array)?;
    assert_eq!(row.field().name(), "row");
    assert_eq!(row.field().field_len(), 2);

    // Children come back as zero-copy one-element slices, by position or by name.
    let (field, column) = row.entry(0).expect("first column");
    assert_eq!(field.name(), "id");
    assert_eq!(column.len(), 1);
    assert_eq!(row.get_by_name("symbol").map(|column| column.len()), Some(1));

    // Arrow's own scalar marker is a shallow clone away.
    let marker = row.into_arrow_scalar();
    let (inner, is_scalar) = marker.get();
    assert!(is_scalar);
    assert_eq!(inner.len(), 1);
    ```

## Materialization budgets

Totals are checked before allocation and cover validity bitmaps, offsets, union buffers, and values behind dictionary or run-end keys.

=== "Rust"

    ```rust
    use yggdryl::arrow::scalar_array;
    use yggdryl::{DataType, Field, Scalar};

    // One logical null, one million and one mandatory physical child slots.
    let items = Field::new(
        "items",
        DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 1_000_001)?,
        true,
    );
    let message = scalar_array(&items, &Scalar::Null).unwrap_err().to_string();
    assert!(message.contains("expanded slots"), "{message}");
    assert!(message.contains("expected at most 1000000"), "{message}");
    assert!(message.contains("got 1000001"), "{message}");

    // Fixed width is counted across siblings, not per column.
    let wide = DataType::from_fields([
        Field::new("left", DataType::fixed_size_binary(40 * 1024 * 1024)?, false),
        Field::new("right", DataType::fixed_size_binary(40 * 1024 * 1024)?, false),
    ])?;
    let message = scalar_array(&Field::new("wide", wide, true), &Scalar::Null)
        .unwrap_err()
        .to_string();
    assert!(message.contains("fixed bytes"), "{message}");
    assert!(message.contains("expected at most 67108864"), "{message}");
    ```

Only what is built is charged: a dense union allocates the selected member, a sparse union every child. Phase reservations end with the phase.

=== "Rust"

    ```rust
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, Field, UnionMode, Scalar};

    // The inactive branch is far past the byte budget, and is never visited.
    let dense = DataType::union(
        [
            (0, Field::new("selected", DataType::Int32, false)),
            (
                1,
                Field::new(
                    "inactive",
                    DataType::fixed_size_binary(64 * 1024 * 1024 + 1)?,
                    false,
                ),
            ),
        ],
        UnionMode::Dense,
    )?;

    let choice = Field::new("choice", dense, false);
    let chosen = Scalar::from_sequence([Scalar::from(0_i8), Scalar::from(11_i32)]);
    let array = scalar_array(&choice, &chosen)?;
    assert_eq!(scalar_value(&choice, array.as_ref())?, chosen);
    ```

The same accounting runs behind `ArrowCast`; see [Cast](../types/cast.md).

## Edges

- Array length other than 1, or a foreign datatype -> `Error::IncompatibleSchema`.
- Field datatype deeper than the bound -> schema error before Arrow's recursive projection, no stack exhaustion.
- `StructScalar::from_parts`: not one row, null root, or foreign columns -> `Error::IncompatibleSchema` naming both schemas.
- Allocator refusal or overflowing `rows * width` -> `Error::Allocation` with the `TryReserveError`.
- Dense union inactive branch past the budget -> never visited, no error.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib arrow::value::
    cargo test --features "parquet iceberg" -p yggdryl --lib types::typed::tests::arrow::
    cargo test --features "parquet iceberg" -p yggdryl --test types value_bounds::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_field.py -k arrow_scalar
    ```
