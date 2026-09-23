# UUID

One 128-bit identifier over sixteen fixed bytes, spelled as the canonical 36-character lowercase hyphenated text.

## Contract

| | |
| --- | --- |
| Owned | `DataType::Uuid` with `uuid()`, `uuid_packed` and `uuid_value`; the `UuidField` marker over `UuidType`; the `Uuid` value with `new`, `from_bytes`, `from_v7`, `from_v8`, `get`, `version`, `is_nil`, `into_bytes`, `render` and `TEXT_LEN`; the canonical `arrow.uuid` projection |
| Validated | one rule wherever a value arrives: sixteen bytes, 32 bare hexadecimal digits, or the 36-character hyphenated spelling, in either case - the same rule for [field](field.md) validation, Arrow ingest and every [cast](cast.md) tier |
| Lazy | nothing; the value is `Copy` and sixteen bytes wide, and `render` writes its spelling into the caller's slot |
| Cached | the [field](field.md)'s Arrow projection; the datatype takes no parameter and caches nothing |
| Refused | a spelling that is neither text nor sixteen bytes, a wrong digit count, a misplaced hyphen, `uuid_packed` or `uuid_value` on another datatype, a UUIDv7 instant outside `0..=281474976710655999` Unix microseconds, and a text bound the 36 characters outgrow |

## DataType

`uuid` takes no parameter: an identifier is sixteen bytes and nothing else,
whichever RFC 9562 version wrote it. `DataTypeId::Uuid` is `0x81`, the first
leaf in the `0x80` UUID family, and the datatype's fixed width is sixteen.

