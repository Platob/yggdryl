# Strings & bytes

One string family in five layouts, one byte family in four, the version and URL values, and the regex that turns named captures into a schema.

## Contract

| | |
| --- | --- |
| Owns | `DataType::String(StringParameters)`, `DataType::Bytes(BytesParameters)`, the values `Str` and `Bytes`, `Version`, `Url` |
| Constructors | `DataType::string` / `DataType::bytes` take the whole declaration; `utf8`, `large_utf8`, `utf8_view`, `ascii`, `fixed_utf8(n)`, `fixed_ascii(n)`, `binary`, `large_binary`, `binary_view`, `fixed_size_binary(n)` pick a layout once |
| Reads back | `string_parameters`, `bytes_parameters`, `charset`, `fixed_byte_width`, `is_string`; a [code](codes.md), a [UUID](uuid.md) and a geospatial value answer no parameters |
| Bound | one number per declaration: the exact width on a fixed layout, the maximum stored bytes elsewhere; zero refused; a fixed layout with no width refused |
| Value | holds UTF-8 (or the payload) beside its layout, charset and fixed width; never a maximum |
| Arrow | text storage for UTF-8 and US-ASCII, binary storage for every other charset; `yggdryl.string` / `yggdryl.bytes` only where Arrow cannot say what is declared |
| Rust only | the value types `Str` and `Bytes`; `StringLayout`, `BytesLayout`; the bindings read the parameters as frozen `StringParameters` / `BytesParameters` values |

### Strings

A string is a layout, the [charset](../charset/index.md) its bytes are written
in, and a bound. Each layout has three spellings that parse to one datatype:
the `string` name is the general one, the `utf8` name is the same layout when
its charset is UTF-8 (the default), and the `ascii` name when it is US-ASCII.
A value renders under the name its charset earns; every other charset renders
under the general name and states itself. Case, `_`, `-` and spaces are
ignored, so `large_utf8`, `largeutf8` and `LargeString` are one datatype.

| layout | general | UTF-8 | US-ASCII | Arrow storage (text / binary) |
| --- | --- | --- | --- | --- |
| 32-bit offsets | `string` | `utf8` | `ascii` | `Utf8` / `Binary` |
| fixed width | `fixed_string(n)` | `fixed_utf8(n)` | `fixed_ascii(n)` | `FixedSizeBinary(n)` |
| view | `string_view` | `utf8_view` | `ascii_view` | `Utf8View` / `BinaryView` |
| 64-bit offsets | `large_string` | `large_utf8` | `large_ascii` | `LargeUtf8` / `LargeBinary` |
| view, large | `large_string_view` | `large_utf8_view` | `large_ascii_view` | `Utf8View` / `BinaryView` |

The general spelling takes `(charset)`, `(bound)` or `(charset, bound)`; a
charset-named spelling takes only `(bound)`, and a charset beside it is
refused. One number, one reading per layout: `ascii(4)` and `utf8(32)` are
maxima, `fixed_ascii(4)` and `fixed_utf8(32)` are widths.

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `utf8` | unbounded UTF-8 | `string`, `str`, `text`, `varchar`, `nvarchar`, `char`, `character varying` |
| `utf8(n)` | at most `n` bytes | `varchar(n)`, `string(n)` |
| `fixed_utf8(n)` | exactly `n` bytes | `char(n)`, `character(n)`, `nchar(n)`, `fixed_string(n)` |
| `ascii` | unbounded US-ASCII | `string(us-ascii)` |
| `fixed_ascii(n)` | exactly `n` US-ASCII bytes | `fixed_string(us-ascii,n)` |
| `string(windows-1252,32)` | at most 32 windows-1252 bytes | any charset alias, case-insensitive |
| `version` | `Version` | - |
| `url` | `Url` | - |

### Bytes

A byte column is one of Arrow's four layouts and a bound. `binary(n)` is a
maximum of `n` bytes; the fixed slot is `fixed_size_binary(n)`, and bytes are
never padded, so a fixed value is exactly its width.

| layout | spelling | with a bound | also parsed as |
| --- | --- | --- | --- |
| 32-bit offsets | `binary` | `binary(n)` | `bytes`, `blob`, `bytea`, `varbinary`, `varbinary(n)` |
| fixed width | `fixed_size_binary(n)` | the exact width | `fixed_binary(n)` |
| 64-bit offsets | `large_binary` | `large_binary(n)` | - |
| view | `binary_view` | `binary_view(n)` | - |

## Use

Declare a string and a byte column, and read the declaration back.

=== "Rust"

    ```rust
    use yggdryl::types::{BytesLayout, StringLayout};
    use yggdryl::{Charset, DataType};

    // Every spelling of a layout is one datatype, rendered under the name
    // its charset earns.
    assert_eq!(DataType::from_str("string")?, DataType::utf8());
    assert_eq!(DataType::from_str("varchar(32)")?.to_string(), "utf8(32)");
    assert_eq!(DataType::from_str("char(8)")?, DataType::fixed_utf8(8)?);
    assert_eq!(DataType::from_str("fixed_string(us-ascii,4)")?, DataType::fixed_ascii(4)?);

    // A charset or a bound is what a string declares, and it reads back.
    let latin = DataType::from_str("string(windows-1252,32)")?;
    let parameters = latin.string_parameters().expect("a string datatype");
    assert_eq!(parameters.layout(), StringLayout::String);
    assert_eq!(parameters.charset(), Charset::Cp1252);
    assert_eq!(parameters.max(), Some(32));
    assert_eq!(latin.charset(), Some(Charset::Cp1252));

    // One number, one reading per layout.
    assert_eq!(DataType::from_str("ascii(4)")?.string_parameters().unwrap().max(), Some(4));
    assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    assert_eq!(DataType::ascii().fixed_byte_width(), None);

    // Bytes: the layout and a bound, nothing else.
    let bounded = DataType::from_str("varbinary(16)")?;
    assert_eq!(bounded.to_string(), "binary(16)");
    assert_eq!(bounded.bytes_parameters().unwrap().layout(), BytesLayout::Binary);
    assert_eq!(bounded.bytes_parameters().unwrap().max(), Some(16));
    assert_eq!(DataType::fixed_size_binary(16)?.fixed_byte_width(), Some(16));
    assert!(DataType::binary().string_parameters().is_none());
    ```

