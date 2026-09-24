# Value stream

One byte stream for any value: the stream's version, the value's datatype identifier, and the payload that identifier says how to read, nested values inside it and a long payload compressed. What pickle carries, and what any caller uses to move one value as bytes and get the same value back, leaf for leaf.

This is the crate's own encoding, and what it keeps is the leaf: width, unit, zone and charset included. [The variant](variant.md) is the other byte form of a value - the Apache Parquet Variant encoding, whose vocabulary is the standard's and therefore smaller - and it is the one every medium writes for a `variant` column. A value crossing to another engine takes that one; a value crossing to itself takes this one.

## Contract

| | |
| --- | --- |
| Owns | `Scalar::encode_value_stream_bytes`, `Scalar::into_value_bytes`, `Scalar::decode_value_bytes`, `Scalar::decode_value_stream_bytes`; the same four on `DataType` and on every `DataTypeValue`, casting to the datatype first; `VALUE_STREAM_VERSION`, `COMPRESS_FROM`, `ValueStream` |
| Frame | byte 0 is the version, `0`; byte 1 is the value's [`DataTypeId`](datatype.md#identity-and-family), the same byte the [digest feed](../hashing.md#encoding) writes as a tag; then the payload |
| Fixed payloads | a number is its little-endian bytes and nothing else, because the identifier says how wide it is: `int32` is two bytes and four, a `uuid` two and sixteen, a boolean two and one, a null two and none; a decimal is its scale as one byte, then its coefficient; a clock is its unit as one byte, its zone as a length-prefixed text where the leaf has one, then its count |
| Variable payloads | text, bytes, a code, a version, a location, a zone, a media type, a geometry: one compression byte - `0` as it is, `1` a zstd frame - the size as an unsigned LEB128 count, then the bytes; past `COMPRESS_FROM`, 4 KiB, the bytes are compressed; a fixed or bounded leaf states its width first |
| Nested payloads | a serie is a count then each child; a map a count then each key and value; a struct a count then, per child sorted by name, a length-prefixed name and the child; a child is the same encoding without the version byte, which the stream stated once |
| Stream | `encode_value_stream_bytes` answers one chunk per leaf, in order, holding what it has yet to write and nothing it wrote; `into_value_bytes` is the same bytes in one buffer; either decoder reads either cut |
| Identifiers | one byte laid out by family: every [`DataTypeKind`](datatype.md#identity-and-family) owns a range, `DataTypeKind::range`, starting at its own number, `DataTypeKind::id` - a placeholder no leaf takes, but for the null family's - and its leaves follow; `DataTypeId::from_u8` reads a byte back and `DataTypeKind::of_u8` says which family a byte is in, so a leaf added later lands beside its family and a stream written before it never moves |
| Refusals | positioned at the byte: another version, a byte naming no datatype or a family's own placeholder, a payload cut short, a compression this does not read, text that is not UTF-8, a struct naming a child twice, a value the leaf refuses, bytes left after the value |
| Pickle | Python's `Scalar.__reduce__` hands pickle the bytes and `Scalar._from_pickle` reads them back; a `FixMsg` pickles its row the same way |
| Bindings | Python `Scalar.into_value_bytes() -> bytes`, `Scalar.from_value_bytes(data)`; JavaScript `intoValueBytes(): Buffer`, `Scalar.fromValueBytes(data)`; the stream and the datatype doors are Rust-only |

## Use

=== "Rust"

    ```rust
    use yggdryl::{COMPRESS_FROM, DataType, DataTypeId, DataTypeKind, Scalar, VALUE_STREAM_VERSION};

    // A number is the version, the identifier and its bytes.
    let bytes = Scalar::from(7_i32).into_value_bytes();
    assert_eq!(bytes, [VALUE_STREAM_VERSION, DataTypeId::Int32.as_u8(), 7, 0, 0, 0]);
    assert_eq!(Scalar::decode_value_bytes(&bytes)?, Scalar::from(7_i32));

    // A tree reads back leaf for leaf, in one buffer or as a stream.
    let quote = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("sizes", Scalar::from_sequence([Scalar::from(100_i64), Scalar::Null])),
    ])?;
    let chunks: Vec<Vec<u8>> = quote.encode_value_stream_bytes().collect();
    assert_eq!(chunks.len(), 7, "the struct, then a name and a value per child");
    assert_eq!(chunks.concat(), quote.into_value_bytes());
    assert_eq!(Scalar::decode_value_stream_bytes(&chunks)?, quote);

    // A long text is compressed once it is past four kibibytes.
    let long = "x".repeat(COMPRESS_FROM + 1);
    let bytes = Scalar::from(long.as_str()).into_value_bytes();
    assert_eq!(bytes[2], 1, "a zstd frame");
    assert!(bytes.len() < 64);
    assert_eq!(Scalar::decode_value_bytes(&bytes)?, Scalar::from(long.as_str()));

    // A datatype casts on the way in and on the way out.
    let bytes = DataType::Int64.encode_value_bytes(&Scalar::from(7_i32))?;
    assert_eq!(bytes[1], DataTypeId::Int64.as_u8());
    assert_eq!(DataType::utf8().decode_value_bytes(&bytes)?, Scalar::from("7"));

    // The identifiers are laid out by family.
    assert_eq!(DataTypeKind::Integer.id(), 0x10);
    assert_eq!(DataTypeKind::of_u8(DataTypeId::Int32.as_u8()), Some(DataTypeKind::Integer));
    assert_eq!(DataTypeId::from_u8(0x10), None, "a family's own number is a placeholder");
    let refused = Scalar::decode_value_bytes(&[VALUE_STREAM_VERSION, 0x10]).unwrap_err();
    assert!(refused.to_string().contains("placeholder"));
    ```

=== "Python"

    ```python
    import pickle

    from yggdryl import DataType, Scalar

    # A number is the version, the identifier and its bytes.
    value = DataType("int32").scalar(7)
    data = value.into_value_bytes()
    assert data[0] == 0 and len(data) == 6
    assert Scalar.from_value_bytes(data) == value

    # A tree reads back leaf for leaf, and pickle carries the same bytes.
    quote = Scalar.from_struct({"symbol": "AAPL", "sizes": [100, None]})
    assert Scalar.from_value_bytes(quote.into_value_bytes()) == quote
    assert pickle.loads(pickle.dumps(quote)) == quote

    # A long text is compressed once it is past four kibibytes.
    long = Scalar.from_("x" * (4 * 1024 + 1))
    data = long.into_value_bytes()
    assert data[2] == 1 and len(data) < 64
    assert Scalar.from_value_bytes(data) == long

    # A refusal names the byte.
    try:
        Scalar.from_value_bytes(b"\x01\x00")
    except ValueError as error:
        assert "version 1" in str(error)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    // A number is the version, the identifier and its bytes.
    const value = new DataType('int32').scalar(7)
    const data = value.intoValueBytes()
    assert.equal(data[0], 0)
    assert.equal(data.length, 6)
    assert.ok(Scalar.fromValueBytes(data).equals(value))

    // A tree reads back leaf for leaf.
    const quote = Scalar.from({ symbol: 'AAPL', sizes: [100, null] })
    assert.ok(Scalar.fromValueBytes(quote.intoValueBytes()).equals(quote))

    // A long text is compressed once it is past four kibibytes.
    const long = Scalar.from('x'.repeat(4 * 1024 + 1))
    const packed = long.intoValueBytes()
    assert.equal(packed[2], 1)
    assert.ok(packed.length < 64)
    assert.ok(Scalar.fromValueBytes(packed).equals(long))

    // A refusal names the byte.
    assert.throws(() => Scalar.fromValueBytes(Buffer.from([1, 0])), /version 1/)
    ```

## One byte says the leaf

The second byte is the value's own `DataTypeId`, so the decoder reads the payload with nothing else: it knows an `int32` is four bytes, a `fixed_ascii(n)` states its width first, a `datetime64` its unit and zone. What a datatype states beside its identifier travels only where the value needs it - a decimal's scale, never its precision, which is the column's rule and not the value's - so the bytes decode to the value that was encoded and its leaf, and a value cast to a datatype first encodes under that datatype's leaf.

The identifiers are the digest feed's tags too: [`Scalar::write_bytes`](../hashing.md#encoding) writes the same byte before a value's canonical bytes. The two encodings differ after it, deliberately: the feed normalizes so that equal values feed alike whatever width holds them, and the value stream keeps the width so that the value reads back as itself.

## Families own ranges

A `DataTypeId` is one byte, and the bytes are laid out by family: every `DataTypeKind` starts a range at its own number - `0x00` null, `0x08` boolean, `0x10` integer, `0x20` floating, `0x28` decimal, `0x30` temporal, `0x40` bytes, `0x50` text, `0x70` code, `0x80` uuid, `0x90` nested, `0xb0` geospatial - and its leaves follow in the range, so the high bits of a leaf say its family. The family's own number is a placeholder no leaf takes, except the null family's, whose one leaf is the null itself: a stream tagged with a placeholder is refused as a value of that family and of no leaf, and a leaf added later takes the next number in its family's range rather than the end of the enum, so no stream written before it moves. `DataTypeId::ALL` states the leaves in that order, and the test pinning every byte is what makes a moved number a failure rather than a surprise.

## Edges

- Another version byte -> refused, naming the version read and the one this reads.
- A family's own number as a tag -> refused as a placeholder, naming the family.
- A byte past every family -> refused as naming no datatype.
- Bytes left after the value -> refused, counting them; a stream is one value.
- A struct naming one child twice -> refused; a struct's children are one map.
- An Arrow-held value -> encodes as the native value it holds, and as a null where it holds none the native reading can spell.
- Exactly 4 KiB -> stored as it is; one byte past -> a zstd frame, unless compressing would not shorten it.
- `decode_value_stream_bytes` -> the chunks gathered, then read as one; any cut of the same bytes reads the same value.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root -- valuestream::stream
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_scalar.py -k value_bytes
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="value bytes" node/tests/text/codec.test.js
    ```