The packed integer is the identifier rather than a code for it: it is the
sixteen storage bytes read big-endian, so it is the same integer in every
process, orders exactly as the bytes do, and is what a stable hash hashes. It
is unsigned because all sixteen bytes carry identity. Iceberg's
[`uuid`](../media/index.md#iceberg) maps onto this datatype.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    // One 128-bit identifier, one canonical spelling, and no parameters.
    let uuid = DataType::uuid();
    assert_eq!(DataType::from_str("uuid")?, uuid);
    assert_eq!(uuid.to_string(), "uuid");
    assert_eq!(uuid.kind(), DataTypeKind::Uuid);
    assert_eq!(uuid.id(), DataTypeId::Uuid);
    assert_eq!(DataTypeId::Uuid.as_u8(), 0x81);
    assert_eq!(uuid.fixed_byte_width(), Some(16));

    // The identity is the sixteen bytes; every spelling is a rendering of them.
    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let packed = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab_u128;
    assert_eq!(uuid.uuid_packed(text.as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(text.to_uppercase().as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(text.replace('-', "").as_bytes())?, packed);
    assert_eq!(uuid.uuid_packed(&packed.to_be_bytes())?, packed);
    assert_eq!(uuid.uuid_value(packed)?, text);

    // Neither door answers for another datatype, and neither guesses.
    assert!(uuid.uuid_packed(b"not-a-uuid").is_err());
    assert!(DataType::utf8().uuid_packed(text.as_bytes()).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    # One 128-bit identifier, one canonical spelling, and no parameters.
    uuid_type = DataType("uuid")
    assert DataType("uuid") == uuid_type
    assert uuid_type.id == "uuid"
    assert uuid_type.kind == "uuid"
    assert str(uuid_type) == "uuid"
    assert uuid_type.fixed_byte_width == 16
    # A UUID is bytes with an identity, so it declares neither string nor byte
    # parameters.
    assert uuid_type.string_parameters is None
    assert uuid_type.bytes_parameters is None

    # The identity is the sixteen bytes; every spelling is a rendering of them.
    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    assert uuid_type.uuid_packed(text) == packed
    assert uuid_type.uuid_packed(text.replace("-", "")) == packed
    assert uuid_type.uuid_packed(packed.to_bytes(16, "big")) == packed
    assert uuid_type.uuid_value(packed) == text
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // One 128-bit identifier, one canonical spelling, and no parameters.
    const uuid = new DataType('uuid')
    assert.equal(uuid.id, 'uuid')
    assert.equal(uuid.kind, 'uuid')
    assert.equal(uuid.toString(), 'uuid')
    assert.equal(uuid.fixedByteWidth, 16)
    assert.equal(uuid.bytesParameters, null)
    assert.ok(DataType.from('uuid').equals(uuid))
    assert.ok(DataType.fromString(uuid.toString()).equals(uuid))
    ```

The packed reading is a Rust and Python door: JavaScript reads an identifier as
its spelling and has no `uuidPacked`.

## Field

`UuidField` is `FieldOf<UuidType>`: the datatype carries no parameter, so
`unit(name, nullable)` is the whole constructor. The bindings have one factory,
nullable unless the call says otherwise, and metadata rides beside the datatype
as on any other field. The field's canonical default is the nil identifier.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, Field, Scalar, Uuid, UuidField};

    // Nothing to pass: the datatype is the whole declaration.
    let id = UuidField::unit("id", false);
    assert_eq!(id.name(), "id");
    assert_eq!(id.dtype(), &DataType::Uuid);
    assert!(!id.is_nullable());

    // Widened, it is the column the root constructor builds.
    let root: Field = id.into_field();
    assert_eq!(root, Field::new("id", DataType::uuid(), false));
    assert!(UuidField::from_field(&root).is_some());

    // The canonical default is the nil identifier.
    assert_eq!(root.default_value()?, Scalar::Uuid(Uuid::new(0)));
    assert!(Uuid::new(0).is_nil());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    id = yggdryl.uuid("id", nullable=False)
    assert isinstance(id, Field)
    assert str(id.dtype) == "uuid"
    assert id.nullable is False
    assert id.default_scalar().as_py() == "00000000-0000-0000-0000-000000000000"

    # Nullable unless the call says otherwise; metadata rides beside it.
    tagged = yggdryl.uuid("id", metadata={"source": "feed"})
    assert tagged.nullable is True
    assert tagged.metadata["source"] == "feed"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const id = fields.uuid('id', { nullable: false })
    assert.ok(id instanceof Field)
    assert.equal(id.dtype.id, 'uuid')
    assert.equal(id.nullable, false)
    assert.equal(id.defaultJSValue(), '00000000-0000-0000-0000-000000000000')

    // The field is the one native Field, so it serializes and reads back.
    assert.ok(Field.fromJSON(id.toJSON()).equals(id))
    assert.equal(fields.uuid('id').nullable, true)
    ```

## Scalar

`Scalar::Uuid` carries a `Uuid`, which is the packed `u128` and nothing else.
`Uuid::new` takes that integer, `Uuid::from_bytes` takes any accepted spelling,
`get` and `into_bytes` answer the two storage readings, and `render` writes the
canonical spelling into a caller's `[u8; Uuid::TEXT_LEN]` without allocating -
`to_string` is the same characters when an owned string is what the caller
wants. In Python and JavaScript the value crosses as its spelling.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, Uuid};

    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let packed = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab_u128;

    // The value is the packed identifier; every spelling reads to it.
    let value = Uuid::new(packed);
    assert_eq!(Uuid::from_bytes(text.as_bytes())?, value);
    assert_eq!(Uuid::from_bytes(&value.into_bytes())?, value);
    assert_eq!(value.get(), packed);
    assert_eq!(value.to_string(), text);

    // The column reads every spelling into the same scalar.
    assert_eq!(DataType::uuid().scalar(text)?, Scalar::Uuid(value));
    assert_eq!(DataType::uuid().scalar(Scalar::Uuid(value))?, Scalar::Uuid(value));

    // The spelling goes into a caller's slot, so a writer that wants a `&str`
    // allocates nothing for it.
    let mut slot = [0_u8; Uuid::TEXT_LEN];
    assert_eq!(value.render(&mut slot), text);
    assert_eq!(Uuid::TEXT_LEN, 36);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    uuid_type = DataType("uuid")

    # The value crosses as its canonical spelling, whichever spelling went in.
    value = uuid_type.scalar(text)
    assert value.kind == "uuid"
    assert value.as_py() == text
    assert uuid_type.scalar(text.upper()) == value
    assert uuid_type.scalar(packed.to_bytes(16, "big")) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const text = '01912d68-783e-7c9a-b1f2-0123456789ab'
    const uuid = new DataType('uuid')

    // The value crosses as its canonical spelling, whichever spelling went in.
    const value = uuid.scalar(text)
    assert.equal(value.kind, 'uuid')
    assert.equal(value.asJs(), text)
    assert.ok(uuid.scalar(text.toUpperCase()).equals(value))
    assert.throws(() => uuid.scalar('not-a-uuid'))
    ```

## Arrow storage

| datatype | Arrow | extension | imports back as |
| --- | --- | --- | --- |
| `uuid` | `FixedSizeBinary(16)` | `arrow.uuid`, with an empty document | `uuid` |

The width is the one the canonical Arrow extension fixes, and sixteen bytes
against the 36 a spelling would take. PyArrow registers `arrow.uuid` itself, so
a column read there comes back as `uuid.UUID`.

=== "Rust"

    ```rust
    use arrow_array::{Array, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, Field};

    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let packed = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab_u128;

    // Storage is the sixteen bytes, and the value reads back spelled out.
    let id = Field::new("id", DataType::uuid(), false);
    let value = id.scalar(text)?;
    let stored = scalar_array(&id, &value)?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), packed.to_be_bytes());
    assert_eq!(scalar_value(&id, stored.as_ref())?, value);

    // The column is recognized by the canonical extension name, and imports back.
    let arrow = id.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(16));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "arrow.uuid");
    assert_eq!(Field::from_arrow_field(&arrow)?, id);
    ```

=== "Python"

    ```python
    import uuid

    import pyarrow as pa

    from yggdryl import Field

    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    id = Field("id", "uuid", nullable=False)

    # PyArrow registers `arrow.uuid` itself, so the projection is `pa.uuid()`.
    arrow = id.into_arrow()
    assert arrow.type == pa.uuid()
    assert arrow.type.storage_type == pa.binary(16)
    assert Field.from_arrow(arrow) == id

    # A cast into the type validates, and the column reads back as `uuid.UUID`.
    stored = id.cast_arrow_array(pa.array([text, text.upper()]))
    assert stored.to_pylist() == [uuid.UUID(text)] * 2
    assert stored.storage.to_pylist() == [packed.to_bytes(16, "big")] * 2
    ```

JavaScript casts an Arrow vector into the column the same way, through
`field.castArrowArray(values)`.

## Text and byte readings

A recognized identifier column reads into every [string](text/string.md)
datatype as the canonical spelling, under that datatype's own layout, charset
and bound, and into every [byte framing](text/bytes.md) as the sixteen bytes.
One [cast](cast.md) tier reads both directions.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
    use yggdryl::FieldValue as _;
    use yggdryl::{ArrowCastOptions, DataType, Field, StructType};

    let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let strict = || ArrowCastOptions::new().with_safe(false);
    let row = |field: Field| -> Field {
        Field::new(
            "row",
            DataType::from(StructType::from_fields([field]).unwrap()),
            false,
        )
    };

    // One spelling in, the sixteen stored bytes out.
    let id = Field::new("id", DataType::Uuid, false);
    let stored =
        id.cast_arrow_array(Arc::new(StringArray::from(vec![text])) as ArrayRef, strict())?;
    let batch = RecordBatch::try_new(
        row(id.clone()).into_arrow_schema()?,
        vec![Arc::clone(&stored)],
    )?;

    // The column carries `arrow.uuid`, so a string target reads the spelling.
    let spelled = row(Field::new("id", DataType::utf8(), false))
        .cast_arrow_batch(batch.clone(), strict())?;
    let column = spelled.column(0);
    assert_eq!(
        column.as_any().downcast_ref::<StringArray>().unwrap().value(0),
        text
    );

    // A sixteen-byte framing reads the bytes, and both read back as the identifier.
    let framed = row(Field::new("id", DataType::from_str("fixed_binary(16)")?, false))
        .cast_arrow_batch(batch.clone(), strict())?;
    for read in [Arc::clone(column), Arc::clone(framed.column(0))] {
        let back = id.cast_arrow_array(read, strict())?;
        assert_eq!(back.as_ref(), stored.as_ref());
    }

    // A bound the 36 characters outgrow is refused, naming the row.
    let refused = row(Field::new("id", DataType::from_str("utf8(8)")?, false))
        .cast_arrow_batch(batch, strict())
        .unwrap_err()
        .to_string();
    assert!(refused.contains("row 0"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    id = Field("id", "uuid", nullable=False)
    stored = id.cast_arrow_array(pa.array([text, text.upper()]))

    # A recognized identifier column renders as its spelling, exactly as a
    # recognized ASCII column renders as its trimmed text.
    batch = pa.record_batch([stored], schema=pa.schema([id.into_arrow()]))
    spelled = DataType.from_fields([Field("id", "utf8")])
    assert spelled.cast_arrow_batch(batch).column(0).to_pylist() == [text, text]

    # A spelling that is not an identifier is refused, naming what is accepted.
    try:
        id.cast_arrow_array(pa.array(["not-a-uuid"]))
    except ValueError as error:
        assert "36-character" in str(error)
    ```

