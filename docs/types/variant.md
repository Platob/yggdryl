# Variant

One semi-structured value as the two binaries the [Apache Parquet Variant binary encoding](https://github.com/apache/parquet-format/blob/master/VariantEncoding.md) states: a `metadata` dictionary of the object keys in the tree, and a `value` payload whose first byte names a primitive, a short string, an object or an array. It is version `1` of that specification, byte for byte, so a value written here is one Spark, Iceberg, Parquet and Arrow read, and one any of them wrote reads back here.

`variant` is a datatype (`DataType::Variant`), a field (`VariantField`) and a value (`Scalar::Variant`, holding a `Variant`). A variant *is* its bytes, the way a geometry is its WKB: the value inside them is `Variant::scalar`, and a cast either way encodes or decodes.

## Contract

| | |
| --- | --- |
| Owns | `Variant` with `new`, `encode`, `metadata`, `value`, `scalar`; `Scalar::Variant`, `Scalar::into_variant`, `Scalar::from_variant`; `DataType::encode_variant` and `DataType::decode_variant`, casting first; `VARIANT_VERSION`, `VARIANT_EXTENSION_NAME`, `VARIANT_METADATA_FIELD`, `VARIANT_VALUE_FIELD` |
| Metadata | a header byte - the version in its low nibble, `sorted_strings` set, the offset width in its top two bits - then the dictionary size, `size + 1` offsets and the key bytes; the keys of every object in the tree, sorted and deduplicated, so the value payload names each one by index |
| Value | one `value_metadata` byte: two bits of basic type - primitive, short string, object, array - and six bits of header; then the payload that byte says how to read. A string under 64 bytes folds its length into the header; an object states its count, its field ids in key order, its offsets and its values; an array the same without the ids |
| Widths | the narrowest the payload allows: a count under 256 is one byte and four past it, an offset or a field id is one to four bytes |
| Primitives | the twenty-one the specification names: `null`, `boolean`, `int8`..`int64`, `float`, `double`, `decimal4`/`8`/`16`, `date`, `timestamp` and `timestamp ntz` in microseconds and in nanoseconds, `time` (microseconds, no zone), `binary`, `string`, `uuid` |
| Arrow | a struct of two required binaries, `metadata` and `value`, under the canonical `arrow.parquet.variant` extension name with an empty document; a foreign writer's `LargeBinary` or `BinaryView` children are the same storage |
| Parquet | `optional group name (VARIANT(1)) { required binary metadata; required binary value; }` - the annotation the format states, the two children carrying no field id, which is what Iceberg requires; a file another writer produced imports as a variant from that annotation alone |
| Avro | a record of `metadata` and `value`, both `bytes`, read by name and carrying no field ids, annotated `"logicalType": "variant"`; a reader that does not know the annotation reads the record, as the specification requires |
| Iceberg | the v3 `variant` type: written as the Parquet group above, and never given bounds - a variant's ordering is not defined |
| Children | the two binaries are storage, not schema children: `field_len()` is `0`, nothing descends into them, and `assign_parquet_field_ids` numbers neither - which is exactly what Parquet, Avro and Iceberg require of them |
| Null | a variant can *spell* null, so `Scalar::Null` in a variant column is the encoding's own null byte in a present cell, never an absent one; an absent cell is the struct's own validity bit |
| Equality | two variants are equal when their bytes are; a value cast into a variant is canonically encoded - keys sorted, sizes narrowest - so values that are equal here encode alike |
| Digest | a variant feeds the digest as the value it holds, so one value digests alike whether it crossed as itself or as a variant column's bytes |
| Structured text | JSON, TOML and YAML write the value a variant holds, which is what every other variant reader shows |
| Refusals | positioned at the byte: metadata of another version, a dictionary cut short, an offset past the key bytes, a field id past the dictionary, a payload cut short, a primitive type version 1 does not name, text that is not UTF-8, an object naming one key twice, bytes left after the value |

## What a value writes as

Every value the crate holds writes, because a leaf the standard has no physical type for takes the spelling the [JSON codec](../media/structured.md) gives it - the same text or number another reader would see. The mapping is exact on the way out for the types the standard names:

| value | variant physical type | reads back as |
| --- | --- | --- |
| `null`, `boolean` | `null`, `boolean` | the same |
| `int8`, `int16`, `int32`, `int64` | the same width | the same width |
| `uint8`, `uint16`, `uint32` | the next signed width up | that width |
| `uint64`, `int128`, `uint128` | `int64`, else `decimal16` | that type |
| `float16`, `float32` | `float` | `float32` |
| `float64` | `double` | `float64` |
| `decimal32/64/128/256` | `decimal4`/`decimal8`/`decimal16` | `decimal32/64/128` |
| `date32`, `date64` | `date` | `date32` |
| `time32`, `time64` | `time`, microseconds, no zone | `time64` |
| `datetime64` | `timestamp`, zoned or not, microseconds or nanoseconds | `datetime64` |
| `uuid` | `uuid`, sixteen bytes big-endian | `uuid` |
| every string, code, version, location, zone and media type | `string` | `utf8` |
| every byte layout, and a geometry or geography's WKB | `binary` | `binary` |
| `duration32`, `duration64` | `string`, the ISO-8601 spelling | `utf8` |
| `interval` | the JSON codec's number or array | that shape |
| a list | `array` | a list |
| a struct, and a mapping whose keys are text | `object` | a struct |

A decimal past thirty-eight digits, a time or a timestamp whose count is not a whole microsecond, a zoned time, a zoned duration and a mapping with a key that is not text are what no reading spells, and each is refused by name.

## Use

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, VARIANT_VERSION, Value, Variant};

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
    assert_eq!(value.id(), yggdryl::DataTypeId::Variant);
    assert_eq!(Variant::from_scalar(&value), Some(&variant));

    // Casting into a variant column encodes; casting out decodes.
    let held = DataType::Variant.scalar(quote.clone())?;
    assert_eq!(held, Scalar::Variant(variant));
    assert_eq!(DataType::Int64.scalar(Scalar::Variant(Variant::encode(&Scalar::from(7_i32))?))?,
               Scalar::from(7_i64));

    // The column is a struct of two binaries under the canonical name.
    let field = DataType::Variant.nullable_field("payload").into_arrow_field()?;
    assert_eq!(field.extension_type_name(), Some(yggdryl::VARIANT_EXTENSION_NAME));
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

