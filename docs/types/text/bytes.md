# Bytes

One byte datatype in six leaves: an opaque payload, the leaf that declares how it is laid out and how long it may be, and the value `Bytes` that holds it.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Bytes(BytesType)`, the six leaves, and the value `Bytes` |
| Validates | The number at construction; a length at the value door, in stored bytes |
| Lazy | Nothing - a leaf is a copy value with no registry, no child and no deferred parse |
| Cached | The Arrow projection of a [`Field`](../field.md); a payload up to `INLINE_BYTES` bytes lives inside the `Bytes` itself |
| Refuses | A number beside `binary`'s three unbounded siblings, `fixed_binary` or `sized_binary` with no number, a bound of zero, and a value longer than the declared bound or unequal to the declared width |
| Kinds | `DataTypeKind::Bytes`, ids `0x41`-`0x46` |
| Bindings | `Bytes` is Rust only; Python and JavaScript read a value as a [`Scalar`](../scalar.md) and the declaration as the frozen `BytesParameters` |

A [UUID](../uuid.md) and a [geospatial](../geospatial/index.md) value are bytes
with an identity, so they are their own datatypes and answer no
`bytes_parameters`, exactly as a [code](../codes/index.md) answers no
`string_parameters`.

## DataType

`DataType::bytes` takes the whole declaration - a layout and a number - and
each leaf constructor is that call with the leaf picked once. `binary(n)` is
`sized_binary(n)` written short, because plain binary is exactly the storage a
bounded column fills; every other leaf would lose itself under a maximum, so it
says so rather than silently becoming something narrower. Bytes are never
padded, so a fixed value is exactly its width.

| leaf | spelling | number | also parsed as |
| --- | --- | --- | --- |
| 32-bit offsets | `binary` | none | `bytes`, `blob`, `bytea`, `varbinary` |
| 64-bit offsets | `large_binary` | none | - |
| view | `binary_view` | none | - |
| view, 64-bit offsets | `large_binary_view` | none | - |
| fixed width | `fixed_binary(n)` | the exact width, required | `fixed_size_binary(n)` |
| bounded | `sized_binary(n)` | the maximum, required | `binary(n)`, `varbinary(n)`, `varbinary_bounded(n)` |

=== "Rust"

    ```rust
    use yggdryl::{BytesType, DataType, DataTypeId, DataTypeKind};

    // The leaf is the whole declaration, and a maximum is its own leaf.
    let bounded = DataType::from_str("varbinary(16)")?;
    assert_eq!(bounded.to_string(), "sized_binary(16)");
    assert_eq!(bounded.bytes_parameters(), Some(BytesType::SizedBinary(16)));
    assert_eq!(bounded.bytes_parameters().unwrap().max(), Some(16));
    assert_eq!(bounded.bytes_parameters().unwrap().storage(), BytesType::Binary);
    assert_eq!(bounded.kind(), DataTypeKind::Bytes);

    // A fixed width is a width; the four unbounded leaves refuse a number.
    assert_eq!(DataType::fixed_binary(16)?.fixed_byte_width(), Some(16));
    assert_eq!(DataType::binary().fixed_byte_width(), None);
    assert!(DataType::from_str("large_binary(16)").is_err());

    // The leaf is the identifier, and the identifier is the wire contract.
    assert_eq!(DataType::binary().id(), DataTypeId::Binary);
    assert_eq!(DataTypeId::Binary.as_u8(), 0x41);
    assert_eq!(bounded.id(), DataTypeId::SizedBinary);
    assert_eq!(BytesType::ALL.len(), 6);

    // A byte column has no charset and answers no string parameters.
    assert!(DataType::binary().string_parameters().is_none());
    assert_eq!(DataType::binary().charset(), None);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import BytesParameters, DataType, types

    # The leaf is the whole declaration, and a maximum is its own leaf.
    bounded = DataType("varbinary(16)")
    assert str(bounded) == "sized_binary(16)"
    assert bounded.bytes_parameters == BytesParameters("sized_binary", 16)
    assert bounded.bytes_parameters.max == 16
    assert bounded == DataType.bytes(bound=16)
    assert bounded.kind == "bytes"

    # A fixed width is a width; the four unbounded leaves refuse a number.
    assert DataType.fixed_size_binary(16).fixed_byte_width == 16
    assert DataType.binary().fixed_byte_width is None
    with pytest.raises(ValueError):
        DataType("large_binary(16)")

    # One factory per leaf, and one that takes the whole declaration.
    assert types.bytes("blob", layout="fixed_binary", fixed=16).dtype.id == "fixed_binary"
    assert types.sized_binary("blob", 16).dtype == bounded

    # A byte column has no charset and answers no string parameters.
    assert DataType.binary().string_parameters is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // The leaf is the whole declaration, and a maximum is its own leaf.
    const bounded = DataType.from('varbinary(16)')
    assert.equal(bounded.toString(), 'sized_binary(16)')
    assert.deepEqual(bounded.bytesParameters, { layout: 'sized_binary', bound: 16, max: 16 })
    assert.ok(DataType.bytes({ max: 16 }).equals(bounded))
    assert.equal(bounded.kind, 'bytes')

    // A fixed width is a width; the four unbounded leaves refuse a number.
    assert.equal(DataType.fixedSizeBinary(16).fixedByteWidth, 16)
    assert.equal(DataType.binary().fixedByteWidth, null)
    assert.throws(() => DataType.from('large_binary(16)'))

    // One factory per leaf, and one that takes the whole declaration.
    assert.equal(fields.bytes('blob', { layout: 'fixed_binary', fixed: 16 }).dtype.id, 'fixed_binary')
    assert.ok(fields.sizedBinary('blob', 16).dtype.equals(bounded))

    // A byte column has no charset and answers no string parameters.
    assert.equal(DataType.binary().stringParameters, null)
    ```

## Field

`BytesField` is the typed marker: one field carrying `BytesType` itself, so the
leaf is read off the payload rather than matched out of a root datatype. The
bindings have one factory per leaf plus `types.bytes` / `fields.bytes` for the
whole declaration, nullable unless the call says otherwise.

=== "Rust"

    ```rust
    use yggdryl::{BytesField, BytesType, DataType, DataTypeId, Field};

    let blob = BytesField::new("blob", BytesType::SizedBinary(16), false);
    assert_eq!(blob.name(), "blob");
    assert_eq!(blob.dtype(), &DataType::sized_binary(16)?);
    assert_eq!(blob.typed_dtype_ref().bound(), Some(16));
    assert!(blob.typed_dtype_ref().is_bounded());
    assert_eq!(blob.id(), DataTypeId::SizedBinary);
    assert!(!blob.is_nullable());

    // Widened, it is the same column the root constructor builds.
    assert_eq!(blob.to_field(), Field::new("blob", DataType::sized_binary(16)?, false));
    // A datatype from another family is refused by name.
    let refused = BytesField::try_new("blob", DataType::utf8(), false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("bytes"), "{refused}");

    // Metadata rides beside the datatype.
    let payload = Field::from_parts("payload", DataType::binary(), true, [("source", "feed")])?;
    assert_eq!(payload.get_metadata("source"), Some("feed"));
    ```

=== "Python"

    ```python
    from yggdryl import Field, types

    blob = types.sized_binary("blob", 16, nullable=False)
    assert isinstance(blob, Field)
    assert blob.name == "blob"
    assert str(blob.dtype) == "sized_binary(16)"
    assert blob.nullable is False

    # One factory per leaf, and one that takes the whole declaration.
    assert types.bytes("blob", max=16).dtype == blob.dtype
    assert str(types.fixed_size_binary("digest", 16).dtype) == "fixed_binary(16)"

    # Metadata rides beside the datatype, never inside it.
    payload = types.binary("payload", metadata={"source": "feed"})
    assert payload.metadata["source"] == "feed"
    assert payload.nullable is True
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const blob = fields.sizedBinary('blob', 16, { nullable: false })
    assert.ok(blob instanceof Field)
    assert.equal(blob.name, 'blob')
    assert.equal(blob.dtype.toString(), 'sized_binary(16)')
    assert.equal(blob.nullable, false)

    // One factory per leaf, and one that takes the whole declaration.
    assert.ok(fields.bytes('blob', { max: 16 }).dtype.equals(blob.dtype))
    assert.equal(fields.fixedSizeBinary('digest', 16).dtype.toString(), 'fixed_binary(16)')

    // Metadata rides beside the datatype, never inside it.
    const payload = fields.binary('payload', { metadata: { source: 'feed' } })
    assert.equal(payload.nullable, true)
    assert.equal(payload.get('source'), 'feed')
    ```

## Scalar

`Scalar::Bytes(Bytes)` is the value: the payload, beside the leaf it is stored
under. A maximum is the column's rule and never the value's, so a cell read out
of `sized_binary(16)` is a `binary`.

=== "Rust"

    ```rust
    use yggdryl::{Bytes, DataType, Scalar};

    // The value door checks the length, and answers the plain leaf rather
    // than the column's maximum.
    let bounded = DataType::from_str("binary(4)")?;
    let value = bounded.scalar(vec![1_u8, 2, 3])?;
    assert_eq!(value.as_bytes(), Some(&[1_u8, 2, 3][..]));
    assert_eq!(value.dtype()?, DataType::binary());
    assert!(bounded.scalar(vec![1_u8, 2, 3, 4, 5]).is_err());

    // `Bytes` is the holder every byte-family API answers with.
    assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(Bytes::new([1_u8, 2, 3])));
    assert_eq!(Bytes::new([1_u8, 2, 3]).as_bytes(), &[1, 2, 3]);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    bounded = DataType("binary(2)")
    value = bounded.scalar(b"\x01\x02")
    assert value.as_py() == b"\x01\x02"
    assert value.dtype == DataType("binary")
    assert value.family == "bytes"
    with pytest.raises(ValueError, match="at most 2 bytes"):
        bounded.scalar(b"\x01\x02\x03")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const bounded = DataType.from('binary(2)')
    const value = bounded.scalar(Buffer.from([1, 2]))
    assert.deepEqual(Array.from(value.asJs()), [1, 2])
    assert.equal(value.dtype.toString(), 'binary')
    assert.equal(value.family, 'bytes')
    assert.throws(() => bounded.scalar(Buffer.from([1, 2, 3])), /at most 2 bytes/)
    ```

`Bytes` itself is Rust only: a payload up to `INLINE_BYTES` (30) bytes lives
inside it with no heap behind it, a `'static` one costs nothing, and a longer
one is one shared `Arc<[u8]>`. Equality, order and hash read the payload alone,
which is the only reason `Borrow<[u8]>` is sound.

