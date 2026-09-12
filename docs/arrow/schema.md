# Schema

A non-null Struct root projected to an Arrow `Schema` and back, in process or across the C Data Interface.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Field::into_arrow_schema`, `Field::into_arrow_exchange_schema`, `Field::from_arrow_schema` |
| Validates | Bounded, non-nullable Struct root; refused, never coerced |
| Metadata | Root metadata becomes schema metadata and comes back |
| Sidecar | `yggdryl:ipc:dictionary-ids` = `v1;<path>=<id>` per non-zero ID, keyed by deterministic numeric field paths; transport only |
| Errors | `Error::IncompatibleSchema` (root), `Error::Core(InvalidMetadataValue)` (sidecar) |
| Feature flag | `arrow` (default) |
| Bindings | Rust; [Python](../extensions/python.md) `Field.from_arrow_schema(schema, name="row")`, `Field.into_arrow_schema()`; JavaScript none |
| Per-field | `Field::into_arrow`, `Field::from_arrow`, `DataType::into_arrow`: [Field](../types/field.md), [DataType](../types/datatype.md) |

## Use

Rust only.

=== "Rust"

    ```rust
    use arrow_schema::{Schema, ffi::FFI_ArrowSchema};
    use yggdryl::{DataType, Field};

    let mut symbol = DataType::dictionary(DataType::Int16, DataType::utf8())?
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

## Strings and bytes

Arrow declares a layout and, for its string layouts, UTF-8; it declares no charset and no length bound. A [string](../types/text.md) rides Arrow's text storage (`Utf8`, `LargeUtf8`, `Utf8View`) when its charset is UTF-8 or US-ASCII, the matching binary storage in every other charset, and `FixedSizeBinary(width)` on the fixed layout. Plain `utf8`, `large_utf8` and `utf8_view` cross bare; the `yggdryl.string` extension document (`{"layout":..,"charset":..,"fixed"|"max":n}`) rides beside the storage only when Arrow cannot say what the string declares - a charset other than UTF-8, a bound, or the large view layout, which Arrow projects onto its one view. Bytes are their own layout in Arrow, so `yggdryl.bytes` rides beside `binary`, `large_binary` or `binary_view` only for a maximum; a fixed width is the storage itself. A document over a storage it does not describe imports as that storage.

JavaScript has no field projection; `Field.fromArrow` reads a datatype expression.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    // Arrow's own string crosses bare.
    let plain = DataType::utf8().required_field("symbol").into_arrow()?;
    assert_eq!(plain.data_type(), &ArrowDataType::Utf8);
    assert!(plain.metadata().is_empty());

    // US-ASCII is UTF-8 text with a narrower repertoire: text storage, plus the
    // document saying so.
    let ascii = DataType::ascii().required_field("venue").into_arrow()?;
    assert_eq!(ascii.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        ascii.metadata().get("ARROW:extension:name").map(String::as_str),
        Some("yggdryl.string"),
    );
    assert_eq!(
        ascii.metadata().get("ARROW:extension:metadata").map(String::as_str),
        Some(r#"{"layout":"string","charset":"us-ascii"}"#),
    );

    // Another charset is not UTF-8, so its bytes ride binary storage.
    let legacy = "string(windows-1252,8)"
        .parse::<DataType>()?
        .required_field("name")
        .into_arrow()?;
    assert_eq!(legacy.data_type(), &ArrowDataType::Binary);
    assert_eq!(
        legacy.metadata().get("ARROW:extension:metadata").map(String::as_str),
        Some(r#"{"layout":"string","charset":"windows-1252","max":8}"#),
    );

    // A fixed width is Arrow's fixed binary; a byte maximum is the one thing
    // bytes need a document for.
    let fixed = DataType::fixed_ascii(4)?.required_field("code").into_arrow()?;
    assert_eq!(fixed.data_type(), &ArrowDataType::FixedSizeBinary(4));
    let bounded = "binary(16)"
        .parse::<DataType>()?
        .required_field("blob")
        .into_arrow()?;
    assert_eq!(bounded.data_type(), &ArrowDataType::Binary);
    assert_eq!(
        bounded.metadata().get("ARROW:extension:name").map(String::as_str),
        Some("yggdryl.bytes"),
    );
    assert!(DataType::fixed_size_binary(16)?
        .required_field("key")
        .into_arrow()?
        .metadata()
        .is_empty());

    // Every projection reads back as the field that wrote it.
    for field in [
        DataType::utf8().required_field("symbol"),
        DataType::ascii().required_field("venue"),
        DataType::fixed_ascii(4)?.required_field("code"),
        DataType::fixed_size_binary(16)?.required_field("key"),
    ] {
        assert_eq!(Field::from_arrow(&field.clone().into_arrow()?)?, field);
    }
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    # Arrow's own string crosses bare.
    plain = Field("symbol", DataType.utf8(), nullable=False).into_arrow()
    assert plain.type == pa.string()
    assert plain.metadata is None

    # US-ASCII is UTF-8 text with a narrower repertoire: text storage, plus the
    # document saying so.
    ascii = Field("venue", DataType.ascii(), nullable=False).into_arrow()
    assert ascii.type == pa.string()
    assert ascii.metadata[b"ARROW:extension:name"] == b"yggdryl.string"
    assert ascii.metadata[b"ARROW:extension:metadata"] == b'{"layout":"string","charset":"us-ascii"}'

    # Another charset is not UTF-8, so its bytes ride binary storage.
    legacy = Field("name", DataType.string("string", "windows-1252", 8), nullable=False).into_arrow()
    assert legacy.type == pa.binary()
    assert legacy.metadata[b"ARROW:extension:metadata"] == b'{"layout":"string","charset":"windows-1252","max":8}'

    # A fixed width is Arrow's fixed binary; a byte maximum is the one thing
    # bytes need a document for.
    assert Field("code", DataType.fixed_ascii(4)).into_arrow().type == pa.binary(4)
    bounded = Field("blob", DataType.bytes("binary", 16)).into_arrow()
    assert bounded.type == pa.binary()
    assert bounded.metadata[b"ARROW:extension:name"] == b"yggdryl.bytes"
    assert Field("key", DataType.fixed_size_binary(16)).into_arrow().metadata is None

    # Every projection reads back as the field that wrote it.
    for field in (
        Field("symbol", DataType.utf8()),
        Field("venue", DataType.ascii()),
        Field("code", DataType.fixed_ascii(4)),
        Field("key", DataType.fixed_size_binary(16)),
    ):
        assert Field.from_arrow(field.into_arrow()) == field
    ```

## Edges

- Root not a Struct, nullable, or unbounded -> `Error::IncompatibleSchema`.
- Caller-set root metadata under the sidecar key -> `into_arrow_exchange_schema` refuses.
- Sidecar malformed, naming a non-dictionary field, or conflicting with a non-zero Arrow ID -> `from_arrow_schema` refuses.
- C Data Interface -> dictionary ordering crosses, IDs do not: `dict_id()` reads `Some(0)`, the sidecar restores it.

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
