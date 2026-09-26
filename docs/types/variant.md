# Variant

One semi-structured value as the two binaries the [Apache Parquet Variant binary encoding](https://github.com/apache/parquet-format/blob/master/VariantEncoding.md) states: a `metadata` dictionary of the object keys in the tree, and a `value` payload whose first byte names a primitive, a short string, an object or an array.

It implements version `1` of that specification. Supported unshredded values exchange with other implementations; unsupported versions and shredded values are refused. A variant *is* its bytes, the way a geometry is its WKB: the value inside them is `Variant::scalar`, and a cast either way encodes or decodes.

## Contract

| | |
| --- | --- |
| Owned | `Variant` with `new`, `encode`, `metadata`, `value`, `scalar`; `Scalar::Variant`, `Scalar::into_variant`, `Scalar::from_variant`; `DataType::encode_variant` and `DataType::decode_variant`, casting first; the `VariantField` marker; `VARIANT_VERSION`, `VARIANT_EXTENSION_NAME`, `VARIANT_METADATA_FIELD`, `VARIANT_VALUE_FIELD` |
| Validated | the metadata header on `Variant::new` - the version, the offset width, the dictionary and its keys - and the whole value payload on `Variant::scalar`; every refusal is positioned at the byte it could not read |
| Lazy | the value payload: `new` reads the dictionary and leaves the payload for `scalar`. The dictionary and offset tables are borrowed while decoding, and converting or cloning an existing `Variant` shares its two buffers without re-encoding |
| Cached | nothing; a variant is its two reference-counted buffers, and equality is over their bytes |
| Refused | positioned at the byte: metadata of another version, a dictionary cut short, an offset past the key bytes, a field id past the dictionary, a payload cut short, a primitive type version 1 does not name, text that is not UTF-8, an object naming one key twice, bytes left after the value - and, at encode time, a value the standard has no spelling for |

## DataType

`variant` is `DataType::Variant`, built bare by `DataType::variant()`. It takes
no parameter and its kind is `Nested`, because a variant holds a tree; the two
binaries are storage rather than schema children, so `field_len()` is `0` and
nothing descends into them.

The parenthesis is the whole of the difference between the two spellings: bare
`variant` is this datatype, and `variant(...)` with members is the
[dense-union sugar](nested/union.md).

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    // One self-describing datatype, no parameters.
    let variant = DataType::variant();
    assert_eq!(variant, DataType::Variant);
    assert_eq!(variant.to_string(), "variant");
    assert_eq!(variant.id(), DataTypeId::Variant);
    assert_eq!(variant.kind(), DataTypeKind::Nested);
    assert_eq!(DataType::from_str("variant")?, variant);

    // The two binaries are storage, not schema children.
    assert_eq!(variant.field_len(), 0);

    // With members, the same word is the dense-union sugar.
    assert_eq!(DataType::from_str("variant(only:int64)")?.id(), DataTypeId::Union);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    # Bare `variant` is the self-describing datatype.
    variant = DataType.variant()
    assert variant.id == "variant"
    assert variant.kind == "nested"
    assert str(variant) == "variant"
    assert DataType("variant") == variant

    # With members, the same word is the dense-union sugar.
    assert DataType("variant(only:int64)").id == "union"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // Bare `variant` is the self-describing datatype.
    const variant = DataType.variant()
    assert.equal(variant.id, 'variant')
    assert.equal(variant.kind, 'nested')
    assert.equal(variant.toString(), 'variant')
    assert.ok(new DataType('variant').equals(variant))
    assert.ok(DataType.fromString(variant.toString()).equals(variant))

    // With members, the same word is the dense-union sugar.
    assert.equal(DataType.fromString('variant(only:int64)').id, 'union')
    ```

## Field

`VariantField` is the marker, and the bindings have one factory each. The
column's nullability is where absence and an encoded null part: a bare
`Scalar::Null` under a field is absence and needs a nullable field, while
`Scalar::Variant(Scalar::Null.into_variant()?)` - what
`DataType::Variant.scalar(Scalar::Null)` builds - is a present encoded null,
which even a required field accepts. The canonical default follows: null where
the column admits one, the encoded null where it does not.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    let optional = DataType::Variant.nullable_field("payload");
    let required = DataType::Variant.required_field("payload");

    // Bare null is absence, and a required column refuses it.
    assert_eq!(optional.scalar(Scalar::Null)?, Scalar::Null);
    assert!(required.scalar(Scalar::Null).is_err());

    // An explicitly encoded null is a present value either column holds.
    let present = DataType::Variant.scalar(Scalar::Null)?;
    assert!(matches!(present, Scalar::Variant(_)));
    assert_eq!(required.scalar(present.clone())?, present);

    // So the canonical default is the one the column can hold.
    assert_eq!(optional.default_value()?, Scalar::Null);
    assert_eq!(required.default_value()?, present);
    assert!(DataType::Variant.is_default_value(&present)?);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    payload = yggdryl.variant("payload", nullable=False)
    assert isinstance(payload, Field)
    assert str(payload.dtype) == "variant"
    assert payload.nullable is False

    # Nullable unless the call says otherwise; metadata rides beside it.
    tagged = yggdryl.variant("payload", metadata={"source": "feed"})
    assert tagged.nullable is True
    assert tagged.metadata["source"] == "feed"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const payload = fields.variant('payload', { nullable: false })
    assert.ok(payload instanceof Field)
    assert.equal(payload.dtype.id, 'variant')
    assert.equal(payload.nullable, false)
    assert.equal(fields.variant('payload').nullable, true)
    ```

## Scalar

`Scalar::Variant` holds a `Variant`, so the value in a variant column is the
encoding itself. `Variant::encode` writes a value into the two binaries and
`Variant::scalar` reads them back; casting into the datatype encodes and
casting out of it decodes, so every cast a value answers a variant answers too.
Two variants are equal when their bytes are - encoding sorts keys and narrows
sizes, but preserved numeric widths can give equal native scalars different
variant bytes. A variant feeds [the digest](../hashing.md#encoding) as the
value it holds, so one value digests alike whether it crossed as itself or as a
variant column's bytes, and [JSON, TOML and YAML](../media/index.md#json) write
the value it holds, which is what every other variant reader shows.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, Scalar, VARIANT_VERSION, Value, Variant};

    // A value encodes as the two binaries the specification states.
    let quote = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("size", Scalar::from(100_i64)),
    ])?;
    let variant = Variant::encode(&quote)?;
    assert_eq!(variant.metadata()[0] & 0x0f, VARIANT_VERSION);
    assert_eq!(variant.scalar()?, quote);

    // A variant is a value: it widens into `Scalar` and narrows back.
    let value = variant.clone().into_scalar();
    assert!(matches!(value, Scalar::Variant(_)));
    assert_eq!(value.id(), DataTypeId::Variant);
    assert_eq!(Variant::from_scalar(&value), Some(&variant));

    // Casting into a variant column encodes; casting out decodes.
    let held = DataType::Variant.scalar(quote.clone())?;
    assert_eq!(held, Scalar::Variant(variant));
    assert_eq!(
        DataType::Int64.scalar(Scalar::Variant(Variant::encode(&Scalar::from(7_i32))?))?,
        Scalar::from(7_i64)
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    # A value cast into a variant column is the variant encoding of it.
    quote = Scalar.from_struct({"symbol": "AAPL", "size": 100})
    held = DataType("variant").scalar(quote)
    assert held.kind == "variant"

    # It crosses to Python as the value its bytes hold.
    assert held.as_py() == {"size": 100, "symbol": "AAPL"}

    # And casting it out of the variant decodes it, so every cast a value
    # answers a variant answers too.
    number = DataType("variant").scalar(7)
    assert DataType("int64").scalar(number).as_py() == 7
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    // A value cast into a variant column is the variant encoding of it.
    const quote = Scalar.from({ symbol: 'AAPL', size: 100 })
    const held = new DataType('variant').scalar(quote)
    assert.equal(held.kind, 'variant')

    // It crosses to JavaScript as the value its bytes hold.
    assert.deepEqual(held.asJs(), { size: 100, symbol: 'AAPL' })

    // And casting it out of the variant decodes it, so every cast a value
    // answers a variant answers too.
    const number = new DataType('variant').scalar(7)
    assert.equal(new DataType('int32').scalar(number).asJs(), 7)
    ```

Rust's `Value` trait and `Scalar` both expose `into_variant` and `from_variant`.
`Value::from_variant` requires the exact leaf produced by the format: an unsigned
byte decodes as `Int16`, so `UInt8::from_variant` refuses it. Use
`DataType::UInt8.decode_variant` when that cast is intended. Converting an
existing `Variant` through the trait, or encoding `Scalar::Variant`, shares its
two buffers without allocating or re-encoding. These trait conversions are
Rust-only.

## Arrow storage

| datatype | Arrow | extension | imports back as |
| --- | --- | --- | --- |
| `variant` | `Struct` of two required binaries, `metadata` then `value` | `arrow.parquet.variant`, with an empty document | `variant` |

Foreign `LargeBinary` or `BinaryView` children are accepted; metadata must be
required, while the value child may be nullable. The two children are storage,
so nothing descends into them and `assign_parquet_field_ids` numbers neither -
which is exactly what Parquet, Avro and Iceberg require of them.

=== "Rust"

    ```rust
    use yggdryl::{
        DataType, Field, VARIANT_EXTENSION_NAME, VARIANT_METADATA_FIELD, VARIANT_VALUE_FIELD,
    };

    // The column is a struct of two binaries under the canonical name.
    let field = Field::new("payload", DataType::variant(), true);
    let arrow = field.clone().into_arrow_field()?;
    assert_eq!(arrow.extension_type_name(), Some(VARIANT_EXTENSION_NAME));

    let arrow_schema::DataType::Struct(children) = arrow.data_type() else {
        panic!("a struct storage, got {}", arrow.data_type());
    };
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name(), VARIANT_METADATA_FIELD);
    assert_eq!(children[1].name(), VARIANT_VALUE_FIELD);
    assert!(!children[0].is_nullable(), "the metadata child is required");

    // The pair is storage, so the schema has no children to descend into.
    assert_eq!(field.field_len(), 0);
    assert_eq!(Field::from_arrow_field(&arrow)?, field);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    payload = Field("payload", "variant")
    arrow = payload.into_arrow()

    # Two required binaries, named by the specification, under the canonical
    # extension name with an empty document.
    assert arrow.type == pa.struct(
        [
            pa.field("metadata", pa.binary(), nullable=False),
            pa.field("value", pa.binary(), nullable=False),
        ]
    )
    assert arrow.metadata[b"ARROW:extension:name"] == b"arrow.parquet.variant"
    assert arrow.metadata[b"ARROW:extension:metadata"] == b""
    assert Field.from_arrow(arrow) == payload
    ```

JavaScript has no field-level Arrow export: a variant column crosses there
as a column, `Serie.fromScalars(field, rows).intoArrowArray()`.

## What a value writes as

Version 1 names twenty-one primitives - `null`, `boolean`, `int8`..`int64`,
`float`, `double`, `decimal4`/`8`/`16`, `date`, `timestamp` and `timestamp ntz`
in microseconds and in nanoseconds, `time` (microseconds, no zone), `binary`,
`string`, `uuid` - and supported leaves without a standard physical type use
the spelling the [JSON codec](../media/index.md#json) gives them. Values the
standard cannot represent are refused. Decoding follows the types the standard
names:

| value | variant physical type | reads back as |
| --- | --- | --- |
| `null`, `boolean` | `null`, `boolean` | the same |
| `int8`, `int16`, `int32`, `int64` | the same width | the same width |
| `uint8`, `uint16`, `uint32` | the next signed width up | that width |
| `uint64`, `int128`, `uint128` | `int64`, else `decimal16` | that type |
| `float16`, `float32` | `float` | `float32` |
| `float64` | `double` | `float64` |
| `decimal32/64/128/256` | `decimal4`/`decimal8`/`decimal16` | `decimal32/64/128` |
| `decimal`, `bigdecimal` | `decimal16` at scale eighteen; a `bigdecimal` past 128 bits of units is refused | `decimal128` |
| `date32`, `date64` | `date` | `date32` |
| `time32`, `time64` | `time`, microseconds, no zone | `time64` |
| `datetime64` | `timestamp`, zoned or not, microseconds or nanoseconds | `datetime64` |
| `uuid` | `uuid`, sixteen bytes big-endian | `uuid` |
| every string, code, version, location, zone and media type | `string` | `utf8` |
| every byte layout, and a geometry or geography's WKB | `binary` | `binary` |
| `duration32`, `duration64` | `string`, the ISO-8601 spelling | `utf8` |
| `interval` | the JSON codec's number or array | that shape |
| a serie | `array` | a serie |
| a struct, and a mapping whose keys are text | `object` | a struct |

A decimal past thirty-eight digits, a time whose count is not a whole microsecond, a zoned time, a zoned duration and a mapping with a key that is not text are what no reading spells, and each is refused by name.

## The dictionary is the tree's keys

The metadata is a header byte - the version in its low nibble, `sorted_strings`
set, the offset width in its top two bits - then the dictionary size, `size + 1`
offsets and the key bytes. The keys of every object anywhere in the value are
gathered first, sorted by their UTF-8 bytes and deduplicated into that one
dictionary, so a key repeated in a thousand rows of one object costs its bytes
once per value and an index per use. The header states `sorted_strings`, which
lets a reader binary-search the field ids, and the object's ids and offsets are
written in that key order - what the specification requires, and what makes a
field lookup a search rather than a scan.

The value payload is one `value_metadata` byte - two bits of basic type
(primitive, short string, object, array) and six bits of header - then the
payload that byte says how to read. A string under 64 bytes folds its length
into the header; an object states its count, its field ids in key order, its
offsets and its values; an array the same without the ids. Widths are the
narrowest the payload allows: a count under 256 is one byte and four past it,
an offset or a field id is one to four bytes.

=== "Rust"

    ```rust
    use yggdryl::{Scalar, VARIANT_VERSION, Variant};

    let quote = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("size", Scalar::from(100_i64)),
    ])?;
    let variant = Variant::encode(&quote)?;

    // The header states the version, the sorted claim and the offset width.
    let header = variant.metadata()[0];
    assert_eq!(header & 0x0f, VARIANT_VERSION);
    assert_eq!((header >> 4) & 0x01, 1, "sorted_strings");
    assert_eq!(usize::from(header >> 6) + 1, 1, "one-byte offsets");

    // Two keys, sorted by their UTF-8 bytes: `size` then `symbol`.
    assert_eq!(variant.metadata()[1], 2);
    assert!(variant.metadata().ends_with(b"sizesymbol"));

    // A string under 64 bytes folds its length into the header byte.
    let short = Variant::encode(&Scalar::from("abc"))?;
    assert_eq!(short.value()[0] & 0x03, 1, "the short-string basic type");
    assert_eq!(short.value()[0] >> 2, 3, "the length in the header");
    assert_eq!(short.value().len(), 4);
    ```

The encoder sorts keys and narrows count and offset widths. Numeric storage widths can still distinguish equal native scalars, so byte equality describes the encoded representation. A variant another engine wrote may use wider offsets or an unsorted dictionary; decoding reads its value, while conversion of an existing `Variant` preserves the original bytes.

## One pair, four media

A variant column is the same two binaries wherever it lands, because every format states the same shape for it:

| medium | what it states |
| --- | --- |
| [Arrow](../arrow/index.md) | a struct of `metadata` and `value` under `arrow.parquet.variant` |
| [Parquet](../media/index.md#parquet) | `optional group name (VARIANT(1)) { required binary metadata; required binary value; }` - the two children carrying no field id, which is what Iceberg requires; a file another writer produced imports as a variant from that annotation alone |
| [Avro](../media/index.md#avro) | a record of `metadata` and `value`, both `bytes`, read by name and carrying no field ids, annotated `"logicalType": "variant"`; a reader that does not know the annotation reads the record, as the specification requires |
| [Iceberg](../media/index.md#iceberg) | the v3 `variant` type: written as the Parquet group above, and never given bounds - a variant's ordering is not defined |

The encoding happens once at the value boundary, and the media layers move the bytes. Avro reads the two fields by name in either wire order; reader-schema resolution interprets the result as a `Variant` when the reader declares that annotation. Parquet restores the annotation inside structs, lists and maps as well as at the root.

## Edges

- Bare `Scalar::Null` in a nullable Variant field -> an absent cell; an explicitly encoded Variant null -> a present cell. Required fields refuse bare null.
- A variant holding a value another datatype can hold -> casts to it, decoding first.
- A foreign variant with wider offsets -> reads as the same value, compares as different bytes.
- A foreign writer's shredded column -> the unshredded rows read as themselves; a row whose `value` is null holds its value in `typed_value`, which this does not read, and refuses by name.
- The `arrow.parquet.variant` name over a storage it does not spell -> a foreign field wearing it: the column imports as that storage.
- A foreign Parquet file's version-one `VARIANT` group -> imports from the annotation alone; a future version remains foreign storage.
- A decimal past thirty-eight digits, or a `uint128` past `i128::MAX` -> refused, naming the digits the standard holds.
- A timestamp in seconds or milliseconds -> exact in microseconds, which is the precision the standard states.
- A nanosecond timestamp -> its own two primitive types, zoned and not.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root -- cast::typed code::datatypes datatype::arrow datatype::families default::datatypes diff::comparison field::arrow geospatial parser::grammar scalar temporal::scalars union::variants uuid variant::encoding variant::internal variant::value
    cargo test -p yggdryl --test value -- canonical::value::readings
    cargo test -p yggdryl --test interop variant
    cargo test -p yggdryl --test allocations variant_
    cargo bench -p yggdryl --bench types -- variant --quick
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test__classes.py python/tests/test__defaults.py python/tests/test__hints.py python/tests/test_datatype.py python/tests/test_protocol.py python/tests/test_scalar.py -k variant
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="variant" node/tests/datatype.test.js
    ```

## Performance

Dictionary and offset tables are borrowed during decoding. An ordered array
allocates its final scalar slice directly; an object allocates its retained
map and shared owner. Reordered physical values require an extra sorted
offset table. Primitive values with no object keys share empty metadata.
Encoded Variant columns copy their borrowed metadata and value bytes directly
into two pre-sized Arrow buffers, with no temporary byte buffers per row.

The counting allocator pins warmed calls over an `int64`, objects with short
field names and integer values, and an outer array containing two 64-item
integer arrays:

| Operation | Allocations per call |
| --- | ---: |
| Primitive encode / decode | 2 / 0 |
| Four-field object encode / decode | 12 / 2 |
| 64-field object encode / decode | 25 / 11 |
| Three-array tree decode | 3 |
| Existing Variant conversion or clone | 0 |

The `types` benchmark measures these shapes, arrays, and the `Value`/`Scalar`
conversion methods. Apache's reference object iterator borrows values; its
timing is separate from decoding a complete native `Scalar` tree.