## RFC 9562 versions

The version is a fact about the value, never about the column: four bits, so
every identifier answers one, and an identifier written by no version scheme
answers whatever those bits hold. `Uuid::from_v7` packs a Unix microsecond
instant and a 64-bit payload: the first 48 bits hold the millisecond it falls
in, the ten bits under the version hold the microsecond within it, `0..=999`,
and the payload takes the two bits `rand_a` has left over and the whole of
`rand_b` - its top two above the variant, its low sixty-two below. Nothing is
hashed, narrowed or combined, so the 128 bits are filled exactly and both
facts read back out: two identifiers are equal only where the microsecond and
all 64 payload bits are. The whole microsecond instant sorts first and the
whole payload after it, because the two variant bits standing between the
payload's halves are the same in every identifier. `Uuid::from_v8` sets the version and variant bits over a payload
that is already resolved. Neither reads a clock, allocates, or supplies
randomness of its own. [`TxHash::into_uuid`](../hashing.md#order-and-uuidv7-projection)
is the one caller that projects an instant and a digest through
`from_v7`.

The two constructors are Rust-only; a binding receives the identifier they made
as any other `uuid` value.

=== "Rust"

    ```rust
    use yggdryl::Uuid;

    // A UUIDv7 carries its instant, so microseconds order before the rest.
    let value = Uuid::from_v7(1_645_557_742_000_123, 0xfedc_ba98_7654_3210)?;
    assert_eq!(value.to_string(), "017f22e2-79b0-71ef-bedc-ba9876543210");
    assert_eq!(value.version(), 7);
    // The millisecond leads, then the microsecond within it, then the payload.
    assert!(Uuid::from_v7(999, u64::MAX)? < Uuid::from_v7(1_000, 0)?);
    assert!(Uuid::from_v7(1, u64::MAX)? < Uuid::from_v7(2, 0)?);

    // The packing is exact rather than a fingerprint: every payload bit is
    // stored, so one bit apart is one identifier apart and both facts come
    // back out of the sixteen bytes.
    assert_ne!(Uuid::from_v7(1, 1)?, Uuid::from_v7(1, 0)?);
    assert_ne!(Uuid::from_v7(1, 1 << 63)?, Uuid::from_v7(1, 0)?);
    let packed = value.get();
    assert_eq!(
        (packed >> 80) * 1_000 + ((packed >> 66) & 0x3ff),
        1_645_557_742_000_123,
    );
    assert_eq!(
        (((packed >> 64) & 0x3) << 62) | (packed & ((1 << 62) - 1)),
        0xfedc_ba98_7654_3210,
    );

    // A UUIDv8 replaces the six version and variant bits and keeps the other 122.
    let derived = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
    assert_eq!(derived.to_string(), "5c146b14-3c52-8afd-938a-375d0df1fbf6");
    assert_eq!(derived.version(), 8);

    // The nil identifier is the one no element has.
    assert!(Uuid::new(0).is_nil());
    assert!(!value.is_nil());

    // An instant outside `0..=281474976710655999` microseconds has no UUIDv7.
    assert!(Uuid::from_v7(-1, 0).is_err());
    assert!(Uuid::from_v7(281_474_976_710_656_000, 0).is_err());
    ```

## Edges

- `not-a-uuid`, a wrong digit count, a misplaced hyphen -> `InvalidRecord` naming the accepted spellings and what was read instead.
- `uuid_packed` or `uuid_value` on another datatype -> `InvalidDataType` naming the type.
- The packed integer -> the same in every process, and what a stable hash hashes; it is unsigned, so a negative integer is not one.
- [Merged](field.md) with `fixed_size_binary(16)` -> those bytes widening, `uuid` narrowing; any other width -> `binary`; `bytes_parameters` -> `None`, a UUID is bytes with an identity rather than a byte column.
- A recognized identifier column into any [string](text/string.md) datatype -> the canonical spelling, under that datatype's own layout, charset and bound; a bound the 36 characters outgrow -> refused naming the row.
- A recognized identifier column into any [byte framing](text/bytes.md) -> the sixteen bytes; a width that is not sixteen -> refused naming both sides, at plan time when the source width is declared and at the row otherwise.
- Into `uuid` -> sixteen bytes read as the identifier they are, a fixed slot of any other width read as the spelling it holds with the slot's padding trimmed, and any text read as a spelling. Sixteen bytes are never trimmed: every one of them carries identity, and a trailing `0x00` is one of them.
- `Uuid::render` -> the same characters `Display` writes, into the caller's slot; `to_string` is that string when an owned one is what the caller needs.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- uuid
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k uuid
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="uuid" node/tests/datatype.test.js
    ```