=== "Python"

    ```python
    from yggdryl import BytesParameters, DataType, StringParameters, types

    # Every spelling of a layout is one datatype, rendered under the name
    # its charset earns.
    assert DataType("string") == DataType.utf8() == DataType("varchar")
    assert str(DataType("varchar(32)")) == "utf8(32)"
    assert DataType("char(8)") == DataType.fixed_utf8(8)
    assert DataType("fixed_string(us-ascii,4)") == DataType.fixed_ascii(4)

    # A charset or a bound is what a string declares, and it reads back as
    # a frozen value.
    latin = DataType.string(charset="windows-1252", bound=32)
    assert str(latin) == "string(windows-1252,32)"
    assert latin.string_parameters == StringParameters("string", "windows-1252", 32)
    assert latin.string_parameters.max == 32
    assert latin.charset == "windows-1252"
    assert types.string("name", charset="windows-1252", max=32).dtype == latin

    # One number, one reading per layout.
    assert DataType("ascii(4)").string_parameters.max == 4
    assert DataType.fixed_ascii(4).fixed_byte_width == 4
    assert DataType.ascii().fixed_byte_width is None

    # Bytes: the layout and a bound, nothing else.
    bounded = DataType("varbinary(16)")
    assert str(bounded) == "binary(16)"
    assert bounded.bytes_parameters == BytesParameters("binary", 16)
    assert bounded == DataType.bytes(bound=16)
    assert DataType.fixed_size_binary(16).fixed_byte_width == 16
    assert types.bytes("blob", layout="fixed_size_binary", fixed=16).dtype.id == "fixed_size_binary"
    assert DataType.binary().string_parameters is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // Every spelling of a layout is one datatype, rendered under the name
    // its charset earns.
    assert.ok(DataType.from('string').equals(DataType.utf8()))
    assert.equal(DataType.from('varchar(32)').toString(), 'utf8(32)')
    assert.ok(DataType.from('char(8)').equals(DataType.fixedUtf8(8)))
    assert.ok(DataType.from('fixed_string(us-ascii,4)').equals(DataType.fixedAscii(4)))

    // A charset or a bound is what a string declares, and it reads back as
    // a plain object.
    const latin = DataType.string({ charset: 'windows-1252', max: 32 })
    assert.equal(latin.toString(), 'string(windows-1252,32)')
    assert.deepEqual(latin.stringParameters, {
      layout: 'string',
      charset: 'windows-1252',
      bound: 32,
      max: 32,
    })
    assert.equal(latin.charset, 'windows-1252')
    assert.ok(fields.string('name', { charset: 'windows-1252', max: 32 }).dtype.equals(latin))

    // One number, one reading per layout.
    assert.equal(DataType.from('ascii(4)').stringParameters.max, 4)
    assert.equal(DataType.fixedAscii(4).fixedByteWidth, 4)
    assert.equal(DataType.ascii().fixedByteWidth, null)

    // Bytes: the layout and a bound, nothing else.
    const bounded = DataType.from('varbinary(16)')
    assert.equal(bounded.toString(), 'binary(16)')
    assert.deepEqual(bounded.bytesParameters, { layout: 'binary', bound: 16, max: 16 })
    assert.ok(DataType.bytes({ max: 16 }).equals(bounded))
    assert.equal(DataType.fixedSizeBinary(16).fixedByteWidth, 16)
    assert.equal(fields.bytes('blob', { layout: 'fixed_size_binary', fixed: 16 }).dtype.id, 'fixed_size_binary')
    assert.equal(DataType.binary().stringParameters, null)
    ```

## Charsets and bounds

