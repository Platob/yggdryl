# UUID

The 128-bit identifier, its spellings, and its `arrow.uuid` storage.

## Contract

| Key | Value |
| --- | --- |
| Spelling | `uuid`; kind `Uuid`; no parameters |
| Packed | `u128` of the sixteen storage bytes, big-endian |
| Canonical text | 36-character lowercase hyphenated |
| Accepts | hyphenated, 32-digit bare hex, either case, sixteen bytes |
| Default | the nil UUID |
| Storage | `FixedSizeBinary(16)`, extension `arrow.uuid` |
| Iceberg | [`uuid`](../media/iceberg/schema.md) maps onto it |
| Validator | one rule for [field](field.md) validation, Arrow ingest, every [cast](cast.md) tier |

## Use

Every spelling reads to the same bytes and writes back canonical.

=== "Rust"

    ```rust
    use arrow_array::{Array, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, DataTypeKind, Field, Scalar};

    // One 128-bit identifier, one canonical spelling, and no parameters.
    let uuid = DataType::uuid();
    assert_eq!(DataType::from_str("uuid")?, uuid);
    assert_eq!(uuid.to_string(), "uuid");
    assert_eq!(uuid.kind(), DataTypeKind::Uuid);

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let packed = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab_u128;
    assert_eq!(uuid.uuid_packed(text.as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(text.to_uppercase().as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(text.replace('-', "").as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(&packed.to_be_bytes())?, packed);
    assert_eq!(uuid.uuid_value(packed)?, text);

    // Storage is the canonical `arrow.uuid` extension over sixteen bytes, and
    // the value reads back spelled out.
    let id = Field::new("id", DataType::Uuid, false);
    let value = id.scalar(text)?;
    let stored = scalar_array(&id, &value)?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), packed.to_be_bytes());
    assert_eq!(scalar_value(&id, stored.as_ref())?, value);

    let arrow = id.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(16));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "arrow.uuid");
    assert_eq!(Field::from_arrow(&arrow)?, id);

    assert!(uuid.uuid_packed(b"not-a-uuid").is_err());
    ```

=== "Python"

    ```python
    import uuid

    import pyarrow as pa

    from yggdryl import DataType, Field

    # One 128-bit identifier, one canonical spelling, and no parameters.
    uuid_type = DataType("uuid")
    assert DataType("uuid") == uuid_type
    assert str(uuid_type) == "uuid"
    assert uuid_type.kind == "uuid"

    # The identity is the sixteen bytes; every spelling is a rendering of them.
    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    id = Field("id", uuid_type, nullable=False)
    assert id.arrow_scalar(text) == pa.scalar(packed.to_bytes(16, "big"), pa.binary(16))
    assert id.arrow_scalar(text.upper()) == id.arrow_scalar(text)
    assert id.arrow_scalar(packed.to_bytes(16, "big")) == id.arrow_scalar(text)
    assert id.default_pyvalue() == "00000000-0000-0000-0000-000000000000"

    # Storage is the canonical `arrow.uuid` extension, which PyArrow registers
    # itself, so a column of them reads back as `uuid.UUID`.
    arrow = id.into_arrow()
    assert arrow.type == pa.uuid()
    assert arrow.type.storage_type == pa.binary(16)
    assert Field.from_arrow(arrow) == id
    stored = id.cast_arrow_array(pa.array([text, text.upper()]))
    assert stored.to_pylist() == [uuid.UUID(text)] * 2

    # A recognized identifier column renders as its spelling.
    batch = pa.record_batch([stored], schema=pa.schema([arrow]))
    spelled = DataType.from_fields([Field("id", "utf8")])
    assert spelled.cast_arrow_batch(batch).column(0).to_pylist() == [text, text]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    // One 128-bit identifier, one canonical spelling, and no parameters.
    const uuid = new DataType('uuid')
    assert.ok(DataType.from('uuid').equals(uuid))
    assert.equal(uuid.toString(), 'uuid')
    assert.equal(uuid.kind, 'uuid')

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    const id = new Field('id', uuid, false)
    assert.equal(id.defaultJSValue(), '00000000-0000-0000-0000-000000000000')
    assert.equal(fields.uuid('id').dtype.id, 'uuid')
    assert.ok(Field.fromJSON(id.toJSON()).equals(id))
    ```

## Edges

- `not-a-uuid`, wrong digit count, misplaced hyphen -> `InvalidRecord` naming the accepted spellings.
- `uuid_packed` or `uuid_value` on another datatype -> `InvalidDataType`.
- Packed integer -> the same in every process, and what a stable hash hashes.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib types::tests::uuid
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k uuid
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="uuid" node/tests/types/datatype.test.js
    ```