## The dictionary is the tree's keys

The keys of every object anywhere in the value are gathered first, sorted by their UTF-8 bytes and deduplicated into one dictionary, so a key repeated in a thousand rows of one object costs its bytes once per value and an index per use. The header states `sorted_strings`, which lets a reader binary-search the field ids, and the object's ids and offsets are written in that key order - what the specification requires, and what makes a field lookup a search rather than a scan.

Because the encoding is canonical here - sorted keys, narrowest widths - two equal values encode alike, which is what makes byte equality a usable equality. A variant another engine wrote may use wider offsets or an unsorted dictionary; it reads back as the same value, and it compares as the different bytes it is.

## One pair, four media

A variant column is the same two binaries wherever it lands, because every format states the same shape for it:

- **Arrow** - a struct of `metadata` and `value` under `arrow.parquet.variant`.
- **Parquet** - a group of two `BYTE_ARRAY` children annotated `VARIANT(1)`.
- **Avro** - a record of two `bytes` fields, annotated `variant`.
- **Iceberg** - the v3 `variant` type, stored as that Parquet group, with no bounds in the manifest.

So the encoding happens once, at the value boundary, and the media layers move the bytes: nothing re-encodes a variant on the way to a file, and reading one back is the bytes plus the decode the caller asks for.

## Edges

- A `Scalar::Null` in a variant column -> the encoding's null byte, a present cell.
- A variant holding a value another datatype can hold -> casts to it, decoding first.
- A foreign variant with wider offsets -> reads as the same value, compares as different bytes.
- A foreign writer's shredded column -> the unshredded rows read as themselves; a row whose `value` is null holds its value in `typed_value`, which this does not read, and refuses by name.
- The `arrow.parquet.variant` name over a storage it does not spell -> a foreign field wearing it: the column imports as that storage.
- A foreign Parquet file's `VARIANT` group -> imports as a variant from the annotation alone, whatever Arrow schema the file carries.
- A decimal past thirty-eight digits, or a `uint128` past `i128::MAX` -> refused, naming the digits the standard holds.
- A timestamp in seconds or milliseconds -> exact in microseconds, which is the precision the standard states.
- A nanosecond timestamp -> its own two primitive types, zoned and not.

## Cost

The encoding's cost is the value's shape. A leaf is a header byte and its
bytes; an object gathers the tree's keys once, sorts them and writes each
one's bytes a single time, naming them by index from then on; a wide object
is its dictionary plus two small integers per field. `cargo bench -p
yggdryl --bench types -- variant` measures encode and decode over a leaf, a
small object and a wide one, and builds the same object through
`parquet-variant` beside it, so the two rows are the same bytes on the same
value.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test types variant
    cargo test -p yggdryl --test interop variant
    cargo bench -p yggdryl --bench types -- variant --quick
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types -k variant
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="variant" node/tests/types/datatype.test.js
    ```