A bound counts **stored bytes**, not scalars: that is what the buffer holds and
what Arrow's offsets measure. UTF-8 and US-ASCII are validated repertoires, so
bytes that are not what they claim are refused naming the charset, and a
US-ASCII value holds no NUL and no byte above `0x7F`. Every other charset is a
declaration that the column holds legacy bytes, and those are *transcribed*: an
unassigned byte reads as its ISO 8859-1 scalar, which is what the WHATWG
Encoding Standard's own index maps it to.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    // Five scalars are five windows-1252 bytes and seven UTF-8 ones.
    assert_eq!(DataType::from_str("string(windows-1252,5)")?.scalar("Grüße")?.as_str(), Some("Grüße"));
    assert!(DataType::from_str("utf8(5)")?.scalar("Grüße").is_err());

    // Bytes take one door, and the charset decides how strict it is.
    let latin = DataType::from_str("string(windows-1252)")?;
    let value = latin.scalar(Scalar::from(vec![0x47_u8, 0x72, 0xFC, 0xDF, 0x65]))?;
    assert_eq!(value.as_str(), Some("Grüße"));
    // `0x81` is unassigned in windows-1252, and still reads ...
    assert_eq!(latin.scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))?.as_str(), Some("ok\u{0081}"));
    // ... where UTF-8 and US-ASCII refuse what is not theirs.
    let refused = DataType::utf8().scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81])).unwrap_err().to_string();
    assert!(refused.contains("utf-8"), "{refused}");
    let refused = DataType::ascii().scalar("caf\u{e9}").unwrap_err().to_string();
    assert!(refused.contains("non-ASCII"), "{refused}");
    assert!(DataType::ascii().scalar("a\0b").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    # Five scalars are five windows-1252 bytes and seven UTF-8 ones.
    assert DataType("string(windows-1252,5)").scalar("Grüße").as_py() == "Grüße"
    with pytest.raises(ValueError, match="at most 5 bytes"):
        DataType("utf8(5)").scalar("Grüße")

    # Bytes take one door, and the charset decides how strict it is.
    latin = DataType.string(charset="windows-1252")
    assert latin.scalar(b"Gr\xfc\xdfe").as_py() == "Grüße"
    # `0x81` is unassigned in windows-1252, and still reads ...
    assert latin.scalar(b"ok\x81").as_py() == "ok\x81"
    # ... where UTF-8 and US-ASCII refuse what is not theirs.
    with pytest.raises(ValueError, match="utf-8"):
        DataType("utf8").scalar(b"ok\x81")
    with pytest.raises(ValueError, match="non-ASCII"):
        DataType("ascii").scalar("café")
    with pytest.raises(ValueError, match="NUL"):
        DataType("ascii").scalar("a\x00b")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const text = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const strict = { safe: false }

    // Five scalars are five windows-1252 bytes and seven UTF-8 ones.
    const latin = fields.string('name', { charset: 'windows-1252', max: 5 })
    assert.deepEqual(
      Array.from(latin.castArrowArray(text(['Grüße']), strict).get(0)),
      [0x47, 0x72, 0xfc, 0xdf, 0x65],
    )
    assert.throws(
      () => fields.string('name', { max: 5 }).castArrowArray(text(['Grüße']), strict),
      /at most 5 bytes/,
    )

    // US-ASCII is a validated repertoire: no NUL, nothing above 0x7F.
    const ascii = fields.ascii('note')
    assert.deepEqual(Array.from(ascii.castArrowArray(text(['USD']), strict)), ['USD'])
    assert.throws(() => ascii.castArrowArray(text(['café']), strict), /non-ASCII/)
    assert.throws(() => ascii.castArrowArray(text(['a\0b']), strict), /NUL/)
    ```

A value in a legacy charset may hold scalars the charset cannot write - that is
what recovering damage means - and the write seam (the Arrow array build, the
cast) refuses them naming the scalar.

## The values

`Str` and `Bytes` are the crate's string and byte string: a value up to
`INLINE_CAPACITY` (23) text bytes or `INLINE_BYTES` (30) payload bytes lives
inside the value with no heap behind it, a `'static` one costs nothing, and a
longer one is one shared `Arc` that clones by reference count. Equality, order
and hash read the characters or the payload alone, so a value is one value
whichever column holds it. A value remembers the layout and charset it is
written under - and the width, on a fixed layout - and never a maximum: a cell
read out of `utf8(32)` is a `utf8`.

Rust only; the bindings read a value as a `Scalar` and its `dtype`.

```rust
use yggdryl::types::{Bytes, BytesLayout, BytesParameters, INLINE_BYTES, INLINE_CAPACITY, Str, StringLayout, StringParameters};
use yggdryl::{Charset, DataType, Scalar};

// Short text lives inside the value; longer text is one shared handle.
let short = Str::new("AAPL");
assert!(short.is_inline());
assert!(!Str::new("a".repeat(INLINE_CAPACITY + 1)).is_inline());
assert_eq!(std::mem::size_of::<Str>(), 32);
assert_eq!(Scalar::from("AAPL"), Scalar::String(short.clone()));

// Restating a value under other parameters keeps the characters and changes
// what `encode` writes and `dtype` declares; a maximum is checked, not kept.
let latin = short.clone().try_with_parameters(StringParameters::new(StringLayout::LargeString, Charset::Cp1252))?;
assert_eq!(latin, short);
assert_eq!(latin.dtype()?, DataType::from_str("large_string(windows-1252)")?);
let bounded = short.clone().try_with_parameters(StringParameters::utf8(StringLayout::String).try_with_bound(8)?)?;
assert_eq!(bounded.parameters(), StringParameters::default());
assert!(short.clone().try_with_parameters(StringParameters::utf8(StringLayout::String).try_with_bound(2)?).is_err());

// The fixed layout pads on the way out and trims on the way in.
let ccy = Str::from_bytes(b"USD\0", StringParameters::ascii(StringLayout::FixedString).try_with_bound(4)?)?;
assert_eq!(ccy, "USD");
assert_eq!(ccy.encode()?.as_ref(), b"USD\0");
assert_eq!(ccy.fixed(), Some(4));

// Bytes: the same holder over a payload, and a fixed width is exact.
let payload = Bytes::new([1_u8, 2, 3]);
assert!(payload.is_inline());
assert!(!Bytes::new(vec![0_u8; INLINE_BYTES + 1]).is_inline());
assert_eq!(std::mem::size_of::<Bytes>(), 40);
assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(payload.clone()));
let fixed = BytesParameters::new(BytesLayout::FixedSizeBinary).try_with_bound(3)?;
assert_eq!(payload.clone().try_with_parameters(fixed)?.dtype()?, DataType::fixed_size_binary(3)?);
assert!(Bytes::new([1_u8, 2]).try_with_parameters(fixed).is_err());
```

The value door is the same rule in every language: a value read out of a
bounded column answers its layout and charset alone.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let bounded = DataType::from_str("ascii(4)")?;
    assert_eq!(bounded.scalar("USD")?.dtype()?, DataType::ascii());
    assert!(bounded.scalar("EURO!").is_err());
    assert_eq!(DataType::from_str("binary(4)")?.scalar(vec![1_u8, 2, 3])?.dtype()?, DataType::binary());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    bounded = DataType("ascii(4)")
    assert bounded.scalar("USD").dtype == DataType("ascii")
    with pytest.raises(ValueError, match="at most 4 bytes"):
        bounded.scalar("EURO!")
    assert DataType("binary(4)").scalar(b"\x01\x02\x03").dtype == DataType("binary")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields, json } = require('yggdryl')

    const bounded = fields.string('code', { charset: 'us-ascii', max: 4 })
    assert.equal(json.loads('"USD"', { field: bounded, scalar: true }).dtype.toString(), 'ascii')
    assert.throws(() => json.loads('"EURO!"', { field: bounded, scalar: true }), /at most 4 bytes/)
    ```

## Arrow storage

Arrow is told the truth about the bytes. UTF-8 and US-ASCII ride Arrow's text
layouts - ASCII bytes are UTF-8 - and every other charset rides the matching
*binary* layout; the fixed layout rides `FixedSizeBinary` in every charset.
What Arrow cannot say rides an extension document on the field, and only then:

| datatype | Arrow storage | extension name | document |
| --- | --- | --- | --- |
| `utf8`, `large_utf8`, `utf8_view` | `Utf8`, `LargeUtf8`, `Utf8View` | none | - |
| `utf8(32)` | `Utf8` | `yggdryl.string` | `{"layout":"string","charset":"utf-8","max":32}` |
| `ascii` | `Utf8` | `yggdryl.string` | `{"layout":"string","charset":"us-ascii"}` |
| `fixed_ascii(4)` | `FixedSizeBinary(4)` | `yggdryl.string` | `{"layout":"fixed_string","charset":"us-ascii","fixed":4}` |
| `large_utf8_view` | `Utf8View` | `yggdryl.string` | `{"layout":"large_string_view","charset":"utf-8"}` |
| `string(windows-1252)` | `Binary` | `yggdryl.string` | `{"layout":"string","charset":"windows-1252"}` |
| `binary`, `large_binary`, `binary_view`, `fixed_size_binary(16)` | the same four | none | - |
| `binary(16)` | `Binary` | `yggdryl.bytes` | `{"layout":"binary","max":16}` |

A document over a storage it does not describe is a foreign field wearing our
name, and it imports as its storage. A code rides its own extension name
([Codes](codes.md)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    // Plain UTF-8 is Arrow's own datatype and crosses bare.
    let plain = Field::new("text", DataType::utf8(), true).into_arrow()?;
    assert_eq!(plain.data_type(), &ArrowDataType::Utf8);
    assert!(!plain.metadata().contains_key("ARROW:extension:name"));

    // US-ASCII is UTF-8, so it rides the text layout; the charset rides the document.
    let note = Field::new("note", DataType::ascii(), false);
    let arrow = note.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.string");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], r#"{"layout":"string","charset":"us-ascii"}"#);
    assert_eq!(Field::from_arrow(&arrow)?, note);

    // A fixed width is Arrow's fixed binary, whatever the charset.
    let ccy = Field::new("ccy", DataType::fixed_ascii(4)?, false);
    let arrow = ccy.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], r#"{"layout":"fixed_string","charset":"us-ascii","fixed":4}"#);
    assert_eq!(Field::from_arrow(&arrow)?, ccy);

    // A legacy charset rides binary storage, because its bytes are not UTF-8.
    let latin = Field::new("name", DataType::from_str("string(windows-1252)")?, true);
    assert_eq!(latin.clone().into_arrow()?.data_type(), &ArrowDataType::Binary);
    assert_eq!(Field::from_arrow(&latin.clone().into_arrow()?)?, latin);

    // Bytes are the layout; only a maximum needs a document.
    assert!(!Field::new("blob", DataType::binary(), true).into_arrow()?.metadata().contains_key("ARROW:extension:name"));
    let capped = Field::new("blob", DataType::from_str("binary(16)")?, true).into_arrow()?;
    assert_eq!(capped.metadata()["ARROW:extension:name"], "yggdryl.bytes");
    assert_eq!(capped.metadata()["ARROW:extension:metadata"], r#"{"layout":"binary","max":16}"#);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, types

    # Plain UTF-8 is Arrow's own datatype and crosses bare.
    assert types.utf8("text").into_arrow().metadata is None

    # US-ASCII is UTF-8, so it rides the text layout; the charset rides the document.
    note = types.ascii("note", nullable=False)
    arrow = note.into_arrow()
    assert arrow.type == pa.string()
    assert arrow.metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": b'{"layout":"string","charset":"us-ascii"}',
    }
    assert Field.from_arrow(arrow) == note

    # A fixed width is Arrow's fixed binary, whatever the charset.
    ccy = types.fixed_ascii("ccy", 4, nullable=False)
    arrow = ccy.into_arrow()
    assert arrow.type == pa.binary(4)
    assert arrow.metadata[b"ARROW:extension:metadata"] == (
        b'{"layout":"fixed_string","charset":"us-ascii","fixed":4}'
    )
    assert Field.from_arrow(arrow) == ccy
    assert Field.from_arrow(pa.field("ccy", pa.binary(4))) == Field("ccy", "fixed_size_binary(4)")

    # A legacy charset rides binary storage, because its bytes are not UTF-8.
    latin = Field("name", DataType.string(charset="windows-1252"))
    assert latin.into_arrow().type == pa.binary()
    assert Field.from_arrow(latin.into_arrow()) == latin

    # Bytes are the layout; only a maximum needs a document.
    assert types.binary("blob").into_arrow().metadata is None
    capped = Field("blob", "binary(16)").into_arrow()
    assert capped.metadata == {
        b"ARROW:extension:name": b"yggdryl.bytes",
        b"ARROW:extension:metadata": b'{"layout":"binary","max":16}',
    }
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

    // Plain UTF-8 is Arrow's own datatype and crosses bare.
    assert.equal(projected(fields.utf8('text')).metadata.get('ARROW:extension:name'), undefined)

    // US-ASCII is UTF-8, so it rides the text layout; the charset rides the document.
    const note = projected(fields.ascii('note', { nullable: false }))
    assert.equal(String(note.type), 'Utf8')
    assert.equal(note.metadata.get('ARROW:extension:name'), 'yggdryl.string')
    assert.equal(
      note.metadata.get('ARROW:extension:metadata'),
      '{"layout":"string","charset":"us-ascii"}',
    )

    // A fixed width is Arrow's fixed binary, whatever the charset.
    const ccy = projected(fields.fixedAscii('ccy', 4, { nullable: false }))
    assert.equal(String(ccy.type), 'FixedSizeBinary[4]')
    assert.equal(
      ccy.metadata.get('ARROW:extension:metadata'),
      '{"layout":"fixed_string","charset":"us-ascii","fixed":4}',
    )

    // A legacy charset rides binary storage, because its bytes are not UTF-8.
    assert.equal(String(projected(fields.string('name', { charset: 'windows-1252' })).type), 'Binary')

    // Bytes are the layout; only a maximum needs a document.
    assert.equal(projected(fields.binary('blob')).metadata.get('ARROW:extension:name'), undefined)
    const capped = projected(fields.bytes('blob', { max: 16 }))
    assert.equal(capped.metadata.get('ARROW:extension:name'), 'yggdryl.bytes')
    assert.equal(capped.metadata.get('ARROW:extension:metadata'), '{"layout":"binary","max":16}')
    ```

