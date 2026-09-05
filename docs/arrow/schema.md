# Schema

A non-null Struct root projected to an Arrow `Schema` and back, in process or across the C Data Interface.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Field::into_arrow_schema`, `Field::into_arrow_exchange_schema`, `Field::from_arrow_schema` |
| Validates | Bounded, non-nullable Struct root; refused, never coerced |
| Metadata | Root metadata becomes schema metadata and comes back |
| Sidecar | `yggdryl:ipc:dictionary-ids` = `v1;<path>=<id>` per non-zero ID; transport only |
| Errors | `Error::IncompatibleSchema` (root), `Error::Core(InvalidMetadataValue)` (sidecar) |
| Feature flag | `arrow` (default) |
| Bindings | Rust; [Python](../extensions/python.md) `Field.from_arrow_schema(schema, name="row")`, `Field.into_arrow_schema()`; JavaScript none |
| Per-field | `Field::into_arrow`, `DataType::into_arrow`: [Field](../types/field.md), [DataType](../types/datatype.md) |

## Use

Rust only.

=== "Rust"

    ```rust
    use arrow_schema::{Schema, ffi::FFI_ArrowSchema};
    use yggdryl::{DataType, Field};

    let mut symbol = DataType::dictionary(DataType::Int16, DataType::Utf8)?
        .nullable_field("symbol");
    symbol.set_dictionary_options(-7, true)?;

    let schema = Field::from_parts(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            symbol,
        ])?,
        false,
        [("owner", "trading")],
    )?;

    // Root metadata becomes Arrow schema metadata, and comes back.
    let projected = schema.clone().into_arrow_exchange_schema()?;
    assert_eq!(projected.fields().len(), 2);
    assert_eq!(projected.metadata().get("owner").map(String::as_str), Some("trading"));
    assert_eq!(Field::from_arrow_schema("row", &projected)?, schema);

    // Arrow's C schema has no dictionary-ID slot.  This is the same boundary
    // PyArrow uses: the ID becomes zero, while the sidecar metadata survives.
    assert_eq!(
        projected
            .metadata()
            .get("yggdryl:ipc:dictionary-ids")
            .map(String::as_str),
        Some("v1;1=-7"),
    );
    let ffi = FFI_ArrowSchema::try_from(&projected)?;
    let crossed = Schema::try_from(&ffi)?;
    #[allow(deprecated)]
    {
        assert_eq!(crossed.field(1).dict_id(), Some(0));
    }
    let restored = Field::from_arrow_schema("row", &crossed)?;
    assert_eq!(restored, schema);
    assert!(!restored.has_metadata("yggdryl:ipc:dictionary-ids"));

    // Field::into_arrow_schema is the shared in-process projection. It retains the ID
    // on Arrow's Field directly and needs no transport sidecar.
    let in_process = schema.clone().into_arrow_schema()?;
    #[allow(deprecated)]
    {
        assert_eq!(in_process.field(1).dict_id(), Some(-7));
    }

    // A root that is not a non-null Struct is refused, not coerced.
    assert!(Field::new("row", DataType::Int64, false)
        .into_arrow_exchange_schema()
        .is_err());
    assert!(schema.with_nullable(true).into_arrow_exchange_schema().is_err());
    ```

## The three projections

| Method | Returns | Dictionary ID | Sidecar |
| --- | --- | --- | --- |
| `Field::into_arrow_schema` | `SchemaRef` | Kept on Arrow's `Field` | None |
| `Field::into_arrow_exchange_schema` | Owned `Schema` | Zeroed across the C interface | Added per non-zero ID |
| `Field::from_arrow_schema` | `Field` | Restored from the sidecar | Validated, then stripped |

## Edges

- Root not a Struct, nullable, or unbounded -> `Error::IncompatibleSchema`.
- Caller-set root metadata under the sidecar key -> `into_arrow_exchange_schema` refuses.
- Sidecar malformed, naming a non-dictionary field, or conflicting with a non-zero Arrow ID -> `from_arrow_schema` refuses.
- `FFI_ArrowSchema` round trip -> `dict_id()` reads `Some(0)`; the sidecar restores it.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test types field::arrow::
    cargo bench -p yggdryl --bench types -- arrow/struct_field
    # Wider context: per-field and per-datatype projections of the same group.
    cargo bench -p yggdryl --bench types -- arrow/
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_field_classes_arrow.py
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    ```

## Performance

The `types` Criterion target times only the three Struct-root methods over one nested fixture built outside the timer; no record batch is allocated. One local Windows x86_64 release run, Criterion point estimates; regenerate on the deployment host.

| operation | estimate | 95% interval |
| --- | ---: | ---: |
| `Field::into_arrow_schema` | 1.48 us | 1.14-2.27 us |
| `Field::into_arrow_exchange_schema` | 1.81 us | 1.71-1.95 us |
| `Field::from_arrow_schema` | 2.59 us | 2.37-2.81 us |

```bash
cargo bench -p yggdryl --bench types -- arrow/struct_field --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
```