```rust
use yggdryl::{Bytes, BytesType, DataType, INLINE_BYTES, Scalar};

// A short payload lives inside the value; a longer one is one shared handle.
let payload = Bytes::new([1_u8, 2, 3]);
assert!(payload.is_inline());
assert!(!Bytes::new(vec![0_u8; INLINE_BYTES + 1]).is_inline());
assert_eq!(std::mem::size_of::<Bytes>(), 40);
assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(payload.clone()));

// Restating a value under another leaf keeps the payload; a width is exact.
let fixed = BytesType::FixedBinary(3);
assert_eq!(payload.clone().try_with_parameters(fixed)?.dtype()?, DataType::fixed_binary(3)?);
assert!(Bytes::new([1_u8, 2]).try_with_parameters(fixed).is_err());
```

## Arrow storage

Arrow already says the layout, so the storage *is* the leaf: `binary`,
`large_binary`, `binary_view` and `fixed_binary(n)` cross bare. Only what no
Arrow type can state - a maximum, and the second view width this crate declares
where Arrow has one - rides the `yggdryl.bytes` document, and a document over a
storage it does not describe imports as that storage.

| datatype | Arrow storage | extension name | document |
| --- | --- | --- | --- |
| `binary`, `large_binary`, `binary_view`, `fixed_binary(16)` | the same four | none | - |
| `sized_binary(16)` | `Binary` | `yggdryl.bytes` | `{"layout":"sized_binary","max":16}` |
| `large_binary_view` | `BinaryView` | `yggdryl.bytes` | `{"layout":"large_binary_view"}` |

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    // Bytes are the layout; only a maximum needs a document.
    let plain = Field::new("blob", DataType::binary(), true).into_arrow_field()?;
    assert_eq!(plain.data_type(), &ArrowDataType::Binary);
    assert!(!plain.metadata().contains_key("ARROW:extension:name"));

    let fixed = Field::new("digest", DataType::fixed_binary(16)?, true);
    assert_eq!(fixed.clone().into_arrow_field()?.data_type(), &ArrowDataType::FixedSizeBinary(16));
    assert_eq!(Field::from_arrow_field(&fixed.clone().into_arrow_field()?)?, fixed);

    let capped = Field::new("blob", DataType::from_str("binary(16)")?, true).into_arrow_field()?;
    assert_eq!(capped.data_type(), &ArrowDataType::Binary);
    assert_eq!(capped.metadata()["ARROW:extension:name"], "yggdryl.bytes");
    assert_eq!(capped.metadata()["ARROW:extension:metadata"], r#"{"layout":"sized_binary","max":16}"#);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, types

    # Bytes are the layout; only a maximum needs a document.
    assert types.binary("blob").into_arrow().metadata is None
    assert types.binary("blob").into_arrow().type == pa.binary()
    assert types.fixed_size_binary("digest", 16).into_arrow().type == pa.binary(16)

    capped = Field("blob", "binary(16)").into_arrow()
    assert capped.type == pa.binary()
    assert capped.metadata == {
        b"ARROW:extension:name": b"yggdryl.bytes",
        b"ARROW:extension:metadata": b'{"layout":"sized_binary","max":16}',
    }
    assert Field.from_arrow(capped) == Field("blob", "binary(16)")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // A cast through a struct root answers the Arrow field a column is written as.
    const projected = (field) =>
      fields
        .struct('row', [field], { nullable: false })
        .castArrow(new arrow.Table({ [field.name]: arrow.vectorFromArray(['A'], new arrow.Utf8()) }))
        .schema.fields[0]

    // Bytes are the layout; only a maximum needs a document.
    assert.equal(projected(fields.binary('blob')).metadata.get('ARROW:extension:name'), undefined)
    const capped = projected(fields.bytes('blob', { max: 16 }))
    assert.equal(capped.metadata.get('ARROW:extension:name'), 'yggdryl.bytes')
    assert.equal(capped.metadata.get('ARROW:extension:metadata'), '{"layout":"sized_binary","max":16}')
    ```

## Casts

A bounded variable byte target checks every cell's length (`BytesIngest`); the
four plain leaves stay Arrow's own kernel. Under `safe` a failing cell becomes
null, under strict an error names the row and the column. The whole cast tier
is on [Cast](../cast.md).

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray};
    use yggdryl::FieldValue as _;
    use yggdryl::{ArrowCastOptions, DataType, Field};

    let strict = ArrowCastOptions::new().with_safe(false);
    let blob = Field::new("blob", DataType::from_str("binary(2)")?, true);
    let bytes: ArrayRef = Arc::new(BinaryArray::from(vec![&b"ab"[..], &b"abc"[..]]));

    // Under `safe` a failing cell is null; strict names the row.
    assert!(blob.cast_arrow_array(Arc::clone(&bytes), ArrowCastOptions::new())?.is_null(1));
    let refused = blob.cast_arrow_array(bytes, strict).unwrap_err().to_string();
    assert!(refused.contains("row 1"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import Field

    blob = Field("blob", "binary(2)")

    # Under `safe` a failing cell is null; strict names the row.
    assert blob.cast_arrow_array(pa.array([b"ab", b"abc"])).to_pylist() == [b"ab", None]
    with pytest.raises(ValueError, match="row 1"):
        blob.cast_arrow_array(pa.array([b"ab", b"abc"]), safe=False)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const blob = fields.bytes('blob', { max: 2 })
    const bytes = arrow.vectorFromArray(
      [Uint8Array.of(1, 2), Uint8Array.of(1, 2, 3)],
      new arrow.Binary(),
    )

    // Under `safe` a failing cell is null; strict names the row.
    assert.equal(blob.castArrowArray(bytes).get(1), null)
    assert.throws(() => blob.castArrowArray(bytes, { safe: false }), /row 1/)
    ```

## Serialized shape

One `binary` tag for every byte column, with `layout` naming the leaf (omitted
when `binary`) and `fixed` or `max` beside it (omitted when the leaf carries no
number). A scalar crosses as `{"type":"bytes","value":...}`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    assert_eq!(DataType::binary().into_json()?, r#"{"type":"binary"}"#);
    assert_eq!(
        DataType::from_str("binary(16)")?.into_json()?,
        r#"{"type":"binary","layout":"sized_binary","max":16}"#
    );
    assert_eq!(
        DataType::fixed_binary(16)?.into_json()?,
        r#"{"type":"binary","layout":"fixed_binary","fixed":16}"#
    );
    let field = Field::new("blob", DataType::from_str("binary(16)")?, true);
    assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    assert DataType("binary").into_dict() == {"type": "binary"}
    assert DataType("binary(16)").into_dict() == {
        "type": "binary",
        "layout": "sized_binary",
        "max": 16,
    }
    assert DataType.fixed_size_binary(16).into_dict() == {
        "type": "binary",
        "layout": "fixed_binary",
        "fixed": 16,
    }
    field = Field("blob", "binary(16)")
    assert Field.from_json(field.into_json()) == field
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    assert.deepEqual(DataType.binary().toJSON(), { type: 'binary' })
    assert.deepEqual(DataType.from('binary(16)').toJSON(), {
      type: 'binary',
      layout: 'sized_binary',
      max: 16,
    })
    assert.deepEqual(DataType.fixedSizeBinary(16).toJSON(), {
      type: 'binary',
      layout: 'fixed_binary',
      fixed: 16,
    })
    const field = new Field('blob', 'binary(16)', true)
    assert.ok(Field.fromJSONBytes(field.toJSONBytes()).equals(field))
    ```

## Edges

- `fixed_binary`, `sized_binary` with no number -> refused; the number is what makes the leaf. A bound of `0` -> refused, `at least one byte, got 0`.
- `binary(16)` -> `sized_binary(16)` written short; `large_binary(16)` -> refused, a large or view leaf holds no maximum.
- Bytes are never padded: a `fixed_binary(n)` value is exactly `n` bytes, and a shorter or longer one is refused.
- A value never carries a maximum: `Scalar::dtype()` of a cell read out of `sized_binary(16)` is `binary`.
- `bytes_parameters` on a [UUID](../uuid.md) or a geospatial value -> `None`; both are bytes with an identity, so each is its own datatype.
- `string_parameters` on a byte column -> `None`, and `charset` answers `None`: a payload has no repertoire to be text in. A UUID reads into text through the one cast tier rather than a renderer of its own.
- A `yggdryl.bytes` document over a storage it does not describe -> imports as the storage.
- Avro and Iceberg have no maximum, so a bounded column crosses them unbounded and the bound is enforced where the values enter.
- A `BytesIngest` refusal under `safe` -> null, which a required column then fills with the default; under strict -> `field "<name>" row <n>: expected ..., got ...`.
- Merging follows [Field](../field.md): two byte types meet parameter by parameter, and a byte column never meets a [string](string.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::bytes field::binary bytes::
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^bytes/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "bytes or binary or byte_column"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="byte|binary" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