## Casts

A string target that declares anything Arrow cannot - a bound, a fixed width,
a charset other than UTF-8 - validates every cell on the way in
(`StringIngest`); a bounded variable byte target checks every cell's length
(`BytesIngest`). Plain unbounded `utf8` and the four plain byte layouts stay
Arrow's own kernel. A source with a `yggdryl.string` document is read under
its own parameters and restated under the target's; a code source is read as
its trimmed text; bare text storage is read as text; bare binary storage is
read as bytes already in the target charset (a fixed source trimmed of NUL
first). Under `safe` a failing cell becomes null, under strict an error names
the row and the column. The fixed layout pads on the way in, and the stored
column read back under `utf8` trims.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
    use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field};

    let strict = ArrowCastOptions::new().with_safe(false);
    let ccy = Field::new("ccy", DataType::fixed_ascii(4)?, true);

    // A cast into the width pads.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EU"]));
    let padded = ccy.cast_arrow_array(text, strict)?;
    let cells = padded.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(cells.value(1), b"EU\0\0");

    // A stored column carrying the document reads back under `utf8` trimmed.
    let stored = ccy.clone().into_arrow()?;
    let batch = arrow_array::RecordBatch::try_new(
        Arc::new(arrow_schema::Schema::new(vec![stored])),
        vec![padded],
    )?;
    let text = DataType::from_fields([DataType::utf8().required_field("ccy")])?.required_field("row");
    let trimmed = text.cast_arrow_batch(batch, strict)?;
    let trimmed = trimmed.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(trimmed.value(1), "EU");

    // Under `safe` a failing cell is null; strict names the row and the column.
    let long: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EURO!"]));
    let nulled = ccy.cast_arrow_array(Arc::clone(&long), ArrowCastOptions::new())?;
    assert!(nulled.is_null(1));
    let refused = ccy.cast_arrow_array(long, strict).unwrap_err().to_string();
    assert!(refused.contains("row 1") && refused.contains("at most 4 bytes of us-ascii, got 5"), "{refused}");

    // A bounded byte target checks the length and writes the layout.
    let blob = Field::new("blob", DataType::from_str("binary(2)")?, true);
    let bytes: ArrayRef = Arc::new(BinaryArray::from(vec![&b"ab"[..], &b"abc"[..]]));
    assert!(blob.cast_arrow_array(Arc::clone(&bytes), ArrowCastOptions::new())?.is_null(1));
    assert!(blob.cast_arrow_array(bytes, strict).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, types

    ccy = types.fixed_ascii("ccy", 4)

    # A cast into the width pads.
    padded = ccy.cast_arrow_array(pa.array(["USD", "EU"]))
    assert padded.to_pylist() == [b"USD\x00", b"EU\x00\x00"]

    # A stored column carrying the document reads back under `utf8` trimmed.
    stored = pa.record_batch([padded], schema=pa.schema([ccy.into_arrow()]))
    text = DataType.from_fields([types.utf8("ccy")])
    assert text.cast_arrow_batch(stored).column(0).to_pylist() == ["USD", "EU"]

    # Under `safe` a failing cell is null; strict names the row and the column.
    assert ccy.cast_arrow_array(pa.array(["USD", "EURO!"])).to_pylist() == [b"USD\x00", None]
    with pytest.raises(ValueError, match="row 1: expected at most 4 bytes of us-ascii, got 5"):
        ccy.cast_arrow_array(pa.array(["USD", "EURO!"]), safe=False)

    # A bounded byte target checks the length and writes the layout.
    blob = Field("blob", "binary(2)")
    assert blob.cast_arrow_array(pa.array([b"ab", b"abc"])).to_pylist() == [b"ab", None]
    with pytest.raises(ValueError, match="row 1"):
        blob.cast_arrow_array(pa.array([b"ab", b"abc"]), safe=False)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const text = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const strict = { safe: false }
    const ccy = fields.fixedAscii('ccy', 4)

    // A cast into the width pads.
    const padded = ccy.castArrowArray(text(['USD', 'EU']), strict)
    assert.deepEqual(Array.from(padded.get(1)), [0x45, 0x55, 0, 0])

    // A stored column carrying the document reads back under `utf8` trimmed.
    const row = fields.struct('row', [ccy], { nullable: false })
    const stored = row.castArrow(new arrow.Table({ ccy: text(['USD', 'EU']) }), strict)
    const back = fields.struct('row', [fields.utf8('ccy')], { nullable: false }).castArrow(stored)
    assert.deepEqual(Array.from(back.getChild('ccy')), ['USD', 'EU'])

    // Under `safe` a failing cell is null; strict names the row and the column.
    assert.equal(ccy.castArrowArray(text(['USD', 'EURO!'])).get(1), null)
    assert.throws(
      () => ccy.castArrowArray(text(['USD', 'EURO!']), strict),
      /row 1.*at most 4 bytes of us-ascii, got 5/,
    )

    // A bounded byte target checks the length and writes the layout.
    const blob = fields.bytes('blob', { max: 2 })
    const bytes = arrow.vectorFromArray([Uint8Array.of(1, 2), Uint8Array.of(1, 2, 3)], new arrow.Binary())
    assert.equal(blob.castArrowArray(bytes).get(1), null)
    assert.throws(() => blob.castArrowArray(bytes, strict), /row 1/)
    ```

## Serialized shape

One `string` tag for every string and one `binary` tag for every byte column,
with `layout` (omitted when `string` / `binary`), `charset` (omitted when
`utf-8`), and `fixed` or `max` (omitted when unbounded). A scalar crosses as
`{"type":"string","value":...}` or `{"type":"bytes","value":...}`; the value is
bare text or bytes under the default parameters and an object otherwise.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    assert_eq!(DataType::utf8().into_json()?, r#"{"type":"string"}"#);
    assert_eq!(DataType::from_str("utf8(32)")?.into_json()?, r#"{"type":"string","max":32}"#);
    assert_eq!(
        DataType::fixed_ascii(4)?.into_json()?,
        r#"{"type":"string","layout":"fixed_string","charset":"us-ascii","fixed":4}"#
    );
    assert_eq!(DataType::from_str("binary(16)")?.into_json()?, r#"{"type":"binary","max":16}"#);
    assert_eq!(
        DataType::fixed_size_binary(16)?.into_json()?,
        r#"{"type":"binary","layout":"fixed_size_binary","fixed":16}"#
    );
    let field = Field::new("name", DataType::from_str("fixed_string(windows-1252,8)")?, true);
    assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
    // The retired tags are not read back.
    assert!(DataType::from_json(r#"{"type":"utf8"}"#).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    assert DataType("utf8").into_dict() == {"type": "string"}
    assert DataType("utf8(32)").into_dict() == {"type": "string", "max": 32}
    assert DataType.fixed_ascii(4).into_dict() == {
        "type": "string",
        "layout": "fixed_string",
        "charset": "us-ascii",
        "fixed": 4,
    }
    assert DataType("binary(16)").into_dict() == {"type": "binary", "max": 16}
    assert DataType.fixed_size_binary(16).into_dict() == {
        "type": "binary",
        "layout": "fixed_size_binary",
        "fixed": 16,
    }
    field = Field("name", "fixed_string(windows-1252,8)")
    assert Field.from_json(field.into_json()) == field
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    assert.deepEqual(DataType.utf8().toJSON(), { type: 'string' })
    assert.deepEqual(DataType.from('utf8(32)').toJSON(), { type: 'string', max: 32 })
    assert.deepEqual(DataType.fixedAscii(4).toJSON(), {
      type: 'string',
      layout: 'fixed_string',
      charset: 'us-ascii',
      fixed: 4,
    })
    assert.deepEqual(DataType.from('binary(16)').toJSON(), { type: 'binary', max: 16 })
    assert.deepEqual(DataType.fixedSizeBinary(16).toJSON(), {
      type: 'binary',
      layout: 'fixed_size_binary',
      fixed: 16,
    })
    const field = new Field('name', 'fixed_string(windows-1252,8)', true)
    assert.ok(Field.fromJSONBytes(field.toJSONBytes()).equals(field))
    assert.throws(() => DataType.fromJSON({ type: 'utf8' }), /unknown variant `utf8`/)
    ```

## Regex captures

`DataType::from_regex` builds one Struct from a byte regex's named captures, in capture order.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let dtype = DataType::from_regex(
        r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)",
        true,
    )?;
    assert_eq!(dtype.field("level")?.dtype(), &DataType::utf8());
    assert_eq!(dtype.field("id")?.dtype(), &DataType::Int64);
    assert!(dtype.field("id")?.is_nullable());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType.from_regex(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)")
    assert dtype["level"].dtype == DataType("utf8")
    assert dtype["id"].dtype == DataType("int64")
    assert dtype["id"].nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dtype = DataType.fromRegex(
      '\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)',
    )
    assert.equal(dtype.field('level').dtype.toString(), 'utf8')
    assert.equal(dtype.field('id').dtype.id, 'int64')
    assert.equal(dtype.field('id').nullable, true)
    ```

| rule | behaviour |
| --- | --- |
| Nullability | every capture field is nullable |
| Autotyping argument | required in Rust, defaults to `true` in Python and JavaScript |
| Typed captures | boolean, integer, finite float, ISO date, time, datetime |
| Broad captures | a capture such as `\S+` stays `utf8` |
| Rows read | none, so [plain-text records](../media/text.md) publish a schema before opening a source |

## Versions

`Version` holds three numeric components in four bytes: `major: u8`, `minor: u8`,
and `patch: u16`. Parsing accepts one to three decimal components and rendering
omits trailing zero components. Equality, hashing and ordering use the numeric
tuple. Python and JavaScript expose the same immutable native value.

The major and minor are strict; the patch is best effort. A tail stating a
number is that number, whether it states it as `.250` or as a case-insensitive
FIX service pack `sp250`. A tail stating no number - a qualifier, a fourth
component, an extension pack - folds into the patch's sixteen bits through the
crate's stable XXH3 rather than refusing the version. A folded patch is an
identity rather than a quantity: the same tail always reads as the same
version, but it orders arbitrarily against a stated patch, two unlike tails can
fold together, and the canonical text states the fold rather than the tail.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Version};

    let version = "005.0.00300".parse::<Version>()?;
    assert_eq!(version, Version::new(5, 0, 300));
    assert_eq!(version.to_string(), "5.0.300");
    assert_eq!((version.major(), version.minor(), version.patch()), (5, 0, 300));
    assert_eq!(std::mem::size_of::<Version>(), 4);
    assert!(Version::new(5, 0, 2) < Version::new(5, 0, 10));

    let field = Field::new("version", DataType::Version, false);
    assert_eq!(field.scalar("5.0.300")?, Scalar::from(version));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, Version, types
    from yggdryl.text import json

    dtype = DataType("version")
    field = types.version("version", nullable=False)
    version = Version.from_str("005.0.00300")
    value = json.loads('"5.0.300"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == version == Version(5, 0, 300)
    assert (version.major, version.minor, version.patch) == (5, 0, 300)
    assert Version(5, 0, 2) < Version(5, 0, 10)
    assert value.into_arrow_scalar(field).as_py() == "5.0.300"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Version, fields, json } = require('yggdryl')

    const dtype = new DataType('version')
    const field = fields.version('version', { nullable: false })
    const version = Version.fromStr('005.0.00300')
    const value = json.loads('"5.0.300"', {
      field,
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.ok(value.asJs().equals(version))
    assert.ok(version.equals(new Version(5, 0, 300)))
    assert.deepEqual([version.major, version.minor, version.patch], [5, 0, 300])
    assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
    assert.equal(value.intoArrowScalar(field), '5.0.300')
    ```

| rule | behaviour |
| --- | --- |
| Layout | exactly four bytes: `u8` major, `u8` minor, `u16` patch; omitted parts are zero |
| Kind | `text`; `VersionField`, `types.version`, and `fields.version` declare this datatype |
| Bounds | major and minor `0..=255`; patch `0..=65535` |
| Ordering | numeric tuple: `5 < 5.0.2 < 5.0.10 < 5.1` |
| Text | one to three decimal components; `5.0.0` renders as `5` |
| Patch tail | `.250` and `sp250` state 250; any other tail folds to `1..=65535` via XXH3 |
| Storage | `Utf8` holding the canonical spelling, extension name `yggdryl.version` |
| Sorting | Arrow string order stays lexicographic; Rust `Ord`, Python comparisons, and JavaScript `compare` use numeric order |

The parser reads a compact FIX service pack itself, so `5.0SP2` is `5.0.2` and
`5.0sp250` is `5.0.250`, case-insensitively and with no separately stored
qualifier.

<div class="ygg-pg" data-playground="versions" markdown="1">
Explore numeric parts, canonical text, hashes and rejected inputs from the
native Version example corpus.
</div>

## Locations

`Url` is the crate's own [`Url`](../holder/index.md) carried as a column: a value read out of a table is a value a handle can be opened from, not prose that happens to look like one. Parsing canonicalizes and validates, so a column holds one spelling per location and nothing that is not a location.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Url};

    let location = Url::from_str("HTTPS://example.com/a%2fb")?;
    assert_eq!(location.to_string(), "https://example.com/a%2Fb");
    // A bare platform path is a `file:` URL, which is what a local handle is.
    assert_eq!(Url::from_str("/lake/part.txt")?.to_string(), "file:///lake/part.txt");

    let field = Field::new("location", DataType::Url, false);
    assert_eq!(field.scalar("HTTPS://example.com/a%2fb")?, Scalar::from(location));
    // Relative text names no location, so it is not one.
    assert!(field.scalar("./relative").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, types
    from yggdryl.text import json

    dtype = DataType("url")
    field = types.url("location", nullable=False)
    value = json.loads('"HTTPS://example.com/a"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == "https://example.com/a"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const dtype = new DataType('url')
    const value = json.loads('"HTTPS://example.com/a"', {
      field: fields.url('location', { nullable: false }),
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.equal(value.asJs(), 'https://example.com/a')
    ```

| rule | behaviour |
| --- | --- |
| Kind | `text`; the aliases are `UrlField`, `types.url`, `fields.url` |
| Value | `crate::Url` behind one shared pointer, so a row clone moves a reference count rather than a URI |
| Storage | `Utf8` holding the canonical text, extension name `yggdryl.url` |
| Ordering | the canonical text's, which is Arrow's own string order; there is no numeric component to sort by |
| Default | `file:///`, the shortest URL the validator accepts, because a location has no zero |
| Merging | only with itself: merging into text would drop the validation that makes it a URL |

## Edges

- `fixed_string`, `fixed_size_binary` with no width -> refused; the width is what makes a layout fixed. A bound of `0` -> refused, `at least one byte, got 0`.
- `utf8(windows-1252)`, `ascii(windows-1252)` -> refused; a charset-named spelling declares its charset in the name.
- `ascii(4)`, `binary(16)` -> maxima; `fixed_ascii(4)`, `fixed_size_binary(16)` -> widths. `varchar(255)` -> `utf8(255)`; `char(8)` -> `fixed_utf8(8)`; bare `char` -> `utf8`.
- A bound counts stored bytes, so `utf8(4)` refuses `Grüß` and `string(windows-1252,4)` holds it.
- `utf8`, `utf8(n)`, `fixed_utf8(n)`, `large_utf8_view` read bytes strictly, naming the charset; `string(<legacy charset>)` transcribes, an unassigned byte as its ISO 8859-1 scalar.
- A US-ASCII value -> no NUL, no byte above `0x7F`, refused naming the byte and its position; a variable US-ASCII value keeps its length, only the fixed layout trims trailing NUL.
- A fixed string stores its value padded with trailing NUL and reads back trimmed; a fixed byte value is exactly its width, never padded.
- Text a legacy charset has no bytes for -> held as a value, refused when the column is written, naming the scalar; the value door counts rather than judges.
- A value never carries a maximum: `Scalar::dtype()` of a cell read out of `utf8(32)` is `utf8`, of `binary(16)` is `binary`.
- `Scalar::from("USD")` and a value read out of an `ascii` column are one value; `Str` equality, order and hash read the characters alone.
- `string_parameters` on a code, `bytes_parameters` on a UUID -> `None`; `fixed_byte_width` answers for a fixed string, fixed bytes, a code, a UUID and the numbers.
- A `yggdryl.string` or `yggdryl.bytes` document over a storage it does not describe -> imports as the storage.
- Arrow JS rows carry no extension identity, so a `fixed_ascii(n)` column arrives as its padded bytes through `readRecords`; declare `utf8` to read text.
- A `StringIngest` or `BytesIngest` refusal under `safe` -> null, which a required column then fills with the default; under strict -> `field "<name>" row <n>: expected ..., got ...`.
- Avro and Iceberg -> a string with text storage (UTF-8 or US-ASCII) crosses as `string`, a fixed one trimmed of padding; any other charset is refused by name; a bounded byte column crosses unbounded, the bound enforced where values enter.
- Merging follows [Field](field.md): two strings and two byte types meet parameter by parameter.
- `from_regex(pattern, false)` -> every capture stays `utf8`; invalid regex syntax or an expression past the recursion limit -> datatype error.
- `005.0.000` -> the canonical `5`; a version whose major or minor exceeds `255`, or whose major is not a decimal number -> refused at the first bad byte; a fourth component, an empty component, a qualifier, or a patch above `65535` -> folded into the patch.
- Fractional or out-of-range constructor arguments in Python or JavaScript -> refused without narrowing.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::string datatype::bytes datatype::ascii field::ascii field::binary
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::string types::bytes types::tests::strings types::tests::bytes types::regex types::tests::version
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(string|bytes)/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "string or bytes or ascii or byte_column"
    python/.venv/bin/python -m pytest python/tests/media/test_text_lines.py -k regex
    python/.venv/bin/python -m pytest python/tests/types/test_version.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="string|byte|ASCII|ascii|regex captures" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    node --test node/tests/types/version.test.js
    npm run --prefix node bench:types
    ```

## Performance

AMD Ryzen 5 150, 12 logical CPUs, Windows; Rust 1.96, Python 3.12.13 and Node
24.18, release builds. Rust reports Criterion point estimates; Python reports
the median of five runs of 10,000 iterations; Node reports throughput over
2,000 iterations after warmup. These harnesses measure different boundaries.

| operation | Rust | Python native boundary | JavaScript native boundary |
| --- | ---: | ---: | ---: |
| parse | 28.8 ns | 187.6 ns/op | 320,631 ops/s |
| numeric compare | 3.78 ns | 142.5 ns/op | 1,160,631 ops/s |
| native parts construction | 3.85 ns | 186.0 ns/op | 558,722 ops/s |
| native value into `Scalar` | — | 317.9 ns/op | 38,527 ops/s |
| `Scalar` into host `Version` | — | 82.7 ns/op | 142,733 ops/s |

The Rust maximum-width parse (`255.255.65535`) measured 35.7 ns. The four-byte
value needs no heap allocation for parsing or comparison; host wrappers and
text or Arrow projections have their own allocation costs.

Rust's native-parts case also reads all three accessors. The binding cases
measure constructor calls. The counting-allocator test
`version_parse_compare_and_render_allocate_nothing` checks the core allocation
claim independently of these timings.

The string and byte family boundaries, on the same host, from the binding
gates that landed the family (Python: the `datatypes.py` benchmark on the
release wheel; Node: `bench:types` with `YGGDRYL_BENCH_ITERATIONS=20000`). The
Rust rows await a regenerate of `cargo bench --bench types -- '^(string|bytes)/'`.

| operation | Python native boundary | JavaScript native boundary |
| --- | ---: | ---: |
| `fixed_ascii(3)` datatype | 195.9 ns/op | 509,466 ops/s |
| `string(windows-1252,32)` datatype | 543.1 ns/op | 223,772 ops/s |
| `string_parameters` read | 214.9 ns/op | 439,389 ops/s |
| `binary(16)` datatype | 387.9 ns/op | 335,892 ops/s |
| `bytes_parameters` read | 121.5 ns/op | — |
| `fixed_byte_width` read | — | 1,584,937 ops/s |
| string field | 1,499.6 ns/op | 88,122 ops/s |
| bytes field | 1,571.7 ns/op | 79,514 ops/s |

```bash
cargo bench -p yggdryl --bench types -- version --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(string|bytes)/'
python/.venv/bin/python python/benchmarks/types/version.py --iterations 10000
python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
YGGDRYL_BENCH_ITERATIONS=2000 npm run --prefix node bench:types
```

On Windows, use `python/.venv/Scripts/python.exe` and set
`$env:YGGDRYL_BENCH_ITERATIONS = '2000'` before the Node command.
